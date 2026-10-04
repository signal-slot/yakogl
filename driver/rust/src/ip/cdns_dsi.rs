// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The Cadence MIPI DSI host controller ("DSITX") with TI's wrapper, as on
//! the AM62P (DSS_DSI0: controller 0x3050_0000, wrapper 0x3027_0000), and
//! its D-PHY ([`crate::ip::cdns_dphy`]): video timings, link bring-up,
//! direct (LP) commands.
//!
//! Written from the AM62Px TRM, TI SPRUJ83D (December 2023, revised January
//! 2026): 12.9.2 "MIPI Display Serial Interface (DSI) Controller", its
//! programming guide 12.9.2.7, and the register chapter 14.8.9.2
//! ([`crate::soc::am62p::regs::dsi`]). What the code relies on:
//!
//! - Order (12.9.2.7.1.3): "All DSI_VID_xxx registers must be programmed ...
//!   before the write to the DSI_VID_MAIN_CTL register. All
//!   DSI_MCTL_DPHY_xxx registers must be programmed ... before the write to
//!   the DSI_MCTL_MAIN_DATA_CTL register to set the [0] LINK_EN bit."
//! - Start-up (12.9.2.7.3, Figure 12-551): program the PLL and start it
//!   (PLL_START, "link has to be enabled": LINK_EN), wait for the PLL
//!   (MCTL_MAIN_STS PLL_LCK), program the physical line, enable the lanes
//!   (MCTL_MAIN_EN CKLANE_EN/DATn_EN), wait for them (CLKLANE_READY /
//!   DATn_READY), then the video settings and IFn_EN "so that the stall
//!   signal is removed". The D-PHY itself is configured "through the DPHY
//!   own APB interface" (12.9.2.7.1.2).
//! - Lanes (14.8.9.2.2.24-.25): MCTL_MAIN_PHY_CTL LANE2_EN..LANE4_EN enable
//!   the second to fourth lane; MCTL_MAIN_EN DAT1_EN..DAT4_EN start them;
//!   "the active lane configuration with the DSI_MCTL_MAIN_PHY_CTL and
//!   DSI_MCTL_MAIN_EN registers must be programmed to match" (12.9.2.7.3).
//! - WAIT_BURST_TIME: "Value 0 is forbidden"; "must be programmed to ensure
//!   that two HS bursts are separated by at least 100 ns ... based on the
//!   period of the tx_byte_clk cycle" (12.9.2.7.2.1); tx_byte_hs_clk = D-PHY
//!   bit rate / 8 (12.9.2.2, Table 12-412).
//! - Video input: VID_IF_SELECT 01 = DPI, IF2_EN = DPI interface; the
//!   wrapper's DSI_DPI_CONTROL DPI_0_EN "Enable for DPI-0 input".
//! - Timings (12.9.2.7.9): see [`Timings::from_mode`].
//! - Direct commands (12.9.2.7.5, Figure 12-552): clear the status, program
//!   DIRECT_CMD_MAIN_SETTINGS and WRDAT, write DIRECT_CMD_SEND (any value),
//!   wait for WRITE_COMPLETED. Only the data types the TRM lists may be
//!   sent: DCS write 0x05, 0x15, 0x39; generic write 0x03, 0x13, 0x23,
//!   0x29 (reads are not implemented). The FIFO holds 2^(DIRCMD_FIFO_DEPTH +
//!   2) bytes (IP_CONF).
//! - Identity: ID_REG resets to vendor CADh, product D5h.
//!
//! Not stated in the TRM, and how it is handled:
//!
//! - MCTL_DPHY_CFG0: each bit "drives dphy_*_pdn / _pso / _rstb output";
//!   their polarity and order are not given. The three values come from
//!   TI's vendored Zephyr driver ([`crate::soc::am62p::ti_pr114866`]):
//!   before the D-PHY is programmed, after it (then PLL_LCK), and with the
//!   lane resets released.
//! - The HS TX / LP RX time-out units (MCTL_DPHY_TIMEOUT1/2: "time-out
//!   detection value", "clock division ratio"): written at their largest,
//!   so they only flag a link that is stuck for good.
//! - MCTL_ULPOUT_TIME is in system clock cycles, 9 bits per lane type,
//!   while 12.9.2.7.2.1 says the D-PHY needs 1 ms to leave ULP: this driver
//!   never requests ULP, so both fields are written at their largest.
//! - REG_WAKEUP_TIME ("estimated time [in clock cycles] to perform LP->HS"):
//!   no value given; a tenth of a line, as the replaced driver had it. The
//!   controller uses it for LP blanking only (REG_BLKLINE_MODE /
//!   REG_BLKEOL_MODE = 1x), which this driver does not select.
//! - MAX_LINE_LIMIT (VID_VCA_SETTING2, "maximum size of the line packet
//!   [packet that fits in blanking line]"): the blanking packet's payload
//!   less one packet's header and CRC (6 bytes, 12.9.2.7.9.2).

use crate::env::Env;
use crate::errno::{EBUSY, EINVAL, EMSGSIZE, ENODEV, ENOTSUP, ETIMEDOUT};
use crate::ip::cdns_dphy::{self as dphy, Pll};
use crate::mmio::{field, get, Mmio};
use crate::soc::am62p::regs::{dsi as r, dsi_wrap as w};
use crate::soc::am62p::ti_pr114866 as ti;
use crate::video::VideoMode;
use crate::ylog;

/// Lane waits: the TRM gives no number; the PLL 1 ms, the lanes 500 ms (the
/// replaced driver's bounds), the direct command 100 ms.
pub const PLL_WAIT_US: u32 = 1_000;
pub const LANES_WAIT_US: u32 = 500_000;
pub const CMD_WAIT_US: u32 = 100_000;
const POLL_STEP_US: u32 = 10;

/// Packet overheads in bytes, 12.9.2.7.9.2-.3: a short packet (HSS, HSE,
/// VSS, VSE) is 4 bytes, a long packet adds a 4-byte header and a 2-byte CRC
/// to its payload.
const SHORT_PKT: u32 = 4;
const LONG_PKT: u32 = 4 + 2;
/// The EoT packet (12.9.2.7.9.3, "4 for EOT").
const EOT_PKT: u32 = 4;

/// The register bases of one DSI host.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Bases {
    pub dsi: usize,
    pub wrap: usize,
    pub dphy: usize,
}

/// What attach is asked for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Request {
    pub mode: VideoMode,
    /// 16 or 24.
    pub bpp: u32,
    /// 1..4.
    pub lanes: u32,
    /// Non-burst with sync pulses; else non-burst with sync events.
    pub sync_pulse: bool,
    /// HOST_EOT_GEN: an EoT packet after each HS transmission.
    pub eot: bool,
    /// The packets' data type (VID_MAIN_CTL HEADER, a MIPI DSI data type).
    pub datatype: u32,
    pub pll_ref_hz: u32,
    pub psm_hz: u32,
}

/// The video timing registers for one mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Timings {
    /// Payload bytes (12.9.2.7.9).
    pub hsa: u32,
    pub hbp: u32,
    pub hact: u32,
    pub hfp: u32,
    /// HLINE x bpp / 8: the bytes of one line.
    pub line: u32,
    pub blkline_event: u32,
    pub blkline_pulse: u32,
    /// tx_byte_clk cycles.
    pub line_duration: u32,
    pub wakeup: u32,
    pub vsa: u32,
    pub vbp: u32,
    pub vfp: u32,
    pub vact: u32,
    pub max_line: u32,
    /// The lane rate the pixel stream needs, bit/s.
    pub rate: u64,
}

fn div_up(a: u32, b: u32) -> u32 {
    a.div_ceil(b)
}

impl Timings {
    /// 12.9.2.7.9 "Programming the DSITX Controller to Match the Incoming DPI
    /// Stream". Horizontal sizes are payload bytes: a mode's pixels x bpp/8
    /// less the packets the controller adds around them:
    ///
    /// - sync pulses (12.9.2.7.9.2): HSA - 14 (HSS 4, blanking header and
    ///   CRC 6, HSE 4), HBP - 12 (blanking 6, active packet 6), HFP - 6;
    /// - sync events (12.9.2.7.9.3): HSA 0, HBP = (HSA + HBP) - 16: the TRM's
    ///   text says "- 12", but its own line sum ("VACT = HSSPkt + HBPPkt +
    ///   HACTPkt + HFPPkt", which must equal HLINE x bpp/8) needs the HSS
    ///   packet as well: 4 + 6 + 6 = 16 (its worked example's 24 does not
    ///   add up either);
    /// - BLKLINE_EVENT_PCK = HLINE x bpp/8 - 10, BLKLINE_PULSE_PCK = HLINE x
    ///   bpp/8 - 20 - HSA;
    /// - REG_LINE_DURATION = ceil(HLINE x bpp/8 / lanes) less the sync part
    ///   (pulses: ceil(DPI_HSA x bpp/8 / lanes); events: ceil(4 / lanes))
    ///   and, with EoT, ceil(4 / lanes);
    /// - vertical: "VSA = DPI_VSI (lines, minimum of 2), VBP = DPI_VBP, VFP <
    ///   DPI_VFP (minimum of 1)": one line less of VFP, so the controller
    ///   waits in LP for the next DPI VSYNC.
    ///
    /// "(DPI_HACT x bpp/32) must be an integer." -EINVAL for a mode that
    /// breaks a minimum or does not fit the fields.
    pub fn from_mode(req: &Request) -> Result<Timings, i32> {
        let m = &req.mode;
        let bpp = req.bpp;
        if !matches!(bpp, 16 | 24) || !(1..=4).contains(&req.lanes) || m.pixel_clock_hz == 0 {
            return Err(-EINVAL);
        }
        if !(m.hactive * bpp).is_multiple_of(32) || m.vsync < 2 || m.vfront < 2 {
            return Err(-EINVAL);
        }
        let bytes = |px: u32| px * bpp / 8;
        let less = |a: u32, b: u32| a.checked_sub(b).ok_or(-EINVAL);
        let (hsa, hbp) = if req.sync_pulse {
            (less(bytes(m.hsync), SHORT_PKT + LONG_PKT + SHORT_PKT)?, less(bytes(m.hback), 2 * LONG_PKT)?)
        } else {
            (0, less(bytes(m.hsync + m.hback), SHORT_PKT + 2 * LONG_PKT)?)
        };
        let hact = bytes(m.hactive);
        let hfp = less(bytes(m.hfront), LONG_PKT)?;
        let line = bytes(m.htotal());
        let lanes = req.lanes;
        let total = div_up(line, lanes);
        let eot = if req.eot { div_up(EOT_PKT, lanes) } else { 0 };
        let sync = if req.sync_pulse { div_up(bytes(m.hsync), lanes) } else { div_up(SHORT_PKT, lanes) };
        let line_duration = less(total, sync + eot)?;
        let blkline_event = less(line, SHORT_PKT + LONG_PKT)?;
        let blkline_pulse = less(line, 20 + hsa)?;
        let max_line = less(if req.sync_pulse { blkline_pulse } else { blkline_event }, LONG_PKT)?;
        let t = Timings {
            hsa,
            hbp,
            hact,
            hfp,
            line,
            blkline_event,
            blkline_pulse,
            line_duration,
            wakeup: line / lanes / 10,
            vsa: m.vsync,
            vbp: m.vback,
            vfp: m.vfront - 1,
            vact: m.vactive,
            max_line,
            rate: u64::from(m.pixel_clock_hz) * u64::from(bpp) / u64::from(lanes),
        };
        let fits = |v: u32, mask: u32| v <= get(mask, mask);
        let ok = fits(t.hsa, r::HSA_LENGTH)
            && fits(t.hbp, r::HBP_LENGTH)
            && fits(t.hfp, r::HFP_LENGTH)
            && fits(t.hact, r::RGB_SIZE)
            && fits(t.blkline_event, r::BLKLINE_EVENT_PCK)
            && fits(t.blkline_pulse, r::BLKLINE_PULSE_PCK)
            && fits(t.line_duration, r::REG_LINE_DURATION)
            && fits(t.wakeup, r::REG_WAKEUP_TIME)
            && fits(t.vsa, r::VSA_LENGTH)
            && fits(t.vbp, r::VBP_LENGTH)
            && fits(t.vfp, r::VFP_LENGTH)
            && fits(t.vact, r::VACT_LENGTH)
            && fits(t.max_line, r::MAX_LINE_LIMIT);
        if !ok {
            return Err(-EINVAL);
        }
        Ok(t)
    }
}

/// WAIT_BURST_TIME for a lane rate: tx_byte_clk cycles in 100 ns, rounded
/// up, at least 1 ("0 is forbidden"), at most the field.
pub fn wait_burst_time(rate: u64) -> u32 {
    let cycles = (rate * 100).div_ceil(8_000_000_000);
    cycles.clamp(1, u64::from(get(r::WAIT_BURST_TIME, r::WAIT_BURST_TIME))) as u32
}

fn rd(m: &impl Mmio, base: usize, off: u32) -> u32 {
    m.read32(base + off as usize)
}

fn wr(m: &impl Mmio, base: usize, off: u32, val: u32) {
    m.write32(base + off as usize, val)
}

fn wait(m: &impl Mmio, env: &impl Env, base: usize, off: u32, mask: u32, us: u32) -> bool {
    let mut t = 0;
    loop {
        if rd(m, base, off) & mask == mask {
            return true;
        }
        if t >= us {
            return false;
        }
        env.busy_wait_us(POLL_STEP_US);
        t += POLL_STEP_US;
    }
}

/// Identify the controller and the D-PHY, and quiesce the controller: link,
/// lanes, interface and every interrupt source off. Returns the D-PHY's WIZ
/// base ([`dphy::find_wiz`]).
pub fn init(m: &impl Mmio, env: &impl Env, b: &Bases) -> Result<u32, i32> {
    let id = rd(m, b.dsi, r::ID_REG);
    if get(r::REV_VENDOR_ID, id) != r::VENDOR_CADENCE || get(r::REV_PRODUCT_ID, id) != r::PRODUCT_DSI {
        ylog!(env, Err, "DSI: ID_REG {:#010x} is not the Cadence DSI host", id);
        return Err(-ENODEV);
    }
    ylog!(env, Inf, "Cadence DSI id {:#010x}, IP_CONF {:#010x}", id, rd(m, b.dsi, r::IP_CONF));
    for off in [
        r::MCTL_MAIN_DATA_CTL,
        r::MCTL_MAIN_EN,
        r::MCTL_MAIN_PHY_CTL,
        r::MCTL_MAIN_STS_CTL,
        r::MCTL_DPHY_ERR_CTL1,
        r::CMD_MODE_STS_CTL,
        r::DIRECT_CMD_STS_CTL,
        r::DIRECT_CMD_RD_STS_CTL,
        r::VID_MODE_STS_CTL,
        r::TVG_STS_CTL,
    ] {
        wr(m, b.dsi, off, 0);
    }
    dphy::probe(m, env, b.dphy)
}

/// What a link that came up runs at.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Link {
    pub pll: Pll,
    pub timings: Timings,
}

/// Bring the link up and start video from the DPI input (module text for
/// the order). `wiz` is [`init`]'s result.
pub fn attach(m: &impl Mmio, env: &impl Env, b: &Bases, wiz: u32, req: &Request) -> Result<Link, i32> {
    let t = Timings::from_mode(req)?;
    let pll = dphy::pll_for(u64::from(req.pll_ref_hz), t.rate)?;
    let lanes = req.lanes;
    let ip_lanes = get(r::IP_CONF_MAX_LANE_NB, rd(m, b.dsi, r::IP_CONF)) + 1;
    if lanes > ip_lanes {
        return Err(-EINVAL);
    }
    let data_lanes = (1u32 << lanes) - 1;

    // The physical line, before LINK_EN (12.9.2.7.1.3).
    wr(m, b.dsi, r::MCTL_DPHY_TIMEOUT1, r::HSTX_TO_VAL | r::CLK_DIV);
    wr(m, b.dsi, r::MCTL_DPHY_TIMEOUT2, r::LPRX_TO_VAL);
    wr(m, b.dsi, r::MCTL_ULPOUT_TIME, r::DATA_ULPOUT_TIME | r::CKLANE_ULPOUT_TIME);
    wr(
        m,
        b.dsi,
        r::MCTL_MAIN_PHY_CTL,
        r::CLK_CONTINUOUS
            | field(r::WAIT_BURST_TIME, wait_burst_time(pll.rate))
            | ((data_lanes >> 1) << r::LANE2_EN.trailing_zeros()),
    );
    // Link and PLL start (Figure 12-551).
    wr(m, b.dsi, r::MCTL_MAIN_DATA_CTL, r::LINK_EN);
    wr(m, b.dsi, r::MCTL_MAIN_EN, r::PLL_START);

    // The D-PHY: powered down while it is programmed through its own APB.
    wr(m, b.dsi, r::MCTL_DPHY_CFG0, ti::CFG0_SEQUENCE[0]);
    dphy::power_on(m, env, b.dphy, wiz, &pll, req.psm_hz)?;
    wr(m, b.dsi, r::MCTL_MAIN_STS_CLR, r::PLL_LOCK_CLR);
    wr(m, b.dsi, r::MCTL_DPHY_CFG0, ti::CFG0_SEQUENCE[1]);
    if !wait(m, env, b.dsi, r::MCTL_MAIN_STS, r::STS_PLL_LCK, PLL_WAIT_US) {
        ylog!(env, Err, "DSI: PLL_LCK not set (MCTL_MAIN_STS {:#010x})", rd(m, b.dsi, r::MCTL_MAIN_STS));
        return Err(-ETIMEDOUT);
    }

    // Lanes: resets released, started, ready.
    let d_rstb = data_lanes * ti::CFG0_D_RSTB_LANE0;
    wr(m, b.dsi, r::MCTL_DPHY_CFG0, ti::CFG0_SEQUENCE[2] | d_rstb);
    m.modify32(
        b.dsi + r::MCTL_MAIN_EN as usize,
        r::CKLANE_EN | (0xf << r::DAT1_EN.trailing_zeros()),
        r::CKLANE_EN | (data_lanes << r::DAT1_EN.trailing_zeros()),
    );
    let ready = r::STS_CLKLANE_READY | (data_lanes << r::STS_DAT1_READY.trailing_zeros());
    if !wait(m, env, b.dsi, r::MCTL_MAIN_STS, ready, LANES_WAIT_US) {
        ylog!(env, Err, "DSI: lanes not ready (MCTL_MAIN_STS {:#010x})", rd(m, b.dsi, r::MCTL_MAIN_STS));
        return Err(-ETIMEDOUT);
    }

    // Video: every VID_* before VID_MAIN_CTL (12.9.2.7.1.3).
    wr(m, b.dsi, r::VID_HSIZE1, field(r::HBP_LENGTH, t.hbp) | field(r::HSA_LENGTH, t.hsa));
    wr(m, b.dsi, r::VID_HSIZE2, field(r::HFP_LENGTH, t.hfp) | field(r::RGB_SIZE, t.hact));
    wr(
        m,
        b.dsi,
        r::VID_VSIZE1,
        field(r::VFP_LENGTH, t.vfp) | field(r::VBP_LENGTH, t.vbp) | field(r::VSA_LENGTH, t.vsa),
    );
    wr(m, b.dsi, r::VID_VSIZE2, field(r::VACT_LENGTH, t.vact));
    wr(m, b.dsi, r::VID_BLKSIZE1, field(r::BLKLINE_EVENT_PCK, t.blkline_event));
    wr(m, b.dsi, r::VID_BLKSIZE2, field(r::BLKLINE_PULSE_PCK, t.blkline_pulse));
    wr(m, b.dsi, r::VID_VCA_SETTING2, field(r::MAX_LINE_LIMIT, t.max_line));
    wr(m, b.dsi, r::VID_DPHY_TIME, field(r::REG_WAKEUP_TIME, t.wakeup) | field(r::REG_LINE_DURATION, t.line_duration));
    let pixel_mode = if req.bpp == 16 { r::PIXEL_MODE_RGB565 } else { r::PIXEL_MODE_RGB888 };
    let mut ctl = field(r::VID_PIXEL_MODE, pixel_mode)
        | field(r::VID_HEADER, req.datatype)
        | field(r::REG_BLKLINE_MODE, r::BLK_MODE_BLANKING_PACKET)
        | field(r::REG_BLKEOL_MODE, r::BLK_MODE_BLANKING_PACKET)
        | r::VID_IGNORE_MISS_VSYNC;
    if req.sync_pulse {
        ctl |= r::SYNC_PULSE_ACTIVE | r::SYNC_PULSE_HORIZONTAL;
    }
    wr(m, b.dsi, r::VID_MAIN_CTL, ctl);

    // The DPI input: the wrapper's enable, video mode on DPI, the stall off.
    wr(m, b.wrap, w::DPI_CONTROL, w::DPI_0_EN);
    let mut data = r::LINK_EN | r::SDI_IF_VID_MODE | field(r::VID_IF_SELECT, r::VID_IF_DPI) | r::VID_EN;
    if req.eot {
        data |= r::HOST_EOT_GEN;
    }
    wr(m, b.dsi, r::MCTL_MAIN_DATA_CTL, data);
    m.modify32(b.dsi + r::MCTL_MAIN_EN as usize, r::IF2_EN, r::IF2_EN);

    ylog!(
        env,
        Inf,
        "DSI link up: {} lanes, {}, line {} B, {} bit/s per lane",
        lanes,
        if req.sync_pulse { "sync pulses" } else { "sync events" },
        t.line,
        pll.rate
    );
    Ok(Link { pll, timings: t })
}

/// Stop video and the link (the PHY stays powered; the next attach
/// reprograms it).
pub fn detach(m: &impl Mmio, b: &Bases) {
    wr(m, b.dsi, r::MCTL_MAIN_DATA_CTL, 0);
    wr(m, b.dsi, r::MCTL_MAIN_EN, 0);
    wr(m, b.wrap, w::DPI_CONTROL, 0);
}

/// Data types a direct write may use (12.9.2.7.5) and whether each is a
/// long packet.
fn write_type(datatype: u8) -> Option<bool> {
    match datatype {
        0x05 | 0x15 | 0x03 | 0x13 | 0x23 => Some(false),
        0x39 | 0x29 => Some(true),
        _ => None,
    }
}

/// Send one direct write command (LP when `lp`) on virtual channel
/// `channel`. Returns the bytes sent. Reads are -ENOTSUP.
pub fn transfer(
    m: &impl Mmio,
    env: &impl Env,
    b: &Bases,
    channel: u8,
    datatype: u8,
    lp: bool,
    tx: &[u8],
) -> Result<usize, i32> {
    let long = write_type(datatype).ok_or(-ENOTSUP)?;
    let fifo = 1usize << (get(r::IP_CONF_DIRCMD_FIFO_DEPTH, rd(m, b.dsi, r::IP_CONF)) + 2);
    if channel > 3 || (!long && tx.len() > 2) || tx.len() > fifo || tx.len() > 0xff {
        return Err(-EMSGSIZE);
    }
    if rd(m, b.dsi, r::DIRECT_CMD_STS) & 1 != 0 {
        return Err(-EBUSY); // CMD_TRANSMISSION: one is being sent
    }
    wr(m, b.dsi, r::DIRECT_CMD_STS_CLR, u32::MAX);
    // "Writing in that register (any value) triggers the command sending"
    // (12.9.2.7.5); the FIFO reset is the same kind of write-only register
    // (14.8.9.2.2.47 states no value), written the same way.
    wr(m, b.dsi, r::DIRECT_CMD_FIFO_RST, 0);
    let mut s = field(r::CMD_NAT, 0) // write command
        | field(r::CMD_HEAD, datatype.into())
        | field(r::CMD_ID, channel.into())
        | field(r::CMD_SIZE, tx.len() as u32);
    if long {
        s |= r::CMD_LONGNOTSHORT;
    }
    if lp {
        s |= r::CMD_LP_EN;
    }
    wr(m, b.dsi, r::DIRECT_CMD_MAIN_SETTINGS, s);
    for chunk in tx.chunks(4) {
        let word = chunk.iter().enumerate().fold(0u32, |w, (i, &x)| w | u32::from(x) << (8 * i));
        wr(m, b.dsi, r::DIRECT_CMD_WRDAT, word);
    }
    wr(m, b.dsi, r::DIRECT_CMD_SEND, 0);
    if !wait(m, env, b.dsi, r::DIRECT_CMD_STS, r::CMD_STS_WRITE_COMPLETED, CMD_WAIT_US) {
        return Err(-ETIMEDOUT);
    }
    Ok(tx.len())
}

#[cfg(test)]
mod tests;
