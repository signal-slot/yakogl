// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Host tests of the render pool's free lists, on a carveout in host memory
//! and the real bump ([`super::super::mem::bo_alloc`]), so the shared
//! physical cursor is the driver's own.
//!
//! First the proof of the port: the golden trace the C pool produced
//! (`golden.rs`), replayed to equality. Then the shapes of the C checks this
//! module replaces (tools/pvr/render_pool_check.c 1-7 and driver/tools/pooltest's
//! invariant cases). The last group is new: what the C checks could not see,
//! because they gave each heap its own memory.

use super::*;
use crate::gpu::pvr::km::{YdPvrKm, YdPvrKmOps};
use crate::gpu::pvr::mmu;
use core::ffi::c_void;
use std::boxed::Box;
use std::cell::RefCell;
use std::string::String;
use std::vec::Vec;

const GEN: usize = 0;
const USC: usize = 2;
const P: u32 = 0x1000;

thread_local! {
    static LOG: RefCell<Vec<(u32, String)>> = const { RefCell::new(Vec::new()) };
}

unsafe extern "C" fn nop_range(_: *mut c_void, _: *const c_void, _: usize) {}
unsafe extern "C" fn nop_fence(_: *mut c_void) {}
unsafe extern "C" fn nop_wait(_: *mut c_void, _: u32) {}
unsafe extern "C" fn log(_: *mut c_void, lvl: u32, buf: *const u8, len: usize) {
    // SAFETY: the crate passes `len` valid bytes.
    let s = unsafe { core::slice::from_raw_parts(buf, len) };
    LOG.with(|l| l.borrow_mut().push((lvl, String::from_utf8_lossy(s).into_owned())));
}

fn ops() -> Box<YdPvrKmOps> {
    Box::new(YdPvrKmOps {
        ctx: core::ptr::null_mut(),
        flush: nop_range,
        invd: nop_range,
        fence: nop_fence,
        busy_wait_us: nop_wait,
        log,
    })
}

/// A carveout of `MIB` MiB in host memory (the golden trace's: `GOLDEN_MIB`):
/// page tables in its first MiB, the render pool after it.
const MIB: usize = 8;
const GOLDEN_MIB: usize = 32;
const POOL_OFF: u32 = 0x10_0000;

struct Rig {
    _mem: Vec<u64>,
    _ops: Box<YdPvrKmOps>,
    k: Box<YdPvrKm>,
    pool: Box<YdPvrPool>,
    bumps: Box<[u32; POOL_HEAPS]>,
}

impl Rig {
    fn new() -> Rig {
        Rig::with_mib(MIB)
    }

    fn with_mib(mib: usize) -> Rig {
        Rig::with_layout(mib * 1024 * 1024, POOL_OFF, 0)
    }

    /// A carveout of `bytes` with the render pool from `pool_off` and the
    /// top region's threshold `top_above` (0: off, the other rigs' setting).
    fn with_layout(bytes: usize, pool_off: u32, top_above: u32) -> Rig {
        LOG.with(|l| l.borrow_mut().clear());
        let mut mem = std::vec![0u64; bytes / 8];
        // SAFETY: both are plain data for which all-zero is the documented
        // "before the first call" state (null pointers, zero counters).
        let (mut k, pool): (Box<YdPvrKm>, Box<YdPvrPool>) =
            unsafe { (Box::new(core::mem::zeroed()), Box::new(core::mem::zeroed())) };
        k.cpu = mem.as_mut_ptr() as *mut u8;
        k.phys = 0x9000_0000;
        k.size = bytes as u64;
        let ops = ops();
        k.ops = &*ops;
        k.pt_area_off = P;
        k.pt_area_end = pool_off;
        k.pt_next = P;
        k.pool_off = pool_off;
        k.pool_next = pool_off;
        k.pool_top_above = top_above;
        mmu::init(&mut k);
        Rig { _mem: mem, _ops: ops, k, pool, bumps: Box::new([0; POOL_HEAPS]) }
    }

    fn b(&mut self) -> Bumps {
        let p = self.bumps.as_mut_ptr();
        [p, p.wrapping_add(1), p.wrapping_add(2)]
    }

    fn try_alloc(&mut self, heap: usize, size: u32) -> Option<PvrGpuBo> {
        let b = self.b();
        self.pool.alloc(&mut self.k, &b, heap, size)
    }

    fn alloc(&mut self, heap: usize, size: u32) -> PvrGpuBo {
        self.try_alloc(heap, size).expect("an allocation")
    }

    fn free_rb(&mut self, bo: &PvrGpuBo, size: u32, rollback: bool) {
        let b = self.b();
        self.pool.free(&mut self.k, &b, bo, size, rollback);
    }

    fn free(&mut self, bo: &PvrGpuBo, size: u32) {
        self.free_rb(bo, size, false);
    }

    fn free_of(&mut self, heap: usize) -> FreeOf {
        let b = self.b();
        self.pool.free_of(&self.k, &b, heap)
    }

    /// Bytes the physical cursor has handed out.
    fn pool_used(&self) -> u32 {
        self.k.pool_next - self.k.pool_off
    }

    fn table_used(&self, heap: usize) -> usize {
        self.pool.free[heap].iter().filter(|s| s.size != 0).count()
    }

    /// The chain's entries, head first (CPU address of each).
    fn chain(&self, heap: usize) -> Vec<*mut u8> {
        let mut v = Vec::new();
        let mut c = self.pool.ovf[heap];
        while !c.is_null() && v.len() < self.pool.ovf_n[heap] as usize {
            v.push(c);
            c = OvfHdr::read(c).next;
        }
        v
    }

    fn logs(&self) -> Vec<(u32, String)> {
        LOG.with(|l| l.borrow().clone())
    }

    /// Conservation for one heap: live + free + dropped = the heap's bump
    /// (VA bytes); no two free ranges overlap, and none touch in both VA
    /// and CPU space (those must have been merged).
    fn conserve(&mut self, heap: usize, live: u64) {
        let f = self.free_of(heap);
        let dropped = u64::from(self.pool.dropped_kib[heap]) * 1024;
        let top = if heap == GEN { u64::from(self.k.pool_top) } else { 0 };
        assert_eq!(live + f.bytes + dropped, u64::from(self.bumps[heap]) + top, "heap {heap}: bytes not conserved");
        let mut r: Vec<(u32, u32, usize)> =
            self.pool.free[heap].iter().filter(|s| s.size != 0).map(|s| (s.hb, s.size, s.cpu as usize)).collect();
        for c in self.chain(heap) {
            let h = OvfHdr::read(c);
            r.push((h.hb, h.size, c as usize));
        }
        r.sort();
        for w in r.windows(2) {
            assert!(w[0].0 + w[0].1 <= w[1].0, "heap {heap}: free ranges overlap: {w:?}");
            let touch = w[0].0 + w[0].1 == w[1].0 && w[0].2 + w[0].1 as usize == w[1].2;
            assert!(!touch, "heap {heap}: two free ranges were left unmerged: {w:?}");
        }
    }
}

/// 2*N one-page blocks, every other one freed.
fn holes(r: &mut Rig, n: usize) -> Vec<PvrGpuBo> {
    let blk: Vec<PvrGpuBo> = (0..2 * n).map(|_| r.alloc(GEN, P)).collect();
    for i in 0..n {
        r.free(&blk[2 * i], P);
    }
    blk
}

/* ---- the port's proof: the C pool's golden trace --------------------------- */

/// Every operation of `golden::TRACE`, replayed on the Rust pool with the C
/// generator's carveout, gives the C pool's answer: the same return code,
/// heap offset and carveout offset for an allocation, and after every
/// operation the same physical cursor, the same three device-VA cursors,
/// the same chain lengths and the same drop, double-free, bad-header and
/// rollback counts.
#[test]
fn the_rust_pool_replays_the_c_pools_golden_trace() {
    replay_the_golden_trace(0);
}

/// The same with the top region on at the driver's threshold: the trace's
/// two GENERAL requests of more than 256 KiB (32 MiB, the whole carveout;
/// 1 MiB once everything is free and merged) are refused and served as the
/// C pool did, and nothing is ever placed in the top region.
#[test]
fn with_the_top_region_on_the_golden_trace_replays_to_equality() {
    let top = replay_the_golden_trace(YD_PVR_POOL_TOP_ABOVE);
    assert_eq!(top, (0, 0), "the top region's bytes now and at most");
}

/// `include/yakogl_drivers.h`'s `YD_PVR_POOL_TOP_ABOVE`, what the driver sets.
const YD_PVR_POOL_TOP_ABOVE: u32 = 256 * 1024;

/// Replay `golden::TRACE` with the top region's threshold `top_above`; the top
/// region's bytes at the end, now and at most.
fn replay_the_golden_trace(top_above: u32) -> (u32, u32) {
    use super::golden::TRACE;
    let mut r = Rig::with_layout(GOLDEN_MIB * 1024 * 1024, POOL_OFF, top_above);
    let mut blk: Vec<PvrGpuBo> = Vec::new();
    let base = r.k.cpu as usize;
    let (mut allocs, mut frees, mut pokes) = (0, 0, 0);
    for (n, (kind, w)) in TRACE.iter().enumerate() {
        let (mut rc, mut off, mut cpu) = (0i64, 0i64, 0i64);
        match kind {
            b'A' => {
                allocs += 1;
                match r.try_alloc(w[0] as usize, w[1] as u32) {
                    Some(bo) => {
                        (off, cpu) = (i64::from(bo.heap_off), (bo.cpu as usize - base) as i64);
                        blk.push(bo);
                    }
                    None => rc = -12,
                }
            }
            b'F' => {
                frees += 1;
                let mut bo = blk[w[0] as usize];
                if w[2] & 2 != 0 {
                    bo.cpu = core::ptr::null_mut();
                }
                if w[2] & 4 != 0 {
                    bo.gpu = 0x1234_0000;
                }
                r.free_rb(&bo, w[1] as u32, w[2] & 1 != 0);
            }
            b'P' => {
                pokes += 1;
                let c = r.chain(w[0] as usize)[w[1] as usize];
                // SAFETY: a free range of the pool's.
                unsafe { c.write(0xEE) };
            }
            _ => panic!("operation {n}: unknown kind {kind}"),
        }
        let p = &r.pool;
        let got = [
            w[0],
            w[1],
            w[2],
            rc,
            off,
            cpu,
            i64::from(r.k.pool_next),
            i64::from(r.bumps[0]),
            i64::from(r.bumps[1]),
            i64::from(r.bumps[2]),
            i64::from(p.ovf_n[0]),
            i64::from(p.ovf_n[1]),
            i64::from(p.ovf_n[2]),
            i64::from(p.dropped_kib.iter().sum::<u32>()),
            i64::from(p.double_frees),
            i64::from(p.ovf_bad),
            i64::from(p.rolled_back_kib.iter().sum::<u32>()),
        ];
        assert_eq!(got, *w, "operation {n} ({}) differs from the C pool", *kind as char);
    }
    /* the population: the trace is what the generator printed, all of it */
    assert_eq!((TRACE.len(), allocs, frees, pokes), (2769, 1384, 1384, 1), "the golden trace's population");
    (r.k.pool_top, r.k.pool_top_hw)
}

/* ---- the C checks' shapes ------------------------------------------------- */

/// The frees past the table go on the chain and none is dropped. Freeing
/// the rest merges it all back into one range, and a block of that size is
/// served from it.
#[test]
fn a_full_table_chains_and_merging_gives_one_range() {
    let mut r = Rig::new();
    let n = 2 * POOL_FREE_SLOTS;
    let blk = holes(&mut r, n);
    assert_eq!(r.pool.dropped_kib, [0; 4]);
    assert_eq!(r.table_used(GEN), POOL_FREE_SLOTS);
    assert_eq!(r.pool.ovf_n[GEN] as usize, n - POOL_FREE_SLOTS);
    assert_eq!(r.pool.slots_peak[GEN] as usize, POOL_FREE_SLOTS);
    assert_eq!(r.pool.ovf_peak[GEN] as usize, n - POOL_FREE_SLOTS);
    r.conserve(GEN, (n as u64) * u64::from(P));
    for i in 0..n {
        r.free(&blk[2 * i + 1], P);
    }
    assert_eq!((r.pool.ovf_n[GEN], r.table_used(GEN)), (0, 1));
    let used = r.pool_used();
    let all = r.alloc(GEN, 2 * n as u32 * P);
    assert_eq!((all.heap_off, r.pool_used()), (0, used), "served from the merged range, not the bump");
}

/// A double free is refused and counted, never dropped, whether the free
/// range sits in the table or on the chain.
#[test]
fn a_double_free_is_refused_in_the_table_and_on_the_chain() {
    let mut r = Rig::new();
    let n = POOL_FREE_SLOTS + 8;
    let blk = holes(&mut r, n);
    let chained = r.pool.ovf_n[GEN];
    r.free(&blk[0], P); /* in the table */
    r.free(&blk[2 * (n - 1)], P); /* on the chain (the newest free) */
    assert_eq!(r.pool.double_frees, 2);
    assert_eq!(r.pool.ovf_n[GEN], chained);
    assert_eq!(r.pool.dropped_kib, [0; 4]);
    assert_eq!(r.logs().iter().filter(|(_, l)| l.contains("double free")).count(), 2);
}

/// Best fit reaches the chain: a request only a chained range can serve
/// gets that range, zeroed, and the rest of it stays free.
#[test]
fn best_fit_reaches_the_chain_and_keeps_the_rest() {
    let mut r = Rig::new();
    let blk: Vec<PvrGpuBo> = (0..2 * POOL_FREE_SLOTS).map(|_| r.alloc(GEN, P)).collect();
    let three = r.alloc(GEN, 3 * P);
    let _guard = r.alloc(GEN, P);
    for i in 0..POOL_FREE_SLOTS {
        r.free(&blk[2 * i], P); /* the table: one-page ranges only */
    }
    // SAFETY: a live block of ours, 3 pages.
    unsafe { core::ptr::write_bytes(three.cpu, 0xA5, 3 * P as usize) };
    r.free(&three, 3 * P);
    assert_eq!(r.pool.ovf_n[GEN], 1, "the table is full: the three pages are chained");
    let two = r.alloc(GEN, 2 * P);
    assert_eq!(two.heap_off, three.heap_off);
    // SAFETY: the two pages just handed out.
    let bytes = unsafe { core::slice::from_raw_parts(two.cpu, 2 * P as usize) };
    assert!(bytes.iter().all(|&x| x == 0), "a range from the chain is handed out zeroed");
    assert_eq!(r.free_of(GEN).ranges as usize, POOL_FREE_SLOTS + 1, "its last page stays free");
}

/// A table slot that empties takes the chain's head.
#[test]
fn an_emptied_slot_takes_the_chains_head() {
    let mut r = Rig::new();
    holes(&mut r, POOL_FREE_SLOTS + 3);
    assert_eq!(r.pool.ovf_n[GEN], 3);
    let _ = r.alloc(GEN, P);
    assert_eq!((r.pool.ovf_n[GEN], r.table_used(GEN)), (2, POOL_FREE_SLOTS));
}

/// A chain header overwritten while free: the chain is cut there, counted,
/// and no range at or past it is ever handed out.
#[test]
fn an_overwritten_chain_header_cuts_the_chain() {
    let mut r = Rig::new();
    holes(&mut r, POOL_FREE_SLOTS + 4);
    let chain = r.chain(GEN);
    assert_eq!(chain.len(), 4);
    /* a stale GPU write into the second entry */
    // SAFETY: a free page of ours.
    unsafe { chain[1].write(0xEE) };
    let f = r.free_of(GEN);
    assert_eq!(r.pool.ovf_bad, 1);
    assert_eq!(r.pool.ovf_n[GEN], 1, "only the entry before the bad one stays");
    assert_eq!(f.ranges as usize, POOL_FREE_SLOTS + 1);
    for _ in 0..2 * POOL_FREE_SLOTS {
        let bo = r.alloc(GEN, P);
        assert!(!chain[1..].contains(&bo.cpu), "a range at or past the cut was handed out");
    }
    assert!(r.logs().iter().any(|(lvl, l)| *lvl == 2 && l.contains("overwritten while free")));
}

/// A, C, B freed in that order end as one range.
#[test]
fn neighbours_on_both_sides_are_absorbed() {
    let mut r = Rig::new();
    let a = r.alloc(GEN, 0x10000);
    let b = r.alloc(GEN, 0x10000);
    let c = r.alloc(GEN, 0x10000);
    let _guard = r.alloc(GEN, P);
    r.free(&a, 0x10000);
    r.free(&c, 0x10000);
    r.free(&b, 0x10000);
    let f = r.free_of(GEN);
    assert_eq!((f.ranges, f.largest), (1, 0x30000));
}

/// Random frees and allocations with the chain in use: bytes conserved, no
/// overlap, nothing left unmerged, nothing dropped.
#[test]
fn a_random_run_conserves_the_pool() {
    let mut r = Rig::with_mib(GOLDEN_MIB);
    let mut live: Vec<(PvrGpuBo, u32)> = Vec::new();
    let mut seed = 0x2545_F491_4F6C_DD1Du64;
    let mut rnd = |m: u64| {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed % m
    };
    /* two thousand blocks first, so that random frees leave more separate
     * holes than the table holds */
    for step in 0..12000 {
        if step >= 2000 && live.len() > 2 && rnd(100) < 50 {
            let (bo, sz) = live.swap_remove(rnd(live.len() as u64) as usize);
            r.free(&bo, sz);
        } else {
            let sz = (1 + rnd(6) as u32) * P - rnd(2) as u32 * 0x800;
            if let Some(bo) = r.try_alloc(GEN, sz) {
                live.push((bo, sz));
            }
        }
        let held: u64 = live.iter().map(|(_, s)| round_up(u64::from(*s), 0x1000)).sum();
        r.conserve(GEN, held);
    }
    assert!(
        r.pool.ovf_peak[GEN] > 0,
        "the run never reached the chain (table peak {} of {POOL_FREE_SLOTS})",
        r.pool.slots_peak[GEN]
    );
    assert_eq!((r.pool.dropped_kib, r.pool.double_frees, r.pool.ovf_bad), ([0; 4], 0, 0));
}

/// With rollback on, the newest block goes back to both cursors; with it
/// off (the default), to the list.
#[test]
fn rollback_returns_the_newest_block_to_the_bump_only_when_on() {
    let mut r = Rig::new();
    let _a = r.alloc(GEN, P);
    let b = r.alloc(GEN, 2 * P);
    r.free(&b, 2 * P);
    assert_eq!((r.pool_used(), r.bumps[GEN]), (3 * P, 3 * P), "off: the bump stays");
    let b = r.alloc(GEN, 2 * P);
    r.free_rb(&b, 2 * P, true);
    assert_eq!((r.pool_used(), r.bumps[GEN]), (P, P), "on: both cursors rewind");
    assert_eq!(r.pool.rolled_back_kib[GEN], 8);
    assert_eq!(r.pool.pool_hw, POOL_OFF + 3 * P, "the high-water outlives the rollback");
}

/// Frees the pool cannot take back are counted per heap, not lost silently.
#[test]
fn foreign_and_unmapped_blocks_are_counted_as_dropped() {
    let mut r = Rig::new();
    let a = r.alloc(GEN, P);
    let unmapped = PvrGpuBo { cpu: core::ptr::null_mut(), ..a };
    r.free(&unmapped, P);
    let foreign = PvrGpuBo { gpu: 0x1234_0000, ..a };
    r.free(&foreign, 2 * P);
    assert_eq!(r.pool.dropped_kib, [4, 0, 0, 8]);
}

/// pooltest 0: a refused allocation moves neither cursor (the bound is
/// checked before either moves; a bump-first edit would let the VA cursor
/// drift past the physical one).
#[test]
fn a_refused_allocation_moves_no_cursor() {
    let mut r = Rig::new();
    let _a = r.alloc(GEN, P);
    let (va, phys) = (r.bumps[GEN], r.k.pool_next);
    assert!(r.try_alloc(GEN, (MIB as u32) * 0x10_0000).is_none());
    assert_eq!((r.bumps[GEN], r.k.pool_next), (va, phys));
    assert_eq!(r.pool.refused[GEN], 1);
}

/// pooltest 1 and 2: the same size freed and taken again lands on the same
/// block without bumping; a small take out of a big free block is followed
/// by the adjacent remainder.
#[test]
fn exact_reuse_and_split() {
    let mut r = Rig::new();
    let a = r.alloc(GEN, 0x4000);
    let used = r.pool_used();
    r.free(&a, 0x4000);
    let b = r.alloc(GEN, 0x4000);
    assert_eq!((b.gpu, r.pool_used()), (a.gpu, used));
    let big = r.alloc(USC, 0x10000);
    r.free(&big, 0x10000);
    let s1 = r.alloc(USC, P);
    let s2 = r.alloc(USC, P);
    assert_eq!((s1.heap_off, s2.heap_off), (big.heap_off, big.heap_off + P));
    assert_eq!(r.pool.reused, [1, 0, 2]);
}

/// pooltest 3: after a refused double free, two allocations of the range's
/// size never get the same address.
#[test]
fn a_double_free_never_hands_one_range_to_two_callers() {
    let mut r = Rig::new();
    let a = r.alloc(USC, 0x2000);
    r.free(&a, 0x2000);
    r.free(&a, 0x2000);
    let x = r.alloc(USC, P);
    let y = r.alloc(USC, P);
    assert_ne!(x.gpu, y.gpu);
    assert_eq!(r.pool.double_frees, 1);
}

/// pooltest 4, on the driver's shared physical cursor: random sizes over all
/// three heaps, interleaved; every heap's bytes conserved after every step.
#[test]
fn a_random_run_over_three_heaps_conserves_each_heap() {
    const SIZES: [u32; 7] = [0x1000, 0x1800, 0x2000, 0x4000, 0x9000, 0x10000, 0x30000];
    let mut r = Rig::new();
    let mut live: Vec<(PvrGpuBo, u32, usize)> = Vec::new();
    let mut seed = 0x9E37_79B9_7F4A_7C15u64;
    let mut rnd = |m: u64| {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed % m
    };
    for _ in 0..6000 {
        if live.len() < 200 && (live.is_empty() || rnd(2) == 0) {
            let heap = rnd(POOL_HEAPS as u64) as usize;
            let sz = SIZES[rnd(SIZES.len() as u64) as usize];
            if let Some(bo) = r.try_alloc(heap, sz) {
                live.push((bo, sz, heap));
            }
        } else {
            let (bo, sz, _) = live.swap_remove(rnd(live.len() as u64) as usize);
            r.free(&bo, sz);
        }
        for h in 0..POOL_HEAPS {
            let held: u64 = live.iter().filter(|l| l.2 == h).map(|(_, s, _)| round_up(u64::from(*s), 0x1000)).sum();
            r.conserve(h, held);
        }
    }
    assert_eq!((r.pool.dropped_kib, r.pool.double_frees, r.pool.ovf_bad), ([0; 4], 0, 0));
}

/* ---- what the C checks could not see -------------------------------------- */

/// Mechanism 3 of docs/notes/pvr-render-pool.md, on the
/// driver's own cursors: GENERAL A, a USC page, GENERAL B are allocated in turn
/// (a program compiled between two buffers), then all three are freed. A and B
/// are adjacent in the GENERAL heap's VA but not in the carveout, so they
/// never merge, and a request of A + B bytes goes to the bump although
/// A + B bytes of GENERAL are free.
///
/// This pins today's behaviour, which is wrong for a pool that has to hold
/// glmark2's terrain. The symptom: the physical cursor grows by the request
/// with as many bytes free. The fix waits for the board's pool summary
/// (which of the three mechanisms drains the pool on glmark2); when it
/// lands, this test becomes the fix's test.
#[test]
fn known_wrong_a_block_of_another_heap_in_between_splits_the_free_space_for_good() {
    let mut r = Rig::new();
    let a = r.alloc(GEN, 0x10_0000);
    let u = r.alloc(USC, P);
    let b = r.alloc(GEN, 0x10_0000);
    assert_eq!(b.heap_off, a.heap_off + 0x10_0000, "VA-adjacent in GENERAL");
    assert_ne!(b.cpu, a.cpu.wrapping_add(0x10_0000), "not CPU-adjacent: the USC page sits between");
    r.free(&a, 0x10_0000);
    r.free(&u, P);
    r.free(&b, 0x10_0000);
    let f = r.free_of(GEN);
    assert_eq!(
        (f.bytes, f.ranges, f.largest),
        (0x20_0000, 2, 0x10_0000),
        "known wrong: 2 MiB of GENERAL free in two pieces that never merge"
    );
    let used = r.pool_used();
    let big = r.alloc(GEN, 0x20_0000);
    assert_eq!(
        r.pool_used(),
        used + 0x20_0000,
        "known wrong: the 2 MiB request bumps the physical cursor by 2 MiB with 2 MiB of GENERAL free \
         (waits for the board's pool summary to choose the fix)"
    );
    assert_eq!(big.heap_off, 0x20_0000);
}

/* ---- the top region --------------------------------------------------------- */

const MIB_B: u32 = 0x10_0000;

/// Requests of more than 1 MiB less a page are large here.
fn top_rig() -> Rig {
    Rig::with_layout(MIB * 1024 * 1024, POOL_OFF, MIB_B - P)
}

/// A large GENERAL block comes from the carveout's end, mapped at
/// `TOP_VA + co`; a small one and every PDS/USC block from the bump.
#[test]
fn a_large_general_block_comes_from_the_top_and_a_small_one_from_the_bump() {
    let mut r = top_rig();
    let small = r.alloc(GEN, MIB_B - P);
    let big = r.alloc(GEN, MIB_B);
    let usc = r.alloc(USC, 2 * MIB_B);
    let end = (MIB * 1024 * 1024) as u32;
    assert_eq!(small.heap_off, 0, "below the threshold: the bump");
    assert_eq!(usc.heap_off, 0, "another heap: the bump");
    assert_eq!(big.heap_off, TOP_VA + end - MIB_B, "the carveout's last MiB");
    assert_eq!(big.cpu, r.k.cpu_at(u64::from(end - MIB_B)));
    assert_eq!(big.gpu, GENERAL_HEAP_BASE + u64::from(TOP_VA) + u64::from(end - MIB_B));
    assert_eq!((r.k.pool_top, r.k.pool_top_hw), (MIB_B, MIB_B));
    for pg in [0, MIB_B / 2, MIB_B - P] {
        let pte = mmu::lookup(&r.k, big.gpu + u64::from(pg));
        assert_eq!(
            pte & 0xFF_FFFF_F000,
            r.k.phys + u64::from(end - MIB_B + pg),
            "page +0x{pg:x} maps its own carveout page"
        );
    }
    assert_eq!((r.pool.bumped[GEN], r.pool.reused[GEN]), (2, 0));
    assert_eq!(r.pool_used(), MIB_B - P + 2 * MIB_B, "the bump took the small GENERAL block and the USC one");
}

/// The top region's blocks merge in both spaces, and a range that reaches
/// the region's floor goes back to the gap: freed in any order, three large
/// blocks leave the gap whole and no free range. Allocated again, the pages
/// are mapped already: no page-table page is taken, no mapping changes.
#[test]
fn freed_top_blocks_merge_and_give_the_gap_back_whole() {
    let mut r = top_rig();
    let gap = r.k.pool_gap();
    let a = r.alloc(GEN, MIB_B);
    let b = r.alloc(GEN, 2 * MIB_B);
    let c = r.alloc(GEN, MIB_B + P);
    assert_eq!(r.k.pool_gap(), gap - u64::from(4 * MIB_B + P));
    r.free(&a, MIB_B); /* the highest: a range on the list */
    assert_eq!((r.free_of(GEN).ranges, r.k.pool_top), (1, 4 * MIB_B + P));
    r.free(&c, MIB_B + P); /* at the floor: back to the gap */
    assert_eq!((r.free_of(GEN).ranges, r.k.pool_top), (1, 3 * MIB_B));
    r.free(&b, 2 * MIB_B); /* merges with a, reaches the floor */
    assert_eq!((r.free_of(GEN).ranges, r.k.pool_top, r.k.pool_gap()), (0, 0, gap));
    assert_eq!(r.k.pool_top_hw, 4 * MIB_B + P);
    let pt = r.k.pt_next;
    r.k.mmu_dirty = 0;
    let again = r.alloc(GEN, 3 * MIB_B);
    assert_eq!(again.heap_off, TOP_VA + (MIB * 1024 * 1024) as u32 - 3 * MIB_B);
    assert_eq!((r.k.pt_next, r.k.mmu_dirty), (pt, 0), "no page mapped again");
}

/// A large request takes a free range of its size before it moves the
/// floor: the next scene's large blocks reuse the last one's.
#[test]
fn a_large_request_reuses_a_freed_top_range_first() {
    let mut r = top_rig();
    let a = r.alloc(GEN, 2 * MIB_B);
    let _keep = r.alloc(GEN, MIB_B);
    r.free(&a, 2 * MIB_B);
    let top = r.k.pool_top;
    let b = r.alloc(GEN, 2 * MIB_B - P);
    assert_eq!((b.heap_off, r.k.pool_top), (a.heap_off, top));
    assert_eq!(r.pool.reused[GEN], 1);
}

/// The two ends share the gap: the bump stops at the top region's floor
/// (its line says what is left), and a small request the bump cannot serve
/// takes a free range of the top region rather than failing.
#[test]
fn the_bump_stops_at_the_floor_and_a_small_request_falls_back_to_a_top_range() {
    let mut r = top_rig();
    let pool = (MIB * 1024 * 1024) as u32 - POOL_OFF;
    let big = r.alloc(GEN, pool / 2);
    let _keep = r.alloc(GEN, MIB_B);
    let rest = r.k.pool_gap() as u32;
    let _fill = r.alloc(USC, rest);
    assert_eq!(r.k.pool_gap(), 0);
    assert!(r.try_alloc(USC, P).is_none(), "nothing between the bump and the floor");
    assert!(r.logs().iter().any(|(_, l)| l == "render pool exhausted: 4 KiB requested, 0 KiB left"));
    r.free(&big, pool / 2);
    let small = r.alloc(GEN, P);
    assert_eq!(small.heap_off, big.heap_off, "the freed top range served it");
    r.conserve(GEN, u64::from(MIB_B + P));
}

/// Frees of ranges the top region never handed out are dropped and counted,
/// as past the bump: an address below the floor belongs to no heap, and a
/// size that runs past the carveout's end is GENERAL's.
#[test]
fn a_free_outside_the_top_region_is_dropped() {
    let mut r = top_rig();
    let big = r.alloc(GEN, MIB_B);
    let below = PvrGpuBo {
        gpu: big.gpu - u64::from(MIB_B),
        cpu: big.cpu.wrapping_sub(MIB_B as usize),
        heap_off: big.heap_off - MIB_B,
    };
    r.free(&below, MIB_B);
    r.free(&big, 2 * MIB_B);
    assert_eq!(r.pool.dropped_kib, [2048, 0, 0, 1024]);
    assert_eq!(r.k.pool_top, MIB_B, "nothing given back");
}

/// Random large and small blocks with the top region on: every heap's bytes
/// conserved (the top region's counted with the bump's), no overlap, nothing
/// left unmerged; with everything freed, the top region is all back in the
/// gap.
#[test]
fn a_random_run_with_the_top_region_conserves_and_ends_whole() {
    const SIZES: [u32; 8] = [0x1000, 0x3000, 0x10000, 0x8_0000, 0x10_0000, 0x18_0000, 0x30_0000, 0x7_6000];
    let mut r = Rig::with_layout(GOLDEN_MIB * 1024 * 1024, POOL_OFF, YD_PVR_POOL_TOP_ABOVE);
    let mut live: Vec<(PvrGpuBo, u32, usize)> = Vec::new();
    let mut seed = 0xD1B5_4A32_D192_ED03u64;
    let mut rnd = |m: u64| {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed % m
    };
    let mut tops = 0;
    for _ in 0..8000 {
        if live.len() < 60 && (live.is_empty() || rnd(2) == 0) {
            let heap = if rnd(4) == 0 { USC } else { GEN };
            let sz = SIZES[rnd(SIZES.len() as u64) as usize] - rnd(2) as u32 * 0x800;
            if let Some(bo) = r.try_alloc(heap, sz) {
                tops += usize::from(bo.heap_off >= TOP_VA);
                live.push((bo, sz, heap));
            }
        } else {
            let (bo, sz, _) = live.swap_remove(rnd(live.len() as u64) as usize);
            r.free(&bo, sz);
        }
        for h in [GEN, USC] {
            let held: u64 = live.iter().filter(|l| l.2 == h).map(|(_, s, _)| round_up(u64::from(*s), 0x1000)).sum();
            r.conserve(h, held);
        }
    }
    eprintln!("random run: {tops} blocks placed in the top region");
    assert!(tops > 100, "the run placed only {tops} blocks in the top region");
    for (bo, sz, _) in live.drain(..) {
        r.free(&bo, sz);
    }
    assert_eq!(r.k.pool_top, 0, "the top region is all back in the gap");
    assert_eq!((r.pool.dropped_kib, r.pool.double_frees, r.pool.ovf_bad), ([0; 4], 0, 0));
}

/* ---- the summary ---------------------------------------------------------- */

/// An exhausted pool says so in the bump's line, then prints the summary at
/// ERR: the free bytes, the largest free range, the high-water, and what
/// happened to every allocation and free so far.
#[test]
fn an_exhaustion_prints_the_summary_after_the_bumps_line() {
    let mut r = Rig::new();
    let a = r.alloc(GEN, 0x20_0000);
    let _u = r.alloc(USC, P);
    let _b = r.alloc(GEN, 0x10_0000);
    r.free(&a, 0x20_0000);
    let want = (MIB as u32) * 0x10_0000; /* more than the carveout */
    assert!(r.try_alloc(GEN, want).is_none());
    let logs = r.logs();
    let i = logs.iter().position(|(_, l)| l.starts_with("render pool exhausted:")).expect("the bump's line");
    let got: Vec<&(u32, String)> = logs[i + 1..].iter().collect();
    assert!(got.iter().all(|(lvl, _)| *lvl == 1), "ERR");
    let got: Vec<&str> = got.iter().map(|(_, l)| l.as_str()).collect();
    assert_eq!(
        got,
        [
            "render pool summary: 7168 KiB, bump 3076 KiB now, 3076 KiB high-water",
            "render pool general: free 2048 KiB in 1 ranges, largest 2048 KiB; 0 reused, 2 bumped, 1 refused; 1 frees (2048 KiB)",
            "render pool pds: free 0 KiB in 0 ranges, largest none; 0 reused, 0 bumped, 0 refused; 0 frees (0 KiB)",
            "render pool usc: free 0 KiB in 0 ranges, largest none; 0 reused, 1 bumped, 0 refused; 0 frees (0 KiB)",
            "render pool losses: dropped 0 KiB, 0 double frees, 0 bad chain headers, rolled back 0 KiB",
        ]
    );
}

/// The run-end report is the same five lines at INF.
#[test]
fn the_run_end_report_is_the_summary_at_inf() {
    let mut r = Rig::new();
    let b = r.b();
    r.pool.report(&r.k, &b, false);
    let logs = r.logs();
    assert_eq!(logs.len(), 5);
    assert!(logs.iter().all(|(lvl, _)| *lvl == 3), "INF");
    assert_eq!(logs[0].1, "render pool summary: 7168 KiB, bump 0 KiB now, 0 KiB high-water");
}

mod replay;
