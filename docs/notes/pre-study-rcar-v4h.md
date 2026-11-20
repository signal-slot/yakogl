<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Pre-study: Renesas R-Car V4H (R8A779G0) on the Sparrow Hawk board

Date 2026-10-29, main `f157ae15`. Host research only: no board was
touched, and this board is not a farm resource yet. Board access is a
separate, later step. Everything below was read from public sources at
named versions, listed in the table. The claim sources are marked with
a short prefix and defined in that table.

Update 2026-11-20: section 9 has the host answers to U1 to U6, U9
and U11. The core is BVNC 30.3.408.101, of the DDK's Volcanic
architecture, not a Rogue one. Verdict lines 2, 5 and 6 below and
sections 3.2 and 8 were written before that and are kept as written.
The port is on hold since 2026-11-20 (section 10): with public
sources only, the GPU cannot be made to draw.

## Verdict in six lines

1. The board and SoC are pinned: the Sparrow Hawk board is the R-Car
   V4H reference board, its part is R8A779G3, the revision 3.0 of
   R8A779G0 (Z: board doc note; L: `r8a779g3.dtsi` header). Zephyr
   already has the board, and only for the Cortex-R52 core (Z:
   `boards/retronix/sparrowhawk_rcar_v4h/`, arch arm).
2. The GPU core is the open question, and it is a naming conflict,
   not a blank: the board documents and the vendor image's own `clinfo`
   name an "IMG A-Series AXM-8-256" (D:), while the task named
   BXS-4-64, which is the AM62P core (M:, L: binding). No open tree —
   Mesa, linux-firmware, the kernel bindings, mainline `r8a779g0.dtsi`,
   the BSP kernel — has any Renesas Gen4 GPU entry. Day one pins it
   with a PBVNC register read-back.
3. The open-source `powervr` kernel driver and Mesa `imagination`
   driver are Rogue-generic: one Rogue architecture for all their
   BVNCs (M: `vulkan/meson.build:143`). Renesas GPUs in mainline DT
   are Gen3 only (M3 family, V3U); V4H is absent (L:, B:).
4. The BSP runs the V4H GPU through an out-of-tree IMG DDK kernel
   module built from a Renesas tarball whose build layout is the rogue
   DDK layout (`build/linux/r8a779g_linux`, working dir named
   `rogue`), plus a closed user module; no kernel has a V4H GPU DT
   node (R: recipes, B:).
5. Our `src/pvr` + `driver/src/pvr_*` back end is Rogue-generic at the
   packet level (Mesa's `csbgen/rogue` headers) and firmware-interface
   generic (DDK `RGXProcessFWImage`), but keyed to one BVNC today,
   36.53.104.796. A second Rogue-family BVNC extends it (device-info,
   USC config, core parameters); the display side (DU, VSP, DSC, DSI)
   is new code, and the Cortex-R52 route is AArch32 while our Rust
   drivers crate targets AArch64 only today (repo:).
6. The port is sized like the AM62P port (the larger of the three
   done), with the Cortex-R52 toolchain gap and the new display path
   as the extra cost; Mali's still-open 126/256 is the caution against
   optimistic sizing (repo: `tools/cts/STATUS.md`).

## What was read, and what was not

Prefixes used below. `L:` and `B:` line numbers are exact at the named
tree state.

| Prefix | What, at what version |
| --- | --- |
| `L:` | mainline Linux, `git://git.kernel.org` master at commit `65ba8f34ec886ec72b2c07ddc8cf3d815c32869d`, `Makefile` VERSION 7 PATCHLEVEL 3 (7.3.0) |
| `B:` | `github.com/rcar-community/linux`, branch `renesas-lts/v6.18.59-2026-10-27-sparrow-hawk` (the BSP kernel; read with `git grep` against that branch, files not checked out) |
| `FW:` | `linux-firmware`, `git.kernel.org` repo, branch `main`, HEAD `d0039fd` |
| `M:` | Mesa 26.2.1, tag `mesa-26.2.1`, commit `da14d65e4499e66468094be52bff9ea0915a695e` (`third_party/mesa-26.2.1/UPSTREAM`); read from the unpacked upstream tarball — the vendored copy holds only `src/imagination/{common,csbgen,include,vulkan}` and `docs/license.rst`, so `docs/drivers/powervr.rst` and `src/imagination/ci/` are cited from the tarball, not from the tree |
| `Z:` | Zephyr at the workspace pin `af17c0c8aa2a82a0b714a423915ad3610063ee68` (4.4.99) |
| `D:` | the Sparrow Hawk pages `https://rcar-community.github.io/Sparrow-Hawk/index.html` and `.../BSP/yocto_bsp.html`, read 2026-10-29 |
| `R:` | `github.com/rcar-community` repos: `meta-sparrow-hawk` (branch `scarthgap`), `rcar-gfx` (branch `main`), file listings and files read through the GitHub API |
| `W:` | Wikipedia, "PowerVR" article, read 2026-10-29 |
| `REN:` | `renesas.com` R-Car V4H Group pages, read 2026-10-29 |
| `repo:` | files in this repository at `1491c87f` |

Every `L:` line quoted here was re-read from the tree in a second pass
after the first reading; the `FW:`, `R:` and `Z:` quotations were each
taken once, directly, and are quoted from that reading.

What was **not** read, and stayed out of the claims:

- The R-Car V4H hardware manual. The product page links "R-Car V4H HW
  User's Manual (R19UH0186EJ)" (REN:) but the document needs a Renesas
  account; nothing of it was read. What the SoC's GPU needs from the
  manual is named as questions in section 7.
- Renesas's own marketing wording for the V4H GPU. The product page
  rendered to the fetcher without a GPU sentence; the search engines
  reachable from this host (one instance) returned nothing usable.
- The contents of the two BSP graphics tarballs (`rcar-gfx`, section
  2.4): the 2 MB kernel-module tarball and the 22 MB user-module
  tarball were seen as file names and sizes only. Reading the smaller
  one is host item H2.
- `docs.imgtec.com` (named by M: `docs/drivers/powervr.rst:63`): not
  readable from this host.
- lore.kernel.org (403 from this host): the 2024 series "gpu:
  powervr-rogue: Add PowerVR support for some Renesas devices" named
  in the task was not read as a mailing-list thread. What landed of it
  is read from the mainline tree instead (section 2.1).
- The V4H GPU's register base, size and interrupt: no DT node carries
  them (sections 1.2, 2.1). They stay unknown (U5).

## 1. The hardware

### 1.1 SoC and board

- R8A779G0 is the R-Car V4H SoC; R8A779G3 is its revision 3.0 part
  (Z: board doc, "r8a779g3 is the part number of the revision 3.0 of
  V4H SoC (R8A779G0)"; L: `arch/arm64/boot/dts/renesas/r8a779g3.dtsi`
  header, "Device Tree Source for the R-Car V4H (R8A779G3) SoC",
  compatibles `"renesas,r8a779g3"`, `"renesas,r8a779g0"`).
- CPU side in mainline: four Cortex-A76 cores, `arm,cortex-a76` at
  `r8a779g0.dtsi:90,102,114,126`, `arm,cortex-a76-pmu` :196. No
  Cortex-R52 node in mainline. The board page lists "4x Arm Cortex-A76,
  3x Arm Cortex-R52" (D:), and Zephyr's board is the Cortex-R52 one
  (section 4).
- Memory and storage per the board page: 8 or 16 GB LPDDR5, 64 MB
  QSPI (D:); the U-Boot boot log recorded in the Zephyr board doc says
  "DRAM: 2 GiB" and finds the environment in an SPI-NOR
  `w77q51nw` 64 MiB (Z: board doc :84-105).
- GIC in mainline: `gic: interrupt-controller@f1000000`, two register
  windows `0xf1000000 0x20000` and `0xf1060000 0x110000`, `arm,gic-v3`
  (L: `r8a779g0.dtsi:2152-2160`). An external interrupt controller
  `intc_ex: interrupt-controller@e61c0000` exists at :502 (it serves
  the board's bridge interrupts, section 1.3). Zephyr's R52 dtsi names
  a different GIC window pair, `interrupt-controller@f0000000`
  `0xf0000000 0x1000` + `0xf0100000 0x20000` (Z:
  `dts/arm/renesas/rcar/gen4/rcar_gen4_v4h_cr52.dtsi:41-48`); see U8.
- IPMMU units serve peripherals in mainline (peripheral nodes carry
  `iommus = <&ipmmu_hc 0/1/2>` at L: `r8a779g0.dtsi:1029,1075,1121`);
  the BSP kernel config enables `CONFIG_IPMMU_VMSA=y`
  (R: `sparrow_hawk.cfg`).

### 1.2 The GPU core and revision — a naming conflict to pin, not a blank

Four independent public statements name two different cores:

| Source | Statement |
| --- | --- |
| task statement | "Renesas lists an Imagination PowerVR GPU for the V4H; the published core name is BXS-4-64" |
| board page (D:) | the board's GPU is "IMG A-Series AXM-8-256" |
| vendor image (D:, weston image page, `clinfo` output quoted there) | "Platform Name: PowerVR / Device Name: PowerVR A-Series AXM-8-256" |
| Wikipedia (W:) | "IMG AXM-8-256" is an IMG A-Series (Albiorix) model, 1 SPU, 8 RACs, 256 SIMD lanes, dated December 2019; BXS-4-64 is a B-Series part (MC1..MC4 rows). The article notes the A-Series is "no longer called PowerVR, they are called IMG" |

What the open trees say about both names:

- BXS-4-64 = 36.53.104.796 is the AM62P core: Mesa lists it as the one
  conformant product (M: `docs/drivers/powervr.rst:7,20`), the kernel
  binding pairs it with `ti,am62p-gpu` and `ti,j721s2-gpu` (L:
  `Documentation/devicetree/bindings/gpu/img,powervr-rogue.yaml:51-54`),
  and our back end is built on it (section 3). The name is real; its
  published home is TI's boards, not Renesas's.
- AXM-8-256 has no entry in Mesa's hand-maintained device-info table:
  the table is 14 BVNCs, `4.40.2.51` through `36.56.104.183`, and its
  device_info headers are named `g6110 gx6250 gx6650 ge7800 ge8300
  axe-1-16m bxe-2-32 bxe-4-32 bxm-4-64 bxs-4-64` (M:
  `src/imagination/common/pvr_device_info.c:34-43,47-64`).
- `grep -rn pvr_drm_configs src/imagination` over Mesa 26.2.1 returns
  nothing: the pairing file named in the task does not exist in this
  Mesa version. (Correction of the task statement, as in
  `docs/notes/pre-study-rpi5.md` for `v3d_packet_v71.xml`.)
- No Renesas Gen4 GPU node exists in any kernel: mainline
  `r8a779g0.dtsi` has no `gpu@` node and no GPU-compatible string, and
  the BSP kernel's Renesas device trees have `gpu@` nodes only for the
  RZ-line SoCs, and those are Mali Bifrost (`gpu@` nodes with
  `arm,mali-bifrost` at B: `r9a07g044.dtsi:1135`, `r9a09g057.dtsi:880`),
  not PowerVR.
- Renesas's own public pages readable from this host (REN:) did not
  display a GPU core name to us; the claim "Renesas published
  BXS-4-64 for the V4H" could not be confirmed, and the two
  Renesas-community sources say AXM-8-256. We do not adopt either
  name as fact.

How it gets pinned, in this order:

1. Read the packed BVNC from the core itself on the board (our
   bring-up already does exactly this read for the AM62P: `repo:
   driver/src/pvr_bringup.c:167-171`, register offset `0x0020`). The
   BVNC is the identity; the marketing name follows from it.
2. Read the BSP kernel-module tarball's device definitions (H2): the
   tarball Renesas builds the V4H kernel module from must contain the
   core's hardware definition, and it is the closest public artifact
   to Renesas's own data.

### 1.3 Display path

The SoC side, from mainline `r8a779g0.dtsi` (all lines cited):

- DU display unit: `display@feb00000`, `renesas,du-r8a779g0`, size
  `0x40000`, IRQs `GIC_SPI 523,524`, clock `CPG_MOD 411`, reset 411,
  power domain `R8A779G0_PD_ALWAYS_ON`, `renesas,vsps = <&vspd0 0>,
  <&vspd1 0>`; its two output ports go to DSI0 and, through the DSC
  bridge, to DSI1 (:2306-2338).
- VSP compositors: `vspd0: vsp@fea20000` (IRQ 546, CPG_MOD 830,
  always-on, with FCP) and `vspd1: vsp@fea28000` (IRQ 551, CPG_MOD
  831) at :2262-2282; two more VSPs for vision, `vspx0 vsp@fedd0000`
  and `vspx1 vsp@fedd8000` (:2284-2304).
- DSC bridge: `bridge@feb8d000`, `renesas,r8a779g0-dsc`, IRQ 559,
  CPG_MOD 2819, always-on (:2340-2347).
- DSI transmitters: `dsi0: dsi@fed80000` (CPG_MOD 415 + `CLK_DSIEXT`
  + `CLK_DSIREF`, always-on) and `dsi1: dsi@fed90000` (CPG_MOD 416,
  same core clocks) at :2551-2589.
- There is no HDMI or LVDS node for V4H in mainline: a case-insensitive
  search for `hdmi` and `lvds` in `r8a779g0.dtsi` and `r8a779g3.dtsi`
  returns nothing.

The board side (L: `r8a779g3-sparrow-hawk.dts`):

- The DP connector is a bridge chip, not SoC output: `&du`, `&dsc` and
  `&dsi1` are enabled (:292-311), DSI1 goes to a
  `ti,sn65dsi86` bridge at `i2c0_mux0 bridge@2c` (block :393-429;
  interrupt via `&intc_ex 0` at :403, `enable-gpios = <&gpio2 1 ...>`
  at :405, its reference clock `sn65dsi86_refclk` defined at :180),
  and the bridge drives a `dp-connector` labeled **CN6** (:135),
  "full-size" (connector block :133-141, endpoint wiring :418 and :425).
- Overlays in mainline define the other panel routes:
  `r8a779g3-sparrow-hawk-olimex-dsi-hdmi.dtso` (DSI0, 2 lanes →
  `lontium,lt8912b` bridge → `hdmi-connector HDMI1`),
  `r8a779g3-sparrow-hawk-ws-display-13in.dtso` (`waveshare,dsi2dpi`
  bridge + `ilitek,ili251x` panel), plus 2.5-inch and 2.7-inch RPI
  display overlays (directory listing). The BSP kernel config enables `ILI9881C` panel and
  `LT8912B` bridge (R: `sparrow_hawk.cfg`).
- So a frame goes: GPU writes a buffer in DRAM → DU (with VSP blend)
  → DSC if the DP route → DSI → on-board bridge (SN65DSI86 for CN6
  DP, LT8912B for the HDMI overlay) → panel. The GPU never drives the
  connector itself.

### 1.4 Boot chain

From the board and BSP docs:

- Recovery/flashing is ipl-burning (`ipl-burning.zip`, D: board page;
  Z: board doc :74-78 cites the BSP section "Flashing loader").
- The loader lives in SPI-NOR/QSPI: the U-Boot SPL boot log recorded in
  the Zephyr board doc shows SPL `2025.07`, "Trying to boot from SPI",
  "R8A779G0 rev 3.0", environment in the 64 MiB SPI-NOR (Z: :84-105).
  The BSP page documents the loader update commands
  `run update_loader_from_mmc/usb/nvme` and the manual fallback
  `load mmc 0:1 ${loadaddr} flash.bin && sf probe && sf update
  ${loadaddr} 0 ${filesize} && reset` (D: yocto_bsp page).
- Boot stages named on the BSP page: Arm Trusted Firmware 2.15.0,
  U-Boot 2026.10, Linux 6.18.5x, Yocto scarthgap (D:). The Linux board
  device tree reserves `tfa@40000000` size `0x8000000` `no-map` for
  TF-A (L: `r8a779g3-sparrow-hawk.dts:126-128`). What exactly runs
  between the SoC's on-chip boot ROM and U-Boot (secure monitor /
  BL2 contents) is not public in what we read (REN:, D:).
- The board page states "U-Boot has been updated to fix the problem
  that Zephyr does not run" (D:): the community U-Boot is the loader
  Zephyr is exercised against today. Zephyr's own board doc shows the
  supported route: load our image and start it through U-Boot
  remoteproc, `tftp 0x40040000 zephyr.bin; rproc init; rproc load 0:3
  0x40040000 0x200000; rproc start 0` (Z: :109-135).
- Today's BSP graphics path runs under Linux (section 2.4). A direct
  boot (no U-Boot proper) is our house route on AM62P and i.MX8MP
  (`docs/notes/am62p-direct-boot.md`, `docs/notes/imx8mp-direct-boot.md`) but is
  not documented for this board, and the community-U-Boot remoteproc
  route is the sign-off route here (section 4.6).

### 1.5 What has public documentation

Read: the two boards'/BSP's doc sites (`D:`), the mainline device
trees and drivers (`L:`), the BSP kernel tree (`B:`), the BSP layer
recipes and the gfx repo listing (`R:`), linux-firmware (`FW:`), Mesa
26.2.1 (`M:`), Zephyr (`Z:`), Wikipedia (`W:`).
Not read, as said above: the R-Car V4H HW User's Manual R19UH0186EJ
(login wall), Renesas press/product GPU wording, the two gfxdrv
tarball contents, `docs.imgtec.com`, and the mailing-list threads of
the 2024 PowerVR-Renesas series (403 from this host).

## 2. The open-source stack today

### 2.1 Kernel driver: `drivers/gpu/drm/imagination` (powervr)

- Device table, at mainline 7.3.0, has three compatible entries:
  `thead,th1520-gpu`, `img,img-rogue`, and the legacy `img,img-axe`
  (L: `pvr_drv.c:1495-1516`). A SoC-specific name like
  `renesas,r8a779a0-gpu` matches through the generic `img,img-rogue`
  fallback. `git grep -i renesas drivers/gpu/drm/imagination` at
  7.3.0 returns nothing: the driver itself is SoC-agnostic and names
  no Renesas part.
- The binding lists what Renesas parts are covered, and Gen4 is not
  among them: `renesas,r8a7796-gpu`/`renesas,r8a77961-gpu` →
  `img,img-gx6250`; `renesas,r8a77965-gpu`/`renesas,r8a779a0-gpu` →
  `img,img-ge7800`; plus `ti,am62-gpu` (`img,img-axe-1-16m`),
  `spacemit,k1/k3`, `thead,th1520`, `ti,am62p-gpu`/`ti,j721s2-gpu`
  (`img,img-bxs-4-64`) (L: `img,powervr-rogue.yaml:15-61`).
- The mainline device trees carry those nodes for the M3 family and
  V3U only: `r8a77960.dtsi:2580-2581`, `r8a77961.dtsi:2460-2461`,
  `r8a77965.dtsi:2455-2456` (gx6250/ge7800), and V3U
  `r8a779a0.dtsi:2340-2354`: `gpu@fd000000`, 256 KiB, one SPI
  interrupt, clocks `ZG`/`S3D1`/`CPG_MOD 0` named core/mem/sys, power
  domains `R8A779A0_PD_3DG_A` + `R8A779A0_PD_3DG_B` named a/b, reset
  `<&cpg 0>`, `status = "disabled"`. This node is the closest public
  blueprint for what a V4H GPU node must say, and it is the pattern
  our Zephyr-side bring-up follows (repo: `pvr_bringup.c` DT compat
  `img,img-rogue`, :25).
- What landed of the 2024 "Add PowerVR support for some Renesas
  devices" series is exactly the above as far as mainline 7.3.0 shows
  it; the threads themselves were not reachable here (lore 403), so we
  cite the landed tree, not the series.
- Device capabilities in the kernel driver are not a fixed table: the
  features, quirks and enhancements flags are copied from bitmasks
  inside the firmware image (`pvr_device_info_set_quirks` from
  firmware device information, `pvr_device_info_set_enhancements`, and
  the `FEATURE_MAPPING(...)` table maps firmware feature bits to
  driver flags) (L: `pvr_device_info.c:84-111`; the feature table
  `FEATURE_MAPPING` at :113-199). The driver therefore
  adapts to an unlisted Rogue BVNC to the extent the firmware tells it
  — a useful property for the V4H case.
- The BSP kernel (B:) builds the driver as a module
  (`CONFIG_DRM_POWERVR=m`, `arch/arm64/configs/defconfig:989`), but
  its Renesas Gen4 device trees still have no GPU node, and the BSP
  graphics stack does not use that driver (section 2.4).

### 2.2 Firmware: what the GPU needs and where it comes from

- The kernel driver asks the firmware loader for one file, named from
  the read-back BVNC: prefix `powervr/rogue`, major version 1
  (`PVR_FW_VERSION_MAJOR 1` at L: `pvr_device.c:37`;
  `pvr_request_firmware()` at :372 builds
  `powervr/rogue_<b>.<v>.<n>.<c>_v1.fw` at :379-380 and calls
  `request_firmware()` at :388).
- Upstream `linux-firmware` carries exactly three PowerVR Rogue
  firmware files, at FW: HEAD:
  `powervr/rogue_33.15.11.3_v1.fw` (AM62, AXE-1-16M),
  `powervr/rogue_36.52.104.182_v1.fw` (BXM-4-64),
  `powervr/rogue_36.53.104.796_v1.fw` (BXS-4-64, the AM62P file);
  license entry: "Redistributable. See LICENSE.powervr", version
  `1.1.OS@6976702` (FW: `WHENCE:10470-10478`). There is no file for
  any 30.x or other BVNC: if the V4H core is not one of these three
  BVNCs, its firmware is not in upstream linux-firmware.
- Mesa's own CI pulls the firmware as a `powervr-<tag>.tar` and keeps
  `./lib/firmware/powervr/*` for its tests (M:
  `src/imagination/ci/gitlab-ci-inc.yml:50-56`), and Mesa's doc points
  at Imagination's linux-firmware fork (M: `docs/drivers/powervr.rst:56`).
- The BSP's closed user-module tarball installs its own files under
  `/usr/lib/firmware/` (R: `gles-user-module.bb`, install lines), so
  the vendor stack ships its own firmware blobs; their names are
  unknown until H3. Our rule stands: firmware from a vendor tarball
  is used on the board, never committed (repo: `CHANGELOG.md:97-98`,
  `driver/tools/mkfwblob.sh`).

### 2.3 Mesa: Vulkan only, one Rogue architecture

- Mesa's `imagination` driver is Vulkan, not GL (M:
  `docs/drivers/powervr.rst:4-5`): "an open source Vulkan driver ...
  starting with those based on the Rogue architecture". It is Vulkan
  1.2 conformant on BXS-4-64 only; active development covers
  AXE-1-16M 33.15.11.3, BXM-4-64 36.52.104.182 and 36.56.104.183,
  BXS-4-64 36.53.104.796 (:7; active-development table :17-21). GX6250
  4.40.2.51 is partially supported (:29); G6110, GX6250 4.45/4.46,
  GE7800 15.5.1.64, GE8300 22.67/22.68/22.102.54.38, BXE-2-32 and
  BXE-4-32 are device-info-only (:42-51). There is no GLES path: ES 2.0 completeness does not come
  from Mesa for any board — it never has, on AM62P either (repo:
  `src/pvr`, our own GL layer).
- One architecture for all of them: the Vulkan driver compiles its
  `pvr_arch_*.c` files once per architecture and the architecture list
  is `['rogue']` for every supported BVNC (M: `vulkan/meson.build:143`).
  `csbgen`
  holds `rogue/` and `volcanic/` XMLs; the build consumes `rogue/`
  (repo: the generated headers `driver/src/pvr/generated/csbgen/rogue/`
  are produced with Mesa's `gen_pack_header.py` exactly as recorded in
  `third_party/mesa-26.2.1/UPSTREAM:54-59`).
- What is per-BVNC in Mesa is the hand-maintained device-info block
  (features/enhancements/quirks structs per BVNC; for
  36.53.104.796 the enhancements struct starts at M:
  `common/device_info/bxs-4-64.h:115`, the quirks struct at :123, both
  wired into the device-info at :140-141; the file even carries a
  note that the table is hand-maintained and auto-generation from the
  hwdefs is the intention, `common/pvr_device_info.c:24-26`). No AXM
  entry exists, so a V4H core as AXM-8-256 is `-ENODEV` for Mesa's
  `pvr_device_info_init` (BVNC switch at :75-92).
- drm-shim exists for offline work: build with
  `-Dvulkan-drivers=imagination -Dtools=drm-shim`, LD_PRELOAD
  `libpowervr_noop_drm_shim.so`; it mocks BXS-4-64 36.53.104.796 by
  default, overridable per BVNC through `PVR_SHIM_DEVICE_BVNC` and the
  quirk/enhancement override variables (M: `docs/drivers/powervr.rst:99-122`,
  `src/imagination/drm-shim/pvr_noop.c:487`). This is the same
  reference method the Mali port used (repo: `docs/notes/mali-*.md`,
  for example the drm-shim reference dump named in
  `docs/notes/mali-loop-break.md`).

### 2.4 The BSP graphics stack (what the vendor actually ships)

From the BSP layer recipes (R:) and the gfx repo listing (R:):

- `recipes-graphics/gles-user-module/gles-user-module.bb`:
  "PowerVR GPU user module", `LICENSE = "CLOSED"`, fetched for
  `r8a779g3` from a gfx tarball, unpacks into a directory named
  **rogue**, installs `powervr.ini`, `usr/lib/*.so`,
  `usr/lib/firmware/*`, the Vulkan ICD JSON `powervr_icd.json`, the
  OpenCL `IMG.icd`, a `rc.pvr` service/init — and depends on
  `kernel-module-gles`.
- `recipes-kernel/kernel-module-gles/kernel-module-gles.bb`: "Kernel
  module of PowerVR GPU", `LICENSE = "GPL-2.0-only & MIT"`, source is
  the gfxdrv tarball, built for `r8a779g3` in
  `${S}/build/linux/r8a779g_linux` for target `aarch64`, installed as
  a kbuild module (comment names it `pvrsrvkm.ko`), with a shipped
  `blacklist.conf` that says `blacklist pvrsrvkm`.
- The gfx repo file names are `gfxdrv/GSX_KM_V4H_SparrowHawk.tar.bz2`
  (2,097,403 B) and
  `opengl/r8a779g3_linux_gsx_binaries_gles.tar.bz2` (21,966,659 B);
  the repo's top `License.txt` is the binary-use licence that forbids
  reverse engineering — see U11 for the licence tension with the
  recipe's GPL/MIT claim.
- The kernel config fragment `sparrow_hawk.cfg` enables `UIO=m`,
  `UIO_PDRV_GENIRQ=m`, `UIO_DMEM_GENIRQ=m`, `IPMMU_VMSA=y`, `FW_LOADER`
  and the panel/bridge drivers, and does not enable any in-tree DRM
  GPU for V4H: the userspace talks to the out-of-tree module, which is
  the IMG DDK pattern (UIO + kernel module), the same pattern our bare
  metal design replaces.
- BSP-side notes: only `rcar_gen4_pcie.bin` (PCIe PHY) is named as a
  closed firmware file on the BSP page (D:, R: `sparrow-hawk-fw.bb`);
  "core-image-minimal: GPU Not supported", "core-image-weston (BSP +
  3D Graphics): Supported" with `weston-simple-egl`,
  `glmark2-es2-wayland`, `vkcube` (D:) — so an ES 2.0 path exists on
  the board today, through the closed stack, and it is named
  "A-Series AXM-8-256" (section 1.2).
- BSP patches touching GPU-adjacent code: `linux-renesas` carries a
  pmdomain backport for r8a779g0, a clk r8a779g0-cpg-mssr backport, a
  "HACK drivers/gpu/drm/drm_file: Ignore flag checking" patch (the
  out-of-tree GPU module is its plausible reason), and a qos DT node
  patch (R: patch file names; contents not read).

### 2.5 What an ES 2.0 port relies on for ISA and command stream

The knowledge base an open port uses is: Mesa's Rogue packet XMLs
(csbgen), the Rogue kernel driver's firmware contract (`rogue_*.fw`,
firmware-carried device info), and Imagination's DDK (which we already
mirror at ABI level through `third_party/ti-img-rogue-ddk-25.2`,
whose vendored hwdefs hold only the `rogue` family —
`repo: third_party/ti-img-rogue-ddk-25.2/hwdefs/rogue`). TI's DDK and
the Renesas BSP module share the same family naming (TI's UPSTREAM
branch name is even a scarthgap BSP line too, `repo:
third_party/ti-img-rogue-ddk-25.2/UPSTREAM`; Renesas builds
`build/linux/r8a779g_linux` in a tree whose top dir is named
`rogue`). Whether "GSX" in the tarball names are a DDK build variant
or a different core family is answered by reading that tarball (H2,
U6). Hardware documentation itself (BVNC-level core manuals, ISA
detail) sits behind `docs.imgtec.com` (unreadable here) and, for the
SoC side, behind the Renesas manual.

## 3. What yakogl already has that applies

### 3.1 What the pvr back end is keyed to

- Bring-up: `repo: driver/src/pvr_bringup.c` binds DT
  `img,img-rogue` (:25), builds the expected PBVNC from the DDK
  `RGX_BVNC_KM_*` values for BXS-4-64 (:53-57), reads the packed BVNC
  back from core register offset `0x0020` and compares (:167-171),
  and takes its carveout from a DT memory-region (:62,:195).
- Firmware: `driver/src/pvr_fw_boot.c` runs the DDK
  `RGXProcessFWImage` entry (the vendored DDK ABI rows
  `tools/check.sh` rows "ddk_abi_check" :1882 and "rgxlayer_check"
  :1940 guard it); the firmware file itself comes from TI's SDK
  through `driver/tools/mkfwblob.sh` and is never committed
  (`CHANGELOG.md:97-98`).
- Packets: `driver/src/pvr/generated/csbgen/rogue/{cr,ipf,pbestate,
  pds,ppp,texstate,vdm}.h` generated from Mesa's `csbgen/rogue` XMLs
  (`third_party/mesa-26.2.1/UPSTREAM:54-59`).
- Shaders: `src/shaderc/online/` has the PowerVR USC back end
  (`usc-backend.md`); the host rows tie its blobs to
  `tools/pco_shc/usc_blobs.txt` (:2496) and `tests/reference/blobs.rs`
  (:3518), keyed to the AM62P BVNC.
- Design notes the code cites: `docs/notes/pvr-bringup-spec.md`,
  `docs/notes/pvr-render-pool.md`, `docs/notes/pvr-fence-wait.md`,
  `docs/notes/pvr-fw-window.md`, `docs/notes/usc-backend.md`.

### 3.2 What carries over, at register/packet/ISA level

Read from Mesa at 26.2.1, not from memory:

- Packet words: the Rogue control-state XMLs are one family for every
  BVNC Mesa drives, including 36.53.104.796 (M: `vulkan/meson.build:143`
  single `rogue` arch; §2.3). If the V4H read-back lands on a
  Rogue-family BVNC, our generated packet headers do not fork; what
  changes per BVNC is the device-info layer (the features/quirks/
  enhancement structs, §2.3) and the core parameters (tile sizes,
  store depths, unit counts — the struct read in
  M: `device_info/bxs-4-64.h:100-113`, which has 16 tile Y, 4 TPU
  instances, 256 unified store depth, 64 USC slots and so on).
- Registers: the CR offsets we use are the Rogue `cr` csbgen set; the
  PBVNC read at offset 0x0020 is the same read the kernel driver
  performs for every Rogue part. A Rogue-family V4H uses the same CR
  map; the base address does not come from any DT today (U5).
- Firmware interface: `RGXProcessFWImage` and the firmware-carried
  device-info bitmasks are DDK-family-generic (the kernel does the
  same read without any fixed BVNC table, §2.1). The BSP's own module
  is built from a tree whose top directory is `rogue` (§2.4), so the
  board's vendor stack speaks the same DDK family. The firmware *file*
  is BVNC-named: a new BVNC needs a new file, found through BSP names
  (H3, D1) — with our rule: use, never commit.
- Shader ISA: USC is the Rogue fragment/vertex unit for every BVNC in
  Mesa's active set including BXS-4-64; the USC blob harness is keyed
  per BVNC (rows above), so a second BVNC means a second USC
  configuration, not a new compiler back end — assuming the read-back
  is a Rogue-family BVNC. That assumption is the core of this port's
  risk, and it is checkable on day one.

### 3.3 What is new, and what does not carry

- Device info for a second BVNC: Mesa has no AXM entry (§1.2); if the
  read-back is an AXM-era BVNC, neither Mesa's table nor our USC/
  core-parameter configuration has values for it. The kernel's trick —
  read the capability/quirk masks from the firmware image — is the
  method to copy for a board-pinned core (L: §2.1).
- Display: nothing in our tree or in Zephyr's drivers covers Renesas
  DU/VSP/DSC/DSI (`git grep` over Zephyr `drivers/reset`,
  `drivers/power`, `drivers/display` returns no R-Car entries, Z:; our
  display code is per-board in `driver/`). The whole V4H scan-out
  path is new code (§1.3). Good news from mainline: on V4H every
  display block (DU, VSPD0/1, DSI0/1, DSC) sits in
  `R8A779G0_PD_ALWAYS_ON` (L: :2267,:2278,:2313,:2345,:2558,:2587),
  so no power-domain control is needed for the display path.
- The other back ends do not apply: v3d, vivante and mali are
  different cores and packet sets; the only relevant back end is
  `src/pvr`.
- A cross-check on "same family" that we should run on the host before
  the board: the BSP module's core naming. TI's DDK builds per-SoC
  directories exactly like Renesas's (`build/linux/r8a779g_linux` vs
  TI's per-SoC build dirs, §2.4/§2.5); H2 confirms or breaks the
  family claim from the tarball itself.

## 4. The board on Zephyr

### 4.1 SoC and board support

- Zephyr has the SoC family `soc/renesas/rcar/rcar_gen4` with three
  SoC choices: `SOC_R8A779F0_R52`, `SOC_R8A779F0_A55`, and
  `SOC_R8A779G0_R52` (Z: `rcar_gen4/Kconfig.soc:9,15,21`). For V4H
  there is a Cortex-R52 choice and **no A-core choice**: the A55
  choice is R8A779F0-only (Z: `rcar_gen4/Kconfig`, the A55 branch is
  gated on `SOC_R8A779F0_A55`, ARM64), while `SOC_R8A779G0_R52`
  selects `ARM` (AArch32), `CPU_CORTEX_R52`,
  `CLOCK_CONTROL_RCAR_CPG_MSSR`, `ARM_ARCH_TIMER`.
- The board exists: `boards/retronix/sparrowhawk_rcar_v4h`, identifier
  `sparrowhawk_rcar_v4h`, SoC `r8a779g0`, **arch arm**, supported
  `gpio`, `clock_control`, `uart` (Z: board yaml/board.yml); dts model
  "Retronix Sparrow Hawk on R-Car V4H", compatible
  `retronix,sparrowhawk-v4h-cr52`, console on HSCIF1 at 921600; the
  board was added in the 4.4 release notes (Z:
  `doc/releases/release-notes-4.4.rst:1032`). Flashing and boot
  procedures in the board doc are written against the community BSP
  (Z: :74-78, :84-135; U-Boot remoteproc route quoted in §1.4).

### 4.2 Cores, and the CPU choice for our image

- The R52 view (Zephyr today): `cortex-r52 cpu@0`, armv8-timer with
  four PPIs, `sram0: memory@40040000` size `0x400000` (4 MiB), GIC-v3
  pair at `0xf0000000`/`0xf0100000` (Z:
  `dts/arm/renesas/rcar/gen4/rcar_gen4_v4h_cr52.dtsi:19-48`; the
  GIC-window difference against mainline's `0xf1000000` pair is U8).
  `NUM_IRQS 991` (Z: `Kconfig.defconfig.r8a779g0:7`), system clock
  from `clock-cl16m_rt` at 16,660,000 Hz.
- The four Cortex-A76 cores are in mainline Linux (L: :90-126) and in
  the board page (D:), but have no Zephyr support for r8a779g0
  (§4.1). The 4 MiB R52 SRAM at 0x40040000 sits inside the region the
  Linux dts reserves for TF-A (`tfa@40000000 0x8000000 no-map`, L:
  §1.4): when Linux is not running, that space is ours; when a BSP
  image is booted, it belongs to TF-A.

### 4.3 GPU and display: what the Zephyr side will need

From mainline (the public statement of the hardware), §1.1/§1.3:
DU `feb00000` 0x40000 IRQ 523/524 CPG_MOD 411; VSPD0/1 `fea20000`/
`fea28000` IRQ 546/551 CPG_MOD 830/831; DSC `feb8d000` IRQ 559
CPG_MOD 2819; DSI0/1 `fed80000`/`fed90000` CPG_MOD 415/416 with core
clocks `CLK_DSIEXT`, `CLK_DSIREF`; all always-on. The GPU's own base,
size, interrupt and clock/power consumer list are not published by any
DT (§1.2, U5); the V3U node (§2.1) is the nearest blueprint (ZG core
clock, a mem and a sys clock, two 3DG domains).

The V4H clock/power tables that exist in mainline (and are what a
Zephyr extension mirrors):

- Core-clock ids in `include/dt-bindings/clock/r8a779g0-cpg-mssr.h`:
  `R8A779G0_CLK_S0D2_U3DG 16`, `S0D4_U3DG 17` (:28-29), `ZB3D2 45`,
  `ZB3D4 46`, `ZG 47` (:57-59), `DSIEXT 65`, `DSIREF 66` (:77-78).
  The Linux driver implements the U3DG pair as fixed clocks from
  `CLK_S0` with dividers 2 and 4 (`drivers/clk/renesas/
  r8a779g0-cpg-mssr.c:114-115`); its table shows no `zg` entry.
- Power domains `R8A779G0_PD_A33DGA 24`, `A23DGB 25` (:19-20); the
  driver lists them as `a33dga` (parent: always-on) and `a23dgb`
  (parent: a33dga) (`drivers/pmdomain/renesas/r8a779g0-sysc.c:23-24`).
  No mainline DT node consumes the 3DG-style domains on V4H, and
  whether the V4H GPU is behind them is a manual question (U5). The
  Gen3 V3U precedent (`PD_3DG_A`/`_B`, §2.1) and the "3DG" clock
  naming are suggestive; nothing more (U5).

The Zephyr side today (Z:):

- The CPG clock driver implements `S0D6_PER`, `S0D12_PER`, `CL16M`,
  `SASYNCPERD1`, `PLL5`, `SDSRC` and module ids 514, 515, 702, 915,
  916, 917; the Zephyr dt-binding header already declares the U3DG
  ids under Zephyr's own numbering (`S0D2_U3DG 24`, `S0D4_U3DG 25` at
  the header's :48-49 — Zephyr numbers differ from Linux's 16/17).
  U3DG, ZG, ZB3D, DSIEXT/DSIREF and the DU/VSP/DSC/DSI module ids are
  not implemented (H5).
- No R-Car driver in `drivers/reset`, `drivers/power`, `drivers/display` (grep, §3.3).

### 4.4 Toolchain: the AArch64 gap

- `repo: driver/rust/Cargo.toml:14` builds the drivers crate as a
  staticlib for `aarch64-unknown-none-softfloat`, and `repo:
  capi/yakogl_capi.cmake:122-123` selects the `_t aarch64-unknown-none`
  target for aarch64-none boards. The V4H route Zephyr supports today
  is Cortex-R52 = AArch32 (§4.1). The drivers crate has no AArch32
  path; either the Rust crate gains one or the board runs the C
  driver surface. Host item H6 settles which.
- An A76 (AArch64) route would reuse the current toolchain but needs a
  new Zephyr SoC definition (nothing exists for r8a779g0 A-cores,
  §4.1): upstream Zephyr work, not a repo item (U10).

### 4.5 Interrupts

GIC-v3 for the R52 view is at Zephyr's window (§4.2); the display
blocks' SPI numbers are known from mainline (§4.3) and 991 IRQs are
in range. The GPU's interrupt number is unknown (U5).

### 4.6 Boot route for sign-off

- Sign-off: the documented community route — flash.bin in SPI-NOR
  (SPL→TF-A→U-Boot), then U-Boot remoteproc loads and starts our
  image at 0x40040000 (Z: :109-135). "U-Boot has been updated to fix
  the problem that Zephyr does not run" (D:) makes this the supported
  vendor-side loader.
- Development: the OpenOCD config `boards/retronix/sparrowhawk_rcar_v4h/board.cmake` (Z:) and the loadb serial path (Z: :129-135) are the loop. A direct boot (no U-Boot proper) like AM62P's (`docs/notes/am62p-direct-boot.md`) is possible only against the SPL/TF-A stage, which is exactly the part we cannot read (§1.4): later, not now.

## 5. What can be done on the host before the board

Each item with its check. None needs the board.

| # | Item | Check |
| --- | --- | --- |
| H1 | drm-shim BVNC matrix: build Mesa 26.2.1 with `-Dvulkan-drivers=imagination -Dtools=drm-shim` (M: §2.3) and dump the compiler's output for every BVNC in Mesa's device-info headers through `PVR_SHIM_DEVICE_BVNC`; record how packets/USC words differ across 33.15.11.3, 36.52/53/56 | a host command sequence written to `tools/pvr/README.md` and, if kept, one new `tools/check.sh` row; the dumps land under `tools/pvr/` like the Mali method (repo: `docs/notes/mali-*.md`) |
| H2 | read the BSP kernel-module tarball `gfxdrv/GSX_KM_V4H_SparrowHawk.tar.bz2` (2,097,403 B, R:) **after** checking the licence position (U11): list its hwdefs/csbgen XMLs, its device-info, its firmware file names, and how its module finds the GPU without a DT node (U4) | the file list and the answers to U4/U6 recorded as a comment on the board issue; diff against Mesa's `csbgen/rogue` XMLs, byte names only — upstream code is read, not copied |
| H3 | list the closed user-module tarball's files (names, sizes, hashes only — the 21,966,659 B `opengl/...gsx_binaries_gles` tarball, R:): its `usr/lib/firmware/*` names answer "which firmware image does the vendor stack run" (U3) | file list in the issue; no contents into the tree, never |
| H4 | a build-not-run gate row: `zephyr_werror_image()` (repo: `tools/check.sh:3244`) building our sample app for `sparrowhawk_rcar_v4h` | the row passes; it is the model of the verdin_am62p rows (:3282-3311) with no board step |
| H5 | Zephyr-side definitions: extend the R-Car Gen4 CPG driver with the U3DG/ZG-pair ids and the DU(411), VSPD(830/831), DSI(415/416, DSIEXT, DSIREF), DSC(2819) module clocks, mirroring the Linux tables quoted in §4.3, plus a GPU/display skeleton dts for our board copy | dt bindings compile, H4 builds against the touched dtsi, and the id list matches the Linux header in a table checked into the commit message or the note update |
| H6 | the AArch32 decision: prototype building a slice of `driver/rust` (and the capi path) for the Cortex-R52 board target triple; pick Rust-with-new-target or C-for-this-board | a recorded decision + whichever build is picked becomes a build row like H4 |

## 6. What needs the board

Board access itself is a separate step (this board is not a farm
resource yet). Day one, in order:

| # | Try | Done when |
| --- | --- | --- |
| D1 | boot the community weston image (D: documents it as GPU-supported) and record: the firmware file the kernel module requests (`dmesg`), the device name/version from its own tooling (already quoted as A-Series AXM-8-256, §1.2), and whether a GPU device-tree node or platform device appears after the module loads (answers U4) | the strings and file names recorded on the issue (raw logs on the issue, not in the tree) |
| D2 | boot our own Zephyr image through the U-Boot remoteproc route (Z: §1.4): console on HSCIF1, memory map print, GIC bring-up, then map the GPU register window (base from D1) and read offset `0x0020` like `driver/src/pvr_bringup.c:167-171` does | a BVNC printed by our own code, in the issue and in the §1.2 table of this note (update commit) |
| D3 | run `pvr_fw_boot`'s entry with the firmware file D1 named (kept board-side only): firmware boot success, then read the firmware's device-info blob the way the kernel driver maps it (L: §2.1) | "firmware booted" + the feature/quirk bitmask dump recorded; the device-info blob becomes the input for our per-BVNC config |

Measurements that tell the port is alive, in the house order:

- M1 first clear: the GPU fills a buffer, DU scan-out shows it on CN6
  (needs the §1.3 display path, so after P8).
- M2 first triangle.
- M3 first `gl_test` case passing.
- M4 the `gl_test` case sweep count, recorded as a row in
  `tools/cts/RESULTS.md` (the AM62P reference: 49/50, the stale-case
  pattern noted in `tools/cts/STATUS.md`).

## 7. Risks and unknowns

Each stays a question. Nothing here is guessed.

| # | Question | What answers it |
| --- | --- | --- |
| U1 | Which core is it really: an AXM-8-256-era BVNC or the BXS-4-64 the task names? | board read-back D2; H2's device definitions **Host answer (2026-11-20, H2 and H3, section 9): BVNC 30.3.408.101.** The BSP kernel module is built for `RGX_BVNC := 30.3.408.101` and the BSP firmware files carry the same four numbers; the board's own read-back (D2) is still to come |
| U2 | Is that BVNC Rogue-family in the sense our code means: will the `csbgen/rogue` packet headers and the USC back end serve it? | D2+D3; Mesa's own answer for unlisted BVNCs is a refusal (§2.3), which tells us the table is not the limit of the ISA, but not what differs **Host answer (2026-11-20, section 9): no.** 30.3.408.101 is defined under the DDK's `hwdefs/volcanic`, not `hwdefs/rogue`: a second architecture with its own register map, device code and firmware interface, a META firmware processor and no public packet set for geometry |
| U3 | Which firmware image does the V4H core need, what is its file name, and under what licence? | D1 (`dmesg`), H3 file names; upstream `linux-firmware` has only the three §2.2 files, so a different BVNC means a vendor-supplied file **Host answer (2026-11-20, H3): `rgx.fw.30.3.408.101` (147,456 B) and `rgx.sh.30.3.408.101` (395,744 B)**, in the closed user-module tarball under the binary-use licence; not in linux-firmware |
| U4 | How does the BSP kernel module bind the GPU when no DT node names it? | H2 (read the module's device-code), or D1 (watch what appears after the module loads) **Host answer (2026-11-20, H2): through a device-tree node.** The BSP kernel has `gsx@fd000000`, compatible `renesas,gsx`, and the module's makefile sets `SYS_RGX_OF_COMPATIBLE` to that string (section 9.3) |
| U5 | GPU register base, size, interrupt, clock inputs and power domains on V4H | H2/D1 for the driver's answer; the R-Car manual (needs a Renesas account) for the hardware truth; U5 blocks our own DT skeleton until answered **Host answer (2026-11-20, the BSP device tree, section 9.3): base 0xfd000000, size 0x800000, `GIC_SPI 464`, clock `CPG_MOD 0` (set to 600 MHz by the module), power domain `R8A779G0_PD_A23DGB`, reset `<&cpg 0>`.** The manual is still unread |
| U6 | Does "GSX" name a DDK variant of the rogue family or a different core family? | H2 (the tarball's own build files answer it in one read) **Host answer (2026-11-20, H2): neither.** "GSX" is Renesas's name for the GPU block (`renesas,gsx`, `rgx_rcar`); the tarball is Imagination's `rogueddk` 25.1 tree, which holds both the `rogue` and the `volcanic` architecture, and the V4H build takes `volcanic` |
| U7 | Can the owner obtain R19UH0186EJ (and the GPU chapters) with a Renesas account? | the owner's Renesas account; until then sections 1.2/4.3 keep the manual-only facts open |
| U8 | Zephyr's GIC window (0xf0000000 pair) differs from mainline Linux's (0xf1000000 pair) — which is right for the R52 view? | the manual, or a board test (D2 exercises interrupts); a wrong window is a bring-up bug we want found on purpose, not by surprise |
| U9 | Does the Rogue firmware on V4H require the IPMMU (the BSP enables it; AM62P runs without an IOMMU in our design)? | H2 (what the vendor module sets up); else D3's boot success/failure **Host answer (2026-11-20, H2): the vendor stack does not put the GPU behind the IPMMU.** The BSP tree has an `ipmmu_3dg` unit, but the `gsx` node has no `iommus` property and `rgx_rcar/sysconfig.c` names no IOMMU |
| U10 | Is the Cortex-A76 route ever worth upstream Zephyr work, versus staying on Cortex-R52 + AArch32? | an upstream Zephyr SoC addition; decide after U1 pins the core; until then plan AArch32 |
| U11 | The `rcar-gfx` top `License.txt` is binary-use-only (no reverse engineering) while `kernel-module-gles.bb` declares `GPL-2.0-only & MIT` for the gfxdrv tarball. Which licence governs the file we read in H2? | the LICENSE files inside the tarball itself (H2's first step); the owner's legal call before any reading that the stricter label would forbid; nothing from a non-redistributable SDK enters the tree either way (task rule) **Host answer (2026-11-20, H2's first step): the kernel-module tarball is Dual MIT/GPLv2 by its own files.** Its `README`, `MIT-COPYING` and `GPL-COPYING` have the sha256 of the three files in `third_party/ti-img-rogue-ddk-25.2/UPSTREAM`; the binary-use `License.txt` is the one that covers the user-module tarball, of which only names, sizes and hashes were taken (section 9.1) |

## 8. Proposed items, ordered

Sizes against the three finished ports: AM62P (dEQP-GLES2 17,330
cases, glmark2 467), i.MX8MP (17,318, glmark2 42), Pi 4 (17,126,
glmark2 195) — repo: `tools/cts/STATUS.md`; the Mali board's still
open 126/256 `gl_test` sweep is the caution (same file).

| # | Item | Size | Depends on | Done when |
| --- | --- | --- | --- | --- |
| P0 | Pin the core: H2 (licence first), D1, D2 in that order; write the BVNC into §1.2 of this note in an update commit | S | board access for D1/D2 | one BVNC, one core name, in the note and the issue |
| P1 | H1 drm-shim matrix + the `tools/pvr/` reference dumps | S | P0 decides which BVNCs are worth a probe beyond Mesa's own list | rows recorded, dumps byte-stable |
| P2 | H3 firmware file names + the no-commit policy restated for this board | S | — | names and hashes in the issue; nothing in the tree |
| P3 | Zephyr prep: H5 (clock ids, display skeleton dts) + H4 (build-not-run row) | M | — | rows green against main tip |
| P4 | AArch32 toolchain decision + implementation (H6) | M | — | the drivers crate (or its C surface) builds for the board target in a gate row |
| P5 | Board bring-up through U-Boot remoteproc: our image on R52, GIC, carveout, PBVNC read-back (D2) | M | P3, P4, board access | BVNC read by our code; DT skeleton filled with the U5 answers |
| P6 | Firmware boot (D3): vendor-named file, `pvr_fw_boot` path, device-info blob read | M | P0, P2, P5 | firmware alive; blob dump recorded |
| P7 | Minimal TA kick: first command submitted and completed (the `pvr-bringup-spec.md` sequence, device-info values from P6) | M-L | P6 | first TA completion on the board |
| P8 | Scan-out: DU + VSPD + DSC + DSI + SN65DSI86 bring-up (new display path, §1.3; the mainline DU driver is the reading base) | L | P3, P5 | M1 first clear visible on CN6 |
| P9 | ES 2.0 core parity: bring-up spec, render pool, fence wait, USC config for the pinned BVNC, then `gl_test` cases and the CTS stages | L | P7, P8 | M2, M3, then M4 with a row in `tools/cts/RESULTS.md` |

Order logic: P0 is the gate on every "Rogue-family" assumption (P7,
P9); P3/P4 are the host-side long poles and can run beside P0-P2. If
P0 shows a non-Rogue family (U1 answers badly), the item list stops
after P0-P4 and the port is re-planned as a new back end — that is
why P0 sits first and alone.

## 9. Host answers, 2026-11-20 (H2 and H3)

Host reading only: no board was touched. Main `1d9022e7`. The sources,
with the same prefixes as above and three new states:

| Prefix | What, at what version |
| --- | --- |
| `G:` | `github.com/rcar-community/rcar-gfx`, branch `main`, commit `7ab92b1f0` (2026-10-06) |
| `KM:` | `G: gfxdrv/GSX_KM_V4H_SparrowHawk.tar.bz2`, 2,097,403 B, sha256 `92a2be149a900f06b94aecc12e4ec768eec2a231c31c0f5719245d4cc75fedf2`, 1,219 entries under `rogue_km/`; `include/pvrversion.h`: "PVR_DDK_Linux+Linux_WS rogueddk 25.1@6794074" |
| `UM:` | `G: opengl/r8a779g3_linux_gsx_binaries_gles.tar.bz2`, 21,966,659 B, sha256 `d1ca1d4c7504c89940c053bb64396eecc78179bcb0da7f64fc6d8f949c457980`; listed with `tar tjvf`, never unpacked |
| `B2:` | `github.com/rcar-community/linux`, branches `renesas-lts/v6.18.64-2026-11-18-sparrow-hawk` and `renesas-slts/v6.12.118-2026-11-19-sparrow-hawk` |
| `L2:` | mainline Linux master on 2026-11-20, `Makefile` 7.4.0-rc3 |
| `M2:` | Mesa `main` at `bf593eb0` (2026-11-20) |

### 9.1 Licence of what was read (U11)

- `KM: README` says the tree may be used under MIT or under GPL
  version 2. `README`, `MIT-COPYING` and `GPL-COPYING` have the sha256
  values `third_party/ti-img-rogue-ddk-25.2/UPSTREAM` lists for TI's
  copy of the same DDK, byte for byte.
- File headers in `KM:`: 974 "Dual MIT/GPLv2", 17 "Synopsys Permissive
  License", 1 "MIT", and 1 "Strictly Confidential"
  (`build/linux/common/window_systems/nullws_drm.mk`). That one file
  was not read.
- `G: License.txt` (binary form only, no reverse engineering, Renesas
  devices only) is the licence of the user-module tarball. Of `UM:`
  only the file names, sizes and the tarball's hash were taken.
- Nothing of either tarball is in this tree.

### 9.2 The core: 30.3.408.101, the Volcanic architecture (U1, U2, U6)

- `KM: build/linux/r8a779g_linux/Makefile` sets
  `RGX_BVNC := 30.3.408.101` and `PVR_SYSTEM := rgx_rcar`.
- The firmware files in `UM:` are `rogue/usr/lib/firmware/rgx.fw.30.3.408.101`
  (147,456 B) and `rgx.sh.30.3.408.101` (395,744 B): the DDK's
  `rgx.fw.B.V.N.C` naming (`KM: services/server/include/fwload.h:93`),
  the same four numbers.
- The DDK holds two architectures. `KM: hwdefs/rogue/km/` has 39 cores
  (36.53.104.796 among them) and `KM: hwdefs/volcanic/km/` has 19.
  30.3.408.101 is `hwdefs/volcanic/km/cores/rgxcore_km_30.3.408.101.h`
  with `configs/rgxconfig_km_30.V.408.101.h`; `build/linux/config/core.mk:263`
  branches on `PVR_ARCH` being `volcanic`.
- The configuration header, against the AM62P's
  (`third_party/ti-img-rogue-ddk-25.2/hwdefs/rogue/km/configs/rgxconfig_km_36.V.104.796.h`):

  | | V4H, 30.V.408.101 | AM62P, 36.V.104.796 |
  | --- | --- | --- |
  | top level | `ALBIORIX_TOP_INFRASTRUCTURE`, `S7_TOP_INFRASTRUCTURE` | `ROGUEXE`, `XE_ARCHITECTURE 1` |
  | firmware processor | `META (MTP219)`, core memory 96 in 8 banks | `RISCV_FW_PROCESSOR` |
  | MMU | `MMU_VERSION 4` | no version feature |
  | tile | 32 x 32 | 16 x 16 |
  | parameter format | no simple format; `SCALABLE_TE_ARCH 1`, `SCALABLE_VCE 1` | `SIMPLE_INTERNAL_PARAMETER_FORMAT` V2 |
  | physical bus | 40 bits | 36 bits |
  | compute stream format | 4 | 1 |
  | ISP/IPP pipes | 4 | 6 |
  | SLC | 128 KiB, 1024-bit lines | 16 KiB, 512-bit lines |

  The core header has `FIX_HW_BRN_71960`, `FIX_HW_BRN_72144`,
  `HW_ERN_65104`, `HW_ERN_69700`.
- The name: the board page and the vendor image say "A-Series
  AXM-8-256" (section 1.2); the DDK names no product in these headers.
  The BVNC is the identity, and it is not BXS-4-64's.
- How far the two architectures are apart in the kernel-side DDK:
  - registers (`rgx_cr_defs_km.h`, offsets only): 661 names in rogue,
    802 in volcanic, 364 names in both, 237 of them at the same
    offset. The packed BVNC is at 0x0020 in both (rogue
    `RGX_CR_CORE_ID__PBVNC`, volcanic `RGX_CR_CORE_ID`), so the D2
    read-back needs no change. `RGX_CR_FWCORE_BOOT` is 0x3090 against
    0x70c0; volcanic has no `RGX_CR_BIF_CAT_BASE0`;
  - device code: `services/server/devices/rogue` (15 files, 23,797
    lines) and `services/server/devices/volcanic` (15 files, 20,923
    lines) are separate; the other 66 entries of
    `services/server/devices` are common;
  - firmware interface: `include/rogue/rgx_fwif_km.h` and
    `include/volcanic/rgx_fwif_km.h` differ in 618 lines,
    `rgx_fwif_shared.h` in 371.
- Our firmware boot is the RISC-V one (`driver/src/pvr_fw_boot.c:344-360`).
  A META firmware processor boots another way.
- Mesa: `M2: src/imagination/csbgen/volcanic/` has `cdm`, `ipf`,
  `lls`, `pbestate`, `pds` and `texstate` XMLs (commit `384ce6d8`,
  2026-02-13, "pvr: add basic volcanic hw-definitions"; also in
  26.2.1). It has no `cr`, `vdm`, `ppp` or `kmd_stream`, which rogue
  has. No Mesa build uses them: `M2: src/imagination/vulkan/meson.build:147`
  still builds `['rogue']` only, and `common/device_info/` has no
  A-Series header besides `axe-1-16m.h`. So there is no public
  geometry packet set, no public driver and no public shader compiler
  for this core. Its shader instruction set was not looked at: no
  public source for it was found.

### 9.3 The GPU node (U4, U5, U9)

`B2: arch/arm64/boot/dts/renesas/r8a779g0.dtsi:2131-2138`, the same
lines in both branches (section 1.2 searched the BSP tree for `gpu@`
nodes; this node is named `gsx@`):

```
gsx: gsx@fd000000 {
	compatible = "renesas,gsx";
	reg = <0 0xfd000000 0 0x800000>;
	interrupts = <GIC_SPI 464 IRQ_TYPE_LEVEL_HIGH>;
	clocks = <&cpg CPG_MOD 0>;
	power-domains = <&sysc R8A779G0_PD_A23DGB>;
	resets = <&cpg 0>;
};
```

- The module's makefile has the same values (`RGX_3DGE_REG_BASE_ADDR
  := 0xFD000000`, `RGX_3DGE_IRQ_NUM := (464 + 32)`), a core clock of
  600,000,000 Hz, and `SYS_RGX_OF_COMPATIBLE := "renesas,gsx"`.
  `KM: services/system/rgx_rcar/sysconfig.c` takes one clock, sets
  that rate, and uses runtime PM for the power domain.
- `A23DGB`'s parent is `A33DGA` (section 4.3), so both 3DG domains
  must be on.
- `B2:` has `ipmmu_3dg: iommu@eee00000`, but the `gsx` node has no
  `iommus` property and `sysconfig.c` names no IOMMU.
- `L2:` `r8a779g0.dtsi` has no `gpu@` or `gsx@` node.

### 9.4 The R-Car V4M has the AM62P's core

`KM: build/linux/r8a779h_linux/Makefile` sets
`RGX_BVNC := 36.53.104.796`: the R-Car V4M (R8A779H0) is built for
exactly the BVNC our back end is keyed to (section 3.1), at the same
register base (0xFD000000, size 0x800000), interrupt (464) and clock
(600 MHz) as the V4H. "BXS-4-64 on a Renesas Gen4 part", which the
task named for the V4H, is true of the V4M. `B2:`'s `r8a779h0.dtsi`
has `ipmmu_3dg` and no `gsx` node; `L2:`'s has no GPU node. No V4M
board, firmware source or Zephyr support was looked at.

### 9.5 What this does to the item list

- P0 is answered from the host as far as it can be: one BVNC, and it
  is not a Rogue one. By section 8's own rule the list stops after
  P0 to P4. P5 to P9 as written assumed the AM62P back end with a
  second device-info block; that assumption is wrong for the V4H.
- A V4H port needs, new: the kernel side for the Volcanic register
  map, MMU version 4 and a META firmware boot; the geometry packets
  (no public definition); a shader back end for a core no public
  compiler targets. The DDK kernel source (MIT) is the only public
  reading base, and it covers the kernel side only.
- Unchanged: the display path (section 1.3), the Zephyr items H4 and
  H5, the AArch32 question H6, the boot route (section 4.6). H5's GPU
  part now has its values (section 9.3).
- H1 (the drm-shim matrix over Mesa's Rogue BVNCs) tells nothing
  about this core.
- Still to be read on a board: the BVNC by our own code (D2).


## 10. The plan after section 9 (2026-11-20)

**On hold (the owner, 2026-11-20).** With public sources only the
V4H's GPU cannot be made to draw: the geometry stream and the shader
instruction set have no public definition (the table below). No item
of this section is started. The port is taken up again when one of
these happens: Mesa gains the rest of the Volcanic definitions and a
compiler target for it; Imagination or Renesas give documentation
that may be used in Apache-2.0 code; Renesas permit, in writing, what
`G: License.txt` forbids.

The owner's two decisions of 2026-11-20 that lead there:

- The board is the V4H (Sparrow Hawk). The V4M of section 9.4 is not
  available and is not planned.
- Public sources only. Watching what the vendor's closed stack makes
  (its control streams, its shader output, its calls into the kernel
  module) is reverse engineering under `G: License.txt` and is not
  done. The firmware files are run on the board as they are and never
  committed (section 2.2).

What the public sources cover, and where they stop:

| Part | Public source | State |
| --- | --- | --- |
| Clock, power domain, reset, register window, interrupt | `B2:` device tree, `KM: services/system/rgx_rcar` | complete (section 9.3) |
| Volcanic registers, MMU version 4, META firmware boot, firmware interface, kernel-side job submission | `KM:` (Dual MIT/GPLv2): `hwdefs/volcanic`, `include/volcanic`, `services/server/devices/volcanic` | complete for the kernel side |
| PDS, PBE state, texture state, IPF, LLS, compute stream words | `M2: csbgen/volcanic/` (661 lines of XML; rogue's set is 5,393) | partial: Mesa merge request 39526 (Erik Faye-Lund, reviewed by Imagination, merged 2026-02-13) says it "isn't complete enough to drive the GPU, but gets us a step on the way" |
| Geometry stream (rogue's `vdm`, `ppp`), `cr`, `kmd_stream` | none | missing |
| Shader instruction set and a compiler for it | none (`M2: pco` is built for rogue only) | missing |
| Display (DU, VSP, DSC, DSI, SN65DSI86) | mainline drivers (section 1.3) | complete as a reading base |

Items for when the port is taken up again, replacing P5 to P9 of
section 8. P1 is dropped (section 9.5);
P2 is done (section 9.2); P3 and P4 stand as written.

| # | Item | Size | Depends on | Done when |
| --- | --- | --- | --- | --- |
| V1 | P3 with the GPU values of section 9.3 in the skeleton dts | M | — | the build-not-run row is green |
| V2 | P4, the AArch32 decision | M | — | the drivers build for the board target in a gate row |
| V3 | Our image on the Cortex-R52 through U-Boot remoteproc; A33DGA and A23DGB on, `CPG_MOD 0` at 600 MHz, reset released; the BVNC read at 0x0020 | M | V1, V2, the board | 30.3.408.101 printed by our own code (D2) |
| V4 | Scan-out with a buffer the CPU fills: DU, VSPD, DSC, DSI1, SN65DSI86 | L | V3 | a picture on CN6. It needs nothing of the GPU |
| V5 | The Volcanic kernel side from `KM:`: MMU version 4 tables, the META firmware boot, the firmware's init and its trace | L | V3 | the firmware answers its first command on the board |
| V6 | Geometry, shaders, ES 2.0 | — | a public definition of the geometry stream and of the shader instruction set | not planned until that exists |

V6 is a stop, not an item, and it is why the port is on hold: V1 to
V5 give the board on Zephyr, its display and a GPU whose firmware
runs, and no OpenGL ES. The follow-ups of merge request 39526 are
the thing to watch (none had landed in `M2:` by 2026-11-20).
