// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GC7000 (i.MX8MP): the CPU depth resolve adds the bytes it reads to the
//! CPU-pass counter of glmark2's PERF page 4 v3
//! (`yakogl_gc_fallback_stats().carveout_cpu_bytes`, mipmap.rs), the one the
//! CPU mipmap filter and the CPU tiled copy add to.
//!
//! The unit is the bytes READ from the carve-out surface: one D24S8 word, 4
//! bytes, a resolved texel (`w * h * 4` a plane), added once a resolve. The
//! floats written to the texture are not counted. Pinned here:
//!
//! 1. one `ds_cache::resolve_depth` of a W x H plane adds exactly
//!    `W * H * 4` (linear and super-tiled surface; a resolve that finds no
//!    surface, or a clear value, adds nothing);
//! 2. a shadow-shaped frame with no sampling draw (no resolve) adds nothing
//!    from the resolve;
//! 3. with dtpu mode 1 (`gc_debug_set_depth_tpu`), where the depth view
//!    replaces the resolve, the sampling frame does not grow the counter,
//!    while mode 0's frame does by `FRAMES * W * H * 4`.
//!
//! Test 3's mode 1 needs `test-knobs`:
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online --test gc_resolve_bytes_counter
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online,test-knobs --test gc_resolve_bytes_counter
//! ```

#![cfg(all(feature = "host-test", feature = "gpu-vivante", feature = "shaderc-online"))]

#[path = "common/gc_scenes.rs"]
mod gc_scenes;

use core::ffi::c_void;
use gc_scenes::*;
use std::collections::BTreeMap;
use std::sync::Mutex;
use yakogl::backend::YakoglBackend;
use yakogl::gl::consts::*;
use yakogl::gl::framebuffer::{glBindFramebuffer, glFramebufferTexture2D};
use yakogl::gl::state::{glClear, glColorMask, glViewport};
use yakogl::gl::types::*;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};
use yakogl::vivante::ds_cache::{self, DsKey};
use yakogl::vivante::layout::Layout;
use yakogl::vivante::mipmap::{yakogl_gc_fallback_stats, YakoglGcFallbackStats};

/// The counter's value now.
fn bytes() -> u64 {
    let mut s = YakoglGcFallbackStats::default();
    // SAFETY: a valid out.
    assert_eq!(unsafe { yakogl_gc_fallback_stats(&mut s) }, 0);
    s.carveout_cpu_bytes
}

/// The counter and the OSAL are process-wide: one test at a time.
static LOCK: Mutex<()> = Mutex::new(());

/* ---- 1: one resolve ------------------------------------------------------------------ */

static ALLOCS: Mutex<BTreeMap<u64, Box<[u64]>>> = Mutex::new(BTreeMap::new());

unsafe extern "C" fn p_gpu_alloc(heap_base: u64, size: usize, _flags: u32, out: *mut OsalGpuMem) -> i32 {
    let mut a = ALLOCS.lock().unwrap_or_else(|e| e.into_inner());
    let mut mem = vec![0u64; size.div_ceil(8)].into_boxed_slice();
    let gpu = heap_base + 0x1000 + a.len() as u64 * (1 << 24);
    let cpu = mem.as_mut_ptr() as *mut c_void;
    // SAFETY: the caller passes a writable descriptor.
    unsafe { *out = OsalGpuMem { cpu, gpu, phys: gpu, size, impl_: cpu } };
    a.insert(gpu, mem);
    0
}

unsafe extern "C" fn p_gpu_free(mem: *mut OsalGpuMem) -> i32 {
    // SAFETY: the caller passes a live descriptor.
    let m = unsafe { *mem };
    if let Some(b) = ALLOCS.lock().unwrap_or_else(|e| e.into_inner()).remove(&m.gpu) {
        core::mem::forget(b);
    }
    // SAFETY: as above.
    unsafe { *mem = OsalGpuMem::ZERO };
    0
}

unsafe extern "C" fn p_cache(_cpu: *mut c_void, _n: usize) {}

static UNIT_OSAL: OsalOps = OsalOps {
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
    submit: None,
    wait: None,
    completed: None,
    reg_read: None,
    reg_write: None,
    query: None,
    log: None,
    wait_vblank: None,
    present: None,
    lock: None,
    unlock: None,
    thread_id: None,
    display_busy: None,
};

const GUARD: usize = 4096;
const ID: u32 = 0x4000_0064;
const KEY: DsKey = DsKey { depth_id: ID, depth_gen: 1, stencil_id: ID, stencil_gen: 1, scale: 1 };

fn resolve(w: u32, h: u32, pitch: u32) -> i32 {
    let mut dst = vec![0f32; (h * pitch) as usize];
    // SAFETY: `h` rows of `pitch` floats.
    unsafe { ds_cache::resolve_depth(ID, dst.as_mut_ptr(), w, h, pitch, GUARD, |_| 0) }
}

/// Run `case` with a drawn `sw` x `sh` surface of `layout` in the cache.
fn with_surface(layout: Layout, sw: u32, sh: u32, case: impl FnOnce()) {
    let _one = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    // SAFETY: LOCK serialises the tests; the table is a static.
    unsafe {
        osal_set_ops(&UNIT_OSAL);
        yakogl::backend::g_backend = &yakogl::vivante::backend::yakogl_backend_vivante;
        assert_eq!(yakogl::vivante::backend::gc_init(), 0, "gc_init");
    }
    ds_cache::select_in(layout, KEY, sw, sh, GUARD, 1, None, |_| false, || 0).expect("a surface");
    ds_cache::drawn(KEY, 1, false, [true, false], [None, None]);
    case();
    yakogl::vivante::backend::gc_shutdown();
}

#[test]
fn one_linear_resolve_adds_w_h_4() {
    with_surface(Layout::Linear, 90, 70, || {
        for (w, h, pitch) in [(90, 70, 90), (90, 70, 97), (1, 1, 1), (33, 5, 64)] {
            let before = bytes();
            assert_eq!(resolve(w, h, pitch), 0);
            assert_eq!(bytes() - before, u64::from(w) * u64::from(h) * 4, "{w}x{h} pitch {pitch}");
        }
        /* two resolves add twice */
        let before = bytes();
        assert_eq!(resolve(64, 64, 64) + resolve(64, 64, 64), 0);
        assert_eq!(bytes() - before, 2 * 64 * 64 * 4);
    });
}

#[test]
fn one_super_tiled_resolve_adds_w_h_4() {
    with_surface(Layout::SuperTiled, 90, 70, || {
        let before = bytes();
        assert_eq!(resolve(90, 70, 90), 0);
        assert_eq!(bytes() - before, 90 * 70 * 4);
    });
}

#[test]
fn a_resolve_that_reads_no_surface_adds_nothing() {
    with_surface(Layout::Linear, 90, 70, || {
        let before = bytes();
        // SAFETY: one row of one float.
        let mut one = [0f32];
        let rc = unsafe { ds_cache::resolve_depth(ID + 1, one.as_mut_ptr(), 1, 1, 1, GUARD, |_| 0) };
        assert_ne!(rc, 0, "an id the cache does not hold");
        assert_eq!(resolve(91, 1, 90), 1, "w past the pitch is refused");
        assert_eq!(bytes(), before);
    });
}

/* ---- 2, 3: the shadow-shaped frame --------------------------------------------------- */

const W: u32 = 96;
const H: u32 = 80;

/// A depth pass into a depth texture; the sampling draw only when `sample`.
fn shadow_frame(sample: bool) -> Box<dyn FnMut()> {
    let depth_tex = depth_texture(W, H);
    let fbo = framebuffer(None, Depth::Texture(depth_tex), W, H);
    let depth_prog = program(MODEL_VS, FLAT_FS, &[]);
    let ground_prog = program(QUAD_VS, GROUND_FS, &[("ShadowMap", 0)]);
    let horse = model_vbo();
    let ground = quad_vbo();
    Box::new(move || {
        step("depth pass");
        // SAFETY: plain GL calls.
        unsafe {
            glBindFramebuffer(GL_FRAMEBUFFER, fbo);
            glFramebufferTexture2D(GL_FRAMEBUFFER, GL_DEPTH_ATTACHMENT, GL_TEXTURE_2D, depth_tex, 0);
            glViewport(0, 0, W as GLsizei, H as GLsizei);
            glColorMask(0, 0, 0, 0);
            glClear(GL_DEPTH_BUFFER_BIT);
        }
        draw(depth_prog, horse, GL_TRIANGLES, 96);
        // SAFETY: plain GL calls.
        unsafe {
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
            glViewport(0, 0, CANVAS_W as GLsizei, CANVAS_H as GLsizei);
            glColorMask(1, 1, 1, 1);
        }
        if sample {
            step("ground");
            bind(GL_TEXTURE0, depth_tex);
            draw(ground_prog, ground, GL_TRIANGLE_STRIP, 4);
            bind(GL_TEXTURE0, 0);
        }
    })
}

/// One run of the frame with dtpu mode `m` (None: the build's default);
/// returns the counter's growth and the CPU resolves the warm frames ran.
fn run_frames(m: Option<i32>, sample: bool) -> (u64, u32) {
    let _one = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    #[cfg(feature = "test-knobs")]
    let prev = std::sync::Arc::new(Mutex::new(None));
    let hooks = Hooks {
        after_init: Some(Box::new({
            #[cfg(feature = "test-knobs")]
            let prev = prev.clone();
            move || {
                #[cfg(feature = "test-knobs")]
                if let Some(m) = m {
                    *prev.lock().unwrap() = Some(yakogl::vivante::depth_sample::gc_debug_set_depth_tpu(m));
                }
                // SAFETY: SERIAL is held by the harness; the tables are leaked.
                unsafe {
                    let t: YakoglBackend = core::ptr::read(counted_table());
                    yakogl::backend::g_backend = Box::leak(Box::new(t));
                    let o: OsalOps = core::ptr::read(&OSAL);
                    osal_set_ops(Box::leak(Box::new(o)));
                }
                let _ = m;
            }
        })),
        ..Hooks::default()
    };
    let before = bytes();
    let run = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        run_with(Gpu::Done, move || shadow_frame(sample), hooks)
    }));
    #[cfg(feature = "test-knobs")]
    if let Some(p) = prev.lock().unwrap().take() {
        yakogl::vivante::depth_sample::gc_debug_set_depth_tpu(p);
    }
    let (counts, _) = run.unwrap_or_else(|e| std::panic::resume_unwind(e));
    (bytes() - before, counts.iter().map(|c| c.resolves).sum())
}

#[test]
fn a_frame_with_no_resolve_adds_nothing_from_it() {
    let (grown, resolves) = run_frames(None, false);
    assert_eq!(resolves, 0, "no sampling draw, no CPU resolve");
    assert_eq!(grown, 0, "nothing else of this frame adds either");
}

#[cfg(feature = "test-knobs")]
#[test]
fn dtpu_mode_1_does_not_grow_the_counter_and_mode_0_does() {
    let (g0, r0) = run_frames(Some(0), true);
    assert!(r0 > 0, "mode 0 resolves on the CPU");
    /* the harness counts the warm frames' resolves only; every frame resolves once */
    assert_eq!(g0, FRAMES as u64 * u64::from(W * H) * 4, "mode 0: every frame's resolve adds W * H * 4");
    let (g1, r1) = run_frames(Some(1), true);
    assert_eq!(r1, 0, "mode 1: the depth view replaces the resolve");
    assert_eq!(g1, 0, "mode 1: the counter does not grow");
}
