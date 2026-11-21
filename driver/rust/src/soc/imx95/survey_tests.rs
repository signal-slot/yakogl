// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! [`super::survey`] against the mock System Manager ([`super::sm_mock`]):
//! it sends no message that writes, every message it sends is one
//! [`read_only`] admits, its lines are `key = value` with keys that do not
//! repeat, and a refusal is a line, not a stop.

use super::sm_mock::{writes, Sm, B, C, F, P};
use super::survey::{read_only, survey, Counts, Stop, CLOCKS, LINE_MAX, PERF_DOMAINS, POWER_DOMAINS};
use crate::firmware::scmi::{self, Transport};

fn run(sm: &mut Sm) -> (Result<Counts, (Stop, Counts)>, Vec<String>) {
    let mut lines = vec![];
    let r = survey(sm, |l| lines.push(l.to_string()));
    (r, lines)
}

fn value<'a>(lines: &'a [String], key: &str) -> &'a str {
    let want = format!("{key} = ");
    let l = lines.iter().find(|l| l.starts_with(&want)).unwrap_or_else(|| panic!("no line {key:?} in {lines:#?}"));
    &l[want.len()..]
}

/// Every message the survey may send reads; every message the power
/// sequences send to change the SM's state is refused by the table.
#[test]
fn the_table_admits_only_reads() {
    for (p, m) in [
        (P, scmi::POWER_STATE_SET),
        (F, scmi::PERF_LIMITS_SET),
        (F, scmi::PERF_LEVEL_SET),
        (C, scmi::CLOCK_RATE_SET),
        (C, scmi::CLOCK_CONFIG_SET),
        (C, scmi::CLOCK_PARENT_SET),
        (scmi::PROTO_RESET, scmi::RESET_RESET),
        (scmi::PROTO_VOLTAGE, scmi::VOLTAGE_LEVEL_SET),
        (scmi::PROTO_VOLTAGE, scmi::VOLTAGE_CONFIG_SET),
        // BASE_SET_DEVICE_PERMISSIONS, _PROTOCOL_PERMISSIONS, BASE_RESET_AGENT_CONFIGURATION
        (B, 0x9),
        (B, 0xa),
        (B, 0xb),
        // a vendor protocol's own messages (NXP's LMM, BBM, MISC)
        (0x80, 0x4),
        (0x81, 0x3),
        (0x84, 0x3),
    ] {
        assert!(!read_only(p, m), "0x{p:02x}/0x{m:02x} is a write");
    }
    for p in [B, P, F, C, 0x80, 0x84] {
        assert!(read_only(p, scmi::PROTOCOL_VERSION) && read_only(p, scmi::PROTOCOL_ATTRIBUTES));
    }
}

/// The Verdin's SM as the mock serves it: nothing written, every message
/// read-only, the facts the mock holds read back in the lines' format.
#[test]
fn the_survey_reads_the_verdin_sm_and_writes_nothing() {
    let mut sm = Sm::verdin();
    let (r, lines) = run(&mut sm);
    let counts = r.expect("the survey ran to its end");

    assert_eq!(writes(&sm.log), vec![], "the survey wrote to the SM");
    for (p, m, _) in &sm.log {
        assert!(read_only(*p, *m), "0x{p:02x}/0x{m:02x} sent");
    }
    assert_eq!(counts.facts as usize, lines.len());
    assert_eq!(counts.broken, 0);

    // one line per fact, keys unique, within the line's bound
    let mut keys: Vec<&str> = lines.iter().map(|l| l.split(" = ").next().unwrap()).collect();
    assert!(lines.iter().all(|l| l.contains(" = ") && l.len() < LINE_MAX), "{lines:#?}");
    let n = keys.len();
    keys.sort_unstable();
    keys.dedup();
    assert_eq!(keys.len(), n, "a key repeats");
    // base 7, three protocols x 2, per power domain 2, per perf domain 4,
    // per clock 5
    assert_eq!(n, 7 + 3 * 2 + POWER_DOMAINS.len() * 2 + PERF_DOMAINS.len() * 4 + CLOCKS.len() * 5);

    // the base protocol: the list came in two replies
    assert_eq!(value(&lines, "base version"), "0x00020001");
    assert_eq!(value(&lines, "base attributes"), "0x00000603 (3 protocols, 6 agents)");
    assert_eq!(value(&lines, "base vendor"), "NXP");
    assert_eq!(value(&lines, "base agent"), "2 AP-NS");
    assert_eq!(value(&lines, "base protocols"), "0x11 0x13 0x14");
    assert_eq!(value(&lines, "base sub-vendor"), "NOT_SUPPORTED (-1)");
    assert_eq!(value(&lines, "proto 0x13 version"), "0x00040000");

    // the GPU and display state after the SM's boot (the mock's)
    assert_eq!(value(&lines, "pd 14 state"), "0x40000000 (off)");
    assert_eq!(value(&lines, "pd 14 attributes"), "NOT_SUPPORTED (-1)");
    assert_eq!(value(&lines, "perf 9 levels"), "0:24000 1:500000 2:800000 3:1000000");
    assert_eq!(value(&lines, "perf 9 level"), "0");
    assert_eq!(value(&lines, "perf 12 level"), "3");
    assert_eq!(value(&lines, "perf 11 level"), "3");
    assert_eq!(value(&lines, "clk 80 rate"), "24000000");
    assert_eq!(value(&lines, "clk 80 parent"), "2");
    assert_eq!(value(&lines, "clk 80 permissions"), "0xe0000000 (state parent rate)");
    assert_eq!(value(&lines, "clk 75 permissions"), "0x00000000 (none)");
    assert_eq!(value(&lines, "clk 174 parent"), "NOT_SUPPORTED (-1)");
}

/// A state the survey must report as it finds it: a domain on, a level set.
#[test]
fn the_survey_reports_the_state_it_finds() {
    let mut sm = Sm::verdin();
    sm.pd_on.insert(13);
    sm.level.insert(9, 2);
    let (r, lines) = run(&mut sm);
    assert!(r.is_ok());
    assert_eq!(value(&lines, "pd 13 state"), "0x00000000 (on)");
    assert_eq!(value(&lines, "perf 9 level"), "2");
    assert_eq!(writes(&sm.log), vec![]);
}

struct Dead(u32);

impl Transport for Dead {
    fn transfer(&mut self, _tx: &[u8], _rx: &mut [u8]) -> Result<usize, i32> {
        self.0 += 1;
        Err(-110)
    }
}

/// A channel that never answers: one line, one message, then the stop.
#[test]
fn a_dead_channel_stops_after_the_base_version() {
    let mut t = Dead(0);
    let mut lines = vec![];
    let r = survey(&mut t, |l| lines.push(l.to_string()));
    assert_eq!(r, Err((Stop::NoBase, Counts { facts: 1, refused: 0, broken: 1 })));
    assert_eq!(lines, ["base version = TRANSPORT ERROR (-110)"]);
    assert_eq!(t.0, 1);
}

/// The C side's bound on a line and its entry are the header's.
#[test]
fn the_header_names_the_survey() {
    const H: &str = include_str!("../../../include/yakogl_drivers.h");
    assert!(H.contains(&format!("#define YD_IMX95_SURVEY_LINE_MAX {LINE_MAX} ")));
    assert!(H.contains("int yd_imx95_scmi_survey("));
}
