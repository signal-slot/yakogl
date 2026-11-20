<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Verdin iMX95 on Mallow: the picture from Zephyr

Status: design (2026-11-20), host reading only. No i.MX 95 display code
exists and nothing here was run on a board. The board is fixed by the owner:
a Toradex Verdin iMX95 module on a Toradex Mallow carrier. The goal is the
other boards' screen path on it: a Zephyr image puts a GPU-rendered picture
on a display, with the console beside it, in the product form (U-Boot starts
Zephyr, no Linux before it). This note names the display hardware, what the
Mallow puts behind it, three routes in the order the house method takes them
(adopt, cold, none), the board checks that pick one, and what is still open.
The overall i.MX 95 pre-study is a separate note; this one is the display
only.

## Sources and rules

| Prefix | Tree or document | Version read |
|---|---|---|
| `ml:` | mainline Linux, `github.com/torvalds/linux` `master` | `65e4fd942f72` (2026-11-19) |
| `nxp:` | NXP's kernel, `github.com/nxp-imx/linux-imx` branch `lf-6.18.y` | `6e694eda40aa` (2026-09-08) |
| `tdx:` | Toradex's kernel, `git.toradex.com/linux-toradex.git` branch `toradex_6.18-2.1.x-imx` (it carries Toradex's overlays in the tree) | `851ac0f91f87` (2026-11-19) |
| `tub:` | Toradex's U-Boot, `git.toradex.com/u-boot-toradex.git` branch `toradex_imx_lf_v2026.04` | `f51b30894e05` (2026-10-28) |
| `nub:` | NXP's U-Boot, `github.com/nxp-imx/uboot-imx` branch `lf_v2026.04` | `bb41975aeb4c` (directory listing and one defconfig) |
| `mub:` | mainline U-Boot, the `github.com/u-boot/u-boot` mirror, `master` | `ece349ade297` (the mirror's last commit, 2026-07-06; directory listing only) |
| `hal:` | `github.com/zephyrproject-rtos/hal_nxp` `master` | `0f32f1caa5cf` (2026-11-19) |
| `z:` | `github.com/zephyrproject-rtos/zephyr` `main` | `4f2d5263e21e` (2026-11-20) |
| `sm:` | NXP's System Manager, `github.com/nxp-imx/imx-sm` `master` | `eec232878ab7` (2026-09-03) |
| `V95:` | Toradex, Verdin iMX95 Datasheet, Rev. 0.7, 20-Jul-2026 (preliminary), `docs.toradex.com/200007-verdin_imx95_datasheet.pdf` | |
| `MAL:` | Toradex, Mallow Carrier Board V1.1 Datasheet, Rev. 1.9, 24-Sep-2025, `docs.toradex.com/116791-mallow_carrier_board_datasheet.pdf` | |
| `repo:` | this repository | `f7ab6fa0` |

Device-tree paths are under `arch/arm64/boot/dts/freescale/` unless they
say otherwise. `nxp:` and `tdx:` carry the same `imx95.dtsi` display nodes at
the same line numbers; the note cites `nxp:`.

Licences, and what each source may be used for:

| Source | Licence | Use here |
|---|---|---|
| Linux drivers (`nxp:` `dpu95/`, the `imx95-*` bridges, `phy-fsl-imx8mp-lvds.c`; `ml:` `clk-imx95-blk-ctl.c`) | GPL-2.0 (`dpu95`, bridges, PHY: GPL-2.0+; the clock file GPL-2.0) | facts and `file:line` only, never code, comments or tables (`docs/source-form.md`) |
| U-Boot drivers (`tub:` `drivers/video/nxp/imx/imx95_dpu.c`, `imx95-ldb.c`, `imx95-pixel-*.c`, `dw_dsi_imx95.c`) | GPL-2.0+ | as Linux |
| Device trees (`ml:`/`nxp:` `imx95.dtsi`; `tdx:` overlays; `ml:` Verdin files) | `GPL-2.0-only OR MIT` (`imx95.dtsi`, the Toradex overlays), `GPL-2.0-or-later OR MIT` (the Verdin and Mallow files) | facts; the MIT side would allow more, not needed |
| `hal:` register headers (`PERI_DISPLAY_SEERIS.h`, `PERI_LDB.h`, `PERI_DISPLAY_*.h`, `PERI_MIPI_DSI.h`, `MIMX9596_ca55_COMMON.h`) and drivers (`fsl_dpu.c` in `drivers/dpu_1`, `fsl_ldb.c` in `drivers/ldb_1`) | BSD-3-Clause (NXP) | **register map and sequence source**; may be generated from or vendored unmodified, as the i.MX8MP's `regs.rs` is (`tools/hal-nxp/gen_regs.py`) |
| `z:` `display_socionext_dpu.c`, `dsi_nxp_dwc.c` and the i.MX 95 M7 device tree | Apache-2.0 | sequence source; may be read and vendored |
| `sm:` configuration | BSD-3-Clause | facts |
| Datasheets | Toradex documents | facts by section and page |

Not read: the i.MX 95 Reference Manual (IMX95RM). Its address
`nxp.com/docs/en/reference-manual/IMX95RM.pdf` answers "Not found" without an
NXP account; an account was not used. Marek Vasut's i.MX95 display series
says "the DPU chapter is missing from the i.MX95 RM" (cover letter,
2025-10-11, read through its LWN copy, `lwn.net/Articles/1041688/`); that is
his statement, not checked here. lore.kernel.org and patchwork were not
searched directly; the two threads below were read through mirrors (LWN,
and the dri-devel archive at `ratatoskr.run`). Toradex's System Manager
configuration was not found (`git.toradex.com/imx-sm.git` does not exist);
NXP's EVK configuration stands in for it. The Verdin iMX95 developer page was
read; the Mallow product page refused the fetch (HTTP 403), its datasheet was
read instead. Toradex's Dahlia and Development Board datasheets were not
read.

## 1. The display hardware

```
 DDR (linear XRGB8888; the DPU node has no IOMMU)
  |  AXI
  v
 DPU "Seeris" 0x4B400000 (4 MiB)                     display CSR 0x4B010000
  FetchLayer0 0x1D0000 --+                           +0x0 engine clock select (CCM | DSI PLL | LDB PLL/7)
  ConstFrame0 0x0F0000 --LayerBlend1 0x170000        +0x4 pixel interleaver control
                         -> ExtDst0 0x110000         +0x8 pixel link control
  display engine 0: DomainBlend0 0x2A0000 -> FrameGen0 0x2B0000 -> Signature0 0x2D0000
  display engine 1: DomainBlend1 0x320000 -> FrameGen1 0x330000
        | disp0, disp1
        v
 pixel interleaver 0x4B0D0000 (two channels, step 0x10000)
        |                                    \ (DSI only)
        v                                     pixel link -> camera/DSI CSRs 0x4AD00000.. -> DW MIPI DSI 0x4ACF0000
 LVDS CSR 0x4B0C0000: +0x0 PHY clock, +0x4 LDB (pixel mapper),     -> combo D-PHY (shared with CSI 1) -> DSI_1 pins
                      +0x8 LVDS0 PHY, +0xC LVDS1 PHY
        |
        v
 LVDS0 (ch 0), LVDS1 (ch 1) -> Verdin pins 88..144
 power: SCMI domain DISPLAY (13); the DSI side also CAMERA (3)
```

| Block | Base, size | Interrupt | Clocks (SCMI ids) | Power | Source |
|---|---|---|---|---|---|
| DPU (`nxp,imx95-dpu`; the M7 tree calls it `socionext,dpu`) | 0x4B400000, 0x400000 | through the display irqsteer, 86 lines (store9, extdst0/1/4/5, domainblend0, framegen0/1, sig0/2, ...) | `pix` DISP1PIX (80), `apb` DISPAPB (75), `axi` DISPAXI (76), `ocram` DISPOCRAM (78), `ldb` LDB PLL / 7, `ldb_vco` LDBPLL_VCO (37) | DISPLAY (13) | `nxp:imx95.dtsi:2584-2726` (clocks `:2696-2701`, domain `:2703`); `z:dts/arm/nxp/imx/nxp_imx95_m7.dtsi:1093-1099`; ids `ml:imx95-clock.h:46-47,87-92`, `ml:imx95-power.h:22` |
| display irqsteer | 0x4B0B0000, 0x1000 | GIC SPI 214-219 (two slots reserved) | DISPAPB | DISPLAY | `nxp:imx95.dtsi:2405-2424` |
| display CSR (`nxp,imx95-display-csr`, the blk-ctrl) | 0x4B010000, 0x10000 | | DISPAPB; sets DISPOCRAM 400 MHz, DISPAPB 133 MHz | DISPLAY | `nxp:imx95.dtsi:2331-2342`; registers `hal:mcux/mcux-sdk/devices/MIMX9596/periph/PERI_DISPLAY_BLK_CTRL_DISPLAYMIX.h:408-415` |
| pixel link (`nxp,imx95-dc-pixel-link`, a register of the display CSR) | display CSR + 0x8 | | | | `nxp:imx95.dtsi:2345-2403` |
| pixel interleaver | 0x4B0D0000, 0x20000 | irqsteer 62 | DISPAPB | | `nxp:imx95.dtsi:2526-2582`; `hal:.../PERI_DISPLAY_INTERLEAVER.h:411-421` |
| LVDS CSR (`nxp,imx95-lvds-csr`), LDB at +0x4, LVDS PHY 0 at +0x8, PHY 1 at +0xC | 0x4B0C0000, 0x10000 | | DISPAPB; LDBPLL (38), LDB PLL / 7 (a fixed /7 of LDBPLL) | DISPLAY | `nxp:imx95.dtsi:341-348,2426-2524`; `hal:.../PERI_LDB.h:408-412`; base `hal:mcux/mcux-sdk/devices/MIMX9596/MIMX9596_ca55_COMMON.h:1817` |
| MIPI DSI host (Synopsys DW MIPI DSI, NXP glue `nxp,imx95-mipi-dsi`) | 0x4ACF0000, 0x10000 | irqsteer 48 | MIPIPHYCFG (66), MIPIPHYPLLBYPASS (67), MIPIPHYPLLREF (68), CAMAPB (62), DISP1PIX (80) | **CAMERA (3)** | `nxp:imx95.dtsi:2104-2127` (domain `:2122`); `nxp:drivers/gpu/drm/bridge/imx/imx95-mipi-dsi.c:23` (the DW core); `hal:.../PERI_MIPI_DSI.h` |
| DSI stream CSR, DSI master CSR (pixel link select at +0x4), combo PHY CSR, MIPI TX PHY CSR | 0x4AD00000, 0x4AD10000, 0x4AD20000 (0x114), 0x4AD20100 (0x14) | | CAMAPB | CAMERA | `nxp:imx95.dtsi:2155-2225`; `hal:.../MIMX9596_ca55_COMMON.h:810-830` |

The DPU's units at their offsets from 0x4B400000, as both GPL drivers and
the BSD header give them: ConstFrame 0x0F0000/0x100000/0x130000/0x140000,
ExtDst 0x110000/0x120000/0x150000/0x160000, LayerBlend 0x170000..0x1C0000,
FetchLayer 0x1D0000/0x1E0000, FetchYUV 0x200000.., FetchEco 0x210000..,
DomainBlend 0x2A0000/0x320000, FrameGen 0x2B0000/0x330000, Signature 0x2D0000
and 0x2E0000 (`nxp:drivers/gpu/drm/imx/dpu95/dpu95-data.h:16-17,54,87-88,107-108,126,141,159-160,196-197`;
`tub:drivers/video/nxp/imx/imx95_dpu.c:669-688,1031-1052`;
`hal:mcux/mcux-sdk-ng/devices/i.MX/i.MX95/periph/PERI_DISPLAY_SEERIS.h:1063-1081,1700-1745,1833-1842`).

Three facts change the house method for this SoC:

- **A permissive register map exists for every block.** hal_nxp (BSD-3)
  has `PERI_DISPLAY_SEERIS.h` for the i.MX 95: 38,294 lines, rev. 4.0 of
  2026-02-28, every DPU register with its fields and a one-line meaning
  (FetchLayer0 `BASEADDRESS0` 0x1D0018 and `BASEADDRESSMSB0` 0x1D001C,
  `SOURCEBUFFERATTRIBUTES0` 0x1D0028, `LAYERPROPERTY0` 0x1D0048; FrameGen0
  `HTCFG1` 0x2B000C, `FGENABLE` 0x2B008C, `FGTIMESTAMP` 0x2B0098, `FGCHSTAT`
  0x2B009C with `PrimSyncStat` bit 8; `PERI_DISPLAY_SEERIS.h:1063-1081,1708-1745,23394`).
  The LVDS CSR, the display CSR, the interleaver, the pixel mapper and the
  DSI host have their own BSD headers. The i.MX8MP display was written from
  its RM and hal_nxp (`driver/docs/imx8mp-display-cold.md`); the i.MX 95 can
  be written from hal_nxp alone, with the GPL drivers only to cross-check.
- **A permissive driver exists for the DPU and the LDB.** hal_nxp's
  `drivers/dpu_1/fsl_dpu.c` (4,918 lines, BSD-3) drives `DISPLAY_SEERIS_Type`
  (`DPU_Init` `:1074`, `DPU_InitFetchUnit` `:1873`, `DPU_InitDisplayTiming`
  `:2303`, `DPU_StartDisplay` `:2466`); `drivers/ldb_1/fsl_ldb.c` (96 lines,
  BSD-3) is the i.MX 95 LVDS sequence (`:33-95`). Upstream Zephyr drives the
  i.MX 95 DPU from the M7 with them: `z:drivers/display/display_socionext_dpu.c`
  (387 lines, Apache-2.0, FetchYUV0, LayerBlend1, ExtDst0, FrameGen0,
  `:209-330`) and `z:drivers/mipi_dsi/dsi_nxp_dwc.c` (the DSI host, combo PHY
  and pixel link, `:109-126`), shown on a Raydium RM692C9 DSI panel
  (`z:boards/shields/nxp_mx9_dsi_oled/`). Zephyr has no LVDS path and no
  Verdin iMX95 board.
- **The DPU has a signature unit.** Signature0 at 0x2D0000 measures a CRC per
  colour channel over up to several windows of the output
  (`CRC_R/G/B_WINDOW0` at 0x2D0400-0x2D0408, reference values and a status
  register; `PERI_DISPLAY_SEERIS.h:1833-1842,1960-1965`). No driver read uses
  it. It is the nearest thing to a camera this display has (section 4).

The SCMI side: the i.MX 95 runs a System Manager on the M33. Linux and
U-Boot reach it over MU2 (0x445B0000) with shared memory in its SRAM
(`ml:imx95.dtsi:338-340,1590-1612`); power domains are SCMI protocol 0x11,
clocks 0x14. In NXP's EVK configuration the A55 logical machine "AP" owns the
DC, the display blk-ctrl, LVDS, MIPI DSI and its PHY, and VIDEO_PLL1, and may
set `CLOCK_DISP1PIX` and `LDB_PLL` (`sm:configs/mx95evk.cfg:443,584,596,608,625-627,798,801-802,886`).
U-Boot, Linux and our image all run in that one logical machine, so a
hand-over between them is not an agent leaving: the System Manager turns a
logical machine's resources off when that machine is reset, not when one
program in it jumps to the next (to be confirmed on the board, U2).

## 2. The Mallow, and the other boards

What the Verdin iMX95 brings to the Verdin edge connector:

- **No HDMI.** The `HDMI_1_*` pins are "Not connected. HDMI not supported on
  Verdin i.MX 95" (`V95:` section 3.2, Table 8, page 17); "HDMI port is not
  natively supported ... can be enabled with a DSI-to-HDMI bridge solution on
  the carrier board" (`V95:` 5.4, page 40).
- **One 4-lane MIPI DSI** on `DSI_1` (pins 23-49), straight from the combo
  D-PHY that the DSI shares with one CSI-2 receiver; no multiplexer on the
  module, the mode chosen by `Tx_Rxz` in `COMBO_PHY_MODE_CONTROL`
  (`V95:` 5.4.1, Table 22, pages 40-41). Backlight `PWM_3_DSI` (pin 19,
  GPIO2_IO19), enable `GPIO_10_DSI` (pin 21) through the module's GPIO
  expander (Table 22, page 41).
- **Two native LVDS channels**: LVDS0 on pins 88-114, LVDS1 on pins 118-144
  (module-specific pins), single or dual channel, 18 or 24 bit; "both LVDS
  displays must use the same resolution and timing" (`V95:` 5.4, 5.4.2,
  Tables 23-24, page 42).
- The display I2C buses: `I2C_2_DSI` is LPI2C3 (pins 53/55), `I2C_3_HDMI` is
  I3C2 (unused here), `I2C_1` is LPI2C4 (pins 12/14)
  (`ml:imx95-verdin.dtsi:387-392,467-486`).

### The Mallow

| Mallow connector | Signals | What sits between the SoC and it | I2C | What the SoC must drive | Source |
|---|---|---|---|---|---|
| X12 LVDS, Hirose DF13 40-pin; X13 touch (FFC 10-pin) | `LVDS_1_A` = Verdin pins 88-114 = **LVDS0**; `LVDS_1_B` = pins 118-144 = LVDS1; backlight PWM pin 35 (Verdin 16, `PWM_2`), enable pin 37 (Verdin 46); +12 V backlight through jumper JP1 | **nothing**: native LVDS, no bridge | touch only (the panel needs none) | DPU, interleaver, LDB, LVDS PHY 0, LDB PLL; TPM5 PWM and one GPIO for the backlight | `MAL:` 2.9.3, Tables 16-17, pages 24-25; `V95:` Table 23 |
| X4 MIPI DSI, Würth 34-pin FFC (Toradex "Simple MIPI DSI") | `DSI_1` clock and 4 lanes, `I2C_DSI` (Verdin 53/55), `GPIO_10_DSI`, `PWM_3_DSI`, `GPIO_9_DSI`, reset, 1.8/3.3/5 V | whatever the display carries: Toradex's 7" and 10.1" DSI displays carry a **TI SN65DSI83** DSI-to-LVDS bridge with a Riverdi LVDS panel | SN65DSI83 at 0x2C on `I2C_2_DSI` (LPI2C3) | DPU, interleaver, pixel link, DW MIPI DSI, combo PHY, then the bridge over I2C | `MAL:` 2.9.1, Table 14, pages 22-23; `tdx:imx95-verdin-panel-cap-touch-10inch-dsi.dtso:33,82-88`, `tdx:imx95-verdin-panel-cap-touch-7inch-dsi.dtso:33,82-88` |
| X14 HDMI | fed from the Verdin `HDMI_1` pins | nothing, and the module leaves those pins unconnected | | **dead with this module** | `MAL:` 2.9.2, Table 15, pages 23-24; `V95:` Table 8 |

So the Mallow has no HDMI with this module. The shortest path to a picture
is **X12 with an LVDS panel**: no bridge, no I2C, no DSI PHY, one power
domain. Toradex's overlay for it (`imx95-verdin-panel-cap-touch-10inch-lvds.dtso`,
"carrier boards with a Toradex standard LVDS display connector (e.g.,
Mallow)", `:3-6`) names the whole chain: panel 1280x800 at 71.1 MHz, VESA
24-bit (`:34-62`); DPU, pixel link, interleaver channel 0, LDB channel 0,
LVDS PHY 0 (`:65-106,126-137`); LDBPLL_VCO 2,986.2 MHz and LDBPLL 497.7 MHz
("LDBPLL_VCO = 6 * 7 * pixelclk, LDBPLL = 7 * pixelclk", `:80-83`);
backlight `PWM_2` on TPM5 and enable `I2S_2_D_OUT` (GPIO5_IO04) (`:22-32`).
The pixel link enabled there is questioned upstream: Liu Ying (NXP) says the
pixel link belongs only to the DSI path (dri-devel RFC thread, 2026-07-06);
U5 below.

Mainline's Mallow file enables `I3C2`, `LPI2C3` and `LPI2C4` and puts only a
temperature sensor (0x4F) and the carrier EEPROM (0x57) on `I2C_1`
(`ml:imx95-verdin-mallow.dtsi:107-131`); no display node is in mainline for
any Verdin carrier.

### The other boards, for comparison

| Board | Display connectors | Between SoC and connector | I2C, address | Source |
|---|---|---|---|---|
| Verdin Development Board | X48 `DSI_1` (Samtec LSS-130); J10 "Verdin Mezzanine LVDS" | DSI: Toradex adapters (DSI to HDMI: Lontium LT8912B; DSI to LVDS: TI SN65DSI84; DSI display adapter to the DSI displays above); LVDS: native | LT8912B 0x48 and SN65DSI84 0x2C on `I2C_1` (LPI2C4) | `tdx:imx95-verdin-dsi-to-hdmi.dtso:5-7,65-75`; `tdx:imx95-verdin-dsi-to-lvds-panel-cap-touch-10inch.dtso:95-101`; `tdx:imx95-verdin-mezzanine-panel-cap-touch-10inch-lvds.dtso` header |
| Dahlia | X17 `DSI_1` (LSS-130) | the same DSI adapters | as above | the same overlays' headers |
| Ivy, Yavia, Zinnia | not studied (Ivy and Zinnia enable neither `I2C_2_DSI` nor `I2C_3_HDMI` in mainline) | | | `ml:imx95-verdin-ivy.dtsi`, `-zinnia.dtsi` |
| NXP i.MX 95 EVK (19x19) | LVDS through NXP's LVDS-to-HDMI adapter; DSI through an ADV7535 adapter | ITE IT6263; ADV7535 | IT6263 0x4C on LPI2C3; ADV7535 0x3D on LPI2C2 | `nxp:imx95-evk-it6263-lvds0-common.dtsi:29-34`; `nxp:imx95-19x19-evk-adv7535.dtso:14-19` |
| NXP FRDM i.MX 95 (15x15) | HDMI on the board | IT6263 on LVDS channel 1 | 0x4C on LPI2C4 | `nxp:imx95-15x15-frdm.dts:343-357,497-526` |

The FRDM's on-board IT6263 is the only HDMI path among the candidates that
needs no adapter; it is also the one path validated by the upstream port
(below). For the Mallow it does not matter.

### What of our code carries over

| Ours | From | On the Mallow |
|---|---|---|
| `ip::sn65dsi8x` (`driver/rust/src/ip/sn65dsi8x.rs`) | the Verdin AM62P's SN65DSI83 and the Verdin iMX8MP's SN65DSI84 (`driver/docs/am62p-display-cold.md`, `driver/docs/imx8mp-display-cold.md`) | **yes**, for a Toradex DSI display on X4 (SN65DSI83 at 0x2C on `I2C_2_DSI`) |
| `ip::dw_mipi_dsi` (`driver/rust/src/ip/dw_mipi_dsi.rs`) | the STM32MP25's Synopsys DW MIPI DSI host | **the host part**, for X4: the i.MX 95 DSI is the same DW core (`nxp:drivers/gpu/drm/bridge/imx/imx95-mipi-dsi.c:23`). Its PHY (NXP combo PHY, CSRs at 0x4AD00000..) is new |
| `firmware::scmi` (`driver/rust/src/firmware/scmi.rs`) | the STM32MP2 over OP-TEE | **the codec and client** (power, clock, perf); the `Transport` trait (`:127`) needs an i.MX MU-plus-SRAM transport |
| `ip::adv7535` | the STM32MP257F-DK | no (NXP's EVK adapter only) |
| `ip::lcdifv3`, `ip::samsung_dsim`, `ip::cdns_dsi`, `ip::ti_dss7` | i.MX8MP, AM62P | no: different controller and DSI host |
| LT8912B | none in the tree | only for the DSI-to-HDMI adapter, which does not fit the Mallow's X4 |

The Mallow X12 panel path needs no bridge code at all.

## 3. What exists upstream, and what U-Boot leaves

| Where | State | Source |
|---|---|---|
| mainline Linux | **no i.MX 95 display.** `imx95.dtsi` has no display node; the clock and power ids exist; the blk-ctrl clock driver knows `nxp,imx95-display-csr`, `-lvds-csr` and `-display-master-csr`; `drivers/gpu/drm/imx/dc/` matches only `fsl,imx8qxp-dc`; `fsl-ldb.c` has no i.MX 95 entry | `ml:imx95.dtsi` (grep); `ml:drivers/clk/imx/clk-imx95-blk-ctl.c:163-257,532-537`; `ml:drivers/gpu/drm/imx/dc/dc-drv.c:247`; `ml:drivers/gpu/drm/bridge/fsl-ldb.c:381-385` |
| posted series | Marek Vasut, "[PATCH 00/39] Add i.MX95 DPU/DSI/LVDS support", 2025-10-11: the DPU in the `dc` driver, tested on a "Toradex i.MX95 Verdin EVK v1.2" (DSI through LT8912, single LVDS panel); review asked for a separate module; not merged. Piyush Patle, "[RFC] drm/imx: upstream direction for i.MX95 display support", 2026-06-24: an LVDS-only port validated on the FRDM (IT6263, channel 1, 1080p60); open issues: a full modeset wedges the ExtDst shadow load (FrameGen must be disabled and re-enabled), the clock list is wrong, the two display engines are not modelled; no series posted by 2026-10-18 | `lwn.net/Articles/1041688/`; the dri-devel thread (read through `ratatoskr.run/dri-devel/2026/06/17170118/t`) |
| NXP's kernel | the full stack: `dpu95/` (11,743 lines; 3,383 of them the blitter, local dimming and other SoCs' data), `imx95-pixel-interleaver.c` (402), `imx95-pixel-link.c` (315), `imx95-ldb.c` (578), `imx95-mipi-dsi.c` (1,348), the LVDS PHY in `phy-fsl-imx8mp-lvds.c` (317, shared with i.MX8MP/93) | `nxp:drivers/gpu/drm/imx/dpu95/`, `nxp:drivers/gpu/drm/bridge/imx/`, `nxp:drivers/phy/freescale/phy-fsl-imx8mp-lvds.c:297-299` |
| Toradex's kernel | NXP's stack plus the Verdin overlays (section 2) | `tdx:` |
| mainline U-Boot | no i.MX 95 video: `drivers/video/imx/` holds IPU, LCDIF and LDB of older SoCs; no Verdin iMX95 defconfig | `mub:drivers/video/imx/`, `mub:configs/` |
| NXP's U-Boot | i.MX 95 video drivers (`imx95_dpu.c`, `imx95-ldb.c`, the interleaver and pixel link, `dw_dsi_imx95.c`) and EVK defconfigs that light a panel (`imx95_19x19_evk_lvds_defconfig`: `VIDEO_IMX95_DPU`, `_LVDS`, `PHY_IMX8MP_LVDS_PHY`, `SPLASH_SCREEN`) | `nub:drivers/video/nxp/imx/`, `nub:configs/imx95_19x19_evk_lvds_defconfig` |
| **Toradex's U-Boot** | carries the same drivers but **does not use them**: `verdin-imx95_defconfig` enables no video; its device tree includes the upstream `imx95.dtsi`, which has no display node; the only Verdin iMX95 device tree in it is `imx95-verdin-wifi-dev` | `tub:configs/verdin-imx95_defconfig:13,41`; `tub:arch/arm/dts/imx95-verdin.dtsi:12` |
| Zephyr | DPU and DSI from the M7 (section 1); no A55 display, no LVDS, no Verdin iMX95 | `z:` |

The U-Boot DPU driver is the shortest whole sequence read: 1,082 lines for
one plane (FetchLayer0 into LayerBlend1 over ConstFrame0, ExtDst, DomainBlend,
FrameGen; XRGB8888; the display CSR clock select `DSIP_CLK_SEL` = 2 for LVDS
PLL / 7) (`tub:drivers/video/nxp/imx/imx95_dpu.c:331-334,543-599,716-822`).
Its `remove` stops the FrameGen and the fetch (`:1017-1029`); the driver
sets only `DM_FLAG_PRE_RELOC` (`:1080`).

## 4. Routes

### (a) Adopt a display something else left running

What the scanout looks like from the CPU, from the BSD header: FrameGen0
`FGENABLE` (0x4B6B008C) bit 0 set, `FGCHSTAT` (0x4B6B009C) `PrimSyncStat`
set, `FGTIMESTAMP` (0x4B6B0098) moving; the plane is the fetch unit the
previous stage used: FetchLayer0 `BASEADDRESS0`/`MSB0` (0x4B5D0018/1C),
`SOURCEBUFFERATTRIBUTES0` (stride and bits per pixel, 0x4B5D0028),
`SOURCEBUFFERDIMENSION0` (0x4B5D002C), `LAYERPROPERTY0` (source buffer enable,
0x4B5D0048), or FetchYUV0 for Zephyr's own M7 driver. The active size is
FrameGen0 `HTCFG1`/`VTCFG1`. Writing pixels into that buffer and reading them
back is the whole adopt. Nothing may be read before the DISPLAY domain is
known to be on (U6).

Who could leave one on the Mallow:

| Previous stage | Leaves a display? | Note |
|---|---|---|
| Toradex's U-Boot as shipped | **no** (section 3) | |
| a Toradex U-Boot rebuilt with NXP's video drivers and a Mallow LVDS device tree | yes, by NXP's EVK precedent | a boot loader of our own to carry, as the T6 note concluded for Rockchip's (`driver/docs/nanopc-t6-display.md`, 2(b)); whether `go` leaves the FrameGen running is open (U9). Not proposed |
| Linux (Toradex's 6.18 BSP with the LVDS overlay), then a crash kexec into Zephyr | yes, if the board runs that kernel with a panel on X12 | the T6's development route (`driver/docs/nanopc-t6-display.md`, 2(a)); the crash path skips the drivers' shutdown, so the DPU keeps scanning; the logical machine is not reset (section 1). Needs kexec and a crash reservation in that kernel (U8) |
| mainline Linux | no (no driver) | |

Needs: a SCMI transport only to read the DISPLAY domain state first; no
clock or power change. Size: small, as on the T6: a reader of a dozen
registers, a pattern, a read-back. Value: it gives the golden dump (the
register values of a working LVDS mode on this board) that the cold path is
checked against, and a known-good link for the proofs below. It is
development only; it is not the product.

### (b) Cold: X12 LVDS from Zephyr (the product form)

The list, in the order U-Boot's and hal_nxp's sequences agree on
(`tub:drivers/video/nxp/imx/imx95_dpu.c:880-1006`;
`hal:mcux/mcux-sdk-ng/drivers/ldb_1/fsl_ldb.c:33-95`;
`z:drivers/display/display_socionext_dpu.c:256-327`):

| Step | Block | Registers (BSD header) | Permissive sequence | GPL cross-check (lines) |
|---|---|---|---|---|
| 1 | SCMI: DISPLAY (13) on; DISPAPB (75), DISPAXI (76), DISPOCRAM (78) on; LDBPLL_VCO (37) 2,986.2 MHz, LDBPLL (38) 497.7 MHz for 71.1 MHz | (SCMI) | `repo:driver/rust/src/firmware/scmi.rs` codec; a new MU2 transport | `nxp:imx95.dtsi:2696-2703`; `tdx:...10inch-lvds.dtso:80-83` |
| 2 | display CSR: engine 0 clock select = LDB PLL / 7; interleaver control | `PERI_DISPLAY_BLK_CTRL_DISPLAYMIX.h:408-415,430-468` | | `ml:clk-imx95-blk-ctl.c:226-251`; `tub:imx95_dpu.c:595-598` |
| 3 | LVDS CSR: PHY clock divider, the four clock gates (U4), LDB control (channel 0 to DI0, 24 bit, SPWG or JEIDA, sync polarities, `VSYNC_ADJ_EN`), LVDS PHY 0 enable | `PERI_LDB.h:408-412` and its field masks | `fsl_ldb.c:33-95` (96 lines) | `nxp:imx95-ldb.c` (578), `phy-fsl-imx8mp-lvds.c` (317); `tub:imx95-ldb.c` (551) |
| 4 | pixel interleaver channel 0: reset, polarities, format | `PERI_DISPLAY_INTERLEAVER.h:411-421` | | `nxp:imx95-pixel-interleaver.c:24-43,84-147` (402); `tub:` (283) |
| 5 | DPU: one plane (FetchLayer0 or FetchYUV0), LayerBlend1 over ConstFrame0, ExtDst0, DomainBlend0, FrameGen0 timing and kick, shadow loads, FrameGen enable, wait for `PrimSyncStat` | `PERI_DISPLAY_SEERIS.h` | `fsl_dpu.c` (`dpu_1`, 4,918 lines for the whole DPU; the one-plane subset Zephyr uses is about 120 lines of calls, `display_socionext_dpu.c:256-327`) | `tub:imx95_dpu.c` (1,082); `nxp:dpu95/` (about 8,400 lines without the blitter, LD and other SoCs) |
| 6 | backlight: TPM5 PWM on `PWM_2`, GPIO5_IO04 high | (TPM, GPIO) | | `tdx:...10inch-lvds.dtso:22-32` |

Not needed for X12: the pixel link (U5), the DSI host and combo PHY, the
CAMERA domain, any I2C. Not needed for a first picture: the irqsteer (poll
`FGCHSTAT`), the signature unit.

Expected size, on the AM62P and i.MX8MP precedents: a register module
generated from hal_nxp as `regs.rs` is today (new header list in the
generator), about 600 lines of Rust for the DPU one-plane path, about 200 for
LDB, PHY and interleaver, about 300 for the MU transport and the SCMI calls,
a board layer, and host tests against the golden dump. One more open choice:
vendoring `fsl_dpu.c`/`fsl_ldb.c` unmodified under `third_party/` and calling
them, as the AM62P vendors TI's Zephyr DSI driver
(`driver/docs/am62p-display-cold.md`, "Documents"), against writing the
subset in Rust from the header. The house rule (`docs/source-form.md`)
allows either; the Rust subset is smaller than the 4,918-line driver.

The DSI variant (X4 with a Toradex DSI display) adds: the CAMERA domain and
four more clocks, the pixel link and DSI master CSR, the DW MIPI DSI host
(`ip::dw_mipi_dsi`), the NXP combo PHY and its PLL (`z:dsi_nxp_dwc.c:109-126`
and the BSD CSR headers; `nxp:imx95-mipi-dsi.c`, 1,348 lines;
`tub:dw_dsi_imx95.c`, 1,146 lines), and `ip::sn65dsi8x` over LPI2C3. Roughly
twice the LVDS path, most of it already ours.

Risks:

| Risk | Note |
|---|---|
| The System Manager refuses a clock rate or a domain to our agent (Toradex's configuration unknown) | SCMI returns `DENIED`; read under Linux first (M3) |
| LDB PLL rate: the VCO must be 6 x 7 x the pixel clock; the PLL may not reach a mode's exact rate | the Toradex panel's rate is the overlay's; other panels are later |
| A full mode change wedges the ExtDst shadow load unless the FrameGen is disabled and re-enabled (the upstream port's open defect) | set the mode once, at start; flips change only the fetch address |
| Reading DPU registers with DISPLAY off | U6; the probe reads the domain state first |

### (c) No display at first

The GPU work does not wait for a display. As on the NanoPC-T6 before its
display existed, `gl_test`, dEQP and glmark2 render into off-screen targets
and check by reading back (`driver/docs/nanopc-t6-mali.md`: pixel read-back
on every case). What it gives for sign-off: every GL result and every number
except "a picture on the panel". What it does not: the console screen, the
scan-out view, and the display item of the product form. Size: nothing new
for the display. It is not a route to a picture; it is what runs while the
display is open.

### Proof without a camera

From the BSD header, all readable by the image: `FGENABLE` and `FGENSTS`
(the FrameGen runs), `FGCHSTAT.PrimSyncStat` (the primary channel is in
sync), `FGTIMESTAMP` read twice (frames advance), FetchLayer0's address,
stride and size read back equal to our buffer, the LDB and PHY control words
equal to the golden dump. Beyond what the other boards had: **Signature0**
gives per-window CRCs of what the display engine actually sends; a CRC that
is stable while the buffer is unchanged, changes when one pixel changes, and
equals the CRC measured under Linux for the same pattern proves the content
without a camera (U7). A person at the panel confirms; the farm has no
capture device for an LVDS panel.

## 5. Recommendation

1. **(c) from day one**: the GPU bring-up and its sign-off do not depend on
   the display.
2. **The day-one reads (section 6)**, which say whether a panel is attached
   and whether the board's Linux drives it.
3. **(a) by crash kexec, if M1-M2 show Toradex's kernel driving a panel on
   X12**: a few days, and it yields the golden dump and a working reference
   link. Skip it otherwise; it is not worth building a Linux setup for.
4. **(b) cold on X12** from hal_nxp's headers and drivers, checked against
   the dump: the product form, signed off with U-Boot starting the image.

Why LVDS on X12 and not DSI on X4: no bridge, no I2C, no PHY PLL, one power
domain, and a permissive LDB sequence of 96 lines; X14 is dead on this
module. Why cold rather than a U-Boot that lights the panel: Toradex's U-Boot
does not, and carrying our own U-Boot build for it is the cost the T6 note
refused.

What would change it:

- **No LVDS panel on the board, a Toradex DSI display on X4 instead**: (b) in
  its DSI variant; `ip::dw_mipi_dsi` and `ip::sn65dsi8x` carry over, the
  combo PHY is new.
- **No panel at all**: the owner chooses a panel (Toradex's 10.1" LVDS
  display fits X12 and X13 directly); until then only (c).
- **An HDMI monitor is required**: the Mallow cannot give it with this
  module. It needs a DSI-to-HDMI device on X4 that the Mallow's FFC takes
  (Toradex's own adapter has the LSS-130 connector of the Development Board
  and Dahlia), or another carrier; then new bridge code (LT8912B or the
  device's own).
- **The board runs mainline Linux or no Linux**: no adopt; (b) without a
  golden dump, from the header and the permissive drivers alone, with the
  signature unit and the FrameGen status as the proof.
- **The System Manager denies the display clocks to the AP**: nothing works
  until Toradex's configuration is changed; that is a firmware question for
  the owner.

## 6. Board checks for day one (reads only)

Under the board's Linux, over ssh, nothing written:

| Check | What is read | What it gives |
|---|---|---|
| M1 | `/proc/device-tree/model` and `compatible` (expected "Toradex Verdin iMX95 ... on Mallow Board"); `uname -r`; `/etc/os-release`; the applied overlays (Toradex's `overlays.txt`) | the carrier; whether the kernel is Toradex's BSP (has `dpu95`) or mainline (has no display) |
| M2 | `/sys/class/drm/*/status` and `modes`; `dmesg` lines of `dpu95`, `imx95-ldb`, `panel`; `i2cdetect -r -y` on the `I2C_2_DSI` bus: 0x2C (SN65DSI83, a DSI display on X4), 0x4A or 0x41 (a display's touch controller) | **is a panel attached, on X12 or X4, and does Linux drive it?** |
| M3 | `dmesg` SCMI and System Manager lines (protocol versions, firmware build); `/sys/kernel/debug/clk/clk_summary` rows for `dispapb`, `dispaxi`, `dispocram`, `disp1pix`, `ldbpll`, `ldbpll_vco`, `videopll1`; `/sys/kernel/debug/pm_genpd/pm_genpd_summary` for display and camera; `/proc/config.gz` for `KEXEC`, `CRASH_DUMP`; `/proc/iomem` for a crash reservation | what the System Manager lets the AP set; whether (a) can run |
| M4 | only if M2 shows a running panel and M3 the display domain on: one 32-bit read each of DPU `COMCTRL_IPIDENTIFIER` (0x4B400000); FetchLayer0 0x4B5D0008-0x4B5D004C and FetchYUV0's first block; ExtDst0 0x4B510008-0x4B510014; FrameGen0 0x4B6B0008-0x4B6B00A0; Signature0 0x4B6D0008-0x4B6D0028 and 0x4B6D0400-0x4B6D0408; display CSR 0x4B010000-0x4B010020; LVDS CSR 0x4B0C0000-0x4B0C000C; interleaver 0x4B0D0000, +0x20, +0x50 | the golden dump, with an allow list as the T6's and X2's (`driver/docs/nanopc-t6-display-allow.txt`) |

Under U-Boot, at its prompt: `dm tree` (no `video` device expected),
`bdinfo` (DRAM banks, where a buffer may live). No register reads: U-Boot
has not powered the display domain.

Under Zephyr (the first probe image, later):

| Check | What the image does | Answers |
|---|---|---|
| Z0 | SCMI base discovery over MU2 (agent id, protocols); power state of domains 13 and 3; rate and state of clocks 75, 76, 78, 80, 37, 38 | does our image reach the System Manager, and is the display domain on? |
| Z1 | only if Z0 says DISPLAY is on: FrameGen0/1 enable and status, the fetch unit's address, stride and size | after a crash kexec: **a scanout to adopt?** |
| Z2 | the cold X12 path of section 4(b), stage by stage, with the read-backs of "Proof without a camera" | **our own picture** |

Which result picks which route:

| M2: panel driven by Linux on X12 | M3: kexec and crash reservation | Route |
|---|---|---|
| yes | yes | (a) by crash kexec for the dump and the reference, then (b) |
| yes | no | the dump from M4, then (b) directly |
| a DSI display on X4 | either | (b), DSI variant |
| no panel | | (c) only; ask the owner for a panel |

## 7. Unknowns

| # | Question | What answers it |
|---|---|---|
| U1 | What display is on the Mallow: an LVDS panel on X12, a DSI display on X4, nothing? | M2, or the owner |
| U2 | Does Toradex's System Manager configuration give the AP the display domain, `LDB_PLL` and `CLOCK_DISP1PIX` as NXP's EVK configuration does, and does it leave them on when U-Boot jumps or Linux crash-kexecs? | M3 under Linux; Z0 under Zephyr; Toradex's configuration source if it is published |
| U3 | Which Linux does the board run: Toradex's BSP (with `dpu95` and the LVDS overlay) or mainline? | M1 |
| U4 | The LVDS CSR word at +0x0: Linux's clock driver has the PHY divider in bit 0 and four set-to-disable gates in bits 1-4 (`ml:clk-imx95-blk-ctl.c:163-218`); hal_nxp's `PERI_LDB.h` names only the divider bit. Which is right? | M4 value under a running panel |
| U5 | Is the pixel link part of the LVDS path? Toradex's LVDS overlay enables it; NXP's Liu Ying says it is DSI only | M4 read of display CSR + 0x8 under a running LVDS panel; a cold run without it |
| U6 | What does a read of a DPU register do with the DISPLAY domain off: a bus error, a stall, zero? | do not try it; Z0 reads the domain first |
| U7 | Is Signature0's CRC usable as a proof: stable over frames, sensitive to one pixel, equal to the value under Linux for the same pattern? Its CRC polynomial is not stated in the header | a board run of the signature unit in Z2, and the same windows under Linux |
| U8 | Does Toradex's kernel support kexec with a crash reservation on the i.MX 95 (as Armbian's does on the T6)? | M3 |
| U9 | If a video U-Boot were ever used: does `go` or `booti` leave the FrameGen running, or does pre-boot device removal run the driver's `remove`? | not on the recommended path; a U-Boot run only if (a) through U-Boot is ever wanted |
| U10 | The touch controller's bus: the Mallow wires X13's I2C to Verdin pins 12/14 (`I2C_1`, LPI2C4, `MAL:` Table 17), Toradex's overlay puts the touch on LPI2C3 (`tdx:...10inch-lvds.dtso:108-123`). No effect on the picture | M2 `i2cdetect` on both buses |
| U11 | Does the IMX95RM, with an NXP account, have the DPU chapter, and is it needed now that hal_nxp's header exists? | someone with an account reads its table of contents; until then the header is the documentation |
| U12 | hal_nxp's `DPU_SetFetchUnitSrcBufferAddr` takes a 32-bit address (`fsl_dpu.c:2518`); the hardware has an MSB register. Is the A55's frame buffer below 4 GiB on this board? | the image's memory map (DRAM starts at 0x80000000; its first 16 MiB are reserved for the M7, `ml:imx95-verdin.dtsi:162-165`); `bdinfo` |
