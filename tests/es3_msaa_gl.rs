// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! ES 3.0 multisample renderbuffers, the GL core's half, on the host
//! (docs/notes/es3-msaa-renderbuffers.md (a), (b); docs/notes/v3d-msaa-es3.md (4)).
//!
//! The GL core is driven through the API into a stand-in back end. Today it
//! keeps no samples for a framebuffer object: GL_MAX_SAMPLES is the constant
//! 1, `glRenderbufferStorageMultisample` refuses 2 or more samples with
//! GL_INVALID_OPERATION and stores 1 as plain storage, `GlesDraw::samples` of
//! an FBO draw is 1. The passing tests pin that (the capability off).
//!
//! The `#[ignore]`d tests are the capability-on half: a back end with
//! `max_samples` 4 (the `YakoglBackend` field `gl-msaa-cap` adds; it does not
//! exist yet, so the stand-in here cannot say 4 and the lines that need it are
//! marked TODO(gl-msaa-cap)). Each ignore reason names the item that lands it,
//! `gl-msaa-cap` or `gl-msaa-resolve`, and the note's (b) item. Those items
//! build the 4x stand-in table and un-ignore them.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_msaa_gl
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
use yakogl::gl::texture::{glBindTexture, glDeleteTextures, glGenTextures, glTexImage2D};
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
    name: c"es3-msaa-gl".as_ptr() as *const c_char,
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
};

/// One context for the whole file; the tests take turns on it.
static LOCK: Mutex<()> = Mutex::new(());
static ONCE: Once = Once::new();

/// The context with the stand-in back end of today: no multisample capability.
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

/// The context with a stand-in back end that says `max_samples` 4.
///
/// TODO(gl-msaa-cap): switch the table to a second `YakoglBackend` with
/// `max_samples: 4` once the field exists; until then this is `context()` and
/// every test that calls it is `#[ignore]`d.
fn context_ms4() -> MutexGuard<'static, ()> {
    context()
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
fn texture() -> GLuint {
    let mut tex = 0;
    // SAFETY: plain GL calls; one name out.
    unsafe {
        glGenTextures(1, &mut tex);
        glBindTexture(GL_TEXTURE_2D, tex);
        glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA8 as GLint, 8, 8, 0, GL_RGBA, GL_UNSIGNED_BYTE, core::ptr::null());
    }
    assert_eq!(err(), GL_NO_ERROR);
    tex
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

const FULL: [GLint; 4] = [0, 0, 8, 8];

/* ---- today, the capability off: passing pins -------------------------------- */

/// GL_MAX_SAMPLES is the constant 1 (src/gl/state.rs), below ES 3.0's 4.
#[test]
fn max_samples_is_one_today() {
    let _g = context();
    assert_eq!(geti(GL_MAX_SAMPLES), 1);
}

/// glGetInternalformativ: one sample count, which is 1, for the renderable
/// colour formats, and no counts for the integer ones.
#[test]
fn internalformat_counts_today() {
    let _g = context();
    for f in [GL_RGBA8, GL_SRGB8_ALPHA8, GL_RGB10_A2, GL_RGB8, GL_RGB565, GL_RGBA4, GL_RGB5_A1, GL_R8, GL_RG8] {
        assert_eq!(internalformat(f, GL_NUM_SAMPLE_COUNTS, 1), (vec![1], GL_NO_ERROR), "{f:#x} count");
        assert_eq!(internalformat(f, GL_SAMPLES, 1), (vec![1], GL_NO_ERROR), "{f:#x} samples");
    }
    for f in [GL_RGBA8I, GL_RGBA8UI, GL_R32I] {
        assert_eq!(internalformat(f, GL_NUM_SAMPLE_COUNTS, 1), (vec![0], GL_NO_ERROR), "{f:#x} count");
        assert_eq!(internalformat(f, GL_SAMPLES, 1), (vec![-777], GL_NO_ERROR), "{f:#x} lists nothing");
    }
}

/// glRenderbufferStorageMultisample: 4 is GL_INVALID_OPERATION, 1 stores
/// GL_RENDERBUFFER_SAMPLES 1, 0 stores 0, a negative count is GL_INVALID_VALUE.
#[test]
fn storage_samples_today() {
    let _g = context();
    for n in [2, 4, 8] {
        assert_eq!(storage(n, GL_RGBA8).0, GL_INVALID_OPERATION, "{n} samples");
    }
    assert_eq!(storage(-1, GL_RGBA8).0, GL_INVALID_VALUE);
    let (e, rb) = storage(1, GL_RGBA8);
    assert_eq!((e, rb_samples(rb)), (GL_NO_ERROR, 1));
    let (e, rb) = storage(0, GL_RGBA8);
    assert_eq!((e, rb_samples(rb)), (GL_NO_ERROR, 0));
    // SAFETY: plain GL call.
    unsafe { glBindRenderbuffer(GL_RENDERBUFFER, 0) };
}

/// ES 3.0 has no multisample integer format: samples > 0 is GL_INVALID_OPERATION.
#[test]
fn integer_formats_with_samples_are_invalid_operation() {
    let _g = context();
    for f in [GL_R8I, GL_R8UI, GL_RG16I, GL_RGBA8I, GL_RGBA8UI, GL_RGBA32UI, GL_RGB10_A2UI] {
        for n in [1, 4] {
            assert_eq!(storage(n, f).0, GL_INVALID_OPERATION, "{f:#x} with {n}");
        }
        assert_eq!(storage(0, f).0, GL_NO_ERROR, "{f:#x} with 0");
    }
}

/// A 1-sample renderbuffer counts as having sample buffers: next to a texture
/// (no sample buffers) the framebuffer is GL_FRAMEBUFFER_INCOMPLETE_MULTISAMPLE.
#[test]
fn one_sample_renderbuffer_with_a_texture_is_incomplete() {
    let _g = context();
    let fbo = framebuffer(1, &[GL_RGBA8]).unwrap();
    let tex = texture();
    // SAFETY: plain GL calls on names this test made.
    unsafe { glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT1, GL_TEXTURE_2D, tex, 0) };
    assert_eq!(err(), GL_NO_ERROR);
    assert_eq!(status(), GL_FRAMEBUFFER_INCOMPLETE_MULTISAMPLE);
    // SAFETY: plain GL call.
    unsafe { glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT1, GL_TEXTURE_2D, 0, 0) };
    assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE, "the texture gone");
    // SAFETY: a texture this test made.
    unsafe { glDeleteTextures(1, &tex) };
    drop(fbo);
}

/// An FBO draw is single-sample, whatever count the renderbuffer stored.
/// (It carries no multisample identity yet: `GlesDraw` has no target_id.)
#[test]
fn fbo_draw_is_one_sample_today() {
    let _g = context();
    for n in [0, 1] {
        let _fbo = framebuffer(n, &[GL_RGBA8]).unwrap();
        program(); /* the framebuffer's drop unbinds it */
        assert_eq!(draw().samples, 1, "renderbuffer stored with {n}");
    }
}

/// glReadPixels from a framebuffer with sample buffers (the 1-sample
/// renderbuffer counts) is GL_INVALID_OPERATION; from a 0-sample one it works.
#[test]
fn read_pixels_from_a_one_sample_fbo_today() {
    let _g = context();
    let mut px = [0u8; 4];
    let read = |px: &mut [u8; 4]| {
        // SAFETY: room for one RGBA8 pixel.
        unsafe { glReadPixels(0, 0, 1, 1, GL_RGBA, GL_UNSIGNED_BYTE, px.as_mut_ptr() as *mut c_void) };
        err()
    };
    let fbo = framebuffer(1, &[GL_RGBA8]).unwrap();
    assert_eq!(read(&mut px), GL_INVALID_OPERATION);
    drop(fbo);
    let _fbo = framebuffer(0, &[GL_RGBA8]).unwrap();
    assert_eq!(read(&mut px), GL_NO_ERROR);
}

/* ---- the capability on: ignored until gl-msaa-cap / gl-msaa-resolve ---------- */

/// Note (b)1: GL_MAX_SAMPLES is the back end's `max_samples`, 4.
#[test]
#[ignore = "gl-msaa-cap, note (b)1: GL_MAX_SAMPLES is max(max_samples, 1) once the capability is wired"]
fn max_samples_is_four() {
    let _g = context_ms4();
    assert_eq!(geti(GL_MAX_SAMPLES), 4);
}

/// Note (b)1: one sample count, 4, for the multisample-renderable formats;
/// none for the integer ones.
#[test]
#[ignore = "gl-msaa-cap, note (b)1: glGetInternalformativ answers NUM_SAMPLE_COUNTS 1 and SAMPLES {4}"]
fn internalformat_counts_four() {
    let _g = context_ms4();
    for f in [GL_RGBA8, GL_SRGB8_ALPHA8, GL_RGB10_A2, GL_RGB8, GL_RGB565, GL_RGBA4, GL_RGB5_A1, GL_R8, GL_RG8] {
        assert_eq!(internalformat(f, GL_NUM_SAMPLE_COUNTS, 1), (vec![1], GL_NO_ERROR), "{f:#x} count");
        assert_eq!(internalformat(f, GL_SAMPLES, 1), (vec![4], GL_NO_ERROR), "{f:#x} samples");
    }
    for f in [GL_RGBA8I, GL_RGBA8UI] {
        assert_eq!(internalformat(f, GL_NUM_SAMPLE_COUNTS, 1), (vec![0], GL_NO_ERROR), "{f:#x} count");
    }
}

/// Note (b)1/(b)2: any request 1..=4 stores 4, 0 stores 0, 5 and more stay
/// GL_INVALID_OPERATION, integer formats stay refused.
#[test]
#[ignore = "gl-msaa-cap, note (b)1/(b)2: glRenderbufferStorageMultisample keeps the real count, 4 for 1..=4"]
fn storage_keeps_four() {
    let _g = context_ms4();
    for n in 1..=4 {
        let (e, rb) = storage(n, GL_RGBA8);
        assert_eq!((e, rb_samples(rb)), (GL_NO_ERROR, 4), "request {n}");
    }
    let (e, rb) = storage(0, GL_RGBA8);
    assert_eq!((e, rb_samples(rb)), (GL_NO_ERROR, 0));
    for n in [5, 8] {
        assert_eq!(storage(n, GL_RGBA8).0, GL_INVALID_OPERATION, "{n} samples");
    }
    assert_eq!(storage(4, GL_RGBA8UI).0, GL_INVALID_OPERATION, "integer format");
    // SAFETY: plain GL call.
    unsafe { glBindRenderbuffer(GL_RENDERBUFFER, 0) };
}

/// Note (b)3: a 4x renderbuffer over 2048x2048 pixels is GL_OUT_OF_MEMORY.
#[test]
#[ignore = "gl-msaa-cap, note (b)3: the 64 MiB sample cap is GL_OUT_OF_MEMORY"]
fn storage_over_the_cap_is_out_of_memory() {
    let _g = context_ms4();
    assert_eq!(storage_sized(4, GL_RGBA8, 2049, 2048).0, GL_OUT_OF_MEMORY);
    assert_eq!(storage_sized(0, GL_RGBA8, 2049, 2048).0, GL_NO_ERROR, "single-sample storage has no cap here");
}

/// Note (b)10: the real counts decide completeness: a 4x renderbuffer next to
/// a texture or a renderbuffer of another count is INCOMPLETE_MULTISAMPLE; the
/// same counts are complete.
#[test]
#[ignore = "gl-msaa-cap, note (b)10: check_status compares the real sample counts of all attachments"]
fn incomplete_multisample_with_real_counts() {
    let _g = context_ms4();
    let fbo = framebuffer(4, &[GL_RGBA8, GL_RGBA8]).unwrap();
    assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE, "4 and 4");
    let (_, rb0) = storage(0, GL_RGBA8);
    // SAFETY: plain GL call on a name this test made.
    unsafe { glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT1, GL_RENDERBUFFER, rb0) };
    assert_eq!(status(), GL_FRAMEBUFFER_INCOMPLETE_MULTISAMPLE, "4 and 0");
    /* a request of 1 stores 4 with the capability on: 4 and 4 again */
    let (_, rb1) = storage(1, GL_RGBA8);
    // SAFETY: plain GL call on a name this test made.
    unsafe { glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT1, GL_RENDERBUFFER, rb1) };
    assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE, "4 and (1 -> 4)");
    let tex = texture();
    // SAFETY: plain GL call on a name this test made.
    unsafe { glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT1, GL_TEXTURE_2D, tex, 0) };
    assert_eq!(status(), GL_FRAMEBUFFER_INCOMPLETE_MULTISAMPLE, "4 and a texture");
    // SAFETY: names this test made.
    unsafe {
        glDeleteTextures(1, &tex);
        glDeleteRenderbuffers(2, [rb0, rb1].as_ptr());
    }
    drop(fbo);
}

/// Note (b)4/(b)5: an FBO draw hands the back end samples 4.
#[test]
#[ignore = "gl-msaa-cap, note (b)4: GlesDraw.samples is framebuffer_samples for an FBO"]
fn fbo_draw_is_four_samples() {
    let _g = context_ms4();
    let _fbo = framebuffer(4, &[GL_RGBA8]).unwrap();
    program();
    assert_eq!(draw().samples, 4);
    // TODO(gl-msaa-cap): the draw also carries target_id (ds_object_id(rb.serial,
    // false)) and target_gen (storage_gen) of the renderbuffer, 0 for the window;
    // record them in Seen once GlesDraw has them and assert: non-zero id, the gen
    // changes when the renderbuffer is respecified, the id does not.
}

/// Note (b)9: a 4x read framebuffer resolves by glBlitFramebuffer into a
/// single-sample draw framebuffer of the same size and format.
#[test]
#[ignore = "gl-msaa-resolve, note (b)9: glBlitFramebuffer resolves a multisample read to a single-sample draw"]
fn resolve_blit_works() {
    let _g = context_ms4();
    let src = framebuffer(4, &[GL_RGBA8]).unwrap();
    let dst = framebuffer(0, &[GL_RGBA8]).unwrap();
    assert_eq!(blit(src.fb, dst.fb, FULL, FULL), GL_NO_ERROR);
}

/// ES 3.0 4.3.3: a resolve cannot scale: the source and destination
/// rectangles must have the same size.
#[test]
#[ignore = "gl-msaa-resolve, note (b)9: a scaled resolve is GL_INVALID_OPERATION"]
fn scaled_resolve_is_invalid_operation() {
    let _g = context_ms4();
    let src = framebuffer(4, &[GL_RGBA8]).unwrap();
    let dst = framebuffer(0, &[GL_RGBA8]).unwrap();
    assert_eq!(blit(src.fb, dst.fb, FULL, [0, 0, 4, 4]), GL_INVALID_OPERATION, "shrink");
    assert_eq!(blit(src.fb, dst.fb, [0, 0, 4, 4], FULL), GL_INVALID_OPERATION, "enlarge");
    assert_eq!(blit(src.fb, dst.fb, FULL, [8, 8, 0, 0]), GL_INVALID_OPERATION, "flipped");
}

/// ES 3.0 4.3.3: the formats of the colour buffers must match.
#[test]
#[ignore = "gl-msaa-resolve, note (b)9: a format-mismatched resolve is GL_INVALID_OPERATION"]
fn format_mismatched_resolve_is_invalid_operation() {
    let _g = context_ms4();
    let src = framebuffer(4, &[GL_RGBA8]).unwrap();
    let dst = framebuffer(0, &[GL_RGB565]).unwrap();
    assert_eq!(blit(src.fb, dst.fb, FULL, FULL), GL_INVALID_OPERATION);
}

/// ES 3.0 4.3.3: the source and destination rectangles of a resolve must be
/// the same region, not only the same size.
#[test]
#[ignore = "gl-msaa-resolve, note (b)9: a region-mismatched resolve is GL_INVALID_OPERATION"]
fn region_mismatched_resolve_is_invalid_operation() {
    let _g = context_ms4();
    let src = framebuffer(4, &[GL_RGBA8]).unwrap();
    let dst = framebuffer(0, &[GL_RGBA8]).unwrap();
    assert_eq!(blit(src.fb, dst.fb, [0, 0, 4, 4], [4, 4, 8, 8]), GL_INVALID_OPERATION, "same size, moved");
    assert_eq!(blit(src.fb, dst.fb, [0, 0, 4, 4], [0, 0, 4, 4]), GL_NO_ERROR, "same region");
}

/// ES 3.0 4.3.3: a draw framebuffer with sample buffers is GL_INVALID_OPERATION,
/// from a single-sample read and from a 4x one.
#[test]
#[ignore = "gl-msaa-resolve, note (b)9: a blit into a 4x draw framebuffer is GL_INVALID_OPERATION"]
fn blit_into_a_4x_draw_framebuffer_is_invalid_operation() {
    let _g = context_ms4();
    let ms = framebuffer(4, &[GL_RGBA8]).unwrap();
    let single = framebuffer(0, &[GL_RGBA8]).unwrap();
    let ms2 = framebuffer(4, &[GL_RGBA8]).unwrap();
    assert_eq!(blit(single.fb, ms.fb, FULL, FULL), GL_INVALID_OPERATION, "1x to 4x");
    assert_eq!(blit(ms2.fb, ms.fb, FULL, FULL), GL_INVALID_OPERATION, "4x to 4x");
}
