// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The render pool's free lists, one per heap the render service serves
//! (GENERAL, PDS, USC), and the pool's summary.
//!
//! [`super::mem::bo_alloc`] bumps two cursors: the pool's physical cursor
//! (one for every heap) and the heap's device-VA cursor. It maps the pages
//! it hands out, and nothing is ever unmapped. A GL application frees and
//! re-creates objects all the time, so freed blocks go to these lists and
//! are handed out first. A block keeps its mapping, so reuse needs only
//! zeroing. On free, a block is merged with its neighbours on both sides
//! when they are contiguous in both VA and CPU space. That is not always
//! true, because the physical cursor is shared: a block of another heap
//! allocated in between splits them
//! (docs/notes/pvr-render-pool.md, mechanism 3).
//!
//! Before 2026-10-04 this was C (`driver/src/pvr_render_pool.h`, a header so
//! that two C host checks compiled the shipped code). It is ported here
//! decision for decision, and proven so by a golden trace: one deterministic
//! sequence of allocations, frees and a chain-header overwrite, run once
//! through the C pool and recorded (`pool/golden.rs`, with the program that
//! made it), which the Rust pool replays to equality (`pool/tests.rs`). The
//! other host tests replace the two C checks and model the shared physical
//! cursor, which they did not.
//!
//! # The top region
//!
//! A GENERAL request of more than `pool_top_above` bytes ([`YdPvrKm`],
//! 256 KiB in the driver) is a large one: a render target, its depth buffer,
//! a texture, a scene's vertex buffer. It is placed from the carveout's end
//! downward ([`super::mem::bo_alloc_top`]), not from the bump. Small blocks
//! (programs, uniforms, HWRT sets, the freelists) keep coming from the bump,
//! below. Both ends draw on the one gap between them.
//!
//! Why: on the bump alone, a small block that outlives a scene lands between
//! that scene's large ones, and after the scene they are free in pieces no
//! later large request fits. glmark2's refract (a 7600 KiB depth buffer after
//! shadow and terrain) met 11.6 MiB free with no range over 3936 KiB
//! (docs/notes/pvr-render-pool.md, "the top region"). In the
//! top region the large blocks lie only beside each other, so a scene's frees
//! merge back into one range; a range that reaches the region's floor goes back
//! to the gap at once, so the gap stays one piece for either end.
//!
//! A top block at carveout offset `co` is mapped at heap offset
//! [`TOP_VA`]` + co`: adjacency in VA is adjacency in the carveout, so the
//! merge rule (contiguous in both) holds for every pair, and a page's
//! mapping never changes. The order:
//! - a large request: best fit over the top region's ranges, then over the
//!   ranges below, then the gap (the floor moves down), then the bump's
//!   refusal (its line, the summary);
//! - a small one: best fit over the ranges below the top region, then the
//!   bump, then a range of the top region (only when the bump is out).
//!
//! With nothing in the top region both orders are the old one (best fit,
//! then a cursor), so the C pool's golden trace replays to equality with
//! the region on (its large requests: 1 MiB from a merged range, and a
//! refused 32 MiB). Where the two differ, `pool/tests/replay.rs` replays
//! glmark2's scenes on the AM62P's layout, both ways.
//!
//! # The table and the overflow chain
//!
//! Each heap has [`POOL_FREE_SLOTS`] table slots (the fast path). A free
//! range that finds the table full goes on the heap's overflow chain. It is
//! never dropped: before 2026-09-29 it was, and its pages were lost for the
//! life of the module (`driver/tools/pvrlab/user/pool_frag.c`, retired with
//! pvrgpu.ko; in the history at 8b53557f). A chained
//! range is described by a header written into its own first bytes. That is
//! always possible: every range here is pool memory with a CPU mapping and
//! at least one page long. So the chain needs no storage of its own and
//! cannot fill. The header is checked every time it is read, before it is
//! followed: the magic, a sum over its fields, and a page-aligned range
//! under the bump. This makes it a canary. Nothing but this code writes a
//! free range, so a bad header means a stale GPU write into freed memory.
//! The chain is cut there, and the event is counted ("bad chain headers");
//! a bad header is never followed.
//!
//! # Counters
//!
//! A silent drop is how a PDS/USC leak stayed invisible once, so every
//! outcome is counted on its own: dropped (KiB the pool could not take
//! back), double frees (refused, not dropped: the bytes are still on the
//! list), rolled back (KiB given back to the bump), frees taken back, and
//! allocations served from a list, from the bump, or refused.

use super::fwif::{GENERAL_HEAP_BASE, PDSCODEDATA_HEAP_BASE, USCCODE_HEAP_BASE};
use super::km::{km, YdPvrKm};
use super::mem::{bo_alloc, bo_alloc_top, bo_rollback, bo_top_rollback, round_up, PvrGpuBo, L0_FLAGS_DATA, TOP_VA};
use core::fmt;

/// The heaps the pool serves, in `enum pvr_render_heap` order.
pub const POOL_HEAPS: usize = 3;
const HEAP_NAME: [&str; POOL_HEAPS] = ["general", "pds", "usc"];
const HEAP_BASE: [u64; POOL_HEAPS] = [GENERAL_HEAP_BASE, PDSCODEDATA_HEAP_BASE, USCCODE_HEAP_BASE];

/// Table slots per heap. The fast path, not a limit: past it, frees go on
/// the overflow chain.
pub const POOL_FREE_SLOTS: usize = 128;

const PAGE: u32 = 0x1000;
const OVF_MAGIC: u32 = 0x464f_5650; /* "PVOF" */

/// One table slot. `size` 0: empty.
#[repr(C)]
#[derive(Clone, Copy)]
struct FreeBlock {
    /// Device-VA offset in its heap (page aligned).
    hb: u32,
    /// Bytes, a multiple of 4 KiB.
    size: u32,
    /// CPU address of the block's first page.
    cpu: *mut u8,
}

/// The header of a range on the overflow chain, in its first bytes.
#[repr(C)]
#[derive(Clone, Copy)]
struct OvfHdr {
    magic: u32,
    hb: u32,
    size: u32,
    sum: u32,
    next: *mut u8,
}

impl OvfHdr {
    const NONE: OvfHdr = OvfHdr { magic: 0, hb: 0, size: 0, sum: 0, next: core::ptr::null_mut() };

    fn sum_of(&self) -> u32 {
        let next = self.next as usize as u64;
        (self.magic ^ self.hb ^ self.size ^ next as u32 ^ (next >> 32) as u32).wrapping_mul(0x9E37_79B1)
    }

    fn read(cpu: *const u8) -> OvfHdr {
        // SAFETY: `cpu` is a free range of ours, at least a page long.
        unsafe { (cpu as *const OvfHdr).read_unaligned() }
    }

    fn write(cpu: *mut u8, hb: u32, size: u32, next: *mut u8) {
        let mut h = OvfHdr { magic: OVF_MAGIC, hb, size, sum: 0, next };
        h.sum = h.sum_of();
        // SAFETY: as `read`.
        unsafe { (cpu as *mut OvfHdr).write_unaligned(h) };
    }
}

/// `struct yd_pvr_pool` (include/yakogl_drivers.h, opaque there): the pool's
/// free lists and counters. One per GPU, owned by the C render service,
/// zero before the first call.
#[repr(C)]
pub struct YdPvrPool {
    free: [[FreeBlock; POOL_FREE_SLOTS]; POOL_HEAPS],
    /// Overflow chain heads.
    ovf: [*mut u8; POOL_HEAPS],
    /// Ranges on each chain now, and at most.
    ovf_n: [u32; POOL_HEAPS],
    ovf_peak: [u32; POOL_HEAPS],
    /// Table slots in use at most.
    slots_peak: [u32; POOL_HEAPS],
    /// Chain headers found overwritten.
    ovf_bad: u32,
    /// KiB a free could not take back, per heap; `[POOL_HEAPS]`: no heap.
    dropped_kib: [u32; POOL_HEAPS + 1],
    /// Frees refused because the range was already free.
    double_frees: u32,
    /// KiB given back to the bump instead of a list.
    rolled_back_kib: [u32; POOL_HEAPS],
    /// Frees taken back (to a list or the bump), and their KiB.
    frees: [u32; POOL_HEAPS],
    freed_kib: [u32; POOL_HEAPS],
    /// The physical cursor's highest value (carveout offset; 0: not seen).
    /// `pool_next` only falls through a rollback, which records it first.
    pool_hw: u32,
    /// Allocations served from a list, from the bump, and refused.
    reused: [u32; POOL_HEAPS],
    bumped: [u32; POOL_HEAPS],
    refused: [u32; POOL_HEAPS],
}

/// The size `include/yakogl_drivers.h` reserves for it (`YD_PVR_POOL_BYTES`).
pub const YD_PVR_POOL_BYTES: usize = 6304;

const _: () = assert!(core::mem::size_of::<YdPvrPool>() == YD_PVR_POOL_BYTES);

/// The heaps' device-VA cursors, `enum pvr_render_heap` order.
pub type Bumps = [*mut u32; POOL_HEAPS];

fn bump(b: &Bumps, h: usize) -> u32 {
    // SAFETY: the C side's cursors, valid for the call.
    unsafe { *b[h] }
}

/// A walk over one heap's chain: `cur` and its checked header `h`, the entry
/// before it (`prev`, null at the head) and how many entries precede it.
#[derive(Clone, Copy)]
struct OvfPos {
    prev: *mut u8,
    cur: *mut u8,
    idx: u32,
    h: OvfHdr,
}

/// Which free ranges a best fit may take: those of the top region, or those
/// below it (every range, with the region off).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Region {
    Below,
    Top,
}

impl Region {
    fn has(self, hb: u32) -> bool {
        (hb >= TOP_VA) == (self == Region::Top)
    }
}

/// Whether `[hb, hb + size)` of `heap` was handed out by this pool: under the
/// heap's bump, or (GENERAL) inside the top region.
fn in_heap(k: &YdPvrKm, b: &Bumps, heap: usize, hb: u32, size: u32) -> bool {
    if heap == GENERAL && hb >= TOP_VA {
        let co = u64::from(hb - TOP_VA);
        return co >= k.pool_floor() && co + u64::from(size) <= k.pool_end();
    }
    u64::from(hb) + u64::from(size) <= u64::from(bump(b, heap))
}

const GENERAL: usize = 0;

/// Free bytes, free ranges and the largest range, of one heap.
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct FreeOf {
    pub bytes: u64,
    pub ranges: u32,
    pub largest: u32,
}

impl YdPvrPool {
    /// Which heap a device address belongs to (each heap is bounded by its
    /// bump, GENERAL also by its top region, and the heaps are far apart).
    fn heap_of_gpu(k: &YdPvrKm, b: &Bumps, gpu: u64) -> Option<usize> {
        (0..POOL_HEAPS).find(|&h| {
            gpu >= HEAP_BASE[h] && gpu - HEAP_BASE[h] < 1 << 32 && in_heap(k, b, h, (gpu - HEAP_BASE[h]) as u32, 1)
        })
    }

    fn relink(&mut self, heap: usize, prev: *mut u8, next: *mut u8) {
        if prev.is_null() {
            self.ovf[heap] = next;
            return;
        }
        let ph = OvfHdr::read(prev);
        OvfHdr::write(prev, ph.hb, ph.size, next);
    }

    /// Read and check `p.cur`'s header. False at the end of the chain, and at
    /// a bad header, where the chain is cut: what follows is unreachable.
    fn load(&mut self, k: &YdPvrKm, b: &Bumps, heap: usize, p: &mut OvfPos) -> bool {
        if p.cur.is_null() {
            return false;
        }
        let h = OvfHdr::read(p.cur);
        p.h = h;
        if h.magic == OVF_MAGIC
            && h.sum == h.sum_of()
            && h.size != 0
            && h.size & 0xFFF == 0
            && h.hb & 0xFFF == 0
            && in_heap(k, b, heap, h.hb, h.size)
        {
            return true;
        }
        self.ovf_bad += 1;
        crate::km_log!(
            k,
            Wrn,
            "render pool: heap {} overflow entry {} overwritten while free; chain cut, {} entries after it abandoned ({} bad so far)",
            heap,
            p.idx,
            self.ovf_n[heap].wrapping_sub(p.idx + 1),
            self.ovf_bad
        );
        self.relink(heap, p.prev, core::ptr::null_mut());
        self.ovf_n[heap] = p.idx;
        p.cur = core::ptr::null_mut();
        false
    }

    fn first(&mut self, k: &YdPvrKm, b: &Bumps, heap: usize) -> (OvfPos, bool) {
        let mut p = OvfPos { prev: core::ptr::null_mut(), cur: self.ovf[heap], idx: 0, h: OvfHdr::NONE };
        let ok = self.load(k, b, heap, &mut p);
        (p, ok)
    }

    fn next(&mut self, k: &YdPvrKm, b: &Bumps, heap: usize, p: &mut OvfPos) -> bool {
        p.prev = p.cur;
        p.cur = p.h.next;
        p.idx += 1;
        self.load(k, b, heap, p)
    }

    /// Unlink `p.cur` and move on to the entry after it; `prev` stays.
    fn remove_next(&mut self, k: &YdPvrKm, b: &Bumps, heap: usize, p: &mut OvfPos) -> bool {
        self.relink(heap, p.prev, p.h.next);
        self.ovf_n[heap] -= 1;
        p.cur = p.h.next;
        self.load(k, b, heap, p)
    }

    /// File a free range that touches no other: into an empty slot, or at
    /// the head of the chain. Never fails.
    fn insert(&mut self, heap: usize, hb: u32, cpu: *mut u8, size: u32) {
        let used = 1 + self.free[heap].iter().filter(|s| s.size != 0).count() as u32;
        if let Some(e) = self.free[heap].iter_mut().find(|s| s.size == 0) {
            *e = FreeBlock { hb, size, cpu };
            self.slots_peak[heap] = self.slots_peak[heap].max(used);
            return;
        }
        OvfHdr::write(cpu, hb, size, self.ovf[heap]);
        self.ovf[heap] = cpu;
        self.ovf_n[heap] += 1;
        self.ovf_peak[heap] = self.ovf_peak[heap].max(self.ovf_n[heap]);
    }

    /// Move chain entries into empty table slots, head first.
    fn promote(&mut self, k: &YdPvrKm, b: &Bumps, heap: usize) {
        for i in 0..POOL_FREE_SLOTS {
            if self.ovf[heap].is_null() {
                return;
            }
            if self.free[heap][i].size != 0 {
                continue;
            }
            let (p, ok) = self.first(k, b, heap);
            if !ok {
                return;
            }
            self.free[heap][i] = FreeBlock { hb: p.h.hb, size: p.h.size, cpu: p.cur };
            self.ovf[heap] = p.h.next;
            self.ovf_n[heap] -= 1;
        }
    }

    /// Best fit from the table and the chain, among the ranges of `region`,
    /// zeroed and cleaned; the rest of the range stays free. `msz` is page
    /// rounded.
    fn take(&mut self, k: &YdPvrKm, b: &Bumps, heap: usize, msz: u32, region: Region) -> Option<PvrGpuBo> {
        let mut best: Option<usize> = None;
        for (i, s) in self.free[heap].iter().enumerate() {
            if s.size >= msz && region.has(s.hb) && best.is_none_or(|j| s.size < self.free[heap][j].size) {
                best = Some(i);
            }
        }
        /* The chain is not empty only while the table is full. */
        let mut bestp: Option<OvfPos> = None;
        let (mut p, mut ok) = self.first(k, b, heap);
        while ok {
            let better = match &bestp {
                Some(q) => p.h.size < q.h.size,
                None => best.is_none_or(|j| p.h.size < self.free[heap][j].size),
            };
            if p.h.size >= msz && region.has(p.h.hb) && better {
                bestp = Some(p);
            }
            ok = self.next(k, b, heap, &mut p);
        }
        let (hb, cpu, size) = if let Some(q) = bestp {
            /* Its `prev` is still the entry before it: the walk changes the
             * chain only by cutting it at a bad header, after this entry. The
             * cut may have rewritten this entry's own link: read it again. */
            let h = OvfHdr::read(q.cur);
            self.relink(heap, q.prev, h.next);
            self.ovf_n[heap] -= 1;
            (h.hb, q.cur, h.size)
        } else {
            let j = best?;
            let s = self.free[heap][j];
            self.free[heap][j].size = 0;
            (s.hb, s.cpu, s.size)
        };
        // SAFETY: [cpu, cpu + msz) is a free range of ours (msz <= size).
        unsafe { core::ptr::write_bytes(cpu, 0, msz as usize) }; /* also wipes a chain header */
        k.flush(cpu, msz as usize);
        if size > msz {
            self.insert(heap, hb + msz, cpu.wrapping_add(msz as usize), size - msz);
        }
        self.promote(k, b, heap);
        Some(PvrGpuBo { gpu: HEAP_BASE[heap] + u64::from(hb), cpu, heap_off: hb })
    }

    /// `pvr_render_alloc`: a list first, the bump second (mapped with
    /// [`L0_FLAGS_DATA`]); a large GENERAL request from the top region
    /// instead, and a small one from the top region's ranges when the bump
    /// is out (the module's "top region", for the order). `None` for a bad
    /// heap or size, and when the pool is exhausted: the bump's own line,
    /// then this pool's summary at ERR.
    pub fn alloc(&mut self, k: &mut YdPvrKm, b: &Bumps, heap: usize, size: u32) -> Option<PvrGpuBo> {
        if heap >= POOL_HEAPS || size == 0 {
            return None;
        }
        let msz = round_up(u64::from(size), u64::from(PAGE)) as u32;
        let large = heap == GENERAL && k.pool_top_on() && msz > k.pool_top_above;
        let got = if large {
            let got = self.take(k, b, heap, msz, Region::Top).or_else(|| self.take(k, b, heap, msz, Region::Below));
            if got.is_none() {
                let bo = bo_alloc_top(k, HEAP_BASE[heap], size);
                if !bo.cpu.is_null() {
                    self.bumped[heap] += 1;
                    return Some(bo);
                }
            }
            got
        } else {
            match self.take(k, b, heap, msz, Region::Below) {
                None if heap == GENERAL && u64::from(msz) > k.pool_gap() => self.take(k, b, heap, msz, Region::Top),
                got => got,
            }
        };
        if let Some(bo) = got {
            self.reused[heap] += 1;
            return Some(bo);
        }
        // SAFETY: the C side's cursor, valid for the call.
        let bo = bo_alloc(k, HEAP_BASE[heap], unsafe { &mut *b[heap] }, size, L0_FLAGS_DATA);
        self.pool_hw = self.pool_hw.max(k.pool_next);
        if bo.cpu.is_null() {
            self.refused[heap] += 1;
            self.report(k, b, true);
            return None;
        }
        self.bumped[heap] += 1;
        Some(bo)
    }

    /// `pvr_render_free`: take a block back. `size` as allocated. With
    /// `rollback`, a merged block that ends at both cursors goes back to the
    /// bump instead of a list (opt-in, `pvr_render_pool_rollback`).
    pub fn free(&mut self, k: &mut YdPvrKm, b: &Bumps, bo: &PvrGpuBo, size: u32, rollback: bool) {
        if size == 0 {
            return;
        }
        let msz = round_up(u64::from(size), u64::from(PAGE)) as u32;
        let heap = Self::heap_of_gpu(k, b, bo.gpu);
        /* A block with no CPU mapping came from pvr_render_map_phys(): only
         * its VA was ever ours. A heap_off that does not match the address
         * is not a block of ours at all. */
        let Some(heap) = heap.filter(|&h| !bo.cpu.is_null() && bo.gpu - HEAP_BASE[h] == u64::from(bo.heap_off)) else {
            self.dropped_kib[heap.unwrap_or(POOL_HEAPS)] += msz / 1024;
            return;
        };
        /* A range past the heap's bump (or outside the top region) was never
         * handed out by this pool: taking it back would put unmapped VA on
         * the list. */
        if !in_heap(k, b, heap, bo.heap_off, msz) {
            crate::km_log!(
                k,
                Wrn,
                "render pool: heap {} free of +0x{:x} ({} KiB) runs past the bump",
                heap,
                bo.heap_off,
                msz / 1024
            );
            self.dropped_kib[heap] += msz / 1024;
            return;
        }
        /* An overlap with a free range is a double free (or a size larger
         * than the allocation): merging would hand one range to two callers.
         * Refused and counted, not dropped (the bytes are still free). Its own
         * pass: the merge pass below clears slots before it has seen them all. */
        let over = |hb: u32, sz: u32| hb < bo.heap_off + msz && bo.heap_off < hb + sz;
        let mut dup = self.free[heap].iter().any(|s| s.size != 0 && over(s.hb, s.size));
        if !dup {
            let (mut p, mut ok) = self.first(k, b, heap);
            while ok && !dup {
                dup = over(p.h.hb, p.h.size);
                ok = !dup && self.next(k, b, heap, &mut p);
            }
        }
        if dup {
            self.double_frees += 1;
            crate::km_log!(
                k,
                Wrn,
                "render pool: heap {} double free at +0x{:x} ({} KiB), refused ({} so far)",
                heap,
                bo.heap_off,
                msz / 1024,
                self.double_frees
            );
            return;
        }
        /* Absorb the neighbours on BOTH sides (A, C, B freed in that order
         * must end as one range). One pass over the table and one over the
         * chain: absorbing the block before moves the start, not the end. */
        let (mut nhb, mut ncpu, mut nsize) = (bo.heap_off, bo.cpu, msz);
        for s in self.free[heap].iter_mut().filter(|s| s.size != 0) {
            if s.hb + s.size == nhb && s.cpu.wrapping_add(s.size as usize) == ncpu {
                (nhb, ncpu, nsize) = (s.hb, s.cpu, nsize + s.size);
                s.size = 0;
            } else if nhb + nsize == s.hb && ncpu.wrapping_add(nsize as usize) == s.cpu {
                nsize += s.size;
                s.size = 0;
            }
        }
        let (mut p, mut ok) = self.first(k, b, heap);
        while ok {
            if p.h.hb + p.h.size == nhb && p.cur.wrapping_add(p.h.size as usize) == ncpu {
                (nhb, ncpu, nsize) = (p.h.hb, p.cur, nsize + p.h.size);
                ok = self.remove_next(k, b, heap, &mut p);
            } else if nhb + nsize == p.h.hb && ncpu.wrapping_add(nsize as usize) == p.cur {
                nsize += p.h.size;
                ok = self.remove_next(k, b, heap, &mut p);
            } else {
                ok = self.next(k, b, heap, &mut p);
            }
        }
        self.frees[heap] += 1;
        self.freed_kib[heap] += msz / 1024;
        /* Offer the MERGED block to the bump first: a large free block just
         * under the bump is not pool for a slightly larger request. */
        self.pool_hw = self.pool_hw.max(k.pool_next);
        /* A top range at the region's floor goes back to the gap, always:
         * the gap is the one piece both ends draw on. */
        if heap == GENERAL && nhb >= TOP_VA && bo_top_rollback(k, nhb, nsize) {
            self.promote(k, b, heap);
            return;
        }
        // SAFETY: the C side's cursor, valid for the call.
        if rollback && bo_rollback(k, unsafe { &mut *b[heap] }, nhb, ncpu, nsize) {
            self.rolled_back_kib[heap] += nsize / 1024;
        } else {
            self.insert(heap, nhb, ncpu, nsize);
        }
        /* the merge may have emptied slots while the chain holds entries */
        self.promote(k, b, heap);
    }

    /// One heap's free ranges (table and chain). Walks the chain, so a bad
    /// header is found, cut and counted here as on any other walk.
    pub fn free_of(&mut self, k: &YdPvrKm, b: &Bumps, heap: usize) -> FreeOf {
        let mut f = FreeOf::default();
        let mut add = |size: u32| {
            f.bytes += u64::from(size);
            f.ranges += 1;
            f.largest = f.largest.max(size);
        };
        for s in self.free[heap].iter().filter(|s| s.size != 0) {
            add(s.size);
        }
        let (mut p, mut ok) = self.first(k, b, heap);
        while ok {
            add(p.h.size);
            ok = self.next(k, b, heap, &mut p);
        }
        f
    }

    /// The summary (five lines, see [`Line`]): at ERR right after an
    /// exhaustion, at INF otherwise (the run-end report).
    pub fn report(&mut self, k: &YdPvrKm, b: &Bumps, exhausted: bool) {
        let mut free = [FreeOf::default(); POOL_HEAPS];
        for (h, f) in free.iter_mut().enumerate() {
            *f = self.free_of(k, b, h);
        }
        let lines = [Line::Head, Line::Heap(0), Line::Heap(1), Line::Heap(2), Line::Tail];
        for line in lines {
            let s = Summary { p: self, k, free, line };
            if exhausted {
                crate::km_log!(k, Err, "{}", s);
            } else {
                crate::km_log!(k, Inf, "{}", s);
            }
        }
    }
}

/// One line of the summary. One line per heap, because the whole summary
/// does not fit one log line ([`super::km::KM_LINE_MAX`] cut it):
///
/// ```text
/// render pool summary: 58880 KiB, bump 52364 KiB now, 52364 KiB high-water
/// render pool general: free 3072 KiB in 5 ranges, largest 1876 KiB; 40 reused, 21 bumped, 1 refused; 30 frees (9001 KiB)
/// render pool pds: ...
/// render pool usc: ...
/// render pool losses: dropped 0 KiB, 0 double frees, 0 bad chain headers, rolled back 0 KiB
/// ```
///
/// A heap whose list is empty says "largest none" (there is no largest
/// range, which is not a range of 0 KiB).
#[derive(Clone, Copy)]
enum Line {
    Head,
    Heap(usize),
    Tail,
}

struct Summary<'a> {
    p: &'a YdPvrPool,
    k: &'a YdPvrKm,
    free: [FreeOf; POOL_HEAPS],
    line: Line,
}

impl fmt::Display for Summary<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (p, k) = (self.p, self.k);
        match self.line {
            Line::Head => {
                let base = u64::from(k.pool_off);
                let kib = |off: u64| off.saturating_sub(base) / 1024;
                let now = u64::from(k.pool_next);
                write!(
                    f,
                    "render pool summary: {} KiB, bump {} KiB now, {} KiB high-water",
                    kib(k.size),
                    kib(now),
                    kib(now.max(u64::from(p.pool_hw)))
                )?;
                if k.pool_top_hw != 0 {
                    write!(f, "; top {} KiB now, {} KiB high-water", k.pool_top / 1024, k.pool_top_hw / 1024)?;
                }
                Ok(())
            }
            Line::Heap(h) => {
                let fr = self.free[h];
                write!(
                    f,
                    "render pool {}: free {} KiB in {} ranges, largest ",
                    HEAP_NAME[h],
                    fr.bytes / 1024,
                    fr.ranges
                )?;
                if fr.ranges == 0 {
                    f.write_str("none")?;
                } else {
                    write!(f, "{} KiB", fr.largest / 1024)?;
                }
                write!(
                    f,
                    "; {} reused, {} bumped, {} refused; {} frees ({} KiB)",
                    p.reused[h], p.bumped[h], p.refused[h], p.frees[h], p.freed_kib[h]
                )
            }
            Line::Tail => write!(
                f,
                "render pool losses: dropped {} KiB, {} double frees, {} bad chain headers, rolled back {} KiB",
                p.dropped_kib.iter().sum::<u32>(),
                p.double_frees,
                p.ovf_bad,
                p.rolled_back_kib.iter().sum::<u32>()
            ),
        }
    }
}

// ---- C entry points (pvr_render.c is wrappers over these) --------------------
//
// Every one: `k` is the C driver's `struct yd_pvr_km`, `pool` the render
// service's `struct yd_pvr_pool`, `bumps` its three device-VA cursors
// (`enum pvr_render_heap` order); all valid and not used by anyone else for
// the call.

/// 0, -EINVAL (a bad heap or size) or -ENOMEM (the pool is exhausted).
///
/// # Safety
/// See the section comment; `out` valid.
#[no_mangle]
pub unsafe extern "C" fn yd_pvr_pool_alloc(
    pool: *mut YdPvrPool,
    k: *mut YdPvrKm,
    bumps: *const Bumps,
    heap: u32,
    size: u32,
    out: *mut PvrGpuBo,
) -> i32 {
    if heap as usize >= POOL_HEAPS || size == 0 {
        return -22; /* -EINVAL */
    }
    // SAFETY: the caller's contract.
    let (pool, k, b, out) = unsafe { (&mut *pool, km(k), &*bumps, &mut *out) };
    match pool.alloc(k, b, heap as usize, size) {
        Some(bo) => {
            *out = bo;
            0
        }
        None => -12, /* -ENOMEM */
    }
}

/// # Safety
/// See the section comment; `bo` valid.
#[no_mangle]
pub unsafe extern "C" fn yd_pvr_pool_free(
    pool: *mut YdPvrPool,
    k: *mut YdPvrKm,
    bumps: *const Bumps,
    bo: *const PvrGpuBo,
    size: u32,
    rollback: bool,
) {
    // SAFETY: the caller's contract.
    let (pool, k, b, bo) = unsafe { (&mut *pool, km(k), &*bumps, &*bo) };
    pool.free(k, b, bo, size, rollback);
}

/// The summary at INF (the run-end report).
///
/// # Safety
/// See the section comment.
#[no_mangle]
pub unsafe extern "C" fn yd_pvr_pool_report(pool: *mut YdPvrPool, k: *mut YdPvrKm, bumps: *const Bumps) {
    // SAFETY: the caller's contract.
    let (pool, k, b) = unsafe { (&mut *pool, km(k), &*bumps) };
    pool.report(k, b, false);
}

/// The pool's usage for the owner report's balance: `out[0]` the bytes the
/// physical cursor and the top region have handed out, `out[1]` the bytes
/// free in every heap's list (the chain walked, a bad header cut and counted
/// as anywhere).
///
/// # Safety
/// See the section comment; `out` points at 2 words.
#[no_mangle]
pub unsafe extern "C" fn yd_pvr_pool_usage(pool: *mut YdPvrPool, k: *mut YdPvrKm, bumps: *const Bumps, out: *mut u64) {
    // SAFETY: the caller's contract.
    let (pool, k, b, out) = unsafe { (&mut *pool, km(k), &*bumps, core::slice::from_raw_parts_mut(out, 2)) };
    out[0] = u64::from(k.pool_next.saturating_sub(k.pool_off)) + u64::from(k.pool_top);
    out[1] = (0..POOL_HEAPS).map(|h| pool.free_of(k, b, h).bytes).sum();
}

/// The bring-up counters: what `pvr_render_debug_freelist` returns
/// (pvr_render.h), and the dropped KiB per heap (`dropped`, which no C
/// caller asks for since the character device went with pvrgpu.ko; the pool
/// report prints their sum as "render pool losses: dropped N KiB").
///
/// # Safety
/// `pool` valid; every out pointer null or valid: room for 4 words in
/// `dropped`, 3 in `slots_peak`, `ovf_peak` and `ovf_now`, 1 in the others.
#[no_mangle]
pub unsafe extern "C" fn yd_pvr_pool_counters(
    pool: *const YdPvrPool,
    dropped: *mut u32,
    slots_peak: *mut u32,
    slots_cap: *mut u32,
    ovf_peak: *mut u32,
    ovf_now: *mut u32,
    bad: *mut u32,
) {
    // SAFETY: the caller's contract.
    let p = unsafe { &*pool };
    let put = |dst: *mut u32, src: &[u32]| {
        if !dst.is_null() {
            // SAFETY: the caller's contract: room for `src.len()` words.
            unsafe { core::ptr::copy_nonoverlapping(src.as_ptr(), dst, src.len()) };
        }
    };
    put(dropped, &p.dropped_kib);
    put(slots_peak, &p.slots_peak);
    put(slots_cap, &[POOL_FREE_SLOTS as u32]);
    put(ovf_peak, &p.ovf_peak);
    put(ovf_now, &p.ovf_n);
    put(bad, &[p.ovf_bad]);
}

#[cfg(test)]
mod golden;
#[cfg(test)]
mod tests;
