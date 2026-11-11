// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! `power::power_up` and `power::power_down` driven through the host model of
//! the whole path ([`crate::model_stm32mp2::Mp2Model`]): the scripted OP-TEE
//! SCMI server, `RCC_GPUCFGR` and the GPU's identity, idle and interrupt
//! registers. `power.rs`'s own tests use local fakes; these check the
//! sequence against a model that records faults, among them the one the
//! OP-TEE panic note warns of (an RCC write before the power domain is on,
//! docs/notes/pre-study-stm32mp257f-dk.md, "The etnaviv panic and what our
//! image avoids"). Plan #322, issues #7 and #1281.

use super::power::{self, Config, Error, Log, Status, Step};
use super::rcc::GpuRcc;
use crate::firmware::scmi::{self, Transport};
use crate::mmio::Mmio;
use crate::model_stm32mp2::{Event, Fault, Mp2Model, GPU_BASE, RCC_BASE};
use std::collections::HashMap;

const VECTORS: &str = include_str!("../../../../../tools/testdata/optee-scmi/scmi_msgs.txt");

const MODEL: usize = 0x020;
const REV: usize = 0x024;
const INTR_ENBL: usize = 0x014;

/// The power-up's exchanges in order: (request vector, reply vector).
const UP: [(&str, &str); 7] = [
    ("req_perf_protocol_version", "rep_perf_protocol_version"),
    ("req_perf_level_get", "rep_perf_level_get"),
    ("req_perf_level_set", "rep_perf_level_set"),
    ("req_perf_level_get", "rep_perf_level_get"),
    ("req_pd_protocol_version", "rep_pd_protocol_version"),
    ("req_pd_state_set_on", "rep_pd_state_set_on"),
    ("req_pd_state_get_on", "rep_pd_state_get_on"),
];
/// The power-down's.
const DOWN: [(&str, &str); 3] = [
    ("req_pd_protocol_version", "rep_pd_protocol_version"),
    ("req_pd_state_set_off", "rep_pd_state_set_off"),
    ("req_pd_state_get_off", "rep_pd_state_get_off"),
];
/// The step each power-up exchange belongs to.
const UP_STEPS: [Step; 7] = [
    Step::PerfVersion,
    Step::PerfLevelGet,
    Step::PerfLevelSet,
    Step::PerfLevelCheck,
    Step::PowerVersion,
    Step::PowerOn,
    Step::PowerOnCheck,
];
/// (protocol, message) of each power-up exchange, for `force_status`.
const UP_IDS: [(u8, u8); 7] = [(0x13, 0), (0x13, 8), (0x13, 7), (0x13, 8), (0x11, 0), (0x11, 4), (0x11, 5)];

fn hex(s: &str) -> Vec<u8> {
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
}

fn to_hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{:02x}", x)).collect()
}

fn vector(name: &str) -> Vec<u8> {
    let m: HashMap<&str, Vec<u8>> = VECTORS
        .lines()
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|l| {
            let (n, h) = l.split_once(' ').unwrap();
            (n, hex(h))
        })
        .collect();
    m.get(name).unwrap_or_else(|| panic!("no vector {}", name)).clone()
}

#[derive(Default)]
struct Lines(Vec<String>);
impl Log for Lines {
    fn line(&mut self, s: &str) {
        self.0.push(s.to_string());
    }
}

/// The model's transport, with an action run before the `at`-th transfer
/// (0-based): the way to fail one step of the sequence, since `force_status`
/// and `fail_transport` act on every later message.
struct Hook<'a, F: Fn(&Mp2Model)> {
    model: &'a Mp2Model,
    n: usize,
    at: usize,
    act: F,
}

impl<F: Fn(&Mp2Model)> Transport for Hook<'_, F> {
    fn transfer(&mut self, tx: &[u8], rx: &mut [u8]) -> Result<usize, i32> {
        if self.n == self.at {
            (self.act)(self.model);
        }
        self.n += 1;
        self.model.scmi().transfer(tx, rx)
    }
}

fn up(model: &Mp2Model, log: &mut Lines) -> Result<power::Up, Error> {
    let rcc = GpuRcc::new(model, RCC_BASE);
    power::power_up(&mut model.scmi(), &rcc, |off| model.read32(GPU_BASE + off), log, &Config::DK)
}

fn down(model: &Mp2Model, log: &mut Lines) -> Result<(), Error> {
    let rcc = GpuRcc::new(model, RCC_BASE);
    power::power_down(&mut model.scmi(), &rcc, log, &Config::DK)
}

fn up_hooked<F: Fn(&Mp2Model)>(model: &Mp2Model, at: usize, act: F, log: &mut Lines) -> Result<power::Up, Error> {
    let rcc = GpuRcc::new(model, RCC_BASE);
    let mut t = Hook { model, n: 0, at, act };
    power::power_up(&mut t, &rcc, |off| model.read32(GPU_BASE + off), log, &Config::DK)
}

/// No RCC or GPU access at all so far.
fn untouched(model: &Mp2Model) -> bool {
    !model.events().iter().any(|e| matches!(e, Event::RccR(..) | Event::RccW(..) | Event::GpuR(..) | Event::GpuW(..)))
}

fn rcc_writes(ev: &[Event]) -> Vec<u32> {
    ev.iter()
        .filter_map(|e| match e {
            Event::RccW(0x83c, v) => Some(*v),
            _ => None,
        })
        .collect()
}

#[test]
fn power_up_on_a_default_model() {
    let m = Mp2Model::new();
    let mut log = Lines::default();
    let u = up(&m, &mut log).unwrap();
    assert!(m.domain_on());
    // Gate (bit 1) set, reset (bit 0) released.
    assert_eq!(m.rcc() & 0x3, 0x2);
    assert_eq!(u.rcc, m.rcc());
    assert_eq!(m.gpu_level(), 800_000);
    assert_eq!((u.level_before, u.level), (800_000, 800_000));
    assert_eq!((u.idle & power::IDLE_MASK, u.idle_reads), (power::IDLE_MASK, 1));
    assert_eq!(m.faults(), vec![]);
    // The identity is Linux's (#1281).
    assert_eq!(m.read32(GPU_BASE + MODEL), 0x8000);
    assert_eq!(m.read32(GPU_BASE + REV), 0x6205);
    // The interrupt enable is storage the driver can write and read back.
    m.write32(GPU_BASE + INTR_ENBL, 0x8000_0001);
    assert_eq!(m.read32(GPU_BASE + INTR_ENBL), 0x8000_0001);
    assert_eq!(m.intr_enbl(), 0x8000_0001);
    assert_eq!(m.faults(), vec![]);
}

#[test]
fn power_up_messages_are_pinned_byte_for_byte() {
    let m = Mp2Model::new();
    up(&m, &mut Lines::default()).unwrap();
    let want: Vec<(Vec<u8>, Vec<u8>)> = UP.iter().map(|(q, p)| (vector(q), vector(p))).collect();
    assert_eq!(m.messages(), want);
    // The literal bytes too, so a changed vectors file cannot move both.
    let lit: [(&str, &str); 7] = [
        ("004c0400", "004c04000000000000000200"),
        ("084c040001000000", "084c04000000000000350c00"),
        ("074c04000100000000350c00", "074c040000000000"),
        ("084c040001000000", "084c04000000000000350c00"),
        ("00440400", "004404000000000000000200"),
        ("04440400000000000000000000000000", "0444040000000000"),
        ("0544040000000000", "054404000000000000000000"),
    ];
    let got: Vec<(String, String)> = m.messages().iter().map(|(a, b)| (to_hex(a), to_hex(b))).collect();
    let lit: Vec<(String, String)> = lit.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect();
    assert_eq!(got, lit);
}

#[test]
fn power_down_messages_are_pinned_byte_for_byte() {
    let m = Mp2Model::new();
    up(&m, &mut Lines::default()).unwrap();
    m.clear_log();
    down(&m, &mut Lines::default()).unwrap();
    let want: Vec<(Vec<u8>, Vec<u8>)> = DOWN.iter().map(|(q, p)| (vector(q), vector(p))).collect();
    assert_eq!(m.messages(), want);
    assert_eq!(to_hex(&m.messages()[1].0), "04440400000000000000000000000040");
}

#[test]
fn order_of_events_no_rcc_before_the_domain_is_on() {
    let m = Mp2Model::new();
    up(&m, &mut Lines::default()).unwrap();
    let ev = m.events();
    let set_on = vector("req_pd_state_set_on");
    let on_at = ev.iter().position(|e| matches!(e, Event::Scmi { tx, .. } if *tx == set_on)).unwrap();
    let first_rcc = ev.iter().position(|e| matches!(e, Event::RccR(..) | Event::RccW(..))).unwrap();
    let first_gpu = ev.iter().position(|e| matches!(e, Event::GpuR(..) | Event::GpuW(..))).unwrap();
    assert!(on_at < first_rcc && first_rcc < first_gpu);
    // Everything before the RCC is SCMI, the performance level before the domain.
    assert!(ev[..first_rcc].iter().all(|e| matches!(e, Event::Scmi { .. })));
    assert_eq!(first_rcc, UP.len());
    // The RCC writes in order: gate on, reset asserted, reset released.
    assert_eq!(rcc_writes(&ev), [0x2, 0x3, 0x2]);
    assert!(ev.iter().all(|e| !matches!(e, Event::GpuW(..))));
    assert_eq!(m.faults(), vec![]);
}

#[test]
fn the_model_flags_an_rcc_write_before_the_domain() {
    // Negative control for the guard above: etnaviv's order, the clock first.
    let m = Mp2Model::new();
    GpuRcc::new(&m, RCC_BASE).enable_clock();
    assert_eq!(m.faults(), vec![Fault::RccBeforeDomain { val: 0x2 }]);
}

#[test]
fn scmi_status_stops_the_sequence_at_the_step() {
    for (at, step) in UP_STEPS.iter().enumerate() {
        for (code, status) in [
            (scmi::STATUS_NOT_SUPPORTED, Status::NotSupported),
            (scmi::STATUS_DENIED, Status::Denied),
            (scmi::STATUS_OUT_OF_RANGE, Status::OutOfRange),
        ] {
            let m = Mp2Model::new();
            let (proto, msg) = UP_IDS[at];
            let e = up_hooked(&m, at, |m| m.force_status(proto, msg, code), &mut Lines::default());
            assert_eq!(e, Err(Error::Refused { step: *step, status }), "step {}", at);
            // The failed message was the last one sent; its reply is status-only.
            assert_eq!(m.messages().len(), at + 1, "step {}", at);
            assert_eq!(m.messages()[at].1.len(), 8);
            // The RCC and the GPU are untouched.
            assert!(untouched(&m), "step {}", at);
            assert_eq!(m.rcc(), 0);
            // The domain is on only if its set (step 5) went through.
            assert_eq!(m.domain_on(), at == 6, "step {}", at);
            assert_eq!(m.faults(), vec![], "step {}", at);
        }
    }
}

#[test]
fn transport_failure_stops_the_sequence_at_the_step() {
    for (at, step) in UP_STEPS.iter().enumerate() {
        let m = Mp2Model::new();
        let e = up_hooked(&m, at, |m| m.fail_transport(Some(-5)), &mut Lines::default());
        assert_eq!(e, Err(Error::Transport { step: *step, code: -5 }), "step {}", at);
        assert_eq!(m.messages().len(), at + 1);
        assert!(m.messages()[at].1.is_empty());
        assert!(untouched(&m), "step {}", at);
        assert_eq!(m.rcc(), 0);
        assert_eq!(m.faults(), vec![], "step {}", at);
    }
    // A transport that is down from the start: nothing but the first request.
    let m = Mp2Model::new();
    m.fail_transport(Some(-110));
    let e = up(&m, &mut Lines::default());
    assert_eq!(e, Err(Error::Transport { step: Step::PerfVersion, code: -110 }));
    assert_eq!(m.messages().len(), 1);
    assert!(untouched(&m) && !m.domain_on());
}

#[test]
fn a_refused_power_on_leaves_the_domain_off() {
    let m = Mp2Model::new();
    m.force_status(0x11, 4, scmi::STATUS_DENIED);
    let e = up(&m, &mut Lines::default());
    assert_eq!(e, Err(Error::Refused { step: Step::PowerOn, status: Status::Denied }));
    assert!(!m.domain_on());
    assert!(untouched(&m));
}

#[test]
fn power_down_after_power_up_is_the_reverse() {
    let m = Mp2Model::new();
    let mut log = Lines::default();
    up(&m, &mut log).unwrap();
    m.clear_log();
    down(&m, &mut log).unwrap();
    // Back where it started: domain off, clock gated, the level as it was.
    // The reset is held (bit 0), which the initial 0 is not: the sequence
    // ends with reset asserted and the gate off, as its documentation says.
    assert!(!m.domain_on());
    assert_eq!(m.rcc(), 0x1);
    assert_eq!(m.gpu_level(), 800_000);
    assert_eq!(m.faults(), vec![]);
    let ev = m.events();
    // Reset asserted (0x3), gate off (0x1), then the SCMI messages.
    assert_eq!(rcc_writes(&ev), [0x3, 0x1]);
    let last_rcc = ev.iter().rposition(|e| matches!(e, Event::RccR(..) | Event::RccW(..))).unwrap();
    let first_scmi = ev.iter().position(|e| matches!(e, Event::Scmi { .. })).unwrap();
    assert!(last_rcc < first_scmi);
    assert_eq!(m.messages().len(), 3);
    assert!(ev.iter().all(|e| !matches!(e, Event::GpuR(..) | Event::GpuW(..))));
    // And a second power-up works from there.
    m.clear_log();
    up(&m, &mut log).unwrap();
    assert!(m.domain_on());
    assert_eq!(m.rcc() & 0x3, 0x2);
    assert_eq!(m.faults(), vec![]);
}

#[test]
fn power_down_failure_keeps_the_gate_off_and_the_domain_on() {
    // The RCC step comes first, so the clock is gated whatever SCMI answers;
    // the domain is left as the failed step found it (the set of step 1 is
    // refused, so it stays on; the read-back of step 2 follows a set that
    // went through, so it is off).
    let steps = [
        (0, Step::PowerVersion, (0x11, 0), true),
        (1, Step::PowerOff, (0x11, 4), true),
        (2, Step::PowerOffCheck, (0x11, 5), false),
    ];
    for (at, step, id, on) in steps {
        let m = Mp2Model::new();
        up(&m, &mut Lines::default()).unwrap();
        m.clear_log();
        let rcc = GpuRcc::new(&m, RCC_BASE);
        let mut t =
            Hook { model: &m, n: 0, at, act: |m: &Mp2Model| m.force_status(id.0, id.1, scmi::STATUS_GENERIC_ERROR) };
        let e = power::power_down(&mut t, &rcc, &mut Lines::default(), &Config::DK);
        assert_eq!(e, Err(Error::Refused { step, status: Status::GenericError }));
        assert_eq!(m.rcc() & 0x3, 0x1);
        assert_eq!(m.domain_on(), on, "step {}", at);
        assert_eq!(m.faults(), vec![], "step {}", at);
    }
}

#[test]
fn every_message_goes_to_the_log() {
    let m = Mp2Model::new();
    let mut log = Lines::default();
    up(&m, &mut log).unwrap();
    down(&m, &mut log).unwrap();
    let scmi_lines: Vec<&str> = log.0.iter().filter(|l| l.starts_with("scmi")).map(String::as_str).collect();
    let want: Vec<String> = m
        .messages()
        .iter()
        .flat_map(|(q, p)| [format!("scmi > {}", to_hex(q)), format!("scmi < {}", to_hex(p))])
        .collect();
    assert_eq!(scmi_lines, want);
    assert_eq!(scmi_lines.len(), 2 * (UP.len() + DOWN.len()));
    // The RCC and idle lines are there too.
    assert!(log.0.iter().any(|l| l == "rcc gpucfgr 0x00000002"));
    assert!(log.0.iter().any(|l| l == "rcc gpucfgr 0x00000001"));
    assert!(log.0.iter().any(|l| l == "gpu idle 0x7fffffff after 1 reads"));
}

#[test]
fn the_request_is_logged_before_it_is_sent_even_when_the_transport_fails() {
    let m = Mp2Model::new();
    let mut log = Lines::default();
    let e = up_hooked(&m, 5, |m| m.fail_transport(Some(-5)), &mut log);
    assert!(e.is_err());
    let n = log.0.len();
    assert_eq!(log.0[n - 2], format!("scmi > {}", to_hex(&vector("req_pd_state_set_on"))));
    assert_eq!(log.0[n - 1], "scmi ! transport -5");
    // A refusal logs the request and the status-only reply.
    let m = Mp2Model::new();
    m.force_status(0x13, 7, scmi::STATUS_DENIED);
    let mut log = Lines::default();
    assert!(up(&m, &mut log).is_err());
    let n = log.0.len();
    assert_eq!(log.0[n - 2], "scmi > 074c04000100000000350c00");
    assert_eq!(log.0[n - 1], "scmi < 074c0400fdffffff");
}
