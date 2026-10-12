// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! What the Vivante backend hands `gles_osal.h`'s `submit()`.
//!
//! The GC7000 has no firmware: a submit is a command stream in GPU memory
//! that the front end fetches. The OSAL runs that stream -- by pointing the
//! front end at it, or by linking it into a ring it owns -- and returns a
//! fence that stands for its completion.
//!
//! The backend writes only its own commands. Everything that makes the stream
//! runnable and observable belongs to the OSAL, which is the side that knows
//! the hardware:
//!
//!   - the EVENT command that raises the completion interrupt. The GC7000 has
//!     30 events and recycles them, so a raw event id cannot be chosen by a
//!     caller that does not know which are in flight. The backend must NOT
//!     emit an EVENT, an END or a LINK.
//!   - the cache flushes and the pipeline stalls the epilogue needs before
//!     the fence is allowed to fire.
//!   - [`GC_KICK_TAIL_BYTES`] of scratch at the end of the stream's
//!     allocation.
//!
//! This is the whole GPU-specific part of the OSAL contract for this backend,
//! so it lives in its own module: the OSAL includes `gc_kick.h`, the C view of
//! this, not the backend.

/// First word of the descriptor, so an OSAL built for another GPU rejects it
/// instead of reading the wrong struct ("GCKK").
pub const GC_KICK_MAGIC: u32 = 0x474b_4b43;

/// Scratch the OSAL may write immediately after the stream, so the backend
/// stops this far short of the end of its command buffer.
///
/// An OSAL that links the stream into a ring rather than copying it needs to
/// write a LINK back to the ring in the eight bytes at `stream_bytes`; that is
/// what the Zephyr driver's render service does (`gc_render.h`,
/// `GC_CMDBUF_TAIL`). Reserving the room unconditionally costs two words per
/// frame and keeps the backend free of a per-OSAL rule.
pub const GC_KICK_TAIL_BYTES: u32 = 8;

/// `struct gc_kick`: the submit descriptor.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct GcKick {
    /// [`GC_KICK_MAGIC`].
    pub magic: u32,
    /// Length of the command stream, a multiple of 8.
    pub stream_bytes: u32,
    /// GPU address of the first command word.
    pub stream_gpu: u64,
}

impl GcKick {
    pub const ZERO: GcKick = GcKick { magic: 0, stream_bytes: 0, stream_gpu: 0 };
}

/// KICK_DIFF: read the render target twice after a kick, to tell a late
/// write from a wrong one (tools/cts/log/investigations/gc-kick-diff.md).
///
/// i.MX8MP gl_test cases 10 and 2 (#9, #25) lose the pixels of a tiny draw
/// that follows a clear in one kick. Two stories fit: the pixel engine's
/// write lands after the fence fired (a late write), or it never lands (a
/// lost or wrong write). The first read happens as soon as the fence is seen.
/// The second happens `delay_us` later, behind a load barrier. If the second
/// read differs from the first, the first came too early. If they are equal,
/// the write did not arrive in `delay_us`.
///
/// A test knob: it exists only with the `test-knobs` feature, and it is off
/// until a harness arms it ([`gc_kick_diff_enable`]). Without the feature
/// none of this is in the build. It never writes to the target and never
/// touches the command stream; it only reads memory the GPU wrote.
#[cfg(feature = "test-knobs")]
pub mod diff {
    use core::sync::atomic::{AtomicU32, Ordering};

    /// Words each read can hold: the 64x64 RGBA8 surface gl_test draws to
    /// (16 KiB). A larger watch is cut to this.
    pub const KICK_DIFF_MAX_WORDS: usize = 64 * 64;

    /// How the two reads compare.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum Verdict {
        /// The two reads are equal. The target was right at the fence, or the
        /// missing write did not arrive in the delay: the instrument alone
        /// cannot say which (the case's own check does).
        Stable,
        /// The second read differs: words changed after the fence. The fence
        /// fired before the pixel engine's write-back landed.
        Late,
    }

    /// The difference of two reads of the same words.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct Diff {
        /// Words compared (the shorter of the two reads).
        pub words: u32,
        /// Words that differ.
        pub changed: u32,
        /// Index of the first changed word (`words` when none).
        pub first: u32,
        /// Index of the last changed word (0 when none).
        pub last: u32,
        /// Rows holding a changed word (rows of `pitch_words`; pitch 0: one row).
        pub rows: u32,
        /// The first changed word, in the first and in the second read.
        pub first_before: u32,
        pub first_after: u32,
    }

    impl Diff {
        pub const fn verdict(&self) -> Verdict {
            if self.changed == 0 {
                Verdict::Stable
            } else {
                Verdict::Late
            }
        }
    }

    /// Compare two reads. `pitch_words` is the row length in words, to count
    /// the rows the changes touch.
    pub fn compare(first: &[u32], second: &[u32], pitch_words: usize) -> Diff {
        let n = first.len().min(second.len());
        let mut d =
            Diff { words: n as u32, changed: 0, first: n as u32, last: 0, rows: 0, first_before: 0, first_after: 0 };
        let mut last_row = usize::MAX;

        for (i, (&a, &b)) in first.iter().zip(second.iter()).enumerate() {
            if a == b {
                continue;
            }
            if d.changed == 0 {
                d.first = i as u32;
                d.first_before = a;
                d.first_after = b;
            }
            d.changed += 1;
            d.last = i as u32;
            let row = i.checked_div(pitch_words).unwrap_or(0);
            if row != last_row {
                d.rows += 1;
                last_row = row;
            }
        }
        d
    }

    /// What the harness armed.
    struct Watch {
        ptr: *const u32,
        words: usize,
        pitch_words: usize,
        delay_us: u32,
        delay: Option<unsafe extern "C" fn(u32)>,
    }

    static mut WATCH: Watch = Watch { ptr: core::ptr::null(), words: 0, pitch_words: 0, delay_us: 0, delay: None };
    static mut FIRST: [u32; KICK_DIFF_MAX_WORDS] = [0; KICK_DIFF_MAX_WORDS];
    static mut SECOND: [u32; KICK_DIFF_MAX_WORDS] = [0; KICK_DIFF_MAX_WORDS];

    /// Counters for the harness's record: checks made, checks with a late
    /// write, and the most words one check saw change.
    static CHECKS: AtomicU32 = AtomicU32::new(0);
    static LATE: AtomicU32 = AtomicU32::new(0);
    static MAX_CHANGED: AtomicU32 = AtomicU32::new(0);

    /// Arm the instrument: watch `words` words at `ptr` (the CPU view of the
    /// render target, a row `pitch_words` long). `delay` waits `delay_us`
    /// microseconds (the OSAL has no sleep op; the harness has one); with
    /// None the second read follows at once, which still catches a CPU-side
    /// reordering. A null `ptr` or 0 `words` disarms. Resets the counters.
    ///
    /// # Safety
    /// `ptr` must stay readable for `words` words until it is disarmed.
    #[no_mangle]
    pub unsafe extern "C" fn gc_kick_diff_enable(
        ptr: *const u32,
        words: u32,
        pitch_words: u32,
        delay_us: u32,
        delay: Option<unsafe extern "C" fn(u32)>,
    ) {
        let words = (words as usize).min(KICK_DIFF_MAX_WORDS);
        // SAFETY: single-threaded by contract, as every test knob.
        unsafe {
            core::ptr::addr_of_mut!(WATCH).write(Watch {
                ptr: if words == 0 { core::ptr::null() } else { ptr },
                words,
                pitch_words: pitch_words as usize,
                delay_us,
                delay,
            });
        }
        CHECKS.store(0, Ordering::Relaxed);
        LATE.store(0, Ordering::Relaxed);
        MAX_CHANGED.store(0, Ordering::Relaxed);
    }

    /// Checks made, checks that saw a late write, the most words one check
    /// saw change, written to `out[0..3]`. For the harness's record.
    ///
    /// # Safety
    /// `out` is null or points at 3 writable `u32`.
    #[no_mangle]
    pub unsafe extern "C" fn gc_kick_diff_stats(out: *mut u32) {
        if out.is_null() {
            return;
        }
        // SAFETY: the caller passes 3 words.
        unsafe {
            out.write(CHECKS.load(Ordering::Relaxed));
            out.add(1).write(LATE.load(Ordering::Relaxed));
            out.add(2).write(MAX_CHANGED.load(Ordering::Relaxed));
        }
    }

    fn load_barrier() {
        #[cfg(target_arch = "aarch64")]
        // SAFETY: a barrier; it touches no memory or register the compiler uses.
        unsafe {
            core::arch::asm!("dmb ishld", options(nostack, preserves_flags))
        };
        core::sync::atomic::fence(Ordering::Acquire);
    }

    /// Read `dst.len()` words at `src` as glReadPixels would see them: the
    /// CPU's view is invalidated first (`read_pixels_sync` does the same), so
    /// on a cached mapping the read reaches memory instead of a line the
    /// cache kept from before the kick. Without it a late write could never
    /// show: both reads would hit the same stale lines and compare equal.
    /// The i.MX8MP maps the carveout Normal non-cacheable, where the
    /// invalidate changes nothing.
    fn read_into(dst: &mut [u32], src: *const u32) {
        // SAFETY: `gc_kick_diff_enable`'s contract: `src` is readable (and
        // the target's memory) for `dst.len()` words.
        unsafe { crate::osal::cache_invd(src as *mut core::ffi::c_void, core::mem::size_of_val(dst)) };
        for (i, w) in dst.iter_mut().enumerate() {
            // SAFETY: `gc_kick_diff_enable`'s contract: `src` is readable for
            // `dst.len()` words. Volatile: the GPU writes it, not this thread.
            *w = unsafe { src.add(i).read_volatile() };
        }
    }

    /// Call it once the kick's fence is seen (after `glFinish`, or in the
    /// backend after `wait_fence`): read the target, wait, read it again,
    /// compare, report. Does nothing when not armed. Returns 0 when off, 1
    /// for [`Verdict::Stable`], 2 for [`Verdict::Late`].
    #[no_mangle]
    pub extern "C" fn gc_kick_diff_after_fence() -> i32 {
        // SAFETY: single-threaded by contract; WATCH is plain data.
        let (ptr, words, pitch, delay_us, delay) = unsafe {
            let w = &*core::ptr::addr_of!(WATCH);
            (w.ptr, w.words, w.pitch_words, w.delay_us, w.delay)
        };
        if ptr.is_null() || words == 0 {
            return 0;
        }
        // SAFETY: the two buffers are this function's; no other borrow exists,
        // and `words` is at most their length (`gc_kick_diff_enable` cuts it).
        let (first, second) = unsafe {
            (
                core::slice::from_raw_parts_mut(core::ptr::addr_of_mut!(FIRST).cast::<u32>(), words),
                core::slice::from_raw_parts_mut(core::ptr::addr_of_mut!(SECOND).cast::<u32>(), words),
            )
        };

        load_barrier();
        read_into(first, ptr);
        if let Some(wait) = delay {
            // SAFETY: the harness's own delay routine.
            unsafe { wait(delay_us) };
        }
        load_barrier();
        read_into(second, ptr);
        report(&compare(first, second, pitch), delay_us)
    }

    /// Count the check and log it: every late one, and the first stable one.
    fn report(d: &Diff, delay_us: u32) -> i32 {
        let n = CHECKS.fetch_add(1, Ordering::Relaxed);
        MAX_CHANGED.fetch_max(d.changed, Ordering::Relaxed);
        match d.verdict() {
            Verdict::Stable => {
                if n == 0 {
                    crate::glog!(
                        "yakogl/vivante: KICK_DIFF stable ({} words, second read {} us later)",
                        d.words,
                        delay_us
                    );
                }
                1
            }
            Verdict::Late => {
                LATE.fetch_add(1, Ordering::Relaxed);
                crate::glog!(
                    "yakogl/vivante: KICK_DIFF late: {} of {} words changed after the fence, rows {}, first word {} {:#010x} -> {:#010x}, last {}, second read {} us later",
                    d.changed,
                    d.words,
                    d.rows,
                    d.first,
                    d.first_before,
                    d.first_after,
                    d.last,
                    delay_us
                );
                2
            }
        }
    }
}
