<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# i.MX8MP display bring-up: what is written where, and why

This is the note for the default C LCDIF and DSIM drivers here
(`imx8mp_lcdif.c`, `imx8mp_dsim.c`). Their Rust replacements are behind
`YAKOGL_DRV_DISPLAY_IMX8MP_LCDIF_RUST` and `YAKOGL_DRV_DSIM_RUST` (default n;
`driver/docs/imx8mp-display-cold.md`). Since 2026-10-03 the chain is also
brought up from a cold U-Boot entry ("The panel console" below).

Status on 2026-09-21: runs on the board. `gc_display_raw` (the driver alone,
a colour ramp in a buffer of its own) reports all six health bits from the
hardware after both the adopt path and a forced cold bring-up
(`imx8mp_display_health()`: LCDIF scanning, DSIM PLL stable, DSIM HS clock,
bridge ID over I2C, bridge PLL locked, PWM3 enabled), and the register
values now match what the board's own Linux leaves in the hardware while
the panel is on. The sections below were written from the mainline drivers
before that; "Measured on 2026-09-21" at the end says where the hardware
disagreed with them.

## Measured on 2026-09-21

The whole chain was dumped from Linux with the panel on (`mmio_rd`, and
`i2cdump -f -y 1 0x2c` for the bridge). Three findings:

1. **The normal kexec route leaves the LCDIF, the DSIM and the bridge
   running and only turns the backlight off** (pwm_bl's shutdown). So the
   driver adopts the chain and, since this change, turns PWM3 back on when
   it finds it disabled. The "dark panel" this file's opening paragraph
   describes was a dark backlight over a live picture.
2. **The board runs NXP's downstream DSIM driver (sec-dsim), not mainline
   samsung-dsim**, and five registers differ from what mainline would write.
   The driver now writes the board's values: LCDIF `CTRL` = `0x08`
   (`INV_PXCK`), DSIM `MVPORCH.CMD_ALLOW` = 0, `PLLTMR` = 0x8000, `CONFIG` =
   `0x0600707f` (no `MFLUSH_VS`), `ESCMODE` stop-state count 0, `TIMEOUT` =
   `0x00ff00ff`, and the D-PHY timings from a table (`PHYTIMING1`
   `0x03120904`, `PHYTIMING2` `0x00040507`) instead of the rounded D-PHY
   minimums, which put `CLK_PREPARE` a hair under 38 ns.
3. **Clocks the board actually uses:** `media_disp1_pix` = VIDEO_PLL1
   (1039.5 MHz) / 15 = 69.3 MHz, not sys_pll2_1000m / 14; `media_mipi_phy1_ref`
   = 24 MHz / 2 = 12 MHz with the DSIM PLL at P5 M711 S2 = 426.6 MHz. The
   driver keeps its 71.43 MHz / 24 MHz / 428 MHz plan, which is inside every
   window; the bridge's DSI clock range is derived from the HS clock it was
   given. If a picture ever fails on the cold path with the health bits all
   set, matching these three exactly is the next step.

Diagnostics that made this possible, all in the i.MX8MP board layer (`driver/zephyr/lib/board/imx8mp`, once the samples' `gc_common`):
the hold time encodes the health bits (`gc_sample_hold_code`, 70 s + 3 s
per unit), a fatal error's exception class / PC / address / fault status
(`GC_SAMPLE_FATAL_ENCODE_*`), and `FORCE_COLD` tears a running chain down
so the cold path can be exercised whatever Linux was showing.

## The problem this solves

The normal kexec path runs Linux's `device_shutdown()`. The DRM drivers turn
the backlight off, stop the LCDIF and put the bridge in reset, so a Zephyr
image jumped to afterwards finds a dark panel. On this carrier (Mallow, debug
header X11 unassembled) the panel is the only console there is, so a dark
panel means a run reports nothing at all.

The kexec **crash** route (`kexec_zephyr zephyr.bin 0x60000000 --crash`) does
not call `device_shutdown()`, so there the display is still running. Both
states are real and the code detects which one it is in from the LCDIF's own
enable bits (`CTRLDESCL0_5_EN` and `DISP_PARA_DISP_ON`), never from a build
option.

## The chain

```
 CCM media_disp1_pix ─┐
                      ▼
 LCDIFv3 0x32e80000 ──► MIPI DSIM + D-PHY 0x32e60000 ──► SN65DSI8x ──► LVDS panel
     (MEDIAMIX pd)          (MIPI_PHY1 pd)               I2C2 @0x2c     1280x800
                                                         EN GPIO4_28
 backlight: PWM3 0x30680000 on GPIO1_IO14, 150 Hz
```

From the live device tree: `lcd-controller@32e80000` (`fsl,imx8mp-lcdif1`),
`mipi_dsi@32e60000` (`fsl,imx8mp-mipi-dsim`), `blk-ctrl@32ec0000`
(`fsl,imx8mp-media-blk-ctrl`), `i2c@30a30000/bridge@2c` (`ti,sn65dsi83`,
`enable-gpios = <&gpio4 28 0>`, `data-lanes = <1 2 3 4>`),
`panel-rvt101hvdwc00b` (`panel-lvds`, `data-mapping = "vesa-24"`),
`backlight-verdin-dsi` (`pwms = <&pwm3 0 6667179 0>`).

Two naming traps:

* Linux's `i2c@30a30000` is **I2C2**, and Zephyr calls it `&i2c2` too. The
  other `bridge@2c`, under `i2c@30a50000` (I2C4) and marked `ti,sn65dsi84`,
  is `status = "disabled"` on this board. The live one is the I2C2 node.
* The part is an SN65DSI84 on the schematic but Linux drives it with
  `compatible = "ti,sn65dsi83"`, i.e. one LVDS link. That is the
  configuration programmed here: `LVDS_LINK_CFG = 1` (channel A only) and
  `CHB_DSI_LANES` left at its default of 3.

## Clock tree

| Clock | Source | Value |
| --- | --- | --- |
| media_axi (CCM root 20, 0x8a00) | SYSTEM_PLL2_CLK (mux 1) / 2 | 500 MHz |
| media_apb (CCM root 21, 0x8a80) | SYSTEM_PLL1_CLK (mux 2) / 4 | 200 MHz |
| media_disp1_pix (CCM root 124, 0xbe00) | VIDEO_PLL1 (mux 1) 1039.5 MHz / 15 | **69.3 MHz** |
| media_mipi_phy1_ref (CCM root 123, 0xbd80) | 24M_REF_CLK (mux 0) / 2 | 12 MHz |
| media gate | CCGR93 (0x45d0), SET alias | on |
| pwm3 (CCM root 105, 0xb480) | 24M_REF_CLK (mux 0) | 24 MHz, CCGR42 (0x42a0) |

Sources, IMX8MPRM Rev. 3, 08/2024: the slice numbers, offsets, mux inputs
and maximum rates are Table 5-1 "Clock Root Table" (media_axi at most
500 MHz, media_apb 200, media_mipi_phy1_ref 125, media_disp1_pix 250); the
gate is Table 5-9 (CCGR93 "MEDIA"); a root's rate is source / (PRE_PODF+1)
/ (POST_PODF+1) (5.1.6.4 "Target Interface"); SYSTEM_PLL1_CLK is 800 MHz
and SYSTEM_PLL2_CLK 1000 MHz (5.1.5.1, input clock table). The driver does
not take those two rates on trust: it reads SYS_PLL1/SYS_PLL2 back from
CCM_ANALOG (locked, out of reset, output gate open, not bypassed, 24 MHz
reference) and computes each rate with the PLL formula of 5.1.5.4.4 before
it switches a root to them (5.1.6.4: the Target Interface needs an active
source). media_axi and media_apb get the fastest rate not above their
Table 5-1 maximum, which is /2 and /4.

**Pixel clock.** The Riverdi panel's `panel-timing` allows 66.3 MHz (min),
71.1 MHz (typical) and 78.9 MHz (max) (the board's Linux devicetree,
/proc/device-tree/panel-rvt101hvdwc00b, read 2026-10-02). The pixel root's
candidate parents (Table 5-1, slice 124) are VIDEO_PLL1, SYSTEM_PLL2_CLK,
SYSTEM_PLL1_CLK and the 24 MHz oscillator, each offered only when it reads
back as running; the search takes the nearest rate at or below 250 MHz.

The device tree asks for the board's Linux plan, **69.3 MHz**:
`video-pll1-frequency = <1039500000>` (golden dump `0x3036002c = 0x000ad021`,
`0x30360030 = 0x4000`), 1039.5 MHz / 15 = 69.3 MHz, **58.761 Hz**. It
satisfies four limits:

1. the panel: 66.3 <= 69.3 <= 78.9 MHz;
2. the DSI link carries the pixels: pixel x 24 bpp / 4 lanes = 415.8 Mbit/s
   <= the 426.6 Mbit/s lane rate (`burst-clock-frequency`, DSIM PLL p 5 m 711
   s 2 from 12 MHz). The SN65DSI84's fields do not depend on the pixel clock:
   DSI clock 213.3 MHz (`CHA_DSI_CLK_RANGE` 42), divider 213.3 / 69.3 = 3,
   LVDS clock 71.1 MHz, as Linux has them (0x0a 0x85, 0x0b 0x10, 0x12 0x2a);
3. VIDEO_PLL1: m + k/65536 = 1039.5 x 2 x 2 / 24 = 173.25, so m 173, k 16384;
4. **the DSI link has room for the packet overhead.** The lane rate must
   exceed pixel x 24 / lanes by `ip::samsung_dsim::LINK_MARGIN_PERMILLE` (25
   per mille): every line also carries the headers and checksums of its sync,
   blanking and pixel packets and the HS exit and entry of burst mode. The
   figure is measured, not derived (how `MHPORCH`'s word counts map onto the
   line has no RM basis): no margin broke the picture twice -- 71.43 MHz on
   428.6 Mbit/s (2026-09-23, "disturbed, flickers, as if not in sync") and
   71.1 MHz on 426.6 Mbit/s (2026-10-06, issue #23: one and a half copies of
   the frame across each line, on gc_gl_triangle's window and on the panel
   console of gc_gl_test alike) -- and 69.3 MHz on 426.6, 25.97 per mille, is
   the plan the panel was seen right at (2026-09-24, and under Linux).

`soc::imx8mp::dsi_chain_tests::verdin_dt` reads the device tree and checks
all four (`limit_4_the_dsi_link_has_the_packet_overhead_margin` names the
margin when it fails), `ip::samsung_dsim::plan` refuses a link without the
margin, and `imx8mp_display.c` has the same rule as a `BUILD_ASSERT` on the
node (`YD_SAMSUNG_DSIM_LINK_MARGIN_PERMILLE` in yakogl_drivers.h, which a
test holds equal to the Rust constant). A device tree asking for 71.1 MHz on
this link no longer builds.

The DSIM's blanking word counts at 69.3 MHz are HFP 21, HBP 43, HSA 12. Linux
has HBP 42: its DSIM is set up for the 71.1 MHz mode while its CCM gives the
LCDIF 69.3 MHz. 43 is what ran on 2026-09-24 (the cold path seen right). The
adopt path keeps whatever the previous owner programmed (Linux: 69.3 MHz).

**The 60 Hz plan, withdrawn (2026-10-02 to 2026-10-06).** The device tree
asked for the panel's typical 71.1 MHz (VIDEO_PLL1 1066.5 MHz, m 178,
k -16384; 60.286 Hz) on the same 426.6 Mbit/s link: 71.1 x 24 / 4 = 426.6
exactly, no margin. Its runs measured the refresh (60.28 Hz) and PASS records,
but nobody looked at the panel until 2026-10-06, which showed the defect of
issue #23. A 60 Hz plan needs a faster link: the bridge's divider 4 (DSI
clock 284.4 MHz, 568.8 Mbit/s a lane, `CHA_DSI_CLK_RANGE` 56, DSIM PLL p 5
m 474 s 1) would give 33 % -- unmeasured. **The M7 still re-tunes
VIDEO_PLL1 to 71.1 MHz when it adopts the panel** (its `src/pixclk_m7.c`,
`CONFIG_YAKOGL_DRV_M7_PIXEL_CLOCK_HZ`): on this 426.6 Mbit/s link that is the same
no-margin plan, and route 3 is exposed to issue #23 until that repository
sets it to 0 or to 69.3 MHz.

Without `video-pll1-frequency` (risk R4 in docs/source-form.md) the nearest is
SYSTEM_PLL2_CLK / 14 = 71.43 MHz, above the 426.6 Mbit/s link; on a 428 MHz
link it broke the picture up (2026-09-23).

VIDEO_PLL1 is a fractional PLL: `FOUT = (m + k/65536) x 24 MHz / (p x 2^s)`
(5.1.5.4.4, with 1 <= p <= 63, 64 <= m <= 1023, 0 <= s <= 6,
-32768 <= k <= 32767). The RM gives no VCO range, so p = 2 and s = 1 are
kept as the board's Linux has them and m, k follow: 1039.5 MHz is m 173,
k 16384; 1066.5 MHz is m 178, k -16384. The programming order (reset low, dividers, 3 us, reset high, wait
for `PLL_LOCK`, then the output gate) and the 1 ms lock bound are not in the
RM; they are the order that ran on this board.

With htotal 1404 (1280 + 36 + 24 + 64) and vtotal 840 (800 + 8 + 8 + 24),
69.3 MHz is **58.761 fps** (71.1 MHz: 60.286).

Measured on the Verdin (2026-10-02, gc_slint PERF page 7, which times 60 vblanks
after the run): the cold path (FORCE_COLD) at the then device tree's 71.1 MHz
**60.28 Hz, 16 587 us** a frame (the refresh; the picture was not looked at); the adopt path at Linux's 69.3 MHz 58.76 Hz, 17 018 us. A
script polling LCDIF1's VSYNC bit from Linux counted 58.756 Hz before the
M7 started and 60.284 Hz after its re-tune.

**DSI link** (the analysis of the old SYS_PLL2 / 14 plan; the device tree now
states `burst-clock-frequency = <426600000>` and a 12 MHz reference, as the
board's Linux runs). 71 428 571 Hz x 24 bpp / 4 lanes = 428 571 426 Hz wanted per
lane. `samsung_dsim_pll_find_pms()` with fin 24 MHz, m in 64..1023 and a VCO
of 1050..2100 MHz gives:

```
p = 3, m = 214, s = 2   VCO = 24 MHz * 214 / 3      = 1712 MHz
                        fout = 24 MHz * 214 / (3<<2) =  428.000 MHz
byte clock  = fout / 8                               =   53.500 MHz
esc divider = ceil(53.5 / 20)                        =    3
esc clock   = 53.5 / 3                               =   17.833 MHz
```

The escape-clock target of 20 MHz is the value the i.MX8MP EVK device trees
give `samsung,esc-clock-frequency`; the PLL reference of 24 MHz is mainline
`imx8mp.dtsi`'s `samsung,pll-clock-frequency`. Note that the **board's own
Linux** sets `media_mipi_phy1_ref` to 12 MHz instead, because the NXP
downstream `sec-dsim` driver expects that; the device tree now asks for the
same 12 MHz (`pll-ref-frequency`), and the driver programs the root itself
either way, so it does not matter which one Linux left behind.

## Register sequences

### 1. Media power (`soc::imx8mp::{gpc, ccm, anatop, media_blk, media}`)

Rust in `driver/rust/src/soc/imx8mp/`, registered as Zephyr power-domain
devices by `driver/drivers/power_domain/power_domain_yakogl_imx8mp_media.c`
(`yakogl,imx8mp-media-blk-ctrl` with the children `pd-lcdif1` and
`pd-mipi-dsi1`, `yakogl,imx8mp-media-pd`). Written from IMX8MPRM Rev. 3,
08/2024; names and bits from hal_nxp `PERI_GPC.h`, `PERI_GPC_PGC.h`,
`PERI_CCM.h`, `PERI_CCM_ANALOG.h`, `PERI_MEDIA_BLK_CTRL.h` (generated,
`tools/hal-nxp/gen_regs.py`).

| Domain | PU trigger bit | A53 mapping bit | PU PGC | ADB400 handshake |
| --- | --- | --- | --- | --- |
| MEDIAMIX | 10 | 12 | 10 (0xD80) | request 14, acknowledge 30 |
| MIPI_PHY1 | 0 | 2 | 0 (0xB00) | none |

(5.2.10.37 `PU_PGC_SW_PUP_REQ`, 5.2.10.72 `PGC_CPU_A53_MAPPING`, the PGC
list of 5.2.10, 5.2.10.57 `PU_PWRHSK`.)

MEDIAMIX on, in this order:

1. SYS_PLL1 and SYS_PLL2 read back as running, then media_axi and
   media_apb (5.1.6.4, Table 5-1); then the MEDIA gate CCGR93 for domain 0
   through its SET alias (5.1.6.1, 5.1.6.3), after its roots (5.1.5.5: a
   root must be enabled for the LPCG that gates it).
2. GPC power-up, 5.2.9.4 "Example Code 4" and 5.2.8 rule 2: map the domain
   to the A53, set its `PU_PGC_SW_PUP_REQ` bit, wait for it to self-clear
   (bounded at 1 ms: the PGC's sequence is under 5000 counts of its 66 MHz
   clock at the reset values, 5.2.11.10 / 5.2.11.12), read
   `A53_PU_PGC_PUP_STATUS0..2` (5.2.10.45: a 1 = the trigger failed), clear
   the PGC's `PCR` (5.2.11.9) and, if `PSR` reported an earlier power-down,
   clear it (5.2.11.12). Then the ADB400 request (`PU_PWRHSK` bit 14; the
   RM describes it as an active-low power-down request).
3. MEDIA_BLK_CTRL bus: `CLK_EN.SFT_EN_BUS_BLK_CLK`, then
   `SFT_RSTN.SFT_EN_BUS_BLK_CLK_RESETN` (13.2.3.1.2 / .3). This is the first
   access to the block control: after its clock root, its gate and the
   power-up.
4. Wait for the ADB400 acknowledge (bit 30, bounded at 1 ms). Not stated in
   the RM: the acknowledge returning to 1 after the request (5.2.10.57 only
   describes power-down) and that it is waited for once the bus clock runs;
   the replaced driver did not wait at all.
5. `LCDIF_ARCACHE_CTRL.GPR_LCDIF_0_RD_HURRY` = 7 (13.2.3.1.14 names the
   field only; 7 in LCDIF_0 alone is what the board's Linux has, golden
   `0x32ec004c = 0x00001c00`; the replaced driver also set LCDIF_1 to 7).

Per sub-domain, after MEDIAMIX: MIPI_PHY1 powered (mipi-dsi1 only, same
GPC sequence, no handshake); the block's clocks enabled, then its resets
released, and for the DSI the D-PHY reset `MIPI_RESET_DIV.GPR_MIPI_M_RESETN`
(13.2.3.1.4) last. LCDIF1 is `SFT_EN_LCDIF_{PIXEL,APB,AXI}` (Figure 13-5:
the unnumbered LCDIF names are LCDIF1), MIPI_DSI1 `SFT_EN_MIPI_DSI_{PCLK,
CLKREF}`. The order of clock enable and reset release, and the 5 us, are
not in the RM (13.2.2 says only that the block "contains clock distribution
and gating controls, as well as reset handling"); they are the replaced
driver's.

The cold path of `imx8mp_display_on()`: stage 2 is both domains on; stage 3
is VIDEO_PLL1, the pixel root, the PHY reference root and a reset pulse of
LCDIF1 and MIPI_DSI1 (resets asserted, clocks on, 5 us, released) so both
blocks start on their new clocks.

Power-down is the reverse (5.2.9.4, 5.2.10.40, 5.2.10.50, 5.2.10.57): the
block's resets asserted and clocks gated; MIPI_PHY1 down with the DSI; the
last domain off drops the ADB400 request, waits for the acknowledge to read
0, sets `PCR` and triggers `PU_PGC_SW_PDN_REQ`. FORCE_COLD (with
`FORCE_COLD_POWER_OFF`, the default) does this after the teardown, so the
cold path powers MEDIAMIX up from off.

A domain whose state says it is on is left alone, so the adopt route and a
domain the previous kernel left on cost no write: MEDIAMIX when it is mapped,
no trigger is pending or failed and the ADB400 has acknowledged; MIPI_PHY1
when it is mapped, clean and the DSI is clocked and out of reset. That is an
inference from the status bits (the GPC has no "is powered" bit). `PSR` is
not used for it: it keeps reporting an earlier power-down until software
clears it, which Linux need not have done.

`imx8mp_media_status()` returns a word for a record without a console
(`YD_MEDIA_ST_*` in `driver/rust/include/yakogl_drivers.h`); gc_slint puts
it in w2 of its record when the panel never came up.

Measured on the board (golden-linux-2026-09-21.txt): the GPC PGC range at
0x303a0800 bus-faults when read from Linux user space, so the PGC registers
are not in the dump; `PU_PWRHSK` 0xda6ffa7f (MEDIAMIX request and
acknowledge both 1), `PGC_CPU_A53_MAPPING` 0x003fffff, `SFT_RSTN`
0x01ffffff, `CLK_EN` 0x00800133, `MIPI_RESET_DIV` 0x00020000. The host test
`soc::imx8mp::media::tests::golden_cold_bring_up_matches_linux_except_the_allow_list`
compares the modelled cold bring-up with these lines.

**Early clocks.** Linux's i2c-imx and pwm-imx gate their clocks while idle
(runtime PM), and a kexec'd image inherits that: clk_summary on the running
board shows i2c2 at enable=0 even while the display is on, and Zephyr's I2C
driver faulted on its first register access (measured 2026-09-20). The
blk-ctrl node's `early-ccm-roots` (I2C2 91, PWM3 105) are enabled and its
`early-ccm-gates` (I2C2 24, PWM3 42, GPIO4 14) opened at PRE_KERNEL_1,
before any driver.

### 2. LCDIFv3 (`imx8mp_lcdif.c`)

From `drivers/gpu/drm/mxsfb/lcdif_regs.h` and `lcdif_kms.c`
(`lcdif_reset_block`, `lcdif_set_formats`, `lcdif_set_mode`,
`lcdif_enable_controller`, `lcdif_crtc_atomic_flush`).

```
CTRL+SET(0x04)   = CTRL_SW_RESET(BIT31); read back; CTRL+CLR(0x08) = same
DISP_PARA(0x10)  = 0x00000000       line pattern RGB888
CTRLDESCL0_5     = 0x09000000       BPP_32_ARGB8888 (i.e. XRGB8888)
CSC0_CTRL(0x21c) = 0x00000001       bypass
CTRL(0x00)       = 0x00000000       HS, VS, DE all active HIGH (see below)
DISP_SIZE(0x14)  = 0x03200500       (vactive<<16) | hactive  = 800, 1280
HSYN_PARA(0x18)  = 0x00400024       (hback<<16)   | hfront   = 64, 36
VSYN_PARA(0x1c)  = 0x00180008       (vback<<16)   | vfront   = 24, 8
VSYN_HSYN_WIDTH  = 0x00080018       (vsync<<16)   | hsync    = 8, 24
CTRLDESCL0_1     = 0x03200500       (height<<16)  | width
CTRLDESCL0_3     = 0x00221400       P_SIZE 2 | T_SIZE 2 | pitch 5120
CTRLDESCL_LOW0_4 = fb & 0xffffffff
CTRLDESCL_HIGH0_4= (fb >> 32) & 0xf
PANIC0_THRES     = 0x00aa0154       low 170, high 340 (1/3 and 2/3 of 511)
INT_ENABLE_D1    = 0x00000001       panic -> NoC priority boost, no interrupt
DISP_PARA       |= BIT(31)          DISP_ON
CTRLDESCL0_5    |= BIT(31)          layer enable
```

`P_SIZE` and `T_SIZE` are undocumented AXI burst-size fields; 2 is the
256-byte burst mainline copies from the downstream kernel.

A page flip is the two address halves plus `CTRLDESCL0_5 |= BIT(30)`
(`SHADOW_LOAD_EN`), which the hardware clears at the next vblank; the code
polls it for 36 ms, about two frames, the same budget
`lcdif_disable_controller()` uses.

**Sync polarity.** `LCDC_V8_CTRL` is left at 0, i.e. HS/VS/DE active high,
even though the panel's timing says `hsync-active = <0>`. That is deliberate:
`samsung_dsim_atomic_check()` forces `PHSYNC | PVSYNC` for
`DSIM_TYPE_IMX8MP`, because the i.MX8MP glue between LCDIFv3 and DSIM does
not invert (unlike the 8M Mini's, which does). The panel's own polarity is
applied at the far end, in the bridge's `LVDS_FMT` register — and because
Linux rewrites the mode before the bridge sees it, mainline ends up writing
positive polarity there too. This panel is DE-driven, so HS/VS polarity
should not matter; see "Open questions".

### 3. MIPI DSIM and D-PHY (`imx8mp_dsim.c`)

From `drivers/gpu/drm/bridge/samsung-dsim.c`. `DSIM_TYPE_IMX8MP` uses
`exynos5433_reg_ofs[]`: STATUS 0x04, SWRST 0x0c, CLKCTRL 0x10, TIMEOUT 0x14,
CONFIG 0x18, ESCMODE 0x1c, MDRESOL 0x20, MVPORCH 0x24, MHPORCH 0x28,
MSYNC 0x2c, INTSRC 0x34, FIFOCTRL 0x4c, PLLCTRL 0x94, PLLTMR 0xa0,
PHYCTRL 0xa4, PHYTIMING 0xb4/0xb8/0xbc — and `pll_p_offset` **14**, not the
Exynos 13.

`samsung_dsim_init()` for a non-Exynos part, in order:

1. `SWRST = BIT(0)` (`RESET_TYPE` is `DSIM_SWRST`, not `FUNCRST`), clear INTSRC.
2. `PLLTMR = 500`, `PLLCTRL = PLL_EN | P(3)<<14 | M(214)<<4 | S(2)<<1`, poll
   `STATUS.PLL_STABLE` (bit 31).
3. `CLKCTRL`: `ESC_CLKEN | BYTE_CLKEN | ESC_PRESCALER(3) |
   LANE_ESC_CLK_EN_CLK | LANE_ESC_CLK_EN_DATA(0xf) | BYTE_CLK_SRC(0) |
   TX_REQUEST_HSCLK`. That last bit is what puts the clock lane into HS,
   which the bridge needs before its CSRs are written.
4. `PHYCTRL = 0` (`imx8mm_dsim_reg_values[]`: ULPS_EXIT 0, VREG_LP 0,
   SLEW_UP 0) and the three PHYTIMING registers, computed rather than
   tabulated, from `phy_mipi_dphy_get_default_config_for_hsclk()` converted
   with `PS_TO_CYCLE(ps, byte_clock)`. At 428 MHz (ui = 2337 ps, byte clock
   53.5 MHz) that gives:

   | field | ps | cycles |
   | --- | --- | --- |
   | LPX | 50000 | 3 |
   | HS_EXIT | 100000 | 5 |
   | CLK_PREPARE | 38000 | 2 |
   | CLK_ZERO | 262000 | 14 |
   | CLK_POST | 60000 + 52*ui = 181524 | 10 |
   | CLK_TRAIL | 60000 | 3 |
   | HS_PREPARE | 40000 + 4*ui = 49348 | 3 |
   | HS_ZERO | 105000 + 6*ui = 119022 | 6 |
   | HS_TRAIL | max(4*8*ui, 60000+4*4*ui) = 97392 | 5 |

   The LPX and HS_TRAIL rows are worth a second look if these are ever
   compared with another port: `phy_mipi_dphy_calc_config()` uses 50 ns for
   LPX (not the 60 ns some datasheets quote as the minimum) and takes the
   reverse-direction n = 4 branch of the spec's
   `T_HS-TRAIL = max(n*8*ui, 60 + n*4*ui)`, which is the conservative one.

   That is `PHYTIMING = 0x0305`, `PHYTIMING1 = 0x020e0a03` and
   `PHYTIMING2 = 0x00030605`.
5. `init_link`: FIFO pointers down then up (10 ms each), `CONFIG =
   VIDEO_MODE | MFLUSH_VS | BURST_MODE | MAIN_PIX_FORMAT_RGB888`, then
   `|= NUM_OF_DATA_LANE(3) | LANE_EN_CLK | LANE_EN(0xf)`; poll `STATUS` for
   all four data lanes in stop state and the clock lane stopped or HS-ready;
   `ESCMODE = STOP_STATE_CNT(0xf) | FORCE_STOP_STATE`;
   `TIMEOUT = BTA(0xff) | LPDR(0xffff)`.
6. Video timings, porches converted to byte clocks less six bytes of packet
   overhead each: hfp 36 px -> **20**, hbp 64 px -> **41**, hsa 24 px -> **11**.
   `MVPORCH = CMD_ALLOW(0xf) | STABLE_VFP(8) | MAIN_VBP(24)`,
   `MHPORCH = HFP(20)<<16 | HBP(41)`,
   `MSYNC = VSA(8)<<22 | HSA(11)`,
   `MDRESOL = VRESOL(800)<<16 | HRESOL(1280)` (12 resolution bits on this
   part), then `MDRESOL |= MAIN_STAND_BY`.

`FORCE_STOP_STATE` stays asserted through all of that, so the clock lane is
in HS but no video is going out. The bridge is programmed in that window.
`dsim_video_start()` then clears it — that is
`samsung_dsim_atomic_enable() -> samsung_dsim_set_stop_state(false)`.

### 4. SN65DSI8x (a device on I2C: `yakogl,sn65dsi8x`)

The bridge is a device of its own: `driver/drivers/display/bridge_sn65dsi8x.c`
(registration, GPIO configuration, a mutex) over
`driver/rust/src/ip/sn65dsi8x.rs`, which holds the register map, the clock
arithmetic and the sequence, all written from the TI data sheets: SN65DSI84
**SLLSEC2H** and SN65DSI83 **SLLSEC1I** (both revised October 2020). Section
and table numbers below are SLLSEC2H's; the module's header maps them to
SLLSEC1I's. The same driver serves the AM62P (an SN65DSI83 on the 7" panel
board, a node that starts itself after its DSI host) and this board (the
Toradex DSI-to-LVDS adapter's SN65DSI84, a node this display drives). The
nine ID bytes are identical on the two parts (Table 7-5), so the driver does
not tell them apart; it keeps every reserved, undefined and channel-B bit at
its reset value by read-modify-write (7.6.1), which is what makes one driver
right for both.

`imx8mp_display.c` claims the EN pin at init at the level it finds the chain
at (`sn65dsi8x_claim_enable`), never writes the bridge on the adopt path (the
health word only reads it), and on the cold path, with the DSIM clock lane in
HS and no video yet:

```
sn65dsi8x_start(mode, lane rate = the DSIM PLL's 426.6 Mbit/s)
  EN low 10 ms, EN high, 10 ms           Table 7-2 Init seq 3, 4; 7.4.2
  read 0x00..0x08 = "58ISD   " 0x01      Table 7-5 (retried up to 1 s: the
                                         adapter's own power-up, a board fact)
  0x0d PLL_EN = 0                        Table 7-6 0x0D: PLL off while 0x0A/0x0B change
  CSRs (Init seq 5), read back:
  0x0a LVDS_CLK_RANGE 2, HS_CLK_SRC 1          -> 0x05 (| PLL_EN_STAT later)
  0x0b DSI_CLK_DIVIDER 3 (field 2)             -> 0x10
  0x10 CHA_DSI_LANES 00 (four)                 -> 0x26 (bits 7:5, 2:1 kept)
  0x11 CHA_DSI_DATA_EQ / CLK_EQ none           -> 0x00
  0x12 CHA_DSI_CLK_RANGE 42 (210..215 MHz)     -> 0x2a
  0x18 24 bpp, format 2, HS/VS/DE positive     -> 0x18 (bit 4 single link kept)
  0x19 CHA_LVDS_VOCM 1.2 V, VOD swing 01       -> 0x05 (the reset value)
  0x1a CHA_LVDS_TERM 200 ohm                   -> 0x03 (the reset value)
  0x1b CHA_LVDS_CM_ADJUST none                 -> 0x00
  0x20/21 1280, 0x24/25 800, 0x28/29 33, 0x2c/2d 24, 0x30/31 8
  0x34 64, 0x36 24, 0x38 36, 0x3a 8, 0x3c CHA_TEST_PATTERN 0
  0x0d PLL_EN = 1, poll PLL_EN_STAT (0x0a.7), 10 ms   Init seq 6
  0x09 SOFT_RESET = 1, 10 ms                          Init seq 7
(LCDIF on, DSIM video)                                Init seq 8
sn65dsi8x_check_link: 5 ms, 0xe5 <- 0xff, 1 ms, read 0xe5   Init seq 8-11
```

The derivations, each cited in the module:

* DSI clock = lane bit rate / 2: the UI is half an HS clock period (6.3, note
  1). 426.6 / 2 = 213.3 MHz.
* `CHA_DSI_CLK_RANGE` = floor(DSI clock / 5 MHz) (Table 7-7, 0x12: "0x08 - 40
  <= frequency < 45 MHz"): 42. The C driver this replaced rounded up and
  wrote 43; Linux on this board writes 42 (golden dump), which is what the
  table gives.
* `DSI_CLK_DIVIDER`: LVDS clock = DSI clock / N (Table 7-6); N =
  floor(DSI clock / pixel clock) = floor(213.3 / 69.3) = 3, so the LVDS clock
  is 71.1 MHz, not below the pixel clock. How to choose N is our reading;
  the data sheet states only the divider ("citation pending" in the module).
* `LVDS_CLK_RANGE` from that LVDS clock (7.3.1; Table 7-6): 62.5..87.5 MHz, 2.
* `CHA_SYNC_DELAY` = 33: at least 32 (Table 7-9); the one above it is the
  value Linux runs here and both boards have run with, not a data-sheet fact.
* `LVDS_VCOM` 0x05, risk R6 in docs/source-form.md: the reset value of 0x19
  (VOCM 1.2 V, VOD swing 01 on both channels), now written as such.

The host tests (`cargo test -p yakogl-drivers`) run this against a model of
the part on I2C (reset image from the data sheet's DEFAULT columns; NACK while
EN is low; PLL_EN_STAT following PLL_EN; RW1C error status) and compare the
register image it leaves for this board's mode with the golden dump below
(`golden-linux-2026-09-21.txt`, "SN65DSI83 i2c-1 0x2c"): every documented
register agrees; the two allowed differences are 0xF4 and 0xF7, outside the
data sheet's CSR map (the part's own; never read or written).

### 5. Backlight (a PWM device: `yakogl,imx-pwm`)

PWM3 at 0x30680000 is a Zephyr PWM device of its own
(`driver/drivers/pwm/pwm_yakogl_imx.c`, a registration shim; the register
logic is Rust, `driver/rust/src/ip/imx_pwm.rs`, written from hal_nxp's
`PERI_PWM.h` field descriptions with the facts the header does not state
listed there with their IMX8MPRM Rev. 3 citations). This driver calls it with
`pwm_set_pulse_dt()` on the node's `pwms = <&backlight_pwm 0 6667179 ...>`
and reads `yakogl_imx_pwm_is_enabled()` for the BACKLIGHT_PWM health bit.

The PWM driver owns its clock: on the first enabling request it runs CCM root
105 (`kCLOCK_RootPwm3`, 0xb480) from the 24 MHz oscillator undivided and opens
CCGR42 (0x42a0) for domain 0 -- what the board's Linux was measured running
(the table above). With the 6 667 179 ns period (150 Hz) `pwm_set_dt` asks for
160012 cycles; the smallest prescaler that fits the 16-bit counter is 3, so
53337 counts: `PWMPR = 53335` and `PWMSAR = 53337` at 100 %,
`PWMCR = 0x01c20021` (EN, prescaler /3, ipg_clk_highfreq, active in debug,
wait and doze).

Why not Zephyr's own PWM driver: docs/source-form.md, group E.

## Where things live

| File | Block |
| --- | --- |
| `imx8mp_display.c` | the device, the DT plumbing, the bring-up order, adopt vs. cold |
| `driver/drivers/power_domain/power_domain_yakogl_imx8mp_media.c` + `driver/rust` | GPC domains, CCM roots and gate, VIDEO_PLL1, MEDIA_BLK_CTRL, the early clocks (power-domain devices) |
| `imx8mp_lcdif.c` | LCDIFv3 |
| `imx8mp_dsim.c` | DSIM host and D-PHY, including the PLL search |
| `imx8mp_lcdif_rust.c`, `imx8mp_dsim_rust.c` + `driver/drivers/{display,mipi_dsi}` + `driver/rust` | the same two blocks from IMX8MPRM Rev. 3 in Rust, Zephyr devices `yakogl,imx8mp-lcdif` / `yakogl,samsung-dsim`, behind `YAKOGL_DRV_DISPLAY_IMX8MP_LCDIF_RUST` / `YAKOGL_DRV_DSIM_RUST` (default n; driver/docs/imx8mp-display-cold.md) |
| `driver/drivers/display/bridge_sn65dsi8x.c` + `driver/rust` | the SN65DSI8x bridge, over I2C (a device on its bus) |
| `driver/drivers/pwm/pwm_yakogl_imx.c` + `driver/rust` | PWM3, the backlight (a PWM device) |
| `imx8mp_display.h` | the API: `imx8mp_display_on/_set_fb/_off/_info` |

The device tree side is `dts/bindings/display/yakogl,imx8mp-display.yaml`
plus `power-domain/yakogl,imx8mp-media-{blk-ctrl,pd}.yaml` (the media
domains and the early clocks) and `yakogl,sn65dsi8x.yaml` (the bridge: EN pin, data lanes, LVDS format,
termination and polarities), and the node itself is
`samples/zephyr/common/verdin_imx8mp_display.dtsi`, included by each
sample's `boards/verdin_imx8mp_mimx8ml8_a53.overlay`.

**That dtsi belongs in the board**, in `driver/boards/toradex/verdin_imx8mp/`:
the pinctrl groups (`i2c2_display`, `pwm3_backlight`), the
`&gpio4` and `&i2c2` enables, the `lvds_bridge` node and the `display0` node
are all board facts, not sample facts. It has run on hardware; the move is
still to do.

## The panel console

`driver/zephyr/lib/board/imx8mp/gc_panel_console.c` has two stages.

* **PRE_KERNEL_1**: if the LCDIF is already scanning, read its geometry, put
  our own framebuffer in front of it and install the printk hook. Nothing
  here needs the kernel, so the console exists as early as it possibly can.
  This is the crash route. Before its first LCDIF read it asks whether the
  LCDIF can answer at all (`soc::imx8mp::media::lcdif_unreachable`, which
  reads only the GPC and the CCM). On a cold U-Boot entry MEDIAMIX is off,
  a read would stall the core, and the stage returns.
* **POST_KERNEL, priority 85**: if stage one found nothing, call
  `imx8mp_display_on()` on our own framebuffer and install the hook then.

Stage two cannot run earlier. The bridge is on I2C behind a GPIO enable line,
the I2C and GPIO drivers initialise at POST_KERNEL 50 and 40, and the bridge
sequence needs millisecond sleeps. So on the normal kexec route **everything
printed before POST_KERNEL 85 is lost**. If that ever matters, the fix is a
ring buffer replayed into the framebuffer when the console arrives, not an
earlier bring-up.

## Open questions only the hardware can answer

1. **Does the panel light up at 71.43 MHz?** Answered: the picture broke up
   (2026-09-23): a lane rate of exactly pixel x 24 / 4 leaves no room for the
   packet overhead. **Sequel (2026-10-06, issue #23):** the 71.1 MHz plan of
   2026-10-02 was the same condition on the 426.6 Mbit/s link (71.1 x 24 / 4
   = 426.6) and the panel showed 1.5 copies of the frame across each line,
   while the LCDIF, DSIM and bridge registers matched Linux's register for
   register (Linux read back that day: XR24 1280x800 pitch 5120, CTRLDESCL0_5
   0x89000000, CONFIG 0x0600707f, bridge 0x18 = 0x18, 0x20/0x21 = 1280, 0xE5
   = 0) -- the one difference was the pixel clock, 71.1 against Linux's 69.3
   MHz. The device tree is back at 69.3 MHz and the margin is a checked limit
   ("Pixel clock", limit 4). The board layer's
   `CONFIG_YAKOGL_BOARD_IMX8MP_DISPLAY_RECORD` writes a DISPLAY record (pixel clock, lane rate, the bridge's IRQ_STAT a
   second into the picture) so the next viewing has numbers beside it.
2. **Is `sys_pll2_1000m` really running when Zephyr arrives?** Everything
   assumes the boot loader left SYS_PLL2 on. A dark panel with the GPC and
   BLK_CTRL logs looking healthy would point here first; read CCM
   `media_disp1_pix` back and check.
3. **DE polarity at the bridge** -- settled as a board fact (2026-10-01).
   `DE_NEG_POLARITY` (0x18 bit 7, SLLSEC2H Table 7-8: "1 - DE is negative
   polarity driven '0' during active pixel transmission on LVDS") is the
   bridge node's `de-active-low` property, no longer derived from the panel's
   `de-active`. This board's node leaves it out: DE positive, which is what
   Linux runs here (golden 0x18 = 0x18) and what the panel's `de-active = <1>`
   means. A panel that wants DE active low says so on the bridge node.
4. **HS/VS polarity through the bridge.** Linux tells the SN65DSI8x that HS
   and VS are active high (samsung-dsim rewrites the mode before the bridge
   driver reads it), while the panel's own timing says active low. The
   bridge node now states the LVDS-side polarity itself
   (`hsync-active-low` / `vsync-active-low`, absent here, so positive, as
   Linux runs it); the panel is DE-driven (`de-active = <1>`) so this should
   not matter -- but if the picture is shifted or torn, adding the two
   properties (0x18 bits 6 and 5) is the first experiment.
5. **The pad electrical settings.** Linux writes 0x400001c6 to both I2C2
   pads and 0x06 to the PWM3 pad. The overlay approximates those with
   Zephyr's pinctrl properties (`drive-strength = "x6"`, pull-up, open
   drain, Schmitt, `input-enable` for the SION bit; `"x4"` for the PWM).
   The exact DSE/FSEL mapping was not verified field by field. I2C failing
   to ACK at 0x2c, or ACKing only at a lower bus speed, points here.
6. **Does the GPC accept these writes from Zephyr at EL1 NS?** Answered:
   yes. On 2026-10-03 an image started by U-Boot, with MEDIAMIX and the GPU
   domains off from reset, powered both up itself (`soc::imx8mp::media`,
   `gpu::vivante::power`) and left the same PICTURE record as the kexec
   route (`docs/notes/imx8mp-netboot.md`, "step C root
   cause and fix").
7. **The ADB400 handshake.** Like Linux, the code delays instead of waiting
   for `PU_PWRHSK` bit 30. If the MEDIA_BLK_CTRL registers read back as
   0xffffffff or the LCDIF ignores writes, the handshake did not complete
   and the delay needs to become a poll (which is safe once the bus clock is
   on).
8. **Burst mode versus sync-event mode on the DSI link.** `DSIM_BURST_MODE`
   is set because that is what the AM62P panel driver asks its DSI host for
   with the same class of bridge. If the bridge reports sync errors in
   `IRQ_STAT`, try clearing `DSIM_BURST_MODE` and setting `DSIM_SYNC_INFORM`.
9. **Whether `device_shutdown()` also powers the MEDIAMIX domain down.** If
   it does, the first LCDIF register read after the jump returns garbage
   rather than zeroes, and `lcdif_is_running()` could in principle read a
   stale "on". It reads two independent bits in two registers, which makes
   that unlikely, but a bus fault on the read would be conclusive.
