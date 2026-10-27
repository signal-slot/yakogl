// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The DSIM's arithmetic (the register sequence against the model is in
//! soc::imx8mp::dsi_chain's tests, where the media domains it needs exist).

use super::*;

/// The Verdin's panel at the board's Linux clock plan.
pub(crate) const RIVERDI: VideoMode = VideoMode {
    pixel_clock_hz: 69_300_000,
    hactive: 1280,
    hfront: 36,
    hsync: 24,
    hback: 64,
    vactive: 800,
    vfront: 8,
    vsync: 8,
    vback: 24,
    hsync_high: true,
    vsync_high: true,
    de_high: true,
};

pub(crate) const LINK: Link = Link {
    mode: RIVERDI,
    lanes: 4,
    pll_ref_hz: 12_000_000,
    lane_rate_hz: 426_600_000,
    esc_max_hz: 20_000_000,
    burst: true,
};

#[test]
fn the_board_pll_is_the_golden_pms() {
    let p = pll_pms(12_000_000, 426_600_000).unwrap();
    assert_eq!((p.p, p.m, p.s, p.fout_hz), (5, 711, 2, 426_600_000));
    // golden PLLCTRL 0x00816C74: PllEn | PMS
    assert_eq!(r::dsi_pllctrl::PLLEN_MASK | field(r::dsi_pllctrl::PMS_MASK, p.field()), 0x0081_6c74);
    assert_eq!(Pms::from_field(crate::mmio::get(r::dsi_pllctrl::PMS_MASK, 0x0081_6c74)).m, 711);
}

#[test]
fn every_setting_is_inside_every_window() {
    for fin in [6_000_000u32, 12_000_000, 24_000_000, 27_000_000, 100_000_000] {
        let mut want = 80_000_000u32;
        while want <= 1_500_000_000 {
            if let Ok(p) = pll_pms(fin, want) {
                let f = u64::from(fin);
                assert!((1..=P_MAX).contains(&p.p) && (M_MIN..=M_MAX).contains(&p.m) && p.s <= S_MAX, "{p:?}");
                assert!(
                    (u64::from(FIN_PLL_MIN) * u64::from(p.p)..=u64::from(FIN_PLL_MAX) * u64::from(p.p)).contains(&f)
                );
                assert!((VCO_MIN..=VCO_MAX).contains(&(f * u64::from(p.m) / u64::from(p.p))));
                assert_eq!(u64::from(p.fout_hz), f * u64::from(p.m) / (u64::from(p.p) << p.s));
                // nearest: within one PFD step of the target at this s
                let step = (f / u64::from(p.p)) >> p.s;
                assert!(u64::from(p.fout_hz).abs_diff(u64::from(want)) <= step, "{fin} {want} {p:?}");
            }
            want += 7_000_000;
        }
    }
    // the board's band always has a setting from its reference
    for want in (400_000_000u32..=450_000_000).step_by(1_000_000) {
        assert!(pll_pms(12_000_000, want).is_ok(), "{want}");
    }
}

#[test]
fn out_of_window_requests_are_refused() {
    assert_eq!(pll_pms(5_000_000, 426_600_000), Err(-EINVAL)); // Fin under 6 MHz
    assert_eq!(pll_pms(12_000_000, 30_000_000), Err(-EINVAL)); // Fout under 32.8 MHz
    assert_eq!(pll_pms(12_000_000, 1_600_000_000), Err(-EINVAL)); // over 1500 MHz
}

#[test]
fn the_pll_timer_covers_3000_pfd_periods_at_the_fastest_pclk() {
    // 12 MHz / 5 = 2.4 MHz: 3000 periods = 1.25 ms = 250 000 cycles at 200 MHz
    assert_eq!(pll_timer(12_000_000, 5), 250_000);
    assert_eq!(pll_timer(24_000_000, 1), 25_000);
}

#[test]
fn the_board_plan() {
    let p = plan(&LINK).unwrap();
    assert_eq!((p.byte_hz, p.esc_div, p.esc_hz), (53_325_000, 3, 17_775_000));
    // HFP 36 px = 27.70 byte clocks, HBP 64 = 49.25, HSA 24 = 18.47; less 6
    assert_eq!((p.hfp, p.hbp, p.hsa), (21, 43, 12));
    assert_eq!(p.plltmr, 250_000);
    assert_eq!(p.timing, dphy_timing::compute(426_600_000).unwrap());
    assert_eq!(clkctrl(&p, 4), 0x91f8_0003, "golden CLKCTRL");
    assert_eq!(config(4, true), 0x0600_707f, "golden CONFIG");
}

#[test]
fn bad_links_are_refused_by_the_plan() {
    for l in [
        Link { lanes: 0, ..LINK },
        Link { lanes: 5, ..LINK },
        Link { pll_ref_hz: 1_000_000, ..LINK },
        // 426.6 Mbit/s x 1 lane cannot carry 69.3 MHz x 24 bits
        Link { lanes: 1, ..LINK },
        Link { mode: VideoMode { hactive: 2048, ..RIVERDI }, ..LINK },
        Link { mode: VideoMode { vback: 0x800, ..RIVERDI }, ..LINK },
        Link { mode: VideoMode { pixel_clock_hz: 0, ..RIVERDI }, ..LINK },
    ] {
        assert!(plan(&l).is_err(), "{l:?}");
    }
    // no rate given: pixel x 24 / lanes with the margin, 415.8 x 1.025 =
    // 426.195 Mbit/s; the slowest the PLL makes at or above that is
    // 12 MHz x 853 / (6 x 4) = 426.5
    assert_eq!(min_lane_rate(69_300_000, 4), 426_195_000);
    let p = plan(&Link { lane_rate_hz: 0, ..LINK }).unwrap();
    assert_eq!(p.pms.fout_hz, 426_500_000);
    // the R4 plan's 71.43 MHz needs 439.29 Mbit/s with the margin: the
    // board's fixed 426.6 cannot carry it, the derived rate is 439.5
    let r4 = VideoMode { pixel_clock_hz: 71_428_571, ..RIVERDI };
    assert_eq!(plan(&Link { mode: r4, ..LINK }), Err(-EINVAL));
    let p = plan(&Link { mode: r4, lane_rate_hz: 0, ..LINK }).unwrap();
    assert_eq!((p.pms.p, p.pms.m, p.pms.s, p.pms.fout_hz), (2, 293, 2, 439_500_000));
}

/// The margin, at the three points it was measured at (LINK_MARGIN_PERMILLE).
#[test]
fn a_link_without_the_margin_is_refused() {
    // 69.3 MHz on 426.6 Mbit/s: 25.97 per mille, the plan the panel showed right
    assert!(plan(&LINK).is_ok());
    // 71.1 MHz on 426.6 (2026-10-06) and 71.43 MHz on 428.6 (2026-09-23): none
    let at = |hz, lane| Link { mode: VideoMode { pixel_clock_hz: hz, ..RIVERDI }, lane_rate_hz: lane, ..LINK };
    assert_eq!(plan(&at(71_100_000, 426_600_000)), Err(-EINVAL));
    assert_eq!(plan(&at(71_428_571, 428_600_000)), Err(-EINVAL));
    // the boundary at 426.6 Mbit/s: 426.6 / 24 x 4 / 1.025 = 69.37 MHz
    assert!(plan(&at(69_365_000, 426_600_000)).is_ok());
    assert_eq!(plan(&at(69_370_000, 426_600_000)), Err(-EINVAL));
}
