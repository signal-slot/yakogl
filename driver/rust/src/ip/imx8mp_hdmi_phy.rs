// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The i.MX 8M Plus HDMI TX PHY (0x32FDFF00): its PLL for one TMDS rate,
//! integer mode, and the lock check.
//!
//! hal_nxp has no header for this block, so register offsets, names, fields
//! and reset values are written here from the i.MX 8M Plus Applications
//! Processor Reference Manual, IMX8MPRM Rev. 3, 08/2024 ("RM"), section 13.10
//! "HDMI TX PHY" (13.10.3.1 "HDMI_TX_PHY register descriptions"; names as the
//! RM gives them, `PHY_REGn` at offset 4 * n, each "Width 8"). What the code
//! relies on:
//!
//! - The PLL makes the 5x TMDS clock: integer mode "FPLL = FREF x M/(P x S)"
//!   (13.10.2.1.2), with the fractional formula named "TCLK5X"; the block
//!   diagram's PLL is "(TMDS 5X Gen)" followed by "Div /5" (Figure 13-78).
//!   The reset values agree: P = 1, M = 62, S = 2 (`PHY_REG1..3`) give
//!   744 MHz, five times the 148.8 MHz near the "TMDS_CLK=148.352 MHz" the
//!   `PHY_REG13` text calls the default.
//! - FREF is 24 MHz ("The nominal reference frequency is 24MHz",
//!   13.10.2.1.2; "24 MHz or 26 MHz reference clock", 13.10.1.2).
//! - "Supports ring VCO from 0.75 GHz to 3GHz" (13.10.2.1.2): read as
//!   FREF x M / P, the frequency before the post divider S (the reset values
//!   put it at 1.488 GHz). TMDS 25 MHz to 594 MHz (13.10.1.2).
//! - Field encodings: `PMS_P` 4 bits, 1..11 = divide by n (0 and 1 both
//!   divide by 1, `PHY_REG1`); `PMS_M` 8 bits, divide by the value
//!   (`PHY_REG2`); `PMS_S` 4 bits, 0000b = 1 and (2n - 1) = 2n up to 16
//!   (`PHY_REG3`); `SDM_K_EN` 0 = integer (`PHY_REG4`); `PLL_PI_EN` 0 =
//!   "Integer N PLL" (`PHY_REG7`).
//! - The frequency-lock detector: `FLD_CK_DIV` (`PHY_REG12` 5:4) divides the
//!   TMDS clock by 1, 2, 4 or 8 into "Int_pllclk", which "should be less than
//!   50MHz"; `FLD_TG_CODE` (9 bits, `PHY_REG13` 7:0 and `PHY_REG14` bit 0)
//!   = (256 / Int_pllclk in MHz) x 24 (`PHY_REG13`; its example, 148.352 MHz
//!   / 4 -> 166, matches).
//! - `PHY_REG33[MODE_SET_DONE]`: 0 "Assert forced global reset", 1 "Release
//!   forced global reset"; reset value 1.
//! - `PHY_REG34` (read only): bit 7 PHY_READY, 6 PLL_LOCK, 5 PHY_CLK_RDY,
//!   4 AFC_DONE, 3:0 AFC code.
//!
//! Not stated in IMX8MPRM Rev. 3, decided here:
//! - The programming order. The RM documents the registers but gives no
//!   sequence; this module holds the forced global reset
//!   (`MODE_SET_DONE` = 0) while it changes the PLL and releases it after.
//! - How long lock takes: [`LOCK_POLL_US`] is a bound of ours.
//! - The fractional mode: its formula in 13.10.2.1.2 leaves `PI` and the SDC
//!   terms undefined, so only integer P/M/S are used; [`plan`] reports the
//!   rate it reaches and its error.
//! - The access width: the RM gives each register as 8 bits wide at a 32-bit
//!   stride; they are accessed as 32-bit words with the value in bits 7:0.
//! - Every analog setting (charge pump, loop filter, AFC, driver amplitude
//!   and pre-emphasis, termination) stays at its RM reset value. Whether the
//!   running Linux changes them is for the board dump to say
//!   (driver/docs/x2-hdmi-cold.md).

use crate::env::Env;
use crate::errno::{EINVAL, ETIMEDOUT};
use crate::mmio::{field, get, Mmio};

/// Offset of `PHY_REGn` (13.10.3.1.1).
pub const fn reg(n: u32) -> u32 {
    4 * n
}

pub mod f {
    //! Fields used here (13.10.3.1), by register.
    /// `PHY_REG1`: PLL pre-divider.
    pub const PMS_P: u32 = 0x0f;
    /// `PHY_REG2`: PLL main divider.
    pub const PMS_M: u32 = 0xff;
    /// `PHY_REG3`: PLL post divider.
    pub const PMS_S: u32 = 0xf0;
    /// `PHY_REG4`: 1 = sigma-delta (fractional), 0 = integer.
    pub const SDM_K_EN: u32 = 0x80;
    /// `PHY_REG7`: 1 = fractional-N, 0 = integer-N.
    pub const PLL_PI_EN: u32 = 0x40;
    /// `PHY_REG12`: the FLD clock divider.
    pub const FLD_CK_DIV: u32 = 0x30;
    /// `PHY_REG13`: FLD target code bits 7:0.
    pub const FLD_TG_CODE_LO: u32 = 0xff;
    /// `PHY_REG14` bit 0: FLD target code bit 8.
    pub const FLD_TG_CODE_HI: u32 = 0x01;
    /// `PHY_REG21`: pixel clock colour depth (000b = 8 bit).
    pub const PCLK_SEL: u32 = 0x70;
    /// `PHY_REG33`.
    pub const MODE_SET_DONE: u32 = 0x80;
    /// `PHY_REG34` (read only).
    pub const PHY_READY: u32 = 0x80;
    pub const PLL_LOCK: u32 = 0x40;
    pub const PHY_CLK_RDY: u32 = 0x20;
    pub const AFC_DONE: u32 = 0x10;
}

/// The reset values of `PHY_REG1..47` (13.10.3.1.1 memory map), index n - 1.
pub const RESET: [u8; 47] = [
    0xd1, 0x3e, 0x15, 0xc0, 0x90, 0x88, 0x42, 0x47, 0x30, 0x33, // 1-10
    0x65, 0x20, 0xa6, 0x24, 0x80, 0x6c, 0xf2, 0x67, 0x00, 0x10, // 11-20
    0x00, 0x00, 0x10, 0x60, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // 21-30
    0x00, 0x00, 0x80, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // 31-40
    0xe0, 0x83, 0x0f, 0x3e, 0xf8, 0x00, 0x00, // 41-47
];

pub const FREF_HZ: u64 = 24_000_000;
const VCO_MIN_HZ: u64 = 750_000_000;
const VCO_MAX_HZ: u64 = 3_000_000_000;
const TMDS_MIN_HZ: u64 = 25_000_000;
const TMDS_MAX_HZ: u64 = 594_000_000;
const FLD_MAX_HZ: u64 = 50_000_000;
/// The post-divider values `PMS_S` can encode (`PHY_REG3`).
const S_VALUES: [u32; 8] = [1, 2, 4, 6, 8, 10, 12, 16];

/// The bound on the lock wait (ours; see the module text).
pub const LOCK_POLL_US: u32 = 20_000;
const POLL_STEP_US: u32 = 10;

/// An integer PLL setting and what it produces.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pll {
    pub p: u32,
    pub m: u32,
    pub s: u32,
    /// The TMDS clock it makes (= the pixel clock at 8 bits per component).
    pub tmds_hz: u32,
    /// Its error against the request, parts per million (signed).
    pub err_ppm: i32,
    /// `FLD_CK_DIV` code and `FLD_TG_CODE`.
    pub fld_div: u32,
    pub fld_code: u32,
}

impl Pll {
    pub fn vco_hz(&self) -> u64 {
        FREF_HZ * u64::from(self.m) / u64::from(self.p)
    }
}

fn s_code(s: u32) -> u32 {
    if s == 1 {
        0
    } else {
        s - 1
    }
}

/// The FLD divider and target code for a TMDS rate (`PHY_REG12/13`): the
/// smallest divider that brings the internal clock under 50 MHz, and
/// 256 x 24 / f(MHz), rounded to nearest.
pub fn fld_for(tmds_hz: u32) -> Option<(u32, u32)> {
    let t = u64::from(tmds_hz);
    for (code, div) in [(0u32, 1u64), (1, 2), (2, 4), (3, 8)] {
        let f = t / div;
        if f < FLD_MAX_HZ {
            // 256 * 24 / (f / 1e6) = 6144e6 / f
            let tg = (6_144_000_000u64 + f / 2) / f;
            return if tg < 512 { Some((code, tg as u32)) } else { None };
        }
    }
    None
}

/// The integer P/M/S nearest to `pixel_hz` x 5 at 8 bits per component,
/// within the VCO range; `None` when nothing lands within `max_ppm`.
pub fn plan(pixel_hz: u32, max_ppm: u32) -> Option<Pll> {
    let want5 = u64::from(pixel_hz) * 5;
    if !(TMDS_MIN_HZ..=TMDS_MAX_HZ).contains(&u64::from(pixel_hz)) {
        return None;
    }
    let mut best: Option<(u64, Pll)> = None;
    for p in 1..=11u32 {
        for &s in &S_VALUES {
            // M nearest to want5 * P * S / FREF
            let num = want5 * u64::from(p) * u64::from(s);
            let m0 = (num + FREF_HZ / 2) / FREF_HZ;
            for m in [m0.saturating_sub(1), m0, m0 + 1] {
                if !(1..=255).contains(&m) {
                    continue;
                }
                let vco = FREF_HZ * m / u64::from(p);
                if !(VCO_MIN_HZ..=VCO_MAX_HZ).contains(&vco) {
                    continue;
                }
                let out5 = vco / u64::from(s);
                let err = out5.abs_diff(want5);
                if best.as_ref().is_none_or(|(e, _)| err < *e) {
                    let tmds = (out5 / 5) as u32;
                    let ppm = ((out5 as i64 - want5 as i64) * 1_000_000 / want5 as i64) as i32;
                    let (fld_div, fld_code) = fld_for(tmds)?;
                    best = Some((err, Pll { p, m: m as u32, s, tmds_hz: tmds, err_ppm: ppm, fld_div, fld_code }));
                }
            }
        }
    }
    best.map(|(_, pll)| pll).filter(|p| p.err_ppm.unsigned_abs() <= max_ppm)
}

fn rd(m: &impl Mmio, base: usize, n: u32) -> u32 {
    m.read32(base + reg(n) as usize) & 0xff
}

fn wr(m: &impl Mmio, base: usize, n: u32, v: u32) {
    m.write32(base + reg(n) as usize, v & 0xff)
}

fn modify(m: &impl Mmio, base: usize, n: u32, mask: u32, v: u32) {
    let cur = rd(m, base, n);
    wr(m, base, n, (cur & !mask) | (v & mask));
}

/// `PHY_REG34`.
pub fn status(m: &impl Mmio, base: usize) -> u32 {
    rd(m, base, 34)
}

/// Locked and ready.
pub fn is_locked(m: &impl Mmio, base: usize) -> bool {
    let s = status(m, base);
    s & (f::PLL_LOCK | f::PHY_READY) == f::PLL_LOCK | f::PHY_READY
}

/// Program the PLL for `pll` and wait for lock (see the module text for the
/// order). On a timeout the returned error carries nothing more; the
/// caller reads [`status`] for the log.
pub fn program(m: &impl Mmio, base: usize, env: &impl Env, pll: &Pll) -> Result<(), i32> {
    if !(1..=11).contains(&pll.p) || !(1..=255).contains(&pll.m) || !S_VALUES.contains(&pll.s) || pll.fld_code > 0x1ff {
        return Err(-EINVAL);
    }
    modify(m, base, 33, f::MODE_SET_DONE, 0);
    modify(m, base, 1, f::PMS_P, field(f::PMS_P, pll.p));
    wr(m, base, 2, pll.m);
    modify(m, base, 3, f::PMS_S, field(f::PMS_S, s_code(pll.s)));
    modify(m, base, 4, f::SDM_K_EN, 0);
    modify(m, base, 7, f::PLL_PI_EN, 0);
    modify(m, base, 12, f::FLD_CK_DIV, field(f::FLD_CK_DIV, pll.fld_div));
    wr(m, base, 13, pll.fld_code & f::FLD_TG_CODE_LO);
    modify(m, base, 14, f::FLD_TG_CODE_HI, pll.fld_code >> 8);
    // 8 bits per component: the pixel clock is the TMDS clock
    modify(m, base, 21, f::PCLK_SEL, 0);
    modify(m, base, 33, f::MODE_SET_DONE, f::MODE_SET_DONE);
    let mut waited = 0;
    loop {
        if is_locked(m, base) {
            return Ok(());
        }
        if waited >= LOCK_POLL_US {
            return Err(-ETIMEDOUT);
        }
        env.busy_wait_us(POLL_STEP_US);
        waited += POLL_STEP_US;
    }
}

/// The P/M/S the PHY holds now (for logs and the adopt path).
pub fn read_pms(m: &impl Mmio, base: usize) -> (u32, u32, u32) {
    let p = get(f::PMS_P, rd(m, base, 1)).max(1);
    let mm = rd(m, base, 2);
    let sc = get(f::PMS_S, rd(m, base, 3));
    (p, mm, if sc == 0 { 1 } else { sc + 1 })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::TestEnv;
    use crate::model_hdmi::{HdmiFaults, HdmiModel, PHY};

    #[test]
    fn the_reset_values_describe_the_rm_default_pll() {
        // P 1, M 62, S 2: 744 MHz = 5 x 148.8 MHz; FLD /4, code 166
        assert_eq!(get(f::PMS_P, u32::from(RESET[0])), 1);
        assert_eq!(u32::from(RESET[1]), 62);
        assert_eq!(get(f::PMS_S, u32::from(RESET[2])), 1); // code 1 = divide by 2
        assert_eq!(get(f::FLD_CK_DIV, u32::from(RESET[11])), 2);
        assert_eq!(u32::from(RESET[12]), 166);
        assert_eq!(fld_for(148_352_000), Some((2, 166)), "the PHY_REG13 example");
        assert_eq!(reg(34), 0x88);
        assert_eq!(reg(47), 0xbc);
    }

    #[test]
    fn plan_for_the_x2_monitor() {
        // 154 MHz has no exact integer P/M/S at 24 MHz; the nearest is
        // P 7, M 225, S 1: 771.43 MHz / 5 = 154.286 MHz, +1855 ppm
        let p = plan(154_000_000, 5000).unwrap();
        assert_eq!((p.p, p.m, p.s), (7, 225, 1));
        assert_eq!(p.tmds_hz, 154_285_714);
        assert_eq!(p.err_ppm, 1855);
        assert!(p.vco_hz() >= 750_000_000);
        assert_eq!((p.fld_div, p.fld_code), (2, 159));
        // exact for 27 MHz (480p) and 25.2 MHz (VGA); not for 148.5 MHz
        // (M would be 495), which the fractional mode would need
        let p = plan(27_000_000, 0).unwrap();
        assert_eq!((p.p, p.m, p.s, p.err_ppm), (1, 45, 8, 0));
        assert!(plan(148_500_000, 0).is_none());
        let p = plan(25_200_000, 0).unwrap();
        assert_eq!(p.tmds_hz, 25_200_000);
        // out of range, or too far off
        assert!(plan(20_000_000, 5000).is_none());
        assert!(plan(600_000_000, 5000).is_none());
        assert!(plan(154_000_000, 1000).is_none());
    }

    #[test]
    fn program_holds_the_global_reset_around_the_pll_writes() {
        let m = HdmiModel::phy_only();
        let e = TestEnv::default();
        let b = m.base(PHY);
        let pll = plan(154_000_000, 5000).unwrap();
        program(&m, b, &e, &pll).unwrap();
        let w = m.writes();
        let r33: Vec<_> = w.iter().enumerate().filter(|(_, x)| x.1 == reg(33)).collect();
        assert_eq!(r33.len(), 2);
        assert_eq!(r33[0].1 .2 & 0x80, 0, "held");
        assert_eq!(r33[1].1 .2 & 0x80, 0x80, "released");
        let pll_writes: Vec<_> =
            w.iter().enumerate().filter(|(_, x)| [reg(1), reg(2), reg(3)].contains(&x.1)).collect();
        assert!(pll_writes.iter().all(|(i, _)| *i > r33[0].0 && *i < r33[1].0), "{w:?}");
        assert_eq!(read_pms(&m, b), (7, 225, 1));
        assert_eq!(m.peek(PHY, reg(1)), 0xd7, "reserved and CLK_SEL bits kept");
        assert_eq!(m.peek(PHY, reg(13)), 159);
        assert!(is_locked(&m, b));
    }

    #[test]
    fn no_lock_is_a_timeout() {
        let m = HdmiModel::phy_only_with(HdmiFaults { phy_no_lock: true, ..Default::default() });
        let e = TestEnv::default();
        let pll = plan(154_000_000, 5000).unwrap();
        assert_eq!(program(&m, m.base(PHY), &e, &pll), Err(-ETIMEDOUT));
        assert!(*e.waited_us.borrow() >= u64::from(LOCK_POLL_US));
    }

    #[test]
    fn bad_settings_are_refused() {
        let m = HdmiModel::phy_only();
        let e = TestEnv::default();
        let good = plan(154_000_000, 5000).unwrap();
        for bad in [Pll { p: 12, ..good }, Pll { m: 0, ..good }, Pll { s: 3, ..good }, Pll { fld_code: 512, ..good }] {
            assert_eq!(program(&m, m.base(PHY), &e, &bad), Err(-EINVAL));
        }
        assert!(m.writes().is_empty());
    }
}
