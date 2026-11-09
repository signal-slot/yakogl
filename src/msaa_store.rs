// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The samples a backend keeps for a multisample colour attachment between
//! jobs: a small table of slots, one per live 4x renderbuffer, shared by the
//! four backends (src/pvr, src/v3d, src/vivante, src/mali).
//!
//! The GL core allocates only the resolved storage of a multisample
//! renderbuffer (`rb.mem`); the samples are the backend's
//! (docs/notes/es3-msaa-renderbuffers.md (b)5-7). They belong to the GL
//! object, so a slot is keyed by the attachment's id and storage generation
//! (`GlesDraw::target_id` / `target_gen`), never by its address: a freed
//! `rb.mem` can come back at the same GPU address and size after a
//! respecification, and its old samples must not come back with it.
//!
//! A slot goes when:
//!
//! - its object's contents are gone in GL's terms ([`MsaaStore::gone`], from
//!   `backend::attachment_gone`: deletion, respecification, a whole
//!   invalidate);
//! - a draw names its id with another generation or size (the storage was
//!   specified again; [`MsaaStore::get_or_alloc`] drops it and allocates
//!   afresh);
//! - the table is full and another attachment needs a slot: the least
//!   recently used one is evicted. That loses only its edge samples, since
//!   every job ends with the resolved image in `rb.mem` ((b)7): the next job
//!   into the evicted attachment starts from that image in every sample.
//!
//! When the backend cannot allocate the samples, `get_or_alloc` returns
//! `None`: "samples not kept" ((e)1), drawn the same way as after an
//! eviction.
//!
//! The table never frees GPU memory itself. Every handle that leaves it is
//! given to the caller's `free`, which decides the timing (`gpu_free`'s
//! contract: after the kicks that may still read it have completed).
//!
//! Id 0 is never kept: it names the window, a texture or an EGLImage, whose
//! samples follow the backend's own surface rule.

/// The slots a backend keeps by default ((e)2: dEQP draws one 4x FBO at a
/// time; the PowerVR, GC7000UL and Mali keep at least 2).
pub const MSAA_SLOTS: usize = 2;

/// The size of one attachment's samples.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct SampleSize {
    pub w: u32,
    pub h: u32,
    /// Bytes of one sample of one pixel (4 for RGBA8).
    pub bytes_per_sample: u32,
    /// Samples per pixel (4).
    pub samples: u32,
}

impl SampleSize {
    /// The bytes all samples take, packed (`w * h * bytes_per_sample *
    /// samples`); `None` when that is 0 or does not fit a `usize`. A
    /// backend whose layout pads (tiles, row alignment) asks its `alloc` for
    /// more itself.
    pub fn bytes(self) -> Option<usize> {
        let b = (self.w as usize)
            .checked_mul(self.h as usize)?
            .checked_mul(self.bytes_per_sample as usize)?
            .checked_mul(self.samples as usize)?;
        (b != 0).then_some(b)
    }
}

/// One attachment's samples: the GL object they belong to and the backend's
/// allocation `H` (its `OsalGpuMem`, or whatever it keeps).
#[derive(Debug)]
pub struct Slot<H> {
    /// The GL object (`GlesDraw::target_id` form, never 0).
    pub id: u64,
    /// Its storage generation (`GlesDraw::target_gen`).
    pub gen: u32,
    pub size: SampleSize,
    /// The backend's allocation.
    pub handle: H,
    /// A job has stored samples here since the slot was made or the
    /// attachment was cleared whole.
    valid: bool,
    /// When the slot was last asked for ([`MsaaStore::get_or_alloc`]).
    used: u64,
}

impl<H> Slot<H> {
    /// The samples hold what earlier jobs drew: a job loads them. When false
    /// (a new slot, or after a whole clear) a job starts from its clear, or
    /// from the resolved image in `rb.mem` in every sample.
    pub fn needs_load(&self) -> bool {
        self.valid
    }

    /// A job stored its samples here.
    pub fn mark_stored(&mut self) {
        self.valid = true;
    }

    /// The attachment was cleared whole: nothing here is worth loading.
    pub fn mark_cleared(&mut self) {
        self.valid = false;
    }
}

/// The table: up to `N` slots ([`MSAA_SLOTS`] unless the backend says
/// otherwise), each holding a handle of type `H`.
pub struct MsaaStore<H, const N: usize = MSAA_SLOTS> {
    s: [Option<Slot<H>>; N],
    /// Bumped on every `get_or_alloc` that returns a slot.
    tick: u64,
}

impl<H, const N: usize> Default for MsaaStore<H, N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<H, const N: usize> MsaaStore<H, N> {
    /// An empty table.
    pub const fn new() -> Self {
        MsaaStore { s: [const { None }; N], tick: 0 }
    }

    /// The slot of attachment `id` at generation `gen` and `size`, made now
    /// when there is none.
    ///
    /// - id 0: `None`, nothing allocated (the window's own rule).
    /// - A slot of `id` with the same `gen` and `size`: that slot, as it is.
    /// - A slot of `id` with another `gen` or `size`: its handle goes to
    ///   `free`, then a new slot is allocated as below.
    /// - No slot: `alloc(size.bytes())` is asked for the handle. `None` from
    ///   it (or a size of 0 bytes) is `None` here: the samples are not kept.
    ///   When the table is full the least recently used slot is evicted (its
    ///   handle to `free`), only after `alloc` succeeded, so a failed
    ///   allocation costs no other attachment its samples.
    ///
    /// A new slot does not need a load ([`Slot::needs_load`] is false).
    pub fn get_or_alloc(
        &mut self,
        id: u64,
        gen: u32,
        size: SampleSize,
        alloc: impl FnOnce(usize) -> Option<H>,
        mut free: impl FnMut(H),
    ) -> Option<&mut Slot<H>> {
        if id == 0 {
            return None;
        }
        self.tick += 1;
        let tick = self.tick;
        if let Some(i) = self.find(id) {
            let fresh = matches!(&self.s[i], Some(s) if s.gen == gen && s.size == size);
            if fresh {
                let s = self.s[i].as_mut()?;
                s.used = tick;
                return Some(s);
            }
            if let Some(old) = self.s[i].take() {
                free(old.handle);
            }
        }
        let handle = alloc(size.bytes()?)?;
        let i = match self.s.iter().position(Option::is_none) {
            Some(i) => i,
            None => {
                let i = self.lru()?;
                if let Some(old) = self.s[i].take() {
                    free(old.handle);
                }
                i
            }
        };
        self.s[i] = Some(Slot { id, gen, size, handle, valid: false, used: tick });
        self.s[i].as_mut()
    }

    /// The slot kept for `id`, whatever its generation, without touching its
    /// age.
    pub fn get(&mut self, id: u64) -> Option<&mut Slot<H>> {
        let i = self.find(id)?;
        self.s[i].as_mut()
    }

    /// The contents of GL object `id` are gone (`backend::attachment_gone`):
    /// its slot's handle goes to `free`. An id the table does not hold
    /// (or 0) is a no-op.
    pub fn gone(&mut self, id: u64, free: impl FnOnce(H)) {
        if let Some(old) = self.find(id).and_then(|i| self.s[i].take()) {
            free(old.handle);
        }
    }

    /// Drop every slot (a backend reset), each handle to `free`.
    pub fn clear(&mut self, mut free: impl FnMut(H)) {
        for s in self.s.iter_mut() {
            if let Some(old) = s.take() {
                free(old.handle);
            }
        }
    }

    /// The slots in use.
    pub fn iter(&self) -> impl Iterator<Item = &Slot<H>> {
        self.s.iter().flatten()
    }

    /// Slots in use.
    pub fn len(&self) -> usize {
        self.iter().count()
    }

    /// No slot in use.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn find(&self, id: u64) -> Option<usize> {
        if id == 0 {
            return None;
        }
        self.s.iter().position(|s| matches!(s, Some(s) if s.id == id))
    }

    /// The least recently used slot (`None` only when `N` is 0).
    fn lru(&self) -> Option<usize> {
        (0..N).filter(|&i| self.s[i].is_some()).min_by_key(|&i| self.s[i].as_ref().map_or(0, |s| s.used))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RGBA8_4X: SampleSize = SampleSize { w: 16, h: 8, bytes_per_sample: 4, samples: 4 };

    /// An allocator handing out numbered handles and recording sizes.
    struct Heap {
        next: u32,
        asked: Vec<usize>,
        freed: Vec<u32>,
        fail: bool,
    }

    impl Heap {
        fn new() -> Heap {
            Heap { next: 1, asked: Vec::new(), freed: Vec::new(), fail: false }
        }
    }

    /// `get_or_alloc` with the heap's alloc and free, returning the slot's
    /// handle.
    fn get(t: &mut MsaaStore<u32>, h: &mut Heap, id: u64, gen: u32, size: SampleSize) -> Option<u32> {
        let (asked, fail, next) = (&mut h.asked, h.fail, &mut h.next);
        let freed = &mut h.freed;
        t.get_or_alloc(
            id,
            gen,
            size,
            |b| {
                asked.push(b);
                (!fail).then(|| {
                    *next += 1;
                    *next - 1
                })
            },
            |x| freed.push(x),
        )
        .map(|s| s.handle)
    }

    #[test]
    fn a_slot_is_allocated_once_and_found_again() {
        let (mut t, mut h) = (MsaaStore::<u32>::new(), Heap::new());
        assert_eq!(get(&mut t, &mut h, 5, 1, RGBA8_4X), Some(1));
        assert_eq!(h.asked, [16 * 8 * 4 * 4], "packed bytes of all samples");
        assert_eq!(get(&mut t, &mut h, 5, 1, RGBA8_4X), Some(1), "same id, gen and size: the same slot");
        assert_eq!(h.asked.len(), 1, "no second allocation");
        assert!(h.freed.is_empty());
        assert_eq!(t.len(), 1);
    }

    /// Revert check: keying by the handle (the address) and size instead of
    /// (id, gen) would keep handle 1 here and load stale samples.
    #[test]
    fn a_respecification_with_a_new_gen_drops_the_old_samples() {
        let (mut t, mut h) = (MsaaStore::<u32>::new(), Heap::new());
        get(&mut t, &mut h, 5, 1, RGBA8_4X);
        t.get(5).unwrap().mark_stored();
        assert_eq!(get(&mut t, &mut h, 5, 2, RGBA8_4X), Some(2), "same size, new gen: allocated afresh");
        assert_eq!(h.freed, [1], "the old handle went to free");
        assert!(!t.get(5).unwrap().needs_load(), "the new slot holds nothing to load");
        assert_eq!(t.len(), 1);
        let big = SampleSize { w: 32, ..RGBA8_4X };
        assert_eq!(get(&mut t, &mut h, 5, 2, big), Some(3), "same gen, another size: allocated afresh");
        assert_eq!(h.freed, [1, 2]);
        assert_eq!(t.get(5).map(|s| s.size), Some(big));
    }

    #[test]
    fn gone_frees_the_slot_and_an_unknown_id_is_a_no_op() {
        let (mut t, mut h) = (MsaaStore::<u32>::new(), Heap::new());
        get(&mut t, &mut h, 5, 1, RGBA8_4X);
        get(&mut t, &mut h, 6, 1, RGBA8_4X);
        let mut freed = Vec::new();
        t.gone(7, |x| freed.push(x));
        t.gone(0, |x| freed.push(x));
        assert!(freed.is_empty() && t.len() == 2, "unknown ids and 0 touch nothing");
        t.gone(5, |x| freed.push(x));
        assert_eq!(freed, [1]);
        assert!(t.get(5).is_none() && t.get(6).is_some());
        t.gone(5, |x| freed.push(x));
        assert_eq!(freed, [1], "gone twice frees once");
        assert_eq!(get(&mut t, &mut h, 5, 1, RGBA8_4X), Some(3), "after gone the same id and gen allocate afresh");
    }

    #[test]
    fn a_full_table_evicts_the_least_recently_used_slot() {
        let (mut t, mut h) = (MsaaStore::<u32>::new(), Heap::new());
        get(&mut t, &mut h, 5, 1, RGBA8_4X); // handle 1
        get(&mut t, &mut h, 6, 1, RGBA8_4X); // handle 2
        get(&mut t, &mut h, 5, 1, RGBA8_4X); // 5 used last
        assert_eq!(get(&mut t, &mut h, 7, 1, RGBA8_4X), Some(3));
        assert_eq!(h.freed, [2], "6 was used least recently");
        assert!(t.get(6).is_none() && t.get(5).is_some() && t.get(7).is_some());
        assert_eq!(t.len(), MSAA_SLOTS);
        assert_eq!(get(&mut t, &mut h, 6, 1, RGBA8_4X), Some(4), "6 comes back without samples");
        assert_eq!(h.freed, [2, 1], "5 was older than 7");
    }

    #[test]
    fn a_wider_table_holds_more_attachments() {
        let (mut t, mut h) = (MsaaStore::<u32, 4>::new(), Heap::new());
        for id in 1..=4 {
            let (asked, next) = (&mut h.asked, &mut h.next);
            let s = t.get_or_alloc(
                id,
                1,
                RGBA8_4X,
                |b| {
                    asked.push(b);
                    *next += 1;
                    Some(*next - 1)
                },
                |_| panic!("nothing evicted"),
            );
            assert!(s.is_some());
        }
        assert_eq!(t.len(), 4);
    }

    /// (e)1: no memory is "samples not kept", and it costs no other
    /// attachment its slot.
    #[test]
    fn an_allocation_failure_returns_none_and_evicts_nothing() {
        let (mut t, mut h) = (MsaaStore::<u32>::new(), Heap::new());
        get(&mut t, &mut h, 5, 1, RGBA8_4X);
        get(&mut t, &mut h, 6, 1, RGBA8_4X);
        h.fail = true;
        assert_eq!(get(&mut t, &mut h, 7, 1, RGBA8_4X), None);
        assert!(h.freed.is_empty() && t.len() == 2, "5 and 6 keep their samples");
        assert_eq!(get(&mut t, &mut h, 5, 1, RGBA8_4X), Some(1), "a kept slot needs no allocation");
        assert_eq!(get(&mut t, &mut h, 5, 2, RGBA8_4X), None, "respecified with no memory");
        assert_eq!(h.freed, [1], "the stale samples go even so");
        assert!(t.get(5).is_none());
        h.fail = false;
        assert_eq!(get(&mut t, &mut h, 5, 2, RGBA8_4X), Some(3), "memory back: allocated");
    }

    #[test]
    fn id_0_and_empty_sizes_are_never_kept() {
        let (mut t, mut h) = (MsaaStore::<u32>::new(), Heap::new());
        assert_eq!(get(&mut t, &mut h, 0, 1, RGBA8_4X), None, "the window");
        assert_eq!(get(&mut t, &mut h, 5, 1, SampleSize { w: 0, ..RGBA8_4X }), None, "0 bytes");
        let huge = SampleSize { w: u32::MAX, h: u32::MAX, bytes_per_sample: u32::MAX, samples: u32::MAX };
        assert_eq!(huge.bytes(), None, "overflow");
        assert_eq!(get(&mut t, &mut h, 5, 1, huge), None);
        assert!(h.asked.is_empty() && t.is_empty(), "alloc never asked");
        assert!(t.get(0).is_none());
    }

    #[test]
    fn the_valid_flag_follows_stores_and_whole_clears() {
        let (mut t, mut h) = (MsaaStore::<u32>::new(), Heap::new());
        get(&mut t, &mut h, 5, 1, RGBA8_4X);
        let s = t.get(5).unwrap();
        assert!(!s.needs_load(), "new: nothing stored yet");
        s.mark_stored();
        assert!(t.get_or_alloc(5, 1, RGBA8_4X, |_| None, |_| ()).unwrap().needs_load(), "kept across lookups");
        t.get(5).unwrap().mark_cleared();
        assert!(!t.get(5).unwrap().needs_load(), "a whole clear: nothing to load");
    }

    #[test]
    fn clear_frees_every_handle() {
        let (mut t, mut h) = (MsaaStore::<u32>::new(), Heap::new());
        get(&mut t, &mut h, 5, 1, RGBA8_4X);
        get(&mut t, &mut h, 6, 1, RGBA8_4X);
        let mut freed = Vec::new();
        t.clear(|x| freed.push(x));
        freed.sort_unstable();
        assert_eq!(freed, [1, 2]);
        assert!(t.is_empty() && t.iter().next().is_none());
    }
}
