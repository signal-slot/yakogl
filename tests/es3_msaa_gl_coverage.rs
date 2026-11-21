// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GlesDraw::sample_mask and alpha_to_coverage of draws into a 4x framebuffer
//! object and a 1-sample one, on the 4x stand-in back end
//! (docs/notes/es3-msaa-renderbuffers.md (b); src/gl/draw.rs set_sample_state).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_msaa_gl_coverage
//! ```

#![cfg(all(feature = "host-test", feature = "shaderc-online", not(feature = "report-es2")))]

use core::ffi::{c_char, c_void};
use std::ffi::CString;
use std::sync::atomic::Ordering;
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{self, GlesDraw, GlesHeap, YakoglBackend};
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

/// What the back end was handed by one draw.
#[derive(Clone, Debug)]
struct Seen {
    /// An application draw (vertices), not a clear alone.
    draws: bool,
    /// `GlesDraw::samples`.
    samples: u8,
    /// `GlesDraw::sample_mask`.
    mask: u8,
    /// `GlesDraw::alpha_to_coverage`.
    a2c: bool,
}

static SEEN: Mutex<Vec<Seen>> = Mutex::new(Vec::new());

unsafe extern "C" fn b_draw(d: *const GlesDraw) -> i32 {
    let d = &*d;
    SEEN.lock().unwrap().push(Seen {
        draws: d.index_count != 0,
        samples: d.samples,
        mask: d.sample_mask,
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
    name: c"es3-msaa-gl-coverage".as_ptr() as *const c_char,
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

/// [`BACKEND`] with the multisample capability on: it keeps 4 samples.
static BACKEND_MS4: YakoglBackend = YakoglBackend { max_samples: 4, ..BACKEND };

/// One context for the whole file; the tests take turns on it.
static LOCK: Mutex<()> = Mutex::new(());
static ONCE: Once = Once::new();

/// The context with `table` as the back end (a test that swapped in the 4x
/// one does not leak it into the next).
fn context_on(table: &'static YakoglBackend) -> MutexGuard<'static, ()> {
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
    // SAFETY: a static table; the lock above serialises the tests. Plain GL calls.
    unsafe {
        backend::g_backend = table;
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glFinish();
    }
    SEEN.lock().unwrap().clear();
    // SAFETY: plain GL call.
    while unsafe { glGetError() } != GL_NO_ERROR {}
    g
}

/// The context with a stand-in back end that says `max_samples` 4.
fn context_ms4() -> MutexGuard<'static, ()> {
    context_on(&BACKEND_MS4)
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

/// `glRenderbufferStorageMultisample(.., samples, format, w, h)` on a fresh
/// renderbuffer: its error and the renderbuffer (left bound).
fn storage_sized(samples: GLsizei, format: GLenum, w: GLsizei, h: GLsizei) -> (GLenum, GLuint) {
    let mut rb = 0;
    // SAFETY: plain GL calls.
    unsafe {
        glGenRenderbuffers(1, &mut rb);
        glBindRenderbuffer(GL_RENDERBUFFER, rb);
        while glGetError() != GL_NO_ERROR {}
        glRenderbufferStorageMultisample(GL_RENDERBUFFER, samples, format, w, h);
    }
    (err(), rb)
}

fn storage(samples: GLsizei, format: GLenum) -> (GLenum, GLuint) {
    storage_sized(samples, format, 8, 8)
}

/// A bound 8x8 framebuffer with a renderbuffer of `samples` on each of
/// `formats`' colour attachments. A storage error comes back as `Err(error)`.
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
    assert_eq!(err(), GL_NO_ERROR);
    Ok(fbo)
}

fn status() -> GLenum {
    // SAFETY: plain GL call.
    unsafe { glCheckFramebufferStatus(GL_FRAMEBUFFER) }
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

/// The current program: one float output.
fn program() {
    let v = shader(GL_VERTEX_SHADER, "#version 300 es\nin vec4 p;\nvoid main() { gl_Position = p; }\n");
    let f = shader(
        GL_FRAGMENT_SHADER,
        "#version 300 es\nprecision highp float;\nlayout(location = 0) out vec4 o;\nvoid main() { o = vec4(0.25, 0.5, 0.75, 1.0); }\n",
    );
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

/// Draw one triangle over the bound framebuffer, the job left unfinished.
fn draw_unfinished() {
    static TRI: [f32; 12] = [-1.0, -1.0, 0.0, 1.0, 3.0, -1.0, 0.0, 1.0, -1.0, 3.0, 0.0, 1.0];
    // SAFETY: TRI is static and outlives the draw.
    unsafe {
        glBindBuffer(GL_ARRAY_BUFFER, 0);
        glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, TRI.as_ptr() as *const c_void);
        glEnableVertexAttribArray(0);
        glDrawArrays(GL_TRIANGLES, 0, 3);
    }
}

/// Draw one triangle; what the back end was given, or None when the GL core
/// dropped the draw (an empty coverage mask on a multisample target).
fn try_draw() -> Option<Seen> {
    SEEN.lock().unwrap().clear();
    draw_unfinished();
    // SAFETY: plain GL call.
    unsafe { glFinish() };
    assert_eq!(err(), GL_NO_ERROR);
    let s = core::mem::take(&mut *SEEN.lock().unwrap());
    s.into_iter().rev().find(|s| s.draws)
}

fn coverage(on: bool, value: f32, invert: bool) {
    // SAFETY: plain GL calls.
    unsafe {
        if on {
            glEnable(GL_SAMPLE_COVERAGE);
            glSampleCoverage(value, invert as GLboolean);
        } else {
            glDisable(GL_SAMPLE_COVERAGE);
        }
    }
    assert_eq!(err(), GL_NO_ERROR);
}

fn a2c(on: bool) {
    // SAFETY: plain GL calls.
    unsafe {
        if on {
            glEnable(GL_SAMPLE_ALPHA_TO_COVERAGE)
        } else {
            glDisable(GL_SAMPLE_ALPHA_TO_COVERAGE)
        }
    }
    assert_eq!(err(), GL_NO_ERROR);
}

/// Coverage state back to the defaults.
struct Reset;
impl Drop for Reset {
    fn drop(&mut self) {
        // SAFETY: plain GL calls.
        unsafe {
            glDisable(GL_SAMPLE_COVERAGE);
            glDisable(GL_SAMPLE_ALPHA_TO_COVERAGE);
            glSampleCoverage(1.0, 0);
        }
    }
}

fn setup(samples: GLsizei) -> Fbo {
    let fbo = framebuffer(samples, &[GL_RGBA8]).expect("storage");
    program();
    fbo
}

#[test]
fn ms4_default_state_draws_full_mask() {
    let _g = context_ms4();
    let _r = Reset;
    let _f = setup(4);
    assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE);
    let s = try_draw().expect("draw");
    assert_eq!((s.samples, s.mask, s.a2c), (4, 0xf, false));
}

#[test]
fn ms4_sample_coverage_masks() {
    let _g = context_ms4();
    let _r = Reset;
    let _f = setup(4);
    for (v, inv, mask) in [
        (0.5, false, 0b0011u8),
        (0.5, true, 0b1100),
        (1.0, false, 0b1111),
        (0.25, false, 0b0001),
        (0.75, false, 0b0111),
    ] {
        coverage(true, v, inv);
        let s = try_draw().expect("draw");
        assert_eq!((s.samples, s.mask), (4, mask), "value {v} invert {inv}");
    }
}

#[test]
fn ms4_sample_coverage_empty_mask_drops_draw() {
    let _g = context_ms4();
    let _r = Reset;
    let _f = setup(4);
    /* ES 3.0 4.1.3: value 0.0 covers no sample (sample_mask 0); 1.0 inverted likewise.
     * The GL core drops such a draw (draw.rs covers_no_sample) rather than sending mask 0. */
    coverage(true, 0.0, false);
    assert!(try_draw().is_none());
    coverage(true, 1.0, true);
    assert!(try_draw().is_none());
    /* 0.0 inverted is all samples */
    coverage(true, 0.0, true);
    assert_eq!(try_draw().expect("draw").mask, 0b1111);
}

#[test]
fn ms4_sample_coverage_disabled_ignores_value() {
    let _g = context_ms4();
    let _r = Reset;
    let _f = setup(4);
    coverage(true, 0.5, false);
    coverage(false, 0.5, false);
    assert_eq!(try_draw().expect("draw").mask, 0b1111);
}

#[test]
fn single_sample_fbo_ignores_coverage() {
    let _g = context_ms4();
    let _r = Reset;
    let _f = setup(0);
    for (v, inv) in [(0.5, false), (0.5, true), (1.0, false), (0.0, false), (1.0, true)] {
        coverage(true, v, inv);
        let s = try_draw().expect("a 1-sample target ignores coverage, the draw is not dropped");
        assert_eq!((s.samples, s.mask, s.a2c), (1, 0xf, false), "value {v} invert {inv}");
    }
}

#[test]
fn alpha_to_coverage_4x_and_1x() {
    let _g = context_ms4();
    let _r = Reset;
    {
        let _f = setup(4);
        a2c(true);
        let s = try_draw().expect("draw");
        assert_eq!((s.samples, s.mask, s.a2c), (4, 0xf, true));
        a2c(false);
        assert!(!try_draw().expect("draw").a2c);
    }
    {
        let _f = setup(0);
        a2c(true);
        let s = try_draw().expect("draw");
        /* ES 3.0 4.1.3: no effect without sample buffers; the GL core sends false */
        assert_eq!((s.samples, s.mask, s.a2c), (1, 0xf, false));
    }
}

#[test]
fn alpha_to_coverage_with_sample_coverage() {
    let _g = context_ms4();
    let _r = Reset;
    let _f = setup(4);
    a2c(true);
    coverage(true, 0.5, true);
    let s = try_draw().expect("draw");
    assert_eq!((s.mask, s.a2c), (0b1100, true));
}

#[test]
fn switching_fbos_updates_samples_and_mask() {
    let _g = context_ms4();
    let _r = Reset;
    let f4 = setup(4);
    let f1 = framebuffer(0, &[GL_RGBA8]).expect("storage");
    coverage(true, 0.5, false);
    a2c(true);
    for _ in 0..2 {
        // SAFETY: framebuffers this test made.
        unsafe { glBindFramebuffer(GL_FRAMEBUFFER, f4.fb) };
        let s = try_draw().expect("draw");
        assert_eq!((s.samples, s.mask, s.a2c), (4, 0b0011, true));
        // SAFETY: as above.
        unsafe { glBindFramebuffer(GL_FRAMEBUFFER, f1.fb) };
        let s = try_draw().expect("draw");
        assert_eq!((s.samples, s.mask, s.a2c), (1, 0xf, false));
    }
}
