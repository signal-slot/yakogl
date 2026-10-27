<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Verdin iMX8M Plus: LCDIF1 and the MIPI DSI host in Rust, from the RM

Status (2026-10-02): designed, written and host-tested; not yet run on a
board. Group E steps 3 and 5 of docs/source-form.md: the last two display
files written after GPL Linux drivers, `src/display/imx8mp_lcdif.c` (+
`imx8mp_lcdif_adopt.c` and the LCDIF lines of `gc_panel_console.c`) and
`src/display/imx8mp_dsim.c`, rewritten in the `yakogl-drivers` crate from
documentation. The C files stay the default until the board has run the new
code: `CONFIG_YAKOGL_DRV_DISPLAY_IMX8MP_LCDIF_RUST` and `CONFIG_YAKOGL_DRV_DSIM_RUST`
(both default n) switch to it.

## Documents

- i.MX 8M Plus Applications Processor Reference Manual, IMX8MPRM Rev. 3,
  08/2024 ("RM"; cited by section, nothing copied): 13.3 LCDIF (13.3.2.6,
  13.3.3.1.x), 13.6 MIPI DSI Host Controller (13.6.3-13.6.5, 13.6.7,
  13.6.8.2 Table 13-26, 13.6.9.1.x), 13.7 MIPI D-PHY (13.7.1.2, 13.7.2.4
  Table 13-27, 13.7.2.16.3 Table 13-32, 13.7.7 Figure 13-69), Table 7-1
  (interrupts), Table 5-1 (clock roots).
- hal_nxp (BSD-3, revision 1e91007b): `PERI_LCDIF.h`, `PERI_MIPI_DSI.h`
  (new in `tools/hal-nxp/gen_regs.py`, sha256 94fceb99...),
  `PERI_MEDIA_BLK_CTRL.h`; the SDK driver `fsl_lcdifv3.c` as the permissive
  reference for the layer buffer word and the panic thresholds.
- MIPI Alliance Specification for D-PHY, Version 1.2: the global operation
  timing parameters (the minimums in `ip/dphy_timing.rs`). The values are the
  widely cited ones; **the table and section number are a citation pending**
  (the specification text was not at hand).
- TI SN65DSI84 SLLSEC2H (the bridge, already `ip::sn65dsi8x`): Table 7-2,
  the order the DSI host must keep.
- Test data: `src/display/golden-linux-2026-09-21.txt`, the board's Linux
  with the panel on (its LCDIF dump stops at 0x32e8002c with a bus error, so
  CTRLDESCL0_x and PANIC0_THRES were never captured).

The Linux drivers of this path (lcdif, samsung-dsim, phy-mipi-dphy) are GPL
and were not read.

## The chain

```
 CCM MEDIA_DISP1_PIX ----------+           CCM MEDIA_MIPI_PHY1_REF --+
 (VIDEO_PLL1 1039.5 MHz / 15)  |           (24 MHz / 2 = 12 MHz)     |
                               v                                     v
 DDR --AXI--> LCDIF1 ==RGB888, HS/VS/DE==> MIPI DSIM ----PPI----> D-PHY master --4 lanes + clk--> SN65DSI84 --LVDS--> panel
              0x32E80000  pixel clock      0x32E60000              PLL: 12 MHz x 711 / (5 x 4)      I2C2 0x2C            1280x800
              13.3        69.3 MHz         13.6                    = 426.6 Mbit/s a lane, 13.7      SLLSEC2H             Riverdi
              IRQ 5       INV_PXCK=1       video, burst, 24 bpp    byte clock 53.325 MHz            DSI clock 213.3 MHz  RVT101HVDWC00-B
                                           esc 17.775 MHz (/3)
 power: MEDIAMIX (GPC PU10) + MIPI_PHY1 (PU0), MEDIA_BLK_CTRL clocks/resets: soc::imx8mp::media (step 4)
```

The clock plan is the devicetree's (`verdin_imx8mp_display.dtsi`): the board's
Linux plan, pixel clock VIDEO_PLL1 1039.5 MHz (m 173, k 16384) / 15 = 69.3
MHz, 58.761 Hz with the 1404 x 840 total; DSIM reference 12 MHz, lane rate
426.6 Mbit/s (`burst-clock-frequency`, golden PLLCTRL 0x00816C74), 2.6 %
above the 415.8 Mbit/s of the pixels -- the room the packet overhead needs
(`ip::samsung_dsim::LINK_MARGIN_PERMILLE`, measured: a link with no margin
broke the picture on 2026-09-23 and, at 71.1 MHz, on 2026-10-06, issue #23).
The SN65DSI84's DSI clock range (42) and divider (3) give an LVDS clock of
71.1 MHz, as on Linux. The 71.1 MHz plan of 2026-10-02 to 2026-10-06
(VIDEO_PLL1 1066.5 MHz, 60.286 Hz) had no margin and is withdrawn. NOTES.md
"Pixel clock" has the arithmetic, `dsi_chain_tests::verdin_dt` checks it
against the devicetree. Note for the record: the task brief's "DSI
153.6 MHz per the bridge logs" is the AM62P's SN65DSI83 (RESULTS, "up, DSI
153600 kHz"); on this board the bridge sees 213.3 MHz (CHA_DSI_CLK_RANGE 42 in
the golden dump). The R4 plan (no `video-pll1-frequency`: SYS_PLL2 / 14 =
71.43 MHz) cannot run on 426.6 Mbit/s at all: 71.43 MHz x 24 / 4 =
428.57 Mbit/s. The new DSIM refuses such a link (`-EINVAL` in the plan
step); with `burst-clock-frequency = <0>` it takes the slowest rate the PLL
makes above 428.57 Mbit/s with the margin (439.29), 12 MHz x 293 / (2 x 4) =
439.5 MHz (429 MHz before the margin was a limit). The old C driver ran it with no
check, which is the "picture broke up" of 2026-09-23.

## What the RM documents, and what it does not

| Fact | Source | Used as |
|---|---|---|
| LCDIF register map, fields, SW_RESET set out of reset | RM 13.3.3.1.x, `PERI_LCDIF.h` | `ip::lcdifv3` (existing) |
| LCDIF order: timings, DISP_ON, DMA, layer EN; SHADOW_LOAD_EN after DMA writes, cleared at VSYNC | RM 13.3.2.6 Figure 13-7, 13.3.3.1.16 | `configure`, `enable`, `set_fb` |
| `INV_PXCK` 1 = consumer samples on the rising edge | RM 13.3.3.1.2.4 | devicetree `pixclk-rising`; the value (1) is the board's Linux (CTRL 0x08), not stated for the DSIM |
| `VS_BLANK_EN` = vertical blanking interrupt; INT_STATUS W1C; LCDIF1 = IRQ 5 | RM 13.3.3.1.8-9, Table 7-1 | `set_vblank_irq`, the shim's ISR |
| `CTRLDESCL0_3` bits 31:16 | RM: reserved (13.3.3.1.13.4) | 0, as NXP's SDK writes (`desc3-extra`, R1) |
| Panic thresholds | RM: units and range (13.3.3.1.24); SDK: "1/3 FIFO", "2/3 FIFO" | devicetree `<170 340>` |
| DSIM register map, field meanings, reset values | RM 13.6.9.1.x, `PERI_MIPI_DSI.h` | `ip::samsung_dsim` |
| PLL: Fout = Fin x M / (P x 2^S); Fin 6-300, Fin/P 2-30 MHz, P 1-63, M 64-1023, 2^S 1-8, Fout 32.8-1500 MHz | RM 13.7.2.4, Table 13-27 | `pll_pms`, `pll_pms_at_least` |
| VCO range | RM: **1050-2100 MHz (13.7.2.4 NOTE) vs 1050-1500 MHz (Table 13-27)** | the NOTE's: the board runs 1706.4 MHz, and 426.6 Mbit/s is unreachable inside 1500 |
| PLLCTRL: PllEn 23, PMS 19:1 | RM 13.6.9.1.25 | |
| P/M/S positions inside PMS | **not stated** (one 19-bit field; Figure 13-55 names PMS_P/M/S) | widths from Table 13-27 (6 + 10 + 3 = 19); order P 19:14, M 13:4, S 3:1 from the golden decode (0x00816C74 = P5 M711 S2 = 426.6 MHz, confirmed by the bridge's clock range) |
| PllStable = PLLTMR counted to 0 | RM 13.6.5, 13.6.9.1.28 ("System clock cycle base") | |
| Lock time 3000 x tp | RM Figure 13-69 | PLLTMR = 3000 PFD periods at the fastest PCLK (200 MHz, Table 13-24 / 5-1): 250 000 |
| Which clock PLLTMR counts | **not stated** (PCLK is the only clock in Table 13-24) | upper bound PCLK; poll bound 20 ms covers a counter down to 12.5 MHz |
| Software reset: all clocks on, PLL first, then SwRst, wait SwRstRls, re-configure | RM Table 13-26 case 16, 13.6.4, 13.6.9.1.5 | `attach` steps 2-3 |
| SwRst self-clearing | **not stated** (the golden dump reads SWRST 0) | wait for SWRST 0 and SwRstRls 1 |
| CLKCTRL: ByteClkSrc 00, PLLBypass 0, Dphy_sel, EscClk = ByteClk / prescaler <= 20 MHz | RM 13.6.9.1.6 | `clkctrl` |
| CONFIG: video, burst, event mode, RGB888, lanes; Mflush_VS 0 = flush enabled (default) | RM 13.6.9.1.8 | `config`; = golden 0x0600707F |
| ESCMODE ForceStopstate bit 20; released after configuration | RM 13.6.9.1.9, Table 13-26 case 1 steps 10-12 | held by `attach`, released by `video_start` (R5) |
| MDRESOL MainStandby: "In Video mode, if 0, data is not transferred" | RM 13.6.9.1.10 | set last in `attach` |
| MVPORCH CmdAllow / StableVfp, MSYNC VSA, MVPORCH VBP in lines | RM 13.6.9.1.11, .13 | CmdAllow 0 (no command packets in video), StableVfp = VFP |
| MHPORCH / MSYNC HSA: "word counts for blank packet" | RM 13.6.9.1.12-13 | **the conversion from pixels is not stated**: pixels x byte clock / pixel clock - 6 (long-packet header + checksum); HFP 21, HSA 12 = golden, HBP 43 vs golden 42 |
| PHYTIMING fields "M_T...CTL[7:0] to D-PHY" | RM 13.6.9.1.33-35, Table 13-32 | **unit not stated**: byte clocks (the only unit in which the golden values form a working D-PHY timing) |
| D-PHY HS 80-1500 Mbit/s, forward HS only (reverse is LP) | RM 13.7.1.2 | `dphy_timing` range, n = 1 in T_HS-TRAIL |
| FIFOCTRL nInit bits | RM 13.6.9.1.21 (names only) | pulse low then high, 1 ms each (**length not stated**) |
| TIMEOUT | RM 13.6.9.1.7 (BTA / LP-read timeouts) | reset value 0x00FFFFFF kept (golden 0x00FF00FF): no BTA or read is ever started |
| PHYCTRL_B1 (B_DPHYCTL) | RM 13.6.9.1.29 refers to BLK_CTRL | reset value 0, as golden |
| The order with the bridge | SLLSEC2H Table 7-2: CSRs with the DSI clock in HS (seq 5), PLL (6), soft reset (7), then the video stream (8) | `attach` returns with the clock lane in HS, the data lanes held |

## The sequence (cold path, `imx8mp_display_on`, switches on)

1. Media power, clocks, resets (step 4, unchanged): MEDIAMIX + LCDIF1,
   MIPI_PHY1 + MIPI_DSI1, VIDEO_PLL1 and the pixel root, the PHY reference
   root (12 MHz), reset pulses. `STOP_AFTER` 2, 3.
2. LCDIF1 configured through `yakogl,imx8mp-lcdif` (`ip::lcdifv3::configure`:
   SW_RESET pulse, CTRL = INV_PXCK, DISP_PARA 0, timings, layer at our buffer,
   CSC bypass; display and layer off). `STOP_AFTER` 4.
3. DSIM attached through `yakogl,samsung-dsim` (`mipi_dsi_attach`,
   `ip::samsung_dsim::attach`): PLLTMR, CLKCTRL {EscClkEn, prescaler},
   PLLCTRL {PllEn, PMS}, poll PllStable (Table 13-26 case 16); SwRst, poll
   SWRST 0 and SwRstRls (13.6.9.1.5, .3); INTSRC cleared; PHYTIMING x 3;
   CLKCTRL with byte and lane escape clocks and TxRequestHsClk; FIFO
   pointers initialised; CONFIG with the lanes; poll data lanes in stop state
   and the clock lane stopped or HS-ready; ESCMODE ForceStopstate; MVPORCH,
   MHPORCH, MSYNC, MDRESOL, MainStandby. Returns with the data lanes held.
   `STOP_AFTER` 5.
4. SN65DSI84 started (`sn65dsi8x_start`, unchanged): its CSRs, PLL and soft
   reset while the clock lane runs in HS and no video flows. `STOP_AFTER` 6.
5. LCDIF1 enabled (panic thresholds, VS_BLANK interrupt, DISP_ON, layer EN
   with a shadow load, waited for: `-ETIMEDOUT` = no pixel clock).
6. `yakogl_samsung_dsim_video_start()`: ForceStopstate released.
7. Bridge link check (IRQ_STAT), backlight. `STOP_AFTER` 7.

Adopt path (the chain the previous kernel left running): unchanged policy;
reads and the flip of the running layer go through the Rust on the facade's
own LCDIF mapping (`yd_lcdif_read_scanout`, `yd_lcdif_flip`), the DSIM's
health bits through the device. `imx8mp_display_off()` detaches the DSIM.

### The DSIM PLL for this board

12 MHz reference, 426.6 Mbit/s wanted: P 5 (2.4 MHz into the PFD, inside
2-30), M 711, S 2: VCO 1706.4 MHz, Fout 426.600 MHz exactly, byte clock
53.325 MHz, escape clock 53.325 / 3 = 17.775 MHz (<= 20). PLLTMR 250 000
(3000 x 5 / 12 MHz = 1.25 ms at 200 MHz). Identical to the golden PLLCTRL
0x00816C74; the bridge derives CHA_DSI_CLK_RANGE 42 and the divider 3 from
the same rate.

### PHYTIMING from the D-PHY minimums

Rule: every interval is ceil(minimum / T_byte) byte clocks (T_byte = 8 UI);
for PREPARE + ZERO the ZERO part is ceil(sum / T_byte) minus the PREPARE
cycles, so the sum holds whatever the parts round to; a PREPARE rounded up
past its maximum is an error (below 160 Mbit/s some rates have no setting and
are refused). Exact integer arithmetic. At 426.6 Mbit/s (UI 2.344 ns,
T_byte 18.753 ns):

| Field | D-PHY minimum | ns | computed | old table (golden) | |
|---|---|---|---|---|---|
| LPX | 50 ns | 50.0 | 3 (56.3) | 3 | = |
| HS_EXIT | 100 ns | 100.0 | **6** (112.5) | 5 (93.8) | old **below** the minimum; new above old |
| CLK_PREPARE | 38 ns (max 95) | 38.0 | 3 (56.3) | 3 | = |
| CLK_ZERO | PREPARE + ZERO >= 300 ns | 300.0 | **13** (sum 16 = 300.05) | 18 (sum 393.8) | new below old, at the minimum |
| CLK_POST | 60 ns + 52 UI | 181.9 | **10** (187.5) | 9 (168.8) | old **below** the minimum; new above old |
| CLK_TRAIL | 60 ns | 60.0 | 4 (75.0) | 4 | = |
| HS_PREPARE | 40 ns + 4 UI (max 85 + 6 UI) | 49.4 | **3** (56.3) | 4 (75.0) | new below old |
| HS_ZERO | PREPARE + ZERO >= 145 ns + 10 UI | 168.4 | **6** (sum 9 = 168.8) | 5 (sum 9) | same sum; new ZERO above old |
| HS_TRAIL | max(8 UI, 60 ns + 4 UI), n = 1 | 69.4 | **4** (75.0) | 7 (131.3) | new below old |

Words: PHYTIMING 0x00000306 (old 0x00000305), PHYTIMING1 0x030D0A04 (old
0x03120904), PHYTIMING2 0x00030604 (old 0x00040507). The replaced C used the
old table between 400 and 450 MHz and otherwise a round-to-nearest formula
(3/5/2/14/10/3, 3/6/5 at 428 MHz: CLK_PREPARE under 38 ns, HS_EXIT under
100 ns). If the link check reports SoT/EoT errors with the new values, the
first experiment is n = 4 for HS_TRAIL (6 cycles), then the old table's
CLK_ZERO.

## The R5 decision: who releases ForceStopstate, and when

Zephyr's MIPI-DSI API is attach / transfer / detach; it has no "start
video" step, and Zephyr PR #111446 (the Samsung DSIM) is not merged. The
SN65DSI84 must be programmed with the DSI clock lane in HS and before the
video stream starts (SLLSEC2H Table 7-2, seq 5-8), and the RM's own video
start (Table 13-26 case 1) releases ForceStopstate after the configuration.

Decision: `attach()` ends with the clock lane in HS and the data lanes held
(ForceStopstate set, MainStandby set); the release is a driver-specific call,
`yakogl_samsung_dsim_video_start()` (`include/yakogl_drv/samsung_dsim.h`),
made by whoever knows that the LCDIF scans:

- the chain facade (`imx8mp_display.c` through `imx8mp_dsim_rust.c`), right
  after the LCDIF is enabled -- the order the board has run since 2026-09-21;
- the LCDIF's Zephyr display device in `blanking_off()`, when its node names
  the DSIM in `dsi-host` -- the Zephyr-API path (a display stack that only
  knows `display_blanking_off()` gets video without knowing about DSI).

Rejected: releasing inside `attach()` (the bridge would be programmed with
the data lanes free and the DSIM in standby with no LCDIF input; what the
DSIM sends then is not stated, and the proven order holds the lanes until the
bridge's soft reset); releasing on a `transfer()` of a made-up packet type
(an abuse of the API); a delayed work item (an ordering by time). A bridge
that attaches its host itself (`yakogl,sn65dsi8x` with `dsi-host`, the AM62P
style) works the same way: blanking_off releases.

## Devicetree and Kconfig

```
lcdif1: display-controller@32e80000 {      mipi_dsi: dsi@32e60000 {
    compatible = "yakogl,imx8mp-lcdif";        compatible = "yakogl,samsung-dsim";
    reg = <0x32e80000 0x10000>;                reg = <0x32e60000 0x1000>;
    interrupts = <GIC_SPI 5 ...>;              pll-ref-frequency = <12000000>;
    hactive = <1280>; vactive = <800>;         burst-clock-frequency = <426600000>;
    pixclk-rising;                             esc-clock-frequency = <20000000>;
    panic-thresholds = <170 340>;              data-lanes = <4>;
    dsi-host = <&mipi_dsi>;                    pixel-clock-frequency = <71100000>;
};                                             burst-mode;
                                           };
display0 (yakogl,imx8mp-display): ... lcdif = <&lcdif1>; dsi = <&mipi_dsi>;
```

- `yakogl,imx8mp-lcdif` (`dts/bindings/display/`): `adopt-only` (never
  configure; adopt what runs), `pixclk-rising`, `*-active-low`,
  `desc3-extra` (R1, default 0), `panic-thresholds`, `interrupts`
  (optional), `dsi-host` (optional). The shim also serves
  `yakogl,imx8mp-lcdif-adopt` nodes as adopt-only instances (the
  Armadillo-X2's LCDIF3), so the X2 needs no devicetree change.
- `yakogl,samsung-dsim` (`dts/bindings/mipi-dsi/`).
- `yakogl,imx8mp-display` gains `lcdif` and `dsi`; the build checks that they
  are the same blocks as its own `reg` ranges and that the clock properties
  agree.
- `CONFIG_YAKOGL_DRV_DISPLAY_IMX8MP_LCDIF_RUST` (default n): the LCDIF device;
  `imx8mp_lcdif_rust.c` replaces `imx8mp_lcdif.c` in the facade; on a board
  with no chain node (X2) the device provides the `imx8mp_display_*` API in
  place of `imx8mp_lcdif_adopt.c`; the panel console's LCDIF reads and flip
  go through `yd_lcdif_read_scanout` / `yd_lcdif_flip_polled`.
- `CONFIG_YAKOGL_DRV_DSIM_RUST` (default n, selects MIPI_DSI; outside the
  `YAKOGL_DRV_DISPLAY_IMX8MP` block because selecting MIPI_DSI under its I2C
  dependency loops): the DSIM device; `imx8mp_dsim_rust.c` replaces
  `imx8mp_dsim.c`.
- The media power domains stay the facade's (no `power-domains` on the new
  nodes yet): moving the policy is step 8.

## Layout

| File | What |
|---|---|
| `driver/rust/src/ip/dphy_timing.rs` | D-PHY minimums, round-up rule |
| `driver/rust/src/ip/samsung_dsim.rs` (+ `tests.rs`) | PLL search, plan, attach / video_start / disable, health |
| `driver/rust/src/ip/lcdifv3.rs` | + adopt (capture once, no write without an address), polled flip, vblank interrupt |
| `driver/rust/src/ffi/imx8mp_dsi.rs` | `yd_lcdif_*`, `yd_samsung_dsim_*`, the structs (pinned both sides) |
| `driver/rust/src/soc/imx8mp/regs.rs` | + `mipi_dsi` (generated) |
| `driver/rust/src/model.rs` | + DSIM and LCDIF1 blocks |
| `driver/rust/src/soc/imx8mp/dsi_chain_tests.rs` | the chain from cold, ordering, golden |
| `driver/drivers/display/display_yakogl_imx8mp_lcdif.c` | display device, ISR -> k_sem, adopt-only facade |
| `driver/drivers/mipi_dsi/dsi_yakogl_samsung.c` | MIPI-DSI device |
| `driver/include/yakogl_drv/imx8mp_lcdif.h`, `samsung_dsim.h` | the driver-specific calls |
| `driver/src/display/imx8mp_lcdif_rust.c`, `imx8mp_dsim_rust.c` | the facade's internal API over the devices |

## Host tests (`cargo test -p yakogl-drivers`)

- `ip::dphy_timing`: the board rate; the old table misses HS_EXIT and
  CLK_POST; every rate 80-1500 Mbit/s in 100 kHz steps either meets every
  minimum with each field tight (one cycle less would miss) or is refused
  because a PREPARE window holds no whole cycle count (only below
  160 Mbit/s); round-up at exact boundaries.
- `ip::samsung_dsim`: the board PLL = golden PMS; every setting inside every
  window, nearest within one PFD step; out-of-window refusals; PLLTMR; the
  plan (CLKCTRL and CONFIG = golden); bad links refused before any write; the
  R4 rate refused at 426.6 and derived at 429.
- `ip::lcdifv3`: adopt with no address writes nothing, a second adopt keeps
  the first capture, the first buffer brings our layout, release gives back;
  refusals; polled flip with and without frame boundaries; VS_BLANK enable.
- `soc::imx8mp::dsi_chain_tests` (media + LCDIF1 + DSIM + SN65DSI84 on one
  model): ordering (PHY reference root before PllEn, PLLTMR before PLLCTRL,
  PllStable polled before SwRst, everything SwRst clears written after it,
  one FIFO initialisation, LCDIF configured before the DSIM and enabled after
  it is held, standby before the LCDIF, release after); ForceStopstate held
  and the HS request set at the bridge's soft reset (an Env wrapper watches
  the I2C write); failure injection (no PLL reference: PLL step timeout,
  nothing after it written; stuck software reset; stuck lanes; a bad link
  writes nothing); disable; the golden comparison with its allow-list (below)
  and the R4 variant.
- `ffi::imx8mp_dsi`: the C structs end to end (cold, then a second instance
  adopting what the first left), the header's flag and IRQ bit values.

Golden allow-list (`dsi_chain_tests::ALLOWED`; every other owned LCDIF1 /
DSIM register equals the board's Linux): TIMEOUT (reset value kept),
MHPORCH (HBP 43 vs 42), PLLTMR (250 000 vs 0x8000), PHYTIMING x 3 (the table
above). R4 adds PLLCTRL (439.5 MHz; HBP is 43 there too).

## Board plan (a follow-up task; the Verdin is busy today)

1. Images: gc_slint and gc_display_raw with both switches on, FORCE_COLD +
   FORCE_COLD_POWER_OFF; the same with the switches off as control.
2. Staged cold bring-up with `CONFIG_YAKOGL_DRV_DISPLAY_IMX8MP_STOP_AFTER`: 4
   (LCDIF configured: Linux back on time = no bus hang), 5 (DSIM held: the
   record's health word must show DSIM_PLL_STABLE and DSIM_HS_CLOCK, the DSIM
   data step 5), 6 (bridge: BRIDGE_PLL_LOCK -- the bridge's PLL runs from the
   DSI clock, so this proves the clock lane in HS while held), 7 and 0
   (picture by camera, judged against the reference ramp; the bridge's
   IRQ_STAT 0).
3. Register image after the cold path, printed on the panel console (the
   facade's adopt path can read every owned register): compare with the
   golden dump through the host test's OWNED / ALLOWED lists; any difference
   not on the list is a finding.
4. Verdin adopt run (crash route, switches on, no FORCE_COLD): picture kept,
   flips by address, `imx8mp_display_off()` detaches the DSIM.
5. Armadillo-X2 adopt run (`CONFIG_YAKOGL_DRV_DISPLAY_IMX8MP_LCDIF_RUST=y`,
   gc_gl_triangle, the existing `yakogl,imx8mp-lcdif-adopt` node): the
   triangle on LCDIF3, nothing written before the first buffer (the
   2026-10-01 wedge), off hands back Linux's buffer and layout.
6. R1: after 10 s of gc_slint, the LCDIF data's last IRQ status has no
   UNDERRUN with `desc3-extra` 0; else `desc3-extra = <0x220000>` (the
   replaced driver's value) as a measured fact.
7. If the picture fails with all health bits set: the PHYTIMING experiments
   above, then MHPORCH HBP 42.

## Risks and open questions

- The P/M/S split, the PHYTIMING unit, the PLLTMR counter clock, the
  blanking word-count conversion and the SwRst self-clearing are not in the
  RM; each is stated in `ip/samsung_dsim.rs` with where it came from.
- The VCO range conflict inside the RM (2100 vs 1500 MHz).
- HS_TRAIL n = 1 (forward HS) is the specification's minimum; the old table
  had 7 cycles. A receiver that wants more shows SoT/EoT errors in IRQ_STAT.
- The D-PHY v1.2 table / section number (citation pending).
- `ffi/hdmi.rs`'s adopt captures the previous owner's layer on every call,
  so a second adopt would make "off" hand back our own buffer (the bug
  `imx8mp_lcdif_adopt.c` fixed on 2026-10-02); `ip::lcdifv3::adopt` has the
  fixed policy and the HDMI front end could use it. Not changed here.
- The new nodes carry no `power-domains`: until step 8 moves the policy, a
  Zephyr display stack that drives `yakogl,imx8mp-lcdif` on its own must have
  the media domains on (the facade does it).
