// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The display chain over the combined model (`model_rk3588_display`).

use super::*;
use crate::ip::hdptx::Refusal as PhyRefusal;
use crate::ip::vop2::Refusal as VopRefusal;
use crate::model_rk3588_display::{
    gate_addr, rst_addr, sel_addr, Faults, Rk3588Display, B_CRU, B_HDMI, B_MMU, B_PHY, B_PHY_GRF, B_PMU, B_VO1_GRF,
    B_VOP, B_VOP_GRF, CRU, HDMI, MMU, PHY, PHY_GRF, PMU, SYS_GRF, TEST_CORE_ID, VO1_GRF, VOP as VOP_BASE, VOP_GRF,
};

// The model's addresses, not the module's: the bases are restated.
const B: Bases = Bases {
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
    bring_up(m, &B, m, c, FB, STRIDE)
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

    // the block order: power, GRF, PHY, DCLK, controller, VOP
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
        (stage::CONTROLLER, &|m| m.wrote_to(B_VOP) || m.wrote_to(B_MMU)),
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
fn the_chain_refuses_a_buffer_the_window_refuses() {
    for (fb, stride, why) in [
        (0xffff_f000, STRIDE, VopRefusal::WindowAbove4G),
        (0x1000_0020, STRIDE, VopRefusal::WindowUnaligned),
        (FB, STRIDE - 4, VopRefusal::StrideTooNarrow),
        (FB, 0x4_0000 * 4, VopRefusal::StrideTooWide),
    ] {
        let m = Rk3588Display::cold();
        let r = bring_up(&m, &B, &m, &cfg(), fb, stride);
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
    shut_down(&m, &B, &m).unwrap();
    no_refusal(&m);
    assert!(!m.usable("vop") && !m.usable("vo1"));
    assert_eq!(m.peek(PHY_GRF + 0x80), 0, "the PHY is off");
    // and it comes up again from there
    up(&m, &cfg()).unwrap();
    no_refusal(&m);
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
    // the model sits at the same addresses (the golden test needs it)
    assert_eq!(B, RK3588);
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
fn the_controller_base_is_checked_before_any_write() {
    // review R4: TX1's base with port 0 was refused only at stage 6
    let m = Rk3588Display::cold();
    let b = Bases { hdmi: 0xfdea_0000, ..B };
    let r = bring_up(&m, &b, &m, &cfg(), FB, STRIDE);
    let refused = dw::Refused { why: dw::Refusal::PortMismatch, found: 0xfdea_0000 };
    assert_eq!(r.err(), Some(Failed { stage: stage::START, error: Error::Hdmi(refused) }));
    assert!(m.writes().is_empty());
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
