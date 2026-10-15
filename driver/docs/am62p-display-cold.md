<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Verdin AM62P: the display path in Rust, from the TRM

Status (2026-10-02): **runs on the Verdin AM62P** and is the only
implementation. A register dump taken while Linux drove the panel is the
golden test's data; the drivers were brought up one at a time against the C
drivers of the same tree and then together (what that settled: "What the
board settled"), no fallback was needed, and the C drivers, the `_RUST` switches and
`display_calc` were deleted. A human look at the panel is still owed: the
evidence is the DSS counters, the bridge's IRQ_STAT, the GPU read-backs and
dEQP, not the picture.

This is docs/source-form.md group E, steps 6 and 7. It replaces the
GPL-derived `driver/drivers/display/display_am62p_dss.c` (after Linux tidss),
`driver/drivers/mipi_dsi/dsi_cdns_j721e.c` (after Linux cdns-dsi and
cdns-dphy) and the `dss_vp_regs_from_mode` / `cdns_dsi_calc` /
`cdns_dphy_pll_calc` parts of `driver/lib/display_calc/display_calc.c`. The
Linux drivers were not read for this work; the replaced C files were read for
their API, devicetree and board parameters and for the original parts kept
below. The SN65DSI83 bridge behind the DSI host is already Rust
(`ip::sn65dsi8x`, step 2) and is unchanged.

## Documents

- **AM62Px Sitara Processors Technical Reference Manual**, Texas Instruments,
  literature number **SPRUJ83D**, December 2023, revised January 2026
  (19927 pages; https://www.ti.com/lit/pdf/spruj83). Cited by section number;
  no text or figures are copied into the repository. Chapters used: 12.9.1
  Display Subsystem (DSS) with 12.9.1.4.1 DISPC, 12.9.2 MIPI DSI Controller
  with its programming guide 12.9.2.7, 12.6.3 MIPI D-PHY Transmitter, and the
  register chapters 14.8.9.1 (DSS), 14.8.9.2 (DSS_DSI) and 14.8.6.3 (DPHY_TX).
- **TI's Zephyr driver of PR #114866** ("Add support for MIPI DSI with the
  Cadence DSI controller", open), `drivers/mipi_dsi/dsi_cdns_ti.c` at commit
  2473da6b52b78f5bd428280f56d7cb47bd104eea, Apache-2.0, Copyright 2026 Texas
  Instruments, **vendored unmodified** at
  `third_party/zephyr-pr114866-2026-07-29-2473da6b/` (UPSTREAM file, sha256
  pinned, NOTICE entry 11). It is the source only where the TRM is silent
  (table below), read by `tools/ti-zephyr/gen_dphy_data.py` into the generated
  `driver/rust/src/soc/am62p/ti_pr114866.rs`; the file itself is not compiled
  (it is a whole driver for another binding, `ti,j721e-dsi`, and the
  clock_control API).
- TI's PR #110013 (`display_tidss.c`, AM62L) was **not** needed: the TRM
  documents everything the DSS driver does.
- hal_ti (d9d5b1c9, the workspace's) has no K3 display headers (only
  simplelink, simplelink_lpf3, mspm0), so there is nothing to generate the
  registers from: `soc/am62p/regs.rs` is written by hand, one `// TRM:`
  citation per constant.

## The chain

```
          DSS1 (TISCI dev 232 + 244)                    DSI0 (TISCI dev 231)      D-PHY TX0 (dev 238)
 DDR --> VID   (0x30226000) --layer 0--+                 controller 0x30500000     0x301C0000
         VIDL1 (0x30222000) --layer 1--+-> OVR2 -> VP2 --DPI--> wrapper 0x30270000  --PPI--> 4 lanes --> SN65DSI83 --> LVDS
         common 0x30220000 (IRQ GIC 30)    0x30228000  0x3022B000    (DPI_0_EN)         (WIZ at +0xF00)   I2C1 0x2C     1024x600
 clocks: DSS func clk (232:8),          VP2 pixel clock     dsi_p_clk (231:2),      PLL ref 25 MHz (238:1),
         VP2 pixel clock (232:4,         = DPI clock         dsi_sys_clk (231:5)     PSM clock (238:16)
         set by TISCI to 51.2 MHz)
```

Clock and power requests are TISCI calls in the C shims (unchanged from the C
drivers, including the 5 % tolerance for a clock another host owns).

## What the TRM documents (and what it does not)

| Fact | TRM | Source used |
|---|---|---|
| DSS register map: common, VID, VIDL1, OVR, VP offsets and fields | 14.8.9.1.2 | TRM |
| DSS soft reset, RESETDONE bits | 12.9.1.4.1.3, 14.8.9.1.2.2-.3 | TRM |
| Shadow registers, GOBIT semantics | 12.9.1.4.1.13, 14.8.9.1.2.271 | TRM |
| VP timing packing (minus-one fields, ranges, PPL multiple of 8) | 14.8.9.1.2.279-.282 | TRM |
| Pixel formats (0x03, 0x07, 0x27) | 12.9.1.4.1.7 | TRM |
| DMA thresholds (high = size-1, low = size-burst), preload rules, burst 8x128 | 12.9.1.4.1.6.1-.2 | TRM |
| MFLAG mechanism | 12.9.1.4.1.6.5 | TRM (threshold values ours) |
| CBA_CFG writable only by a secure host | 12.9.1.4.1.6.6 | TRM (not written) |
| Interrupt aggregation (VP1/2, VID/VIDL1 bits), W1C, EOI | 12.9.1.4.1.5, 14.8.9.1.2.4-.16 | TRM |
| DSI controller register map | 14.8.9.2.2 | TRM |
| DSI start-up order, "DPHY regs before LINK_EN", "VID regs before VID_MAIN_CTL" | 12.9.2.7.1.3, 12.9.2.7.3 | TRM |
| DSI horizontal/vertical timing rules (packet overheads, BLKLINE sizes, REG_LINE_DURATION) | 12.9.2.7.9 | TRM (event-mode HBP corrected, below) |
| WAIT_BURST_TIME (>= 100 ns, 0 forbidden) | 12.9.2.7.2.1, 14.8.9.2.2.24 | TRM |
| Direct command procedure, allowed data types, FIFO depth | 12.9.2.7.5, 14.8.9.2.2.21 | TRM |
| ID_REG / IP_CONF reset values | 14.8.9.2.2.101, .21 | TRM |
| HSTX/LPRX time-out units | "detection value", "clock division ratio" only | not stated: written at the maximum |
| ULPOUT time vs the "1 ms" to leave ULP | 9-bit field in sys clocks vs 12.9.2.7.2.1 | inconsistent: ULP never used, maximum written |
| REG_WAKEUP_TIME value | field only | not stated: a tenth of a line (the replaced driver's), unused in blanking-packet mode |
| MAX_LINE_LIMIT value | field only | not stated: blanking payload - 6 |
| MCTL_DPHY_CFG0 polarity and sequence | "drives dphy_*_pdn/_pso/_rstb output" | **vendored TI driver** (`dsi_hs_init`) |
| D-PHY PLL dividers: IPDIV/OPDIV ranges, FBDIV = ROUND(...) | 14.8.6.3.2.13 | TRM |
| PSM_FREQ "divided psm clock is 1 MHz" | 14.8.6.3.2.16 | TRM |
| LANE_RSTB_CMN "released after APB register programming" | 14.8.6.3.2.15 | TRM |
| CMN start state machines (TBIT2), O_CMN_READY, PLL_LOCK | 14.8.6.3.2.1, .13, .14 | TRM |
| WIZ MOD_VER MODULE_ID 992h | 14.8.6.3.2.12 | TRM (used to find the WIZ) |
| **WIZ registers' offset in the D-PHY region** | summary says 0h-14h | **contradicted by TI's driver (0xF00)**: probed at run time |
| **PCS band table** (PCS_BAND_CTL_REG_L/R) | names band 0 "80_100" only | **vendored TI driver** (`cdns_dphy_tx_bands[]`), risk R7 confirmed |
| **PLL gm PWM divider values** (TBIT10) | fields only | **vendored TI driver** (6, 0x101, 8) |
| PLL lock time | "defined by the PHY documentation" | not public: 1 ms bound |

## The sequences

DSS (`soc::am62p::dss::dss_init`, `ip::ti_dss7`):

1. Wait until SYSSTATUS shows the power-on reset done (FUNC and VP RESETDONE),
   pulse SOFTRESET, wait until SOFTRESET reads 0 *and* RESETDONE is back
   (12.9.1.4.1.3). The C driver polled RESETDONE only, which may never have
   dropped.
2. MFLAG dynamic (MFLAG_CTRL 2); CBA_CFG left alone (secure host only).
3. Each pipeline: high = BUFSIZE-1, low = BUFSIZE-8, preload = low, MFLAG
   LT/HT = 1/4 / 1/2 of the buffer, BUFPRELOAD 0 (12.9.1.4.1.6.2).
4. The port: DATALINES 24, TIMING_H/V, POL_FREQ, SIZE_SCREEN, CONFIG 0,
   overlay background black.
5. Safety layer: VIDL1, ARGB8888, buffer 0, layer 1 enabled; application
   layer 0 (VID) disabled.
6. Interrupts (with an IRQ line): VP VSYNC + SYNC LOST, both pipelines'
   underflow, the first-level bits. The port stays off until
   `am62p_dss_output_enable` (the sample) or `blanking_off`.

The two-plane protocol (kept from the C driver, see "Original logic"):
commits mark themselves pending, write, then GO only if GO is clear (else
they ride along, `go_waits`); never wait; a vsync seen during a commit
rewrites it once; the ISR settles `app_seq_shown` and the safety flip when it
finds GO consumed.

DSI + D-PHY (`ip::cdns_dsi::attach`, `ip::cdns_dphy::power_on`):

1. `init`: ID_REG must be Cadence/D5h, quiesce link/lanes/interfaces/IRQ
   controls, find the WIZ by MOD_VER (0xF00 first, then the TRM's 0).
2. Before LINK_EN (12.9.2.7.1.3): DPHY time-outs, ULPOUT, MAIN_PHY_CTL
   (lanes 2-4, continuous clock, WAIT_BURST_TIME).
3. LINK_EN, PLL_START (Figure 12-551).
4. CFG0 = TI value 0 (all down), D-PHY through its APB: PSM divider, PWM
   dividers, PLL dividers with PLL powered, LANE_RSTB_CMN, band, CMN SSM +
   TX mode; wait PLL_LOCK and O_CMN_READY.
5. Clear PLL lock status, CFG0 = TI value 1, wait PLL_LCK.
6. CFG0 = TI value 2 | D_RSTB(lanes); then CKLANE_EN + DATn_EN (Figure
   12-551 enables the lanes after the PLL); wait CLKLANE/DATn READY.
7. VID_HSIZE/VSIZE/BLKSIZE/VCA/DPHY_TIME, then VID_MAIN_CTL (12.9.2.7.1.3).
8. Wrapper DPI_0_EN, MAIN_DATA_CTL video mode on DPI, IF2_EN (stall off).

## Differences from the replaced C drivers

Each follows from a TRM statement; the board decides whether any matters.

| Register | C driver | Rust | Why |
|---|---|---|---|
| DSS soft reset | poll RESETDONE | poll SOFTRESET clear + RESETDONE | 12.9.1.4.1.3 |
| DSS_CBA_CFG | written (PRI 1/0) | not written | 12.9.1.4.1.6.6: secure host only |
| VID/VIDL1 BUF_THRESHOLD, PRELOAD | low = size/2 | low = size-8, preload = low | 12.9.1.4.1.6.2 |
| MFLAG_THRESHOLD | 2/3, 1/3 | 1/2, 1/4 | ours (TRM gives none) |
| BA_EXT_0/1 | 0 | address bits 47:32 | 14.8.9.1.2.60 |
| DISPC_IRQ_EOI | 0 (no action) | 1 | 14.8.9.1.2.4 |
| DSI VID_VSIZE1 | VSA 11, VBP 22, VFP 12 | VSA 10, VBP 23, VFP 11 | 12.9.2.7.9.1 ("VFP < DPI_VFP") |
| VID_DPHY_TIME line duration | 1012 | 1011 | 12.9.2.7.9.3 (unused in blanking-packet mode) |
| MCTL_MAIN_PHY_CTL WAIT_BURST_TIME | 0 (forbidden) | 4 | 14.8.9.2.2.24, 12.9.2.7.2.1 |
| MCTL_DPHY_TIMEOUT1/2 | a frame | maximum | units not stated |
| MCTL_ULPOUT_TIME | sys_hz/1000, overflowing 9 bits | maximum | field width |
| Lane start | with PLL_START | after PLL_LCK | Figure 12-551 |
| Wrapper DPI_0_EN | first | just before the interface | no order stated; the input is enabled with the interface |
| D-PHY FBDIV | ceil | ROUND, next up if below the request | 14.8.6.3.2.13, 12.9.2.7.8.2 (same value, 394, for the board) |
| D-PHY PLL_CTRL PLL_PD/PSO | 0 (implicitly) | 0 (stated) | 14.8.6.3.2.13 |
| DSI ID check | logged only | refused if not CADh/D5h | 14.8.9.2.2.101 |
| Direct command FIFO | 16 bytes | IP_CONF depth (64) | 14.8.9.2.2.21 |

Everything else (the RVT70's VP timings, HSIZE/BLKSIZE/VCA, pixel mode and
data type, VID_MAIN_CTL, CFG0 values, PLL dividers and band, PSM, PWM, SSM)
is the same value as before; `ip::cdns_dsi::tests::register_image_against_the_replaced_driver`
and `soc::am62p::dss::tests::init_resets_then_programs_the_port_and_the_safety_layer`
pin both lists.

## Devicetree and Kconfig

No binding changes: `safeui,am62p-dss` and `safeui,cdns-dsi-j721e` keep their
properties, and the board DTS and the sample overlays are unchanged. The Rust
DSS shim maps only the overlay and port of `video-port` (VP2 on this board).
`sys-clk-hz` is no longer needed (ULPOUT is written at its maximum) and is
ignored by the Rust shim.

```
CONFIG_SAFEUI_DSS_AM62P=y           (default y with an enabled node; ARM64): display_am62p_dss_rust.c
CONFIG_SAFEUI_DSI_CDNS_J721E=y      (likewise): dsi_cdns_rust.c
```

Either selects `SAFEUI_DISPLAY_RUST` and the crate feature `am62p`. Until
2026-10-02 `SAFEUI_DSS_AM62P_RUST` / `SAFEUI_DSI_CDNS_J721E_RUST` (default n)
chose between these and the C drivers; both are gone with the C drivers.
Init priorities are unchanged (DSI 70, DSS 80, bridge 95).

## Original logic kept (moved to Rust as it was)

From `display_am62p_dss.c`, original to this project: the two-plane safety
layer (VID under VIDL1), the shared never-waited GO with `go_waits`, the
pending-before-GO marking, `app_seq_pending`/`app_seq_shown`, the
safety double buffer and `safety_back_free`, the commit retry after a vsync,
the statistics, the polling fallback. Kept in the C shim: the vsync semaphore
and callback, the `__noinit` safety buffers cleared and cache-flushed at
init, `dss_write` into the front buffer, and the TISCI power/clock code with
its 5 % tolerance. From `dsi_cdns_j721e.c`: the TISCI requests and the PSM
clock fallback.

## Layout

```
driver/rust/src/soc/am62p/regs.rs          registers, by hand from SPRUJ83D (// TRM: per constant)
driver/rust/src/soc/am62p/ti_pr114866.rs   GENERATED from the vendored TI driver (4 facts)
driver/rust/src/soc/am62p/dss.rs           DSS1 as the board uses it; the two-plane protocol
driver/rust/src/ip/ti_dss7.rs              DSS7 DISPC: reset, FIFOs, VP timing packing, planes, GO, IRQs
driver/rust/src/ip/cdns_dsi.rs             Cadence DSI host: timings, attach, direct commands
driver/rust/src/ip/cdns_dphy.rs            D-PHY TX + WIZ: PLL, band, power-up, WIZ probe
driver/rust/src/ffi/am62p.rs               C entry points; layouts pinned in include/yakogl_drivers.h
driver/rust/src/model_am62p.rs             the host model (shadow/GO, resets, PLL/lanes, W1C)
driver/drivers/display/display_am62p_dss_rust.c   shim: device, am62p_dss API, ISR, TISCI, buffers
driver/drivers/mipi_dsi/dsi_cdns_rust.c           shim: mipi_dsi device, TISCI
tools/ti-zephyr/gen_dphy_data.py                  generator (check.sh: regenerate == committed)
third_party/zephyr-pr114866-2026-07-29-2473da6b/  TI's driver, unmodified
```

## Host tests (`cargo test -p yakogl-drivers`)

- DSS: init order (reset first and complete, CBA untouched, FIFO values,
  timings, safety layer, IRQ enables), reset never done / stuck, bad
  configuration refused before any write, commits with the port off, the
  shared GO (one GO write for two writers, `go_waits`, nothing latched before
  the vsync, everything after), GO never consumed, a vsync during a commit
  (rewritten, latched whole), ISR W1C of exactly what was read + EOI,
  polling, plane validation and layout (RGB565 padding, 48-bit address),
  premultiplied alpha on VIDL1 only, safety buffer alternation.
- VP packing: the display_calc ztest values for the RVT70 (ported), field
  limits, polarities.
- D-PHY: the display_calc PLL test (ported: 2/8/394, band 5), ROUND vs next
  divider, the TRM's range tables and gaps, bands, PSM divider.
- DSI: the display_calc timing test (ported, with the two TRM differences),
  the TRM's 1920x1080 worked example (pulse and event, the line sums),
  refusals, WAIT_BURST_TIME, init identification/quiesce/WIZ probe (both
  offsets, none, foreign ID), the attach order against the TRM, the register
  image against the replaced driver, failures (no lock, no CMN ready, lanes
  stuck, impossible rates) and bounded waits, the WIZ at the TRM's offset,
  detach, direct commands, the FFI structs.

- Golden (`soc::am62p::display_golden`): the model's image after
  `cdns_dsi::init` + `attach` and `dss_init` + `output_enable`, read through
  the model, against the Linux dump, outside the allow-list.

The `driver/tests/display_calc` ztest that tested the C arithmetic went with
the C drivers.

## Board plan (a): the golden dump's read list

The golden dump is taken while Linux runs the panel (Torizon;
read-only, 32-bit reads with `devmem2`/`memtool`). Safe ranges (no
read-to-clear, no FIFO pops):

```
DSS1 common   0x30220000 + 0x00..0xA0          (IRQ status reads do not clear)
DSS1 VIDL1    0x30222000 + 0x20..0x3C, 0x1FC..0x230
DSS1 VID      0x30226000 + 0x20..0x3C, 0x1FC..0x230
DSS1 OVR1/2   0x30227000 / 0x30228000 + 0x00..0x28
DSS1 VP1/2    0x3022A000 / 0x3022B000 + 0x00..0x58
DSI wrapper   0x30270000 + 0x00..0x10
DSI ctrl      0x30500000 + 0x000..0x07C, 0x084, 0x088, 0x0A4, 0x0A8, 0x0B0..0x0F8,
                           0x100..0x114, 0x130..0x148, 0x170..0x188, 0x1A0..0x1AC, 0x1FC
              (skip 0x080 SEND, 0x08C RD_INIT, 0x090 WRDAT, 0x094 FIFO_RST,
               0x0A0 RDDAT (pops the RX FIFO), 0x150..0x168 the *_CLR registers)
D-PHY         0x301C0000 + 0x000..0x014 (the TRM's WIZ place: does MOD_VER 69924001h read here?),
                           0x020, 0x040, 0x04C, 0x050, 0x154, 0x26C, 0x36C, 0x46C, 0x56C,
                           0xB00, 0xB04, 0x0F8, 0x0FC, 0xF00..0xF14, 0xFF8, 0xFFC
```

Plus the SN65DSI83 i2cdump. The dump is
`driver/docs/am62p-display-dump-linux.txt` (taken 2026-10-01); the golden
test `driver/rust/src/soc/am62p/display_golden.rs` compares the model's image
after `dss_init` + `attach` + `output_enable` with it, masked by
`driver/docs/am62p-display-allow.txt` (address registers, status and event
bits, and this project's choices, each with its reason).

## What the board settled (2026-10-02)

The staged runs are in `tools/cts/log/board-sessions/am62p-display-rust-2026-10-02.md`.
What they settled:

- **The WIZ offset.** MOD_VER (MODULE_ID 992h, 69924881h on this silicon)
  reads at +F00h, and 0h..14h read 0. TI's driver is right; the SPRUJ83D
  register summary (14.8.6.3.2: MOD_VER at 0h) is not. The probe keeps the
  fallback to 0h and logs where it found it.
- **The TRM's values work.** VSIZE1 10/23/11, WAIT_BURST_TIME 4, FIFO low =
  size-8 with preload = low, MFLAG 1/4..1/2, lanes after PLL_LCK, no
  CBA_CFG write, EOI 1 and maximum time-outs gave 0 underflows and 0 sync
  losses in 40 s with a static plane and in 60 s of 58 fps flips, with the
  same vsync count as the C drivers (58.8 Hz = 51.2 MHz / (1349 x 645)).
  Neither fallback was needed. Linux itself agrees with the C drivers on
  those values, so they stay in the seeded part of the allow-list.
- **The model's reset values.** The golden comparison found TRM reset
  values the model lacked (GLOBAL_BUFFER 4688h, ATTRIBUTES2 3C000000h,
  MCTL_3DVIDEO_CTL 10h, CMD_MODE_CTL2 7FF800h, DPI_CFG 8000000h, the
  wrapper's REVISION 640A1101h; IRQENABLE_CLR reads the enables). They are
  in the model. No driver change came from the comparison. The PSM clock is
  16 MHz on the board.

## Open questions

- A human look at the panel with the Rust drivers is owed (nobody could see
  it on 2026-10-02); the counters, IRQ_STAT and read-backs cannot show a
  colour or position error in the scan-out itself. On 2026-10-14 the owner
  reported the panel black (backlight on) under a kexec'd yakogl; the last
  picture seen was the C drivers' (2026-09-14). On 2026-10-15 the board's
  own register image under a red-fill run was read and equals the Linux
  dump except the allow-list and the application plane; the bridge read
  IRQ_STAT 0 with video flowing. The picture is still unseen. The
  C-vs-Rust differences that a register image cannot show, the
  known-colour fill (`CONFIG_PVR_SAMPLE_DISPLAY_FILL`), the register dump
  (`CONFIG_PVR_SAMPLE_DISPLAY_DUMP`), the board readings and the
  procedure are in
  `tools/cts/log/board-sessions/am62p-display-black-2026-10-15.md`.
- The WIZ offset is answered (above); the TRM summary could be reported to TI.
- The DSS and DSI Kconfig symbols now depend on ARM64 (the Rust staticlib is
  built for aarch64-unknown-none-softfloat); an R5F/M4 build that compiled the
  C drivers would no longer get them.
- The TRM's event-mode HBP ("- 12", example 24) contradicts its own line sum
  (needs 16); reported here, worth a TI e2e question.
- MFLAG thresholds and the TRM's FIFO thresholds: 0 underflows in the runs
  above; a heavier memory load (Qt Quick 3D with the GPU busy) has not been
  measured with them.
- The vendored TI driver says its PWM values are "from Linux j721e_dphy_ops";
  it carries TI's Apache-2.0 notice. The owner's rule vendors it as TI
  published it; its provenance inside TI is TI's statement.
