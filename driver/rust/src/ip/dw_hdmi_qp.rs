// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The Synopsys DesignWare HDMI QP TX controller (HDMI 2.1) as the RK3588
//! carries it (TX0 0xfde80000, TX1 0xfdea0000), with the RK3588 GRF glue it
//! needs: bind-time values, the TMDS enable in DVI mode (no InfoFrames, no
//! audio; the HDMI bit is a parameter), the HDCP2 bypass, the hot-plug level
//! and the status read-back. Step D6 of `driver/docs/nanopc-t6-display.md`
//! (section 7); the facts are its section 1 "DW HDMI QP TX0".
//!
//! This is not a variant of [`crate::ip::dw_hdmi_tx`] (the older 8-bit
//! DesignWare HDMI TX of the i.MX 8M Plus): the QP controller has 32-bit
//! registers and takes the timing from the VOP; it is programmed with no
//! timing at all.
//!
//! Sources: Linux v6.18, GPL-2.0, read for facts only (address, bit, value,
//! order; `docs/source-form.md`): `drivers/gpu/drm/bridge/synopsys/
//! dw-hdmi-qp.h` ("qp.h"), `dw-hdmi-qp.c` ("qp.c"),
//! `drivers/gpu/drm/rockchip/dw_hdmi_qp-rockchip.c` ("glue"),
//! `rockchip_vop2_reg.c` ("vop2_reg.c"), `rockchip_drm_vop2.c` / `.h`
//! ("vop2.c", "vop2.h"),
//! `arch/arm64/boot/dts/rockchip/rk3588-base.dtsi` ("dtsi"). Facts:
//!
//! 1. Controller registers are 32 bits, stride 4, the last at 0x482c
//!    (qp.c:1006-1011). Identity: `CORE_ID` 0x0, `VER_NUMBER` 0x4,
//!    `VER_TYPE` 0x8, `CONFIG_REG` 0xc (qp.h:13-16). Linux reads none of them
//!    and names no expected value, so the expected `CORE_ID` is the caller's
//!    (the board dump's, D0), never a constant here.
//! 2. Bind (qp.c:1013-1032): `MAINUNIT_0_INT_MASK_N` 0x3014 = 0,
//!    `MAINUNIT_1_INT_MASK_N` 0x3024 = 0 (qp.h:761, :776), then
//!    `TIMER_BASE_CONFIG0` 0x80 (qp.h:35) = 428571429, the reference clock in
//!    Hz as Linux assumes it ([`LINUX_TIMER_BASE_HZ`]); the board passes the
//!    rate the dump's `clk_summary` gives (note, U14). The T6 measured
//!    `clk_hdmitx0_ref` = 428571429 Hz under Linux 6.18 on 2026-11-12, so
//!    the value is confirmed for this board. Linux then resets and
//!    sets up the I2C master (`I2CM_*` 0xe0-0x118, qp.h:56-80) and clears
//!    its done / NACK interrupts in `MAINUNIT_1_INT_CLEAR` 0x3028: that is
//!    EDID, step D10, and nothing here reads or writes those registers.
//! 3. Enable (qp.c:834-866): the HDPTX PHY is powered first (step D5),
//!    then `HDCP2LOGIC_CONFIG0` 0x8e0 bit 0 (`HDCP2_BYPASS`) is set, then
//!    `LINK_CONFIG0` 0x968 bit 4 (`OPMODE_DVI`) is set for a DVI sink and
//!    cleared for HDMI (qp.h:155-156, :177-179).
//!    Both are read-modify-write of that one bit. HDMI also needs the AVI
//!    InfoFrame (qp.c:720-755), which this module does not send.
//! 4. The TMDS character rate is the mode's pixel clock for RGB at 8 bits
//!    per component (glue:114-135 asks for 8 bpc RGB;
//!    `drivers/gpu/drm/display/drm_hdmi_helper.c:211-257`: clock x bpc / 8),
//!    and the controller refuses a rate above 340 MHz (qp.c:36, :901-909).
//! 5. Disable (qp.c:868-876, glue:163-169): Linux writes no controller
//!    register; it powers the PHY off. So the power-off order is: the PHY
//!    off (D5), then the controller's clocks and PD_VO1 (D4, `soc::rk3588`);
//!    the controller keeps its register values until its domain goes off.
//!    No function here is needed for it.
//! 6. CMU: `CMU_STATUS` 0xb0, the clocks are locked when `& 0x3f` reads
//!    0x15 (qp.h:42-44); `CMU_VIDQPCLK_FREQ` 0xb8 and `CMU_LINKQPCLK_FREQ`
//!    0xbc measure the video and link clocks (qp.h:51-52; units not given:
//!    compared with the dump, note U7). `MAINUNIT_STATUS0` 0x180 (qp.h:97),
//!    `VIDEO_MONITOR_STATUS0-6` 0x884-0x89c (qp.h:147-153): fields not
//!    named, compared with the dump.
//! 7. Words that request an action are never read: `GLOBAL_SWRESET_REQUEST`
//!    0x40, `TMDS_FIFO_CONTROL0` 0x974, `PKTSCHED_PKT_SEND` 0xab0 (qp.h:23,
//!    :183, :216; note section 6). Nothing here reads them or writes them.
//! 8. Left out, besides the I2C master, InfoFrames and audio: scrambling
//!    (Linux uses it only above 340 MHz, which [`tmds_char_rate`] refuses),
//!    CEC, HDCP 1.4, and the main interrupt (Linux requests it, qp.c:1071;
//!    here its masks stay 0 and nothing takes it: every status is polled).
//! 9. Nothing here waits on hot-plug: [`hpd`] is a single read for the
//!    status line. The T6's panel gives no hot-plug level and no EDID
//!    (board, Linux 6.18, 2026-11-12), so the board uses one fixed mode and
//!    [`Sink::Dvi`]; Linux too picks DVI when there is no EDID.
//!
//! The RK3588 glue (glue, and dtsi:1606-1630: the controller's
//! `rockchip,grf` is sys_grf 0xfd58c000, its `rockchip,vo-grf` is vo1_grf
//! 0xfd5a8000, dtsi:582-585, :608-612). Every GRF word carries its write
//! mask in bits 31:16, and a bit is written only where its mask bit is set
//! (`include/linux/hw_bitfield.h:26-33`). The port is told by the
//! controller's base: 0xfde80000 is port 0, 0xfdea0000 port 1
//! (glue:406-414, :467-477). In Linux's order (glue:510-557): clocks on,
//! then the GRF words 1-4, then the controller (fact 2), then word 5:
//!
//! 1. vo1_grf `CON3` 0x0c (port 0) or `CON6` 0x18 (port 1): bits 9 SCLIN,
//!    10 SDAIN, 11 MODE, 13 I2S_SEL set (glue:75-80, :353-359).
//! 2. sys_grf `SOC_CON7` 0x31c: bits 12 (HDMI0) and 13 (HDMI1) HPD IO
//!    enable, both, whichever the port (glue:69-71, :361-363).
//! 3. vo1_grf `CON9` 0x24: bit 10 (port 0) or 12 (port 1) GRANT_SEL set
//!    (glue:81-83, :365-369).
//! 4. sys_grf `SOC_CON2` 0x308: the port's HPD interrupt mask set, bit 13
//!    (port 0) or 15 (port 1) (glue:64-68, :371-375).
//! 5. After the controller: `SOC_CON2` HPD interrupt clear (bit 12 / 14) = 1
//!    and mask = 0, in one word (glue:183-196). This driver polls the level
//!    and takes no interrupt; the word is written because Linux leaves it so.
//!
//! The HPD level is sys_grf `SOC_STATUS1` 0x384 bit 16 (port 0), bit 24
//! (port 1) (glue:72-74, :171-181).
//!
//! The VOP side of the same link (vop2_reg.c:1690-1714) writes two more GRF
//! words when it routes a video port to HDMI: vop_grf `GRF_VOP_CON2` 0x08
//! bit 1 (port 0) or 4 (port 1) = 1, and vo1_grf `GRF_VO1_CON0` 0x00 bits
//! 6:5 (port 0) or 8:7 (port 1) = a sync polarity field (vop2.h:400-401).
//! They belong to the VOP's enable (step D1 / D7), so this module only forms
//! them ([`vop_grf_con2_word`], [`vo1_con0_sync_word`]) and writes neither.
//! The field Linux v6.18 writes is not the plain "hsync, vsync positive"
//! pair: the VOP builds flags with bit 0 = +hsync, bit 1 = +vsync, bit 3 =
//! dclk inverted (vop2.c:1655-1661; `rockchip_drm_vop.h:346-347`,
//! vop2.h:770), and the RK3588 HDMI case tests them against DRM's NHSYNC
//! (bit 1) and NVSYNC (bit 3) (vop2_reg.c:1660-1668; `drm_mode.h:71-74`), so
//! field bit 0 follows +vsync and field bit 1 the dclk inversion (0 for
//! HDMI). [`linux_sync_field`] reproduces it, so the image can equal the
//! dump; what the bits mean in hardware is not in any source we have.

use crate::mmio::Mmio;
use crate::video::VideoMode;

/// Controller bases (dtsi:1606-1608; `rk3588-extra.dtsi:243`).
pub const TX0_BASE: usize = 0xfde8_0000;
pub const TX1_BASE: usize = 0xfdea_0000;
/// The GRFs the glue writes (dtsi:582-585, :597-600, :608-612).
pub const SYS_GRF_BASE: usize = 0xfd58_c000;
pub const VOP_GRF_BASE: usize = 0xfd5a_4000;
pub const VO1_GRF_BASE: usize = 0xfd5a_8000;

/// Controller register offsets (qp.h; see the module text for each line).
pub mod reg {
    pub const CORE_ID: usize = 0x000;
    pub const VER_NUMBER: usize = 0x004;
    pub const VER_TYPE: usize = 0x008;
    pub const CONFIG_REG: usize = 0x00c;
    /// Never read (module text, fact 7).
    pub const GLOBAL_SWRESET_REQUEST: usize = 0x040;
    pub const TIMER_BASE_CONFIG0: usize = 0x080;
    pub const CMU_STATUS: usize = 0x0b0;
    pub const CMU_VIDQPCLK_FREQ: usize = 0x0b8;
    pub const CMU_LINKQPCLK_FREQ: usize = 0x0bc;
    /// The I2C master's first and last register (qp.h:56, :80): step D10.
    pub const I2CM_FIRST: usize = 0x0e0;
    pub const I2CM_LAST: usize = 0x118;
    pub const MAINUNIT_STATUS0: usize = 0x180;
    pub const VIDEO_MONITOR_STATUS0: usize = 0x884;
    pub const HDCP2LOGIC_CONFIG0: usize = 0x8e0;
    pub const LINK_CONFIG0: usize = 0x968;
    /// Never read (module text, fact 7).
    pub const TMDS_FIFO_CONTROL0: usize = 0x974;
    /// Never read (module text, fact 7).
    pub const PKTSCHED_PKT_SEND: usize = 0xab0;
    pub const MAINUNIT_0_INT_MASK_N: usize = 0x3014;
    pub const MAINUNIT_1_INT_MASK_N: usize = 0x3024;
    /// The last register (qp.c:1010).
    pub const LAST: usize = 0x482c;
}

/// `HDCP2LOGIC_CONFIG0.HDCP2_BYPASS` (qp.h:156).
pub const HDCP2_BYPASS: u32 = 1 << 0;
/// `LINK_CONFIG0.OPMODE_DVI` (qp.h:179).
pub const OPMODE_DVI: u32 = 1 << 4;
/// `CMU_STATUS`: the clock monitor bits and their "locked" value (qp.h:43-44).
pub const CMU_CLK_MONITOR: u32 = 0x3f;
pub const CMU_CLK_LOCKED: u32 = 0x15;
/// The reference clock rate Linux writes to `TIMER_BASE_CONFIG0` (qp.c:1017).
pub const LINUX_TIMER_BASE_HZ: u32 = 428_571_429;
/// The largest TMDS character rate the controller takes (qp.c:36).
pub const TMDS_MAX_HZ: u64 = 340_000_000;
/// `VIDEO_MONITOR_STATUS0` .. `6`.
pub const VIDEO_MONITOR_WORDS: usize = 7;

/// GRF offsets and bits (glue:64-83).
pub mod grf {
    /// sys_grf
    pub const SOC_CON2: usize = 0x308;
    pub const SOC_CON7: usize = 0x31c;
    pub const SOC_STATUS1: usize = 0x384;
    /// vo1_grf
    pub const VO1_CON0: usize = 0x00;
    pub const VO1_CON3: usize = 0x0c;
    pub const VO1_CON6: usize = 0x18;
    pub const VO1_CON9: usize = 0x24;
    /// vop_grf (vop2.h:400)
    pub const VOP_CON2: usize = 0x08;

    /// `VO1_CON3` / `VO1_CON6`
    pub const SCLIN: u32 = 1 << 9;
    pub const SDAIN: u32 = 1 << 10;
    pub const MODE: u32 = 1 << 11;
    pub const I2S_SEL: u32 = 1 << 13;
    /// `SOC_CON7`
    pub const HPD_IO_EN_HDMI0: u32 = 1 << 12;
    pub const HPD_IO_EN_HDMI1: u32 = 1 << 13;
}

/// A GRF word: `bits` (inside `mask`) with the write mask in 31:16.
pub const fn hiword(mask: u32, bits: u32) -> u32 {
    (mask << 16) | (bits & mask)
}

/// Which of the two controllers (and so which GRF bits).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Port {
    Hdmi0 = 0,
    Hdmi1 = 1,
}

impl Port {
    /// The port of the controller at `base`, as the glue tells it.
    pub const fn of_base(base: usize) -> Option<Port> {
        match base {
            TX0_BASE => Some(Port::Hdmi0),
            TX1_BASE => Some(Port::Hdmi1),
            _ => None,
        }
    }

    /// vo1_grf: the I/O word's register (`CON3` / `CON6`).
    pub const fn io_con(self) -> usize {
        match self {
            Port::Hdmi0 => grf::VO1_CON3,
            Port::Hdmi1 => grf::VO1_CON6,
        }
    }
    /// vo1_grf `CON9`: GRANT_SEL.
    pub const fn grant_sel(self) -> u32 {
        match self {
            Port::Hdmi0 => 1 << 10,
            Port::Hdmi1 => 1 << 12,
        }
    }
    /// sys_grf `SOC_CON2`: HPD interrupt clear.
    pub const fn hpd_int_clr(self) -> u32 {
        match self {
            Port::Hdmi0 => 1 << 12,
            Port::Hdmi1 => 1 << 14,
        }
    }
    /// sys_grf `SOC_CON2`: HPD interrupt mask.
    pub const fn hpd_int_msk(self) -> u32 {
        match self {
            Port::Hdmi0 => 1 << 13,
            Port::Hdmi1 => 1 << 15,
        }
    }
    /// sys_grf `SOC_STATUS1`: HPD level.
    pub const fn hpd_level(self) -> u32 {
        match self {
            Port::Hdmi0 => 1 << 16,
            Port::Hdmi1 => 1 << 24,
        }
    }
    /// vo1_grf `GRF_VO1_CON0`: the sync polarity field.
    pub const fn sync_field(self) -> u32 {
        match self {
            Port::Hdmi0 => 0x3 << 5,
            Port::Hdmi1 => 0x3 << 7,
        }
    }
    /// vop_grf `GRF_VOP_CON2`: the HDMI select bit.
    pub const fn vop_con2_bit(self) -> u32 {
        match self {
            Port::Hdmi0 => 1 << 1,
            Port::Hdmi1 => 1 << 4,
        }
    }
}

/// The GRF words of the glue, in the order they are written (module text,
/// words 1-4 before the controller, word 5 after it).
pub const fn grf_io_word() -> u32 {
    let m = grf::SCLIN | grf::SDAIN | grf::MODE | grf::I2S_SEL;
    hiword(m, m)
}
pub const fn grf_hpd_io_en_word() -> u32 {
    let m = grf::HPD_IO_EN_HDMI0 | grf::HPD_IO_EN_HDMI1;
    hiword(m, m)
}
pub const fn grf_grant_word(port: Port) -> u32 {
    hiword(port.grant_sel(), port.grant_sel())
}
pub const fn grf_hpd_mask_word(port: Port) -> u32 {
    hiword(port.hpd_int_msk(), port.hpd_int_msk())
}
pub const fn grf_hpd_unmask_clear_word(port: Port) -> u32 {
    hiword(port.hpd_int_clr() | port.hpd_int_msk(), port.hpd_int_clr())
}

/// vop_grf `GRF_VOP_CON2` for a VP routed to `port` (module text; written
/// by the VOP side, not here).
pub const fn vop_grf_con2_word(port: Port) -> u32 {
    hiword(port.vop_con2_bit(), port.vop_con2_bit())
}

/// The 2-bit field Linux v6.18 writes into `GRF_VO1_CON0` for `mode` on an
/// HDMI output: bit 0 = the mode's vsync is positive, bit 1 = 0 (module
/// text; vop2_reg.c:1660-1668 over vop2.c:1655-1661).
pub const fn linux_sync_field(mode: &VideoMode) -> u32 {
    if mode.vsync_high {
        1
    } else {
        0
    }
}

/// vo1_grf `GRF_VO1_CON0` with `field` (2 bits) in `port`'s place (written
/// by the VOP side, not here).
pub const fn vo1_con0_sync_word(port: Port, field: u32) -> u32 {
    let m = port.sync_field();
    hiword(m, (field & 0x3) << m.trailing_zeros())
}

/// The bases a bind needs: the controller and the two GRFs it writes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Bases {
    pub ctrl: usize,
    pub sys_grf: usize,
    pub vo1_grf: usize,
}

/// What the board knows only from its dump (note, D0).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BindParams {
    /// The `CORE_ID` the dump read on this SoC.
    pub core_id: u32,
    /// The controller's reference clock in Hz (`TIMER_BASE_CONFIG0`):
    /// [`LINUX_TIMER_BASE_HZ`] unless the dump's `clk_summary` says another.
    /// The T6's `clk_hdmitx0_ref` is 428571429 Hz (measured 2026-11-12,
    /// Linux 6.18), equal to [`LINUX_TIMER_BASE_HZ`].
    pub timer_base_hz: u32,
}

/// The identity words (fact 1).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Ident {
    pub core_id: u32,
    pub ver_number: u32,
    pub ver_type: u32,
    pub config: u32,
}

/// Read the identity words.
pub fn ident(m: &impl Mmio, ctrl: usize) -> Ident {
    Ident {
        core_id: m.read32(ctrl + reg::CORE_ID),
        ver_number: m.read32(ctrl + reg::VER_NUMBER),
        ver_type: m.read32(ctrl + reg::VER_TYPE),
        config: m.read32(ctrl + reg::CONFIG_REG),
    }
}

/// Why a step refused. The value crosses to C as one number per cause.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum Refusal {
    /// `CORE_ID` is not the one the board expects (another block at that
    /// address). Nothing was written. This is no guard against a controller
    /// whose clocks or PD_VO1 are off: on the RK3588 such a read can stall
    /// the bus and never return. The caller (D7) runs the power and clock
    /// step (D4) before [`bind`].
    WrongId = 1,
    /// `TIMER_BASE_CONFIG0` read back other than written.
    TimerReadBack = 2,
    /// A TMDS rate of 0 or above [`TMDS_MAX_HZ`]. Nothing was written.
    TmdsRate = 3,
    /// `LINK_CONFIG0` or `HDCP2LOGIC_CONFIG0` read back without the bits
    /// just written.
    EnableReadBack = 4,
    /// The port is not the one of the controller base (`Port::of_base`), or
    /// the base is no controller. `found` is the base. Nothing was written.
    PortMismatch = 5,
}

/// A refusal and the value it is about (the id found, the word read back,
/// the rate asked for, cut to 32 bits).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Refused {
    pub why: Refusal,
    pub found: u32,
}

/// Bind the controller of `port` at `b`: check its identity, then the glue
/// words 1-4, the controller's interrupt masks and timer base (fact 2),
/// then glue word 5 (module text). The controller's clocks and PD_VO1 must
/// be on (D4) before this is called: the first read is the identity.
/// `port` must be the one of `b.ctrl` ([`Port::of_base`], as the glue tells
/// it, glue:467-477). A port that disagrees or a wrong identity refuses
/// before any write, GRF included.
pub fn bind(m: &impl Mmio, b: &Bases, port: Port, p: &BindParams) -> Result<Ident, Refused> {
    if Port::of_base(b.ctrl) != Some(port) {
        return Err(Refused { why: Refusal::PortMismatch, found: b.ctrl as u32 });
    }
    let id = ident(m, b.ctrl);
    if id.core_id != p.core_id {
        return Err(Refused { why: Refusal::WrongId, found: id.core_id });
    }
    m.write32(b.vo1_grf + port.io_con(), grf_io_word());
    m.write32(b.sys_grf + grf::SOC_CON7, grf_hpd_io_en_word());
    m.write32(b.vo1_grf + grf::VO1_CON9, grf_grant_word(port));
    m.write32(b.sys_grf + grf::SOC_CON2, grf_hpd_mask_word(port));

    m.write32(b.ctrl + reg::MAINUNIT_0_INT_MASK_N, 0);
    m.write32(b.ctrl + reg::MAINUNIT_1_INT_MASK_N, 0);
    m.write32(b.ctrl + reg::TIMER_BASE_CONFIG0, p.timer_base_hz);
    let t = m.read32(b.ctrl + reg::TIMER_BASE_CONFIG0);
    if t != p.timer_base_hz {
        return Err(Refused { why: Refusal::TimerReadBack, found: t });
    }

    m.write32(b.sys_grf + grf::SOC_CON2, grf_hpd_unmask_clear_word(port));
    Ok(id)
}

/// The TMDS character rate for `mode`, RGB 8 bits per component (fact 4).
pub fn tmds_char_rate(mode: &VideoMode) -> Result<u64, Refused> {
    let hz = u64::from(mode.pixel_clock_hz);
    if hz == 0 || hz > TMDS_MAX_HZ {
        return Err(Refused { why: Refusal::TmdsRate, found: mode.pixel_clock_hz });
    }
    Ok(hz)
}

/// What the sink takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sink {
    /// DVI: `OPMODE_DVI` set, no data islands.
    Dvi,
    /// HDMI: `OPMODE_DVI` clear. The AVI InfoFrame is not sent here (D10).
    Hdmi,
}

/// The `LINK_CONFIG0` value for `sink`, given the value found: only
/// `OPMODE_DVI` changes.
pub const fn link_config0_for(found: u32, sink: Sink) -> u32 {
    match sink {
        Sink::Dvi => found | OPMODE_DVI,
        Sink::Hdmi => found & !OPMODE_DVI,
    }
}

/// Start the TMDS link for `sink` (fact 3): the HDCP2 bypass, then the
/// operating mode, each one bit read-modify-written and read back. The PHY
/// must already be powered at the mode's [`tmds_char_rate`] (D5).
///
/// The rate is not checked here (that would need the mode as a parameter):
/// the caller runs [`tmds_char_rate`] first and does not enable a mode it
/// refuses, as Linux rejects such a mode before its enable (qp.c:901-909).
/// Both words are always written, also when the bit is already as wanted;
/// Linux skips an unchanged write (qp.c:165-168), so the two differ only in
/// a register trace, not in the hardware state.
pub fn enable(m: &impl Mmio, ctrl: usize, sink: Sink) -> Result<(), Refused> {
    m.modify32(ctrl + reg::HDCP2LOGIC_CONFIG0, HDCP2_BYPASS, HDCP2_BYPASS);
    let h = m.read32(ctrl + reg::HDCP2LOGIC_CONFIG0);
    if h & HDCP2_BYPASS == 0 {
        return Err(Refused { why: Refusal::EnableReadBack, found: h });
    }
    let found = m.read32(ctrl + reg::LINK_CONFIG0);
    let want = link_config0_for(found, sink);
    m.write32(ctrl + reg::LINK_CONFIG0, want);
    let l = m.read32(ctrl + reg::LINK_CONFIG0);
    if l & OPMODE_DVI != want & OPMODE_DVI {
        return Err(Refused { why: Refusal::EnableReadBack, found: l });
    }
    Ok(())
}

/// The hot-plug level of `port` (a cable and a powered sink).
pub fn hpd(m: &impl Mmio, sys_grf: usize, port: Port) -> bool {
    m.read32(sys_grf + grf::SOC_STATUS1) & port.hpd_level() != 0
}

/// The controller's part of the note's section 5 read-backs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Status {
    pub cmu_status: u32,
    pub vidqpclk_freq: u32,
    pub linkqpclk_freq: u32,
    pub mainunit_status0: u32,
    pub video_monitor: [u32; VIDEO_MONITOR_WORDS],
    pub hdcp2logic_config0: u32,
    pub link_config0: u32,
    pub hpd: bool,
}

impl Status {
    /// The controller sees its video and link clocks (fact 6).
    pub const fn clocks_locked(&self) -> bool {
        self.cmu_status & CMU_CLK_MONITOR == CMU_CLK_LOCKED
    }
    /// `OPMODE_DVI` is set.
    pub const fn dvi(&self) -> bool {
        self.link_config0 & OPMODE_DVI != 0
    }
}

/// Read the status words, read only: no request word, no I2C master word.
pub fn status(m: &impl Mmio, ctrl: usize, sys_grf: usize, port: Port) -> Status {
    let r = |o: usize| m.read32(ctrl + o);
    let mut video_monitor = [0; VIDEO_MONITOR_WORDS];
    for (i, w) in video_monitor.iter_mut().enumerate() {
        *w = r(reg::VIDEO_MONITOR_STATUS0 + 4 * i);
    }
    Status {
        cmu_status: r(reg::CMU_STATUS),
        vidqpclk_freq: r(reg::CMU_VIDQPCLK_FREQ),
        linkqpclk_freq: r(reg::CMU_LINKQPCLK_FREQ),
        mainunit_status0: r(reg::MAINUNIT_STATUS0),
        video_monitor,
        hdcp2logic_config0: r(reg::HDCP2LOGIC_CONFIG0),
        link_config0: r(reg::LINK_CONFIG0),
        hpd: hpd(m, sys_grf, port),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::video::WUXGA_60_RB;
    use std::cell::RefCell;
    use std::collections::BTreeMap;

    // The model's own addresses and offsets, restated from the sources (not
    // shared with the driver, so a wrong constant cannot agree with itself).
    const CTRL: usize = 0xfde8_0000;
    const CTRL1: usize = 0xfdea_0000;
    const SYS: usize = 0xfd58_c000;
    const VO1: usize = 0xfd5a_8000;
    const B: Bases = Bases { ctrl: CTRL, sys_grf: SYS, vo1_grf: VO1 };
    const B1: Bases = Bases { ctrl: CTRL1, ..B };
    /// A test value only: the real one comes from the dump (D0).
    const ID: u32 = 0x1234_5678;
    const P: BindParams = BindParams { core_id: ID, timer_base_hz: 428_571_429 };

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum Op {
        R(usize, u32),
        W(usize, u32),
    }

    /// Both controllers (32-bit words up to 0x482c), sys_grf and vo1_grf with
    /// hiword-masked writes. It fails the test on a read of a request word,
    /// any access to the I2C master, an access outside the blocks or past
    /// the last register, and a GRF write with a value bit outside its mask.
    struct Soc {
        regs: RefCell<BTreeMap<usize, u32>>,
        ops: RefCell<Vec<Op>>,
        /// Writes to `TIMER_BASE_CONFIG0` are ignored.
        timer_dead: bool,
        /// `LINK_CONFIG0` bit 4 ignores writes.
        opmode_stuck: bool,
    }

    impl Soc {
        fn new() -> Soc {
            let s = Soc {
                regs: RefCell::new(BTreeMap::new()),
                ops: RefCell::new(Vec::new()),
                timer_dead: false,
                opmode_stuck: false,
            };
            s.set(CTRL, ID);
            s.set(CTRL1, ID);
            s
        }
        /// The controller offset of `a`, or `None` for a GRF address.
        fn ctrl_off(a: usize) -> Option<usize> {
            [CTRL, CTRL1].into_iter().find(|b| (*b..*b + 0x2_0000).contains(&a)).map(|b| a - b)
        }
        fn set(&self, a: usize, v: u32) {
            self.regs.borrow_mut().insert(a, v);
        }
        fn get(&self, a: usize) -> u32 {
            *self.regs.borrow().get(&a).unwrap_or(&0)
        }
        fn writes(&self) -> Vec<(usize, u32)> {
            self.ops.borrow().iter().filter_map(|o| if let Op::W(a, v) = *o { Some((a, v)) } else { None }).collect()
        }
        fn reads(&self) -> Vec<usize> {
            self.ops.borrow().iter().filter_map(|o| if let Op::R(a, _) = *o { Some(a) } else { None }).collect()
        }
        fn check(&self, a: usize) {
            assert_eq!(a % 4, 0, "unaligned {a:#x}");
            if let Some(o) = Soc::ctrl_off(a) {
                assert!(o <= 0x482c, "past the last controller register: {o:#x}");
                assert!(!(0xe0..=0x118).contains(&o), "I2C master register {o:#x} touched (D10)");
            } else {
                assert!(
                    (SYS..SYS + 0x1000).contains(&a) || (VO1..VO1 + 0x4000).contains(&a),
                    "outside the blocks: {a:#x}"
                );
            }
        }
    }

    impl Mmio for Soc {
        fn read32(&self, a: usize) -> u32 {
            self.check(a);
            if let Some(o) = Soc::ctrl_off(a) {
                assert!(![0x40, 0x974, 0xab0].contains(&o), "request word {o:#x} read");
            }
            let v = self.get(a);
            self.ops.borrow_mut().push(Op::R(a, v));
            v
        }
        fn write32(&self, a: usize, v: u32) {
            self.check(a);
            self.ops.borrow_mut().push(Op::W(a, v));
            if let Some(o) = Soc::ctrl_off(a) {
                match o {
                    0x80 if self.timer_dead => {}
                    0x968 if self.opmode_stuck => self.set(a, (v & !0x10) | (self.get(a) & 0x10)),
                    _ => self.set(a, v),
                }
            } else {
                let mask = v >> 16;
                assert_eq!(v & 0xffff & !mask, 0, "GRF write {a:#x} = {v:#x}: a value bit outside its mask");
                self.set(a, (self.get(a) & !mask) | (v & mask));
            }
        }
        fn read8(&self, _: usize) -> u8 {
            unreachable!("the QP controller and the GRFs are accessed 32 bits wide")
        }
        fn write8(&self, _: usize, _: u8) {
            unreachable!("the QP controller and the GRFs are accessed 32 bits wide")
        }
    }

    #[test]
    fn layout_is_linux_v6_18() {
        // qp.h:13-16, :23, :35, :42, :51-52, :56, :80, :97, :147, :155, :177,
        // :183, :216, :761, :776; qp.c:1010
        assert_eq!((reg::CORE_ID, reg::VER_NUMBER, reg::VER_TYPE, reg::CONFIG_REG), (0, 4, 8, 0xc));
        assert_eq!((reg::GLOBAL_SWRESET_REQUEST, reg::TIMER_BASE_CONFIG0), (0x40, 0x80));
        assert_eq!((reg::CMU_STATUS, reg::CMU_VIDQPCLK_FREQ, reg::CMU_LINKQPCLK_FREQ), (0xb0, 0xb8, 0xbc));
        assert_eq!((reg::I2CM_FIRST, reg::I2CM_LAST, reg::MAINUNIT_STATUS0), (0xe0, 0x118, 0x180));
        assert_eq!((reg::VIDEO_MONITOR_STATUS0, reg::HDCP2LOGIC_CONFIG0, reg::LINK_CONFIG0), (0x884, 0x8e0, 0x968));
        assert_eq!((reg::TMDS_FIFO_CONTROL0, reg::PKTSCHED_PKT_SEND), (0x974, 0xab0));
        assert_eq!((reg::MAINUNIT_0_INT_MASK_N, reg::MAINUNIT_1_INT_MASK_N, reg::LAST), (0x3014, 0x3024, 0x482c));
        assert_eq!((HDCP2_BYPASS, OPMODE_DVI, CMU_CLK_MONITOR, CMU_CLK_LOCKED), (1, 0x10, 0x3f, 0x15));
        assert_eq!((LINUX_TIMER_BASE_HZ, TMDS_MAX_HZ), (428_571_429, 340_000_000));
        // glue:64-83; vop2.h:400-401; dtsi
        assert_eq!((grf::SOC_CON2, grf::SOC_CON7, grf::SOC_STATUS1), (0x308, 0x31c, 0x384));
        assert_eq!(
            (grf::VO1_CON0, grf::VO1_CON3, grf::VO1_CON6, grf::VO1_CON9, grf::VOP_CON2),
            (0, 0xc, 0x18, 0x24, 8)
        );
        assert_eq!((TX0_BASE, TX1_BASE), (0xfde8_0000, 0xfdea_0000));
        assert_eq!((SYS_GRF_BASE, VOP_GRF_BASE, VO1_GRF_BASE), (0xfd58_c000, 0xfd5a_4000, 0xfd5a_8000));
        assert_eq!(
            (Port::of_base(TX0_BASE), Port::of_base(TX1_BASE), Port::of_base(0)),
            (Some(Port::Hdmi0), Some(Port::Hdmi1), None)
        );
    }

    #[test]
    fn every_grf_word_is_its_bits_under_their_own_mask() {
        // value, then the exact mask Linux writes with it
        let words = [
            (grf_io_word(), 0x2e00, 0x2e00),
            (grf_hpd_io_en_word(), 0x3000, 0x3000),
            (grf_grant_word(Port::Hdmi0), 0x0400, 0x0400),
            (grf_grant_word(Port::Hdmi1), 0x1000, 0x1000),
            (grf_hpd_mask_word(Port::Hdmi0), 0x2000, 0x2000),
            (grf_hpd_mask_word(Port::Hdmi1), 0x8000, 0x8000),
            (grf_hpd_unmask_clear_word(Port::Hdmi0), 0x3000, 0x1000),
            (grf_hpd_unmask_clear_word(Port::Hdmi1), 0xc000, 0x4000),
            (vop_grf_con2_word(Port::Hdmi0), 0x0002, 0x0002),
            (vop_grf_con2_word(Port::Hdmi1), 0x0010, 0x0010),
            (vo1_con0_sync_word(Port::Hdmi0, 3), 0x0060, 0x0060),
            (vo1_con0_sync_word(Port::Hdmi1, 1), 0x0180, 0x0080),
        ];
        for (w, mask, bits) in words {
            assert_eq!(w, (mask << 16) | bits, "{w:#010x}");
        }
        // a field wider than 2 bits stays inside its place
        assert_eq!(vo1_con0_sync_word(Port::Hdmi0, 0xff), 0x0060_0060);
    }

    #[test]
    fn linux_sync_field_follows_vsync_only() {
        let m = WUXGA_60_RB; // +hsync -vsync
        assert_eq!(linux_sync_field(&m), 0);
        assert_eq!(linux_sync_field(&VideoMode { vsync_high: true, ..m }), 1);
        assert_eq!(linux_sync_field(&VideoMode { hsync_high: false, vsync_high: true, ..m }), 1);
    }

    #[test]
    fn bind_writes_the_glue_then_the_controller_then_the_hpd_word() {
        let s = Soc::new();
        let id = bind(&s, &B, Port::Hdmi0, &P).unwrap();
        assert_eq!(id.core_id, ID);
        assert_eq!(
            s.writes(),
            vec![
                (VO1 + 0x0c, 0x2e00_2e00),
                (SYS + 0x31c, 0x3000_3000),
                (VO1 + 0x24, 0x0400_0400),
                (SYS + 0x308, 0x2000_2000),
                (CTRL + 0x3014, 0),
                (CTRL + 0x3024, 0),
                (CTRL + 0x80, 428_571_429),
                (SYS + 0x308, 0x3000_1000),
            ]
        );
        // the HPD interrupt ends unmasked with the clear bit written
        assert_eq!(s.get(SYS + 0x308) & 0x3000, 0x1000);
    }

    #[test]
    fn port_1_uses_its_own_bits_and_con6() {
        let s = Soc::new();
        bind(&s, &B1, Port::Hdmi1, &P).unwrap();
        let w = s.writes();
        assert_eq!(w[0], (VO1 + 0x18, 0x2e00_2e00));
        assert_eq!(w[2], (VO1 + 0x24, 0x1000_1000));
        assert_eq!(w[3], (SYS + 0x308, 0x8000_8000));
        assert_eq!(w[7], (SYS + 0x308, 0xc000_4000));
    }

    /// The port follows the controller base (glue:467-477): a port that
    /// disagrees with it, or a base that is no controller, is refused before
    /// any write, so one controller is never bound with the other's GRF bits.
    #[test]
    fn a_port_that_is_not_the_controllers_is_refused_with_nothing_written() {
        for (b, port) in [(B, Port::Hdmi1), (B1, Port::Hdmi0), (Bases { ctrl: CTRL + 0x1000, ..B }, Port::Hdmi0)] {
            let s = Soc::new();
            s.set(b.ctrl, ID);
            let r = bind(&s, &b, port, &P);
            assert_eq!(
                r,
                Err(Refused { why: Refusal::PortMismatch, found: b.ctrl as u32 }),
                "{port:?} at {:#x}",
                b.ctrl
            );
            assert!(s.writes().is_empty(), "{:?}", s.writes());
        }
    }

    #[test]
    fn each_port_reads_its_own_hpd_level() {
        let s = Soc::new();
        for (level, hdmi0, hdmi1) in [(1u32 << 16, true, false), (1 << 24, false, true), (0, false, false)] {
            // the measured board value 0x240 under the level bits
            s.set(SYS + 0x384, 0x240 | level);
            assert_eq!((hpd(&s, SYS, Port::Hdmi0), hpd(&s, SYS, Port::Hdmi1)), (hdmi0, hdmi1), "{level:#x}");
            assert_eq!(status(&s, CTRL1, SYS, Port::Hdmi1).hpd, hdmi1);
        }
    }

    #[test]
    fn grf_bits_outside_the_masks_are_kept() {
        let s = Soc::new();
        s.set(VO1 + 0x0c, 0x0000_d1ff);
        s.set(SYS + 0x308, 0x0000_4fff);
        s.set(VO1 + 0x24, 0x0000_0bff);
        bind(&s, &B, Port::Hdmi0, &P).unwrap();
        assert_eq!(s.get(VO1 + 0x0c), 0xd1ff | 0x2e00);
        assert_eq!(s.get(VO1 + 0x24), 0x0fff);
        assert_eq!(s.get(SYS + 0x308), (0x4fff & !0x2000) | 0x1000);
    }

    #[test]
    fn a_wrong_id_is_refused_with_nothing_written() {
        for found in [0, 0xffff_ffff, ID ^ 1] {
            let s = Soc::new();
            s.set(CTRL, found);
            assert_eq!(bind(&s, &B, Port::Hdmi0, &P), Err(Refused { why: Refusal::WrongId, found }));
            assert!(s.writes().is_empty(), "{:?}", s.writes());
        }
    }

    #[test]
    fn the_timer_base_is_the_callers_and_is_read_back() {
        let s = Soc::new();
        bind(&s, &B, Port::Hdmi0, &BindParams { timer_base_hz: 200_000_000, ..P }).unwrap();
        assert_eq!(s.get(CTRL + 0x80), 200_000_000);

        let s = Soc { timer_dead: true, ..Soc::new() };
        assert_eq!(bind(&s, &B, Port::Hdmi0, &P), Err(Refused { why: Refusal::TimerReadBack, found: 0 }));
    }

    fn enable_from(found: u32, sink: Sink) -> (Soc, Result<(), Refused>) {
        let s = Soc::new();
        s.set(CTRL + 0x968, found);
        s.set(CTRL + 0x8e0, 0x0000_0100);
        let r = enable(&s, CTRL, sink);
        (s, r)
    }

    #[test]
    fn dvi_and_hdmi_differ_by_opmode_dvi_only() {
        for found in [0, 0xffff_ffff, 0x0000_0101, 0x0000_0010] {
            let (d, rd) = enable_from(found, Sink::Dvi);
            let (h, rh) = enable_from(found, Sink::Hdmi);
            assert_eq!((rd, rh), (Ok(()), Ok(())));
            assert_eq!(d.get(CTRL + 0x968), found | 0x10);
            assert_eq!(h.get(CTRL + 0x968), found & !0x10);
            assert_eq!(d.get(CTRL + 0x968) ^ h.get(CTRL + 0x968), 0x10);
            // the same register set in the same order, the bypass first
            let at = |s: &Soc| s.writes().iter().map(|w| w.0).collect::<Vec<_>>();
            assert_eq!(at(&d), vec![CTRL + 0x8e0, CTRL + 0x968]);
            assert_eq!(at(&d), at(&h));
            assert_eq!(d.get(CTRL + 0x8e0), 0x101);
            assert_eq!(h.get(CTRL + 0x8e0), 0x101);
        }
    }

    #[test]
    fn an_opmode_that_does_not_take_is_refused() {
        let s = Soc { opmode_stuck: true, ..Soc::new() };
        assert_eq!(enable(&s, CTRL, Sink::Dvi), Err(Refused { why: Refusal::EnableReadBack, found: 0 }));
        // stuck at the value asked for is not a refusal
        assert_eq!(enable(&s, CTRL, Sink::Hdmi), Ok(()));
    }

    #[test]
    fn tmds_rate_is_the_pixel_clock_up_to_340_mhz() {
        let m = VideoMode { pixel_clock_hz: 148_500_000, ..WUXGA_60_RB };
        assert_eq!(tmds_char_rate(&m), Ok(148_500_000));
        assert_eq!(tmds_char_rate(&VideoMode { pixel_clock_hz: 25_200_000, ..m }), Ok(25_200_000));
        // the T6's fixed mode, 1024x768 at 60 Hz
        assert_eq!(tmds_char_rate(&VideoMode { pixel_clock_hz: 65_000_000, ..m }), Ok(65_000_000));
        assert_eq!(tmds_char_rate(&VideoMode { pixel_clock_hz: 340_000_000, ..m }), Ok(340_000_000));
        let over = VideoMode { pixel_clock_hz: 340_000_001, ..m };
        assert_eq!(tmds_char_rate(&over), Err(Refused { why: Refusal::TmdsRate, found: 340_000_001 }));
        assert_eq!(tmds_char_rate(&VideoMode { pixel_clock_hz: 0, ..m }).unwrap_err().why, Refusal::TmdsRate);
    }

    #[test]
    fn status_reads_only_status_words() {
        let s = Soc::new();
        s.set(CTRL + 0xb0, 0xffff_ffd5); // 0x15 under the monitor mask
        s.set(CTRL + 0xb8, 1485);
        s.set(CTRL + 0xbc, 1486);
        s.set(CTRL + 0x968, 0x10);
        for i in 0..7 {
            s.set(CTRL + 0x884 + 4 * i, 0x100 + i as u32);
        }
        s.set(SYS + 0x384, 1 << 16);
        let st = status(&s, CTRL, SYS, Port::Hdmi0);
        assert!(st.clocks_locked() && st.dvi() && st.hpd);
        assert_eq!((st.vidqpclk_freq, st.linkqpclk_freq), (1485, 1486));
        assert_eq!(st.video_monitor, [0x100, 0x101, 0x102, 0x103, 0x104, 0x105, 0x106]);
        assert!(s.writes().is_empty());
        assert!(!status(&s, CTRL, SYS, Port::Hdmi1).hpd);

        s.set(CTRL + 0xb0, 0x17); // video clock off bit (1) set
        assert!(!status(&s, CTRL, SYS, Port::Hdmi0).clocks_locked());
    }

    #[test]
    fn no_step_reads_a_request_word_or_touches_the_i2c_master() {
        // the model asserts on each access; this runs every step through it
        let s = Soc::new();
        bind(&s, &B, Port::Hdmi0, &P).unwrap();
        enable(&s, CTRL, Sink::Dvi).unwrap();
        status(&s, CTRL, SYS, Port::Hdmi0);
        ident(&s, CTRL);
        hpd(&s, SYS, Port::Hdmi0);
        for a in s.reads().into_iter().chain(s.writes().into_iter().map(|w| w.0)) {
            if let Some(o) = Soc::ctrl_off(a) {
                assert!(![0x40, 0x974, 0xab0].contains(&o) && !(0xe0..=0x118).contains(&o), "{o:#x}");
            }
        }
    }
}
