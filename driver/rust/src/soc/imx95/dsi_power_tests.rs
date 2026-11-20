// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! [`super::dsi_power`] against the mock System Manager
//! ([`super::sm_mock`]). The expected requests are written out by hand
//! (`sm_mock::dsi_on`, `dsi_off`), not computed by the code under test.

use super::dsi_power::{self, Config, Error, Step, CLK_CAMAPB, CLK_MIPIPHYPLLBYPASS, CLK_TPM6};
use super::sm::{Level, Status};
use super::sm_mock::{dsi_off, dsi_on, req, writes, Sm, C, F, PERM_NONE, PERM_SET};
use crate::firmware::scmi::{self, Transport, CLOCK_PARENT_GET, CLOCK_PARENT_SET};
use std::format;
use std::vec::Vec;

#[test]
fn the_ids_are_the_sms() {
    assert_eq!(dsi_power::PD_CAMERA, 3);
    assert_eq!(dsi_power::PERF_CAM, 11);
    assert_eq!(
        [
            dsi_power::CLK_CAMAPB,
            dsi_power::CLK_MIPIPHYCFG,
            CLK_MIPIPHYPLLBYPASS,
            dsi_power::CLK_MIPIPHYPLLREF,
            CLK_TPM6
        ],
        [62, 66, 67, 68, 155]
    );
    assert_eq!((dsi_power::CLK_OSC24M, dsi_power::CLK_VIDEOPLL1), (2, 19));
}

#[test]
fn power_on_sends_camera_then_reads_the_level_then_the_parents_then_the_clocks() {
    let mut sm = Sm::verdin();
    let up = dsi_power::power_on(&mut sm, &Config::VERDIN).unwrap();
    assert_eq!(sm.log, dsi_on());
    // the level as Toradex's SM starts it: ODV (3)
    assert_eq!((up.level, up.chosen), (3, None));
    assert_eq!((up.cfg_hz, up.ref_hz, up.tpm_hz), (24_000_000, 24_000_000, 24_000_000));
    assert!(sm.on(3));
    for c in [66, 68, 155] {
        assert!(sm.enabled(c), "clock {c}");
    }
    assert!(!sm.enabled(67), "the bypass is never enabled");
    assert_eq!((sm.parent[&66], sm.parent[&68], sm.parent[&67]), (2, 2, 19));
}

#[test]
fn a_cam_rate_sets_the_fastest_level_at_or_below_it() {
    let mut sm = Sm::verdin();
    let up = dsi_power::power_on(&mut sm, &Config { cam_hz: Some(700_000_000), ..Config::VERDIN }).unwrap();
    assert_eq!(up.chosen, Some(Level { index: 2, khz: 666_667 }));
    assert_eq!(up.level, 2);
    let mut want = dsi_on();
    want.insert(2, req(F, scmi::PERF_DESCRIBE_LEVELS, &[11, 0]));
    want[6] = req(F, scmi::PERF_LEVEL_SET, &[11, 2]);
    want.insert(7, req(F, scmi::PERF_LEVEL_GET, &[11]));
    assert_eq!(sm.log, want);

    let mut sm = Sm::verdin();
    let e = dsi_power::power_on(&mut sm, &Config { cam_hz: Some(100_000_000), ..Config::VERDIN }).unwrap_err();
    assert_eq!(e, Error::NoLevel { rate_hz: 100_000_000 });
    assert!(writes(&sm.log).is_empty(), "nothing written");
}

#[test]
fn power_off_sends_the_clocks_then_the_domain_and_reads_it_off() {
    let mut sm = Sm::verdin();
    dsi_power::power_on(&mut sm, &Config::VERDIN).unwrap();
    sm.log.clear();
    dsi_power::power_off(&mut sm).unwrap();
    assert_eq!(sm.log, dsi_off());
    assert!(!sm.on(3));
    assert_eq!(sm.level_of(11), 3, "the level is left");
}

#[test]
fn camapb_and_rates_are_never_sent_and_the_bypass_never_enabled() {
    let mut sm = Sm::verdin();
    dsi_power::power_on(&mut sm, &Config::VERDIN).unwrap();
    dsi_power::power_off(&mut sm).unwrap();
    for (p, m, w) in &sm.log {
        assert!(!(*p == C && *m == scmi::CLOCK_RATE_SET), "a rate was set: {w:?}");
        if *p == C && *m != scmi::PROTOCOL_VERSION {
            assert!(w[0] != CLK_CAMAPB, "{m:#x} {w:?}");
        }
        if *p == C && *m == scmi::CLOCK_CONFIG_SET {
            assert!(w[0] != CLK_MIPIPHYPLLBYPASS, "{w:?}");
        }
    }
}

#[test]
fn without_set_parents_no_parent_is_sent() {
    let mut sm = Sm::verdin();
    dsi_power::power_on(&mut sm, &Config { set_parents: false, ..Config::VERDIN }).unwrap();
    let want: Vec<_> = dsi_on().into_iter().filter(|r| r.1 != CLOCK_PARENT_SET && r.1 != CLOCK_PARENT_GET).collect();
    assert_eq!(sm.log, want);
}

#[test]
fn a_denied_domain_is_named_and_nothing_after_it_is_sent() {
    let mut sm = Sm::verdin();
    sm.pd_perm.insert(3, PERM_NONE);
    let e = dsi_power::power_on(&mut sm, &Config::VERDIN).unwrap_err();
    assert_eq!(e, Error::Refused { step: Step::PowerOn, status: Status::Denied });
    assert_eq!(format!("{e}"), "POWER_STATE_SET (0x11/0x04): SCMI DENIED (-3)");
    assert_eq!(sm.log, dsi_on()[..4]);
}

#[test]
fn a_parent_below_exclusive_is_denied_and_named() {
    let mut sm = Sm::verdin();
    sm.clk_perm.insert(67, PERM_SET);
    let e = dsi_power::power_on(&mut sm, &Config::VERDIN).unwrap_err();
    assert_eq!(e, Error::Refused { step: Step::ParentSet(67), status: Status::Denied });
    assert_eq!(format!("{e}"), "CLOCK_PARENT_SET (0x14/0x0d) clock 67: SCMI DENIED (-3)");
    assert_eq!(sm.log, dsi_on()[..11]);
    assert!(sm.on(3), "what was switched on stays on");
}

#[test]
fn a_denied_clock_is_named_with_its_id_and_stops_there() {
    let mut sm = Sm::verdin();
    sm.clk_perm.insert(68, PERM_NONE);
    let e = dsi_power::power_on(&mut sm, &Config { set_parents: false, ..Config::VERDIN }).unwrap_err();
    assert_eq!(e, Error::Refused { step: Step::Enable(68), status: Status::Denied });
    assert_eq!(format!("{e}"), "CLOCK_CONFIG_SET (0x14/0x07) clock 68: SCMI DENIED (-3)");
    assert_eq!(sm.log.last(), Some(&req(C, scmi::CLOCK_CONFIG_SET, &[68, 1, 0])), "nothing after it");
}

#[test]
fn a_phy_clock_off_24_mhz_is_refused_and_tpm_takes_any_rate_but_0() {
    // a board that left MIPIPHYCFG on VIDEOPLL1, without the parent step
    let mut sm = Sm::verdin();
    sm.parent.insert(66, 19);
    sm.rate.insert(19, 420_000_000);
    let e = dsi_power::power_on(&mut sm, &Config { set_parents: false, ..Config::VERDIN }).unwrap_err();
    assert_eq!(e, Error::Rate { clock: 66, want_hz: 24_000_000, got_hz: 420_000_000 });
    // the parent step puts it back
    let mut sm = Sm::verdin();
    sm.parent.insert(66, 19);
    sm.rate.insert(19, 420_000_000);
    assert_eq!(dsi_power::power_on(&mut sm, &Config::VERDIN).unwrap().cfg_hz, 24_000_000);

    let mut sm = Sm::verdin();
    sm.rate.insert(CLK_TPM6, 100_000_000);
    assert_eq!(dsi_power::power_on(&mut sm, &Config::VERDIN).unwrap().tpm_hz, 100_000_000);

    let mut sm = Sm::verdin();
    sm.rate.insert(CLK_TPM6, 0);
    let e = dsi_power::power_on(&mut sm, &Config::VERDIN).unwrap_err();
    assert_eq!(e, Error::Rate { clock: 155, want_hz: 0, got_hz: 0 });
}

#[test]
fn levels_over_several_replies_are_all_read() {
    let mut sm = Sm::verdin();
    sm.page = 1;
    dsi_power::power_on(&mut sm, &Config { cam_hz: Some(400_000_000), ..Config::VERDIN }).unwrap();
    let describes: Vec<_> =
        sm.log.iter().filter(|r| (r.0, r.1) == (scmi::PROTO_PERF, scmi::PERF_DESCRIBE_LEVELS)).cloned().collect();
    assert_eq!(describes, (0..4).map(|i| req(F, scmi::PERF_DESCRIBE_LEVELS, &[11, i])).collect::<Vec<_>>());
}

#[test]
fn a_camera_domain_another_agent_holds_fails_the_off_read_back() {
    let mut sm = Sm::verdin();
    dsi_power::power_on(&mut sm, &Config::VERDIN).unwrap();
    sm.pd_held.insert(3);
    let e = dsi_power::power_off(&mut sm).unwrap_err();
    assert_eq!(e, Error::Readback { step: Step::PowerOffCheck, want: scmi::POWER_OFF, got: scmi::POWER_ON });
}

#[test]
fn a_wrong_version_or_a_transport_failure_stops_before_any_write() {
    struct Old(Sm);
    impl Transport for Old {
        fn transfer(&mut self, tx: &[u8], rx: &mut [u8]) -> Result<usize, i32> {
            let n = self.0.transfer(tx, rx)?;
            let h = scmi::word(tx, 0);
            if h & 0xff == scmi::PROTOCOL_VERSION as u32 && h >> 10 & 0xff == scmi::PROTO_CLOCK as u32 {
                rx[8..12].copy_from_slice(&0x20000u32.to_le_bytes());
            }
            Ok(n)
        }
    }
    let mut t = Old(Sm::verdin());
    let e = dsi_power::power_on(&mut t, &Config::VERDIN).unwrap_err();
    assert_eq!(e, Error::Version { step: Step::ClockVersion, version: 0x20000 });
    assert!(writes(&t.0.log).is_empty());

    struct Dead;
    impl Transport for Dead {
        fn transfer(&mut self, _: &[u8], _: &mut [u8]) -> Result<usize, i32> {
            Err(-5)
        }
    }
    assert_eq!(
        dsi_power::power_on(&mut Dead, &Config::VERDIN).unwrap_err(),
        Error::Transport { step: Step::PowerVersion, code: -5 }
    );
}
