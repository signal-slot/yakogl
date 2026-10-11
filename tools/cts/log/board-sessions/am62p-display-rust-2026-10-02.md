<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Verdin AM62P: the Rust display path brought up on the board (2026-10-02)

Status: finished record, moved here from `driver/docs/am62p-display-cold.md` ("Board plan" (b) and (c), "On the board (2026-10-02)") on 2026-10-11. The Rust DSS and DSI drivers matched the C drivers on the board and replaced them; the `_RUST` switches named below are gone. The design and what the runs settled are `driver/docs/am62p-display-cold.md`.

## The staged bring-up plan

**(b) Staged bring-up on the board**, each a `gl_triangle` image with
`CONFIG_PVR_SAMPLE_HOLD_SECONDS`, judged by the console and the camera against
the C-driver image of the same tree:

1. `CONFIG_SAFEUI_DSI_CDNS_J721E_RUST=y` only. Console: "Cadence DSI id
   0xcadd5131", the WIZ found at 0xf00, "D-PHY: 307812500 bit/s per lane,
   band 5", "DSI link up". Then the bridge's PLL lock and IRQ_STAT 0
   (sn65dsi8x self-start) and the picture.
2. `CONFIG_SAFEUI_DSS_AM62P_RUST=y` only. Console: "DSS rev 0x64040901",
   "DSS ready: VP2 1024x600". Counters after the hold (`am62p_dss_get_stats`,
   printed by the sample): vsyncs ~ 60 x seconds, underflows 0, sync_lost 0.
3. Both. Then Qt/Slint pictures (`rust_slint`, the route-3 SafeUI ring)
   against references, and the safety-layer flip rate unchanged.

If step 1 fails at the lanes: swap the lane start back to the C order (with
PLL_START) as a single change; if the picture rolls or tears: VID_VSIZE1 back
to the DPI totals; both are one-line changes recorded here first.

**(c) Then** flip both defaults to y, delete the two C drivers and the three
functions of `display_calc.c` with their ztest, and update the group E rows.
(Done 2026-10-02, with the switches removed rather than left at y.)

## On the board (2026-10-02)

**The dump** (driver/docs/am62p-display-dump-linux.txt): Torizon 6.6.138
driving the panel, STRICT_DEVMEM without IO_STRICT, so every address of the
read list above was readable as root; one 32-bit load each, no writes.
SN65DSI83 registers by `i2cdump -f` (IRQ_STAT 0).

**The WIZ offset**: MOD_VER (MODULE_ID 992h, 69924881h on this silicon) reads
at +F00h, and 0h..14h read 0. TI's driver is right; the SPRUJ83D register
summary (14.8.6.3.2: MOD_VER at 0h) is not. The probe keeps the fallback to 0h
(harmless) and logs where it found it.

**Golden differences.** Model gaps fixed in the model (TRM reset values it
lacked: GLOBAL_BUFFER 4688h, ATTRIBUTES2 3C000000h, MCTL_3DVIDEO_CTL 10h,
CMD_MODE_CTL2 7FF800h, DPI_CFG 8000000h, the wrapper's REVISION 640A1101h;
IRQENABLE_CLR reads the enables, 14.8.9.1.2.8) and the test reads status
through the model (PLL lock, CMN ready, lane ready). The rest are allow-list
lines with their reasons: live status (IRQSTATUS_RAW, LANE_STS, VID_VPOS/HPOS,
VID_MODE_STS, the *_STS_FLAG registers, the DPI FIFO level), the WIZ revision
field, and this project's choices (VSYNC instead of FRAMEDONE, the safety
layer ARGB8888 straight alpha on overlay layer 1 over VID on layer 0, no
gamma table, CBA_CFG never written). The PSM clock is 16 MHz on the board
(Linux's clk_summary), so the test asks for 16 MHz and PSM_FREQ agrees. No
driver change came from the comparison. Linux itself agrees with the C
drivers on every TRM difference of the table above (VSIZE1 11/22/12,
WAIT_BURST_TIME 0, FIFO low = size/2, MFLAG 2/3 / 1/3), so those stay in the
seeded part of the allow-list: the board, below, decides them.

**Staged runs** (`gl_triangle`, 40 s hold; the sample now logs the DSS
counters before and after it; same tree, C drivers vs Rust):

| image | console | counters after the hold | record |
|---|---|---|---|
| C drivers | DSI link up, sn65dsi8x link clean (IRQ_STAT 0x00) | vsyncs 2354, underflows 0, sync_lost 0 | `594b0400 ff403f80 35706c4e 04000258` |
| DSI Rust | `Cadence DSI id 0xcadd5131`, `WIZ at +0xf00 (MOD_VER 0x69924881)`, `307812500 bit/s per lane, band 5`, `DSI link up: 4 lanes, sync events, line 4047 B`, IRQ_STAT 0x00 | 2354 / 0 / 0 | same |
| DSS Rust | `DSS rev 0x64040901`, `DSS ready: VP2 1024x600 @ 51200000 Hz` | 2354 / 0 / 0 | same |
| both | the union | 2354 / 0 / 0 | same |
| gl_slint, both | 3525 frames in 60 s, 58 fps, GL error 0, the five read-backs equal the 2026-09-23 reference run's | 3524 application frames latched, 0 underflows, 0 sync_lost, 0 go_waits | PASS |
| dEQP fbo + shaders.texture_functions, both | | (port off in dEQP) | 731 passed / 0 / 0 / 0, complete=1 |

2354 vsyncs in 40.0 s is 58.8 Hz = 51.2 MHz / (1349 x 645), the RVT70's
totals: the same for both drivers. So on this board the TRM's values (VSIZE1
10/23/11, WAIT_BURST_TIME 4, FIFO low = size-8 with preload = low, MFLAG
1/4..1/2, lanes after PLL_LCK, no CBA_CFG write, EOI 1, maximum time-outs)
work: neither fallback was needed, no underflow or sync loss in 40 s with a
static plane nor in 60 s of 58 fps flips.
