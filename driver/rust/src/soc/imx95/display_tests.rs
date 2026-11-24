// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! [`super::display_power`], [`super::dpu`] and [`super::display`] against
//! the mock System Manager ([`super::sm_mock`]) and the display block model
//! ([`crate::model_imx95_display`]), for the Toradex 10.1" DSI display's
//! 1280x800 mode. The expected requests and register words are written out
//! by hand from the sources the modules cite (the values are worked in the
//! comments), not computed by the code under test.

use super::display::{self, Bases, Error, StreamConfig, VERDIN_DSI_10INCH};
use super::display_power::{self, Config, PixelSource, Step, CLK_DISPAPB, CLK_DISPAXI, CLK_DISPOCRAM};
use super::dpu::{self, Form, Framebuffer, Refusal, Route};
use super::sm::Status;
use super::sm_mock::{req, writes, Req, Sm, C, F, P, PERM_SET};
use crate::firmware::scmi::{self, Transport};
use crate::model_imx95_display::{Access, Display};
use std::cell::RefCell;
use std::rc::Rc;

/// The mock SM shared between the SCMI side and the register model.
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

fn cfg() -> StreamConfig {
    StreamConfig {
        mode: VERDIN_DSI_10INCH,
        route: Route::Dsi,
        fb: FB,
        power: Config::VERDIN_DSI_10INCH,
        form: Form::Hal,
    }
}

fn nodelay() -> impl FnMut(u32) {
    |_| {}
}

/// The power-on's requests with the DISP level left as the SM has it.
fn power_on_reqs() -> Vec<Req> {
    vec![
        req(P, scmi::PROTOCOL_VERSION, &[]),
        req(F, scmi::PROTOCOL_VERSION, &[]),
        req(C, scmi::PROTOCOL_VERSION, &[]),
        req(P, scmi::POWER_STATE_SET, &[0, 13, 0]),
        req(P, scmi::POWER_STATE_GET, &[13]),
        req(F, scmi::PERF_LEVEL_GET, &[12]),
        req(C, 0xd, &[80, 19]),
        req(C, 0xe, &[80]),
        // 3,360,000,000 = 0xc845_8800
        req(C, scmi::CLOCK_RATE_SET, &[0, 18, 0xc845_8800, 0]),
        req(C, scmi::CLOCK_RATE_SET, &[0, 19, 420_000_000, 0]),
        req(C, scmi::CLOCK_RATE_SET, &[0, 80, 70_000_000, 0]),
        req(C, scmi::CLOCK_CONFIG_SET, &[18, 1, 0]),
        req(C, scmi::CLOCK_CONFIG_SET, &[19, 1, 0]),
        req(C, scmi::CLOCK_CONFIG_SET, &[80, 1, 0]),
        req(C, scmi::CLOCK_RATE_GET, &[80]),
    ]
}

fn power_off_reqs() -> Vec<Req> {
    vec![
        req(C, scmi::CLOCK_CONFIG_SET, &[80, 0, 0]),
        req(C, scmi::CLOCK_CONFIG_SET, &[19, 0, 0]),
        req(C, scmi::CLOCK_CONFIG_SET, &[18, 0, 0]),
        req(P, scmi::POWER_STATE_SET, &[0, 13, 0x4000_0000]),
        req(P, scmi::POWER_STATE_GET, &[13]),
    ]
}

/// What `prepare` writes for the 10.1" panel with [`FB`], in order.
fn prepare_writes() -> Vec<(&'static str, u32, u32)> {
    vec![
        // display CSR: DSIP_CLK0_SEL 0 (CCM), interleaver bypass
        ("csr", 0x0, 0),
        ("csr", 0x4, 0),
        // interleaver channel 0: SW_RST pulse, HSYNC|VSYNC active low
        ("il", 0x20, 0x2),
        ("il", 0x20, 0),
        ("il", 0x0, 0x600),
        // DPU_Init: ExtDst0's domain mask
        ("dpu", 0x200c, 0xffff_ffff),
        // DPU_PreparePathConfig for the path
        ("dpu", 0x11_1008, 0),
        ("dpu", 0x11_100c, 0),
        ("dpu", 0x17_1008, 0),
        ("dpu", 0x1d_000c, 0xff),
        // DPU_InitPipeline(ExtDst0): ShdEn, powerdown clear
        ("dpu", 0x11_1008, 0x1),
        // DPU_InitExtDst: KICK_MODE | ShdEn; source LayerBlend1 (0x14)
        ("dpu", 0x11_0008, 0x101),
        ("dpu", 0x11_100c, 0x14),
        // LayerBlend1: ShdEn | ShdTokSel 2 << 3 | ShdLdSel 2 << 1; position;
        // prim ConstFrame0 0x0c, sec FetchLayer0 0x1a << 8, clken 1 << 24
        ("dpu", 0x17_0008, 0x15),
        ("dpu", 0x17_0014, 0),
        ("dpu", 0x17_1008, 0x0100_1a0c),
        // sec colour and alpha * 1 (0x10 | 0x1000), prim * 0; MODE blend
        ("dpu", 0x17_0010, 0x1010),
        ("dpu", 0x17_000c, 0x1),
        // DomainBlend0: ShdEn | 4 << 4 | 4 << 1; BLENDCONTROL 0x10
        ("dpu", 0x2a_0008, 0x49),
        ("dpu", 0x2a_0018, 0x10),
        // ConstFrame0: 799 << 16 | 1279, black
        ("dpu", 0xf_0008, 1),
        ("dpu", 0xf_000c, 0x031f_04ff),
        ("dpu", 0xf_0010, 0),
        // FetchLayer0: ShdEn; 16 buffers, burst 16; frame; RawPixel off;
        // resampling deltas 4 << 18 | 4 << 12
        ("dpu", 0x1d_0008, 1),
        ("dpu", 0x1d_0010, 0x1010),
        ("dpu", 0x1d_01d8, 0x031f_04ff),
        ("dpu", 0x1d_01e0, 0),
        ("dpu", 0x1d_01dc, 0x10_4000),
        // sublayer 0: address, 32 bpp << 16 | 5119, 799 << 16 | 1279,
        // bits 8/8/8/0, shifts 16/8/0/0, constant alpha 0xff, offset 0,
        // SourceBufferEnable
        ("dpu", 0x1d_0018, 0x9000_0000),
        ("dpu", 0x1d_001c, 0),
        ("dpu", 0x1d_0028, 0x0020_13ff),
        ("dpu", 0x1d_002c, 0x031f_04ff),
        ("dpu", 0x1d_0030, 0x0808_0800),
        ("dpu", 0x1d_0034, 0x1008_0000),
        ("dpu", 0x1d_0044, 0xff),
        ("dpu", 0x1d_0038, 0),
        ("dpu", 0x1d_0048, 0x8000_0000),
        // pipeline and DomainBlend0 shadow loads
        ("dpu", 0x11_1014, 1),
        ("dpu", 0x2a_000c, 1),
        // FrameGen0: ShdEn; DSI polarity DE high, syncs low (PolEn 0x4)
        ("dpu", 0x2b_0008, 1),
        ("dpu", 0x31_1008, 0x4),
        // V front 8 - 1 = 7, back 24 + 1 = 25 (DSI): vtotal - 1 = 839;
        // HTCFG1 1403 << 16 | 1280; HTCFG2 HsEn | 87 << 16 | 23;
        // VTCFG1 839 << 16 | 800; VTCFG2 VsEn | 32 << 16 | 7;
        // kick: En | 840 << 16 | 1404
        ("dpu", 0x2b_000c, 0x057b_0500),
        ("dpu", 0x2b_0010, 0x8057_0017),
        ("dpu", 0x2b_0014, 0x0347_0320),
        ("dpu", 0x2b_0018, 0x8020_0007),
        ("dpu", 0x2b_002c, 0x8348_057c),
        ("dpu", 0x2b_0030, 0x8348_057c),
        // areas at (1, 1); primary only; panic test pattern; constant
        // colour white, alpha 1 << 30; shadow token
        ("dpu", 0x2b_0078, 0x0001_0001),
        ("dpu", 0x2b_007c, 0x0001_0001),
        ("dpu", 0x2b_0080, 2),
        ("dpu", 0x2b_0084, 6),
        ("dpu", 0x2b_0088, 0x7fff_ffff),
        ("dpu", 0x2b_0090, 1),
        // pixel link 0: valid, then enable (0x3)
        ("csr", 0x8, 0x2),
        ("csr", 0x8, 0x3),
    ]
}

#[test]
fn the_ids_are_the_sms_and_linuxs() {
    // imx95-power.h IMX95_PD_DISPLAY 13, IMX95_PERF_DISP 12; imx95-clock.h
    // VIDEOPLL1_VCO 18, VIDEOPLL1 19, CCM_NUM_CLK_SRC 41: DISPAPB +34,
    // DISPAXI +35, DISPOCRAM +37, DISP1PIX +39.
    assert_eq!((display_power::PD_DISPLAY, display_power::PERF_DISP), (13, 12));
    assert_eq!((display_power::CLK_VIDEOPLL1_VCO, display_power::CLK_VIDEOPLL1), (18, 19));
    assert_eq!(display_power::CLK_DISP1PIX, 41 + 39);
    assert_eq!((CLK_DISPAPB, CLK_DISPAXI, CLK_DISPOCRAM), (41 + 34, 41 + 35, 41 + 37));
    assert_eq!((scmi::CLOCK_PARENT_SET, scmi::CLOCK_PARENT_GET), (0xd, 0xe));
}

#[test]
fn power_on_powers_the_domain_then_parents_rates_and_enables_the_pixel_clock() {
    let (mut t, _) = rig();
    let up = display_power::power_on(&mut t, &Config::VERDIN_DSI_10INCH).unwrap();
    let sm = t.0.borrow();
    assert_eq!(sm.log, power_on_reqs());
    // the SM left DISP at overdrive and nothing asked otherwise
    assert_eq!((up.level, up.chosen, up.pix_hz), (3, None, 70_000_000));
    assert!(sm.on(13) && sm.enabled(18) && sm.enabled(19) && sm.enabled(80));
    // 3,360 / 8 = 420; 420 / 6 = 70
    assert_eq!((sm.rate_of(19), sm.rate_of(80)), (420_000_000, 70_000_000));
}

#[test]
fn a_level_asked_for_is_described_set_and_read_back() {
    let (mut t, _) = rig();
    let cfg = Config { axi_hz: Some(700_000_000), ..Config::VERDIN_DSI_10INCH };
    let up = display_power::power_on(&mut t, &cfg).unwrap();
    // 666.667 MHz is the fastest DISP level at or below 700 MHz: NOM, 2
    assert_eq!((up.level, up.chosen.unwrap().index), (2, 2));
    let mut want = power_on_reqs();
    want.insert(2, req(F, scmi::PERF_DESCRIBE_LEVELS, &[12, 0]));
    want[6] = req(F, scmi::PERF_LEVEL_SET, &[12, 2]);
    want.insert(7, req(F, scmi::PERF_LEVEL_GET, &[12]));
    assert_eq!(t.0.borrow().log, want);
}

#[test]
fn the_sm_owned_display_clocks_are_never_sent() {
    let (mut t, m) = rig();
    let mut d = nodelay();
    let p = display::prepare(&mut t, &m, &Bases::PHYS, &cfg(), &mut d).unwrap();
    let s = display::start(&m, &Bases::PHYS, &p, &mut d).unwrap();
    display::stop(&mut t, &m, &Bases::PHYS, &s, &mut d).unwrap();
    for (p, _, w) in &t.0.borrow().log {
        if *p == C && !w.is_empty() {
            let clk = if w.len() == 4 { w[1] } else { w[0] };
            assert!(![CLK_DISPAPB, CLK_DISPAXI, CLK_DISPOCRAM].contains(&clk), "{w:?}");
        }
    }
}

#[test]
fn a_denied_parent_is_named_and_nothing_follows() {
    let (mut t, _) = rig();
    // SET is enough to enable, not to reparent (ClockParentSet: EXCLUSIVE)
    t.0.borrow_mut().clk_perm.insert(80, PERM_SET);
    let e = display_power::power_on(&mut t, &Config::VERDIN_DSI_10INCH).unwrap_err();
    assert_eq!(e, display_power::Error::Refused { step: Step::ParentSet(80), status: Status::Denied });
    assert_eq!(format!("{e}"), "CLOCK_PARENT_SET (0x14/0x0d) clock 80: SCMI DENIED (-3)");
    assert_eq!(t.0.borrow().log, power_on_reqs()[..7].to_vec());
}

#[test]
fn a_denied_domain_writes_nothing_else() {
    let (mut t, _) = rig();
    t.0.borrow_mut().pd_perm.clear();
    let e = display_power::power_on(&mut t, &Config::VERDIN_DSI_10INCH).unwrap_err();
    assert_eq!(e, display_power::Error::Refused { step: Step::PowerOn, status: Status::Denied });
    assert_eq!(writes(&t.0.borrow().log), vec![req(P, scmi::POWER_STATE_SET, &[0, 13, 0])]);
}

#[test]
fn a_pixel_clock_the_pll_cannot_make_is_refused() {
    let (mut t, _) = rig();
    // 420 MHz cannot give 71.1 MHz: /6 is 70 MHz, 1.5 % below
    let mut cfg = Config::VERDIN_DSI_10INCH;
    cfg.pixel.pix_hz = 71_100_000;
    let e = display_power::power_on(&mut t, &cfg).unwrap_err();
    assert_eq!(e, display_power::Error::Rate { want_hz: 71_100_000, got_hz: 70_000_000 });
}

#[test]
fn power_off_disables_the_chain_backwards_then_the_domain() {
    let (mut t, _) = rig();
    display_power::power_on(&mut t, &Config::VERDIN_DSI_10INCH).unwrap();
    t.0.borrow_mut().log.clear();
    display_power::power_off(&mut t, PixelSource::VideoPll1).unwrap();
    let sm = t.0.borrow();
    assert_eq!(sm.log, power_off_reqs());
    assert!(!sm.on(13) && !sm.enabled(80));
}

#[test]
fn the_10inch_timing_words() {
    let t = dpu::timing(&VERDIN_DSI_10INCH, Route::Dsi).unwrap();
    assert_eq!(
        (t.htcfg1, t.htcfg2, t.vtcfg1, t.vtcfg2, t.kick, t.polarity),
        (0x057b_0500, 0x8057_0017, 0x0347_0320, 0x8020_0007, 0x8348_057c, 0x4)
    );
    // 70 MHz / (1404 x 840) = 59.36 Hz: a frame is 16,848 µs
    assert_eq!(display::frame_us(&VERDIN_DSI_10INCH, 70_000_000), 16_848);
}

#[test]
fn prepare_writes_the_route_and_the_pipeline_in_order_after_the_power() {
    let (mut t, m) = rig();
    let p = display::prepare(&mut t, &m, &Bases::PHYS, &cfg(), &mut nodelay()).unwrap();
    assert_eq!(t.0.borrow().log, power_on_reqs());
    assert_eq!(m.writes(), prepare_writes());
    assert!(m.refused().is_empty());
    assert_eq!((p.level, p.pix_hz, p.mode, p.fb), (3, 70_000_000, VERDIN_DSI_10INCH, FB));
    // configured, not started
    assert!(!m.writes().iter().any(|w| w.1 == 0x2b_008c));
}

#[test]
fn start_runs_framegen0_until_the_primary_channel_syncs() {
    let (mut t, m) = rig();
    let mut d = nodelay();
    let p = display::prepare(&mut t, &m, &Bases::PHYS, &cfg(), &mut d).unwrap();
    m.clear_log();
    let s = display::start(&m, &Bases::PHYS, &p, &mut d).unwrap();
    assert_eq!(m.writes(), vec![("dpu", 0x2b_008c, 1)]);
    assert!(s.sync_us > 0 && s.sync_us < display::SYNC_WAIT_US);
    assert_eq!(m.scanned(), Some(0x9000_0000));
}

#[test]
fn no_sync_is_a_timeout_with_the_status() {
    let (mut t, mut m) = rig();
    m.faults.no_sync = true;
    let mut d = nodelay();
    let p = display::prepare(&mut t, &m, &Bases::PHYS, &cfg(), &mut d).unwrap();
    let mut waited = 0;
    let e = display::start(&m, &Bases::PHYS, &p, &mut |us| waited += us).unwrap_err();
    let Error::NoSync(st) = e else { panic!("{e:?}") };
    assert_eq!((st.fgenable, st.fgensts, st.fgchstat & 0x100), (1, 1, 0));
    assert_eq!(waited, display::SYNC_WAIT_US);
}

#[test]
fn without_the_pixel_clock_framegen0_never_runs() {
    let (mut t, m) = rig();
    let mut d = nodelay();
    let p = display::prepare(&mut t, &m, &Bases::PHYS, &cfg(), &mut d).unwrap();
    t.0.borrow_mut().clk_on.remove(&80);
    let e = display::start(&m, &Bases::PHYS, &p, &mut d).unwrap_err();
    assert!(matches!(e, Error::NoSync(s) if s.fgensts == 0));
}

#[test]
fn the_dpu_is_not_touched_before_the_domain_is_on() {
    let (mut t, m) = rig();
    t.0.borrow_mut().pd_perm.clear();
    let e = display::prepare(&mut t, &m, &Bases::PHYS, &cfg(), &mut nodelay()).unwrap_err();
    assert!(matches!(e, Error::Power(_)));
    assert!(m.log().is_empty() && m.refused().is_empty());
    // and the model refuses what a careless caller would do
    dpu::start(&m, Bases::PHYS.dpu);
    assert_eq!(m.refused(), vec![Access::W("dpu", 0x2b_008c, 1)]);
}

#[test]
fn bad_modes_and_buffers_send_and_write_nothing() {
    let cases = [
        (Framebuffer { addr: 0x9000_0002, stride: 5120 }, Refusal::Align),
        (Framebuffer { addr: 0x9000_0000, stride: 5118 }, Refusal::Align),
        (Framebuffer { addr: 0x9000_0000, stride: 5116 }, Refusal::Stride),
        (Framebuffer { addr: 0x9000_0000, stride: 0x1_0004 }, Refusal::Stride),
        (Framebuffer { addr: 1 << 40, stride: 5120 }, Refusal::Address),
    ];
    for (fb, why) in cases {
        let (mut t, m) = rig();
        let e = display::prepare(&mut t, &m, &Bases::PHYS, &StreamConfig { fb, ..cfg() }, &mut nodelay());
        assert_eq!(e.unwrap_err(), Error::Refused(why), "{fb:?}");
        assert!(t.0.borrow().log.is_empty() && m.log().is_empty());
    }
    let mut mode = VERDIN_DSI_10INCH;
    mode.hfront = 0x4000;
    assert_eq!(dpu::timing(&mode, Route::Dsi), Err(Refusal::Mode));
    let (mut t, m) = rig();
    let mut c = cfg();
    c.power.pixel.pix_hz = 71_100_000;
    assert_eq!(
        display::prepare(&mut t, &m, &Bases::PHYS, &c, &mut nodelay()).unwrap_err(),
        Error::Clock { mode_hz: 70_000_000, pix_hz: 71_100_000 }
    );
    assert!(t.0.borrow().log.is_empty());
}

#[test]
fn a_flip_loads_the_new_address_at_the_next_frame() {
    let (mut t, m) = rig();
    let mut d = nodelay();
    let p = display::prepare(&mut t, &m, &Bases::PHYS, &cfg(), &mut d).unwrap();
    let mut s = display::start(&m, &Bases::PHYS, &p, &mut d).unwrap();
    m.clear_log();
    display::flip(&m, &Bases::PHYS, &mut s, 0x1_9040_0000, &mut d).unwrap();
    assert_eq!(
        m.writes(),
        vec![
            // clear ExtDst0's shadow-load flag (bit 3), the address (bits
            // 39:32 in the MSB register), the pipeline's sync trigger, then
            // the flag cleared after it was seen
            ("dpu", 0x1020, 0x8),
            ("dpu", 0x1d_0018, 0x9040_0000),
            ("dpu", 0x1d_001c, 0x1),
            ("dpu", 0x11_1014, 1),
            ("dpu", 0x1020, 0x8),
        ]
    );
    assert_eq!((m.scanned(), s.fb.addr), (Some(0x1_9040_0000), 0x1_9040_0000));
}

#[test]
fn a_flip_that_never_loads_times_out_after_three_frames() {
    let (mut t, mut m) = rig();
    let mut d = nodelay();
    let p = display::prepare(&mut t, &m, &Bases::PHYS, &cfg(), &mut d).unwrap();
    let mut s = display::start(&m, &Bases::PHYS, &p, &mut d).unwrap();
    m.faults.no_shadow_load = true;
    let e = display::flip(&m, &Bases::PHYS, &mut s, 0x9100_0000, &mut d).unwrap_err();
    assert_eq!(e, Error::FlipTimeout(0));
    // the stream keeps the buffer it had
    assert_eq!(s.fb, FB);
}

#[test]
fn the_proof_reads_back_what_was_set_and_the_crc_follows_the_content() {
    let (mut t, mut m) = rig();
    m.content.insert(0x9000_0000, 0x1234);
    m.content.insert(0x9100_0000, 0x1235);
    let mut d = nodelay();
    let p = display::prepare(&mut t, &m, &Bases::PHYS, &cfg(), &mut d).unwrap();
    let mut s = display::start(&m, &Bases::PHYS, &p, &mut d).unwrap();
    m.clear_log();
    let pr = display::proof(&m, &Bases::PHYS, &s, &mut d);
    assert!(pr.consistent(&s), "{pr:?}");
    assert_eq!((pr.dpu.attributes, pr.dpu.dimension, pr.dpu.layerproperty), (0x0020_13ff, 0x031f_04ff, 0x8000_0000));
    assert_eq!((pr.dpu.htcfg1, pr.dpu.vtcfg1), (0x057b_0500, 0x0347_0320));
    assert_eq!((pr.clock_control, pr.interleaver_control, pr.pixel_link), (0, 0, 0x3));
    // Signature0 window 0 over the active area: corners (0, 0), (1279, 799)
    assert_eq!(
        m.writes(),
        vec![
            ("dpu", 0x2d_0008, 1),
            ("dpu", 0x2d_0014, 0),
            ("dpu", 0x2d_001c, 1),
            ("dpu", 0x2d_0030, 0),
            ("dpu", 0x2d_0034, 0x031f_04ff),
            ("dpu", 0x2d_002c, 1),
            ("dpu", 0x2d_0018, 1),
        ]
    );
    let a = pr.crc.unwrap();
    // stable while the buffer is the same
    assert_eq!(display::proof(&m, &Bases::PHYS, &s, &mut d).crc, Some(a));
    // and it changes with the content
    display::flip(&m, &Bases::PHYS, &mut s, 0x9100_0000, &mut d).unwrap();
    let b = display::proof(&m, &Bases::PHYS, &s, &mut d).crc.unwrap();
    assert_ne!(a, b);
}

#[test]
fn stop_turns_the_link_and_framegen0_off_then_the_power() {
    let (mut t, m) = rig();
    let mut d = nodelay();
    let p = display::prepare(&mut t, &m, &Bases::PHYS, &cfg(), &mut d).unwrap();
    let s = display::start(&m, &Bases::PHYS, &p, &mut d).unwrap();
    m.clear_log();
    t.0.borrow_mut().log.clear();
    display::stop(&mut t, &m, &Bases::PHYS, &s, &mut d).unwrap();
    // enable off, then valid; FrameGen0 off
    assert_eq!(m.writes(), vec![("csr", 0x8, 0x2), ("csr", 0x8, 0), ("dpu", 0x2b_008c, 0)]);
    assert_eq!(t.0.borrow().log, power_off_reqs());
}

#[test]
fn a_framegen_that_does_not_stop_keeps_the_power_on() {
    let (mut t, mut m) = rig();
    let mut d = nodelay();
    let p = display::prepare(&mut t, &m, &Bases::PHYS, &cfg(), &mut d).unwrap();
    let s = display::start(&m, &Bases::PHYS, &p, &mut d).unwrap();
    m.faults.fg_stuck = true;
    let e = display::stop(&mut t, &m, &Bases::PHYS, &s, &mut d).unwrap_err();
    assert!(matches!(e, Error::NotStopped(st) if st.fgensts == 1));
    assert!(t.0.borrow().on(13));
}
