<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Direct boot on the Verdin AM62P: Zephyr in place of U-Boot proper

Goal (2026-10-02): the same power-on -> Zephyr -> cluster form as on the
i.MX8MP, for the Toradex Verdin AM62P on the Dahlia carrier, which boots
Torizon from its eMMC today.

Status (2026-10-02): images built and checked offline, **nothing has run on
the board**, and the board was not touched (it was busy with a dEQP run; one
read-only ssh attempt timed out). Unlike the X2, a picture is possible on the
first direct boot: the cold display path (DSS1 -> DSI -> SN65DSI83 -> LVDS
panel) is in the C drivers and first ran on this board under a U-Boot `go`
entry. **Size**: `triangle` is 15.7 MB and `slint` 18.9 MB (most of
it the offline shader table, about 12 MB), and this module's boot partitions
are 4 MiB each (`/sys/block/mmcblk0boot*/size` = 8192 sectors, read
2026-10-02), so the u-boot.img slot holds 1.49 MiB. The GL images therefore
go in two parts: a second-stage loader (95.9 KB) takes the slot and reads
the image, deflate-compressed, from a raw sector range of the eMMC user area
("The second stage" below). Built and checked offline; not run.

**2026-10-21.** Ported from the `am62p/direct-boot` and `am62p/direct-boot-size`
branches of 2026-10-02 onto today's tree (the board variant, the second-stage
loader and `tools/am62p-mkboot.py` unchanged in substance; the sample glue
moved to the board layer, so the boot-time marks are
`CONFIG_YAKOGL_BOARD_BOOT_TIME` in `driver/zephyr/lib/board`). The loader
gained a second payload source, **the payload linked into the loader itself**
(`BOOT_STAGE2_SOURCE_EMBEDDED`, now the default): with the in-process shader
compiler instead of the offline table the direct triangle is 1.81 MB and
0.87 MB deflated, so loader plus payload fit the 1.49 MiB u-boot.img slot and
the eMMC user area is not written at all. The eMMC source below stays for an
image whose compressed form outgrows the slot. The same loader serves the
Verdin iMX8MP (docs/notes/imx8mp-direct-boot.md). Still nothing has run on
the board; the owner's target is now 100 ms from power-on to the first frame,
and the first board run's job is the per-stage times.

## The chain

```
Boot ROM ── eMMC boot operation on the enabled boot partition (boot0 today)
  └─ tiboot3.bin @0x0: R5 SPL + TIFS + board configuration (TI-signed), WKUP R5F
       DDR training, ESM setup (CONFIG_ESM_K3), 32 kHz LFXOSC bypass (spl_board_init)
       └─ loads tispl.bin @0x80000 (raw sector 0x400), a FIT:
            atf          TF-A BL31 v2.13   -> 0x80000000, entry
            tee          OP-TEE 4.7        -> 0x9e800000
            dm           DM firmware (ELF) -> 0x89000000 staging, the R5 SPL relocates it
            spl          A53 U-Boot SPL    -> 0x80080000
            tifsstub-*   TIFS stub         -> 0x9ca00000
            fdt-0        the A53 SPL's devicetree
       └─ enables the GTC (system counter), starts the A53 at BL31, becomes the DM
            └─ BL31 runs OP-TEE's init, enters BL33 = PRELOADED_BL33_BASE 0x80080000
               = the A53 SPL, in non-secure EL2, x0 = K3_HW_CONFIG_BASE 0x82000000
                 └─ A53 SPL: console, CTRL_SLEEP_MOCI# hog, reads u-boot.img @0x280000
                    (raw sector 0x1400 of the same boot partition), spl_load_simple_fit:
                      uboot  "Zephyr (in place of U-Boot proper)" -> 0x82000000, entry
                      fdt-0  U-Boot's devicetree, TI-signed, appended after uboot
                    dcache and MMU off (spl_board_prepare_for_boot), jumps, no arguments
                      └─ Zephyr: __start, EL2 -> EL1, the sample
```

The X2 puts Zephyr in BL33's slot. Here BL33 is the A53 SPL and it stays: it
is the stage that reads the eMMC, and it is inside `tispl.bin`, which the R5
SPL loads and TIFS authenticates. Zephyr takes the place of U-Boot proper, the
SPL's next stage. tiboot3.bin and tispl.bin are Toradex's files, unchanged.

## What the prebuilt images are

Torizon OS 7.6.1+build.38 for the Verdin AM62P, Toradex's release of
2026-05-13, from
<https://artifacts.toradex.com/artifactory/torizoncore-oe-prod-frankfurt/scarthgap-7.x.y/release/38/verdin-am62p/torizon/torizon-docker/oedeploy/>.
The sha256 values below are the ones Artifactory reports for those files; the
copies used here (from a Torizon image already on this laptop) match them. The
board's U-Boot environment says `ver=U-Boot 2024.04-ti-g11cb0162a795`
(docs/notes/am62p-netboot.md), the same string as in this u-boot.img.
That the board's boot0 holds exactly these bytes is checked at the board
(write-emmc.txt step 2).

| file | bytes | sha256 |
|---|---|---|
| `tiboot3-am62px-hs-fs-verdin.bin` | 321,832 | `71117f3960aaf1a8a100009d2c0d81917aa95aa6596403ac1562b5d8b80db1ee` |
| `tispl.bin` | 1,350,223 | `8af94cd0aabd3baed0e77295c1bf8a7ab79d0293e77e9acd7c3550ae1385c2f3` |
| `u-boot.img` | 1,049,243 | `6c3fc9f6396e3080fc6961da1fce39f04032c8dc4f8f125d59f4950abd53e1d2` |
| `torizon-docker-verdin-am62p-Tezi_7.6.1+build.38.tar` | 254,300,160 | `de1522ce25f91ab7a5a9d4a92e21fd74a39858a2732e825b1250208b89328f61` |

Where they live: the Tezi `image.json` writes all three raw into
`mmcblk0boot0` (`"erase": true`) at `seek=0`, `seek=1024` and `seek=5120`
sectors, that is 0x0, 0x80000 and 0x280000. Those are also the SPLs' raw
sectors (`CONFIG_SYS_MMCSD_RAW_MODE_U_BOOT_SECTOR` 0x400 in
`verdin-am62p_r5_defconfig`, 0x1400 in `verdin-am62p_a53_defconfig`). U-Boot's
environment is in boot0 too: `CONFIG_ENV_OFFSET=0xFFFFDE00` (0x2200 bytes
before the end), `ENV_SIZE 0x2000`, `CONFIG_ENV_MMC_EMMC_HW_PARTITION=1`; the
board's `/etc/fw_env.config` agrees (`/dev/emmc-boot0`, `-0x2200`). In eMMC
boot mode the SPLs read from the partition that EXT_CSD PARTITION_CONFIG
enables (`default_spl_mmc_emmc_boot_partition`: 1 = boot0, 2 = boot1), so a
complete set written to boot1 boots from boot1 once boot1 is enabled.

Layout of the prebuilt `u-boot.img` (`tools/am62p-mkboot.py info`):

```
u-boot.img: 1049243 bytes, FIT header 1049243 bytes, data embedded
  uboot  "U-Boot for Verdin AM62P5 Boards" firmware os u-boot size 0x0eb56c load 0x80800000 entry -  cert 1700 bytes crc32 ok
  fdt-0  "k3-am62p5-verdin-wifi-dev"       flat_dt            size 0x014644                          cert 1700 bytes crc32 ok
  conf-0 (default): firmware=uboot; loadables=uboot; fdt=fdt-0
```

Every payload starts with a TI x509 certificate (`30 82 06 a0 ...`).
`tispl.bin` has the same form (atf, tee, dm, spl, tifsstub-hs, tifsstub-fs,
fdt-0; conf-0 firmware=atf, loadables=tee, tifsstub-hs, tifsstub-fs, dm, spl).

### Facts from the source

U-Boot v2024.04 and master (<https://source.denx.de/u-boot/u-boot>), TF-A
v2.13.0 (<https://github.com/ARM-software/arm-trusted-firmware>). The board's
U-Boot is TI's 2024.04 tree (`-ti-g11cb0162a795`), so the mainline files are
the reference, not a byte match; the A53 SPL's own strings confirm the parts
marked so.

- TF-A `plat/ti/k3/platform.mk`: `PRELOADED_BL33_BASE ?= 0x80080000`,
  `BL32_BASE ?= 0x9e800000`, `K3_HW_CONFIG_BASE ?= 0x82000000`.
  `plat/ti/common/ti_bl31_setup.c`: BL33 enters in EL2 when EL2 exists
  (`k3_get_spsr_for_bl33_entry`), all exceptions masked, x0 =
  `K3_HW_CONFIG_BASE`. `plat_get_syscnt_freq2` reads the GTC and prints "GTC
  is disabled! ... Fix Bootloader" if nobody enabled it. In this tispl.bin
  BL31 loads at 0x80000000 (512 KiB; U-Boot's `fdt_fixup_reserved` moves the
  `tfa` reservation there for Linux), not at the 0x9e780000 the DT names.
- `common/spl/spl_fit.c` `spl_load_simple_fit`: the `firmware` image is
  loaded at its `load` (entry = `entry`, else `load`); an `os = "u-boot"`
  image gets the configuration's `fdt` appended at `ALIGN(load + size, 8)`.
  With `data-position` the SPL reads only the FIT header into a malloc'd
  buffer and each payload straight to its load address; with embedded data it
  reads the whole FIT first (into malloc, or failing that at
  `CONFIG_TEXT_BASE`). This SPL checks no FIT hashes (no `SPL_FIT_SIGNATURE`;
  "## Checking hash" is not among its strings).
- `arch/arm/mach-k3/security.c`: each payload goes through
  `ti_secure_image_post_process`. On an HS-FS device a payload without a
  certificate is loaded after "Warning: Did not detect image signing
  certificate. Skipping authentication to prevent boot failure. This will fail
  on Security Enforcing(HS-SE) devices" (the string is in the A53 SPL).
- `drivers/remoteproc/ti_k3_arm64_rproc.c` `k3_arm64_load`: the R5 SPL writes
  `GTC_CNTFID0` and sets `GTC_CNTCR` EN just before it starts the A53. **So the
  system counter starts there, not at power-on**: ROM, R5 SPL, DDR training
  and the tispl.bin load are not in it.
- `verdin-am62p_a53_defconfig`: `CONFIG_SPL_STACK_R_ADDR=0x82000000` (stack
  and simple malloc pool below it), `SPL_BSS_START_ADDR=0x80c80000`; no
  `SPL_GZIP` and no SPL filesystem (no "Uncompressing" or FAT/ext4 strings in
  the A53 SPL): raw eMMC, USB DFU and RAM are its only sources.
- The A53 SPL's `dfu_alt_info_ram` (its strings, and the recovery U-Boot's):
  `u-boot.img ram 0x81000000 0x400000`, a 4 MiB slot.

## The Zephyr side: `verdin_am62p/am62p54/a53/direct`

`driver/boards/toradex/verdin_am62p`: `verdin_am62p_am62p54_a53_direct.{dts,yaml,_defconfig}`
(board.yml variant `direct`, cpucluster a53). The a53 variant is unchanged.

- **Link address 0x82000000, the a53 variant's.** The FIT carries
  load = entry = 0x82000000, so the SPL puts Zephyr where the U-Boot `go` and
  kexec entries do, and one memory map serves all three. 0x80800000 (U-Boot
  proper's) would work too, but would move the image's RAM and buy nothing.
- **DDR map** (2 GiB at 0x80000000):

  | range | owner |
  |---|---|
  | 0x80000000 + 512 KiB | TF-A BL31, secure, never mapped |
  | 0x80080000 .. 0x82000000 | the A53 SPL, its BSS, stack and malloc pool; dead once Zephyr runs |
  | 0x82000000 + 344 MiB | Zephyr (`zephyr,sram`). The SPL appends U-Boot's DT after zephyr.bin, inside Zephyr's .bss; the R5 SPL's DM staging copy at 0x89000000 is consumed before the A53 starts |
  | 0x97800000 + 64 MiB | GPU carveout |
  | 0x9b800000 .. 0x9e7fffff | MCU R5F and WKUP/DM R5F regions (the DM runs here) |
  | 0x9e800000 + 24 MiB | OP-TEE, secure |

- **EL2 entry**, MMU and data cache off: TF-A enters the SPL in EL2, the SPL
  stays there and calls `dcache_disable()` before the jump. Zephyr's reset
  path drops to EL1, as under U-Boot `go` (where the caches were turned off
  by hand first).
- **Display on in the board DTS**: DSS1, DSI, the SN65DSI83 on I2C1, the GPIO
  banks and the TISCI power-on list, which the a53 variant leaves to each
  sample's overlay. Nobody else owns the panel on this route.
- **No inherited watchdog.** The direct confs set
  `CONFIG_YAKOGL_AM62P_RTI_SELF_ARM=y` (the netboot plan's Z1) with the
  sample's bound (150 s triangle, 200 s slint).
- **Boot time**: `CONFIG_YAKOGL_BOARD_BOOT_TIME=y` prints
  `system counter at main(): N us` and `system counter at the first frame: N us`
  (after the window is on the DSS plane). The counter is the GTC at 200 MHz,
  started by the R5 SPL just before the A53 (above); the host clock from the
  R5 SPL's banner to `Starting ATF on ARM64 core...` gives the part before.

### What U-Boot proper did that Zephyr now has to do

Nothing that the display or the GPU depends on. U-Boot proper on this board
(the payload's strings, `board/toradex/verdin-am62p/verdin-am62p.c`, the
defconfig) loads its environment, picks the DT variant, probes Ethernet and
MMC for `bootflow scan`, fixes up Linux's DT (memory size, reserved regions,
thermal trips) and boots Linux. It has no video/tidss driver and no RTI
watchdog driver (only EFI's watchdog strings). What remains, and who does it:

| what | U-Boot proper route | direct |
|---|---|---|
| DDR, PLLs, ESM, 32 kHz LFXOSC, GTC | R5 SPL, TIFS, DM | the same |
| GIC, PSCI for the other cores | TF-A | the same |
| console pads (main_uart0), CTRL_SLEEP_MOCI# hog | SPLs (DT `bootph-all`) | the same |
| power domains of GPU, DSS, DSI | Zephyr drivers (TISCI) | the same |
| I2C1, GPIO0, GPIO1 on | Zephyr `yakogl,tisci-power-on` | the same, now in the board DTS |
| panel and I2C pads | Zephyr pinctrl | the same |
| RTI watchdog | nobody (no driver in U-Boot) | Zephyr arms it (`rti_arm.c`) |
| DDR size | U-Boot detects it for Linux | Zephyr's DT is static (2 GiB module) |
| caches off before the jump | `dcache off; icache off; go` by hand | the SPL's `dcache_disable()` |

So the direct image starts from the state the U-Boot `go` entry had,
minus U-Boot proper's Ethernet/MMC/USB probing. That this is the whole
difference is unmeasured.

## Building

The board files and samples are in this repository. Build from the workspace
that holds Zephyr (af17c0c8 was used), with the module list given explicitly
when that workspace also carries another checkout
(`tools/cts/zephyr-modules.sh`), and with the PowerVR firmware header in
`driver/src/fw/` (never committed):

```sh
west build -p always -b verdin_am62p/am62p54/a53/direct -d build/am62p/triangle_direct \
    yakogl/examples/triangle
RUSTUP_TOOLCHAIN=nightly west build -p always -b verdin_am62p/am62p54/a53/direct \
    -d build/am62p/slint_direct yakogl/examples/slint \
    -- -DSLINT_NOSTD_DIR=<no_std Slint workspace>/slint
tools/am62p-mkboot.py build --base u-boot.img --zephyr build/am62p/triangle_direct/zephyr/zephyr.bin \
    -o build/am62p/u-boot-triangle.img
tools/am62p-mkboot.py bootpart --tiboot3 tiboot3-am62px-hs-fs-verdin.bin --tispl tispl.bin \
    --uboot build/am62p/u-boot-triangle.img --size <boot partition bytes> -o bootpart.bin
```

`am62p-mkboot.py` (standard library only) keeps every node and property of
Toradex's u-boot.img and changes the firmware's payload, description,
`load`, `entry` and crc32. It moves all data out of the FDT (`data-position`,
4 KiB aligned) and copies fdt-0 with its certificate byte for byte. It then
re-parses the result: the configuration, fdt-0 and its properties are as in
the base, the payload is zephyr.bin, load = entry, the data are sector
aligned, and the image's `image_size` (from the arm64 header, so .bss
included) ends below the GPU carveout. `check` repeats this on a written
file (a wrong zephyr.bin makes it fail). `repack` writes the control: U-Boot
proper unchanged, with external data; if that boots Torizon, the packing is
right. `bootpart` lays out a whole boot partition and refuses a u-boot.img
that reaches U-Boot's environment. `dtc -I dtb` decompiles the result.
`--self-test` (a `tools/check.sh` row) runs all of this, and the refusals, on
a synthetic FIT in the prebuilt's form, so it needs no Toradex file.

Results on 2026-10-02 (this branch, Zephyr af17c0c8; the warnings are the a53
variant's own: two DSI macro redefinitions, two unused functions in
pvr_fw_boot.c, tisci.c's format, "No SOURCES ... drivers__display", and the
no_std Slint trees' Rust cfg warnings):

2026-10-21, today's tree, the direct variants with the loader (the whole
boot image is loader + deflated payload; `tools/am62p-mkboot.py bootpart`
refuses one that reaches U-Boot's environment):

| image | zephyr.bin | deflate | note |
|---|---|---|---|
| triangle (in-process compiler) | 1.81 MB | 0.87 MB | fits the slot |
| slint (in-process compiler) | 4.78 MB | 1.77 MB | does not fit: the eMMC source, or the next row |
| slint, precompiled femtovg shaders only, no compiler (`YAKOGL_SHADERC_PRECOMPILED_ONLY`, `YAKOGL_PRECOMPILED_APPS="femtovg"`) | 5.48 MB | 1.38 MB | fits; and compiles no shader at boot |

The 2026-10-02 figures, for the record:

| image | zephyr.bin | u-boot.img | needs a boot partition of |
|---|---|---|---|
| triangle | 15,730,672 B, `47dcb4d9...` | 15,820,356 B, `e0570e58...` | 18,450,500 B (0x1198844) |
| slint | 18,929,864 B, `b418d40d...` | 19,019,332 B, `bb2867ba...` | 21,649,476 B (0x14a5844) |
| hello_world (GPU and display off) | 69,992 B, `57934948...` | 161,348 B, `568081bd...` | 2,791,492 B (fits 4 MiB) |
| control (U-Boot proper repacked) | - | 1,054,276 B, `f05edda0...` | 3,684,420 B |

triangle's .rodata is 15.3 MB; the offline shader blobs are about 12 MB
of it (`SHC_BLOB_TABLE` and the blobs it points to), the GPU firmware 229 KB.

## The second stage

The u-boot.img slot of a 4 MiB boot partition runs from 0x280000 to U-Boot's
environment 0x2200 bytes before the end: 1,564,160 bytes, less the FIT
header and fdt-0 (85 KB with its certificate). What was measured and weighed
(2026-10-02, Zephyr af17c0c8):

| option | finding | verdict |
|---|---|---|
| (a) in-process compiler, no table (`CONFIG_YAKOGL_SHADERC_ONLINE=y`) | triangle 1,542,472 B (triangle calls `gles_offline_shader_source`, so this is a size-only link with that symbol stubbed), slint 4,774,432 B | neither fits the slot with fdt-0 (triangle is 64 KB over, slint 3.3 MB). Rejected. |
| (b) a second-stage loader in the slot, the image in the user area | Zephyr af17c0c8 has `drivers/sdhc/sdhc_ti_am654.c` (TI, "ti,am654-sdhci", ADMA2) and the `zephyr,mmc-disk` disk driver; `sk_am62` uses them for its eMMC. No U-Boot rebuild, nothing proprietary | **chosen** |
| (c) rebuild the A53 SPL (tispl.bin) to read u-boot.img from the user area | U-Boot v2024.04 `common/spl/spl_mmc.c` `default_spl_mmc_emmc_boot_partition` takes `CONFIG_SYS_MMCSD_RAW_MODE_EMMC_BOOT_PARTITION` (with `SUPPORT_EMMC_BOOT_OVERRIDE_PART_CONFIG`) in place of PARTITION_CONFIG, so the partition is a config choice; but the override value is used as is (only the PARTITION_CONFIG path maps 7 to 0), the free user-area room is 4 MiB anyway (below), so an 18.9 MB image would still need `SPL_GZIP`/`SPL_LZMA` (`spl_fit.c` supports both), and tispl.bin must be re-assembled around a rebuilt SPL from TI's tree (TF-A, OP-TEE, the DM firmware and the TIFS stubs taken from Toradex's tispl.bin) | more moving parts than (b) and a U-Boot build to maintain. Rejected. |
| (d) `mmc bootpart-resize` | one-time, irreversible change of the eMMC's partitioning | not proposed |

**Where in the user area.** The board's eMMC (`sfdisk -d /dev/mmcblk0`,
read-only, 2026-10-02): a DOS label, one partition, `mmcblk0p1` from sector
8192 to the end (30,539,776 sectors, 14.6 GiB), filled by Torizon's ext4
(`otaroot`, 3,816,448 blocks of 4 KiB = the whole partition). There is no
free space after it. Sectors 1..8191 before it were read and are all zero;
U-Boot's environment and Toradex's config block are in boot0, not here. So
the image goes at **sectors 64..8191** (header at 64 = byte 0x8000; 64 keeps
clear of sector 0, the DOS table, and of sectors 1..33, where a GPT would
be), 4,161,536 bytes of payload. Uncompressed neither GL image fits that
either, which is why the payload is raw deflate: triangle 2,001,795 B,
slint 2,944,051 B (Python zlib level 9). A dedicated partition was
not chosen: making room means shrinking Torizon's root filesystem offline.

**The loader** (`driver/samples/zephyr/boot_stage2`, board
`verdin_am62p/am62p54/a53` with the sample's overlay):

- The A53 SPL loads it from u-boot.img like the direct images, at
  **0x80800000** (U-Boot proper's address in Toradex's FIT; the SPL's BSS at
  0x80c80000 and its stack below 0x82000000 are past the 95.9 KB file and
  the 85 KB devicetree the SPL appends). Its RAM is 0x80800000 + 24 MiB,
  ending where the image begins; `KERNEL_VM_SIZE` keeps device mappings in
  it. One core (`SMP=n`): the others stay off for the image.
- The overlay maps 0x82000000..0x97800000 (the image's region, up to the GPU
  carveout) identity, write-back (`zephyr,memory-region` +
  `zephyr,memory-attr`), and turns on MMC0 (`main_sdhci0`) with the AM62P's
  values from Linux (`k3-am62p-j722s-common-main.dtsi`, "ti,am64-sdhci-8bit":
  DLL present, `ti,trm-icp` 8, the legacy/HS tap delays; 50 ohm from
  `k3-am62p-verdin.dtsi`), 8 bit, HS200 removed so the card runs HS at 52 MHz
  with no tuning. Its interrupt (GIC SPI 133) is now in the AM62P dtsi.
  MMC0 is switched on through TISCI first (`tisci_power_on`, device 57, as
  Linux's `power-domains`): after an eMMC boot the SPLs left it on, after a
  USB (DFU) boot -- the no-write test -- nobody did.
- `main()` (C): `disk_access_init("emmc")` (the SD stack starts from CMD0,
  which also leaves the boot partition the SPL read from: PARTITION_ACCESS
  is reset by CMD0), reads the header sector, reads the payload in 1 MiB
  reads to the staging address 0x90000000 (deflate) or straight to
  0x82000000 (uncompressed), unpacks, then quiesces: the SDHC interrupt off
  and the controller reset, `irq_lock()`, `sys_cache_data_flush_and_invd_all()`,
  `sys_cache_instr_invd_all()`, `bs2_jump()`. Any refusal prints the reason
  and stops (no reset loop).
- The logic is Rust (`loader/`, crate `yakogl-boot-stage2`, no
  dependencies, `aarch64-unknown-none-softfloat`, built through
  `capi/yakogl_capi.cmake`): the header check (magic, header CRC-32,
  version, load address = the one the loader serves, entry inside the image,
  stored size within the user-area range, image size below the staging
  buffer), CRC-32 of the stored bytes, a raw-deflate decoder written from
  RFC 1951 (flat output buffer, every malformed input an error), length and
  CRC-32 of the image, the arm64 Image magic, and the jump: DAIF masked, both
  EL1 timers stopped, SCTLR_EL1 M/C/I off, `ic iallu`, `tlbi vmalle1`,
  `br` to the entry with x0..x3 = 0. The loader's code is identity mapped
  (KERNEL_VM_BASE = its RAM base), so turning the MMU off is safe in place.
- **EL1, not EL2**: Zephyr's reset path dropped the loader to EL1, so the
  image is entered at EL1. Its reset path takes EL1 entry
  (`arch/arm64/core/reset.S`, the `1:` branch), and the EL2 set-up it would
  have done (HCR_EL2, CNTHCTL, ICC_SRE_EL2) the loader's identical reset path
  already did. This is the one difference from the one-stage route.

The header (one 512-byte sector, little-endian; `loader/src/lib.rs`):
magic `YKSTAGE2`, version 1, header size 512, compression (0 none, 1 raw
deflate), load, entry, stored bytes, image bytes, CRC-32 of each, a 32-byte
name, and at 508 the CRC-32 of bytes 0..508. `am62p-mkboot.py stage2img`
writes it (and re-reads the result: header, both CRCs, zlib decode = the
zephyr.bin), `stage2check` re-reads a file, `stage2-vectors` writes the test
images that the crate's host test decodes (`tools/check.sh` row "am62p
second stage": Python's zlib against the loader's Rust).

Building:

```sh
west build -p always -b verdin_am62p/am62p54/a53 -d build/am62p/boot_stage2 \
    yakogl/driver/samples/zephyr/boot_stage2
tools/am62p-mkboot.py build --base u-boot.img --zephyr build/am62p/boot_stage2/zephyr/zephyr.bin \
    --load 0x80800000 --description "YakoGL boot_stage2 (in place of U-Boot proper)" -o u-boot-stage2.img
tools/am62p-mkboot.py bootpart --tiboot3 tiboot3-am62px-hs-fs-verdin.bin --tispl tispl.bin \
    --uboot u-boot-stage2.img --size 4194304 -o bootpart-stage2-4MiB.bin
tools/am62p-mkboot.py stage2img --zephyr build/am62p/slint_direct/zephyr/zephyr.bin \
    --name slint -o stage2-slint.img       # for user-area sector 64 (dd seek=64)
```

| file | bytes | where |
|---|---|---|
| boot_stage2 zephyr.bin | 95,900 | u-boot.img payload, 0x80800000 |
| u-boot-stage2.img | 185,924 | boot partition @0x280000 (slot 1,564,160) |
| stage2-triangle.img | 2,002,432 (3,911 sectors) | user area, sectors 64..3974 |
| stage2-slint.img | 2,945,024 (5,752 sectors) | user area, sectors 64..5815 |

## At the board

**Done on 2026-10-21, 22:13-22:50 JST (the owner at the board, USB-C X3 to
the laptop, the console through the ESP32 bridge with host time stamps):**

1. RAM-only trials over DFU, nothing written. `tools/am62p-mkboot.py build`'s
   plain image (the direct triangle at 0x82000000, no loader) booted at the
   first try: `sample:` 80 ms after the A53 SPL's "Authentication passed".
   The loader image was silent after that line: the loader was built on the
   a53 board with its default `CONFIG_MAX_XLAT_TABLES=8`, too few for its
   own 24 MiB, the 344 MiB payload region and the devices, and the arm64 MMU
   init fails before the console exists. `boards/verdin_am62p_am62p54_a53.conf`
   now sets 64; with it the loader boots, unpacks the triangle in 482 ms
   and jumps.
2. boot0 and boot1 read back equal to the backups (`verdin-am62p-boot0.bin`
   `4a8437c0...`, boot1 all zero); PARTITION_CONFIG was 0x48.
3. **boot1 written** from Torizon over ssh with
   `bootpart-triangle-4MiB.bin` (3,667,968 bytes, `3b11de84...`): `force_ro`
   cleared, `dd bs=1M conv=fsync`, read back equal, `mmc bootpart enable 2 1
   /dev/mmcblk0`; PARTITION_CONFIG now **0x50** (boot1, boot acknowledge).
   boot0 and the user area untouched.
4. Power-cycle: **the triangle at power-on**, Linux and U-Boot proper never
   run. The owner saw it on the panel; the camera and the UART agree
   (`inside wrong 0`). From the R5 SPL's first UART line:

   | stage | time |
   |---|---|
   | R5 SPL (DDR training) -> "Starting ATF" | +0.36 s |
   | A53 SPL starts | +0.56 s |
   | the loader (Zephyr) boots | +0.71 s |
   | payload inflated (482 ms), jump to 0x82000000 | +1.23 s |
   | the image's first line | +1.30 s |
   | DSS ready / DSI link / bridge up | +1.34 / +1.57 / +1.64 s |
   | **first frame** | **+1.95 s** (system counter 1.637 s) |

   The system counter (GTC, started by the R5 SPL) says 1.370 s at the
   image's application init and 1.637 s at the first frame. The owner's
   target is 100 ms; the pieces: the inflate (482 ms for 1.8 MB, far slower
   than a cached A53 should manage: check the staging/payload mapping
   attributes and the table build), the display bring-up (0.33 s, DSI and
   the bridge's power sequence), the GPU bring-up (0.2 s, firmware load and
   the 100 ms clock measurement), and TI's two SPL stages (0.56 s, outside
   this repository).
5. The triangle image holds 40 s and then resets itself
   (`CONFIG_YAKOGL_SAMPLE_HOLD_S`, `YAKOGL_SAMPLE_REBOOT`), so the board now boots
   in a loop; a demo image sets a longer hold or no reboot.

6. **The eMMC source, 23:33 (the Qt image, 66 MB):** the payload does not fit
   the boot partition or the 4 MiB before Torizon's partition, and Torizon's
   one ext4 partition spans the device. The slot is therefore a file inside
   that filesystem: `fallocate -l 64M /var/lib/yakogl-stage2/slot.img`
   gave one extent (`filefrag -v`: blocks 2633148..2649531 of 4 KiB), so
   the header is at absolute sector 8192 + 2633148 * 8 = 21073376 and the
   loader is built with `CONFIG_BOOT_STAGE2_SECTOR=21073376
   CONFIG_BOOT_STAGE2_END_SECTOR=21204448` (`am62p-mkboot.py stage2img
   --sector --end-sector`). The image is written into the file with
   `dd conv=notrunc`, checked by hashing the raw sectors, and the file made
   immutable (`chattr +i`) so ostree cannot move or drop it. boot1 took
   `bootpart-emmc-4MiB.bin`. Power-on: loader at +0.15 s, eMMC ready at
   +1.22 s (1088 ms of SDHC/TISCI init), 49,961 sectors read in 698 ms,
   **inflate of 66 MB in 25.4 s**, the image at +27.3 s. The probe line
   `SCTLR_EL1 M=1 C=1 I=1; memcpy 1 MiB in 153684 us` says the caches are
   on but the payload region writes at 6.8 MB/s: the `STAGE2_PAYLOAD`
   region's write-back attribute is not what the mapping gets. That is the
   triangle's 482 ms too, and the first thing to fix.
   The Qt scene itself did not render on this build (the offline shader
   table lacks seven of today's Qt shaders; the online path stalls the GPU
   on the IBL prefilter): #1635.

7. **23:50: principledmaterial renders at power-on** (the offline-table Qt
   build with the ES 3.0 report; the ES 2.0 report of item 6 was the black
   View3D: Qt then looks for GLSL 100 shaders only and the prefilter has
   none). 0 table misses; loader +0.07 s, eMMC ready +1.17 s, read 50,007
   sectors in 701 ms, inflate 25.5 s, Qt EGL up +28.0 s.
   **The inflate speed is a mapping bug:** `STAGE2_PAYLOAD` carried
   `DT_MEM_ARM64(ATTR_ARM64_CACHE_WB)` without `DT_MEM_CACHEABLE`, which
   `DT_MEM_ATTR_TO_MT` maps MT_NORMAL_NC; inflate reads its output back for
   every match, so 6.8 MB/s. Both overlays now say `DT_MEM_ARM64_MMU_NORMAL`.
   For tonight the slot also takes the Qt image uncompressed
   (`stage2img --compress none`, 129,282 of the slot's 131,072 sectors):
   no inflate, about 1.8 s of eMMC read. The GPU clock measurement in
   `driver/src/pvr_bringup.c` is 10 ms instead of 100.

8. **2026-10-22 01:03, the uncompressed payload and the cache-fixed loader:**
   `memcpy 1 MiB in 4449 us` (236 MB/s; 155 ms before: the mapping fix
   holds), eMMC ready +1.23 s, the 66 MB read in about 1.37 s (48 MB/s),
   then `refused: CRC-32 of the stored payload mismatch` at +3.09 s. The
   probe itself was the corruption: it copied 1 MiB to LOAD + 16 MiB, and
   with an uncompressed payload the eMMC source reads the payload to LOAD
   first. The probe now runs before the read and targets LOAD; the fixed
   loader is `bootpart-emmc-4MiB.bin` (34eea049...) for the next window.

9. **2026-10-22 01:19, the result of the night:** principledmaterial from
   power-on, the uncompressed 66 MB payload, the cache-fixed loader with the
   probe before the read. From the R5 SPL's first line: loader +0.13 s,
   eMMC ready +1.24 s (1088 ms init), `memcpy 1 MiB in 5310 us`, 129,273
   sectors read in 1394 ms (47 MB/s), "unpacked" (the CRC-32 pass over
   66 MB) 960 ms, **jump +3.63 s**, Qt's EGL up +4.01 s, fonts +4.93 s,
   the first GPU kicks of the scene about +12.2 s (no first-frame mark in
   the Qt image; the heartbeat at +14 s counts 60 kicks). 0 table misses.
   What is left, biggest first: Qt's own start (EGL to the first draws,
   about 8 s: QML load, the scene, the IBL bake at 16 samples), the eMMC
   init 1.09 s, the read 1.39 s, the CRC pass 0.96 s.

**Back to Torizon**: from Torizon (boot0 still holds it), `sudo mmc bootpart
enable 1 1 /dev/mmcblk0`; without Linux, the recovery U-Boot's
`mmc partconf 0 1 1 0` (see Recovery).

**Procedure, as it was planned:**

**First, without writing anything (the plan's step 1a, now with the board's
own files):** USB recovery mode, then `dfu-util` sends the board's tiboot3
and tispl (byte-identical to Torizon 7.6.1's, extracted from the boot0
backup of 2026-10-21) and our u-boot.img (loader + payload) from RAM:

```
dfu-util -R -a bootloader -D tiboot3-am62px-hs-fs-verdin.bin
dfu-util -R -a tispl.bin  -D tispl.bin
dfu-util -R -a u-boot.img -D build/am62p/direct/u-boot-direct-triangle.img
```

(each stage enumerates its own DFU device and asks for the next file; the
alt names are the SPLs' `dfu_alt_info`). The UART then shows the whole
chain, the loader's lines and the image's boot-time lines; nothing on the
eMMC changes and the next power-cycle boots Torizon.


`build/am62p/write-emmc.txt` has every command, in this order. In short:

1. **Without writing anything**: USB recovery mode (hold SW5 on the Dahlia,
   power on with SW3, keep SW5 for 10 s; USB-C X3 to the host; Dahlia
   datasheet <https://docs.toradex.com/107809-dahlia-carrier-board-datasheet.pdf>).
   (a) `dfu-util` sends Toradex Easy Installer 7.7.0's tiboot3/tispl and then
   **our** `u-boot-hello.img` as `u-boot.img`: the direct chain itself
   (A53 SPL -> Zephyr), from RAM. Only an image under 4 MiB fits that slot.
   (b) Or the recovery U-Boot (`bootcmd=fastboot usb 0`) takes
   `triangle_direct/zephyr/zephyr.bin` over fastboot (uuu) and `go`es to
   it: U-Boot proper runs here, but no Linux and no eMMC write, so the cold
   display path and the RTI self-arm are tested on the direct image.
2. **Read-only**: boot partition sizes, EXT_CSD PARTITION_CONFIG, that boot0
   holds exactly Torizon 7.6.1's three files, what boot1 holds.
3. **Back up** boot0 and boot1 to the laptop.
4. **Write boot1 only** with a whole boot-partition image, verify it, enable
   boot1. boot0 (Torizon's bootloader and U-Boot's environment) and the user
   area are not touched.
5. Power-cycle and log the UART with host time stamps.

With the second stage (a 4 MiB boot partition, the GL images), steps 1-4
become:

1. **No write**: `dfu-util` sends `u-boot-stage2.img` as `u-boot.img`. With
   nothing at user-area sector 64 yet the loader must initialise the eMMC,
   read sector 64 and stop with "header at sector 64 refused: no YKSTAGE2
   header": the SPL -> loader hand-off, TISCI, the SDHC driver and a read,
   proven without a write.
2. Read-only checks as before, plus the partition table and that sectors
   64..8191 are still zero.
3. Back up boot0, boot1 **and user-area sectors 64..8191**.
4. **User area** (Torizon running, reversible from the backup):
   `dd if=stage2-slint.img of=/dev/mmcblk0 bs=512 seek=64 conv=fsync`,
   compare. Then the DFU boot of step 1 again: now the loader finds the
   image and starts slint, still with no boot-partition write.
5. **boot1** with `bootpart-stage2-4MiB.bin`, enable boot1, power-cycle.

## Recovery

- **Back to Torizon**: USB recovery mode, the recovery U-Boot, then
  `mmc partconf 0 <ack> 1 0` (boot0) through uuu
  (`build/am62p/uuu/back-to-boot0.lst`). Everything Torizon needs is still in
  boot0 and the user area.
- **If that fails**: Toradex Easy Installer from the same recovery mode
  (`recovery-linux.sh` in `Verdin-AM62P_ToradexEasyInstaller_7.7.0+build.13.zip`,
  sha256 `928c90e873e5f3655a2073b4ad6a8ac7b3fa71494656665376572ba1cdc4db3a`,
  <https://artifacts.toradex.com/artifactory/tezi-oe-prod-frankfurt/scarthgap-7.x.y/release/13/verdin-am62p/tezi/tezi-run/oedeploy/>)
  reinstalls Torizon (boot0 and the user area: the board's data is lost).
  Toradex: <https://developer.toradex.com/easy-installer/toradex-easy-installer/loading-toradex-easy-installer/>.
- **Restore a backup** from Linux: `dd` the saved file back to
  `mmcblk0boot0`/`mmcblk0boot1` after `echo 0 > /sys/block/mmcblk0bootN/force_ro`.
- An SD-card route like the X2's was not looked for: whether this module can
  be strapped to boot from the Dahlia's SD slot was not checked. USB recovery
  needs no card and no change on the module.

## Risks and open questions

- **Not run.** Everything about runtime behaviour comes from source and bytes.
- **Boot partition size: 4 MiB** (measured). The GL images go through the
  second stage; see there. Its own risks:
  - The TI SDHCI driver and Zephyr's MMC stack have not run on this SoC.
    The AM62P's MMC0 PHY values come from Linux's devicetree; the driver's
    DLL path is avoided (HS, 52 MHz), but its PHY calibration at init
    (`ti,dll-present`) does run. A failure stops in the loader with the
    reason on the UART (nothing is written, Torizon's boot0 is untouched).
  - On a DFU boot MMC0 is cold: TISCI switches it on with its clocks at
    their defaults, and the driver takes the base clock from the
    controller's capabilities register. If those disagree the card runs at
    the wrong rate; the eMMC boot (boot1) does not have this question.
  - The image starts at EL1 instead of EL2 (above). Unrun.
  - Cache hand-off: the whole D-cache is cleaned and invalidated by set/way
    with the cache still on, then turned off, as U-Boot does before its own
    jumps; a speculative refill in between is architecturally possible.
  - The user-area sectors 64..8191 are outside every partition, but nothing
    reserves them: a Toradex Easy Installer run (repartitioning) or any tool
    that writes the gap would destroy the image (the loader then refuses it
    by its CRC). Torizon's OTA updates write the filesystem and boot0 only,
    as far as known.
- **HS-FS assumed.** Torizon's Tezi image installs the hs-fs tiboot3 for this
  product (0099), and HS-FS loads an uncertified payload with a warning. An
  HS-SE device would stop at "Authentication failed!". Step 2 reads the
  device type from the boot log.
- **The RTI self-arm has not run on silicon** (the netboot plan's step B is
  still open). Until it has, a hung direct image holds the board until the
  power is cut; recovery mode still works.
- **Reset loop.** With `YAKOGL_SAMPLE_REBOOT=y` the image resets the SoC after
  its hold, and the next boot is the same image: one boot-time sample per
  reset, but the board comes back to Torizon only through recovery mode.
- **TI tree against mainline.** The SPL facts are from mainline; TI's tree
  could differ (for example a fixed `CONFIG_SYS_MMCSD_RAW_MODE_EMMC_BOOT_PARTITION`).
  If its A53 SPL always read boot0, the boot1 test would boot Torizon's U-Boot
  instead: a safe failure, visible in the log.
- **Torizon's use of boot1** is unknown; step 2 reads it and step 3 backs it
  up before it is overwritten.
- **Boot time**: no number yet. The GTC does not cover ROM + R5 SPL + DDR
  training; the host clock from the first UART byte does.
