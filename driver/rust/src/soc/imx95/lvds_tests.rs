// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The X12 LVDS path's SoC half: [`super::display_power`] with the LDB PLL,
//! [`super::dpu`] and [`super::display`] on [`Route::Lvds`], and
//! [`super::ldb`], against the mock System Manager ([`super::sm_mock`]) and
//! the display block model ([`crate::model_imx95_display`]), for the Toradex
//! 10.1" LVDS panel; and the register state after the bring-up against the
//! board's Linux dump (driver/docs/imx95-display-dump-linux.txt) through the
//! allow list (driver/docs/imx95-display-allow.txt). The expected requests
//! and words are written out by hand from the sources the modules cite and
//! the dump, not computed by the code under test.

use super::display::{self, Bases, Error, StreamConfig, VERDIN_LVDS_10INCH};
use super::display_power::{self, ldb_rates, Config, PixelClock, PixelSource};
use super::dpu::{self, Form, Framebuffer, Refusal, Route};
use super::ldb::{self, Words};
use super::sm_mock::{req, writes, Req, Sm, C, F, P};
use crate::firmware::scmi::{self, Transport};
use crate::model_imx95_display::{self as dm, Access, Display};
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;
use std::vec::Vec;

struct Port(Rc<RefCell<Sm>>);

impl Transport for Port {
    fn transfer(&mut self, tx: &[u8], rx: &mut [u8]) -> Result<usize, i32> {
        self.0.borrow_mut().transfer(tx, rx)
    }
}

fn rig() -> (Port, Display) {
    let sm = Rc::new(RefCell::new(Sm::verdin()));
    (Port(sm.clone()), Display::new(sm))
}

const FB: Framebuffer = Framebuffer { addr: 0x9000_0000, stride: 1280 * 4 };
const LVDS: Route = Route::Lvds { pixel_link: true };

fn cfg() -> StreamConfig {
    StreamConfig { mode: VERDIN_LVDS_10INCH, route: LVDS, fb: FB, power: Config::VERDIN_LVDS_10INCH, form: Form::Hal }
}

fn nodelay() -> impl FnMut(u32) {
    |_| {}
}

/// The power-on's requests for the LVDS panel: no DISP1PIX, no VIDEOPLL1.
fn lvds_on() -> Vec<Req> {
    vec![
        req(P, scmi::PROTOCOL_VERSION, &[]),
        req(F, scmi::PROTOCOL_VERSION, &[]),
        req(C, scmi::PROTOCOL_VERSION, &[]),
        req(P, scmi::POWER_STATE_SET, &[0, 13, 0]),
        req(P, scmi::POWER_STATE_GET, &[13]),
        req(F, scmi::PERF_LEVEL_GET, &[12]),
        // 2,986,200,000 = 0xb1fd_cbc0; 497,700,000 = 0x1daa_4ca0
        req(C, scmi::CLOCK_RATE_SET, &[0, 37, 0xb1fd_cbc0, 0]),
        req(C, scmi::CLOCK_RATE_SET, &[0, 38, 0x1daa_4ca0, 0]),
        req(C, scmi::CLOCK_CONFIG_SET, &[37, 1, 0]),
        req(C, scmi::CLOCK_CONFIG_SET, &[38, 1, 0]),
        req(C, scmi::CLOCK_RATE_GET, &[38]),
    ]
}

#[test]
fn the_ldb_pll_ids_are_linuxs() {
    // imx95-clock.h IMX95_CLK_LDBPLL_VCO 37, IMX95_CLK_LDBPLL 38
    assert_eq!((display_power::CLK_LDBPLL_VCO, display_power::CLK_LDBPLL), (37, 38));
    assert_eq!(PixelSource::LdbPll.chain(), &[37, 38]);
    assert_eq!(PixelSource::VideoPll1.chain(), &[18, 19, 80]);
    assert_eq!(ldb::LDB_BASE, dm::LDB);
}

#[test]
fn the_ldb_pll_rates_are_6_x_7_x_the_pixel_clock() {
    // 71.1 MHz x 7 = 497.7 MHz (Linux's ldbpll), x 6 = 2,986.2 MHz
    // (ldbpll_vco): the overlay's "LDBPLL_VCO = 6 * 7 * pixelclk"
    let p = ldb_rates(71_100_000).unwrap();
    assert_eq!(p, PixelClock { vco_hz: 2_986_200_000, pll_hz: 497_700_000, pix_hz: 71_100_000 });
    assert_eq!(p, Config::VERDIN_LVDS_10INCH.pixel);
    // the VCO's range 2.52..4.992 GHz bounds the pixel clock at 60 MHz and
    // 118.857 MHz
    assert_eq!(ldb_rates(60_000_000).unwrap().vco_hz, 2_520_000_000);
    assert_eq!(ldb_rates(59_999_999), None);
    assert_eq!(ldb_rates(118_857_142).unwrap().vco_hz, 4_991_999_964);
    assert_eq!(ldb_rates(118_857_143), None);
    assert_eq!(ldb_rates(0), None);
}

#[test]
fn power_on_sets_and_enables_the_ldb_pll_and_reads_the_pixel_clock_as_ldbpll_over_7() {
    let (mut t, _) = rig();
    let up = display_power::power_on(&mut t, &Config::VERDIN_LVDS_10INCH).unwrap();
    let sm = t.0.borrow();
    assert_eq!(sm.log, lvds_on());
    assert_eq!((up.level, up.pix_hz), (3, 71_100_000));
    assert!(sm.on(13) && sm.enabled(37) && sm.enabled(38));
    // DISP1PIX stays as it was, unused (Linux: 24 MHz, no user)
    assert!(!sm.enabled(80) && !sm.enabled(18) && !sm.enabled(19));
    assert_eq!(sm.rate_of(80), 24_000_000);
}

#[test]
fn an_ldb_chain_that_is_not_6_x_7_x_the_pixel_clock_sends_nothing() {
    let cases = [
        PixelClock { vco_hz: 2_986_200_000, pll_hz: 497_700_000, pix_hz: 70_000_000 },
        PixelClock { vco_hz: 3_360_000_000, pll_hz: 420_000_000, pix_hz: 70_000_000 },
        // 50 MHz: a 2,100 MHz VCO, below the range
        PixelClock { vco_hz: 2_100_000_000, pll_hz: 350_000_000, pix_hz: 50_000_000 },
    ];
    for pixel in cases {
        let (mut t, _) = rig();
        let cfg = Config { pixel, ..Config::VERDIN_LVDS_10INCH };
        let e = display_power::power_on(&mut t, &cfg).unwrap_err();
        assert_eq!(e, display_power::Error::LdbChain(pixel));
        assert!(t.0.borrow().log.is_empty(), "{pixel:?}");
    }
}

#[test]
fn a_denied_ldb_pll_is_named_and_nothing_follows() {
    let (mut t, _) = rig();
    t.0.borrow_mut().clk_perm.insert(37, super::sm_mock::PERM_SET);
    let e = display_power::power_on(&mut t, &Config::VERDIN_LVDS_10INCH).unwrap_err();
    assert_eq!(
        e,
        display_power::Error::Refused { step: display_power::Step::RateSet(37), status: super::sm::Status::Denied }
    );
    assert_eq!(format!("{e}"), "CLOCK_RATE_SET (0x14/0x05) clock 37: SCMI DENIED (-3)");
    assert_eq!(t.0.borrow().log, lvds_on()[..7].to_vec());
}

#[test]
fn power_off_turns_the_ldb_pll_off_backwards_then_the_domain() {
    let (mut t, _) = rig();
    display_power::power_on(&mut t, &Config::VERDIN_LVDS_10INCH).unwrap();
    t.0.borrow_mut().log.clear();
    display_power::power_off(&mut t, PixelSource::LdbPll).unwrap();
    assert_eq!(
        t.0.borrow().log,
        vec![
            req(C, scmi::CLOCK_CONFIG_SET, &[38, 0, 0]),
            req(C, scmi::CLOCK_CONFIG_SET, &[37, 0, 0]),
            req(P, scmi::POWER_STATE_SET, &[0, 13, 0x4000_0000]),
            req(P, scmi::POWER_STATE_GET, &[13]),
        ]
    );
}

#[test]
fn the_lvds_timing_words_are_linuxs_framegen0() {
    // The dump: HTCFG1 0x059F0500 (htotal - 1 = 1439, 1280), HTCFG2
    // 0x80630027 (HsEn, hsync + hback - 1 = 99, hsync - 1 = 39), VTCFG1
    // 0x03360320 (vtotal - 1 = 822, 800), VTCFG2 0x800F0008 (VsEn, 15, 8):
    // no DSI quirk on LVDS.
    let t = dpu::timing(&VERDIN_LVDS_10INCH, LVDS).unwrap();
    assert_eq!((t.htcfg1, t.htcfg2, t.vtcfg1, t.vtcfg2), (0x059f_0500, 0x8063_0027, 0x0336_0320, 0x800f_0008));
    // hal's kick: row vtotal = 823, column htotal = 1440, both kicks
    assert_eq!((t.kick, t.skick), (0x8337_05a0, Some(0x8337_05a0)));
    assert_eq!((t.panic, t.ccr, t.burst, t.alpha_const), (6, 0x7fff_ffff, 0x1010, false));
    // Linux's: row 800, column 1281 (0x83200501), no secondary kick, panic
    // constant colour green, the burst word 0x1FE7F010, AlphaConstEnable
    let l = dpu::timing_form(&VERDIN_LVDS_10INCH, LVDS, Form::Linux).unwrap();
    assert_eq!((l.htcfg1, l.htcfg2, l.vtcfg1, l.vtcfg2), (t.htcfg1, t.htcfg2, t.vtcfg1, t.vtcfg2));
    assert_eq!((l.kick, l.skick), (0x8320_0501, None));
    assert_eq!((l.panic, l.ccr, l.burst, l.alpha_const), (1, 0x000f_fc00, 0x1fe7_f010, true));
    // the DSI words do not move (display_tests' the_10inch_timing_words)
    let d = dpu::timing(&display::VERDIN_DSI_10INCH, Route::Dsi).unwrap();
    assert_eq!((d.vtcfg1, d.kick, d.skick), (0x0347_0320, 0x8348_057c, Some(0x8348_057c)));
    // 71.1 MHz / (1440 x 823) = 59.99 Hz: a frame is 16,669 µs
    assert_eq!(display::frame_us(&VERDIN_LVDS_10INCH, 71_100_000), 16_669);
    assert_eq!(VERDIN_LVDS_10INCH.htotal() * VERDIN_LVDS_10INCH.vtotal(), 1440 * 823);
}

#[test]
fn prepare_on_lvds_selects_the_ldb_clock_and_keeps_the_pixel_link_as_linux() {
    let (mut t, m) = rig();
    let p = display::prepare(&mut t, &m, &Bases::PHYS, &cfg(), &mut nodelay()).unwrap();
    assert_eq!(t.0.borrow().log, lvds_on());
    assert!(m.refused().is_empty());
    assert_eq!((p.pix_hz, p.source, p.route), (71_100_000, PixelSource::LdbPll, LVDS));
    let w = m.writes();
    // DSIP_CLK0_SEL 2 (LDB PLL / 7) and bypass first; the interleaver's
    // reset pulse and CTRL 0x600; the pixel link valid then enable, last
    assert_eq!(&w[..5], &[("csr", 0x0, 2), ("csr", 0x4, 0), ("il", 0x20, 0x2), ("il", 0x20, 0), ("il", 0x0, 0x600)]);
    assert_eq!(&w[w.len() - 2..], &[("csr", 0x8, 0x2), ("csr", 0x8, 0x3)]);
    // nothing of the LDB here: that is the LVDS side's step
    assert!(!w.iter().any(|x| x.0 == "ldb"));
    assert!(!w.iter().any(|x| x.1 == 0x2b_008c));
}

#[test]
fn without_the_pixel_link_option_the_link_is_never_written() {
    let (mut t, m) = rig();
    let c = StreamConfig { route: Route::Lvds { pixel_link: false }, ..cfg() };
    let p = display::prepare(&mut t, &m, &Bases::PHYS, &c, &mut nodelay()).unwrap();
    let s = display::start(&m, &Bases::PHYS, &p, &mut nodelay()).unwrap();
    let pr = display::proof(&m, &Bases::PHYS, &s, &mut nodelay());
    assert!(pr.consistent(&s), "{pr:?}");
    assert_eq!((pr.clock_control, pr.pixel_link), (2, 0));
    display::stop(&mut t, &m, &Bases::PHYS, &s, &mut nodelay()).unwrap();
    assert!(!m.writes().iter().any(|x| x.0 == "csr" && x.1 == 0x8));
}

#[test]
fn framegen0_runs_only_from_the_ldb_pll_when_the_select_is_2() {
    let (mut t, m) = rig();
    let p = display::prepare(&mut t, &m, &Bases::PHYS, &cfg(), &mut nodelay()).unwrap();
    // LDBPLL off: the engine has no clock
    t.0.borrow_mut().clk_on.remove(&38);
    let e = display::start(&m, &Bases::PHYS, &p, &mut nodelay()).unwrap_err();
    assert!(matches!(e, Error::NoSync(s) if s.fgensts == 0), "{e:?}");
    // and DISP1PIX does not stand in for it
    let mut sm = t.0.borrow_mut();
    sm.clk_on.insert(80);
    drop(sm);
    assert!(display::start(&m, &Bases::PHYS, &p, &mut nodelay()).is_err());
}

#[test]
fn a_mode_whose_clock_is_not_the_ldb_chains_is_refused_before_anything() {
    let (mut t, m) = rig();
    let mut c = cfg();
    c.mode.pixel_clock_hz = 70_000_000;
    assert_eq!(
        display::prepare(&mut t, &m, &Bases::PHYS, &c, &mut nodelay()).unwrap_err(),
        Error::Clock { mode_hz: 70_000_000, pix_hz: 71_100_000 }
    );
    let mut c = cfg();
    c.mode.vfront = 0x4000;
    assert_eq!(
        display::prepare(&mut t, &m, &Bases::PHYS, &c, &mut nodelay()).unwrap_err(),
        Error::Refused(Refusal::Mode)
    );
    assert!(t.0.borrow().log.is_empty() && m.log().is_empty());
}

#[test]
fn the_ldb_words_are_the_dumps_and_hals_init_is_the_named_alternative() {
    // the dump: 0x14, 0x00082221, 0x1075, 0
    let l = ldb::words(&VERDIN_LVDS_10INCH, ldb::Form::Linux);
    assert_eq!(l, Words { phy_clock: Some(0x14), mapper: 0x0008_2221, lvds0: 0x1075, lvds1: None });
    // LDB_Init(base, 0, 0, LVDS_SPWG) from reset: CH0 and CH1 to DI0 (0x5),
    // both 24 bit (0xa0), DI0 VSYNC low (0x200); both PHYs 0x1075; the clock
    // control untouched
    let h = ldb::words(&VERDIN_LVDS_10INCH, ldb::Form::HalInit);
    assert_eq!(h, Words { phy_clock: None, mapper: 0x2a5, lvds0: 0x1075, lvds1: Some(0x1075) });
    // the PHY's parked word: CH0_EN | LVDS_EN ("1: disable") | BG_EN
    assert_eq!(ldb::PHY_PARKED, 0x7);
    // a panel with active-high syncs drops the two polarity bits
    let mut hi = VERDIN_LVDS_10INCH;
    hi.hsync_high = true;
    hi.vsync_high = true;
    assert_eq!(ldb::words(&hi, ldb::Form::Linux).mapper, 0x0008_0021);
}

#[test]
fn the_ldb_comes_up_in_fsl_ldbs_order_with_the_100_ms_restart() {
    let (mut t, m) = rig();
    display::prepare(&mut t, &m, &Bases::PHYS, &cfg(), &mut nodelay()).unwrap();
    m.clear_log();
    let mut waits = Vec::new();
    ldb::up(&m, ldb::LDB_BASE, &ldb::words(&VERDIN_LVDS_10INCH, ldb::Form::Linux), &mut |us| waits.push(us));
    assert_eq!(
        m.writes(),
        vec![("ldb", 0x0, 0x14), ("ldb", 0x4, 0x0008_2221), ("ldb", 0x8, 0x7), ("ldb", 0x8, 0x1075)]
    );
    assert_eq!(waits, vec![100_000]);
    m.clear_log();
    ldb::up(&m, ldb::LDB_BASE, &ldb::words(&VERDIN_LVDS_10INCH, ldb::Form::HalInit), &mut nodelay());
    assert_eq!(
        m.writes(),
        vec![("ldb", 0x4, 0x2a5), ("ldb", 0x8, 0x7), ("ldb", 0xc, 0x7), ("ldb", 0x8, 0x1075), ("ldb", 0xc, 0x1075)]
    );
    m.clear_log();
    ldb::down(&m, ldb::LDB_BASE);
    assert_eq!(m.writes(), vec![("ldb", 0x8, 0), ("ldb", 0xc, 0), ("ldb", 0x4, 0), ("ldb", 0x0, 0x1e)]);
}

#[test]
fn the_model_refuses_the_seven_words_that_fault_on_the_board() {
    let (mut t, m) = rig();
    display::prepare(&mut t, &m, &Bases::PHYS, &cfg(), &mut nodelay()).unwrap();
    for off in dm::FAULTS {
        crate::mmio::Mmio::read32(&m, dm::DPU + off as usize);
    }
    assert_eq!(m.refused().len(), 7);
    assert!(m.refused().iter().all(|a| matches!(a, Access::R("dpu", _, 0))));
}

// ---- the register state against the board's Linux --------------------------

const DUMP: &str = include_str!("../../../../docs/imx95-display-dump-linux.txt");
const ALLOW: &str = include_str!("../../../../docs/imx95-display-allow.txt");

fn hex(s: &str) -> u32 {
    u32::from_str_radix(s.trim_start_matches("0x"), 16).unwrap()
}

/// The dump's words, address to value.
fn dump() -> BTreeMap<u32, u32> {
    DUMP.lines()
        .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
        .map(|l| {
            let f: Vec<&str> = l.split_whitespace().collect();
            assert_eq!(f[1], "32", "{l}");
            (hex(f[0]), hex(f[2]))
        })
        .collect()
}

/// The dump's faulting words (its "# ADDRESS FAULT" lines).
fn faults() -> BTreeSet<u32> {
    DUMP.lines()
        .filter_map(|l| l.strip_prefix("# 0x"))
        .filter(|l| l.ends_with(" FAULT"))
        .map(|l| hex(l.split_whitespace().next().unwrap()))
        .collect()
}

/// The allow list's masks, address to the bits allowed to differ.
fn allow() -> BTreeMap<u32, u32> {
    let mut a = BTreeMap::new();
    for l in ALLOW.lines().filter(|l| !l.starts_with('#') && !l.trim().is_empty()) {
        let f: Vec<&str> = l.split_whitespace().collect();
        *a.entry(hex(f[0])).or_insert(0) |= hex(f[1]);
    }
    a
}

/// The model's block and offset of a physical address in the dump.
fn block(a: u32) -> (&'static str, u32) {
    let a = a as usize;
    if (dm::CSR..dm::CSR + 0x1_0000).contains(&a) {
        ("csr", (a - dm::CSR) as u32)
    } else if (dm::LDB..dm::LDB + 0x1_0000).contains(&a) {
        ("ldb", (a - dm::LDB) as u32)
    } else if (dm::IL..dm::IL + 0x2_0000).contains(&a) {
        ("il", (a - dm::IL) as u32)
    } else {
        ("dpu", (a - dm::DPU) as u32)
    }
}

/// The SoC half of the X12 bring-up (display::prepare, ldb::up,
/// display::start) with these forms; the model afterwards.
fn brought_up(form: Form, ldb_form: ldb::Form) -> Display {
    let (mut t, m) = rig();
    let c = StreamConfig { form, ..cfg() };
    let p = display::prepare(&mut t, &m, &Bases::PHYS, &c, &mut nodelay()).unwrap();
    ldb::up(&m, ldb::LDB_BASE, &ldb::words(&p.mode, ldb_form), &mut nodelay());
    display::start(&m, &Bases::PHYS, &p, &mut nodelay()).unwrap();
    assert!(m.refused().is_empty(), "{:?}", m.refused());
    m
}

/// Every dumped word the bring-up wrote, (address, ours, Linux's), that
/// differs outside `mask`.
fn differ(m: &Display, mask: &BTreeMap<u32, u32>) -> Vec<(u32, u32, u32)> {
    let written: BTreeSet<(&str, u32)> = m.writes().iter().map(|w| (w.0, w.1)).collect();
    dump()
        .into_iter()
        .filter(|(a, _)| written.contains(&block(*a)))
        .filter_map(|(a, linux)| {
            let (b, o) = block(a);
            let ours = m.peek(b, o);
            ((ours ^ linux) & !mask.get(&a).copied().unwrap_or(0) != 0).then_some((a, ours, linux))
        })
        .collect()
}

#[test]
fn the_dump_parses_and_the_faulting_words_are_the_models() {
    let d = dump();
    assert_eq!(d.len(), 4 + 9 + 3 + 1 + 16 + 17 + 3 + 37 + 11);
    assert_eq!(d[&0x4b0c_0000], 0x14);
    assert_eq!(d[&0x4b01_0008], 0x3);
    let f = faults();
    let want: BTreeSet<u32> = dm::FAULTS.iter().map(|o| dm::DPU as u32 + o).collect();
    assert_eq!(f, want);
    assert!(f.iter().all(|a| !d.contains_key(a)));
}

#[test]
fn with_linuxs_forms_every_word_the_path_writes_equals_the_dump_but_the_buffer() {
    let m = brought_up(Form::Linux, ldb::Form::Linux);
    // only the frame buffer's address may differ
    let buffer = BTreeMap::from([(0x4b5d_0018, 0xffff_ffff), (0x4b5d_001c, 0xffff_ffff)]);
    assert_eq!(differ(&m, &buffer), vec![]);
}

#[test]
fn with_the_defaults_the_words_that_differ_are_the_named_options_and_the_allow_list_covers_them() {
    let m = brought_up(Form::Hal, ldb::Form::Linux);
    let buffer = BTreeMap::from([(0x4b5d_0018, 0xffff_ffff), (0x4b5d_001c, 0xffff_ffff)]);
    // fsl_dpu.c's words where the dump has Linux's (dpu::Form::Linux)
    assert_eq!(
        differ(&m, &buffer),
        vec![
            (0x4b5d_0010, 0x0000_1010, 0x1fe7_f010),
            (0x4b5d_0048, 0x8000_0000, 0x8000_0200),
            (0x4b6b_002c, 0x8337_05a0, 0x8320_0501),
            (0x4b6b_0030, 0x8337_05a0, 0x00f0_0140),
            (0x4b6b_0084, 0x0000_0006, 0x0000_0001),
            (0x4b6b_0088, 0x7fff_ffff, 0x000f_fc00),
        ]
    );
    // and the allow list names exactly those bits (with the buffer, the
    // status words and the CRCs)
    assert_eq!(differ(&m, &allow()), vec![]);
}

#[test]
fn hals_ldb_init_differs_from_the_dump_in_the_mapper_and_phy_1() {
    let m = brought_up(Form::Linux, ldb::Form::HalInit);
    let buffer = BTreeMap::from([(0x4b5d_0018, 0xffff_ffff), (0x4b5d_001c, 0xffff_ffff)]);
    assert_eq!(differ(&m, &buffer), vec![(0x4b0c_0004, 0x2a5, 0x0008_2221), (0x4b0c_000c, 0x1075, 0)]);
}

#[test]
fn the_dumped_words_the_path_does_not_write_are_these() {
    // What the cold path leaves at its reset value (or never touches): the
    // dump's value there is Linux's or the reset value, which a read before
    // our first write on the board tells apart (the probe imx95_lvds_ids).
    let m = brought_up(Form::Hal, ldb::Form::Linux);
    let written: BTreeSet<(&str, u32)> = m.writes().iter().map(|w| (w.0, w.1)).collect();
    let untouched: Vec<u32> = dump().into_keys().filter(|a| !written.contains(&block(*a))).collect();
    let mut want = vec![0x4b0c_000c];
    want.extend([0x4b01_000c, 0x4b01_0010, 0x4b01_0014, 0x4b01_0018, 0x4b01_001c, 0x4b01_0020]);
    want.push(0x4b0d_0050);
    want.push(0x4b40_0000);
    want.extend([0x4b5d_0020, 0x4b5d_0024, 0x4b5d_003c, 0x4b5d_0040]);
    want.extend((0x4b60_0008..=0x4b60_004c).step_by(4).filter(|a| *a != 0x4b60_0024));
    want.extend([0x4b51_000c, 0x4b51_0014]);
    want.extend((0x4b6b_001c..=0x4b6b_0028).step_by(4));
    want.extend((0x4b6b_0034..=0x4b6b_0074).step_by(4));
    want.extend([0x4b6b_0094, 0x4b6b_0098, 0x4b6b_009c]);
    want.extend((0x4b6d_0008..=0x4b6d_0028).step_by(4).filter(|a| *a != 0x4b6d_0020));
    want.extend([0x4b6d_0400, 0x4b6d_0404, 0x4b6d_0408]);
    want.sort();
    assert_eq!(untouched, want);
}

#[test]
fn the_sms_display_clocks_are_never_sent_on_lvds_either() {
    let (mut t, m) = rig();
    let p = display::prepare(&mut t, &m, &Bases::PHYS, &cfg(), &mut nodelay()).unwrap();
    let s = display::start(&m, &Bases::PHYS, &p, &mut nodelay()).unwrap();
    display::stop(&mut t, &m, &Bases::PHYS, &s, &mut nodelay()).unwrap();
    for w in writes(&t.0.borrow().log) {
        let clk = if w.2.len() == 4 { w.2[1] } else { w.2[0] };
        if w.0 == C {
            assert!([37, 38].contains(&clk), "{w:?}");
        }
    }
    assert!(!t.0.borrow().on(13));
}
