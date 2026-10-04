// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.

//! Where a draw's CPU time goes, step by step, on the OSAL's clock
//! (`osal::clock_ns`, OSAL_QUERY_CLOCK_NS).
//!
//! The question it answers: the i.MX8MP cluster's frame 2 spent about
//! 2.75 s inside glDraw* with the online compiler, none of it compiling and
//! about none of it waiting for the GPU (CHECK_RUN, 2026-10-05). The GL
//! core's `build_common` and the GC7000 backend's `draw_one` each cut their
//! work into [`Step`]s with a [`Timer`]; [`yakogl_draw_steps`] hands the
//! running totals to the application, which reads them as differences over
//! a frame and records the slowest (record.rs CHECK_RUN_STEP_*).
//!
//! A step's time is exclusive: a draw nested inside another step (a
//! pending clear the GL core draws on its own) is counted under its own
//! steps and taken out of the outer step's. A lap that could not read the
//! clock is counted in `untimed`, never as zero time; an OSAL without a
//! clock makes [`yakogl_draw_steps`] return -ENOTSUP.

use crate::osal;
use crate::util::Global;

/// One timed step. The numbering is the C view's ([`yakogl_draw_steps`]):
/// record.rs's CHECK_RUN_STEP_* are these plus [`RECORD_STEP_BASE`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(usize)]
pub enum Step {
    /// GL core: the draw's target, the program's uploaded code, its
    /// sampler-state and blend variants, and the pending clear's fusion.
    GlProgram = 0,
    /// GL core: the uniform blocks filled and staged.
    GlUniforms,
    /// GL core: the vertex attributes resolved, converted and staged.
    GlAttribs,
    /// GL core: the textures each sampler reads.
    GlTextures,
    /// GL core: the fixed-function state.
    GlState,
    /// GC7000: the refusal checks, the multisample set-up and the draw's
    /// bookkeeping around the steps below.
    GcRest,
    /// GC7000: the depth/stencil surface chosen (`ds_cache::select`; a
    /// plane moved on the CPU is here).
    GcDs,
    /// GC7000: the tiled copies of mipmapped layered textures.
    GcTiledCopy,
    /// GC7000: a frame submitted and the next opened (a wait for a slot's
    /// kick is here), and memory for a draw too large for the arena.
    GcFrame,
    /// GC7000: the uniform blocks copied into the stream or memory.
    GcUniforms,
    /// GC7000: the vertex input and the indices.
    GcVertex,
    /// GC7000: the clear and the draw written into the stream.
    GcEmit,
}

/// How many [`Step`]s there are.
pub const STEPS: usize = Step::GcEmit as usize + 1;

/// record.rs numbers the steps after its two the application measures
/// itself (the other GL calls, and glDraw* outside these steps).
pub const RECORD_STEP_BASE: u32 = 2;

struct Totals {
    ns: [u64; STEPS],
    /// Every ns counted under any step, for [`Timer`]'s exclusive laps.
    sum: u64,
    /// Laps either end of which the clock could not read.
    untimed: u32,
}

static TOTALS: Global<Totals> = Global::new(Totals { ns: [0; STEPS], sum: 0, untimed: 0 });

fn totals() -> &'static mut Totals {
    // SAFETY: single-threaded by contract (the GL entry points); each
    // function here takes it once and lets go before it returns.
    unsafe { TOTALS.get() }
}

/// Times consecutive steps: each [`Timer::lap`] counts the time since the
/// previous lap (or [`Timer::start`]) under a step, less what nested timers
/// counted meanwhile.
pub struct Timer {
    at: Option<u64>,
    counted: u64,
}

impl Timer {
    pub fn start() -> Timer {
        Timer { at: osal::clock_ns(), counted: totals().sum }
    }

    /// The time since the last lap, less any nested timer's, under `step`.
    pub fn lap(&mut self, step: Step) {
        let now = osal::clock_ns();
        let t = totals();
        match (self.at, now) {
            (Some(a), Some(b)) => {
                let nested = t.sum.wrapping_sub(self.counted);
                let own = b.saturating_sub(a).saturating_sub(nested);
                t.ns[step as usize] = t.ns[step as usize].wrapping_add(own);
                t.sum = t.sum.wrapping_add(own);
            }
            _ => t.untimed = t.untimed.wrapping_add(1),
        }
        self.at = now;
        self.counted = t.sum;
    }
}

/// The running totals, in ns, by [`Step`] number into `out` (`n` entries,
/// at most [`STEPS`]) and the laps that could not be timed into `untimed`
/// (may be null). Returns the number of entries written, or -ENOTSUP when
/// the OSAL has no clock (nothing is written then: not measured, not zero).
///
/// # Safety
/// `out` is writable for `n` u64s; `untimed` is null or writable.
#[no_mangle]
pub unsafe extern "C" fn yakogl_draw_steps(out: *mut u64, n: usize, untimed: *mut u32) -> i32 {
    if osal::clock_ns().is_none() {
        return -crate::util::ENOTSUP;
    }
    if out.is_null() {
        return -crate::util::EINVAL;
    }
    let t = totals();
    let n = n.min(STEPS);
    // SAFETY: the caller's contract; n is at most what it gave.
    unsafe { core::ptr::copy_nonoverlapping(t.ns.as_ptr(), out, n) };
    if !untimed.is_null() {
        // SAFETY: as above.
        unsafe { untimed.write(t.untimed) };
    }
    n as i32
}
