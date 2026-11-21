// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! ES 3.0 multisample renderbuffers, the GL core's half, on the host:
//! clears, scissor, invalidate and attachment queries on a 4x framebuffer
//! object (docs/notes/es3-msaa-renderbuffers.md (a), (b);
//! docs/notes/v3d-msaa-es3.md (4)).
//!
//! The stand-in back end says `max_samples` 4 (copied from
//! tests/es3_msaa_gl.rs, which this file does not edit). Its `finish` stands
//! in for the job-end resolve: it writes the resolved colour into a 4x
//! target that was drawn, and not before.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_msaa_gl_clear
//! ```

#![cfg(all(feature = "host-test", feature = "shaderc-online", not(feature = "report-es2")))]
// The stand-in tables are copied from tests/es3_msaa_gl.rs; some pieces are unused here.
#![allow(dead_code)]

use core::ffi::{c_char, c_void};
use std::ffi::CString;
use std::sync::atomic::Ordering;
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{self, GlesDraw, GlesHeap, YakoglBackend};
use yakogl::gl::buffer::glBindBuffer;
use yakogl::gl::consts::*;
use yakogl::gl::draw::*;
use yakogl::gl::es3::{glClearBufferfi, glClearBufferfv, glClearBufferiv};
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

/* ---- clears ------------------------------------------------------------ */

/// The draws and clear-only kicks the back end saw since the last look.
fn take_seen() -> Vec<Seen> {
    core::mem::take(&mut *SEEN.lock().unwrap())
}

/// glFinish, which must leave no error.
fn finish() {
    // SAFETY: plain GL call.
    unsafe { glFinish() };
    assert_eq!(err(), GL_NO_ERROR);
}

/// Resolve the 4x `src` into a fresh 8x8 RGBA8 texture framebuffer and
/// return its pixels; the blit's error must be none.
fn resolved(src: &Fbo) -> Vec<[u8; 4]> {
    let tex = texture();
    let mut fb = 0;
    // SAFETY: plain GL calls on names made here.
    unsafe {
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex, 0);
    }
    assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE);
    assert_eq!(blit(src.fb, fb, FULL, FULL), GL_NO_ERROR);
    finish();
    let px = pixels(fb);
    // SAFETY: names made here.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glDeleteFramebuffers(1, &fb);
        glDeleteTextures(1, &tex);
    }
    px
}

/// glClear of the colour buffer of a complete 4x framebuffer is no error,
/// reaches the back end with 4 samples (a clear-only kick, no vertices), and
/// the resolve blit that follows sees the resolved image.
#[test]
fn glclear_on_4x_fbo_reaches_back_end_with_4_samples() {
    let _g = context_ms4();
    let src = framebuffer(4, &[GL_RGBA8]).unwrap();
    assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE);
    // SAFETY: plain GL calls.
    unsafe {
        glClearColor(1.0, 0.0, 0.0, 1.0);
        glClear(GL_COLOR_BUFFER_BIT);
    }
    assert_eq!(err(), GL_NO_ERROR);
    finish();
    let seen = take_seen();
    assert!(!seen.is_empty(), "the clear reached the back end");
    assert!(seen.iter().all(|s| !s.draws && s.samples == 4), "{seen:?}");
    let px = resolved(&src);
    assert!(px.iter().all(|p| *p == RESOLVED), "the resolve waited for the job: {px:?}");
}

/// glClearBufferfv on the colour buffer of a 4x framebuffer: the same.
#[test]
fn glclearbufferfv_on_4x_fbo() {
    let _g = context_ms4();
    let src = framebuffer(4, &[GL_RGBA8]).unwrap();
    let v = [0.0f32, 1.0, 0.0, 1.0];
    // SAFETY: four floats.
    unsafe { glClearBufferfv(GL_COLOR, 0, v.as_ptr()) };
    assert_eq!(err(), GL_NO_ERROR);
    finish();
    let seen = take_seen();
    assert!(!seen.is_empty() && seen.iter().all(|s| !s.draws && s.samples == 4), "{seen:?}");
    let px = resolved(&src);
    assert!(px.iter().all(|p| *p == RESOLVED), "{px:?}");
}

/// glClearBufferiv (stencil), glClearBufferfi and glClearBufferfv (depth)
/// on a 4x framebuffer with a 4x depth-stencil renderbuffer: no error, and
/// every kick the back end sees has 4 samples.
#[test]
fn glclearbuffer_depth_stencil_on_4x_fbo() {
    let _g = context_ms4();
    let src = framebuffer(4, &[GL_RGBA8]).unwrap();
    let (e, ds) = storage(4, GL_DEPTH24_STENCIL8);
    assert_eq!(e, GL_NO_ERROR);
    // SAFETY: plain GL calls on names made here; one GLint / one float.
    unsafe {
        glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_DEPTH_STENCIL_ATTACHMENT, GL_RENDERBUFFER, ds);
        assert_eq!(glGetError(), GL_NO_ERROR);
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE);
        let s: GLint = 7;
        glClearBufferiv(GL_STENCIL, 0, &s);
        assert_eq!(glGetError(), GL_NO_ERROR, "glClearBufferiv GL_STENCIL");
        glClearBufferfi(GL_DEPTH_STENCIL, 0, 0.5, 3);
        assert_eq!(glGetError(), GL_NO_ERROR, "glClearBufferfi");
        let d = 0.25f32;
        glClearBufferfv(GL_DEPTH, 0, &d);
        assert_eq!(glGetError(), GL_NO_ERROR, "glClearBufferfv GL_DEPTH");
        glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_DEPTH_STENCIL_ATTACHMENT, GL_RENDERBUFFER, 0);
        glDeleteRenderbuffers(1, &ds);
    }
    finish();
    let seen = take_seen();
    assert!(seen.iter().all(|s| !s.draws && s.samples == 4), "{seen:?}");
    drop(src);
}

/// A scissored glClear of a 4x framebuffer: no error, the kick has 4
/// samples, and the resolve still follows. (The stand-in records the samples
/// only, not the scissor rectangle.)
#[test]
fn scissored_clear_on_4x_fbo() {
    let _g = context_ms4();
    let src = framebuffer(4, &[GL_RGBA8]).unwrap();
    // SAFETY: plain GL calls.
    let e = unsafe {
        glEnable(GL_SCISSOR_TEST);
        glScissor(2, 2, 3, 3);
        glClearColor(0.0, 0.0, 1.0, 1.0);
        glClear(GL_COLOR_BUFFER_BIT);
        let e = glGetError();
        glDisable(GL_SCISSOR_TEST);
        e
    };
    assert_eq!(e, GL_NO_ERROR);
    finish();
    let seen = take_seen();
    assert!(!seen.is_empty() && seen.iter().all(|s| !s.draws && s.samples == 4), "{seen:?}");
    let px = resolved(&src);
    assert!(px.iter().all(|p| *p == RESOLVED), "{px:?}");
}

/* ---- invalidate -------------------------------------------------------- */

/// glInvalidateFramebuffer / glInvalidateSubFramebuffer on a 4x framebuffer:
/// no error for the colour point, a point the framebuffer does not have is
/// ignored (ES 3.0.6 4.5), a negative size is GL_INVALID_VALUE, a bad enum is
/// GL_INVALID_ENUM. The stand-in has no `attachment_gone`, so the back end is
/// told nothing and the 4x framebuffer still resolves afterwards.
#[test]
fn invalidate_on_4x_fbo() {
    let _g = context_ms4();
    let src = drawn_4x();
    let a = [GL_COLOR_ATTACHMENT0];
    let d = [GL_DEPTH_ATTACHMENT];
    let bad = [GL_RGBA8];
    // SAFETY: plain GL calls; arrays of enums.
    unsafe {
        glInvalidateFramebuffer(GL_FRAMEBUFFER, 1, a.as_ptr());
        assert_eq!(glGetError(), GL_NO_ERROR);
        glInvalidateSubFramebuffer(GL_FRAMEBUFFER, 1, a.as_ptr(), 1, 1, 4, 4);
        assert_eq!(glGetError(), GL_NO_ERROR);
        glInvalidateSubFramebuffer(GL_FRAMEBUFFER, 1, a.as_ptr(), 0, 0, -1, 4);
        assert_eq!(glGetError(), GL_INVALID_VALUE);
        glInvalidateFramebuffer(GL_FRAMEBUFFER, 1, d.as_ptr());
        assert_eq!(glGetError(), GL_NO_ERROR);
        glInvalidateFramebuffer(GL_FRAMEBUFFER, 1, bad.as_ptr());
        assert_eq!(glGetError(), GL_INVALID_ENUM);
    }
    finish();
    let px = resolved(&src);
    assert!(px.iter().all(|p| *p == RESOLVED), "{px:?}");
}

/* ---- queries ----------------------------------------------------------- */

/// GL_RENDERBUFFER_SAMPLES follows each respecification: 4, then 0, then 4.
#[test]
fn renderbuffer_samples_follow_respecification() {
    let _g = context_ms4();
    let (e, rb) = storage(4, GL_RGBA8);
    assert_eq!(e, GL_NO_ERROR);
    assert_eq!(rb_samples(rb), 4);
    // SAFETY: plain GL call on the bound renderbuffer.
    unsafe { glRenderbufferStorageMultisample(GL_RENDERBUFFER, 0, GL_RGBA8, 8, 8) };
    assert_eq!(err(), GL_NO_ERROR);
    assert_eq!(rb_samples(rb), 0);
    // SAFETY: as above.
    unsafe { glRenderbufferStorageMultisample(GL_RENDERBUFFER, 4, GL_RGBA8, 8, 8) };
    assert_eq!(err(), GL_NO_ERROR);
    assert_eq!(rb_samples(rb), 4);
    // SAFETY: a name made here.
    unsafe { glDeleteRenderbuffers(1, &rb) };
}

/// glGetFramebufferAttachmentParameteriv on a 4x renderbuffer attachment:
/// type GL_RENDERBUFFER, its name, the RGBA8 component sizes, and the texture
/// level / cube face queries are GL_INVALID_ENUM (ES 3.0.6 6.1.13).
#[test]
fn attachment_parameters_of_4x_attachment() {
    let _g = context_ms4();
    let fbo = framebuffer(4, &[GL_RGBA8]).unwrap();
    let get = |pname: GLenum| -> (GLint, GLenum) {
        let mut v: GLint = -777;
        // SAFETY: one GLint out.
        unsafe { glGetFramebufferAttachmentParameteriv(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, pname, &mut v) };
        (v, err())
    };
    assert_eq!(get(GL_FRAMEBUFFER_ATTACHMENT_OBJECT_TYPE), (GL_RENDERBUFFER as GLint, GL_NO_ERROR));
    assert_eq!(get(GL_FRAMEBUFFER_ATTACHMENT_OBJECT_NAME), (fbo.rbs[0] as GLint, GL_NO_ERROR));
    assert_eq!(get(GL_FRAMEBUFFER_ATTACHMENT_RED_SIZE), (8, GL_NO_ERROR));
    assert_eq!(get(GL_FRAMEBUFFER_ATTACHMENT_ALPHA_SIZE), (8, GL_NO_ERROR));
    assert_eq!(get(GL_FRAMEBUFFER_ATTACHMENT_COMPONENT_TYPE), (GL_UNSIGNED_NORMALIZED as GLint, GL_NO_ERROR));
    assert_eq!(get(GL_FRAMEBUFFER_ATTACHMENT_TEXTURE_LEVEL).1, GL_INVALID_ENUM);
    assert_eq!(get(GL_FRAMEBUFFER_ATTACHMENT_TEXTURE_CUBE_MAP_FACE).1, GL_INVALID_ENUM);
}

/// GL_SAMPLES and GL_SAMPLE_BUFFERS: 4 and 1 with a complete 4x framebuffer
/// bound; 0 and 0 with the window bound (a 1x window here) and with a
/// 0-sample renderbuffer framebuffer.
#[test]
fn samples_and_sample_buffers_by_binding() {
    let _g = context_ms4();
    assert_eq!((geti(GL_SAMPLE_BUFFERS), geti(GL_SAMPLES)), (0, 0), "window");
    let fbo = framebuffer(4, &[GL_RGBA8]).unwrap();
    assert_eq!((geti(GL_SAMPLE_BUFFERS), geti(GL_SAMPLES)), (1, 4), "4x");
    drop(fbo);
    assert_eq!((geti(GL_SAMPLE_BUFFERS), geti(GL_SAMPLES)), (0, 0), "window again");
    let fbo = framebuffer(0, &[GL_RGBA8]).unwrap();
    assert_eq!((geti(GL_SAMPLE_BUFFERS), geti(GL_SAMPLES)), (0, 0), "0 samples");
    drop(fbo);
}

/// ES 3.0.6 4.3.2: glReadPixels from a framebuffer whose GL_SAMPLE_BUFFERS
/// is 1 is GL_INVALID_OPERATION.
#[test]
fn readpixels_from_4x_read_framebuffer_is_invalid_operation() {
    let _g = context_ms4();
    let fbo = framebuffer(4, &[GL_RGBA8]).unwrap();
    let mut px = [0u8; 4];
    // SAFETY: room for one pixel.
    unsafe {
        glBindFramebuffer(GL_READ_FRAMEBUFFER, fbo.fb);
        glReadBuffer(GL_COLOR_ATTACHMENT0);
        while glGetError() != GL_NO_ERROR {}
        glReadPixels(0, 0, 1, 1, GL_RGBA, GL_UNSIGNED_BYTE, px.as_mut_ptr() as *mut c_void);
    }
    assert_eq!(err(), GL_INVALID_OPERATION);
}

/// Deleting a 4x renderbuffer that is still attached to the bound
/// framebuffer detaches it (ES 3.0.6 4.4.2.1): the attachment reads as
/// GL_NONE, the framebuffer is GL_FRAMEBUFFER_INCOMPLETE_MISSING_ATTACHMENT,
/// and the sample queries fall back to 0.
#[test]
fn delete_attached_4x_renderbuffer() {
    let _g = context_ms4();
    let fbo = framebuffer(4, &[GL_RGBA8]).unwrap();
    assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE);
    // SAFETY: a name made here.
    unsafe { glDeleteRenderbuffers(1, &fbo.rbs[0]) };
    assert_eq!(err(), GL_NO_ERROR);
    let mut v: GLint = -777;
    // SAFETY: one GLint out.
    unsafe {
        glGetFramebufferAttachmentParameteriv(
            GL_FRAMEBUFFER,
            GL_COLOR_ATTACHMENT0,
            GL_FRAMEBUFFER_ATTACHMENT_OBJECT_TYPE,
            &mut v,
        )
    };
    assert_eq!((v, err()), (GL_NONE as GLint, GL_NO_ERROR));
    assert_eq!(status(), GL_FRAMEBUFFER_INCOMPLETE_MISSING_ATTACHMENT);
    assert_eq!((geti(GL_SAMPLE_BUFFERS), geti(GL_SAMPLES)), (0, 0));
}
