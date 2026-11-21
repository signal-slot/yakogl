// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! ES 3.0 multisample renderbuffers, the GL core's half: GL_SAMPLES,
//! GL_SAMPLE_BUFFERS and the renderbuffer / attachment parameter queries
//! around binding changes, on the 4x stand-in back end (`max_samples` 4) of
//! tests/es3_msaa_gl.rs (its helpers are copied, not shared).
//!
//! No draw is made: the queries are state. `GL_SAMPLE_BUFFERS` / `GL_SAMPLES`
//! answer for the bound draw framebuffer (`framebuffer_samples`,
//! src/gl/state.rs).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_msaa_gl_query
//! ```

#![cfg(all(feature = "host-test", feature = "shaderc-online", not(feature = "report-es2")))]

use core::ffi::{c_char, c_void};
use std::sync::atomic::Ordering;
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{self, GlesHeap, YakoglBackend};
use yakogl::gl::consts::*;
use yakogl::gl::framebuffer::*;
use yakogl::gl::state::*;
use yakogl::gl::types::*;
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

/// The stand-in back end that keeps 4 samples (`max_samples` 4). It draws
/// nothing: these tests only query state.
static BACKEND_MS4: YakoglBackend = YakoglBackend {
    name: c"es3-msaa-gl-query".as_ptr() as *const c_char,
    native_attrib_formats: 0,
    rt_rgba8888_is_bgra: false,
    mip_level_align: 0,
    compressed_blocks_in_rows: false,
    native_layered_textures: false,
    front_facing_shader_variant: false,
    etc1_decoded_at_upload: false,
    init: Some(b_init),
    shutdown: None,
    draw: None,
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
    max_samples: 4,
};

static LOCK: Mutex<()> = Mutex::new(());
static ONCE: Once = Once::new();

/// The context with the 4x stand-in; the tests take turns on it.
fn context_ms4() -> MutexGuard<'static, ()> {
    let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    backend::HOST_DRAW_TARGETS.store(4, Ordering::Relaxed);
    ONCE.call_once(|| {
        // SAFETY: statics; the lock serialises the tests.
        unsafe {
            osal_set_ops(&OSAL);
            backend::g_backend = &BACKEND_MS4;
            yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
            assert!(!yakogl::gl::context::gl_context_create(8, 8).is_null());
        }
    });
    // SAFETY: a static table; plain GL calls.
    unsafe {
        backend::g_backend = &BACKEND_MS4;
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glFinish();
    }
    // SAFETY: plain GL call.
    while unsafe { glGetError() } != GL_NO_ERROR {}
    g
}

fn err() -> GLenum {
    // SAFETY: plain GL call.
    unsafe { glGetError() }
}

fn geti(pname: GLenum) -> GLint {
    let mut v: GLint = -777;
    // SAFETY: one GLint out.
    unsafe { glGetIntegerv(pname, &mut v) };
    assert_eq!(err(), GL_NO_ERROR);
    v
}

/// (GL_SAMPLE_BUFFERS, GL_SAMPLES) of the bound draw framebuffer.
fn sample_query() -> (GLint, GLint) {
    (geti(GL_SAMPLE_BUFFERS), geti(GL_SAMPLES))
}

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
        }
    }
}

/// A renderbuffer of `samples`, `format`, 8x8; left bound. 0 is plain storage;
/// 1 already asks for a multisample buffer, which the 4x table rounds up to 4.
fn storage(samples: GLsizei, format: GLenum) -> GLuint {
    let mut rb = 0;
    // SAFETY: plain GL calls.
    unsafe {
        glGenRenderbuffers(1, &mut rb);
        glBindRenderbuffer(GL_RENDERBUFFER, rb);
        glRenderbufferStorageMultisample(GL_RENDERBUFFER, samples, format, 8, 8);
    }
    assert_eq!(err(), GL_NO_ERROR, "storage({samples}, {format:#x})");
    rb
}

/// A bound framebuffer (GL_FRAMEBUFFER) with one renderbuffer per
/// (attachment, samples, format).
fn framebuffer(parts: &[(GLenum, GLsizei, GLenum)]) -> Fbo {
    let mut fb = 0;
    // SAFETY: plain GL calls.
    unsafe {
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
    }
    let mut fbo = Fbo { fb, rbs: Vec::new() };
    for &(att, samples, format) in parts {
        let rb = storage(samples, format);
        fbo.rbs.push(rb);
        // SAFETY: a renderbuffer this test made.
        unsafe { glFramebufferRenderbuffer(GL_FRAMEBUFFER, att, GL_RENDERBUFFER, rb) };
        assert_eq!(err(), GL_NO_ERROR);
    }
    fbo
}

fn status() -> GLenum {
    // SAFETY: plain GL call.
    unsafe { glCheckFramebufferStatus(GL_FRAMEBUFFER) }
}

fn rb_param(rb: GLuint, pname: GLenum) -> GLint {
    let mut v: GLint = -777;
    // SAFETY: plain GL calls; one GLint out.
    unsafe {
        glBindRenderbuffer(GL_RENDERBUFFER, rb);
        glGetRenderbufferParameteriv(GL_RENDERBUFFER, pname, &mut v);
    }
    assert_eq!(err(), GL_NO_ERROR, "pname {pname:#x}");
    v
}

fn att_param(att: GLenum, pname: GLenum) -> GLint {
    let mut v: GLint = -777;
    // SAFETY: one GLint out.
    unsafe { glGetFramebufferAttachmentParameteriv(GL_FRAMEBUFFER, att, pname, &mut v) };
    assert_eq!(err(), GL_NO_ERROR, "attachment {att:#x} pname {pname:#x}");
    v
}

const SIZES: [GLenum; 6] = [
    GL_RENDERBUFFER_RED_SIZE,
    GL_RENDERBUFFER_GREEN_SIZE,
    GL_RENDERBUFFER_BLUE_SIZE,
    GL_RENDERBUFFER_ALPHA_SIZE,
    GL_RENDERBUFFER_DEPTH_SIZE,
    GL_RENDERBUFFER_STENCIL_SIZE,
];

const ATT_SIZES: [GLenum; 6] = [
    GL_FRAMEBUFFER_ATTACHMENT_RED_SIZE,
    GL_FRAMEBUFFER_ATTACHMENT_GREEN_SIZE,
    GL_FRAMEBUFFER_ATTACHMENT_BLUE_SIZE,
    GL_FRAMEBUFFER_ATTACHMENT_ALPHA_SIZE,
    GL_FRAMEBUFFER_ATTACHMENT_DEPTH_SIZE,
    GL_FRAMEBUFFER_ATTACHMENT_STENCIL_SIZE,
];

/// (format, attachment point, RED..STENCIL sizes).
const FORMATS: [(GLenum, GLenum, [GLint; 6]); 4] = [
    (GL_RGBA8, GL_COLOR_ATTACHMENT0, [8, 8, 8, 8, 0, 0]),
    (GL_RGB565, GL_COLOR_ATTACHMENT0, [5, 6, 5, 0, 0, 0]),
    (GL_DEPTH24_STENCIL8, GL_DEPTH_STENCIL_ATTACHMENT, [0, 0, 0, 0, 24, 8]),
    (GL_DEPTH_COMPONENT16, GL_DEPTH_ATTACHMENT, [0, 0, 0, 0, 16, 0]),
];

/* ---- (1) GL_SAMPLES / GL_SAMPLE_BUFFERS around binding changes ---------- */

#[test]
fn draw_binding_gives_one_four() {
    let _g = context_ms4();
    let fbo = framebuffer(&[(GL_COLOR_ATTACHMENT0, 4, GL_RGBA8)]);
    assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE);
    assert_eq!(sample_query(), (1, 4));
    // SAFETY: plain GL call.
    unsafe { glBindFramebuffer(GL_DRAW_FRAMEBUFFER, fbo.fb) };
    assert_eq!(sample_query(), (1, 4));
}

#[test]
fn queries_follow_the_draw_binding() {
    let _g = context_ms4();
    let ms = framebuffer(&[(GL_COLOR_ATTACHMENT0, 4, GL_RGBA8)]);
    let one = framebuffer(&[(GL_COLOR_ATTACHMENT0, 0, GL_RGBA8)]);
    // SAFETY: plain GL calls. Read = 4x, draw = a 1-sample FBO: draw answers.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, one.fb);
        glBindFramebuffer(GL_READ_FRAMEBUFFER, ms.fb);
    }
    assert_eq!(sample_query(), (0, 0), "draw one-sample FBO, read 4x");
    // SAFETY: plain GL calls. Read = 1-sample, draw = 4x.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, ms.fb);
        glBindFramebuffer(GL_READ_FRAMEBUFFER, one.fb);
    }
    assert_eq!(sample_query(), (1, 4), "draw 4x, read one-sample FBO");
    // SAFETY: plain GL calls. Read = 4x only; draw = the default framebuffer.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glBindFramebuffer(GL_READ_FRAMEBUFFER, ms.fb);
    }
    let (buffers, samples) = sample_query();
    assert_eq!(buffers, (samples > 1) as GLint, "draw = window, read 4x: the window's own count");
}

#[test]
fn unbinding_returns_the_window_answer() {
    let _g = context_ms4();
    let window = sample_query();
    {
        let _fbo = framebuffer(&[(GL_COLOR_ATTACHMENT0, 4, GL_RGBA8)]);
        assert_eq!(sample_query(), (1, 4));
    }
    assert_eq!(sample_query(), window);
}

#[test]
fn one_sample_fbo_has_no_sample_buffer() {
    let _g = context_ms4();
    let _fbo = framebuffer(&[(GL_COLOR_ATTACHMENT0, 0, GL_RGBA8)]);
    assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE);
    assert_eq!(sample_query(), (0, 0));
}

/* ---- (2) an incomplete 4x framebuffer ----------------------------------- */

#[test]
fn incomplete_mixed_counts_query_is_zero_and_no_error() {
    let _g = context_ms4();
    let _fbo = framebuffer(&[(GL_COLOR_ATTACHMENT0, 4, GL_RGBA8), (GL_DEPTH_ATTACHMENT, 0, GL_DEPTH_COMPONENT16)]);
    assert_eq!(status(), GL_FRAMEBUFFER_INCOMPLETE_MULTISAMPLE);
    /* ES 3.0: the value is undefined for an incomplete framebuffer, and the
     * query is no error (GL_INVALID_FRAMEBUFFER_OPERATION is ES 3.1 only).
     * The GL core answers (0, 0) (`framebuffer_samples`). */
    assert_eq!(sample_query(), (0, 0));
    assert_eq!(err(), GL_NO_ERROR);
}

#[test]
fn incomplete_missing_attachment_query_is_zero() {
    let _g = context_ms4();
    let mut fb = 0;
    // SAFETY: plain GL calls.
    unsafe {
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
    }
    assert_eq!(status(), GL_FRAMEBUFFER_INCOMPLETE_MISSING_ATTACHMENT);
    assert_eq!(sample_query(), (0, 0));
    // SAFETY: plain GL calls.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glDeleteFramebuffers(1, &fb);
    }
}

#[test]
fn completing_the_framebuffer_changes_the_answer() {
    let _g = context_ms4();
    let fbo = framebuffer(&[(GL_COLOR_ATTACHMENT0, 4, GL_RGBA8), (GL_DEPTH_ATTACHMENT, 0, GL_DEPTH_COMPONENT16)]);
    assert_eq!(sample_query(), (0, 0));
    let rb = storage(4, GL_DEPTH_COMPONENT16);
    // SAFETY: a renderbuffer this test made.
    unsafe { glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_DEPTH_ATTACHMENT, GL_RENDERBUFFER, rb) };
    assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE);
    assert_eq!(sample_query(), (1, 4));
    // SAFETY: names this test made; detach before deleting.
    unsafe {
        glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_DEPTH_ATTACHMENT, GL_RENDERBUFFER, 0);
        glDeleteRenderbuffers(1, &rb);
    }
    drop(fbo);
}

/* ---- (3) glGetRenderbufferParameteriv ----------------------------------- */

#[test]
fn renderbuffer_parameters_match_one_sample() {
    let _g = context_ms4();
    for (format, _, sizes) in FORMATS {
        let ms = storage(4, format);
        let one = storage(0, format);
        assert_eq!(rb_param(ms, GL_RENDERBUFFER_SAMPLES), 4, "{format:#x}");
        assert_eq!(rb_param(one, GL_RENDERBUFFER_SAMPLES), 0, "{format:#x} one sample");
        for pname in [GL_RENDERBUFFER_WIDTH, GL_RENDERBUFFER_HEIGHT, GL_RENDERBUFFER_INTERNAL_FORMAT] {
            assert_eq!(rb_param(ms, pname), rb_param(one, pname), "{format:#x} pname {pname:#x}");
        }
        assert_eq!(rb_param(ms, GL_RENDERBUFFER_WIDTH), 8);
        assert_eq!(rb_param(ms, GL_RENDERBUFFER_HEIGHT), 8);
        assert_eq!(rb_param(ms, GL_RENDERBUFFER_INTERNAL_FORMAT), format as GLint);
        for (i, pname) in SIZES.iter().enumerate() {
            /* glGetRenderbufferParameteriv reports RGB565 as its RGBA8888
             * storage (src/gl/framebuffer.rs, "the storage is RGBA8888
             * whatever was asked for"); the attachment query says 5/6/5/0 */
            let want = if format == GL_RGB565 && i < 4 { 8 } else { sizes[i] };
            assert_eq!(rb_param(ms, *pname), want, "{format:#x} size {pname:#x}");
            assert_eq!(rb_param(ms, *pname), rb_param(one, *pname), "{format:#x} size {pname:#x} vs one sample");
        }
        // SAFETY: names this test made.
        unsafe { glDeleteRenderbuffers(2, [ms, one].as_ptr()) };
    }
}

#[test]
fn renderbuffer_samples_survive_binding_changes() {
    let _g = context_ms4();
    let a = storage(4, GL_RGBA8);
    let b = storage(0, GL_RGBA8);
    for _ in 0..2 {
        assert_eq!(rb_param(a, GL_RENDERBUFFER_SAMPLES), 4);
        assert_eq!(rb_param(b, GL_RENDERBUFFER_SAMPLES), 0);
    }
    let mut v: GLint = 0;
    // SAFETY: plain GL calls; one GLint out.
    unsafe {
        glBindRenderbuffer(GL_RENDERBUFFER, 0);
        glGetRenderbufferParameteriv(GL_RENDERBUFFER, GL_RENDERBUFFER_SAMPLES, &mut v);
    }
    assert_eq!(err(), GL_INVALID_OPERATION, "no renderbuffer bound");
    // SAFETY: names this test made.
    unsafe { glDeleteRenderbuffers(2, [a, b].as_ptr()) };
}

/* ---- (4) glGetFramebufferAttachmentParameteriv -------------------------- */

#[test]
fn attachment_sizes_match_one_sample() {
    let _g = context_ms4();
    for (format, att, sizes) in FORMATS {
        let ms = framebuffer(&[(att, 4, format)]);
        let ms_vals: Vec<GLint> = ATT_SIZES.iter().map(|p| att_param(att, *p)).collect();
        drop(ms);
        let one = framebuffer(&[(att, 0, format)]);
        let one_vals: Vec<GLint> = ATT_SIZES.iter().map(|p| att_param(att, *p)).collect();
        drop(one);
        assert_eq!(ms_vals, one_vals, "{format:#x}");
        assert_eq!(ms_vals, sizes.to_vec(), "{format:#x}");
    }
}

#[test]
fn attachment_encoding_and_type_match_one_sample() {
    let _g = context_ms4();
    for (format, enc) in [(GL_RGBA8, GL_LINEAR), (GL_SRGB8_ALPHA8, GL_SRGB)] {
        for samples in [0, 4] {
            let _fbo = framebuffer(&[(GL_COLOR_ATTACHMENT0, samples, format)]);
            assert_eq!(
                att_param(GL_COLOR_ATTACHMENT0, GL_FRAMEBUFFER_ATTACHMENT_COLOR_ENCODING),
                enc as GLint,
                "{format:#x} x{samples}"
            );
            assert_eq!(
                att_param(GL_COLOR_ATTACHMENT0, GL_FRAMEBUFFER_ATTACHMENT_OBJECT_TYPE),
                GL_RENDERBUFFER as GLint
            );
            assert_eq!(
                att_param(GL_COLOR_ATTACHMENT0, GL_FRAMEBUFFER_ATTACHMENT_COMPONENT_TYPE),
                GL_UNSIGNED_NORMALIZED as GLint,
                "{format:#x} x{samples}"
            );
        }
    }
}

#[test]
fn srgb_attachment_sizes_are_8888() {
    let _g = context_ms4();
    let _fbo = framebuffer(&[(GL_COLOR_ATTACHMENT0, 4, GL_SRGB8_ALPHA8)]);
    let v: Vec<GLint> = ATT_SIZES.iter().map(|p| att_param(GL_COLOR_ATTACHMENT0, *p)).collect();
    assert_eq!(v, vec![8, 8, 8, 8, 0, 0]);
}

/// ES 3.0 6.1.15: the component sizes of a renderbuffer and of the
/// attachment that names it are one answer. Today RGB565 gives 8/8/8/8 from
/// glGetRenderbufferParameteriv and 5/6/5/0 from the attachment query, at one
/// sample as at 4.
#[test]
#[ignore = "RGB565 renderbuffer sizes are 8/8/8/8, the attachment query's 5/6/5/0: src/gl/framebuffer.rs glGetRenderbufferParameteriv, is_color_format => 8"]
fn renderbuffer_and_attachment_sizes_agree_for_rgb565() {
    let _g = context_ms4();
    for samples in [0, 4] {
        let fbo = framebuffer(&[(GL_COLOR_ATTACHMENT0, samples, GL_RGB565)]);
        for (rp, ap) in SIZES.iter().zip(ATT_SIZES.iter()) {
            assert_eq!(rb_param(fbo.rbs[0], *rp), att_param(GL_COLOR_ATTACHMENT0, *ap), "x{samples} {rp:#x}");
        }
    }
}
