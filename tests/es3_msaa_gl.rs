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
//! The capability-on half runs on a second stand-in table that says
//! `max_samples` 4 (`context_ms4`): GL_MAX_SAMPLES 4, renderbuffers stored
//! with 4, completeness on the real counts, FBO draws of 4 samples
//! (gl-msaa-cap), and the resolve blit (gl-msaa-resolve). That table's
//! `finish` stands in for the job-end resolve of note (b)7: it writes the
//! resolved colour into the renderbuffer's memory then, and not before, so a
//! blit that copied without waiting for the job would read zeros.
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

/// The 4x targets drawn since the last `finish`: (`target_gpu`, w, h).
static PENDING: Mutex<Vec<(u64, u32, u32)>> = Mutex::new(Vec::new());

/// The colour the stand-in's job-end resolve leaves in an RGBA8 target.
const RESOLVED: [u8; 4] = [0x40, 0x80, 0xc0, 0xff];

unsafe extern "C" fn b_draw(d: *const GlesDraw) -> i32 {
    let d = &*d;
    SEEN.lock().unwrap().push(Seen { draws: d.index_count != 0, samples: d.samples });
    if d.samples > 1 {
        PENDING.lock().unwrap().push((d.target_gpu, d.target_w, d.target_h));
    }
    0
}

/// Note (b)7 in the stand-in: when the jobs are complete, `rb.mem` of every
/// 4x renderbuffer drawn holds the resolved colour, and not before (a copy
/// that did not wait reads the zeros the storage was made with).
unsafe extern "C" fn b_finish_resolve() -> i32 {
    for (gpu, w, h) in core::mem::take(&mut *PENDING.lock().unwrap()) {
        /* the stand-in OSAL's gpu address is the CPU pointer; RGBA8 rows of w*4 */
        let px = core::slice::from_raw_parts_mut(gpu as *mut [u8; 4], (w * h) as usize);
        px.fill(RESOLVED);
    }
    0
}

/// What `depth_resolve` / `stencil_resolve` were asked for: (id, w, h).
static DS_READS: Mutex<Vec<(u32, u32, u32)>> = Mutex::new(Vec::new());
/// One `ds_write`: (id, plane, x, y, w, h, the first element as bits).
type DsWrite = (u32, u32, u32, u32, u32, u32, u32);
/// What `ds_write` was handed.
static DS_WRITES: Mutex<Vec<DsWrite>> = Mutex::new(Vec::new());

/// The depth the stand-in keeps for every object: one sample a pixel (note (b)8).
const DEPTH: f32 = 0.375;
const STENCIL: u8 = 0x5a;

unsafe extern "C" fn b_depth_resolve(id: u32, dst: *mut f32, w: u32, h: u32, pitch: u32) -> i32 {
    DS_READS.lock().unwrap().push((id, w, h));
    core::slice::from_raw_parts_mut(dst, (pitch * h) as usize).fill(DEPTH);
    0
}
unsafe extern "C" fn b_stencil_resolve(id: u32, dst: *mut u8, w: u32, h: u32, pitch: u32) -> i32 {
    DS_READS.lock().unwrap().push((id, w, h));
    core::slice::from_raw_parts_mut(dst, (pitch * h) as usize).fill(STENCIL);
    0
}
#[allow(clippy::too_many_arguments)]
unsafe extern "C" fn b_ds_write(
    id: u32,
    plane: u32,
    src: *const c_void,
    x: u32,
    y: u32,
    w: u32,
    h: u32,
    _pitch: u32,
) -> i32 {
    let first =
        if plane == backend::DS_PLANE_STENCIL { *(src as *const u8) as u32 } else { (*(src as *const f32)).to_bits() };
    DS_WRITES.lock().unwrap().push((id, plane, x, y, w, h, first));
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
    max_samples: 0,
};

/// [`BACKEND`] with the multisample capability on: it keeps 4 samples.
static BACKEND_MS4: YakoglBackend = YakoglBackend { max_samples: 4, finish: Some(b_finish_resolve), ..BACKEND };

/// [`BACKEND_MS4`] that also keeps depth/stencil planes per object and reads
/// them back one sample a pixel (note (b)8).
static BACKEND_MS4_DS: YakoglBackend = YakoglBackend {
    depth_resolve: Some(b_depth_resolve),
    stencil_resolve: Some(b_stencil_resolve),
    ds_write: Some(b_ds_write),
    ..BACKEND_MS4
};

/// One context for the whole file; the tests take turns on it.
static LOCK: Mutex<()> = Mutex::new(());
static ONCE: Once = Once::new();

/// The context with the stand-in back end of today: no multisample capability.
fn context() -> MutexGuard<'static, ()> {
    context_on(&BACKEND)
}

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
    PENDING.lock().unwrap().clear();
    DS_READS.lock().unwrap().clear();
    DS_WRITES.lock().unwrap().clear();
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
    blit_with(read, draw_fb, src, dst, GL_COLOR_BUFFER_BIT, GL_NEAREST)
}

/// glBlitFramebuffer from `read` to `draw_fb` with any mask and filter; the
/// GL error.
fn blit_with(
    read: GLuint,
    draw_fb: GLuint,
    src: [GLint; 4],
    dst: [GLint; 4],
    mask: GLbitfield,
    filter: GLenum,
) -> GLenum {
    // SAFETY: plain GL calls on names this test made.
    unsafe {
        glBindFramebuffer(GL_READ_FRAMEBUFFER, read);
        glBindFramebuffer(GL_DRAW_FRAMEBUFFER, draw_fb);
        glReadBuffer(GL_COLOR_ATTACHMENT0);
        while glGetError() != GL_NO_ERROR {}
        glBlitFramebuffer(src[0], src[1], src[2], src[3], dst[0], dst[1], dst[2], dst[3], mask, filter);
    }
    err()
}

/// The 8x8 RGBA8 pixels of colour attachment 0 of single-sample `fb`, row
/// by row from the bottom.
fn pixels(fb: GLuint) -> Vec<[u8; 4]> {
    let mut px = vec![[0x11u8; 4]; 64];
    // SAFETY: room for 8x8 RGBA/UNSIGNED_BYTE pixels (rows of 32 bytes).
    unsafe {
        glBindFramebuffer(GL_READ_FRAMEBUFFER, fb);
        glReadBuffer(GL_COLOR_ATTACHMENT0);
        glReadPixels(0, 0, 8, 8, GL_RGBA, GL_UNSIGNED_BYTE, px.as_mut_ptr() as *mut c_void);
    }
    assert_eq!(err(), GL_NO_ERROR);
    px
}

/// A 4x RGBA8 framebuffer drawn into, its job submitted but not finished.
fn drawn_4x() -> Fbo {
    let src = framebuffer(4, &[GL_RGBA8]).unwrap();
    program();
    draw_unfinished();
    assert_eq!(err(), GL_NO_ERROR);
    assert_eq!(PENDING.lock().unwrap().len(), 1, "the 4x draw's job is pending");
    src
}

/// Is pixel (x, y) of an 8x8 image inside rectangle `r` (x0, y0, x1, y1)?
fn inside(i: usize, r: [GLint; 4]) -> bool {
    let (x, y) = ((i % 8) as GLint, (i / 8) as GLint);
    x >= r[0] && x < r[2] && y >= r[1] && y < r[3]
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

/* ---- the capability on (gl-msaa-cap, gl-msaa-resolve) ---------------------- */

/// Note (b)1: GL_MAX_SAMPLES is the back end's `max_samples`, 4.
#[test]
fn max_samples_is_four() {
    let _g = context_ms4();
    assert_eq!(geti(GL_MAX_SAMPLES), 4);
}

/// Note (b)1: one sample count, 4, for the multisample-renderable formats;
/// none for the integer ones.
#[test]
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
fn storage_over_the_cap_is_out_of_memory() {
    let _g = context_ms4();
    assert_eq!(storage_sized(4, GL_RGBA8, 2049, 2048).0, GL_OUT_OF_MEMORY);
    assert_eq!(storage_sized(0, GL_RGBA8, 2049, 2048).0, GL_NO_ERROR, "single-sample storage has no cap here");
}

/// Note (b)10: the real counts decide completeness: a 4x renderbuffer next to
/// a texture or a renderbuffer of another count is INCOMPLETE_MULTISAMPLE; the
/// same counts are complete.
#[test]
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
fn fbo_draw_is_four_samples() {
    let _g = context_ms4();
    let _fbo = framebuffer(4, &[GL_RGBA8]).unwrap();
    program();
    assert_eq!(draw().samples, 4);
    // TODO(gl-msaa-resolve): the draw also carries target_id (ds_object_id(rb.serial,
    // false)) and target_gen (storage_gen) of the renderbuffer, 0 for the window;
    // record them in Seen once GlesDraw has them and assert: non-zero id, the gen
    // changes when the renderbuffer is respecified, the id does not.
}

/// Note (b)7/(b)9: a 4x read framebuffer resolves by glBlitFramebuffer into a
/// single-sample draw framebuffer of the same size and format. The draw's job
/// is still pending: the blit flushes and waits for it, then copies the
/// resolved colour the back end left in `rb.mem`.
#[test]
fn resolve_blit_works() {
    let _g = context_ms4();
    let dst = framebuffer(0, &[GL_RGBA8]).unwrap();
    let src = drawn_4x();
    assert_eq!(blit(src.fb, dst.fb, FULL, FULL), GL_NO_ERROR);
    assert!(PENDING.lock().unwrap().is_empty(), "the blit waited for the 4x job");
    assert!(pixels(dst.fb).iter().all(|p| *p == RESOLVED), "the resolved colour, every pixel");
}

/// ES 3.0 4.3.3: a resolve cannot scale: the source and destination
/// rectangles must have the same size, and a flip is not the same bounds.
/// The argument errors outrank it. Nothing is copied.
#[test]
fn scaled_resolve_is_invalid_operation() {
    let _g = context_ms4();
    let dst = framebuffer(0, &[GL_RGBA8]).unwrap();
    let src = drawn_4x();
    assert_eq!(blit(src.fb, dst.fb, FULL, [0, 0, 4, 4]), GL_INVALID_OPERATION, "shrink");
    assert_eq!(blit(src.fb, dst.fb, [0, 0, 4, 4], FULL), GL_INVALID_OPERATION, "enlarge");
    assert_eq!(blit(src.fb, dst.fb, FULL, [8, 8, 0, 0]), GL_INVALID_OPERATION, "flipped");
    assert_eq!(blit(src.fb, dst.fb, FULL, [8, 0, 0, 8]), GL_INVALID_OPERATION, "mirrored in x");
    let c = GL_COLOR_BUFFER_BIT;
    assert_eq!(blit_with(src.fb, dst.fb, FULL, [0, 0, 4, 4], c, GL_LINEAR), GL_INVALID_OPERATION, "LINEAR");
    assert_eq!(blit_with(src.fb, dst.fb, FULL, [0, 0, 4, 4], c, GL_RGBA), GL_INVALID_ENUM, "the filter first");
    assert_eq!(blit_with(src.fb, dst.fb, FULL, [0, 0, 4, 4], c | 0x2, GL_NEAREST), GL_INVALID_VALUE, "the mask first");
    assert_eq!(PENDING.lock().unwrap().len(), 1, "a refused blit waits for nothing");
    assert!(pixels(dst.fb).iter().all(|p| *p == [0; 4]), "no copy");
}

/// ES 3.0 4.3.3: the formats of the colour buffers must be identical, the
/// argument errors outranking it. The sRGB encoding is folded away first, as
/// Mesa does (blit.c `compatible_resolve_formats`: "Blits between linear and
/// sRGB formats are also allowed").
#[test]
fn format_mismatched_resolve_is_invalid_operation() {
    let _g = context_ms4();
    let src = framebuffer(4, &[GL_RGBA8]).unwrap();
    for f in [GL_RGB565, GL_RGB8, GL_RGBA4, GL_RGB5_A1, GL_RGB10_A2] {
        let dst = framebuffer(0, &[f]).unwrap();
        assert_eq!(blit(src.fb, dst.fb, FULL, FULL), GL_INVALID_OPERATION, "RGBA8 into {f:#x}");
        let e = blit_with(src.fb, dst.fb, FULL, FULL, GL_COLOR_BUFFER_BIT, GL_RGBA);
        assert_eq!(e, GL_INVALID_ENUM, "the filter outranks it, {f:#x}");
    }
    let srgb = framebuffer(0, &[GL_SRGB8_ALPHA8]).unwrap();
    assert_eq!(blit(src.fb, srgb.fb, FULL, FULL), GL_NO_ERROR, "linear into sRGB");
    let src = framebuffer(4, &[GL_SRGB8_ALPHA8]).unwrap();
    let dst = framebuffer(0, &[GL_RGBA8]).unwrap();
    assert_eq!(blit(src.fb, dst.fb, FULL, FULL), GL_NO_ERROR, "sRGB into linear");
    assert_eq!(blit(src.fb, srgb.fb, FULL, FULL), GL_NO_ERROR, "sRGB into sRGB");
    let dst = framebuffer(0, &[GL_RGB565]).unwrap();
    assert_eq!(blit(src.fb, dst.fb, FULL, FULL), GL_INVALID_OPERATION, "sRGB into RGB565");
}

/// ES 3.0 4.3.3: the source and destination rectangles of a resolve must be
/// the same region, not only the same size; the same region copies only it.
#[test]
fn region_mismatched_resolve_is_invalid_operation() {
    let _g = context_ms4();
    let dst = framebuffer(0, &[GL_RGBA8]).unwrap();
    let src = drawn_4x();
    assert_eq!(blit(src.fb, dst.fb, [0, 0, 4, 4], [4, 4, 8, 8]), GL_INVALID_OPERATION, "same size, moved");
    let e = blit_with(src.fb, dst.fb, [0, 0, 4, 4], [4, 4, 8, 8], GL_COLOR_BUFFER_BIT | 0x2, GL_NEAREST);
    assert_eq!(e, GL_INVALID_VALUE, "the mask outranks it");
    assert_eq!(PENDING.lock().unwrap().len(), 1, "a refused blit waits for nothing");
    assert!(pixels(dst.fb).iter().all(|p| *p == [0; 4]), "no copy");
    let region = [2, 1, 6, 5];
    assert_eq!(blit(src.fb, dst.fb, region, region), GL_NO_ERROR, "same region");
    for (i, p) in pixels(dst.fb).iter().enumerate() {
        let want = if inside(i, region) { RESOLVED } else { [0; 4] };
        assert_eq!(*p, want, "pixel ({}, {})", i % 8, i / 8);
    }
}

/// ES 3.0 4.3.3: a draw framebuffer with sample buffers is GL_INVALID_OPERATION,
/// from a single-sample read and from a 4x one, the argument errors
/// outranking it.
#[test]
fn blit_into_a_4x_draw_framebuffer_is_invalid_operation() {
    let _g = context_ms4();
    let ms = framebuffer(4, &[GL_RGBA8]).unwrap();
    let single = framebuffer(0, &[GL_RGBA8]).unwrap();
    let ms2 = framebuffer(4, &[GL_RGBA8]).unwrap();
    assert_eq!(blit(single.fb, ms.fb, FULL, FULL), GL_INVALID_OPERATION, "1x to 4x");
    assert_eq!(blit(ms2.fb, ms.fb, FULL, FULL), GL_INVALID_OPERATION, "4x to 4x");
    let e = blit_with(single.fb, ms.fb, FULL, FULL, GL_COLOR_BUFFER_BIT, GL_RGBA);
    assert_eq!(e, GL_INVALID_ENUM, "the filter outranks it");
}

/// Note (b)8: a 4x depth/stencil renderbuffer resolves through the existing
/// `depth_resolve` / `stencil_resolve` entries, one sample a pixel, into a
/// single-sample one through `ds_write`.
#[test]
fn depth_stencil_resolve_reads_one_sample_a_pixel() {
    let _g = context_on(&BACKEND_MS4_DS);
    let with_ds = |samples: GLsizei| {
        let mut fbo = framebuffer(samples, &[GL_RGBA8]).unwrap();
        let (e, rb) = storage(samples, GL_DEPTH24_STENCIL8);
        assert_eq!(e, GL_NO_ERROR);
        fbo.rbs.push(rb);
        // SAFETY: a renderbuffer this test made, on the bound framebuffer.
        unsafe { glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_DEPTH_STENCIL_ATTACHMENT, GL_RENDERBUFFER, rb) };
        assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE, "{samples} samples");
        fbo
    };
    let src = with_ds(4);
    let dst = with_ds(0);
    let ds = GL_DEPTH_BUFFER_BIT | GL_STENCIL_BUFFER_BIT;
    assert_eq!(blit_with(src.fb, dst.fb, FULL, [0, 0, 4, 4], ds, GL_NEAREST), GL_INVALID_OPERATION, "scaled");
    assert_eq!(blit_with(src.fb, dst.fb, FULL, FULL, ds, GL_LINEAR), GL_INVALID_OPERATION, "LINEAR");
    assert!(DS_READS.lock().unwrap().is_empty() && DS_WRITES.lock().unwrap().is_empty(), "no copy");
    assert_eq!(blit_with(src.fb, dst.fb, FULL, FULL, ds, GL_NEAREST), GL_NO_ERROR);
    let reads = DS_READS.lock().unwrap().clone();
    /* a write of no pixels is the "can it be written" probe (dst_writable) */
    let writes: Vec<_> = DS_WRITES.lock().unwrap().iter().copied().filter(|w| w.4 * w.5 != 0).collect();
    /* the copy reads the source's plane, and the destination's for the
     * pixels it keeps; every read is the whole 8x8 plane */
    assert!(reads.iter().all(|r| (r.1, r.2) == (8, 8)), "8x8: {reads:?}");
    let dst_id = writes.first().expect("a plane written").0;
    let src_ids: Vec<u32> = reads.iter().map(|r| r.0).filter(|&id| id != dst_id).collect();
    assert!(src_ids.len() == 2 && src_ids[0] == src_ids[1], "depth and stencil of the 4x object: {reads:?}");
    let src_id = src_ids[0];
    let mut planes: Vec<(u32, u32)> = writes.iter().map(|w| (w.1, w.6)).collect();
    planes.sort();
    assert_eq!(
        planes,
        vec![(backend::DS_PLANE_DEPTH, DEPTH.to_bits()), (backend::DS_PLANE_STENCIL, STENCIL as u32)],
        "{writes:?}"
    );
    for w in &writes {
        assert!(w.0 != src_id && (w.2, w.3, w.4, w.5) == (0, 0, 8, 8), "the whole plane of the other object: {w:?}");
    }
}
