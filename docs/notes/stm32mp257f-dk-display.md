<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# STM32MP257F-DK: the display path, LTDC to HDMI (P8 groundwork)

**Status (2026-11-12): a host study from ST's sources; nothing here has run
on the board.** It is the design note a later port (roadmap P8, pre-study Q7,
issues #1281 and #7) relies on: what the path is made of, which register
blocks and clocks it needs, who owns each of them, and the least that puts
one GPU-rendered buffer on the HDMI connector. No code changed. RM0457 (the
reference manual) is **unread**: every register name below is the name ST's
drivers use, and every "derived" fact is marked.

## Sources, and how to read the citations

| Tag | Tree | Note |
| --- | --- | --- |
| **L** | ST Linux `v6.6-stm32mp-r3.1` (`Makefile:2-4` says 6.6.129, the version the board prints) | `drivers/gpu/drm/stm/{ltdc.c,dw_mipi_dsi-stm.c}`, `drivers/gpu/drm/bridge/{adv7511/,synopsys/dw-mipi-dsi.c}`, `drivers/clk/stm32/`, `arch/arm64/boot/dts/st/` |
| **U** | ST U-Boot `v2023.10-stm32mp-r3.1` | `drivers/video/stm32/`, `board/st/stm32mp2/stm32mp2.c`, `configs/stm32mp25_defconfig`, `arch/arm/dts/stm32mp257f-dk.dts` |
| **O** | ST OP-TEE `4.0.0-stm32mp-r3.1` (commit `0063536697f1`) | `core/arch/arm/dts/` (the DK's SCMI, RCC and RIF configuration), `core/include/dt-bindings/soc/stm32mp25-rifsc.h` |
| **P** | this repository | `docs/notes/pre-study-stm32mp257f-dk.md`, `driver/rust/src/soc/stm32mp2/` |

All three are cloned (depth 1) under `$CONNECTED_DEPS_DIR` (`st-linux/linux`,
`st-uboot/u-boot-r3.1`, `st-optee-os`). A citation is `tag:file:line` with
the directory dropped where the tag implies it. **[D]** marks a fact that is
derived (computed, or concluded from several citations) and not stated by a
source. The three trees are GPL or BSD code read for **facts only**; nothing
is copied (pre-study table, "ST U-Boot" row). The board's own OP-TEE and
U-Boot releases are not known to be exactly these tags (P, sources table).

## The chain

```
 SYSCFG DISPLAYCLKCR          RCC FLEXGEN_27 (PLL8 594 MHz / 4 = 148.5 MHz [D])
 0x44235000 [D] = 2  ------>  ck_ker_ltdc  ----pixel clock---+
                                                             v
 DDR --AXI--> LTDC 0x48010000 ==24 bit RGB, HS/VS/DE==> DSI host+PHY 0x48000000
              layers L1,L2 (+L3)                         4 lanes + clock, <= 2.5 Gbit/s a lane
              IRQ SPI 158, 159                           PHY PLL ref = FLEXGEN_28 (594/22 = 27 MHz [D])
                                                                   |
   I2C2 0x40130000 (PF0 SDA, PF2 SCL) ---- control -----> ADV7535 (I2C 0x3d main, 0x3c cec, 0x3f edid, 0x38 packet)
   PB6 reset, PB4 interrupt                                        |  TMDS
                                                                   v
                                                      HDMI connector (type A), +5 V from LDO5
 (the LVDS bridge 0x48060000 and the 1024x600 LVDS panel hang off a second LTDC endpoint; not our path)
```

The path is LTDC to DSI to ADV7535 to HDMI, not LVDS (L:`stm32mp257f-dk.dts:536-553`
the LTDC's two endpoints, `:328-351` the DSI, `:428-471` the bridge, `:129-140` the
connector). The pre-study says the board file does not enable the LVDS bridge
(P:`:219-222`); in this tag it does (L:`stm32mp257f-dk.dts:555-578`, together with
an EDT 1024x600 panel and a PWM backlight, `:147-184`), and **U-Boot's board code
turns the LVDS side off again when no panel answers**
(U:`board/st/stm32mp2/stm32mp2.c:329-352`, `:718-745`). The DK's HDMI path needs
no panel and no backlight.

## Register blocks

| Block | Base | Size | Source |
| --- | --- | --- | --- |
| LTDC | 0x48010000 | 0x400 | L:`stm32mp251.dtsi:2150-2153` |
| DSI host + wrapper + PHY | 0x48000000 | 0x800 | L:`stm32mp255.dtsi:123-126` |
| LVDS (unused) | 0x48060000 | 0x2000 | L:`stm32mp255.dtsi:136-141` |
| RCC | 0x44200000 | | L:`stm32mp251.dtsi:2542` (the `rcc` node) |
| SYSCFG (clock mux `DISPLAYCLKCR` at +0x5000) | 0x44230000 | 0x10000 | L:`stm32mp251.dtsi:2735-2739`, L:`clk-stm32mp25-syscfg.c:13` |
| I2C2 (the ADV7535's bus) | 0x40130000 | 0x400 | L:`stm32mp251.dtsi:935-937`; pins PF2 SCL, PF0 SDA, AF9, open drain (L:`stm32mp25-pinctrl.dtsi:276-284`); 400 kHz (L:`stm32mp257f-dk.dts:395-402`) |
| GPIOB (ADV7535 reset PB6, interrupt PB4) | 0x44250000 | | L:`stm32mp251.dtsi:2771-2777`, `stm32mp257f-dk.dts:436-438` |

Derived addresses [D]: `RCC_LTDCCFGR` 0x44200840 and `RCC_DSICFGR` 0x44200844
(RCC base plus L:`stm32mp25_rcc.h:439-440`); `DISPLAYCLKCR` 0x44235000
(SYSCFG base plus L:`clk-stm32mp25-syscfg.c:13`). **The CPU must not touch
anything above 0x800 of the GPU (P, section 6); the LTDC and DSI have no
such rule in any source read, but RM0457 has not been read.**

## The LTDC

**Identity and variant.** The IP is the LTDC `HWVER_40100`/`40101` family
(L:`ltdc.c:58-59`, `:2552-2553`); the identity register `LTDC_IDR` is at
offset 0 (L:`ltdc.c:70`). U-Boot accepts the same two values and uses the same
layer map (U:`stm32_ltdc.c:942-946`). The first read at the board is
`IDR` (expected 0x00040100 or 0x00040101 [D from the constants]).

**Registers** (names and offsets as L:`ltdc.c:69-94` for the global block):
`IDR` 0x00, `LCR` 0x04 (layer count), `SSCR` 0x08, `BPCR` 0x0c, `AWCR` 0x10,
`TWCR` 0x14, `GCR` 0x18, `GC1R` 0x1c, `GC2R` 0x20, `SRCR` 0x24, `GACR` 0x28,
`BCCR` 0x2c, `IER` 0x34, `ISR` 0x38, `ICR` 0x3c, `LIPCR` 0x40, `CPSR` 0x44,
`CDSR` 0x48, `EDCR` 0x60, `CCRCR` 0x7c, rotation `RB0AR` 0x80 to `RIFCR` 0x8c,
`FUT` (FIFO underrun threshold) 0x90.

**Layers** on this version start at 0x100 and are 0x100 apart (`LAY_OFS_1`,
L:`ltdc.c:64-66`, `:2554`; the table `ltdc_layer_regs_a2`, L:`ltdc.c:461-505`,
the same in U:`stm32_ltdc.c:127-158`): for layer 1, `L1C0R` 0x100, `L1C1R`
0x104, `L1RCR` (reload) 0x108, `L1CR` 0x10c, `L1WHPCR` 0x110, `L1WVPCR` 0x114,
`L1CKCR` 0x118, `L1PFCR` 0x11c, `L1CACR` 0x120, `L1DCCR` 0x124, `L1BFCR`
0x128, `L1BLCR` (burst length) 0x12c, `L1PCR` 0x130, `L1CFBAR` 0x134,
`L1CFBLR` 0x138, `L1CFBLNR` 0x13c, auxiliary buffers 0x140-0x14c, `L1CLUTWR`
0x150, scaler 0x154-0x168, `L1CYR0R/1R` 0x16c/0x170, `L1FPF0R/1R` 0x174/0x178.
Layer *n* adds `n * 0x100` (L:`ltdc.c:1793`, `+ lofs`). With the 0x400 window
only layers 1 to 3 fit **[D]**. The number of layers is read from `LCR`
and clamped to `LTDC_MAX_LAYER` = 4 (L:`ltdc.c:2413-2415`, `ltdc.h:14`). **A
firewall can hide layer 3**: if the `l3` RIF entry is refused, Linux drops that
layer and the CRC (L:`ltdc.c:2445-2460`). Layer 2 shares a RIF entry with
layer 1 (`l1l2`).

**Pixel formats.** The `PF` field is bits 2:0 of `LxPFCR` (L:`ltdc.c:224`,
`:1816`). On this version (table `ltdc_pix_fmt_a2`, L:`ltdc.c:318-326`):
0 ARGB8888, 1 ABGR8888, 2 RGBA8888, 3 BGRA8888, 4 RGB565, 5 BGR565, 6 RGB888,
7 none. Anything else (BGR888, ARGB1555/4444, L8, YUV) goes through the
"flexible pixel format" registers `L1FPF0R/1R` (L:`ltdc.c:1810-1812`, the
`pix_fmt_flex` capability at `:2560`). Linux maps `XRGB8888` to ARGB8888
and `XBGR8888` to ABGR8888 (L:`ltdc.c:822-835`), and exposes the list at
L:`ltdc.c:355-372`. U-Boot hard-codes 16 bit RGB565 (U:`stm32_ltdc.c:1035`).
For a GPU buffer the straight choice is ARGB8888 (code 0) **[D]**; which
byte order the GPU's resolve engine writes for its X8R8G8B8 target is the
GPU side's question (pre-study, galcore formats), and RGBA/BGRA are in the
table if the swizzle is wanted in the scan-out and not in the shader.

**Pitch, length and alignment.** `L1CFBLR` holds the pitch in bytes in bits
31:16 and the line length in bits 12:0 (L:`ltdc.c:233-234`, `:1856-1866`).
Linux writes line length = `cpp * width + (bus_width / 8) - 1` and pitch =
`fb->pitches[0]` (L:`ltdc.c:1857-1866`); `bus_width` is the AXI data width
read from `GC2R` bits 6:4, `8 << field` (L:`ltdc.c:2472-2473`, `:169`). The
line number goes to `L1CFBLNR` (L:`ltdc.c:1870`). A 1920-pixel ARGB8888 line is
7680 bytes, which fits the 16-bit pitch and any bus width **[D]**. **No
source states a base-address or pitch alignment** beyond that the pitch is a
16-bit byte count; the DRM side asks for none (the CMA allocator aligns
to a page). Open question 8. The burst length is `L1BLCR`, written from the
DT property `st,burstlen` (absent on the DK: 0, the maximum)
(L:`ltdc.c:2654-2661`, `:1966`). The FIFO underrun threshold `FUT` defaults
to 128 (L:`ltdc.c:267`, `:1276`).

**Window, blending, enabling.** Window position is in LTDC coordinates:
horizontal start `x1 + 1 + AHBP`, stop `x2 + AHBP`; vertical start
`y1 + 1 + AVBP`, stop `y2 + AVBP`, where `AHBP = htotal - hsync_start - 1` and
`AVBP = vtotal - vsync_start - 1` (L:`ltdc.c:1793-1800`, `:1184-1190`). The
constant alpha is `alpha >> 8` (`:1820`), the default colour black (`:1823`),
blending factors `BF1_CA | BF2_1CA` for a format with no alpha, otherwise
`BF1_PAXCA | BF2_1PAXCA` (L:`ltdc.c:1827-1841`, `:260-263`), the z order is the
`BOR` field (`:231`). The layer is enabled by `LEN` in `L1CR`
(L:`ltdc.c:2008`), and the new configuration takes effect at the next vertical
blanking after a reload: bit `VBR` of `L1RCR` (`LXRCR_VBR`, L:`ltdc.c:254`,
`:2014`; `plane_reg_shadow` is true on this version, `:2565`). For the very
first picture an immediate reload (`IMR`, `:2081`) is the same step without the
wait **[D]**.

**Timing registers** (L:`ltdc.c:1183-1262`). With the DRM names, the driver
programs `SSCR = (hsync_len - 1) << 16 | (vsync_len - 1)`; `BPCR = (AHBP << 16)
| AVBP`; `AWCR = ((AHBP + hdisplay) << 16) | (AVBP + vdisplay)`; `TWCR =
((htotal - 1) << 16) | (vtotal - 1)`; `LIPCR = AAH + 1`; then polarity in
`GCR` (`HSPOL` bit 31 and `VSPOL` bit 30 set for a positive sync, `DEPOL`
bit 29 for data-enable low, `PCPOL` bit 28 for pixel data on the falling
edge; the CTA 1080p60 and 720p60 modes have positive syncs, so the first two
are set **[D]**); background colour `BCCR` =
0 (black); error interrupts `FUWIE | FUEIE | TERRIE` in `IER`; `FUT`; and
last `GCR.LTDCEN` (bit 0) (L:`ltdc.c:1183-1279`, `:138-146`). Each field is 12
bit wide in the low half-word and bits 27:16 in the high one
(`ltdc_conf_regs_a1`, L:`ltdc.c:506-515`).

**Interrupts.** The LTDC has two lines, SPI 158 and SPI 159, level high
(L:`stm32mp251.dtsi:2155-2156`; two lines because `nb_irq = 2`, L:`ltdc.c:2562`).
The flags are `LIF` (line), `FUWIF`, `TERRIF`, `RRIF` (register reload, bit 3),
`FUEIF`, `CRCIF`, `FURIF` (L:`ltdc.c:199-205`). The reload flag is how a driver
learns that a new buffer address took effect (L:`ltdc.c:1005`, the thread).
Which of the two lines carries which flags is not in the sources read (open
question 9).

**What U-Boot leaves.** See "What U-Boot hands over".

## The DSI host and PHY

**IP and PHY.** The DSI is a Synopsys DesignWare MIPI DSI host with ST's wrapper
and a new D-PHY: `HWVER_141` (IP version 1.41, `DSI_VERSION` at offset 0, upper 24
bits compared; L:`dw_mipi_dsi-stm.c:27-33`, `:1344-1360`). It supports up to four
data lanes (`max_data_lanes = 4`, L:`dw_mipi_dsi-stm.c:1249-1253`) at 80 to
2500 Mbit/s a lane (L:`dw_mipi_dsi-stm.c:127-128`). U-Boot's DSI driver is the same
code base (U:`stm32_dsi.c:34`, `:696`).

**ST wrapper registers** (L:`dw_mipi_dsi-stm.c:37-90`): `DSI_WCFGR` 0x400
(`DSIM` bit 0: 0 = video mode; `COLMUX` bits 3:1: `DSI_RGB888` = 5, L:`:115-122`),
`DSI_WCR` 0x404 (`DSIEN` bit 3), `DSI_WISR` 0x40c, `DSI_WPCR1` 0x430 (`CCF`
bits 5:0, `HSFR` bits 14:8), `DSI_WRPCR0` 0x434 (`IDF` bits 3:0, `NDIV` bits
13:4, `PSC`), `DSI_WRPCR1` 0x438 (`PROP`, `GMP`, `INT`, `BIAS`, `VCO`, `ODF`
bits 29:28), `DSI_WRPCR2` 0x43c (`SEL` bits 1:0, `PLLEN` bit 8, `UPD` bit 16,
`CLR` bit 24). Host registers: `DSI_PWR_UP` 0x04, `DSI_PTCR0` 0xb4, `DSI_PCTLR`
0xa0 (`PWEN`, `DEN`, `CKEN`), `DSI_PSR` 0xb0 (`PSSC`, clock lane stop state)
(L:`dw_mipi_dsi-stm.c:92-107`). The rest is the standard DesignWare map:
`DSI_CLKMGR_CFG`, `DSI_DPI_*`, `DSI_PCKHDL_CFG`, `DSI_VID_MODE_CFG`,
`DSI_PHY_TMR_CFG`, `DSI_PHY_IF_CFG`, `DSI_PHY_RSTZ`, `DSI_PHY_STATUS` and so
on, written by the generic core (L:`bridge/synopsys/dw-mipi-dsi.c:560-906`). Its
offsets are in that file's top and are not repeated here.

**Clock tree for the DSI** (L:`stm32mp255.dtsi:123-134`, `clk-stm32mp25.c:166-175`,
`:909-939`):

- `pclk` = `CK_BUS_DSI` (the APB gate, `RCC_DSICFGR` bit 1);
- `ref` = `CK_KER_DSIPHY`, a mux `RCC_DSICFGR.DSIPHYCKREFSEL` (bit 15) between
  `FLEXGEN_28` and the HSE (L:`clk-stm32mp25.c:171-174`, `stm32mp25_rcc.h:3542`);
- `px_clk` = `CK_KER_LTDC`, the LTDC's kernel clock;
- the DSI also *provides* a clock, `txbyteclk` (the PHY PLL's byte clock,
  `<&dsi>` is an input of the RCC node, L:`stm32mp255.dtsi:116`), which feeds
  the RCC's `clk_lanebyte` through the mux `DSIBLSEL` (`RCC_DSICFGR` bit 12:
  `DSI_TXBYTE` or `ck_ker_ltdc`; L:`clk-stm32mp25.c:166-169`, `:233`,
  `stm32mp25_rcc.h:3541`).

OP-TEE's boot configuration of the DK sets `DSIPHY` to `FLEX28` and `DSIBLANE`
to `DSIPHY` (O:`stm32mp257f-dk-ca35tdcid-rcc.dtsi:105-106`), and
`FLEXGEN_28` to PLL8 divided by 22 (O:`:65`, with PLL8 = 594 MHz,
`cfg = <297 5 1 4>` from the 40 MHz HSE, O:`:7`, `:190-193`). So the PHY
reference is **594 / 22 = 27 MHz** and `FLEXGEN_27`, the LTDC kernel clock,
is PLL8 / 4 = **148.5 MHz** (O:`:64`) [D from the divider semantics
`value + 1`, which the 594 MHz name and the 148.5 MHz result both fit]. These are the
values at the OP-TEE tag; the board's OP-TEE may differ (open question 1).

**The PHY PLL** (for the new PHY, L:`dw_mipi_dsi-stm.c:339-398`, `:444-484`,
`:670-754`): the output frequency is `pll_in * ndiv / (idf * odf)`
(L:`:198-207`), `idf` 1 to 16, `ndiv` 64 to 625 (`:109-112`), the lane rate is
twice the PLL output. The `odf` and the `hsfreqrange`, `vco`, `prop` values
come from a table keyed by the lane rate (`dppa_map`, L:`:273-338`, 68 rows
from 80 to 2500 Mbit/s); the configuration clock `CCF = (pll_in_MHz - 17) * 4`
(L:`:702`). Programming order in `set_rate`: `WPCR1 = (hsfreq << 8) | ccf`;
`WRPCR0 = ((ndiv - 2) << 4) | (idf - 1)`; `WRPCR1 = (odf << 28) | (vco << 24) |
(bias << 16) | (int << 8) | (gmp << 6) | prop` with `bias = 0x10`, `int = 0`,
`gmp = 1`; `PCTLR = CKEN`; `WRPCR2.SEL = 1`; pulse `WRPCR2.UPD` (1 ms each
way); then `PCTLR |= PWEN | DEN` (L:`:699-731`). The enable
order: `PCTLR = 0`, pulse `WRPCR2.CLR`, pulse `PTCR0.TRSEN` (the PHY test
reset), set the PLL rate, then enable the PLL by `WRPCR2.PLLEN` after
the clock lane reaches its stop state (`PSR.PSSC`, L:`dw_mipi_dsi-stm.c:399-442`,
`:514-557`).

**Host sequence** (generic core, L:`bridge/synopsys/dw-mipi-dsi.c:958-1007`,
the call order of `dw_mipi_dsi_mode_set`): enable `pclk`; ask the PHY for the
lane rate; power the block; `dw_mipi_dsi_init` (`DSI_PWR_UP = RESET`, then
`CLKMGR_CFG` with the TX escape clock divider `(lane_mbps / 8) / 20 + 1`,
`:618-648`); DPI colour coding and polarity (`:650-690`: 24 bit is `DPI_COLOR_CODING_24BIT`);
packet handler (`EOTP` off for the ADV7535 because of `NO_EOT_PACKET`, `:703-711`);
video mode type (non-burst sync pulses, `:560-588`); video packet, line timer and
vertical timing; PHY reset, PHY timing, PHY interface (`PHY_STOP_WAIT_TIME(0x20)`,
`N_LANES`, `:828-886`); clear the error flags; the ST PHY init above; the
PHY enable and wait for the lock and the clock lane's stop state (`:888-906`);
wait two frames (`:287-293`); switch to command mode and power on (the ST
wrapper sets `WCR.DSIEN`, L:`dw_mipi_dsi-stm.c:828-836`). Later, with the sink
ready, `dw_mipi_dsi_set_mode(VIDEO)` writes `MODE_CFG` = video and `LPCLK_CTRL`
= request high speed (`:590-610`, `:1030-1038`).

**What the ADV7535 asks of the host** (L:`adv7511/adv7533.c:162-195`): RGB888,
`MIPI_DSI_MODE_VIDEO | VIDEO_SYNC_PULSE | NO_EOT_PACKET | VIDEO_HSE`, channel 0,
the DT's lane count (4 on the DK, `stm32mp257f-dk.dts:433`). **Not burst
mode**, so the ST code applies its non-burst checks and no 20 % margin
(L:`dw_mipi_dsi-stm.c:1126-1225`): the lane rate must be within 5 per mille of
`pixel clock * bpp / lanes` (`clock_tolerance`, `:1021`, `:1170-1184`);
`hsync` and `hbp` must be at least `ceil(32 / bpp)` pixels (a 4-byte
short packet, `:1186-1198`); and the front or back porch must hold
`(data_hs2lp + data_lp2hs) * lanes * 8 / 2 / bpp` pixels so the link can drop
to low power between lines (`:1200-1223`). The high-speed to low-power
timings come from `hstt_phy_141_table` (L:`dw_mipi_dsi-stm.c:936-1000`, selected
by the lane rate at `:1002-1017`).

**Modes, with numbers [D].** The standard CTA-861 timings are not in the
trees read; the lane arithmetic uses the formulas above.

| Mode | Pixel clock | htotal x vtotal (porches) | Lanes | Lane rate | PHY PLL (27 MHz ref, the search of `:339-398`) |
| --- | --- | --- | --- | --- | --- |
| 1080p60 (VIC 16) | 148.5 MHz | 2200 x 1125 (hfp 88, hsync 44, hbp 148; vfp 4, vs 5, vbp 36) | 4 | 891 Mbit/s | `idf 4`, `ndiv 66`, `odf 1`: 445.5 MHz exactly; table row 900 (`hsfreq 0x29`, `vco 9`, `prop 0xb`); `ccf` 40 |
| 720p60 (VIC 4) | 74.25 MHz | 1650 x 750 (hfp 110, hsync 40, hbp 220; vfp 5, vs 5, vbp 20) | **3** (see below) | 594 Mbit/s | `idf 3`, `ndiv 66`, `odf 2`: 297 MHz exactly; table row 600 (`hsfreq 0x07`, `vco 9`, `prop 0xb`) |

The 1080p60 lane rate, 891 Mbit/s, is exactly the ADV7535's ceiling
`mode->clock * bpp <= 891000 * lanes` (L:`adv7533.c:111-119`) and its pixel clock
ceiling is 148.5 MHz (`:111`): **1080p60 is the largest mode the bridge accepts**,
and it sits on the limit. With 4 lanes at 720p the driver would run 445.5
Mbit/s; the Linux bridge code instead *changes the lane count*: above 80 MHz it
uses 4 lanes, otherwise 3 (L:`adv7533.c:123-148`), and divides the clock by
`{6, 4, 3}[lanes - 2]` in CEC register 0x16 bits (L:`adv7533.c:27-38`). Whether
four lanes also work below 80 MHz is open (open question 5); the safe first
mode is 1080p60, 4 lanes, which needs no lane change. If the reference were
the 40 MHz HSE instead (`DSIPHYCKREFSEL`), the same search gives `idf 15`,
`ndiv 167`, 890.67 Mbit/s a lane and `ccf` 92 (a 0.04 % error, inside the
5 per mille the check allows).

## The ADV7535

**Bus and addresses.** It sits on I2C2 at 7-bit address **0x3d** (main map),
with ancillary maps CEC **0x3c**, EDID **0x3f**, packet memory **0x38**
(L:`stm32mp257f-dk.dts:428-431`; the defaults in `adv7511.h:98-100` are the
same three). The driver re-points the maps by writing `addr << 1` into the
main map: `EDID_I2C_ADDR` 0x43, `PACKET_I2C_ADDR` 0x45, `CEC_I2C_ADDR` 0xe1
(L:`adv7511.h:56-58`, `:92`, `adv7511_drv.c:1288-1299`, `:1086-1100`). Other
devices on the same bus: the camera sensor at 0x1a and the touch controller at
0x41 (`stm32mp257f-dk.dts:407`, `:473`). Its interrupt is PB4, falling edge;
its reset is PB6, active low (`:436-438`). **Linux's driver does not use that
reset property** (the only GPIO it requests is `pd`, L:`adv7511_drv.c:1248`; no
other "reset" in the file or in `adv7533.c`) **[D]**; U-Boot's detection code
pulses it: asserts it, 1 ms, releases it, 10 ms
(U:`stm32mp2.c:186-203`) and then reads chip revision register 0x00, expecting
**0x14** (`:249-271`). The chip needs its supplies: 1.8 V from `BUCK5` for
`avdd`, `dvdd`, `pvdd`, `a2vdd`, `v1p2`; 3.3 V from `BUCK7` for `v3p3`
(L:`stm32mp257f-dk.dts:439-444`); and the connector's +5 V is `LDO5`
`v5v_hdmi` (L:`:129-140`, `:752-755`), which is **not** marked always-on or
boot-on in OP-TEE's DK description (O:`stm32mp257f-dk.dts:293-300`) **[D]**.

**Chip bring-up** (what Linux does, in order):

1. Probe: read register 0x00 (chip revision, `adv7511_drv.c:1265`). Apply the
   fixed-register patch to the main map: 0x16=0x20, 0x9a=0xe0, 0xba=0x70,
   0xde=0x82, 0xe4=0x40, 0xe5=0x80 (L:`adv7533.c:10-17`; the ADV7511 patch is
   a different list). Disable all packets (`adv7511_drv.c:1279`). Set the EDID,
   packet and CEC map addresses (as above) and apply the CEC-map patch:
   0x15=0xd0, 0x17=0xd0, 0x24=0x20, 0x57=0x11, 0x05=0xc8
   (L:`adv7533.c:19-25`, `:150-160`, `adv7511_drv.c:1100-1109`; the CEC map has an
   offset of 0x70 for the CEC engine itself, `adv7511.h:229`). Then power off
   (`adv7511_drv.c:1307`), which sets `POWER` (0x41) bit 6.
2. EDID: `__adv7511_power_on` (clear `POWER.PD`, 0x41 bit 6; for the ADV7535
   set `POWER2.HPD_OVERRIDE`, 0xd6 bit 6, so the HPD pin is ignored), then read the
   EDID through the EDID map (`adv7511_drv.c:328-364`, `:540-640`; registers
   `DDC_STATUS` 0xc8, `EDID_SEGMENT` 0xc4, `adv7511.h:81-82`).
3. Mode set (`adv7511_mode_set`, `adv7511_drv.c:706-805`): `0x4a` bits 3:2 =
   the low-refresh-rate class, and `0x17` bits 6:5 = the sync polarity
   (`vsync << 6 | hsync << 5`). The colour-space conversion and the
   AVI info-frame go with it (`adv7511_set_config_csc`, `:203-280`).
4. Enable (`adv7511_bridge_enable` -> `adv7511_power_on`, `:893-897`, `:367-376`):
   power on, `regcache_sync`, then `adv7533_dsi_power_on` (L:`adv7533.c:66-100`):
   timing generator on: CEC regs 0x28-0x37 get the horizontal and vertical
   totals and porches in a 12-bit split (`reg = value >> 4`, `reg + 1 = (value
   << 4) & 0xff`: `htotal`, `hsw`, `hfp`, `hbp` at 0x28-0x2f; `vtotal`, `vsw`,
   `vfp`, `vbp` at 0x30-0x37, L:`adv7533.c:27-64`); CEC 0x16 = divider
   `<< 3`; CEC 0x1c = `lanes << 4`; CEC 0x27: 0xcb, 0x8b, 0xcb (reset the
   timing generator); CEC 0x03 = 0x89 (enable HDMI); CEC 0x55 = 0; then the
   CEC patch again. Without the timing generator (`adi,disable-timing-generator`)
   0x27 = 0x0b. The DK's device tree does not set that property, so the
   timing generator is on (L:`adv7533.c:197-217`, `stm32mp257f-dk.dts:428-471`).
5. Power off: CEC 0x03 = 0x0b, CEC 0x27 = 0x0b, `POWER.PD`.

**HDMI audio** is a second port (`I2S2`, L:`stm32mp257f-dk.dts:464-469`, `:496-509`);
no audio is needed for a picture.

**U-Boot has no ADV7535 driver** (nothing under `drivers/video` mentions
ADV7511/7533/7535 [D from a search of the tree]); its only use of the chip is
the revision read in `detect_stm32mp25x_adv753x`, and for the DK that function is
not even called (the DK's init runs only the LVDS panel detection,
U:`stm32mp2.c:329-352`, `:623-626`).

## Clocks, resets, power and RIF

**Who owns what.** Two worlds meet here, as in the GPU's case (P, section 4):

| Item | Where it is | Source |
| --- | --- | --- |
| LTDC bus clock `CK_BUS_LTDC` (206), kernel clock `CK_KER_LTDC` (339) | **RCC directly**: one gate, `RCC_LTDCCFGR` (0x840) bit 1 (`LTDCEN`; bit 2 `LTDCLPEN`) | L:`clk-stm32mp25.c:550`, `:897-907`, `stm32mp25_rcc.h:3532-3535`, `st,stm32mp25-rcc.h:229`, `:361` |
| LTDC reset `LTDC_R` (76) | RCC, `RCC_LTDCCFGR` bit 0 | L:`clk-stm32mp25.c:2329`, reset header `st,stm32mp25-rcc.h:86` |
| DSI bus clock `CK_BUS_DSI` (205), PHY clock `CK_KER_DSIPHY` (289), lane byte `CK_KER_DSIBLANE` (288) | **RCC directly**: one gate, `RCC_DSICFGR` (0x844) bit 1; muxes in bits 12 and 15 | L:`clk-stm32mp25.c:491`, `:233-234`, `:909-939`, `stm32mp25_rcc.h:3537-3542`, `st,stm32mp25-rcc.h:228`, `:310-311` |
| DSI reset `DSI_R` (77) | RCC, `RCC_DSICFGR` bit 0 | L:`clk-stm32mp25.c:2330`, reset header `st,stm32mp25-rcc.h:87` |
| `FLEXGEN_27` (the rate source of `CK_KER_LTDC`), `FLEXGEN_28` (the PHY reference), `ICN_DISPLAY` (the display NoC clock) | **SCMI** clocks, served by OP-TEE: `rcc` lists them as inputs | L:`stm32mp255.dtsi:35-119`, `clk-stm32mp25.c:903-906`, `:171-174`, `st,stm32mp25-rcc.h:404`, `:428-429` |
| The pixel-clock mux `DISPLAYCLKCR` | **SYSCFG** register at +0x5000, a 2-bit mux: 0 `txbyteclk`, 1 `clk_pix_lvds`, 2 `ck_ker_ltdc` | L:`clk-stm32mp25-syscfg.c:13-62`; U writes 2 (`DISPLAYCLKCR_DPI`) for a DSI sink and 1 for LVDS (U:`stm32_ltdc.c:321-323`, `:887`, `:1013-1021`) |
| Supplies: DSI `vdd` (core, `BUCK2`), `vdda18` (`BUCK5`); chip 1.8 V / 3.3 V; HDMI +5 V (`LDO5`) | **SCMI voltage domains** (protocol 0x17) | L:`stm32mp257f-dk.dts:328-331`, `:439-444`, `:727-755`; O:`stm32mp25-st-scmi-cfg.dtsi:918-950` list the A35 agent's domains, `stm32mp257f-dk.dts:439-460` the STPMIC2 ones |
| Power domain | the plain `d1_pd` (`st,stm32mp-pm-domain`) for LTDC, DSI, LVDS | L:`stm32mp251.dtsi:189-193`, `:2160`, `stm32mp255.dtsi:131`; Linux's power-domain layer is a no-op for us **[D]** |

**The pixel rate goes through SCMI.** Linux sets the rate on the SYSCFG mux
(`ref`), which passes `CLK_SET_RATE_PARENT` down to `ck_ker_ltdc` and from there
to `FLEXGEN_27` (L:`clk-stm32mp25-syscfg.c:46-62`, `clk-stm32mp25.c:903-906`;
`ltdc.c:1105-1135`), and `FLEXGEN_27` is the one display clock OP-TEE lets the
non-secure agent re-rate: `SCMI_CLOCK_ALLOW_SET_RATE`, plus default-enabled
(O:`stm32mp25-st-scmi-cfg.dtsi:243-249`); `FLEXGEN_28` is only default-enabled
(`:251-256`), `ICN_DISPLAY` is default-enabled (`:70-75`). Its boot value is
already 148.5 MHz (above). So **1080p60 needs no SCMI clock request at all**,
and a 74.25 MHz 720p needs one `CLOCK_RATE_SET` on `FLEXGEN_27` (or the
`PLL8` 4:1 divider ratio **[D]** gives 148.5, 99.0 (594/6), 74.25 (594/8),
59.4 MHz (594/10) as other whole-divider values, no more). The transport is
the same open question as the GPU's (Q3: can our image open OP-TEE's SCMI
session): pre-study, section 4, and item optee-session-audit.

**RIF (the plain `stm32mp257f-dk` variant).** The LTDC has four RIFSC entries,
the DSI one: common 80, `l1l2` 119, `l3` 120, `rot` 121, DSI common 81
(L:`stm32mp251.dtsi:2150-2178`, `stm32mp255.dtsi:131`; ids in
O:`stm32mp25-rifsc.h:85-86`, `:118-120`; I2C2 is 42, O:`:51`,
L:`stm32mp251.dtsi:947`). OP-TEE's `ca35tdcid` DK configuration puts all five
at `RIF_NSEC, RIF_NPRIV` and unlocked, the CID filter unused, semaphore off
(O:`stm32mp257f-dk-ca35tdcid-rif.dtsi:82-88` for LTDC and DSI, `:50` for
I2C2): **non-secure, non-privileged access is open and nothing is to be
programmed for them on this variant** (same status and same caveat as the GPU's
entry 79 in P, Q4: the board's OP-TEE is not the tag). RCC clock gates that
belong to a RIFSC-guarded peripheral are themselves guarded by that entry
(`SEC_RIFSC(80)` for the LTDC clocks, `SEC_RIFSC(81)` for the DSI's, L:`clk-stm32mp25.c:2114-2115`,
`:2238-2242`), which is open for the same reason. U-Boot grants itself the
LTDC's `cmn` and `l1l2` entries before touching the registers
(U:`stm32_ltdc.c:830-870`, `stm32_rifsc_grant_access_by_id`, `:31`). **The
memory side:** the LTDC reads the frame buffer by DMA, so the master's RIF
(an `RIMU` entry) and the target's RISAF region matter. The RIMU table of the DK
file configures sixteen entries (O:`...-rif.dtsi:625-640`) without naming which
one is the LTDC's: open question 2. For RISAF the DK reserves two regions
of the 4 GiB map for the LTDC's secure layer (8 MiB at **0xfe800000**) and the
rotation buffer (16 MiB at **0xff000000**) and marks them non-secure, readable
and writable for CID0 and CID1 (L:`stm32mp257f-dk-resmem.dtsi:175-183`;
O:`...-rif.dtsi:988-995`, RISAF4 regions 13 and 14, `:1001`). They are `no-map`
and the Linux LTDC driver uses them for rotation only **[D from the DT; the
driver reads no such property]**. Any normal DDR region the A35 can write is
the scan-out buffer; the GPU carveout at 0xfa800000 (64 MiB) is RISAF region
10 with the same access (O:`...-rif.dtsi:984-986`, P section 3 memory map),
which settles that the GPU's output buffer, if it lives there, can be the
scan-out buffer, **if the LTDC master passes the RISAF filter like the GPU
master does** (open question 2).

**Resets.** `assert` sets the bit, `deassert` clears it, and the helper then
waits for the bit to read back as zero (L:`drivers/clk/stm32/reset-stm32.c:54-100`,
`:123-136`; `set_clr` is 0 for `LTDC_R` and `DSI_R`). That answers the polarity
question `soc::stm32mp2::rcc` left open for the GPU's `RCC_GPUCFGR` bit 0 (`rcc.rs:15-19`)
for the same register family **[D]**; it is not a reading of the GPU's own
entry.

## What U-Boot hands over

**Derived from the code: nothing is lit on a stock DK, and the LTDC and DSI
are untouched.** The facts: U-Boot's defconfig has `CONFIG_VIDEO`, the STM32
LTDC, DSI and LVDS drivers and a 1920 x 1200 maximum (U:`configs/stm32mp25_defconfig:168-178`);
the LTDC driver is flagged to be probed before the OS (`DM_FLAG_OS_PREPARE`,
U:`stm32_ltdc.c:1121`); at probe it takes the RIFSC entries, writes `DISPLAYCLKCR`
= 2, enables the bus and kernel clocks, reads `IDR`, then walks the endpoints
and, for each, looks for a **video bridge and then a panel behind it**
(U:`stm32_ltdc.c:739-811`, `:962-981`). For the DSI endpoint the remote is the
ADV7535, which is neither a panel nor a driver U-Boot has (above); the LVDS
endpoint leads to the EDT panel, which the board code detects through the
touch controller's firmware string and otherwise unbinds, together with the
LVDS bridge "to avoid a bad clock tree" (U:`stm32mp2.c:223-246`, `:329-352`).
With no LVDS panel plugged in the probe therefore fails and the DSI is never
set up **[D]**. If a panel were attached it would show a 16-bit RGB565
frame buffer cleared white and the LTDC would be running when U-Boot starts
the OS (U:`stm32_ltdc.c:1035-1058`). The DK's Linux then gets the nodes'
`status` fixed up by U-Boot (`ft_board_setup`, U:`stm32mp2.c:746-765`,
`:718-745`).
What that means for us: our image can assume the LTDC and DSI are idle and their
clocks possibly *on* (the probe enables them before it fails: U:`stm32_ltdc.c:895-921`)
**[D]**; the first thing it should do is read `RCC_LTDCCFGR`, `RCC_DSICFGR`,
`DISPLAYCLKCR` and `LTDC_IDR` and print them (open question 4). This also answers
the display half of Q8 in the pre-study as "off, by the code" until a board run
says otherwise.

## Backlight

There is none on the HDMI path: the ADV7535 drives a TMDS link, and a monitor
has its own. The only backlight in the DK's tree is the LVDS panel's
`pwm-backlight` on PWM3 channel 1 (period 1 ms, ten brightness levels,
default level 10) with the panel enable on PI4
(L:`stm32mp257f-dk.dts:147-184`). It is not needed and not to be touched.

## The minimum order for one GPU-rendered frame on the screen

Prerequisites: the GPU's output is a linear buffer in normal DDR the A35
writes (for example the carveout), the sink supports the chosen mode, an HDMI
cable with a monitor that accepts 1080p60, and the image runs at EL2
non-secure with OP-TEE's SCMI reachable (Q3). Every register is the ST
driver's; the order is Linux's with the DRM layers taken away; the LTDC runs
first because the DSI core waits two frames for it **[D]** (DRM enables the
CRTC before the bridge chain: `drivers/gpu/drm/drm_atomic_helper.c:1512-1553`,
and the DSI's `mode_set` sleeps for two frames, `dw-mipi-dsi.c:287-293`, `:1001`).

0. **Rails.** Over SCMI voltage (0x17): `LDO5` (`v5v_hdmi`) on. `BUCK5`, `BUCK7`
   and `BUCK2` are on already since the board is running **[D]** (they supply
   the SoC and the SD card and Ethernet); check, do not assume.
1. **Clocks (no SCMI needed for 1080p60).** Read `FLEXGEN_27`: 148.5 MHz.
   `RCC_LTDCCFGR`: clear `LTDCRST` (bit 0), set `LTDCEN` (bit 1);
   `RCC_DSICFGR`: clear `DSIRST`, set `DSIEN`, and keep `DSIBLSEL`/
   `DSIPHYCKREFSEL` as OP-TEE set them. Read each back.
   `SYSCFG + 0x5000` = 2.
2. **ADV7535 up.** Pulse PB6 (low 1 ms, high 10 ms). I2C2 at 400 kHz (the
   camera and the touch controller share the bus): read 0x3d reg 0x00, expect
   0x14. Apply the main-map patch, disable packets, program the EDID, packet and
   CEC addresses, apply the CEC patch. Read the EDID if there is a monitor, or
   take 1080p60 from the CTA table. Do not yet clear the power-down bit.
3. **LTDC timing, layer 1 (shadow, not enabled).** Program `SSCR`, `BPCR`,
   `AWCR`, `TWCR`, polarity in `GCR`, `BCCR`, `FUT`; layer 1: window,
   `PFCR` = 0 (ARGB8888), `CACR`, `DCCR`, `BFCR`, `CFBAR` = buffer address,
   `CFBLR` (pitch << 16 | length), `CFBLNR`; `L1CR.LEN`; reload (`L1RCR.VBR`
   or `IMR`). Enable `GCR.LTDCEN` (also bit 0 of the same register).
4. **DSI.** `dw_mipi_dsi_mode_set` as above: PHY PLL (table, `ccf`, `WRPCR0/1`),
   host registers, PHY enable, wait the clock lane's stop state, two frames,
   `WCR.DSIEN`. Then `DSI_MODE_CFG` = video, `LPCLK_CTRL` = high speed. The
   check: `DSI_PHY_STATUS` shows the lock and the stop states; `WISR.PLLLS`
   (bit 8) if the PLL lock is reported there (it is read by the older PHY's path;
   for the new PHY Linux polls `PSR.PSSC`, L:`dw_mipi_dsi-stm.c:514-540`).
5. **ADV7535 on.** `POWER` (0x41) bit 6 cleared, `POWER2` bit 6 set; the timing
   generator registers in the CEC map; 0x1c = `4 << 4`; 0x27 = 0xcb, 0x8b, 0xcb;
   0x03 = 0x89; 0x55 = 0; the CEC patch; polarity (0x17) and 0x4a; the
   colour-space and AVI info-frame if the sink wants HDMI and not DVI. Read
   the HPD and DDC status registers for the result.
6. **A new frame.** Write the new buffer address into `L1CFBAR`, request a
   reload (`L1RCR.VBR`) and wait for `ISR.RRIF` or the next line interrupt
   (L:`ltdc.c:1854`, `:2014`); the buffer must be written back and visible
   to the LTDC master (the GPU carveout is mapped non-cacheable by our image,
   P section 3; a CPU-written test pattern needs a flush).

A first board session should run steps 1 to 3 and read back, then 4, then 5,
printing the status after each (a `yk-record` line per step, as the GPU
probe does), and judge the picture last. A pattern from the DSI's own video
pattern generator (`VID_MODE_VPG_ENABLE`, L:`dw-mipi-dsi.c:575-582`, behind
`CONFIG_DEBUG_FS`) is a way to cut the LTDC out of the first test; the LTDC's
`BCCR` background colour does the same on the other side.

## Open questions

1. **The board's OP-TEE configuration.** Are `FLEXGEN_27` = PLL8/4 (148.5 MHz),
   `FLEXGEN_28` = PLL8/22 (27 MHz) and `ALLOW_SET_RATE` on the running binary,
   as in the tag's DK file? Read `CK_KER_LTDC`'s source and `RCC_DSICFGR`
   at the board (the first probe line).
2. **The LTDC's RIMU.** Which `RIMU_ID` is the LTDC's master and its CID
   selection (`RIF_CIDSEL_P`), and does it pass RISAF4 for the buffer's region?
   RM0457's RIMU table is needed; the DK file lists sixteen entries without
   names (O:`...-rif.dtsi:625-640`).
3. **The HDMI +5 V.** Is `LDO5` off at hand-over, and does the ADV7535 need it
   (HPD/DDC pull-ups on the connector) to read an EDID? The first
   board session reads the SCMI voltage state.
4. **U-Boot's residue.** Confirm on the board what the code says: no video
   device, the LTDC and DSI clock gates in their state, `DISPLAYCLKCR`.
   (Q8's display half.)
5. **ADV7535 below 80 MHz.** Whether four lanes work at 720p (the driver
   moves to three, L:`adv7533.c:123-148`), and which divider goes with it.
6. **The reset line.** Linux never drives PB6 (above), so the chip is out of
   reset by whatever the board's pull and the pad's reset state leave. Does
   0x3d ACK before any PB6 pulse? If not, U-Boot's pulse (1 ms / 10 ms) is
   the reference.
7. **Burst or not.** The ADV7535 asks for non-burst sync pulses; the
   non-burst checks and the low-power gap (`delay_to_lp`) must hold for the
   mode (L:`dw_mipi_dsi-stm.c:1186-1223`). Computed here **[D]**: 1080p60
   at 891 Mbit/s uses the table row `HSTT(900, 115, 58, 88, 35)` (L:`:936-1000`;
   clock lp2hs/hs2lp, data lp2hs/hs2lp), so `delay_to_lp = ceil((35 + 88) * 4 * 8 /
   2 / 24)` = 82 pixels, and `hfp` 88 and `hbp` 146 (148 minus the 2-pixel HSE
   packet) both clear it; 720p60 on three lanes at 594 Mbit/s (row
   `HSTT(600, 82, 46, 64, 27)`) needs 46 against `hfp` 110. Both modes pass the
   check; the sync width (44 and 40 pixels) passes the 2-pixel minimum.
8. **Alignment.** No source states a base or pitch alignment for `L1CFBAR` and
   `L1CFBLR` (only that the pitch is 16 bits and the line length 13). RM0457.
9. **Which IRQ line carries what.** Two lines (SPI 158, 159); the split
   between the line/reload flags and the error flags is not in the code read.
10. **RM0457** (LTDC chapter, DSI chapter, RCC, SYSCFG `DISPLAYCLKCR`, RIFSC/RIMU) is
    unread: every offset and bit above is the drivers' and has not been
    checked against the manual. The same holds for the IP-version
    constants.
11. **EDID and mode choice.** First run: fixed 1080p60 against the monitor's
    EDID. A monitor that refuses 1080p60 at 148.5 MHz would need
    a second `FLEXGEN_27` setting over SCMI.
12. **The yakogl display runtime.** No board layer of ours has an
    LTDC/DSI/I2C2 owner yet; `driver/rust/src/soc/stm32mp2/` has `power`,
    `rcc` and `smc_wdt` only. The new code will need an I2C master (I2C2,
    its kernel clock `CK_KER_I2C2` 263, an RCC gate, L:`stm32mp251.dtsi:940`,
    and the pin mux of PF0/PF2), a GPIO output (PB6) and a read of PB4. Their
    RIF entries (I2C2 42; the GPIO banks are SCMI-clocked,
    L:`stm32mp251.dtsi:2778`) are non-secure in OP-TEE's file for I2C2 and
    unknown for the GPIO bank.

## Corrections to the pre-study found on the way (not made here)

- `pre-study-stm32mp257f-dk.md:219-222`: "the DK board file does not enable"
  the LVDS bridge. In ST's tag the board file enables it (`stm32mp257f-dk.dts:555-558`);
  U-Boot unbinds it again when no panel is present. The DK's HDMI path is
  unaffected.
- Q7's "Open: layers, formats, stride alignment, the DSI host and the ADV7535's
  setup, the backlight" is answered above except the alignment (open
  question 8); the P8 row can cite this note.
