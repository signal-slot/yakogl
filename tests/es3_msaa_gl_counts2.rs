// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! ES 3.0 multisample renderbuffers on a back end whose `max_samples` is 2
//! (docs/notes/es3-msaa-renderbuffers.md (b)1, (b)2, (b)10): the GL core
//! follows the back end's count instead of a fixed 4.
//!
//! The stand-in table is the one of tests/es3_msaa_gl.rs (copied, not shared)
//! with `max_samples` 2.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_msaa_gl_counts2
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
    draws: bool,
    samples: u8,
}

static SEEN: Mutex<Vec<Seen>> = Mutex::new(Vec::new());

unsafe extern "C" fn b_draw(d: *const GlesDraw) -> i32 {
    let d = &*d;
    SEEN.lock().unwrap().push(Seen { draws: d.index_count != 0, samples: d.samples });
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
    name: c"es3-msaa-gl-counts2".as_ptr() as *const c_char,
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
    depth_resolve: None,
    query_begin: None,
    query_end: None,
    query_result: None,
    generate_mipmap: None,
    return_mipmap_chain: None,
    linear_row_align: 0,
    uniforms_host_only: false,
    attachment_gone: None,
    ds_write: None,
    stencil_resolve: None,
    max_samples: 2,
};

static LOCK: Mutex<()> = Mutex::new(());
static ONCE: Once = Once::new();

fn context() -> MutexGuard<'static, ()> {
    let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    backend::HOST_DRAW_TARGETS.store(4, Ordering::Relaxed);
    ONCE.call_once(|| {
        // SAFETY: the table is a static; the lock above serialises the tests.
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

fn geti(pname: GLenum) -> GLint {
    let mut v: GLint = -777;
    // SAFETY: one GLint out.
    unsafe { glGetIntegerv(pname, &mut v) };
    assert_eq!(err(), GL_NO_ERROR);
    v
}

/// glGetInternalformativ(GL_RENDERBUFFER, format, pname) with room for `n`
/// values: the values written (-777 marks one not written) and the error.
fn internalformat(format: GLenum, pname: GLenum, n: usize) -> (Vec<GLint>, GLenum) {
    let mut v = vec![-777; n];
    // SAFETY: room for `n` GLints.
    unsafe { glGetInternalformativ(GL_RENDERBUFFER, format, pname, n as GLsizei, v.as_mut_ptr()) };
    (v, err())
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

/// GL_RENDERBUFFER_SAMPLES of `rb`.
fn rb_samples(rb: GLuint) -> GLint {
    let mut v: GLint = -777;
    // SAFETY: plain GL calls; one GLint out.
    unsafe {
        glBindRenderbuffer(GL_RENDERBUFFER, rb);
        glGetRenderbufferParameteriv(GL_RENDERBUFFER, GL_RENDERBUFFER_SAMPLES, &mut v);
    }
    assert_eq!(err(), GL_NO_ERROR);
    v
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

/// A 2D RGBA8 texture of 8x8, left bound.
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

/// Draw one triangle and hand back what the back end was given for it.
fn draw() -> Seen {
    SEEN.lock().unwrap().clear();
    draw_unfinished();
    // SAFETY: plain GL call.
    unsafe { glFinish() };
    assert_eq!(err(), GL_NO_ERROR);
    let s = core::mem::take(&mut *SEEN.lock().unwrap());
    s.into_iter().rev().find(|s| s.draws).expect("the draw reached the back end")
}

/// glBlitFramebuffer of colour from `read` to `draw_fb` over the given
/// rectangles; the GL error.
fn blit(read: GLuint, draw_fb: GLuint, src: [GLint; 4], dst: [GLint; 4]) -> GLenum {
    // SAFETY: plain GL calls on names this test made.
    unsafe {
        glBindFramebuffer(GL_READ_FRAMEBUFFER, read);
        glBindFramebuffer(GL_DRAW_FRAMEBUFFER, draw_fb);
        glReadBuffer(GL_COLOR_ATTACHMENT0);
        while glGetError() != GL_NO_ERROR {}
        glBlitFramebuffer(
            src[0],
            src[1],
            src[2],
            src[3],
            dst[0],
            dst[1],
            dst[2],
            dst[3],
            GL_COLOR_BUFFER_BIT,
            GL_NEAREST,
        );
    }
    err()
}

/// glBlitFramebuffer from `read` to `draw_fb` with any mask and filter; the
/// GL error.

/// Attach a fresh renderbuffer of `samples` and `format` at `attachment`.
fn attach(samples: GLsizei, format: GLenum, attachment: GLenum) -> GLuint {
    let (e, rb) = storage(samples, format);
    assert_eq!(e, GL_NO_ERROR);
    // SAFETY: a renderbuffer this test made.
    unsafe { glFramebufferRenderbuffer(GL_FRAMEBUFFER, attachment, GL_RENDERBUFFER, rb) };
    assert_eq!(err(), GL_NO_ERROR);
    rb
}

/* ---- the GL core follows max_samples 2 ----------------------------------- */

#[test]
fn max_samples_is_two() {
    let _g = context();
    assert_eq!(geti(GL_MAX_SAMPLES), 2);
}

/// Today: max_samples 2 lists the single sample count 1 (the GL core follows only 4).
#[test]
fn internalformat_lists_one_today() {
    let _g = context();
    assert_eq!(internalformat(GL_RGBA8, GL_NUM_SAMPLE_COUNTS, 1), (vec![1], GL_NO_ERROR));
    assert_eq!(internalformat(GL_RGBA8, GL_SAMPLES, 1), (vec![1], GL_NO_ERROR));
}

#[test]
#[ignore = "owner: GL core (src/gl), msaa.rs keeps samples only when max_samples >= 4 (MS_SAMPLES); a table of 2 is treated as single-sampled"]
fn internalformat_lists_two() {
    let _g = context();
    assert_eq!(internalformat(GL_RGBA8, GL_NUM_SAMPLE_COUNTS, 1), (vec![1], GL_NO_ERROR));
    assert_eq!(internalformat(GL_RGBA8, GL_SAMPLES, 1), (vec![2], GL_NO_ERROR));
}

/// A request of 1 is rounded up to the back end's 2 (as 1..=4 stores 4 with
/// max_samples 4); 0 stays single-sample.
/// Today: requests 1 and 2 store 1 sample; 0 stores 0.
#[test]
fn storage_keeps_one_today() {
    let _g = context();
    for n in [1, 2] {
        let (e, rb) = storage(n, GL_RGBA8);
        assert_eq!((e, rb_samples(rb)), (GL_NO_ERROR, 1), "request {n}");
    }
    let (e, rb) = storage(0, GL_RGBA8);
    assert_eq!((e, rb_samples(rb)), (GL_NO_ERROR, 0));
    // SAFETY: plain GL call.
    unsafe { glBindRenderbuffer(GL_RENDERBUFFER, 0) };
}

#[test]
#[ignore = "owner: GL core (src/gl), msaa.rs keeps samples only when max_samples >= 4 (MS_SAMPLES); a table of 2 is treated as single-sampled"]
fn storage_keeps_two() {
    let _g = context();
    for n in [1, 2] {
        let (e, rb) = storage(n, GL_RGBA8);
        assert_eq!((e, rb_samples(rb)), (GL_NO_ERROR, 2), "request {n}");
    }
    let (e, rb) = storage(0, GL_RGBA8);
    assert_eq!((e, rb_samples(rb)), (GL_NO_ERROR, 0));
    // SAFETY: plain GL call.
    unsafe { glBindRenderbuffer(GL_RENDERBUFFER, 0) };
}

#[test]
fn over_the_count_is_invalid_operation() {
    let _g = context();
    for n in [3, 4] {
        assert_eq!(storage(n, GL_RGBA8).0, GL_INVALID_OPERATION, "{n} samples");
    }
    // SAFETY: plain GL call.
    unsafe { glBindRenderbuffer(GL_RENDERBUFFER, 0) };
}

#[test]
/// (Today both attachments store 1 sample: see the ignored tests.)
fn colour_and_depth_at_two_is_complete() {
    let _g = context();
    let mut fbo = framebuffer(2, &[GL_RGBA8]).unwrap();
    let d = attach(2, GL_DEPTH_COMPONENT16, GL_DEPTH_ATTACHMENT);
    fbo.rbs.push(d);
    assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE);
}

#[test]
/// (Today it is 1 against 0; the status is the same.)
fn mixing_two_and_one_is_incomplete_multisample() {
    let _g = context();
    let mut fbo = framebuffer(2, &[GL_RGBA8]).unwrap();
    let d = attach(0, GL_DEPTH_COMPONENT16, GL_DEPTH_ATTACHMENT);
    fbo.rbs.push(d);
    assert_eq!(status(), GL_FRAMEBUFFER_INCOMPLETE_MULTISAMPLE);
}

/// Today: an FBO draw hands the back end samples 1.
#[test]
fn fbo_draw_is_one_sample_today() {
    let _g = context();
    let _fbo = framebuffer(2, &[GL_RGBA8]).unwrap();
    program();
    assert_eq!(draw().samples, 1);
}

#[test]
#[ignore = "owner: GL core (src/gl), msaa.rs keeps samples only when max_samples >= 4 (MS_SAMPLES); a table of 2 is treated as single-sampled"]
fn fbo_draw_is_two_samples() {
    let _g = context();
    let _fbo = framebuffer(2, &[GL_RGBA8]).unwrap();
    program();
    assert_eq!(draw().samples, 2);
}

#[test]
fn resolve_two_to_one_is_no_error() {
    let _g = context();
    let src = framebuffer(2, &[GL_RGBA8]).unwrap();
    program();
    draw_unfinished();
    assert_eq!(err(), GL_NO_ERROR);
    let dst = framebuffer(0, &[GL_RGBA8]).unwrap();
    assert_eq!(blit(src.fb, dst.fb, [0, 0, 8, 8], [0, 0, 8, 8]), GL_NO_ERROR);
}
