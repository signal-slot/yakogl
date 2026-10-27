// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! PU power domains of the i.MX 8M Plus General Power Controller (GPC).
//!
//! Written from the i.MX 8M Plus Applications Processor Reference Manual,
//! IMX8MPRM Rev. 3, 08/2024 ("RM" below), chapter 5.2; register names and
//! bits from hal_nxp `PERI_GPC.h` / `PERI_GPC_PGC.h` ([`super::regs`]).
//!
//! The software power-up of a PU domain, RM 5.2.9.4 "Example Code 4" and
//! 5.2.8 "Domain control for PGCs" rule 2:
//! 1. map the domain to the A53 (`PGC_CPU_A53_MAPPING`, 5.2.10.72): a
//!    software trigger only takes effect for a PGC mapped to the CPU that
//!    writes it;
//! 2. set its bit in `PU_PGC_SW_PUP_REQ` (5.2.10.37) and wait for it to
//!    self-clear (5.2.9.4: "wait software power up request self clear");
//! 3. read `A53_PU_PGC_PUP_STATUS0..2` (5.2.10.45): a 1 means the trigger
//!    failed (domain control, a power-up already in progress, the time-slot
//!    controller busy);
//! 4. clear the PGC's `PCR` (5.2.11.9: 0 = "Do not switch off power even if
//!    pdn_req is asserted"), after the power-up has completed (the same
//!    section: PCR must not change between pdn_req and the end of the
//!    power-up), and clear `PSR` (5.2.11.12: "Software should clear this bit
//!    after power up").
//!
//! A domain behind an ADB400 bridge (MEDIAMIX, GPUMIX) has a handshake in
//! `PU_PWRHSK` (5.2.10.57): the request bit is an active-low power-down
//! request ("Write 0 and wait according ackn before power down") and the
//! acknowledge reads 0 once the bridge is down. For power-up the RM states
//! neither the order nor that the acknowledge returns to 1; this module
//! raises the request after the domain is up and lets the caller wait for
//! the acknowledge once the bridge's clocks run ([`adb_wait_up`]) -- the
//! order the replaced driver ran on this board.
//!
//! Power-down is the reverse (RM 5.2.9.4, second half): ADB request to 0 and
//! wait for the acknowledge to read 0, `PCR` to 1, `PU_PGC_SW_PDN_REQ`
//! (5.2.10.40) and wait for it to self-clear, `A53_PU_PGC_PDN_STATUS0..2`
//! (5.2.10.50).
//!
//! The waits are bounded ([`POLL_US`]): the RM gives the PGC sequence in
//! counts of its 66 MHz ipg clock (5.2.11.10 SW / SW2ISO, 5.2.11.12
//! PUP_CLK_DIV_SEL), at reset under 5000 counts, i.e. below 100 us; a
//! request that has not completed after ten times that is reported, never
//! waited on forever.
//!
//! The same module serves the GPU domains (`GPU_SHARE_LOGIC`, `GPU_2D`,
//! `GPU_3D` and the GPUMIX ADB400) for the Vivante bring-up
//! (gpu::vivante::power).

use super::regs::{gpc, gpc_pgc};
use crate::env::Env;
use crate::errno::{EIO, ETIMEDOUT};
use crate::mmio::Mmio;

/// The ADB400 handshake bits of a domain in `PU_PWRHSK`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Adb {
    /// The active-low power-down request (write 1: the bridge may be up).
    pub req: u32,
    /// The active-low power-down acknowledge.
    pub ack: u32,
}

/// One PU power domain.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PuDomain {
    pub name: &'static str,
    /// Its bit in `PU_PGC_SW_PUP_REQ` / `_PDN_REQ` and their status
    /// registers (the same position in all of them).
    pub trigger: u32,
    /// Its bit in `PGC_CPU_A53_MAPPING`.
    pub a53_map: u32,
    /// Its index in the PU PGC array (`GPC_PGC_CTRL[n]`, 0xB00 + 0x40 n).
    pub pgc: u32,
    pub adb: Option<Adb>,
}

/// MEDIAMIX: PGC at 0xD80 (RM 5.2.10 memory-map list "PGC for MEDIMIX"),
/// i.e. index 10; trigger bit 10 (5.2.10.37), mapping bit 12 (5.2.10.72),
/// ADB400 request bit 14 / acknowledge bit 30 (5.2.10.57).
pub const MEDIAMIX: PuDomain = PuDomain {
    name: "mediamix",
    trigger: gpc::pu_pgc_sw_pup_req::MEDIMIX_SW_PUP_REQ_MASK,
    a53_map: gpc::pgc_cpu_a53_mapping::MEDIMIX_DOMAIN_MASK,
    pgc: 10,
    adb: Some(Adb {
        req: gpc::pu_pwrhsk::GPC_MEDIAMIX_NOC_ADBS_PWRDNREQN_MASK,
        ack: gpc::pu_pwrhsk::GPC_MEDIAMIX_NOC_ADBS_PWRDNACKN_MASK,
    }),
};

/// MIPI_PHY1: PGC at 0xB00 (index 0), trigger bit 0, mapping bit 2; no
/// ADB400.
pub const MIPI_PHY1: PuDomain = PuDomain {
    name: "mipi-phy1",
    trigger: gpc::pu_pgc_sw_pup_req::MIPI_PHY1_SW_PUP_REQ_MASK,
    a53_map: gpc::pgc_cpu_a53_mapping::MIPI_PHY1_DOMAIN_MASK,
    pgc: 0,
    adb: None,
};

/// GPU share logic: PGC at 0xCC0 (index 7), trigger bit 7, mapping bit 9;
/// the GPUMIX ADB400, request bit 9 / acknowledge bit 25.
pub const GPU_SHARE_LOGIC: PuDomain = PuDomain {
    name: "gpu-share-logic",
    trigger: gpc::pu_pgc_sw_pup_req::GPU_SHARE_LOGIC_SW_PUP_REQ_MASK,
    a53_map: gpc::pgc_cpu_a53_mapping::GPU_SHARE_LOGIC_DOMAIN_MASK,
    pgc: 7,
    adb: Some(Adb {
        req: gpc::pu_pwrhsk::GPC_GPUMIX_NOC_ADBS_PWRDNREQN_MASK,
        ack: gpc::pu_pwrhsk::GPC_GPUMIX_NOC_ADBS_PWRDNACKN_MASK,
    }),
};

/// GPU2D: PGC at 0xC80 (index 6), trigger bit 6, mapping bit 8.
pub const GPU_2D: PuDomain = PuDomain {
    name: "gpu-2d",
    trigger: gpc::pu_pgc_sw_pup_req::GPU_2D_SW_PUP_REQ_MASK,
    a53_map: gpc::pgc_cpu_a53_mapping::GPU_2D_DOMAIN_MASK,
    pgc: 6,
    adb: None,
};

/// GPU3D: PGC at 0xD40 (index 9), trigger bit 9, mapping bit 11.
pub const GPU_3D: PuDomain = PuDomain {
    name: "gpu-3d",
    trigger: gpc::pu_pgc_sw_pup_req::GPU_3D_SW_PUP_REQ_MASK,
    a53_map: gpc::pgc_cpu_a53_mapping::GPU3D_DOMAIN_MASK,
    pgc: 9,
    adb: None,
};

/// The bound on every wait, in microseconds (see the module text).
pub const POLL_US: u32 = 1000;

/// Why a GPC operation failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fail {
    /// The power-up trigger did not self-clear.
    PupTimeout,
    /// The power-up trigger cleared but `A53_PU_PGC_PUP_STATUSn` says it
    /// failed; the value is which of STATUS0..2 (bit n) reported it.
    PupRefused(u32),
    /// The power-down trigger did not self-clear.
    PdnTimeout,
    /// The power-down trigger was refused (`A53_PU_PGC_PDN_STATUSn`).
    PdnRefused(u32),
    /// The ADB400 acknowledge did not follow the request.
    AdbTimeout,
}

impl Fail {
    pub fn errno(self) -> i32 {
        match self {
            Fail::PupTimeout | Fail::PdnTimeout | Fail::AdbTimeout => -ETIMEDOUT,
            Fail::PupRefused(_) | Fail::PdnRefused(_) => -EIO,
        }
    }
}

/// What [`power_up`] found and did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PowerUp {
    /// `PSR` read 1 before the power-up: the domain had been powered down.
    pub was_down: bool,
}

/// What the GPC says about a domain, read only.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DomainState {
    pub mapped: bool,
    /// No power-up trigger pending and none of STATUS0..2 reports a failed one.
    pub pup_clean: bool,
    pub pcr_clear: bool,
    pub psr: bool,
    pub adb_req: bool,
    pub adb_ack: bool,
}

fn poll(m: &impl Mmio, env: &impl Env, addr: usize, mask: u32, want: u32) -> bool {
    for _ in 0..POLL_US {
        if m.read32(addr) & mask == want {
            return true;
        }
        env.busy_wait_us(1);
    }
    m.read32(addr) & mask == want
}

fn set_bits(m: &impl Mmio, addr: usize, bits: u32) {
    let v = m.read32(addr);
    if v & bits != bits {
        m.write32(addr, v | bits);
    }
}

fn clear_bits(m: &impl Mmio, addr: usize, bits: u32) {
    let v = m.read32(addr);
    if v & bits != 0 {
        m.write32(addr, v & !bits);
    }
}

/// Which of the three status registers at `base_off` report `trigger`.
fn status_bits(m: &impl Mmio, gpc_base: usize, first: fn(u32) -> u32, trigger: u32) -> u32 {
    (0..3).filter(|&n| m.read32(gpc_base + first(n) as usize) & trigger != 0).fold(0, |a, n| a | 1 << n)
}

/// Read a domain's state; writes nothing.
pub fn state(m: &impl Mmio, gpc_base: usize, d: &PuDomain) -> DomainState {
    let map = m.read32(gpc_base + gpc::PGC_CPU_A53_MAPPING as usize);
    let pup = m.read32(gpc_base + gpc::PU_PGC_SW_PUP_REQ as usize);
    let refused = status_bits(m, gpc_base, gpc::a53_pu_pgc_pup_status, d.trigger);
    let ctrl = m.read32(gpc_base + gpc_pgc::pu_ctrl(d.pgc) as usize);
    let sr = m.read32(gpc_base + gpc_pgc::pu_sr(d.pgc) as usize);
    let (adb_req, adb_ack) = match d.adb {
        Some(a) => {
            let h = m.read32(gpc_base + gpc::PU_PWRHSK as usize);
            (h & a.req != 0, h & a.ack != 0)
        }
        None => (false, false),
    };
    DomainState {
        mapped: map & d.a53_map != 0,
        pup_clean: pup & d.trigger == 0 && refused == 0,
        pcr_clear: ctrl & gpc_pgc::pu_ctrl::PCR_MASK == 0,
        psr: sr & gpc_pgc::pu_sr::PSR_MASK != 0,
        adb_req,
        adb_ack,
    }
}

/// Power a domain up (steps 1-4 of the module text). The ADB400 request is
/// raised too when the domain has one; its acknowledge is [`adb_wait_up`]'s.
pub fn power_up(m: &impl Mmio, gpc_base: usize, env: &impl Env, d: &PuDomain) -> Result<PowerUp, Fail> {
    let sr = gpc_base + gpc_pgc::pu_sr(d.pgc) as usize;
    let was_down = m.read32(sr) & gpc_pgc::pu_sr::PSR_MASK != 0;
    set_bits(m, gpc_base + gpc::PGC_CPU_A53_MAPPING as usize, d.a53_map);
    let pup = gpc_base + gpc::PU_PGC_SW_PUP_REQ as usize;
    m.write32(pup, m.read32(pup) | d.trigger);
    if !poll(m, env, pup, d.trigger, 0) {
        return Err(Fail::PupTimeout);
    }
    let refused = status_bits(m, gpc_base, gpc::a53_pu_pgc_pup_status, d.trigger);
    if refused != 0 {
        return Err(Fail::PupRefused(refused));
    }
    clear_bits(m, gpc_base + gpc_pgc::pu_ctrl(d.pgc) as usize, gpc_pgc::pu_ctrl::PCR_MASK);
    if was_down {
        // write 1 to clear (5.2.11.12)
        m.write32(sr, gpc_pgc::pu_sr::PSR_MASK);
    }
    if let Some(a) = d.adb {
        set_bits(m, gpc_base + gpc::PU_PWRHSK as usize, a.req);
    }
    Ok(PowerUp { was_down })
}

/// Wait for the ADB400 acknowledge after [`power_up`] raised the request.
/// A domain without an ADB400 passes.
pub fn adb_wait_up(m: &impl Mmio, gpc_base: usize, env: &impl Env, d: &PuDomain) -> Result<(), Fail> {
    match d.adb {
        Some(a) if !poll(m, env, gpc_base + gpc::PU_PWRHSK as usize, a.ack, a.ack) => Err(Fail::AdbTimeout),
        _ => Ok(()),
    }
}

/// Power a domain down (the module text's last paragraph).
pub fn power_down(m: &impl Mmio, gpc_base: usize, env: &impl Env, d: &PuDomain) -> Result<(), Fail> {
    if let Some(a) = d.adb {
        let h = gpc_base + gpc::PU_PWRHSK as usize;
        clear_bits(m, h, a.req);
        if !poll(m, env, h, a.ack, 0) {
            return Err(Fail::AdbTimeout);
        }
    }
    set_bits(m, gpc_base + gpc::PGC_CPU_A53_MAPPING as usize, d.a53_map);
    set_bits(m, gpc_base + gpc_pgc::pu_ctrl(d.pgc) as usize, gpc_pgc::pu_ctrl::PCR_MASK);
    let pdn = gpc_base + gpc::PU_PGC_SW_PDN_REQ as usize;
    m.write32(pdn, m.read32(pdn) | d.trigger);
    if !poll(m, env, pdn, d.trigger, 0) {
        return Err(Fail::PdnTimeout);
    }
    let refused = status_bits(m, gpc_base, gpc::a53_pu_pgc_pdn_status, d.trigger);
    if refused != 0 {
        return Err(Fail::PdnRefused(refused));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Access, Faults, Model, TestEnv, GPC};

    fn gpc(m: &Model) -> usize {
        m.base(GPC)
    }

    #[test]
    fn layout_is_the_rm() {
        // 5.2.10 memory map: PUP 0xD8, PDN 0xE4, PWRHSK 0x190, A53 mapping
        // 0x1CC, PU PUP status 0x124 + 4n; PGC for MEDIMIX at 0xD80, its SR
        // at 0xD8C (GPC_PGC_PU10_SR); MIPI PHY1 at 0xB00.
        assert_eq!(gpc::PU_PGC_SW_PUP_REQ, 0xd8);
        assert_eq!(gpc::PU_PGC_SW_PDN_REQ, 0xe4);
        assert_eq!(gpc::PU_PWRHSK, 0x190);
        assert_eq!(gpc::PGC_CPU_A53_MAPPING, 0x1cc);
        assert_eq!(gpc::a53_pu_pgc_pup_status(2), 0x12c);
        assert_eq!(gpc_pgc::pu_ctrl(MEDIAMIX.pgc), 0xd80);
        assert_eq!(gpc_pgc::pu_sr(MEDIAMIX.pgc), 0xd8c);
        assert_eq!(gpc_pgc::pu_ctrl(MIPI_PHY1.pgc), 0xb00);
        assert_eq!(gpc_pgc::pu_ctrl(GPU_3D.pgc), 0xd40);
        assert_eq!((MEDIAMIX.trigger, MEDIAMIX.a53_map), (1 << 10, 1 << 12));
        assert_eq!(MEDIAMIX.adb, Some(Adb { req: 1 << 14, ack: 1 << 30 }));
    }

    #[test]
    fn power_up_maps_triggers_waits_for_the_self_clear_then_clears_pcr() {
        let m = Model::new();
        let e = TestEnv::default();
        m.poke(GPC, 0xd80, 1); // PCR set, as a power-down leaves it
        m.poke(GPC, 0xd8c, 1); // PSR: it was powered down
        let r = power_up(&m, gpc(&m), &e, &MEDIAMIX).unwrap();
        assert!(r.was_down);
        assert_eq!(m.powered() & MEDIAMIX.trigger, MEDIAMIX.trigger);
        let w = m.writes();
        let pos = |o| w.iter().position(|x| x.0 == GPC && x.1 == o).unwrap();
        assert!(pos(0x1cc) < pos(0xd8), "mapped before the trigger: {w:?}");
        assert!(pos(0xd8) < pos(0xd80), "PCR cleared after the power-up: {w:?}");
        assert_eq!(m.peek(GPC, 0xd80) & 1, 0);
        assert_eq!(m.peek(GPC, 0xd8c) & 1, 0, "PSR cleared");
        assert_eq!(m.peek(GPC, 0xd8) & (1 << 10), 0, "the trigger self-cleared");
        // the trigger was polled at least twice (it reads set once)
        let polls = m.accesses().iter().filter(|a| matches!(a, Access::R(GPC, 0xd8, _))).count();
        assert!(polls >= 3, "{polls}");
        assert_eq!(m.peek(GPC, 0x190) & (1 << 14), 1 << 14, "ADB request raised");
        adb_wait_up(&m, gpc(&m), &e, &MEDIAMIX).unwrap();
    }

    #[test]
    fn a_stuck_trigger_is_a_timeout_not_a_hang() {
        let m = Model::with_faults(Faults { gpc_pup_stuck: true, ..Default::default() });
        let e = TestEnv::default();
        assert_eq!(power_up(&m, gpc(&m), &e, &MIPI_PHY1), Err(Fail::PupTimeout));
        assert_eq!(*e.waited_us.borrow(), u64::from(POLL_US));
        assert_eq!(Fail::PupTimeout.errno(), -ETIMEDOUT);
    }

    #[test]
    fn a_refused_trigger_is_reported_from_the_status_registers() {
        let m = Model::with_faults(Faults { gpc_pup_refused: true, ..Default::default() });
        let e = TestEnv::default();
        assert_eq!(power_up(&m, gpc(&m), &e, &MEDIAMIX), Err(Fail::PupRefused(1)));
        assert_eq!(m.powered(), 0);
        // nothing after the refusal: PCR and the ADB request untouched
        assert!(!m.writes().iter().any(|w| w.1 == 0xd80 || w.1 == 0x190));
    }

    #[test]
    fn no_adb_ack_is_a_timeout() {
        let m = Model::with_faults(Faults { adb_no_ack: true, ..Default::default() });
        let e = TestEnv::default();
        power_up(&m, gpc(&m), &e, &MEDIAMIX).unwrap();
        assert_eq!(adb_wait_up(&m, gpc(&m), &e, &MEDIAMIX), Err(Fail::AdbTimeout));
        // a domain without a bridge has nothing to wait for
        assert_eq!(adb_wait_up(&m, gpc(&m), &e, &MIPI_PHY1), Ok(()));
    }

    #[test]
    fn power_down_drops_the_bridge_sets_pcr_then_triggers() {
        let m = Model::new();
        let e = TestEnv::default();
        power_up(&m, gpc(&m), &e, &MEDIAMIX).unwrap();
        m.clear_log();
        power_down(&m, gpc(&m), &e, &MEDIAMIX).unwrap();
        assert_eq!(m.powered() & MEDIAMIX.trigger, 0);
        let w = m.writes();
        let pos = |o| w.iter().position(|x| x.0 == GPC && x.1 == o).unwrap();
        assert!(pos(0x190) < pos(0xd80) && pos(0xd80) < pos(0xe4), "{w:?}");
        let s = state(&m, gpc(&m), &MEDIAMIX);
        assert!(s.psr && !s.adb_req && !s.adb_ack && !s.pcr_clear);
        // and up again: PSR seen, then cleared
        assert!(power_up(&m, gpc(&m), &e, &MEDIAMIX).unwrap().was_down);
        assert!(!state(&m, gpc(&m), &MEDIAMIX).psr);
    }

    #[test]
    fn a_stuck_power_down_is_a_timeout() {
        let m = Model::with_faults(Faults { gpc_pdn_stuck: true, ..Default::default() });
        let e = TestEnv::default();
        m.set_powered(MIPI_PHY1.trigger);
        assert_eq!(power_down(&m, gpc(&m), &e, &MIPI_PHY1), Err(Fail::PdnTimeout));
    }
}
