// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! [`super::gpu_power`] against a mock System Manager: a host model of the
//! part of NXP's SM the GPU sequence reaches, with the permission checks of
//! its SCMI handlers (rpc_scmi_power.c `PowerStateSet`: SET; rpc_scmi_perf.c
//! `PerformanceLevelSet`: PRIV; rpc_scmi_clock.c `ClockConfigSet`: SET,
//! `ClockRateSet`: EXCLUSIVE, and INVALID_PARAMETERS for a rate on a gate,
//! dev_sm_clock.c `DEV_SM_ClockRateSet`) and the AP-NS agent's permissions
//! of Toradex's verdin-imx95 configuration (docs/notes/imx95-gpu-power.md).
//! Every request is recorded as (protocol, message, payload words). The
//! model and the power-on's requests serve board::imx95_gpu's bring-up test
//! too.

use super::gpu_power::{
    self, pick_level, Config, Error, Level, Status, Step, CLK_GPU, CLK_GPUAPB, CLK_GPU_CGC, PD_GPU, PERF_GPU,
};
use crate::firmware::scmi::{self, Transport};
use std::collections::HashMap;

/// SM permission levels (rpc_scmi_config.h).
const PERM_NONE: u8 = 0;
const PERM_SET: u8 = 3;
const PERM_PRIV: u8 = 4;
const PERM_EXCLUSIVE: u8 = 5;
const PERM_ALL: u8 = 255;

/// One request: protocol, message, payload words.
pub(crate) type Req = (u8, u8, Vec<u32>);

pub(crate) struct Sm {
    clk_perm: HashMap<u32, u8>,
    pub(crate) pd_perm: HashMap<u32, u8>,
    perf_perm: HashMap<u32, u8>,
    /// (kHz, index) of domain 9, in index order.
    levels: Vec<(u32, u32)>,
    /// Levels per DESCRIBE_LEVELS reply.
    page: usize,
    pub(crate) pd_on: bool,
    /// Another agent's vote keeps the domain on.
    pd_held: bool,
    pub(crate) level: u32,
    pub(crate) clk_on: bool,
    /// What the clock root makes of a level's rate (a PLL's rounding).
    rate_offset: i64,
    pub(crate) log: Vec<Req>,
}

impl Sm {
    /// The Verdin iMX95's SM as the AP-NS agent sees it: power domain 14,
    /// performance domain 9 and clock 174 at ALL; clocks 83 and 84 not
    /// granted; the GPU's levels PRK, LOW, NOM, ODV (dev_sm_perf.c).
    pub(crate) fn verdin() -> Sm {
        Sm {
            clk_perm: HashMap::from([(CLK_GPU_CGC, PERM_ALL)]),
            pd_perm: HashMap::from([(PD_GPU, PERM_ALL)]),
            perf_perm: HashMap::from([(PERF_GPU, PERM_ALL)]),
            levels: vec![(24_000, 0), (500_000, 1), (800_000, 2), (1_000_000, 3)],
            page: 8,
            pd_on: false,
            pd_held: false,
            level: 0,
            clk_on: false,
            rate_offset: 0,
            log: Vec::new(),
        }
    }

    fn perm(map: &HashMap<u32, u8>, id: u32) -> u8 {
        *map.get(&id).unwrap_or(&PERM_NONE)
    }

    fn rate(&self) -> u64 {
        let khz = self.levels.iter().find(|l| l.1 == self.level).map_or(0, |l| l.0);
        (khz as i64 * 1000 + self.rate_offset) as u64
    }

    /// The SM's answer: status and return values.
    fn serve(&mut self, proto: u8, msg: u8, w: &[u32]) -> (i32, Vec<u32>) {
        const OK: i32 = scmi::STATUS_SUCCESS;
        match (proto, msg) {
            (scmi::PROTO_POWER, scmi::PROTOCOL_VERSION) => (OK, vec![0x30001]),
            (scmi::PROTO_PERF, scmi::PROTOCOL_VERSION) => (OK, vec![0x40000]),
            (scmi::PROTO_CLOCK, scmi::PROTOCOL_VERSION) => (OK, vec![0x30000]),
            (scmi::PROTO_POWER, scmi::POWER_STATE_SET) => {
                if Self::perm(&self.pd_perm, w[1]) < PERM_SET {
                    return (scmi::STATUS_DENIED, vec![]);
                }
                self.pd_on = w[2] == scmi::POWER_ON || self.pd_held;
                (OK, vec![])
            }
            (scmi::PROTO_POWER, scmi::POWER_STATE_GET) => {
                (OK, vec![if self.pd_on { scmi::POWER_ON } else { scmi::POWER_OFF }])
            }
            (scmi::PROTO_PERF, scmi::PERF_DESCRIBE_LEVELS) => {
                let skip = w[1] as usize;
                let rest = &self.levels[skip.min(self.levels.len())..];
                let n = rest.len().min(self.page);
                let mut v = vec![n as u32 | ((rest.len() - n) as u32) << 16];
                for &(khz, index) in &rest[..n] {
                    v.extend([khz, 0, 1, khz, index]);
                }
                (OK, v)
            }
            (scmi::PROTO_PERF, scmi::PERF_LEVEL_SET) => {
                if Self::perm(&self.perf_perm, w[0]) < PERM_PRIV {
                    return (scmi::STATUS_DENIED, vec![]);
                }
                if w[1] as usize >= self.levels.len() {
                    return (scmi::STATUS_INVALID_PARAMETERS, vec![]);
                }
                self.level = w[1];
                (OK, vec![])
            }
            (scmi::PROTO_PERF, scmi::PERF_LEVEL_GET) => (OK, vec![self.level]),
            (scmi::PROTO_CLOCK, scmi::CLOCK_CONFIG_SET) => {
                if Self::perm(&self.clk_perm, w[0]) < PERM_SET {
                    return (scmi::STATUS_DENIED, vec![]);
                }
                self.clk_on = w[1] & 3 == scmi::CLOCK_ENABLE;
                (OK, vec![])
            }
            (scmi::PROTO_CLOCK, scmi::CLOCK_RATE_SET) => {
                if Self::perm(&self.clk_perm, w[1]) < PERM_EXCLUSIVE {
                    return (scmi::STATUS_DENIED, vec![]);
                }
                // A gate has no rate of its own (DEV_SM_ClockRateSet).
                (scmi::STATUS_INVALID_PARAMETERS, vec![])
            }
            (scmi::PROTO_CLOCK, scmi::CLOCK_RATE_GET) => {
                let r = self.rate();
                (OK, vec![r as u32, (r >> 32) as u32])
            }
            _ => (scmi::STATUS_NOT_SUPPORTED, vec![]),
        }
    }
}

impl Transport for Sm {
    fn transfer(&mut self, tx: &[u8], rx: &mut [u8]) -> Result<usize, i32> {
        assert!(tx.len() >= 4 && tx.len().is_multiple_of(4), "a request is whole words");
        let hdr = scmi::word(tx, 0);
        assert_eq!(hdr >> 8 & 3, 0, "a command");
        assert_eq!(hdr >> 18 & 0x3ff, gpu_power::TOKEN as u32, "the token");
        let proto = (hdr >> 10 & 0xff) as u8;
        let msg = (hdr & 0xff) as u8;
        let words: Vec<u32> = (1..tx.len() / 4).map(|i| scmi::word(tx, i)).collect();
        self.log.push((proto, msg, words.clone()));
        let (status, values) = self.serve(proto, msg, &words);
        rx[..4].copy_from_slice(&hdr.to_le_bytes());
        rx[4..8].copy_from_slice(&status.to_le_bytes());
        for (i, v) in values.iter().enumerate() {
            rx[8 + 4 * i..12 + 4 * i].copy_from_slice(&v.to_le_bytes());
        }
        Ok(8 + 4 * values.len())
    }
}

const P: u8 = scmi::PROTO_POWER;
const F: u8 = scmi::PROTO_PERF;
const C: u8 = scmi::PROTO_CLOCK;

fn req(p: u8, m: u8, w: &[u32]) -> Req {
    (p, m, w.to_vec())
}

/// The reads before the first write, from one DESCRIBE_LEVELS reply.
fn reads() -> Vec<Req> {
    vec![
        req(P, scmi::PROTOCOL_VERSION, &[]),
        req(F, scmi::PROTOCOL_VERSION, &[]),
        req(F, scmi::PERF_DESCRIBE_LEVELS, &[PERF_GPU, 0]),
        req(C, scmi::PROTOCOL_VERSION, &[]),
    ]
}

/// The power-on's requests for level `index`.
pub(crate) fn on(index: u32) -> Vec<Req> {
    let mut v = reads();
    v.extend([
        req(P, scmi::POWER_STATE_SET, &[0, 14, 0]),
        req(P, scmi::POWER_STATE_GET, &[14]),
        req(F, scmi::PERF_LEVEL_SET, &[9, index]),
        req(F, scmi::PERF_LEVEL_GET, &[9]),
        req(C, scmi::CLOCK_CONFIG_SET, &[174, 1, 0]),
        req(C, scmi::CLOCK_RATE_GET, &[174]),
    ]);
    v
}

fn off() -> Vec<Req> {
    vec![
        req(C, scmi::CLOCK_CONFIG_SET, &[174, 0, 0]),
        req(P, scmi::POWER_STATE_SET, &[0, 14, 0x4000_0000]),
        req(P, scmi::POWER_STATE_GET, &[14]),
    ]
}

/// Every request of `log` that changes the SM's state.
fn writes(log: &[Req]) -> Vec<Req> {
    let set = [
        (P, scmi::POWER_STATE_SET),
        (F, scmi::PERF_LEVEL_SET),
        (F, scmi::PERF_LIMITS_SET),
        (C, scmi::CLOCK_CONFIG_SET),
        (C, scmi::CLOCK_RATE_SET),
    ];
    log.iter().filter(|r| set.contains(&(r.0, r.1))).cloned().collect()
}

#[test]
fn the_ids_are_the_sms_and_linuxs() {
    // dev_sm_power.h DEV_SM_PD_GPU 14; dev_sm_perf.h DEV_SM_PERF_GPU 9;
    // dev_sm_clock.h DEV_SM_CLK_GPUAPB 83, DEV_SM_CLK_GPU 84,
    // DEV_SM_CLK_GPU_CGC 174. Linux imx95-clock.h: CCM_NUM_CLK_SRC 41,
    // GPUAPB +42, GPU +43, GPU_CGC +123+10; imx95-power.h PD 14, PERF 9.
    assert_eq!((PD_GPU, PERF_GPU), (14, 9));
    assert_eq!((CLK_GPUAPB, CLK_GPU, CLK_GPU_CGC), (41 + 42, 41 + 43, 41 + 123 + 10));
    assert_eq!(gpu_power::OPP_HZ, [500_000_000, 800_000_000, 1_000_000_000]);
}

#[test]
fn power_on_sends_power_then_level_then_gate_with_the_verdin_ids() {
    let mut sm = Sm::verdin();
    let up = gpu_power::power_on(&mut sm, &Config::VERDIN).unwrap();
    assert_eq!(sm.log, on(2));
    assert_eq!(up.level, Level { index: 2, khz: 800_000 });
    assert_eq!((up.requested_hz, up.rate_hz), (800_000_000, 800_000_000));
    assert!(sm.pd_on && sm.clk_on && sm.level == 2);
}

#[test]
fn power_state_set_is_the_den0056_bytes() {
    struct Bytes(Vec<Vec<u8>>, Sm);
    impl Transport for Bytes {
        fn transfer(&mut self, tx: &[u8], rx: &mut [u8]) -> Result<usize, i32> {
            self.0.push(tx.to_vec());
            self.1.transfer(tx, rx)
        }
    }
    let mut t = Bytes(Vec::new(), Sm::verdin());
    gpu_power::power_on(&mut t, &Config::VERDIN).unwrap();
    // header: message 4 | protocol 0x11 << 10, token 0; flags 0, domain 14, state 0
    assert_eq!(t.0[4], [0x04, 0x44, 0, 0, 0, 0, 0, 0, 14, 0, 0, 0, 0, 0, 0, 0]);
    // PERFORMANCE_LEVEL_SET: message 7 | protocol 0x13 << 10; domain 9, level 2
    assert_eq!(t.0[6], [0x07, 0x4c, 0, 0, 9, 0, 0, 0, 2, 0, 0, 0]);
    // CLOCK_CONFIG_SET: message 7 | protocol 0x14 << 10; clock 174, enable, ext 0
    assert_eq!(t.0[8], [0x07, 0x50, 0, 0, 174, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0]);
}

#[test]
fn power_off_sends_gate_then_domain_and_reads_it_off() {
    let mut sm = Sm::verdin();
    gpu_power::power_on(&mut sm, &Config::VERDIN).unwrap();
    sm.log.clear();
    gpu_power::power_off(&mut sm).unwrap();
    assert_eq!(sm.log, off());
    assert!(!sm.pd_on && !sm.clk_on);
    // the level is left as it was
    assert_eq!(sm.level, 2);
}

#[test]
fn neither_the_gpu_root_nor_gpuapb_nor_a_clock_rate_is_ever_sent() {
    let mut sm = Sm::verdin();
    gpu_power::power_on(&mut sm, &Config { rate_hz: 1_000_000_000 }).unwrap();
    gpu_power::power_off(&mut sm).unwrap();
    for (p, m, w) in &sm.log {
        assert_ne!((*p, *m), (C, scmi::CLOCK_RATE_SET));
        if *p == C && !w.is_empty() {
            assert_eq!(w[0], CLK_GPU_CGC, "{:?}", (p, m, w));
        }
    }
}

#[test]
fn a_denied_power_domain_is_named_and_not_retried() {
    let mut sm = Sm::verdin();
    sm.pd_perm.clear();
    let e = gpu_power::power_on(&mut sm, &Config::VERDIN).unwrap_err();
    assert_eq!(e, Error::Refused { step: Step::PowerOn, status: Status::Denied });
    assert_eq!(format!("{e}"), "POWER_STATE_SET (0x11/0x04): SCMI DENIED (-3)");
    // the reads, then the one refused request, and nothing after it
    let mut want = reads();
    want.push(req(P, scmi::POWER_STATE_SET, &[0, 14, 0]));
    assert_eq!(sm.log, want);
    assert!(!sm.pd_on);
}

#[test]
fn a_denied_level_stops_before_the_clock_and_leaves_the_domain_on() {
    let mut sm = Sm::verdin();
    // SET is not enough for a level (PerformanceLevelSet wants PRIV)
    sm.perf_perm.insert(PERF_GPU, PERM_SET);
    let e = gpu_power::power_on(&mut sm, &Config::VERDIN).unwrap_err();
    assert_eq!(e, Error::Refused { step: Step::PerfLevelSet, status: Status::Denied });
    assert_eq!(format!("{e}"), "PERFORMANCE_LEVEL_SET (0x13/0x07): SCMI DENIED (-3)");
    assert_eq!(sm.log, on(2)[..7].to_vec());
    assert_eq!(writes(&sm.log).iter().filter(|r| r.1 == scmi::PERF_LEVEL_SET).count(), 1);
    assert!(sm.pd_on && !sm.clk_on);
}

#[test]
fn a_denied_gate_is_named_and_not_retried() {
    let mut sm = Sm::verdin();
    // as AP-NS stands with GPUAPB: no permission at all
    sm.clk_perm.clear();
    let e = gpu_power::power_on(&mut sm, &Config::VERDIN).unwrap_err();
    assert_eq!(e, Error::Refused { step: Step::ClockEnable, status: Status::Denied });
    assert_eq!(format!("{e}"), "CLOCK_CONFIG_SET (0x14/0x07): SCMI DENIED (-3)");
    assert_eq!(sm.log, on(2)[..9].to_vec());
    assert!(!sm.clk_on);
}

#[test]
fn a_denied_power_off_is_named_and_not_retried() {
    let mut sm = Sm::verdin();
    gpu_power::power_on(&mut sm, &Config::VERDIN).unwrap();
    sm.log.clear();
    sm.pd_perm.clear();
    let e = gpu_power::power_off(&mut sm).unwrap_err();
    assert_eq!(e, Error::Refused { step: Step::PowerOff, status: Status::Denied });
    assert_eq!(sm.log, off()[..2].to_vec());
}

#[test]
fn a_rate_between_levels_is_rounded_down_and_the_sms_rounding_is_kept() {
    let mut sm = Sm::verdin();
    // the root's PLL makes a little less than the level's 800 MHz
    sm.rate_offset = -8;
    let up = gpu_power::power_on(&mut sm, &Config { rate_hz: 900_000_000 }).unwrap();
    assert_eq!(up.level, Level { index: 2, khz: 800_000 });
    assert_eq!((up.requested_hz, up.rate_hz), (900_000_000, 799_999_992));
    assert_eq!(sm.log, on(2));

    let mut sm = Sm::verdin();
    let up = gpu_power::power_on(&mut sm, &Config { rate_hz: 1_000_000_000 }).unwrap();
    assert_eq!((up.level.index, up.rate_hz), (3, 1_000_000_000));
    let mut sm = Sm::verdin();
    let up = gpu_power::power_on(&mut sm, &Config { rate_hz: 799_999_999 }).unwrap();
    assert_eq!((up.level.index, up.rate_hz), (1, 500_000_000));
}

#[test]
fn a_clock_above_the_level_or_at_zero_is_refused() {
    let mut sm = Sm::verdin();
    sm.rate_offset = 1;
    let e = gpu_power::power_on(&mut sm, &Config::VERDIN).unwrap_err();
    assert_eq!(e, Error::Rate { level_hz: 800_000_000, got_hz: 800_000_001 });
    assert_eq!(sm.log, on(2));

    let mut sm = Sm::verdin();
    sm.rate_offset = -800_000_000;
    let e = gpu_power::power_on(&mut sm, &Config::VERDIN).unwrap_err();
    assert_eq!(e, Error::Rate { level_hz: 800_000_000, got_hz: 0 });
}

#[test]
fn a_rate_below_the_lowest_running_level_writes_nothing() {
    let mut sm = Sm::verdin();
    // 24 MHz is a level, but the parking one: never chosen
    let e = gpu_power::power_on(&mut sm, &Config { rate_hz: 400_000_000 }).unwrap_err();
    assert_eq!(e, Error::NoLevel { rate_hz: 400_000_000 });
    assert_eq!(sm.log, reads()[..3].to_vec());
    assert!(writes(&sm.log).is_empty());
}

#[test]
fn levels_over_several_replies_are_all_read() {
    let mut sm = Sm::verdin();
    sm.page = 3;
    gpu_power::power_on(&mut sm, &Config { rate_hz: 1_000_000_000 }).unwrap();
    let describes: Vec<_> = sm.log.iter().filter(|r| (r.0, r.1) == (F, scmi::PERF_DESCRIBE_LEVELS)).collect();
    assert_eq!(describes, [&req(F, scmi::PERF_DESCRIBE_LEVELS, &[9, 0]), &req(F, scmi::PERF_DESCRIBE_LEVELS, &[9, 3])]);
    assert_eq!(sm.level, 3);
}

#[test]
fn an_empty_or_short_level_reply_is_refused_before_any_write() {
    struct Empty(Sm);
    impl Transport for Empty {
        fn transfer(&mut self, tx: &[u8], rx: &mut [u8]) -> Result<usize, i32> {
            let n = self.0.transfer(tx, rx)?;
            if scmi::word(tx, 0) & 0x3ffff == (F as u32) << 10 | scmi::PERF_DESCRIBE_LEVELS as u32 {
                // two levels announced, one carried
                rx[8..12].copy_from_slice(&2u32.to_le_bytes());
                return Ok(8 + 4 + 20);
            }
            Ok(n)
        }
    }
    let mut t = Empty(Sm::verdin());
    let e = gpu_power::power_on(&mut t, &Config::VERDIN).unwrap_err();
    assert_eq!(e, Error::Levels { returned: 2, remaining: 0 });
    assert!(writes(&t.0.log).is_empty());
}

#[test]
fn a_domain_another_agent_holds_on_fails_the_off_read_back() {
    let mut sm = Sm::verdin();
    gpu_power::power_on(&mut sm, &Config::VERDIN).unwrap();
    sm.pd_held = true;
    let e = gpu_power::power_off(&mut sm).unwrap_err();
    assert_eq!(e, Error::Readback { step: Step::PowerOffCheck, want: scmi::POWER_OFF, got: scmi::POWER_ON });
}

#[test]
fn a_wrong_protocol_version_stops_before_any_write() {
    struct Old(Sm);
    impl Transport for Old {
        fn transfer(&mut self, tx: &[u8], rx: &mut [u8]) -> Result<usize, i32> {
            let n = self.0.transfer(tx, rx)?;
            if scmi::word(tx, 0) & 0x3ffff == (F as u32) << 10 {
                // a version 3 performance protocol: no level indexing
                rx[8..12].copy_from_slice(&0x30000u32.to_le_bytes());
            }
            Ok(n)
        }
    }
    let mut t = Old(Sm::verdin());
    let e = gpu_power::power_on(&mut t, &Config::VERDIN).unwrap_err();
    assert_eq!(e, Error::Version { step: Step::PerfVersion, version: 0x30000 });
    assert!(writes(&t.0.log).is_empty());
}

#[test]
fn a_transport_failure_and_a_foreign_reply_are_typed() {
    struct Dead;
    impl Transport for Dead {
        fn transfer(&mut self, _: &[u8], _: &mut [u8]) -> Result<usize, i32> {
            Err(-110)
        }
    }
    let e = gpu_power::power_on(&mut Dead, &Config::VERDIN).unwrap_err();
    assert_eq!(e, Error::Transport { step: Step::PowerVersion, code: -110 });
    assert_eq!(format!("{e}"), "PROTOCOL_VERSION (0x11/0x00): transport error -110");

    struct Foreign;
    impl Transport for Foreign {
        fn transfer(&mut self, _: &[u8], rx: &mut [u8]) -> Result<usize, i32> {
            rx[..12].copy_from_slice(&[0xff, 0xff, 0, 0, 0, 0, 0, 0, 0, 0, 3, 0]);
            Ok(12)
        }
    }
    let e = gpu_power::power_on(&mut Foreign, &Config::VERDIN).unwrap_err();
    assert_eq!(e, Error::Echo { step: Step::PowerVersion, sent: 0x4400, got: 0xffff });
}

#[test]
fn pick_level_never_parks() {
    let l = [Level { index: 0, khz: 24_000 }, Level { index: 1, khz: 500_000 }, Level { index: 2, khz: 800_000 }];
    assert_eq!(pick_level(&l, 24_000_000), None);
    assert_eq!(pick_level(&l, 500_000_000), Some(l[1]));
    assert_eq!(pick_level(&l, u64::MAX), Some(l[2]));
}
