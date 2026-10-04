<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Direct boot on the Armadillo-X2: Zephyr as TF-A's BL33

Goal (2026-10-02): no Linux and no kexec. The Armadillo-X2 powers on into a
Zephyr image, and the first picture appears soon after power-on. The user's
realistic target is about 0.5 s to the first frame. That is a target to
measure against, not a promise.

Status (2026-10-01): the image is built and checked offline, but it **has not
run on the board**. The board could not be power-cycled that day, and its
eMMC was not read (see "Open questions"). The first image is
`gc_gl_triangle`, which renders off-screen and reports on UART2. There is no
picture yet: nothing in the chain brings up LCDIF3 -> HDMI from cold, and the
adopt display path needs a Linux that has already lit the display.

## The chain

```
Boot ROM ── reads the imx-boot image (IVT at image offset 0) from the boot device
  └─ U-Boot SPL 2020.04-at31 (OCRAM 0x920000): DDR training, PMIC, UART2 banner
       └─ loads the FIT at image offset + 0x58000 through the ROM API:
            uboot-1 = zephyr.bin   -> 0x40200000   (no entry property)
            fdt-1   = Atmark's DTB -> appended after uboot-1 (unused by Zephyr)
            atf-1   = TF-A BL31    -> 0x970000, entry
            tee-1   = OP-TEE       -> 0x56000000, entry
       └─ jumps to the first loadable that has an entry point: BL31
            └─ BL31 v2.4 (imx-atf, SPD=opteed) runs OP-TEE's init, which returns
            └─ enters BL33 = 0x40200000 in non-secure EL2, MMU/caches off, DAIF masked
                 └─ Zephyr: __start (image header word 0 is "b __start"),
                    EL2 -> EL1, gc_gl_triangle
```

U-Boot proper is gone. Its slot in the FIT holds Zephyr. Every other part of
Atmark's image is copied unchanged.

## What the prebuilt image is

The image is Atmark's prebuilt `imx-boot_armadillo_x2_2020.04-at31`, the
current "latest", from <https://download.atmark-techno.com/armadillo-x2/bootloader/>:

| file | sha256 |
|---|---|
| `imx-boot_armadillo_x2_2020.04-at31` (1,839,472 bytes) | `c1f164e95eba0ebc29d9b75e4552218d06528f86395c554dd526bd3b820e557e` (matches Atmark's `.sha256`) |
| `imx-boot_armadillo_x2_2020.04-at30` (3,466,992 bytes), what ABOS 3.24 runs | `65c677402c8f8a167527a183952804ff2924e5d1328d47deb21053bf379034aa` |

at31 rather than at30, for one reason. at30 carries OP-TEE as a 2.1 MB padded
binary, so at30 plus a 2.4 MB Zephyr is about 5 MB. That does not fit the eMMC
boot partition, which holds U-Boot's environment at 0x3FA000 and 0x3FE000. at31
switched to `tee-raw.bin`; its Changelog says "imx-boot size reduced by 1.5MB",
and its OP-TEE is 0.48 MB. BL31 is byte-identical in both. at31's other change
is to U-Boot proper ("x2 chosen/fallback-boot support"), which this chain does
not run. The SPL banner therefore says at31, while the board's current U-Boot is
at30.

Layout of the at31 image (`tools/x2-mkflash.py info`):

```
IVT @0x0: entry 0x920000 self 0x91ffc0 boot_data 0x91ffe0 csf 0x94a5c0
boot data: start 0x91ffc0 length 0x2c660 (what the ROM loads: SPL + DDR firmware)
FIT @0x58000: header 1031 bytes, external data (data-position)
  uboot-1 "U-Boot (64-bit)"      standalone pos 0x005000 size 0x0d8790 load 0x40200000 entry -
  fdt-1   "evk"                  flat_dt    pos 0x0dd790 size 0x0079f0
  atf-1   "ARM Trusted Firmware" firmware   pos 0x0e5180 size 0x00b150 load 0x970000   entry 0x970000
  tee-1   "TEE firmware"         firmware   pos 0x0f02d0 size 0x078ea0 load 0x56000000 entry 0x56000000
config-1 (default): firmware=uboot-1, loadables=atf-1, tee-1, fdt=fdt-1
```

The FIT also has an i.MX IVT at FIT + 0x1000 for HAB. SPL checks it only when
HAB is closed (`board_spl_fit_post_load`). With HAB open, a FIT without an IVT
passes silently. The new FIT has none.

### Facts from Atmark's source, imx-boot-2020.04-at30.tar.gz

The source tarball was read, but nothing was built from it. Building would
need the NXP firmware EULA.

- TF-A `plat/imx/imx8m/imx8mp`: `PLAT_NS_IMAGE_OFFSET 0x40200000` is the BL33
  entry, and `get_spsr_for_bl33_entry()` returns EL2 with SP_ELx and all
  exceptions masked. `BL32_BASE 0x56000000` and `BL32_SIZE 0x2000000` are the
  OP-TEE region; they are passed to BL33 in x1/x2, and x3 = `BL32_FDT_OVERLAY_ADDR`
  = 0x43200000. Zephyr reads none of x0-x3. `IMX_BOOT_UART_BASE 0x30890000` is
  UART2, so BL31 prints there. The build uses `SPD=opteed`, so BL31 always runs
  BL32 at 0x56000000 first, and **OP-TEE has to stay in the FIT**: without it,
  BL31 would jump into empty DDR. BL31 also writes 0 to 0x43200000 before BL33
  runs, and `bl31_platform_setup` releases the Cortex-M7 from reset.
- U-Boot `spl_imx_romapi.c`: the FIT is read from `image_offset +
  CONFIG_SYS_MMCSD_RAW_MODE_U_BOOT_SECTOR * 512 - 0x8000` = image offset +
  0x58000. `x2_defconfig` has `CONFIG_SYS_TEXT_BASE=0x40200000`,
  `CONFIG_IMX_HAB=y`, `CONFIG_ENV_OFFSET=0x3FE000`, and
  `CONFIG_ENV_OFFSET_REDUND=0x3FA000`.
- `spl_load_simple_fit`: an image with no `entry` hands the jump to the first
  loadable that has one. The "firmware" gets the FIT's `fdt` appended at
  `load + size`, and SPL fails if there is none. So `fdt-1` stays, and the
  Zephyr payload is padded to 16 bytes.
- Where the image lives on the eMMC: Atmark's installer (`flash_image.sh`)
  writes the bootloader to the boot partition and selects it with
  `mmc partconf ... 1 0`. ABOS updates (mkswu `pre_boot.sh` / `post_boot.sh`)
  keep two copies, in `mmcblk2boot0` and `mmcblk2boot1`, write the inactive one
  at offset 0, and flip `mmc bootpart enable N 0`. On an SD card the image
  sits at 32 KiB. The "image offset 0x8000" in Atmark's SPL log is that
  SD/user-area case.

## The Zephyr side: `armadillo_x2/mimx8ml8/a53/direct`

The board variant lives in `driver/boards/atmark/armadillo_x2`
(`armadillo_x2_mimx8ml8_a53_direct.{dts,yaml,_defconfig}`). The kexec variant
is unchanged.

- **Link address** 0x40200000 (`zephyr,sram`, 256 MiB), so
  `CONFIG_SRAM_BASE_ADDRESS` and `CONFIG_KERNEL_VM_BASE` are 0x40200000. The
  crash-window address 0x60000000 is not used.
- **DRAM map** (2 GiB at 0x40000000):

  | range | owner |
  |---|---|
  | 0x40000000 - 0x401fffff | below the entry; SPL's saved DDR timing for TF-A (`SAVED_DRAM_TIMING_BASE`) |
  | 0x40200000 + 256 MiB | Zephyr. zephyr.bin is 2.4 MB; `_end` is 0x42779000 |
  | 0x43200000 | TF-A/OP-TEE write here **before** BL33 runs, so zephyr.bin must stay under 48 MiB |
  | 0x56000000 + 32 MiB | OP-TEE, secure, never mapped |
  | 0x80000000 + 64 MiB | GPU carveout (the same as the kexec variant) |

- **EL2 entry**: Zephyr's `reset.S` handles EL3, EL2 and EL1 entry.
  `z_arm64_el2_init` drops to EL1. The SoC's `ARM64_BOOT_DISABLE_DCACHE`
  (default y for this SoC, "when booting from EL2") covers the cache state.
  `CONFIG_ARMV8_A_NS=y` matches the non-secure entry.
- **GIC**: TF-A initialised GICv3 for the secure side, and
  `GIC_SAFE_CONFIG=n` re-initialises the non-secure distributor, as on the
  kexec route. The secondary cores start through PSCI CPU_ON, which TF-A
  serves.
- **Timer**: SPL starts the system counter and programs CNTFRQ to 8 MHz, which
  is the SoC's `SYS_CLOCK_HW_CYCLES_PER_SEC` default.
- **No display node**: nothing has powered MEDIAMIX or clocked LCDIF3, and the
  adopt driver would read a dead block. The console is UART2.
- **The GPU** is powered by the driver itself, through the GPC and CCM
  (`gpu::vivante::power` in driver/rust); its clock parent is SYS_PLL1, which
  SPL enables.

`gc_gl_triangle` has `boards/armadillo_x2_mimx8ml8_a53_direct.conf` (UART
console, no panel console, no display driver). It also prints the system
counter at `main()` and at the verdict. On this route nothing restarts that
counter after SPL starts it, so the value is close to the time since power-on,
minus the Boot ROM.

Build it (`-DZEPHYR_MODULES` as in `driver/samples/zephyr/README.md`; a
nightly cargo for the Rust parts):

```sh
RUSTUP_TOOLCHAIN=nightly west build -p always -b armadillo_x2/mimx8ml8/a53/direct \
    -d build/x2/gc_gl_triangle_direct yakogl/driver/samples/zephyr/gc_gl_triangle
```

The two warnings this build prints are the kexec variant's too:
`GC_COMMON_PANEL_CONSOLE_SCALE` is assigned while the panel console is off, and
"No SOURCES given to Zephyr library: drivers__gpio".

## Assembling the image

```sh
tools/x2-mkflash.py build --base imx-boot_armadillo_x2_2020.04-at31 \
    --bl33 build/x2/gc_gl_triangle_direct/zephyr/zephyr.bin -o build/x2/flash-zephyr.bin
```

The tool uses the Python standard library only; no `mkimage` or `dtc` is
needed. It copies the first 0x58000 bytes (IVT, SPL, DDR firmware) unchanged.
It writes a new FIT with the same nodes and properties: the firmware's
description becomes "Zephyr (BL33)", the data positions start at 0x3000 and
are 4 KiB-aligned, and `uboot-1` carries zephyr.bin. It then reads the result
back and checks four things: the payloads, the load addresses, that the SPL
region is identical, and that the image stays under 0x3FA000, the environment.

The FIT writer was checked by rebuilding at31 with its own U-Boot and
decompiling both FITs with `dtc`. The only differences are the data positions
and the description. The rebuilt image is kept as
`build/x2/flash-uboot-repacked.bin`, a control: if it boots ABOS, the packing
is right.

The result on 2026-10-01 (from `a53/direct` at this commit):

```
flash-zephyr.bin: 3415712 bytes (0x341ea0)
  uboot-1 "Zephyr (BL33)" pos 0x003000 size 0x259990 load 0x40200000 entry -
  fdt-1   pos 0x25d000 size 0x0079f0
  atf-1   pos 0x265000 size 0x00b150 load 0x970000   entry 0x970000
  tee-1   pos 0x271000 size 0x078ea0 load 0x56000000 entry 0x56000000
uboot-1 word 0: 0x14012be3 = b 0x4024af8c = __start; "ARM\x64" at 0x38
```

## Tomorrow at the board

### 1. Read-only checks (root shell on ABOS)

```sh
cat /sys/block/mmcblk2boot0/size /sys/block/mmcblk2boot1/size   # 512-byte sectors; 8192 = 4 MiB expected
mmc extcsd read /dev/mmcblk2 | grep -i -A2 'PARTITION_CONFIG'    # "Boot partition N enabled" = active copy
dd if=/dev/mmcblk2boot0 bs=4 count=1 2>/dev/null | xxd             # d1002041 = an IVT at offset 0
dd if=/dev/mmcblk2boot1 bs=4 count=1 2>/dev/null | xxd
hexdump -C -s 0x1c -n 4 /sys/bus/nvmem/devices/imx-ocotp0/nvmem  # bank 1 word 3: bit 25 (0x02000000) set = HAB CLOSED
```

**Stop if HAB is closed.** SPL would then refuse the unsigned FIT. Otherwise,
call the active boot partition A (N = 1 means boot0, 2 means boot1) and the
other one B.

### 2. Back up, write the inactive copy, flip

```sh
# laptop (BOARD = how ssh reaches it: armadillo.local, or -P 2222 root@localhost via the tunnel)
scp build/x2/flash-zephyr.bin root@BOARD:/tmp/
# board
dd if=/dev/mmcblk2boot0 of=/tmp/mmcblk2boot0.bak bs=1M
dd if=/dev/mmcblk2boot1 of=/tmp/mmcblk2boot1.bak bs=1M
# laptop: keep the backups (/tmp on the board is lost at power-off)
scp 'root@BOARD:/tmp/mmcblk2boot*.bak' build/x2/
# board, B = the inactive partition, e.g. boot1 when boot0 is active
echo 0 > /sys/block/mmcblk2boot1/force_ro
dd if=/tmp/flash-zephyr.bin of=/dev/mmcblk2boot1 bs=1M conv=fsync
echo 1 > /sys/block/mmcblk2boot1/force_ro
cmp -n 3415712 /tmp/flash-zephyr.bin /dev/mmcblk2boot1 && echo written
mmc bootpart enable 2 0 /dev/mmcblk2        # 2 = boot1; use 1 for boot0
poweroff                                     # then remove and restore power
```

The write ends at 3.4 MB, so U-Boot's environment at 0x3FA000 and 0x3FE000 is
untouched, and partition A still holds the untouched at30 image.

### 3. What to look for on `/dev/ttyUSB0` (UART2, 115200 8N1)

The following prints each line with the seconds since the first byte, which is
SPL's banner, so it does not include the Boot ROM's time:

```sh
stty -F /dev/ttyUSB0 115200 raw -echo
python3 -u -c '
import os, time
fd = os.open("/dev/ttyUSB0", os.O_RDONLY); t0 = None; buf = b""
while True:
    b = os.read(fd, 4096); now = time.monotonic(); t0 = t0 or now; buf += b
    *lines, buf = buf.split(b"\n")
    for l in lines: print("%8.3f %s" % (now - t0, l.decode(errors="replace").rstrip()))
' | tee build/x2/boot-$(date +%H%M%S).log
```

The expected sequence:

1. `U-Boot SPL 2020.04-at31 ...`, DDR, then `Trying to boot from BOOTROM` and
   `image offset 0x0, pagesize 0x200, ivt offset 0x0` (boot partition; 0x8000 on SD).
2. `NOTICE:  BL31: v2.4(release):2020.04-at28`, then OP-TEE's `I/TC:` lines.
3. `*** Booting Zephyr OS build ... ***`, then `gc_gl_triangle: 1280x800 window ...`,
   `system counter at main(): N us`, and the GPU lines, then
   `system counter at the verdict: N us` and `PASS` or `FAIL`.

After `GC_SAMPLE_HOLD_SECONDS` (60 s), the sample resets the SoC through PSCI.
On this route that boots the same image again, so each reset gives another
boot-time sample. If it stops after step 2, write
`build/x2/flash-uboot-repacked.bin` the same way. If ABOS then boots, the
packing is right and the fault is in Zephyr's entry.

The first measurement to record has three parts: the host time from the SPL
banner to the Zephyr banner, the system counter at `main()`, and the counter
at the verdict.

## Recovery

- **Back to ABOS, eMMC intact.** Close JP1, put an SD card with Atmark's
  image at 32 KiB in CON1, power on, and stop U-Boot on UART2. Then
  `mmc partconf 2 0 1 0` (boot0; `2 0 2 0` for boot1), open JP1, and `reset`.
  If U-Boot cannot be stopped, it boots ABOS from the eMMC; then
  `mmc bootpart enable 1 0 /dev/mmcblk2` from Linux does the same thing.
  Prepare the card on the laptop (the device name is an example; check it
  with `lsblk` first):

  ```sh
  sudo dd if=build/x2/imx-boot_armadillo_x2_2020.04-at30 of=/dev/sdb bs=1k seek=32 conv=fsync
  ```

- **Zero-risk first try.** The same card trick runs the direct image without
  touching the eMMC:
  `sudo dd if=build/x2/flash-zephyr.bin of=/dev/sdb bs=1k seek=32 conv=fsync`,
  close JP1, and power on. The write starts at 32 KiB, so the card's partition
  table (MBR or GPT) survives.
- **Restore a backup:** `dd if=mmcblk2bootN.bak of=/dev/mmcblk2bootN bs=1M conv=fsync`
  after `echo 0 > /sys/block/mmcblk2bootN/force_ro`.

## Risks and open questions

- **Not run.** Every statement above about runtime behaviour comes from
  Atmark's source and the prebuilt image's bytes, not from the board.
- **The board's real boot image was not read.** On 2026-10-01 reading
  `mmcblk2boot*` and `/proc/iomem` over ssh was denied by the session's
  permission check. The at30 in the table is what the board's U-Boot version
  string says, not a byte comparison. Step 1 confirms where the image is.
- **The HAB state is assumed open.** It is checked in step 1.
- **OP-TEE** stays, because BL31 is built with `SPD=opteed`. It costs boot
  time; measure it from the gap between the BL31 and Zephyr lines. Dropping it
  needs a BL31 without opteed, which means building TF-A ourselves.
- **0x43200000 / zephyr.bin size**: TF-A and OP-TEE write there before Zephyr
  runs. A zephyr.bin over 48 MiB would be corrupted, so the limit is checked
  by eye for now (gc_slint is larger than gc_gl_triangle).
- **Boot partition size**: assumed 4 MiB, because of the environment at
  0x3FE000. The tool refuses images over 0x3FA000; for an SD card use
  `--limit 0`.
- **Cortex-M7**: BL31 releases it from reset with whatever is in its TCM.
  ABOS does the same today.
- **The display**: the cold LCDIF3 -> PVI -> DW-HDMI bring-up is separate
  work. Until it exists, this route shows nothing on HDMI.

# Direct boot on the Verdin AM62P: Zephyr in place of U-Boot proper

Goal (2026-10-02): the same power-on -> Zephyr -> cluster form as on the
i.MX8MP, for the Toradex Verdin AM62P on the Dahlia carrier, which boots
Torizon from its eMMC today.

Status (2026-10-02): images built and checked offline, **nothing has run on
the board**, and the board was not touched (it was busy with a dEQP run; one
read-only ssh attempt timed out). Unlike the X2, a picture is possible on the
first direct boot: the cold display path (DSS1 -> DSI -> SN65DSI83 -> LVDS
panel) is in the C drivers and first ran on this board under a U-Boot `go`
entry. **Size**: `gl_triangle` is 15.7 MB and `gl_slint` 18.9 MB (most of
it the offline shader table, about 12 MB), and this module's boot partitions
are 4 MiB each (`/sys/block/mmcblk0boot*/size` = 8192 sectors, read
2026-10-02), so the u-boot.img slot holds 1.49 MiB. The GL images therefore
go in two parts: a second-stage loader (95.9 KB) takes the slot and reads
the image, deflate-compressed, from a raw sector range of the eMMC user area
("The second stage" below). Built and checked offline; not run.

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
(driver/docs/am62p-netboot-plan.md), the same string as in this u-boot.img.
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
  path drops to EL1, as under U-Boot `go` (where SafeUI turned the caches off
  by hand first).
- **Display on in the board DTS**: DSS1, DSI, the SN65DSI83 on I2C1, the GPIO
  banks and the TISCI power-on list, which the a53 variant leaves to each
  sample's overlay. Nobody else owns the panel on this route.
- **No inherited watchdog.** The direct confs set
  `CONFIG_YAKOGL_AM62P_RTI_SELF_ARM=y` (the netboot plan's Z1) with the
  sample's bound (150 s gl_triangle, 200 s gl_slint).
- **Boot time**: `CONFIG_PVR_SAMPLE_BOOT_TIME=y` prints
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
| I2C1, GPIO0, GPIO1 on | Zephyr `safeui,tisci-power-on` | the same, now in the board DTS |
| panel and I2C pads | Zephyr pinctrl | the same |
| RTI watchdog | nobody (no driver in U-Boot) | Zephyr arms it (`rti_arm.c`) |
| DDR size | U-Boot detects it for Linux | Zephyr's DT is static (2 GiB module) |
| caches off before the jump | `dcache off; icache off; go` by hand | the SPL's `dcache_disable()` |

So the direct image starts from the state the SafeUI U-Boot `go` entry had,
minus U-Boot proper's Ethernet/MMC/USB probing. That this is the whole
difference is unmeasured.

## Building

The board files and samples are in this repository. Build from the workspace
that holds Zephyr (af17c0c8 was used), with the module list given explicitly
when that workspace also carries a SafeUI checkout
(`tools/cts/zephyr-modules.sh`), and with the PowerVR firmware header in
`driver/src/fw/` (never committed):

```sh
west build -p always -b verdin_am62p/am62p54/a53/direct -d build/am62p/gl_triangle_direct \
    yakogl/driver/samples/zephyr/gl_triangle
RUSTUP_TOOLCHAIN=nightly west build -p always -b verdin_am62p/am62p54/a53/direct \
    -d build/am62p/slint_direct yakogl/driver/samples/zephyr/slint \
    -- -DSLINT_NOSTD_DIR=<no_std Slint workspace>/slint   # boards/verdin_am62p_am62p54_a53_direct.conf
# (until Step 1 of driver/docs/one-method.md the route's Slint image was gl_slint's; the sizes and
# results below were measured with it)
tools/am62p-mkboot.py build --base u-boot.img --zephyr build/am62p/gl_triangle_direct/zephyr/zephyr.bin \
    -o build/am62p/u-boot-gl_triangle.img
tools/am62p-mkboot.py bootpart --tiboot3 tiboot3-am62px-hs-fs-verdin.bin --tispl tispl.bin \
    --uboot build/am62p/u-boot-gl_triangle.img --size <boot partition bytes> -o bootpart.bin
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

| image | zephyr.bin | u-boot.img | needs a boot partition of |
|---|---|---|---|
| gl_triangle | 15,730,672 B, `47dcb4d9...` | 15,820,356 B, `e0570e58...` | 18,450,500 B (0x1198844) |
| gl_slint | 18,929,864 B, `b418d40d...` | 19,019,332 B, `bb2867ba...` | 21,649,476 B (0x14a5844) |
| hello_world (GPU and display off) | 69,992 B, `57934948...` | 161,348 B, `568081bd...` | 2,791,492 B (fits 4 MiB) |
| control (U-Boot proper repacked) | - | 1,054,276 B, `f05edda0...` | 3,684,420 B |

gl_triangle's .rodata is 15.3 MB; the offline shader blobs are about 12 MB
of it (`SHC_BLOB_TABLE` and the blobs it points to), the GPU firmware 229 KB.

## The second stage

The u-boot.img slot of a 4 MiB boot partition runs from 0x280000 to U-Boot's
environment 0x2200 bytes before the end: 1,564,160 bytes, less the FIT
header and fdt-0 (85 KB with its certificate). What was measured and weighed
(2026-10-02, Zephyr af17c0c8):

| option | finding | verdict |
|---|---|---|
| (a) in-process compiler, no table (`CONFIG_YAKOGL_SHADERC_ONLINE=y`) | gl_triangle 1,542,472 B (gl_triangle calls `gles_offline_shader_source`, so this is a size-only link with that symbol stubbed), gl_slint 4,774,432 B | neither fits the slot with fdt-0 (gl_triangle is 64 KB over, gl_slint 3.3 MB). Rejected. |
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
either, which is why the payload is raw deflate: gl_triangle 2,001,795 B,
gl_slint 2,944,051 B (Python zlib level 9). A dedicated partition was
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
| stage2-gl_triangle.img | 2,002,432 (3,911 sectors) | user area, sectors 64..3974 |
| stage2-gl_slint.img | 2,945,024 (5,752 sectors) | user area, sectors 64..5815 |

## At the board

`build/am62p/write-emmc.txt` has every command, in this order. In short:

1. **Without writing anything**: USB recovery mode (hold SW5 on the Dahlia,
   power on with SW3, keep SW5 for 10 s; USB-C X3 to the host; Dahlia
   datasheet <https://docs.toradex.com/107809-dahlia-carrier-board-datasheet.pdf>).
   (a) `dfu-util` sends Toradex Easy Installer 7.7.0's tiboot3/tispl and then
   **our** `u-boot-hello.img` as `u-boot.img`: the direct chain itself
   (A53 SPL -> Zephyr), from RAM. Only an image under 4 MiB fits that slot.
   (b) Or the recovery U-Boot (`bootcmd=fastboot usb 0`) takes
   `gl_triangle_direct/zephyr/zephyr.bin` over fastboot (uuu) and `go`es to
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
   image and starts the Slint sample, still with no boot-partition write.
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
- **Reset loop.** With `PVR_SAMPLE_REBOOT=y` the image resets the SoC after
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
