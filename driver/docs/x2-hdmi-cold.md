<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Armadillo-X2: HDMI from cold

Status (2026-10-01): designed, written and host-tested; not yet run on the
board. The board's display today is LCDIF3 adopted from the Linux the image
is kexec'd out of (`yakogl,imx8mp-lcdif-adopt`). The goal is a picture when
Zephyr boots straight from power-on with no Linux before it, so Zephyr has to
power, clock and program the whole chain itself.

This is a group E addition under docs/source-form.md: original code in Rust in
the `yakogl-drivers` crate, written from the i.MX 8M Plus Applications
Processor Reference Manual, IMX8MPRM Rev. 3, 08/2024 ("RM", cited by section,
no text or figures copied), with register names and bits from NXP's BSD-3
hal_nxp headers (generated into `soc/imx8mp/regs.rs`). The Linux drivers of
this path (dw-hdmi, imx8mp-hdmi-tx/-pvi, the Samsung HDMI PHY driver,
imx8mp-blk-ctrl) are GPL and were not read. A register dump of the running
Linux is test data only (board plan (a)).

## The chain

```
             HDMIMIX (GPC PU14, PGC 0xE80, ADB400 on the NoC)            HDMI PHY domain (PU15, PGC 0xEC0)
 DDR --AXI--> LCDIF3 --pixel link--> HTX_PVI --> DW HDMI TX controller --TXD0..2--> HDMI TX PHY --> CON8
             0x32FC6000              0x32FC4000   0x32FD8000                         0x32FDFF00
             13.3                    13.13        13.9                               13.10
 clocks:     apb  HDMI_APB_CLK       apb HDMI_APB  iapbclk/hpi HDMI_APB              apb (TX_PHY_APB_CLK_EN)
             b    HDMI_B_CLK         pix htx_p_clk ipixelclk htx_p_clk                int_clk = reference:
             pix  htx_p_clk                        sfr, prep, tmds (from the PHY)     24 MHz or VPLL_PHY
             (LCDIF_*_CLK_EN)        (TX_VID_LINK_  (TX_*_CLK_EN, RTX_CLK_CTL1)       (HTXPHY_CLK_SEL)
                                      PIX_CLK_EN)                                    PLL: 5x TMDS, PIXEL_CLKO
 resets:     LCDIF_APB_RESET_N,      VID_LINK_SLV_  TX_RSTZ, TX_APBRSTZ              TX_PHY_PRESETN,
             LCDIF_ASYNC_RESET_N     RESETN                                          TX_CONTROL0[TX_PHY_PDOWN]
             all in HDMI TX BLK_CTRL RTX_RESET_CTL0 (13.11.3.1.4), active low, asserted out of reset
 CCM roots:  HDMI_APB (slice 22), HDMI_AXI (23), HDMI_REF_266M (120); gate CCGR95 for every module above (Table 5-2)
```

The pixel clock: HTX_PIPE_CLK_SEL "select[s] between Frac PLL clock output and
the PIXEL_CLK_OUT output from PHY" (13.11.3.1.6.4), the PHY has an "integrated
pixel clock generator" (13.10.1.2), the PVI and the controller run on
`htx_p_clk` (13.13.1.1). Read together: the PHY's PLL makes the pixel clock of
the whole chain, LCDIF3's included. The RM never draws that last arrow (13.3.2.2
says only "pix_clk Pixel Clock"); it is the first thing the board run checks
(stage 7 times out if it is wrong).

Not needed and not touched: EDID/E-DDC (the mode is fixed), HDCP ("HDCP is not
supported", 13.9.1.2), audio (PAI, GPA, ACR), CEC, hot-plug interrupts (the
sink is assumed connected; HPD is read and only logged), the colour space
converter (bypassed), InfoFrames (DVI mode).

## What the RM documents, and what it does not

| Need | Where | Documented? |
|---|---|---|
| LCDIF3 registers, order, shadow load, test modes | 13.3 (Fig. 13-7 software flow; 13.3.3.1.x) | yes, except the AXI burst bits drivers outside the RM put in CTRLDESCL0_3 31:16 ("Reserved") |
| HDMIMIX power: PU14/PU15, PGCs, ADB400 | 5.2.10 (memory map list, 5.2.10.37, .45, .57, .72) | yes; the ADB acknowledge on power-up only by inference (as for MEDIAMIX) |
| CCM roots and gate | Table 5-1, Table 5-2 | yes |
| Block control clocks, resets, PHY power-down, status | 13.11.3.1 | registers yes; the polarity of HTX_PIPE_CLK_SEL / HTXPHY_CLK_SEL no; LCDIF_NOC_HURRY's scale no; no sequence |
| PVI | 13.13 | registers yes; the timing generator's coordinate origin no (not used) |
| DW HDMI TX controller registers | 13.9.3 (all sub-blocks; hal_nxp has the same as PERI_IDENTIFICATION/INTERRUPT/VIDEOSAMPLER/VIDEOPACKETIZER/FRAMECOMPOSER/PHYCONFIGURATION/MAINCONTROLLER/...) | **yes**, the full register map with field descriptions |
| DW HDMI TX programming sequence | 13.9 | **no**: no init flow. Built here from the register semantics (below) |
| `phy_conf0`, `mc_phyrstz` on this SoC | 13.9.3.6.2, 13.9.3.9.5 | **no**: "refer to the DesignWare Cores HDMI TX PHY Databook", but this SoC's PHY is not a DesignWare one. Board dump; reset values until then |
| Frame composer preamble filters | 13.9.3.5.22-24 | meaning yes, values no (not written) |
| HDMI PHY registers | 13.10.3.1 (PHY_REG1..47, fields, reset values; no hal_nxp header) | yes |
| PHY PLL formula | 13.10.2.1.2 | integer mode yes; fractional mode incompletely (PI and SDC terms undefined) |
| PHY programming sequence, lock time | 13.10 | **no** |
| PHY analog settings per rate (charge pump, LPF, AFC, amplitude, pre-emphasis) | 13.10.3.1 | fields yes, values per rate no: kept at reset |

So the RM does document the HDMI TX controller registers (13.9.3, 280 pages);
what it lacks is the order of operations for the controller and the PHY, and
what the DesignWare PHY-interface bits mean on this SoC. No other source is
needed for the registers; the missing order is derived below, and the few
values the RM does not state are measured from the board dump and allow-listed.

## The sequence

`soc::imx8mp::hdmi_chain::bring_up`, stage by stage (the stage numbers are the
`stop-after` values):

1. **start** -- check the mode; plan the PHY PLL (below); if LCDIF3 is
   scanning, stop its layer at a frame boundary (13.3.3.1.16.4 shadow load);
   with `power-cycle`, power the chain down first (step 2 reversed).
2. **power** --
   CCM: HDMI_APB = SYSTEM_PLL1 (800 MHz) / 4 = 200 MHz (mux 010, Table 5-1
   max 200 MHz); HDMI_AXI = SYSTEM_PLL2_DIV2 = 500 MHz (mux 111, max 500);
   HDMI_REF_266M = SYSTEM_PLL1_DIV3 = 266.67 MHz (mux 100; the source
   rates are 5.1.5.1 "Input Clocks"); through the
   Target Interface (5.1.6.4), then CCGR95 open (5.1.7.6). GPC: HDMIMIX
   power-up (map bit 16, trigger bit 14, wait for self-clear, status, PCR and
   PSR cleared, 5.2.9.4 / 5.2.8 rule 2 as `soc::imx8mp::gpc`), its ADB400
   request raised, the block control's bus clocks on (GLOBAL_APB/B_CLK_EN,
   NOC_HDMI_CLK_EN, 13.11.3.1.5), the ADB400 acknowledge waited for
   (5.2.10.57), then the HDMI PHY domain (bit 15/17).
3. **block control** -- RTX_CLK_CTL0: global APB, bus, REF266M, XTAL24M,
   TX pixel clocks, IRQ steer, HDMI NoC, LCDIF apb/b/pdi/pix (13.11.3.1.5);
   RTX_CLK_CTL1: TX hpi/apb/pixel/sfr/prep, PHY apb/int, PVI pixel
   (13.11.3.1.6); the two clock selects only when the devicetree states them;
   a reset pulse (assert, 10 us, release) on NoC, LCDIF async/APB, TX core/APB,
   PHY APB, IRQ steer, PVI (13.11.3.1.4); TX_CONTROL0[TX_PHY_PDOWN] cleared;
   LCDIF_NOC_HURRY (13.11.3.1.13). RTX_CLK_CTL2..4 stay 00 = "Default clock
   gating logic" (13.11.3.1.7.2).
4. **PHY** -- `PHY_REG33[MODE_SET_DONE]` = 0 ("Assert forced global reset"),
   P/M/S, integer mode (`SDM_K_EN` = 0, `PLL_PI_EN` = 0), the frequency-lock
   detector divider and target code, 8-bit pixel clock (`PCLK_SEL` = 0),
   `MODE_SET_DONE` = 1, then wait for `PHY_REG34` PLL_LOCK and PHY_READY
   (13.10.3.1.2-35). Bounded at 20 ms (ours).
5. **controller** -- byte accesses (every register "Width 8" at its own
   address): `mc_clkdis` turns off the CEC, CSC and audio domains;
   `tx_invid0` = RGB 4:4:4 8 bit; `vp_pr_cd` 24 bpp, no repetition;
   `vp_conf` its reset value; the frame composer's input timing,
   `fc_invidconf` with the mode's polarities and `DVI_modez` = 0 (DVI);
   `fc_ctrldur` 12 and `fc_exctrldur` 32 (the stated minimums),
   `fc_exctrlspac` the largest value within 50 ms at the TMDS rate;
   `fc_prconf` reset; `mc_flowctrl` = 0 (CSC bypassed); the TMDS reset request
   in `mc_swrstzreq_1` followed by a write to `fc_invidconf`, as 13.9.3.9.3
   requires; `phy_conf0` / `mc_phyrstz` only when given (13.9.3.x as cited in
   ip/dw_hdmi_tx.rs).
6. **PVI** -- HTX_PVI_CTRL: LCDIF path (mode 2), timing generator off, inputs
   active high, outputs with the mode's polarities, the rest at reset
   (background insertion on underflow stays on); FIFO flags cleared; enabled
   (13.13.3.1.2).
7. **LCDIF3** -- `SW_RESET` pulsed and checked clear (it is set out of reset,
   13.3.3.1.1); HS/VS/DE active high, timings, layer (pitch 64-byte aligned,
   address 8-byte aligned, ARGB8888), CSC bypassed; panic thresholds;
   `DISP_ON`, then the layer `EN` with `SHADOW_LOAD_EN`, which must clear
   within two frames -- the proof that the pixel clock runs (13.3.2.6,
   13.3.3.1.16.4).
8. **video** -- clear `mc_lockonclock_1`, wait half the frame bound, require
   PHY lock (TX_STATUS0), the controller's pixel and TMDS clock-present flags,
   the PVI on and no PVI FIFO error; HPD absent is a warning only.

Why this order: the PHY PLL is the pixel clock of steps 5-7, so it locks
first; the controller is configured before video reaches it; LCDIF3 starts
last because its first shadow load needs the running pixel clock.

**Staging pictures** (`test-picture`): `hdmi-forced` turns on
`fc_dbgforce.forcevideo` after stage 5 -- the controller sends fixed video
(black: the FC_DBGTMDS colour registers are not in the RM) with no input, so a
monitor that shows "signal present" proves power, clocks, PHY and controller
without the PVI or LCDIF3; `lcdif-background` puts LCDIF3 in "Test Mode1
(BGND Color Display)" (13.3.3.1.3.3) after stage 7: a solid `test-rgb` with no
memory fetch.

### The mode and the PLL

The monitor runs 1920x1200 at 60 Hz with reduced blanking (VESA CVT 1.2:
154 MHz, H 1920/48/32/80 +, V 1200/3/6/26 -), the devicetree's mode. The PHY
PLL makes the 5x TMDS clock, `FREF x M / (P x S)` with FREF 24 MHz, the VCO
(FREF x M / P) within 0.75-3 GHz (13.10.2.1.2). 770 MHz has no exact integer
solution (M would exceed 255); the nearest is **P 7, M 225, S 1: 771.43 MHz,
TMDS 154.286 MHz, +1855 ppm, 60.06 Hz**, within HDMI's 0.5 % (`max-ppm`).
FLD: TMDS / 4 = 38.6 MHz (< 50 MHz), target code 6144 / 38.57 = 159 (the
PHY_REG13 formula, which reproduces its own 148.352 MHz -> 166 example). The
fractional mode would hit 154 MHz exactly, but its formula leaves PI and the
SDC terms undefined; if the dump shows Linux using it, the registers will say
how (open question 1).

## Code

| File | What |
|---|---|
| `driver/rust/src/ip/lcdifv3.rs` | LCDIFv3, any instance: configure / enable / disable / set_fb / set_layout / read_scanout / test_mode; the same module for the Verdin's LCDIF1 (group E step 3) |
| `driver/rust/src/ip/imx8mp_hdmi_phy.rs` | the PHY: register map from 13.10.3.1, PLL planner, program + lock |
| `driver/rust/src/ip/dw_hdmi_tx.rs` | the controller: DVI-mode configuration, forced video, status |
| `driver/rust/src/soc/imx8mp/hdmi.rs` | HDMIMIX power (on `soc::imx8mp::gpc`, `ccm`), block control, PVI |
| `driver/rust/src/soc/imx8mp/hdmi_chain.rs` | the staged bring-up, health bits, shut-down |
| `driver/rust/src/ffi/hdmi.rs` | C entry points; adopt-or-cold policy; layout pins |
| `driver/rust/src/video.rs` | the shared `VideoMode` |
| `driver/rust/src/model_hdmi.rs` | the test model of the seven blocks (8-bit controller) |
| `driver/rust/src/soc/imx8mp/hdmi_golden.rs` | golden test against the Linux dump |
| `driver/rust/src/mmio.rs` | `read8`/`write8` (ldrb/strb + dmb, as sys_read8/sys_write8) |
| `tools/hal-nxp/gen_regs.py` | 8-bit blocks (`REG_BITS`), RW/SET/CLR/TOG register groups, LCDIF, HDMI_TX_BLK_CTL, HTX_PVI and the controller's blocks added (pinned) |
| `driver/rust/include/yakogl_drivers.h` | `yd_imx8mp_hdmi_*`, pinned |
| `driver/drivers/display/display_yakogl_imx8mp_hdmi.c` | the Zephyr device (display API with CONFIG_DISPLAY: write = flip by address, blanking on/off, capabilities ARGB8888) and the `imx8mp_display_*` facade the samples call |
| `driver/dts/bindings/display/yakogl,imx8mp-hdmi.yaml` | binding |
| `driver/boards/atmark/armadillo_x2/armadillo_x2_mimx8ml8_a53.dts` | `hdmi: display@32fd8000`, next to the adopt node |
| `driver/zephyr/Kconfig`, `CMakeLists.txt` | `YAKOGL_DRV_DISPLAY_IMX8MP_HDMI` (default n while the adopt node exists: the choice), its init priority; the crate's `imx8mp` feature |

**Devicetree / Kconfig.** One node, `yakogl,imx8mp-hdmi`, reg-names
`hdmi lcdif phy blk pvi gpc ccm`, the mode as flat panel-timing properties
(the names of `yakogl,imx8mp-display`), and the board-dump properties
(`htx-pipe-clk-sel`, `htxphy-clk-sel`, `dw-phy-conf0`, `dw-phy-rstz`,
`lcdif-pixclk-rising`, `lcdif-desc3-extra`, `lcdif-noc-hurry`) that are left
out until the dump states them. `force-cold`, `power-cycle`, `stop-after`,
`test-picture` are the bring-up controls. The X2 devicetree carries both
nodes; `CONFIG_YAKOGL_DRV_DISPLAY_IMX8MP_HDMI=y` picks the cold driver (the adopt
one is built otherwise), so the adopt route keeps working until the cold path
is proven, then the adopt node goes.

**Coordination with group E step 3.** `ip::lcdifv3` is the LCDIF module step 3
foresees: base address from the devicetree, no knowledge of what sits
downstream. The Verdin's `imx8mp_lcdif.c` moves onto it with its own polarity
(`Output`) and the measured burst bits (`Layer::desc3_extra`, risk R1); its
adopt path is `read_scanout` + `set_layout` + `set_fb`, which `ffi/hdmi.rs`
already uses.

## Tests

Host (`cargo test -p yakogl-drivers --all-features`), against
`model_hdmi.rs`, which refuses any access to a block whose domain, CCM
root/gate, block-control clock or reset does not allow it, and any 32-bit
access to the controller:

- lcdifv3: register layout = RM; configure writes the mode and leaves the
  display off; DISP_ON before EN; no pixel clock = timeout after two frames;
  bad pitch/alignment/address refused before any write; a stuck SW_RESET is an
  error; flip, re-layout, test mode, disable.
- imx8mp_hdmi_phy: the reset values describe the RM's default PLL and its FLD
  example; the X2 plan (7/225/1, +1855 ppm, FLD 2/159); exact plans for 27 and
  25.2 MHz, none for 148.5 MHz, refusals; the PLL writes sit inside the forced
  global reset; no lock = timeout; invalid settings refused unwritten.
- dw_hdmi_tx: layout; `fc_exctrlspac` within 50 ms; every timing byte for the
  X2 mode; the TMDS reset request followed by a frame-composer write; PHY
  interface untouched unless given; modes that overflow a field refused; IDs,
  forced video, status.
- hdmi: layout (roots 0x8B00/0x8B80/0xBC00, CCGR95 0x45F0, PGCs 0xE80/0xEC0,
  bits); roots before the gate before the domains; clocks before the reset
  pulse, assert before release; selects only when given; no ADB acknowledge
  stops before the PHY domain; power off and on again loses the block state;
  PVI value 0x0033_7005 for the X2 mode.
- hdmi_chain: a cold bring-up through all eight stages in block order;
  `stop-after` at every stage leaves the later blocks untouched; every
  injected failure names its stage; a running chain is stopped and
  power-cycled first; both staging pictures; shut-down.
- ffi: flag mapping; cold on, flip, off; stop-after reported with its stage;
  adopt touches LCDIF3 only and gives the previous layout back; a mode
  mismatch on adopt is refused.
- golden: the allow-list parses and names modelled blocks; with a dump present,
  every dumped register equals the model's image outside the allow-list.
- `gen_regs.py --self-test`: 8-bit blocks, lower-case field names, mixed
  widths refused.

## Board plan

### (a) The golden dump, from the running Linux

Linux on the X2 with HDMI-A-1 showing fbcon at 1920x1200. Read only the
documented registers of the display blocks, one block at a time, in this
order, and stop at the first read that does not return (0x007E0000 wedged this
board once; the GPC PGC range bus-faulted from Linux on the Verdin). Widths
matter: the controller's registers are bytes and must be read 8 bits wide.
With busybox `devmem` (`devmem ADDRESS WIDTH`):

```
# context (into the dump header)
uname -a; cat /proc/cmdline; date -u
cat /sys/class/drm/card*-HDMI-A-1/status /sys/class/drm/card*-HDMI-A-1/modes
grep -iE 'hdmi|lcdif|pix|video_pll|sys_pll' /sys/kernel/debug/clk/clk_summary   # if debugfs is mounted
cat /sys/kernel/debug/dri/*/state                                                 # the mode Linux runs

# 1. CCM roots and gate (32 bit): HDMI_APB, HDMI_AXI, HDMI_REF_266M, CCGR95
for a in 0x30388b00 0x30388b80 0x3038bc00 0x303845f0; do echo "$a 32 $(devmem $a 32)"; done
# 2. HDMI TX block control (32 bit), base registers only, not the aliases
for o in 0x00 0x10 0x20 0x40 0x50 0x60 0x70 0x80 0x90 0xa0 0xb0 0x200 0x220 0x230; do
  a=$(printf 0x%x $((0x32fc0000 + o))); echo "$a 32 $(devmem $a 32)"; done
# 3. PVI (32 bit), 0x00..0x40
for o in $(seq 0 4 64); do a=$(printf 0x%x $((0x32fc4000 + o))); echo "$a 32 $(devmem $a 32)"; done
# 4. LCDIF3 (32 bit), the documented registers
for o in 0x00 0x10 0x14 0x18 0x1c 0x20 0x24 0x28 0x30 0x34 0x200 0x208 0x20c 0x210 0x214 \
         0x21c 0x220 0x224 0x228 0x22c 0x230 0x234 0x238; do
  a=$(printf 0x%x $((0x32fc6000 + o))); echo "$a 32 $(devmem $a 32)"; done
# 5. HDMI PHY (32-bit reads of the 8-bit registers), PHY_REG1..47 (no PHY_REG0 in the RM)
for n in $(seq 1 47); do a=$(printf 0x%x $((0x32fdff00 + 4 * n))); echo "$a 32 $(devmem $a 32)"; done
# 6. HDMI TX controller, 8 bit: identification, ih_mute, video sampler,
#    packetizer, frame composer timing/control, PHY interface, main controller,
#    CSC config. No I2C master, E-DDC, CEC or HDCP registers.
for o in 0x0000 0x0001 0x0002 0x0003 0x01ff 0x0200 0x0201 0x0800 0x0801 0x0802 0x0803 0x0804 \
         $(seq 4096 4115) 0x10e0 0x1200 0x3000 0x3004 0x3006 0x3007 \
         0x4001 0x4002 0x4004 0x4005 0x4006 0x4100 0x4101; do
  a=$(printf 0x%x $((0x32fd8000 + o))); echo "$a 8 $(devmem $a 8)"; done
# 7. last, optional, one at a time: GPC PU_PGC_SW_PUP_REQ, PU_PWRHSK, A53 mapping
#    (not the PGC range 0xE80/0xEC0)
for a in 0x303a00d8 0x303a0190 0x303a01cc; do echo "$a 32 $(devmem $a 32)"; done
```

The output lines are already in the golden file's format
(`ADDRESS WIDTH VALUE`); `seq 4096 4115` is 0x1000..0x1013. They go into
`driver/docs/x2-hdmi-dump-linux.txt` under its header, the context lines as `#`
comments. Then `cargo test -p yakogl-drivers --all-features hdmi_golden`
compares; each difference becomes either a driver change with its RM section,
a devicetree property (the board-dump ones), or an allow-list line in
`x2-hdmi-allow.txt` with its reason.

The dump answers, in this order: the two clock selects and the LCDIF3 pixel
clock source (RTX_CLK_CTL1), whether Linux runs the PHY fractional (PHY_REG4
bit 7, REG1-7), the PHY analog values, `phy_conf0`/`mc_phyrstz`, the PVI mode
and whether its timing generator is on, `INV_PXCK` and the CTRLDESCL0_3 burst
bits, `LCDIF_NOC_HURRY`, the mode Linux runs (154 MHz CVT-RB or the EDID's
preferred one).

### (b) First cold run: kexec crash route, Linux's display off first

An image built with `-DCONFIG_YAKOGL_DRV_DISPLAY_IMX8MP_HDMI=y` (and the dump's
properties in the devicetree). Build evidence (2026-10-01): gc_gl_triangle for
`armadillo_x2/mimx8ml8/a53` links with it (`yd_imx8mp_hdmi_*` in the image, no
FP/SIMD instruction in libyakogl_drivers.a) and, without the option, still
links the adopt driver. gc_slint at 1920x1200 needs the scaled UI of the
x2/gc-render-hang merge (`CONFIG_GC_SLINT_UI_SCALED`); on a base without it,
its 1280x800 layout asserts at build time, independently of this driver. On the X2, before the jump, turn
Linux's display off so that any picture afterwards can only come from the cold
path: `echo 4 > /sys/class/graphics/fb0/blank` (FB_BLANK_POWERDOWN, DPMS off
through the DRM fbdev emulation), and check `devmem 0x32fc6214 32` shows
CTRLDESCL0_5 EN clear. Then the usual crash-route jump. The node has
`force-cold` and `power-cycle`, so even a chain left running is torn down and
HDMIMIX powered from off.

Bisect with `stop-after` 1..7 (devicetree), as on the Verdin with
`YAKOGL_DRV_DISPLAY_IMX8MP_STOP_AFTER`: each run says on the UART
`hdmi: stopped after stage N` or `failed in stage N: errno`, and
`imx8mp_display_health()` (gc_slint logs it after `imx8mp_display_on`) gives
the bits: 0 LCDIF running, 8 HDMIMIX up, 9 block control enabled, 10 PHY PLL
lock, 11 PHY ready, 12 HPD, 13 TMDS clock present, 14 pixel clock present, 15
PVI on, 16 PVI FIFO error, 17 RX sense. Then `test-picture = "hdmi-forced"`
with `stop-after = <5>` (monitor shows a black picture with signal, not "no
signal"), then `"lcdif-background"` (a green screen), then the real
framebuffer. If stage 7 times out, the pixel clock is not reaching LCDIF3:
try the other `htx-pipe-clk-sel`.

### (c) Direct boot, later

The same image booted from U-Boot or as the first stage after TF-A, no Linux:
the CCM's SYSTEM_PLL1/2 are as the boot firmware leaves them (this code reads
nothing else from Linux). Needs the direct-boot image work (x2-directboot);
the display part is the same code.

## Risks

- R-H1: LCDIF3's pixel clock source and the HTX_PIPE_CLK_SEL polarity are
  inferred, not stated (stage 7 tells; the dump answers).
- R-H2: the PHY's analog defaults are the RM reset values, tuned for the
  148.35 MHz default; 154.3 MHz is close, but Linux may change charge pump,
  LPF or driver levels per rate (the dump shows).
- R-H3: integer PLL at +1855 ppm: inside HDMI's 0.5 %, but a monitor that
  only accepts the EDID timing exactly could refuse (60.06 Hz).
- R-H4: `phy_conf0` / `mc_phyrstz` meaning on this SoC is undocumented; reset
  values may hold the PHY interface in power-down (TXPWRON 0, PDDQ 0 at reset
  06h) -- the dump decides.
- R-H5: the PHY lock bound (20 ms), the reset pulse (10 us) and the forced
  global reset around the PLL writes are ours, not the RM's.
- R-H6: the PHY's 8-bit registers are accessed 32 bits wide at their 4-byte
  stride.
- R-H7: the GPC registers may not be readable from Linux (secure world), so
  the dump may lack them; the power-up path is the media-power one, proven
  for MEDIAMIX.
- R-H8: SYSTEM_PLL1/2 rates are assumed as the boot firmware sets them
  (800/1000 MHz); on the direct boot nobody else sets them.
- R-H9: DVI mode: no AVI InfoFrame, RGB full range; a TV-type sink might
  expect HDMI mode (a monitor accepts DVI on HDMI).

## Open questions

1. Does Linux run the PHY in fractional mode at exactly 154 MHz, and if so, do
   its registers make the fractional formula unambiguous?
2. Is the PVI's timing generator on under Linux (and with what coordinates)?
3. Which `HTX_PIPE_CLK_SEL` / `HTXPHY_CLK_SEL` values does Linux set?
4. Does LCDIF3 need `LCDIF_SPU_CLK_EN` (13.11.3.1.5: "spu_clk", purpose not
   stated) or `TX_PHY_PIXEL_CLK_EN` (hal_nxp only; the RM marks bit 23
   reserved)?
5. Is HPD wired and muxed on the X2 (the pad's IOMUX is not set here)?
