<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Direct boot on the Verdin iMX95: our image in place of U-Boot proper

Goal: the Verdin iMX95 on the Mallow powers on into our Zephyr image from
the eMMC, with no U-Boot proper and no Linux, and a lab board can always be
brought back to Toradex's boot. This is item H5 and board step B7 of
`docs/notes/pre-study-imx95.md`. The model is `docs/standalone-boot.md` (per
board: what replaces what, the fallback, the proof) and the i.MX8MP's direct
route (`docs/notes/imx8mp-direct-boot.md`), which ran on 2026-10-21.

Status (2026-11-21): designed from public sources only. **Nothing has run
on a board.** `tools/imx95-mkboot.py` builds the boot image and checks it
field by field. It was run on Toradex's published imx-boot for this module,
where it found every container and every hash as the SPL and mkimage
define them, and it rebuilt container 2 around a Zephyr image
(section 7). `tools/imx95_mkboot_selftest.py` tests it on synthetic images.

Prefixes used below. Line numbers are exact at the named version.

| Prefix | What, at what version |
| --- | --- |
| `MK:` | NXP `imx-mkimage`, branch `lf-6.6.52_2.2.1` at `81fca643`: the revision meta-freescale pins (`MF: recipes-bsp/imx-mkimage/imx-mkimage_git.inc:8-9`). The pre-study read `lf-6.18.20_2.0.0`; the i.MX 95 targets are the same |
| `TU:` | Toradex U-Boot `toradex_imx_lf_v2024.04` at `b7c747d8`, the recipe's `SRCREV` and the version string of the SPL in the published image below |
| `TA:` | NXP `imx-atf` `lf_v2.10_6.6.52_2.2.x` at `7e374c5f` (`MF: recipes-bsp/imx-atf/imx-atf_2.10.bb:17-18`); the published BL31 says `v2.10.0 (release):lf-6.6.52-2.2.1-dirty` |
| `TL:` | Toradex `meta-toradex-nxp` `scarthgap-7.x.y` at `b0b9f0fd` |
| `MF:` | `meta-freescale` `scarthgap` at `44fb1b89` |
| `SMT:` | Toradex `imx-sm-toradex` `main` at `3aa8a77a` |
| `UUU:` | NXP `mfgtools` (uuu, BSD-3-Clause) `master` at `450ef665` |
| `TZ:` | Toradex Easy Installer 7.8.0+build.14 for `verdin-imx95` (2026-10-01), public at `artifacts.toradex.com` under `tezi-oe-prod-frankfurt/scarthgap-7.x.y/release/14/verdin-imx95/tezi/tezi-run/`: `teziimage/imx-boot-sd` (sha256 `675a8572...`, 2449408 bytes), `teziimage/uuu.auto`, `teziimage/image.json`, `oedeploy/bl31-imx95.bin-optee` (`65c2fb30...`). Read only, never run, not in the tree |

Not read: NXP's i.MX 95 reference manual (login), so the Boot ROM's own
behaviour is known only through what the SM, U-Boot and mkimage do with it;
Toradex's OEI source; Toradex's Mallow documentation.

## 1. The chain

```
Boot ROM (Cortex-M33) + ELE ── eMMC boot operation on the enabled boot partition
  │                             (Toradex writes imx-boot to mmcblk0boot0 at 0: TZ image.json)
  ├─ 0x000000 container 0  ELE firmware, NXP-signed               (ROM + ELE)
  ├─ 0x004000 V2X container  V2X firmware, NXP-signed              (ROM + ELE)
  └─ 0x008000 container 1  (ROM loads, ELE checks, M33 runs)
        DDR dummy, OEI (M33, DDR training with the LPDDR4X firmware) -> TCM 0x1FFC0000
        System Manager (Toradex's verdin-imx95)                      -> TCM 0x1FFC0000
        A55 U-Boot SPL                                               -> OCRAM 0x20480000
        V2X dummy (0x8B000000)
     SM: starts LM2 "AP" on A55 core 0 at SPL, then serves SCMI for the board's life
  SPL (EL3, secure): console, clocks over SCMI, reads container 2 from the same boot partition
  └─ 0x0c3400 container 2  (SPL loads: no hash check, no ELE)
        [0] TF-A BL31  -> 0x8A200000, entry: SPL jumps here
        [1] U-Boot     -> 0x90200000   <- BL33, the slot we replace
        [2] OP-TEE     -> 0x8C000000
     BL31: OP-TEE's init, then BL33 at 0x90200000, non-secure EL2
  └─ 0x256000 end of the image set (end of the file)
```

The offsets are those of Toradex's 7.8.0 image (`tools/imx95-mkboot.py info`
on `TZ: imx-boot-sd`); the layout rules behind them follow.

**How it is built.** Toradex's machine builds mkimage's `flash_a55` target
(`TL: conf/machine/verdin-imx95.conf:57`) with `OEI=YES` and the LPDDR4X
firmware (`MF: recipes-bsp/imx-mkimage/imx-boot_1.0.bb:81-85`), for SoC
revision B0 (`MF: conf/machine/include/imx-base.inc:188`), so with container
version 2 (`MK: iMX95/soc.mak:74-80`). `flash_a55` appends the ELE container,
builds container 1 from the OEI, the SM and SPL, then appends
`u-boot-atf-container.img` at the next 1 KiB (`MK: iMX95/soc.mak:180-187,
335-339`). That second file is BL31 at `ATF_LOAD_ADDR` 0x8A200000,
`u-boot-hash.bin` (U-Boot plus mkimage's commit id) at `UBOOT_LOAD_ADDR`
0x90200000 and, when `tee.bin` exists, OP-TEE at 0x8C000000
(`MK: iMX95/soc.mak:43-44, 62, 235-237, 250-274`). Toradex's machine has
`optee` (`TL: verdin-imx95.conf:19`), so OP-TEE is there and BL31 is the
`SPD=opteed` build (`MF: imx-boot_1.0.bb:45, 191-194`): the BL31 in
the published image is `oedeploy/bl31-imx95.bin-optee` byte for byte.

**The container format** (`MK: src/imx8qxb0.c:111-147`; U-Boot's reader
`TU: include/imx_container.h:20-57`). A header of 16 bytes (version, length,
tag 0x87, flags, software and fuse versions, image count, signature block
offset), 128 bytes per image (file offset relative to the header, size,
destination, entry, flags = type | core << 4 | hash type << 8, meta, a 64-byte
hash, a 32-byte IV), then a 16-byte signature block header (tag 0x90).
Version 2 headers are 0x4000-aligned (`MK: imx8qxb0.c:54-55`;
`TU: arch/arm/mach-imx/imx9/scmi/soc.c:1595-1598`), and a lone version-2
container's images start at 0xC000 (`MK: imx8qxb0.c:711`). Each image is
padded to a 0x400 sector and its size field is the padded size; its hash is
SHA-384 over the padded bytes (`MK: imx8qxb0.c:49, 234-296`, the `tmp_pad`
command at :273). An A55 image has core 2 and meta 2, "A55 core 0"
(`MK: imx8qxb0.c:522-546`).

**How SPL finds container 2** (`TU: arch/arm/mach-imx/image-container.c:240-343,
360-371`): the container at offset 0 of the boot partition, the next one at
its header length aligned to 0x4000; if that one is the V2X firmware (version
0, images of type 0xB then 0xC), the boot container is after it; the end of
the boot container's images, rounded up to 1 KiB, is container 2. The
"secondary set" branch of that code exists only for the SCU-based i.MX 8
(`:214-238`, `IS_ENABLED(CONFIG_ARCH_IMX8)`); on the i.MX 95 SPL always reads
the primary set. In eMMC boot mode (`CONFIG_SUPPORT_EMMC_BOOT=y`,
`TU: configs/verdin-imx95_defconfig:135`; `arch/arm/mach-imx/spl.c:242-252`)
SPL reads from the boot partition that `PARTITION_CONFIG` enables
(`image-container.c:373-398`; `common/spl/spl_mmc.c:400-411`): the one the ROM
booted. A whole boot image in boot1 with boot1 enabled is therefore
self-contained.

**What SPL does with container 2** (`TU: common/spl/spl_imx_container.c:24-188`):
it copies each image to its destination (`:52-58`), takes image 0's entry
as the jump target (`:167-170`) and checks nothing unless
`CONFIG_AHAB_BOOT` is set (`:60-63, 149-155`). Toradex's defconfig does not
set it, and the option has no default (`TU: arch/arm/mach-imx/imx9/Kconfig:3-6`).
SPL's heap while it loads is 0x93200000 + 512 KiB
(`TU: configs/verdin-imx95_defconfig:57-59`): the container header is read
into it, so no image of container 2 may land there.

**What BL31 hands BL33** (`TA: plat/imx/imx95/imx95_bl31_setup.c:49-106`):
pc = `PLAT_NS_IMAGE_OFFSET` 0x90200000 (`TA: plat/imx/imx95/include/
platform_def.h:37`, a constant; NXP's newer `lf_v2.12` has it unchanged),
EL2h with all of DAIF masked, `SCTLR_EL2` = RES1 only, so the MMU and caches
off (`TA: lib/el3_runtime/aarch64/context_mgmt.c:994-1009`). With OP-TEE:
x0 = 0 (no devicetree), x1 = 0x8C000000 and x2 = 0x02000000 (OP-TEE's base
and size), x3 = 0x93200000, where BL31 writes a zero word and OP-TEE then
places its devicetree overlay (`BL32_FDT_OVERLAY_ADDR`). PSCI refuses a
non-secure entry point below 0x90200000 (`TA: plat/imx/imx95/imx95_psci.c:494-502`),
which matters for `CPU_ON` of a second core. `SYSTEM_RESET` is the SM's
full-system reset in Toradex's build (pre-study section 1.5).

## 2. What is authenticated on an open part

| Container | Who reads it | What is checked |
| --- | --- | --- |
| 0, ELE firmware | Boot ROM | NXP's signature (it is NXP-signed for every lifecycle). Copied unchanged |
| V2X firmware | Boot ROM / ELE | NXP-signed. Copied unchanged |
| 1, OEI + SM + SPL | Boot ROM / ELE | Unsigned (empty signature block): Toradex ships it so (`info` on `TZ: imx-boot-sd`), and an OEM-open part boots it. Each image carries its SHA-384, which the AHAB design checks on load; the reference manual is not read, so treat a hash mismatch there as fatal. **Copied unchanged**: the tool requires the bytes before container 2 to be identical and reports every container-1 hash (all match on `TZ: imx-boot-sd`) |
| 2, BL31 + BL33 + OP-TEE | U-Boot SPL | Nothing: no ELE call, no hash check (above). The tool still writes the correct SHA-384 for every image, so the container also passes a reader that checks hashes, for example an SPL built with `CONFIG_AHAB_BOOT` on an open part |

A closed part (OEM closed) would need the container signed with the OEM's
key and an SPL with `CONFIG_AHAB_BOOT`; the tool writes unsigned containers
and refuses a base whose container 2 carries a signature. SPL prints the
lifecycle as `LC: 0x...` at boot (`TU: board/toradex/verdin-imx95/spl.c:80-81`);
for the i.MX 95 U-Boot's table names 0x10 "OEM Open"
(`TU: arch/arm/mach-imx/ele_ahab.c:418-462`). Whether SPL's value uses that
table's encoding is a board read (D1).

## 3. Where our image can go in

| | What replaces what | We must provide | We lose | Verdict |
| --- | --- | --- | --- | --- |
| (a) BL33 | U-Boot proper's slot at 0x90200000 in container 2 | an image (or a stub) entered at 0x90200000 in EL2 with the MMU off; containers 0 and 1 untouched | U-Boot proper and everything it does (below) | **recommended** |
| (b) SPL falcon | SPL loads our image as the OS | a rebuilt SPL with `CONFIG_SPL_OS_BOOT` (not in Toradex's defconfig), so a new container 1 | Toradex's SPL as shipped; BL31 (PSCI, the SM's AP-S agent) unless SPL still starts it | rejected: changes container 1 for nothing (a) lacks |
| (c) U-Boot hook | nothing; `bootcmd` runs `ext4load` + `go` | the hook (`tools/uboot-netboot-lib.sh`) | nothing | the baseline and the sign-off route (`docs/standalone-boot.md`, pre-study B2..B5) |
| (d) ROM A/B | a second image set chosen by the ROM | — | — | not a fallback for container 2 (section 5) |

### (a) in detail

There are four ways to fill the slot; two are kept.

- **(a1) A trampoline and the image as a fourth image.** SPL copies every
  image of container 2 to its destination, so `zephyr.bin` goes into
  container 2 as image 3 at its link address 0xd0000000 (the board's
  `zephyr,sram`, PR #9043). The slot at 0x90200000 holds 16 bytes:
  `ldr x16, #8; br x16; .quad 0xd0000000`. BL31 enters the trampoline, the
  trampoline enters the image with x0..x3, EL2 and the MMU state unchanged.
  Nothing new runs on the board but those two instructions, no Zephyr
  loader is built, and the load is SPL's own eMMC read. The image is not
  compressed, so it must fit the boot partition (section 4).
- **(a2) The second-stage loader in the slot.** `driver/samples/zephyr/
  boot_stage2` linked at 0x90200000 with the deflated image linked in
  (`BOOT_STAGE2_SOURCE_EMBEDDED`), inflating to 0xd0000000, as on the
  i.MX8MP (`docs/notes/imx8mp-direct-boot.md`, "The Zephyr side") and the
  AM62P. For an image whose raw size does not fit. Its port is an overlay
  and a few Kconfig defaults for the Verdin iMX95 board, which is in PR
  #9043, not yet on main: RAM 0x90200000 + 32 MiB (it must end below
  0x93200000, OP-TEE's overlay), the payload region 0xd0000000 + 256 MiB
  identity-mapped write-back, `BOOT_STAGE2_LOAD` 0xd0000000,
  `MAX_XLAT_TABLES` 64 (the i.MX8MP's lesson). The loader must arm WDOG3
  before it inflates (section 5).
- (a3) Relinking the image at 0x90200000: rejected. Everything from
  0x93200000 is SPL's heap during the load and OP-TEE's overlay afterwards,
  so the image would be limited to 48 MiB including its RAM, and the GL
  images' carve-out alone is larger (the triangle's header gives an
  effective size of 85 MiB).
- (a4) A TF-A with BL33 at 0xd0000000: rejected. Zephyr's i.MX 95 EVK
  does this with NXP's `bl31-imx95-zephyr.bin` (Zephyr
  `boards/nxp/imx95_evk/CMakeLists.txt:84` at the pin, the SPSDK route), but
  here it would replace Toradex's BL31 (with its full-reset patch) by our
  build, and `BL32_FDT_OVERLAY_ADDR` would move into our image.

**What SPL has already done at BL33 entry** (`TU: board/toradex/verdin-imx95/spl.c:53-96`;
`arch/arm/mach-imx/imx9/scmi/soc.c:1308-1320`): the SCMI channel to the SM (MU2,
the AP-NS agent's channel, the one our image uses) is up and idle, the
LPUART1 console has pins and clock, the A55 runs at its full clock, the
SMMU is disabled, DDR is trained (by the OEI, before SPL), the eMMC
(USDHC1) is initialised, **WDOG3 and WDOG4 are disabled**, and SPL's
devicetree has no watchdog node, so SPL starts none (its DT in `TZ:
imx-boot-sd` has only the gpio, i2c, mmc, serial and usb aliases).

**What U-Boot proper did that we lose:** the DRAM size probe (not needed:
0xd0000000 + 256 MiB is below 4 GiB on every SKU, pre-study 1.1), the
watchdog it arms (WDOG3, `CONFIG_ULP_WATCHDOG`, 30 s, `TU:
configs/verdin-imx95_defconfig:180-181`), the device tree fix-ups and
fused-off node deletion (our devicetree is built in), and the
environment-driven boot (bootcount, Torizon's rollback). The image already
arms WDOG3 at `PRE_KERNEL_1` on the U-Boot route (PR #9043); on this route
that arming is the only watchdog there is.

## 4. Sizes and addresses

Toradex's 7.8.0 image: containers 0 and 1 end at 0xC3400 (781 KiB);
container 2 is 0xC000 of header space, BL31 0x9800, U-Boot 0xEC400 and
OP-TEE 0x91000 bytes, ending at 0x256000 (2.34 MiB). Measured with the tool:

| BL33 side | the image set ends at |
| --- | --- |
| Toradex's U-Boot | 0x256000 |
| (a1) trampoline + `imx95-hello` (49604 bytes) | 0x176400 |
| (a1) trampoline + `examples/triangle` for `verdin_imx95` (3379352 bytes, PR #9043 build) | 0x4A3400 |
| (a2) loader + triangle deflated (zlib -9: 1356020 bytes) + loader (~0.1 MB, the AM62P's) | about 0x2CD000, estimated |

The boot partition's size is not known for this module (the other two
Verdins have 4 MiB, `docs/standalone-boot.md`); U-Boot's environment takes
its last 0x2200 bytes (`CONFIG_ENV_OFFSET=0xFFFFDE00`,
`CONFIG_SYS_MMC_ENV_PART=1`: boot0, `TU: verdin-imx95_defconfig:10-11, 96`).
With 4 MiB the limit is 0x3FDE00: (a1) fits small images, not the
triangle; (a2) fits the triangle. The rules the tool enforces on every
image of container 2: in DDR, not overlapping each other, nothing in
0x93200000..0x93280000, BL31 within 0x8A200000..0x8A240000, OP-TEE within
0x8C000000 + 32 MiB, our image at 0x90200000 or above, 4 KiB aligned, with
an AArch64 Image header and `text_offset` 0.

## 5. The fallback and the way back

**The ROM's A/B does not cover us.** The ROM and the ELE load containers 0
and 1 and stop there. Container 2 is read by SPL, which checks nothing and
always reads the primary set (section 1), so a bad BL33 is never seen by
the ROM. The ROM does know a secondary image set: the ROM passover data
has a boot stage (primary 0x6, secondary 0x9, recovery 0xA, serial 0x5) and
the image set it booted from (`SMT: devices/MIMX95/sm/dev_sm_rom.h:96-99,
141-164`), and the SM can reset into a stage and container by writing
`SRC_GEN` GPR16 and GPR15 (`SMT: devices/MIMX95/sm/dev_sm_rom.c:265-325`,
`dev_sm_system.c:182-204`). The only caller is the SM's debug monitor
(`SMT: sm/utilities/monitor/monitor_cmds.c:1168-1226`), which Toradex's
build leaves out (`TL: recipes-bsp/imx-system-manager/imx-sm-toradex_1.0.0.bb:25`,
`PACKAGECONFIG = "m0"`, `MF: dynamic-layers/arm-toolchain/recipes-bsp/
imx-system-manager/imx-system-manager.inc:12`; the shipped SM has none of
the monitor's strings). No SCMI message reaches it. Where the ROM looks
for a secondary set on eMMC, and what makes it try one, is in the
reference manual only. Our image can read which set it booted from through
SCMI (`ROM_PASSOVER_GET`, as `TU: arch/arm/mach-imx/imx9/scmi/soc.c:58-70`
does), and should record it.

**The eMMC's boot partition switch is the fallback,** as on the i.MX8MP:
Toradex's image stays in boot0 untouched, ours goes into boot1, and
`PARTITION_CONFIG` says which one the ROM reads. Back is one EXT_CSD write
(`mmc bootpart enable 1 <ack> /dev/mmcblk0` from Linux, `mmc partconf 0
<ack> 1 0` from U-Boot). The question is who writes it when the direct
image is the one that boots:

1. **A person, with USB recovery.** The Mallow's recovery button held at
   power-on and its USB device port to the host, as for the Verdin iMX8M
   Plus on the same carrier (`docs/notes/imx8mp-direct-boot.md`,
   "Recovery"). `uuu` (NXP, BSD-3-Clause) then runs a whole boot image from
   RAM: the ROM takes container 0 and 1 over SDP, SPL takes the rest
   (`UUU: uuu/spl_boot.lst`; `-skipspl`). Toradex's SPL enumerates as
   0x1b67:0x4fff, which uuu's built-in table does not know, so the script
   needs Toradex's `CFG:` line (`TZ: uuu.auto`):

   ```
   uuu_version 1.5.165
   SDPS: boot -f <imx-boot>
   CFG: SDPV: -chip SPL1 -vid 0x1b67 -pid 0x4fff
   SDPV: delay 1000
   SDPV: write -f <imx-boot> -skipspl
   SDPV: jump
   ```

   With Toradex's own `imx-boot-sd` this boots U-Boot from RAM, which boots
   Torizon from the eMMC; from there the EXT_CSD write. Or Toradex's
   `recovery-linux.sh` (Toradex Easy Installer), which erases and rewrites
   boot0 (`TZ: image.json`, `mmcblk0boot0`, `seek=0`).
2. **The direct image itself, counted** (a lab default for N boots, the
   `default <image> N` of the other Verdins): the loader (a2) or a small
   pre-stage of the image keeps a count in a sector of boot1 below the
   limit, decrements it before it starts the image, and at 0 writes
   `PARTITION_CONFIG` back to boot0 and resets through PSCI. The AM62P
   loader already writes that byte with CMD6 and reads it back
   (`driver/samples/zephyr/boot_stage2/src/main.c`,
   `BOOT_STAGE2_BOOT0_ON_REQUEST`). It needs Zephyr's USDHC driver on the
   i.MX 95, whose A55 devicetree at the pin has no USDHC node. Not built;
   the next host item if the lab wants a direct default without a person.
3. **The product form needs none,** as `docs/standalone-boot.md` says: a
   product has no Linux to go back to.

**What a failed direct boot looks like.** On UART_3 (Mallow X11, if a
bridge is fitted; pre-study U7) SPL's lines come first in every case:
`U-Boot SPL 2024.04-7.8.0+git.b7c747d8c084`, `SOC:`, `LC:`, `Primary set
selected`, `Load image from MMC/SD 0xc3400`. Then:

| What is wrong | What the console shows | Then |
| --- | --- | --- |
| container 2 header unreadable | `Wrong container header`, `SPL: failed to boot from all boot devices` | SPL hangs; no watchdog runs: **stranded until power-cycle and recovery** |
| BL31 or OP-TEE corrupt | nothing after SPL's lines, or a BL31 panic | hang, no watchdog: stranded |
| the trampoline or the image's start | BL31's `NOTICE:` lines and OP-TEE's, then nothing | hang before `PRE_KERNEL_1`, no watchdog: stranded |
| a hang after the image armed WDOG3 | the image's lines, then silence | WDOG3 resets the board (sys_reset, pre-study 1.5) into **the same image**: a reset loop |

So every failure strands the board until the recovery button, and a
working image that resets ends in itself again. Without a console the only
signs are the panel and the network (no Linux ever answers). This is why
the first run is from RAM (below), and why the loader (a2) and any
pre-stage must arm WDOG3 first thing: from SPL to the image's arming,
nothing bounds a hang.

## 6. `tools/board.sh imx95`: what the direct verbs would do

The board entry (branch `work/imx95-board-sh`) has the U-Boot hook's
verbs only. The direct route would add, in the i.MX8MP's procedure, which
`board.sh` does not automate today on any board:

- `direct-read` (Linux, read-only): `PARTITION_CONFIG` (`mmc extcsd read
  /dev/mmcblk0`), the sizes of `mmcblk0boot0` and `mmcblk0boot1`, a dump of
  both kept outside the checkout (the `IMX95_BOOT_BASE` input of the tool),
  and `imx95-mkboot.py check` on boot0's dump.
- `direct-build <zephyr.bin>` (host): `imx95-mkboot.py build --base <boot0
  dump> --image <zephyr.bin> --limit <boot1 size - 0x2200>` (or `--bl33
  <loader>`), then `check`.
- `direct-ram <image>` (`BOARD_PERSON_PRESENT=1`: the recovery button):
  the uuu script above with the built image. Nothing is written.
- `direct-install <image>` (`BOARD_PERSON_PRESENT=1`, the class of
  `default ... forever`: no Linux comes back by itself): print the undo
  first; `echo 0 > /sys/block/mmcblk0boot1/force_ro`, `dd` the image to
  boot1 with `conv=fsync`, read it back by sha256, `mmc bootpart enable 2
  <ack> /dev/mmcblk0`, read `PARTITION_CONFIG` back. boot0 and the user
  area are not written.
- `direct-uninstall` (Linux, after a recovery boot): `mmc bootpart enable 1
  <ack> /dev/mmcblk0`, read back; boot1 restored from its dump.

`<ack>` is the boot-acknowledge bit as `direct-read` found it (0x48 on the
i.MX8MP: boot0 with ack).

## 7. The host tool

`tools/imx95-mkboot.py` (Apache-2.0, Python standard library only; no code
from mkimage, which is GPL-2.0, only its format, read from it and from
U-Boot's reader):

- `info IMAGE`: every container as SPL finds it, every image with its role,
  type, core, hash and whether the hash matches.
- `check IMAGE [--limit N]`: the rules of section 4, container 2's
  structure, every hash, image 0 = BL31, exactly one image at 0x90200000,
  a trampoline's target loaded, nothing signed in container 2, the limit.
- `extract IMAGE DIR`: container 2's images as files (BL31 and OP-TEE are
  the inputs a build reuses).
- `build [--base IMAGE] (--image zephyr.bin [--load 0xd0000000] | --bl33
  FILE) -o OUT --limit N [--bl31 FILE] [--tee FILE | --no-tee]`: copies
  everything before container 2 byte for byte (and verifies that it did),
  rebuilds container 2 in mkimage's form with the base's BL31 and OP-TEE,
  runs `check` on the result. The base is the board's own boot image
  (`--base` or `IMX95_BOOT_BASE`): a dump of boot0, or Toradex's
  `imx-boot-sd`. No vendor binary is committed.

Run on `TZ: imx-boot-sd`: `info` finds container 0 at 0 (signed, 1 image),
the V2X container at 0x4000 (version 0, tag 0x82, signed), container 1 at
0x8000 (5 images), container 2 at 0xC3400 (BL31, U-Boot, OP-TEE), all 11
hashes matching, and `check --limit 0x3FDE00` passes. `build --image` with
`imx95-hello` gives an image whose first 0xC3400 bytes are the base's and
whose container 2 checks clean.

`tools/imx95_mkboot_selftest.py` builds a synthetic image of the same
form with its own packing (no vendor bytes), and checks the tool's output
field by field: the bytes before container 2, the header, the image order,
offsets and padding, every SHA-384, the trampoline's two instructions and
target, the image at its load address, the base's container flags; the
V2X and no-V2X layouts; and every refusal (limit, no Image header, load
below 0x90200000, SPL's heap, `--no-tee` without `--bl31`, no base, a
signed container 2, a changed byte, BL31 not first, a trampoline to
nothing, no container at SPL's offset).

## 8. Recommendation

(a1) for images that fit the boot partition uncompressed, (a2) for the GL
images, both built by `tools/imx95-mkboot.py` from the board's own boot0,
installed in boot1, with boot0 kept as Toradex's. (a1) first: it needs no
new code on the board, so the first direct run tests only the chain.
The U-Boot hook (c) stays the sign-off route, and the direct route comes
after it (pre-study B7).

## 9. Board steps, in order

Each step stops at the first line that differs.

| # | Step | Needs | Done when |
| --- | --- | --- | --- |
| D1 | `direct-read` from Torizon: `PARTITION_CONFIG`, boot0/boot1 sizes, both dumps; `imx95-mkboot.py check` on boot0 | Linux on the board (B1) | the module's boot0 parses as Toradex's 7.8.0 image does, the sizes and the ack bit recorded; SPL's `LC:` line if a console is there |
| D2 | RAM trial (a1) with `imx95-hello`: recovery button, the uuu script | a person; USB to the host; a console or the panel | the hello banner after BL31's lines; then a power-cycle boots Torizon (nothing written) |
| D3 | RAM trial (a1) with an image that arms WDOG3 and never feeds | D2 | the WDOG3 reset boots from the eMMC: Toradex's boot0, then Torizon |
| D4 | RAM trial (a2) with the loader and the triangle | the loader port after PR #9043 | the triangle on the X4 panel; the loader's time |
| D5 | `direct-install` the image of D2 or D4 to boot1, power-cycle | a person | the image at power-on |
| D6 | the way back: recovery, Toradex's `imx-boot-sd` over uuu, Torizon, `direct-uninstall` | a person | Torizon at the next power-on from boot0 |

D3 is skipped if the watchdog was proved on the U-Boot route (pre-study B2).

## 10. Open points only the board answers

1. The boot partitions' size and `PARTITION_CONFIG` (boot0 enabled, the
   ack bit), and that boot1 is empty (D1).
2. That the module's boot0 is Toradex's 7.8.0 image or one of the same
   form: the V2X container present, container 2 at the offset SPL computes,
   OP-TEE in it (D1; the tool's `check`).
3. The lifecycle (`LC:`): open, as all of this assumes (D1/D2 with a
   console).
4. That the ROM, booting boot1, accepts the unchanged containers 0 and 1
   there (D5; the same bytes boot from boot0 today).
5. Where the ROM finds a secondary image set on eMMC and when it uses one
   (reference manual; not needed by this design).
6. That SPL's copy of an image to 0xd0000000 is coherent at BL33 entry, as
   its copy of U-Boot to 0x90200000 is today (D2).
7. Whether Toradex Easy Installer's recovery resets `PARTITION_CONFIG` to
   boot0 (D6; until then the way back is U-Boot from RAM and Linux).
