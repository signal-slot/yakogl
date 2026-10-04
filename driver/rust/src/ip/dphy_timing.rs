// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! MIPI D-PHY transmitter timings in byte-clock cycles, computed from the
//! D-PHY minimums and rounded UP, so that every interval the transmitter
//! produces is at least the specification's minimum.
//!
//! Source: MIPI Alliance Specification for D-PHY, Version 1.2, the global
//! operation timing parameters table (the i.MX 8M Plus D-PHY is "D-PHY spec
//! v1.2 compatible", IMX8MPRM Rev. 3 13.7.1.2). The values are the widely
//! cited minimums of that table; the table and section number are a
//! citation pending: the specification text was not at hand when this was
//! written, and the numbers below must be checked against it.
//!
//! | parameter | minimum | maximum |
//! |---|---|---|
//! | T_LPX | 50 ns | |
//! | T_HS-EXIT | 100 ns | |
//! | T_CLK-PREPARE | 38 ns | 95 ns |
//! | T_CLK-PREPARE + T_CLK-ZERO | 300 ns | |
//! | T_CLK-POST | 60 ns + 52 UI | |
//! | T_CLK-TRAIL | 60 ns | |
//! | T_HS-PREPARE | 40 ns + 4 UI | 85 ns + 6 UI |
//! | T_HS-PREPARE + T_HS-ZERO | 145 ns + 10 UI | |
//! | T_HS-TRAIL | max(n x 8 UI, 60 ns + n x 4 UI) | |
//!
//! UI is one bit time on a lane (the reciprocal of the lane bit rate). In
//! T_HS-TRAIL, n = 1 for forward-direction high-speed transmission and n = 4
//! for reverse-direction high-speed transmission. A display host only ever
//! transmits forward in HS (the i.MX 8M Plus D-PHY: "All lanes support
//! high-speed transmission in the forward direction", reverse transfer is
//! low-power only, IMX8MPRM 13.7.1.2), so n = 1.
//!
//! The rounding rule: a duration t becomes ceil(t / T_byte) cycles, with
//! T_byte = 8 UI. For the two sums, the ZERO part is ceil(sum / T_byte) minus
//! the PREPARE cycles already chosen, so the sum is met however the parts
//! round. Everything is integer arithmetic on exact rationals (picoseconds
//! times the bit rate), so the result is exact, not a float rounded twice.
//!
//! The registers these go into count byte clocks: a unit the reference manual
//! does not state for the DSIM (IMX8MPRM 13.6.9.1.33-35 name the fields only);
//! see [`crate::ip::samsung_dsim`] for why it is the byte clock.

use crate::errno::EINVAL;

/// Picoseconds per second.
const PS: u64 = 1_000_000_000_000;

/// The D-PHY minimum of one interval: `ps` picoseconds plus `ui` unit
/// intervals.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Min {
    pub ps: u64,
    pub ui: u64,
}

pub const LPX: Min = Min { ps: 50_000, ui: 0 };
pub const HS_EXIT: Min = Min { ps: 100_000, ui: 0 };
pub const CLK_PREPARE: Min = Min { ps: 38_000, ui: 0 };
pub const CLK_PREPARE_MAX: Min = Min { ps: 95_000, ui: 0 };
pub const CLK_PREPARE_ZERO: Min = Min { ps: 300_000, ui: 0 };
pub const CLK_POST: Min = Min { ps: 60_000, ui: 52 };
pub const CLK_TRAIL: Min = Min { ps: 60_000, ui: 0 };
pub const HS_PREPARE: Min = Min { ps: 40_000, ui: 4 };
pub const HS_PREPARE_MAX: Min = Min { ps: 85_000, ui: 6 };
pub const HS_PREPARE_ZERO: Min = Min { ps: 145_000, ui: 10 };
/// T_HS-TRAIL, n = 1: the larger of these two.
pub const HS_TRAIL_A: Min = Min { ps: 0, ui: 8 };
pub const HS_TRAIL_B: Min = Min { ps: 60_000, ui: 4 };

/// The timings in byte-clock cycles.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Timing {
    pub lpx: u32,
    pub hs_exit: u32,
    pub clk_prepare: u32,
    pub clk_zero: u32,
    pub clk_post: u32,
    pub clk_trail: u32,
    pub hs_prepare: u32,
    pub hs_zero: u32,
    pub hs_trail: u32,
}

/// The lowest and highest lane bit rates the computation accepts: the
/// i.MX 8M Plus D-PHY's HS range, 80-1500 Mbit/s (IMX8MPRM 13.7.1.2).
pub const RATE_MIN: u32 = 80_000_000;
pub const RATE_MAX: u32 = 1_500_000_000;

/// Cycles of the byte clock (bit rate / 8) that cover `m` at `rate` bit/s,
/// rounded up: ceil((ps / 1e12 + ui / rate) * rate / 8)
/// = ceil((ps * rate + ui * 1e12) / (8 * 1e12)).
pub fn cycles(m: Min, rate: u32) -> u64 {
    let num = m.ps * u64::from(rate) + m.ui * PS;
    num.div_ceil(8 * PS)
}

/// The duration of `n` byte clocks, scaled like a [`Min`] (picoseconds x
/// rate): n x 8 UI x rate x 1e12 / rate = n x 8 x 1e12, whatever the rate.
fn span(n: u64, _rate: u32) -> u64 {
    n * 8 * PS
}

fn le(m: Min, n: u64, rate: u32) -> bool {
    m.ps * u64::from(rate) + m.ui * PS <= span(n, rate)
}

/// Every field from the minimums at a lane bit rate of `rate` bit/s.
/// `Err(-EINVAL)` for a rate outside the PHY's range, a field that does not
/// fit 8 bits, or a PREPARE that rounding pushed over its maximum.
pub fn compute(rate: u32) -> Result<Timing, i32> {
    if !(RATE_MIN..=RATE_MAX).contains(&rate) {
        return Err(-EINVAL);
    }
    let c = |m| cycles(m, rate);
    let clk_prepare = c(CLK_PREPARE);
    let hs_prepare = c(HS_PREPARE);
    // A PREPARE rounded up may not exceed its maximum (the receiver's
    // window): with T_byte = 8 UI and the windows 57 ns / 45 ns + 2 UI wide,
    // this holds at every rate in range, but it is checked, not assumed.
    if span(clk_prepare, rate) > CLK_PREPARE_MAX.ps * u64::from(rate) + CLK_PREPARE_MAX.ui * PS
        || span(hs_prepare, rate) > HS_PREPARE_MAX.ps * u64::from(rate) + HS_PREPARE_MAX.ui * PS
    {
        return Err(-EINVAL);
    }
    let t = Timing {
        lpx: c(LPX) as u32,
        hs_exit: c(HS_EXIT) as u32,
        clk_prepare: clk_prepare as u32,
        clk_zero: c(CLK_PREPARE_ZERO).saturating_sub(clk_prepare) as u32,
        clk_post: c(CLK_POST) as u32,
        clk_trail: c(CLK_TRAIL) as u32,
        hs_prepare: hs_prepare as u32,
        hs_zero: c(HS_PREPARE_ZERO).saturating_sub(hs_prepare) as u32,
        hs_trail: c(HS_TRAIL_A).max(c(HS_TRAIL_B)) as u32,
    };
    let all =
        [t.lpx, t.hs_exit, t.clk_prepare, t.clk_zero, t.clk_post, t.clk_trail, t.hs_prepare, t.hs_zero, t.hs_trail];
    if all.iter().any(|&v| v > 0xff) {
        return Err(-EINVAL);
    }
    Ok(t)
}

impl Timing {
    /// Each interval the timing produces meets its minimum at `rate` (the
    /// property the rounding rule guarantees; the tests check it at every
    /// rate in range).
    pub fn meets_minimums(&self, rate: u32) -> bool {
        let n = |v: u32| u64::from(v);
        le(LPX, n(self.lpx), rate)
            && le(HS_EXIT, n(self.hs_exit), rate)
            && le(CLK_PREPARE, n(self.clk_prepare), rate)
            && le(CLK_PREPARE_ZERO, n(self.clk_prepare + self.clk_zero), rate)
            && le(CLK_POST, n(self.clk_post), rate)
            && le(CLK_TRAIL, n(self.clk_trail), rate)
            && le(HS_PREPARE, n(self.hs_prepare), rate)
            && le(HS_PREPARE_ZERO, n(self.hs_prepare + self.hs_zero), rate)
            && le(HS_TRAIL_A, n(self.hs_trail), rate)
            && le(HS_TRAIL_B, n(self.hs_trail), rate)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The board's lane rate: 12 MHz x 711 / (5 x 2^2) = 426.6 Mbit/s, byte
    /// clock 53.325 MHz (18.753 ns), UI 2.344 ns.
    const RATE: u32 = 426_600_000;

    #[test]
    fn the_board_rate_rounded_up() {
        let t = compute(RATE).unwrap();
        // LPX 50/18.753 = 2.67; HS_EXIT 5.33; CLK_PREPARE 2.03;
        // CLK_PREPARE+ZERO 16.00 (15.9975); CLK_POST 181.9 ns = 9.70;
        // CLK_TRAIL 3.20; HS_PREPARE 49.4 ns = 2.63; HS_PREPARE+ZERO 168.4 ns
        // = 8.98; HS_TRAIL 69.4 ns = 3.70.
        assert_eq!(
            t,
            Timing {
                lpx: 3,
                hs_exit: 6,
                clk_prepare: 3,
                clk_zero: 13,
                clk_post: 10,
                clk_trail: 4,
                hs_prepare: 3,
                hs_zero: 6,
                hs_trail: 4,
            }
        );
        assert!(t.meets_minimums(RATE));
    }

    #[test]
    fn the_old_table_misses_two_minimums_at_this_rate() {
        // PHYTIMING 0x305, PHYTIMING1 0x03120904, PHYTIMING2 0x00040507 (the
        // board's Linux, golden dump): HS_EXIT 5 = 93.8 ns < 100 ns and
        // CLK_POST 9 = 168.8 ns < 181.9 ns, if the unit is the byte clock.
        let old = Timing {
            lpx: 3,
            hs_exit: 5,
            clk_prepare: 3,
            clk_zero: 18,
            clk_post: 9,
            clk_trail: 4,
            hs_prepare: 4,
            hs_zero: 5,
            hs_trail: 7,
        };
        assert!(!old.meets_minimums(RATE));
        assert!(!le(HS_EXIT, 5, RATE));
        assert!(!le(CLK_POST, 9, RATE));
        assert!(le(HS_PREPARE_ZERO, 9, RATE)); // 4 + 5, met
    }

    #[test]
    fn every_rate_in_range_meets_every_minimum_or_is_refused_for_a_reason() {
        // A PREPARE has a window, not just a minimum: where no whole number of
        // byte clocks falls inside it (below 84.2 Mbit/s for CLK_PREPARE's 95 ns
        // maximum, and narrow bands near 100 Mbit/s for HS_PREPARE), there is
        // no setting, and the computation says so instead of breaking a limit.
        let fits = |lo: Min, hi: Min, rate: u32| {
            let n = cycles(lo, rate);
            span(n, rate) <= hi.ps * u64::from(rate) + hi.ui * PS
        };
        let (mut ok, mut refused) = (0, 0);
        let mut rate = RATE_MIN;
        while rate <= RATE_MAX {
            match compute(rate) {
                Ok(t) => {
                    ok += 1;
                    assert!(t.meets_minimums(rate), "{rate}: {t:?}");
                    // tight: one cycle less would miss
                    assert!(!le(LPX, u64::from(t.lpx) - 1, rate));
                    assert!(!le(CLK_POST, u64::from(t.clk_post) - 1, rate));
                    assert!(!le(CLK_PREPARE_ZERO, u64::from(t.clk_prepare + t.clk_zero) - 1, rate));
                }
                Err(_) => {
                    refused += 1;
                    assert!(rate < 160_000_000, "{rate}: refused above the low band");
                    assert!(
                        !fits(CLK_PREPARE, CLK_PREPARE_MAX, rate) || !fits(HS_PREPARE, HS_PREPARE_MAX, rate),
                        "{rate} refused though both PREPARE windows hold a whole cycle count"
                    );
                }
            }
            rate += 100_000;
        }
        assert!(ok > 13_900 && refused < 250, "{ok} {refused}");
        assert_eq!(compute(84_000_000), Err(-EINVAL));
        // the board's band is clear
        for rate in (300_000_000u32..=600_000_000).step_by(100_000) {
            assert!(compute(rate).is_ok(), "{rate}");
        }
    }

    #[test]
    fn exact_boundaries_round_up_not_to_nearest() {
        // 400 Mbit/s: T_byte = 20 ns exactly. LPX 50 ns = 2.5 -> 3 (to
        // nearest would give 2 or 3); CLK_PREPARE 38 = 1.9 -> 2; 300 ns = 15
        // exactly -> 15 (no rounding needed, none added).
        let t = compute(400_000_000).unwrap();
        assert_eq!((t.lpx, t.clk_prepare, t.clk_prepare + t.clk_zero), (3, 2, 15));
        assert_eq!(cycles(Min { ps: 20_000, ui: 0 }, 400_000_000), 1);
        assert_eq!(cycles(Min { ps: 20_001, ui: 0 }, 400_000_000), 2);
    }

    #[test]
    fn out_of_range_rates_are_refused() {
        assert_eq!(compute(RATE_MIN - 1), Err(-EINVAL));
        assert_eq!(compute(RATE_MAX + 1), Err(-EINVAL));
        assert_eq!(compute(0), Err(-EINVAL));
    }
}
