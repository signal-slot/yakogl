// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Step D8's run over the combined model (`model_rk3588_display`): from a
//! cold chip, from what a normal kexec leaves after Linux showed another
//! mode, and from Linux's picture still running.

use super::*;
use crate::env::{Level, LINE_MAX};
use crate::ip::hdptx::Refusal as PhyRefusal;
use crate::mmio::Mmio;
use crate::model_rk3588_display::{
    gate_addr, sel_addr, virt, Faults, Mapped, Rk3588Display, CRU, HDMI, MMU, PHY, PHY_GRF, PMU, SYS_GRF, VO1_GRF, VOP,
    VOP_GRF,
};

// The physical addresses (the real ones, restated by the model).
const PHYS: ColdBases = ColdBases {
    display: Bases { vop: VOP, vop_mmu: MMU, pmu: PMU, cru: CRU, sys_grf: SYS_GRF, phy_grf: PHY_GRF, hdmi: HDMI },
    vop_grf: VOP_GRF,
    vo1_grf: VO1_GRF,
    phy: PHY,
};

// What the probe passes: every base mapped, none at its physical address
// (model_rk3588_display::Mapped translates them back).
const B: ColdBases = ColdBases {
    display: Bases {
        vop: virt(VOP),
        vop_mmu: [virt(MMU[0]), virt(MMU[1])],
        pmu: virt(PMU),
        cru: virt(CRU),
        sys_grf: virt(SYS_GRF),
        phy_grf: virt(PHY_GRF),
        hdmi: virt(HDMI),
    },
    vop_grf: virt(VOP_GRF),
    vo1_grf: virt(VO1_GRF),
    phy: virt(PHY),
};

const FB: u64 = 0x1001_2000;
const WIN_ESMART0: u32 = 1 << 4;

/// The model with the board's controller identity: the T6's configuration
/// binds only to the `CORE_ID` the D0 dump read.
fn chip(faults: Faults) -> Rk3588Display {
    let mut m = Rk3588Display::with_faults(faults);
    m.core_id = chain::t6::BIND.core_id;
    m.hpd = true;
    m
}

fn lines(m: &Rk3588Display) -> Vec<String> {
    m.lines().into_iter().map(|(_, l)| l).collect()
}

fn has(m: &Rk3588Display, start: &str) -> bool {
    lines(m).iter().any(|l| l.starts_with(start))
}

/// The state a normal kexec leaves after Linux ran another mode on HDMI0
/// (1920x1080 at 60 Hz, both syncs positive): Linux's shutdown has put VP0
/// in STANDBY, moved DCLK_VOP0 back to its CRU source and gated it, gated
/// the VOP's bus clocks (ACLK_VOP 52/9, HCLK_VOP 52/8), stopped the PHY's
/// PLL and left paging off (driver/docs/nanopc-t6-display.md, section 2
/// (a)). PD_VOP and PD_VO1 stay powered.
fn after_normal_kexec() -> Rk3588Display {
    let m = chip(Faults::default());
    // the chain stands in for Linux's bring-up; then Linux's mode words
    assert_eq!(cold(&Mapped(&m), &B, &m, flags::POWER_CYCLE, 0, 0x2000_0000), Ok(Cold::default()));
    // VP0: htotal 2200, hsync 44; hact 192..2112; vtotal 1125, vsync 5; vact 41..1121
    for (off, w) in
        [(0xc48, 2200 << 16 | 44), (0xc4c, 192 << 16 | 2112), (0xc50, 1125 << 16 | 5), (0xc54, 41 << 16 | 1121)]
    {
        m.poke(VOP + off, w);
    }
    m.poke(VO1_GRF, m.peek(VO1_GRF) | 3 << 5);
    m.write32(VOP + 0xc00, 1 << 31);
    m.busy_wait_us(40_000);
    m.poke(sel_addr(0, 112), m.peek(sel_addr(0, 112)) & !(3 << 7));
    m.poke(gate_addr(0, 52), m.peek(gate_addr(0, 52)) | 1 << 13 | 1 << 9 | 1 << 8);
    m.poke(PHY_GRF, 0xffff_0000);
    m.busy_wait_us(1);
    m.clear_log();
    m.clear_lines();
    m
}

#[test]
fn a_cold_chip_shows_the_buffer_and_logs_every_stage() {
    let m = chip(Faults::default());
    assert_eq!(cold(&Mapped(&m), &B, &m, flags::POWER_CYCLE, 0, FB), Ok(Cold::default()));
    assert_eq!(m.refused(), Vec::<String>::new());
    assert!(m.port_runs(0));
    assert_eq!(m.fetching(), WIN_ESMART0);
    assert_eq!(m.peek(VOP + 0x1814), FB as u32, "Esmart0's YRGB_MST");
    let l = lines(&m);
    // the reads before the chain come first, then one line per stage, then
    // the report
    let at = |start: &str| l.iter().position(|x| x.starts_with(start)).unwrap_or_else(|| panic!("{start}: {l:#?}"));
    let order = [
        "t6 display: before: PD_VOP PWR ",
        "t6 display: before: PD_VOP powered 0 usable 0, PD_VO1 powered 0 usable 0",
        "t6 display: before: vop not readable: DomainOff(\"vop\")",
        "t6 display: before: hdmi-tx0 not readable: DomainOff(\"vo1\")",
        "t6 display: before: hdptx0 not readable: Gated(",
        "t6 display: before: CLKSEL_CON(112) 0x00000000 (DCLK_VOP0 mux 0), PHY GRF not read",
        "t6 display: stage 1 START",
        "t6 display: found VOP off",
        "t6 display: stage 2 POWER",
        "t6 display: stage 8 VIDEO",
        "t6 display: up, 1024x768 at 0x10012000",
        "t6 display: START: running 0x0, stopped 0x0, no pixel clock 0x0, PHY powered off 0",
        "t6 display: POWER: ACLK_VOP set ",
        "t6 display: PHY: locked after ",
        "t6 display: CONTROLLER: CORE_ID 0x48515458 ",
        "t6 display: VOP: VERSION_INFO 0x40176786 ",
        "t6 display: VOP: overlay OVL_LAYER_SEL ",
        "t6 display: VIDEO: frame start 1 underflow 0 bus error 0 0, YRGB_MST 0x10012000, ",
    ];
    for w in order.windows(2) {
        assert!(at(w[0]) < at(w[1]), "{} before {}", w[0], w[1]);
    }
    // no line is cut at the log's buffer (env::LINE_MAX)
    for x in &l {
        assert!(x.len() < LINE_MAX && !x.ends_with("..."), "{x}");
    }
}

#[test]
fn after_a_normal_kexec_from_another_mode_the_power_cycle_runs_through() {
    // with power_cycle (D8's run): the chain makes VOP2 readable, finds no
    // port running and powers everything off first; Linux's mode is not
    // looked at
    let m = after_normal_kexec();
    assert_eq!(cold(&Mapped(&m), &B, &m, flags::POWER_CYCLE, 0, FB), Ok(Cold::default()));
    assert_eq!(m.refused(), Vec::<String>::new());
    assert!(m.port_runs(0) && m.fetching() == WIN_ESMART0);
    assert_eq!(m.peek(VO1_GRF) & 3 << 5, 0, "the sync field is the mode's (both negative)");
    assert!(has(&m, "t6 display: before: PD_VOP powered 1 usable 1, PD_VO1 powered 1 usable 1"));
    assert!(has(&m, "t6 display: before: vop not readable: Gated("));
    assert!(has(&m, "t6 display: START: running 0x0, stopped 0x0, no pixel clock 0x0, PHY powered off 0"));

    // without it: VOP2 is powered but its bus clocks are gated, so whether
    // a port runs is not known; refused before any write
    let m = after_normal_kexec();
    let c = cold(&Mapped(&m), &B, &m, 0, 0, FB).unwrap();
    assert_eq!((c.stage, c.error, c.index), (stage::START, ColdError::VopUnreadable as u32, 2));
    assert!(m.writes().is_empty());
    assert!(m
        .lines()
        .iter()
        .any(|(lv, l)| *lv == Level::Err && l.starts_with("t6 display: stage 1 START failed: VopUnreadable(Gated(")));

    // and with its bus clocks open the gated pixel clock refuses it
    let m = after_normal_kexec();
    m.poke(gate_addr(0, 52), m.peek(gate_addr(0, 52)) & !(1 << 9 | 1 << 8));
    let c = cold(&Mapped(&m), &B, &m, flags::FORCE_COLD, 0, FB).unwrap();
    assert_eq!((c.stage, c.error, c.detail), (stage::START, ColdError::Unclocked as u32, 1));
    assert!(m.writes().is_empty());
}

#[test]
fn a_port_left_running_is_refused_without_force_cold() {
    let m = chip(Faults::default());
    assert_eq!(cold(&Mapped(&m), &B, &m, flags::POWER_CYCLE, 0, 0x2000_0000), Ok(Cold::default()));
    m.leave_linux_running(false);
    m.clear_lines();
    let c = cold(&Mapped(&m), &B, &m, flags::POWER_CYCLE, 0, FB).unwrap();
    assert_eq!(c, Cold { stage: stage::START, error: ColdError::FoundRunning as u32, detail: 1, ..Cold::default() });
    let c = cold(&Mapped(&m), &B, &m, flags::POWER_CYCLE | flags::FORCE_COLD, 0, FB).unwrap();
    assert_eq!(c, Cold::default());
    assert_eq!(m.fetching(), WIN_ESMART0);
}

#[test]
fn each_stop_after_is_reported_as_asked_and_names_its_stage_last() {
    for n in stage::START..=stage::VIDEO {
        let m = chip(Faults::default());
        let c = cold(&Mapped(&m), &B, &m, flags::POWER_CYCLE, n, FB).unwrap();
        assert_eq!(c, Cold { stage: n, error: ColdError::Stopped as u32, ..Cold::default() }, "stop_after {n}");
        let l = lines(&m);
        let last_stage = l.iter().rfind(|x| x.starts_with("t6 display: stage ")).unwrap();
        assert_eq!(*last_stage, format!("t6 display: stage {n} {}", stage::name(n)));
        assert_eq!(l.last().unwrap(), &format!("t6 display: stopped after stage {n} {} as asked", stage::name(n)));
        assert_eq!(m.refused(), Vec::<String>::new());
    }
}

#[test]
fn a_failure_is_recorded_with_its_stage_and_numbers() {
    let m = chip(Faults { phy_no_lock: true, ..Faults::default() });
    let c = cold(&Mapped(&m), &B, &m, flags::POWER_CYCLE, 0, FB).unwrap();
    assert_eq!((c.stage, c.error, c.refusal), (stage::PHY, ColdError::Phy as u32, PhyRefusal::LockTimeout as u32));
    assert!(m.lines().iter().any(|(lv, l)| *lv == Level::Err && l.starts_with("t6 display: stage 4 PHY failed: Phy(")));
    let m = chip(Faults { underflow: true, ..Faults::default() });
    let c = cold(&Mapped(&m), &B, &m, flags::POWER_CYCLE, 0, FB).unwrap();
    assert_eq!((c.stage, c.error, c.detail), (stage::VIDEO, ColdError::Video as u32, chain::video_check::UNDERFLOW));
    // a GRF word that does not take is named by its GRF and offset, the
    // same on the board as here (not by its mapped address)
    let m = chip(Faults { grf_stuck: true, ..Faults::default() });
    let c = cold(&Mapped(&m), &B, &m, flags::POWER_CYCLE, 0, FB).unwrap();
    assert_eq!((c.stage, c.error, c.index), (stage::GRF, ColdError::Grf as u32, 1 << 16 | 0x08));
    assert!(m.lines().iter().any(|(_, l)| l.starts_with("t6 display: stage 3 GRF failed: Grf(VopCon2, ")));
    let m = chip(Faults { dead: Some("vo1"), ..Faults::default() });
    let c = cold(&Mapped(&m), &B, &m, flags::POWER_CYCLE, 0, FB).unwrap();
    assert_eq!((c.stage, c.error, c.index), (stage::POWER, ColdError::Display as u32, 2));
}

#[test]
fn the_mode_flag_shows_the_same_buffer_at_1920x1080() {
    let m = chip(Faults::default());
    assert_eq!(cold(&Mapped(&m), &B, &m, flags::POWER_CYCLE | flags::MODE_1920X1080, 0, FB), Ok(Cold::default()));
    assert_eq!(m.refused(), Vec::<String>::new());
    assert!(m.port_runs(0));
    assert_eq!(m.fetching(), WIN_ESMART0);
    // VP0 runs the second mode's timing (the words the 1920x1080 dump reads)
    assert_eq!(m.peek(VOP + 0xc48), 0x0898_002c, "VP0 DSP_HTOTAL_HS_END");
    assert_eq!(m.peek(VOP + 0xc50), 0x0465_0005, "VP0 DSP_VTOTAL_VS_END");
    // Esmart0 shows the buffer as it is, 1024x768 from the active area's
    // first pixel, and the rest of the screen is the background, black
    assert_eq!(m.peek(VOP + 0x1814), FB as u32, "Esmart0's YRGB_MST");
    assert_eq!(m.peek(VOP + 0x1820), 767 << 16 | 1023, "Esmart0's ACT_INFO");
    assert_eq!(m.peek(VOP + 0x1824), 767 << 16 | 1023, "Esmart0's DSP_INFO");
    assert_eq!(m.peek(VOP + 0xc2c), 0, "VP0 DSP_BG");
    assert!(has(&m, "t6 display: up, 1920x1080 at 0x10012000"), "{:#?}", lines(&m));
}

#[test]
fn bad_arguments_are_refused_before_anything_is_read() {
    let m = chip(Faults::default());
    assert_eq!(cold(&Mapped(&m), &B, &m, 1 << 3, 0, FB), Err(Refused::Flags(1 << 3)));
    assert_eq!(
        cold(&Mapped(&m), &B, &m, flags::POWER_CYCLE, stage::SHUT_DOWN, FB),
        Err(Refused::StopAfter(stage::SHUT_DOWN))
    );
    assert!(m.writes().is_empty() && m.lines().is_empty());
}

#[test]
fn the_bases_map_onto_the_chains() {
    assert_eq!(PHYS.chain(), chain::RK3588);
    assert_eq!(B.chain(), crate::model_rk3588_display::chain_bases());
}
