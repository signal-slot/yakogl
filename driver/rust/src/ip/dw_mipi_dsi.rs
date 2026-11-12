// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The Synopsys DesignWare MIPI DSI host controller ("DSI", 0x4800_0000 on the
//! STM32MP25, IP version 1.41) driving a video-mode link, with the PHY behind
//! [`Phy`] (on the MP25: ST's wrapper and PLL, `ip::stm32_dsi_phy`).
//!
//! Facts from Linux v6.18 (commit 7d0a66e4bb90; GPL-2.0: facts and file:line
//! only, nothing copied), `drivers/gpu/drm/bridge/synopsys/dw-mipi-dsi.c`,
//! cited as `dw-mipi-dsi.c:N`. Every register write below names the line of
//! that file it reproduces. Where ST's tree (STMicroelectronics/linux branch
//! `v6.6-stm32mp`, commit 548f960c059b, the tree the pre-study read) differs
//! it is cited as `ST dw-mipi-dsi.c:N` or `ST dw_mipi_dsi-stm.c:N`.
//!
//! The order is `dw_mipi_dsi_mode_set` (dw-mipi-dsi.c:979-1023), then
//! `atomic_enable` (:1046-1055):
//!
//! 1. the PHY's lane rate (`phy_ops->get_lane_mbps`, :989);
//! 2. `PWR_UP` 0 and `CLKMGR_CFG`: the TX escape clock divider
//!    `(lane_mbps >> 3) / esc + 1` with esc 20 MHz unless the PHY says
//!    otherwise, the timeout divider 0 (:656-692);
//! 3. DPI: virtual channel, colour coding, sync polarities (:694-722);
//! 4. packet handler: CRC and ECC reception, BTA, and the EoTp unless the
//!    device asks for none (:724-732);
//! 5. video mode: low-power in every blanking region, burst, non-burst
//!    with sync pulses or with sync events (:598-626); the video packet
//!    size is the active width (:734-749);
//! 6. command mode: the HS TX and LP RX timeouts 0, the BTA timeout 0xd00,
//!    `MODE_CFG` command mode (:751-766);
//! 7. horizontal timings in lane byte clocks (:776-831): from the pixel
//!    clock in non-burst mode, `ceil(x * bpp / (lanes * 8))`, from the lane
//!    rate in burst mode, never under 10, 5, 4, 3 for 1-4 lanes (:768);
//!    vertical timings in lines (:833-847);
//! 8. the PHY: shut down and reset, test interface cleared (:899-907), its
//!    transition times (`phy_ops->get_timing`) in `PHY_TMR_CFG` (from IP
//!    1.31 on, with the read time in `PHY_TMR_RD_CFG`) and
//!    `PHY_TMR_LPCLK_CFG` (:849-886), `PHY_IF_CFG` lanes and a stop wait of
//!    0x20 (:888-897); the interrupt status read and masked (:929-935);
//!    `phy_ops->init` (:1010); `PHY_RSTZ` all on, then `PHY_STATUS` lock and
//!    clock-lane stop state, 10 ms each (:909-927);
//! 9. two frames (:288-295, :1016), then command mode with the clock lane
//!    requested in HS (continuous unless the device allows otherwise) and
//!    `PWR_UP` 1 (:628-648), and `phy_ops->power_on` (:1021);
//! 10. [`video_on`]: the same switch to video mode (:1052).
//!
//! Where this driver differs from Linux, and why:
//!
//! - Linux only logs a PHY that does not lock or a clock lane that does not
//!   reach the stop state (:919-926) and carries on; [`attach`] stops there
//!   with `-ETIMEDOUT` and the step, so a dead link is reported where it
//!   died. A PHY whose lock is not reported in `PHY_STATUS` says so
//!   ([`Phy::lock_in_phy_status`]); the lock wait is then skipped.
//! - Linux carries on when a `phy_ops` hook fails (:991, :1011, :859); here
//!   the error stops the attach.
//! - `VID_NUM_CHUNKS` and `VID_NULL_SIZE`: Linux never writes them (its TODO
//!   at :737-742 names them) and so relies on their reset value; this
//!   driver writes that value, 0, so a link an earlier stage left set up
//!   cannot keep chunks Linux would not have.
//! - `DATAEN_ACTIVE_LOW`: mainline never sets it; ST's tree sets it for a
//!   connector whose bus flags say DE low (ST dw-mipi-dsi.c:695-696). It is
//!   set here from [`VideoMode::de_high`].
//! - Linux's burst-mode line time multiplies in 32 bits (:785); here in 64.
//! - The version: Linux's ST glue accepts IP 1.30, 1.31 and 1.41 only
//!   (ST dw_mipi_dsi-stm.c:1344-1351); [`probe`] does the same.
//!
//! Not used on the ADV7535 path, kept minimal: command-mode writes
//! ([`transfer`], :374-465, generic and DCS writes as drm_mipi_dsi.c builds
//! them); reads, dual-link and the video pattern generator are not
//! implemented. The ADV7535 itself is set up over I2C (`ip::adv7535`).

use crate::env::Env;
use crate::errno::{EINVAL, ENODEV, ETIMEDOUT};
use crate::mmio::Mmio;
use crate::video::VideoMode;
use crate::ylog;

/// Register offsets and bits (dw-mipi-dsi.c:35-224).
pub mod regs {
    pub const HWVER_130: u32 = 0x3133_3000;
    pub const HWVER_131: u32 = 0x3133_3100;
    /// ST dw_mipi_dsi-stm.c:29, the MP25's.
    pub const HWVER_141: u32 = 0x3134_3100;

    pub const VERSION: u32 = 0x00;
    pub const VERSION_MASK: u32 = 0xffff_ff00;
    pub const PWR_UP: u32 = 0x04;
    pub const RESET: u32 = 0;
    pub const POWERUP: u32 = 1 << 0;
    pub const CLKMGR_CFG: u32 = 0x08;
    pub const DPI_VCID: u32 = 0x0c;
    pub const DPI_COLOR_CODING: u32 = 0x10;
    pub const LOOSELY18_EN: u32 = 1 << 8;
    pub const DPI_COLOR_CODING_16BIT_1: u32 = 0x0;
    pub const DPI_COLOR_CODING_18BIT_1: u32 = 0x3;
    pub const DPI_COLOR_CODING_18BIT_2: u32 = 0x4;
    pub const DPI_COLOR_CODING_24BIT: u32 = 0x5;
    pub const DPI_CFG_POL: u32 = 0x14;
    pub const HSYNC_ACTIVE_LOW: u32 = 1 << 2;
    pub const VSYNC_ACTIVE_LOW: u32 = 1 << 1;
    pub const DATAEN_ACTIVE_LOW: u32 = 1 << 0;
    pub const DPI_LP_CMD_TIM: u32 = 0x18;
    pub const PCKHDL_CFG: u32 = 0x2c;
    pub const CRC_RX_EN: u32 = 1 << 4;
    pub const ECC_RX_EN: u32 = 1 << 3;
    pub const BTA_EN: u32 = 1 << 2;
    pub const EOTP_TX_EN: u32 = 1 << 0;
    pub const MODE_CFG: u32 = 0x34;
    pub const ENABLE_VIDEO_MODE: u32 = 0;
    pub const ENABLE_CMD_MODE: u32 = 1 << 0;
    pub const VID_MODE_CFG: u32 = 0x38;
    pub const ENABLE_LOW_POWER: u32 = 0x3f << 8;
    pub const VID_MODE_TYPE_NON_BURST_SYNC_PULSES: u32 = 0x0;
    pub const VID_MODE_TYPE_NON_BURST_SYNC_EVENTS: u32 = 0x1;
    pub const VID_MODE_TYPE_BURST: u32 = 0x2;
    pub const ENABLE_LOW_POWER_CMD: u32 = 1 << 15;
    pub const VID_PKT_SIZE: u32 = 0x3c;
    pub const VID_PKT_SIZE_MASK: u32 = 0x3fff;
    pub const VID_NUM_CHUNKS: u32 = 0x40;
    pub const VID_NULL_SIZE: u32 = 0x44;
    pub const VID_HSA_TIME: u32 = 0x48;
    pub const VID_HBP_TIME: u32 = 0x4c;
    pub const VID_HLINE_TIME: u32 = 0x50;
    pub const VID_VSA_LINES: u32 = 0x54;
    pub const VID_VBP_LINES: u32 = 0x58;
    pub const VID_VFP_LINES: u32 = 0x5c;
    pub const VID_VACTIVE_LINES: u32 = 0x60;
    pub const CMD_MODE_CFG: u32 = 0x68;
    pub const ACK_RQST_EN: u32 = 1 << 1;
    /// MAX_RD_PKT_SIZE_LP | DCS_LW/SR_0P/SW_1P/SW_0P_TX_LP | GEN_LW/SR_2P..0P/SW_2P..0P_TX_LP
    /// (dw-mipi-dsi.c:120-146).
    pub const CMD_MODE_ALL_LP: u32 = (1 << 24) | (0xf << 16) | (0x7f << 8);
    pub const GEN_HDR: u32 = 0x6c;
    pub const GEN_PLD_DATA: u32 = 0x70;
    pub const CMD_PKT_STATUS: u32 = 0x74;
    pub const GEN_PLD_W_EMPTY: u32 = 1 << 2;
    pub const GEN_PLD_W_FULL: u32 = 1 << 3;
    pub const GEN_CMD_FULL: u32 = 1 << 1;
    pub const GEN_CMD_EMPTY: u32 = 1 << 0;
    pub const TO_CNT_CFG: u32 = 0x78;
    pub const BTA_TO_CNT: u32 = 0x8c;
    pub const LPCLK_CTRL: u32 = 0x94;
    pub const AUTO_CLKLANE_CTRL: u32 = 1 << 1;
    pub const PHY_TXREQUESTCLKHS: u32 = 1 << 0;
    pub const PHY_TMR_LPCLK_CFG: u32 = 0x98;
    pub const PHY_TMR_CFG: u32 = 0x9c;
    pub const PHY_RSTZ: u32 = 0xa0;
    pub const PHY_ENFORCEPLL: u32 = 1 << 3;
    pub const PHY_ENABLECLK: u32 = 1 << 2;
    pub const PHY_UNRSTZ: u32 = 1 << 1;
    pub const PHY_UNSHUTDOWNZ: u32 = 1 << 0;
    pub const PHY_IF_CFG: u32 = 0xa4;
    pub const PHY_STATUS: u32 = 0xb0;
    pub const PHY_STOP_STATE_CLK_LANE: u32 = 1 << 2;
    pub const PHY_LOCK: u32 = 1 << 0;
    pub const PHY_TST_CTRL0: u32 = 0xb4;
    pub const PHY_TESTCLR: u32 = 1 << 0;
    pub const INT_ST0: u32 = 0xbc;
    pub const INT_ST1: u32 = 0xc0;
    pub const INT_MSK0: u32 = 0xc4;
    pub const INT_MSK1: u32 = 0xc8;
    pub const PHY_TMR_RD_CFG: u32 = 0xf4;

    /// TO_CLK_DIVISION | TX_ESC_CLK_DIVISION (dw-mipi-dsi.c:45-46).
    pub const fn clkmgr_cfg(to_div: u32, esc_div: u32) -> u32 {
        ((to_div & 0xff) << 8) | (esc_div & 0xff)
    }
    /// OUTVACT_LPCMD_TIME | INVACT_LPCMD_TIME (:68-69).
    pub const fn dpi_lp_cmd_tim(outvact: u32, invact: u32) -> u32 {
        ((outvact & 0xff) << 16) | (invact & 0xff)
    }
    /// HSTX_TO_CNT | LPRX_TO_CNT (:161-162).
    pub const fn to_cnt_cfg(hstx: u32, lprx: u32) -> u32 {
        ((hstx & 0xffff) << 16) | (lprx & 0xffff)
    }
    /// PHY_CLKHS2LP_TIME | PHY_CLKLP2HS_TIME (:175-176).
    pub const fn phy_tmr_lpclk_cfg(hs2lp: u32, lp2hs: u32) -> u32 {
        ((hs2lp & 0x3ff) << 16) | (lp2hs & 0x3ff)
    }
    /// PHY_HS2LP_TIME | PHY_LP2HS_TIME | MAX_RD_TIME, before IP 1.31 (:179-181).
    pub const fn phy_tmr_cfg(hs2lp: u32, lp2hs: u32, max_rd: u32) -> u32 {
        ((hs2lp & 0xff) << 24) | ((lp2hs & 0xff) << 16) | (max_rd & 0x7fff)
    }
    /// PHY_HS2LP_TIME_V131 | PHY_LP2HS_TIME_V131 (:182-183).
    pub const fn phy_tmr_cfg_v131(hs2lp: u32, lp2hs: u32) -> u32 {
        ((hs2lp & 0x3ff) << 16) | (lp2hs & 0x3ff)
    }
    /// PHY_STOP_WAIT_TIME | N_LANES (:196-197).
    pub const fn phy_if_cfg(stop_wait: u32, lanes: u32) -> u32 {
        ((stop_wait & 0xff) << 8) | (lanes.wrapping_sub(1) & 0x3)
    }
}

use regs as r;

/// `mode_flags` bits of a DSI device (Linux include/drm/drm_mipi_dsi.h:116-140).
pub mod flags {
    pub const MODE_VIDEO: u32 = 1 << 0;
    pub const MODE_VIDEO_BURST: u32 = 1 << 1;
    pub const MODE_VIDEO_SYNC_PULSE: u32 = 1 << 2;
    pub const MODE_VIDEO_HSE: u32 = 1 << 4;
    pub const MODE_NO_EOT_PACKET: u32 = 1 << 9;
    pub const CLOCK_NON_CONTINUOUS: u32 = 1 << 10;
    /// What the ADV7533/7535 asks of its host (adv7533.c:152-153): video,
    /// non-burst with sync pulses, no EoT packet, HSE packets (the last has
    /// no effect on this host: dw-mipi-dsi.c never reads it).
    pub const ADV7535: u32 = MODE_VIDEO | MODE_VIDEO_SYNC_PULSE | MODE_NO_EOT_PACKET | MODE_VIDEO_HSE;
}

/// `enum mipi_dsi_pixel_format`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Rgb888,
    Rgb666,
    Rgb666Packed,
    Rgb565,
}

impl Format {
    /// `mipi_dsi_pixel_format_to_bpp`.
    pub const fn bpp(self) -> u32 {
        match self {
            Format::Rgb888 => 24,
            Format::Rgb666 | Format::Rgb666Packed => 18,
            Format::Rgb565 => 16,
        }
    }

    /// `DPI_COLOR_CODING` (dw-mipi-dsi.c:699-712).
    pub const fn color_coding(self) -> u32 {
        match self {
            Format::Rgb888 => r::DPI_COLOR_CODING_24BIT,
            Format::Rgb666 => r::DPI_COLOR_CODING_18BIT_2 | r::LOOSELY18_EN,
            Format::Rgb666Packed => r::DPI_COLOR_CODING_18BIT_1,
            Format::Rgb565 => r::DPI_COLOR_CODING_16BIT_1,
        }
    }
}

/// What the device on the link asks for (`struct mipi_dsi_device`'s lanes,
/// channel, format, mode_flags) and the mode it is driven at.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Link {
    pub mode: VideoMode,
    /// Data lanes, 1-4 (`N_LANES` is two bits).
    pub lanes: u32,
    /// Virtual channel, 0-3.
    pub channel: u32,
    pub format: Format,
    /// [`flags`].
    pub flags: u32,
}

/// `struct dw_mipi_dsi_dphy_timing`: the PHY's transition times in lane byte
/// clocks (include/drm/bridge/dw_mipi_dsi.h:26-31).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DphyTiming {
    pub data_hs2lp: u32,
    pub data_lp2hs: u32,
    pub clk_hs2lp: u32,
    pub clk_lp2hs: u32,
}

/// The PHY behind the host: `struct dw_mipi_dsi_phy_ops`
/// (include/drm/bridge/dw_mipi_dsi.h:33-44), the hooks [`attach`] calls at
/// the points dw-mipi-dsi.c calls them. A hook's error stops the attach.
pub trait Phy {
    /// `get_lane_mbps` (dw-mipi-dsi.c:989): the lane bit rate in Mbit/s the
    /// PHY will run for `link`. The PHY may choose and keep its PLL setting
    /// here; [`Phy::init`] programs it.
    fn lane_mbps(&mut self, m: &impl Mmio, link: &Link) -> Result<u32, i32>;

    /// `get_timing` (:856): the transition times at `lane_mbps`.
    fn timing(&self, lane_mbps: u32) -> Result<DphyTiming, i32>;

    /// `get_esc_clk_rate` (:667-673): the escape clock not to exceed, MHz.
    /// `None` is Linux's default, 20 MHz.
    fn esc_clk_mhz(&self) -> Option<u32> {
        None
    }

    /// `init` (:1010): the PHY's PLL and wrapper, after the host's PHY
    /// registers are set and before `PHY_RSTZ` releases the PHY.
    fn init(&mut self, m: &impl Mmio, env: &impl Env) -> Result<(), i32>;

    /// `power_on` (:1021), once the host is up in command mode.
    fn power_on(&mut self, _m: &impl Mmio) {}

    /// `power_off` (:951-952).
    fn power_off(&mut self, _m: &impl Mmio) {}

    /// Whether `PHY_STATUS[PHY_LOCK]` reports this PHY's PLL lock. A PHY
    /// whose lock is read elsewhere (and waited for in [`Phy::init`])
    /// returns false, and [`attach`] waits only for the clock lane's stop
    /// state.
    fn lock_in_phy_status(&self) -> bool {
        true
    }
}

/// Linux's escape clock default, MHz (dw-mipi-dsi.c:673).
pub const ESC_DEFAULT_MHZ: u32 = 20;
/// `minimum_lbccs` (dw-mipi-dsi.c:768), by lanes - 1.
pub const MINIMUM_LBCCS: [u32; 4] = [10, 5, 4, 3];
/// `PHY_STATUS_TIMEOUT_US` and `CMD_PKT_STATUS_TIMEOUT_US` (:226-227).
pub const PHY_STATUS_TIMEOUT_US: u32 = 10_000;
pub const CMD_PKT_STATUS_TIMEOUT_US: u32 = 20_000;
/// The stop wait (:895), the BTA timeout (:764) and the read time (:875,
/// :880) Linux writes.
pub const PHY_STOP_WAIT: u32 = 0x20;
pub const BTA_TO: u32 = 0xd00;
pub const MAX_RD_TIME: u32 = 10_000;
const POLL_US: u32 = 10;

/// Everything [`attach`] programs, computed before the first write.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Plan {
    pub lane_mbps: u32,
    pub esc_div: u32,
    pub timing: DphyTiming,
    /// Lane byte clocks.
    pub hline: u32,
    pub hsa: u32,
    pub hbp: u32,
    /// Lines.
    pub vsa: u32,
    pub vbp: u32,
    pub vfp: u32,
    pub vactive: u32,
    /// The two-frame wait, ms (dw-mipi-dsi.c:288-295).
    pub settle_ms: u32,
}

/// `(lane_mbps >> 3) / esc + 1` (dw-mipi-dsi.c:681).
pub fn esc_division(lane_mbps: u32, esc_mhz: u32) -> u32 {
    (lane_mbps >> 3) / esc_mhz.max(1) + 1
}

/// `dw_mipi_dsi_get_hcomponent_lbcc` (dw-mipi-dsi.c:776-808): `hcomponent`
/// pixels in lane byte clocks. `mode->clock` is the pixel clock in kHz.
pub fn hcomponent_lbcc(link: &Link, lane_mbps: u32, hcomponent: u32) -> u32 {
    let clock = u64::from(link.mode.pixel_clock_hz / 1000);
    let h = u64::from(hcomponent);
    let lbcc = if link.flags & flags::MODE_VIDEO_BURST != 0 {
        h * u64::from(lane_mbps) * 1000 / 8
    } else {
        h * clock * u64::from(link.format.bpp()) / (u64::from(link.lanes) * 8)
    };
    let lbcc = lbcc.div_ceil(clock.max(1)) as u32;
    lbcc.max(MINIMUM_LBCCS[(link.lanes - 1) as usize])
}

/// `drm_mode_vrefresh` and the two frames of dw-mipi-dsi.c:292-294:
/// `DIV_ROUND_UP(1000, refresh) * 2` ms.
pub fn two_frames_ms(mode: &VideoMode) -> u32 {
    let frame = u64::from(mode.htotal()) * u64::from(mode.vtotal());
    let clock_hz = u64::from(mode.pixel_clock_hz / 1000) * 1000;
    let refresh = ((clock_hz + frame / 2) / frame.max(1)).max(1);
    (1000u64.div_ceil(refresh) * 2) as u32
}

/// The link checked against what the registers hold.
pub fn check(link: &Link) -> Result<(), i32> {
    link.mode.check()?;
    if !(1..=4).contains(&link.lanes) || link.channel > 3 || link.mode.hactive > r::VID_PKT_SIZE_MASK {
        return Err(-EINVAL);
    }
    if link.mode.pixel_clock_hz < 1000 {
        return Err(-EINVAL);
    }
    Ok(())
}

/// The plan for `link` at the rate and timing the PHY gave.
pub fn plan(link: &Link, lane_mbps: u32, esc_mhz: u32, timing: DphyTiming) -> Result<Plan, i32> {
    check(link)?;
    if lane_mbps == 0 {
        return Err(-EINVAL);
    }
    let md = &link.mode;
    let esc_div = esc_division(lane_mbps, esc_mhz);
    if esc_div > 0xff {
        return Err(-EINVAL);
    }
    Ok(Plan {
        lane_mbps,
        esc_div,
        timing,
        // htotal; hsync_end - hsync_start; htotal - hsync_end (:815-817)
        hline: hcomponent_lbcc(link, lane_mbps, md.htotal()),
        hsa: hcomponent_lbcc(link, lane_mbps, md.hsync),
        hbp: hcomponent_lbcc(link, lane_mbps, md.hback),
        vsa: md.vsync,
        vbp: md.vback,
        vfp: md.vfront,
        vactive: md.vactive,
        settle_ms: two_frames_ms(md),
    })
}

/// [`attach`]'s steps, for a failure report.
pub mod step {
    pub const CHECK: u32 = 1;
    /// `get_lane_mbps`, `get_timing`.
    pub const PHY_RATE: u32 = 2;
    pub const PHY_INIT: u32 = 3;
    pub const PHY_LOCK: u32 = 4;
    pub const STOP_STATE: u32 = 5;
}

/// A failure and the step it happened in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Failed {
    pub step: u32,
    pub errno: i32,
}

fn rd(m: &impl Mmio, base: usize, off: u32) -> u32 {
    m.read32(base + off as usize)
}

fn wr(m: &impl Mmio, base: usize, off: u32, v: u32) {
    m.write32(base + off as usize, v)
}

/// `readl_poll_timeout`: read, test, wait, up to `limit_us`.
fn poll(m: &impl Mmio, env: &impl Env, base: usize, off: u32, limit_us: u32, ok: impl Fn(u32) -> bool) -> bool {
    let mut waited = 0;
    loop {
        if ok(rd(m, base, off)) {
            return true;
        }
        if waited >= limit_us {
            return false;
        }
        env.busy_wait_us(POLL_US);
        waited += POLL_US;
    }
}

/// `VERSION` with the patch byte cut (dw-mipi-dsi.c:869).
pub fn version(m: &impl Mmio, base: usize) -> u32 {
    rd(m, base, r::VERSION) & r::VERSION_MASK
}

/// The IP version, if it is one ST's glue drives (ST
/// dw_mipi_dsi-stm.c:1344-1351); `-ENODEV` otherwise (a block without its
/// clock or behind a firewall reads 0 or all ones).
pub fn probe(m: &impl Mmio, base: usize) -> Result<u32, i32> {
    match version(m, base) {
        v @ (r::HWVER_130 | r::HWVER_131 | r::HWVER_141) => Ok(v),
        _ => Err(-ENODEV),
    }
}

/// `VID_MODE_CFG` (dw-mipi-dsi.c:598-626).
pub fn vid_mode_cfg(link: &Link) -> u32 {
    r::ENABLE_LOW_POWER
        | if link.flags & flags::MODE_VIDEO_BURST != 0 {
            r::VID_MODE_TYPE_BURST
        } else if link.flags & flags::MODE_VIDEO_SYNC_PULSE != 0 {
            r::VID_MODE_TYPE_NON_BURST_SYNC_PULSES
        } else {
            r::VID_MODE_TYPE_NON_BURST_SYNC_EVENTS
        }
}

/// `DPI_CFG_POL` (dw-mipi-dsi.c:714-717; DE: ST dw-mipi-dsi.c:695-696).
pub fn dpi_cfg_pol(mode: &VideoMode) -> u32 {
    (if mode.vsync_high { 0 } else { r::VSYNC_ACTIVE_LOW })
        | (if mode.hsync_high { 0 } else { r::HSYNC_ACTIVE_LOW })
        | (if mode.de_high { 0 } else { r::DATAEN_ACTIVE_LOW })
}

/// `PCKHDL_CFG` (dw-mipi-dsi.c:724-732).
pub fn pckhdl_cfg(link: &Link) -> u32 {
    let v = r::CRC_RX_EN | r::ECC_RX_EN | r::BTA_EN | r::EOTP_TX_EN;
    if link.flags & flags::MODE_NO_EOT_PACKET != 0 {
        v & !r::EOTP_TX_EN
    } else {
        v
    }
}

/// `dw_mipi_dsi_set_mode` (dw-mipi-dsi.c:628-648): video mode if `video`,
/// else command mode; the clock lane requested in HS either way.
pub fn set_mode(m: &impl Mmio, base: usize, link: &Link, video: bool) {
    wr(m, base, r::PWR_UP, r::RESET); // dw-mipi-dsi.c:633
    if video {
        wr(m, base, r::MODE_CFG, r::ENABLE_VIDEO_MODE); // dw-mipi-dsi.c:636
        wr(m, base, r::VID_MODE_CFG, vid_mode_cfg(link)); // dw-mipi-dsi.c:637, :625
    } else {
        wr(m, base, r::MODE_CFG, r::ENABLE_CMD_MODE); // dw-mipi-dsi.c:639
    }
    let mut lpclk = r::PHY_TXREQUESTCLKHS;
    if link.flags & flags::CLOCK_NON_CONTINUOUS != 0 {
        lpclk |= r::AUTO_CLKLANE_CTRL;
    }
    wr(m, base, r::LPCLK_CTRL, lpclk); // dw-mipi-dsi.c:645
    wr(m, base, r::PWR_UP, r::POWERUP); // dw-mipi-dsi.c:647
}

/// `dw_mipi_dsi_mode_set` (dw-mipi-dsi.c:979-1023): the host and the PHY up,
/// the link in command mode with the clock lane in HS. [`video_on`] starts
/// video. The host's APB clock and reset, the PHY's reference clock and
/// supplies are the caller's (soc::stm32mp2).
pub fn attach<P: Phy>(m: &impl Mmio, env: &impl Env, base: usize, phy: &mut P, link: &Link) -> Result<Plan, Failed> {
    let at = |step| move |errno| Failed { step, errno };
    check(link).map_err(at(step::CHECK))?;
    let lane_mbps = phy.lane_mbps(m, link).map_err(at(step::PHY_RATE))?; // :989
    let timing = phy.timing(lane_mbps).map_err(at(step::PHY_RATE))?; // :856
    let esc_mhz = phy.esc_clk_mhz().unwrap_or(ESC_DEFAULT_MHZ);
    let p = plan(link, lane_mbps, esc_mhz, timing).map_err(at(step::CHECK))?;
    let md = &link.mode;

    // dw_mipi_dsi_init (:656-692)
    wr(m, base, r::PWR_UP, r::RESET); // dw-mipi-dsi.c:683
    wr(m, base, r::CLKMGR_CFG, r::clkmgr_cfg(0, p.esc_div)); // dw-mipi-dsi.c:690

    // dw_mipi_dsi_dpi_config (:694-722)
    wr(m, base, r::DPI_VCID, link.channel & 0x3); // dw-mipi-dsi.c:719
    wr(m, base, r::DPI_COLOR_CODING, link.format.color_coding()); // dw-mipi-dsi.c:720
    wr(m, base, r::DPI_CFG_POL, dpi_cfg_pol(md)); // dw-mipi-dsi.c:721

    wr(m, base, r::PCKHDL_CFG, pckhdl_cfg(link)); // dw-mipi-dsi.c:731
    wr(m, base, r::VID_MODE_CFG, vid_mode_cfg(link)); // dw-mipi-dsi.c:625 (from :998)

    // dw_mipi_dsi_video_packet_config (:734-749): the whole line in one
    // packet, no chunks, no null packets (the latter two: module text)
    wr(m, base, r::VID_PKT_SIZE, md.hactive & r::VID_PKT_SIZE_MASK); // dw-mipi-dsi.c:745
    wr(m, base, r::VID_NUM_CHUNKS, 0); // dw-mipi-dsi.c:737-742 (reset value)
    wr(m, base, r::VID_NULL_SIZE, 0); // dw-mipi-dsi.c:737-742 (reset value)

    // dw_mipi_dsi_command_mode_config (:751-766)
    wr(m, base, r::TO_CNT_CFG, r::to_cnt_cfg(0, 0)); // dw-mipi-dsi.c:758
    wr(m, base, r::BTA_TO_CNT, BTA_TO); // dw-mipi-dsi.c:764
    wr(m, base, r::MODE_CFG, r::ENABLE_CMD_MODE); // dw-mipi-dsi.c:765

    // dw_mipi_dsi_line_timer_config (:810-831)
    wr(m, base, r::VID_HLINE_TIME, p.hline); // dw-mipi-dsi.c:824
    wr(m, base, r::VID_HSA_TIME, p.hsa); // dw-mipi-dsi.c:827
    wr(m, base, r::VID_HBP_TIME, p.hbp); // dw-mipi-dsi.c:830

    // dw_mipi_dsi_vertical_timing_config (:833-847)
    wr(m, base, r::VID_VACTIVE_LINES, p.vactive); // dw-mipi-dsi.c:843
    wr(m, base, r::VID_VSA_LINES, p.vsa); // dw-mipi-dsi.c:844
    wr(m, base, r::VID_VFP_LINES, p.vfp); // dw-mipi-dsi.c:845
    wr(m, base, r::VID_VBP_LINES, p.vbp); // dw-mipi-dsi.c:846

    // dw_mipi_dsi_dphy_init (:899-907)
    wr(m, base, r::PHY_RSTZ, 0); // dw-mipi-dsi.c:902
    wr(m, base, r::PHY_TST_CTRL0, 0); // dw-mipi-dsi.c:904
    wr(m, base, r::PHY_TST_CTRL0, r::PHY_TESTCLR); // dw-mipi-dsi.c:905
    wr(m, base, r::PHY_TST_CTRL0, 0); // dw-mipi-dsi.c:906

    // dw_mipi_dsi_dphy_timing_config (:849-886)
    let t = &p.timing;
    if version(m, base) >= r::HWVER_131 {
        wr(m, base, r::PHY_TMR_CFG, r::phy_tmr_cfg_v131(t.data_hs2lp, t.data_lp2hs)); // dw-mipi-dsi.c:872
        wr(m, base, r::PHY_TMR_RD_CFG, MAX_RD_TIME & 0x7fff); // dw-mipi-dsi.c:875
    } else {
        // dw-mipi-dsi.c:877
        wr(m, base, r::PHY_TMR_CFG, r::phy_tmr_cfg(t.data_hs2lp, t.data_lp2hs, MAX_RD_TIME));
    }
    wr(m, base, r::PHY_TMR_LPCLK_CFG, r::phy_tmr_lpclk_cfg(t.clk_hs2lp, t.clk_lp2hs)); // dw-mipi-dsi.c:883

    wr(m, base, r::PHY_IF_CFG, r::phy_if_cfg(PHY_STOP_WAIT, link.lanes)); // dw-mipi-dsi.c:895

    // dw_mipi_dsi_clear_err (:929-935)
    rd(m, base, r::INT_ST0);
    rd(m, base, r::INT_ST1);
    wr(m, base, r::INT_MSK0, 0); // dw-mipi-dsi.c:933
    wr(m, base, r::INT_MSK1, 0); // dw-mipi-dsi.c:934

    phy.init(m, env).map_err(at(step::PHY_INIT))?; // :1010

    // dw_mipi_dsi_dphy_enable (:909-927)
    wr(m, base, r::PHY_RSTZ, r::PHY_ENFORCEPLL | r::PHY_ENABLECLK | r::PHY_UNRSTZ | r::PHY_UNSHUTDOWNZ); // dw-mipi-dsi.c:914
    if phy.lock_in_phy_status() && !poll(m, env, base, r::PHY_STATUS, PHY_STATUS_TIMEOUT_US, |s| s & r::PHY_LOCK != 0) {
        ylog!(env, Err, "DSI: PHY not locked (PHY_STATUS {:#010x})", rd(m, base, r::PHY_STATUS));
        return Err(Failed { step: step::PHY_LOCK, errno: -ETIMEDOUT });
    }
    if !poll(m, env, base, r::PHY_STATUS, PHY_STATUS_TIMEOUT_US, |s| s & r::PHY_STOP_STATE_CLK_LANE != 0) {
        ylog!(env, Err, "DSI: clock lane not in stop state (PHY_STATUS {:#010x})", rd(m, base, r::PHY_STATUS));
        return Err(Failed { step: step::STOP_STATE, errno: -ETIMEDOUT });
    }

    env.sleep_ms(p.settle_ms); // :1016
    set_mode(m, base, link, false); // :1019
    phy.power_on(m); // :1021
    ylog!(
        env,
        Inf,
        "DSI up: {} lanes at {} Mbit/s, esc div {}, line {} lbcc",
        link.lanes,
        p.lane_mbps,
        p.esc_div,
        p.hline
    );
    Ok(p)
}

/// Video on (`atomic_enable`, dw-mipi-dsi.c:1046-1055).
pub fn video_on(m: &impl Mmio, base: usize, link: &Link) {
    set_mode(m, base, link, true);
}

/// Back to command mode, PHY off, host and PHY held in reset
/// (`post_atomic_disable`, dw-mipi-dsi.c:937-963).
pub fn disable<P: Phy>(m: &impl Mmio, base: usize, phy: &mut P, link: &Link) {
    set_mode(m, base, link, false); // :949
    phy.power_off(m); // :951-952
    wr(m, base, r::PWR_UP, r::RESET); // dw-mipi-dsi.c:652
    wr(m, base, r::PHY_RSTZ, 0); // dw-mipi-dsi.c:653
}

/// The write data types this driver sends (include/video/mipi_display.h:28-47).
pub mod dt {
    pub const GENERIC_SHORT_WRITE_0_PARAM: u8 = 0x03;
    pub const GENERIC_SHORT_WRITE_1_PARAM: u8 = 0x13;
    pub const GENERIC_SHORT_WRITE_2_PARAM: u8 = 0x23;
    pub const DCS_SHORT_WRITE: u8 = 0x05;
    pub const DCS_SHORT_WRITE_PARAM: u8 = 0x15;
    pub const GENERIC_LONG_WRITE: u8 = 0x29;
    pub const DCS_LONG_WRITE: u8 = 0x39;
}

/// One command-mode write (`struct mipi_dsi_msg` without reads).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Msg<'a> {
    pub channel: u32,
    pub dtype: u8,
    pub tx: &'a [u8],
    /// `MIPI_DSI_MSG_USE_LPM`: send in low power.
    pub lpm: bool,
    /// `MIPI_DSI_MSG_REQ_ACK`.
    pub ack: bool,
}

/// `mipi_dsi_create_packet` (Linux drm_mipi_dsi.c:541-583) for the write
/// types: the header word (byte 3, the ECC, is the hardware's) and whether a
/// payload follows.
pub fn packet_header(msg: &Msg<'_>) -> Result<(u32, bool), i32> {
    let long = match msg.dtype {
        dt::GENERIC_LONG_WRITE | dt::DCS_LONG_WRITE => true,
        dt::GENERIC_SHORT_WRITE_0_PARAM
        | dt::GENERIC_SHORT_WRITE_1_PARAM
        | dt::GENERIC_SHORT_WRITE_2_PARAM
        | dt::DCS_SHORT_WRITE
        | dt::DCS_SHORT_WRITE_PARAM => false,
        _ => return Err(-EINVAL),
    };
    if msg.channel > 3 || (long && msg.tx.len() > 0xffff) || (!long && msg.tx.len() > 2) {
        return Err(-EINVAL);
    }
    let h0 = ((msg.channel & 0x3) << 6) | u32::from(msg.dtype & 0x3f);
    let (h1, h2) = if long {
        let n = msg.tx.len() as u32;
        (n & 0xff, (n >> 8) & 0xff)
    } else {
        (u32::from(msg.tx.first().copied().unwrap_or(0)), u32::from(msg.tx.get(1).copied().unwrap_or(0)))
    };
    Ok((h0 | (h1 << 8) | (h2 << 16), long))
}

/// `dw_mipi_dsi_host_transfer` for a write (dw-mipi-dsi.c:501-537): the
/// message configuration (:374-402), the payload a word at a time
/// (:431-460), then the header (:404-429). Returns the packet's size, as
/// Linux does (:533).
pub fn transfer(m: &impl Mmio, env: &impl Env, base: usize, msg: &Msg<'_>) -> Result<usize, i32> {
    let (hdr, long) = packet_header(msg)?;

    // dw_mipi_message_config (:374-402)
    wr(m, base, r::DPI_LP_CMD_TIM, r::dpi_lp_cmd_tim(16, 4)); // dw-mipi-dsi.c:386
    let cfg = (if msg.ack { r::ACK_RQST_EN } else { 0 }) | (if msg.lpm { r::CMD_MODE_ALL_LP } else { 0 });
    wr(m, base, r::CMD_MODE_CFG, cfg); // dw-mipi-dsi.c:394
    let v = rd(m, base, r::VID_MODE_CFG);
    let v = if msg.lpm { v | r::ENABLE_LOW_POWER_CMD } else { v & !r::ENABLE_LOW_POWER_CMD };
    wr(m, base, r::VID_MODE_CFG, v); // dw-mipi-dsi.c:401

    // dw_mipi_dsi_write (:431-465)
    let payload: &[u8] = if long { msg.tx } else { &[] };
    for chunk in payload.chunks(4) {
        let mut w = [0u8; 4];
        w[..chunk.len()].copy_from_slice(chunk);
        wr(m, base, r::GEN_PLD_DATA, u32::from_le_bytes(w)); // dw-mipi-dsi.c:443, :447
        if !poll(m, env, base, r::CMD_PKT_STATUS, CMD_PKT_STATUS_TIMEOUT_US, |s| s & r::GEN_PLD_W_FULL == 0) {
            return Err(-ETIMEDOUT);
        }
    }

    // dw_mipi_dsi_gen_pkt_hdr_write (:404-429)
    if !poll(m, env, base, r::CMD_PKT_STATUS, CMD_PKT_STATUS_TIMEOUT_US, |s| s & r::GEN_CMD_FULL == 0) {
        return Err(-ETIMEDOUT);
    }
    wr(m, base, r::GEN_HDR, hdr); // dw-mipi-dsi.c:417
    let done = r::GEN_CMD_EMPTY | r::GEN_PLD_W_EMPTY;
    if !poll(m, env, base, r::CMD_PKT_STATUS, CMD_PKT_STATUS_TIMEOUT_US, |s| s & done == done) {
        return Err(-ETIMEDOUT);
    }
    Ok(4 + payload.len())
}

/// `mipi_dsi_generic_write` (drm_mipi_dsi.c:744-772): the data type by size.
pub fn generic_write(m: &impl Mmio, env: &impl Env, base: usize, channel: u32, data: &[u8]) -> Result<usize, i32> {
    let dtype = match data.len() {
        0 => dt::GENERIC_SHORT_WRITE_0_PARAM,
        1 => dt::GENERIC_SHORT_WRITE_1_PARAM,
        2 => dt::GENERIC_SHORT_WRITE_2_PARAM,
        _ => dt::GENERIC_LONG_WRITE,
    };
    transfer(m, env, base, &Msg { channel, dtype, tx: data, lpm: false, ack: false })
}

/// `mipi_dsi_dcs_write_buffer` (drm_mipi_dsi.c:920-948): the command byte
/// and its parameters; the data type by size.
pub fn dcs_write(m: &impl Mmio, env: &impl Env, base: usize, channel: u32, data: &[u8]) -> Result<usize, i32> {
    let dtype = match data.len() {
        0 => return Err(-EINVAL),
        1 => dt::DCS_SHORT_WRITE,
        2 => dt::DCS_SHORT_WRITE_PARAM,
        _ => dt::DCS_LONG_WRITE,
    };
    transfer(m, env, base, &Msg { channel, dtype, tx: data, lpm: false, ack: false })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::env::Level;
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;
    use std::string::String;
    use std::vec::Vec;

    const BASE: usize = 0x4800_0000;

    /// The MP25's IP version (1.41) with a patch byte that the mask cuts.
    const MP25_VERSION: u32 = r::HWVER_141 | 0x2a;

    /// CEA-861 VIC 4, 1280x720 at 60 Hz: 74.25 MHz, H 1280 110 40 220 (+),
    /// V 720 5 5 20 (+), htotal 1650, vtotal 750.
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

    const ADV7535_720P: Link =
        Link { mode: CEA_720P60, lanes: 4, channel: 0, format: Format::Rgb888, flags: flags::ADV7535 };

    /// The lane rate ST's MP25 glue computes for this link
    /// (ST dw_mipi_dsi-stm.c:445-484, :339-397), from a 40 MHz PHY
    /// reference (the DK's HSE; CK_KER_DSIPHY is HSE or FLEXGEN_28,
    /// clk-stm32mp25.c:171-174, which one the board runs is the display
    /// study's): PLL out = 74250 kHz x 24 / (4 x 2) = 222750 kHz; non-burst,
    /// so no +20 %; inside 80-2500 Mbit/s. dppa_map: 2 x 222750 / 1000 =
    /// 445 < 450, so ODF 2 (:295). The IDF 1-16 x NDIV 64-625 search for
    /// round(40000 x n / (idf x 2)) nearest 222750 ends at idf 15, n 167:
    /// 222667 kHz (83 kHz off). lane_mbps = 2 x 222667 / 1000 = 445.
    const LANE_MBPS: u32 = 445;
    /// hstt_phy_141_table's row for 445 Mbit/s: the first maxfreq above it,
    /// 450 (ST dw_mipi_dsi-stm.c:957, :1003-1015): clk_lp2hs 65, clk_hs2lp
    /// 40, data_lp2hs 49, data_hs2lp 23.
    const TIMING_445: DphyTiming = DphyTiming { data_hs2lp: 23, data_lp2hs: 49, clk_hs2lp: 40, clk_lp2hs: 65 };

    #[derive(Clone, Debug, PartialEq, Eq)]
    enum Op {
        R(u32, u32),
        W(u32, u32),
        Phy(&'static str),
    }
    use Op::{Phy as P, R, W};

    type Log = Rc<RefCell<Vec<Op>>>;

    /// The host's registers: a plain register file, with `PHY_STATUS`
    /// reporting lock and the clock lane's stop state once `PHY_RSTZ` has
    /// released the PHY (unless a fault holds them), and `CMD_PKT_STATUS`
    /// reporting empty FIFOs.
    struct Dsi {
        regs: RefCell<[u32; 0x40]>,
        log: Log,
        never_locks: bool,
        clock_lane_stuck: bool,
    }

    impl Dsi {
        fn new(log: &Log, version: u32) -> Dsi {
            let d =
                Dsi { regs: RefCell::new([0; 0x40]), log: log.clone(), never_locks: false, clock_lane_stuck: false };
            d.regs.borrow_mut()[0] = version;
            d
        }
    }

    impl Mmio for Dsi {
        fn read32(&self, addr: usize) -> u32 {
            let off = (addr - BASE) as u32;
            let regs = self.regs.borrow();
            let v = match off {
                r::PHY_STATUS => {
                    let up = regs[(r::PHY_RSTZ / 4) as usize] == 0xf;
                    (if up && !self.never_locks { r::PHY_LOCK } else { 0 })
                        | (if up && !self.clock_lane_stuck { r::PHY_STOP_STATE_CLK_LANE } else { 0 })
                }
                r::CMD_PKT_STATUS => r::GEN_CMD_EMPTY | r::GEN_PLD_W_EMPTY,
                _ => regs[(off / 4) as usize],
            };
            self.log.borrow_mut().push(R(off, v));
            v
        }
        fn write32(&self, addr: usize, val: u32) {
            let off = (addr - BASE) as u32;
            self.log.borrow_mut().push(W(off, val));
            self.regs.borrow_mut()[(off / 4) as usize] = val;
        }
        fn read8(&self, _: usize) -> u8 {
            unreachable!()
        }
        fn write8(&self, _: usize, _: u8) {
            unreachable!()
        }
    }

    /// The PHY as ST's MP25 glue would answer for [`ADV7535_720P`], its
    /// hooks entered into the same log.
    struct FakePhy {
        log: Log,
        mbps: u32,
        lock_in_status: bool,
    }

    impl Phy for FakePhy {
        fn lane_mbps(&mut self, _: &impl Mmio, link: &Link) -> Result<u32, i32> {
            assert_eq!(link, &ADV7535_720P);
            self.log.borrow_mut().push(P("lane_mbps"));
            Ok(self.mbps)
        }
        fn timing(&self, lane_mbps: u32) -> Result<DphyTiming, i32> {
            assert_eq!(lane_mbps, self.mbps);
            self.log.borrow_mut().push(P("timing"));
            Ok(TIMING_445)
        }
        fn init(&mut self, _: &impl Mmio, _: &impl Env) -> Result<(), i32> {
            self.log.borrow_mut().push(P("init"));
            Ok(())
        }
        fn power_on(&mut self, _: &impl Mmio) {
            self.log.borrow_mut().push(P("power_on"));
        }
        fn power_off(&mut self, _: &impl Mmio) {
            self.log.borrow_mut().push(P("power_off"));
        }
        fn lock_in_phy_status(&self) -> bool {
            self.lock_in_status
        }
    }

    #[derive(Default)]
    struct FakeEnv {
        waited_us: Cell<u32>,
        slept_ms: Cell<u32>,
        lines: RefCell<Vec<String>>,
    }

    impl Env for FakeEnv {
        fn busy_wait_us(&self, us: u32) {
            self.waited_us.set(self.waited_us.get() + us);
        }
        fn sleep_ms(&self, ms: u32) {
            self.slept_ms.set(self.slept_ms.get() + ms);
        }
        fn log_str(&self, _: Level, line: &str) {
            self.lines.borrow_mut().push(line.into());
        }
        fn i2c_write(&self, _: u32, _: &[u8]) -> Result<(), i32> {
            unreachable!()
        }
        fn i2c_write_read(&self, _: u32, _: &[u8], _: &mut [u8]) -> Result<(), i32> {
            unreachable!()
        }
        fn gpio_set(&self, _: u32, _: bool) -> Result<(), i32> {
            unreachable!()
        }
    }

    fn rig(version: u32) -> (Log, Dsi, FakePhy, FakeEnv) {
        let log: Log = Rc::new(RefCell::new(Vec::new()));
        let dsi = Dsi::new(&log, version);
        let phy = FakePhy { log: log.clone(), mbps: LANE_MBPS, lock_in_status: true };
        (log, dsi, phy, FakeEnv::default())
    }

    /// The writes up to `phy.init`, as dw-mipi-dsi.c makes them for the
    /// ADV7535 at 720p60 over 4 lanes (line numbers per write).
    fn expected_head(tmr: &[Op]) -> Vec<Op> {
        let mut v = vec![
            P("lane_mbps"),
            P("timing"),
            W(r::PWR_UP, 0),               // :683
            W(r::CLKMGR_CFG, 0x0000_0003), // :690, (445 >> 3) / 20 + 1 = 3
            W(r::DPI_VCID, 0),             // :719
            W(r::DPI_COLOR_CODING, 0x5),   // :720, 24 bit
            W(r::DPI_CFG_POL, 0),          // :721, +hsync +vsync, DE high
            W(r::PCKHDL_CFG, 0x1c),        // :731, CRC, ECC, BTA; no EoTp
            W(r::VID_MODE_CFG, 0x3f00),    // :625, LP everywhere, sync pulses
            W(r::VID_PKT_SIZE, 1280),      // :745
            W(r::VID_NUM_CHUNKS, 0),
            W(r::VID_NULL_SIZE, 0),
            W(r::TO_CNT_CFG, 0),     // :758
            W(r::BTA_TO_CNT, 0xd00), // :764
            W(r::MODE_CFG, 1),       // :765
            // 1650 x 24 / 32 = 1237.5 -> 1238; 40 -> 30; 220 -> 165
            W(r::VID_HLINE_TIME, 1238),   // :824
            W(r::VID_HSA_TIME, 30),       // :827
            W(r::VID_HBP_TIME, 165),      // :830
            W(r::VID_VACTIVE_LINES, 720), // :843
            W(r::VID_VSA_LINES, 5),       // :844
            W(r::VID_VFP_LINES, 5),       // :845
            W(r::VID_VBP_LINES, 20),      // :846
            W(r::PHY_RSTZ, 0),            // :902
            W(r::PHY_TST_CTRL0, 0),       // :904
            W(r::PHY_TST_CTRL0, 1),       // :905
            W(r::PHY_TST_CTRL0, 0),       // :906
        ];
        v.extend_from_slice(tmr);
        v.extend([
            W(r::PHY_TMR_LPCLK_CFG, (40 << 16) | 65), // :883
            W(r::PHY_IF_CFG, (0x20 << 8) | 3),        // :895
            R(r::INT_ST0, 0),                         // :931
            R(r::INT_ST1, 0),                         // :932
            W(r::INT_MSK0, 0),                        // :933
            W(r::INT_MSK1, 0),                        // :934
            P("init"),                                // :1010
            W(r::PHY_RSTZ, 0xf),                      // :914
        ]);
        v
    }

    const UP: u32 = r::PHY_LOCK | r::PHY_STOP_STATE_CLK_LANE;

    fn v131_tmr() -> Vec<Op> {
        vec![
            R(r::VERSION, MP25_VERSION),        // :869
            W(r::PHY_TMR_CFG, (23 << 16) | 49), // :872
            W(r::PHY_TMR_RD_CFG, 10_000),       // :875
        ]
    }

    #[test]
    fn the_adv7535_720p60_sequence_over_four_lanes() {
        let (log, dsi, mut phy, env) = rig(MP25_VERSION);
        let p = attach(&dsi, &env, BASE, &mut phy, &ADV7535_720P).unwrap();
        let mut want = expected_head(&v131_tmr());
        want.extend([
            R(r::PHY_STATUS, UP), // :917
            R(r::PHY_STATUS, UP), // :922
            // set_mode(dsi, 0) (:1019)
            W(r::PWR_UP, 0),     // :633
            W(r::MODE_CFG, 1),   // :639
            W(r::LPCLK_CTRL, 1), // :645, continuous clock
            W(r::PWR_UP, 1),     // :647
            P("power_on"),       // :1021
        ]);
        assert_eq!(*log.borrow(), want);
        // 60 Hz: DIV_ROUND_UP(1000, 60) x 2 = 34 ms (:293)
        assert_eq!(env.slept_ms.get(), 34);
        assert_eq!(env.waited_us.get(), 0);
        assert_eq!(
            p,
            Plan {
                lane_mbps: 445,
                esc_div: 3,
                timing: TIMING_445,
                hline: 1238,
                hsa: 30,
                hbp: 165,
                vsa: 5,
                vbp: 20,
                vfp: 5,
                vactive: 720,
                settle_ms: 34,
            }
        );

        log.borrow_mut().clear();
        video_on(&dsi, BASE, &ADV7535_720P);
        assert_eq!(
            *log.borrow(),
            [
                W(r::PWR_UP, 0),            // :633
                W(r::MODE_CFG, 0),          // :636
                W(r::VID_MODE_CFG, 0x3f00), // :625
                W(r::LPCLK_CTRL, 1),        // :645
                W(r::PWR_UP, 1),            // :647
            ]
        );

        log.borrow_mut().clear();
        disable(&dsi, BASE, &mut phy, &ADV7535_720P);
        assert_eq!(
            *log.borrow(),
            [
                W(r::PWR_UP, 0),
                W(r::MODE_CFG, 1),
                W(r::LPCLK_CTRL, 1),
                W(r::PWR_UP, 1),
                P("power_off"),
                W(r::PWR_UP, 0),   // :652
                W(r::PHY_RSTZ, 0), // :653
            ]
        );
    }

    #[test]
    fn the_lane_rate_st_computes_for_720p_is_445() {
        // ST dw_mipi_dsi-stm.c:339-397, :458, :478 at a 40 MHz reference
        let round = |a: u32, b: u32| (a + b / 2) / b;
        let (clkin, out, odf) = (40_000, 74_250 * 24 / (4 * 2), 2);
        assert_eq!(out, 222_750);
        let mut best = (u32::MAX, 0, 0);
        for idf in 1..=16 {
            for n in 64..=625 {
                let d = round(clkin * n, idf * odf).abs_diff(out);
                if d < best.0 {
                    best = (d, idf, n);
                }
            }
        }
        assert_eq!(best, (83, 15, 167));
        assert_eq!(2 * round(clkin * 167, 15 * odf) / 1000, LANE_MBPS);
        // the stream needs 74.25 x 24 / 4 = 445.5 Mbit/s a lane; Linux's
        // non-burst line times come from the pixel clock, not this rate
        assert_eq!(hcomponent_lbcc(&ADV7535_720P, LANE_MBPS, 1650), 1238);
    }

    /// CEA-861 VIC 16, 1920x1080 at 60 Hz: 148.5 MHz, H 1920 88 44 148 (+),
    /// V 1080 4 5 36 (+), htotal 2200, vtotal 1125 (drm_edid.c:820-823).
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

    /// A PHY that answers one fixed rate; the DK's two modes below differ in
    /// it, the host's registers under test do not depend on its timing.
    struct RatePhy(u32);

    impl Phy for RatePhy {
        fn lane_mbps(&mut self, _: &impl Mmio, _: &Link) -> Result<u32, i32> {
            Ok(self.0)
        }
        fn timing(&self, _: u32) -> Result<DphyTiming, i32> {
            Ok(TIMING_445)
        }
        fn init(&mut self, _: &impl Mmio, _: &impl Env) -> Result<(), i32> {
            Ok(())
        }
    }

    /// The DK's two modes on the ADV7535 (4 lanes, RGB888, adv7533.c's
    /// flags: VIDEO | VIDEO_SYNC_PULSE | NO_EOT_PACKET | VIDEO_HSE, as the
    /// module's `flags::ADV7535` cites at adv7533.c:152-153 and the
    /// display-bringup note at adv7533.c:162-195 in ST's tree), with the
    /// lane rate ST's dw_mipi_dsi-stm.c picks (non-burst, so no +20 %; PLL
    /// out = clock x bpp / lanes / 2, lane rate = 2 x PLL out / 1000 kHz):
    /// 720p 74250 x 24 / 4 / 2 = 222750 kHz -> 445 Mbit/s (445.5 exact at
    /// the 27 MHz FLEXGEN_28 reference, 66 / 4 / 2; 40 MHz HSE: 445, see
    /// `the_lane_rate_st_computes_for_720p_is_445`); 1080p 148500 x 24 / 4 /
    /// 2 = 445500 kHz -> 891 Mbit/s (27 MHz x 66 / 4 / 1 exact), both inside
    /// 80-2500 Mbit/s (ST dw_mipi_dsi-stm.c:127-128).
    fn dk_link(mode: VideoMode) -> Link {
        Link { mode, lanes: 4, channel: 0, format: Format::Rgb888, flags: flags::ADV7535 }
    }

    /// Everything the host holds after `attach` and `video_on`.
    struct Pinned {
        mode: VideoMode,
        lane_mbps: u32,
        clkmgr: u32,
        pkt_size: u32,
        hsa: u32,
        hbp: u32,
        hline: u32,
        vsa: u32,
        vbp: u32,
        vfp: u32,
        vactive: u32,
        settle_ms: u32,
    }

    fn check_dk_mode(p: &Pinned) {
        let link = dk_link(p.mode);
        let (_, dsi, _, env) = rig(MP25_VERSION);
        let mut phy = RatePhy(p.lane_mbps);
        let plan = attach(&dsi, &env, BASE, &mut phy, &link).unwrap();
        video_on(&dsi, BASE, &link);
        let reg = |off: u32| dsi.regs.borrow()[(off / 4) as usize];
        // dw_mipi_dsi_video_packet_config: the line in one packet (:745);
        // the chunks and nulls stay 0 (:737-742)
        assert_eq!(reg(r::VID_PKT_SIZE), p.pkt_size);
        assert_eq!(reg(r::VID_NUM_CHUNKS), 0);
        assert_eq!(reg(r::VID_NULL_SIZE), 0);
        // dw_mipi_dsi_get_hcomponent_lbcc, non-burst (:776-808): ceil(x x
        // clock x 24 / (4 x 8) / clock) = ceil(x x 3 / 4) lane byte clocks,
        // each component of the mode, written by line_timer_config
        // (:815-830): htotal, hsync_end - hsync_start, htotal - hsync_end
        assert_eq!(reg(r::VID_HLINE_TIME), p.hline);
        assert_eq!(reg(r::VID_HSA_TIME), p.hsa);
        assert_eq!(reg(r::VID_HBP_TIME), p.hbp);
        // dw_mipi_dsi_vertical_timing_config (:833-847), in lines
        assert_eq!(reg(r::VID_VSA_LINES), p.vsa);
        assert_eq!(reg(r::VID_VBP_LINES), p.vbp);
        assert_eq!(reg(r::VID_VFP_LINES), p.vfp);
        assert_eq!(reg(r::VID_VACTIVE_LINES), p.vactive);
        // dw_mipi_dsi_video_mode_config (:598-626): LP in every blanking
        // region (0x3f << 8), non-burst with sync pulses (type 0)
        assert_eq!(reg(r::VID_MODE_CFG), 0x3f00);
        // dw_mipi_dsi_set_mode (:628-648): video mode, clock lane in HS and
        // continuous (adv7533's flags lack CLOCK_NON_CONTINUOUS)
        assert_eq!(reg(r::MODE_CFG), r::ENABLE_VIDEO_MODE);
        assert_eq!(reg(r::LPCLK_CTRL), r::PHY_TXREQUESTCLKHS);
        assert_eq!(reg(r::PWR_UP), r::POWERUP);
        // dw_mipi_dsi_phy_init (:888-897): 4 lanes is N_LANES 3, stop wait 0x20
        assert_eq!(reg(r::PHY_IF_CFG), 0x2003);
        // dw_mipi_dsi_dpi_config (:694-722): RGB888 is 24 bit (5), channel 0,
        // both syncs and DE active high -> no polarity bits
        assert_eq!(reg(r::DPI_COLOR_CODING), 0x5);
        assert_eq!(reg(r::DPI_VCID), 0);
        assert_eq!(reg(r::DPI_CFG_POL), 0);
        // dw_mipi_dsi_packet_handler_config (:724-732): NO_EOT_PACKET clears
        // EOTP_TX_EN
        assert_eq!(reg(r::PCKHDL_CFG), r::CRC_RX_EN | r::ECC_RX_EN | r::BTA_EN);
        // dw_mipi_dsi_init (:656-692): (lane_mbps >> 3) / 20 + 1
        assert_eq!(reg(r::CLKMGR_CFG), p.clkmgr);
        assert_eq!(
            plan,
            Plan {
                lane_mbps: p.lane_mbps,
                esc_div: p.clkmgr,
                timing: TIMING_445,
                hline: p.hline,
                hsa: p.hsa,
                hbp: p.hbp,
                vsa: p.vsa,
                vbp: p.vbp,
                vfp: p.vfp,
                vactive: p.vactive,
                settle_ms: p.settle_ms,
            }
        );
        // the sync pulses and the line add up: the line time is the active
        // packet plus the three blanking parts, within the ceil's rounding
        let active = hcomponent_lbcc(&link, p.lane_mbps, p.mode.hactive);
        let sum = p.hsa + p.hbp + hcomponent_lbcc(&link, p.lane_mbps, p.mode.hfront) + active;
        assert!(sum.abs_diff(p.hline) <= 3, "{sum} vs {}", p.hline);
    }

    #[test]
    fn the_dk_720p60_over_four_lanes() {
        // 1650 x 3 / 4 = 1237.5 -> 1238; 40 x 3 / 4 = 30; 220 x 3 / 4 = 165;
        // 60 Hz: DIV_ROUND_UP(1000, 60) x 2 = 34 ms (:293)
        check_dk_mode(&Pinned {
            mode: CEA_720P60,
            lane_mbps: 445,
            clkmgr: 3, // (445 >> 3) / 20 + 1 = 55 / 20 + 1
            pkt_size: 1280,
            hsa: 30,
            hbp: 165,
            hline: 1238,
            vsa: 5,
            vbp: 20,
            vfp: 5,
            vactive: 720,
            settle_ms: 34,
        });
    }

    #[test]
    fn the_dk_1080p60_over_four_lanes() {
        // 2200 x 3 / 4 = 1650; (2052 - 2008 = 44) x 3 / 4 = 33; (2200 - 2052
        // = 148) x 3 / 4 = 111, all exact; V: 1084 - 1080 = 4 front, 5 sync,
        // 1125 - 1089 = 36 back
        check_dk_mode(&Pinned {
            mode: CEA_1080P60,
            lane_mbps: 891,
            clkmgr: 6, // (891 >> 3) / 20 + 1 = 111 / 20 + 1
            pkt_size: 1920,
            hsa: 33,
            hbp: 111,
            hline: 1650,
            vsa: 5,
            vbp: 36,
            vfp: 4,
            vactive: 1080,
            settle_ms: 34,
        });
    }

    #[test]
    fn the_dk_modes_line_times_do_not_depend_on_the_lane_rate() {
        // non-burst line times come from the pixel clock (:785-790), so the
        // 3-lane 594 Mbit/s 720p option changes only the lane count's divisor:
        // 1650 x 24 / 24 = 1650 (3 lanes x 8 bit)
        let mut l = dk_link(CEA_720P60);
        assert_eq!(hcomponent_lbcc(&l, 445, 1650), hcomponent_lbcc(&l, 594, 1650));
        l.lanes = 3;
        assert_eq!(hcomponent_lbcc(&l, 594, 1650), 1650);
        assert_eq!(hcomponent_lbcc(&l, 594, 40), 40);
        assert_eq!(hcomponent_lbcc(&l, 594, 220), 220);
    }

    #[test]
    fn a_phy_that_never_locks_times_out_and_stops_there() {
        let (log, mut dsi, mut phy, env) = rig(MP25_VERSION);
        dsi.never_locks = true;
        let e = attach(&dsi, &env, BASE, &mut phy, &ADV7535_720P).unwrap_err();
        assert_eq!(e, Failed { step: step::PHY_LOCK, errno: -ETIMEDOUT });
        // the whole 10 ms (:226), polled, then nothing more: no mode switch,
        // no power_on
        assert_eq!(env.waited_us.get(), PHY_STATUS_TIMEOUT_US);
        assert_eq!(env.slept_ms.get(), 0);
        let ops = log.borrow();
        let head = expected_head(&v131_tmr());
        assert_eq!(ops[..head.len()], head[..]);
        let polls = (PHY_STATUS_TIMEOUT_US / POLL_US) as usize + 1;
        // the polls, and the one read for the log line
        assert_eq!(ops.len(), head.len() + polls + 1);
        assert!(ops[head.len()..].iter().all(|o| *o == R(r::PHY_STATUS, r::PHY_STOP_STATE_CLK_LANE)));
        assert!(env.lines.borrow().iter().any(|l| l.contains("PHY not locked")));
    }

    #[test]
    fn a_clock_lane_that_never_stops_times_out() {
        let (_, mut dsi, mut phy, env) = rig(MP25_VERSION);
        dsi.clock_lane_stuck = true;
        let e = attach(&dsi, &env, BASE, &mut phy, &ADV7535_720P).unwrap_err();
        assert_eq!(e, Failed { step: step::STOP_STATE, errno: -ETIMEDOUT });
        assert_eq!(env.waited_us.get(), PHY_STATUS_TIMEOUT_US);
    }

    #[test]
    fn a_phy_whose_lock_is_elsewhere_is_not_waited_for_in_phy_status() {
        let (log, mut dsi, mut phy, env) = rig(MP25_VERSION);
        dsi.never_locks = true;
        phy.lock_in_status = false;
        attach(&dsi, &env, BASE, &mut phy, &ADV7535_720P).unwrap();
        let head = expected_head(&v131_tmr()).len();
        assert_eq!(log.borrow()[head], R(r::PHY_STATUS, r::PHY_STOP_STATE_CLK_LANE));
        assert_eq!(log.borrow()[head + 1], W(r::PWR_UP, 0));
    }

    #[test]
    fn before_1_31_the_read_time_shares_phy_tmr_cfg() {
        let (log, dsi, mut phy, env) = rig(r::HWVER_130);
        attach(&dsi, &env, BASE, &mut phy, &ADV7535_720P).unwrap();
        let tmr = [R(r::VERSION, r::HWVER_130), W(r::PHY_TMR_CFG, (23 << 24) | (49 << 16) | 10_000)]; // :877
        let head = expected_head(&tmr);
        assert_eq!(log.borrow()[..head.len()], head[..]);
    }

    #[test]
    fn probe_takes_the_versions_st_drives() {
        for (v, ok) in [(MP25_VERSION, true), (r::HWVER_131 | 1, true), (r::HWVER_130, true), (0, false), (!0, false)] {
            let (_, dsi, _, _) = rig(v);
            assert_eq!(probe(&dsi, BASE).is_ok(), ok, "{v:#x}");
        }
    }

    #[test]
    fn the_mode_registers_follow_the_flags() {
        let mut l = ADV7535_720P;
        assert_eq!(pckhdl_cfg(&l), 0x1c);
        assert_eq!(vid_mode_cfg(&l), 0x3f00);
        l.flags = flags::MODE_VIDEO;
        assert_eq!(pckhdl_cfg(&l), 0x1d);
        assert_eq!(vid_mode_cfg(&l), 0x3f01);
        l.flags = flags::MODE_VIDEO | flags::MODE_VIDEO_BURST | flags::MODE_VIDEO_SYNC_PULSE;
        assert_eq!(vid_mode_cfg(&l), 0x3f02);
        let mut md = CEA_720P60;
        (md.hsync_high, md.vsync_high, md.de_high) = (false, false, false);
        assert_eq!(dpi_cfg_pol(&md), 0x7);
        assert_eq!(Format::Rgb666.color_coding(), 0x104);
        assert_eq!(Format::Rgb565.color_coding(), 0);
    }

    #[test]
    fn line_times_burst_and_the_minimum() {
        let mut l = ADV7535_720P;
        l.flags |= flags::MODE_VIDEO_BURST;
        // burst: 1650 x 445 x 1000 / 8 = 91781250, / 74250 = 1236.1 -> 1237
        assert_eq!(hcomponent_lbcc(&l, 445, 1650), 1237);
        // a 1-pixel sync on 1 lane: 1 x 24 / 8 = 3 < 10
        l.lanes = 1;
        assert_eq!(hcomponent_lbcc(&l, 445, 1), 10);
    }

    #[test]
    fn bad_links_are_refused_before_any_write() {
        for f in [
            |l: &mut Link| l.lanes = 0,
            |l: &mut Link| l.lanes = 5,
            |l: &mut Link| l.channel = 4,
            |l: &mut Link| l.mode.hactive = 0x4000,
            |l: &mut Link| l.mode.pixel_clock_hz = 0,
        ] {
            let mut l = ADV7535_720P;
            f(&mut l);
            let (log, dsi, mut phy, env) = rig(MP25_VERSION);
            assert_eq!(attach(&dsi, &env, BASE, &mut phy, &l), Err(Failed { step: step::CHECK, errno: -EINVAL }));
            assert!(log.borrow().is_empty());
        }
    }

    #[test]
    fn command_writes_short_and_long() {
        let (log, dsi, _, env) = rig(MP25_VERSION);
        // DCS short write with a parameter: header 0x15 | 0x36 << 8
        assert_eq!(dcs_write(&dsi, &env, BASE, 0, &[0x36, 0x00]), Ok(4));
        assert_eq!(
            *log.borrow(),
            [
                W(r::DPI_LP_CMD_TIM, 0x0010_0004), // :386
                W(r::CMD_MODE_CFG, 0),             // :394
                R(r::VID_MODE_CFG, 0),             // :396
                W(r::VID_MODE_CFG, 0),             // :401
                R(r::CMD_PKT_STATUS, 0x5),         // :409
                W(r::GEN_HDR, 0x0000_3615),        // :417
                R(r::CMD_PKT_STATUS, 0x5),         // :420
            ]
        );
        log.borrow_mut().clear();
        // generic long write of 5 bytes on channel 1, in low power
        let msg = Msg { channel: 1, dtype: dt::GENERIC_LONG_WRITE, tx: &[1, 2, 3, 4, 5], lpm: true, ack: false };
        assert_eq!(transfer(&dsi, &env, BASE, &msg), Ok(9));
        assert_eq!(
            *log.borrow(),
            [
                W(r::DPI_LP_CMD_TIM, 0x0010_0004),
                W(r::CMD_MODE_CFG, r::CMD_MODE_ALL_LP),
                R(r::VID_MODE_CFG, 0),
                W(r::VID_MODE_CFG, r::ENABLE_LOW_POWER_CMD),
                W(r::GEN_PLD_DATA, 0x0403_0201), // :447
                R(r::CMD_PKT_STATUS, 0x5),       // :452
                W(r::GEN_PLD_DATA, 0x0000_0005), // :443
                R(r::CMD_PKT_STATUS, 0x5),
                R(r::CMD_PKT_STATUS, 0x5),
                W(r::GEN_HDR, 0x0000_0569), // 0x29 | 1 << 6, length 5
                R(r::CMD_PKT_STATUS, 0x5),
            ]
        );
        assert_eq!(CMD_MODE_ALL_LP_BITS, r::CMD_MODE_ALL_LP);
        assert_eq!(dcs_write(&dsi, &env, BASE, 0, &[]), Err(-EINVAL));
        let bad = Msg { channel: 0, dtype: 0x14, tx: &[], lpm: false, ack: false };
        assert_eq!(transfer(&dsi, &env, BASE, &bad), Err(-EINVAL));
        assert_eq!(generic_write(&dsi, &env, BASE, 0, &[]), Ok(4));
    }

    /// CMD_MODE_ALL_LP bit by bit (dw-mipi-dsi.c:120-146).
    const CMD_MODE_ALL_LP_BITS: u32 = (1 << 24)
        | (1 << 19)
        | (1 << 18)
        | (1 << 17)
        | (1 << 16)
        | (1 << 14)
        | (1 << 13)
        | (1 << 12)
        | (1 << 11)
        | (1 << 10)
        | (1 << 9)
        | (1 << 8);
}
