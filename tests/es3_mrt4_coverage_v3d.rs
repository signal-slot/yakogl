// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Sample coverage and alpha-to-coverage with four colour targets on the V3D,
//! through the GL API, on the host, under the four-target stand-in
//! (`backend::HOST_DRAW_TARGETS` = 4, what testutil's `backend_draws_targets(4)`
//! stores). Item a2c4-v3d of plan 368; rig from `es3_mrt4_msaa_v3d.rs`.
//!
//! No 4x framebuffer exists today (`GL_MAX_SAMPLES` is 1, a 4-sample
//! renderbuffer is GL_INVALID_OPERATION), so no pixel of a multisampled
//! target is computed or resolved here. What is pinned is the state the
//! back end is handed per draw and what the V3D state layer records for it:
//!
//! - the coverage state (`GlesDraw::sample_mask`, `alpha_to_coverage`) is one
//!   value per draw, not per target: the four-target draw carries one mask,
//!   whichever targets it writes (ES 3.0 4.1.3: coverage comes from output 0's
//!   alpha only and applies to every target);
//! - on 1-sample storage both are ignored (mask 0xf, no alpha-to-coverage);
//! - a hand-made 4x four-target draw: `glSampleCoverage(0.5, FALSE)` is mask
//!   0x3 and `TRUE` 0xc in `state::job_state`'s one SAMPLE_STATE word;
//! - alpha-to-coverage is a named gap in `state::unsupported` (V3D 4.2 has no
//!   control-list bit; Mesa does it in the shader), with or without four
//!   targets.
//!
//! The `#[ignore]`d tests are the expected result once 4x storage and
//! alpha-to-coverage exist (docs/notes/v3d-msaa-es3.md): the per-attachment
//! resolve of the covered samples.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_mrt4_coverage_v3d
//! ```

#![cfg(all(feature = "host-test", feature = "shaderc-online", feature = "gpu-v3d", not(feature = "report-es2")))]

use core::ffi::{c_char, c_void};
use std::ffi::CString;
use std::sync::atomic::Ordering;
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{self, GlesDraw, GlesHeap, YakoglBackend, *};
use yakogl::gl::buffer::glBindBuffer;
use yakogl::gl::consts::*;
use yakogl::gl::draw::*;
use yakogl::gl::framebuffer::*;
use yakogl::gl::program::*;
use yakogl::gl::shader::*;
use yakogl::gl::state::*;
use yakogl::gl::types::*;
use yakogl::gl::vertex::{glEnableVertexAttribArray, glVertexAttribPointer};
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};
use yakogl::v3d::state;

/* ---- a context on the host: the smallest OSAL and back end that make one -- */

unsafe extern "C" fn m_gpu_alloc(_heap: u64, size: usize, _flags: u32, out: *mut OsalGpuMem) -> i32 {
    let mem = Box::leak(vec![0u8; size].into_boxed_slice());
    let p = mem.as_mut_ptr();
    *out = OsalGpuMem { cpu: p as *mut c_void, gpu: p as u64, phys: p as u64, size, impl_: p as *mut c_void };
    0
}
unsafe extern "C" fn m_gpu_free(_mem: *mut OsalGpuMem) -> i32 {
    0
}
unsafe extern "C" fn m_cache(_cpu: *mut c_void, _n: usize) {}
unsafe extern "C" fn b_init() -> i32 {
    0
}
unsafe extern "C" fn b_heap_base(_heap: GlesHeap) -> u64 {
    0x1000_0000
}
unsafe extern "C" fn b_ok() -> i32 {
    0
}

/// What the back end was handed by one draw, and what the V3D made of it.
#[derive(Clone, Debug)]
struct Seen {
    /// An application draw (vertices), not a clear alone.
    draws: bool,
    /// `targets[..num_targets]`: (gpu, format).
    targets: Vec<(u64, u8)>,
    /// `GlesDraw::samples`.
    samples: u8,
    /// `state::refusal` / `state::unsupported`.
    refused: Option<&'static str>,
    /// `state::job_state`'s SAMPLE_STATE mask, when the V3D takes the draw.
    sample_mask: Option<u32>,
    /// `GlesDraw::sample_mask` and `alpha_to_coverage` as the core made them.
    core_mask: u8,
    a2c: bool,
}

static SEEN: Mutex<Vec<Seen>> = Mutex::new(Vec::new());

unsafe extern "C" fn b_draw(d: *const GlesDraw) -> i32 {
    let d = &*d;
    let n = (d.num_targets.max(1) as usize).min(GLES_MAX_TARGETS);
    let refused = state::refusal(d).or_else(|| state::unsupported(d));
    let sample_mask = if d.index_count != 0 { state::job_state(d).ok().map(|j| j.sample_mask) } else { None };
    SEEN.lock().unwrap().push(Seen {
        draws: d.index_count != 0,
        targets: d.targets[..n].iter().map(|t| (t.gpu, t.format)).collect(),
        samples: d.samples,
        refused,
        sample_mask,
        core_mask: d.sample_mask,
        a2c: d.alpha_to_coverage,
    });
    0
}

static OSAL: OsalOps = OsalOps {
    init: None,
    shutdown: None,
    gpu_alloc: Some(m_gpu_alloc),
    gpu_free: Some(m_gpu_free),
    mmu_map: None,
    map_phys: None,
    gpu_write: None,
    gpu_read: None,
    cache_flush: Some(m_cache),
    cache_invd: Some(m_cache),
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

static BACKEND: YakoglBackend = YakoglBackend {
    name: c"es3-mrt4-coverage".as_ptr() as *const c_char,
    native_attrib_formats: 0,
    rt_rgba8888_is_bgra: false,
    mip_level_align: 0,
    compressed_blocks_in_rows: false,
    native_layered_textures: false,
    front_facing_shader_variant: false,
    etc1_decoded_at_upload: false,
    init: Some(b_init),
    shutdown: None,
    draw: Some(b_draw),
    flush: Some(b_ok),
    finish: Some(b_ok),
    finish_previous: None,
    kick_id: None,
    kick_pending: None,
    heap_base: Some(b_heap_base),
    counters: None,
    texture_released: None,
    surface_swapped: None,
    texture_uploaded: None,
    max_instance_divisor: 1,
    wait_kick: None,
    samples_texel_copies: false,
    attachment_gone: None,
    depth_resolve: None,
    query_begin: None,
    query_end: None,
    query_result: None,
    generate_mipmap: None,
    return_mipmap_chain: None,
    linear_row_align: 0,
    uniforms_host_only: false,
    ds_write: None,
    stencil_resolve: None,
    max_samples: 0,
};

/// One context for the whole file; the tests take turns on it.
static LOCK: Mutex<()> = Mutex::new(());
static ONCE: Once = Once::new();

fn context() -> MutexGuard<'static, ()> {
    let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    backend::HOST_DRAW_TARGETS.store(4, Ordering::Relaxed);
    ONCE.call_once(|| {
        // SAFETY: both tables are statics; the lock above serialises the tests.
        unsafe {
            osal_set_ops(&OSAL);
            backend::g_backend = &BACKEND;
            yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
            assert!(!yakogl::gl::context::gl_context_create(8, 8).is_null());
        }
    });
    // SAFETY: plain GL calls.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glFinish();
    }
    SEEN.lock().unwrap().clear();
    // SAFETY: plain GL call.
    while unsafe { glGetError() } != GL_NO_ERROR {}
    g
}

fn err() -> GLenum {
    // SAFETY: plain GL call.
    unsafe { glGetError() }
}

/* ---- framebuffers, the program, the draw ------------------------------- */

struct Fbo {
    fb: GLuint,
    rbs: Vec<GLuint>,
}

impl Drop for Fbo {
    fn drop(&mut self) {
        // SAFETY: names this test made.
        unsafe {
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
            glDeleteFramebuffers(1, &self.fb);
            glDeleteRenderbuffers(self.rbs.len() as GLsizei, self.rbs.as_ptr());
            glUseProgram(0);
        }
    }
}

/// The error of `glRenderbufferStorageMultisample(.., samples, format, 8, 8)`
/// on a fresh renderbuffer, and the renderbuffer.
fn storage(samples: GLsizei, format: GLenum) -> (GLenum, GLuint) {
    let mut rb = 0;
    // SAFETY: plain GL calls.
    unsafe {
        glGenRenderbuffers(1, &mut rb);
        glBindRenderbuffer(GL_RENDERBUFFER, rb);
        while glGetError() != GL_NO_ERROR {}
        glRenderbufferStorageMultisample(GL_RENDERBUFFER, samples, format, 8, 8);
    }
    (err(), rb)
}

/// A bound 8x8 framebuffer with a renderbuffer of `samples` on each of
/// `formats`' colour attachments, all draw buffers. A storage error comes back
/// as `Err(error)`.
fn framebuffer(samples: GLsizei, formats: &[GLenum]) -> Result<Fbo, GLenum> {
    let mut fb = 0;
    // SAFETY: room for one name; plain GL call.
    unsafe {
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
    }
    let mut fbo = Fbo { fb, rbs: Vec::new() };
    for (i, f) in formats.iter().enumerate() {
        let (e, rb) = storage(samples, *f);
        fbo.rbs.push(rb);
        if e != GL_NO_ERROR {
            return Err(e);
        }
        // SAFETY: a renderbuffer this test made.
        unsafe { glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0 + i as GLenum, GL_RENDERBUFFER, rb) };
    }
    let bufs: Vec<GLenum> = (0..formats.len()).map(|i| GL_COLOR_ATTACHMENT0 + i as GLenum).collect();
    // SAFETY: plain GL calls.
    unsafe {
        glDrawBuffers(bufs.len() as GLsizei, bufs.as_ptr());
        assert_eq!(err(), GL_NO_ERROR);
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE, "{formats:#x?}");
    }
    Ok(fbo)
}

fn shader(ty: GLenum, text: &str) -> GLuint {
    let text = CString::new(text).unwrap();
    // SAFETY: plain GL calls; one NUL-terminated source.
    unsafe {
        let s = glCreateShader(ty);
        let p: [*const c_char; 1] = [text.as_ptr()];
        glShaderSource(s, 1, p.as_ptr(), core::ptr::null());
        glCompileShader(s);
        let mut ok: GLint = 0;
        glGetShaderiv(s, GL_COMPILE_STATUS, &mut ok);
        assert_eq!(ok, 1, "{text:?} did not compile");
        s
    }
}

/// The current program: `n` float outputs.
fn program(n: usize) {
    let mut fs = String::from("#version 300 es\nprecision highp float;\n");
    for i in 0..n {
        fs += &format!("layout(location = {i}) out vec4 o{i};\n");
    }
    fs += "void main() {\n";
    for i in 0..n {
        fs += &format!("o{i} = vec4(0.25, 0.5, 0.75, 1.0);\n");
    }
    fs += "}\n";
    let v = shader(GL_VERTEX_SHADER, "#version 300 es\nin vec4 p;\nvoid main() { gl_Position = p; }\n");
    let f = shader(GL_FRAGMENT_SHADER, &fs);
    // SAFETY: plain GL calls.
    unsafe {
        let p = glCreateProgram();
        glAttachShader(p, v);
        glAttachShader(p, f);
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        assert_eq!(ok, 1);
        glUseProgram(p);
    }
}

/// Draw one triangle and hand back what the back end was given for it.
fn draw() -> Seen {
    static TRI: [f32; 12] = [-1.0, -1.0, 0.0, 1.0, 3.0, -1.0, 0.0, 1.0, -1.0, 3.0, 0.0, 1.0];
    SEEN.lock().unwrap().clear();
    // SAFETY: TRI is static and outlives the draw.
    unsafe {
        glBindBuffer(GL_ARRAY_BUFFER, 0);
        glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, TRI.as_ptr() as *const c_void);
        glEnableVertexAttribArray(0);
        glDrawArrays(GL_TRIANGLES, 0, 3);
        glFinish();
    }
    assert_eq!(err(), GL_NO_ERROR);
    let s = core::mem::take(&mut *SEEN.lock().unwrap());
    s.into_iter().rev().find(|s| s.draws).expect("the draw reached the back end")
}

const FOUR: [GLenum; 4] = [GL_RGBA8; 4];

fn enable(cap: GLenum, on: bool) {
    // SAFETY: plain GL calls.
    unsafe {
        if on {
            glEnable(cap)
        } else {
            glDisable(cap)
        }
    }
}

/// Leave the coverage state off for the next test.
struct CoverageOff;
impl Drop for CoverageOff {
    fn drop(&mut self) {
        enable(GL_SAMPLE_COVERAGE, false);
        enable(GL_SAMPLE_ALPHA_TO_COVERAGE, false);
        // SAFETY: plain GL call.
        unsafe { glSampleCoverage(1.0, 0) };
    }
}

/// A hand-made 4x draw with four RGBA8 targets and the given coverage state.
fn draw_4x(mask: u8, a2c: bool, targets: usize) -> GlesDraw {
    let mut d = GlesDraw::ZERO;
    d.target_gpu = 0x0100_0000;
    (d.target_w, d.target_h, d.target_format, d.target_alpha_bits) = (8, 8, GLES_SURF_RGBA8888, 8);
    (d.samples, d.prim, d.index_count, d.line_width) = (4, GLES_PRIM_TRIANGLES, 3, 1.0);
    (d.viewport, d.depth_range, d.color_mask, d.color_write) = ([0, 0, 8, 8], [0.0, 1.0], 0xf, true);
    (d.sample_mask, d.alpha_to_coverage) = (mask, a2c);
    d.num_targets = targets as u8;
    for (i, target) in d.targets.iter_mut().enumerate().skip(1).take(targets - 1) {
        *target = GlesDrawTarget {
            gpu: 0x0100_0000 + 0x1000 * i as u64,
            w: 8,
            h: 8,
            format: GLES_SURF_RGBA8888,
            out_shift: 0,
        };
    }
    d
}

/* ---- the GL core: one coverage state per draw, four targets ---------------- */

/// On 1-sample storage with four targets the coverage state is ignored: the
/// draw carries mask 0xf and no alpha-to-coverage, however it is set, and the
/// V3D takes it.
#[test]
fn single_sample_four_targets_ignore_coverage_state() {
    let _g = context();
    let _off = CoverageOff;
    let _fbo = framebuffer(1, &FOUR).unwrap();
    program(4);
    enable(GL_SAMPLE_COVERAGE, true);
    enable(GL_SAMPLE_ALPHA_TO_COVERAGE, true);
    // SAFETY: plain GL call.
    unsafe { glSampleCoverage(0.5, 1) };
    let s = draw();
    assert_eq!((s.samples, s.targets.len(), s.refused), (1, 4, None));
    assert_eq!((s.core_mask, s.a2c), (0xf, false));
    assert_eq!(s.sample_mask, Some(0xf));
}

/// The coverage state is a context state, not a framebuffer one: it reads back
/// the same with four targets bound, and enabling it raises no error.
#[test]
fn coverage_state_reads_back_with_four_targets_bound() {
    let _g = context();
    let _off = CoverageOff;
    let _fbo = framebuffer(1, &FOUR).unwrap();
    enable(GL_SAMPLE_COVERAGE, true);
    enable(GL_SAMPLE_ALPHA_TO_COVERAGE, true);
    // SAFETY: plain GL calls, one value each.
    unsafe {
        glSampleCoverage(0.5, 1);
        assert_eq!(glIsEnabled(GL_SAMPLE_COVERAGE), 1);
        assert_eq!(glIsEnabled(GL_SAMPLE_ALPHA_TO_COVERAGE), 1);
        let mut v = 0.0f32;
        glGetFloatv(GL_SAMPLE_COVERAGE_VALUE, &mut v);
        assert_eq!(v, 0.5);
        let mut inv: GLint = -1;
        glGetIntegerv(GL_SAMPLE_COVERAGE_INVERT, &mut inv);
        assert_eq!(inv, 1);
    }
    assert_eq!(err(), GL_NO_ERROR);
}

/// The draw has one `sample_mask` and one flag whatever its target count: a
/// one- and a four-target draw of the same state carry the same pair.
#[test]
fn coverage_is_per_draw_not_per_target() {
    let _g = context();
    let _off = CoverageOff;
    enable(GL_SAMPLE_COVERAGE, true);
    // SAFETY: plain GL call.
    unsafe { glSampleCoverage(0.5, 0) };
    let mut seen = Vec::new();
    for n in [1usize, 4] {
        let _fbo = framebuffer(1, &FOUR[..n]).unwrap();
        program(n);
        let s = draw();
        assert_eq!(s.targets.len(), n);
        seen.push((s.core_mask, s.a2c, s.sample_mask));
    }
    assert_eq!(seen[0], seen[1]);
}

/* ---- the V3D state layer: one SAMPLE_STATE word, any target count ---------- */

/// glSampleCoverage(0.5, GL_FALSE / GL_TRUE) on 4 samples: the first two
/// samples (0x3), then the other two (0xc); the job's one mask is the same
/// for one, two, three and four targets.
#[test]
fn sample_coverage_masks_the_same_samples_for_every_target_count() {
    for (mask, want) in [(0x3u8, 0x3u32), (0xc, 0xc)] {
        for n in 1..=4 {
            let d = draw_4x(mask, false, n);
            assert_eq!(state::refusal(&d), None, "{n} targets");
            assert_eq!(state::unsupported(&d), None, "{n} targets");
            let j = state::job_state(&d).expect("a 4x draw's job state");
            assert_eq!(j.sample_mask, want, "mask {mask:#x}, {n} targets");
        }
    }
}

/// Alpha-to-coverage is one flag per draw: the V3D names it as a gap for
/// one target and for four alike, so four targets cannot take a different
/// path (output 0's alpha only, applied to all of them, is not yet computed).
#[test]
fn alpha_to_coverage_is_a_named_gap_at_every_target_count() {
    for n in 1..=4 {
        let d = draw_4x(0xf, true, n);
        assert_eq!(state::refusal(&d), None, "{n} targets");
        assert_eq!(state::unsupported(&d), Some("alpha-to-coverage is not implemented"), "{n} targets");
    }
    /* without the flag the same four-target draw is taken */
    assert_eq!(state::unsupported(&draw_4x(0xf, false, 4)), None);
}

/* ---- the expected result: ignored until the design lands ------------------- */

#[test]
#[ignore = "defect: owner src/gl/framebuffer.rs (4x renderbuffer storage refused, GL_MAX_SAMPLES 1) and src/v3d/state.rs:62 (alpha-to-coverage not implemented): ES 3.0 4.1.3 alpha-to-coverage takes output 0's alpha only and masks all four 4x targets alike; output 3 with alpha 0 must not drop samples"]
fn alpha_to_coverage_uses_output_0_alpha_for_all_four_targets() {
    let _g = context();
    let _off = CoverageOff;
    let fbo = framebuffer(4, &FOUR).expect("4 samples are legal storage");
    program(4);
    enable(GL_SAMPLE_ALPHA_TO_COVERAGE, true);
    let s = draw();
    assert_eq!((s.samples, s.targets.len(), s.refused), (4, 4, None));
    drop(fbo);
}

#[test]
#[ignore = "defect: owner src/gl/framebuffer.rs (4x renderbuffer storage refused) and src/v3d/backend.rs draw_call (4x with several colour targets refused): glSampleCoverage(0.5, FALSE/TRUE) must mask the same samples on all four targets after a glBlitFramebuffer resolve of each"]
fn sample_coverage_resolves_the_same_samples_on_all_four_targets() {
    let _g = context();
    let _off = CoverageOff;
    let fbo = framebuffer(4, &FOUR).expect("4 samples are legal storage");
    program(4);
    enable(GL_SAMPLE_COVERAGE, true);
    // SAFETY: plain GL call.
    unsafe { glSampleCoverage(0.5, 0) };
    let s = draw();
    assert_eq!((s.samples, s.targets.len(), s.refused, s.core_mask), (4, 4, None, 0x3));
    drop(fbo);
}
