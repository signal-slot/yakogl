// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GlesDraw sample counts across binding switches and redraws after a resolve,
//! on the 4x stand-in (docs/notes/es3-msaa-renderbuffers.md (b)4, (b)5, (b)7).
//!
//! Helpers are copied from tests/es3_msaa_gl.rs (its header says what the
//! stand-in is). Here the stand-in also records the `samples` of every
//! application `GlesDraw` it receives in `ALL`. `GlesDraw::target_id` /
//! `target_gen` do not exist yet and are not asserted.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_msaa_gl_draws
//! ```

#![allow(dead_code)]
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

/// `samples` of every application draw the back end received, in order.
static ALL: Mutex<Vec<u8>> = Mutex::new(Vec::new());

static SEEN: Mutex<Vec<Seen>> = Mutex::new(Vec::new());

/// The 4x targets drawn since the last `finish`: (`target_gpu`, w, h).
static PENDING: Mutex<Vec<(u64, u32, u32)>> = Mutex::new(Vec::new());

/// The colour the stand-in's job-end resolve leaves in an RGBA8 target.
const RESOLVED: [u8; 4] = [0x40, 0x80, 0xc0, 0xff];

unsafe extern "C" fn b_draw(d: *const GlesDraw) -> i32 {
    let d = &*d;
    SEEN.lock().unwrap().push(Seen { draws: d.index_count != 0, samples: d.samples });
    if d.index_count != 0 {
        ALL.lock().unwrap().push(d.samples);
    }
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
    ALL.lock().unwrap().clear();
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

/// Bind `fb` and draw one triangle with the current program: the samples the
/// back end was handed for it.
fn draw_on(fb: GLuint) -> u8 {
    // SAFETY: a framebuffer name this test made, or 0.
    unsafe { glBindFramebuffer(GL_FRAMEBUFFER, fb) };
    draw().samples
}

/* ---- the draws ------------------------------------------------------------ */

/// 4x FBO, 1x FBO, the window, the 4x FBO again: the samples follow the
/// binding each time, nothing sticks from the draw before.
#[test]
fn samples_follow_the_binding() {
    let _g = context_ms4();
    program();
    let window = draw_on(0);
    let ms = framebuffer(4, &[GL_RGBA8]).unwrap();
    let one = framebuffer(0, &[GL_RGBA8]).unwrap();
    assert_eq!(draw_on(ms.fb), 4, "4x FBO");
    assert_eq!(draw_on(one.fb), 1, "1x FBO");
    assert_eq!(draw_on(0), window, "the window, its own count");
    assert_eq!(draw_on(ms.fb), 4, "the 4x FBO again");
    assert_eq!(draw_on(one.fb), 1, "the 1x FBO again");
    assert_eq!(ALL.lock().unwrap().as_slice(), &[window, 4, 1, window, 4, 1][..], "the back end saw them in order");
}

/// Only the 4x draws leave a job to resolve: the 1x and window draws add none.
#[test]
fn only_four_x_draws_are_pending() {
    let _g = context_ms4();
    program();
    let ms = framebuffer(4, &[GL_RGBA8]).unwrap();
    let one = framebuffer(0, &[GL_RGBA8]).unwrap();
    for fb in [one.fb, 0, ms.fb, one.fb, 0] {
        // SAFETY: names this test made.
        unsafe { glBindFramebuffer(GL_FRAMEBUFFER, fb) };
        draw_unfinished();
    }
    assert_eq!(PENDING.lock().unwrap().len(), 1);
}

/// Draw into 4x, resolve, draw into the same 4x FBO again, resolve again:
/// both draws are 4x, each resolve waits for the job before it, and the
/// second one's pixels come from the second job.
#[test]
fn redraw_after_a_resolve() {
    let _g = context_ms4();
    let dst1 = framebuffer(0, &[GL_RGBA8]).unwrap();
    let dst2 = framebuffer(0, &[GL_RGBA8]).unwrap();
    let src = framebuffer(4, &[GL_RGBA8]).unwrap();
    program();
    draw_unfinished();
    assert_eq!(PENDING.lock().unwrap().len(), 1);
    assert_eq!(blit(src.fb, dst1.fb, FULL, FULL), GL_NO_ERROR);
    assert!(PENDING.lock().unwrap().is_empty(), "the first resolve waited");
    assert!(pixels(dst1.fb).iter().all(|p| *p == RESOLVED), "first resolve");
    // SAFETY: a name this test made.
    unsafe { glBindFramebuffer(GL_FRAMEBUFFER, src.fb) };
    draw_unfinished();
    assert_eq!(err(), GL_NO_ERROR);
    assert_eq!(PENDING.lock().unwrap().len(), 1, "the second draw is a 4x job again");
    assert_eq!(*ALL.lock().unwrap().last().unwrap(), 4);
    assert!(pixels(dst2.fb).iter().all(|p| *p == [0; 4]), "nothing resolved into dst2 yet");
    assert_eq!(blit(src.fb, dst2.fb, FULL, FULL), GL_NO_ERROR);
    assert!(PENDING.lock().unwrap().is_empty(), "the second resolve waited for the second job");
    assert!(pixels(dst2.fb).iter().all(|p| *p == RESOLVED), "second resolve");
    assert!(pixels(dst1.fb).iter().all(|p| *p == RESOLVED), "the first copy stays");
}

/// A partial resolve after a redraw copies only its rectangle (the stand-in
/// runs no clears, so the destination is a fresh one holding zeros).
#[test]
fn partial_resolve_after_a_redraw() {
    let _g = context_ms4();
    let dst = framebuffer(0, &[GL_RGBA8]).unwrap();
    let src = drawn_4x();
    assert_eq!(blit(src.fb, src.fb, FULL, FULL), GL_INVALID_OPERATION, "a 4x draw target");
    // SAFETY: a name this test made.
    unsafe { glBindFramebuffer(GL_FRAMEBUFFER, src.fb) };
    draw_unfinished();
    let r = [2, 2, 6, 6];
    assert_eq!(blit(src.fb, dst.fb, r, r), GL_NO_ERROR);
    for (i, p) in pixels(dst.fb).iter().enumerate() {
        let want = if inside(i, r) { RESOLVED } else { [0; 4] };
        assert_eq!(*p, want, "pixel {i}");
    }
}

/// glRenderbufferStorageMultisample on the attached renderbuffer between
/// draws: the FBO keeps drawing with 4 samples after a respecification of
/// the same count, a request of 1 (stored as 4), and another format.
#[test]
fn respecify_between_draws() {
    let _g = context_ms4();
    let fbo = framebuffer(4, &[GL_RGBA8]).unwrap();
    program();
    assert_eq!(draw().samples, 4);
    // SAFETY: plain GL calls on the attached renderbuffer.
    unsafe {
        glBindRenderbuffer(GL_RENDERBUFFER, fbo.rbs[0]);
        glRenderbufferStorageMultisample(GL_RENDERBUFFER, 4, GL_RGBA8, 8, 8);
    }
    assert_eq!(err(), GL_NO_ERROR);
    assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE);
    assert_eq!(draw().samples, 4, "same storage again");
    // SAFETY: as above.
    unsafe { glRenderbufferStorageMultisample(GL_RENDERBUFFER, 1, GL_RGBA8, 8, 8) };
    assert_eq!(err(), GL_NO_ERROR);
    assert_eq!(draw().samples, 4, "a request of 1 stores 4");
    // SAFETY: as above.
    unsafe { glRenderbufferStorageMultisample(GL_RENDERBUFFER, 4, GL_RGBA4, 8, 8) };
    assert_eq!(err(), GL_NO_ERROR);
    assert_eq!(draw().samples, 4, "another format");
}

/// Respecifying the attachment to 0 samples makes the FBO a 1x one: the next
/// draw is single-sample; back to 4 it is 4x again.
#[test]
fn respecify_to_single_and_back() {
    let _g = context_ms4();
    let fbo = framebuffer(4, &[GL_RGBA8]).unwrap();
    program();
    assert_eq!(draw().samples, 4);
    // SAFETY: plain GL calls on the attached renderbuffer.
    unsafe {
        glBindRenderbuffer(GL_RENDERBUFFER, fbo.rbs[0]);
        glRenderbufferStorageMultisample(GL_RENDERBUFFER, 0, GL_RGBA8, 8, 8);
    }
    assert_eq!(err(), GL_NO_ERROR);
    assert_eq!(draw().samples, 1, "0 samples: single");
    // SAFETY: as above.
    unsafe { glRenderbufferStorageMultisample(GL_RENDERBUFFER, 4, GL_RGBA8, 8, 8) };
    assert_eq!(err(), GL_NO_ERROR);
    assert_eq!(draw().samples, 4, "4 again");
}

/// A respecified renderbuffer resolves as the new storage: draw, respecify
/// to 4x, draw, resolve.
#[test]
fn resolve_after_respecify() {
    let _g = context_ms4();
    let dst = framebuffer(0, &[GL_RGBA8]).unwrap();
    let src = drawn_4x();
    // SAFETY: plain GL calls on names this test made.
    unsafe {
        glBindRenderbuffer(GL_RENDERBUFFER, src.rbs[0]);
        glRenderbufferStorageMultisample(GL_RENDERBUFFER, 4, GL_RGBA8, 8, 8);
        glBindFramebuffer(GL_FRAMEBUFFER, src.fb);
    }
    assert_eq!(err(), GL_NO_ERROR);
    draw_unfinished();
    assert_eq!(blit(src.fb, dst.fb, FULL, FULL), GL_NO_ERROR);
    assert!(pixels(dst.fb).iter().all(|p| *p == RESOLVED));
}

/// A draw with the 4x FBO incomplete (a texture next to the renderbuffer)
/// is GL_INVALID_FRAMEBUFFER_OPERATION and never reaches the back end; once
/// complete again the draw goes through with 4 samples.
#[test]
fn incomplete_draw_never_reaches_the_back_end() {
    let _g = context_ms4();
    let fbo = framebuffer(4, &[GL_RGBA8]).unwrap();
    program();
    let tex = texture();
    // SAFETY: plain GL calls on names this test made.
    unsafe { glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT1, GL_TEXTURE_2D, tex, 0) };
    assert_eq!(status(), GL_FRAMEBUFFER_INCOMPLETE_MULTISAMPLE);
    ALL.lock().unwrap().clear();
    draw_unfinished();
    assert_eq!(err(), GL_INVALID_FRAMEBUFFER_OPERATION);
    // SAFETY: plain GL call.
    unsafe { glFinish() };
    assert!(ALL.lock().unwrap().is_empty(), "the back end got no draw");
    assert!(PENDING.lock().unwrap().is_empty());
    // SAFETY: plain GL calls.
    unsafe {
        glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT1, GL_TEXTURE_2D, 0, 0);
        glDeleteTextures(1, &tex);
    }
    assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE);
    assert_eq!(draw().samples, 4);
    drop(fbo);
}

/// Mixed counts on two colour attachments (4 and 0) are incomplete too: the
/// draw is refused, then reaches the back end once the counts match.
#[test]
fn mixed_counts_draw_is_refused() {
    let _g = context_ms4();
    let fbo = framebuffer(4, &[GL_RGBA8]).unwrap();
    program();
    let (_, rb0) = storage(0, GL_RGBA8);
    // SAFETY: plain GL call on a name this test made.
    unsafe { glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT1, GL_RENDERBUFFER, rb0) };
    assert_eq!(status(), GL_FRAMEBUFFER_INCOMPLETE_MULTISAMPLE);
    draw_unfinished();
    assert_eq!(err(), GL_INVALID_FRAMEBUFFER_OPERATION);
    // SAFETY: plain GL call.
    unsafe { glFinish() };
    assert!(ALL.lock().unwrap().is_empty());
    // SAFETY: names this test made.
    unsafe {
        glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT1, GL_RENDERBUFFER, 0);
        glDeleteRenderbuffers(1, &rb0);
    }
    assert_eq!(draw().samples, 4);
    drop(fbo);
}
