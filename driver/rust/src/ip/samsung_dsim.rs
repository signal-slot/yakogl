// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The MIPI DSI host of the i.MX 8M Plus ("MIPI_DSI", 0x32E6_0000; the
//! Samsung DSIM core) and its D-PHY PLL, driving a video-mode link.
//!
//! Written from the i.MX 8M Plus Applications Processor Reference Manual,
//! IMX8MPRM Rev. 3, 08/2024 ("RM"): 13.6 "MIPI DSI Host Controller" (the
//! register descriptions 13.6.9.1.x, the programming model Table 13-26) and
//! 13.7 "MIPI D-PHY" (the PLL, 13.7.2.4 and Table 13-27; the initialisation,
//! Figure 13-69). Register names and bits from hal_nxp `PERI_MIPI_DSI.h`
//! ([`crate::soc::imx8mp::regs::mipi_dsi`]). The D-PHY timings are
//! [`crate::ip::dphy_timing`]'s.
//!
//! What the RM states and the code relies on:
//!
//! - PLL: Fout = Fin x M / (P x 2^S) (13.7.2.4); Fin 6-300 MHz, Fin / P
//!   2-30 MHz, P 1-63, M 64-1023, S such that 2^S is 1, 2, 4 or 8, Fout
//!   32.8125-1500 MHz (Table 13-27). "Setting P or M divider values as zero
//!   can cause malfunction of the PLL" (13.7.2.4).
//! - VCO: the NOTE in 13.7.2.4 says 1050-2100 MHz, Table 13-27 says
//!   1050-1500 MHz. The code takes the NOTE's range: the board's own Linux
//!   runs the PLL at 12 MHz x 711 / 5 = 1706.4 MHz (golden dump PLLCTRL
//!   0x00816C74, 2026-09-21), and 426.6 Mbit/s per lane cannot be reached
//!   inside 1050-1500 at all (S = 1 needs a VCO of 853 MHz, S = 2 one of
//!   1706 MHz). The conflict is the RM's; the board decides.
//! - `PLLCTRL[PllEn]` bit 23 and `PMS` bits 19:1 (13.6.9.1.25). `PllStable`
//!   is set "when the timer value in PLL timer register reads 0x00000000"
//!   (13.6.5): it is a timer, not a lock detector, so `PLLTMR` must cover
//!   the lock time. The lock time is "3000 x tp" (Figure 13-69, "PLL Locking
//!   Time"). The timer counts "System clock cycle[s]" (13.6.9.1.28); the only
//!   clock Table 13-24 lists for the block is PCLK, the APB clock, whose
//!   root (MEDIA_APB) runs at most 200 MHz (Table 5-1).
//! - `CLKCTRL` (13.6.9.1.6): `ByteClkSrc` "must be 2'b00" (D-PHY PLL, "PLL_out
//!   clock is used to generate ByteClk by dividing 8"), `PLLBypass` "must be
//!   set to 0", `Dphy_sel` 0 = the 1.5 Gbps D-PHY (the i.MX 8M Plus D-PHY:
//!   80-1500 Mbps, 13.7.1.2), EscClk = ByteClk / EscPrescaler, "up to
//!   20MHz"; `LaneEscClkEn[0]` the clock lane, `[4:1]` data lanes 0-3.
//! - Software reset (Table 13-26 case 16): "all clocks must be turned on
//!   because software reset is a synchronous reset": PLLTMR, CLKCTRL
//!   {EscClkEn, EscPrescaler}, PLLCTRL {PllEn, PMS}, wait for PLL stable,
//!   then `SwRst`, wait for the release (`STATUS[SwRstRls]`, 13.6.9.1.3),
//!   then "re-configure all registers" -- `SwRst` resets every register but
//!   STATUS, SWRST, CLKCTRL, PLLCTRL, PLLTMR (13.6.9.1.5).
//! - `CONFIG` (13.6.9.1.8): `VideoMode`, `BurstMode` (RGB data only in the
//!   RGB area), `SyncInform` 0 = event mode (pulse mode is non-burst only),
//!   `MainPixFormat` 111 = 24-bit RGB, `NumOfDatLane` lanes - 1, `LaneEn[0]`
//!   clock lane, `[4:1]` data lanes; `Mflush_VS` 0 = the main FIFO is flushed
//!   by VSYNC ("0 = Enable (default)"), `EoT_r03` 0 = EoT packets sent.
//! - `MDRESOL` (13.6.9.1.10): resolutions 1-2047; "In Video mode, if
//!   [MainStandby] is 0, data is not transferred". `MVPORCH` (13.6.9.1.11):
//!   `CmdAllow` lines where command packets may go after the stable VFP,
//!   `StableVfp` lines after the active region where they may not, `MainVbp`
//!   in lines. `MSYNC` (13.6.9.1.13): `MainVsa` in lines.
//! - `ESCMODE[ForceStopstate]` bit 20 "Forces Stopstate for D-PHY"
//!   (13.6.9.1.9); the programming model releases it after the
//!   configuration (Table 13-26 case 1 step 11, which names DSI_CONFIG but
//!   the field is ESCMODE's) and waits for the stop state.
//! - `STATUS` (13.6.9.1.3): `StopstateDat` bits 3:0, `StopstateClk` 8,
//!   `TxReadyHsClk` 10, `SwRstRls` 20, `PllStable` 31.
//! - `FIFOCTRL` bits 4:0 are the active-low FIFO pointer initialisations
//!   (`nInitRx` .. `nInitMain`, 13.6.9.1.21).
//!
//! Not stated in IMX8MPRM Rev. 3, and where the code takes it from:
//!
//! - The split of `PMS` into P, M and S. The widths follow from Table 13-27
//!   (P 1-63: six bits, M 64-1023: ten, S 0-3: three bits of an S field, 6 +
//!   10 + 3 = 19 = `PMS[19:1]`). The order, P in 19:14, M in 13:4, S in 3:1,
//!   is the only one that decodes the golden dump's 0x00816C74 into a valid
//!   setting: P 5, M 711, S 2, 426.6 MHz from the 12 MHz reference -- the
//!   rate the bridge's `CHA_DSI_CLK_RANGE` 42 (210-215 MHz DSI clock, i.e.
//!   420-430 Mbit/s) in the same dump confirms.
//! - The unit of `PHYTIMING*` (13.6.9.1.33-35 give "M_TLPXCTL[7:0] to D-PHY"
//!   and nothing more): byte clocks. Of the clocks the DSIM has, only the
//!   byte clock makes the golden values a working D-PHY timing (in UI,
//!   CLK_ZERO 18 would be 42 ns against 300 ns - CLK_PREPARE; in escape
//!   clocks, CLK_PREPARE 3 would be 169 ns against a 95 ns maximum).
//! - How `MHPORCH` / `MSYNC`'s "word counts for blank packet" relate to the
//!   pixel porches. The code gives each blanking interval's duration in byte
//!   clocks (pixels x byte clock / pixel clock) less the 6 bytes of a long
//!   packet's header and checksum (4 + 2), which is what the board's Linux
//!   has for HFP and HSA (21, 12); its HBP is one lower (42 against 43), with
//!   no RM basis for either.
//! - The length of the FIFO pointer initialisation (1 ms each way here:
//!   thousands of cycles of every clock the block has), the bound of the
//!   software-reset release (1 ms) and of the lane stop state (10 ms), and
//!   the self-clearing of `SwRst` (the board's Linux leaves SWRST 0 after
//!   writing 1, golden dump; the code waits for both SWRST 0 and SwRstRls 1).
//! - `TIMEOUT` is left at its reset value 0x00FF_FFFF (the longest BTA and
//!   LP-receive timeouts, 13.6.9.1.7): both apply to bus turn-around and
//!   low-power reads, which this driver never starts.
//! - `PHYCTRL_B1` (B_DPHYCTL, "refer to ... Media Block Control") is left at
//!   its reset value 0, as on the board's Linux.

use crate::env::Env;
use crate::errno::{EINVAL, EIO, ETIMEDOUT};
use crate::ip::dphy_timing::{self, Timing};
use crate::mmio::{field, Mmio};
use crate::soc::imx8mp::regs::mipi_dsi as r;
use crate::video::VideoMode;

/// Table 13-27.
pub const FIN_MIN: u32 = 6_000_000;
pub const FIN_MAX: u32 = 300_000_000;
pub const FIN_PLL_MIN: u32 = 2_000_000;
pub const FIN_PLL_MAX: u32 = 30_000_000;
pub const P_MAX: u32 = 63;
pub const M_MIN: u32 = 64;
pub const M_MAX: u32 = 1023;
pub const S_MAX: u32 = 3;
pub const FOUT_MIN: u64 = 32_812_500;
pub const FOUT_MAX: u64 = 1_500_000_000;
/// 13.7.2.4 NOTE (see the module text for the conflict with Table 13-27).
pub const VCO_MIN: u64 = 1_050_000_000;
pub const VCO_MAX: u64 = 2_100_000_000;
/// 13.6.9.1.6: "The escape clock frequency range varies up to 20MHz".
pub const ESC_MAX: u32 = 20_000_000;
/// The PLL timer's clock at its fastest: PCLK = MEDIA_APB, at most 200 MHz
/// (Table 13-24, Table 5-1).
pub const PCLK_MAX: u64 = 200_000_000;
/// Figure 13-69: the PLL locks within 3000 periods of its input.
pub const LOCK_PERIODS: u64 = 3000;

/// Bounds of the waits (see the module text).
pub const PLL_WAIT_US: u32 = 20_000;
pub const SWRST_WAIT_US: u32 = 1_000;
pub const LANE_WAIT_US: u32 = 10_000;
const POLL_US: u32 = 10;

/// A PLL setting: Fout = fin x m / (p x 2^s).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pms {
    pub p: u32,
    pub m: u32,
    pub s: u32,
    pub fout_hz: u32,
}

impl Pms {
    /// `PLLCTRL[PMS]`'s 19 bits: P 18:13, M 12:3, S 2:0 of the field (P
    /// 19:14, M 13:4, S 3:1 of the register; see the module text).
    pub const fn field(&self) -> u32 {
        (self.p << 13) | (self.m << 3) | self.s
    }

    pub const fn from_field(v: u32) -> Pms {
        Pms { p: (v >> 13) & 0x3f, m: (v >> 3) & 0x3ff, s: v & 0x7, fout_hz: 0 }
    }
}

/// The PLL setting nearest `want` Hz from `fin` Hz inside every Table 13-27
/// window (VCO: the NOTE's). Ties go to the smaller p, then the smaller s.
pub fn pll_pms(fin: u32, want: u32) -> Result<Pms, i32> {
    if !(FIN_MIN..=FIN_MAX).contains(&fin) || !(FOUT_MIN..=FOUT_MAX).contains(&u64::from(want)) {
        return Err(-EINVAL);
    }
    let mut best: Option<(u64, Pms)> = None;
    for p in 1..=P_MAX {
        // FIN_PLL_MIN <= fin / p <= FIN_PLL_MAX, exactly
        let (lo, hi) = (u64::from(FIN_PLL_MIN) * u64::from(p), u64::from(FIN_PLL_MAX) * u64::from(p));
        if !(lo..=hi).contains(&u64::from(fin)) {
            continue;
        }
        for s in 0..=S_MAX {
            // the m nearest want x p x 2^s / fin, and its neighbour
            let num = (u64::from(want) * u64::from(p)) << s;
            let lo = num / u64::from(fin);
            for m in [lo, lo + 1] {
                if !(u64::from(M_MIN)..=u64::from(M_MAX)).contains(&m) {
                    continue;
                }
                let vco = u64::from(fin) * m / u64::from(p);
                if !(VCO_MIN..=VCO_MAX).contains(&vco) {
                    continue;
                }
                let fout = (u64::from(fin) * m) / (u64::from(p) << s);
                if !(FOUT_MIN..=FOUT_MAX).contains(&fout) {
                    continue;
                }
                let err = fout.abs_diff(u64::from(want));
                if best.is_none_or(|(e, _)| err < e) {
                    best = Some((err, Pms { p, m: m as u32, s, fout_hz: fout as u32 }));
                }
            }
        }
    }
    best.map(|(_, pms)| pms).ok_or(-EINVAL)
}

/// The slowest PLL setting at or above `min` Hz (a lane rate that must carry
/// a given stream), inside the same windows as [`pll_pms`].
pub fn pll_pms_at_least(fin: u32, min: u32) -> Result<Pms, i32> {
    if !(FIN_MIN..=FIN_MAX).contains(&fin) || u64::from(min) > FOUT_MAX {
        return Err(-EINVAL);
    }
    let mut best: Option<Pms> = None;
    for p in 1..=P_MAX {
        let (lo, hi) = (u64::from(FIN_PLL_MIN) * u64::from(p), u64::from(FIN_PLL_MAX) * u64::from(p));
        if !(lo..=hi).contains(&u64::from(fin)) {
            continue;
        }
        for s in 0..=S_MAX {
            // the smallest m with fin x m / (p x 2^s) >= min
            let m = ((u64::from(min) * u64::from(p)) << s).div_ceil(u64::from(fin)).max(u64::from(M_MIN));
            if m > u64::from(M_MAX) {
                continue;
            }
            let vco = u64::from(fin) * m / u64::from(p);
            let fout = (u64::from(fin) * m) / (u64::from(p) << s);
            if !(VCO_MIN..=VCO_MAX).contains(&vco) || !(FOUT_MIN..=FOUT_MAX).contains(&fout) || fout < u64::from(min) {
                continue;
            }
            if best.is_none_or(|b| fout < u64::from(b.fout_hz)) {
                best = Some(Pms { p, m: m as u32, s, fout_hz: fout as u32 });
            }
        }
    }
    best.ok_or(-EINVAL)
}

/// `PLLTMR` for a PLL whose PFD runs at fin / p: 3000 PFD periods counted at
/// the fastest PCLK, rounded up.
pub fn pll_timer(fin: u32, p: u32) -> u32 {
    let t = (LOCK_PERIODS * u64::from(p) * PCLK_MAX).div_ceil(u64::from(fin.max(1)));
    t.min(u64::from(u32::MAX)) as u32
}

/// What the link needs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Link {
    /// The mode, with the pixel clock the LCDIF really runs at.
    pub mode: VideoMode,
    /// Data lanes, 1-4.
    pub lanes: u32,
    /// The PLL's reference (MEDIA_MIPI_PHY1_REF).
    pub pll_ref_hz: u32,
    /// The lane bit rate to run (the nearest the PLL makes); 0 = the slowest
    /// rate the PLL makes at or above pixel clock x 24 / lanes (no margin
    /// beyond the PLL's step). Either way a rate that cannot carry the
    /// stream is refused.
    pub lane_rate_hz: u32,
    /// The escape clock not to exceed (capped at 20 MHz).
    pub esc_max_hz: u32,
    /// Burst mode (else non-burst with sync events).
    pub burst: bool,
}

/// What [`plan`] computed, and what [`attach`] programmed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Plan {
    pub pms: Pms,
    pub plltmr: u32,
    pub byte_hz: u32,
    pub esc_div: u32,
    pub esc_hz: u32,
    pub timing: Timing,
    pub hfp: u32,
    pub hbp: u32,
    pub hsa: u32,
}

/// A long packet's header and checksum, bytes (DSI: 4 + 2).
const PACKET_OVERHEAD: u32 = 6;

fn blank_wc(pixels: u32, byte_hz: u32, pixel_hz: u32) -> u32 {
    let bytes = u64::from(pixels) * u64::from(byte_hz) / u64::from(pixel_hz);
    (bytes as u32).saturating_sub(PACKET_OVERHEAD)
}

/// Everything [`attach`] writes, computed and range-checked before any write.
pub fn plan(l: &Link) -> Result<Plan, i32> {
    let m = &l.mode;
    m.check()?;
    if !(1..=4).contains(&l.lanes) || m.hactive > 2047 || m.vactive > 2047 {
        return Err(-EINVAL);
    }
    // MVPORCH / MSYNC line fields: VBP and the stable VFP 11 bits, VSA 10
    if m.vback > 0x7ff || m.vfront > 0x7ff || m.vsync > 0x3ff {
        return Err(-EINVAL);
    }
    let need = (u64::from(m.pixel_clock_hz) * 24).div_ceil(u64::from(l.lanes));
    let pms = if l.lane_rate_hz != 0 {
        pll_pms(l.pll_ref_hz, l.lane_rate_hz)?
    } else {
        pll_pms_at_least(l.pll_ref_hz, u32::try_from(need).map_err(|_| -EINVAL)?)?
    };
    let timing = dphy_timing::compute(pms.fout_hz)?;
    let byte_hz = pms.fout_hz / 8;
    let esc_max = if l.esc_max_hz == 0 { ESC_MAX } else { l.esc_max_hz.min(ESC_MAX) };
    let esc_div = byte_hz.div_ceil(esc_max);
    if esc_div == 0 || esc_div > 0xffff {
        return Err(-EINVAL);
    }
    // the link must carry the pixels: 24 bits a pixel over `lanes` lanes
    if u64::from(pms.fout_hz) < need {
        return Err(-EINVAL);
    }
    Ok(Plan {
        pms,
        plltmr: pll_timer(l.pll_ref_hz, pms.p),
        byte_hz,
        esc_div,
        esc_hz: byte_hz / esc_div,
        timing,
        hfp: blank_wc(m.hfront, byte_hz, m.pixel_clock_hz),
        hbp: blank_wc(m.hback, byte_hz, m.pixel_clock_hz),
        hsa: blank_wc(m.hsync, byte_hz, m.pixel_clock_hz),
    })
}

/// [`attach`]'s steps, for a failure report and the C side's `stage`.
pub mod step {
    pub const PLAN: u32 = 1;
    pub const PLL: u32 = 2;
    pub const RESET: u32 = 3;
    pub const LANES: u32 = 4;
    /// Configured, clock lane in HS, data lanes held in stop state.
    pub const HELD: u32 = 5;
    /// `ForceStopstate` released: video flows when the LCDIF sends it.
    pub const VIDEO: u32 = 6;
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

fn poll(
    m: &impl Mmio,
    base: usize,
    env: &impl Env,
    limit_us: u32,
    mut ok: impl FnMut(&dyn Fn(u32) -> u32) -> bool,
) -> bool {
    let mut waited = 0;
    loop {
        if ok(&|off| rd(m, base, off)) {
            return true;
        }
        if waited >= limit_us {
            return false;
        }
        env.busy_wait_us(POLL_US);
        waited += POLL_US;
    }
}

fn lane_bits(lanes: u32) -> u32 {
    (1 << lanes) - 1
}

/// `CLKCTRL` with every clock this link needs and the HS clock request.
pub fn clkctrl(p: &Plan, lanes: u32) -> u32 {
    r::dsi_clkctrl::TXREQUESTHSCLK_MASK
        | r::dsi_clkctrl::ESCCLKEN_MASK
        | r::dsi_clkctrl::BYTECLKEN_MASK
        | field(r::dsi_clkctrl::LANEESCCLKEN_MASK, (lane_bits(lanes) << 1) | 1)
        | field(r::dsi_clkctrl::ESCPRESCALER_MASK, p.esc_div)
}

/// `CONFIG` for a 24-bit RGB video link.
pub fn config(lanes: u32, burst: bool) -> u32 {
    r::dsi_config::VIDEOMODE_MASK
        | if burst { r::dsi_config::BURSTMODE_MASK } else { 0 }
        | field(r::dsi_config::MAINPIXFORMAT_MASK, 0b111)
        | field(r::dsi_config::NUMOFDATLANE_MASK, lanes - 1)
        | field(r::dsi_config::LANEEN_MASK, (lane_bits(lanes) << 1) | 1)
}

/// Bring the link up to the point where the clock lane runs in HS and the
/// data lanes are held in stop state (`ForceStopstate`), with the mode
/// programmed and `MainStandby` set: everything but video. [`video_start`]
/// releases the data lanes. The PLL's reference clock (MEDIA_MIPI_PHY1_REF),
/// MIPI_PHY1's power and the block's clocks and resets are the caller's
/// (soc::imx8mp::media), and the pixel clock must run (RM Table 13-26 case
/// 16: the software reset is synchronous).
pub fn attach(m: &impl Mmio, base: usize, env: &impl Env, l: &Link) -> Result<Plan, Failed> {
    let at = |step| move |errno| Failed { step, errno };
    let p = plan(l).map_err(at(step::PLAN))?;

    // Table 13-26 case 16: the PLL with the escape prescaler, then reset.
    wr(m, base, r::DSI_PLLTMR, p.plltmr);
    wr(m, base, r::DSI_CLKCTRL, r::dsi_clkctrl::ESCCLKEN_MASK | field(r::dsi_clkctrl::ESCPRESCALER_MASK, p.esc_div));
    wr(m, base, r::DSI_PLLCTRL, r::dsi_pllctrl::PLLEN_MASK | field(r::dsi_pllctrl::PMS_MASK, p.pms.field()));
    if !poll(m, base, env, PLL_WAIT_US, |rd| rd(r::DSI_STATUS) & r::dsi_status::PLLSTABLE_MASK != 0) {
        return Err(Failed { step: step::PLL, errno: -ETIMEDOUT });
    }
    wr(m, base, r::DSI_SWRST, r::dsi_swrst::SWRST_MASK);
    if !poll(m, base, env, SWRST_WAIT_US, |rd| {
        rd(r::DSI_SWRST) & r::dsi_swrst::SWRST_MASK == 0 && rd(r::DSI_STATUS) & r::dsi_status::SWRSTRLS_MASK != 0
    }) {
        return Err(Failed { step: step::RESET, errno: -ETIMEDOUT });
    }
    // INTSRC: write one to clear (Table 13-26 case 13)
    let pending = rd(m, base, r::DSI_INTSRC);
    wr(m, base, r::DSI_INTSRC, pending);

    // "re-configure all registers"
    let t = &p.timing;
    wr(
        m,
        base,
        r::DSI_PHYTIMING,
        field(r::dsi_phytiming::M_TLPXCTL_MASK, t.lpx) | field(r::dsi_phytiming::M_THSEXITCTL_MASK, t.hs_exit),
    );
    wr(
        m,
        base,
        r::DSI_PHYTIMING1,
        field(r::dsi_phytiming1::M_TCLKPRPRCTL_MASK, t.clk_prepare)
            | field(r::dsi_phytiming1::M_TCLKZEROCTL_MASK, t.clk_zero)
            | field(r::dsi_phytiming1::M_TCLKPOSTCTL_MASK, t.clk_post)
            | field(r::dsi_phytiming1::M_TCLKTRAILCTL_MASK, t.clk_trail),
    );
    wr(
        m,
        base,
        r::DSI_PHYTIMING2,
        field(r::dsi_phytiming2::M_THSPRPRCTL_MASK, t.hs_prepare)
            | field(r::dsi_phytiming2::M_THSZEROCTL_MASK, t.hs_zero)
            | field(r::dsi_phytiming2::M_THSTRAILCTL_MASK, t.hs_trail),
    );
    wr(m, base, r::DSI_CLKCTRL, clkctrl(&p, l.lanes));
    let ninit = r::dsi_fifoctrl::NINITRX_MASK
        | r::dsi_fifoctrl::NINITSFR_MASK
        | r::dsi_fifoctrl::NINITI80_MASK
        | r::dsi_fifoctrl::NINITSUB_MASK
        | r::dsi_fifoctrl::NINITMAIN_MASK;
    let fifo = rd(m, base, r::DSI_FIFOCTRL) & !ninit;
    wr(m, base, r::DSI_FIFOCTRL, fifo);
    env.sleep_ms(1);
    wr(m, base, r::DSI_FIFOCTRL, fifo | ninit);
    env.sleep_ms(1);

    // the lanes on; every data lane and the clock lane in stop state (or the
    // clock lane already in HS: TxRequestHsClk is set)
    wr(m, base, r::DSI_CONFIG, config(l.lanes, l.burst));
    let dat = field(r::dsi_status::STOPSTATEDAT_MASK, lane_bits(l.lanes));
    if !poll(m, base, env, LANE_WAIT_US, |rd| {
        let s = rd(r::DSI_STATUS);
        s & dat == dat && s & (r::dsi_status::STOPSTATECLK_MASK | r::dsi_status::TXREADYHSCLK_MASK) != 0
    }) {
        return Err(Failed { step: step::LANES, errno: -EIO });
    }
    // hold the data lanes until the receiver is ready (risk R5): no command
    // lines in the VFP, the stop-state count 0 (it only times the automatic
    // BTA after a read, which is never sent)
    wr(m, base, r::DSI_ESCMODE, r::dsi_escmode::FORCESTOPSTATE__MASK);

    let md = &l.mode;
    wr(
        m,
        base,
        r::DSI_MVPORCH,
        field(r::dsi_mvporch::CMDALLOW_MASK, 0)
            | field(r::dsi_mvporch::STABLEVFP_MASK, md.vfront)
            | field(r::dsi_mvporch::MAINVBP_MASK, md.vback),
    );
    wr(
        m,
        base,
        r::DSI_MHPORCH,
        field(r::dsi_mhporch::MAINHFP_MASK, p.hfp) | field(r::dsi_mhporch::MAINHBP_MASK, p.hbp),
    );
    wr(m, base, r::DSI_MSYNC, field(r::dsi_msync::MAINVSA_MASK, md.vsync) | field(r::dsi_msync::MAINHSA_MASK, p.hsa));
    let res = field(r::dsi_mdresol::MAINVRESOL_MASK, md.vactive) | field(r::dsi_mdresol::MAINHRESOL_MASK, md.hactive);
    wr(m, base, r::DSI_MDRESOL, res);
    wr(m, base, r::DSI_MDRESOL, res | r::dsi_mdresol::MAINSTANDBY_MASK);
    Ok(p)
}

/// Release the data lanes: video packets go out as the LCDIF sends lines.
pub fn video_start(m: &impl Mmio, base: usize) {
    let v = rd(m, base, r::DSI_ESCMODE);
    wr(m, base, r::DSI_ESCMODE, v & !r::dsi_escmode::FORCESTOPSTATE__MASK);
}

/// True while the data lanes are held ([`attach`] done, [`video_start`] not).
pub fn is_held(m: &impl Mmio, base: usize) -> bool {
    rd(m, base, r::DSI_ESCMODE) & r::dsi_escmode::FORCESTOPSTATE__MASK != 0
}

/// Stop: lanes held, no standby, clocks and HS request off, PLL off.
pub fn disable(m: &impl Mmio, base: usize) {
    let e = rd(m, base, r::DSI_ESCMODE);
    wr(m, base, r::DSI_ESCMODE, e | r::dsi_escmode::FORCESTOPSTATE__MASK);
    let md = rd(m, base, r::DSI_MDRESOL);
    wr(m, base, r::DSI_MDRESOL, md & !r::dsi_mdresol::MAINSTANDBY_MASK);
    let c = rd(m, base, r::DSI_CLKCTRL);
    wr(
        m,
        base,
        r::DSI_CLKCTRL,
        c & !(r::dsi_clkctrl::TXREQUESTHSCLK_MASK
            | r::dsi_clkctrl::ESCCLKEN_MASK
            | r::dsi_clkctrl::BYTECLKEN_MASK
            | r::dsi_clkctrl::LANEESCCLKEN_MASK),
    );
    let pl = rd(m, base, r::DSI_PLLCTRL);
    wr(m, base, r::DSI_PLLCTRL, pl & !r::dsi_pllctrl::PLLEN_MASK);
}

/// `STATUS` as it stands.
pub fn status(m: &impl Mmio, base: usize) -> u32 {
    rd(m, base, r::DSI_STATUS)
}

pub fn pll_is_stable(m: &impl Mmio, base: usize) -> bool {
    status(m, base) & r::dsi_status::PLLSTABLE_MASK != 0
}

pub fn hs_clock_is_ready(m: &impl Mmio, base: usize) -> bool {
    status(m, base) & r::dsi_status::TXREADYHSCLK_MASK != 0
}

/// The PLL setting programmed now (`PLLCTRL`), with its output at `fin`.
pub fn programmed_pms(m: &impl Mmio, base: usize, fin: u32) -> Option<Pms> {
    let v = rd(m, base, r::DSI_PLLCTRL);
    if v & r::dsi_pllctrl::PLLEN_MASK == 0 {
        return None;
    }
    let mut p = Pms::from_field(crate::mmio::get(r::dsi_pllctrl::PMS_MASK, v));
    if p.p == 0 || p.m == 0 {
        return None;
    }
    p.fout_hz = (u64::from(fin) * u64::from(p.m) / (u64::from(p.p) << p.s)) as u32;
    Some(p)
}

#[cfg(test)]
mod tests;
