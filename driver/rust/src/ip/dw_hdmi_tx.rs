// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The Synopsys DesignWare HDMI TX controller as the i.MX 8M Plus carries it
//! (0x32FD8000): a fixed mode, RGB 4:4:4 at 8 bits per component, sent in
//! DVI mode (no data islands: no InfoFrames, no audio, no HDCP).
//!
//! Written from the i.MX 8M Plus Applications Processor Reference Manual,
//! IMX8MPRM Rev. 3, 08/2024 ("RM"), section 13.9 "HDMI TX Controller", whose
//! register chapter 13.9.3 is the controller's register map (the RM notes it
//! as Synopsys information used with permission); register names and bits
//! from the hal_nxp headers generated into [`crate::soc::imx8mp::regs`]
//! (`identification`, `videosampler`, `videopacketizer`, `framecomposer`,
//! `phyconfiguration`, `maincontroller`). Every register is 8 bits wide at its
//! own byte address, so all accesses here are byte accesses. What the code
//! relies on:
//!
//! - Sub-block offsets from the controller base: Identification 0h,
//!   Interrupt 100h, VideoSampler 200h, VideoPacketizer 800h, FrameComposer
//!   1000h, PHYConfiguration 3000h, MainController 4000h (the "registers base
//!   address" line of 13.9.3.1, .2, .3, .4, .5, .6, .9; 13.9.2.1 and 13.9.2.5
//!   say the same for 200h and 1000h).
//! - `tx_invid0.video_mapping` 0x01 = RGB 4:4:4 / 8 bits, the internal DE
//!   generator off (13.9.3.3.2).
//! - `vp_pr_cd`: colour depth 0000b = 24 bits per pixel, 8-bit packing; no
//!   pixel repetition (13.9.3.4.3). `vp_conf` keeps its reset value 24h
//!   (pixel packing on, which at 24 bpp is the 8-bit pass-through; output
//!   from the packing block) -- the RM marks the bypass fields "deprecated"
//!   and gives no other 24-bit setting (13.9.3.4.6).
//! - Frame composer input timing (13.9.3.5.2-15): `fc_invidconf` polarities
//!   (1 = active high), `DVI_modez` 0 = DVI, `in_I_P` 0 = progressive; active
//!   and blanking sizes, the HSYNC front porch ("delay") and width in pixel
//!   clocks, the VSYNC front porch and width in lines, split low byte /
//!   high bits.
//! - `fc_ctrldur` "minimum of 12 pixel clock cycles", `fc_exctrldur`
//!   "minimum of 32", `fc_exctrlspac` "maximum of 50ms", the spacing being
//!   (1 / TMDS clock) x 256 x 512 x (n + 1) on an HDMI 2.0 controller
//!   (13.9.3.5.19-21; the controller is "compatible with the HDMI v2.0a
//!   spec", 13.9.1).
//! - `fc_prconf` reset 10h: incoming pixel repetition 1, none added
//!   (13.9.3.5.1 memory map).
//! - `mc_flowctrl` 0 = the colour space converter bypassed (13.9.3.9.4);
//!   `mc_clkdis` bits 5..0 disable the CEC, CSC, audio, pixel-repetition,
//!   TMDS and pixel clock domains, reset all enabled (13.9.3.9.2).
//! - `mc_swrstzreq_1`: writing 0 to a bit requests that domain's reset; the
//!   TMDS one "is required to perform a write action on one of" the frame
//!   composer timing registers (13.9.3.9.3), so `fc_invidconf` is written
//!   again after it.
//! - `fc_dbgforce.forcevideo` makes the controller send fixed video instead
//!   of its input (13.9.3.5.1 memory map: "allows to force the controller to
//!   output audio and video data the values configured"): a staging step
//!   that needs no pixel stream from the LCDIF.
//! - Status: `phy_stat0` TX_PHY_LOCK / HPD / RX_SENSE (13.9.3.6.6);
//!   `mc_lockonclock_1` pclk / tclk "the clock is present", write 1 to clear
//!   (13.9.3.9.6). The interrupt mute `ih_mute` resets to 03h, all muted
//!   (13.9.3.2.21); this driver polls and leaves it so.
//!
//! Not stated in IMX8MPRM Rev. 3: what `phy_conf0` (13.9.3.6.2, "refer to the
//! DesignWare Cores HDMI TX PHY Databook") and `mc_phyrstz` ("PHY GEN1 /
//! GEN2", 13.9.3.9.5) drive on this SoC, whose PHY is not a DesignWare one.
//! They are written only with values the caller passes ([`PhyIf`]), which come
//! from the board dump; the default leaves both at their reset values.
//! The preamble filters `fc_ch0..2pream` have no stated values and are not
//! written.

use crate::errno::EINVAL;
use crate::mmio::{field, get, Mmio};
use crate::soc::imx8mp::regs::{framecomposer as fc, maincontroller as mc, phyconfiguration as phy};
use crate::soc::imx8mp::regs::{identification as id, videopacketizer as vp, videosampler as vs};
use crate::video::VideoMode;

/// Sub-block offsets from the controller base (see the module text).
pub const IDENTIFICATION: u32 = 0x0000;
pub const INTERRUPT: u32 = 0x0100;
pub const VIDEOSAMPLER: u32 = 0x0200;
pub const VIDEOPACKETIZER: u32 = 0x0800;
pub const FRAMECOMPOSER: u32 = 0x1000;
pub const PHYCONFIGURATION: u32 = 0x3000;
pub const MAINCONTROLLER: u32 = 0x4000;

/// `ih_mute` (Interrupt block + FFh, 13.9.3.2.21) and its reset value.
pub const IH_MUTE: u32 = INTERRUPT + 0xff;
pub const IH_MUTE_RESET: u8 = 0x03;

const VIDEO_MAPPING_RGB444_8: u32 = 0x01;
const VP_CONF_RESET: u32 = 0x24;
const FC_PRCONF_RESET: u32 = 0x10;
/// The clock domains this driver does not use: CEC, CSC, audio.
const CLKDIS_UNUSED: u32 =
    mc::mc_clkdis::CECCLK_DISABLE_MASK | mc::mc_clkdis::CSCCLK_DISABLE_MASK | mc::mc_clkdis::AUDCLK_DISABLE_MASK;
const CTRLDUR_MIN: u32 = 12;
const EXCTRLDUR_MIN: u32 = 32;

fn rd(m: &impl Mmio, base: usize, off: u32) -> u32 {
    u32::from(m.read8(base + off as usize))
}

fn wr(m: &impl Mmio, base: usize, off: u32, v: u32) {
    m.write8(base + off as usize, v as u8)
}

/// `design_id`, `revision_id`, `product_id0`, `product_id1`.
pub fn ids(m: &impl Mmio, base: usize) -> [u8; 4] {
    [id::DESIGN_ID, id::REVISION_ID, id::PRODUCT_ID0, id::PRODUCT_ID1]
        .map(|o| m.read8(base + (IDENTIFICATION + o) as usize))
}

/// What goes to the PHY interface registers (see the module text): `None`
/// leaves a register untouched.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PhyIf {
    pub conf0: Option<u8>,
    pub phyrstz: Option<u8>,
}

/// `fc_exctrlspac` for a TMDS clock: the largest n whose spacing
/// 256 x 512 x (n + 1) / f stays within 50 ms.
pub fn exctrlspac(tmds_hz: u32) -> u32 {
    let periods = u64::from(tmds_hz) / 20; // f x 50 ms
    let n1 = periods / (256 * 512);
    n1.clamp(1, 256) as u32 - 1
}

fn fits(mode: &VideoMode) -> bool {
    mode.hactive < 1 << 14
        && mode.hblank() < 1 << 13
        && mode.vactive < 1 << 13
        && mode.vblank() < 1 << 8
        && mode.hfront < 1 << 13
        && mode.hsync < 1 << 10
        && mode.vfront < 1 << 8
        && mode.vsync < 1 << 6
}

fn invidconf(mode: &VideoMode) -> u32 {
    // DVI_modez 0: DVI; in_I_P 0: progressive
    (if mode.vsync_high { fc::fc_invidconf::VSYNC_IN_POLARITY_MASK } else { 0 })
        | (if mode.hsync_high { fc::fc_invidconf::HSYNC_IN_POLARITY_MASK } else { 0 })
        | (if mode.de_high { fc::fc_invidconf::DE_IN_POLARITY_MASK } else { 0 })
}

/// Program the controller for `mode` at `tmds_hz`, in the order of the
/// module text: clock domains, input mapping, packetizer, frame composer,
/// colour-space bypass, the TMDS reset request, then the PHY interface.
pub fn configure(m: &impl Mmio, base: usize, mode: &VideoMode, tmds_hz: u32, phy_if: &PhyIf) -> Result<(), i32> {
    mode.check()?;
    if !fits(mode) || tmds_hz == 0 {
        return Err(-EINVAL);
    }
    let f = |o: u32| FRAMECOMPOSER + o;
    wr(m, base, MAINCONTROLLER + mc::MC_CLKDIS, CLKDIS_UNUSED);

    wr(m, base, VIDEOSAMPLER + vs::TX_INVID0, field(vs::tx_invid0::VIDEO_MAPPING_MASK, VIDEO_MAPPING_RGB444_8));
    wr(m, base, VIDEOSAMPLER + vs::TX_INSTUFFING, 0);

    wr(m, base, VIDEOPACKETIZER + vp::VP_PR_CD, 0);
    wr(m, base, VIDEOPACKETIZER + vp::VP_STUFF, 0);
    wr(m, base, VIDEOPACKETIZER + vp::VP_REMAP, 0);
    wr(m, base, VIDEOPACKETIZER + vp::VP_CONF, VP_CONF_RESET);

    wr(m, base, f(fc::FC_INVIDCONF), invidconf(mode));
    wr(m, base, f(fc::FC_INHACTIV0), mode.hactive & 0xff);
    wr(m, base, f(fc::FC_INHACTIV1), (mode.hactive >> 8) & 0x3f);
    wr(m, base, f(fc::FC_INHBLANK0), mode.hblank() & 0xff);
    wr(m, base, f(fc::FC_INHBLANK1), (mode.hblank() >> 8) & 0x1f);
    wr(m, base, f(fc::FC_INVACTIV0), mode.vactive & 0xff);
    wr(m, base, f(fc::FC_INVACTIV1), (mode.vactive >> 8) & 0x1f);
    wr(m, base, f(fc::FC_INVBLANK), mode.vblank());
    wr(m, base, f(fc::FC_HSYNCINDELAY0), mode.hfront & 0xff);
    wr(m, base, f(fc::FC_HSYNCINDELAY1), (mode.hfront >> 8) & 0x1f);
    wr(m, base, f(fc::FC_HSYNCINWIDTH0), mode.hsync & 0xff);
    wr(m, base, f(fc::FC_HSYNCINWIDTH1), (mode.hsync >> 8) & 0x03);
    wr(m, base, f(fc::FC_VSYNCINDELAY), mode.vfront);
    wr(m, base, f(fc::FC_VSYNCINWIDTH), mode.vsync);
    wr(m, base, f(fc::FC_CTRLDUR), CTRLDUR_MIN);
    wr(m, base, f(fc::FC_EXCTRLDUR), EXCTRLDUR_MIN);
    wr(m, base, f(fc::FC_EXCTRLSPAC), exctrlspac(tmds_hz));
    wr(m, base, f(fc::FC_PRCONF), FC_PRCONF_RESET);

    wr(m, base, MAINCONTROLLER + mc::MC_FLOWCTRL, 0);

    // TMDS reset request, then a write to a frame composer timing register
    wr(m, base, MAINCONTROLLER + mc::MC_SWRSTZREQ_1, 0xff & !mc::mc_swrstzreq_1::TMDSSWRST_REQ_MASK);
    wr(m, base, f(fc::FC_INVIDCONF), invidconf(mode));

    if let Some(v) = phy_if.conf0 {
        wr(m, base, PHYCONFIGURATION + phy::PHY_CONF0, u32::from(v));
    }
    if let Some(v) = phy_if.phyrstz {
        wr(m, base, MAINCONTROLLER + mc::MC_PHYRSTZ, u32::from(v));
    }
    Ok(())
}

/// Send the fixed debug video instead of the input (staging), or the input.
pub fn force_video(m: &impl Mmio, base: usize, on: bool) {
    let off = FRAMECOMPOSER + fc::FC_DBGFORCE;
    let v = rd(m, base, off) & !fc::fc_dbgforce::FORCEVIDEO_MASK;
    wr(m, base, off, v | if on { fc::fc_dbgforce::FORCEVIDEO_MASK } else { 0 });
}

/// What the controller reports, read only.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Status {
    pub phy_lock: bool,
    pub hpd: bool,
    /// RX_SENSE 0..3: a sink terminates the lanes.
    pub rx_sense: u32,
    pub pixel_clock: bool,
    pub tmds_clock: bool,
}

/// `phy_stat0` and `mc_lockonclock_1`.
pub fn status(m: &impl Mmio, base: usize) -> Status {
    let s = rd(m, base, PHYCONFIGURATION + phy::PHY_STAT0);
    let l = rd(m, base, MAINCONTROLLER + mc::MC_LOCKONCLOCK_1);
    Status {
        phy_lock: s & phy::phy_stat0::TX_PHY_LOCK_MASK != 0,
        hpd: s & phy::phy_stat0::HPD_MASK != 0,
        rx_sense: get(0xf0, s),
        pixel_clock: l & mc::mc_lockonclock_1::PCLK_MASK != 0,
        tmds_clock: l & mc::mc_lockonclock_1::TCLK_MASK != 0,
    }
}

/// Clear the clock-present flags so the next [`status`] tells whether the
/// clocks are still there.
pub fn clear_clock_flags(m: &impl Mmio, base: usize) {
    wr(
        m,
        base,
        MAINCONTROLLER + mc::MC_LOCKONCLOCK_1,
        mc::mc_lockonclock_1::PCLK_MASK | mc::mc_lockonclock_1::TCLK_MASK,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model_hdmi::{HdmiModel, HDMI};
    use crate::video::WUXGA_60_RB;

    #[test]
    fn layout_is_the_rm() {
        // 13.9.3: register offsets within the sub-blocks
        assert_eq!((vs::TX_INVID0, vp::VP_PR_CD, vp::VP_CONF), (0, 1, 4));
        assert_eq!((fc::FC_INVIDCONF, fc::FC_INVBLANK, fc::FC_VSYNCINWIDTH, fc::FC_EXCTRLSPAC), (0, 7, 0xd, 0x13));
        assert_eq!((fc::FC_PRCONF, fc::FC_DBGFORCE), (0xe0, 0x200));
        assert_eq!((phy::PHY_CONF0, phy::PHY_STAT0), (0, 4));
        assert_eq!((mc::MC_CLKDIS, mc::MC_SWRSTZREQ_1, mc::MC_FLOWCTRL, mc::MC_PHYRSTZ), (1, 2, 4, 5));
        assert_eq!(crate::soc::imx8mp::regs::framecomposer::REG_BITS, 8);
    }

    #[test]
    fn exctrlspac_stays_within_50_ms() {
        // 154.29 MHz: 7.71e6 periods in 50 ms / 131072 = 58.85 -> n = 57
        assert_eq!(exctrlspac(154_285_714), 57);
        let n = exctrlspac(25_000_000);
        assert!(256.0 * 512.0 * f64::from(n + 1) / 25e6 <= 0.050);
        assert_eq!(exctrlspac(1), 0);
    }

    #[test]
    fn configure_writes_bytes_in_order_and_the_timing_fields() {
        let m = HdmiModel::hdmi_only();
        let b = m.base(HDMI);
        configure(&m, b, &WUXGA_60_RB, 154_285_714, &PhyIf::default()).unwrap();
        let p = |o| m.peek(HDMI, o);
        assert_eq!(p(MAINCONTROLLER + 1), 0x38);
        assert_eq!(p(VIDEOSAMPLER), 0x01);
        assert_eq!(p(VIDEOPACKETIZER + 4), 0x24);
        // +hsync -vsync DE high, DVI, progressive
        assert_eq!(p(FRAMECOMPOSER), 0x30);
        assert_eq!((p(0x1001), p(0x1002)), (0x80, 0x07)); // 1920
        assert_eq!((p(0x1003), p(0x1004)), (0xa0, 0x00)); // 160
        assert_eq!((p(0x1005), p(0x1006)), (0xb0, 0x04)); // 1200
        assert_eq!(p(0x1007), 35);
        assert_eq!((p(0x1008), p(0x1009), p(0x100a), p(0x100b)), (48, 0, 32, 0));
        assert_eq!((p(0x100c), p(0x100d)), (3, 6));
        assert_eq!((p(0x1011), p(0x1012), p(0x1013)), (12, 32, 57));
        assert_eq!(p(MAINCONTROLLER + 4), 0);
        // PHY interface untouched by default
        assert_eq!(p(PHYCONFIGURATION), 0x06);
        assert!(!m.writes().iter().any(|w| w.1 == MAINCONTROLLER + 5));
        // the TMDS reset request is followed by a frame composer write
        let w = m.writes();
        let rst = w.iter().position(|x| x.1 == MAINCONTROLLER + 2).unwrap();
        assert_eq!(w[rst].2, 0xfd);
        assert_eq!(w[rst + 1].1, FRAMECOMPOSER);
    }

    #[test]
    fn phy_interface_values_are_written_when_given() {
        let m = HdmiModel::hdmi_only();
        let b = m.base(HDMI);
        configure(&m, b, &WUXGA_60_RB, 154_285_714, &PhyIf { conf0: Some(0x0e), phyrstz: Some(1) }).unwrap();
        assert_eq!(m.peek(HDMI, PHYCONFIGURATION), 0x0e);
        assert_eq!(m.peek(HDMI, MAINCONTROLLER + 5), 1);
    }

    #[test]
    fn modes_that_do_not_fit_the_fields_are_refused() {
        let m = HdmiModel::hdmi_only();
        let b = m.base(HDMI);
        let tall_blank = VideoMode { vback: 300, ..WUXGA_60_RB };
        assert_eq!(configure(&m, b, &tall_blank, 154_000_000, &PhyIf::default()), Err(-EINVAL));
        let wide_sync = VideoMode { vsync: 64, ..WUXGA_60_RB };
        assert_eq!(configure(&m, b, &wide_sync, 154_000_000, &PhyIf::default()), Err(-EINVAL));
        assert!(m.writes().is_empty());
    }

    #[test]
    fn ids_force_and_status() {
        let m = HdmiModel::hdmi_only();
        let b = m.base(HDMI);
        assert_eq!(ids(&m, b)[0], 0x21, "DWC_hdmi_tx 2.13a (13.9.3.1.2.3)");
        force_video(&m, b, true);
        assert_eq!(m.peek(HDMI, FRAMECOMPOSER + 0x200), 1);
        force_video(&m, b, false);
        assert_eq!(m.peek(HDMI, FRAMECOMPOSER + 0x200), 0);
        let s = status(&m, b);
        assert!(s.hpd && !s.phy_lock);
    }
}
