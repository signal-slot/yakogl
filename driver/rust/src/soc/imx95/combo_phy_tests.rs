// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! [`super::combo_phy`]: the PLL arithmetic for the 10.1" panel's lane rate
//! and across the PHY's range, hal_nxp's tables, the test interface's pin
//! protocol, and `init`'s CSR and test-code writes against the register
//! model of [`super::dsi_model`].

use super::combo_phy::{self, code, pll, regs, test_read, test_write, vco_div, Bases, ComboPhy, TestCode, Tuning};
use super::dsi_model::{Ev, Log, Soc};
use super::sm_mock::Sm;
use crate::errno::{EINVAL, EIO};
use crate::ip::dw_mipi_dsi::{regs as d, DphyTiming, Phy};
use crate::model::TestEnv;
use std::cell::RefCell;
use std::rc::Rc;
use std::vec::Vec;

const REF: u64 = 24_000_000;
const LANE_420: u64 = 420_000_000;

/// The model with the CAMERA domain on.
fn soc() -> (Log, Soc) {
    let log: Log = Rc::new(RefCell::new(Vec::new()));
    let mut sm = Sm::verdin();
    sm.pd_on.insert(3);
    let s = Soc::new(&log, Bases::PHYS, 0x4251_0000, Rc::new(RefCell::new(sm)));
    (log, s)
}

#[test]
fn the_10_inch_panels_420_mbit_pll_is_m70_n4_and_hals_controls() {
    // Fout = 210 MHz; the 210 MHz row of hal's VCO table (25 = 0x19, div 2):
    // VCO 420 MHz = 24 MHz x 70 / 4 (ref / N = 6 MHz), exact.
    let p = pll(REF, LANE_420, &Tuning::HAL).unwrap();
    assert_eq!((p.fout_hz, p.vco_hz, p.m, p.n), (210_000_000, 420_000_000, 70, 4));
    assert_eq!(p.fout_actual_hz, 210_000_000);
    assert_eq!((p.vco_cntrl, vco_div(p.vco_cntrl)), (0x19, 2));
    assert_eq!((p.prop_cntrl, p.pll_17), (0x0d, 0x0d));
    assert_eq!(p.sr_osc_freq_target, 0x384);
    assert_eq!(p.hsfreqrange, 0x05);
}

#[test]
fn linuxs_values_change_only_the_disputed_bytes() {
    let h = pll(REF, LANE_420, &Tuning::HAL).unwrap();
    let l = pll(REF, LANE_420, &Tuning::LINUX_420).unwrap();
    assert_eq!((l.m, l.n, l.vco_hz, vco_div(l.vco_cntrl)), (h.m, h.n, h.vco_hz, 2));
    assert_eq!((l.vco_cntrl, l.hsfreqrange, l.sr_osc_freq_target, l.pll_17), (0x1f, 0x16, 0x7d0, 0xcd));
    let (hc, hn) = h.test_codes();
    let (lc, ln) = l.test_codes();
    assert_eq!(hn, ln);
    let differ: Vec<_> =
        hc[..hn].iter().zip(&lc[..ln]).filter(|(a, b)| a != b).map(|(a, b)| (a.code, a.data, b.data)).collect();
    // the design note's U14 and U15 rows: SLEW_6/5, PLL_30, PLL_17
    assert_eq!(
        differ,
        [
            (code::TX_SLEW_6, 0x03, 0x07),
            (code::TX_SLEW_5, 0x84, 0xd0),
            (code::TX_PLL_30, 0xb3, 0xbf),
            (code::TX_PLL_17, 0x0d, 0xcd)
        ]
    );
}

#[test]
fn the_test_codes_for_420_mbit_in_hals_order() {
    let p = pll(REF, LANE_420, &Tuning::HAL).unwrap();
    let (c, n) = p.test_codes();
    let t = |code, data| TestCode { code, data };
    use code::*;
    assert_eq!(
        c[..n],
        [
            t(TX_PLL_13, 0x03),
            t(TX_CB_1, 0x06),
            t(TX_CB_0, 0x53),
            t(TX_CB_2, 0x10), // below 450 Mbit/s
            t(TX_CLK_TERMLOWCAP, 0x02),
            t(TX_SLEW_7, 0x00),
            t(TX_SLEW_6, 0x03),
            t(TX_SLEW_5, 0x84),
            t(TX_SLEW_7, 0x10),
            t(TX_PLL_29, 0x00),
            t(TX_PLL_28, 0x44), // M - 2 = 68
            t(TX_PLL_27, 0x98), // override, N - 1 = 3
            t(TX_PLL_30, 0xb3), // override, vco_cntrl 0x19, M override
            t(TX_PLL_1, 0x10),
            t(TX_PLL_5, 0x01),
            t(TX_PLL_22, 0x02),
            t(TX_PLL_23, 0x00),
            t(TX_PLL_24, 0x60),
            t(TX_PLL_25, 0x03),
            t(TX_PLL_9, 0x04),
            t(TX_PLL_17, 0x0d),
            t(TX_PLL_31, 0x03),
        ]
    );
    // from 450 Mbit/s on, CB_2 is not written
    assert_eq!(pll(REF, 450_000_000, &Tuning::HAL).unwrap().test_codes().1, n - 1);
}

/// hal's `DSI_DphyGetPllDivider` (fsl_mipi_dsi.c:522-563) as written there:
/// n 0-15, m 62-623, out = ref x (m + 2) / (n + 1) (truncated), the first
/// smallest error; here limited to the N the databook's 2-8 MHz allows.
fn hal_divider(ref_hz: u64, want: u64) -> (u32, u32, u64) {
    let mut best = (0, 0, u64::MAX);
    for n in 0..16u64 {
        if !(2_000_000 * (n + 1)..=8_000_000 * (n + 1)).contains(&ref_hz) {
            continue;
        }
        for m in 62..=623u64 {
            let out = ref_hz * (m + 2) / (n + 1);
            let diff = out.abs_diff(want);
            if diff < best.2 {
                best = ((m + 2) as u32, (n + 1) as u32, diff);
            }
        }
    }
    best
}

/// hal's `Pll_Set_Pll_Vco_Freq` (fsl_mipi_dsi.c:1153-1182).
fn hal_vco(fout_hz: u64) -> u64 {
    match fout_hz {
        f if f >= 320_000_000 => f,
        f if f >= 160_000_000 => f * 2,
        f if f >= 80_000_000 => f * 4,
        f => f * 8,
    }
}

#[test]
fn across_80_to_2500_mbit_the_pll_matches_hals_search_and_vco() {
    let mut rate = combo_phy::LANE_MIN_BPS;
    while rate <= combo_phy::LANE_MAX_BPS {
        let p = pll(REF, rate, &Tuning::HAL).unwrap();
        // the divider from vco_cntrl's bits is hal's VCO multiple
        assert_eq!(p.vco_hz, hal_vco(p.fout_hz), "{rate}");
        let (_, _, hal_err) = hal_divider(REF, p.vco_hz);
        let ours = (REF * u64::from(p.m) / u64::from(p.n)).abs_diff(p.vco_hz);
        // our rounding search is never worse than hal's truncating one
        assert!(ours <= hal_err, "{rate}: ours {ours} hal {hal_err}");
        assert!((combo_phy::M_MIN..=combo_phy::M_MAX).contains(&p.m) && (3..=12).contains(&p.n), "{rate}");
        // the lane rate the PLL makes is within 0.5 % of the one asked for
        // (its step at the VCO is ref / N, 2 MHz at N = 12: 409 Mbit/s
        // comes out at 408)
        assert!((p.fout_actual_hz * 2).abs_diff(rate) * 200 <= rate, "{rate}: {}", p.fout_actual_hz);
        rate += 7_000_000;
    }
}

#[test]
fn rates_outside_the_phy_are_refused() {
    assert_eq!(pll(REF, 79_999_999, &Tuning::HAL), Err(-EINVAL));
    assert_eq!(pll(REF, 2_500_000_001, &Tuning::HAL), Err(-EINVAL));
    assert_eq!(pll(0, LANE_420, &Tuning::HAL), Err(-EINVAL));
    // a reference no N brings into 2-8 MHz
    assert_eq!(pll(200_000_000, LANE_420, &Tuning::HAL), Err(-EINVAL));
}

#[test]
fn hals_tables_at_their_edges() {
    assert_eq!(combo_phy::hsfreqrange(420_000_000), 0x05);
    assert_eq!(combo_phy::hsfreqrange(425_000_000), 0x05);
    assert_eq!(combo_phy::hsfreqrange(425_000_001), 22);
    assert_eq!(combo_phy::hsfreqrange(80_000_000), 0);
    assert_eq!(combo_phy::hsfreqrange(2_500_000_000), 73);
    assert_eq!(combo_phy::vco_cntrl(210), 25);
    assert_eq!(combo_phy::vco_cntrl(209), 31);
    assert_eq!(combo_phy::vco_cntrl(1250), 1);
    assert_eq!(combo_phy::vco_cntrl(40), 63);
    assert_eq!(combo_phy::prop_cntrl(1150), 14);
    assert_eq!(combo_phy::prop_cntrl(210), 13);
    assert_eq!(combo_phy::sr_osc_freq_target(501), 0x4e2);
    assert_eq!(combo_phy::sr_osc_freq_target(1001), 0x7d0);
    assert_eq!(combo_phy::cfgclkfreqrange(24_000_000), Ok(0x1c));
    assert_eq!(combo_phy::cfgclkfreqrange(16_000_000), Err(-EINVAL));
    // 420 Mbit/s takes the 450 row
    assert_eq!(combo_phy::timing(420), DphyTiming { clk_lp2hs: 65, clk_hs2lp: 40, data_lp2hs: 49, data_hs2lp: 23 });
    assert_eq!(combo_phy::timing(80), DphyTiming { clk_lp2hs: 21, clk_hs2lp: 17, data_lp2hs: 15, data_hs2lp: 10 });
    assert_eq!(combo_phy::timing(9000), DphyTiming { clk_lp2hs: 281, clk_hs2lp: 118, data_lp2hs: 227, data_hs2lp: 67 });
}

#[test]
fn the_test_interface_pins_latch_the_code_and_the_data() {
    let (log, s) = soc();
    let (c0, c1) = (combo_phy::DSI_BASE + d::PHY_TST_CTRL0 as usize, combo_phy::DSI_BASE + d::PHY_TST_CTRL1 as usize);
    test_write(&s, c0, c1, 0x17b, 0xb3);
    test_write(&s, c0, c1, 0x402, 0x02);
    assert_eq!(s.codes("dsi"), [(0x17b, 0xb3), (0x402, 0x02)]);
    assert_eq!(test_read(&s, c0, c1, 0x17b), 0xb3);
    assert_eq!(test_read(&s, c0, c1, 0x402), 0x02);
    // hal's sequence: 17 pin writes a code write (fsl_mipi_dsi.c:1212-1235)
    let pins = log.borrow().iter().filter(|e| matches!(e, Ev::W(..))).count();
    assert_eq!(pins, 2 * 17 + 2 * 15);
}

fn init(tuning: Tuning, lanes: u32) -> (Soc, Result<(), i32>) {
    let (_, s) = soc();
    let mut phy = ComboPhy::new(Bases::PHYS, REF, REF);
    phy.tuning = tuning;
    phy.pll = Some(pll(REF, LANE_420, &tuning).unwrap());
    phy.lanes = lanes;
    let r = phy.init(&s, &TestEnv::default());
    (s, r)
}

#[test]
fn init_with_hals_values_writes_the_csrs_around_the_test_codes() {
    let (s, r) = init(Tuning::HAL, 4);
    r.unwrap();
    assert_eq!(s.block_writes(combo_phy::MASTER_CSR_BASE, 0x10), [(0x4, 0), (0x8, 0), (0x0, 0x2)]);
    assert_eq!(s.block_writes(combo_phy::STREAM_CSR_BASE, 0x10), [(0x0, 0)]);
    assert_eq!(
        s.block_writes(combo_phy::PHY_CSR_BASE, 0x200),
        [(regs::COMBO_PHY_MODE_CONTROL, 0x03), (regs::COMBO_PHY_FREQ_CONTROL, 0x0005_001c)]
    );
    assert!(s.block_writes(combo_phy::CSI1_BASE, 0x100).is_empty(), "no CSI-side bypass with hal's values");
    let (want, n) = pll(REF, LANE_420, &Tuning::HAL).unwrap().test_codes();
    let want: Vec<_> = want[..n].iter().map(|c| (c.code, c.data)).collect();
    assert_eq!(s.codes("dsi"), want);
    // the PHY's registers hold them (SLEW_7 last written 0x10)
    assert_eq!(s.dsi_if.borrow().regs[&code::TX_PLL_28], 0x44);
    assert_eq!(s.dsi_if.borrow().regs[&code::TX_SLEW_7], 0x10);
}

#[test]
fn init_with_linuxs_values_adds_the_bypass_the_lane_enables_and_turndisable() {
    let (s, r) = init(Tuning::LINUX_420, 4);
    r.unwrap();
    assert_eq!(
        s.block_writes(combo_phy::PHY_CSR_BASE, 0x200),
        [
            (regs::COMBO_PHY_MODE_CONTROL, 0xf3),
            (regs::COMBO_PHY_FREQ_CONTROL, 0x0016_001c),
            (regs::COMBO_PHY_TEST_MODE_CONTROL, 0x2000),
        ]
    );
    assert_eq!(s.codes("csi"), [(code::RX_CLKLANE_LANE_1, 0x02)]);
    // the CSI side first, the DSI test codes after the two PHY CSR writes
    let log = s.log.borrow();
    let csi = log.iter().position(|e| matches!(e, Ev::Code("csi", ..))).unwrap();
    let mode = log.iter().position(|e| *e == Ev::W(combo_phy::PHY_CSR_BASE + 0x100, 0xf3)).unwrap();
    let first_dsi = log.iter().position(|e| matches!(e, Ev::Code("dsi", ..))).unwrap();
    let turn = log.iter().position(|e| *e == Ev::W(combo_phy::PHY_CSR_BASE + 0x108, 0x2000)).unwrap();
    assert!(csi < mode && mode < first_dsi && first_dsi < turn);
}

#[test]
fn a_bypass_that_does_not_read_back_stops_init_before_the_phy() {
    let (_, s) = soc();
    s.csi_if.borrow_mut().stuck = Some(code::RX_CLKLANE_LANE_1);
    let mut phy = ComboPhy::new(Bases::PHYS, REF, REF);
    phy.tuning = Tuning::LINUX_420;
    phy.pll = Some(pll(REF, LANE_420, &phy.tuning).unwrap());
    phy.lanes = 4;
    let env = TestEnv::default();
    assert_eq!(phy.init(&s, &env), Err(-EIO));
    assert!(s.block_writes(combo_phy::PHY_CSR_BASE, 0x200).is_empty());
    assert!(env.lines.borrow().iter().any(|l| l.1.contains("termination bypass")));
}

#[test]
fn init_without_a_rate_or_with_a_bad_cfg_clock_writes_nothing() {
    let (_, s) = soc();
    let mut phy = ComboPhy::new(Bases::PHYS, REF, REF);
    assert_eq!(phy.init(&s, &TestEnv::default()), Err(-EINVAL));
    let mut phy = ComboPhy::new(Bases::PHYS, REF, 10_000_000);
    phy.pll = Some(pll(REF, LANE_420, &Tuning::HAL).unwrap());
    assert_eq!(phy.init(&s, &TestEnv::default()), Err(-EINVAL));
    assert!(s.log.borrow().is_empty());
}

#[test]
fn power_off_gates_everything_and_stops_the_pll_clocks() {
    let (_, s) = soc();
    let mut phy = ComboPhy::new(Bases::PHYS, REF, REF);
    phy.power_off(&s);
    assert_eq!(s.block_writes(combo_phy::MASTER_CSR_BASE, 0x10), [(0x0, 0x1f)]);
    assert_eq!(s.block_writes(combo_phy::PHY_CSR_BASE, 0x200), [(0x100, 0x1)]);
}
