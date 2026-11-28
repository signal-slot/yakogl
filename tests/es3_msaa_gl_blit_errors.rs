// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! ES 3.0.6 4.3.3 glBlitFramebuffer: one table of every error a resolve blit
//! can raise, on the 4x stand-in back end of tests/es3_msaa_gl.rs (its helpers
//! are copied here). A row is a read setup, a draw setup, the blit arguments
//! and the error the spec gives; after every row the destination is checked
//! unchanged (a colour destination by its pixels, every destination by "the
//! back end was handed no depth/stencil write"), and the one valid resolve
//! row checks the destination did change to the resolved colour.
//!
//! Spec text, 4.3.3 (ES 3.0.6): "If SAMPLE_BUFFERS for the draw framebuffer
//! is greater than zero, an INVALID_OPERATION error is generated"; "if
//! SAMPLE_BUFFERS for the read framebuffer is greater than zero and the draw
//! framebuffer's is zero, ... the dimensions of the source and destination
//! rectangles ... must be identical, and the formats of the read and draw
//! buffers must match, else INVALID_OPERATION"; a LINEAR filter with a
//! depth or stencil mask is INVALID_OPERATION; unknown mask bits are
//! INVALID_VALUE; an incomplete read or draw framebuffer is
//! INVALID_FRAMEBUFFER_OPERATION; depth/stencil formats that differ are
//! INVALID_OPERATION. Mirroring: a flip on both sides leaves the rectangles
//! identical (valid); a flip on one side makes them differ (error), which is
//! also the rule of src/gl/blit.rs (`k.src != k.dst`).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_msaa_gl_blit_errors
//! ```

#![cfg(all(feature = "host-test", feature = "shaderc-online", not(feature = "report-es2")))]
#![allow(dead_code)]

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
use yakogl::gl::texture::{glBindTexture, glGenTextures, glTexImage2D};
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

/* ---- the table ---------------------------------------------------------- */

/// One framebuffer of a row: renderbuffers only. `colour` is (samples,
/// format; samples 0 is a single-sample buffer, 1 would round up to 4) on attachment 0, `ds` is (samples, format, attachment point). Both
/// `None` is an empty framebuffer: incomplete (no attachment).
#[derive(Clone, Copy)]
struct Setup {
    colour: Option<(GLsizei, GLenum)>,
    ds: Option<(GLsizei, GLenum, GLenum)>,
}

const EMPTY: Setup = Setup { colour: None, ds: None };
const MS4: Setup = Setup { colour: Some((4, GL_RGBA8)), ds: None };
const ONE: Setup = Setup { colour: Some((0, GL_RGBA8)), ds: None };

const fn ds(samples: GLsizei, format: GLenum, point: GLenum) -> Setup {
    Setup { colour: None, ds: Some((samples, format, point)) }
}

struct Row {
    name: &'static str,
    read: Setup,
    draw: Setup,
    src: [GLint; 4],
    dst: [GLint; 4],
    mask: GLbitfield,
    filter: GLenum,
    expect: GLenum,
}

const FULL: [GLint; 4] = [0, 0, 8, 8];
const COLOUR: GLbitfield = GL_COLOR_BUFFER_BIT;

fn row(name: &'static str, read: Setup, draw: Setup, src: [GLint; 4], dst: [GLint; 4], expect: GLenum) -> Row {
    Row { name, read, draw, src, dst, mask: COLOUR, filter: GL_NEAREST, expect }
}

fn build(s: Setup) -> Fbo {
    let mut fb = 0;
    // SAFETY: one name out; plain GL call.
    unsafe {
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
    }
    let mut fbo = Fbo { fb, rbs: Vec::new() };
    if let Some((n, f)) = s.colour {
        let (e, rb) = storage(n, f);
        fbo.rbs.push(rb);
        assert_eq!(e, GL_NO_ERROR);
        // SAFETY: a renderbuffer this test made.
        unsafe { glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_RENDERBUFFER, rb) };
    }
    if let Some((n, f, point)) = s.ds {
        let (e, rb) = storage(n, f);
        fbo.rbs.push(rb);
        assert_eq!(e, GL_NO_ERROR);
        // SAFETY: a renderbuffer this test made.
        unsafe { glFramebufferRenderbuffer(GL_FRAMEBUFFER, point, GL_RENDERBUFFER, rb) };
    }
    assert_eq!(err(), GL_NO_ERROR);
    fbo
}

/// A 4x read framebuffer is drawn into (its job pending) so the resolve has a
/// source; a 1x colour destination is cleared to a colour the resolve is not.
fn prepare(s: Setup, f: &Fbo, is_read: bool) {
    // SAFETY: plain GL calls on a framebuffer this test made.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, f.fb);
        if s.colour.is_some() && is_read && s.colour.unwrap().0 > 0 {
            program();
            draw_unfinished();
        } else if s.colour.is_some() && !is_read {
            glClearColor(0.9, 0.1, 0.9, 0.5);
            glClear(GL_COLOR_BUFFER_BIT);
        }
        glUseProgram(0);
    }
    /* an incomplete framebuffer refuses the draw and the clear itself */
    while err() != GL_NO_ERROR {}
}

/// The destination's RGBA/UNSIGNED_BYTE pixels, when it is a colour one.
fn dst_pixels(s: Setup, f: &Fbo) -> Option<Vec<u8>> {
    if s.colour?.0 != 0 {
        return None; /* glReadPixels of a 4x framebuffer is itself an error */
    }
    let mut px = vec![0x11u8; 8 * 8 * 4];
    // SAFETY: room for 8x8 RGBA/UNSIGNED_BYTE pixels.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, f.fb);
        glReadBuffer(GL_COLOR_ATTACHMENT0);
        glReadPixels(0, 0, 8, 8, GL_RGBA, GL_UNSIGNED_BYTE, px.as_mut_ptr() as *mut c_void);
    }
    assert_eq!(err(), GL_NO_ERROR);
    Some(px)
}

fn run(r: &Row) {
    let _g = context_on(&BACKEND_MS4_DS);
    let read = build(r.read);
    let draw_fbo = build(r.draw);
    prepare(r.read, &read, true);
    prepare(r.draw, &draw_fbo, false);
    let before = dst_pixels(r.draw, &draw_fbo);
    DS_WRITES.lock().unwrap().clear();
    let e = blit_with(read.fb, draw_fbo.fb, r.src, r.dst, r.mask, r.filter);
    assert_eq!(e, r.expect, "{}: glBlitFramebuffer error", r.name);
    // SAFETY: plain GL call.
    unsafe { glFinish() };
    assert_eq!(err(), GL_NO_ERROR);
    let after = dst_pixels(r.draw, &draw_fbo);
    if r.expect == GL_NO_ERROR {
        let after = after.expect("a colour destination");
        assert!(after.chunks(4).all(|p| p == RESOLVED), "{}: the destination holds the resolved colour", r.name);
    } else {
        assert_eq!(before, after, "{}: the destination is unchanged", r.name);
        assert!(DS_WRITES.lock().unwrap().is_empty(), "{}: no depth/stencil written", r.name);
    }
}

fn run_all(rows: Vec<Row>) {
    for r in &rows {
        run(r);
    }
}

#[test]
fn a_valid_same_size_resolve_is_no_error() {
    run_all(vec![
        row("4x to 1x, same rectangle", MS4, ONE, FULL, FULL, GL_NO_ERROR),
        row("4x to 1x, mirrored on both sides", MS4, ONE, [8, 8, 0, 0], [8, 8, 0, 0], GL_NO_ERROR),
    ]);
}

#[test]
fn a_draw_framebuffer_with_sample_buffers_is_invalid_operation() {
    run_all(vec![
        row("4x to 4x", MS4, MS4, FULL, FULL, GL_INVALID_OPERATION),
        row("1x to 4x", ONE, MS4, FULL, FULL, GL_INVALID_OPERATION),
        row("1x to 4x, scaled", ONE, MS4, FULL, [0, 0, 4, 4], GL_INVALID_OPERATION),
    ]);
}

#[test]
fn a_resolve_between_different_rectangles_is_invalid_operation() {
    run_all(vec![
        row("scaled up", MS4, ONE, [0, 0, 4, 4], FULL, GL_INVALID_OPERATION),
        row("scaled down", MS4, ONE, FULL, [0, 0, 4, 4], GL_INVALID_OPERATION),
        row("offset", MS4, ONE, [0, 0, 4, 4], [2, 2, 6, 6], GL_INVALID_OPERATION),
        row("scaled in x only", MS4, ONE, FULL, [0, 0, 4, 8], GL_INVALID_OPERATION),
        row("mirrored in x on the destination only", MS4, ONE, FULL, [8, 0, 0, 8], GL_INVALID_OPERATION),
        row("mirrored in y on the source only", MS4, ONE, [0, 8, 8, 0], FULL, GL_INVALID_OPERATION),
        row("mirrored in x and y on the destination only", MS4, ONE, FULL, [8, 8, 0, 0], GL_INVALID_OPERATION),
    ]);
}

#[test]
fn a_resolve_into_another_format_is_invalid_operation() {
    let rgb565 = Setup { colour: Some((0, GL_RGB565)), ds: None };
    let rgba4 = Setup { colour: Some((0, GL_RGBA4)), ds: None };
    let rgba8_ms = MS4;
    run_all(vec![
        row("RGBA8 into RGB565", rgba8_ms, rgb565, FULL, FULL, GL_INVALID_OPERATION),
        row("RGBA8 into RGBA4", rgba8_ms, rgba4, FULL, FULL, GL_INVALID_OPERATION),
        row(
            "RGB565 into RGBA8",
            Setup { colour: Some((4, GL_RGB565)), ds: None },
            ONE,
            FULL,
            FULL,
            GL_INVALID_OPERATION,
        ),
    ]);
}

#[test]
fn a_linear_filter_with_depth_or_stencil_is_invalid_operation() {
    let mut rows = Vec::new();
    for (name, mask) in [
        ("depth", GL_DEPTH_BUFFER_BIT),
        ("stencil", GL_STENCIL_BUFFER_BIT),
        ("colour and depth", COLOUR | GL_DEPTH_BUFFER_BIT),
        ("colour and stencil", COLOUR | GL_STENCIL_BUFFER_BIT),
    ] {
        rows.push(Row { mask, filter: GL_LINEAR, ..row(name, MS4, ONE, FULL, FULL, GL_INVALID_OPERATION) });
        rows.push(Row { mask, filter: GL_LINEAR, ..row(name, ONE, ONE, FULL, [0, 0, 4, 4], GL_INVALID_OPERATION) });
    }
    run_all(rows);
}

#[test]
fn a_mask_with_unknown_bits_is_invalid_value() {
    let mut rows = Vec::new();
    for (name, mask) in [
        ("0x8 alone", 0x8),
        ("colour and 0x8", COLOUR | 0x8),
        ("colour and 0x1", COLOUR | 0x1),
        ("0x80000000", 0x8000_0000),
        ("every bit", !0),
    ] {
        rows.push(Row { mask, ..row(name, MS4, ONE, FULL, FULL, GL_INVALID_VALUE) });
    }
    run_all(rows);
}

#[test]
fn an_incomplete_framebuffer_is_invalid_framebuffer_operation() {
    /* a 4x colour next to a 1x depth: GL_FRAMEBUFFER_INCOMPLETE_MULTISAMPLE */
    let mixed = Setup { colour: Some((4, GL_RGBA8)), ds: Some((0, GL_DEPTH_COMPONENT16, GL_DEPTH_ATTACHMENT)) };
    run_all(vec![
        row("empty read", EMPTY, ONE, FULL, FULL, GL_INVALID_FRAMEBUFFER_OPERATION),
        row("empty draw", MS4, EMPTY, FULL, FULL, GL_INVALID_FRAMEBUFFER_OPERATION),
        row("both empty", EMPTY, EMPTY, FULL, FULL, GL_INVALID_FRAMEBUFFER_OPERATION),
        row("read with mixed sample counts", mixed, ONE, FULL, FULL, GL_INVALID_FRAMEBUFFER_OPERATION),
        row("draw with mixed sample counts", MS4, mixed, FULL, FULL, GL_INVALID_FRAMEBUFFER_OPERATION),
        Row {
            filter: GL_LINEAR,
            mask: GL_DEPTH_BUFFER_BIT,
            ..row("incomplete before the LINEAR check", EMPTY, ONE, FULL, FULL, GL_INVALID_FRAMEBUFFER_OPERATION)
        },
    ]);
}

#[test]
fn depth_and_stencil_formats_that_do_not_match_are_invalid_operation() {
    let d = |n, f| ds(n, f, GL_DEPTH_ATTACHMENT);
    let ds_ = |n, f| ds(n, f, GL_DEPTH_STENCIL_ATTACHMENT);
    let depth = GL_DEPTH_BUFFER_BIT;
    let stencil = GL_STENCIL_BUFFER_BIT;
    run_all(vec![
        Row {
            mask: depth,
            ..row(
                "D16 into D24",
                d(0, GL_DEPTH_COMPONENT16),
                d(0, GL_DEPTH_COMPONENT24),
                FULL,
                FULL,
                GL_INVALID_OPERATION,
            )
        },
        Row {
            mask: depth,
            ..row(
                "D24 into D32F",
                d(0, GL_DEPTH_COMPONENT24),
                d(0, GL_DEPTH_COMPONENT32F),
                FULL,
                FULL,
                GL_INVALID_OPERATION,
            )
        },
        Row {
            mask: depth,
            ..row(
                "D24S8 into D16",
                ds_(0, GL_DEPTH24_STENCIL8),
                d(0, GL_DEPTH_COMPONENT16),
                FULL,
                FULL,
                GL_INVALID_OPERATION,
            )
        },
        Row {
            mask: depth,
            ..row(
                "D24S8 into D32FS8",
                ds_(0, GL_DEPTH24_STENCIL8),
                ds_(0, GL_DEPTH32F_STENCIL8),
                FULL,
                FULL,
                GL_INVALID_OPERATION,
            )
        },
        Row {
            mask: depth | stencil,
            ..row(
                "D24S8 into D32FS8, both",
                ds_(0, GL_DEPTH24_STENCIL8),
                ds_(0, GL_DEPTH32F_STENCIL8),
                FULL,
                FULL,
                GL_INVALID_OPERATION,
            )
        },
        Row {
            mask: depth,
            ..row(
                "4x D16 into D24",
                d(4, GL_DEPTH_COMPONENT16),
                d(0, GL_DEPTH_COMPONENT24),
                FULL,
                FULL,
                GL_INVALID_OPERATION,
            )
        },
        Row {
            mask: depth,
            ..row(
                "4x D24S8 into D32FS8",
                ds_(4, GL_DEPTH24_STENCIL8),
                ds_(0, GL_DEPTH32F_STENCIL8),
                FULL,
                FULL,
                GL_INVALID_OPERATION,
            )
        },
    ]);
}
