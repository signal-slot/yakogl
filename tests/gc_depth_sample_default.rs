// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GC7000 (i.MX8MP): the depth-sample path of glmark2's shadow pass, pinned
//! in every dtpu mode (docs/notes/gc-depth-sample.md section "(d) Draft host
//! test", docs/notes/gc-depth-sample-wire.md "(a) Host test"), and the
//! product default, mode 4 (`DEPTH_TPU_D24X8_FLUSH`: the D24X8 view with a
//! texture-cache flush in front of the sampling draw). gl_test case 222
//! `dtpu` gave verdict code 3 with check 7 failing (stale texels at index 32
//! with no wait or flush), measured on the Armadillo-X2 in place of the
//! Verdin iMX8MP; that selects outcome "(a) plus a texture-cache flush".
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
//! * mode 0 (the CPU resolve): the floats the back end's
//!   `depth_resolve` wrote for the texture, captured at the table. Each must
//!   be the reference `depth / (2^24 - 1)` within 2^-23 (the 24-bit format's
//!   step is 2^-24).
//! * mode 1 (D24X8 view): the surface at the view `ds_cache::depth_view`
//!   gives the draw (address, stride), decoded as the texture unit decodes
//!   D24X8 (bits 31..8 over 2^24 - 1), against the same reference.
//! * mode 2 (raw A8R8G8B8): the same surface decoded as (D1, D0, S, D2),
//!   which must give back depth and stencil exactly.
//! * mode 4 (the default): mode 1's check, and the same depth with its low
//!   byte dropped or rounded to 16 bits (what verdict code 3 allows the
//!   texture unit to return) within the code-3 bound 255 / (2^24 - 1) of
//!   tests/gc_depth_view_precision.rs.
//!
//! The CPU depth resolves each mode ran in a warm frame are counted through
//! the harness's counted table and printed (`--nocapture`). Under the default
//! (with or without `test-knobs`) the test pins: `mode()` is 4, no warm frame
//! runs a CPU depth resolve and the sampling draw records none, and every
//! frame's stream holds `emit_view_flush`'s seven events once, in the window
//! of the draw that writes texture slot 0 (the sampling draw), in front of
//! that draw (`the_default_flushes_in_front_of_the_sampling_draw`, on its own
//! runner: it records every allocation from before `gc_init`, the stream
//! buffers included). This file is plan #417's acceptance c2.
//! Modes 0, 1 and 2 need `gc_debug_set_depth_tpu`, so only a `test-knobs`
//! build runs them; the knob is put back to its value before, even when a mode's
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
use yakogl::vivante::depth_sample::{
    mode, DepthView, DEPTH_TPU_D24X8, DEPTH_TPU_D24X8_FLUSH, DEPTH_TPU_DEFAULT, DEPTH_TPU_OFF, DEPTH_TPU_RAW8888,
};
use yakogl::vivante::ds_cache;
use yakogl::vivante::kick::{GcKick, GC_KICK_MAGIC};
use yakogl::vivante::regs::*;

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
/// The CPU depth resolves the sampling draw (step "ground") ran, every frame.
static GROUND_RESOLVES: Mutex<Vec<bool>> = Mutex::new(Vec::new());
/// The decoded kicks of the frame being recorded.
static CAPTURE: Mutex<Vec<Vec<Ev>>> = Mutex::new(Vec::new());

/// The code-3 bound: the depth with its low byte dropped
/// (tests/gc_depth_view_precision.rs `BOUND`).
const CODE3_BOUND: f64 = 255.0 / MAX_D;

/// One decoded stream item.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Ev {
    /// A register write.
    State(u32, u32),
    /// The front end's own STALL word (token).
    FeStall(u32),
    /// A draw.
    Draw,
}

/// Decode one stream into register writes, front-end stalls and draws (the
/// walk of tests/gc_depth_view_flush_mode.rs).
fn decode(words: &[u32]) -> Vec<Ev> {
    let mut out = Vec::new();
    let mut i = 0usize;
    while i < words.len() {
        let w = words[i];
        match w & 0xf800_0000 {
            GC_OP_LOAD_STATE => {
                let count = ((w >> 16) & 0x3ff) as usize;
                let base = (w & 0xffff) * 4;
                for (k, &v) in words.iter().skip(i + 1).take(count).enumerate() {
                    out.push(Ev::State(base + 4 * k as u32, v));
                }
                i += 1 + count;
            }
            GC_OP_DRAW_INSTANCED => {
                out.push(Ev::Draw);
                i += 4;
            }
            GC_OP_STALL => {
                out.push(Ev::FeStall(words.get(i + 1).copied().unwrap_or(0)));
                i += 2;
            }
            _ => i += 1,
        }
    }
    out
}

/// The seven events of `emit_view_flush` (src/vivante/depth_sample.rs).
fn flush_block() -> [Ev; 7] {
    let fe_pe = stall_token(GC_SYNC_FE, GC_SYNC_PE);
    let ra_pe = stall_token(GC_SYNC_RA, GC_SYNC_PE);
    [
        Ev::State(GC_GL_FLUSH_CACHE, GC_FLUSH_DEPTH),
        Ev::State(GC_GL_SEMAPHORE_TOKEN, fe_pe),
        Ev::FeStall(fe_pe),
        Ev::State(GC_GL_FLUSH_CACHE, GC_FLUSH_TEXTURE),
        Ev::State(GC_GL_FLUSH_CACHE, GC_FLUSH_TEXTUREVS),
        Ev::State(GC_GL_SEMAPHORE_TOKEN, ra_pe),
        Ev::State(GC_GL_STALL_TOKEN, ra_pe),
    ]
}

unsafe extern "C" fn rec_submit(kick: *const c_void, len: usize) -> i64 {
    if len == core::mem::size_of::<GcKick>() {
        // SAFETY: the back end passes a live GcKick of that size.
        let k = unsafe { *(kick as *const GcKick) };
        let allocs = ALLOCS.lock().unwrap().clone();
        let found =
            allocs.range(..=k.stream_gpu).next_back().filter(|(&gpu, &(_, size))| k.stream_gpu < gpu + size as u64);
        if let (true, Some((&gpu, &(cpu, size)))) = (k.magic == GC_KICK_MAGIC, found) {
            let at = (k.stream_gpu - gpu) as usize;
            let n = (k.stream_bytes as usize).min(size - at) / 4;
            // SAFETY: inside a live allocation of the stand-in, read as dwords.
            let words = unsafe { core::slice::from_raw_parts((cpu + at) as *const u32, n) };
            CAPTURE.lock().unwrap().push(decode(words));
        }
    }
    // SAFETY: handed on as it came.
    unsafe { (OSAL.submit.expect("the harness submits"))(kick, len) }
}

/// Where `emit_view_flush`'s block starts in each kick of one frame, as
/// (kick, event index).
fn blocks_of(kicks: &[Vec<Ev>]) -> Vec<(usize, usize)> {
    let b = flush_block();
    let mut out = Vec::new();
    for (k, evs) in kicks.iter().enumerate() {
        out.extend((0..evs.len()).filter(|&i| evs[i..].starts_with(&b)).map(|i| (k, i)));
    }
    out
}

/// Under the default: every frame records the flush block once, in the
/// window (since the kick's previous draw) of a draw that writes texture slot
/// 0's descriptor address, the frame's one sampling draw, in front of it.
fn check_flush_in_front_of_the_sampling_draw(frames: &[Vec<Vec<Ev>>]) {
    assert_eq!(frames.len(), FRAMES, "every frame's kicks captured");
    let n = flush_block().len();
    for (f, kicks) in frames.iter().enumerate() {
        let blocks = blocks_of(kicks);
        assert_eq!(blocks.len(), 1, "frame {}: one flush block, at {blocks:?}", f + 1);
        let (k, at) = blocks[0];
        let evs = &kicks[k];
        let start = evs[..at].iter().rposition(|e| *e == Ev::Draw).map_or(0, |i| i + 1);
        let draw = evs[at + n..].iter().position(|e| *e == Ev::Draw).map(|i| at + n + i);
        let Some(draw) = draw else { panic!("frame {}: no draw behind the flush block", f + 1) };
        let slot0 = nte_descriptor_addr(0);
        let samples = evs[start..draw].iter().any(|e| matches!(e, Ev::State(r, _) if *r == slot0));
        assert!(samples, "frame {}: the draw behind the flush block writes texture slot 0", f + 1);
        let earlier_draws: usize =
            kicks[..k].iter().map(|e| e.iter().filter(|e| **e == Ev::Draw).count()).sum::<usize>()
                + evs[..at].iter().filter(|e| **e == Ev::Draw).count();
        assert!(earlier_draws >= 1, "frame {}: the depth pass's draw comes before the flush", f + 1);
    }
}

unsafe extern "C" fn rec_gpu_alloc(heap_base: u64, size: usize, flags: u32, out: *mut OsalGpuMem) -> i32 {
    let rc = unsafe { (OSAL.gpu_alloc.expect("the harness allocates"))(heap_base, size, flags, out) };
    if rc == 0 {
        let m = unsafe { *out };
        ALLOCS.lock().unwrap().insert(m.gpu, (m.cpu as usize, m.size));
    }
    rc
}

unsafe extern "C" fn rec_gpu_free(mem: *mut OsalGpuMem) -> i32 {
    // SAFETY: the caller's live descriptor.
    let gpu = unsafe { (*mem).gpu };
    ALLOCS.lock().unwrap().remove(&gpu);
    // SAFETY: handed on as it came.
    unsafe { (OSAL.gpu_free.expect("the harness frees"))(mem) }
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
        DEPTH_TPU_D24X8 | DEPTH_TPU_RAW8888 | DEPTH_TPU_D24X8_FLUSH => {
            for y in 0..H {
                for x in 0..W {
                    let t = words[y as usize * stride_w + x as usize];
                    let want = word(x, y);
                    n += 1;
                    if m != DEPTH_TPU_RAW8888 {
                        let (got, ref_d) = ((t >> 8) as f64 / MAX_D, (want >> 8) as f64 / MAX_D);
                        if (got - ref_d).abs() > 2f64.powi(-23) {
                            return problem(format!("mode {m}: ({x},{y}) sampled {got}, reference {ref_d}"));
                        }
                        /* verdict code 3: fewer bits than 24, the low byte
                         * dropped or the depth rounded to 16 bits */
                        let d24 = t >> 8;
                        let cut = (d24 & !0xff) as f64 / MAX_D;
                        let r16 = (d24 >> 8) as f64 / 65_535.0;
                        for (what, got) in [("low byte dropped", cut), ("16-bit", r16)] {
                            if (got - ref_d).abs() > CODE3_BOUND + 1e-15 {
                                return problem(format!(
                                    "mode {m}: ({x},{y}) {what} {got} is outside the code-3 bound of {ref_d}"
                                ));
                            }
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
    *SCENE.lock().unwrap() = Some((fbo, depth_tex));
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
        GROUND_RESOLVES.lock().unwrap().push(RESOLVED.lock().unwrap().is_some());
        if m != DEPTH_TPU_OFF && find_view().is_none() {
            return problem(format!("mode {m}: the surface is gone after the sampling draw"));
        }
        check_frame(m, &view, words);
    })
}

/// The shadow frame's framebuffer and depth texture, deleted at the end of
/// each run ([`forget_scene`]).
static SCENE: Mutex<Option<(GLuint, GLuint)>> = Mutex::new(None);

/// Delete the run's framebuffer and depth texture, in the run's last frame.
/// The GL core keeps a depth texture's render kick and its resolved kick per
/// context slot and texture name (`gl::framebuffer` `TARGET_USE`,
/// `DEPTH_TARGET`) across back-end runs, while each `gc_init` starts the kick
/// ids again: left in place, a mode 0 run's resolved kick would match a later
/// run's render kick by chance and the texture would read as resolved there.
/// Deleting the texture clears the marks (`clear_target_mark`).
fn forget_scene() {
    if let Some((fbo, tex)) = SCENE.lock().unwrap().take() {
        // SAFETY: plain GL calls on the run's live context.
        unsafe {
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
            yakogl::gl::framebuffer::glDeleteFramebuffers(1, &fbo);
            yakogl::gl::texture::glDeleteTextures(1, &tex);
        }
    }
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
    GROUND_RESOLVES.lock().unwrap().clear();
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
        after_frame: Some(Box::new(|n| {
            if n == FRAMES {
                forget_scene();
            }
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
            let ground = GROUND_RESOLVES.lock().unwrap().clone();
            assert_eq!(ground.len(), FRAMES, "mode {m}: every frame's sampling draw seen");
            if m == DEPTH_TPU_DEFAULT {
                /* the default takes the non-CPU path: no resolve a warm
                 * frame and none on the sampling draw
                 * (docs/notes/gc-depth-sample-wire.md); its flush is
                 * `the_default_flushes_in_front_of_the_sampling_draw`'s */
                assert_eq!(resolves, vec![0; resolves.len()], "default, {gpu:?}: no CPU depth resolve a warm frame");
                assert_eq!(ground, vec![false; FRAMES], "default, {gpu:?}: the sampling draw resolves nothing");
            } else if m == DEPTH_TPU_OFF {
                assert_eq!(ground, vec![true; FRAMES], "mode 0, {gpu:?}: the sampling draw resolves on the CPU");
            }
        }
    }
}

/// The product default is mode 4, `DEPTH_TPU_D24X8_FLUSH`: the X2's verdict
/// code 3 with check 7 failing. A build that does not set the knob runs it.
#[test]
fn the_product_default_is_the_depth_view_with_its_flush() {
    let _one = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    assert_eq!(DEPTH_TPU_DEFAULT, DEPTH_TPU_D24X8_FLUSH);
    assert_eq!(DEPTH_TPU_DEFAULT, 4);
    assert_eq!(mode(), DEPTH_TPU_D24X8_FLUSH, "nothing set the knob: the default");
}

/// The mode the build runs when nothing sets the knob (`mode()`, mode 4):
/// the sampled values are the reference within the code-3 bound, no CPU
/// resolve runs, and the flush block is in front of the sampling draw.
#[test]
fn the_default_mode_samples_the_reference_depth() {
    {
        let _one = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        assert_eq!(mode(), DEPTH_TPU_D24X8_FLUSH, "the default is mode 4");
    }
    run_modes(&[mode()]);
}

/// The stand-in's table with the recording entries, made once: every
/// allocation (from before `gc_init` on, so the stream buffers too) and every
/// submitted stream.
fn recording_osal() -> *const OsalOps {
    static T: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
    *T.get_or_init(|| {
        // SAFETY: plain data (function pointers); the copy is a second table.
        let mut o: OsalOps = unsafe { core::ptr::read(&OSAL) };
        o.gpu_alloc = Some(rec_gpu_alloc);
        o.gpu_free = Some(rec_gpu_free);
        o.submit = Some(rec_submit);
        Box::leak(Box::new(o)) as *const OsalOps as usize
    }) as *const OsalOps
}

/// The shadow frames under the default, each frame's submitted streams
/// decoded: the recording table is installed before `gc_init` (the harness's
/// `run_full` installs its own there), the rest as `run_full` does it.
fn default_frames() -> Vec<Vec<Vec<Ev>>> {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    with_platform(|p| *p = Platform::default());
    ALLOCS.lock().unwrap().clear();
    PROBLEMS.lock().unwrap().clear();
    *COMPARED.lock().unwrap() = 0;
    GROUND_RESOLVES.lock().unwrap().clear();
    CAPTURE.lock().unwrap().clear();
    // SAFETY: SERIAL is held; the tables are statics or leaked.
    let ctx = unsafe {
        osal_set_ops(recording_osal());
        let mut t: YakoglBackend = core::ptr::read(counted_table());
        t.depth_resolve = Some(rec_depth_resolve);
        yakogl::backend::g_backend = Box::leak(Box::new(t));
        yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
        assert_eq!(yakogl::vivante::backend::gc_init(), 0, "gc_init");
        let c = yakogl::gl::context::gl_context_create(CANVAS_W, CANVAS_H);
        assert!(!c.is_null(), "context pool exhausted");
        c
    };
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        step("setup");
        // SAFETY: plain GL calls (glmark2's Scene::setup state).
        unsafe {
            use yakogl::gl::state::{glCullFace, glEnable};
            glViewport(0, 0, CANVAS_W as GLsizei, CANVAS_H as GLsizei);
            glEnable(GL_DEPTH_TEST);
            glEnable(GL_CULL_FACE);
            glCullFace(GL_BACK);
        }
        let mut frame = shadow_frame(DEPTH_TPU_DEFAULT);
        no_error("the scene's setup");
        let mut all = Vec::new();
        for n in 1..=FRAMES {
            CAPTURE.lock().unwrap().clear();
            canvas_clear();
            frame();
            swap();
            no_error(&format!("frame {n}"));
            all.push(std::mem::take(&mut *CAPTURE.lock().unwrap()));
        }
        forget_scene();
        all
    }));
    // SAFETY: the context made above.
    unsafe {
        yakogl::gl::context::gl_context_destroy(ctx);
        yakogl::vivante::backend::gc_shutdown();
    }
    let frames = r.unwrap_or_else(|e| std::panic::resume_unwind(e));
    let problems = PROBLEMS.lock().unwrap().clone();
    assert!(problems.is_empty(), "default: {problems:?}");
    assert_eq!(*COMPARED.lock().unwrap(), (W * H) as usize * FRAMES, "default: every texel of every frame compared");
    assert_eq!(*GROUND_RESOLVES.lock().unwrap(), vec![false; FRAMES], "default: the sampling draw resolves nothing");
    frames
}

/// Under the default, every shadow frame records `emit_view_flush`'s block
/// once, in front of the draw that samples the depth texture, after the
/// depth pass's draw.
#[test]
fn the_default_flushes_in_front_of_the_sampling_draw() {
    let _one = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    assert_eq!(mode(), DEPTH_TPU_D24X8_FLUSH, "nothing set the knob: the default");
    let frames = default_frames();
    check_flush_in_front_of_the_sampling_draw(&frames);
}

/// Modes 0, 1, 2 and 4 through the knob, each against the same reference.
#[cfg(feature = "test-knobs")]
#[test]
fn every_dtpu_mode_samples_the_reference_depth() {
    run_modes(&[DEPTH_TPU_OFF, DEPTH_TPU_D24X8, DEPTH_TPU_RAW8888, DEPTH_TPU_D24X8_FLUSH]);
}
