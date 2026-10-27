// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.

//! Where a draw's CPU time goes, step by step, on the OSAL's clock
//! (`osal::clock_ns`, OSAL_QUERY_CLOCK_NS), reported to the OSAL.
//!
//! The question it answers: the i.MX8MP cluster's frame 2 spent about
//! 2.75 s inside glDraw* with the online compiler, none of it compiling and
//! about none of it waiting for the GPU (CHECK_RUN, 2026-10-05). The GL
//! core's `build_common` and the GC7000 backend's `draw_one` each cut their
//! work into [`Step`]s with a [`Timer`]. When the outermost timer of a
//! scope ends, the scope's steps go to the OSAL in one
//! OSAL_QUERY_DRAW_STATS ([`OsalDrawStats`]); the OSAL keeps the totals and
//! an application reads them there (zephyr_timing.c `osal_draw_stats`), as
//! it reads the OSAL's GPU waits. The library keeps no totals and exports
//! nothing of its own for it.
//!
//! A step's time is exclusive: a draw nested inside another step (a
//! pending clear the GL core draws on its own) is counted under its own
//! steps and taken out of the outer step's, and the scope is reported once,
//! by the outermost timer. A lap that could not read the clock is counted
//! in `untimed`, never as zero time.

use crate::osal;
use crate::util::Global;
use core::sync::atomic::{AtomicUsize, Ordering};

/// One timed step. The numbering is `struct osal_draw_stats` ns index:
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
    /// GL core: the textures each sampler reads, and the depth a sampled
    /// depth texture is brought up to first (texture.rs `resolve_depth`: the
    /// wait for the kick that rendered it and the copy into its texels).
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

/// `struct osal_draw_stats` (osal_internal.h): one scope's time per step.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct OsalDrawStats {
    pub ns: [u64; STEPS],
    /// Laps either end of which the clock could not read.
    pub untimed: u32,
    pub reserved: u32,
}
/* zephyr_timing.c pins the C side to the same size: 8 * OSAL_DRAW_STEPS + 8 */
const _: () = assert!(STEPS == 12 && core::mem::size_of::<OsalDrawStats>() == 8 * STEPS + 8);

/// The scope being timed: its steps so far, the running sum the exclusive
/// laps subtract from, and how many timers are live in it.
struct Scope {
    stats: OsalDrawStats,
    sum: u64,
    depth: u32,
}

static SCOPE: Global<Scope> =
    Global::new(Scope { stats: OsalDrawStats { ns: [0; STEPS], untimed: 0, reserved: 0 }, sum: 0, depth: 0 });

fn scope() -> &'static mut Scope {
    // SAFETY: single-threaded by contract (the GL entry points); each
    // function here takes it once and lets go before it returns.
    unsafe { SCOPE.get() }
}

/// The OSAL table (`osal::g_osal`'s address) whose first report answered
/// -ENOTSUP, or 0. While the installed table is that one nothing is timed
/// or reported: no clock reads, no laps, no report. Another table (an OSAL
/// installed later, one that may keep totals) is a new answer, so it is
/// asked again.
static OFF_FOR: AtomicUsize = AtomicUsize::new(0);

fn ops_id() -> usize {
    osal::ops().map_or(0, |o| o as *const osal::OsalOps as usize)
}

/// The installed OSAL has said it keeps no totals.
fn off() -> bool {
    let id = OFF_FOR.load(Ordering::Relaxed);
    if id == 0 {
        return false;
    }
    if id == ops_id() {
        return true;
    }
    OFF_FOR.store(0, Ordering::Relaxed);
    false
}

/// Times consecutive steps: each [`Timer::lap`] counts the time since the
/// previous lap (or [`Timer::start`]) under a step, less what nested timers
/// counted meanwhile. The outermost timer reports the scope when it ends.
pub struct Timer {
    at: Option<u64>,
    counted: u64,
}

impl Timer {
    pub fn start() -> Timer {
        let s = scope();
        s.depth += 1;
        Timer { at: if off() { None } else { osal::clock_ns() }, counted: s.sum }
    }

    /// The time since the last lap, less any nested timer's, under `step`.
    pub fn lap(&mut self, step: Step) {
        if off() {
            return;
        }
        let now = osal::clock_ns();
        let s = scope();
        match (self.at, now) {
            (Some(a), Some(b)) => {
                let nested = s.sum.wrapping_sub(self.counted);
                let own = b.saturating_sub(a).saturating_sub(nested);
                s.stats.ns[step as usize] = s.stats.ns[step as usize].wrapping_add(own);
                s.sum = s.sum.wrapping_add(own);
            }
            _ => s.stats.untimed = s.stats.untimed.wrapping_add(1),
        }
        self.at = now;
        self.counted = s.sum;
    }
}

impl Drop for Timer {
    fn drop(&mut self) {
        let s = scope();
        s.depth -= 1;
        if s.depth != 0 {
            return;
        }
        let mut stats = core::mem::take(&mut s.stats);
        s.sum = 0;
        if off() {
            return;
        }
        /* an OSAL that keeps no totals answers -ENOTSUP: stop timing */
        if osal::report_draw_stats(&mut stats) == -crate::util::ENOTSUP {
            OFF_FOR.store(ops_id(), Ordering::Relaxed);
        }
    }
}
