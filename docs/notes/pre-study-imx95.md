<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Pre-study: NXP i.MX 95 (Mali-G310) for the Mali back end

Date 2026-11-20, main `1d9022e7`. This is host research only: no board was
touched, and which i.MX 95 board we get is not fixed. It covers the SoC
fully and compares the candidate boards. It was written on a host whose
session refused every network route (git over HTTPS, curl, web fetch, web
search). So it rests on the sources this host holds at pinned versions,
listed in the table below. Everything that needed the network is named
under "not read", with the item that reads it (H0).

## Verdict in six lines

1. The GPU is the Mali-G310 in Mesa's table, and it is not the arch 10.8
   part the task assumed. Mesa 26.2.1 lists the G310 as `PAN_PROD_ID(10,
   12, 4)`: arch 10.12, product 4. It has five variants, chosen by
   `CORE_FEATURES & 0xff`, with tile buffers of 16 KiB (v1-v3) or 32 KiB
   (v4-v5) (M: `model/pan_model.c:90-100`, `lib/kmod/panthor_kmod.c:162`).
   The G610 is `PAN_PROD_ID(10, 8, 7)` (:88). Both are Valhall arch 10.
2. Arch 10 means our shader compiler and descriptor packers apply as they
   are. Mesa's Valhall compiler keys on the arch major only: its one quirk
   table returns nothing for arch 10 (M: `bi_quirks.h:29-42`), and it reads
   the model only for statistics (M: `bifrost_compile.c:4038-4045`). The
   firmware is a different file, though: Panthor names it by arch
   major.minor (repo: `docs/notes/mali-panthor-kmsg.md:220-221`), so this
   part reads `arch10.12`, not the `arch10.8` file we pin (repo:
   `tools/cts/deps.toml:305-317`).
3. What is G610-specific in our tree is a short list of gates and
   constants (section 3.1): the `G610_GPU_ID` stop, VA 48 / PA 40, tiler
   levels == 8, the 16 KiB tile budget, the 64-byte L2 line and the
   non-coherent memory attributes. On top of that sits the whole RK3588
   SoC layer (CRU, PMU, RK806 PMIC, QoS). On the i.MX 95 that layer is
   replaced by SCMI requests to the System Manager.
4. The System Manager on the Cortex-M33 is the boot core and stays
   resident, serving SCMI for clocks, power, sensors and pins (Z:
   `boards/nxp/imx95_evk/doc/index.rst:323-329`). Zephyr's A55 port already
   powers a mix domain and sets its clocks over SCMI (Z:
   `soc/nxp/imx/imx9/imx95/a55/soc.c:62-99`, NETC), which is the pattern
   the GPU needs (`IMX95_PD_GPU` 14; `IMX95_CLK_GPUAPB`, `IMX95_CLK_GPU`).
   Whether the System Manager lets *our* agent switch the GPU's domain and
   clocks is the first unknown (U1).
5. Zephyr at our pin has the i.MX 95 EVK in both packages for the A55
   (single core and SMP), with console, SCMI, watchdog and a direct-boot
   route in which TF-A starts Zephyr with no U-Boot (Z: board doc
   :121-129). It has no GPU node and no A55 display. The display controller
   ("Seeris", `socionext,dpu` in Zephyr) has a Zephyr driver and a DSI
   panel only on the Cortex-M7 (Z: `nxp_imx95_m7.dtsi:982-1001`), plus
   NXP's BSD-3 MCUX drivers for DPU, LDB and DSI in hal_nxp.
6. Sizing: the GPU side is a port of a working back end (the T6's Zephyr
   `gl_test` is 50/50), so it is small to medium. The display path is the
   large new item, as it still is on the T6 (repo:
   `driver/docs/nanopc-t6-display.md:5`). The order is H0 (read what this
   host could not), then the parameterisation that needs no board, then
   three board days.

## What was read, and what was not

Prefixes used below. Line numbers are exact at the named version.

| Prefix | What, at what version |
| --- | --- |
| `Z:` | Zephyr at the workspace pin `af17c0c8aa2a82a0b714a423915ad3610063ee68` (`VERSION` 4.4.99), the west workspace of this host's dependency layout (`ZEPHYR_WS`) |
| `HAL:` | `hal_nxp` at `1e91007b164c52b74d58d4e75a856efbc5d4de05`, the revision Zephyr's `west.yml:215-217` pins; mainly `mcux/mcux-sdk-ng/devices/i.MX/i.MX95/` (NXP's per-part headers, BSD-3) and `mcux/mcux-sdk-ng/drivers/` |
| `M:` | Mesa 26.2.1, tag `mesa-26.2.1`, the tarball named in `third_party/mesa-26.2.1/UPSTREAM` (sha256 `c47e81bd…4442`, checked here). Read from an unpacked copy in the dependency layout; each cited file was compared with the tarball's member by sha256 and is identical. That copy also holds our own `pan_shc`, which is not cited as Mesa. Paths are under `src/panfrost/` unless they start with `src/` |
| `FW:` | the Mali firmware file the layout holds at `MALI_CSFFW`: sha256 `a27847ea…405b`, the value `tools/cts/deps.toml:317` pins (checked here) |
| `repo:` | this repository at `1d9022e7` |
| `T:` | facts the task statement gives as verified on 2026-11-20 from mainline Linux master (7.4-rc3) and linux-firmware. This host could not re-read them, so they are marked `T:` wherever they are used, never as read |

**Not read** (no network from this host). Each one is the input of item H0
(section 5):

- Mainline Linux, all of it: `arch/arm64/boot/dts/freescale/imx95.dtsi`
  and the board device trees (`imx95-19x19-evk.dts`, `imx95-15x15-evk.dts`,
  the FRDM i.MX 95 and the Toradex Verdin iMX95 trees and overlays),
  `drivers/gpu/drm/panthor/` (the `nxp,imx95-mali` match, `panthor_hw.c`'s
  model table), the `arm,mali-valhall-csf` binding, the display drivers and
  bindings for this SoC, and `include/dt-bindings/clock/nxp,imx95-clock.h`.
  The `T:` facts come from there.
- NXP's kernel `github.com/nxp-imx/linux-imx` (no branch was read): the
  display nodes and drivers NXP ships.
- `linux-firmware`: whether `arm/mali/arch10.12/mali_csffw.bin` exists, its
  size, hash and `WHENCE` entry.
- NXP's System Manager `github.com/nxp-imx/imx-sm`: the board
  configurations, the logical machines, and the agents' rights over the GPU
  and display domains and clocks (U1).
- `imx-mkimage`, U-Boot's i.MX 95 board document, TF-A's `plat/imx/imx95`.
- NXP's product pages, fact sheet, data sheet and reference manual. The
  reference manual and data sheet need an NXP account, so they would stay
  unread even with a network. Toradex's Verdin iMX95 data sheet and carrier
  board pages. The FRDM i.MX 95 page.
- Mailing lists (`lore.kernel.org`) for a Panthor probe log of an i.MX 95,
  Mesa's GitLab issues and merge requests, and Zephyr upstream `main`.
- The `imx95-evk-boot-firmware-6.12.34-2.1.0.bin` blob that Zephyr's
  direct route uses (HAL: `zephyr/module.yml:274-281`). It is behind a
  click-through licence (`LA_OPT_NXP_Software_License`) and is not fetched
  in this layout.

## 1. The hardware

### 1.1 The SoC family

- NXP's headers list thirteen i.MX 95 parts: MIMX9506, 9534, 9536, 9546,
  9554, 9556, 9574, 9576, 9586, 9594, 9596, 95N4 and 95N6 (HAL:
  `devices/i.MX/i.MX95/` directory and `periph_mapping.md:2`, one shared
  `periph` folder for all thirteen). Every part's header has the same GPU
  register bases (`GPU__REG__GPU_REGISTERS_BASE`, two hits in each
  `MIMX95*_cm7_COMMON.h`), and the feature headers of MIMX9596 and
  MIMX9506 differ only in their guard names (`diff`, this reading).
  **Which parts have the GPU fused off, how many A55 cores each has, and
  whether the NPU or ISP is present is therefore not in any source read
  here.** It is in NXP's data sheet (login) and fact sheet (unread, H0).
- The Mali register header's processor list names only MIMX9594 and
  MIMX9596 variants (HAL: the header comment of
  `periph/PERI_GPU_MALI_GPU_REGISTERS.h`, from :3, 54 lines naming only
  `MIMX9594…` and `MIMX9596…` processors). Other peripheral headers list every
  part, for example `PERI_GPU_BLK_CTRL_GPUMIX.h:3` starts at
  `MIMX9506AVZXN`. This is a hint, not a statement, that the Mali block
  is a 9594/9596 feature. Both EVKs are MIMX9596 (Z: `board.yml:6`, the
  board names `imx95_evk/mimx9596`, `imx95_evk_15x15/mimx9596`).
- CPU side, as Zephyr describes it: six Cortex-A55 (`cpu@0` … `cpu@500`,
  Z: `dts/arm64/nxp/nxp_mimx95_a55.dtsi:22-61`), plus a Cortex-M33 (the
  System Manager) and a Cortex-M7 (Z: board doc :16-19). GIC-v3:
  distributor `0x48000000`, redistributors `0x48060000` size `0xc0000`, ITS
  `0x48040000` (dtsi :75-90). `NUM_IRQS` is 407 without ITS and 16384 with
  it (Z: `soc/nxp/imx/imx9/imx95/Kconfig.defconfig.mimx95.a55:32-34`).
  `GIC_SAFE_CONFIG` is on by default "to run multiple OSes on Cortex-A
  Cores" (:15-17).
- Zephyr's pin also has the i.MX 952 and i.MX 943 EVKs. The i.MX 952 has a
  `IMX952_PD_GPU 12` (Z: `include/zephyr/dt-bindings/power/imx952_power.h:51`).
  It is a sibling to keep in mind, not studied here.

### 1.2 The GPU core

What the sources here state:

| Fact | Value | Source |
| --- | --- | --- |
| Product id | `PAN_PROD_ID(10, 12, 4)`: arch major 10, arch minor 12, product major 4, so `GPU_ID` reads `0xac?4????` (arch rev and version unknown) | M: `model/pan_model.c:91-100`; field layout `model/pan_model.h:25-41`, the same as HAL `PERI_GPU_MALI_GPU_REGISTERS.h:277-316` |
| Variants | v1..v5 = `gpu_variant` 0..4, Mesa reads it from `CORE_FEATURES & 0xff`. Tile buffer color/Z: v1-v3 16384/8192, v4-v5 32768/16384 (G610: 32768/16384). Rates per core (pixel, texel, varying, cvt, fma, sfu): v1 2,2,8,16,16,4 … v5 4,8,16,64,64,16, with Mesa's own caveat "var/cvt/sfu rates might not be correct" | M: `model/pan_model.c:88-100`, `lib/kmod/panthor_kmod.c:162` |
| Variant field | NXP documents `CORE_FEATURES[7:0] CORE_VARIANT` with five encodings `0b00000000`…`0b00000100` and no names | HAL: `PERI_GPU_MALI_GPU_REGISTERS.h:343-355` |
| Which variant NXP built | **not stated in any source read here** | U2 |
| Register window | `GPU_REGISTERS` `0x4D900000`, `USER` `0x4D910000`, `IPA_CONTROL` `0x4D940000`, `DOORBELLS` `0x4D980000` (doorbells at +0x80000, as on the T6: repo `src/osal/zephyr_mali.c:75-77`) | HAL: `MIMX9596_cm7_COMMON.h:1979, 3684, 2164, 1606` |
| Window size | `0x480000` | T: `imx95.dtsi` `gpu@4d900000` |
| Register layout | `GPU_ID` +0x0, `L2_FEATURES` +0x4, `CORE_FEATURES` +0x8, `TILER_FEATURES` +0xC, `MEM_FEATURES` +0x10, `MMU_FEATURES` +0x14, `AS_PRESENT` +0x18, `CSF_ID` +0x1C, `TEXTURE_FEATURES_0..3` +0xB0..0xBC, `SHADER_PRESENT` +0x100, `TILER_PRESENT` +0x110, `L2_PRESENT` +0x120; 8 address-space slots in the layout | HAL: `PERI_GPU_MALI_GPU_REGISTERS.h:133-177` |
| Around the GPU | `GPU_BLK_CTRL_GPUMIX` `0x4D810000` with `STRIPING_GRANULE` +0x0, `TEXFMTENABLE` +0x4 (32 bits), `GPURESET` +0x8 (bit 0 `GPURR`, "GPU Reset Release"), `DBGCTRL` +0xC; a `GPU_TCU` at `0x4D800000`; a GPU mix TRDC (its error interrupt is named) | HAL: `MIMX9596_cm7_COMMON.h:1969, 1989, 392`; `PERI_GPU_BLK_CTRL_GPUMIX.h:411-414, 446-450` |
| Interrupts | job SPI 289, mmu 290, gpu 288 | T: `imx95.dtsi`. The M7's header numbers them 490-492 ("GPUMIX GPU/Job/MMU Interrupt", HAL `MIMX9596_cm7_COMMON.h:581-583`). The M7 map is not the A55 GIC map, for example MU2: A55 SPI 226 (Z: dtsi :489) against M7 236 (HAL :327), so the M7 numbers say nothing about the A55's |
| Clocks | SCMI clock ids `IMX95_CLK_GPUAPB` = 41+42 = 83, `IMX95_CLK_GPU` = 41+43 = 84 in Zephyr's binding | Z: `include/zephyr/dt-bindings/clock/imx95_clock.h:51, 95-96`. T: Linux's node names `IMX95_CLK_GPU_CGC` and `IMX95_CLK_GPUAPB`; Zephyr's header has no `CGC` name, so reconcile the ids in H0 |
| Power domain | `IMX95_PD_GPU` 14 (SCMI power protocol) | Z: `include/zephyr/dt-bindings/power/imx95_power.h:24` |
| OPPs | 500, 800, 1000 MHz | T: `imx95.dtsi` |

What no source here states, and what pins it on day one: `GPU_ID`
(revision), `CORE_FEATURES` (the variant, and so the tile buffer),
`SHADER_PRESENT` (core count), `L2_FEATURES` (line size), `TILER_FEATURES`
(hierarchy levels), `MMU_FEATURES` (VA/PA bits), `MEM_FEATURES`,
`COHERENCY_FEATURES`, `TEXTURE_FEATURES_0` (bit 25 = AFRC present, M:
`lib/pan_props.c:100-108`), `CSF_ID`. Our `mali::ident` already reads all
of these in one call (repo: `driver/rust/src/gpu/mali/mod.rs:130-146`).
Day one prints them (section 6). A Panthor probe line from any i.MX 95
mailing-list post would pin them earlier (H0). None was readable here.

### 1.3 The display path

The SoC side, from NXP's headers and Zephyr (Linux unread):

- Display controller: `DISPLAY_SEERIS` at `0x4B400000` (HAL:
  `MIMX9596_cm7_COMMON.h:1558`). Zephyr names it `dpu: dpu@4b400000`,
  `compatible = "socionext,dpu"`, interrupt 215 on the M7 (Z:
  `dts/arm/nxp/imx/nxp_imx95_m7.dtsi:982-989`). Its driver programs the
  display-controller part at `0x4B780000` (Z:
  `drivers/display/display_socionext_dpu.c:17`), through NXP's `fsl_dpu`
  (`DPU_Init`, fetch unit, layer blend, const frame, ext dst: :257-306).
  NXP's MCUX driver for this block is `drivers/dpu_1/fsl_dpu.c` (BSD-3,
  4929 lines, typed on `DISPLAY_SEERIS_Type`; changelog 2.3.0 "Supported
  the SEERIS MDR7", HAL).
- Around it: `DISPLAY__BLK_CTRL_DISPLAYMIX` `0x4B010000`, the pixel mapper
  `0x4B000000`, `PIXEL_INTERLEAVER1` `0x4B0D0000`, `DPU_IRQSTEER`
  `0x4B0B0000`, the display mix's own fractional PLL `0x44481900` (HAL:
  `MIMX9596_cm7_COMMON.h:1528, 1548, 1538, 1616, 3217`). The display
  interrupts go through an IRQSTEER: `DISP_IRQSTEER0..7` (HAL :305-310),
  and the M7 tree's `disp_irqsteer` at `0x4b0b0000` with 512 inputs (Z:
  `nxp_imx95_m7.dtsi:911-925`).
- LVDS: one `LDB` instance, `LVDS_BASE` `0x4B0C0000` (HAL :2244), with the
  MCUX driver `drivers/ldb_1` (BSD-3).
- MIPI DSI: `MIPI_DSI_BASE` `0x4ACF0000`. It sits in the camera mix, next
  to `CAMERA__DSI_MASTER_CSR` `0x4AD10000` and the shared DSI/CSI combo PHY
  `0x4AD20000` (HAL :2444, 1237, 1247). Zephyr's `nxp,mipi-dsi-dwc` driver
  configures it for the M7 only (`#if CONFIG_SOC_MIMX9596_M7`, Z:
  `drivers/mipi_dsi/dsi_nxp_dwc.c:108-115`). It writes the combo PHY's
  frequency range, selects the pixel link and enables
  `DISPLAYMIX PIXEL_LINK_CTRL PL0`. NXP's driver is
  `drivers/mipi_dsi2_dwc` (BSD-3).
- Power and clocks: `IMX95_PD_DISPLAY` 13, `IMX95_PD_CAMERA` 3 (Z:
  `imx95_power.h:13, 23`); the M7 tree puts the IRQSTEER in
  `power-domain@d` (13) through SCMI (Z: `nxp_imx95_m7.dtsi:116-124,
  918`). The clocks are `IMX95_CLK_DISPAPB/DISPAXI/DISPDP/DISPOCRAM/
  DISP1PIX/DISP2PIX/DISP3PIX` and `VIDEOPLL1` (Z: `imx95_clock.h:27-28,
  87-94`).
- What already lights a panel from Zephyr: the M7 build with the
  `nxp_mx9_dsi_oled` shield, a Raydium RM692C9 1080x2340 four-lane DSI OLED
  (Z: `boards/shields/nxp_mx9_dsi_oled/nxp_mx9_dsi_oled.overlay:20-21,
  50, 54`; doc :37-41 builds it for `imx95_evk/mimx9596/m7`).
- HDMI and DisplayPort: no HDMI transmitter or DP block appears among the
  bases read here. An HDMI output on a board would be a bridge chip behind
  LVDS or DSI. Which bridges the boards carry is a device-tree question
  (H0).

The board side, compared:

| Board | Package, memory | Display connectors | Source |
| --- | --- | --- | --- |
| i.MX 95 EVK 19x19 (IMX95LPD5EVK-19) | MIMX9596, "128-Gbit LPDDR5" | "LVDS interface: two mini-SAS connectors each with x4-lane configuration"; "MIPI CSIDSI interface: … one 36-pin miniSAS connector using x4 lane"; no HDMI connector listed | Z: `boards/nxp/imx95_evk/doc/index.rst:6-11, 21, 52-56` |
| i.MX 95 EVK 15x15 (IMX95LP4XEVK-15) | MIMX9596, "8-Gbit LPDDR4x" (as written) | the same two LVDS mini-SAS and one CSI/DSI mini-SAS | Z: `boards/nxp/imx95_evk_15x15/doc/index.rst:6-11, 21, 47-49` |
| FRDM i.MX 95 | not read | not read | H0 (NXP page, Linux DT) |
| Toradex Verdin iMX95 + carriers (Dahlia, Verdin Development Board, Mallow, Yavia, Ivy) | not read | not read (the Verdin family carries DSI and LVDS on the module edge; for this SoC that is not read) | H0 (Toradex pages, Linux DT) |

So a frame goes: GPU writes DRAM → Seeris display controller (fetch,
blend) → pixel link → LDB (LVDS) or DSI → a panel, or a bridge on the
board. As on every port, the GPU never drives the connector.

### 1.4 The boot chain

From Zephyr's board document (the only boot description read here):

- The Cortex-M33 is the boot core. It runs the boot ROM, which loads the
  System Manager "(and other boot code)". The SM sets up the isolation
  mechanisms, starts the other cores, and then "enters a service mode
  where it provides access to clocking, power, sensor, and pin control via
  a client RPC API based on ARM's SCMI" (Z: board doc :323-329).
- The boot image `flash.bin` holds "ELE+V2X firmware, System Manager, TCM
  OEI, TF-A images etc." (:124-127; for the M7: "ELE+V2X firmware, System
  Manager, TCM OEI, Cortex-M7 image", :331-333). The debug USB gives four
  serial ports: the first is the M7's and the fourth is the M33 System
  Manager's (:155-157).
- So the System Manager is resident in every route. SCMI answers whether
  Linux, U-Boot or Zephyr runs on the A55: **for SCMI, nobody else must be
  running**. Our A55 image talks to it over `mu2` (`0x445b0000`, SPI 226)
  with shared memory `0x445b1000` (0x80 bytes) (Z: `nxp_mimx95_a55.dtsi:
  92-133, 486-492`). Which SM agent that channel is, and what that agent
  may do, is set by the SM's build configuration (U1).
- Routes for the A55 image, all from Zephyr's document:
  1. **Direct, no U-Boot.** The SPSDK runner builds a `flash.bin` with the
     NXP boot firmware, and "as no U-Boot image is available, so TF-A will
     boot up Zephyr on the first Cortex-A55 Core directly" (:121-129;
     `CONFIG_BOARD_NXP_SPSDK_IMAGE`, `board.cmake:3-17` names the blob
     `imx95-evk-boot-firmware-6.12.34-2.1.0`). This is the i.MX 95's
     upstream equivalent of our direct boots
     (`docs/notes/imx8mp-direct-boot.md`, `docs/notes/am62p-direct-boot.md`),
     with nothing of ours in the chain before Zephyr. It can load through
     USB serial download (`west flash -r spsdk`), or be written to SD or
     eMMC (:162-172).
  2. **U-Boot `go`.** `tftp 0xd0000000 zephyr.bin` or `fatload`, then
     `dcache off; icache flush; go 0xd0000000` on core 0 (:175-226). The
     `cpu <n> release` form for the other cores needs NXP's Real-Time
     Edge U-Boot (:178-183).
  3. Linux remoteproc, from the Real-Time Edge release (:245-250).
  4. J-Link with the board stopped at U-Boot (:252-261).
- Both A55 defconfigs set `CONFIG_ARMV8_A_NS=y` (Z:
  `imx95_evk_mimx9596_a55_defconfig:8`): Zephyr runs non-secure under
  TF-A.

**Sign-off route, proposed.** U-Boot starting our image (route 2) is the
route the house signs off on ("The sign-off form (U-Boot starting the
image)", `driver/docs/nanopc-t6-mali.md:802`).
It needs only NXP's stock boot image, which carries U-Boot, plus `go` on
core 0. Route 1 is the fast route: it is upstream-documented and puts TF-A
directly in front of Zephyr, but it needs the click-through boot-firmware
blob, which must never enter the tree (the same rule as the PowerVR and
Mali firmware, `CONTRIBUTING.md:230-244`). Unlike the T6, no kexec from
Linux is needed for development: route 2 is also the development loop.

## 2. The open-source stack today

### 2.1 Kernel: Panthor

- T: Panthor matches `arm,mali-valhall-csf`; the i.MX 95 node is
  `compatible = "nxp,imx95-mali", "arm,mali-valhall-csf"`, and
  `panthor_hw.c` names the G310. Not re-read here, and the remaining
  questions are H0: what in Panthor, if anything, is i.MX 95-specific (a
  match-data entry, a reset or a blk-ctrl step for `GPURESET`), whether the
  node is `dma-coherent` or behind an IOMMU, and its supply.
- What we already know of Panthor from our own reading of v6.18.55 (repo:
  `docs/notes/mali-panthor-kmsg.md:11-24`): the firmware file is
  `mali_csffw.bin` under `arm/mali/arch<major>.<minor>/` (:220-221; for
  the G610 that is `arch10.8`).

### 2.2 Firmware

- Ours: `arm/mali/arch10.8/mali_csffw.bin` from linux-firmware `930ef904`,
  282624 bytes, sha256 `a27847ea…405b` (repo: `tools/cts/deps.toml:305-317`;
  FW: the same hash on this host). Our parser and loader expect interface
  1.5.0 from it (repo: `driver/rust/src/gpu/mali/fw.rs:102-103`).
- The G310 is arch 10.12 (section 1.2), so Panthor would request
  `arm/mali/arch10.12/mali_csffw.bin`, **a different file**. T: linux-firmware
  has `arch10.12`. Its size, hash, licence line and interface version are
  not read here (H0, H6). Whether its interface is the 1.5.0 our
  `csf.rs`/`fw.rs` speak is U3.

### 2.3 Mesa: panfrost and panvk for the G310

- The model table entry and variants are in section 1.2. The table
  differences from the G610 are only the tile buffer size (half on v1-v3)
  and the rates. Neither G310 nor G610 sets a quirk (`max_4x_msaa`,
  `no_hierarchical_tiling` are Midgard-only, M: `model/pan_model.c:63-72`),
  and both have anisotropy on all revisions (`MODEL_ANISO(ALL)`).
- The tile buffer size feeds the maximum colour-buffer count and the MSAA
  limit Mesa advertises (M: `src/gallium/drivers/panfrost/pan_screen.c:
  128-151`) and the framebuffer descriptors. Mesa notes that "For Mali-G510
  and Mali-G310, we will need extra logic to query the tilebuffer size for
  the particular variant. The CORE_FEATURES register might help"
  (M: `lib/pan_props.c:114-115`). The table already does that through
  `gpu_variant`.
- Core count and the core-id range come from `SHADER_PRESENT` (M:
  `lib/pan_props.c:40-55`) and feed the TLS and stack sizing (M:
  `src/gallium/drivers/panfrost/pan_device.c:64-66`).
- AFRC is optional on v10, signalled by `TEXTURE_FEATURES_0` bit 25 (M:
  `lib/pan_props.c:100-108`). We use neither AFRC nor AFBC (repo:
  `src/mali/frame.rs:63-64`).
- The compiler: arch 10 for both. `bifrost_get_quirks` keys on
  `(arch_major << 4) | arch_minor` and returns 0 for anything but G71, G72
  and G31 (M: `compiler/bifrost/bi_quirks.h:29-42`). The model is looked
  up only for the statistics (M: `bifrost_compile.c:4038-4045, 4943`). The
  one arch-minor test outside the compiler is `EARLY_ZS_ALWAYS` (7.2 and
  later), which is true for both (M: `src/gallium/drivers/panfrost/
  pan_fb_preload.c:1230-1249`).
- **drm-shim takes the G310 already.** `PAN_GPU_ID` is the top 16 bits of
  `GPU_ID` in hex, with an optional `:variant`, and the shim's own example
  is a G310: "PAN_GPU_ID=ac04:4" (M: `drm-shim/panfrost_noop.c:19-48`).
  It reports `gpu_id = PAN_GPU_ID << 16` and `core_features = variant`
  (:169-174), but every other register it returns is "Dumped from a G610":
  `l2_features 0x7120306`, `tiler_features 0x809`, `mmu_features 0x2830`,
  `texture_features[0] 0xc1ffff9e`, `shader_present 0x50005` (:176-191).
  So a shim reference shows what the product id and variant change (tile
  buffer, model name). It does not show what a different core count, L2
  line or tiler would change. Those come from the board's registers.

### 2.4 The display driver

- In the trees read here: Zephyr's `socionext,dpu` and
  `nxp,mipi-dsi-dwc` drivers (M7 only), and NXP's MCUX `dpu_1`, `ldb_1`
  and `mipi_dsi2_dwc` (BSD-3), all in section 1.3.
- Mainline Linux's and NXP linux-imx's state for this display controller
  is **not read** (H0). The task's search of mainline `imx95.dtsi` for
  `display|dpu|lvds|dsi|hdmi` found nothing, so the node is elsewhere
  (another dtsi, an overlay, or only in NXP's tree). H0 settles which,
  with the branch named.

## 3. What yakogl already has that applies

### 3.1 Keyed to the G610 today (must be read from the hardware or parameterised)

The kernel side, `driver/rust/src/gpu/mali/` and the T6 board modules:

| Where | What | i.MX 95 |
| --- | --- | --- |
| `gpu/mali/mod.rs:97-98`; the stop at `board/nanopc_t6_gpu.rs:387-388` (`Stop::NotG610`, :136) | `G610_GPU_ID 0xa867_0005`, an exact match | an allow-list by arch/product (10.8.7, 10.12.4); without it the G310 stops at Z1 |
| `gpu/mali/pgtable.rs:30-35, 57`; check `board/nanopc_t6_mmu.rs:172`; uses `board/nanopc_t6_submit.rs:492, 1096` | `VA_BITS 48`, `PA_BITS 40`, 4 levels, a 48-bit output mask; the board stops unless VA is exactly 48 and PA at least 40 | from `MMU_FEATURES` (Z2 already takes VA from it at `nanopc_t6_mmu.rs:208-209`) |
| `gpu/mali/mod.rs:93-95, 317`; `nanopc_t6_mmu.rs:38, 209`; `nanopc_t6_submit.rs:1096`; `gpu/mali/fw.rs:157-161` | non-coherent: `COHERENCY_NONE` written unconditionally, the `MEMATTR` word copied live from the T6, `transcfg(…, false)`, firmware sections cached on that assumption | from `COHERENCY_FEATURES` and the board's DT fact (U4); compute `MEMATTR` with `mmu::mair_to_memattr` (`gpu/mali/mmu.rs:110`) |
| `board/nanopc_t6_submit.rs:179-180, 302-304` | heap pool and context sized for a 64-byte L2 line (a larger line is refused) | from `L2_FEATURES` (the stride rule is already read from it, `gpu/mali/heap.rs:57-63`) |
| `gpu/mali/heap.rs:115-116`, `nanopc_t6_submit.rs:301`, `nanopc_t6_osal.rs:127-128` | heap and carve-out GPU VAs at `0x8000_…` and `0x40_0000_0000` need ≥ 48-bit VA | holds if the G310 reads VA 48 (U2) |
| `gpu/mali/mod.rs:315-316` | L2 power-on takes the lowest bit of `L2_PRESENT` ("the G610 has one") | generic while the G310 has one core group |

Already read from the hardware or the firmware, so they carry over:
`shader_present` as `core_en_mask` and the compute mask (`gpu/mali/csf.rs:
796-828`, `gpu/mali/sched.rs:694-698`), the CS work-register count
(`csf.rs:308`, `board/nanopc_t6_osal.rs:722-723`), the tiler-heap stride,
the address-space registers (`gpu/mali/mmu.rs:42-43, 95, 110`), and the
whole takeover and reset path (`gpu/mali/takeover.rs`, panthor register
offsets). The host never powers the shader cores or the tiler: the
firmware does (`gpu/mali/mod.rs:17-19`). No interrupt is used: the waits
poll the raw status registers (for example the soft reset,
`mod.rs:197-200`), so the GPU's interrupt numbers do not block the port.

The RK3588 layer is replaced, not parameterised: `driver/rust/src/soc/
rk3588.rs` (CRU `0xfd7c0000` :30, the GPU clock mux and gates :148-167,
OPP table :280-289, PMU `0xfd8d8000` :483, `pd_gpu` :511-527, QoS :534,
PMU registers :601-618, `PD_GPU` :656-669, SPI2 :1089-1091), the RK806
supply check on SPI2 (`board/nanopc_t6_gpu.rs:82-90`), the QoS policy
(:199), and the power-on sequence (:287-378; documented in
`driver/docs/nanopc-t6-mali.md:1433-1627`). On the i.MX 95 the same steps
are: SCMI power-on of `IMX95_PD_GPU`, SCMI clock enable and rate for
`IMX95_CLK_GPUAPB`/`IMX95_CLK_GPU`, `GPURESET.GPURR` in the GPU blk-ctrl
if nobody else releases it (section 1.2; who does is U2's day-one read),
then our unchanged soft reset, coherency write and L2 power-on
(`nanopc-t6-mali.md:99-102`). There is no PMIC step on our side: supplies
belong to the System Manager.

The GL back end, `src/mali/`:

| Where | What | i.MX 95 |
| --- | --- | --- |
| `frame.rs:280-283`, used by `tile_size_of` :1966-1968 | `TIB_BUDGET = 32768 / 2`, the G610's tile buffer halved (Mesa 25.0.7's rule, cited there) | from the model and variant: 16384 (v1-v3) or 32768 (v4-v5) colour bytes, with the same rule. On v1-v3 every tile size halves, and the four-target MSAA cases that fit the G610's budget may not fit (the MRT and MSAA limits follow the tile buffer, M: `pan_screen.c:128-151`) |
| `frame.rs:267-268, 2620` (message :189-190) | `MEASURED_TILER_LEVELS = 8`; any other `TILER_FEATURES` max_levels is refused | the G310's value is unknown (U2); the descriptor field is general, the gate is not |
| `frame.rs:284-285` | `MAX_EFFECTIVE_TILE = 32 * 32` for arch 10 | carries over (arch 10) |
| `frame.rs:2074`, `frame.rs:3694` | IDVS used unconditionally | U9: is IDVS present on every G310 variant? |
| `src/osal/mali.rs:44-53` | `OsalMaliGpu` passes only `gpu_id`, `tiler_features` and the CS register counts | must also pass `core_features` (the variant) |
| `backend.rs:3714` | renderer string "Arm Mali-G610 (NanoPC-T6)" | cosmetic |
| `genxml/mod.rs`, `genxml/v10.rs` (from Mesa `v10.xml`) | every descriptor and CS instruction is v10 | carries over (arch 10) |

The compiler, `src/shaderc/online/mali/`, has no `gpu_id`, product, quirk
or warp-size branch. Its ISA table `isa/v10.rs` is generated for arch 10
(`isa/mod.rs:9-14`), and the work-register split `WORK_REGS_SPLIT = 32` is
per-ISA (`ra.rs:258-259, 730`). Given section 2.3, the compiler is
expected to emit identical bytes for the G310. H1 checks that.

The Zephyr side:

- `src/osal/zephyr_mali.c:75-84` hard-codes the T6's physical blocks and
  the 24 MHz timer, and `:237-264` hands the RK3588 blocks to
  `yd_t6_osal_up`. The i.MX 95 needs an OSAL entry that passes the GPU base
  `0x4D900000` and an SCMI-backed power and clock path instead. The timer
  is 24 MHz here too (Z: `Kconfig.defconfig.mimx95.a55:36-37`; board doc
  :81).
- The carve-out is a 2 MiB-aligned `.noinit` static made Normal-NC
  (`zephyr_mali.c:143, 170`), 32 MiB by default and 64 MiB for dEQP
  (`zephyr/Kconfig:402-407`, `driver/samples/zephyr/deqp/boards/
  nanopc_t6.conf:16`). It needs no board address. The i.MX 95 EVK's
  Zephyr `dram` is only `0xd0000000` + 10 MiB (Z:
  `imx95_evk_mimx9596_a55.dts:50-52`), so our board copy must enlarge it
  (U6).
- `zephyr/Kconfig:76-79`: `YAKOGL_GPU_MALI` "Arm Mali-G610 (NanoPC-T6,
  RK3588)" `depends on YAKOGL_NANOPC_T6`. It must open to a second board.
- Our T6 board has no GPU node in its DT (repo: `driver/boards/friendlyarm/
  nanopc_t6/nanopc_t6.dts:42-74`) because the OSAL holds the addresses.
  The i.MX 95 board copy can do the same, or carry the `T:` node.

The references:

- `tools/mali/drm_shim_ref.sh:72-75` runs with `PAN_GPU_ID=a867` fixed.
- `tools/mali/pan_shc.c:1184-1185` defaults to `gpu_id 0xa8670005` and
  already takes `--gpu-id HEX` and `--gpu-variant N` (:15-17, :1171).

### 3.2 What is new

1. **SCMI requests from Zephyr for the GPU.** Zephyr has the API:
   `scmi_power_state_set`/`_get` (Z: `include/zephyr/drivers/firmware/
   scmi/power.h:89-98`), `scmi_clock_config_set`, `_rate_set`,
   `_parent_set` (Z: `drivers/firmware/scmi/clk.c:109-214`), and the SoC
   init that uses them for NETC (Z: `soc.c:62-99`). Note a trap: Zephyr's
   clock driver returns success *without acting* when the SM's
   permissions deny state control (Z: `drivers/clock_control/
   clock_control_arm_scmi.c:51-59`, `return 0` when
   `!SCMI_CLK_STATE_CONTROL_ALLOWED`). A refused GPU clock would look
   like a success. Our code must call the SCMI layer directly and check
   the permissions, as `soc.c` does.
2. **The display path** (section 1.3): Seeris display controller, pixel
   link, LDB or DSI, the display mix's power domain and PLL, the IRQSTEER.
   The readable references are NXP's BSD-3 MCUX drivers and Zephyr's
   Apache-2.0 M7 drivers. Both are permissive, so under
   `docs/source-form.md` they are vendorable or readable. No GPL source is
   needed.
3. **The boot route** (section 1.4): `go 0xd0000000` from NXP's U-Boot,
   then the SPSDK direct image. Neither needs our own loader stage.

## 4. The board on Zephyr

### 4.1 What the pin has

- Boards: `imx95_evk` (19x19) with the A55 targets
  `imx95_evk/mimx9596/a55` and `…/a55/smp`, and M7 targets in TCM, DDR and
  flash (Z: `boards/nxp/imx95_evk/board.yml:1-13`). `imx95_evk_15x15` has
  `a55` and `m7` (directory listing).
- Console: `lpuart1` at 115200 (Z: `imx95_evk_mimx9596_a55.dts:20-22,
  122-127`). Its clock comes from the SM over SCMI: parent 24 MHz, enable,
  rate (Z: `soc.c:19-57`).
- SCMI: `arm,scmi` over `mu2` with shared memory; power protocol `0x11`,
  clock `0x14`, pinctrl `0x19` (Z: `nxp_mimx95_a55.dtsi:102-133`), all
  initialised at priority 31 after the GIC and ITS (Z:
  `boards/nxp/imx95_evk/Kconfig.defconfig:13-29`). Zephyr also has NXP's
  vendor extensions and CPU protocol (Z: `drivers/firmware/scmi/nxp/
  Kconfig`, `cpu.c`), a reset protocol helper, and a power-domain driver
  `arm,scmi-power-domain` (Z: `drivers/power_domain/
  power_domain_arm_scmi.c:6`).
- SMP: the `smp` variant, `CONFIG_SMP=y`, `MP_MAX_NUM_CPUS=6`, PSCI CPU ops
  (Z: `imx95_evk_mimx9596_a55_smp_defconfig:21-24`). The single-core board
  keeps `cpu@500` enabled in its DT and disables `cpu@0`…`cpu@400` (Z:
  `imx95_evk_mimx9596_a55.dts:28-48`).
- Memory: `zephyr,sram = &dram`, "sram node actually locates at DDR DRAM",
  `memory@d0000000` 10 MiB (Z: dts :23-24, 50-52). Images are loaded at
  `0xd0000000` (board doc :197, 226), with `CONFIG_AARCH64_IMAGE_HEADER=y`,
  `XIP=n` (defconfig :7, 19).
- Watchdog: `wdog3`/`wdog4` (`nxp,wdog32`, SPI 77/78) in the SoC dtsi
  (:175-197); the board enables `wdog4` as `watchdog0` (dts :16-18,
  129-131; `supported: watchdog`, yaml :21). Our images need it, and it is
  there.
- Also enabled: NETC Ethernet (PSI0, RTL8211F), CAN2, I2C 4/5/7, a PCAL6408
  expander (dts :54-138).
- How the image is loaded: section 1.4.

### 4.2 What it lacks

- A GPU node, and anything Mali: there is no `mali` compatible in Zephyr
  (no binding under `dts/bindings` names one).
- An A55 display: the DPU, DSI and IRQSTEER nodes exist in the M7's dtsi
  only (Z: `nxp_imx95_m7.dtsi:905-1001`), and the DSI driver's PHY and
  pixel-link code is under `CONFIG_SOC_MIMX9596_M7` (Z:
  `dsi_nxp_dwc.c:108`).
- A RAM region big enough for our images (10 MiB, section 3.1).
- The 15x15 A55 board lists no watchdog or I2C in `supported` (Z:
  `imx95_evk_15x15_mimx9596_a55.yaml:15-18`).

### 4.3 The other candidate boards

- At the pin: no FRDM i.MX 95 (`boards/nxp/` has `frdm_imx91`,
  `frdm_imx93`, `frdm_imxrt1186` only) and no Toradex Verdin iMX95
  (`boards/toradex/` has `colibri_imx7d`, `verdin_am62`, `verdin_imx8mm`,
  `verdin_imx8mp`). Directory listings of Z.
- Upstream `main`: not read (H0).
- Since the SoC dtsi and SCMI are shared, a board of ours for the FRDM or
  the Verdin is a board file over `nxp_mimx95_19x19_a55.dtsi` or
  `nxp_mimx95_15x15_a55.dtsi` (Z: the two package dtsi files include the
  common one, `nxp_mimx95_19x19_a55.dtsi:7`). It needs that board's UART,
  pinmux and memory, plus its display wiring from its Linux DT.

## 5. What can be done before the board arrives

Each item with its check. None needs the board.

| # | Item | Check |
| --- | --- | --- |
| H0 | Read what this host could not (the "not read" list): mainline `imx95.dtsi` and the four boards' DTs; Panthor's i.MX 95 handling; linux-firmware `arm/mali/arch10.12/` (size, sha256, `WHENCE`); `imx-sm`'s board configurations (agents, logical machines, rights over `PD_GPU`, `PD_DISPLAY` and the GPU and display clocks); `linux-imx`'s display nodes and driver (branch named); NXP and Toradex board pages; a Panthor probe log of an i.MX 95 on the lists | an update commit of this note that replaces every `T:` with a citation and empties "not read", or names why an entry stays (login wall); U1 and U3 answered or narrowed |
| H1 | G310 references from the existing corpus. `pan_shc --gpu-id 0xac040000 --gpu-variant N` for N = 0 and 4 (v1 and v5, the two tile-buffer sizes), and `drm_shim_ref.sh` with `PAN_GPU_ID=ac04:N` (made a parameter instead of the fixed `a867`, `tools/mali/drm_shim_ref.sh:73`), over `tools/mali/testdata/` and the dEQP reference families; byte comparison against the `a867` outputs | a table, by artefact, of what differs. Expected from section 2.3: shader binaries identical; framebuffer and tiler descriptors differ in tile size and colour-buffer allocation on v1-v3. The measured difference is the real size of the GPU-side work. The limits of the shim (section 2.3) are stated with it |
| H2 | Kernel side reads its parameters: the arch/product allow-list for the identity; VA/PA bits from `MMU_FEATURES`; coherency from `COHERENCY_FEATURES` plus a board flag; `MEMATTR` computed; heap pool from the L2 line. Host tests with a mock G310 register file (the mocks exist, at `0xfb000000`, for example `gpu/mali/mod.rs:336`) with `GPU_ID 0xac04…`, a different `SHADER_PRESENT` and both coherency modes | the T6's Z1-Z4 expectations unchanged (their host tests pass as they are); the G310 mock passes Z1/Z2 to the same records with its own values |
| H3 | `src/mali` from the model: `OsalMaliGpu` gains `core_features`; `TIB_BUDGET` from (product, variant) as Mesa's table gives it; the tiler-level gate becomes a statement of what is built | `src/mali` tests for both identities; with the G610 identity, byte-identical frames to today's |
| H4 | The SoC layer for the i.MX 95 over SCMI: PD_GPU on and polled, `GPUAPB` and `GPU` enabled with a rate from the OPP list, the clock permissions read first, `GPURESET` released. Written against a trait with a recording mock | a host test pins the request sequence and fails on a denied permission instead of continuing (the Zephyr trap of section 3.2) |
| H5 | Board copy plus a build-not-run gate row: an out-of-tree board (or extension) over `imx95_evk/mimx9596/a55` with the enlarged RAM and the GPU block mapped, then `examples/triangle` and `gl_test` built with the Mali back end, through `zephyr_werror_image` (`tools/check.sh:4337`) like the T6 rows (:4466-4468, :4652-4653) | the rows pass in the gate; `hal_nxp` is already in the manifest (`west.yml:31`) |
| H6 | The arch 10.12 firmware: a second `MALI_CSFFW` entry in `tools/cts/deps.toml` pinned to linux-firmware's file (after H0), and our `fw.rs` parser run on it in a host test | the test lists its sections and interface version; if that version is not 1.5.0, U3 becomes the next item |
| H7 | Display design for the i.MX 95: Seeris DC + pixel link + LDB or DSI, from NXP's MCUX drivers and Zephyr's M7 drivers; the first panel chosen (the MX9 DSI OLED that Zephyr already drives from the M7, or an LVDS panel) | `docs/notes/imx95-display.md` with the register sequence and its sources; a build-not-run row for its probe image |

## 6. What needs the board

Day one, in order:

| # | Try | Done when |
| --- | --- | --- |
| D1 | Boot NXP's stock image (Linux) once and record: Panthor's probe lines (`GPU_ID`, the features, `shader_present`), the firmware file it loads, the GPU node's `dma-coherent`/`iommus`, `/proc/interrupts` for the three GPU lines, `/proc/iomem`, the GPU clock rates, and the SM's console (fourth serial port) | the values in the issue and in section 1.2 of this note (update commit) |
| D2 | Our image through U-Boot `go 0xd0000000`: console; SCMI base protocol (agent name, protocol list); clock permissions for `GPUAPB`/`GPU`; PD_GPU on; clocks on; `GPURESET` read and released; then `mali::ident` | **the GPU identity read by our code** (all of section 1.2's unknown registers), printed and compared with D1 |
| D3 | The firmware with the arch 10.12 file: our Z3 path (sections mapped, MCU started, GLB control block) | **the firmware's first answer**: the GLB version and the CS features word, as Z3 recorded them on the T6 (`tools/cts/STATUS.md:17`) |

Then the measurements that say the port is alive, in the house order:

- M1 the first submitted frame read back (the T6's Z4): the first job
  completed with the expected pixels.
- M2 the first `gl_test` case on read-back, then the 50-case set (the T6
  reached 50/50 on read-back before having any display, `STATUS.md:17`).
- M3 the first clear on a display (after H7): DPU scan-out of a buffer
  the GPU filled.
- M4 the first triangle on that display.
- M5 the direct route (SPSDK, TF-A → Zephyr) running the same image.

## 7. Risks and unknowns

Each stays a question. Nothing here is guessed.

| # | Question | What answers it |
| --- | --- | --- |
| U1 | Does the System Manager let a non-Linux A55 agent request `PD_GPU` and the GPU clocks? The A55 channel (`mu2`) is the one Zephyr's NETC init already uses to power `PD_NETC` (Z: `soc.c:73-88`), so that agent has at least NETC rights in the EVK's stock SM build. The GPU's rights are in `imx-sm`'s board configuration | H0 (the `imx-sm` configuration on GitHub); D2 (SCMI permission queries, and the power state read back after the request) |
| U2 | The G310's configuration: variant (tile buffer 16 or 32 KiB), core count, L2 line, tiler levels, VA/PA bits, AFRC; and who releases `GPURESET` | a public Panthor probe log (H0); D1 and D2 for certain |
| U3 | Does the arch 10.12 firmware speak the CSF interface our code speaks (1.5.0 on the T6), with the same section layout? | H6 on the host; D3 on the board |
| U4 | Is the GPU's memory path coherent, or behind the `GPU_TCU` (`0x4D800000`) and a TRDC that the SM or TF-A must open? Our code assumes non-coherent with no IOMMU | H0 (the Linux node's `dma-coherent` and `iommus`, `imx-sm`'s TRDC setup); D1 and D2 |
| U5 | The display controller's documentation: the reference manual is behind an NXP login. Is what is public (NXP's BSD-3 MCUX drivers, Zephyr's M7 drivers, and the Linux driver if mainline has one) enough for the A55 route, including the pixel link and the LDB? | H0 for Linux; H7 writes the sequence and lists the gaps |
| U6 | Which DDR may an image use beyond `0xd0000000` + 10 MiB, and what do TF-A, the SM and the boot firmware reserve? | H0 (Linux board DTs' `memory` and `reserved-memory`, TF-A's platform file); D2 prints the map |
| U7 | The A55 GIC numbers of the GPU interrupts (T: SPI 288-290). Only matters if we stop polling | H0; D1's `/proc/interrupts` |
| U8 | Which board, and what it changes: FRDM and Verdin have no Zephyr board at the pin; the Verdin brings Toradex's boot chain and its own SM build | the owner's choice; H0 for their DTs and SM configurations |
| U9 | Does a reduced G310 drop anything our back end relies on? Candidates: IDVS (used unconditionally, `src/mali/frame.rs:2074`), eight tiler levels (gated, :2620), the tile budget for four targets with MSAA on a 16 KiB variant. AFRC/AFBC are not used | H1 (the descriptors), H3 (what the budget rejects), D2 (the registers) |
| U10 | Is the GPU's rate ours to set (SCMI clock rate) with the voltage following inside the SM, or does the SM expose the GPU through the performance protocol only? | H0 (`imx-sm`, the Linux node's OPP handling); D2 |

The three most important are U1, U2 and U3. U1 decides whether our image
can power the GPU at all without Linux. U2 decides the tile budget and
which gates open. U3 decides whether the firmware half of our kernel side
carries over as it is.

## 8. Proposed items, ordered

Sizes are against the finished work in `tools/cts/STATUS.md`. The T6's
own kernel side went from the first power-on (Z1, 2026-10-19, repo:
`driver/docs/nanopc-t6-mali.md:89`) to the first submitted frame (Z4,
2026-11-12) and to `gl_test` 50/50 on Zephyr (2026-11-16) (`STATUS.md:17`).
The Zephyr dEQP-GLES2 tree runs end to end with 6,644 of 17,165 passing,
failures mostly the compiler's refusals (`STATUS.md:18`). Those refusals
are shared code, so their fixes carry to the i.MX 95 unchanged. The T6
has no display on Zephyr yet (`driver/docs/nanopc-t6-display.md:5`).

Host items (no board):

| # | Item | Size | Depends on | Done when |
| --- | --- | --- | --- | --- |
| P0 | H0, the reading pass, on a host with network | S | — | this note updated, "not read" emptied or justified; U1, U3 narrowed |
| P1 | H1, G310 references from the corpus | S | — | the difference table committed with the reference dumps (`tools/mali/`) |
| P2 | H2, kernel side parameterised, with the G310 mock | M | P1 (which fields matter) | host tests pass for both identities, T6 records unchanged |
| P3 | H3, `src/mali` from the model and variant | S-M | P1 | both identities tested; G610 frames byte-identical |
| P4 | H4, the SCMI SoC layer with the recording mock | S | P0 (the ids and rights) | the sequence pinned in a test |
| P5 | H5, board copy and the build-not-run rows | S | P2-P4 | gate rows green |
| P6 | H6, the arch 10.12 firmware entry and parser test | S | P0 | the parser test passes; the interface version recorded |
| P7 | H7, the display design | M | P0 | `docs/notes/imx95-display.md` and its probe's build row |

Board items:

| # | Item | Size | Depends on | Done when |
| --- | --- | --- | --- | --- |
| B1 | D1, the stock image's reading | S | the board | section 1.2 filled from the board |
| B2 | D2, identity and power through SCMI from our image | S | P2, P4, P5, B1 | `mali::ident` printed by our code |
| B3 | D3, firmware boot (the T6's Z3) | S-M | P6, B2 | the GLB answer recorded |
| B4 | M1, first submitted frame (the T6's Z4) | M | B3, P3 | read-back pixels as predicted |
| B5 | M2, `gl_test` on read-back to 50/50 | M | B4 | a row in `tools/cts/RESULTS.md` |
| B6 | M3/M4, display bring-up | L | P7, B2 | first clear, then the first triangle on the panel |
| B7 | M5, the direct route and the U-Boot sign-off image | S | B5 | both routes run the same image; sign-off recorded |
| B8 | the CTS stages on the board (dEQP-GLES2 tree as on the T6) | L (shared with the T6's compiler work) | B5 | the tree's tally in `RESULTS.md` |

Order logic: P0 comes first because U1 and U3 can turn the plan. A
System Manager that refuses our agent the GPU means a custom SM build or
a different route, and a firmware with another interface means CSF work
before B3. P1 to P5 can run beside P0. B6 does not block B5: as on the T6,
read-back carries the GL work until the display exists.
