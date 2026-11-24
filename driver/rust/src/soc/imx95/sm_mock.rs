// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! A mock System Manager for the host tests of [`super::gpu_power`],
//! [`super::display_power`] and [`super::dsi_power`] (tests only): a host model of the part of NXP's
//! SM those sequences reach, with the permission checks of its SCMI handlers
//! (rpc_scmi_power.c `PowerStateSet`: SET; rpc_scmi_perf.c
//! `PerformanceLevelSet`: PRIV; rpc_scmi_clock.c `ClockConfigSet`: SET,
//! `ClockRateSet` and `ClockParentSet`: EXCLUSIVE, and INVALID_PARAMETERS
//! for a rate on a gate, dev_sm_clock.c `DEV_SM_ClockRateSet`) and the
//! AP-NS agent's permissions of Toradex's verdin-imx95 configuration
//! (`configs/verdin-imx95/config_scmi.h`: PD 3, 13 and 14, perf 9, 11
//! and 12, clocks 18, 19, 37, 38, 66, 67, 68, 80, 155 and 174 at ALL;
//! TPM5 154 at ALL too, which the configuration as the design note read it
//! does not list: Linux runs the X12 backlight on TPM5, and the probe
//! imx95_lvds_ids reads the grant on the board; CAMAPB 62,
//! DISPAPB/DISPAXI/DISPOCRAM 75, 76, 78 and GPUAPB/GPU 83, 84 not
//! granted), and NXP's BBM vendor protocol 0x81 for the record
//! ([`super::bbm`]: rpc_scmi_bbm.c `BbmGprSet` EXCLUSIVE, `BbmGprGet` GET,
//! NOT_FOUND from `SM_NUM_GPR` = 8 on; AP-NS holds GPR4..7 at ALL,
//! config_scmi.h:453-456, GPR0..3 not granted). Every request is recorded as
//! (protocol, message, payload words).
//!
//! Rates, as the SM's clock code makes them (dev_sm_clock.c,
//! fsl_clock.c), restated here and not taken from the code under test:
//! - a gate (174) runs at its performance domain's level (`rate_offset`
//!   models a PLL's rounding);
//! - a PLL VCO (18, 37) takes any rate in 2.52..4.992 GHz
//!   (MIMX95_elec_spec.h `ES_MIN_HZ_PLLVCO`, `ES_MAX_HZ_PLLVCO`), else
//!   OUT_OF_RANGE;
//! - a PLL output (19, 38) is its VCO divided by an integer, rounded down
//!   (the SCMI default; flags 0);
//! - a clock root (80) is its parent divided by an integer 1..=256,
//!   rounded down, at most 350 MHz (`ES_MAX_HZ_DISP1PIX`); its parents are
//!   OSC24M (2), AUDIOPLL1 (15) and VIDEOPLL1 (19) (fsl_clock.c
//!   `CLOCK_ROOT_DISP1PIX` mux table), any other is INVALID_PARAMETERS;
//! - the MIPI PHY roots (66, 67, 68) run at their parent's rate (divider
//!   1), their parents OSC24M (2, the reset mux), AUDIOPLL1 (15) and
//!   VIDEOPLL1 (19) (fsl_clock.c:264-277); TPM6 (155) and TPM5 (154) at
//!   24 MHz, their reset parent the oscillator (:689).

use super::sm::TOKEN;
use crate::firmware::scmi::{self, Transport};
use std::collections::{BTreeMap, BTreeSet};

/// SM permission levels (rpc_scmi_config.h).
pub const PERM_NONE: u8 = 0;
pub const PERM_GET: u8 = 1;
pub const PERM_SET: u8 = 3;
pub const PERM_PRIV: u8 = 4;
pub const PERM_EXCLUSIVE: u8 = 5;
pub const PERM_ALL: u8 = 255;

use scmi::{CLOCK_PARENT_GET, CLOCK_PARENT_SET};

/// One request: protocol, message, payload words.
pub type Req = (u8, u8, Vec<u32>);

pub const P: u8 = scmi::PROTO_POWER;
pub const F: u8 = scmi::PROTO_PERF;
pub const C: u8 = scmi::PROTO_CLOCK;
pub const B: u8 = scmi::PROTO_BASE;
pub const BBM: u8 = super::bbm::PROTO_BBM;

pub fn req(p: u8, m: u8, w: &[u32]) -> Req {
    (p, m, w.to_vec())
}

/// The ids the mock gives a meaning (dev_sm_power.h, dev_sm_perf.h,
/// dev_sm_clock.h), restated.
const PD_DISPLAY: u32 = 13;
const PD_GPU: u32 = 14;
const PERF_GPU: u32 = 9;
const PERF_DISP: u32 = 12;
const CLK_OSC24M: u32 = 2;
const CLK_AUDIOPLL1: u32 = 15;
const CLK_VIDEOPLL1_VCO: u32 = 18;
const CLK_VIDEOPLL1: u32 = 19;
const CLK_LDBPLL_VCO: u32 = 37;
const CLK_LDBPLL: u32 = 38;
const CLK_TPM5: u32 = 154;
const CLK_DISP1PIX: u32 = 80;
const CLK_GPU_CGC: u32 = 174;
const PD_CAMERA: u32 = 3;
const PERF_CAM: u32 = 11;
const CLK_CAMAPB: u32 = 62;
const CLK_MIPIPHYCFG: u32 = 66;
const CLK_MIPIPHYPLLBYPASS: u32 = 67;
const CLK_MIPIPHYPLLREF: u32 = 68;
const CLK_TPM6: u32 = 155;
const MIPI_ROOTS: [u32; 3] = [CLK_MIPIPHYCFG, CLK_MIPIPHYPLLBYPASS, CLK_MIPIPHYPLLREF];

pub struct Sm {
    pub clk_perm: BTreeMap<u32, u8>,
    pub pd_perm: BTreeMap<u32, u8>,
    pub perf_perm: BTreeMap<u32, u8>,
    /// (kHz, index) per performance domain, in index order.
    pub levels: BTreeMap<u32, Vec<(u32, u32)>>,
    /// Levels per DESCRIBE_LEVELS reply.
    pub page: usize,
    /// Power domains on.
    pub pd_on: BTreeSet<u32>,
    /// Domains another agent's vote keeps on.
    pub pd_held: BTreeSet<u32>,
    /// The level of each performance domain (PRK, 0, unless set).
    pub level: BTreeMap<u32, u32>,
    /// Clocks enabled.
    pub clk_on: BTreeSet<u32>,
    /// What the GPU clock root makes of a level's rate (a PLL's rounding).
    pub rate_offset: i64,
    /// Rates of the clocks with a rate of their own (18, 19, 80), Hz.
    pub rate: BTreeMap<u32, u64>,
    /// Parents of the roots.
    pub parent: BTreeMap<u32, u32>,
    /// The BBNSM's GPR0..7 (`DEV_SM_NUM_GPR`) and the agent's permission on
    /// each.
    pub gpr: [u32; 8],
    pub gpr_perm: BTreeMap<u32, u8>,
    pub log: Vec<Req>,
}

impl Sm {
    /// The Verdin iMX95's SM as the AP-NS agent sees it (module
    /// documentation); the GPU's levels PRK, LOW, NOM, ODV (dev_sm_perf.c:
    /// 24 MHz, 500, 800, 1000 MHz) and the display's (24 MHz,
    /// `ES_LOW_KHZ_DISP` 400 MHz, `ES_NOM_KHZ_DISP` 666.667 MHz,
    /// `ES_ODV_KHZ_DISP` 800 MHz, MIMX95_elec_spec.h:198-200). After boot
    /// every domain is off; the GPU's level is parked, the display's at
    /// overdrive (3): Toradex's board code starts every domain without a
    /// local PLL at `BOARD_PERF_LEVEL` = ODV (`brd_sm_tdx_common.c:107,163`,
    /// driver/docs/imx95-display.md section 1). VIDEOPLL1 is unset, DISP1PIX
    /// runs from the oscillator.
    pub fn verdin() -> Sm {
        Sm {
            clk_perm: BTreeMap::from([
                (CLK_GPU_CGC, PERM_ALL),
                (CLK_VIDEOPLL1_VCO, PERM_ALL),
                (CLK_VIDEOPLL1, PERM_ALL),
                (CLK_DISP1PIX, PERM_ALL),
                (CLK_MIPIPHYCFG, PERM_ALL),
                (CLK_MIPIPHYPLLBYPASS, PERM_ALL),
                (CLK_MIPIPHYPLLREF, PERM_ALL),
                (CLK_TPM6, PERM_ALL),
                (CLK_LDBPLL_VCO, PERM_ALL),
                (CLK_LDBPLL, PERM_ALL),
                (CLK_TPM5, PERM_ALL),
                (CLK_CAMAPB, PERM_NONE),
            ]),
            pd_perm: BTreeMap::from([(PD_GPU, PERM_ALL), (PD_DISPLAY, PERM_ALL), (PD_CAMERA, PERM_ALL)]),
            perf_perm: BTreeMap::from([(PERF_GPU, PERM_ALL), (PERF_DISP, PERM_ALL), (PERF_CAM, PERM_ALL)]),
            levels: BTreeMap::from([
                (PERF_GPU, vec![(24_000, 0), (500_000, 1), (800_000, 2), (1_000_000, 3)]),
                (PERF_DISP, vec![(24_000, 0), (400_000, 1), (666_667, 2), (800_000, 3)]),
                (PERF_CAM, vec![(24_000, 0), (400_000, 1), (666_667, 2), (800_000, 3)]),
            ]),
            page: 8,
            pd_on: BTreeSet::new(),
            pd_held: BTreeSet::new(),
            level: BTreeMap::from([(PERF_DISP, 3), (PERF_CAM, 3)]),
            clk_on: BTreeSet::new(),
            rate_offset: 0,
            rate: BTreeMap::from([
                (CLK_VIDEOPLL1_VCO, 0),
                (CLK_VIDEOPLL1, 0),
                (CLK_DISP1PIX, 24_000_000),
                (CLK_TPM6, 24_000_000),
                (CLK_TPM5, 24_000_000),
                (CLK_LDBPLL_VCO, 0),
                (CLK_LDBPLL, 0),
            ]),
            parent: BTreeMap::from([
                (CLK_DISP1PIX, CLK_OSC24M),
                (CLK_MIPIPHYCFG, CLK_OSC24M),
                (CLK_MIPIPHYPLLBYPASS, CLK_OSC24M),
                (CLK_MIPIPHYPLLREF, CLK_OSC24M),
            ]),
            gpr: [0; 8],
            gpr_perm: BTreeMap::from([(4, PERM_ALL), (5, PERM_ALL), (6, PERM_ALL), (7, PERM_ALL)]),
            log: Vec::new(),
        }
    }

    fn perm(map: &BTreeMap<u32, u8>, id: u32) -> u8 {
        *map.get(&id).unwrap_or(&PERM_NONE)
    }

    pub fn on(&self, pd: u32) -> bool {
        self.pd_on.contains(&pd)
    }

    pub fn level_of(&self, perf: u32) -> u32 {
        *self.level.get(&perf).unwrap_or(&0)
    }

    pub fn enabled(&self, clk: u32) -> bool {
        self.clk_on.contains(&clk)
    }

    /// The rate a clock runs at now, Hz.
    pub fn rate_of(&self, clk: u32) -> u64 {
        match clk {
            CLK_GPU_CGC => {
                let lv = self.level_of(PERF_GPU);
                let khz = self.levels[&PERF_GPU].iter().find(|l| l.1 == lv).map_or(0, |l| l.0);
                (khz as i64 * 1000 + self.rate_offset) as u64
            }
            CLK_DISP1PIX => {
                let p = self.parent[&CLK_DISP1PIX];
                let prate = match p {
                    CLK_OSC24M => 24_000_000,
                    _ => self.rate_of(p),
                };
                let want = *self.rate.get(&CLK_DISP1PIX).unwrap_or(&0);
                Self::root_rate(prate, want)
            }
            c if MIPI_ROOTS.contains(&c) => match self.parent[&c] {
                CLK_OSC24M => 24_000_000,
                p => self.rate_of(p),
            },
            _ => *self.rate.get(&clk).unwrap_or(&0),
        }
    }

    /// A root divider of 1..=256 on `parent`, rounded down to at most `want`
    /// (the fastest divider, then the parent itself when `want` is 0).
    fn root_rate(parent: u64, want: u64) -> u64 {
        if want == 0 || parent == 0 {
            return parent;
        }
        let div = parent.div_ceil(want).clamp(1, 256);
        parent / div
    }

    /// The SM's answer: status and return values.
    fn serve(&mut self, proto: u8, msg: u8, w: &[u32]) -> (i32, Vec<u32>) {
        const OK: i32 = scmi::STATUS_SUCCESS;
        match (proto, msg) {
            (P, scmi::PROTOCOL_VERSION) => (OK, vec![0x30001]),
            (F, scmi::PROTOCOL_VERSION) => (OK, vec![0x40000]),
            (C, scmi::PROTOCOL_VERSION) => (OK, vec![0x30000]),
            (BBM, scmi::PROTOCOL_VERSION) => (OK, vec![0x10000]),
            (BBM, scmi::PROTOCOL_ATTRIBUTES) => (OK, vec![self.gpr.len() as u32]),
            (BBM, super::bbm::BBM_GPR_SET) => {
                if w.len() < 2 {
                    return (scmi::STATUS_PROTOCOL_ERROR, vec![]);
                }
                if w[0] as usize >= self.gpr.len() {
                    return (scmi::STATUS_NOT_FOUND, vec![]);
                }
                if Self::perm(&self.gpr_perm, w[0]) < PERM_EXCLUSIVE {
                    return (scmi::STATUS_DENIED, vec![]);
                }
                self.gpr[w[0] as usize] = w[1];
                (OK, vec![])
            }
            (BBM, super::bbm::BBM_GPR_GET) => {
                if w.is_empty() {
                    return (scmi::STATUS_PROTOCOL_ERROR, vec![]);
                }
                if w[0] as usize >= self.gpr.len() {
                    return (scmi::STATUS_NOT_FOUND, vec![]);
                }
                if Self::perm(&self.gpr_perm, w[0]) < PERM_GET {
                    return (scmi::STATUS_DENIED, vec![]);
                }
                (OK, vec![self.gpr[w[0] as usize]])
            }
            (P, scmi::POWER_STATE_SET) => {
                if Self::perm(&self.pd_perm, w[1]) < PERM_SET {
                    return (scmi::STATUS_DENIED, vec![]);
                }
                if w[2] == scmi::POWER_ON || self.pd_held.contains(&w[1]) {
                    self.pd_on.insert(w[1]);
                } else {
                    self.pd_on.remove(&w[1]);
                }
                (OK, vec![])
            }
            (P, scmi::POWER_STATE_GET) => (OK, vec![if self.on(w[0]) { scmi::POWER_ON } else { scmi::POWER_OFF }]),
            (F, scmi::PERF_DESCRIBE_LEVELS) => {
                let Some(all) = self.levels.get(&w[0]) else {
                    return (scmi::STATUS_NOT_FOUND, vec![]);
                };
                let skip = w[1] as usize;
                let rest = &all[skip.min(all.len())..];
                let n = rest.len().min(self.page);
                let mut v = vec![n as u32 | ((rest.len() - n) as u32) << 16];
                for &(khz, index) in &rest[..n] {
                    v.extend([khz, 0, 1, khz, index]);
                }
                (OK, v)
            }
            (F, scmi::PERF_LEVEL_SET) => {
                if Self::perm(&self.perf_perm, w[0]) < PERM_PRIV {
                    return (scmi::STATUS_DENIED, vec![]);
                }
                if w[1] as usize >= self.levels.get(&w[0]).map_or(0, |l| l.len()) {
                    return (scmi::STATUS_INVALID_PARAMETERS, vec![]);
                }
                self.level.insert(w[0], w[1]);
                (OK, vec![])
            }
            (F, scmi::PERF_LEVEL_GET) => (OK, vec![self.level_of(w[0])]),
            (C, scmi::CLOCK_CONFIG_SET) => {
                if Self::perm(&self.clk_perm, w[0]) < PERM_SET {
                    return (scmi::STATUS_DENIED, vec![]);
                }
                if w[1] & 3 == scmi::CLOCK_ENABLE {
                    self.clk_on.insert(w[0]);
                } else {
                    self.clk_on.remove(&w[0]);
                }
                (OK, vec![])
            }
            (C, scmi::CLOCK_RATE_SET) => {
                let (flags, clk, hz) = (w[0], w[1], w[2] as u64 | (w[3] as u64) << 32);
                if Self::perm(&self.clk_perm, clk) < PERM_EXCLUSIVE {
                    return (scmi::STATUS_DENIED, vec![]);
                }
                assert_eq!(flags, 0, "a synchronous, round-down request");
                match clk {
                    CLK_VIDEOPLL1_VCO | CLK_LDBPLL_VCO => {
                        if !(2_520_000_000..=4_992_000_000).contains(&hz) {
                            return (scmi::STATUS_OUT_OF_RANGE, vec![]);
                        }
                        self.rate.insert(clk, hz);
                    }
                    CLK_VIDEOPLL1 | CLK_LDBPLL => {
                        let vco = self.rate[&(clk - 1)];
                        if vco == 0 || hz == 0 {
                            return (scmi::STATUS_INVALID_PARAMETERS, vec![]);
                        }
                        self.rate.insert(clk, vco / vco.div_ceil(hz));
                    }
                    CLK_DISP1PIX => {
                        if hz > 350_000_000 {
                            return (scmi::STATUS_OUT_OF_RANGE, vec![]);
                        }
                        self.rate.insert(clk, hz);
                    }
                    // A gate has no rate of its own (DEV_SM_ClockRateSet).
                    _ => return (scmi::STATUS_INVALID_PARAMETERS, vec![]),
                }
                (OK, vec![])
            }
            (C, scmi::CLOCK_RATE_GET) => {
                let r = self.rate_of(w[0]);
                (OK, vec![r as u32, (r >> 32) as u32])
            }
            (C, CLOCK_PARENT_SET) => {
                if Self::perm(&self.clk_perm, w[0]) < PERM_EXCLUSIVE {
                    return (scmi::STATUS_DENIED, vec![]);
                }
                if !(w[0] == CLK_DISP1PIX || MIPI_ROOTS.contains(&w[0]))
                    || ![CLK_OSC24M, CLK_AUDIOPLL1, CLK_VIDEOPLL1].contains(&w[1])
                {
                    return (scmi::STATUS_INVALID_PARAMETERS, vec![]);
                }
                self.parent.insert(w[0], w[1]);
                (OK, vec![])
            }
            (C, CLOCK_PARENT_GET) => match self.parent.get(&w[0]) {
                Some(&p) => (OK, vec![p]),
                None => (scmi::STATUS_NOT_SUPPORTED, vec![]),
            },
            // The base protocol's discovery, for the survey (`super::survey`):
            // version 2.1, the NXP SM's vendor and the AP-NS agent (2); the
            // protocol list is the three this mock serves, two to a reply,
            // so the survey asks again with a skip.
            (B, scmi::PROTOCOL_VERSION) => (OK, vec![0x20001]),
            (B, scmi::PROTOCOL_ATTRIBUTES) => (OK, vec![3 | 6 << 8]),
            (B, scmi::BASE_DISCOVER_VENDOR) => (OK, name16(b"NXP")),
            (B, 0x7) if w[0] == 0xffff_ffff => {
                let mut v = vec![2];
                v.extend(name16(b"AP-NS"));
                (OK, v)
            }
            (B, 0x6) => {
                let all = [P, F, C];
                let rest = &all[(w[0] as usize).min(all.len())..];
                let k = rest.len().min(2);
                let mut packed = 0u32;
                for (i, p) in rest[..k].iter().enumerate() {
                    packed |= u32::from(*p) << (8 * i);
                }
                (OK, if k == 0 { vec![0] } else { vec![k as u32, packed] })
            }
            // CLOCK_GET_PERMISSIONS: state from SET, parent and rate from
            // EXCLUSIVE (the mock's reading of the handlers' checks above).
            (C, 0xf) => {
                let p = Self::perm(&self.clk_perm, w[0]);
                let mut v = 0u32;
                if p >= PERM_SET {
                    v |= 1 << 31;
                }
                if p >= PERM_EXCLUSIVE {
                    v |= 1 << 30 | 1 << 29;
                }
                (OK, vec![v])
            }
            _ => (scmi::STATUS_NOT_SUPPORTED, vec![]),
        }
    }
}

/// A 16-byte SCMI name as four words, NUL-padded.
fn name16(s: &[u8]) -> Vec<u32> {
    let mut b = [0u8; 16];
    b[..s.len()].copy_from_slice(s);
    (0..4).map(|i| u32::from_le_bytes([b[4 * i], b[4 * i + 1], b[4 * i + 2], b[4 * i + 3]])).collect()
}

impl Transport for Sm {
    fn transfer(&mut self, tx: &[u8], rx: &mut [u8]) -> Result<usize, i32> {
        assert!(tx.len() >= 4 && tx.len().is_multiple_of(4), "a request is whole words");
        let hdr = scmi::word(tx, 0);
        assert_eq!(hdr >> 8 & 3, 0, "a command");
        assert_eq!(hdr >> 18 & 0x3ff, TOKEN as u32, "the token");
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

/// Every request of `log` that changes the SM's state.
pub fn writes(log: &[Req]) -> Vec<Req> {
    let set = [
        (P, scmi::POWER_STATE_SET),
        (F, scmi::PERF_LEVEL_SET),
        (F, scmi::PERF_LIMITS_SET),
        (C, scmi::CLOCK_CONFIG_SET),
        (C, scmi::CLOCK_RATE_SET),
        (C, CLOCK_PARENT_SET),
        (BBM, super::bbm::BBM_GPR_SET),
    ];
    log.iter().filter(|r| set.contains(&(r.0, r.1))).cloned().collect()
}

/// The DSI side's power-on ([`super::dsi_power::power_on`] with
/// `Config::VERDIN`: the CAM level read, the PHY clocks' parents set).
pub fn dsi_on() -> Vec<Req> {
    vec![
        req(P, scmi::PROTOCOL_VERSION, &[]),
        req(F, scmi::PROTOCOL_VERSION, &[]),
        req(C, scmi::PROTOCOL_VERSION, &[]),
        req(P, scmi::POWER_STATE_SET, &[0, 3, 0]),
        req(P, scmi::POWER_STATE_GET, &[3]),
        req(F, scmi::PERF_LEVEL_GET, &[11]),
        req(C, CLOCK_PARENT_SET, &[66, 2]),
        req(C, CLOCK_PARENT_GET, &[66]),
        req(C, CLOCK_PARENT_SET, &[68, 2]),
        req(C, CLOCK_PARENT_GET, &[68]),
        req(C, CLOCK_PARENT_SET, &[67, 19]),
        req(C, CLOCK_PARENT_GET, &[67]),
        req(C, scmi::CLOCK_CONFIG_SET, &[66, 1, 0]),
        req(C, scmi::CLOCK_RATE_GET, &[66]),
        req(C, scmi::CLOCK_CONFIG_SET, &[68, 1, 0]),
        req(C, scmi::CLOCK_RATE_GET, &[68]),
        req(C, scmi::CLOCK_CONFIG_SET, &[155, 1, 0]),
        req(C, scmi::CLOCK_RATE_GET, &[155]),
    ]
}

/// The DSI side's power-off.
pub fn dsi_off() -> Vec<Req> {
    vec![
        req(C, scmi::CLOCK_CONFIG_SET, &[155, 0, 0]),
        req(C, scmi::CLOCK_CONFIG_SET, &[68, 0, 0]),
        req(C, scmi::CLOCK_CONFIG_SET, &[66, 0, 0]),
        req(P, scmi::POWER_STATE_SET, &[0, 3, 0x4000_0000]),
        req(P, scmi::POWER_STATE_GET, &[3]),
    ]
}
