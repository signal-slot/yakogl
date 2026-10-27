// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The compiler's memory: one bump arena over a buffer the caller lends us.
//!
//! The `yakogl` crate is `no_std` with no dependencies and no allocator --
//! `grep -rn "use alloc::" src/` finds nothing -- because the product targets
//! are Zephyr and bare metal, where the GL core's only heap use is the copy
//! `glShaderSource` keeps. An in-process compiler therefore cannot allocate:
//! it is handed a `&mut [u8]` scratch buffer and everything it builds lives
//! inside it, with `u32` indices in place of pointers wherever a node refers
//! to another node.
//!
//! Running out of scratch is an ordinary, reportable outcome
//! ([`ArenaError::OutOfScratch`]), not a panic: the caller sees "this shader
//! needs a bigger buffer" and can say so in the info log. Every allocation
//! returns a `Result`, so no path can silently truncate -- absence has its
//! own representation, which is the rule the rest of this project runs on.

use core::cell::Cell;
use core::marker::PhantomData;
use core::mem::{align_of, size_of};
use core::slice;

/// Why an allocation failed. One variant today; it is an enum so that a
/// caller matches on a reason rather than on a bare `None`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ArenaError {
    /// The scratch buffer is too small for this compile.
    OutOfScratch,
}

/// How many given-back blocks the arena remembers; see `Arena::free`.
const FREE_SLOTS: usize = 16;

/// A bump allocator over a borrowed buffer.
///
/// Allocation takes `&self` (an interior-mutable bump pointer) so that the
/// references handed out live as long as the buffer rather than as long as a
/// `&mut` borrow of the arena; that is what lets a parser hold several arena
/// slices at once. Nothing is ever freed individually: the whole arena dies
/// when the borrow ends.
pub struct Arena<'a> {
    base: *mut u8,
    len: usize,
    head: Cell<usize>,
    /// Bytes a growing [`AVec`] left behind when it moved: a bump arena
    /// cannot take a buffer back, so every move abandons the old one.
    /// Counted so the phases that waste scratch can be named.
    abandoned: Cell<usize>,
    /// Blocks given back that were not the most recent allocation --
    /// chiefly the old buffer of a vector that grew by moving, which
    /// nothing can refer to any more -- as `(offset, len)`, `len` 0 for an
    /// empty slot. A later allocation that fits takes from the smallest
    /// block that holds it. Without this a vector that doubles leaves as
    /// much behind as it keeps, and the IR's instruction and node lists
    /// doubled their way to half a megabyte of garbage on dEQP's
    /// uniform_api.random.23 shape.
    free: [Cell<(usize, usize)>; FREE_SLOTS],
    /// The highest the bump pointer has been. [`Arena::used`] is where it is
    /// NOW, which [`Arena::reset_to`] can move back below the peak.
    peak: Cell<usize>,
    /// Allocations come from the buffer's END downwards
    /// ([`Arena::new_top_down`]): `head` still counts the bytes handed out,
    /// and they are `[len - head, len)`.
    top_down: bool,
    /// The arena owns the exclusive borrow of the caller's buffer.
    _buf: PhantomData<&'a mut [u8]>,
}

impl<'a> Arena<'a> {
    /// Lend the arena a buffer for the lifetime of this compile.
    pub fn new(buf: &'a mut [u8]) -> Arena<'a> {
        Arena {
            base: buf.as_mut_ptr(),
            len: buf.len(),
            head: Cell::new(0),
            abandoned: Cell::new(0),
            free: [const { Cell::new((0, 0)) }; FREE_SLOTS],
            peak: Cell::new(0),
            top_down: false,
            _buf: PhantomData,
        }
    }

    /// An arena that hands out the buffer from its END downwards: what it
    /// has handed out is always the last [`Arena::used`] bytes of the
    /// buffer.
    ///
    /// For the keep region (`store`), which places a record that lives as
    /// long as a shader object at the top of a free run and one a program
    /// holds at the bottom, so that the first kind, freed, rejoins the free
    /// run instead of leaving a hole below the second. A vector never grows
    /// in place here (its start would have to move), so it moves instead,
    /// which the free list recycles as it does upward.
    /// [`Arena::reset_to`] is the scratch's and is not available.
    pub fn new_top_down(buf: &'a mut [u8]) -> Arena<'a> {
        let a = Arena::new(buf);
        Arena { top_down: true, ..a }
    }

    /// Bytes handed out so far. The caller sizes its buffer from this after a
    /// compile, and a test asserts on it, so it is part of the interface.
    pub fn used(&self) -> usize {
        self.head.get()
    }

    /// Bytes still available (an upper bound: an aligned allocation may need
    /// padding first).
    pub fn left(&self) -> usize {
        self.len - self.head.get()
    }

    /// The most the arena has held at once: what its buffer must be sized
    /// for, when a reset has moved `used` back since.
    pub fn peak(&self) -> usize {
        self.peak.get().max(self.head.get())
    }

    /// Forget every remembered free block, so the next allocations come
    /// from the top: `lower::relocate` needs its table above everything it
    /// moves.
    pub fn forget_free(&self) {
        for slot in &self.free {
            slot.set((0, 0));
        }
    }

    /// The address of the buffer's first byte, for a caller that moves
    /// blocks within it (`lower::relocate`).
    pub fn base_addr(&self) -> usize {
        self.base as usize
    }

    /// Bytes abandoned by vectors that grew by moving; see the field.
    pub fn abandoned(&self) -> usize {
        self.abandoned.get()
    }

    /// Give a slice's memory back for reuse (the bump pointer if it is the
    /// latest allocation, else a remembered free block).
    ///
    /// # Safety
    /// Nothing may read or write `s` afterwards: every reference into it is
    /// dangling once this returns. The callers are the front end's phase
    /// boundaries, where a phase's whole output is consumed.
    pub unsafe fn give_back<T>(&self, s: &[T]) {
        /* In a test build the bytes are poisoned first, so a reader the
         * safety argument missed reads garbage and fails its test. */
        #[cfg(test)]
        unsafe {
            core::ptr::write_bytes(s.as_ptr() as *mut u8, 0xa5, core::mem::size_of_val(s));
        }
        self.recycle(s.as_ptr() as *mut u8, core::mem::size_of_val(s));
    }

    /// Give back `[ptr, ptr + bytes)`, which nothing refers to any more:
    /// the bump pointer moves back when it is the latest allocation, and
    /// otherwise the block is remembered for reuse (replacing a smaller
    /// remembered one when every slot is taken).
    fn recycle(&self, ptr: *mut u8, bytes: usize) {
        if bytes == 0 || self.release_last(ptr, bytes) {
            return;
        }
        let start = (ptr as usize).wrapping_sub(self.base as usize);
        if start >= self.len {
            return;
        }
        let mut victim = None;
        let mut smallest = bytes;
        for (k, slot) in self.free.iter().enumerate() {
            let (_, l) = slot.get();
            if l == 0 {
                victim = Some(k);
                break;
            }
            if l < smallest {
                smallest = l;
                victim = Some(k);
            }
        }
        if let Some(k) = victim {
            self.free[k].set((start, bytes));
        }
    }

    /// A block from the remembered ones that holds `bytes` at `align`: the
    /// smallest that does, carved from its front.
    fn take_free(&self, bytes: usize, align: usize) -> Option<*mut u8> {
        let addr = self.base as usize;
        let mut best: Option<(usize, usize, usize)> = None;
        for (k, slot) in self.free.iter().enumerate() {
            let (s, l) = slot.get();
            if l == 0 {
                continue;
            }
            let pad = (align - (addr.wrapping_add(s) % align)) % align;
            if pad.checked_add(bytes).is_some_and(|need| need <= l) && best.is_none_or(|(_, _, bl)| l < bl) {
                best = Some((k, pad, l));
            }
        }
        let (k, pad, _) = best?;
        let (s, l) = self.free[k].get();
        let used = pad + bytes;
        self.free[k].set((s + used, l - used));
        // SAFETY: `[s + pad, s + pad + bytes)` lies inside the remembered
        // block, which is inside the buffer and referred to by nothing.
        Some(unsafe { self.base.add(s + pad) })
    }

    /// Extend the allocation that ends at `end` (one past its last byte) by
    /// `more` bytes when it is the most recent one: the bump pointer simply
    /// moves. `false` when it is not the last, or the buffer has no room.
    fn grow_last(&self, end: *mut u8, more: usize) -> bool {
        if self.top_down {
            return false;
        }
        let head = self.head.get();
        if end as usize != (self.base as usize).wrapping_add(head) {
            return false;
        }
        match head.checked_add(more) {
            Some(n) if n <= self.len => {
                self.head.set(n);
                true
            }
            _ => false,
        }
    }

    /// Move the bump pointer back to `mark` (an earlier [`Arena::used`]),
    /// forgetting every allocation made after it, and any remembered free
    /// block above it.
    ///
    /// # Safety
    /// Nothing allocated after `mark` may be read or written again: every
    /// reference into that range is dangling once this returns. The caller
    /// proves it by having copied out what it keeps (see
    /// `lower::relocate`) and dropping the originals.
    pub unsafe fn reset_to(&self, mark: usize) {
        debug_assert!(!self.top_down, "reset_to is the scratch's; a top-down arena has no such mark");
        if self.top_down || mark > self.head.get() {
            return;
        }
        self.peak.set(self.peak());
        for slot in &self.free {
            if slot.get().0 >= mark {
                slot.set((0, 0));
            }
        }
        self.head.set(mark);
    }

    /// Fill `[from, used())` with `byte`: a test's way of making a stale
    /// reference into memory given back by [`Arena::reset_to`] read
    /// garbage instead of the value it used to hold.
    #[cfg(test)]
    pub fn poison_above(&self, from: usize, to: usize, byte: u8) {
        let to = to.min(self.len);
        if from < to {
            // SAFETY: `[from, to)` is inside the buffer, and the caller
            // says nothing live is there.
            unsafe { core::ptr::write_bytes(self.base.add(from), byte, to - from) };
        }
    }

    /// Give back the allocation `[ptr, ptr + bytes)` when it is the most
    /// recent one; `false` (and nothing happens) when it is not.
    fn release_last(&self, ptr: *mut u8, bytes: usize) -> bool {
        let head = self.head.get();
        let start = (ptr as usize).wrapping_sub(self.base as usize);
        if self.top_down {
            /* The latest allocation is the lowest: it starts where the
             * handed-out bytes do. Its alignment padding above it stays
             * handed out, as an upward arena's padding below does. */
            if start != self.len.wrapping_sub(head) || start.checked_add(bytes).is_none_or(|e| e > self.len) {
                return false;
            }
            self.peak.set(self.peak());
            self.head.set(self.len - (start + bytes));
            return true;
        }
        if start.checked_add(bytes) != Some(head) || start > head {
            return false;
        }
        self.peak.set(self.peak());
        self.head.set(start);
        true
    }

    /// Raw bytes, aligned for `T`, without initialising them.
    fn raw(&self, bytes: usize, align: usize) -> Result<*mut u8, ArenaError> {
        if let Some(p) = self.take_free(bytes, align) {
            return Ok(p);
        }
        let head = self.head.get();
        let addr = self.base as usize;
        if self.top_down {
            /* Below everything handed out, aligned down on the address. */
            let top = self.len - head;
            let Some(lowest) = top.checked_sub(bytes) else {
                return Err(ArenaError::OutOfScratch);
            };
            let Some(start) = lowest.checked_sub(addr.wrapping_add(lowest) % align) else {
                return Err(ArenaError::OutOfScratch);
            };
            self.head.set(self.len - start);
            // SAFETY: `start + bytes <= top <= self.len`, so the range is
            // inside the buffer, below every byte handed out before.
            return Ok(unsafe { self.base.add(start) });
        }
        /* Align relative to the buffer's start AND to the pointer itself: the
         * caller's buffer need not be aligned for T. */
        let pad = (align - (addr.wrapping_add(head) % align)) % align;
        let start = head.checked_add(pad).ok_or(ArenaError::OutOfScratch)?;
        let end = start.checked_add(bytes).ok_or(ArenaError::OutOfScratch)?;
        if end > self.len {
            return Err(ArenaError::OutOfScratch);
        }
        self.head.set(end);
        // SAFETY: `start + bytes <= self.len`, so the range is inside the
        // buffer, and the bump pointer never hands the same byte out twice.
        Ok(unsafe { self.base.add(start) })
    }

    /// An uninitialised slice of `n` `T`s.
    ///
    /// # Safety
    /// The caller writes every element before reading any of it.
    unsafe fn alloc_uninit<T: Copy>(&self, n: usize) -> Result<&'a mut [T], ArenaError> {
        if n == 0 {
            return Ok(&mut []);
        }
        let bytes = size_of::<T>().checked_mul(n).ok_or(ArenaError::OutOfScratch)?;
        let p = self.raw(bytes, align_of::<T>())? as *mut T;
        // SAFETY: `p` is a fresh, correctly aligned region of `n * size_of::<T>()`
        // bytes that no other reference points into, and the caller's contract
        // is to initialise it. Its lifetime is the arena's buffer borrow.
        Ok(unsafe { slice::from_raw_parts_mut(p, n) })
    }

    /// `n` copies of `v`.
    pub fn alloc_filled<T: Copy>(&self, n: usize, v: T) -> Result<&'a mut [T], ArenaError> {
        // SAFETY: every element is written by the loop below before return.
        let s = unsafe { self.alloc_uninit::<T>(n) }?;
        for e in s.iter_mut() {
            *e = v;
        }
        Ok(s)
    }

    /// A copy of `src` inside the arena.
    pub fn alloc_copy<T: Copy>(&self, src: &[T]) -> Result<&'a mut [T], ArenaError> {
        // SAFETY: the copy below writes every element before return.
        let s = unsafe { self.alloc_uninit::<T>(src.len()) }?;
        s.copy_from_slice(src);
        Ok(s)
    }

    /// A copy of `src` as arena bytes, returned as `&str`.
    pub fn alloc_str(&self, src: &str) -> Result<&'a str, ArenaError> {
        let b = self.alloc_copy(src.as_bytes())?;
        // SAFETY: `b` is a byte-for-byte copy of a `&str`, so it is UTF-8.
        Ok(unsafe { core::str::from_utf8_unchecked(b) })
    }
}

/// A growable vector in an [`Arena`].
///
/// Growth reallocates at the arena's top and copies, which wastes the old
/// block -- a bump allocator cannot reuse it. That is deliberate: a compiler
/// pass is short-lived and the waste is bounded by the final size, while the
/// alternative (counting every list before building it) doubles the parser.
/// `with_capacity` avoids the copies where the count is known.
pub struct AVec<'a, T: Copy> {
    ptr: *mut T,
    len: usize,
    cap: usize,
    _m: PhantomData<&'a mut [T]>,
}

impl<'a, T: Copy> AVec<'a, T> {
    /// An empty vector that has not allocated yet.
    pub fn new() -> AVec<'a, T> {
        AVec { ptr: core::ptr::NonNull::<T>::dangling().as_ptr(), len: 0, cap: 0, _m: PhantomData }
    }

    /// An empty vector with room for `cap` elements already reserved.
    pub fn with_capacity(a: &Arena<'a>, cap: usize) -> Result<AVec<'a, T>, ArenaError> {
        let mut v = AVec::new();
        v.reserve(a, cap)?;
        Ok(v)
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Room for `extra` more elements without another copy.
    pub fn reserve(&mut self, a: &Arena<'a>, extra: usize) -> Result<(), ArenaError> {
        let want = self.len.checked_add(extra).ok_or(ArenaError::OutOfScratch)?;
        if want <= self.cap {
            return Ok(());
        }
        let cap = want.max(self.cap * 2).max(8);
        /* The vector is the arena's most recent allocation: grow it where it
         * is. A bump arena cannot free the old buffer of a move, so a vector
         * that doubles by moving leaves as much behind as it keeps. */
        if self.cap > 0 && size_of::<T>() > 0 {
            // SAFETY: `ptr + cap` is one past the end of this vector's own
            // allocation, which lies inside the arena's buffer.
            let end = unsafe { self.ptr.add(self.cap) } as *mut u8;
            if a.grow_last(end, (cap - self.cap) * size_of::<T>()) {
                self.cap = cap;
                return Ok(());
            }
        }
        // SAFETY: the copy below initialises `[0, len)`; the rest stays
        // uninitialised and is never read, because `as_slice` stops at `len`.
        let fresh = unsafe { a.alloc_uninit::<T>(cap) }?;
        fresh[..self.len].copy_from_slice(self.as_slice());
        a.abandoned.set(a.abandoned.get() + self.cap * size_of::<T>());
        /* Nothing refers to the old buffer: the vector is its only owner,
         * and every borrow of its elements ended before this `&mut self`
         * call. */
        if self.cap > 0 {
            a.recycle(self.ptr as *mut u8, self.cap * size_of::<T>());
        }
        self.ptr = fresh.as_mut_ptr();
        self.cap = cap;
        Ok(())
    }

    /// Drop the vector and give its buffer back to the arena, which is
    /// possible only while it is the arena's most recent allocation (a
    /// scratch list consumed before anything else was allocated). Otherwise
    /// the bytes stay where they are, as they always did.
    pub fn release(self, a: &Arena<'a>) {
        if self.cap > 0 && size_of::<T>() > 0 {
            a.recycle(self.ptr as *mut u8, self.cap * size_of::<T>());
        }
    }

    pub fn push(&mut self, a: &Arena<'a>, v: T) -> Result<(), ArenaError> {
        self.reserve(a, 1)?;
        // SAFETY: `reserve` guarantees `len < cap`, so this element is inside
        // the allocation.
        unsafe { self.ptr.add(self.len).write(v) };
        self.len += 1;
        Ok(())
    }

    pub fn extend_from_slice(&mut self, a: &Arena<'a>, src: &[T]) -> Result<(), ArenaError> {
        self.reserve(a, src.len())?;
        // SAFETY: `reserve` made room for `src.len()` elements at `len`.
        unsafe { slice::from_raw_parts_mut(self.ptr.add(self.len), src.len()) }.copy_from_slice(src);
        self.len += src.len();
        Ok(())
    }

    pub fn pop(&mut self) -> Option<T> {
        if self.len == 0 {
            return None;
        }
        self.len -= 1;
        // SAFETY: index `len` was initialised by the `push` that wrote it.
        Some(unsafe { self.ptr.add(self.len).read() })
    }

    pub fn as_slice(&self) -> &[T] {
        // SAFETY: `[0, len)` was initialised by the pushes that grew it, and
        // the allocation is alive for the arena's lifetime.
        unsafe { slice::from_raw_parts(self.ptr, self.len) }
    }

    pub fn as_mut_slice(&mut self) -> &mut [T] {
        // SAFETY: as in `as_slice`; `&mut self` makes the borrow exclusive.
        unsafe { slice::from_raw_parts_mut(self.ptr, self.len) }
    }

    /// The finished contents, borrowed for the arena's lifetime.
    pub fn into_slice(self) -> &'a [T] {
        // SAFETY: as in `as_slice`. `self` is consumed, so no other reference
        // to the elements survives this call.
        unsafe { slice::from_raw_parts(self.ptr, self.len) }
    }
}

impl<T: Copy> Default for AVec<'_, T> {
    fn default() -> Self {
        AVec::new()
    }
}

impl<T: Copy> core::ops::Index<usize> for AVec<'_, T> {
    type Output = T;
    fn index(&self, i: usize) -> &T {
        &self.as_slice()[i]
    }
}

impl<T: Copy> core::ops::IndexMut<usize> for AVec<'_, T> {
    fn index_mut(&mut self, i: usize) -> &mut T {
        &mut self.as_mut_slice()[i]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allocations_are_aligned_and_disjoint() {
        /* A deliberately misaligned view of the buffer: the caller's scratch
         * need not be aligned for anything we put in it. */
        let mut buf = [0u8; 1024];
        let a = Arena::new(&mut buf[1..]);
        let x = a.alloc_filled::<u32>(3, 7).expect("u32s");
        let y = a.alloc_copy::<u64>(&[1, 2]).expect("u64s");
        assert_eq!(x.as_ptr() as usize % align_of::<u32>(), 0, "u32 slice is aligned");
        assert_eq!(y.as_ptr() as usize % align_of::<u64>(), 0, "u64 slice is aligned");
        assert_eq!(x, [7, 7, 7]);
        assert_eq!(y, [1, 2]);
        let xr = x.as_ptr() as usize..x.as_ptr() as usize + 12;
        assert!(!xr.contains(&(y.as_ptr() as usize)), "the two allocations do not overlap");
    }

    /// A top-down arena hands out the END of its buffer: what it has handed
    /// out is exactly the last `used()` bytes, every allocation aligned,
    /// disjoint and below the one before; full is reported, not a panic.
    #[test]
    fn a_top_down_arena_hands_out_the_end_of_its_buffer() {
        let mut buf = [0u8; 1024];
        let base = buf.as_ptr() as usize + 1;
        let a = Arena::new_top_down(&mut buf[1..]);
        let x = a.alloc_filled::<u32>(3, 7).expect("u32s");
        let y = a.alloc_copy::<u64>(&[1, 2]).expect("u64s");
        let z = a.alloc_str("abc").expect("a str");
        assert_eq!((x.as_ptr() as usize % 4, y.as_ptr() as usize % 8), (0, 0), "aligned");
        assert_eq!((&x[..], &y[..], z), (&[7u32, 7, 7][..], &[1u64, 2][..], "abc"));
        let (xs, ys, zs) = (x.as_ptr() as usize, y.as_ptr() as usize, z.as_ptr() as usize);
        assert!(zs + 3 <= ys && ys + 16 <= xs && xs + 12 <= base + 1023, "each below the one before");
        assert_eq!(zs, base + 1023 - a.used(), "the handed-out bytes are the buffer's last used()");
        /* The latest allocation given back moves the bump pointer back up
         * (release_last's top-down arm; without it the bytes are only
         * remembered as a free block and used() stays). */
        let used = a.used();
        // SAFETY: nothing reads `z` again.
        unsafe { a.give_back(z.as_bytes()) };
        assert_eq!(a.used(), used - 3, "given back at the bottom");
        let full = Arena::new_top_down(&mut buf[..16]);
        assert!(full.alloc_filled::<u32>(4, 0).is_ok(), "16 bytes hold four u32");
        assert_eq!(full.alloc_filled::<u32>(1, 0), Err(ArenaError::OutOfScratch), "the next one does not fit");
        assert_eq!(full.left(), 0);
    }

    /// Top-down, a vector cannot grow in place (its start would move): it
    /// moves, keeps its elements, and its old buffers are recycled without
    /// aliasing, as upward.
    #[test]
    fn a_top_down_vector_grows_by_moving_and_keeps_its_contents() {
        let mut buf = std::vec![0u8; 1 << 20];
        let a = Arena::new_top_down(&mut buf);
        let (mut x, mut y) = (AVec::<u64>::new(), AVec::<u64>::new());
        for i in 0..4000u64 {
            x.push(&a, i).expect("room");
            y.push(&a, !i).expect("room");
        }
        let small = a.alloc_filled(4, 9u64).expect("room");
        assert!(x.as_slice().iter().copied().eq(0..4000), "x kept its elements");
        assert!(y.as_slice().iter().copied().eq((0..4000).map(|i: u64| !i)), "y kept its elements");
        assert_eq!(*small, [9u64; 4]);
        assert!(a.abandoned() > 0, "the vectors moved");
    }

    /// `grow_last` never extends a top-down allocation. Its upward test
    /// (the allocation's end is `base + head`) can hold top-down by
    /// coincidence: 16 bytes then 32 in a 64-byte buffer put the second at
    /// `[16, 48)`, and `head` is 48. Growing it would hand out `[48, 56)`,
    /// which the first allocation owns. Revert check: without the
    /// `top_down` arm of `grow_last` this fails.
    #[test]
    fn a_top_down_allocation_is_never_grown_in_place() {
        let mut buf = [0u8; 64];
        let base = buf.as_ptr() as usize;
        let a = Arena::new_top_down(&mut buf);
        let first = a.alloc_filled::<u8>(16, 1).expect("room");
        let second = a.alloc_filled::<u8>(32, 2).expect("room");
        let end = second.as_ptr() as usize + second.len();
        assert_eq!((end, a.used()), (base + 48, 48), "the coincidence this test is about");
        assert!(!a.grow_last(end as *mut u8, 8), "grown into the first allocation");
        assert_eq!(*first, [1u8; 16]);
    }

    #[test]
    fn a_full_arena_reports_rather_than_panics() {
        let mut buf = [0u8; 16];
        let a = Arena::new(&mut buf);
        assert!(a.alloc_filled::<u32>(4, 0).is_ok(), "16 bytes hold four u32");
        assert_eq!(a.alloc_filled::<u32>(1, 0), Err(ArenaError::OutOfScratch), "the next one does not fit");
        assert_eq!(a.left(), 0);
    }

    #[test]
    fn a_vec_keeps_its_contents_across_growth() {
        let mut buf = [0u8; 4096];
        let a = Arena::new(&mut buf);
        let mut v = AVec::<u32>::new();
        for i in 0..100u32 {
            v.push(&a, i).expect("push");
        }
        assert_eq!(v.len(), 100);
        assert!(v.as_slice().iter().copied().eq(0..100), "growth kept every element in order");
        assert_eq!(v.pop(), Some(99));
        assert_eq!(v.into_slice().len(), 99, "the copies did not lose an element");
    }

    #[test]
    fn a_vec_that_outgrows_the_arena_reports_it() {
        let mut buf = [0u8; 64];
        let a = Arena::new(&mut buf);
        let mut v = AVec::<u32>::new();
        let mut err = None;
        for i in 0..1000u32 {
            if let Err(e) = v.push(&a, i) {
                err = Some(e);
                break;
            }
        }
        assert_eq!(err, Some(ArenaError::OutOfScratch), "growth stops with a reason, not a panic");
    }

    /// A vector growing alone grows where it is: no copy, nothing left
    /// behind. Revert check: drop the `grow_last` arm of `reserve` and the
    /// arena uses about twice the bytes.
    #[test]
    fn a_vector_that_is_the_last_allocation_grows_in_place() {
        let mut buf = std::vec![0u8; 1 << 16];
        let a = Arena::new(&mut buf);
        let mut v = AVec::<u32>::new();
        for i in 0..1000u32 {
            v.push(&a, i).expect("room");
        }
        assert!(v.as_slice().iter().copied().eq(0..1000));
        assert!(a.used() <= 1024 * 4 + 8, "grew in place: {} bytes used", a.used());
        assert_eq!(a.abandoned(), 0);
    }

    /// Two vectors growing in turn: each move leaves its old buffer behind,
    /// and later allocations are carved from those blocks instead of taking
    /// fresh bytes -- without the recycled memory ever aliasing a live
    /// element. Revert check: make `recycle` forget the block and the
    /// small allocations below move `used`.
    #[test]
    fn a_moved_vectors_old_buffer_is_reused_and_never_aliased() {
        let mut buf = std::vec![0u8; 1 << 20];
        let a = Arena::new(&mut buf);
        let (mut x, mut y) = (AVec::<u64>::new(), AVec::<u64>::new());
        for i in 0..4000u64 {
            x.push(&a, i).expect("room");
            y.push(&a, !i).expect("room");
        }
        let before = a.used();
        assert!(a.abandoned() > 30_000, "the two vectors moved: {}", a.abandoned());
        let mut smalls = std::vec::Vec::new();
        for k in 0..500u64 {
            smalls.push(a.alloc_filled(4, k).expect("room"));
        }
        assert_eq!(a.used(), before, "16 KB of small allocations came out of the moved buffers");
        assert!(x.as_slice().iter().copied().eq(0..4000), "x kept its elements");
        assert!(y.as_slice().iter().copied().eq((0..4000).map(|i: u64| !i)), "y kept its elements");
        for (k, s) in smalls.iter().enumerate() {
            assert_eq!(**s, [k as u64; 4], "no two allocations overlap");
        }
    }

    #[test]
    fn strings_are_copied_into_the_arena() {
        let mut buf = [0u8; 64];
        let a = Arena::new(&mut buf);
        let s = a.alloc_str("v_uv").expect("str");
        assert_eq!(s, "v_uv");
        assert!(a.used() >= 4);
    }
}
