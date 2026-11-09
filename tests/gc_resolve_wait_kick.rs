// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GC7000 (i.MX8MP): the CPU depth resolve (ds_cache.rs `resolve_depth`)
//! waits for the kick that last wrote the depth surface, and for nothing
//! after it (`kick::wait_kick`), where it used to wait for every kick slot.
//!
//! How it runs: the real Vivante back end (`gc_init`, `gc_draw`) over a
//! stand-in OSAL whose kicks stay in flight until something waits for them.
//! The stand-in GPU "renders" when a kick completes: every draw a test makes
//! writes its own depth value into its surface's D24S8 words at the moment
//! the fence of the kick it was recorded in is waited for. A resolve that
//! reads before that kick completed, or after waiting for the wrong one,
//! answers another draw's depth.
//!
//! * Two kicks in flight, only the first wrote the depth: the resolve waits
//!   for the first; the second is still running after it.
//! * The newest kick wrote the depth (the open frame): it is submitted and
//!   waited for.
//! * No kick pending: no wait, nothing submitted.
//! * The depth-writing draw spilled into the next kick: a full stream (or
//!   arena) submitted the frame open when it was selected, or `select`
//!   copied a plane and finished it. The resolve waits for the kick the
//!   draw is in, not the one open when it began.
//!
//! Every case's pixels are also read after `gc_finish` (the old full
//! wait): they match.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online --test gc_resolve_wait_kick
//! ```

#![cfg(all(feature = "host-test", feature = "gpu-vivante", feature = "shaderc-online"))]

use core::ffi::c_void;
use std::collections::BTreeMap;
use std::sync::Mutex;
use yakogl::backend::{GlesDraw, GlesDrawTarget, GLES_SURF_RGBA8888};
use yakogl::ds_retention::unorm24_depth;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};
use yakogl::vivante::backend::{
    gc_depth_resolve, gc_draw, gc_finish, gc_flush, gc_init, gc_kick_id, gc_kick_pending, gc_shutdown, gc_wait_kick,
    yakogl_backend_vivante,
};
use yakogl::vivante::kick::{GcKick, GC_KICK_MAGIC};

/* ---- the stand-in OSAL: host memory, kicks that complete when waited for ---------- */

/// A depth write the stand-in GPU makes when kick `fence` completes.
struct Write {
    fence: i64,
    /// The surface's pixels: allocation base and the guard before them.
    surface: u64,
    guard: usize,
    depth: u32,
}

#[derive(Default)]
struct Platform {
    allocs: BTreeMap<u64, Box<[u64]>>,
    bump: BTreeMap<u64, u64>,
    /// The last fence handed out, and the last one completed.
    fence: i64,
    done: i64,
    /// Every fence waited for, in order.
    waits: Vec<i64>,
    writes: Vec<Write>,
    faults: Vec<String>,
}

static PLATFORM: Mutex<Option<Platform>> = Mutex::new(None);
/// Serialises the tests: the OSAL pointer and the back end are process-global.
static SERIAL: Mutex<()> = Mutex::new(());

fn with_platform<R>(f: impl FnOnce(&mut Platform) -> R) -> R {
    let mut g = PLATFORM.lock().unwrap_or_else(|e| e.into_inner());
    f(g.get_or_insert_with(Platform::default))
}

unsafe extern "C" fn p_gpu_alloc(heap_base: u64, size: usize, _flags: u32, out: *mut OsalGpuMem) -> i32 {
    if size == 0 || size > 1 << 30 {
        return -12;
    }
    with_platform(|p| {
        let mut mem = vec![0u64; size.div_ceil(8)].into_boxed_slice();
        let at = p.bump.entry(heap_base).or_insert(0);
        let gpu = heap_base + 0x1000 + *at;
        *at += (size as u64).div_ceil(4096) * 4096;
        let cpu = mem.as_mut_ptr() as *mut c_void;
        // SAFETY: the caller passes a writable descriptor.
        unsafe { *out = OsalGpuMem { cpu, gpu, phys: gpu, size, impl_: cpu } };
        p.allocs.insert(gpu, mem);
        0
    })
}

unsafe extern "C" fn p_gpu_free(mem: *mut OsalGpuMem) -> i32 {
    // SAFETY: the caller passes a live descriptor.
    let m = unsafe { *mem };
    with_platform(|p| {
        /* leaked, not dropped: a write still queued for it stays harmless */
        if let Some(b) = p.allocs.remove(&m.gpu) {
            core::mem::forget(b);
        }
    });
    // SAFETY: as above; a freed descriptor is cleared (the OSAL contract).
    unsafe { *mem = OsalGpuMem::ZERO };
    0
}

unsafe extern "C" fn p_cache(_cpu: *mut c_void, _n: usize) {}

unsafe extern "C" fn p_submit(kick: *const c_void, len: usize) -> i64 {
    if len != core::mem::size_of::<GcKick>() {
        return -22;
    }
    // SAFETY: the back end passes a live GcKick of that size.
    let k = unsafe { *(kick as *const GcKick) };
    with_platform(|p| {
        if k.magic != GC_KICK_MAGIC {
            p.faults.push(format!("submit with magic 0x{:x}", k.magic));
            return -22;
        }
        p.fence += 1;
        p.fence
    })
}

/// Kick `fence` and every one before it complete: their depth writes land.
fn complete(p: &mut Platform, fence: i64) {
    if fence > p.fence {
        p.faults.push(format!("wait for fence {fence}, which was never handed out (last {})", p.fence));
    }
    p.done = p.done.max(fence.min(p.fence));
    let done = p.done;
    let (now, later): (Vec<Write>, Vec<Write>) =
        core::mem::take(&mut p.writes).into_iter().partition(|w| w.fence <= done);
    p.writes = later;
    for w in now {
        let Some(mem) = p.allocs.get_mut(&w.surface) else { continue };
        // SAFETY: the allocation's own memory, as u32 words.
        let words = unsafe { core::slice::from_raw_parts_mut(mem.as_mut_ptr() as *mut u32, mem.len() * 2) };
        let px = &mut words[w.guard / 4..(w.guard + SURFACE_BYTES) / 4];
        px.iter_mut().for_each(|v| *v = (w.depth << 8) | (*v & 0xff));
    }
}

unsafe extern "C" fn p_wait(fence: i64, _timeout_us: i64) -> i32 {
    with_platform(|p| {
        p.waits.push(fence);
        complete(p, fence);
    });
    0
}

unsafe extern "C" fn p_completed() -> i64 {
    with_platform(|p| p.done)
}

unsafe extern "C" fn p_query(what: u32, out: *mut c_void, out_sz: usize) -> i32 {
    use yakogl::osal::{OSAL_QUERY_DRAW_STATS, OSAL_QUERY_GC_FRAME_OPEN};
    if (what == OSAL_QUERY_GC_FRAME_OPEN && out_sz == 4 && !out.is_null()) || what == OSAL_QUERY_DRAW_STATS {
        return 0;
    }
    -22
}

static OSAL: OsalOps = OsalOps {
    init: None,
    shutdown: None,
    gpu_alloc: Some(p_gpu_alloc),
    gpu_free: Some(p_gpu_free),
    mmu_map: None,
    map_phys: None,
    gpu_write: None,
    gpu_read: None,
    cache_flush: Some(p_cache),
    cache_invd: Some(p_cache),
    submit: Some(p_submit),
    wait: Some(p_wait),
    completed: Some(p_completed),
    reg_read: None,
    reg_write: None,
    query: Some(p_query),
    log: None,
    wait_vblank: None,
    present: None,
    lock: None,
    unlock: None,
    thread_id: None,
    display_busy: None,
};

/* ---- the rig -------------------------------------------------------------------------- */

const SIZE: u32 = 64;
/// A 64 x 64 surface: 64-aligned both ways, a word a pixel.
const SURFACE_BYTES: usize = (SIZE * SIZE * 4) as usize;
const TARGET_A: u64 = 0x7000_0000;
const TARGET_B: u64 = 0x7010_0000;

/// The back end up over the stand-in, for the test's lifetime.
struct Rig {
    _serial: std::sync::MutexGuard<'static, ()>,
    /// Per depth/stencil pair: its surface's allocation and guard.
    surfaces: BTreeMap<(u32, u32), (u64, usize)>,
    /// The next depth a draw writes (24-bit).
    next_depth: u32,
}

impl Rig {
    fn new() -> Rig {
        let serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        with_platform(|p| *p = Platform::default());
        // SAFETY: the guard serialises the tests; the tables are statics.
        unsafe {
            osal_set_ops(&OSAL);
            yakogl::backend::g_backend = &yakogl_backend_vivante;
            assert_eq!(gc_init(), 0, "gc_init");
        }
        let rig = Rig { _serial: serial, surfaces: BTreeMap::new(), next_depth: 0x10_0000 };
        assert_eq!(gc_kick_id() as i64 - 1, with_platform(|p| p.fence), "kick ids and fences run together");
        rig
    }

    /// A draw into `target` that writes the depth of the pair (depth `d`,
    /// stencil `s`) everywhere (a scissored depth clear, drawn as a quad:
    /// a draw the back end records with its own shaders). The stand-in GPU
    /// writes the draw's own depth when the kick it is in completes; that
    /// depth is returned.
    fn draw(&mut self, target: u64, d: u32, s: u32) -> u32 {
        let mut gl = GlesDraw::ZERO;
        gl.target_gpu = target;
        gl.target_w = SIZE;
        gl.target_h = SIZE;
        gl.target_format = GLES_SURF_RGBA8888;
        gl.targets[0] = GlesDrawTarget { gpu: target, w: SIZE, h: SIZE, format: GLES_SURF_RGBA8888, out_shift: 0 };
        gl.viewport = [0, 0, SIZE as i32, SIZE as i32];
        gl.depth_range = [0.0, 1.0];
        gl.line_width = 1.0;
        (gl.depth_id, gl.depth_gen, gl.stencil_id, gl.stencil_gen) = (d, 1, s, u32::from(s != 0));
        (gl.do_clear_depth, gl.clear_quad, gl.clear_depth) = (true, true, 0.5);
        let before: Vec<u64> = with_platform(|p| p.allocs.keys().copied().collect());
        // SAFETY: a complete draw on this stack frame.
        assert_eq!(unsafe { gc_draw(&gl) }, 0, "the draw is recorded");
        if let std::collections::btree_map::Entry::Vacant(v) = self.surfaces.entry((d, s)) {
            /* the pair's surface is the allocation of its size this draw made */
            let made: Vec<(u64, usize)> = with_platform(|p| {
                p.allocs
                    .iter()
                    .filter(|(a, m)| !before.contains(a) && m.len() * 8 >= SURFACE_BYTES)
                    .map(|(&a, m)| (a, m.len() * 8))
                    .collect()
            });
            assert_eq!(made.len(), 1, "one surface allocated for the pair: {made:x?}");
            let (base, bytes) = made[0];
            v.insert((base, (bytes - SURFACE_BYTES) / 2));
        }
        let (surface, guard) = self.surfaces[&(d, s)];
        let depth = self.next_depth;
        self.next_depth += 0x1_0101;
        /* the kick the draw is in: the one being recorded now */
        let fence = i64::from(gc_kick_id());
        with_platform(|p| p.writes.push(Write { fence, surface, guard, depth }));
        depth
    }

    /// The resolve of depth object `id`, through the back end's entry point.
    fn resolve(&self, id: u32) -> Vec<f32> {
        let mut t = vec![-1.0f32; (SIZE * SIZE) as usize];
        // SAFETY: SIZE rows of SIZE floats.
        assert_eq!(unsafe { gc_depth_resolve(id, t.as_mut_ptr(), SIZE, SIZE, SIZE) }, 0, "resolved");
        t
    }

    /// The waits since the last call.
    fn waits(&self) -> Vec<i64> {
        with_platform(|p| core::mem::take(&mut p.waits))
    }

    fn submitted(&self) -> i64 {
        with_platform(|p| p.fence)
    }

    /// The resolve after the old full wait answers `got` too, and both are
    /// `depth` everywhere.
    fn matches_full_wait(&self, id: u32, got: &[f32], depth: u32) {
        let want = unorm24_depth(depth);
        assert!(got.iter().all(|v| v.to_bits() == want.to_bits()), "the resolve reads depth 0x{depth:06x}");
        assert_eq!(gc_finish(), 0);
        assert_eq!(self.resolve(id), got, "the same pixels as after a full wait");
    }
}

impl Drop for Rig {
    fn drop(&mut self) {
        let faults = with_platform(|p| core::mem::take(&mut p.faults));
        gc_shutdown();
        if !std::thread::panicking() {
            assert!(faults.is_empty(), "{faults:?}");
        }
    }
}

/// Two kicks in flight; only the first wrote depth object 5. The resolve
/// waits for the first kick's fence alone: the second is still pending
/// after it, and its depth (object 6, another surface) is not touched.
///
/// Revert check: waiting for every slot (`finish`) waits for fence 2 too.
#[test]
fn only_the_kick_that_wrote_the_depth_is_waited_for() {
    let mut rig = Rig::new();
    let depth = rig.draw(TARGET_A, 5, 0);
    let first = gc_kick_id();
    rig.draw(TARGET_B, 6, 0); /* another target: the first kick is submitted */
    let second = gc_kick_id();
    assert_eq!(gc_flush(), 0);
    assert_eq!((rig.submitted(), first + 1), (2, second), "two kicks in flight");
    assert!(gc_kick_pending(first) && gc_kick_pending(second));

    let got = rig.resolve(5);
    assert_eq!(rig.waits(), [i64::from(first)], "the kick that wrote the depth, alone");
    assert!(gc_kick_pending(second), "the later kick still runs");
    rig.matches_full_wait(5, &got, depth);
}

/// The newest kick wrote the depth: the frame still being recorded is
/// submitted and waited for; the kick before it, which wrote the same
/// object's depth earlier, is complete with it (kicks run in order).
///
/// Revert check: stamped with the kick open at `select`, the draw that
/// changed targets (and so submitted that kick) waits for kick 2.
#[test]
fn a_depth_written_by_the_newest_kick_waits_for_that_kick() {
    let mut rig = Rig::new();
    rig.draw(TARGET_A, 5, 0);
    rig.draw(TARGET_B, 6, 0);
    let depth = rig.draw(TARGET_A, 5, 0); /* kick 3, open */
    let newest = gc_kick_id();
    assert_eq!(rig.submitted(), 2, "the newest kick is not submitted yet");

    let got = rig.resolve(5);
    assert_eq!(rig.submitted(), 3, "the open frame was submitted for the wait");
    assert_eq!(rig.waits(), [i64::from(newest)], "that kick's own fence, once");
    rig.matches_full_wait(5, &got, depth);

    /* the sync waits' entry point is the same wait */
    let depth = rig.draw(TARGET_B, 6, 0);
    let kick = gc_kick_id();
    rig.waits();
    assert_eq!(gc_wait_kick(kick), 0);
    assert_eq!(rig.waits(), [i64::from(kick)]);
    rig.matches_full_wait(6, &rig.resolve(6), depth);
}

/// The kick that wrote the depth has completed: the resolve waits for
/// nothing and submits nothing, not even the frame being recorded.
#[test]
fn with_no_pending_kick_the_resolve_does_not_wait() {
    let mut rig = Rig::new();
    let depth = rig.draw(TARGET_A, 5, 0);
    assert_eq!(gc_finish(), 0);
    rig.draw(TARGET_B, 6, 0); /* an open frame that does not write object 5 */
    rig.waits();
    let submitted = rig.submitted();

    let got = rig.resolve(5);
    assert_eq!(rig.waits(), [] as [i64; 0], "no wait");
    assert_eq!(rig.submitted(), submitted, "nothing submitted");
    assert!(gc_kick_pending(gc_kick_id()), "the open frame stays open");
    rig.matches_full_wait(5, &got, depth);
}

/// The draw that wrote the depth did not fit the frame open when its surface
/// was selected: the back end submitted that frame and recorded the draw in
/// the next kick. The resolve waits for that next kick, where the draw is.
///
/// Revert check: stamped with the kick open at `select`, the resolve waits
/// for the full kick only and reads the depth of the draw before the
/// spilled one.
#[test]
fn a_draw_spilled_into_the_next_kick_is_waited_for_there() {
    let mut rig = Rig::new();
    let mut depth;
    let mut draws = 0;
    loop {
        let before = rig.submitted();
        depth = rig.draw(TARGET_A, 5, 0);
        draws += 1;
        if rig.submitted() != before {
            break;
        }
        assert!(draws < 200_000, "the stream never filled");
    }
    let spilled = gc_kick_id();
    assert_eq!(rig.submitted(), 1, "the full frame went alone; the draw is in kick 2, open");

    let got = rig.resolve(5);
    assert_eq!(rig.waits(), [i64::from(spilled)], "the kick the draw is in");
    rig.matches_full_wait(5, &got, depth);
}

/// The depth object is drawn with another stencil object: `select` copies
/// its plane into the new pair's surface after finishing the open frame,
/// and the draw goes into the next kick. The resolve waits for that kick.
///
/// Revert check: stamped with the kick open at `select`, which the copy
/// finished, the resolve waits for nothing and reads the copied depth.
#[test]
fn a_draw_behind_a_plane_copy_is_waited_for_in_its_kick() {
    let mut rig = Rig::new();
    let copied = rig.draw(TARGET_A, 5, 6);
    let depth = rig.draw(TARGET_A, 5, 7); /* depth 5 moves to the (5, 7) surface */
    let kick = gc_kick_id();
    assert_eq!(rig.waits(), [1], "the copy finished the first kick");
    assert_eq!(rig.submitted(), 1, "the draw after the copy is in the open frame");
    assert_ne!(copied, depth);

    let got = rig.resolve(5);
    assert_eq!(rig.waits(), [i64::from(kick)], "the kick the draw is in");
    rig.matches_full_wait(5, &got, depth);
}
