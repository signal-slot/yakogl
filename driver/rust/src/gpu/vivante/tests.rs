// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Host tests of the GC7000 kernel side on the model bus.
//!
//! The first test replays the scenario of driver/tools/gclab/kerneltest
//! (deleted with the C, 4c25beb0) and holds the result to what the C kernel
//! side did in it ([`c_golden`], the recorded data):
//! the register traces, the MMU tables byte for byte, the ring after 1000
//! submits byte for byte, the interrupt's bookkeeping. The rest are the
//! sequences' own properties and their failures.

use super::c_golden as golden;
use super::*;
use crate::env::Level;
use crate::errno::*;
use crate::model::{Access, Faults, Model, TestEnv, CCM, GC, GPC};
use crate::soc::imx8mp::gpc as soc_gpc;

const CARVEOUT_PHYS: u64 = 0x5800_0000;
const BIG: usize = 64 * 1024 * 1024;
const SMALL: usize = 1024 * 1024;

/// A zeroed, 8-byte aligned carveout.
struct Carveout(Vec<u64>);

impl Carveout {
    fn new(bytes: usize) -> Carveout {
        Carveout(vec![0u64; bytes / 8])
    }
    fn ptr(&mut self) -> *mut u8 {
        self.0.as_mut_ptr().cast()
    }
    fn bytes(&self) -> &[u8] {
        // SAFETY: a Vec<u64> viewed as bytes.
        unsafe { core::slice::from_raw_parts(self.0.as_ptr().cast(), self.0.len() * 8) }
    }
    fn word(&self, off: usize) -> u32 {
        u32::from_le_bytes(self.bytes()[off..off + 4].try_into().unwrap())
    }
}

fn fnv(bytes: &[u8]) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    for &b in bytes {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x100_0000_01b3);
    }
    h
}

fn fnv_more(mut h: u64, bytes: &[u8]) -> u64 {
    for &b in bytes {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x100_0000_01b3);
    }
    h
}

/// The board's identity registers, as kerneltest pokes them.
fn poke_identity(m: &Model) {
    for (off, v) in [
        (0x18, 0x7000_6204),
        (0x20, 0x7000),
        (0x24, 0x6204),
        (0x28, 0x2020_0129),
        (0x2c, 0x0000_1234),
        (0xa8, 0x70003),
        (0x1c, 0xe068_7ca5),
        (0x34, 0xc979_9eff),
    ] {
        m.poke(GC, off, v);
    }
}

/// The model's GC accesses as the golden tables spell them.
fn gc_trace(m: &Model) -> Vec<(u8, u32, u32)> {
    m.accesses()
        .into_iter()
        .filter_map(|a| match a {
            Access::R(GC, o, v) => Some((b'R', o, v)),
            Access::W(GC, o, v) => Some((b'W', o, v)),
            _ => None,
        })
        .collect()
}

fn core_over(m: &Model, c: &mut Carveout, size: usize) -> GcCore {
    GcCore::new(m.base(GC), c.ptr(), CARVEOUT_PHYS, size)
}

#[test]
fn layouts_are_the_c_structs() {
    use core::mem::{offset_of, size_of};
    let rust: &[(&str, usize)] = &[
        ("dev", offset_of!(GcCore, dev)),
        ("id", offset_of!(GcCore, id)),
        ("regs", offset_of!(GcCore, regs)),
        ("carveout", offset_of!(GcCore, carveout)),
        ("carveout_phys", offset_of!(GcCore, carveout_phys)),
        ("carveout_size", offset_of!(GcCore, carveout_size)),
        ("mtlb", offset_of!(GcCore, mtlb)),
        ("pta", offset_of!(GcCore, pta)),
        ("stlb_used", offset_of!(GcCore, stlb_used)),
        ("import_va", offset_of!(GcCore, import_va)),
        ("mmu_on", offset_of!(GcCore, mmu_on)),
        ("ring", offset_of!(GcCore, ring)),
        ("ring_gpu", offset_of!(GcCore, ring_gpu)),
        ("ring_pos", offset_of!(GcCore, ring_pos)),
        ("wait_pos", offset_of!(GcCore, wait_pos)),
        ("fe_running", offset_of!(GcCore, fe_running)),
        ("event_free", offset_of!(GcCore, event_free)),
        ("event_fence", offset_of!(GcCore, event_fence)),
        ("fence_next", offset_of!(GcCore, fence_next)),
        ("fence_done", offset_of!(GcCore, fence_done)),
        ("irq_errors", offset_of!(GcCore, irq_errors)),
        ("fence_timeouts", offset_of!(GcCore, fence_timeouts)),
        ("max_quiet_ms", offset_of!(GcCore, max_quiet_ms)),
        ("fence_lost", offset_of!(GcCore, fence_lost)),
        ("resets", offset_of!(GcCore, resets)),
        ("idle_resets", offset_of!(GcCore, idle_resets)),
        ("event_busy", offset_of!(GcCore, event_busy)),
        ("wait_ms", offset_of!(GcCore, wait_ms)),
        ("event_t0_ns", offset_of!(GcCore, event_t0_ns)),
        ("perf_last_done_ns", offset_of!(GcCore, perf_last_done_ns)),
        ("perf_busy_ns", offset_of!(GcCore, perf_busy_ns)),
        ("perf_kicks_done", offset_of!(GcCore, perf_kicks_done)),
        ("perf_wait_ns", offset_of!(GcCore, perf_wait_ns)),
        ("perf_waits", offset_of!(GcCore, perf_waits)),
        ("perf_wait_sleeps", offset_of!(GcCore, perf_wait_sleeps)),
        ("pool_double_frees", offset_of!(GcCore, pool_double_frees)),
        ("dump_idle", offset_of!(GcCore, dump_idle)),
        ("dump_fence", offset_of!(GcCore, dump_fence)),
        ("dump_seen", offset_of!(GcCore, dump_seen)),
    ];
    assert_eq!(rust, golden::GC_DEV_OFFSETS);
    assert_eq!(offset_of!(GcCore, dump_seen) + 1, golden::GC_DEV_SIZE_TO_DUMP_SEEN);
    let id: &[(&str, usize)] = &[
        ("model", offset_of!(GcIdentity, model)),
        ("revision", offset_of!(GcIdentity, revision)),
        ("product_id", offset_of!(GcIdentity, product_id)),
        ("customer_id", offset_of!(GcIdentity, customer_id)),
        ("eco_id", offset_of!(GcIdentity, eco_id)),
        ("date", offset_of!(GcIdentity, date)),
        ("time", offset_of!(GcIdentity, time)),
        ("features", offset_of!(GcIdentity, features)),
        ("minor_features", offset_of!(GcIdentity, minor_features)),
        ("secure_ahb", offset_of!(GcIdentity, secure_ahb)),
        ("from_table", offset_of!(GcIdentity, from_table)),
        ("stream_count", offset_of!(GcIdentity, stream_count)),
        ("register_max", offset_of!(GcIdentity, register_max)),
        ("thread_count", offset_of!(GcIdentity, thread_count)),
        ("shader_core_count", offset_of!(GcIdentity, shader_core_count)),
        ("vertex_cache_size", offset_of!(GcIdentity, vertex_cache_size)),
        ("vertex_output_buffer_size", offset_of!(GcIdentity, vertex_output_buffer_size)),
        ("pixel_pipes", offset_of!(GcIdentity, pixel_pipes)),
        ("instruction_count", offset_of!(GcIdentity, instruction_count)),
        ("num_constants", offset_of!(GcIdentity, num_constants)),
        ("varyings_count", offset_of!(GcIdentity, varyings_count)),
    ];
    assert_eq!(id, golden::GC_IDENTITY_OFFSETS);
    assert_eq!(size_of::<GcIdentity>(), golden::GC_IDENTITY_SIZE);
    assert_eq!(size_of::<GcSubmit>(), golden::GC_SUBMIT_SIZE);
}

/// kerneltest's whole scenario, against the C's record of it.
#[test]
fn the_c_scenario_comes_out_identical() {
    let m = Model::new();
    let e = TestEnv::default();
    poke_identity(&m);
    let regs = m.base(GC);

    // 1. identify + reset. The reads differ by design (the five minor
    // feature registers galcore reads); the writes are the C's.
    let id = hw::identify(&m, regs, &e).unwrap();
    hw::reset(&m, regs, &e, &id).unwrap();
    assert_eq!(golden::ATTACH_RC, 0);
    assert_eq!(*e.waited_us.borrow(), golden::ATTACH_WAITED_US);
    let t = gc_trace(&m);
    let c_reads: Vec<_> = golden::ATTACH.iter().take(10).copied().collect();
    assert_eq!(&t[..10], &c_reads[..], "the C's identity reads, in its order");
    let extra: Vec<u32> = t[10..15].iter().map(|a| a.1).collect();
    assert_eq!(extra, [0x74, 0x84, 0x88, 0x94, 0xa0], "then minor feature words 1..5");
    assert!(t[10..15].iter().all(|a| a.0 == b'R'));
    assert_eq!(&t[15..], &golden::ATTACH[10..], "the reset: every access as the C");
    assert_eq!(id.secure_ahb, golden::ID_SECURE_AHB);
    assert_eq!(id.from_table, golden::ID_FROM_TABLE);
    let limits = [
        id.stream_count,
        id.register_max,
        id.thread_count,
        id.shader_core_count,
        id.vertex_cache_size,
        id.vertex_output_buffer_size,
        id.pixel_pipes,
        id.instruction_count,
        id.num_constants,
        id.varyings_count,
    ];
    // One documented difference: galcore's database says 8 vertex streams
    // where the C's table said 16; nothing in the driver reads it.
    assert_eq!(golden::ID_LIMITS[0], 16);
    assert_eq!(limits[0], 8);
    assert_eq!(limits[1..], golden::ID_LIMITS[1..]);

    // the reset alone, as recovery runs it
    m.clear_log();
    hw::reset(&m, regs, &e, &id).unwrap();
    assert_eq!(gc_trace(&m), golden::RESET);
    assert_eq!(golden::RESET_RC, 0);

    // 2. init after reset
    m.clear_log();
    hw::init(&m, regs, &e);
    assert_eq!(golden::HW_INIT_RC, 0);
    assert_eq!(gc_trace(&m), golden::HW_INIT);

    // 3. the MMU
    let mut co = Carveout::new(BIG);
    let mut c = core_over(&m, &mut co, BIG);
    c.id = &id;
    assert_eq!(mmu::init(&mut c, &e).err().unwrap_or(0), golden::MMU_INIT_RC);
    assert_eq!(mmu::map(&mut c, &e, 0x1000_0000, 0x1_2345_6000, 3 * 4096).err().unwrap_or(0), golden::MMU_MAP_HIGH_RC);
    assert_eq!(mmu::map(&mut c, &e, 0x103f_f000, 0x4000_0000, 2 * 4096).err().unwrap_or(0), golden::MMU_MAP_CROSS_RC);
    assert_eq!(c.stlb_used, golden::STLB_USED);
    m.clear_log();
    assert_eq!(mmu::enable(&mut c, &m, &e).err().unwrap_or(0), golden::MMU_ENABLE_RC);
    assert_eq!(gc_trace(&m), golden::MMU_ENABLE);
    assert_eq!(c.mmu_on, golden::MMU_ON);
    let words = |c: &GcCore, off: usize, n: usize| (0..n).map(|i| co_word(c, off + i * 4)).collect::<Vec<_>>();
    let mtlb0 = ((CARVEOUT_PHYS >> 22) - 1) as usize;
    assert_eq!(words(&c, OFF_MTLB + mtlb0 * 4, 18), golden::MTLB_CARVEOUT);
    assert_eq!(words(&c, OFF_MTLB + 0x40 * 4, 4), golden::MTLB_IMPORT);
    assert_eq!(words(&c, OFF_PTA, 4), golden::PTA_HEAD);
    assert_eq!(words(&c, OFF_MMU_CMD, 8), golden::MMU_CMD);
    assert_eq!(words(&c, OFF_STLB, 8), golden::STLB0_HEAD);
    assert_eq!(words(&c, OFF_STLB + 16 * 4096, 4), golden::STLB_IMPORT_HEAD);
    assert_eq!(words(&c, OFF_STLB + 16 * 4096 + 1022 * 4, 2), golden::STLB_CROSS);
    assert_eq!(words(&c, OFF_STLB + 17 * 4096, 2), golden::STLB_CROSS_NEXT);
    assert_eq!(golden::MMU_FNV_LEN, OFF_STLB + c.stlb_used as usize * 4096);
    assert_eq!(fnv(&co.bytes()[..golden::MMU_FNV_LEN]), golden::MMU_FNV, "the MMU tables, byte for byte");

    // 4. the ring
    m.clear_log();
    cmdbuf::ring_init(&mut c, &m, &e, 800_000_000);
    assert_eq!(golden::RING_INIT_RC, 0);
    assert_eq!(gc_trace(&m), golden::RING_INIT);
    assert_eq!(words(&c, OFF_RING, 4), golden::RING_IDLE);
    m.clear_log();
    let flags = [
        0,
        submit::FLUSH_COLOR,
        submit::FLUSH_DEPTH | submit::FLUSH_TEX,
        submit::NO_PIPE_SEL,
        submit::NO_PIPE_SEL | submit::FLUSH_COLOR | submit::FLUSH_DEPTH | submit::FLUSH_TEX,
    ];
    let mut pos_fnv = 0xcbf2_9ce4_8422_2325u64;
    let mut wrap_at = 0;
    for k in 0..1000u32 {
        let s = GcSubmit {
            cmd_gpu: CARVEOUT_PHYS as u32 + OFF_POOL as u32 + (k % 8) * 4096,
            cmd_bytes: 8 * (1 + k % 7),
            flags: flags[(k % 5) as usize],
        };
        let before = c.ring_pos;
        cmdbuf::ring_submit(&mut c, &e, &s, k % EVENT_COUNT, k + 1, 800_000_000).unwrap();
        if c.ring_pos < before && wrap_at == 0 {
            wrap_at = k;
        }
        pos_fnv = fnv_more(pos_fnv, &c.ring_pos.to_le_bytes());
        pos_fnv = fnv_more(pos_fnv, &c.wait_pos.to_le_bytes());
        if k == 2 {
            assert_eq!(words(&c, OFF_RING, 96), golden::RING_AFTER_3, "the first three segments");
        }
    }
    assert_eq!(wrap_at, golden::RING_WRAP_AT);
    assert_ne!(wrap_at, 0, "the scenario wraps the ring");
    assert_eq!(pos_fnv, golden::RING_POS_FNV);
    assert_eq!((c.ring_pos, c.wait_pos), golden::RING_END);
    assert_eq!(fnv(&co.bytes()[OFF_RING..OFF_RING + RING_SIZE]), golden::RING_FNV, "the ring, byte for byte");
    assert_eq!(fnv(&co.bytes()[OFF_POOL..OFF_POOL + 8 * 4096]), golden::POOL_FNV, "the LINKs back");
    assert_eq!(gc_trace(&m), golden::SUBMITS, "a submit touches no register");
    let bad = GcSubmit { cmd_gpu: CARVEOUT_PHYS as u32 + OFF_POOL as u32, cmd_bytes: 12, flags: 0 };
    assert_eq!(cmdbuf::ring_submit(&mut c, &e, &bad, 0, 1, 800_000_000), Err(golden::SUBMIT_MISALIGNED_RC));
    let out = GcSubmit { cmd_gpu: 0x1000_0000, cmd_bytes: 64, flags: 0 };
    assert_eq!(cmdbuf::ring_submit(&mut c, &e, &out, 0, 1, 800_000_000), Err(golden::SUBMIT_OUTSIDE_RC));

    // 5. events and the interrupt
    c.event_free = EVENT_ALL;
    c.event_fence = [0; EVENT_COUNT as usize];
    c.fence_done = 0;
    let ids: Vec<i32> = (1..=5).map(|f| irq::claim(&mut c, f).unwrap().unwrap() as i32).collect();
    assert_eq!(
        ids,
        [
            golden::EVENT_FOR_FENCE_1,
            golden::EVENT_FOR_FENCE_2,
            golden::EVENT_FOR_FENCE_3,
            golden::EVENT_FOR_FENCE_4,
            golden::EVENT_FOR_FENCE_5
        ]
    );
    c.event_t0_ns[0] = 100;
    c.event_t0_ns[2] = 200;
    m.clear_log();
    let mut sem = 0;
    m.poke(GC, 0x10, (1 << 0) | (1 << 2) | (1 << 29) | (1 << 30) | (1 << 31));
    let h = irq::handle(&mut c, &m, 6000);
    sem += h.freed;
    assert_eq!(h.ours, golden::IRQ1);
    let state = |c: &GcCore, sem| {
        (c.fence_done, c.event_free, c.irq_errors, c.perf_kicks_done, c.perf_busy_ns, c.perf_last_done_ns, sem)
    };
    assert_eq!(state(&c, sem), golden::IRQ1_STATE);
    m.poke(GC, 0x10, 1 << 1);
    let h = irq::handle(&mut c, &m, 7000);
    sem += h.freed;
    assert_eq!(h.ours, golden::IRQ2);
    assert_eq!(state(&c, sem), golden::IRQ2_STATE, "an older fence retiring late does not move fence_done back");
    assert_eq!(irq::handle(&mut c, &m, 8000).ours, golden::IRQ_NONE);
    assert_eq!(gc_trace(&m), golden::IRQ);

    // 6. MMU refusals
    assert_eq!(mmu::map(&mut c, &e, 0x2000_0800, 0x4000_0000, 4096), Err(golden::MMU_MAP_MISALIGNED_RC));
    assert_eq!(mmu::map(&mut c, &e, 0x2000_0000, 0x4000_0000, 64 * 4 * 1024 * 1024), Err(golden::MMU_MAP_EXHAUST_RC));
    assert_eq!(c.stlb_used, golden::STLB_USED_EXHAUSTED);
    assert_eq!(fnv(&co.bytes()[..OFF_RING]), golden::MMU_EXHAUST_FNV);
}

fn co_word(c: &GcCore, off: usize) -> u32 {
    c.mem().r32(off)
}

#[test]
fn power_on_clocks_first_then_gpumix_then_gpu3d() {
    let m = Model::new();
    let e = TestEnv::default();
    poke_identity(&m);
    m.require_gpu_power();
    let rep = power::power_on(&m, m.base(CCM), m.base(GPC), &e).unwrap();
    assert_eq!(rep.roots_written, 0b1111);
    assert_eq!(rep.gates_opened, 0b11);
    assert!(!rep.adb_no_ack);
    // with the domains and clocks up, the GPU answers (the model fails the
    // test on any GC access before that)
    let id = hw::identify(&m, m.base(GC), &e).unwrap();
    assert!(id.from_table);

    let w = m.writes();
    let pos =
        |b: &str, o: u32| w.iter().position(|x| x.0 == b && x.1 == o).unwrap_or_else(|| panic!("{b} {o:#x}: {w:?}"));
    // roots (Table 5-1 offsets), with their mux and dividers
    assert_eq!(m.peek(CCM, 0x8180), (1 << 28) | (2 << 24));
    assert_eq!(m.peek(CCM, 0x8200), (1 << 28) | (2 << 24));
    assert_eq!(m.peek(CCM, 0x8c00), (1 << 28) | (1 << 24));
    assert_eq!(m.peek(CCM, 0x8c80), (1 << 28) | (1 << 24) | 1);
    assert!(pos(CCM, 0x8c80) < pos(CCM, 0x4574), "roots before gates");
    assert!(pos(CCM, 0x4574) < pos(CCM, 0x4464), "CCGR87 then CCGR70");
    let pups: Vec<u32> = w.iter().filter(|x| x.0 == GPC && x.1 == 0xd8).map(|x| x.2).collect();
    assert_eq!(pups.len(), 2);
    assert_eq!(pups[0] & (1 << 7), 1 << 7, "GPUMIX first");
    assert_eq!(pups[1] & (1 << 9), 1 << 9, "then GPU3D");
    assert!(pos(CCM, 0x4464) < pos(GPC, 0xd8), "clocks before the domains");
    assert!(pos(GPC, 0x190) < w.iter().rposition(|x| x.0 == GPC && x.1 == 0xd8).unwrap(), "the handshake before GPU3D");
    assert_eq!(m.peek(GPC, 0x190) & (1 << 9), 1 << 9, "the GPUMIX handshake raised");
    assert_eq!(m.powered() & ((1 << 7) | (1 << 9)), (1 << 7) | (1 << 9));
    assert_eq!(m.powered() & (1 << 6), 0, "GPU2D stays off");
    assert!(*e.waited_us.borrow() >= u64::from(power::SETTLE_US));

    // a second power-on writes nothing to the CCM: everything is already set
    m.clear_log();
    let rep = power::power_on(&m, m.base(CCM), m.base(GPC), &e).unwrap();
    assert_eq!((rep.roots_written, rep.gates_opened), (0, 0));
    assert!(!m.writes().iter().any(|x| x.0 == CCM));
}

/// The netboot breadcrumbs (gc_result.h kind STAGE 0x31..0x37) name the
/// sub-step a dead board was in: each mark must come before that sub-step's
/// first write and after the previous one's last, and marking must not change
/// the register sequence.
#[test]
fn power_on_marks_each_sub_step_before_its_first_write() {
    let plain = Model::new();
    plain.require_gpu_power();
    power::power_on(&plain, plain.base(CCM), plain.base(GPC), &TestEnv::default()).unwrap();

    let m = Model::new();
    m.require_gpu_power();
    let mut marks: Vec<(u32, usize)> = Vec::new();
    power::power_on_marked(&m, m.base(CCM), m.base(GPC), &TestEnv::default(), &mut |n| {
        marks.push((n, m.writes().len()))
    })
    .unwrap();
    let w = m.writes();
    assert_eq!(w, plain.writes(), "marking changed the register sequence");

    use power::step;
    let order: Vec<u32> = marks.iter().map(|x| x.0).collect();
    // GPU_SHARE_LOGIC has the GPUMIX handshake, GPU_3D has none
    assert_eq!(order, [step::ROOTS, step::GATES, step::DOMAIN0, step::DOMAIN0 + 1, step::DOMAIN0 + 2, step::SETTLE]);
    let at = |n: u32| marks.iter().find(|x| x.0 == n).unwrap().1;
    let first = |b: &str, o: u32| w.iter().position(|x| x.0 == b && x.1 == o).unwrap();
    let last = |b: &str, o: u32| w.iter().rposition(|x| x.0 == b && x.1 == o).unwrap();
    assert_eq!(at(step::ROOTS), 0, "nothing written before the first mark");
    assert!(at(step::ROOTS) <= first(CCM, 0x8180) && last(CCM, 0x8c80) < at(step::GATES));
    assert!(at(step::GATES) <= first(CCM, 0x4574) && last(CCM, 0x4464) < at(step::DOMAIN0));
    let pups: Vec<usize> = w.iter().enumerate().filter(|x| x.1 .0 == GPC && x.1 .1 == 0xd8).map(|x| x.0).collect();
    assert_eq!(pups.len(), 2);
    assert!(at(step::DOMAIN0) <= pups[0] && pups[0] < at(step::DOMAIN0 + 1));
    // the handshake wait only reads (power_up raised the request)
    assert!(at(step::DOMAIN0 + 1) <= at(step::DOMAIN0 + 2));
    assert!(at(step::DOMAIN0 + 2) <= pups[1] && pups[1] < at(step::SETTLE));
    assert_eq!(at(step::SETTLE), w.len(), "the settle delay writes nothing");
}

#[test]
fn power_failures() {
    let e = TestEnv::default();
    let m = Model::with_faults(Faults { gpc_pup_stuck: true, ..Default::default() });
    assert_eq!(power::power_on(&m, m.base(CCM), m.base(GPC), &e), Err(-ETIMEDOUT));
    assert_eq!(m.writes().iter().filter(|x| x.0 == GPC && x.1 == 0xd8).count(), 1, "GPU3D never triggered");

    let m = Model::with_faults(Faults { gpc_pup_refused: true, ..Default::default() });
    assert_eq!(power::power_on(&m, m.base(CCM), m.base(GPC), &e), Err(-EIO));

    // a missing handshake acknowledge is a warning; the identity read decides
    let m = Model::with_faults(Faults { gpumix_adb_no_ack: true, ..Default::default() });
    let e = TestEnv::default();
    let rep = power::power_on(&m, m.base(CCM), m.base(GPC), &e).unwrap();
    assert!(rep.adb_no_ack);
    assert!(e.lines.borrow().iter().any(|l| l.0 == Level::Wrn && l.1.contains("handshake")));
    assert_eq!(m.powered() & (1 << 9), 1 << 9, "GPU3D still powered");
    assert_eq!(soc_gpc::GPU_SHARE_LOGIC.adb.unwrap().ack, 1 << 25);
}

#[test]
fn identify_refuses_a_gpu_that_does_not_answer() {
    for model in [0, 0xffff_ffff, 0xdead_de00, 0xdead_de42] {
        let m = Model::new();
        let e = TestEnv::default();
        poke_identity(&m);
        m.poke(GC, 0x20, model);
        assert_eq!(hw::identify(&m, m.base(GC), &e), Err(-ENODEV), "{model:#x}");
        assert!(m.writes().is_empty(), "identify writes nothing");
    }
}

#[test]
fn an_unknown_part_is_identified_from_registers_alone() {
    let m = Model::new();
    let e = TestEnv::default();
    poke_identity(&m);
    m.poke(GC, 0x24, 0x6214);
    let id = hw::identify(&m, m.base(GC), &e).unwrap();
    assert!(!id.from_table && !id.secure_ahb);
    assert_eq!((id.stream_count, id.num_constants), (0, 0));
    assert!(e.lines.borrow().iter().any(|l| l.0 == Level::Wrn && l.1.contains("not in the hardware database")));
    // a wrong product id is not the database's part either
    let m = Model::new();
    poke_identity(&m);
    m.poke(GC, 0xa8, 0x70002);
    assert!(!hw::identify(&m, m.base(GC), &e).unwrap().from_table);
}

#[test]
fn the_database_facts_are_folded_into_the_feature_words() {
    let m = Model::new();
    let e = TestEnv::default();
    poke_identity(&m);
    let id = hw::identify(&m, m.base(GC), &e).unwrap();
    assert_eq!(id.features, 0xe068_7ca5, "the register, as galcore keeps it");
    assert_ne!(id.minor_features[1] & regs::CHIP_MINOR_FEATURES1_MMU_VERSION, 0);
    assert_ne!(id.minor_features[7] & regs::CHIP_MINOR_FEATURES7_BIT_SECURITY, 0);
    assert_ne!(id.minor_features[10] & regs::CHIP_MINOR_FEATURES10_SECURITY_AHB, 0);
    assert!(id.secure_ahb);
}

#[test]
fn a_gpu_that_never_idles_fails_the_reset_after_a_bounded_wait() {
    let m = Model::with_faults(Faults { gc_never_idle: true, ..Default::default() });
    let e = TestEnv::default();
    let mut id = GcIdentity { secure_ahb: true, ..Default::default() };
    id.minor_features[1] = regs::CHIP_MINOR_FEATURES1_MMU_VERSION;
    assert_eq!(hw::reset(&m, m.base(GC), &e, &id), Err(-EBUSY));
    let per = u64::from(hw::RESET_HOLD_US + hw::RESET_RETRY_US);
    assert_eq!(*e.waited_us.borrow(), u64::from(hw::RESET_ATTEMPTS) * per);
    assert!(e.lines.borrow().iter().any(|l| l.0 == Level::Err && l.1.contains("FE not idle")));
}

#[test]
fn an_mmu_left_on_fails_the_reset() {
    let m = Model::with_faults(Faults { gc_mmu_stuck: true, ..Default::default() });
    let e = TestEnv::default();
    poke_identity(&m);
    let id = hw::identify(&m, m.base(GC), &e).unwrap();
    assert_eq!(hw::reset(&m, m.base(GC), &e, &id), Err(-EBUSY));
    // without MMU v2 the check is skipped
    let mut old = id;
    old.minor_features[1] &= !regs::CHIP_MINOR_FEATURES1_MMU_VERSION;
    assert_eq!(hw::reset(&m, m.base(GC), &e, &old), Ok(()));
}

#[test]
fn without_the_secure_ahb_the_reset_uses_clock_control() {
    let m = Model::new();
    let e = TestEnv::default();
    let id = GcIdentity::default();
    hw::reset(&m, m.base(GC), &e, &id).unwrap();
    let w = m.writes();
    assert!(!w.iter().any(|x| x.1 == regs::VIVS_MMUv2_AHB_CONTROL), "{w:?}");
    assert!(w.iter().any(|x| x.1 == 0 && x.2 & regs::VIVS_HI_CLOCK_CONTROL_SOFT_RESET != 0));
    // isolate, soft reset, release, de-isolate, in that order
    let cc: Vec<u32> = w.iter().filter(|x| x.1 == 0).map(|x| x.2).collect();
    assert_eq!(cc, [0x300, 0x100, 0x80100, 0x81100, 0x80100, 0x100, 0x30100]);
}

#[test]
fn fe_wait_idle_times_out() {
    let m = Model::with_faults(Faults { gc_never_idle: true, ..Default::default() });
    let e = TestEnv::default();
    let mut co = Carveout::new(SMALL);
    let c = core_over(&m, &mut co, SMALL);
    assert_eq!(cmdbuf::fe_wait_idle(&c, &m, &e, 3), Err(-ETIMEDOUT));
    assert_eq!(*e.waited_us.borrow(), 3000);
}

#[test]
fn mmu_enable_failures() {
    let e = TestEnv::default();
    // already on: nothing is written
    let m = Model::with_faults(Faults { gc_mmu_stuck: true, ..Default::default() });
    let mut co = Carveout::new(SMALL);
    let mut c = core_over(&m, &mut co, SMALL);
    mmu::init(&mut c, &e).unwrap();
    assert_eq!(mmu::enable(&mut c, &m, &e), Ok(()));
    assert!(c.mmu_on && m.writes().is_empty());

    // the PTA stream does not finish: the MMU stays off
    let m = Model::with_faults(Faults { gc_never_idle: true, ..Default::default() });
    let mut co = Carveout::new(SMALL);
    let mut c = core_over(&m, &mut co, SMALL);
    mmu::init(&mut c, &e).unwrap();
    assert_eq!(mmu::enable(&mut c, &m, &e), Err(-ETIMEDOUT));
    assert!(!c.mmu_on);
    assert!(!m.writes().iter().any(|x| x.1 == regs::VIVS_MMUv2_SEC_CONTROL));
}

#[test]
fn mmu_init_refuses_a_carveout_it_cannot_use() {
    let m = Model::new();
    let e = TestEnv::default();
    let mut co = Carveout::new(SMALL);
    let mut c = GcCore::new(m.base(GC), co.ptr(), 0xffff_0000, SMALL);
    assert_eq!(mmu::init(&mut c, &e), Err(-ERANGE));
    let mut c = GcCore::new(m.base(GC), co.ptr(), CARVEOUT_PHYS, OFF_POOL);
    assert_eq!(mmu::init(&mut c, &e), Err(-ENOSPC));
}

#[test]
fn mmu_entries_are_galcores_layout() {
    // bits 31:12, bits 39:32 in 11:4, writable, present
    assert_eq!(mmu::stlb_entry(0xab_1234_5000), 0x1234_5000 | (0xab << 4) | 0b101);
    let m = Model::new();
    let e = TestEnv::default();
    let mut co = Carveout::new(SMALL);
    let mut c = core_over(&m, &mut co, SMALL);
    mmu::init(&mut c, &e).unwrap();
    let mem = c.mem();
    // untouched master and slave entries fault; the safe page has its pattern
    assert_eq!(mem.r32(OFF_MTLB), mmu::PTE_EXCEPTION);
    assert_eq!(mem.r32(OFF_SAFE_PAGE + 4092), mmu::SAFE_PATTERN);
    let slot = (CARVEOUT_PHYS >> 22) as usize;
    let mtlb = mem.r32(OFF_MTLB + slot * 4);
    assert_eq!(mtlb, (CARVEOUT_PHYS as u32 + OFF_STLB as u32) | mmu::PTE_PRESENT);
    assert_eq!(mem.r32(OFF_STLB + 256 * 4), mmu::PTE_EXCEPTION, "past the 1 MiB carveout");
    // mapping into an existing slave table reuses it
    let used = c.stlb_used;
    mmu::map(&mut c, &e, CARVEOUT_PHYS as u32 + 0x20_0000, 0x9000_0000, 4096).unwrap();
    assert_eq!(c.stlb_used, used);
    assert_eq!(mem.r32(OFF_STLB + 512 * 4), 0x9000_0005);
}

#[test]
fn the_ring_commit_writes_the_address_before_the_header() {
    // The commit turns the old WAIT into a LINK. Both words are checked
    // after the fact; the order is the barrier's, which dmb_sy() provides
    // and the identical byte image above cannot show -- so check the
    // invariant the order protects: a LINK header is never left pointing at
    // a stale address.
    let m = Model::new();
    let e = TestEnv::default();
    let mut co = Carveout::new(SMALL);
    let mut c = core_over(&m, &mut co, SMALL);
    let id = GcIdentity { secure_ahb: true, ..Default::default() };
    c.id = &id;
    mmu::init(&mut c, &e).unwrap();
    cmdbuf::ring_init(&mut c, &m, &e, 800_000_000);
    let w0 = c.wait_pos;
    let s = GcSubmit { cmd_gpu: CARVEOUT_PHYS as u32 + OFF_POOL as u32, cmd_bytes: 64, flags: 0 };
    cmdbuf::ring_submit(&mut c, &e, &s, 3, 1, 800_000_000).unwrap();
    let mem = c.mem();
    assert_eq!(mem.r32(OFF_RING + w0 as usize * 4) >> 27, 8, "the old WAIT is a LINK now");
    assert_eq!(mem.r32(OFF_RING + w0 as usize * 4 + 4), c.ring_gpu + 4 * 4, "to the new segment");
    assert_eq!(mem.r32(OFF_RING + c.wait_pos as usize * 4) >> 27, 7, "a fresh WAIT");
    // the caller's buffer ends in a LINK back to the epilogue
    assert_eq!(mem.r32(OFF_POOL + 64) >> 27, 8);
    // the FE start mirrored into the secure command control
    let w = m.writes();
    assert_eq!(w.iter().filter(|x| x.1 == regs::VIVS_MMUv2_SEC_COMMAND_CONTROL).count(), 1);
}

#[test]
fn events_run_out_and_a_corrupt_bitmap_is_refused() {
    let m = Model::new();
    let mut co = Carveout::new(SMALL);
    let mut c = core_over(&m, &mut co, SMALL);
    c.event_free = EVENT_ALL;
    for f in 1..=EVENT_COUNT {
        assert_eq!(irq::claim(&mut c, f), Ok(Some(f - 1)));
    }
    assert_eq!(irq::claim(&mut c, 99), Ok(None));
    c.event_free = 1 << 31;
    assert_eq!(irq::claim(&mut c, 1), Err(-EIO));
    assert_eq!(c.event_free, 0, "the bad id is taken out");
}

#[test]
fn fence_done_moves_forward_across_the_wrap() {
    let m = Model::new();
    let mut co = Carveout::new(SMALL);
    let mut c = core_over(&m, &mut co, SMALL);
    c.event_free = EVENT_ALL;
    c.fence_done = u32::MAX - 1;
    irq::claim(&mut c, u32::MAX).unwrap();
    irq::claim(&mut c, 2).unwrap();
    m.poke(GC, 0x10, 0b10);
    irq::handle(&mut c, &m, 10);
    assert_eq!(c.fence_done, 2, "2 is after u32::MAX - 1");
    m.poke(GC, 0x10, 0b01);
    irq::handle(&mut c, &m, 20);
    assert_eq!(c.fence_done, 2, "u32::MAX retiring late does not move it back");
}

#[test]
fn errors_count_and_unknown_ids_are_ignored() {
    let m = Model::new();
    let mut co = Carveout::new(SMALL);
    let mut c = core_over(&m, &mut co, SMALL);
    m.poke(GC, 0x10, 0xc000_0000);
    let h = irq::handle(&mut c, &m, 1);
    assert!(h.ours);
    assert_eq!((h.freed, c.irq_errors), (0, 2));
    assert_eq!(m.peek(GC, 0x10), 0, "read to clear");
}

#[test]
fn idle_state_names_the_busy_modules() {
    let mut b = [0u8; 32];
    let n = hw::busy_modules(0x7fff_fff2, &mut b);
    assert_eq!(&b[..n], b" FE PE SH");
    assert_eq!(hw::busy_modules(0x7fff_ffff, &mut b), 0);
    let e = TestEnv::default();
    hw::log_idle_state(&e, 0x7fff_fff2);
    assert!(e.lines.borrow()[0].1.contains("BUSY -> FE PE SH"));
}

#[test]
fn wait_cycles_follow_the_core_clock_within_the_field() {
    assert_eq!(cmdbuf::wait_cycles(800_000_000), 24414);
    assert_eq!(cmdbuf::wait_cycles(1_000_000), cmdbuf::WAIT_CYCLES_MIN);
    assert_eq!(cmdbuf::wait_cycles(u32::MAX), cmdbuf::WAIT_CYCLES_MAX);
}

#[test]
fn the_pta_stream_is_one_load_state_and_an_end() {
    let m = Model::new();
    let mut co = Carveout::new(SMALL);
    let c = core_over(&m, &mut co, SMALL);
    assert_eq!(mmu::build_pta_config(&c, 0), 16);
    assert_eq!(co.word(OFF_MMU_CMD), 0x0801_006b);
    assert_eq!(co.word(OFF_MMU_CMD + 8), regs::VIV_FE_END_HEADER_OP_END);
}
