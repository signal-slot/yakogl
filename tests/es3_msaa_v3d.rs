// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! 4x multisampling of several colour attachments and of ES 3.0 colour
//! formats on the V3D, through the GL API (t-msaa-v3d), on the host.
//!
//! docs/notes/v3d-msaa-es3.md section (1): the GL core never makes a 4x
//! framebuffer object. `GL_MAX_SAMPLES` is 1, `glRenderbufferStorageMultisample`
//! gives `GL_INVALID_OPERATION` for 2 or more samples, one sample is plain
//! storage. So `draw_call`'s two refusals by name, "4x multisampling of several
//! colour attachments" and "4x multisampling of an ES 3.0 colour format"
//! (`src/v3d/backend.rs`), cannot fire from an application, and what the
//! application sees is the storage error pinned here. Only the window is 4x
//! (`default_samples`), with one RGBA8 target: the one shape the V3D takes.
//!
//! The GL core is driven through the API into a stand-in back end that runs
//! the V3D's own `state::refusal` / `state::unsupported` on what it is handed.
//!
//! Passing tests pin today. The `#[ignore]`d ones are the expected result
//! after the note's design (per-RT 4x tile buffer, resolve at store), each
//! naming the note section and the owner: GL core (`src/gl`, note (4)) or the
//! V3D part (`src/v3d`, `driver/src/v3d/v3d_job.c`, note (3)). They fail
//! today at `glRenderbufferStorageMultisample`.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_msaa_v3d
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
}

static SEEN: Mutex<Vec<Seen>> = Mutex::new(Vec::new());

unsafe extern "C" fn b_draw(d: *const GlesDraw) -> i32 {
    let d = &*d;
    let n = (d.num_targets.max(1) as usize).min(GLES_MAX_TARGETS);
    let refused = state::refusal(d).or_else(|| state::unsupported(d));
    SEEN.lock().unwrap().push(Seen {
        draws: d.index_count != 0,
        targets: d.targets[..n].iter().map(|t| (t.gpu, t.format)).collect(),
        samples: d.samples,
        refused,
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
    name: c"es3-msaa".as_ptr() as *const c_char,
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

/// Resolve blit of `read`'s colour 0 into a fresh single-sample RGBA8
/// framebuffer of the same size; the GL error.
fn resolve_blit(read: &Fbo) -> GLenum {
    let dst = framebuffer(0, &[GL_RGBA8]).expect("a single-sample target");
    // SAFETY: plain GL calls on names this test made.
    unsafe {
        glBindFramebuffer(GL_READ_FRAMEBUFFER, read.fb);
        glBindFramebuffer(GL_DRAW_FRAMEBUFFER, dst.fb);
        glReadBuffer(GL_COLOR_ATTACHMENT0);
        while glGetError() != GL_NO_ERROR {}
        glBlitFramebuffer(0, 0, 8, 8, 0, 0, 8, 8, GL_COLOR_BUFFER_BIT, GL_NEAREST);
    }
    let e = err();
    drop(dst);
    e
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

fn internalformat(format: GLenum, pname: GLenum) -> GLint {
    let mut v = -1;
    // SAFETY: room for one value.
    unsafe { glGetInternalformativ(GL_RENDERBUFFER, format, pname, 1, &mut v) };
    assert_eq!(err(), GL_NO_ERROR);
    v
}

const RGBA8: [GLenum; 1] = [GL_RGBA8];
const ES3_FORMAT: [GLenum; 1] = [GL_SRGB8_ALPHA8];
const TWO_RTS: [GLenum; 2] = [GL_RGBA8, GL_RGBA8];

/* ---- the expected job words (note (3), after Mesa) ----------------------- */

/// What the 4x job of `formats` should carry: one `multisample_mode_4x` bit
/// for the job, the tile of Mesa's `v3d_choose_tile_size` for V3D 4.x (index =
/// RT class 0/1/2 for 1/2/3-4 RTs + 2 for 4x + the largest internal bpp 0/1/2
/// for 32/64/128), and for each RT a load and a kept store (ALL_SAMPLES, the
/// RT's own sample buffer, UIF at twice the size, the format's own cpp) and a
/// resolve store (decimate 4X, the raster target).
#[derive(Debug, PartialEq)]
struct JobWords {
    multisample_mode_4x: bool,
    tile: (u32, u32),
    kept_stores: usize,
    resolve_stores: usize,
    sample_buffer_cpp: Vec<u32>,
}

fn expected_job(formats: &[GLenum]) -> JobWords {
    const TILES: [(u32, u32); 7] = [(64, 64), (64, 32), (32, 32), (32, 16), (16, 16), (16, 8), (8, 8)];
    /* RGBA8 is internal type 8 at 32 bpp; sRGB8_A8, RGB10_A2 and the 16F rows
     * are 16F at 64 bpp (note (2)) */
    let bpp = |f: GLenum| usize::from(matches!(f, GL_SRGB8_ALPHA8 | GL_RGB10_A2 | GL_RGBA16F));
    let cpp = |f: GLenum| if f == GL_RGBA16F { 8 } else { 4 };
    let rt_class = [0usize, 0, 1, 2, 2][formats.len().min(4)];
    let max_bpp = formats.iter().map(|f| bpp(*f)).max().unwrap_or(0);
    JobWords {
        multisample_mode_4x: true,
        tile: TILES[rt_class + 2 + max_bpp],
        kept_stores: formats.len(),
        resolve_stores: formats.len(),
        sample_buffer_cpp: formats.iter().map(|f| cpp(*f)).collect(),
    }
}

/// The table above against the note's own numbers: one RGBA8 at 4x is 32x32,
/// a 64-bpp RT alone 32x16, {RGBA8, sRGB8_A8} 16x16 (index 1+2+1), four RTs
/// with a 64-bpp one 16x8.
#[test]
fn the_expected_tile_sizes_are_the_notes() {
    assert_eq!(expected_job(&RGBA8).tile, (32, 32));
    assert_eq!(expected_job(&ES3_FORMAT).tile, (32, 16));
    assert_eq!(expected_job(&[GL_RGBA8, GL_SRGB8_ALPHA8]).tile, (16, 16));
    assert_eq!(expected_job(&TWO_RTS).tile, (32, 16));
    assert_eq!(expected_job(&[GL_RGBA8, GL_RGBA8, GL_RGBA8, GL_SRGB8_ALPHA8]).tile, (16, 8));
}

/* ---- the queries the GL core answers ------------------------------------ */

/// `GL_MAX_SAMPLES` is 1 (note (1), `src/gl/state.rs`).
#[test]
fn max_samples_is_one_today() {
    let _g = context();
    assert_eq!(geti(GL_MAX_SAMPLES), 1);
}

/// 4 once the GL core reads a back-end capability (note (4)).
#[test]
#[ignore = "note (4), owner src/gl: GL_MAX_SAMPLES is the constant 1 until the back end gets a max_samples capability"]
fn max_samples_is_four() {
    let _g = context();
    assert_eq!(geti(GL_MAX_SAMPLES), 4);
}

/// `glGetInternalformativ`: one sample count, 1, for a renderable format; no
/// counts for an integer one (ES 3.0 4.4.2.1).
#[test]
fn internalformat_counts_today() {
    let _g = context();
    for f in [GL_RGBA8, GL_SRGB8_ALPHA8, GL_RGB10_A2, GL_RGB565, GL_R8, GL_RG8, GL_RGBA4, GL_RGB5_A1, GL_RGB8] {
        assert_eq!(internalformat(f, GL_NUM_SAMPLE_COUNTS), 1, "{f:#x}");
        assert_eq!(internalformat(f, GL_SAMPLES), 1, "{f:#x}");
    }
    for f in [GL_R8I, GL_RG16UI, GL_RGBA32I, GL_RGB10_A2UI] {
        assert_eq!(internalformat(f, GL_NUM_SAMPLE_COUNTS), 0, "{f:#x}");
    }
}

/// The queries after the GL core part: sample count 4 for the
/// multisample-renderable formats, none for the integer ones.
#[test]
#[ignore = "note (4), owner src/gl: glGetInternalformativ answers the count 1, not 4, for the multisample-renderable formats"]
fn internalformat_counts_four() {
    let _g = context();
    for f in [GL_RGBA8, GL_SRGB8_ALPHA8, GL_RGB10_A2, GL_RGB565, GL_R8, GL_RG8] {
        assert_eq!(internalformat(f, GL_NUM_SAMPLE_COUNTS), 1, "{f:#x}");
        assert_eq!(internalformat(f, GL_SAMPLES), 4, "{f:#x}");
    }
    for f in [GL_R8I, GL_RGBA32UI] {
        assert_eq!(internalformat(f, GL_NUM_SAMPLE_COUNTS), 0, "{f:#x}");
    }
}

/// GL_SAMPLES and GL_SAMPLE_BUFFERS of the window and of framebuffers of
/// zero-sample and one-sample storage, and the renderbuffer's own count.
#[test]
fn samples_queries_today() {
    let _g = context();
    assert_eq!((geti(GL_SAMPLE_BUFFERS), geti(GL_SAMPLES)), (0, 0), "the window of the 8x8 test context");
    let fbo = framebuffer(0, &RGBA8).unwrap();
    assert_eq!((geti(GL_SAMPLE_BUFFERS), geti(GL_SAMPLES)), (0, 0));
    assert_eq!(rb_samples(fbo.rbs[0]), 0);
    drop(fbo);
    /* one sample is plain storage "with sample buffers" */
    let fbo = framebuffer(1, &RGBA8).unwrap();
    assert_eq!((geti(GL_SAMPLE_BUFFERS), geti(GL_SAMPLES)), (1, 1));
    assert_eq!(rb_samples(fbo.rbs[0]), 1);
}

/* ---- what the application sees: the storage errors ------------------------ */

/// 4 samples on RGBA8: `GL_INVALID_OPERATION`, no storage (the count is past
/// GL_MAX_SAMPLES, 1). The V3D takes a 4x RGBA8 target only as the window.
#[test]
fn four_samples_of_rgba8_is_invalid_operation() {
    let _g = context();
    assert_eq!(storage(4, GL_RGBA8).0, GL_INVALID_OPERATION);
    assert_eq!(storage(2, GL_RGBA8).0, GL_INVALID_OPERATION);
}

/// 4 samples on an ES 3.0 colour format, the refusal "4x multisampling of an
/// ES 3.0 colour format": the application gets the same error at storage time
/// and the V3D's guard is never reached. Integer formats are refused by the
/// spec itself at any count above 0.
#[test]
fn four_samples_of_an_es3_format_is_invalid_operation() {
    let _g = context();
    for f in [GL_SRGB8_ALPHA8, GL_RGB10_A2, GL_RGBA16F, GL_R8, GL_RG8, GL_RGB565, GL_RGB8] {
        assert_eq!(storage(4, f).0, GL_INVALID_OPERATION, "{f:#x}");
    }
    for f in [GL_R8I, GL_RGBA8UI, GL_RG16I, GL_RGBA32UI] {
        assert_eq!(storage(1, f).0, GL_INVALID_OPERATION, "{f:#x}: no multisample integer format");
        assert_eq!(storage(4, f).0, GL_INVALID_OPERATION, "{f:#x}");
    }
}

/// Two colour attachments at 4 samples, the refusal "4x multisampling of
/// several colour attachments": the first attachment already fails.
#[test]
fn four_samples_of_two_attachments_is_invalid_operation() {
    let _g = context();
    assert_eq!(framebuffer(4, &TWO_RTS).err(), Some(GL_INVALID_OPERATION));
    assert_eq!(framebuffer(4, &[GL_RGBA8, GL_SRGB8_ALPHA8]).err(), Some(GL_INVALID_OPERATION));
}

/// A negative count is `GL_INVALID_VALUE`, checked before the limit.
#[test]
fn a_negative_count_is_invalid_value() {
    let _g = context();
    assert_eq!(storage(-1, GL_RGBA8).0, GL_INVALID_VALUE);
}

/// What works today: one sample. A draw into one and into two such
/// attachments (the back end gets `samples == 1`, the V3D takes it) and a
/// resolve blit into a single-sample framebuffer.
#[test]
fn one_sample_draws_and_blits_today() {
    let _g = context();
    for formats in [&RGBA8[..], &ES3_FORMAT[..], &TWO_RTS[..]] {
        let fbo = framebuffer(1, formats).unwrap();
        program(formats.len());
        let s = draw();
        assert_eq!(s.samples, 1);
        assert_eq!(s.targets.len(), formats.len());
        assert_eq!(s.refused, None, "{formats:#x?}");
        assert_eq!(resolve_blit(&fbo), GL_NO_ERROR, "{formats:#x?}");
    }
}

/// A blit *into* a multisample framebuffer is refused by the spec.
#[test]
fn a_blit_into_a_multisample_framebuffer_is_invalid_operation() {
    let _g = context();
    let ms = framebuffer(1, &RGBA8).unwrap();
    let single = framebuffer(0, &[GL_RGBA8]).unwrap();
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
    assert_eq!(storage(4, GL_RGBA32UI).0, GL_INVALID_OPERATION);
}

/* ---- the V3D's side, below the GL core ------------------------------------ */

/// A hand-made 4x draw of the window shape (one RGBA8 target) passes the
/// V3D's state layer, and so does an sRGB one: the two refusals by name are in
/// `draw_call` (`src/v3d/backend.rs`), after it, and covered at that level by
/// `src/v3d/backend_tests.rs`.
#[test]
fn the_state_layer_takes_a_4x_draw() {
    let mut d = GlesDraw::ZERO;
    d.target_gpu = 0x0100_0000;
    (d.target_w, d.target_h, d.target_format, d.target_alpha_bits) = (8, 8, GLES_SURF_RGBA8888, 8);
    (d.samples, d.prim, d.index_count, d.line_width) = (4, GLES_PRIM_TRIANGLES, 3, 1.0);
    (d.viewport, d.depth_range, d.color_mask, d.color_write) = ([0, 0, 8, 8], [0.0, 1.0], 0xf, true);
    assert_eq!(state::refusal(&d), None);
    assert!(state::job_state(&d).is_ok());
    d.target_format = GLES_SURF_SRGB8_A8;
    assert_eq!(state::refusal(&d), None, "the format is a drawable row");
    d.samples = 2;
    assert!(state::refusal(&d).is_some(), "only 1 and 4");
}

/* ---- the expected result: ignored until the design lands ------------------- */

/// One 4x framebuffer of `formats`: storage, completeness, GL_SAMPLES, the
/// draw as the back end gets it, the job words, and the resolve blit.
fn expect_4x(formats: &[GLenum]) {
    let _g = context();
    let fbo = framebuffer(4, formats).expect("4 samples are legal storage");
    assert_eq!(rb_samples(fbo.rbs[0]), 4);
    assert_eq!(geti(GL_SAMPLES), 4);
    program(formats.len());
    let s = draw();
    assert_eq!((s.samples, s.targets.len(), s.refused), (4, formats.len(), None));
    assert_eq!(resolve_blit(&fbo), GL_NO_ERROR);
    /* the job: the comparison with the recorded command list belongs to the
     * driver self-test of note (3) "Host tests" */
    let w = expected_job(formats);
    assert!(w.multisample_mode_4x);
    assert_eq!((w.kept_stores, w.resolve_stores), (formats.len(), formats.len()));
}

/// RGBA8 at 4 samples, one RT: 32x32 tile, load + kept-samples store +
/// resolve store (the window's shape, as an FBO).
#[test]
#[ignore = "note (4), owner src/gl: glRenderbufferStorageMultisample(4) is GL_INVALID_OPERATION, an FBO is never 4x (draw.rs samples = 1)"]
fn rgba8_4x_draw_and_resolve() {
    assert_eq!(expected_job(&RGBA8).tile, (32, 32));
    expect_4x(&RGBA8);
}

/// An ES 3.0 colour format at 4 samples: the sample buffer in the format's
/// cpp, 64-bpp tile 32x16 (sRGB8_A8 is 16F at 64 bpp).
#[test]
#[ignore = "note (3) item 3 + (4), owners src/v3d (job builder, ms_buf cpp) and src/gl: refusal '4x multisampling of an ES 3.0 colour format'"]
fn srgb8_alpha8_4x_draw_and_resolve() {
    assert_eq!(expected_job(&ES3_FORMAT).tile, (32, 16));
    expect_4x(&ES3_FORMAT);
}

/// Two RGBA8 attachments at 4 samples: tile 32x16 (RT class 1 + 2 + bpp 0), a
/// load, a kept store and a resolve store for each RT.
#[test]
#[ignore = "note (3) items 1-4 + (4), owners src/v3d (per-RT sample buffers) and src/gl: refusal '4x multisampling of several colour attachments'"]
fn two_rgba8_4x_draw_and_resolve() {
    assert_eq!(expected_job(&TWO_RTS).tile, (32, 16));
    expect_4x(&TWO_RTS);
}

/// {RGBA8, sRGB8_A8}: tile 16x16 (index 1+2+1), the note's example.
#[test]
#[ignore = "note (3) 'Host tests' + (4), owners src/v3d and src/gl: both refusals at once"]
fn rgba8_and_srgb_4x_draw_and_resolve() {
    assert_eq!(expected_job(&[GL_RGBA8, GL_SRGB8_ALPHA8]).tile, (16, 16));
    expect_4x(&[GL_RGBA8, GL_SRGB8_ALPHA8]);
}

/// The other multisample-renderable colour formats of ES 3.0 4.4.2.1 at 4
/// samples.
#[test]
#[ignore = "note (2)/(4), owner src/gl: the multisample-renderable list of ES 3.0 4.4.2.1 is not accepted at 4 samples"]
fn other_es3_formats_4x_storage() {
    let _g = context();
    for f in [GL_RGB10_A2, GL_R8, GL_RG8, GL_RGB565, GL_RGBA4, GL_RGB5_A1, GL_RGB8] {
        assert_eq!(storage(4, f).0, GL_NO_ERROR, "{f:#x}");
    }
}
