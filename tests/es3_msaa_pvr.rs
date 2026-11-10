// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! 4x multisample renderbuffers on the PowerVR (BXS-4-64, AM62P), through the
//! GL API (msaa-pin-pvr-file), on the host.
//!
//! docs/notes/es3-msaa-renderbuffers.md (0), (b) and (c1): the GL core never
//! makes a 4x framebuffer object. `GL_MAX_SAMPLES` is 1,
//! `glRenderbufferStorageMultisample` gives `GL_INVALID_OPERATION` for 2 or
//! more samples, one sample is plain storage with `GL_RENDERBUFFER_SAMPLES` 1,
//! and a draw into it reaches the back end with `GlesDraw::samples` 1. Only the
//! window is 4x (`default_samples`, the AM62P's ms4 image).
//!
//! The GL core is driven through the API into a stand-in back end that records
//! what it is handed (a draw's samples and depth identity, `attachment_gone`)
//! and fills no pixels: the PowerVR's own draw needs the board's job memory.
//!
//! Passing tests pin today. The `#[ignore]`d ones are the expected result
//! after the note's design, each naming the note section and the owner: the GL
//! core (`gl-msaa-cap`, `gl-msaa-resolve`, note (b)) or the PowerVR back end
//! (`pvr-msaa-fbo`, note (c1), which also needs the multisample smp form of
//! `pvr-ms-smp`). They fail today at `glRenderbufferStorageMultisample`. The
//! pixel expectations are what a back end that draws and resolves gives (the
//! 4x pattern of the ms4 image: two of four samples on a mid-pixel vertical
//! edge); they read the resolved single-sample framebuffer, which this
//! stand-in does not fill, so whoever un-ignores them runs them against the
//! PowerVR's own host path (`pvr-msaa-fbo`).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test es3_msaa_pvr
//! ```

#![cfg(all(feature = "host-test", feature = "shaderc-online", feature = "gpu-powervr", not(feature = "report-es2")))]

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
    /// `targets[..num_targets]`: (gpu, format).
    targets: Vec<(u64, u8)>,
    /// `GlesDraw::samples`.
    samples: u8,
    /// `GlesDraw::depth_id`, 0 without a depth attachment.
    depth_id: u32,
}

static SEEN: Mutex<Vec<Seen>> = Mutex::new(Vec::new());
/// The ids `attachment_gone` was told, in order.
static GONE: Mutex<Vec<u32>> = Mutex::new(Vec::new());

unsafe extern "C" fn b_draw(d: *const GlesDraw) -> i32 {
    let d = &*d;
    let n = (d.num_targets.max(1) as usize).min(GLES_MAX_TARGETS);
    SEEN.lock().unwrap().push(Seen {
        draws: d.index_count != 0,
        targets: d.targets[..n].iter().map(|t| (t.gpu, t.format)).collect(),
        samples: d.samples,
        depth_id: d.depth_id,
    });
    0
}

unsafe extern "C" fn b_gone(id: u32) {
    GONE.lock().unwrap().push(id);
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
    name: c"es3-msaa-pvr".as_ptr() as *const c_char,
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
    attachment_gone: Some(b_gone),
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
        glDisable(GL_DEPTH_TEST);
        glFinish();
    }
    SEEN.lock().unwrap().clear();
    GONE.lock().unwrap().clear();
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

/// A bound 8x8 framebuffer with one RGBA8 colour renderbuffer of `samples`
/// and, with `depth`, a DEPTH_COMPONENT24 one of the same count. A storage
/// error comes back as `Err(error)`.
fn framebuffer(samples: GLsizei, depth: bool) -> Result<Fbo, GLenum> {
    let mut fb = 0;
    // SAFETY: room for one name; plain GL call.
    unsafe {
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
    }
    let mut fbo = Fbo { fb, rbs: Vec::new() };
    let (e, rb) = storage(samples, GL_RGBA8);
    fbo.rbs.push(rb);
    if e != GL_NO_ERROR {
        return Err(e);
    }
    // SAFETY: a renderbuffer this test made.
    unsafe { glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_RENDERBUFFER, rb) };
    if depth {
        let (e, rb) = storage(samples, GL_DEPTH_COMPONENT24);
        fbo.rbs.push(rb);
        if e != GL_NO_ERROR {
            return Err(e);
        }
        // SAFETY: a renderbuffer this test made.
        unsafe { glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_DEPTH_ATTACHMENT, GL_RENDERBUFFER, rb) };
    }
    let buf = GL_COLOR_ATTACHMENT0;
    // SAFETY: plain GL calls.
    unsafe {
        glDrawBuffers(1, &buf);
        assert_eq!(err(), GL_NO_ERROR);
        assert_eq!(
            glCheckFramebufferStatus(GL_FRAMEBUFFER),
            GL_FRAMEBUFFER_COMPLETE,
            "{samples} samples, depth {depth}"
        );
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

/// The current program: one float output of `colour`, vertices at depth `z`
/// (clip space; 0.5 is window depth 0.75).
fn program(colour: [f32; 4], z: f32) {
    let [r, g, b, a] = colour;
    let fs = format!(
        "#version 300 es\nprecision highp float;\nlayout(location = 0) out vec4 o0;\nvoid main() {{ o0 = vec4({r:?}, {g:?}, {b:?}, {a:?}); }}\n"
    );
    let vs = format!("#version 300 es\nin vec4 p;\nvoid main() {{ gl_Position = vec4(p.xy, {z:?}, 1.0); }}\n");
    let v = shader(GL_VERTEX_SHADER, &vs);
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

/// The 8x8 target's whole area: one oversized triangle.
static FULL: [f32; 12] = [-1.0, -1.0, 0.0, 1.0, 3.0, -1.0, 0.0, 1.0, -1.0, 3.0, 0.0, 1.0];
/// Left of x = -0.125: three whole columns of 8 (0..=2) and the left half of
/// column 3, so a 4x pixel there has two of its four samples covered.
static EDGE: [f32; 24] = [
    -1.0, -1.0, 0.0, 1.0, -0.125, -1.0, 0.0, 1.0, -0.125, 1.0, 0.0, 1.0, -1.0, -1.0, 0.0, 1.0, -0.125, 1.0, 0.0, 1.0,
    -1.0, 1.0, 0.0, 1.0,
];

/// Draw `verts` (`n` vertices of 4 floats) and hand back what the back end was
/// given for it.
fn draw(verts: &'static [f32], n: GLsizei) -> Seen {
    SEEN.lock().unwrap().clear();
    // SAFETY: `verts` is static and outlives the draw.
    unsafe {
        glBindBuffer(GL_ARRAY_BUFFER, 0);
        glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, verts.as_ptr() as *const c_void);
        glEnableVertexAttribArray(0);
        glDrawArrays(GL_TRIANGLES, 0, n);
        glFinish();
    }
    assert_eq!(err(), GL_NO_ERROR);
    let s = core::mem::take(&mut *SEEN.lock().unwrap());
    s.into_iter().rev().find(|s| s.draws).expect("the draw reached the back end")
}

fn clear(colour: [f32; 4]) {
    // SAFETY: plain GL calls.
    unsafe {
        glClearColor(colour[0], colour[1], colour[2], colour[3]);
        glClearDepthf(1.0);
        glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT);
    }
    assert_eq!(err(), GL_NO_ERROR);
}

/// Resolve blit of `read` (colour, and depth when `depth`) into a fresh
/// single-sample framebuffer of the same size; the GL error, and the
/// framebuffer (left bound for reading) when it worked.
fn resolve_blit(read: &Fbo, depth: bool) -> (GLenum, Fbo) {
    let dst = framebuffer(0, depth).expect("a single-sample target");
    let mask = GL_COLOR_BUFFER_BIT | if depth { GL_DEPTH_BUFFER_BIT } else { 0 };
    // SAFETY: plain GL calls on names this test made.
    unsafe {
        glBindFramebuffer(GL_READ_FRAMEBUFFER, read.fb);
        glBindFramebuffer(GL_DRAW_FRAMEBUFFER, dst.fb);
        glReadBuffer(GL_COLOR_ATTACHMENT0);
        while glGetError() != GL_NO_ERROR {}
        glBlitFramebuffer(0, 0, 8, 8, 0, 0, 8, 8, mask, GL_NEAREST);
    }
    (err(), dst)
}

/// The 8x8 RGBA8 pixels of `fbo`'s colour attachment, row-major from the
/// bottom left.
fn pixels(fbo: &Fbo) -> Vec<[u8; 4]> {
    let mut v = vec![[0u8; 4]; 64];
    // SAFETY: room for 64 RGBA8 pixels; a framebuffer this test made.
    unsafe {
        glBindFramebuffer(GL_READ_FRAMEBUFFER, fbo.fb);
        glReadBuffer(GL_COLOR_ATTACHMENT0);
        glReadPixels(0, 0, 8, 8, GL_RGBA, GL_UNSIGNED_BYTE, v.as_mut_ptr() as *mut c_void);
    }
    assert_eq!(err(), GL_NO_ERROR);
    v
}

fn near(a: [u8; 4], b: [u8; 4], tol: i32) -> bool {
    a.iter().zip(b).all(|(x, y)| (*x as i32 - y as i32).abs() <= tol)
}

fn geti(pname: GLenum) -> GLint {
    let mut v = -1;
    // SAFETY: one value.
    unsafe { glGetIntegerv(pname, &mut v) };
    v
}

fn rb_samples(rb: GLuint) -> GLint {
    let mut v = -1;
    // SAFETY: one value; a renderbuffer this test made.
    unsafe {
        glBindRenderbuffer(GL_RENDERBUFFER, rb);
        glGetRenderbufferParameteriv(GL_RENDERBUFFER, GL_RENDERBUFFER_SAMPLES, &mut v);
    }
    v
}

const COLOUR: [f32; 4] = [0.25, 0.5, 0.75, 1.0];
/// COLOUR as bytes (round to nearest).
const COLOUR8: [u8; 4] = [64, 128, 191, 255];

/* ---- pins: what the GL core does today (note (0)) ------------------------- */

/// `GL_MAX_SAMPLES` is the constant 1 (`src/gl/state.rs`), below ES 3.0's 4.
#[test]
fn max_samples_is_one_today() {
    let _g = context();
    assert_eq!(geti(GL_MAX_SAMPLES), 1);
}

/// 2 or 4 samples: `GL_INVALID_OPERATION`, no storage (past `GL_MAX_SAMPLES`).
#[test]
fn four_samples_is_invalid_operation() {
    let _g = context();
    for samples in [2, 4, 8] {
        assert_eq!(storage(samples, GL_RGBA8).0, GL_INVALID_OPERATION, "{samples}");
    }
    assert_eq!(storage(4, GL_DEPTH_COMPONENT24).0, GL_INVALID_OPERATION);
    assert_eq!(framebuffer(4, false).err(), Some(GL_INVALID_OPERATION));
    assert_eq!(framebuffer(4, true).err(), Some(GL_INVALID_OPERATION));
}

/// A negative count is `GL_INVALID_VALUE`, checked before the limit.
#[test]
fn a_negative_count_is_invalid_value() {
    let _g = context();
    assert_eq!(storage(-1, GL_RGBA8).0, GL_INVALID_VALUE);
}

/// One sample is plain storage: no error, `GL_RENDERBUFFER_SAMPLES` 1 (0 for a
/// request of 0), and the framebuffer reports a sample buffer of 1 sample.
#[test]
fn one_sample_is_plain_storage() {
    let _g = context();
    let (e, rb) = storage(1, GL_RGBA8);
    assert_eq!(e, GL_NO_ERROR);
    assert_eq!(rb_samples(rb), 1);
    let fbo = framebuffer(0, false).unwrap();
    assert_eq!(rb_samples(fbo.rbs[0]), 0);
    assert_eq!((geti(GL_SAMPLE_BUFFERS), geti(GL_SAMPLES)), (0, 0));
    drop(fbo);
    let fbo = framebuffer(1, false).unwrap();
    assert_eq!(rb_samples(fbo.rbs[0]), 1);
    assert_eq!((geti(GL_SAMPLE_BUFFERS), geti(GL_SAMPLES)), (1, 1));
    // SAFETY: a renderbuffer this test made.
    unsafe { glDeleteRenderbuffers(1, &rb) };
}

/// A draw into a 1-sample FBO, with and without depth, reaches the back end
/// with `GlesDraw::samples == 1` and one target.
#[test]
fn a_one_sample_draw_has_samples_one() {
    let _g = context();
    for depth in [false, true] {
        let fbo = framebuffer(1, depth).unwrap();
        program(COLOUR, 0.0);
        let s = draw(&FULL, 3);
        assert_eq!((s.samples, s.targets.len()), (1, 1), "depth {depth}");
        assert_eq!(s.depth_id != 0, depth);
        // SAFETY: plain GL call.
        unsafe { glUseProgram(0) };
        drop(fbo);
    }
}

/// `glGetInternalformativ`: one sample count, 1, for a renderable colour
/// format; no counts for an integer one (ES 3.0 4.4.2.1).
#[test]
fn internalformat_counts_today() {
    let _g = context();
    let q = |format: GLenum, pname: GLenum| {
        let mut v = -1;
        // SAFETY: room for one value.
        unsafe { glGetInternalformativ(GL_RENDERBUFFER, format, pname, 1, &mut v) };
        assert_eq!(err(), GL_NO_ERROR);
        v
    };
    for f in [GL_RGBA8, GL_SRGB8_ALPHA8, GL_RGB10_A2, GL_RGB565, GL_R8, GL_RG8] {
        assert_eq!((q(f, GL_NUM_SAMPLE_COUNTS), q(f, GL_SAMPLES)), (1, 1), "{f:#x}");
    }
    for f in [GL_R8I, GL_RGBA32UI] {
        assert_eq!(q(f, GL_NUM_SAMPLE_COUNTS), 0, "{f:#x}");
    }
}

/// A blit *into* a multisample framebuffer is refused by the spec at any
/// count: `GL_INVALID_OPERATION`.
#[test]
fn a_blit_into_a_multisample_framebuffer_is_invalid_operation() {
    let _g = context();
    let ms = framebuffer(1, false).unwrap();
    let single = framebuffer(0, false).unwrap();
    // SAFETY: plain GL calls on names this test made.
    unsafe {
        glBindFramebuffer(GL_READ_FRAMEBUFFER, single.fb);
        glBindFramebuffer(GL_DRAW_FRAMEBUFFER, ms.fb);
        while glGetError() != GL_NO_ERROR {}
        glBlitFramebuffer(0, 0, 8, 8, 0, 0, 8, 8, GL_COLOR_BUFFER_BIT, GL_NEAREST);
    }
    assert_eq!(err(), GL_INVALID_OPERATION);
}

/// An integer format stays refused at any count, 4x or not: the spec's rule,
/// not a limit, so it holds after the work too.
#[test]
fn integer_formats_stay_refused() {
    let _g = context();
    for f in [GL_R8I, GL_RGBA8UI, GL_RGBA32UI] {
        assert_eq!(storage(4, f).0, GL_INVALID_OPERATION, "{f:#x}");
    }
}

/// The window is the only 4x target: the 8x8 test context is a 1x window, and
/// binding it back leaves `GL_SAMPLES` at its own count, 0.
#[test]
fn the_test_window_is_single_sample() {
    let _g = context();
    assert_eq!((geti(GL_SAMPLE_BUFFERS), geti(GL_SAMPLES)), (0, 0));
}

/* ---- the expected result: ignored until the design lands -------------------- */

/// 4 once the GL core reads a back-end capability (note (b)1).
#[test]
#[ignore = "note (b)1, owner GL core gl-msaa-cap: GL_MAX_SAMPLES is the constant 1 until the back end sets max_samples 4 (msaa-fbo)"]
fn max_samples_is_four() {
    let _g = context();
    assert_eq!(geti(GL_MAX_SAMPLES), 4);
    let mut v = -1;
    // SAFETY: room for one value.
    unsafe { glGetInternalformativ(GL_RENDERBUFFER, GL_RGBA8, GL_SAMPLES, 1, &mut v) };
    assert_eq!(v, 4);
}

/// Storage, count and completeness of a 4x FBO, and the draw's `samples`.
#[test]
#[ignore = "note (b)1-4, owner GL core gl-msaa-cap: glRenderbufferStorageMultisample(4) is GL_INVALID_OPERATION and an FBO draw has samples 1"]
fn rgba8_4x_fbo_draws_with_samples_four() {
    let _g = context();
    let fbo = framebuffer(4, true).expect("4 samples are legal storage");
    assert_eq!(rb_samples(fbo.rbs[0]), 4);
    assert_eq!(geti(GL_SAMPLES), 4);
    program(COLOUR, 0.0);
    let s = draw(&FULL, 3);
    assert_eq!((s.samples, s.targets.len()), (4, 1), "one colour attachment, as fbo.msaa needs (note (c1))");
}

/// A 4x RGBA8 FBO drawn with a half-covered edge, resolved by a blit: the
/// interior is exact, the edge pixel the average of two covered samples out of
/// four (colour over the 0 clear: half of every channel, alpha included), the
/// uncovered side the clear.
#[test]
#[ignore = "note (c1) + (b)7, owners PowerVR pvr-msaa-fbo (4x FBO draw, PBE downscale store, needs pvr-ms-smp) and GL core gl-msaa-resolve (the blit)"]
fn the_resolve_averages_the_edge_and_keeps_the_interior() {
    let _g = context();
    let fbo = framebuffer(4, false).expect("4 samples are legal storage");
    clear([0.0; 4]);
    program(COLOUR, 0.0);
    assert_eq!(draw(&EDGE, 6).samples, 4);
    let (e, dst) = resolve_blit(&fbo, false);
    assert_eq!(e, GL_NO_ERROR);
    let px = pixels(&dst);
    for y in 0..8 {
        for x in 0..8 {
            let p = px[y * 8 + x];
            match x {
                0..=2 => assert_eq!(p, COLOUR8, "interior ({x},{y})"),
                3 => assert!(near(p, [32, 64, 96, 128], 2), "edge ({x},{y}) {p:?}"),
                _ => assert_eq!(p, [0, 0, 0, 0], "outside ({x},{y})"),
            }
        }
    }
}

/// The resolve of a draw that fills the target is exact everywhere.
#[test]
#[ignore = "note (c1), owners PowerVR pvr-msaa-fbo and GL core gl-msaa-resolve: no 4x FBO to draw into"]
fn a_full_4x_draw_resolves_exactly() {
    let _g = context();
    let fbo = framebuffer(4, false).expect("4 samples are legal storage");
    clear([0.0; 4]);
    program(COLOUR, 0.0);
    draw(&FULL, 3);
    let (e, dst) = resolve_blit(&fbo, false);
    assert_eq!(e, GL_NO_ERROR);
    assert!(pixels(&dst).iter().all(|p| *p == COLOUR8));
}

/// Depth resolves to sample 0 (note (b)7, (c1): the 2x2 spread's first
/// element), not an average. The edge quad is drawn at window depth 0.5; a
/// second draw of a full triangle at window depth 0.75 with GL_LESS into the
/// resolved framebuffer fails where the kept depth is 0.5 (columns 0..=2),
/// passes where it is the clear (columns 4..=7), and column 3 follows whichever
/// sample 0 holds, the same in every row.
#[test]
#[ignore = "note (b)7 + (c1), owners PowerVR pvr-msaa-fbo (ds_write/depth_resolve at the 4x plane) and GL core gl-msaa-resolve; which sample is sample 0 on the BXS-4-64 is BOARD-UNVERIFIED (note (e) open point 6)"]
fn depth_resolve_keeps_sample_zero() {
    let _g = context();
    let fbo = framebuffer(4, true).expect("4 samples are legal storage");
    clear([0.0; 4]);
    // SAFETY: plain GL calls.
    unsafe {
        glEnable(GL_DEPTH_TEST);
        glDepthFunc(GL_LESS);
    }
    program([1.0, 0.0, 0.0, 1.0], 0.0);
    draw(&EDGE, 6);
    let (e, dst) = resolve_blit(&fbo, true);
    assert_eq!(e, GL_NO_ERROR);
    // SAFETY: plain GL calls on a framebuffer this test made.
    unsafe { glBindFramebuffer(GL_FRAMEBUFFER, dst.fb) };
    /* only colour is cleared: a depth clear would hide the resolved depth */
    // SAFETY: plain GL calls.
    unsafe {
        glClearColor(0.0, 0.0, 0.0, 0.0);
        glClear(GL_COLOR_BUFFER_BIT);
    }
    program([0.0, 1.0, 0.0, 1.0], 0.5);
    draw(&FULL, 3);
    let px = pixels(&dst);
    let green = [0, 255, 0, 255];
    for y in 0..8 {
        for x in 0..8 {
            let p = px[y * 8 + x];
            match x {
                0..=2 => assert_eq!(p, [0, 0, 0, 0], "kept depth rejects ({x},{y})"),
                3 => assert_eq!(p == green, px[3] == green, "column 3 is sample 0's, in every row"),
                _ => assert_eq!(p, green, "clear depth passes ({x},{y})"),
            }
        }
    }
}

/// No stale samples after a same-size respecification: the renderbuffer is
/// given storage again at 4x and 8x8, the back end is told its attachment is
/// gone (note (b)5/(b)6: samples are keyed by the attachment, not the address),
/// and a resolve before any draw shows nothing of the edge drawn before.
#[test]
#[ignore = "note (b)5-6 + (c1), owners GL core gl-msaa-cap (attachment_gone on respecification, target_id/target_gen) and PowerVR pvr-msaa-fbo (MsStore keyed by id, gen)"]
fn no_stale_samples_after_a_same_size_respecification() {
    let _g = context();
    let fbo = framebuffer(4, false).expect("4 samples are legal storage");
    clear([0.0; 4]);
    program(COLOUR, 0.0);
    draw(&EDGE, 6);
    GONE.lock().unwrap().clear();
    // SAFETY: the colour renderbuffer this test made, bound again.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, fbo.fb);
        glBindRenderbuffer(GL_RENDERBUFFER, fbo.rbs[0]);
        glRenderbufferStorageMultisample(GL_RENDERBUFFER, 4, GL_RGBA8, 8, 8);
    }
    assert_eq!(err(), GL_NO_ERROR);
    assert!(!GONE.lock().unwrap().is_empty(), "the back end freed what it kept for the old storage");
    let (e, dst) = resolve_blit(&fbo, false);
    assert_eq!(e, GL_NO_ERROR);
    let px = pixels(&dst);
    assert!(px.iter().all(|p| p[3] != 255), "no sample of the old edge came back: {:?}", px[0]);
}
