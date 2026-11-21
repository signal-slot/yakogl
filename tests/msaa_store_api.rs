// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The public `yakogl::msaa_store` API driven as a back end drives it: look up
//! or allocate a slot per draw, load/store flags across a flush and a redraw,
//! `gone` from `attachment_gone`, `clear` on reset. A counting fake allocator
//! checks that no handle is freed twice or leaked.
//!
//! Run: `cargo test --no-default-features --features host-test,gpu-powervr --test msaa_store_api`
//! (the module itself is not feature-gated; the library needs one GPU
//! feature to compile, and this is the narrowest set).

use std::collections::HashSet;
use yakogl::msaa_store::{MsaaStore, SampleSize, MSAA_SLOTS};

const SZ: SampleSize = SampleSize { w: 16, h: 8, bytes_per_sample: 4, samples: 4 };

/// Allocator that hands out numbered handles and records every free.
#[derive(Default)]
struct Heap {
    next: u32,
    allocs: Vec<u32>,
    freed: Vec<u32>,
}

impl Heap {
    /// Leak and double-free check: every handle allocated is freed once.
    fn assert_balanced(&self) {
        let set: HashSet<_> = self.freed.iter().copied().collect();
        assert_eq!(set.len(), self.freed.len(), "a handle was freed twice: {:?}", self.freed);
        assert_eq!(self.allocs.len(), self.freed.len(), "allocs {:?} vs frees {:?}", self.allocs, self.freed);
        assert!(self.allocs.iter().all(|a| set.contains(a)), "leak");
    }

    /// `clear` the table into the free list, then check the balance.
    fn assert_balanced_after_clear<const N: usize>(&mut self, t: &mut MsaaStore<u32, N>) {
        let mut freed = Vec::new();
        t.clear(|x| freed.push(x));
        self.freed.extend(freed);
        self.assert_balanced();
    }
}

fn get<const N: usize>(t: &mut MsaaStore<u32, N>, h: &mut Heap, id: u64, gen: u32, size: SampleSize) -> Option<u32> {
    let Heap { next, allocs, freed } = h;
    t.get_or_alloc(
        id,
        gen,
        size,
        |_| {
            *next += 1;
            allocs.push(*next);
            Some(*next)
        },
        |x| freed.push(x),
    )
    .map(|s| s.handle)
}

#[test]
fn new_id_allocates_once_and_same_id_and_gen_reuses() {
    let (mut t, mut h) = (MsaaStore::<u32>::new(), Heap::default());
    assert!(t.is_empty());
    let a = get(&mut t, &mut h, 7, 1, SZ).expect("slot");
    assert_eq!(h.allocs.len(), 1);
    assert_eq!(get(&mut t, &mut h, 7, 1, SZ), Some(a));
    assert_eq!(get(&mut t, &mut h, 7, 1, SZ), Some(a));
    assert_eq!(h.allocs.len(), 1, "reuse allocates nothing");
    assert!(h.freed.is_empty());
    assert_eq!(t.len(), 1);
    let s = t.get(7).unwrap();
    assert_eq!((s.id, s.gen, s.size), (7, 1, SZ));
}

#[test]
fn new_gen_or_new_size_frees_and_reallocates() {
    let (mut t, mut h) = (MsaaStore::<u32>::new(), Heap::default());
    let a = get(&mut t, &mut h, 7, 1, SZ).unwrap();
    t.get(7).unwrap().mark_stored();
    let b = get(&mut t, &mut h, 7, 2, SZ).unwrap();
    assert_ne!(a, b);
    assert_eq!(h.freed, [a]);
    assert!(!t.get(7).unwrap().needs_load(), "fresh slot has nothing to load");
    let big = SampleSize { w: 64, ..SZ };
    let c = get(&mut t, &mut h, 7, 2, big).unwrap();
    assert_eq!(h.freed, [a, b]);
    assert_eq!(t.len(), 1, "same id keeps one slot");
    assert_eq!(t.get(7).unwrap().handle, c);
    h.assert_balanced_after_clear(&mut t);
}

#[test]
fn overfilling_evicts_the_lru_and_frees_exactly_once() {
    let (mut t, mut h) = (MsaaStore::<u32>::new(), Heap::default());
    for id in 1..=MSAA_SLOTS as u64 {
        get(&mut t, &mut h, id, 1, SZ).unwrap();
    }
    assert!(h.freed.is_empty());
    // Touch id 1 so id 2 is the least recently used (MSAA_SLOTS is 2).
    get(&mut t, &mut h, 1, 1, SZ).unwrap();
    let n = MSAA_SLOTS as u64 + 1;
    get(&mut t, &mut h, n, 1, SZ).unwrap();
    assert_eq!(h.freed.len(), 1, "exactly one eviction");
    assert_eq!(t.len(), MSAA_SLOTS);
    assert!(t.get(2).is_none());
    assert!(t.get(1).is_some() && t.get(n).is_some());
    h.assert_balanced_after_clear(&mut t);
}

#[test]
fn wider_table_holds_n_before_evicting() {
    let (mut t, mut h) = (MsaaStore::<u32, 4>::new(), Heap::default());
    for id in 1..=4 {
        get(&mut t, &mut h, id, 1, SZ).unwrap();
    }
    assert!(h.freed.is_empty() && t.len() == 4);
    get(&mut t, &mut h, 5, 1, SZ).unwrap();
    assert_eq!(h.freed.len(), 1);
    h.assert_balanced_after_clear(&mut t);
}

#[test]
fn gone_frees_and_a_later_get_is_none() {
    let (mut t, mut h) = (MsaaStore::<u32>::new(), Heap::default());
    let a = get(&mut t, &mut h, 7, 1, SZ).unwrap();
    let mut freed = Vec::new();
    t.gone(7, |x| freed.push(x));
    assert_eq!(freed, [a]);
    assert!(t.get(7).is_none());
    t.gone(7, |x| freed.push(x));
    t.gone(0, |x| freed.push(x));
    t.gone(99, |x| freed.push(x));
    assert_eq!(freed, [a], "repeated or unknown gone frees nothing more");
    h.freed.extend(freed);
    h.assert_balanced();
}

#[test]
fn clear_frees_everything() {
    let (mut t, mut h) = (MsaaStore::<u32>::new(), Heap::default());
    for id in 1..=MSAA_SLOTS as u64 {
        get(&mut t, &mut h, id, 1, SZ).unwrap();
    }
    h.assert_balanced_after_clear(&mut t);
    assert!(t.is_empty());
    assert!(t.get(1).is_none());
    // A second clear frees nothing.
    let mut again = Vec::new();
    t.clear(|x| again.push(x));
    assert!(again.is_empty());
}

#[test]
fn load_flag_follows_draw_flush_redraw_and_clear() {
    let (mut t, mut h) = (MsaaStore::<u32>::new(), Heap::default());
    // Draw 1: new slot, nothing to load.
    get(&mut t, &mut h, 7, 1, SZ).unwrap();
    assert!(!t.get(7).unwrap().needs_load());
    // Flush: the job stored its samples.
    t.get(7).unwrap().mark_stored();
    // Redraw: the same slot, which now loads.
    get(&mut t, &mut h, 7, 1, SZ).unwrap();
    assert!(t.get(7).unwrap().needs_load());
    // Whole clear: nothing worth loading; a later store loads again.
    t.get(7).unwrap().mark_cleared();
    assert!(!t.get(7).unwrap().needs_load());
    t.get(7).unwrap().mark_stored();
    assert!(t.get(7).unwrap().needs_load());
    assert_eq!(h.allocs.len(), 1);
}

#[test]
fn id_0_and_empty_sizes_are_never_kept() {
    let (mut t, mut h) = (MsaaStore::<u32>::new(), Heap::default());
    assert_eq!(get(&mut t, &mut h, 0, 1, SZ), None);
    assert_eq!(get(&mut t, &mut h, 3, 1, SampleSize { w: 0, ..SZ }), None);
    assert!(h.allocs.is_empty() && t.is_empty());
}

#[test]
fn failed_alloc_is_none_and_evicts_nothing() {
    let (mut t, mut h) = (MsaaStore::<u32>::new(), Heap::default());
    for id in 1..=MSAA_SLOTS as u64 {
        get(&mut t, &mut h, id, 1, SZ).unwrap();
    }
    let mut freed = Vec::new();
    let r = t.get_or_alloc(99, 1, SZ, |_| None, |x| freed.push(x));
    assert!(r.is_none());
    assert!(freed.is_empty() && t.len() == MSAA_SLOTS);
}

#[test]
fn a_whole_session_balances() {
    let (mut t, mut h) = (MsaaStore::<u32>::new(), Heap::default());
    let big = SampleSize { w: 32, ..SZ };
    for round in 0..5u32 {
        for id in 1..=4u64 {
            get(&mut t, &mut h, id, 1 + round / 2, if id % 2 == 0 { SZ } else { big }).unwrap();
        }
        let mut freed = Vec::new();
        t.gone(2, |x| freed.push(x));
        h.freed.extend(freed);
    }
    h.assert_balanced_after_clear(&mut t);
}
