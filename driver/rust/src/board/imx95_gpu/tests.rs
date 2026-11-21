// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The i.MX 95's bring-up end to end on the host: the Zephyr Mali OSAL's
//! `up` ([`Osal::up_on`]) over [`Imx95Power`], against the mock System
//! Manager of soc::imx95::gpu_power's test (the Verdin's AP-NS permissions)
//! and the Mali-G310 (`mali::mock::G310`'s identity) on the Z4 MCU model
//! (nanopc_t6_submit's: the synthetic CSF firmware, CSG 0 / CS 0, the ring).
//! The board here answers a GPU register only while the SM has the GPU's
//! power domain and clock gate on, as the SoC does (an external abort
//! otherwise), and logs, in one order, every SCMI request and every GPU
//! register access.
//! Run: `cargo test -p yakogl-drivers --lib board::imx95_gpu`.

use super::*;
use crate::board::nanopc_t6_fw::tests::{backings, TABLES_PA};
use crate::board::nanopc_t6_fw::{STEP_GPU, STEP_NONE as FW_STEP_NONE};
use crate::board::nanopc_t6_osal::{Osal, OsalMem, CARVE_VA, HEAP_CHUNK_SIZE, HEAP_CHUNK_VA, HEAP_POOL_VA, QUEUE_VA};
use crate::board::nanopc_t6_submit::tests::{QUEUE, VM_TABLES_PA, Z4};
use crate::board::nanopc_t6_submit::{SubmitUp, HEAP_POOL_SIZE, QUEUE_VM_BYTES, STEP_FW, STEP_NONE};
use crate::firmware::scmi;
use crate::gpu::mali::csf::tests::{GPU, MEM};
use crate::gpu::mali::csf::{MCU_CONTROL, MCU_CONTROL_AUTO};
use crate::gpu::mali::fw::synth;
use crate::gpu::mali::mock::{G310, G310_ASSUMED, G310_GPU_ID};
use crate::gpu::mali::pgtable::{Table, ENTRIES};
use crate::gpu::mali::{CMD_SOFT_RESET, GPU_CMD, GPU_ID, L2_PWRON};
use crate::soc::imx95::gpu_power::{Status, Step, CLK_GPU_CGC, PD_GPU};
use crate::soc::imx95::gpu_power_tests::on;
use crate::soc::imx95::sm_mock::{Req, Sm};
use std::cell::RefCell;

/// What the board saw, in order.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Ev {
    /// An SCMI request: protocol, message, payload words.
    Scmi(Req),
    /// A read of a GPU register (the offset from the GPU's base).
    R(usize),
    /// A write of a GPU register.
    W(usize, u32),
}

/// The i.MX 95: the SM behind the SCMI channel and the GPU behind its
/// power domain.
struct Imx95 {
    z: Box<Z4>,
    sm: RefCell<Sm>,
    log: RefCell<Vec<Ev>>,
}

impl Imx95 {
    fn new(sm: Sm) -> Imx95 {
        Imx95::with_ident(sm, &G310)
    }

    /// The board whose GPU reads `ident` (offset, value) for its identity.
    fn with_ident(sm: Sm, ident: &[(usize, u32)]) -> Imx95 {
        let blob = synth::small();
        let mut z = Z4::with_regions(
            blob,
            &[
                (QUEUE_VA, QUEUE_VM_BYTES),
                (HEAP_POOL_VA, HEAP_POOL_SIZE),
                (HEAP_CHUNK_VA, u64::from(HEAP_CHUNK_SIZE)),
                (CARVE_VA, 16 << 20),
            ],
        );
        // the G610's model under a G310's identity, its RK3588 power
        // domain left on: the SM's state below is what gates the GPU
        z.t6 = z.t6.pd_gpu_powered();
        z.ident = ident.to_vec();
        Imx95 { z: Box::new(z), sm: RefCell::new(sm), log: RefCell::new(vec![]) }
    }

    fn is_gpu(a: usize) -> bool {
        (GPU..GPU + GPU_SIZE).contains(&a)
    }

    /// A GPU register with the domain or the gate off is an external abort.
    fn check(&self, a: usize) {
        let sm = self.sm.borrow();
        assert!(
            sm.on(PD_GPU) && sm.enabled(CLK_GPU_CGC),
            "GPU register {:#x} with the GPU off: an external abort",
            a - GPU
        );
    }

    fn log(&self) -> Vec<Ev> {
        self.log.borrow().clone()
    }
}

impl Mmio for Imx95 {
    fn read32(&self, a: usize) -> u32 {
        if Self::is_gpu(a) {
            self.check(a);
            self.log.borrow_mut().push(Ev::R(a - GPU));
        }
        self.z.read32(a)
    }
    fn write32(&self, a: usize, v: u32) {
        if Self::is_gpu(a) {
            self.check(a);
            self.log.borrow_mut().push(Ev::W(a - GPU, v));
        }
        self.z.write32(a, v)
    }
    fn read8(&self, a: usize) -> u8 {
        assert!(a >= MEM);
        self.z.read8(a)
    }
    fn write8(&self, a: usize, v: u8) {
        assert!(a >= MEM);
        self.z.write8(a, v)
    }
}

/// The board's SCMI channel: the SM's transfer, logged with the GPU's
/// accesses.
struct Chan<'a>(&'a Imx95);

impl scmi::Transport for Chan<'_> {
    fn transfer(&mut self, tx: &[u8], rx: &mut [u8]) -> Result<usize, i32> {
        let mut sm = self.0.sm.borrow_mut();
        let n = sm.transfer(tx, rx)?;
        let req = sm.log.last().unwrap().clone();
        self.0.log.borrow_mut().push(Ev::Scmi(req));
        Ok(n)
    }
}

struct Run {
    up: Result<(), u32>,
    osal: Box<Osal>,
    z1: GpuUp,
    o: SubmitUp,
    handed_over: Vec<GpuUp>,
}

/// The OSAL's bring-up on `b`, as the Verdin's board half calls it.
fn run(b: &Imx95) -> Run {
    let blob = synth::small();
    let vm = b.z.backings();
    let mut pool: Vec<Table> = vec![[0u64; ENTRIES]; 16];
    let mut vm_pool: Vec<Table> = vec![[0u64; ENTRIES]; 32];
    let mut osal = Box::new(Osal::EMPTY);
    let mut o = SubmitUp::default();
    let mut z1 = GpuUp::default();
    let mut handed_over = vec![];
    let mem = OsalMem {
        queue: QUEUE,
        ring: vm[0],
        heap_pool: vm[1],
        heap_chunks: vm[2],
        carve: vm[3],
        vm_pool: &mut vm_pool,
        vm_tables_pa: VM_TABLES_PA,
        heap_initial_chunks: 1,
        carve_reserved: 0,
    };
    let mut power = Imx95Power::new(Chan(b), Config::VERDIN);
    let up = osal.up_on(
        b,
        GPU,
        &mut power,
        &blob,
        &backings(&blob),
        &mut pool,
        TABLES_PA,
        24_000_000,
        mem,
        &|a, n| b.z.t6.clean(a, n),
        &mut |_| {},
        &mut |h| handed_over.push(*h),
        &mut z1,
        &mut o,
    );
    Run { up, osal, z1, o, handed_over }
}

/// The SCMI power-on first (the SM's requests in gpu_power's order, every
/// one before the first GPU register), then `GPU_ID`, the soft reset, the
/// L2, the CSF firmware's boot and the group: the OSAL is up on the G310.
#[test]
fn the_imx95_powers_the_g310_over_scmi_then_boots_its_firmware() {
    let b = Imx95::new(Sm::verdin());
    let r = run(&b);
    assert_eq!((r.up, r.o.step, r.o.refusal, r.o.detail), (Ok(()), STEP_NONE, 0, 0));
    assert!(r.osal.is_up());

    let log = b.log();
    // the SM's ten requests, nominal drive: level 2, 800 MHz
    let scmi: Vec<Ev> = on(2).into_iter().map(Ev::Scmi).collect();
    assert_eq!(log[..scmi.len()], scmi[..]);
    assert_eq!(log.iter().filter(|e| matches!(e, Ev::Scmi(_))).count(), scmi.len(), "no SCMI request after");
    // then the GPU's own part: GPU_ID first
    assert_eq!(log[scmi.len()], Ev::R(GPU_ID));
    let at = |e: &Ev| log.iter().position(|x| x == e).unwrap_or_else(|| panic!("{e:?} not in the log"));
    let reset = at(&Ev::W(GPU_CMD, CMD_SOFT_RESET));
    let l2 = log.iter().position(|e| matches!(e, Ev::W(a, _) if *a == L2_PWRON)).expect("the L2's power-on");
    let mcu = at(&Ev::W(MCU_CONTROL, MCU_CONTROL_AUTO));
    assert!(scmi.len() < reset && reset < l2 && l2 < mcu, "reset {reset}, L2 {l2}, MCU start {mcu}");

    // what the SM did, and the hand-over hook saw it before any GPU access
    let up = r.z1.power.expect("the power-on's record");
    assert_eq!((up.level.index, up.level.khz, up.rate_hz), (2, 800_000, 800_000_000));
    assert_eq!(r.handed_over.len(), 1);
    assert_eq!((r.handed_over[0].power, r.handed_over[0].ident.gpu_id), (Some(up), 0));
    assert_eq!((r.z1.stop, r.z1.refusal, r.z1.detail, r.z1.power_error), (Stop::None, 0, 0, None));
    assert_eq!(r.z1.ident_after_reset.gpu_id, G310_GPU_ID);
    assert_eq!(r.z1.ident_after_reset.shader_present, u64::from(G310_ASSUMED.shader_present));

    // Z3's record: the G310's identity, its two cores enabled, the boot
    // acknowledged, no supply of ours
    let fw = &r.o.fw;
    assert_eq!((fw.step, fw.gpu_id, fw.shader_present), (FW_STEP_NONE, G310_GPU_ID, [0x3, 0]));
    assert_eq!(fw.core_en_mask, [0x3, 0]);
    assert_eq!((fw.version, fw.ping_acked), (0x0105_0000, 1));
    assert_eq!((fw.supply_enabled, fw.supply_after, fw.supply_waited_us), (0, [0, 0], 0));
    // the group came up and the queries answer the G310
    assert_eq!((r.o.csg_req, r.o.csg_ack, r.o.cs_req, r.o.cs_ack), (0x11, 0x11, 0x0511, 0x0011));
    assert_eq!(r.osal.gpu_info().map(|g| g.gpu_id), Some(G310_GPU_ID));
}

/// The SM refuses the power domain (an AP-NS without `PD_GPU`): the run
/// stops at Z1 with the SCMI status, no GPU register is touched, nothing
/// is sent after the refusal, and the OSAL stays down.
#[test]
fn a_refused_power_on_reads_no_gpu_register() {
    let mut sm = Sm::verdin();
    sm.pd_perm.clear();
    let b = Imx95::new(sm);
    let r = run(&b);
    assert_eq!((r.up, r.o.step), (Err(STEP_FW), STEP_FW));
    assert!(!r.osal.is_up());
    let log = b.log();
    assert!(log.iter().all(|e| matches!(e, Ev::Scmi(_))), "{log:?}");
    let last = match log.last() {
        Some(Ev::Scmi(q)) => q.clone(),
        e => panic!("{e:?}"),
    };
    assert_eq!(last, (scmi::PROTO_POWER, scmi::POWER_STATE_SET, vec![0, PD_GPU, scmi::POWER_ON]));
    assert!(r.handed_over.is_empty());

    let e = Error::Refused { step: Step::PowerOn, status: Status::Denied };
    assert_eq!(r.z1.power_error, Some(e));
    assert_eq!(r.z1.power, None);
    let detail = u64::from(step_code(Step::PowerOn)) << 32 | u64::from(scmi::STATUS_DENIED as u32);
    assert_eq!((r.z1.stop, r.z1.refusal, r.z1.detail), (Stop::Scmi, 2, detail));
    assert_eq!(step_code(Step::PowerOn), 0x1104);
    // Z3's record names Z1's stop as the T6's does
    let fw = &r.o.fw;
    assert_eq!((fw.step, fw.stop, fw.refusal), (STEP_GPU, Stop::Scmi as u32, 2));
    assert_eq!(fw.detail, [detail as u32, (detail >> 32) as u32]);
    assert_eq!(e.to_string(), "POWER_STATE_SET (0x11/0x04): SCMI DENIED (-3)");
}

/// A GPU_ID no model names (the SM powered it): the run stops before the
/// reset with the T6's stop number for it.
#[test]
fn an_unknown_gpu_behind_the_domain_is_not_reset() {
    let mut ident = G310.to_vec();
    ident[0] = (GPU_ID, 0xb002_0000);
    let b = Imx95::with_ident(Sm::verdin(), &ident);
    let r = run(&b);
    assert_eq!((r.up, r.o.fw.step, r.o.fw.stop), (Err(STEP_FW), STEP_GPU, 6));
    assert_eq!((r.z1.stop, r.z1.detail), (Stop::UnsupportedGpu, 0xb002_0000));
    assert!(!b.log().contains(&Ev::W(GPU_CMD, CMD_SOFT_RESET)));
}

/// The record's stop numbers are the NanoPC-T6's for the GPU's own steps
/// (the OSAL's line names them alike).
#[test]
fn the_gpu_stops_keep_the_t6_numbers() {
    use crate::board::nanopc_t6_gpu::Stop as T6;
    for (a, b) in [
        (Stop::None, T6::None),
        (Stop::UnsupportedGpu, T6::UnsupportedGpu),
        (Stop::Reset, T6::Reset),
        (Stop::L2, T6::L2),
    ] {
        assert_eq!(a as u32, b as u32);
    }
    // the SCMI stop is none of the T6's
    assert!((0..=14).all(|n| n != Stop::Scmi as u32));
    assert_eq!((GPU_BASE, GPU_SIZE, IRQ_JOB, IRQ_MMU, IRQ_GPU), (0x4d90_0000, 0x48_0000, 289, 290, 288));
}

/// B3 (the pre-study's, imx95_fw): the power-on over SCMI, the firmware's
/// boot, then CSG 0's control block read back. After the `PING` no GPU
/// register is written: no group is started.
#[test]
fn the_firmware_boot_reads_back_csg0_and_starts_no_group() {
    use crate::board::nanopc_t6_submit::tests::BOARD_SUSPEND_SIZE;
    let b = Imx95::new(Sm::verdin());
    let blob = synth::small();
    let mut pool: Vec<Table> = vec![[0u64; ENTRIES]; 16];
    let mut z1 = GpuUp::default();
    let mut power = Imx95Power::new(Chan(&b), Config::VERDIN);
    let (fw, c) = fw_up(
        &b,
        GPU,
        &mut power,
        &blob,
        &backings(&blob),
        &mut pool,
        TABLES_PA,
        24_000_000,
        &|a, n| b.z.t6.clean(a, n),
        &mut |_| {},
        &mut |_| {},
        &mut z1,
    );
    assert_eq!((fw.step, fw.version, fw.ping_acked, fw.gpu_id), (FW_STEP_NONE, 0x0105_0000, 1, G310_GPU_ID));
    assert_eq!(fw.group_num, 8);
    assert_eq!(
        c,
        Csg0 {
            read: 1,
            features: c.features,
            suspend_size: BOARD_SUSPEND_SIZE,
            protm_suspend_size: BOARD_SUSPEND_SIZE,
            stream_num: fw.csg0[2],
            refusal: 0,
            detail: 0
        }
    );
    // the last GPU write is the PING's doorbell; nothing after the boot
    let log = b.log();
    let last_w = log.iter().rposition(|e| matches!(e, Ev::W(..))).unwrap();
    assert!(matches!(log[last_w], Ev::W(a, _) if a == crate::gpu::mali::csf::doorbell(0)), "{:?}", log[last_w]);
}

/// A refused power-on: no boot, so nothing is read back.
#[test]
fn a_refused_power_on_reads_back_no_csg() {
    let mut sm = Sm::verdin();
    sm.pd_perm.insert(PD_GPU, crate::soc::imx95::sm_mock::PERM_NONE);
    let b = Imx95::new(sm);
    let blob = synth::small();
    let mut pool: Vec<Table> = vec![[0u64; ENTRIES]; 16];
    let mut z1 = GpuUp::default();
    let mut power = Imx95Power::new(Chan(&b), Config::VERDIN);
    let (fw, c) = fw_up(
        &b,
        GPU,
        &mut power,
        &blob,
        &backings(&blob),
        &mut pool,
        TABLES_PA,
        24_000_000,
        &|a, n| b.z.t6.clean(a, n),
        &mut |_| {},
        &mut |_| {},
        &mut z1,
    );
    assert_eq!((fw.step, z1.stop), (STEP_GPU, Stop::Scmi));
    assert_eq!(c, Csg0::default());
    assert!(!b.log().iter().any(|e| matches!(e, Ev::R(_) | Ev::W(..))), "a GPU register was touched");
}
