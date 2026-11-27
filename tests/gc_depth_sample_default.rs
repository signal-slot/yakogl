// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GC7000 (i.MX8MP): the depth-sample path of glmark2's shadow pass, pinned
//! in every dtpu mode (docs/notes/gc-depth-sample.md section "(d) Draft host
//! test", docs/notes/gc-depth-sample-wire.md "(a) Host test").
//!
//! The frame is shadow-shaped, on the harness of `gc_scene_kicks.rs`
//! (`tests/common/gc_scenes.rs`): a depth pass into an FBO whose depth
//! attachment is a depth texture (96 x 80, so the 64-aligned surface is wider
//! and taller than the texture), then a pass that samples that texture. No
//! GPU runs here, so the test writes the surface's D24S8 words itself (a hash
//! of the pixel for depth, another for stencil, depth 0 and 2^24 - 1 at the
//! corners) between the two passes, and reads back what a sampler would
//! see, mode by mode:
//!
//! * mode 0 (the CPU resolve; the default today): the floats the back end's
//!   `depth_resolve` wrote for the texture, captured at the table. Each must
//!   be the reference `depth / (2^24 - 1)` within 2^-23 (the 24-bit format's
//!   step is 2^-24).
//! * mode 1 (D24X8 view): the surface at the view `ds_cache::depth_view`
//!   gives the draw (address, stride), decoded as the texture unit decodes
//!   D24X8 (bits 31..8 over 2^24 - 1), against the same reference.
//! * mode 2 (raw A8R8G8B8): the same surface decoded as (D1, D0, S, D2),
//!   which must give back depth and stencil exactly.
//!
//! The CPU depth resolves each mode ran in a warm frame are counted through
//! the harness's counted table and printed (`--nocapture`); the test pins no
//! count and says nothing about which mode is the default.
//! Modes 1 and 2 need `gc_debug_set_depth_tpu`, so only a `test-knobs` build
//! runs them; the knob is put back to its value before, even when a mode's
//! run panics. The surface is addressed without a guard (a build without
//! `vivante-depth-guard`).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online --test gc_depth_sample_default -- --nocapture
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online,test-knobs --test gc_depth_sample_default -- --nocapture
//! ```

#![cfg(all(feature = "host-test", feature = "gpu-vivante", feature = "shaderc-online"))]

#[path = "common/gc_scenes.rs"]
mod gc_scenes;

use gc_scenes::*;
use std::collections::BTreeMap;
use std::sync::Mutex;
use yakogl::backend::YakoglBackend;
use yakogl::gl::consts::*;
use yakogl::gl::framebuffer::{glBindFramebuffer, glFramebufferTexture2D};
use yakogl::gl::state::{glClear, glColorMask, glViewport};
use yakogl::gl::types::*;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};
use yakogl::vivante::depth_sample::{mode, DepthView, DEPTH_TPU_D24X8, DEPTH_TPU_OFF, DEPTH_TPU_RAW8888};
use yakogl::vivante::ds_cache;

const W: u32 = 96;
const H: u32 = 80;
const MAX_D: f64 = 16_777_215.0;

/// gpu address -> (cpu address, size) of every allocation of the run.
static ALLOCS: Mutex<BTreeMap<u64, (usize, usize)>> = Mutex::new(BTreeMap::new());
/// One recorded resolve: (w, h, pitch, data).
type Resolved = (u32, u32, u32, Vec<f32>);
/// The floats of the last `depth_resolve` of the texture.
static RESOLVED: Mutex<Option<Resolved>> = Mutex::new(None);
/// What the frames found wrong, and the texels they compared.
static PROBLEMS: Mutex<Vec<String>> = Mutex::new(Vec::new());
static COMPARED: Mutex<usize> = Mutex::new(0);

unsafe extern "C" fn rec_gpu_alloc(heap_base: u64, size: usize, flags: u32, out: *mut OsalGpuMem) -> i32 {
    let rc = unsafe { (OSAL.gpu_alloc.expect("the harness allocates"))(heap_base, size, flags, out) };
    if rc == 0 {
        let m = unsafe { *out };
        ALLOCS.lock().unwrap().insert(m.gpu, (m.cpu as usize, m.size));
    }
    rc
}

unsafe extern "C" fn rec_depth_resolve(id: u32, dst: *mut f32, w: u32, h: u32, pitch: u32) -> i32 {
    let counted = unsafe { (*counted_table()).depth_resolve.expect("counted") };
    let rc = unsafe { counted(id, dst, w, h, pitch) };
    if rc == 0 {
        let data = unsafe { core::slice::from_raw_parts(dst, h as usize * pitch as usize) }.to_vec();
        *RESOLVED.lock().unwrap() = Some((w, h, pitch, data));
    }
    rc
}

/// The reference word of pixel (x, y): depth in bits 31..8, stencil in 7..0.
fn word(x: u32, y: u32) -> u32 {
    let d = match (x, y) {
        (0, 0) => 0,
        (x, y) if (x, y) == (W - 1, H - 1) => 0xff_ffff,
        _ => (y * W + x).wrapping_mul(2_654_435_761).rotate_left(11) & 0xff_ffff,
    };
    d << 8 | ((x ^ y) & 0xff)
}

fn problem(s: String) {
    PROBLEMS.lock().unwrap().push(s);
}

/// The id the depth texture's surface is kept under (bit 30: a texture).
fn find_view() -> Option<(u32, DepthView)> {
    (0x4000_0001u32..0x4000_0400)
        .find_map(|id| ds_cache::depth_view(id, 0, 0).map(|(gpu, stride, rows)| (id, DepthView { gpu, stride, rows })))
}

fn surface_words(v: &DepthView) -> &'static mut [u32] {
    let allocs = ALLOCS.lock().unwrap();
    let &(cpu, size) = allocs.get(&v.gpu).expect("the surface is an allocation of the run");
    assert!(size >= v.stride as usize * v.rows as usize);
    // SAFETY: host memory the stand-in OSAL keeps (leaked at the run's end), whole words.
    unsafe { core::slice::from_raw_parts_mut(cpu as *mut u32, v.stride as usize * v.rows as usize / 4) }
}

/// Mode 0 .. 2 checks of one frame's sampled values against the reference.
fn check_frame(m: i32, view: &DepthView, words: &[u32]) {
    let mut n = 0;
    let stride_w = (view.stride / 4) as usize;
    match m {
        DEPTH_TPU_OFF => {
            let r = RESOLVED.lock().unwrap().take();
            let Some((w, h, pitch, data)) = r else {
                return problem("mode 0: the sampling draw ran no CPU depth resolve".into());
            };
            if (w, h) != (W, H) {
                return problem(format!("mode 0: resolved {w}x{h}, not {W}x{H}"));
            }
            for y in 0..H {
                for x in 0..W {
                    let want = (word(x, y) >> 8) as f64 / MAX_D;
                    let got = data[(y * pitch + x) as usize] as f64;
                    n += 1;
                    if (got - want).abs() > 2f64.powi(-23) {
                        return problem(format!("mode 0: ({x},{y}) sampled {got}, reference {want}"));
                    }
                }
            }
        }
        DEPTH_TPU_D24X8 | DEPTH_TPU_RAW8888 => {
            for y in 0..H {
                for x in 0..W {
                    let t = words[y as usize * stride_w + x as usize];
                    let want = word(x, y);
                    n += 1;
                    if m == DEPTH_TPU_D24X8 {
                        let (got, ref_d) = ((t >> 8) as f64 / MAX_D, (want >> 8) as f64 / MAX_D);
                        if (got - ref_d).abs() > 2f64.powi(-23) {
                            return problem(format!("mode 1: ({x},{y}) sampled {got}, reference {ref_d}"));
                        }
                    } else {
                        /* A8R8G8B8, identity swizzle: the texel is (R, G, B, A) = (D1, D0, S, D2) */
                        let (r, g, b, a) = ((t >> 16) & 0xff, (t >> 8) & 0xff, t & 0xff, t >> 24);
                        let d = a << 16 | r << 8 | g;
                        if (d << 8 | b) != want {
                            return problem(format!("mode 2: ({x},{y}) decoded {:#x}, word {want:#x}", d << 8 | b));
                        }
                    }
                }
            }
        }
        _ => unreachable!(),
    }
    *COMPARED.lock().unwrap() += n;
}

/// The shadow-shaped frame: depth pass, the test's words in the surface,
/// the pass that samples the texture, then the check of what it sampled.
fn shadow_frame(m: i32) -> Box<dyn FnMut()> {
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
        let Some((_, view)) = find_view() else {
            return problem("no live depth surface after the depth pass".into());
        };
        assert!(view.covers(W, H), "the surface covers the texture");
        let words = surface_words(&view);
        let stride_w = (view.stride / 4) as usize;
        for y in 0..H {
            for x in 0..W {
                words[y as usize * stride_w + x as usize] = word(x, y);
            }
        }
        *RESOLVED.lock().unwrap() = None;
        step("ground");
        bind(GL_TEXTURE0, depth_tex);
        draw(ground_prog, ground, GL_TRIANGLE_STRIP, 4);
        bind(GL_TEXTURE0, 0);
        if m != DEPTH_TPU_OFF && find_view().is_none() {
            return problem(format!("mode {m}: the surface is gone after the sampling draw"));
        }
        check_frame(m, &view, words);
    })
}

/// The knob's value before the run set it (`None`: not set).
#[cfg(feature = "test-knobs")]
static PREV_KNOB: Mutex<Option<i32>> = Mutex::new(None);

/// Put the knob back, whether the run finished or panicked.
fn restore_knob() {
    #[cfg(feature = "test-knobs")]
    if let Some(prev) = PREV_KNOB.lock().unwrap().take() {
        yakogl::vivante::depth_sample::gc_debug_set_depth_tpu(prev);
    }
}

/// The knob and the statics above are process-wide: one test at a time.
static LOCK: Mutex<()> = Mutex::new(());

/// One run of the frame in `m`; returns the warm frames' counts.
fn run_mode(m: i32, gpu: Gpu) -> Vec<Counts> {
    ALLOCS.lock().unwrap().clear();
    PROBLEMS.lock().unwrap().clear();
    *COMPARED.lock().unwrap() = 0;
    #[cfg(feature = "test-knobs")]
    let before = mode();
    let hooks = Hooks {
        after_init: Some(Box::new(move || {
            #[cfg(feature = "test-knobs")]
            {
                *PREV_KNOB.lock().unwrap() = Some(yakogl::vivante::depth_sample::gc_debug_set_depth_tpu(m));
            }
            // SAFETY: SERIAL is held by the harness; the table is leaked, the OSAL copy too.
            unsafe {
                let mut t: YakoglBackend = core::ptr::read(counted_table());
                t.depth_resolve = Some(rec_depth_resolve);
                yakogl::backend::g_backend = Box::leak(Box::new(t));
                let mut o: OsalOps = core::ptr::read(&OSAL);
                o.gpu_alloc = Some(rec_gpu_alloc);
                osal_set_ops(Box::leak(Box::new(o)));
            }
            #[cfg(feature = "test-knobs")]
            assert_eq!(mode(), m, "the knob took");
        })),
        ..Hooks::default()
    };
    let run = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run_with(gpu, move || shadow_frame(m), hooks)));
    restore_knob();
    #[cfg(feature = "test-knobs")]
    assert_eq!(mode(), before, "the knob is put back");
    let (counts, _) = run.unwrap_or_else(|e| std::panic::resume_unwind(e));
    let problems = PROBLEMS.lock().unwrap().clone();
    assert!(problems.is_empty(), "mode {m}, {gpu:?}: {problems:?}");
    let compared = *COMPARED.lock().unwrap();
    assert_eq!(compared, (W * H) as usize * FRAMES, "mode {m}: every texel of every frame compared");
    counts
}

fn run_modes(modes: &[i32]) {
    let _one = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    for gpu in [Gpu::Lag, Gpu::Done] {
        for &m in modes {
            let counts = run_mode(m, gpu);
            let resolves: Vec<u32> = counts.iter().map(|c| c.resolves).collect();
            eprintln!("dtpu mode {m} ({gpu:?}): CPU depth resolves per warm frame {resolves:?}");
            // wire-verdict adds here: the default mode takes the non-CPU path,
            // 0 resolves a warm frame (docs/notes/gc-depth-sample-wire.md).
        }
    }
}

/// The mode the build runs when nothing sets the knob (`mode()`; today the
/// CPU resolve, mode 0): the sampled values are the reference. This test does
/// not say which mode that is.
#[test]
fn the_default_mode_samples_the_reference_depth() {
    run_modes(&[mode()]);
}

/// Modes 0, 1 and 2 through the knob, each against the same reference.
#[cfg(feature = "test-knobs")]
#[test]
fn every_dtpu_mode_samples_the_reference_depth() {
    run_modes(&[DEPTH_TPU_OFF, DEPTH_TPU_D24X8, DEPTH_TPU_RAW8888]);
}
