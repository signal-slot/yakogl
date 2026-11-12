// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The RK3588's VOP2 (video output processor 2, at 0xfdd90000, 0x4200 bytes,
//! in PD_VOP; Linux v6.18 arch/arm64/boot/dts/rockchip/rk3588-base.dtsi:
//! 1369-1395): one linear XRGB8888 frame buffer in an Esmart window, alone on
//! one video port, the port's timing for a mode, the HDMI interface and its
//! dividers, `REG_CFG_DONE`, the VOP's own power domains, STANDBY, and the
//! status the board reads back (driver/docs/nanopc-t6-display.md, sections
//! 1, 5 and 7 row D1). The power domain PD_VOP, the clocks, the pixel clock
//! and the GRF writes are the caller's (`soc::rk3588`, step D4/D7); this
//! module only computes the `vop_grf` word (the `vo1_grf` one is `ip::dw_hdmi_qp`'s).
//!
//! Facts from Linux v6.18 (GPL-2.0: facts and file:line only, nothing
//! transcribed). `h:` is drivers/gpu/drm/rockchip/rockchip_drm_vop2.h, `c:`
//! rockchip_drm_vop2.c, `r:` rockchip_vop2_reg.c, `vop.h:`
//! rockchip_drm_vop.h, `drv.h:` rockchip_drm_drv.h, all in that directory.
//!
//! 1. Identity: `VERSION_INFO` (0x004) reads 0x40176786 on the RK3588
//!    (h:16, h:20); Linux refuses to go on when it reads otherwise (c:822-826).
//! 2. System registers (h:404-432): `REG_CFG_DONE` 0x000, `VERSION_INFO`
//!    0x004, `SYS_AUTO_GATING_CTRL` 0x008 (bit 31 auto gating, h:715),
//!    `DSP_IF_EN` 0x028, `DSP_IF_CTRL` 0x02c, `DSP_IF_POL` 0x030,
//!    `SYS_PD_CTRL` 0x034, `VP_LINE_FLAG(vp)` 0x070 + 4·vp,
//!    `SYS0_INT_EN/CLR/STATUS` 0x080/0x084/0x088, `SYS1_*` 0x090/0x094/0x098,
//!    `VP_INT_EN/CLR/STATUS/RAW_STATUS(vp)` 0x0a0/0x0a4/0x0a8/0x0ac + 0x10·vp.
//!    Interrupt bits (h:762-768): 0 frame start, 1 bus error (SYS), 4
//!    POST_BUF_EMPTY, 6 DSP_HOLD_VALID. An enable or clear write carries the
//!    bit and the bit << 16 (c:746-747, c:852-859).
//! 3. `REG_CFG_DONE` (h:638, h:847-864): bit 15 is the global enable and has
//!    no write-enable bit; bit n is video port n with its write-enable at
//!    n + 16. Linux sets bit 15, bit vp and bit vp + 16 together. Window and
//!    port registers take effect at the next frame after it
//!    (driver/docs/nanopc-t6-display.md, section 1).
//! 4. VOP-internal power (h:69-75, h:414; c:788-797): `SYS_PD_CTRL` bit n is
//!    Cluster n (0-3), bit 7 every Esmart window; 0 = on. Linux clears those
//!    five bits and polls nothing.
//! 5. Bring-up of the block (c:799-860): read the version, clear the five
//!    power bits, read `OVL_LAYER_SEL` and `OVL_PORT_SEL`, write
//!    `REG_CFG_DONE` = bit 15, clear auto gating, then for `SYS0_INT` and
//!    `SYS1_INT` clear and enable the bus error bit.
//! 6. Video ports (h:484-510; r:1057-1090): bases 0xc00, 0xd00, 0xe00, 0xf00;
//!    `DSP_CTRL` +0x00 (bit 31 STANDBY, `OUT_MODE` 3:0, `DITHER_DOWN_SEL`
//!    19:18; h:640-654), `MIPI_CTRL` +0x04, `CLK_CTRL` +0x0c
//!    (`DCLK_CORE_DIV` 1:0, `DCLK_OUT_DIV` 3:2; h:658-659), `DSP_BG` +0x2c,
//!    `PRE_SCAN_HTIMING` +0x30, `POST_DSP_HACT_INFO` +0x34,
//!    `POST_DSP_VACT_INFO` +0x38, `POST_SCL_FACTOR_YRGB` +0x3c,
//!    `POST_SCL_CTRL` +0x40, `DSP_HTOTAL_HS_END` +0x48, `DSP_HACT_ST_END`
//!    +0x4c, `DSP_VTOTAL_VS_END` +0x50, `DSP_VACT_ST_END` +0x54. Largest
//!    output: VP0-2 4096x2304, VP3 2048x1536; VP0-2 have 10-bit output.
//!    `pre_scan_max_dly[3]` is 54 for VP0 and VP1, 52 for VP2 and VP3.
//! 7. The timing words (c:1615-1624, c:1697-1727): `DSP_HTOTAL_HS_END` =
//!    htotal << 16 | hsync; `DSP_HACT_ST_END` = hact_st << 16 | hact_st +
//!    hactive with hact_st = htotal - hsync_start = hsync + hback;
//!    `DSP_VACT_ST_END` likewise with vsync + vback; `DSP_VTOTAL_VS_END` =
//!    vtotal << 16 | vsync; `VP_LINE_FLAG` = vact_end << 16 | vact_end (c:1724-
//!    1725 with 0 us, c:1599-1602); `MIPI_CTRL` = 0 (c:1734). All 16-bit
//!    fields (c:1615-1624 are u16).
//! 8. The post-scaler at 100 % margins (c:1546-1597): `POST_DSP_HACT_INFO` =
//!    hact_st << 16 | hact_st + hsize with hsize = hactive rounded down to
//!    even, the same for V; `POST_SCL_FACTOR_YRGB` = f(vactive, vsize) << 16
//!    | f(hactive, hsize) with f(s, d) = ((s - 1) << 12) / (d - 1)
//!    (vop.h:361-364); `POST_SCL_CTRL` bit 0 when hsize differs from hactive,
//!    bit 1 for V (h:661-662); `DSP_BG` = 0. Before them the background
//!    delay (r:2408-2423, r:2459): `VP_BG_MIX_CTRL(vp)` 0x6e0 + 4·vp =
//!    `pre_scan_max_dly[3]` << 24 (h:528, h:729); `PRE_SCAN_HTIMING` =
//!    (dly + hactive / 2 - 1) << 16 | hsync.
//! 9. `DSP_CTRL` for HDMI RGB (c:1679-1695, c:1517-1544; dw_hdmi_qp-rockchip.c
//!    :144): the HDMI encoder asks for `OUT_MODE` AAAA (15, drv.h:36), which a
//!    port without 10-bit output (VP3) gets as P888 (0, drv.h:27); no swap,
//!    no dither for an RGB bus; `DITHER_DOWN_SEL` ALLEGRO = 0 (vop.h:341).
//!    Linux's order: timings, pixel clock, post-scaler, `REG_CFG_DONE` for
//!    the port, and last `DSP_CTRL` with STANDBY clear (c:1697-1780).
//! 10. HDMI interface (r:1548-1642, r:1670-1779): the port runs four pixels
//!     per core clock: `DCLK_CORE_DIV` = log2(4) = 2; the interface's
//!     `PCLK_DIV` = log2(2) = 1 and `DCLK_DIV` = log2(4) = 2 (RGB, not
//!     YUV420). HDMI0: `DSP_IF_EN` bit 3 and mux 17:16 = vp; `DSP_IF_CTRL`
//!     `DCLK_DIV` 17:16, `PCLK_DIV` 18. HDMI1: bit 5, mux 19:18, 21:20, 22
//!     (h:679-700). `DSP_IF_POL` gets bit 28 (`CFG_DONE_IMD`, h:717) and no
//!     HDMI polarity. Written in the order `VP_CLK_CTRL`, `DSP_IF_EN`,
//!     `DSP_IF_CTRL`, `DSP_IF_POL` (r:1773-1776). The VOP's own GRF word:
//!     `vop_grf` `GRF_VOP_CON2` 0x08 bit 1 (HDMI1 bit 4) = 1 with its
//!     write-enable bit (h:400, r:1699, r:1712). The sync field in `vo1_grf`
//!     `GRF_VO1_CON0` (0x00 bits 6:5, HDMI1 8:7; h:401, r:1700-1701, r:1713)
//!     is not this module's: the HDMI controller module (`ip::dw_hdmi_qp`,
//!     step D6) owns it, so there is one formula. For reading it: Linux
//!     builds a polarity word with bit 0 for a positive HSYNC, bit 1 for a
//!     positive VSYNC and bit 3 for an inverted pixel clock (c:1655-1661,
//!     vop.h:346-347, h:770) and passes that word, not the mode's flags, to
//!     the RK3588 interface setup (c:1671); the field helper tests its
//!     argument with the bits of the mode flags NHSYNC (bit 1) and NVSYNC
//!     (bit 3) (r:1660-1668, include/uapi/drm/drm_mode.h:72, :74), so field
//!     bit 0 follows a positive VSYNC, field bit 1 the clock inversion, and
//!     HSYNC has no effect. Measured on the T6 (2026-11-12, Linux scanning
//!     1024x768@60 with both syncs negative on both HDMI ports): `vo1_grf`
//!     0x00 reads 0x00000000.
//! 11. Overlay (h:513-530, h:725-746; r:2098-2271, r:2316-2341): `OVL_CTRL`
//!     0x600 (`LAYERSEL_REGDONE_SEL` 31:30, `LAYERSEL_REGDONE_IMD` bit 28,
//!     `YUV_MODE(vp)` bit vp), `OVL_LAYER_SEL` 0x604 (layer n's window id in
//!     bits 4n+3:4n; Linux clears 3 bits of it), `OVL_PORT_SEL` 0x608
//!     (per-window port 31:16, Esmart0 25:24, Esmart1 27:26, Esmart2 29:28,
//!     Esmart3 31:30; `PORTn_MUX` 4n+3:4n = the last layer of port n, 8 for
//!     a port without layers, `PORT3_MUX` fixed 7 on the RK3588). Moving a
//!     window to layer L moves the window found on L to the window's old
//!     layer. A changed `OVL_CTRL` gets `LAYERSEL_REGDONE_SEL` = vp. A
//!     changed port select is followed by `REG_CFG_DONE` for the port and a
//!     poll, every 0 us for at most 50 ms, until `OVL_PORT_SEL` reads the
//!     value written: it reads the value in force, not the one written
//!     (r:2063-2076, r:2260-2264). Then `OVL_LAYER_SEL`. Linux does this
//!     after the port runs (`atomic_begin` after `atomic_enable`, c:1838-1845).
//! 12. With one layer on VP0 (r:1960-2056, r:2273-2314): `HDR0_SRC_COLOR_CTRL`
//!     0x6c0 = 0 (h:524); `CLUSTER_DLY_NUM` 0x6f0 = 0; `SMART_DLY_NUM` 0x6f8
//!     holds the window's delay, 23 for an Esmart (r:1214), in 7:0 for
//!     Esmart0, 15:8 Esmart1, 23:16 Esmart2, 31:24 Esmart3 (h:529-530,
//!     h:757-760).
//! 13. Esmart windows (h:537-542, h:571-584, h:618; r:1199-1266): bases
//!     0x1800, 0x1a00, 0x1c00, 0x1e00; layer ids 2, 3, 6, 7; AXI bus 0, 0, 1,
//!     1; YRGB read ids 0x0a, 0x0c, 0x0a, 0x0c; UV read ids 0x0b, 0x01, 0x0b,
//!     0x0d. Registers (r:350-392): `CTRL0` +0x00 (Y2R bit 0, R2Y bit 1, CSC
//!     3:2), `CTRL1` +0x04 (YRGB read id 8:4, UV read id 16:12, Y mirror bit
//!     31), `AXI_CTRL` +0x08 (bus id bit 1), `REGION0_CTRL` +0x10 (enable bit
//!     0, format 5:1, scale-down skip bits 8 and 9, dither up bit 12, R/B swap
//!     bit 14, UV swap bit 16, YUV clip bit 17), `REGION0_YRGB_MST` +0x14,
//!     `REGION0_VIR` +0x1c (YRGB stride in 32-bit words 15:0),
//!     `REGION0_ACT_INFO` +0x20, `REGION0_DSP_INFO` +0x24 (both (h - 1) << 16
//!     | (w - 1), c:1251-1252), `REGION0_DSP_ST` +0x28 (y << 16 | x, c:1387),
//!     `REGION0_SCL_CTRL` +0x30 (H mode 1:0, H filter 3:2, V mode 5:4, V
//!     filter 7:6), `REGION0_SCL_FACTOR_YRGB` +0x34, `COLOR_KEY_CTRL` +0xd0
//!     (enable bit 31).
//! 14. A linear XRGB8888 window, unscaled (c:1132-1402): format 0 (ARGB8888,
//!     c:71-72, c:179-183), no R/B swap (c:257-270), no UV swap, no dither up
//!     (c:302-311), stride = pitch / 4 rounded up (c:1358), no mirror; the
//!     scale factors 0 for equal sizes (c:530-536); scale modes none (0) and
//!     both filters "down, bilinear" (1) for a size that is not scaled up
//!     (c:583-594, vop.h:311-313, h:59-63); color key off (c:1096-1098); no
//!     colour conversion for RGB in and out (c:731-739); the AXI ids of the
//!     window (c:1263-1267); enable written last (c:1394).
//! 15. Disabling a window clears enable and YUV clip (c:1064-1080).
//! 16. STANDBY (c:950-969): `DSP_CTRL` = STANDBY takes effect at the end of
//!     the frame and raises DSP_HOLD_VALID; Linux waits for it at most 50 ms,
//!     and the bus clocks must not stop before it.
//! 17. Cluster windows (h:533-536, h:544, h:566; r:282-315, r:1121-1198):
//!     bases 0x1000, 0x1200, 0x1400, 0x1600; layer ids 0, 1, 4, 5; port
//!     fields in `OVL_PORT_SEL` 17:16, 19:18, 21:20, 23:22 (h:738-741).
//!     `CLUSTER_WIN_CTRL0` +0x000 has the window enable in bit 0 and YUV clip
//!     in bit 19; `CLUSTER_CTRL` +0x100 has `CLUSTER_ENABLE` in bit 0. Linux
//!     disables a window with enable 0 and, for a Cluster, `CLUSTER_ENABLE` 0
//!     (c:149-155), YUV clip 0 (c:1078-1079), and disables every plane of a
//!     port before STANDBY (c:946-947). Linux puts its own picture on
//!     Cluster0 of VP0 (and of VP1 for the T6's second HDMI output; board
//!     reading of 2026-11-12) and splits the eight layers evenly over the
//!     ports it uses (c:2474-2484).
//!
//! The take-over from a system that left the VOP running (route (a) of
//! driver/docs/nanopc-t6-display.md, section 2): (1) [`take_over`] disables
//! every window of the ports it names (VP0, and VP1 when Linux drives the
//! second output) except the one asked to keep, writes `REG_CFG_DONE` for
//! those ports and waits for one frame start on each; (2)
//! `rk_iommu::paging_off`, so nothing fetches through Linux's page tables
//! any more by the time they stop being used; (3) [`overlay_single`] and
//! [`window_show`] with our buffer. Doing (2) before (1) would let Linux's
//! windows read their IOVAs as physical addresses for up to a frame; doing
//! (3) before (1) leaves Linux's windows enabled outside every port's layer
//! range, whose fetch behaviour the source does not tell.
//!
//! What is ours, not Linux's: the window must lie inside the active area (Linux
//! clips, c:1211-1231), start on 64 bytes (the Mali render target's rule,
//! nanopc-t6-display.md section 4; Linux has no rule) and end below 4 GiB
//! (the IOMMU is off, so `YRGB_MST`'s 32 bits are the physical address); the
//! status reads use `VP_INT_RAW_STATUS` and polls instead of interrupts.
//! `DCLK_OUT_DIV` is written 0 for HDMI: r:1548-1642 never assigns it for an
//! HDMI port (the variable is left as declared, r:1673), so the board dump
//! (step D0) is what confirms it.

use crate::mmio::{field, Mmio};
use crate::video::VideoMode;

/// `VERSION_INFO` of the RK3588 (fact 1).
pub const VERSION_RK3588: u32 = 0x4017_6786;

/// System registers (fact 2).
pub mod sys {
    pub const REG_CFG_DONE: usize = 0x000;
    pub const VERSION_INFO: usize = 0x004;
    pub const AUTO_GATING_CTRL: usize = 0x008;
    pub const DSP_IF_EN: usize = 0x028;
    pub const DSP_IF_CTRL: usize = 0x02c;
    pub const DSP_IF_POL: usize = 0x030;
    pub const PD_CTRL: usize = 0x034;
    pub const SYS0_INT_EN: usize = 0x080;
    pub const SYS0_INT_CLR: usize = 0x084;
    pub const SYS0_INT_STATUS: usize = 0x088;
    pub const SYS1_INT_EN: usize = 0x090;
    pub const SYS1_INT_CLR: usize = 0x094;
    pub const SYS1_INT_STATUS: usize = 0x098;

    pub const fn vp_line_flag(vp: usize) -> usize {
        0x070 + 4 * vp
    }
    pub const fn vp_int_en(vp: usize) -> usize {
        0x0a0 + 0x10 * vp
    }
    pub const fn vp_int_clr(vp: usize) -> usize {
        0x0a4 + 0x10 * vp
    }
    pub const fn vp_int_status(vp: usize) -> usize {
        0x0a8 + 0x10 * vp
    }
    pub const fn vp_int_raw_status(vp: usize) -> usize {
        0x0ac + 0x10 * vp
    }

    /// `REG_CFG_DONE` (fact 3).
    pub const CFG_DONE_GLB: u32 = 1 << 15;
    /// `SYS_AUTO_GATING_CTRL` auto gating enable.
    pub const AUTO_GATING_EN: u32 = 1 << 31;
    /// `SYS_PD_CTRL`: Cluster0-3 and every Esmart (fact 4).
    pub const PD_CLUSTERS_ESMART: u32 = 0xf | 1 << 7;
    /// `DSP_IF_POL` `CFG_DONE_IMD`.
    pub const POL_CFG_DONE_IMD: u32 = 1 << 28;

    /// Interrupt bits (fact 2).
    pub const INT_FS: u32 = 1 << 0;
    pub const INT_BUS_ERROR: u32 = 1 << 1;
    pub const INT_POST_BUF_EMPTY: u32 = 1 << 4;
    pub const INT_DSP_HOLD_VALID: u32 = 1 << 6;
}

/// Video port registers, offsets from [`vp::base`] (fact 6).
pub mod vp {
    pub const DSP_CTRL: usize = 0x00;
    pub const MIPI_CTRL: usize = 0x04;
    pub const CLK_CTRL: usize = 0x0c;
    pub const DSP_BG: usize = 0x2c;
    pub const PRE_SCAN_HTIMING: usize = 0x30;
    pub const POST_DSP_HACT_INFO: usize = 0x34;
    pub const POST_DSP_VACT_INFO: usize = 0x38;
    pub const POST_SCL_FACTOR_YRGB: usize = 0x3c;
    pub const POST_SCL_CTRL: usize = 0x40;
    pub const DSP_HTOTAL_HS_END: usize = 0x48;
    pub const DSP_HACT_ST_END: usize = 0x4c;
    pub const DSP_VTOTAL_VS_END: usize = 0x50;
    pub const DSP_VACT_ST_END: usize = 0x54;

    pub const fn base(vp: usize) -> usize {
        0xc00 + 0x100 * vp
    }

    pub const DSP_CTRL_STANDBY: u32 = 1 << 31;
    pub const DSP_CTRL_OUT_MODE: u32 = 0xf;
    pub const OUT_MODE_P888: u32 = 0;
    pub const OUT_MODE_AAAA: u32 = 15;
    pub const CLK_CTRL_DCLK_CORE_DIV: u32 = 0x3;
    pub const CLK_CTRL_DCLK_OUT_DIV: u32 = 0x3 << 2;
    pub const POST_SCL_CTRL_HSCALEDOWN: u32 = 1 << 0;
    pub const POST_SCL_CTRL_VSCALEDOWN: u32 = 1 << 1;
}

/// Overlay registers (facts 8, 11, 12).
pub mod ovl {
    pub const CTRL: usize = 0x600;
    pub const LAYER_SEL: usize = 0x604;
    pub const PORT_SEL: usize = 0x608;
    pub const HDR0_SRC_COLOR_CTRL: usize = 0x6c0;
    pub const CLUSTER_DLY_NUM: usize = 0x6f0;
    pub const SMART_DLY_NUM: usize = 0x6f8;

    pub const fn vp_bg_mix_ctrl(vp: usize) -> usize {
        0x6e0 + 4 * vp
    }

    pub const CTRL_REGDONE_SEL: u32 = 0x3 << 30;
    pub const CTRL_REGDONE_IMD: u32 = 1 << 28;
    pub const PORT_SEL_WINDOWS: u32 = 0xffff_0000;
    pub const PORT3_MUX_RK3588: u32 = 7;
    /// `PORTn_MUX` of a port without layers.
    pub const MUX_NO_LAYER: u32 = 8;
    pub const BG_MIX_CTRL_BG_DLY: u32 = 0xff << 24;
}

/// Esmart window registers, offsets from [`Esmart::base`] (fact 13).
pub mod esmart {
    pub const CTRL0: usize = 0x00;
    pub const CTRL1: usize = 0x04;
    pub const AXI_CTRL: usize = 0x08;
    pub const REGION0_CTRL: usize = 0x10;
    pub const REGION0_YRGB_MST: usize = 0x14;
    pub const REGION0_VIR: usize = 0x1c;
    pub const REGION0_ACT_INFO: usize = 0x20;
    pub const REGION0_DSP_INFO: usize = 0x24;
    pub const REGION0_DSP_ST: usize = 0x28;
    pub const REGION0_SCL_CTRL: usize = 0x30;
    pub const REGION0_SCL_FACTOR_YRGB: usize = 0x34;
    pub const COLOR_KEY_CTRL: usize = 0xd0;

    pub const CTRL0_CSC: u32 = 0xf;
    pub const CTRL1_YRGB_R_ID: u32 = 0x1f << 4;
    pub const CTRL1_UV_R_ID: u32 = 0x1f << 12;
    pub const CTRL1_YMIRROR: u32 = 1 << 31;
    pub const AXI_CTRL_BUS_ID: u32 = 1 << 1;
    pub const R0_ENABLE: u32 = 1 << 0;
    pub const R0_FORMAT: u32 = 0x1f << 1;
    pub const R0_VSD_GT2: u32 = 1 << 8;
    pub const R0_VSD_GT4: u32 = 1 << 9;
    pub const R0_DITHER_UP: u32 = 1 << 12;
    pub const R0_RB_SWAP: u32 = 1 << 14;
    pub const R0_UV_SWAP: u32 = 1 << 16;
    pub const R0_YUV_CLIP: u32 = 1 << 17;
    pub const VIR_YRGB: u32 = 0xffff;
    pub const SCL_CTRL_YRGB: u32 = 0xff;
    /// H and V filter "down, bilinear", both modes none (fact 14).
    pub const SCL_CTRL_UNSCALED: u32 = 1 << 2 | 1 << 6;
    pub const COLOR_KEY_EN: u32 = 1 << 31;
    /// Format code of ARGB8888 and XRGB8888.
    pub const FMT_ARGB8888: u32 = 0;
}

/// A video port.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Vp {
    Vp0 = 0,
    Vp1 = 1,
    Vp2 = 2,
    Vp3 = 3,
}

impl Vp {
    pub const fn id(self) -> usize {
        self as usize
    }
    /// The largest active area (fact 6).
    pub const fn max_output(self) -> (u32, u32) {
        match self {
            Vp::Vp3 => (2048, 1536),
            _ => (4096, 2304),
        }
    }
    /// `pre_scan_max_dly[3]` (fact 6).
    pub const fn bg_dly(self) -> u32 {
        match self {
            Vp::Vp0 | Vp::Vp1 => 54,
            Vp::Vp2 | Vp::Vp3 => 52,
        }
    }
    /// What `OUT_MODE` AAAA becomes on this port (fact 9).
    pub const fn hdmi_out_mode(self) -> u32 {
        match self {
            Vp::Vp3 => vp::OUT_MODE_P888,
            _ => vp::OUT_MODE_AAAA,
        }
    }
}

/// An Esmart window (fact 13).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Esmart {
    E0,
    E1,
    E2,
    E3,
}

impl Esmart {
    pub const ALL: [Esmart; 4] = [Esmart::E0, Esmart::E1, Esmart::E2, Esmart::E3];
    pub const fn base(self) -> usize {
        match self {
            Esmart::E0 => 0x1800,
            Esmart::E1 => 0x1a00,
            Esmart::E2 => 0x1c00,
            Esmart::E3 => 0x1e00,
        }
    }
    /// The id `OVL_LAYER_SEL` names it by.
    pub const fn layer_sel_id(self) -> u32 {
        match self {
            Esmart::E0 => 2,
            Esmart::E1 => 3,
            Esmart::E2 => 6,
            Esmart::E3 => 7,
        }
    }
    /// (bus id, YRGB read id, UV read id).
    pub const fn axi(self) -> (u32, u32, u32) {
        match self {
            Esmart::E0 => (0, 0x0a, 0x0b),
            Esmart::E1 => (0, 0x0c, 0x01),
            Esmart::E2 => (1, 0x0a, 0x0b),
            Esmart::E3 => (1, 0x0c, 0x0d),
        }
    }
    /// Its port field in `OVL_PORT_SEL`.
    pub const fn port_sel_field(self) -> u32 {
        match self {
            Esmart::E0 => 0x3 << 24,
            Esmart::E1 => 0x3 << 26,
            Esmart::E2 => 0x3 << 28,
            Esmart::E3 => 0x3 << 30,
        }
    }
    /// Its field in `SMART_DLY_NUM`.
    pub const fn dly_field(self) -> u32 {
        match self {
            Esmart::E0 => 0xff,
            Esmart::E1 => 0xff << 8,
            Esmart::E2 => 0xff << 16,
            Esmart::E3 => 0xff << 24,
        }
    }
}

/// A Cluster window (fact 17). This module never shows one; it only turns
/// off the one Linux leaves on (the take-over).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cluster {
    C0,
    C1,
    C2,
    C3,
}

impl Cluster {
    pub const ALL: [Cluster; 4] = [Cluster::C0, Cluster::C1, Cluster::C2, Cluster::C3];
    pub const fn base(self) -> usize {
        match self {
            Cluster::C0 => 0x1000,
            Cluster::C1 => 0x1200,
            Cluster::C2 => 0x1400,
            Cluster::C3 => 0x1600,
        }
    }
    /// Its port field in `OVL_PORT_SEL`.
    pub const fn port_sel_field(self) -> u32 {
        match self {
            Cluster::C0 => 0x3 << 16,
            Cluster::C1 => 0x3 << 18,
            Cluster::C2 => 0x3 << 20,
            Cluster::C3 => 0x3 << 22,
        }
    }
}

/// Cluster window registers, offsets from [`Cluster::base`] (fact 17).
pub mod cluster {
    pub const WIN_CTRL0: usize = 0x00;
    pub const CTRL: usize = 0x100;
    pub const WIN_CTRL0_EN: u32 = 1 << 0;
    pub const WIN_CTRL0_YUV_CLIP: u32 = 1 << 19;
    pub const CTRL_CLUSTER_EN: u32 = 1 << 0;
}

/// The delay of an Esmart window in the default mode (fact 12).
pub const ESMART_DLY: u32 = 23;

/// An HDMI transmitter the VOP feeds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HdmiPort {
    Hdmi0,
    Hdmi1,
}

/// How long a frame-boundary event is waited for: Linux's 50 ms (facts 11, 16).
pub const FRAME_WAIT_US: u32 = 50_000;
/// The poll step of those waits.
pub const POLL_STEP_US: u32 = 100;

/// Why a call refused or failed. The value crosses to C, one number per cause.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum Refusal {
    /// `VERSION_INFO` is not the RK3588's: not this VOP, or one not powered
    /// or clocked. Nothing was written.
    VersionMismatch = 1,
    /// The mode does not fit the port's limits or the 16-bit fields.
    BadMode = 2,
    /// The window would reach at or above 4 GiB.
    WindowAbove4G = 3,
    /// The address is not on 64 bytes, or the stride not on 4 bytes.
    WindowUnaligned = 4,
    /// The stride in 32-bit words does not fit `REGION0_VIR`'s 16 bits.
    StrideTooWide = 5,
    /// The stride is shorter than one line of the window.
    StrideTooNarrow = 6,
    /// Zero size, or the window does not lie inside the active area.
    WindowOutside = 7,
    /// `OVL_PORT_SEL` did not take the new value within [`FRAME_WAIT_US`]:
    /// the port does not run frames.
    PortSelTimeout = 8,
    /// DSP_HOLD_VALID did not come within [`FRAME_WAIT_US`] of STANDBY.
    StandbyTimeout = 9,
    /// The port cannot take the request (the single-layer overlay on VP3).
    BadPort = 10,
    /// An event bit still reads 1 right after `VP_INT_CLR`: a later 1 could
    /// be the old event, so it is not taken as new.
    EventNotCleared = 11,
    /// No frame start within [`FRAME_WAIT_US`]: the port does not run.
    FrameTimeout = 12,
}

/// A refusal and the register value it is about (0 when there is none).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Refused {
    pub why: Refusal,
    pub read_back: u32,
}

const fn refuse(why: Refusal) -> Refused {
    Refused { why, read_back: 0 }
}

fn rd(m: &impl Mmio, base: usize, off: usize) -> u32 {
    m.read32(base + off)
}

fn wr(m: &impl Mmio, base: usize, off: usize, v: u32) {
    m.write32(base + off, v)
}

fn modify(m: &impl Mmio, base: usize, off: usize, mask: u32, v: u32) {
    m.modify32(base + off, mask, v)
}

/// `VERSION_INFO`.
pub fn version(m: &impl Mmio, base: usize) -> u32 {
    rd(m, base, sys::VERSION_INFO)
}

/// What [`init`] found and left.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Init {
    pub version: u32,
    /// `SYS_PD_CTRL` before and after the five power bits were cleared.
    pub pd_found: u32,
    pub pd_now: u32,
    /// `OVL_LAYER_SEL` and `OVL_PORT_SEL` as found, the values in force.
    pub layer_sel: u32,
    pub port_sel: u32,
}

/// Bring the block up as Linux does once PD_VOP and its clocks are on
/// (fact 5): check the version (nothing is written when it is wrong), power
/// the Cluster and Esmart domains, `REG_CFG_DONE` global, auto gating off,
/// bus error interrupts cleared and enabled. `pd_now` is read back for the
/// caller to log (Linux polls nothing, nanopc-t6-display.md U6).
pub fn init(m: &impl Mmio, base: usize) -> Result<Init, Refused> {
    let version = version(m, base);
    if version != VERSION_RK3588 {
        return Err(Refused { why: Refusal::VersionMismatch, read_back: version });
    }
    let pd_found = rd(m, base, sys::PD_CTRL);
    wr(m, base, sys::PD_CTRL, pd_found & !sys::PD_CLUSTERS_ESMART);
    let pd_now = rd(m, base, sys::PD_CTRL);
    let layer_sel = rd(m, base, ovl::LAYER_SEL);
    let port_sel = rd(m, base, ovl::PORT_SEL);
    wr(m, base, sys::REG_CFG_DONE, sys::CFG_DONE_GLB);
    modify(m, base, sys::AUTO_GATING_CTRL, sys::AUTO_GATING_EN, 0);
    let bus = sys::INT_BUS_ERROR << 16 | sys::INT_BUS_ERROR;
    wr(m, base, sys::SYS0_INT_CLR, bus);
    wr(m, base, sys::SYS0_INT_EN, bus);
    wr(m, base, sys::SYS1_INT_CLR, bus);
    wr(m, base, sys::SYS1_INT_EN, bus);
    Ok(Init { version, pd_found, pd_now, layer_sel, port_sel })
}

/// `REG_CFG_DONE` for one port (fact 3): its shadowed registers take effect
/// at its next frame. With the write-enable bits only this port's bit is
/// touched, so a plain write is Linux's read-modify-write.
pub fn cfg_done(m: &impl Mmio, base: usize, port: Vp) {
    let bit = 1 << port.id();
    wr(m, base, sys::REG_CFG_DONE, sys::CFG_DONE_GLB | bit << 16 | bit);
}

/// The port's bit in `REG_CFG_DONE` reads 1 (whether it clears once taken
/// is nanopc-t6-display.md U8).
pub fn cfg_pending(m: &impl Mmio, base: usize, port: Vp) -> bool {
    rd(m, base, sys::REG_CFG_DONE) & 1 << port.id() != 0
}

/// The words a mode puts in a video port (facts 6-9), for HDMI RGB 8 bit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VpTiming {
    /// The port the words are for: [`vp_set_timing`] and [`vp_start`] write
    /// them there and nowhere else.
    pub port: Vp,
    pub htotal_hs_end: u32,
    pub hact_st_end: u32,
    pub vtotal_vs_end: u32,
    pub vact_st_end: u32,
    pub line_flag: u32,
    pub bg_mix_ctrl: u32,
    pub pre_scan_htiming: u32,
    pub post_dsp_hact_info: u32,
    pub post_dsp_vact_info: u32,
    pub post_scl_factor_yrgb: u32,
    pub post_scl_ctrl: u32,
    pub dsp_ctrl: u32,
}

/// f(s, d) of fact 8.
const fn post_scale(src: u32, dst: u32) -> u32 {
    (((src - 1) << 12) / (dst - 1)) & 0xffff
}

/// The timing words of `mode` on `port` (facts 6-9). Refused when a field is
/// 0 or wider than 16 bits (each is bounded before any sum, so nothing can
/// wrap), a word field would overflow 16 bits, or the active area exceeds
/// the port's largest output.
pub fn vp_timing(mode: &VideoMode, port: Vp) -> Result<VpTiming, Refused> {
    let (max_w, max_h) = port.max_output();
    let fields = [mode.hactive, mode.hfront, mode.hsync, mode.hback, mode.vactive, mode.vfront, mode.vsync, mode.vback];
    if fields.iter().any(|&f| f > 0xffff) {
        return Err(refuse(Refusal::BadMode));
    }
    if mode.check().is_err() || mode.hactive < 2 || mode.vactive < 2 || mode.hactive > max_w || mode.vactive > max_h {
        return Err(refuse(Refusal::BadMode));
    }
    let hact_st = mode.hsync + mode.hback;
    let vact_st = mode.vsync + mode.vback;
    let hact_end = hact_st + mode.hactive;
    let vact_end = vact_st + mode.vactive;
    let dly = port.bg_dly();
    let pre_scan = dly + mode.hactive / 2 - 1;
    // every 16-bit half (VideoMode::check bounds htotal and vtotal)
    if [hact_end, vact_end, pre_scan].iter().any(|&x| x > 0xffff) {
        return Err(refuse(Refusal::BadMode));
    }
    let hsize = mode.hactive & !1;
    let vsize = mode.vactive & !1;
    let mut post_scl_ctrl = 0;
    if hsize != mode.hactive {
        post_scl_ctrl |= vp::POST_SCL_CTRL_HSCALEDOWN;
    }
    if vsize != mode.vactive {
        post_scl_ctrl |= vp::POST_SCL_CTRL_VSCALEDOWN;
    }
    Ok(VpTiming {
        port,
        htotal_hs_end: mode.htotal() << 16 | mode.hsync,
        hact_st_end: hact_st << 16 | hact_end,
        vtotal_vs_end: mode.vtotal() << 16 | mode.vsync,
        vact_st_end: vact_st << 16 | vact_end,
        line_flag: vact_end << 16 | vact_end,
        bg_mix_ctrl: field(ovl::BG_MIX_CTRL_BG_DLY, dly),
        pre_scan_htiming: pre_scan << 16 | mode.hsync,
        post_dsp_hact_info: hact_st << 16 | (hact_st + hsize),
        post_dsp_vact_info: vact_st << 16 | (vact_st + vsize),
        post_scl_factor_yrgb: post_scale(mode.vactive, vsize) << 16 | post_scale(mode.hactive, hsize),
        post_scl_ctrl,
        dsp_ctrl: field(vp::DSP_CTRL_OUT_MODE, port.hdmi_out_mode()),
    })
}

/// The interface words of one port driving one HDMI transmitter (fact 10):
/// `VP_CLK_CTRL` in full, and for each shared register the bits it owns
/// (`mask`) and their value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HdmiIf {
    pub clk_ctrl: u32,
    pub if_en_mask: u32,
    pub if_en: u32,
    pub if_ctrl_mask: u32,
    pub if_ctrl: u32,
}

/// [`HdmiIf`] for `port` feeding `hdmi`, RGB.
pub const fn hdmi_if(port: Vp, hdmi: HdmiPort) -> HdmiIf {
    let core_div = 2; // log2(4)
    let pclk_div = 1; // log2(2)
    let dclk_div = 2; // log2(4)
    let vp = port.id() as u32;
    let (en_bit, mux_shift, dclk_shift, pclk_bit) = match hdmi {
        HdmiPort::Hdmi0 => (3, 16, 16, 18),
        HdmiPort::Hdmi1 => (5, 18, 20, 22),
    };
    HdmiIf {
        clk_ctrl: core_div, // DCLK_OUT_DIV 0: see the module text
        if_en_mask: 1 << en_bit | 0x3 << mux_shift,
        if_en: 1 << en_bit | vp << mux_shift,
        if_ctrl_mask: 0x3 << dclk_shift | 1 << pclk_bit,
        if_ctrl: dclk_div << dclk_shift | pclk_div << pclk_bit,
    }
}

/// Route `port` to `hdmi` (fact 10), in Linux's order.
pub fn hdmi_route(m: &impl Mmio, base: usize, port: Vp, hdmi: HdmiPort) {
    let w = hdmi_if(port, hdmi);
    wr(m, base, vp::base(port.id()) + vp::CLK_CTRL, w.clk_ctrl);
    modify(m, base, sys::DSP_IF_EN, w.if_en_mask, w.if_en);
    modify(m, base, sys::DSP_IF_CTRL, w.if_ctrl_mask, w.if_ctrl);
    modify(m, base, sys::DSP_IF_POL, sys::POL_CFG_DONE_IMD, sys::POL_CFG_DONE_IMD);
}

/// `GRF_VOP_CON2` in the VOP GRF (fact 10).
pub const GRF_VOP_CON2: usize = 0x08;

/// The `GRF_VOP_CON2` word an HDMI route needs (fact 10), mask in 31:16.
/// The caller writes it to `vop_grf` + [`GRF_VOP_CON2`]. The `vo1_grf` sync
/// word is the HDMI controller module's (fact 10).
pub const fn hdmi_vop_grf(hdmi: HdmiPort) -> u32 {
    let bit = match hdmi {
        HdmiPort::Hdmi0 => 1,
        HdmiPort::Hdmi1 => 4,
    };
    1 << (bit + 16) | 1 << bit
}

/// Write the port's timing (fact 7): the pixel clock is set after this and
/// before [`vp_start`], as Linux does.
pub fn vp_set_timing(m: &impl Mmio, base: usize, t: &VpTiming) {
    let port = t.port;
    let v = vp::base(port.id());
    wr(m, base, v + vp::DSP_HTOTAL_HS_END, t.htotal_hs_end);
    wr(m, base, v + vp::DSP_HACT_ST_END, t.hact_st_end);
    wr(m, base, v + vp::DSP_VACT_ST_END, t.vact_st_end);
    wr(m, base, sys::vp_line_flag(port.id()), t.line_flag);
    wr(m, base, v + vp::DSP_VTOTAL_VS_END, t.vtotal_vs_end);
    wr(m, base, v + vp::MIPI_CTRL, 0);
}

/// Start the port with its pixel clock running (facts 8, 9): background
/// delay, post-scaler, `REG_CFG_DONE` for the port, then `DSP_CTRL` with
/// STANDBY clear, last.
pub fn vp_start(m: &impl Mmio, base: usize, t: &VpTiming) {
    let port = t.port;
    let v = vp::base(port.id());
    wr(m, base, ovl::vp_bg_mix_ctrl(port.id()), t.bg_mix_ctrl);
    wr(m, base, v + vp::PRE_SCAN_HTIMING, t.pre_scan_htiming);
    wr(m, base, v + vp::POST_DSP_HACT_INFO, t.post_dsp_hact_info);
    wr(m, base, v + vp::POST_DSP_VACT_INFO, t.post_dsp_vact_info);
    wr(m, base, v + vp::POST_SCL_FACTOR_YRGB, t.post_scl_factor_yrgb);
    wr(m, base, v + vp::POST_SCL_CTRL, t.post_scl_ctrl);
    wr(m, base, v + vp::DSP_BG, 0);
    cfg_done(m, base, port);
    wr(m, base, v + vp::DSP_CTRL, t.dsp_ctrl);
}

/// `OVL_LAYER_SEL` with `win` moved to layer `layer` and the window found
/// there moved to `win`'s old layer (fact 11). A window found on no layer
/// leaves the others where they are. `None` for a layer past the eighth
/// (the RK3588 has eight windows, r:2529).
pub const fn layer_sel_with(found: u32, layer: u32, win: Esmart) -> Option<u32> {
    if layer >= 8 {
        return None;
    }
    let id = win.layer_sel_id();
    let mut old_layer = 0;
    while old_layer < 8 && (found >> (4 * old_layer)) & 0xf != id {
        old_layer += 1;
    }
    let displaced = (found >> (4 * layer)) & 0xf;
    let mut sel = found & !(0x7 << (4 * layer)) | id << (4 * layer);
    if old_layer < 8 && old_layer != layer {
        sel = sel & !(0x7 << (4 * old_layer)) | displaced << (4 * old_layer);
    }
    Some(sel)
}

/// `OVL_PORT_SEL` with `win` as the only layer of `port` (0-2) and no layer
/// on the other ports (fact 11).
pub const fn port_sel_with(found: u32, port: Vp, win: Esmart) -> u32 {
    let mut sel = found & ovl::PORT_SEL_WINDOWS;
    sel = sel & !win.port_sel_field() | field(win.port_sel_field(), port.id() as u32);
    let mut p = 0;
    while p < 3 {
        let mux = if p == port.id() { 0 } else { ovl::MUX_NO_LAYER };
        sel |= mux << (4 * p);
        p += 1;
    }
    sel | ovl::PORT3_MUX_RK3588 << 12
}

/// What [`overlay_single`] wrote.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Overlay {
    pub layer_sel: u32,
    pub port_sel: u32,
    /// µs until `OVL_PORT_SEL` read the new value (0 when it was unchanged).
    pub waited_us: u32,
}

/// Make `win` the only, bottom layer of a running `port` (0-2; facts 11,
/// 12): `OVL_CTRL`, `OVL_PORT_SEL` with its `REG_CFG_DONE` and the wait for
/// it to take, `OVL_LAYER_SEL`, the mixer and the delays. The layer select
/// takes effect with the next [`cfg_done`].
pub fn overlay_single(
    m: &impl Mmio,
    base: usize,
    port: Vp,
    win: Esmart,
    delay_us: &mut impl FnMut(u32),
) -> Result<Overlay, Refused> {
    if port == Vp::Vp3 {
        return Err(refuse(Refusal::BadPort));
    }
    let ctrl_found = rd(m, base, ovl::CTRL);
    let port_found = rd(m, base, ovl::PORT_SEL);
    let layer_found = rd(m, base, ovl::LAYER_SEL);
    let port_sel = port_sel_with(port_found, port, win);
    let Some(layer_sel) = layer_sel_with(layer_found, 0, win) else {
        return Err(refuse(Refusal::BadPort));
    };

    let mut ctrl = ctrl_found & !(ovl::CTRL_REGDONE_IMD | ovl::CTRL_REGDONE_SEL | 1 << port.id());
    if layer_sel != layer_found || port_sel != port_found {
        ctrl |= field(ovl::CTRL_REGDONE_SEL, port.id() as u32);
    }
    wr(m, base, ovl::CTRL, ctrl);

    let mut waited_us = 0;
    if port_sel != port_found {
        wr(m, base, ovl::PORT_SEL, port_sel);
        cfg_done(m, base, port);
        loop {
            let now = rd(m, base, ovl::PORT_SEL);
            if now == port_sel {
                break;
            }
            if waited_us >= FRAME_WAIT_US {
                return Err(Refused { why: Refusal::PortSelTimeout, read_back: now });
            }
            delay_us(POLL_STEP_US);
            waited_us += POLL_STEP_US;
        }
    }
    wr(m, base, ovl::LAYER_SEL, layer_sel);
    if port == Vp::Vp0 {
        wr(m, base, ovl::HDR0_SRC_COLOR_CTRL, 0);
    }
    wr(m, base, ovl::CLUSTER_DLY_NUM, 0);
    wr(m, base, ovl::SMART_DLY_NUM, field(win.dly_field(), ESMART_DLY));
    Ok(Overlay { layer_sel, port_sel, waited_us })
}

/// Pixel formats a window takes here.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    /// DRM XRGB8888: bytes B, G, R, X in memory.
    Xrgb8888,
}

impl Format {
    pub const fn bytes_per_pixel(self) -> u32 {
        4
    }
    /// (format code, R/B swap).
    pub const fn code(self) -> (u32, bool) {
        (esmart::FMT_ARGB8888, false)
    }
}

/// A linear frame buffer shown unscaled at (`x`, `y`) of the active area.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Window {
    /// Physical address of the first pixel (the IOMMU is off).
    pub addr: u64,
    pub width: u32,
    pub height: u32,
    pub stride_bytes: u32,
    pub x: u32,
    pub y: u32,
    pub format: Format,
}

/// The largest active area a window is placed in: the RK3588's largest
/// input, 4096x4320 (r:2525); every port's output is smaller (fact 6).
pub const MAX_ACTIVE: (u32, u32) = (4096, 4320);

/// The alignment of a window's address (the module text).
pub const WINDOW_ALIGN: u64 = 64;

/// Whether `w` can be shown in an active area of `active_w` x `active_h`
/// (the module text): inside it, on 64 bytes, a stride of whole 32-bit words
/// that holds a line and fits 16 bits, and every byte below 4 GiB. The
/// active area itself is at most [`MAX_ACTIVE`], so every size and position
/// word fits its 16-bit half; the end address is computed without wrapping.
pub fn window_check(w: &Window, active_w: u32, active_h: u32) -> Result<(), Refused> {
    let line = u64::from(w.width) * u64::from(w.format.bytes_per_pixel());
    if active_w > MAX_ACTIVE.0
        || active_h > MAX_ACTIVE.1
        || w.width == 0
        || w.height == 0
        || u64::from(w.x) + u64::from(w.width) > u64::from(active_w)
        || u64::from(w.y) + u64::from(w.height) > u64::from(active_h)
    {
        return Err(refuse(Refusal::WindowOutside));
    }
    if !w.addr.is_multiple_of(WINDOW_ALIGN) || !w.stride_bytes.is_multiple_of(4) {
        return Err(refuse(Refusal::WindowUnaligned));
    }
    if w.stride_bytes / 4 > esmart::VIR_YRGB {
        return Err(refuse(Refusal::StrideTooWide));
    }
    if u64::from(w.stride_bytes) < line {
        return Err(refuse(Refusal::StrideTooNarrow));
    }
    let end = u64::from(w.stride_bytes)
        .checked_mul(u64::from(w.height - 1))
        .and_then(|rows| rows.checked_add(line))
        .and_then(|span| span.checked_add(w.addr));
    match end {
        Some(e) if e <= 1 << 32 => Ok(()),
        _ => Err(refuse(Refusal::WindowAbove4G)),
    }
}

/// Program Esmart `win` for `w` and enable it (facts 13, 14), with enable
/// written last; [`window_check`] first, so a refused window writes nothing.
/// Nothing takes effect before the port's [`cfg_done`].
pub fn window_set(m: &impl Mmio, base: usize, win: Esmart, w: &Window, active: (u32, u32)) -> Result<(), Refused> {
    window_check(w, active.0, active.1)?;
    let b = base + win.base();
    let (bus, yrgb_id, uv_id) = win.axi();
    let (fmt, rb_swap) = w.format.code();
    modify(m, b, esmart::AXI_CTRL, esmart::AXI_CTRL_BUS_ID, field(esmart::AXI_CTRL_BUS_ID, bus));
    modify(
        m,
        b,
        esmart::CTRL1,
        esmart::CTRL1_YRGB_R_ID | esmart::CTRL1_UV_R_ID | esmart::CTRL1_YMIRROR,
        field(esmart::CTRL1_YRGB_R_ID, yrgb_id) | field(esmart::CTRL1_UV_R_ID, uv_id),
    );
    modify(m, b, esmart::REGION0_VIR, esmart::VIR_YRGB, w.stride_bytes / 4);
    wr(m, b, esmart::REGION0_YRGB_MST, w.addr as u32);
    wr(m, b, esmart::REGION0_SCL_FACTOR_YRGB, 0);
    modify(m, b, esmart::REGION0_SCL_CTRL, esmart::SCL_CTRL_YRGB, esmart::SCL_CTRL_UNSCALED);
    modify(m, b, esmart::COLOR_KEY_CTRL, esmart::COLOR_KEY_EN, 0);
    let size = (w.height - 1) << 16 | (w.width - 1);
    wr(m, b, esmart::REGION0_ACT_INFO, size);
    wr(m, b, esmart::REGION0_DSP_INFO, size);
    wr(m, b, esmart::REGION0_DSP_ST, w.y << 16 | w.x);
    modify(m, b, esmart::CTRL0, esmart::CTRL0_CSC, 0);
    let r0_mask = esmart::R0_ENABLE
        | esmart::R0_FORMAT
        | esmart::R0_VSD_GT2
        | esmart::R0_VSD_GT4
        | esmart::R0_DITHER_UP
        | esmart::R0_RB_SWAP
        | esmart::R0_UV_SWAP;
    let r0 = esmart::R0_ENABLE | field(esmart::R0_FORMAT, fmt) | if rb_swap { esmart::R0_RB_SWAP } else { 0 };
    modify(m, b, esmart::REGION0_CTRL, r0_mask, r0);
    Ok(())
}

/// [`window_set`], then [`cfg_done`] for `port`: the window is shown from
/// the port's next frame.
pub fn window_show(
    m: &impl Mmio,
    base: usize,
    port: Vp,
    win: Esmart,
    w: &Window,
    active: (u32, u32),
) -> Result<(), Refused> {
    window_set(m, base, win, w, active)?;
    cfg_done(m, base, port);
    Ok(())
}

/// Point a shown window at another buffer of the same geometry (a flip):
/// `REGION0_YRGB_MST`, then [`cfg_done`]. Checked as [`window_check`].
pub fn window_flip(
    m: &impl Mmio,
    base: usize,
    port: Vp,
    win: Esmart,
    w: &Window,
    active: (u32, u32),
) -> Result<(), Refused> {
    window_check(w, active.0, active.1)?;
    wr(m, base + win.base(), esmart::REGION0_YRGB_MST, w.addr as u32);
    cfg_done(m, base, port);
    Ok(())
}

/// Disable a window (fact 15); it goes at the port's next [`cfg_done`].
pub fn window_disable(m: &impl Mmio, base: usize, win: Esmart) {
    modify(m, base + win.base(), esmart::REGION0_CTRL, esmart::R0_ENABLE | esmart::R0_YUV_CLIP, 0);
}

/// Disable a Cluster window (fact 17): `WIN0_EN` and YUV clip, then
/// `CLUSTER_ENABLE`; it goes at the port's next [`cfg_done`].
pub fn cluster_disable(m: &impl Mmio, base: usize, win: Cluster) {
    let b = base + win.base();
    modify(m, b, cluster::WIN_CTRL0, cluster::WIN_CTRL0_EN | cluster::WIN_CTRL0_YUV_CLIP, 0);
    modify(m, b, cluster::CTRL, cluster::CTRL_CLUSTER_EN, 0);
}

/// Which windows [`windows_off`] disabled: bit n Cluster n, bit 4 + n Esmart n.
pub type WindowMask = u32;

/// Disable every window whose `OVL_PORT_SEL` field names a port in `ports`
/// (bit n = VPn), except `keep` (fact 17). Nothing takes effect before each
/// port's [`cfg_done`]. Returns the windows disabled.
pub fn windows_off(m: &impl Mmio, base: usize, ports: u32, keep: Option<Esmart>) -> WindowMask {
    let sel = rd(m, base, ovl::PORT_SEL);
    let on = |f: u32| ports & 1 << ((sel & f) >> f.trailing_zeros()) != 0;
    let mut off = 0;
    for (i, c) in Cluster::ALL.iter().enumerate() {
        if on(c.port_sel_field()) {
            cluster_disable(m, base, *c);
            off |= 1 << i;
        }
    }
    for (i, e) in Esmart::ALL.iter().enumerate() {
        if Some(*e) != keep && on(e.port_sel_field()) {
            window_disable(m, base, *e);
            off |= 1 << (4 + i);
        }
    }
    off
}

/// Clear `bits` of `port` and check they read 0 (review S1): a bit that
/// stays set is [`Refusal::EventNotCleared`], never a new event.
fn clear_checked(m: &impl Mmio, base: usize, port: Vp, bits: u32) -> Result<(), Refused> {
    vp_clear(m, base, port, bits);
    let raw = vp_raw_status(m, base, port);
    if raw & bits != 0 {
        return Err(Refused { why: Refusal::EventNotCleared, read_back: raw });
    }
    Ok(())
}

/// Wait for the next frame start of `port`: cleared and checked first, then
/// polled for at most [`FRAME_WAIT_US`]. Returns the µs waited.
pub fn wait_frame(m: &impl Mmio, base: usize, port: Vp, delay_us: &mut impl FnMut(u32)) -> Result<u32, Refused> {
    clear_checked(m, base, port, sys::INT_FS)?;
    let mut waited = 0;
    loop {
        let raw = vp_raw_status(m, base, port);
        if raw & sys::INT_FS != 0 {
            return Ok(waited);
        }
        if waited >= FRAME_WAIT_US {
            return Err(Refused { why: Refusal::FrameTimeout, read_back: raw });
        }
        delay_us(POLL_STEP_US);
        waited += POLL_STEP_US;
    }
}

/// The first step of taking the VOP over from the system before us
/// (fact 17, the module text's take-over order): every window on `ports`
/// except `keep` disabled, `REG_CFG_DONE` for each of those ports, then one
/// frame start on each, after which no disabled window fetches any more.
/// Only then may the IOMMU be switched off (`rk_iommu::paging_off`) and the
/// overlay changed ([`overlay_single`]).
pub fn take_over(
    m: &impl Mmio,
    base: usize,
    ports: &[Vp],
    keep: Option<Esmart>,
    delay_us: &mut impl FnMut(u32),
) -> Result<WindowMask, Refused> {
    let mask = ports.iter().fold(0, |a, p| a | 1 << p.id());
    let off = windows_off(m, base, mask, keep);
    for p in ports {
        cfg_done(m, base, *p);
    }
    // A frame start after CFG_DONE: the update was taken at that boundary.
    for p in ports {
        wait_frame(m, base, *p, delay_us)?;
    }
    Ok(off)
}

/// `REGION0_YRGB_MST` of a window.
pub fn window_address(m: &impl Mmio, base: usize, win: Esmart) -> u32 {
    rd(m, base + win.base(), esmart::REGION0_YRGB_MST)
}

/// Put `port` in STANDBY and wait for DSP_HOLD_VALID (fact 16): clear the
/// event, write `DSP_CTRL` = STANDBY, poll `VP_INT_RAW_STATUS` for at most
/// [`FRAME_WAIT_US`]. Returns the µs waited; on a timeout the bus clocks must
/// stay on (the caller's rule). A DSP_HOLD_VALID that the clear does not
/// clear is [`Refusal::EventNotCleared`] and STANDBY is not written. A port
/// already in STANDBY raises no new DSP_HOLD_VALID and times out: the caller
/// reads `DSP_CTRL` first.
pub fn standby(m: &impl Mmio, base: usize, port: Vp, delay_us: &mut impl FnMut(u32)) -> Result<u32, Refused> {
    let hold = sys::INT_DSP_HOLD_VALID;
    clear_checked(m, base, port, hold)?;
    wr(m, base, vp::base(port.id()) + vp::DSP_CTRL, vp::DSP_CTRL_STANDBY);
    let mut waited = 0;
    loop {
        let raw = rd(m, base, sys::vp_int_raw_status(port.id()));
        if raw & hold != 0 {
            return Ok(waited);
        }
        if waited >= FRAME_WAIT_US {
            return Err(Refused { why: Refusal::StandbyTimeout, read_back: raw });
        }
        delay_us(POLL_STEP_US);
        waited += POLL_STEP_US;
    }
}

/// `VP_INT_RAW_STATUS` of a port (events whether enabled or not).
pub fn vp_raw_status(m: &impl Mmio, base: usize, port: Vp) -> u32 {
    rd(m, base, sys::vp_int_raw_status(port.id()))
}

/// Clear events `bits` of a port (`VP_INT_CLR`, the bits and their mask).
pub fn vp_clear(m: &impl Mmio, base: usize, port: Vp, bits: u32) {
    wr(m, base, sys::vp_int_clr(port.id()), (bits & 0xffff) << 16 | (bits & 0xffff));
}

/// What [`frame_check`] saw over its wait (nanopc-t6-display.md section 5).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrameCheck {
    /// A frame started: the port's timing runs on a pixel clock.
    pub frame_start: bool,
    /// POST_BUF_EMPTY: the window's fetch fell behind.
    pub underflow: bool,
    /// `SYS0_INT_STATUS` and `SYS1_INT_STATUS` bus error bits.
    pub bus_error: [bool; 2],
    /// `VP_INT_RAW_STATUS` at the end.
    pub raw: u32,
}

/// Clear frame start and POST_BUF_EMPTY of `port`, wait `wait_us` (more than
/// one frame), then read them and the two bus error bits back. Bits that
/// the clear does not clear are [`Refusal::EventNotCleared`].
pub fn frame_check(
    m: &impl Mmio,
    base: usize,
    port: Vp,
    wait_us: u32,
    delay_us: &mut impl FnMut(u32),
) -> Result<FrameCheck, Refused> {
    clear_checked(m, base, port, sys::INT_FS | sys::INT_POST_BUF_EMPTY)?;
    delay_us(wait_us);
    let raw = vp_raw_status(m, base, port);
    let s0 = rd(m, base, sys::SYS0_INT_STATUS);
    let s1 = rd(m, base, sys::SYS1_INT_STATUS);
    Ok(FrameCheck {
        frame_start: raw & sys::INT_FS != 0,
        underflow: raw & sys::INT_POST_BUF_EMPTY != 0,
        bus_error: [s0 & sys::INT_BUS_ERROR != 0, s1 & sys::INT_BUS_ERROR != 0],
        raw,
    })
}

/// The registers the board prints after a bring-up (section 5): read only.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReadBack {
    pub version: u32,
    pub pd_ctrl: u32,
    pub cfg_done: u32,
    pub dsp_if_en: u32,
    pub dsp_ctrl: u32,
    pub htotal_hs_end: u32,
    pub vtotal_vs_end: u32,
    pub port_sel: u32,
    pub layer_sel: u32,
    pub region0_ctrl: u32,
    pub yrgb_mst: u32,
    pub vir: u32,
    pub raw_status: u32,
    pub sys0_int_status: u32,
    pub sys1_int_status: u32,
}

pub fn read_back(m: &impl Mmio, base: usize, port: Vp, win: Esmart) -> ReadBack {
    let v = vp::base(port.id());
    let b = win.base();
    ReadBack {
        version: version(m, base),
        pd_ctrl: rd(m, base, sys::PD_CTRL),
        cfg_done: rd(m, base, sys::REG_CFG_DONE),
        dsp_if_en: rd(m, base, sys::DSP_IF_EN),
        dsp_ctrl: rd(m, base, v + vp::DSP_CTRL),
        htotal_hs_end: rd(m, base, v + vp::DSP_HTOTAL_HS_END),
        vtotal_vs_end: rd(m, base, v + vp::DSP_VTOTAL_VS_END),
        port_sel: rd(m, base, ovl::PORT_SEL),
        layer_sel: rd(m, base, ovl::LAYER_SEL),
        region0_ctrl: rd(m, base, b + esmart::REGION0_CTRL),
        yrgb_mst: rd(m, base, b + esmart::REGION0_YRGB_MST),
        vir: rd(m, base, b + esmart::REGION0_VIR),
        raw_status: vp_raw_status(m, base, port),
        sys0_int_status: rd(m, base, sys::SYS0_INT_STATUS),
        sys1_int_status: rd(m, base, sys::SYS1_INT_STATUS),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::BTreeMap;

    const BASE: usize = 0xfdd9_0000;

    /// CEA-861 VIC 16: 1920x1080 at 60 Hz, 148.5 MHz, H 1920 88 44 148 (+),
    /// V 1080 4 5 36 (+).
    const CEA_1080P60: VideoMode = VideoMode {
        pixel_clock_hz: 148_500_000,
        hactive: 1920,
        hfront: 88,
        hsync: 44,
        hback: 148,
        vactive: 1080,
        vfront: 4,
        vsync: 5,
        vback: 36,
        hsync_high: true,
        vsync_high: true,
        de_high: true,
    };

    /// CEA-861 VIC 4: 1280x720 at 60 Hz, 74.25 MHz, H 1280 110 40 220 (+),
    /// V 720 5 5 20 (+).
    const CEA_720P60: VideoMode = VideoMode {
        pixel_clock_hz: 74_250_000,
        hactive: 1280,
        hfront: 110,
        hsync: 40,
        hback: 220,
        vactive: 720,
        vfront: 5,
        vsync: 5,
        vback: 20,
        hsync_high: true,
        vsync_high: true,
        de_high: true,
    };

    // Offsets restated from the module text's facts, not shared with the
    // driver, so a wrong constant cannot agree with itself.
    const CFG_DONE: usize = 0x000;
    const VERSION: usize = 0x004;
    const PD_CTRL: usize = 0x034;
    const SYS0_INT_STATUS: usize = 0x088;
    const SYS1_INT_STATUS: usize = 0x098;
    const VP0_INT_CLR: usize = 0x0a4;
    const VP0_INT_RAW: usize = 0x0ac;
    const PORT_SEL: usize = 0x608;
    const VP0_DSP_CTRL: usize = 0xc00;
    const E0: usize = 0x1800;
    const E0_REGION0_CTRL: usize = 0x1810;
    const E0_MST: usize = 0x1814;

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum Op {
        R(usize, u32),
        W(usize, u32),
        Delay(u32),
    }

    #[derive(Default, Clone, Copy)]
    struct Faults {
        /// No pixel clock: the port never times a frame.
        no_frames: bool,
        /// STANDBY never raises DSP_HOLD_VALID.
        no_dsp_hold: bool,
        /// The fetch underflows every frame.
        underflow: bool,
        /// The AXI fetch fails (SYS0 bus error, when enabled).
        bus_error: bool,
        /// `VP_INT_CLR` does not clear the raw bits (review S1).
        clr_ignored: bool,
    }

    #[derive(Default)]
    struct State {
        regs: BTreeMap<usize, u32>,
        log: Vec<Op>,
        /// `OVL_PORT_SEL` written, taken at VP0's next frame after its CFG_DONE.
        port_sel_written: Option<u32>,
        /// VP0's CFG_DONE is pending.
        pending: bool,
        raw: u32,
        /// STANDBY was written while running: the frame in progress ends it.
        stopping: bool,
        now_us: u64,
        next_frame_us: u64,
    }

    /// VP0 of the VOP2 as the module text describes it: frames every
    /// `frame_us` while `DSP_CTRL` STANDBY is clear and a pixel clock runs;
    /// at a frame start the pending CFG_DONE is taken (its bit clears, the
    /// port select becomes the value in force), frame start is raised, and
    /// STANDBY written while running stops the port and raises DSP_HOLD_VALID.
    /// `VP_INT_CLR` clears the raw bits whose write-enable is set.
    struct Vop {
        s: RefCell<State>,
        frame_us: u64,
        f: Faults,
    }

    impl Vop {
        fn new(f: Faults) -> Vop {
            let v = Vop { s: RefCell::new(State::default()), frame_us: 16_667, f };
            {
                let mut s = v.s.borrow_mut();
                s.regs.insert(VERSION, VERSION_RK3588);
                // after the domain comes up: every internal domain off, VP0
                // in standby, the layer select Linux would read at reset
                // (window ids 0-7 on layers 0-7 is not known; any permutation)
                s.regs.insert(PD_CTRL, 0xef);
                s.regs.insert(VP0_DSP_CTRL, 1 << 31);
                s.regs.insert(0x604, 0x7654_3210);
                s.regs.insert(PORT_SEL, 0x8000_8888);
            }
            v
        }
        fn running(s: &State) -> bool {
            s.stopping || s.regs.get(&VP0_DSP_CTRL).copied().unwrap_or(0) & 1 << 31 == 0
        }
        fn advance(&self, us: u32) {
            let mut s = self.s.borrow_mut();
            s.log.push(Op::Delay(us));
            let end = s.now_us + u64::from(us);
            while !self.f.no_frames && Self::running(&s) && s.next_frame_us <= end {
                s.now_us = s.next_frame_us;
                s.next_frame_us += self.frame_us;
                if s.stopping {
                    s.stopping = false;
                    if !self.f.no_dsp_hold {
                        s.raw |= 1 << 6;
                    }
                    break;
                }
                s.raw |= 1;
                if s.pending {
                    s.pending = false;
                    if let Some(p) = s.port_sel_written.take() {
                        s.regs.insert(PORT_SEL, p);
                    }
                }
                if self.f.underflow {
                    s.raw |= 1 << 4;
                }
                if self.f.bus_error && s.regs.get(&0x080).copied().unwrap_or(0) & 2 != 0 {
                    s.regs.insert(SYS0_INT_STATUS, 2);
                }
            }
            s.now_us = end;
        }
        fn ops(&self) -> Vec<Op> {
            self.s.borrow().log.clone()
        }
        fn writes(&self) -> Vec<(usize, u32)> {
            self.ops()
                .into_iter()
                .filter_map(|o| if let Op::W(a, v) = o { Some((a - BASE, v)) } else { None })
                .collect()
        }
        fn reg(&self, off: usize) -> u32 {
            self.s.borrow().regs.get(&off).copied().unwrap_or(0)
        }
        fn clear_log(&self) {
            self.s.borrow_mut().log.clear();
        }
    }

    impl Mmio for Vop {
        fn read32(&self, addr: usize) -> u32 {
            let off = addr - BASE;
            // the HDR and LUT registers from 0x2000 on are never touched
            assert!(off < 0x2000 && off & 3 == 0, "read outside the VOP's first 8 KiB: {off:#x}");
            let mut s = self.s.borrow_mut();
            let v = match off {
                CFG_DONE => s.regs.get(&CFG_DONE).copied().unwrap_or(0) | u32::from(s.pending),
                VP0_INT_RAW => s.raw,
                _ => s.regs.get(&off).copied().unwrap_or(0),
            };
            s.log.push(Op::R(addr, v));
            v
        }
        fn write32(&self, addr: usize, val: u32) {
            let off = addr - BASE;
            assert!(off < 0x2000 && off & 3 == 0, "write outside the VOP's first 8 KiB: {off:#x}");
            assert_ne!(off, VERSION, "VERSION_INFO is read only");
            let mut s = self.s.borrow_mut();
            s.log.push(Op::W(addr, val));
            match off {
                CFG_DONE => {
                    // bit 15 has no write-enable; bit 0 lands only with bit 16
                    s.regs.insert(CFG_DONE, val & 1 << 15);
                    if val & 1 != 0 && val & 1 << 16 != 0 {
                        s.pending = true;
                    }
                }
                VP0_INT_CLR if !self.f.clr_ignored => s.raw &= !(val & (val >> 16) & 0xffff),
                VP0_INT_CLR => {}
                PORT_SEL => s.port_sel_written = Some(val),
                VP0_DSP_CTRL => {
                    let was = Self::running(&s);
                    s.regs.insert(off, val);
                    if !was && Self::running(&s) {
                        s.next_frame_us = s.now_us + self.frame_us;
                    }
                    if was && val & 1 << 31 != 0 {
                        s.stopping = true;
                    }
                }
                _ => {
                    s.regs.insert(off, val);
                }
            }
        }
        fn read8(&self, _: usize) -> u8 {
            unreachable!("the VOP2 is accessed 32 bits wide")
        }
        fn write8(&self, _: usize, _: u8) {
            unreachable!("the VOP2 is accessed 32 bits wide")
        }
    }

    /// A window filling 1920x1080 at the image window's first 64-byte step
    /// (nanopc-t6-display.md section 4: stride 7680).
    const FB: Window = Window {
        addr: 0x1000_0000,
        width: 1920,
        height: 1080,
        stride_bytes: 7680,
        x: 0,
        y: 0,
        format: Format::Xrgb8888,
    };

    #[test]
    fn the_layout_equals_the_design_notes_section_1() {
        assert_eq!(VERSION_RK3588, 0x40 << 24 | 0x17 << 16 | 0x6786);
        assert_eq!(
            [sys::REG_CFG_DONE, sys::VERSION_INFO, sys::AUTO_GATING_CTRL, sys::DSP_IF_EN, sys::DSP_IF_CTRL],
            [0x000, 0x004, 0x008, 0x028, 0x02c]
        );
        assert_eq!([sys::DSP_IF_POL, sys::PD_CTRL], [0x030, 0x034]);
        assert_eq!([sys::SYS0_INT_STATUS, sys::SYS1_INT_STATUS], [0x088, 0x098]);
        assert_eq!([sys::vp_int_clr(0), sys::vp_int_raw_status(0), sys::vp_int_raw_status(3)], [0xa4, 0xac, 0xdc]);
        assert_eq!([vp::base(0), vp::base(1), vp::base(2), vp::base(3)], [0xc00, 0xd00, 0xe00, 0xf00]);
        assert_eq!([vp::DSP_CTRL, vp::MIPI_CTRL, vp::CLK_CTRL, vp::DSP_BG], [0x00, 0x04, 0x0c, 0x2c]);
        assert_eq!(
            [vp::DSP_HTOTAL_HS_END, vp::DSP_HACT_ST_END, vp::DSP_VTOTAL_VS_END, vp::DSP_VACT_ST_END],
            [0x48, 0x4c, 0x50, 0x54]
        );
        assert_eq!([ovl::CTRL, ovl::LAYER_SEL, ovl::PORT_SEL, ovl::vp_bg_mix_ctrl(0)], [0x600, 0x604, 0x608, 0x6e0]);
        assert_eq!(
            [Esmart::E0.base(), Esmart::E1.base(), Esmart::E2.base(), Esmart::E3.base()],
            [0x1800, 0x1a00, 0x1c00, 0x1e00]
        );
        assert_eq!(
            [esmart::CTRL0, esmart::CTRL1, esmart::AXI_CTRL, esmart::REGION0_CTRL, esmart::REGION0_YRGB_MST],
            [0x00, 0x04, 0x08, 0x10, 0x14]
        );
        assert_eq!(
            [esmart::REGION0_VIR, esmart::REGION0_ACT_INFO, esmart::REGION0_DSP_INFO, esmart::REGION0_DSP_ST],
            [0x1c, 0x20, 0x24, 0x28]
        );
        // Esmart0: layer id 2, AXI bus 0, read ids 0x0a/0x0b
        assert_eq!((Esmart::E0.layer_sel_id(), Esmart::E0.axi()), (2, (0, 0x0a, 0x0b)));
        // REG_CFG_DONE: global bit 15; SYS_PD_CTRL: clusters 0-3 and bit 7
        assert_eq!((sys::CFG_DONE_GLB, sys::PD_CLUSTERS_ESMART), (0x8000, 0x8f));
        assert_eq!(esmart::FMT_ARGB8888, 0);
    }

    #[test]
    fn the_timing_words_of_1080p60() {
        let t = vp_timing(&CEA_1080P60, Vp::Vp0).unwrap();
        // htotal 2200, hsync 44, hact_st 44 + 148 = 192, hact_end 2112
        assert_eq!(t.htotal_hs_end, 2200 << 16 | 44);
        assert_eq!(t.hact_st_end, 192 << 16 | 2112);
        // vtotal 1125, vsync 5, vact_st 5 + 36 = 41, vact_end 1121
        assert_eq!(t.vtotal_vs_end, 1125 << 16 | 5);
        assert_eq!(t.vact_st_end, 41 << 16 | 1121);
        assert_eq!(t.line_flag, 1121 << 16 | 1121);
        assert_eq!(t.bg_mix_ctrl, 54 << 24);
        assert_eq!(t.pre_scan_htiming, (54 + 960 - 1) << 16 | 44);
        assert_eq!((t.post_dsp_hact_info, t.post_dsp_vact_info), (t.hact_st_end, t.vact_st_end));
        assert_eq!((t.post_scl_factor_yrgb, t.post_scl_ctrl), (0x1000_1000, 0));
        assert_eq!(t.dsp_ctrl, 15);
    }

    #[test]
    fn the_timing_words_of_720p60() {
        let t = vp_timing(&CEA_720P60, Vp::Vp0).unwrap();
        // htotal 1650, hact_st 40 + 220 = 260; vtotal 750, vact_st 5 + 20 = 25
        assert_eq!(t.htotal_hs_end, 1650 << 16 | 40);
        assert_eq!(t.hact_st_end, 260 << 16 | 1540);
        assert_eq!(t.vtotal_vs_end, 750 << 16 | 5);
        assert_eq!(t.vact_st_end, 25 << 16 | 745);
        assert_eq!(t.pre_scan_htiming, (54 + 640 - 1) << 16 | 40);
        // VP2's delay is 52, VP3 has no 10-bit output: P888
        assert_eq!(vp_timing(&CEA_720P60, Vp::Vp2).unwrap().bg_mix_ctrl, 52 << 24);
        assert_eq!(vp_timing(&CEA_720P60, Vp::Vp3).unwrap().dsp_ctrl, 0);
    }

    #[test]
    fn an_odd_active_size_is_scaled_down_to_even_as_linux_does() {
        let m = VideoMode { hactive: 1365, vactive: 767, ..CEA_720P60 };
        let t = vp_timing(&m, Vp::Vp0).unwrap();
        assert_eq!(t.post_scl_ctrl, 3);
        assert_eq!(t.post_dsp_hact_info, 260 << 16 | (260 + 1364));
        assert_eq!(t.post_scl_factor_yrgb, (((766 << 12) / 765) << 16) | ((1364 << 12) / 1363));
    }

    #[test]
    fn a_mode_outside_the_ports_limits_or_the_fields_is_refused() {
        for (mode, port) in [
            (VideoMode { hactive: 4097, ..CEA_1080P60 }, Vp::Vp0),
            (VideoMode { vactive: 2305, ..CEA_1080P60 }, Vp::Vp1),
            (VideoMode { hactive: 2560, ..CEA_1080P60 }, Vp::Vp3),
            (VideoMode { hsync: 0, ..CEA_1080P60 }, Vp::Vp0),
            (VideoMode { hfront: 0x10000, ..CEA_1080P60 }, Vp::Vp0),
            (VideoMode { pixel_clock_hz: 0, ..CEA_1080P60 }, Vp::Vp0),
        ] {
            assert_eq!(vp_timing(&mode, port).unwrap_err().why, Refusal::BadMode, "{mode:?} on {port:?}");
        }
        assert!(vp_timing(&VideoMode { hactive: 4096, vactive: 2160, ..CEA_1080P60 }, Vp::Vp0).is_ok());
    }

    #[test]
    fn the_hdmi_interface_dividers_and_the_vop_grf_word() {
        let w = hdmi_if(Vp::Vp0, HdmiPort::Hdmi0);
        assert_eq!(w.clk_ctrl, 2);
        assert_eq!((w.if_en_mask, w.if_en), (1 << 3 | 3 << 16, 1 << 3));
        assert_eq!((w.if_ctrl_mask, w.if_ctrl), (3 << 16 | 1 << 18, 2 << 16 | 1 << 18));
        let w = hdmi_if(Vp::Vp1, HdmiPort::Hdmi1);
        assert_eq!((w.if_en_mask, w.if_en), (1 << 5 | 3 << 18, 1 << 5 | 1 << 18));
        assert_eq!(w.if_ctrl, 2 << 20 | 1 << 22);

        assert_eq!(hdmi_vop_grf(HdmiPort::Hdmi0), 0x0002_0002);
        assert_eq!(hdmi_vop_grf(HdmiPort::Hdmi1), 0x0010_0010);
    }

    #[test]
    fn the_hdmi_route_keeps_the_other_interfaces_bits() {
        let v = Vop::new(Faults::default());
        v.s.borrow_mut().regs.insert(0x028, 1 << 5 | 1 << 18 | 3 << 16);
        v.s.borrow_mut().regs.insert(0x02c, 0xffff_ffff);
        hdmi_route(&v, BASE, Vp::Vp0, HdmiPort::Hdmi0);
        assert_eq!(v.reg(0xc0c), 2);
        assert_eq!(v.reg(0x028), 1 << 5 | 1 << 18 | 1 << 3);
        assert_eq!(v.reg(0x02c), !(1u32 << 16));
        assert_eq!(v.reg(0x030), 1 << 28);
        let order: Vec<usize> = v.writes().iter().map(|w| w.0).collect();
        assert_eq!(order, vec![0xc0c, 0x028, 0x02c, 0x030]);
    }

    #[test]
    fn init_checks_the_version_before_any_write_then_follows_linuxs_order() {
        let v = Vop::new(Faults::default());
        v.s.borrow_mut().regs.insert(VERSION, 0x4015_8023); // the RK3568's
        assert_eq!(init(&v, BASE).unwrap_err(), Refused { why: Refusal::VersionMismatch, read_back: 0x4015_8023 });
        assert!(v.writes().is_empty());

        let v = Vop::new(Faults::default());
        let i = init(&v, BASE).unwrap();
        assert_eq!((i.pd_found, i.pd_now), (0xef, 0x60));
        assert_eq!(
            v.writes(),
            vec![
                (0x034, 0x60),
                (0x000, 0x8000),
                (0x008, 0),
                (0x084, 0x2_0002),
                (0x080, 0x2_0002),
                (0x094, 0x2_0002),
                (0x090, 0x2_0002)
            ]
        );
    }

    #[test]
    fn the_port_starts_with_cfg_done_before_dsp_ctrl_and_dsp_ctrl_last() {
        let v = Vop::new(Faults::default());
        let t = vp_timing(&CEA_1080P60, Vp::Vp0).unwrap();
        vp_set_timing(&v, BASE, &t);
        vp_start(&v, BASE, &t);
        let w = v.writes();
        assert_eq!(
            &w[..6],
            &[
                (0xc48, t.htotal_hs_end),
                (0xc4c, t.hact_st_end),
                (0xc54, t.vact_st_end),
                (0x070, t.line_flag),
                (0xc50, t.vtotal_vs_end),
                (0xc04, 0)
            ]
        );
        let n = w.len();
        assert_eq!(w[n - 2], (0x000, 0x8000 | 1 << 16 | 1));
        assert_eq!(w[n - 1], (0xc00, 15));
        assert_eq!(v.reg(0x6e0), 54 << 24);
        // the port runs: a frame starts within one frame time
        let c = frame_check(&v, BASE, Vp::Vp0, 20_000, &mut |us| v.advance(us)).unwrap();
        assert!(c.frame_start && !c.underflow && c.bus_error == [false, false]);
    }

    #[test]
    fn a_port_without_a_pixel_clock_shows_no_frame_start() {
        let v = Vop::new(Faults { no_frames: true, ..Faults::default() });
        let t = vp_timing(&CEA_1080P60, Vp::Vp0).unwrap();
        vp_set_timing(&v, BASE, &t);
        vp_start(&v, BASE, &t);
        let c = frame_check(&v, BASE, Vp::Vp0, 50_000, &mut |us| v.advance(us)).unwrap();
        assert!(!c.frame_start);
    }

    #[test]
    fn underflow_and_bus_errors_are_read_back() {
        let v = Vop::new(Faults { underflow: true, bus_error: true, ..Faults::default() });
        init(&v, BASE).unwrap();
        let t = vp_timing(&CEA_1080P60, Vp::Vp0).unwrap();
        vp_start(&v, BASE, &t);
        let c = frame_check(&v, BASE, Vp::Vp0, 40_000, &mut |us| v.advance(us)).unwrap();
        assert!(c.frame_start && c.underflow);
        assert_eq!(c.bus_error, [true, false]);
        let r = read_back(&v, BASE, Vp::Vp0, Esmart::E0);
        assert_eq!((r.sys0_int_status, r.sys1_int_status, r.dsp_ctrl, r.version), (2, 0, 15, VERSION_RK3588));
        assert_eq!((v.reg(SYS0_INT_STATUS), v.reg(SYS1_INT_STATUS)), (2, 0));
    }

    fn running_vop() -> Vop {
        let v = Vop::new(Faults::default());
        init(&v, BASE).unwrap();
        let t = vp_timing(&CEA_1080P60, Vp::Vp0).unwrap();
        vp_set_timing(&v, BASE, &t);
        vp_start(&v, BASE, &t);
        v.advance(20_000);
        v.clear_log();
        v
    }

    #[test]
    fn a_window_is_refused_above_4g_unaligned_or_with_a_stride_over_16_bits_before_any_write() {
        let active = (1920, 1080);
        for (w, why) in [
            (Window { addr: 0x1_0000_0000, ..FB }, Refusal::WindowAbove4G),
            // the last line ends past 4 GiB
            (Window { addr: 0x1_0000_0000 - 7680 * 1079, ..FB }, Refusal::WindowAbove4G),
            (Window { addr: 0x1000_0020, ..FB }, Refusal::WindowUnaligned),
            (Window { stride_bytes: 7682, ..FB }, Refusal::WindowUnaligned),
            (Window { width: 16, height: 16, stride_bytes: 0x4_0000, ..FB }, Refusal::StrideTooWide),
            (Window { stride_bytes: 7676, ..FB }, Refusal::StrideTooNarrow),
            (Window { x: 1, ..FB }, Refusal::WindowOutside),
            (Window { height: 0, ..FB }, Refusal::WindowOutside),
        ] {
            let v = running_vop();
            let r = window_show(&v, BASE, Vp::Vp0, Esmart::E0, &w, active);
            assert_eq!(r.unwrap_err().why, why, "{w:?}");
            assert!(v.writes().is_empty(), "{w:?}: {:?}", v.writes());
        }
        // the largest stride that fits and a window whose last byte is the
        // last below 4 GiB are taken
        let v = running_vop();
        let w = Window { width: 16, height: 16, stride_bytes: 0xffff * 4, ..FB };
        assert!(window_show(&v, BASE, Vp::Vp0, Esmart::E0, &w, active).is_ok());
        let w = Window { addr: (1 << 32) - 7680 * 1080, ..FB };
        assert!(window_check(&w, 1920, 1080).is_ok());
    }

    #[test]
    fn the_window_is_written_enable_last_then_cfg_done() {
        let v = running_vop();
        window_show(&v, BASE, Vp::Vp0, Esmart::E0, &FB, (1920, 1080)).unwrap();
        let w = v.writes();
        let n = w.len();
        assert_eq!(w[n - 1], (CFG_DONE, 0x8000 | 1 << 16 | 1), "CFG_DONE is the last write");
        assert_eq!(w[n - 2].0, E0_REGION0_CTRL, "enable is the last window write");
        assert!(w[..n - 1].iter().all(|(a, _)| (E0..E0 + 0x100).contains(a)));
        assert_eq!(v.reg(E0_REGION0_CTRL), 1);
        assert_eq!(v.reg(E0_MST), 0x1000_0000);
        assert_eq!(v.reg(E0 + 0x1c), 1920);
        assert_eq!(v.reg(E0 + 0x20), 1079 << 16 | 1919);
        assert_eq!(v.reg(E0 + 0x24), 1079 << 16 | 1919);
        assert_eq!(v.reg(E0 + 0x28), 0);
        assert_eq!(v.reg(E0 + 0x04), 0x0a << 4 | 0x0b << 12);
        assert_eq!(v.reg(E0 + 0x08), 0);
        assert_eq!(v.reg(E0 + 0x30), 0x44);
        // the update is pending until the port's next frame takes it
        assert!(cfg_pending(&v, BASE, Vp::Vp0));
        v.advance(17_000);
        assert!(!cfg_pending(&v, BASE, Vp::Vp0));
        assert_eq!(window_address(&v, BASE, Esmart::E0), 0x1000_0000);
    }

    #[test]
    fn a_flip_writes_only_the_address_then_cfg_done() {
        let v = running_vop();
        let next = Window { addr: 0x1080_0000, ..FB };
        window_flip(&v, BASE, Vp::Vp0, Esmart::E0, &next, (1920, 1080)).unwrap();
        assert_eq!(v.writes(), vec![(E0_MST, 0x1080_0000), (CFG_DONE, 0x8000 | 1 << 16 | 1)]);
        let v = running_vop();
        let r = window_flip(&v, BASE, Vp::Vp0, Esmart::E0, &Window { addr: 0x1_0000_0000, ..FB }, (1920, 1080));
        assert_eq!(r.unwrap_err().why, Refusal::WindowAbove4G);
        assert!(v.writes().is_empty());
    }

    #[test]
    fn disabling_a_window_clears_enable_and_yuv_clip_only() {
        let v = running_vop();
        v.s.borrow_mut().regs.insert(E0_REGION0_CTRL, 1 | 1 << 17 | 1 << 14);
        window_disable(&v, BASE, Esmart::E0);
        assert_eq!(v.reg(E0_REGION0_CTRL), 1 << 14);
    }

    #[test]
    fn the_layer_select_swaps_esmart0_onto_layer_0() {
        // ids 0-7 on layers 0-7: Esmart0 (2) on layer 2 trades with layer 0
        assert_eq!(layer_sel_with(0x7654_3210, 0, Esmart::E0), Some(0x7654_3012));
        // already there: unchanged
        assert_eq!(layer_sel_with(0x7654_3012, 0, Esmart::E0), Some(0x7654_3012));
        // Linux clears three bits of the nibble (r:2229): bit 3 stays
        assert_eq!(layer_sel_with(0x0000_0028, 0, Esmart::E0), Some(0x0000_008a));
        // not on any layer: only layer 0 changes
        assert_eq!(layer_sel_with(0x1111_1111, 0, Esmart::E0), Some(0x1111_1112));
    }

    #[test]
    fn the_port_select_puts_esmart0_alone_on_vp0() {
        // windows' port fields kept, Esmart0's set to 0; PORT0_MUX 0,
        // PORT1/2_MUX 8 (no layer), PORT3_MUX 7
        assert_eq!(port_sel_with(0x8300_8888, Vp::Vp0, Esmart::E0), 0x8000_7880);
        assert_eq!(port_sel_with(0, Vp::Vp1, Esmart::E0), 0x0100_7808);
    }

    #[test]
    fn the_overlay_waits_for_the_port_select_then_writes_the_layer_select() {
        let v = running_vop();
        let o = overlay_single(&v, BASE, Vp::Vp0, Esmart::E0, &mut |us| v.advance(us)).unwrap();
        assert_eq!((o.layer_sel, o.port_sel), (0x7654_3012, 0x8000_7880));
        assert!(o.waited_us > 0 && o.waited_us <= 16_800, "{}", o.waited_us);
        let w = v.writes();
        let at = |off| w.iter().position(|x| x.0 == off).unwrap();
        assert_eq!(w[0], (0x600, 0));
        assert!(at(PORT_SEL) < at(CFG_DONE) && at(CFG_DONE) < at(0x604));
        assert_eq!(v.reg(0x604), 0x7654_3012);
        assert_eq!((v.reg(0x6c0), v.reg(0x6f0), v.reg(0x6f8)), (0, 0, 23));
        // a second call changes nothing and does not wait
        v.clear_log();
        let o = overlay_single(&v, BASE, Vp::Vp0, Esmart::E0, &mut |us| v.advance(us)).unwrap();
        assert_eq!(o.waited_us, 0);
        assert!(v.writes().iter().all(|w| w.0 != PORT_SEL && w.0 != CFG_DONE));
    }

    #[test]
    fn the_overlay_on_a_port_that_does_not_run_times_out_by_name() {
        let v = Vop::new(Faults::default());
        let r = overlay_single(&v, BASE, Vp::Vp0, Esmart::E0, &mut |us| v.advance(us)).unwrap_err();
        assert_eq!(r, Refused { why: Refusal::PortSelTimeout, read_back: 0x8000_8888 });
        assert!(v.writes().iter().all(|w| w.0 != 0x604), "no layer select after the timeout");
        assert_eq!(overlay_single(&v, BASE, Vp::Vp3, Esmart::E0, &mut |_| {}).unwrap_err().why, Refusal::BadPort);
    }

    #[test]
    fn standby_waits_for_dsp_hold() {
        let v = running_vop();
        v.s.borrow_mut().raw = 1 << 6; // a stale event from before, no frame start
        let waited = standby(&v, BASE, Vp::Vp0, &mut |us| v.advance(us)).unwrap();
        // the hold comes at the end of the frame in progress
        assert!(waited > 0 && waited <= 16_800, "{waited}");
        assert_eq!(vp_raw_status(&v, BASE, Vp::Vp0) & 1, 0, "no frame after the hold");
        assert_eq!(v.writes(), vec![(VP0_INT_CLR, 0x40_0040), (VP0_DSP_CTRL, 1 << 31)]);
        // nothing after the hold: the port stays stopped
        v.advance(100_000);
        assert_eq!(vp_raw_status(&v, BASE, Vp::Vp0) & 1, 0);
        // the stale event was cleared before STANDBY: the hold seen is new
        let ops = v.ops();
        let clr = ops.iter().position(|o| matches!(o, Op::W(a, _) if *a == BASE + VP0_INT_CLR)).unwrap();
        let sb = ops.iter().position(|o| matches!(o, Op::W(a, _) if *a == BASE + VP0_DSP_CTRL)).unwrap();
        assert!(clr < sb);
    }

    #[test]
    fn standby_times_out_by_name_when_dsp_hold_never_comes() {
        let v = Vop::new(Faults { no_dsp_hold: true, ..Faults::default() });
        let t = vp_timing(&CEA_1080P60, Vp::Vp0).unwrap();
        vp_start(&v, BASE, &t);
        v.s.borrow_mut().raw |= 1 << 6;
        let r = standby(&v, BASE, Vp::Vp0, &mut |us| v.advance(us)).unwrap_err();
        assert_eq!(r.why, Refusal::StandbyTimeout);
        assert_eq!(r.read_back & 1 << 6, 0);
        let waited: u32 = v.ops().iter().filter_map(|o| if let Op::Delay(d) = o { Some(*d) } else { None }).sum();
        assert_eq!(waited, FRAME_WAIT_US);
    }

    #[test]
    fn the_whole_order_brings_a_picture_up_in_the_model() {
        let v = Vop::new(Faults::default());
        init(&v, BASE).unwrap();
        let t = vp_timing(&CEA_1080P60, Vp::Vp0).unwrap();
        hdmi_route(&v, BASE, Vp::Vp0, HdmiPort::Hdmi0);
        vp_set_timing(&v, BASE, &t);
        vp_start(&v, BASE, &t);
        overlay_single(&v, BASE, Vp::Vp0, Esmart::E0, &mut |us| v.advance(us)).unwrap();
        window_show(&v, BASE, Vp::Vp0, Esmart::E0, &FB, (1920, 1080)).unwrap();
        let c = frame_check(&v, BASE, Vp::Vp0, 40_000, &mut |us| v.advance(us)).unwrap();
        assert!(c.frame_start && !c.underflow);
        let r = read_back(&v, BASE, Vp::Vp0, Esmart::E0);
        assert_eq!(
            (r.yrgb_mst, r.vir, r.region0_ctrl & 1, r.port_sel, r.cfg_done & 1),
            (0x1000_0000, 1920, 1, 0x8000_7880, 0)
        );
        assert_eq!(r.pd_ctrl & 0x8f, 0);
        assert_eq!(r.dsp_if_en, 1 << 3);
    }

    /// Review B1: Linux left its picture on Cluster0 of VP0 (measured on
    /// the board, 2026-11-12) and split the layers four and four over VP0
    /// and VP1 (c:2474-2484: PORT0_MUX 3, PORT1_MUX 7).
    #[test]
    fn taking_over_from_linux_leaves_no_other_window_enabled_on_the_port() {
        let v = running_vop();
        {
            let mut s = v.s.borrow_mut();
            s.regs.insert(0x1000, 1);
            s.regs.insert(0x1100, 1);
            s.regs.insert(0x1010, 0x0040_0000);
            s.regs.insert(0x604, 0x7654_3210);
            s.regs.insert(PORT_SEL, 0x0000_7873);
        }
        // Esmart1 also on VP0 and enabled; Cluster1 enabled on VP1
        v.s.borrow_mut().regs.insert(0x1a10, 1);
        v.s.borrow_mut().regs.insert(0x1200, 1);
        v.s.borrow_mut().regs.insert(0x1300, 1);
        v.s.borrow_mut().regs.insert(PORT_SEL, 0x0004_7873);
        v.clear_log();
        let off = take_over(&v, BASE, &[Vp::Vp0], Some(Esmart::E0), &mut |us| v.advance(us)).unwrap();
        // Cluster0, Cluster2, Cluster3, Esmart1-3 (port field 0); not Cluster1
        // (VP1), not the window kept
        assert_eq!(off, 0b1110_1101);
        assert_eq!((v.reg(0x1000) & 1, v.reg(0x1100) & 1), (0, 0), "Cluster0 still enabled");
        assert_eq!(v.reg(0x1a10) & 1, 0, "Esmart1 still enabled");
        assert_eq!((v.reg(0x1200), v.reg(0x1300)), (1, 1), "a window of VP1 was touched");
        // the disables, then CFG_DONE, then one frame start waited for
        let ops = v.ops();
        let last_disable =
            ops.iter().rposition(|o| matches!(o, Op::W(a, _) if (0x1000..0x1f00).contains(&(a - BASE)))).unwrap();
        let cfg = ops.iter().position(|o| *o == Op::W(BASE + CFG_DONE, 0x8000 | 1 << 16 | 1)).unwrap();
        let delay = ops.iter().position(|o| matches!(o, Op::Delay(_))).unwrap();
        assert!(last_disable < cfg && cfg < delay);
        assert!(!cfg_pending(&v, BASE, Vp::Vp0), "the frame took the disables");

        // then (paging_off, not in this model) the overlay and our window
        overlay_single(&v, BASE, Vp::Vp0, Esmart::E0, &mut |us| v.advance(us)).unwrap();
        window_show(&v, BASE, Vp::Vp0, Esmart::E0, &FB, (1920, 1080)).unwrap();
        v.advance(17_000);
        assert_eq!((v.reg(0x1000) & 1, v.reg(0x1100) & 1), (0, 0));
        assert_eq!(v.reg(E0_REGION0_CTRL) & 1, 1);
    }

    #[test]
    fn the_take_over_of_a_port_that_does_not_run_times_out_by_name() {
        let v = Vop::new(Faults { no_frames: true, ..Faults::default() });
        let t = vp_timing(&CEA_1080P60, Vp::Vp0).unwrap();
        vp_start(&v, BASE, &t);
        let r = take_over(&v, BASE, &[Vp::Vp0], None, &mut |us| v.advance(us)).unwrap_err();
        assert_eq!(r.why, Refusal::FrameTimeout);
    }

    /// Review S1: a clear that does not clear must not pass for a new event.
    #[test]
    fn an_event_the_clear_does_not_clear_is_refused_not_taken_as_new() {
        let v = Vop::new(Faults { clr_ignored: true, ..Faults::default() });
        v.s.borrow_mut().raw = 1 << 6 | 1;
        let r = standby(&v, BASE, Vp::Vp0, &mut |us| v.advance(us)).unwrap_err();
        assert_eq!((r.why, r.read_back), (Refusal::EventNotCleared, 1 << 6 | 1));
        assert!(v.writes().iter().all(|w| w.0 != VP0_DSP_CTRL), "STANDBY written after a failed clear");
        let r = frame_check(&v, BASE, Vp::Vp0, 20_000, &mut |us| v.advance(us)).unwrap_err();
        assert_eq!(r.why, Refusal::EventNotCleared);
        let r = wait_frame(&v, BASE, Vp::Vp0, &mut |us| v.advance(us)).unwrap_err();
        assert_eq!(r.why, Refusal::EventNotCleared);
    }

    /// Review S3: the end address is computed without wrapping.
    #[test]
    fn a_window_whose_end_wraps_is_refused() {
        let w = Window { addr: u64::MAX - 63, width: 16, height: 1, stride_bytes: 64, ..FB };
        assert_eq!(window_check(&w, 1920, 1080).unwrap_err().why, Refusal::WindowAbove4G);
    }

    /// Review S4: every field is bounded before any sum.
    #[test]
    fn a_mode_whose_sums_wrap_is_refused() {
        let m = VideoMode { hsync: 0x8000_0000, hback: 0x8000_0000 + 160, ..CEA_720P60 };
        assert_eq!(vp_timing(&m, Vp::Vp0).unwrap_err().why, Refusal::BadMode);
    }

    /// Review S6: a layer past the eighth and an active area past 16 bits.
    #[test]
    fn a_layer_past_seven_and_a_huge_active_area_are_refused() {
        assert_eq!(layer_sel_with(0x7654_3210, 8, Esmart::E0), None);
        let w = Window { x: 0x1_0000, width: 16, height: 16, ..FB };
        assert_eq!(window_check(&w, 0x2_0000, 1080).unwrap_err().why, Refusal::WindowOutside);
    }

    /// The board's LCD: no hot-plug, no EDID, one fixed mode, 1024x768 at
    /// 60 Hz, 65 MHz, H 1024 1048 1184 1344, V 768 771 777 806, both syncs
    /// negative (VESA DMT; Linux runs VP0 with it, output_mode 0xf).
    const DMT_1024X768_60: VideoMode = VideoMode {
        pixel_clock_hz: 65_000_000,
        hactive: 1024,
        hfront: 24,
        hsync: 136,
        hback: 160,
        vactive: 768,
        vfront: 3,
        vsync: 6,
        vback: 29,
        hsync_high: false,
        vsync_high: false,
        de_high: true,
    };

    #[test]
    fn the_timing_words_of_the_boards_1024x768() {
        let t = vp_timing(&DMT_1024X768_60, Vp::Vp0).unwrap();
        assert_eq!(
            [
                t.htotal_hs_end,
                t.hact_st_end,
                t.vtotal_vs_end,
                t.vact_st_end,
                t.line_flag,
                t.bg_mix_ctrl,
                t.pre_scan_htiming
            ],
            [0x0540_0088, 0x0128_0528, 0x0326_0006, 0x0023_0323, 0x0323_0323, 0x3600_0000, 0x0235_0088]
        );
        assert_eq!((t.dsp_ctrl, t.post_scl_factor_yrgb, t.post_scl_ctrl), (0xf, 0x1000_1000, 0));
    }

    /// Review S5: a window away from (0, 0); DSP_ST is y << 16 | x (c:1387).
    #[test]
    fn a_window_at_x_y_is_placed_there() {
        let v = running_vop();
        let w = Window { width: 640, height: 480, stride_bytes: 2560, x: 16, y: 8, ..FB };
        window_show(&v, BASE, Vp::Vp0, Esmart::E0, &w, (1920, 1080)).unwrap();
        assert_eq!(v.reg(E0 + 0x28), 8 << 16 | 16);
        assert_eq!(v.reg(E0 + 0x24), 479 << 16 | 639);
        // the far corner must stay inside
        let w = Window { x: 1281, ..w };
        assert_eq!(window_check(&w, 1920, 1080).unwrap_err().why, Refusal::WindowOutside);
    }

    /// Review S5: every Esmart's table, restated from r:1199-1266 and
    /// h:734-737, h:757-760; and what window_set and overlay_single write
    /// from it.
    #[test]
    fn the_esmart_tables_are_linuxs() {
        // (window, base, layer id, AXI bus, YRGB id, UV id, port field shift, delay shift)
        let rows = [
            (Esmart::E0, 0x1800, 2, 0, 0x0a, 0x0b, 24, 0),
            (Esmart::E1, 0x1a00, 3, 0, 0x0c, 0x01, 26, 8),
            (Esmart::E2, 0x1c00, 6, 1, 0x0a, 0x0b, 28, 16),
            (Esmart::E3, 0x1e00, 7, 1, 0x0c, 0x0d, 30, 24),
        ];
        for (e, b, id, bus, y, uv, port_shift, dly_shift) in rows {
            assert_eq!((e.base(), e.layer_sel_id(), e.axi()), (b, id, (bus, y, uv)), "{e:?}");
            assert_eq!((e.port_sel_field(), e.dly_field()), (3 << port_shift, 0xff << dly_shift), "{e:?}");
            let v = running_vop();
            window_show(&v, BASE, Vp::Vp0, e, &FB, (1920, 1080)).unwrap();
            assert_eq!(v.reg(b + 0x08), bus << 1, "{e:?} AXI_CTRL");
            assert_eq!(v.reg(b + 0x04), y << 4 | uv << 12, "{e:?} CTRL1");
            overlay_single(&v, BASE, Vp::Vp0, e, &mut |us| v.advance(us)).unwrap();
            assert_eq!(v.reg(0x6f8), 23 << dly_shift, "{e:?} SMART_DLY_NUM");
            assert_eq!(v.reg(PORT_SEL) & 3 << port_shift, 0, "{e:?} on VP0");
            assert_eq!(v.reg(0x604) & 0xf, id, "{e:?} on layer 0");
        }
        let clusters = [
            (Cluster::C0, 0x1000, 16),
            (Cluster::C1, 0x1200, 18),
            (Cluster::C2, 0x1400, 20),
            (Cluster::C3, 0x1600, 22),
        ];
        for (c, b, shift) in clusters {
            assert_eq!((c.base(), c.port_sel_field()), (b, 3 << shift), "{c:?}");
        }
    }

    /// Review S6: the words go to the port they were made for.
    #[test]
    fn timing_words_carry_their_port() {
        let v = Vop::new(Faults::default());
        let t = vp_timing(&CEA_720P60, Vp::Vp3).unwrap();
        assert_eq!((t.port, t.dsp_ctrl), (Vp::Vp3, 0));
        vp_set_timing(&v, BASE, &t);
        vp_start(&v, BASE, &t);
        let w = v.writes();
        assert!(
            w.iter().all(|(a, _)| *a == 0x07c || *a == 0x6ec || *a == CFG_DONE || (0xf00..0x1000).contains(a)),
            "{w:x?}"
        );
        assert_eq!(w.last(), Some(&(0xf00, 0)));
        assert_eq!(w[w.len() - 2], (CFG_DONE, 0x8000 | 1 << 19 | 1 << 3));
    }
}
