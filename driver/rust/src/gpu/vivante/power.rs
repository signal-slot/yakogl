// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Power and clocks for the GC7000UL: the GPUMIX and GPU3D domains of the
//! GPC and the four GPU clock roots and two gates of the CCM, over
//! [`crate::soc::imx8mp::gpc`] and [`crate::soc::imx8mp::ccm`].
//!
//! Every number is the i.MX 8M Plus reference manual's (IMX8MPRM Rev. 3,
//! 08/2024):
//! - roots (§5.1.2 Table 5-1 "Clock Root Table"): slice 3
//!   GPU3D_CORE_CLK_ROOT (0x8180, core, at most 1000 MHz) and 4
//!   GPU3D_SHADER_CLK_ROOT (0x8200, core, 1000 MHz), mux 010 =
//!   SYSTEM_PLL1_CLK; slice 24 GPU_AXI_CLK_ROOT (0x8C00, bus, 800 MHz) and 25
//!   GPU_AHB_CLK_ROOT (0x8C80, bus, 400 MHz), mux 001 = SYSTEM_PLL1_CLK
//!   (800 MHz). Core, shader and AXI run undivided at 800 MHz, AHB /2 at
//!   400 MHz, all within the table's maxima. A core slice has no pre divider
//!   (§5.1.7.10 PRE_PODF: "does not apply for CORE"), so pre stays /1.
//! - gates (§5.1.6.3 Table 5-9 "CCGR Mapping Table"): CCGR87 "GPU" (the
//!   GPUMIX bus clocks: §5.1.2 Table 5-2 gates GPU AXI and AHB through it)
//!   and CCGR70 "GPU3D" (core and shader). CCGR69 GPU2D stays closed.
//! - domains (§5.2.10.37 / .72, §5.2.11 PGC list): GPU_SHARE_LOGIC (PGC 7,
//!   the GPUMIX "share logic", with the GPUMIX NOC/ADB handshake in
//!   PU_PWRHSK bits 9 / 25, §5.2.10.57) and GPU_3D (PGC 9); GPU_2D is not
//!   powered.
//!
//! The order -- roots, gates, GPUMIX, its handshake, GPU3D -- is not stated
//! by the RM (it describes neither a GPU power-up order nor the handshake on
//! the way up). It is the order the C bring-up ran on the Verdin iMX8M Plus in
//! every board run since 2026-09-20 (`gc_identify` PASS onwards): clocks
//! first, so the bridge clocks run before its handshake. Two things differ
//! from that C, both towards the RM: the PGC status registers are checked
//! after each trigger and PSR is cleared ([`gpc::power_up`]), and the
//! GPUMIX handshake acknowledge is waited for ([`gpc::adb_wait_up`]). The C
//! only read it back for the log, so a missing acknowledge is logged as a
//! warning and the bring-up goes on to the identity read, which then decides
//! (a GPU that does not answer is -ENODEV there). The board run says which.

use crate::env::Env;
use crate::mmio::Mmio;
use crate::soc::imx8mp::ccm::{self, RootSetting};
use crate::soc::imx8mp::gpc;
use crate::ylog;

/// One GPU clock root: slice index, mux input, post divider (divide-by).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Root {
    pub name: &'static str,
    pub slice: u32,
    pub mux: u32,
    pub post: u32,
}

/// The roots, in the order they are set (Table 5-1, see the module text).
pub const ROOTS: [Root; 4] = [
    Root { name: "gpu3d_core", slice: 3, mux: 2, post: 1 },
    Root { name: "gpu3d_shader", slice: 4, mux: 2, post: 1 },
    Root { name: "gpu_axi", slice: 24, mux: 1, post: 1 },
    Root { name: "gpu_ahb", slice: 25, mux: 1, post: 2 },
];

/// CCGR87 "GPU" (GPUMIX bus) then CCGR70 "GPU3D" (Table 5-9).
pub const GATES: [u32; 2] = [87, 70];

/// The domains, GPUMIX first.
pub const DOMAINS: [&gpc::PuDomain; 2] = [&gpc::GPU_SHARE_LOGIC, &gpc::GPU_3D];

/// After the last domain, before the first GPU register access: the C's
/// 50 us, kept (the RM gives the PGC sequence in ipg-clock counts, below
/// 100 us at reset values, §5.2.11.10, and the trigger's self-clear already
/// waited for it).
pub const SETTLE_US: u32 = 50;

/// What [`power_on`] found.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PowerReport {
    /// Roots rewritten / gates opened (a bit per entry of ROOTS / GATES).
    pub roots_written: u32,
    pub gates_opened: u32,
    /// Domains that had been powered down (a bit per DOMAINS entry).
    pub was_down: u32,
    /// The GPUMIX handshake acknowledge did not come.
    pub adb_no_ack: bool,
}

/// The sub-steps of [`power_on`], in order, as [`power_on_marked`] reports
/// them: each is passed to the mark callback just BEFORE that sub-step's
/// first register access, so the last mark a dead board left names the
/// sub-step it was in. The numbers are what the netboot breadcrumb records
/// (driver/zephyr/lib/board/imx8mp/gc_result.h, kind STAGE, tags 0x31..0x37 =
/// 0x30 + the step); a change here is a change there.
pub mod step {
    /// The four clock roots ([`super::ROOTS`]).
    pub const ROOTS: u32 = 1;
    /// The two gates ([`super::GATES`]).
    pub const GATES: u32 = 2;
    /// Domain `i` of [`super::DOMAINS`]: its power-up is `DOMAIN0 + 2 * i`,
    /// its ADB400 handshake wait (GPUMIX only) `DOMAIN0 + 2 * i + 1`.
    pub const DOMAIN0: u32 = 3;
    /// The settle delay after the last domain.
    pub const SETTLE: u32 = 7;
}

// Every domain's two numbers fall below SETTLE.
const _: () = assert!(step::DOMAIN0 + 2 * DOMAINS.len() as u32 <= step::SETTLE);

/// Clock the GPU, then power GPUMIX and GPU3D (the module text's order).
pub fn power_on(m: &impl Mmio, ccm_base: usize, gpc_base: usize, env: &impl Env) -> Result<PowerReport, i32> {
    power_on_marked(m, ccm_base, gpc_base, env, &mut |_| {})
}

/// [`power_on`], calling `mark` with a [`step`] number before each sub-step.
/// `mark` must not touch the CCM, the GPC or the GPU: the register sequence
/// is [`power_on`]'s, access for access.
pub fn power_on_marked(
    m: &impl Mmio,
    ccm_base: usize,
    gpc_base: usize,
    env: &impl Env,
    mark: &mut impl FnMut(u32),
) -> Result<PowerReport, i32> {
    let mut rep = PowerReport::default();
    mark(step::ROOTS);
    for (i, r) in ROOTS.iter().enumerate() {
        if ccm::set_root(m, ccm_base, r.slice, RootSetting::on(r.mux, 1, r.post))? {
            rep.roots_written |= 1 << i;
        }
    }
    mark(step::GATES);
    for (i, &g) in GATES.iter().enumerate() {
        if ccm::gate_on(m, ccm_base, g)? {
            rep.gates_opened |= 1 << i;
        }
    }
    for (i, d) in DOMAINS.iter().enumerate() {
        let n = step::DOMAIN0 + 2 * i as u32;
        mark(n);
        let up = gpc::power_up(m, gpc_base, env, d).map_err(|f| {
            ylog!(env, Err, "GPC: {} power-up failed: {:?}", d.name, f);
            f.errno()
        })?;
        if up.was_down {
            rep.was_down |= 1 << i;
        }
        if d.adb.is_some() {
            mark(n + 1);
            if gpc::adb_wait_up(m, gpc_base, env, d).is_err() {
                rep.adb_no_ack = true;
                ylog!(env, Wrn, "GPC: {} handshake not acknowledged; going on to the identity read", d.name);
            }
        }
        ylog!(env, Inf, "GPC: {} on", d.name);
    }
    mark(step::SETTLE);
    env.busy_wait_us(SETTLE_US);
    Ok(rep)
}
