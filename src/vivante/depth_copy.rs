// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The buffers of outcome (b).1's depth copy: one SUPER_TILED buffer per
//! sampled depth object, which the resolve engine fills from the row-linear
//! D24S8 surface and the texture unit samples as D24X8
//! (docs/notes/gc-depth-sample-wire.md, "(b).1 Primary: an RS copy into a
//! supertiled depth view", the bullet "The copy").
//!
//! Only dtpu knob mode 3 (`depth_sample::DEPTH_TPU_RSCOPY`, a `test-knobs`
//! build) uses it: `depth_sample` keeps the one pool, makes the buffer when
//! `gc_depth_sampled` answers, records the copy into it before the draw
//! (`depth_sample::emit_copies`), stamps it with the draw's kick
//! (`depth_sample::binding`), retires it from `gc_attachment_gone` and reaps
//! it at `finish` and `finish_previous`. The copy's words are
//! [`super::depth_sample::copy_blit`], its size
//! [`super::depth_sample::copy_bytes`] and the descriptor that samples it
//! [`super::depth_sample::copy_sampler_words`]. Whether the texture unit reads
//! the copy right is the board's (gc-depth-sample-wire.md (b).1, "Board
//! check").
//!
//! The rules, the same ones `ds_cache` (`used_by`, `gone`) and `tiled_copy`
//! (`used_by`, `stale`) keep for their buffers:
//!
//! - **Key.** A buffer belongs to (GL id, surface generation) and is
//!   [`copy_bytes`](super::depth_sample::copy_bytes) of the view long. While
//!   both match, [`CopyPool::buffer`] hands back the same buffer.
//! - **Retired.** A new generation or a new size of the same id, or the
//!   object's contents going away ([`CopyPool::gone`], from
//!   `attachment_gone`), retires the buffer: it is never matched again.
//! - **Kick-stamped.** [`CopyPool::note_read`] stamps the live buffer of an id
//!   with the kick that samples it. A retired buffer whose stamped kick has
//!   not completed is kept (the GPU may still read it) and freed by a later
//!   call once that kick completed ([`CopyPool::reap`], which every entry
//!   point runs first). A buffer never read (stamp 0) goes at once.
//!
//! The allocator and the "is this kick still pending" query are a
//! [`CopyHost`], so the unit tests below drive the pool without a GPU;
//! [`OsalHost`] is the product one (the OSAL's data heap and
//! `backend::kick_pending_retired`).

use super::depth_sample::{copy_bytes, DepthView, COPY_ALIGN};
use crate::osal::{self, OsalGpuMem, OSAL_MEM_DATA};
use crate::util::{ENOMEM, ENOTSUP};

/// Buffers held at once, live and retired together. glmark2's shadow and
/// refract sample one depth texture a frame; a retired buffer is kept only
/// until its kick completed, a frame or two. A full pool first frees the
/// retired buffers whose kicks completed, then the least recently read live
/// one no pending kick reads (it is made again on its next use).
pub const ENTRIES: usize = 8;

/// What the pool allocates with and asks about kicks. Single-threaded, as
/// the rest of this backend.
pub trait CopyHost {
    /// `bytes` of GPU memory the RS may write and the TPU read, or `None`.
    fn alloc(&mut self, bytes: usize) -> Option<OsalGpuMem>;
    /// Give back what [`alloc`](CopyHost::alloc) returned.
    fn free(&mut self, mem: OsalGpuMem);
    /// Is kick `kick` (never 0) still to complete?
    fn pending(&self, kick: u32) -> bool;
}

/// The product [`CopyHost`]: the OSAL's data heap, where the depth surfaces
/// and the tiled copies live, and the backend's kick query.
pub struct OsalHost;

impl CopyHost for OsalHost {
    fn alloc(&mut self, bytes: usize) -> Option<OsalGpuMem> {
        let mut mem = OsalGpuMem::ZERO;
        if osal::gpu_alloc(super::backend::heap_data(), bytes, OSAL_MEM_DATA, &mut mem) != 0 {
            return None;
        }
        Some(mem)
    }
    fn free(&mut self, mut mem: OsalGpuMem) {
        osal::gpu_free(&mut mem);
    }
    fn pending(&self, kick: u32) -> bool {
        super::backend::kick_pending_retired(kick)
    }
}

/// What a buffer was made for.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Key {
    id: u32,
    gen: u32,
    bytes: usize,
}

struct Entry {
    key: Key,
    mem: OsalGpuMem,
    /// `mem.gpu` rounded up to [`COPY_ALIGN`] (the allocation has the room).
    gpu: u64,
    /// The kick that last read it (0: none): not freed before it completed.
    used_by: u32,
    /// For the LRU of a full pool.
    stamp: u32,
    /// Never matched again; freed once `used_by` completed.
    retired: bool,
}

/// The pool: a fixed table, `const`-constructible so the wiring can keep one
/// in a `Global`.
pub struct CopyPool {
    entries: [Option<Entry>; ENTRIES],
    clock: u32,
}

impl Default for CopyPool {
    fn default() -> Self {
        Self::new()
    }
}

impl CopyPool {
    pub const fn new() -> Self {
        CopyPool { entries: [const { None }; ENTRIES], clock: 0 }
    }

    /// The buffer of object `id` at surface generation `gen` for the surface
    /// `view`: its GPU address, on [`COPY_ALIGN`], [`copy_bytes`] long. The
    /// same buffer while `(id, gen)` and the size match; otherwise the old one
    /// is retired and a new one made. -ENOTSUP for `id` 0 or a view
    /// [`copy_bytes`] refuses, -ENOMEM when no slot or no memory is free.
    pub fn buffer(&mut self, h: &mut impl CopyHost, id: u32, gen: u32, view: &DepthView) -> Result<u64, i32> {
        self.reap(h);
        let Some(bytes) = copy_bytes(view).filter(|_| id != 0) else { return Err(-ENOTSUP) };
        let key = Key { id, gen, bytes };
        self.clock = self.clock.wrapping_add(1);
        let clock = self.clock;
        if let Some(e) = self.entries.iter_mut().flatten().find(|e| !e.retired && e.key == key) {
            e.stamp = clock;
            return Ok(e.gpu);
        }
        /* another generation or size of this id: retired, freed when idle */
        self.retire(h, id);
        let slot = match self.entries.iter().position(Option::is_none) {
            Some(i) => i,
            None => self.evict_idle(h).ok_or(-ENOMEM)?,
        };
        let align = COPY_ALIGN as usize;
        let mem = h.alloc(bytes + align - 1).ok_or(-ENOMEM)?;
        let gpu = mem.gpu.next_multiple_of(COPY_ALIGN);
        if gpu + bytes as u64 > 1 << 32 {
            /* the RS address registers are 32 bits (copy_blit refuses it) */
            h.free(mem);
            return Err(-ENOMEM);
        }
        self.entries[slot] = Some(Entry { key, mem, gpu, used_by: 0, stamp: clock, retired: false });
        Ok(gpu)
    }

    /// Kick `kick` samples the live buffer of `id` (a draw recorded in it):
    /// the buffer is not freed until that kick completed.
    pub fn note_read(&mut self, id: u32, kick: u32) {
        if let Some(e) = self.entries.iter_mut().flatten().find(|e| !e.retired && e.key.id == id) {
            e.used_by = kick;
        }
    }

    /// The contents of object `id` are gone (`attachment_gone`, a delete):
    /// its buffers are retired, and freed now if no pending kick reads them.
    pub fn gone(&mut self, h: &mut impl CopyHost, id: u32) {
        self.retire(h, id);
        self.reap(h);
    }

    /// Free every retired buffer whose stamped kick completed.
    pub fn reap(&mut self, h: &mut impl CopyHost) {
        for slot in self.entries.iter_mut() {
            if slot.as_ref().is_some_and(|e| e.retired && !busy(e, h)) {
                if let Some(e) = slot.take() {
                    h.free(e.mem);
                }
            }
        }
    }

    /// Free every buffer, live, retired or read by a pending kick (backend
    /// shutdown, after which no kick reads anything).
    pub fn release_all(&mut self, h: &mut impl CopyHost) {
        for slot in self.entries.iter_mut() {
            if let Some(e) = slot.take() {
                h.free(e.mem);
            }
        }
    }

    /// Buffers held, live and retired.
    pub fn held(&self) -> usize {
        self.entries.iter().flatten().count()
    }

    /// Bytes held (the allocations, alignment room included).
    pub fn held_bytes(&self) -> usize {
        self.entries.iter().flatten().map(|e| e.mem.size).sum()
    }

    /// The live buffer of `id`: its GPU address, generation and the kick
    /// that last read it.
    pub fn live(&self, id: u32) -> Option<(u64, u32, u32)> {
        self.entries.iter().flatten().find(|e| !e.retired && e.key.id == id).map(|e| (e.gpu, e.key.gen, e.used_by))
    }

    fn retire(&mut self, h: &mut impl CopyHost, id: u32) {
        for slot in self.entries.iter_mut() {
            let Some(e) = slot.as_mut() else { continue };
            if e.key.id != id {
                continue;
            }
            e.retired = true;
            if !busy(e, h) {
                if let Some(e) = slot.take() {
                    h.free(e.mem);
                }
            }
        }
    }

    /// A full pool: free the least recently used live buffer no pending kick
    /// reads (retired ones were reaped already), and give its slot.
    fn evict_idle(&mut self, h: &mut impl CopyHost) -> Option<usize> {
        let clock = self.clock;
        let i = self
            .entries
            .iter()
            .enumerate()
            .filter_map(|(i, e)| e.as_ref().filter(|e| !busy(e, h)).map(|e| (i, clock.wrapping_sub(e.stamp))))
            .max_by_key(|&(_, age)| age)?
            .0;
        if let Some(e) = self.entries[i].take() {
            h.free(e.mem);
        }
        Some(i)
    }
}

fn busy(e: &Entry, h: &impl CopyHost) -> bool {
    e.used_by != 0 && h.pending(e.used_by)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An allocator over a fake 32-bit address space and a completed-kick
    /// watermark: kicks up to `done` completed.
    struct Fake {
        next: u64,
        done: u32,
        allocs: u32,
        frees: u32,
        live_bytes: usize,
        fail: bool,
    }

    impl Fake {
        fn new() -> Self {
            Fake { next: 0x4000_0010, done: 0, allocs: 0, frees: 0, live_bytes: 0, fail: false }
        }
    }

    impl CopyHost for Fake {
        fn alloc(&mut self, bytes: usize) -> Option<OsalGpuMem> {
            if self.fail {
                return None;
            }
            assert!(bytes <= 16 << 20);
            /* 32 slots of 16 MiB, each one off 64 as an OSAL's may be: more
             * than the pool ever holds, so the live ones never overlap */
            let gpu = self.next + u64::from(self.allocs % 32) * (16 << 20);
            self.allocs += 1;
            self.live_bytes += bytes;
            Some(OsalGpuMem { gpu, size: bytes, ..OsalGpuMem::ZERO })
        }
        fn free(&mut self, mem: OsalGpuMem) {
            self.frees += 1;
            self.live_bytes -= mem.size;
        }
        fn pending(&self, kick: u32) -> bool {
            kick > self.done
        }
    }

    /// glmark2's shadow map: 1600x1200 depth in a 64-aligned surface.
    fn view_1600x1200() -> DepthView {
        DepthView { gpu: 0x4100_0000, stride: 1600 * 4, rows: 1216 }
    }

    fn view_256() -> DepthView {
        DepthView { gpu: 0x4200_0000, stride: 256 * 4, rows: 256 }
    }

    #[test]
    fn reuse_while_the_key_matches() {
        let (mut p, mut h) = (CopyPool::new(), Fake::new());
        let v = view_1600x1200();
        let a = p.buffer(&mut h, 7, 1, &v).unwrap();
        assert_eq!(a % COPY_ALIGN, 0);
        p.note_read(7, 1);
        let b = p.buffer(&mut h, 7, 1, &v).unwrap();
        assert_eq!((a, h.allocs, p.held()), (b, 1, 1));
        assert_eq!(p.live(7), Some((a, 1, 1)));
        /* sized copy_bytes plus the alignment room */
        assert_eq!(p.held_bytes(), 1600 * 4 * 1216 + COPY_ALIGN as usize - 1);
    }

    #[test]
    fn regrow_on_a_size_change() {
        let (mut p, mut h) = (CopyPool::new(), Fake::new());
        let small = p.buffer(&mut h, 7, 1, &view_256()).unwrap();
        p.note_read(7, 3);
        let big = p.buffer(&mut h, 7, 1, &view_1600x1200()).unwrap();
        assert_ne!(small, big);
        /* the small one is still read by kick 3: kept, retired */
        assert_eq!((h.allocs, h.frees, p.held()), (2, 0, 2));
        assert_eq!(p.live(7).map(|l| l.0), Some(big));
        h.done = 3;
        p.reap(&mut h);
        assert_eq!((h.frees, p.held()), (1, 1));
        assert_eq!(h.live_bytes, p.held_bytes());
    }

    #[test]
    fn a_new_generation_is_a_new_buffer() {
        let (mut p, mut h) = (CopyPool::new(), Fake::new());
        let v = view_256();
        p.buffer(&mut h, 7, 1, &v).unwrap();
        /* never read: freed at once */
        p.buffer(&mut h, 7, 2, &v).unwrap();
        assert_eq!((h.allocs, h.frees, p.held()), (2, 1, 1));
        assert_eq!(p.live(7).map(|l| l.1), Some(2));
    }

    #[test]
    fn free_is_deferred_until_the_kick_completes() {
        let (mut p, mut h) = (CopyPool::new(), Fake::new());
        p.buffer(&mut h, 9, 1, &view_1600x1200()).unwrap();
        p.note_read(9, 5);
        p.gone(&mut h, 9);
        assert_eq!((h.frees, p.held(), p.live(9)), (0, 1, None));
        h.done = 4;
        p.reap(&mut h);
        assert_eq!((h.frees, p.held()), (0, 1));
        /* a retired buffer is never matched again, even under its old key */
        let again = p.buffer(&mut h, 9, 1, &view_1600x1200()).unwrap();
        assert_eq!((h.allocs, p.held()), (2, 2));
        assert_eq!(p.live(9).map(|l| l.0), Some(again));
    }

    #[test]
    fn freed_after_completion() {
        let (mut p, mut h) = (CopyPool::new(), Fake::new());
        p.buffer(&mut h, 9, 1, &view_256()).unwrap();
        p.note_read(9, 5);
        h.done = 5;
        p.gone(&mut h, 9);
        assert_eq!((h.frees, p.held(), h.live_bytes), (1, 0, 0));
        /* gone of an id without buffers is nothing */
        p.gone(&mut h, 9);
        assert_eq!(h.frees, 1);
    }

    #[test]
    fn refusals() {
        let (mut p, mut h) = (CopyPool::new(), Fake::new());
        let odd = DepthView { gpu: 0x4100_0000, stride: 100 * 4, rows: 64 };
        assert_eq!(p.buffer(&mut h, 1, 1, &odd), Err(-ENOTSUP));
        assert_eq!(p.buffer(&mut h, 0, 1, &view_256()), Err(-ENOTSUP));
        h.fail = true;
        assert_eq!(p.buffer(&mut h, 1, 1, &view_256()), Err(-ENOMEM));
        assert_eq!((h.allocs, p.held()), (0, 0));
    }

    #[test]
    fn a_full_pool_evicts_the_oldest_idle_and_never_a_busy_one() {
        let (mut p, mut h) = (CopyPool::new(), Fake::new());
        let v = view_256();
        let mut first = 0;
        for id in 1..=ENTRIES as u32 {
            let g = p.buffer(&mut h, id, 1, &v).unwrap();
            if id == 1 {
                first = g;
            }
            p.note_read(id, id);
        }
        /* every one is read by a pending kick: no room */
        assert_eq!(p.buffer(&mut h, 100, 1, &v), Err(-ENOMEM));
        h.done = 2;
        /* ids 1 and 2 are idle; 1 is the older one */
        p.buffer(&mut h, 100, 1, &v).unwrap();
        assert_eq!(p.live(1), None);
        assert!(p.live(2).is_some());
        assert_eq!(p.held(), ENTRIES);
        assert_ne!(p.live(100).map(|l| l.0), Some(first));
    }

    /// glmark2 shadow / refract: each frame makes a 1600x1200 depth texture
    /// (a new id, or the same id at a new generation), samples it once in
    /// that frame's kick, and deletes it; the GPU completes kicks `lag`
    /// frames behind. The pool holds at most `lag + 1` buffers and, once the
    /// GPU is idle, none: every allocation is freed.
    #[test]
    fn no_leak_over_frames_of_glmark2s_pattern() {
        for lag in 0..4u32 {
            for same_id in [false, true] {
                let (mut p, mut h) = (CopyPool::new(), Fake::new());
                let v = view_1600x1200();
                let frames = 500u32;
                for kick in 1..=frames {
                    h.done = kick.saturating_sub(lag + 1);
                    let (id, gen) = if same_id { (42, kick) } else { (1000 + kick, 1) };
                    p.buffer(&mut h, id, gen, &v).unwrap();
                    p.note_read(id, kick);
                    p.gone(&mut h, id);
                    assert!(p.held() as u32 <= lag + 1, "lag {lag}: {} held at kick {kick}", p.held());
                    assert_eq!(h.live_bytes, p.held_bytes());
                }
                h.done = frames;
                p.reap(&mut h);
                assert_eq!((p.held(), h.live_bytes), (0, 0), "lag {lag}");
                assert_eq!(h.allocs, h.frees, "lag {lag}");
                assert_eq!(h.allocs, frames);
            }
        }
    }
}
