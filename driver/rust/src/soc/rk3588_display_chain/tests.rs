// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The display chain over the combined model (`model_rk3588_display`).

use super::*;
use crate::ip::hdptx::Refusal as PhyRefusal;
use crate::ip::vop2::Refusal as VopRefusal;
use crate::model_rk3588_display::{
    chain_bases, gate_addr, rst_addr, sel_addr, Faults, Mapped, Rk3588Display, B_CRU, B_HDMI, B_MMU, B_PHY, B_PHY_GRF,
    B_PMU, B_VO1_GRF, B_VOP, B_VOP_GRF, CRU, HDMI, MMU, PHY, PHY_GRF, PMU, SYS_GRF, TEST_CORE_ID, VO1_GRF,
    VOP as VOP_BASE, VOP_GRF,
};

// The bases as the probe passes them: mapped, never the physical ones
// (`model_rk3588_display::Mapped`). The model's own addresses (CRU, PHY_GRF,
// ...) are the physical ones its log records.
const B: Bases = chain_bases();

// An invented PHY image: test data only, no value is Linux's or the board's
// (the board's is the D0 dump's, `rk3588_display_golden`).
const CMN: [(u16, u32); 3] = [(0x08, 0x11), (0x5e, 0x0f), (0x86, 0x01)];
const SB: [(u16, u32); 1] = [(0x100, 0x33)];
const LTOP: [(u16, u32); 1] = [(0x201, 0x55)];
const LANES: [(u16, u32); 2] = [(0x300, 0x77), (0x400, 0x79)];
const IMG: Image = Image { cmn: &CMN, sb: &SB, lane_top: &LTOP, lanes: &LANES };

const FB: u64 = 0x1000_0000;
const STRIDE: u32 = 1024 * 4;

fn cfg() -> Config<'static> {
    Config {
        mode: MODE_1024X768_60,
        window: (1024, 768),
        plan: hdptx::plan(65_000_000).unwrap(),
        image: IMG,
        bind: BindParams { core_id: TEST_CORE_ID, timer_base_hz: dw::LINUX_TIMER_BASE_HZ },
        sink: Sink::Dvi,
        force_cold: false,
        power_cycle: false,
        stop_after: 0,
    }
}

fn up(m: &Rk3588Display, c: &Config) -> Result<Report, Failed> {
    bring_up(&Mapped(m), &B, m, c, FB, STRIDE)
}

fn no_refusal(m: &Rk3588Display) {
    assert_eq!(m.refused(), Vec::<String>::new());
}

/// The first write to `addr`.
fn first_to(m: &Rk3588Display, addr: usize) -> Option<usize> {
    m.writes().iter().position(|(a, _)| *a == addr)
}

const WIN_ESMART0: u32 = 1 << 4;

#[test]
fn a_cold_bring_up_runs_the_blocks_in_order_and_shows_the_buffer() {
    let m = Rk3588Display::cold();
    let r = up(&m, &cfg()).unwrap();
    no_refusal(&m);
    assert_eq!(r.found, Found::default(), "nothing reachable, nothing running");
    assert!(!r.power.vop.was_on && !r.power.vo1.was_on);
    assert_eq!(r.ident.core_id, TEST_CORE_ID);
    assert_eq!(r.vop.version, 0x4017_6786);
    assert!(r.frame.frame_start && !r.frame.underflow && r.frame.bus_error == [false, false]);
    assert!(r.phy_status.up() && r.hdmi_status.clocks_locked() && r.hdmi_status.dvi());
    assert_eq!(r.read_back.yrgb_mst, FB as u32);
    assert_eq!(r.read_back.vir, STRIDE / 4);
    // the picture: VP0 runs, Esmart0 alone fetches
    assert!(m.port_runs(0));
    assert_eq!(m.fetching(), WIN_ESMART0);

    // the block order: power, GRF, PHY, DCLK, the controller's bind, VOP
    let pmu = m.first_write(B_PMU).unwrap();
    let grf = m.first_write(B_VOP_GRF).unwrap();
    let phy_grf = m.first_write(B_PHY_GRF).unwrap();
    let phy = m.first_write(B_PHY).unwrap();
    let dclk = first_to(&m, sel_addr(0, 112)).unwrap();
    let hdmi = m.first_write(B_HDMI).unwrap();
    let vop = m.first_write(B_VOP).unwrap();
    assert!(pmu < grf && grf < phy_grf && phy_grf < phy, "{pmu} {grf} {phy_grf} {phy}");
    assert!(phy < dclk && dclk < hdmi && hdmi < vop, "{phy} {dclk} {hdmi} {vop}");
    // vo1_grf: the sync word in the GRF stage, before the controller's glue
    assert!(m.first_write(B_VO1_GRF).unwrap() < phy_grf);
    // the IOMMU off before VP0 is started, VP0 started before the window
    let mmu = m.first_write(B_MMU).unwrap();
    let start = first_to(&m, VOP_BASE + 0xc00).unwrap();
    // Linux's order (vop2 fact 9): the interface route (VP0's CLK_CTRL) and
    // the timing before DSP_CTRL starts the port
    assert!(first_to(&m, VOP_BASE + 0xc0c).unwrap() < start && first_to(&m, VOP_BASE + 0xc48).unwrap() < start);
    let win = m.writes().iter().rposition(|(a, v)| *a == VOP_BASE + 0x1810 && v & 1 != 0).unwrap();
    assert!(vop < mmu && mmu < start && start < win);
    // Linux's order (drm_atomic_helper.c:1687-1689, :1940-1943): the
    // controller's enable (HDCP2LOGIC_CONFIG0, then LINK_CONFIG0) after VP0
    // started, before the window's layer and the window
    let enable = first_to(&m, HDMI + 0x8e0).unwrap();
    let layer = first_to(&m, VOP_BASE + 0x604).unwrap();
    assert!(start < enable && enable < first_to(&m, HDMI + 0x968).unwrap(), "{start} {enable}");
    assert!(enable < layer && layer < win, "{enable} {layer} {win}");
    // DCLK_VOP0 on the PHY (CLKSEL_CON(112)[8:7] = 1), its gate and reset open
    assert_eq!((m.peek(sel_addr(0, 112)) >> 7) & 3, 1);
    assert_eq!(m.peek(gate_addr(0, 52)) & (1 << 13), 0);
    assert_eq!(m.peek(rst_addr(0, 52)) & (1 << 13), 0);
    // the GRF words
    assert_eq!(m.peek(VOP_GRF + 0x08) & 0x2, 0x2);
    assert_eq!(m.peek(VO1_GRF) & 0x60, 0, "both syncs negative: field 0");
    assert_eq!(m.peek(VO1_GRF + 0x24) & (1 << 10), 1 << 10);
}

/// Whether the stage after a stop wrote anything.
type Touched = &'static dyn Fn(&Rk3588Display) -> bool;

#[test]
fn every_stop_after_stops_there_and_leaves_the_next_stage_untouched() {
    // (stage, what the next stage would write first)
    let next: [(u32, Touched); 7] = [
        (stage::START, &|m| m.wrote_to(B_PMU) || m.wrote_to(B_CRU)),
        (stage::POWER, &|m| m.wrote_to(B_VOP_GRF) || m.wrote_to(B_VO1_GRF)),
        (stage::GRF, &|m| m.wrote_to(B_PHY_GRF) || m.wrote_to(B_PHY)),
        (stage::PHY, &|m| first_to(m, sel_addr(0, 112)).is_some()),
        (stage::DCLK, &|m| m.wrote_to(B_HDMI)),
        // the controller bound, not enabled: the enable is the VOP stage's
        (stage::CONTROLLER, &|m| {
            m.wrote_to(B_VOP)
                || m.wrote_to(B_MMU)
                || [HDMI + 0x8e0, HDMI + 0x968].iter().any(|a| first_to(m, *a).is_some())
        }),
        // the video check's first write: clearing VP0's events after the window
        (stage::VOP, &|m| {
            let w = m.writes();
            let win = w.iter().rposition(|(a, v)| *a == VOP_BASE + 0x1810 && v & 1 != 0).unwrap();
            w[win..].iter().any(|(a, _)| *a == VOP_BASE + 0xa4)
        }),
    ];
    for (stage, touched) in next {
        let m = Rk3588Display::cold();
        let r = up(&m, &Config { stop_after: stage, ..cfg() });
        assert_eq!(r.err(), Some(Failed { stage, error: Error::Stopped }), "stage {stage}");
        assert!(!touched(&m), "stage {stage}: the next stage was started");
        no_refusal(&m);
    }
    // stop_after VIDEO stops after the check, the picture up
    let m = Rk3588Display::cold();
    let r = up(&m, &Config { stop_after: stage::VIDEO, ..cfg() });
    assert_eq!(r.err(), Some(Failed { stage: stage::VIDEO, error: Error::Stopped }));
    assert!(m.port_runs(0) && m.fetching() == WIN_ESMART0);
    // and the stage before each one did its work: after PHY the PLL is locked
    let m = Rk3588Display::cold();
    let _ = up(&m, &Config { stop_after: stage::PHY, ..cfg() });
    assert_eq!(m.peek(PHY_GRF + 0x80) & 0xe, 0xe);
    // after DCLK, VP0 is clocked but not started
    let m = Rk3588Display::cold();
    let _ = up(&m, &Config { stop_after: stage::DCLK, ..cfg() });
    assert!(!m.port_runs(0));
    assert_eq!((m.peek(sel_addr(0, 112)) >> 7) & 3, 1);
}

/// The "stage N NAME" lines of a run, in order.
fn stage_lines(m: &Rk3588Display) -> Vec<String> {
    m.lines().into_iter().map(|(_, l)| l).filter(|l| l.starts_with("t6 display: stage ")).collect()
}

#[test]
fn each_stage_logs_one_line_and_the_last_line_is_the_stage_that_stopped() {
    let all: Vec<String> =
        (stage::START..=stage::VIDEO).map(|n| format!("t6 display: stage {n} {}", stage::name(n))).collect();
    assert_eq!(all[3], "t6 display: stage 4 PHY");
    let m = Rk3588Display::cold();
    up(&m, &cfg()).unwrap();
    assert_eq!(stage_lines(&m), all);
    for n in stage::START..=stage::VIDEO {
        let m = Rk3588Display::cold();
        let _ = up(&m, &Config { stop_after: n, ..cfg() });
        assert_eq!(stage_lines(&m), all[..n as usize], "stop_after {n}");
    }
    // a failure: the stage that failed is the last one named
    let m = Rk3588Display::with_faults(Faults { phy_no_lock: true, ..Faults::default() });
    assert_eq!(up(&m, &cfg()).err().map(|f| f.stage), Some(stage::PHY));
    assert_eq!(stage_lines(&m).last().map(String::as_str), Some("t6 display: stage 4 PHY"));
    assert_eq!((stage::name(stage::SHUT_DOWN), stage::name(0), stage::name(10)), ("SHUT_DOWN", "?", "?"));
}

#[test]
fn each_injected_failure_names_its_stage() {
    let cases: [(Faults, u32); 8] = [
        (Faults { dead: Some("vop"), ..Faults::default() }, stage::POWER),
        (Faults { dead: Some("vo1"), ..Faults::default() }, stage::POWER),
        (Faults { grf_stuck: true, ..Faults::default() }, stage::GRF),
        (Faults { phy_no_clk: true, ..Faults::default() }, stage::PHY),
        (Faults { phy_no_lock: true, ..Faults::default() }, stage::PHY),
        (Faults { dclk_mux_stuck: true, ..Faults::default() }, stage::DCLK),
        (Faults { underflow: true, ..Faults::default() }, stage::VIDEO),
        (Faults { bus_error: true, ..Faults::default() }, stage::VIDEO),
    ];
    for (f, stage) in cases {
        let m = Rk3588Display::with_faults(f);
        let r = up(&m, &cfg());
        assert_eq!(r.map_err(|x| x.stage), Err(stage), "{f:?}");
    }
    // the PHY's two time-outs are told apart
    let m = Rk3588Display::with_faults(Faults { phy_no_clk: true, ..Faults::default() });
    assert!(matches!(up(&m, &cfg()), Err(Failed { error: Error::Phy(e), .. }) if e.why == PhyRefusal::ClkRdyTimeout));
    let m = Rk3588Display::with_faults(Faults { phy_no_lock: true, ..Faults::default() });
    assert!(matches!(up(&m, &cfg()), Err(Failed { error: Error::Phy(e), .. }) if e.why == PhyRefusal::LockTimeout));
    // another controller at the address
    let mut m = Rk3588Display::cold();
    m.core_id = 0x0bad_0bad;
    let r = up(&m, &cfg());
    assert!(matches!(r, Err(Failed { stage: stage::CONTROLLER, error: Error::Hdmi(e) }) if e.found == 0x0bad_0bad));
    // another VOP
    let m = Rk3588Display::with_faults(Faults { vop_version: Some(0x4015_8888), ..Faults::default() });
    let r = up(&m, &cfg());
    assert!(
        matches!(r, Err(Failed { stage: stage::VOP, error: Error::Vop(e) }) if e.why == VopRefusal::VersionMismatch)
    );
    // a configuration that cannot run: refused at START with nothing written
    let bad_mode = VideoMode { pixel_clock_hz: 400_000_000, ..MODE_1024X768_60 };
    let wrong_plan = Config { plan: hdptx::plan(74_250_000).unwrap(), ..cfg() };
    let bad_image = Config { image: Image { cmn: &[(0x51, 0)], ..IMG }, ..cfg() };
    for c in [Config { mode: bad_mode, ..cfg() }, wrong_plan, bad_image] {
        let m = Rk3588Display::cold();
        assert_eq!(up(&m, &c).map_err(|f| f.stage), Err(stage::START));
        assert!(m.writes().is_empty());
    }
}

#[test]
fn the_controller_is_enabled_only_while_vp0_runs() {
    // The words the enable writes answer only with the video clock (the
    // model refuses them while VP0 does not run, issue #11856): a cold run
    // touches neither before VP0 starts, and the model refuses nothing.
    let m = Rk3588Display::cold();
    up(&m, &cfg()).unwrap();
    no_refusal(&m);
    let start = first_to(&m, VOP_BASE + 0xc00).unwrap();
    for a in [HDMI + 0x8e0, HDMI + 0x968] {
        assert!(first_to(&m, a).unwrap() > start, "{a:#x} written before VP0 started");
    }
    // and the model does refuse them before VP0 runs: the stop after the
    // controller's stage leaves VP0 in STANDBY
    let m = Rk3588Display::cold();
    let _ = up(&m, &Config { stop_after: stage::CONTROLLER, ..cfg() });
    m.write32(HDMI + 0x8e0, 1);
    assert_eq!(m.refused().len(), 1, "{:?}", m.refused());
    assert_eq!(m.peek(HDMI + 0xb0) & 0xa, 0xa, "IPI_CLK_OFF and VIDQPCLK_OFF without VP0");
}

#[test]
fn a_video_clock_reported_off_is_refused_before_the_enable() {
    // VP0 runs, the controller's clock monitor still reports the video clock
    // off: the enable refuses after its bounded wait, names the status word
    // and writes nothing (a bus error on the board otherwise)
    let m = Rk3588Display::with_faults(Faults { cmu_video_off: true, ..Faults::default() });
    let r = up(&m, &cfg());
    assert_eq!(
        r.err(),
        Some(Failed {
            stage: stage::VOP,
            error: Error::Hdmi(dw::Refused { why: dw::Refusal::VideoClockOff, found: 1 << 3 })
        })
    );
    assert!(first_to(&m, HDMI + 0x8e0).is_none() && first_to(&m, HDMI + 0x968).is_none());
    assert!(m.port_runs(0));
    no_refusal(&m);
    assert_eq!(stage_lines(&m).last().map(String::as_str), Some("t6 display: stage 7 VOP"));
}

#[test]
fn the_chain_refuses_a_buffer_the_window_refuses() {
    for (fb, stride, why) in [
        (0xffff_f000, STRIDE, VopRefusal::WindowAbove4G),
        (0x1000_0020, STRIDE, VopRefusal::WindowUnaligned),
        (FB, STRIDE - 4, VopRefusal::StrideTooNarrow),
        (FB, 0x4_0000 * 4, VopRefusal::StrideTooWide),
    ] {
        let m = Rk3588Display::cold();
        let r = bring_up(&Mapped(&m), &B, &m, &cfg(), fb, stride);
        assert_eq!(
            r.err(),
            Some(Failed { stage: stage::START, error: Error::Vop(vop2::Refused { why, read_back: 0 }) }),
            "{fb:#x} {stride}"
        );
        assert!(m.writes().is_empty(), "{why:?}: nothing written");
    }
}

#[test]
fn off_then_on_loses_block_state() {
    let m = Rk3588Display::cold();
    up(&m, &cfg()).unwrap();
    // marks in registers the chain never writes
    let marks = [(VOP_BASE + 0x1a1c, 0x1234), (HDMI + 0x880, 0x5678), (PHY + 4 * 0xa0, 0x9a)];
    for (a, v) in marks {
        m.write32(a, v);
        assert_eq!(m.read32(a), v);
    }
    let found = up(&m, &Config { force_cold: true, power_cycle: true, ..cfg() }).unwrap().found;
    no_refusal(&m);
    assert!(found.vop_reachable && found.running == 1 && found.stopped == 1 && found.phy_powered_off);
    for (a, _) in marks {
        assert_eq!(m.peek(a), 0, "{a:#x} kept its value across the power cycle");
    }
    // and the domains really went off on the way
    let w = m.writes();
    for (pwr, name) in [(1 << 1, "PD_VOP"), (1 << 3, "PD_VO1")] {
        let off = pwr << 16 | pwr;
        assert!(w.iter().any(|(a, v)| *a == PMU + 0x150 && v & off == off), "{name} switched off");
    }
    // the PHY loses its registers also when it was not locked (its
    // power-off then does nothing; the APB reset does it)
    let m = Rk3588Display::cold();
    let _ = up(&m, &Config { stop_after: stage::POWER, ..cfg() });
    m.write32(PHY + 4 * 0xa0, 0x9a);
    let r = up(&m, &Config { power_cycle: true, stop_after: stage::START, ..cfg() });
    assert_eq!(r.err().map(|f| f.error), Some(Error::Stopped));
    assert_eq!(m.peek(PHY + 4 * 0xa0), 0);
    assert_ne!(m.peek(rst_addr(0, 72)) & (1 << 5), 0, "left in its APB reset");
    // without the power cycle the same marks survive a cold re-run
    let m = Rk3588Display::cold();
    up(&m, &cfg()).unwrap();
    m.write32(VOP_BASE + 0x1a1c, 0x1234);
    up(&m, &Config { force_cold: true, ..cfg() }).unwrap();
    assert_eq!(m.peek(VOP_BASE + 0x1a1c), 0x1234);
}

#[test]
fn a_running_port_is_left_alone_without_force_cold() {
    let m = Rk3588Display::cold();
    up(&m, &cfg()).unwrap();
    m.clear_log();
    let r = up(&m, &cfg());
    assert_eq!(r.err(), Some(Failed { stage: stage::START, error: Error::FoundRunning(1) }));
    assert!(m.writes().is_empty());
    assert!(m.port_runs(0));
}

#[test]
fn force_cold_stops_linux_on_both_ports_before_the_iommu_goes_off() {
    // the state a crash kexec leaves: Linux on Cluster0 of VP0 and Cluster1
    // of VP1, through the IOMMU (design note section 2 (a))
    let m = Rk3588Display::cold();
    up(&m, &cfg()).unwrap();
    m.leave_linux_running(true);
    assert!(m.port_runs(1));
    m.clear_log();
    let r = up(&m, &Config { force_cold: true, ..cfg() }).unwrap();
    no_refusal(&m);
    assert_eq!((r.found.running, r.found.stopped, r.found.unclocked), (0b11, 0b11, 0));
    assert!(r.iommu.was_paging());
    // no window read an IOVA as a physical address
    assert_eq!(r.frame.bus_error, [false, false]);
    assert_eq!(m.fetching(), WIN_ESMART0);
    assert!(!m.port_runs(1));
    // both ports went to STANDBY before the first vop_mmu write
    let w = m.writes();
    let standby = |p: usize| w.iter().position(|(a, v)| *a == VOP_BASE + 0xc00 + 0x100 * p && v & 1 << 31 != 0);
    let mmu = m.first_write(B_MMU).unwrap();
    assert!(standby(0).unwrap() < mmu && standby(1).unwrap() < mmu);
}

#[test]
fn the_iommu_turned_off_under_a_fetching_window_is_seen_by_the_model() {
    // the model's side of the test above: Linux's window still fetching
    // when paging goes off raises the bus error the video check reads
    let m = Rk3588Display::cold();
    up(&m, &cfg()).unwrap();
    m.leave_linux_running(false);
    let mut delay = |us| m.busy_wait_us(us);
    rk_iommu::paging_off(&m, &MMU, &mut delay).unwrap();
    m.busy_wait_us(40_000);
    assert_eq!(m.peek(VOP_BASE + 0x88) & 2, 2);
}

#[test]
fn a_port_with_standby_clear_but_no_pixel_clock_does_not_stop_the_chain() {
    // review R3-S1 of D1: VP2's DSP_CTRL reads STANDBY clear, its clock is off
    let m = Rk3588Display::cold();
    up(&m, &cfg()).unwrap();
    m.write32(VOP_BASE + 0xe00, 0);
    m.clear_log();
    let r = up(&m, &Config { force_cold: true, ..cfg() }).unwrap();
    assert_eq!((r.found.running, r.found.stopped, r.found.unclocked), (0b101, 0b001, 0b100));
    no_refusal(&m);
}

#[test]
fn shut_down_powers_the_chain_off() {
    let m = Rk3588Display::cold();
    up(&m, &cfg()).unwrap();
    shut_down(&Mapped(&m), &B, &m).unwrap();
    no_refusal(&m);
    assert!(!m.usable("vop") && !m.usable("vo1"));
    assert_eq!(m.peek(PHY_GRF + 0x80), 0, "the PHY is off");
    // and it comes up again from there
    up(&m, &cfg()).unwrap();
    no_refusal(&m);
}

/// The second mode with the first one's window: what `t6::config` gives
/// for `t6::Mode::M1920x1080`, with the tests' image and identity.
fn cfg_1920x1080() -> Config<'static> {
    Config { mode: MODE_1920X1080_60, plan: hdptx::plan(148_500_000).unwrap(), ..cfg() }
}

#[test]
fn a_window_smaller_than_the_mode_is_shown_at_the_top_left() {
    let m = Rk3588Display::cold();
    up(&m, &cfg_1920x1080()).unwrap();
    no_refusal(&m);
    assert!(m.port_runs(0));
    assert_eq!(m.peek(VOP_BASE + 0xc48), 0x0898_002c, "VP0 DSP_HTOTAL_HS_END");
    assert_eq!(m.peek(VOP_BASE + 0xc50), 0x0465_0005, "VP0 DSP_VTOTAL_VS_END");
    assert_eq!(m.peek(VOP_BASE + 0x1814), FB as u32, "Esmart0's YRGB_MST");
    assert_eq!(m.peek(VOP_BASE + 0x1820), 767 << 16 | 1023, "Esmart0's ACT_INFO: the window, not the mode");
    assert_eq!(m.peek(VOP_BASE + 0x1824), 767 << 16 | 1023, "Esmart0's DSP_INFO: unscaled");
    assert_eq!(m.peek(VOP_BASE + 0xc2c), 0, "VP0 DSP_BG: black around the window");
}

#[test]
fn the_chain_refuses_a_window_larger_than_the_mode() {
    let m = Rk3588Display::cold();
    let c = Config { window: (1920, 1080), ..cfg() };
    let r = bring_up(&Mapped(&m), &B, &m, &c, FB, 1920 * 4);
    assert_eq!(
        r.err(),
        Some(Failed {
            stage: stage::START,
            error: Error::Vop(vop2::Refused { why: VopRefusal::WindowOutside, read_back: 0 })
        })
    );
    assert!(m.writes().is_empty(), "nothing written");
}

#[test]
fn the_second_mode_is_the_measured_one() {
    let m = MODE_1920X1080_60;
    assert_eq!((m.hactive, m.hactive + m.hfront, m.hactive + m.hfront + m.hsync, m.htotal()), (1920, 2008, 2052, 2200));
    assert_eq!((m.vactive, m.vactive + m.vfront, m.vactive + m.vfront + m.vsync, m.vtotal()), (1080, 1084, 1089, 1125));
    assert!(m.hsync_high && m.vsync_high);
    assert_eq!(frame_us(&m), 16_667, "2200 x 1125 / 148.5 MHz, rounded up");
}

#[test]
fn the_mode_is_the_measured_one() {
    let m = MODE_1024X768_60;
    assert_eq!((m.hactive, m.hactive + m.hfront, m.hactive + m.hfront + m.hsync, m.htotal()), (1024, 1048, 1184, 1344));
    assert_eq!((m.vactive, m.vactive + m.vfront, m.vactive + m.vfront + m.vsync, m.vtotal()), (768, 771, 777, 806));
    assert!(!m.hsync_high && !m.vsync_high);
    assert_eq!(frame_us(&m), 16_666, "1344 x 806 / 65 MHz, rounded up");
}

#[test]
fn the_real_bases_are_the_design_notes() {
    assert_eq!(RK3588.cru, 0xfd7c_0000);
    assert_eq!(RK3588.pmu, 0xfd8d_8000);
    assert_eq!((RK3588.sys_grf, RK3588.vop_grf, RK3588.vo1_grf), (0xfd58_c000, 0xfd5a_4000, 0xfd5a_8000));
    assert_eq!(RK3588.phy, hdptx::Bases { phy: 0xfed6_0000, grf: 0xfd5e_0000 });
    assert_eq!((RK3588.hdmi, RK3588.vop), (0xfde8_0000, 0xfdd9_0000));
    assert_eq!(RK3588.vop_mmu, [0xfdd9_7e00, 0xfdd9_7f00]);
    // the model sits at the same physical addresses (the golden test needs
    // it), and the tests' bases are none of them: the probe's are mappings
    let model = Bases {
        cru: CRU,
        pmu: PMU,
        sys_grf: SYS_GRF,
        vop_grf: VOP_GRF,
        vo1_grf: VO1_GRF,
        phy: hdptx::Bases { phy: PHY, grf: PHY_GRF },
        hdmi: HDMI,
        vop: VOP_BASE,
        vop_mmu: MMU,
    };
    assert_eq!(model, RK3588);
    let phys = [CRU, PMU, SYS_GRF, VOP_GRF, VO1_GRF, PHY, PHY_GRF, HDMI, VOP_BASE, MMU[0], MMU[1]];
    let ours = [B.cru, B.pmu, B.sys_grf, B.vop_grf, B.vo1_grf, B.phy.phy, B.phy.grf, B.hdmi, B.vop];
    for b in ours.iter().chain(&B.vop_mmu) {
        assert!(!phys.contains(b), "{b:#x} is a physical base");
    }
}

#[test]
fn a_powered_vop_that_cannot_be_read_is_refused_without_power_cycle() {
    // review B1: Linux's picture runs, one VOP bus gate (PCLK_VOP_ROOT 52/3)
    // reads closed, so DSP_CTRL cannot be read
    let m = Rk3588Display::cold();
    up(&m, &cfg()).unwrap();
    m.leave_linux_running(false);
    m.poke(gate_addr(0, 52), m.peek(gate_addr(0, 52)) | 1 << 3);
    assert!(m.port_runs(0));
    m.clear_log();
    let r = up(&m, &Config { force_cold: true, ..cfg() });
    assert!(
        matches!(r, Err(Failed { stage: stage::START, error: Error::VopUnreadable(_) })),
        "a VOP that runs but cannot be read is not 'nothing runs': {r:?}"
    );
    assert!(m.writes().is_empty());
    no_refusal(&m);
    // with the power cycle the gate is opened, the port stopped, the
    // domain switched off: then the bring-up goes on
    let r = up(&m, &Config { force_cold: true, power_cycle: true, ..cfg() }).unwrap();
    no_refusal(&m);
    assert_eq!((r.found.vop_unreadable.is_some(), r.found.running, r.found.stopped), (true, 1, 1));
    assert_eq!(r.frame.bus_error, [false, false]);
    // and without force_cold the port it then finds running is refused
    let m = Rk3588Display::cold();
    up(&m, &cfg()).unwrap();
    m.leave_linux_running(false);
    m.poke(gate_addr(0, 52), m.peek(gate_addr(0, 52)) | 1 << 3);
    let r = up(&m, &Config { power_cycle: true, ..cfg() });
    assert_eq!(r.err().map(|f| f.error), Some(Error::FoundRunning(1)));
}

#[test]
fn an_unclocked_vp0_is_refused_without_power_cycle() {
    // review B2: VP0 has STANDBY clear and Linux's window on it, its pixel
    // clock gated (52/13); the chain would give the clock back while the
    // window's disable is pending and paging still on
    let m = Rk3588Display::cold();
    up(&m, &cfg()).unwrap();
    m.leave_linux_running(false);
    m.poke(gate_addr(0, 52), m.peek(gate_addr(0, 52)) | 1 << 13);
    assert!(!m.port_runs(0));
    m.clear_log();
    let r = up(&m, &Config { force_cold: true, ..cfg() });
    assert_eq!(r.err().map(|f| (f.stage, f.error)), Some((stage::START, Error::Unclocked(1))));
    no_refusal(&m);
    // nothing of VP0 was written but the event clear of the frame wait, so
    // a second try finds the same state and is refused the same way
    assert!(m.writes().iter().all(|(a, _)| *a == VOP_BASE + 0xa4), "{:x?}", m.writes());
    let r = up(&m, &Config { force_cold: true, ..cfg() });
    assert_eq!(r.err().map(|f| f.error), Some(Error::Unclocked(1)));
    // the power cycle drops the pending state with PD_VOP
    let r = up(&m, &Config { force_cold: true, power_cycle: true, ..cfg() }).unwrap();
    no_refusal(&m);
    assert_eq!(r.frame.bus_error, [false, false]);
}

#[test]
fn a_locked_phy_whose_apb_clock_is_gated_is_not_touched_by_the_power_cycle() {
    // the PHY's PLL runs (its GRF says locked) while PCLK_HDPTX0 (72/5) reads
    // gated: its register file cannot be reached (D4's HDPTX0 block), so the
    // power-off, which writes it, is not run; its resets still are
    let m = Rk3588Display::cold();
    up(&m, &cfg()).unwrap();
    m.poke(gate_addr(0, 72), m.peek(gate_addr(0, 72)) | 1 << 5);
    assert_eq!(m.peek(PHY_GRF + 0x80) & 0x8, 0x8);
    m.clear_log();
    let r = up(&m, &Config { force_cold: true, power_cycle: true, ..cfg() }).unwrap();
    no_refusal(&m);
    assert!(!r.found.phy_powered_off);
    assert_eq!(r.found.phy_status, 0, "STATUS is not read while the PHY is not reachable");
}

#[test]
fn each_video_check_fails_on_its_own() {
    use video_check::*;
    for (f, bit) in [
        (Faults { underflow: true, ..Faults::default() }, UNDERFLOW),
        (Faults { bus_error: true, ..Faults::default() }, BUS_ERROR),
        (Faults { mst_stuck: true, ..Faults::default() }, ADDRESS),
        (Faults { phy_rdy_lost: true, ..Faults::default() }, PHY),
        (Faults { cmu_unlocked: true, ..Faults::default() }, CMU),
    ] {
        let m = Rk3588Display::with_faults(f);
        assert_eq!(up(&m, &cfg()).err(), Some(Failed { stage: stage::VIDEO, error: Error::Video(bit) }), "{f:?}");
    }
}

#[test]
fn a_cfg_done_bit_that_stays_set_is_logged_not_failed() {
    // design note U8: whether the port's REG_CFG_DONE bit clears once the
    // update is taken is not known; the picture does not depend on it
    let m = Rk3588Display::with_faults(Faults { cfg_done_sticky: true, ..Faults::default() });
    let r = up(&m, &cfg()).unwrap();
    assert_eq!(r.read_back.cfg_done & 1, 1);
    assert!(m.lines().iter().any(|(_, l)| l.contains("(U8)")));
}

#[test]
fn a_running_port_whose_standby_does_not_take_stops_the_chain() {
    // the port keeps running frames after STANDBY: nothing of it may be
    // turned off under it
    let m = Rk3588Display::with_faults(Faults { standby_ignored: true, ..Faults::default() });
    up(&m, &cfg()).unwrap();
    m.leave_linux_running(false);
    m.clear_log();
    let r = up(&m, &Config { force_cold: true, ..cfg() });
    assert!(
        matches!(r, Err(Failed { stage: stage::START, error: Error::Standby(e) }) if e.why == VopRefusal::StandbyTimeout),
        "{r:?}"
    );
    assert!(!m.wrote_to(B_PMU) && !m.wrote_to(B_MMU));
}

#[test]
fn power_cycle_does_not_take_a_running_port_without_force_cold() {
    let m = Rk3588Display::cold();
    up(&m, &cfg()).unwrap();
    m.clear_log();
    let r = up(&m, &Config { power_cycle: true, ..cfg() });
    assert_eq!(r.err(), Some(Failed { stage: stage::START, error: Error::FoundRunning(1) }));
    assert!(m.writes().is_empty());
}

#[test]
fn a_window_left_enabled_on_a_stopped_port_is_switched_off_before_the_port_starts() {
    // a system before us stopped VP0 but left its Cluster0 window enabled,
    // at an IOVA, with paging on: nothing runs, so START writes nothing;
    // stage 7 turns paging off and must switch the window off before VP0
    // starts, or it fetches its IOVA as a physical address
    let m = Rk3588Display::cold();
    up(&m, &cfg()).unwrap();
    m.leave_linux_running(false);
    m.poke(VOP_BASE + 0xc00, 1 << 31 | 0xf);
    assert!(!m.port_runs(0));
    m.clear_log();
    let r = up(&m, &cfg()).unwrap();
    assert_eq!(r.found.running, 0);
    assert_eq!(r.frame.bus_error, [false, false]);
    assert_eq!(m.fetching(), WIN_ESMART0);
    no_refusal(&m);
}

#[test]
fn the_chain_writes_only_its_own_fields_of_shared_words() {
    // CRU and GRF words take their write mask in 31:16: the chain's words
    // leave the fields of other blocks as found
    let m = Rk3588Display::cold();
    let others = [
        (sel_addr(0, 112), 0x0000_007f), // CLKSEL_CON(112) 6:0, not DCLK_VOP0's mux
        (VOP_GRF + 0x08, 1 << 4),        // GRF_VOP_CON2: HDMI1's select
        (VO1_GRF, 0x3 << 7),             // GRF_VO1_CON0: HDMI1's sync field
        (VO1_GRF + 0x24, 1 << 12),       // GRF_VO1_CON9: HDMI1's GRANT_SEL
        (SYS_GRF + 0x308, 1 << 15),      // SOC_CON2: HDMI1's HPD mask
    ];
    for (a, v) in others {
        m.poke(a, v);
    }
    up(&m, &cfg()).unwrap();
    for (a, v) in others {
        assert_eq!(m.peek(a) & v, v, "{a:#x}: another block's field changed");
    }
}

#[test]
fn a_shut_down_failure_names_the_shut_down() {
    let m = Rk3588Display::with_faults(Faults { standby_ignored: true, ..Faults::default() });
    up(&m, &cfg()).unwrap();
    let r = shut_down(&Mapped(&m), &B, &m);
    assert!(matches!(r, Err(Failed { stage: stage::SHUT_DOWN, error: Error::Standby(_) })), "{r:?}");
    assert!(m.usable("vop"), "the domain stays on under the running port");
}

/// PD_VOP's idle request set by the system before us (PMU 0x10c bits 13,
/// 14, with the write mask), the status words settled.
fn request_vop_idle(m: &Rk3588Display) {
    m.write32(PMU + 0x10c, (3 << 13) << 16 | 3 << 13);
    for _ in 0..4 {
        let _ = m.read32(PMU + 0x290);
    }
    assert!(!m.usable("vop"));
}

#[test]
fn a_powered_vop_with_its_idle_request_set_is_not_taken_as_off() {
    // review R5: PD_VOP powered, its bus idle-requested, VP0 still timed
    // with Linux's window: "not usable" is not "off"
    let m = Rk3588Display::cold();
    up(&m, &cfg()).unwrap();
    m.leave_linux_running(false);
    request_vop_idle(&m);
    m.clear_log();
    let r = up(&m, &cfg());
    assert!(matches!(r, Err(Failed { stage: stage::START, error: Error::VopUnreadable(_) })), "{r:?}");
    assert!(m.writes().is_empty());
    no_refusal(&m);
    // with the power cycle the domain is taken out of idle, VP0 seen and
    // stopped before anything turns its clock off
    let r = up(&m, &Config { force_cold: true, power_cycle: true, ..cfg() }).unwrap();
    no_refusal(&m);
    assert_eq!((r.found.running, r.found.stopped), (1, 1));
}

#[test]
fn a_vp0_stopping_without_its_clock_is_refused_without_power_cycle() {
    // review R6: the system before us wrote STANDBY to VP0, then gated
    // DCLK_VOP0 before that frame ended; DSP_CTRL reads STANDBY
    let m = Rk3588Display::cold();
    up(&m, &cfg()).unwrap();
    m.leave_linux_running(false);
    m.write32(VOP_BASE + 0xc00, 1 << 31);
    m.poke(gate_addr(0, 52), m.peek(gate_addr(0, 52)) | 1 << 13);
    m.busy_wait_us(1);
    m.clear_log();
    let r = up(&m, &Config { force_cold: true, ..cfg() });
    assert_eq!(r.err().map(|f| (f.stage, f.error)), Some((stage::START, Error::Unclocked(1))));
    assert!(m.writes().is_empty());
    no_refusal(&m);
    let r = up(&m, &Config { force_cold: true, power_cycle: true, ..cfg() }).unwrap();
    no_refusal(&m);
    assert_eq!(r.frame.bus_error, [false, false]);
}

#[test]
fn a_port_on_the_phys_pixel_clock_counts_as_clocked_here() {
    // DCLK_VOP1 can take clk_hdmiphy_pixel0 too (clk-rk3588.c:504, 2104):
    // VP1 on the PHY's clock, which is off, runs Linux's Cluster1 window
    // mid-frame; VP0 runs from dclk_vop0_src. Bringing the PHY up would
    // give VP1 its clock back with the window and paging as they are.
    let m = Rk3588Display::cold();
    up(&m, &cfg()).unwrap();
    m.leave_linux_running(true);
    let sel = sel_addr(0, 112);
    m.poke(sel, (m.peek(sel) & !(3 << 7 | 3 << 9)) | 1 << 9);
    m.poke(gate_addr(0, 52), m.peek(gate_addr(0, 52)) & !(1 << 10));
    m.poke(rst_addr(0x3_0000, 3), m.peek(rst_addr(0x3_0000, 3)) | 1 << 12);
    m.busy_wait_us(1);
    assert!(m.port_runs(0) && !m.port_runs(1));
    m.clear_log();
    let r = up(&m, &Config { force_cold: true, ..cfg() });
    assert_eq!(r.err().map(|f| (f.stage, f.error)), Some((stage::START, Error::Unclocked(1 << 1))));
    assert!(m.writes().is_empty());
    no_refusal(&m);
}

#[test]
fn shut_down_opens_an_unreadable_vop_and_stops_it_first() {
    // review N8: Linux's picture runs, PCLK_VOP_ROOT (52/3) reads gated
    let m = Rk3588Display::cold();
    up(&m, &cfg()).unwrap();
    m.leave_linux_running(false);
    m.poke(gate_addr(0, 52), m.peek(gate_addr(0, 52)) | 1 << 3);
    shut_down(&Mapped(&m), &B, &m).unwrap();
    no_refusal(&m);
    assert!(!m.usable("vop") && !m.port_runs(0));
}

#[test]
fn a_vop_reset_found_asserted_is_released_by_the_power_cycle() {
    // review N9: SRST_A_VOP (52/9) asserted by the system before us
    let m = Rk3588Display::cold();
    up(&m, &cfg()).unwrap();
    m.write32(rst_addr(0, 52), (1 << 9) << 16 | 1 << 9);
    m.clear_log();
    let r = up(&m, &Config { force_cold: true, power_cycle: true, ..cfg() }).unwrap();
    no_refusal(&m);
    assert_eq!(r.found.vop_unreadable, Some(NotReady::InReset(reset::A_VOP)));
    assert!(r.found.vop_reachable, "read again once the reset was released");
}

/// One input set of the pixel clock rule (`pixel_clocks`) for a port, and
/// whether the port must count as clocked.
struct ClockRow {
    port: usize,
    gate_open: bool,
    /// SRST_D_VOP0 released (VP0 only).
    reset_released: bool,
    mux: u32,
    /// The port's own source gate (52/10 + port) open.
    src_open: bool,
    /// The next port's source gate open: VP1's row must not read it.
    other_src_open: bool,
    phy_ready: bool,
    clocked: bool,
}

#[test]
fn the_pixel_clock_rule_reads_each_of_its_inputs() {
    let row = |port, gate_open, mux, src_open, phy_ready, clocked| ClockRow {
        port,
        gate_open,
        reset_released: true,
        mux,
        src_open,
        other_src_open: false,
        phy_ready,
        clocked,
    };
    let rows = [
        // VP0: gate 52/13, reset 52/13, mux 112[8:7], source gate 52/10
        row(0, true, 0, true, false, true),
        row(0, true, 0, false, true, false),
        row(0, false, 0, true, true, false),
        ClockRow { reset_released: false, ..row(0, true, 0, true, true, false) },
        row(0, true, 1, false, true, true),
        row(0, true, 1, true, false, false),
        row(0, true, 2, true, true, false),
        // VP1: gate 53/0, mux 112[10:9], source gate 52/11
        row(1, true, 0, true, false, true),
        ClockRow { other_src_open: true, ..row(1, true, 0, false, true, false) },
        row(1, false, 0, true, true, false),
        row(1, true, 1, false, true, true),
        row(1, true, 1, true, false, false),
        row(1, true, 2, true, true, false),
    ];
    for r in &rows {
        // cold: every gate closed, every reset asserted, every mux 0
        let m = Rk3588Display::cold();
        let set = |addr: usize, bit: u32, on: bool| {
            let v = m.peek(addr);
            m.poke(addr, if on { v | 1 << bit } else { v & !(1 << bit) });
        };
        let (gate_con, gate_bit) = [(52, 13), (53, 0)][r.port];
        set(gate_addr(0, gate_con), gate_bit, !r.gate_open);
        set(rst_addr(0, 52), 13, !r.reset_released);
        set(gate_addr(0, 52), 10 + r.port as u32, !r.src_open);
        set(gate_addr(0, 52), 11 + r.port as u32, !r.other_src_open);
        let shift = 7 + 2 * r.port;
        m.poke(sel_addr(0, 112), r.mux << shift);
        let phy_status = if r.phy_ready { hdptx::grf::PHY_CLK_RDY } else { 0 };
        let (here, now) = pixel_clocks(&Mapped(&m), &B, phy_status);
        let bit = 1 << r.port;
        assert_eq!(
            now & bit != 0,
            r.clocked,
            "VP{} gate {} reset {} mux {} src {} other {} phy {}",
            r.port,
            r.gate_open,
            r.reset_released,
            r.mux,
            r.src_open,
            r.other_src_open,
            r.phy_ready
        );
        // VP0 is always the chain's; another port only when its mux is on the PHY0
        assert_eq!(here & bit != 0, r.port == 0 || r.mux == 1, "VP{} mux {}", r.port, r.mux);
    }
}

#[test]
fn a_port_on_the_phys_clock_that_shows_no_frame_is_refused_under_force_cold() {
    // VP1 on the PHY0's pixel clock, every clock input reading as running,
    // but its timing shows no frame: it may be in the middle of one
    let mut m = Rk3588Display::cold();
    up(&m, &cfg()).unwrap();
    m.leave_linux_running(true);
    let sel = sel_addr(0, 112);
    m.poke(sel, (m.peek(sel) & !(3 << 9)) | 1 << 9);
    m.faults.no_frames = 1 << 1;
    m.clear_log();
    let r = up(&m, &Config { force_cold: true, ..cfg() });
    assert_eq!(r.err().map(|f| (f.stage, f.error)), Some((stage::START, Error::Unclocked(1 << 1))));
    // only the event clears of the frame waits were written
    assert!(m.writes().iter().all(|(a, _)| [VOP_BASE + 0xa4, VOP_BASE + 0xb4].contains(a)), "{:x?}", m.writes());
    no_refusal(&m);
}

#[test]
fn a_vop_that_stays_in_reset_after_its_release_is_refused() {
    // SRST_A_VOP found asserted and stuck: after the power cycle's release
    // VOP2 still cannot be read, which is not "nothing runs"
    let mut m = Rk3588Display::cold();
    up(&m, &cfg()).unwrap();
    m.write32(rst_addr(0, 52), (1 << 9) << 16 | 1 << 9);
    m.faults.vop_reset_stuck = true;
    m.clear_log();
    let r = up(&m, &Config { force_cold: true, power_cycle: true, ..cfg() });
    assert_eq!(
        r.err().map(|f| (f.stage, f.error)),
        Some((stage::START, Error::VopUnreadable(NotReady::InReset(reset::A_VOP))))
    );
}

/// Start states a system before us can leave, each built from Linux's
/// picture left running (VP0, Cluster0 at an IOVA, paging on).
#[derive(Clone, Copy, Debug)]
enum Start {
    /// PD_VOP off.
    Cold,
    /// VP0 runs, clocked (a crash kexec).
    Running,
    /// PD_VOP powered, its idle request set, VP0 still timed.
    IdleRequested,
    /// VP0's STANDBY taken, then DCLK_VOP0 gated (a clean shutdown).
    StoppedGated,
    /// VP0 runs, SRST_D_VOP0 asserted under it.
    ResetUnderRunning,
    /// VP0 runs on the PHY0's clock, the PHY's PLL turned off.
    PhyOff,
    /// VP0's STANDBY written, DCLK_VOP0 gated before the frame ended.
    StoppingGated,
    /// The last one, with PD_VOP's idle request set too.
    StoppingGatedIdle,
    /// VP0 stopped and on dclk_vop0_src, its gates open.
    StoppedOnSrc,
}

fn start_state(s: Start) -> Rk3588Display {
    let m = Rk3588Display::cold();
    if let Start::Cold = s {
        return m;
    }
    up(&m, &cfg()).unwrap();
    m.leave_linux_running(false);
    let gate52 = |bit: u32, closed: bool| {
        let v = m.peek(gate_addr(0, 52));
        m.poke(gate_addr(0, 52), if closed { v | 1 << bit } else { v & !(1 << bit) });
    };
    let standby = |wait: bool| {
        m.write32(VOP_BASE + 0xc00, 1 << 31);
        if wait {
            m.busy_wait_us(40_000);
        }
    };
    match s {
        Start::Cold | Start::Running => {}
        Start::IdleRequested => request_vop_idle(&m),
        Start::StoppedGated => {
            standby(true);
            gate52(13, true);
        }
        Start::ResetUnderRunning => m.poke(rst_addr(0, 52), m.peek(rst_addr(0, 52)) | 1 << 13),
        Start::PhyOff => m.poke(PHY_GRF, 0xffff_0000),
        Start::StoppingGated => {
            standby(false);
            gate52(13, true);
        }
        Start::StoppingGatedIdle => {
            standby(false);
            gate52(13, true);
            request_vop_idle(&m);
        }
        Start::StoppedOnSrc => {
            standby(true);
            gate52(10, false);
            m.poke(sel_addr(0, 112), m.peek(sel_addr(0, 112)) & !(3 << 7));
        }
    }
    m.busy_wait_us(1);
    m.clear_log();
    m
}

#[test]
fn each_start_state_is_refused_or_brought_up_as_its_flags_allow() {
    // Expected START outcome per start state for (no flag, force_cold,
    // power_cycle, both): None = Ok. Without power_cycle a refusal writes
    // nothing; with it every run ends Ok or FoundRunning. No run may see a
    // model refusal or a bus error.
    use Error::*;
    let unread = VopUnreadable(NotReady::DomainOff("vop"));
    let cases: &[(Start, [Option<Error>; 4])] = &[
        (Start::Cold, [None, None, None, None]),
        (Start::Running, [Some(FoundRunning(1)), None, Some(FoundRunning(1)), None]),
        (Start::IdleRequested, [Some(unread), Some(unread), Some(FoundRunning(1)), None]),
        (Start::StoppedGated, [Some(Unclocked(1)), Some(Unclocked(1)), None, None]),
        (Start::ResetUnderRunning, [Some(Unclocked(1)), Some(Unclocked(1)), Some(FoundRunning(1)), None]),
        (Start::PhyOff, [Some(Unclocked(1)), Some(Unclocked(1)), Some(FoundRunning(1)), None]),
        (Start::StoppingGated, [Some(Unclocked(1)), Some(Unclocked(1)), None, None]),
        (Start::StoppingGatedIdle, [Some(unread), Some(unread), None, None]),
        (Start::StoppedOnSrc, [None, None, None, None]),
    ];
    for &(s, want) in cases {
        for (i, (force_cold, power_cycle)) in
            [(false, false), (true, false), (false, true), (true, true)].into_iter().enumerate()
        {
            let m = start_state(s);
            let r = up(&m, &Config { force_cold, power_cycle, ..cfg() });
            let what = format!("{s:?} force_cold {force_cold} power_cycle {power_cycle}");
            match (&r, want[i]) {
                (Ok(rep), None) => assert_eq!(rep.frame.bus_error, [false, false], "{what}"),
                (Err(f), Some(e)) => {
                    assert_eq!((f.stage, f.error), (stage::START, e), "{what}");
                    if !power_cycle {
                        assert!(m.writes().is_empty(), "{what}: written before the refusal");
                    }
                }
                _ => panic!("{what}: {r:?}, expected {:?}", want[i]),
            }
            assert_eq!(m.refused(), Vec::<String>::new(), "{what}");
        }
    }
}
