<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Direct boot on the Verdin iMX95: our image in place of U-Boot proper

Goal: the Verdin iMX95 on the Mallow powers on into our Zephyr image from
the eMMC, with no U-Boot proper and no Linux, and a lab board can always be
brought back to Toradex's boot. This is item H5 and board step B7 of
`docs/notes/pre-study-imx95.md`. The model is `docs/standalone-boot.md` (per
board: what replaces what, the fallback, the proof) and the i.MX8MP's direct
route (`docs/notes/imx8mp-direct-boot.md`), which ran on 2026-10-21.

Status (2026-11-25): **nothing of the direct route has run on the board.**
No direct image has been written to the eMMC, no image has been started by
BL31, and the second-stage loader has not run. What the board has shown is
D1 (section 9: the module's boot partitions, `PARTITION_CONFIG` and boot0's
image, all read from Linux; the console's SPL lines) and the U-Boot route's
images (`go 0xd0000000`: WDOG3, PSCI reset, SCMI, gl_test). Built and
host-tested, not run: the second-stage loader for this board with the
counted way back (section 5, "The counted way back"), and
`tools/imx95-mkboot.py` with the count record (section 7). Its first board
run is section 11.

Earlier (2026-11-21): `tools/imx95-mkboot.py` builds the boot image and
checks it field by field. It was run on Toradex's published imx-boot for this
module, where it found every container and every hash as the SPL and mkimage
define them, and it rebuilt container 2 around a Zephyr image (section 7).
`tools/imx95_mkboot_selftest.py` tests it on synthetic images.

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
(`TU: arch/arm/mach-imx/ele_ahab.c:418-462`). The board prints `LC: 0x40010`
(2026-11-25). Its low 16 bits are 0x0010, "OEM Open" in that table; the
upper bits were not decoded here. The part is open whatever they mean:
Toradex's container 1 is unsigned (`info` on boot0's dump), and a closed
part does not boot an unsigned container.

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
  AM62P. For an image whose raw size does not fit, and the carrier of the
  counted way back (section 5). **Built, not run** (2026-11-25):
  `boards/verdin_imx95_mimx9596_a55.{overlay,conf}` and Kconfig defaults.
  The loader's RAM is 0x90200000 + 32 MiB, ending at 0x92200000; a
  `BUILD_ASSERT` keeps it below 0x93200000, OP-TEE's overlay. The payload
  region is the board's `memory@d0000000`, 256 MiB, identity-mapped
  write-back; `BOOT_STAGE2_LOAD` 0xd0000000, `_STAGING` 0xdc000000 (192 MiB
  of image at most), `_REGION_END` 0xe0000000. `KERNEL_VM_SIZE` 32 MiB
  inside the loader's RAM, `MAX_XLAT_TABLES` 64. Built with the
  `imx95_wdog` probe as payload: 127616 bytes, AArch64 header with
  `text_offset` 0 and `image_size` 0x89000 (RAM to 0x92289000).
  **WDOG3 is armed by the board glue**
  (`driver/zephyr/lib/board/verdin_imx95/verdin_imx95_wdog.c`, in every image
  for the board, this one included), at `PRE_KERNEL_1` priority 0: the first
  init hook, before the console, SCMI, the count or the inflate. This is the
  arming that ran on the board on 2026-11-25 (U-Boot route: found stopped,
  rc 0, unfed reset after 60 s). Before it run only BL31's hand-over and
  Zephyr's reset path (`__start`, the MMU set-up, the `.bss` clear), which
  nothing bounds. The glue also feeds WDOG3 while the loader runs. Its 60 s
  lifetime then takes a PSCI reset, and `bs2_jump` stops the timers. The
  payload arms WDOG3 again at its own `PRE_KERNEL_1`, now finding it running
  (`yd_wdog32_arm` reconfigures a running WDOG32; host-tested in
  `ip::wdog32`, not yet seen on the board).
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

The boot partitions are 31.5 MiB each on this module (D1, 2026-11-25:
`BOOT_SIZE_MULT` 0xfc, 64512 sectors). U-Boot's environment takes boot0's
last 0x2200 bytes, sectors 64495..64511 (`CONFIG_ENV_OFFSET=0xFFFFDE00`,
`CONFIG_SYS_MMC_ENV_PART=1`: boot0, `TU: verdin-imx95_defconfig:10-11, 96`).
boot1 has no environment. The counted way back keeps its record in boot1's
sector 64494 (section 5). So an image for boot1 ends at or below 0x1F7DC00
(`--limit boot1`), and one for boot0 at or below 0x1F7DE00 (`--limit
boot0`). Both (a1) and (a2) fit every image in the tree with room to spare:
the triangle's (a1) set ends at 0x4A3400. The rules the tool enforces on every
image of container 2: in DDR, not overlapping each other, nothing in
0x93200000..0x93280000, BL31 within 0x8A200000..0x8A240000, OP-TEE within
0x8C000000 + 32 MiB, our image at 0x90200000 or above, 4 KiB aligned, with
an AArch64 Image header and `text_offset` 0; a BL33 that carries an AArch64
header (the loader) with its `image_size` ending at or below 0x93200000.

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
   `default <image> N` of the other Verdins). **Built and host-tested, not
   run** (2026-11-25): the loader (a2) keeps a count in boot1's sector 64494.
   It writes the count back decremented, and reads it back, before it
   starts its payload. At 0 it writes `PARTITION_CONFIG` back to boot0 and
   resets through PSCI ("The counted way back", below).
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

So without the count every failure strands the board until the recovery
button, and a working image that resets ends in itself again. Without a
console the only signs are the panel and the network (no Linux ever
answers). This is why the loader (a2) arms WDOG3 first thing: from SPL to
the arming, nothing bounds a hang. The count below changes the last row
and the working image's reset. It cannot change the rows before the
loader runs.

### The counted way back (built, not run)

`driver/samples/zephyr/boot_stage2` for `verdin_imx95/mimx9596/a55`,
Kconfig `BOOT_STAGE2_COUNT` (default y on this board). The logic is the
loader crate's `loader/src/emmc.rs` (the record, the verdict, the command
sequence) and `loader/src/usdhc.rs` (the controller). The C side
(`src/main.c`, `count_step`) maps the controller, reads its clock rate,
prints the lines and resets.

**Where the count lives: boot1, sector 64494** (byte offset 0x1F7DC00,
`BOOT_STAGE2_COUNT_SECTOR`). The board's boot partitions are 64512 sectors.
U-Boot's environment is boot0's sectors 64495..64511 (section 4). 64494 is
the highest sector outside that range in either partition:

- a record written to the wrong partition cannot reach U-Boot's environment;
- an image in boot1 may take everything below it (`--limit boot1`);
- the ROM and SPL read only the image set from offset 0, which ends far
  below it.

The record is not in the user area: Torizon owns the user area, and a
record there could be overwritten by an update, or could overwrite one.
The loader checks the sector against the card's `BOOT_SIZE_MULT`.

**The record** (one sector, `tools/imx95-mkboot.py count`). Magic
`YKBOOTCT`, version 1, `remaining`, `initial`, its own sector number, zero
bytes, and a CRC-32 over bytes 0..508 (layout in `emmc.rs`). Any record
that does not check means the way back:

- boot1 as delivered (all zero bytes);
- a torn write;
- a CRC mismatch;
- a record copied from another sector;
- `remaining` > `initial`.

So a single-sector write needs no reliable-write mode: an interrupted
write fails toward boot0.

**The sequence, in order.** It runs first in `main()`, after the init hooks
(WDOG3 above, the console, SCMI) and before the payload's header is even
read. Neither the count nor the way back depends on the payload.

1. The uSDHC: software reset (`RSTA`), `VSELECT` kept as found, the tuning,
   DLL and HS400 bits off, 1-bit little-endian, polled (no interrupt, no
   DMA), 80 init clocks. The SD clock is the root rate over SCMI
   (`IMX95_CLK_USDHC1`, `CLOCK_RATE_GET`, read only) divided to at most
   400 kHz, later at most 20 MHz. When SCMI does not answer, the loader
   assumes 1.6 GHz, which only makes the clocks slower.
2. The card from CMD0: CMD1 until ready (sector-addressed, else stop),
   CMD2, CMD3 (RCA 2), CMD7, CMD13 until TRAN.
3. EXT_CSD (CMD8): `PARTITION_CONFIG` as found, `BOOT_SIZE_MULT`, `BOOT_WP`.
4. CMD6 `PARTITION_CONFIG` = found with access 2 (boot1). Only the volatile
   access field changes; `BOOT_ACK` and `BOOT_PARTITION_ENABLE` are written
   with the values just read. Then CMD17 of the record.
5. The record checks and `remaining` > 0: CMD24 of `remaining - 1`, CMD13
   until TRAN, CMD17, compare. Then CMD6 access 0 (user area), and the
   payload. The decremented count is on the card before the payload's
   first byte is unpacked.
6. Otherwise the way back:
   - when the cause is spent, invalid, a failed step of 4 or 5, or the
     word on the console: CMD6 access 0;
   - then CMD6 `PARTITION_CONFIG` = (found & 0x40) | 0x08: `BOOT_ACK` as
     found, boot0, user area (0x48 on this board from 0x48, 0x50 or 0x52);
   - EXT_CSD read again and the byte compared;
   - the board glue's PSCI `SYSTEM_RESET`.

Only `PARTITION_CONFIG` and the count sector are ever written. The card
model's CMD6 asserts that every CMD6 is to byte 179. The console request is
`BOOT_STAGE2_BOOT0_WORD` (`YK-BOOT0-9E3A71C5`, the AM62P's): looked for when
the loader starts and once more before the jump. It is cheap, because the
polled UART is the console already. It is safe: it takes the same way back,
and a 17-character word does not arrive by accident (the AM62P's lesson,
2026-11-12). A request leaves the count as it is.

**What the loader relies on from SPL, and what it redoes.** SPL booted from
this card and initialised USDHC1 (section 3). The loader relies on:

- USDHC1's clock root being on;
- its pads muxed to USDHC1;
- the A55 non-secure side's access to the block. Linux uses USDHC1 on this
  board, so the System Manager grants it.

The pads are not set again: Linux's pad settings for `usdhc1` could not be
read on the host that built this (no network), so the loader keeps what SPL
left. Three pads carry anything in 1-bit mode. The loader redoes the
controller's reset and set-up, the SD clock, and the whole card
initialisation from CMD0. SPL's bus width, timing and tuning (HS400 or
HS200, 8-bit) are not reused. CMD0 resets the card's volatile EXT_CSD fields
(bus width, timing, the access field), so the found `PARTITION_CONFIG` shows
no access left by SPL. Zephyr's own `imx_usdhc` driver cannot be used at the
pin: its MCUX `fsl_usdhc` needs `USDHC_Type`, which the MIMX9596 A55 device
header names `WAKEUP_USDHC_Type`, with no USDHC features. So the loader has
its own polled path. The node is `stage2_emmc` in the loader's overlay
(binding `yakogl,imx95-usdhc-boot`): reg 0x42850000, the SCMI clock.

**The order of safety.** The direct chain's failures with the count, in
section 5's form (`D5c`: the loader with `BOOT_STAGE2_COUNT` installed in
boot1 with a record of N):

| What is wrong | What the console shows | Then | Cost |
| --- | --- | --- | --- |
| the ROM does not boot boot1's containers 0 and 1 (open point 4) | no SPL line | unknown (reference manual) | **stranded**: one USB recovery |
| container 2 unreadable, BL31 or OP-TEE corrupt | SPL's lines, then `Wrong container header` or nothing | hang, no watchdog | **stranded**: one USB recovery |
| the loader does not start, or hangs before `PRE_KERNEL_1` (BL31's entry differs from U-Boot's `go`) | BL31's and OP-TEE's lines, then nothing | hang, no watchdog | **stranded**: one USB recovery |
| the loader hangs after WDOG3 is armed, before the count | the loader's banner and WDOG3 line | WDOG3 resets into the same chain, every time | **stranded** (a reset loop): one USB recovery |
| the eMMC path fails before EXT_CSD is read | `boot_stage2: count: FAILED at CMD1 ...` (or CMD0..CMD7, EXT_CSD read) | stop, the glue's 60 s lifetime reset, the same chain | **stranded** (a loop): one USB recovery |
| a step fails after EXT_CSD (access, record read, write, read-back) | `count: failure at ...`, then `a step failed: PARTITION_CONFIG 0x50 -> 0x48` | the way back, reset | none: boot0, U-Boot, Linux |
| the way back's CMD6 or its read-back fails | `count: FAILED at CMD6 PARTITION_CONFIG boot0` | stop, lifetime reset, the same chain | **stranded** (a loop): one USB recovery |
| no starts left, or no valid record | `no starts left` / `no valid record: PARTITION_CONFIG 0x50 -> 0x48` | reset into boot0 | none |
| the payload hangs, panics or resets | the payload's lines | WDOG3 or its PSCI reset, then the loader counts | none: after N starts, boot0 |
| power lost during the record's write | `no valid record` at the next start | the way back | none |

Everything in the stranding rows runs before or below the count. Which of
them can be ruled out before boot1 is enabled is section 11's staging: the
eMMC path and the way back run first from U-Boot (`go 0x90200000`, boot0
enabled, so the same writes change nothing that boots). The BL31 entry runs
next from RAM over USB, if the recovery cable is there. What then remains
for the install itself is the ROM and SPL reading boot1 (open point 4). Each
of those costs one USB recovery at most, because nothing written before it
is lost: boot0 and the user area are never written.

Host tests (`cargo test -p yakogl-boot-stage2`, `loader/src/emmc_tests.rs`,
18 tests) run the sequence against a model of an eMMC 5.1 card, with:

- the states, the R1 status and SWITCH_ERROR;
- `PARTITION_CONFIG`'s persistent and volatile fields, and the boot
  partitions; the card starts as the board's did;
- every fault in the table that can happen after the loader starts:
  - a card that is never ready, or is byte-addressed;
  - a CMD6 refused, or silently not applied;
  - write protection;
  - a torn write;
  - a card that stays busy;
  - a boot partition that is too small.

The same sequence runs again through a model of the uSDHC's registers
(mode bits in `MIX_CTRL`, one block, the data port only after BRR/BWR, the
R2 layout, `VSELECT` kept, `IPP_RST_N` kept). The model is this module's
reading of the registers, not the hardware: the board run (section 11) is
the test of that reading.

## 6. `tools/board.sh imx95`: what the direct verbs would do

The board entry (branch `work/imx95-board-sh`) has the U-Boot hook's
verbs only. The direct route would add, in the i.MX8MP's procedure, which
`board.sh` does not automate today on any board:

- `direct-read` (Linux, read-only): `PARTITION_CONFIG` (`mmc extcsd read
  /dev/mmcblk0`), the sizes of `mmcblk0boot0` and `mmcblk0boot1`, a dump of
  both kept outside the checkout (the `IMX95_BOOT_BASE` input of the tool),
  and `imx95-mkboot.py check` on boot0's dump.
- `direct-build <zephyr.bin>` (host): `imx95-mkboot.py build --base <boot0
  dump> --image <zephyr.bin> --limit boot1` (or `--bl33 <loader>`), then
  `check`; with the loader, `imx95-mkboot.py count --remaining N` for the
  record.
- `direct-ram <image>` (`BOARD_PERSON_PRESENT=1`: the recovery button):
  the uuu script above with the built image. Nothing is written.
- `direct-install <image>` (`BOARD_PERSON_PRESENT=1`, the class of
  `default ... forever`: no Linux comes back by itself): print the undo
  first; `echo 0 > /sys/block/mmcblk0boot1/force_ro`, `dd` the image to
  boot1 with `conv=fsync`, read it back by sha256; with the loader, the
  record to boot1's sector 64494 (`dd ... seek=64494`), read back the same
  way; then `mmc bootpart enable 2 <ack> /dev/mmcblk0`, read
  `PARTITION_CONFIG` back. boot0 and the user area are not written. With
  the counted loader this is the class of `default <image> N`: Linux comes
  back by itself after N starts (section 5), unless one of the table's
  stranding rows happens.
- `direct-uninstall` (Linux, after a recovery boot): `mmc bootpart enable 1
  <ack> /dev/mmcblk0`, read back; boot1 restored from its dump.

`<ack>` is the boot-acknowledge bit as `direct-read` found it: 1 on this
board (`PARTITION_CONFIG` 0x48, D1), as on the i.MX8MP.

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
  `imx-boot-sd`. No vendor binary is committed. `--limit` takes `boot1`
  (0x1F7DC00, up to the count sector) or `boot0` (0x1F7DE00, up to U-Boot's
  environment) by name.
- `count --remaining N [--initial M] -o OUT`: the loader's count record,
  one sector, and the `dd` line that writes it to boot1's sector 64494.
  `count-info FILE` (a sector, a whole boot1 dump, or `-`): the record as the
  loader will read it, or why it will take the way back.

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
nothing, no container at SPL's offset). Since 2026-11-25 also: a loader in
the BL33 slot and the refusal of one whose `image_size` reaches 0x93200000,
the named limits, and the count record. Its bytes, `count-info` on a sector,
on a boot1 dump, on zeros and on a changed byte, and a CRC-32 pinned on both
sides (0xB8414914 for 3 of 3, the same value the loader's
`record_layout_and_crc` test pins).

On the board (2026-11-25, the owner's session): `check` on boot0's dump
passes ("ok: container 2 at 0xc3400, 3 images (bl31, bl33, tee), ends at
0x256000"), and `build --base <dump> --image <imx95_wdog zephyr.bin>` gives
a 4-image container 2 ending at 0x178400 that checks clean.

## 8. Recommendation

(a1) for images that fit the boot partition uncompressed, (a2) for the GL
images, both built by `tools/imx95-mkboot.py` from the board's own boot0,
installed in boot1, with boot0 kept as Toradex's. For trials without a
person at the board, only (a2) with the count: (a1) has no way back but USB
recovery, so it is for a run with the recovery cable connected and a person
there. The counted loader's first run is from U-Boot (section 11), so the
eMMC path and the way back are proved before anything boots from boot1.
The U-Boot hook (c) stays the sign-off route, and the direct route comes
after it (pre-study B7).

## 9. Board steps, in order

Each step stops at the first line that differs.

| # | Step | Needs | Done when |
| --- | --- | --- | --- |
| D1 | `direct-read` from Torizon: `PARTITION_CONFIG`, boot0/boot1 sizes, both dumps; `imx95-mkboot.py check` on boot0 | Linux on the board (B1) | **done 2026-11-25** (below) |
| C1 | the counted loader from U-Boot (`go 0x90200000`), boot0 enabled, a record of 1 in boot1: the count path | the console; Linux and U-Boot as they are; no person | section 11, step 3 |
| C2 | the same once more, the record now 0: the way back (writes 0x48 over 0x48) and the PSCI reset | C1 | section 11, step 4 |
| D2 | RAM trial (a1) with `imx95-hello`: recovery button, the uuu script | a person; the USB recovery cable to the host; `uuu` on the host; a console or the panel | the hello banner after BL31's lines; then a power-cycle boots Torizon (nothing written) |
| D3 | RAM trial (a1) with an image that arms WDOG3 and never feeds | D2 | the WDOG3 reset boots from the eMMC: Toradex's boot0, then Torizon |
| D4 | RAM trial (a2): the counted direct image over uuu (BL31 enters the loader) | D2; C1, C2 | the loader's lines after BL31's; the count as in C1 |
| D5 | `direct-install` the counted direct image (record 1) to boot1, enable boot1, reboot | C1, C2; D4 if the cable is there | section 11, step 6: the payload, then by itself boot0, U-Boot, Linux |
| D6 | the way back by hand: recovery, Toradex's `imx-boot-sd` over uuu, Torizon, `direct-uninstall` | a person | Torizon at the next power-on from boot0 (only when D5 strands) |

D3 is skipped: the watchdog was proved on the U-Boot route (2026-11-25,
`imx95_wdog`). D2 and D4 need the cable and `uuu`, which the lab does not
have yet. Section 11 says what the first run does without them.

**D1, 2026-11-25** (read by the owner's session from the board's Linux):

- `PARTITION_CONFIG` 0x48: boot0 enabled, `BOOT_ACK` set, no access to a
  boot partition. `BOOT_BUS_CONDITIONS` 0x12, `BOOT_INFO` 0x07,
  `BOOT_SIZE_MULT` 0xfc.
- `/sys/block/mmcblk0boot{0,1}/size` 64512 each. boot1 is all zero bytes.
- The eMMC is "0IM20E", manfid 0x13, MMC 5.1, HS400ES, 29.6 GiB.
- boot0's dump passes `check`. `info`: container 0 (ELE, signed), V2X at
  0x4000, container 1 at 0x8000 (5 images, unsigned), and container 2 at
  0xc3400: BL31 0x9800 bytes to 0x8a200000, U-Boot 0xec400 to 0x90200000,
  OP-TEE 0x91000 to 0x8c000000. All hashes ok.
- SPL's console lines are `U-Boot SPL 2024.04-7.8.0+git.b7c747d8c084`,
  `SOC: 0xb0009500`, `LC: 0x40010`, `Trying to boot from MMC1`, `Primary
  set selected`, `Load image from MMC/SD 0xc3400`, then `NOTICE:  BL31:
  v2.10.0 (release):lf-6.6.52-2.2.1-dirty`.
- U-Boot's `saveenv` writes boot0's last 0x2200 bytes.

## 10. Open points only the board answers

1. **Answered (D1, 2026-11-25).** The boot partitions are 64512 sectors
   (31.5 MiB) each. `PARTITION_CONFIG` is 0x48: boot0 enabled, ack set.
   boot1 is all zero bytes.
2. **Answered (D1).** boot0 is Toradex's 7.8.0 form: the V2X container at
   0x4000, container 2 at 0xc3400 where SPL computes it (SPL prints `Load
   image from MMC/SD 0xc3400`), OP-TEE in it, `check` ok.
3. **Answered (D1, console).** `LC: 0x40010`; the part is open (section 2:
   the unsigned container 1 boots).
4. That the ROM, booting boot1, accepts the unchanged containers 0 and 1
   there (D5; the same bytes boot from boot0 today).
5. Where the ROM finds a secondary image set on eMMC and when it uses one
   (reference manual; not needed by this design).
6. That SPL's copy of an image to 0xd0000000 is coherent at BL33 entry, as
   its copy of U-Boot to 0x90200000 is today (D2).
7. Whether Toradex Easy Installer's recovery resets `PARTITION_CONFIG` to
   boot0 (D6; until then the way back is U-Boot from RAM and Linux).
8. The counted loader's reading of the hardware (C1, C2). Four things the
   host tests cannot settle:
   - the uSDHC register sequence (the model is this module's reading of the
     MCUX header and of U-Boot's driver);
   - that USDHC1's root is on and its pads still muxed when the loader runs;
   - the root rate SCMI reports;
   - that `BOOT_WP` (EXT_CSD 173, which D1 did not record) leaves boot1
     writable. A protected boot1 fails the record's write; that row of the
     table goes back to boot0.
9. That BL31's entry into the loader (MMU off, EL2, x0..x3 as section 1
   says) starts it as U-Boot's `go` does (D4, or D5 without the cable).

## 11. The first board run of the counted loader (planned, not run)

For the lab as it is on 2026-11-25:

- the console on UART_3 through a USB adapter, read and written by the
  owner's session;
- Linux with `mmc`, `dd` and `fw_setenv`;
- U-Boot with its autoboot interruptible on the console;
- **no `uuu` on the host and no USB recovery cable.**

Steps 1 to 5 need neither: boot0 stays enabled throughout, so every reset
lands in Toradex's chain. Step 6 enables boot1. **Before step 6 a person
connects the Mallow's USB device port to the host and installs `uuu` with
Toradex's `imx-boot-sd` and the script of section 5.** A stranding failure
there then costs one USB recovery: the recovery button held at power-on by
a person, then D6. Placeholders: `<board>` is the board's Linux (ssh, from
the farm's lease), `<console>` its UART_3 console, `<boot0.bin>` the D1
dump of boot0 (kept outside the checkout), `<staged>` the path in `mmc 0:1`
under which Linux stages a file for U-Boot (`/var/yakogl`, as the
`uboot-mmc` route stages it, `docs/standalone-boot.md`).

**1. Host: build.** Built with `-DYAKOGL_WERROR=ON` on 2026-11-25 (payload
`imx95_wdog`: 57896 bytes, stored 23418 deflated; loader 127616 bytes).

```sh
west build -p always -b verdin_imx95/mimx9596/a55 -d build/imx95_wdog \
  yakogl/tools/probes/verdin_imx95/imx95_wdog -- -DYAKOGL_WERROR=ON
python3 tools/am62p-mkboot.py stage2img --zephyr build/imx95_wdog/zephyr/zephyr.bin \
  --load 0xd0000000 --compress deflate --name imx95_wdog --staging 0xdc000000 -o build/imx95_wdog.img
west build -p always -b verdin_imx95/mimx9596/a55 -d build/bs2-imx95 \
  yakogl/driver/samples/zephyr/boot_stage2 -- -DYAKOGL_WERROR=ON \
  -DBOOT_STAGE2_PAYLOAD=$PWD/build/imx95_wdog.img
python3 tools/imx95-mkboot.py count --remaining 1 -o build/count1.bin
python3 tools/imx95-mkboot.py build --base <boot0.bin> --bl33 build/bs2-imx95/zephyr/zephyr.bin \
  -o build/direct-bs2.bin --limit boot1
```

`imx95_wdog` as the payload: it never feeds WDOG3, so each start of it
ends in WDOG3's reset 60 s later. That is a payload "that resets", with no
person. The direct image was not built here: its base is the board's boot0
dump, which is not on the build host. The selftest builds the same form
from a synthetic base.

**2. Linux on `<board>`: the record, boot0 still enabled.** Costs nothing
if it fails: boot1 is not booted.

```sh
mmc extcsd read /dev/mmcblk0 | grep -E 'PARTITION_CONFIG|BOOT_WP'
#   PARTITION_CONFIG: 0x48; BOOT_WP 0x00 expected (not recorded by D1: open point 8)
echo 0 > /sys/block/mmcblk0boot1/force_ro
dd if=count1.bin of=/dev/mmcblk0boot1 bs=512 seek=64494 count=1 conv=fsync
dd if=/dev/mmcblk0boot1 bs=512 skip=64494 count=1 | sha256sum   # = sha256 of count1.bin
echo 1 > /sys/block/mmcblk0boot1/force_ro
cp bs2-imx95.bin <staged>/bs2-imx95.bin && sync && reboot
```

**3. C1, the count, from U-Boot** (interrupt autoboot on `<console>`):

```
=> ext4load mmc 0:1 0x90200000 <staged>/bs2-imx95.bin
=> dcache off; icache flush; go 0x90200000
```

If `ext4load` refuses the address as reserved, load at 0x90400000
(`loadaddr`) and `cp.b 0x90400000 0x90200000 0x20000` first. Expected, in
order:

```
*** Booting Zephyr OS build ... ***
verdin_imx95: WDOG3 armed: CS 0x00002900 -> 0x... (was stopped), TOVAL 0x400 -> 0x1d4c, timeout 60000 ms, ...
board: watchdog armed
boot_stage2: count: eMMC mid 0x13, PARTITION_CONFIG 0x48, boot partitions 64512 sectors, BOOT_WP 0x00; SD clock <=400000/<=20000000 Hz from <rate> Hz
boot_stage2: count: boot1 sector 64494: 1 of 1 left
boot_stage2: count: 0 left after this start (written, read back); starting the payload
boot_stage2: image carried in this loader (24064 bytes at 0x...), load 0xd0000000
boot_stage2: "imx95_wdog": 23418 bytes stored (deflate), 57896 bytes image, entry 0xd0000000, ...
boot_stage2: SCTLR_EL1 M=1 C=1 I=1; memcpy 1 MiB in ... us
boot_stage2: "imx95_wdog" ok (CRC-32 45bc9d3c): ... jumping to 0xd0000000 at ... ms
*** Booting Zephyr OS build ... ***
verdin_imx95: WDOG3 armed: CS 0x... -> 0x... (was running), ...
imx95_wdog: not fed: reset expected within 60000 ms of the arming ...
imx95_wdog: alive at 10 s ... alive at 50 s
U-Boot SPL 2024.04-7.8.0+git.b7c747d8c084 ... (boot0) ... LM Boot reason: fccu ... ; Linux
```

The sizes and CRCs are those `stage2img` printed for this build. `<rate>` is
what the System Manager reports for `IMX95_CLK_USDHC1` (0 shows as the
assumed 1600000000). Then from Linux: `dd if=/dev/mmcblk0boot1 bs=512
skip=64494 count=1 > c.bin`, and on the host `imx95-mkboot.py count-info
c.bin`: "0 of 1 starts left: the next start takes the way back".
`PARTITION_CONFIG` is still 0x48.

What a failure costs here:

- the loader hangs before its WDOG3 line (U-Boot left WDOG3 stopped): one
  power cycle (a person, or the farm's power control if the board has one);
- any later hang: WDOG3's reset, 60 s;
- `count: FAILED ...`: the glue's 60 s lifetime reset.

Each of these ends in boot0, U-Boot, Linux. The one residual risk is a
`PARTITION_CONFIG` write other than the one designed: the loader writes
179 with the enable and ack fields it read, so a fault of this module's
reading of the hardware is the risk the host model cannot remove.

**4. C2, the way back, from U-Boot:** the same two commands. Expected:

```
boot_stage2: count: eMMC mid 0x13, PARTITION_CONFIG 0x48, boot partitions 64512 sectors, ...
boot_stage2: count: boot1 sector 64494: 0 of 1 left
boot_stage2: count: no starts left: PARTITION_CONFIG 0x48 -> 0x48 (boot0, read back)
verdin_imx95: boot_stage2: count: the next start is boot0: PSCI SYSTEM_RESET at ... ms
U-Boot SPL ... LM Boot reason: sw ... ; Linux
```

The write here is 0x48 over 0x48: the same CMD6, its busy, its status and
the read-back as from boot1, with nothing changing what boots. Failure
costs as in step 3.

**5. (Optional) C3, the request:** step 2's record of 1 again, then the
`go` while the host sends `YK-BOOT0-9E3A71C5` repeatedly on `<console>`.
Expected `"YK-BOOT0-9E3A71C5" on the console ...`, then `requested on the
console: PARTITION_CONFIG 0x48 -> 0x48 (boot0, read back)` and the reset.
`count-info` afterwards still shows "1 of 1": the request leaves the count.

**6. D5, the counted direct boot from boot1.** Only after 3 and 4 showed
their lines. With the cable and `uuu` there, D4 (the same image from RAM
over uuu) first rules out the BL31 entry (open point 9). Linux on `<board>`:

```sh
mmc extcsd read /dev/mmcblk0 | grep PARTITION_CONFIG      # 0x48; undo at any time: mmc bootpart enable 1 1 /dev/mmcblk0
echo 0 > /sys/block/mmcblk0boot1/force_ro
dd if=direct-bs2.bin of=/dev/mmcblk0boot1 bs=512 conv=fsync
dd if=count1.bin of=/dev/mmcblk0boot1 bs=512 seek=64494 count=1 conv=fsync
dd if=/dev/mmcblk0boot1 bs=512 count=<sectors of direct-bs2.bin> | sha256sum   # = direct-bs2.bin's
dd if=/dev/mmcblk0boot1 bs=512 skip=64494 count=1 | sha256sum                 # = count1.bin's
echo 1 > /sys/block/mmcblk0boot1/force_ro
mmc bootpart enable 2 1 /dev/mmcblk0
mmc extcsd read /dev/mmcblk0 | grep PARTITION_CONFIG      # 0x50
reboot
```

Expected, with no hand on the board:

```
U-Boot SPL 2024.04-7.8.0+git.b7c747d8c084 ... Primary set selected / Load image from MMC/SD 0xc3400
NOTICE:  BL31: v2.10.0 (release):lf-6.6.52-2.2.1-dirty ... (OP-TEE's lines)
*** Booting Zephyr OS build ... ***                        <- the loader, entered by BL31
verdin_imx95: WDOG3 armed: ... ; board: watchdog armed
boot_stage2: count: eMMC mid 0x13, PARTITION_CONFIG 0x50, boot partitions 64512 sectors, ...
boot_stage2: count: boot1 sector 64494: 1 of 1 left
boot_stage2: count: 0 left after this start (written, read back); starting the payload
... imx95_wdog's lines; 60 s; WDOG3's reset ...
U-Boot SPL ... NOTICE:  BL31 ... (boot1 again) ... the loader
boot_stage2: count: eMMC mid 0x13, PARTITION_CONFIG 0x50, ...
boot_stage2: count: boot1 sector 64494: 0 of 1 left
boot_stage2: count: no starts left: PARTITION_CONFIG 0x50 -> 0x48 (boot0, read back)
verdin_imx95: boot_stage2: count: the next start is boot0: PSCI SYSTEM_RESET at ... ms
U-Boot SPL ... U-Boot ... Linux                            <- boot0
```

In Linux, `PARTITION_CONFIG` reads 0x48 again. What each failure costs is
the table in section 5, "The order of safety". After steps 3 and 4 the
stranding rows left are:

- the ROM or SPL with boot1's image set (open point 4; no SPL line, or
  SPL's lines and then nothing);
- the loader not starting from BL31 (BL31's lines and then nothing; open
  point 9, ruled out first by D4 when the cable is there).

Each costs one USB recovery (D6). After a run that came back by itself, the
next counted run is a new record (`count --remaining N`, step 2's `dd`) and
`mmc bootpart enable 2 1 /dev/mmcblk0`: boot1's image stays where it is.

**What still needs a person:**

- connecting the USB recovery cable and providing `uuu` before step 6;
- the recovery button if step 6 strands (D6);
- a power cycle if the loader hangs before its WDOG3 line in step 3 or 4,
  unless the farm can power-cycle this board.
