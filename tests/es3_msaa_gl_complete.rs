// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! ES 3.0 multisample renderbuffers, the GL core's framebuffer completeness
//! on the host (docs/notes/es3-msaa-renderbuffers.md (b)10; ES 3.0.6 section
//! 4.4.4).
//!
//! The same stand-in back end as `es3_msaa_gl.rs` with `max_samples` 4
//! (`context_ms4`). The tests drive glCheckFramebufferStatus over mixed
//! sample counts and attachment kinds: 4x colour with 4x depth/stencil, with
//! no-sample depth, with a texture, MRT with equal and unequal counts,
//! separate depth and stencil renderbuffers, requests of 2 and 4 that both
//! store 4, respecification after a status check, the read framebuffer, and a
//! draw on an incomplete framebuffer.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_msaa_gl_complete
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

/// `GlesDraw::index_count != 0` of each draw the back end was handed.
static SEEN: Mutex<Vec<bool>> = Mutex::new(Vec::new());

unsafe extern "C" fn b_draw(d: *const GlesDraw) -> i32 {
    SEEN.lock().unwrap().push((*d).index_count != 0);
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
static BACKEND_MS4: YakoglBackend = YakoglBackend { max_samples: 4, ..BACKEND };

/// One context for the whole file; the tests take turns on it.
static LOCK: Mutex<()> = Mutex::new(());
static ONCE: Once = Once::new();

/// The context with `table` as the back end.
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

/* ---- framebuffers ------------------------------------------------------- */

/// Names to delete when the test ends; leaves the default framebuffer bound.
#[derive(Default)]
struct Names {
    fbs: Vec<GLuint>,
    rbs: Vec<GLuint>,
}

impl Drop for Names {
    fn drop(&mut self) {
        // SAFETY: names this test made (textures are left to the context).
        unsafe {
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
            glDeleteFramebuffers(self.fbs.len() as GLsizei, self.fbs.as_ptr());
            glDeleteRenderbuffers(self.rbs.len() as GLsizei, self.rbs.as_ptr());
            glUseProgram(0);
        }
    }
}

impl Names {
    /// A new framebuffer, bound to GL_FRAMEBUFFER.
    fn fbo(&mut self) -> GLuint {
        let mut fb = 0;
        // SAFETY: room for one name; plain GL call.
        unsafe {
            glGenFramebuffers(1, &mut fb);
            glBindFramebuffer(GL_FRAMEBUFFER, fb);
        }
        self.fbs.push(fb);
        fb
    }

    /// A new 8x8 renderbuffer of `format` stored with a request of `samples`.
    fn rb(&mut self, samples: GLsizei, format: GLenum) -> GLuint {
        let mut rb = 0;
        // SAFETY: plain GL calls.
        unsafe {
            glGenRenderbuffers(1, &mut rb);
            glBindRenderbuffer(GL_RENDERBUFFER, rb);
            glRenderbufferStorageMultisample(GL_RENDERBUFFER, samples, format, 8, 8);
        }
        assert_eq!(err(), GL_NO_ERROR, "storage of {samples} samples, {format:#x}");
        self.rbs.push(rb);
        rb
    }
}

/// A 2D RGBA8 texture of 8x8.
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

/// Attach `rb` at `attachment` of the bound GL_FRAMEBUFFER.
fn attach_rb(attachment: GLenum, rb: GLuint) {
    // SAFETY: a renderbuffer this test made.
    unsafe { glFramebufferRenderbuffer(GL_FRAMEBUFFER, attachment, GL_RENDERBUFFER, rb) };
    assert_eq!(err(), GL_NO_ERROR);
}

fn attach_tex(attachment: GLenum, tex: GLuint) {
    // SAFETY: a texture this test made.
    unsafe { glFramebufferTexture2D(GL_FRAMEBUFFER, attachment, GL_TEXTURE_2D, tex, 0) };
    assert_eq!(err(), GL_NO_ERROR);
}

/// Respecify `rb` (bound by this call) with `samples` and `format`, 8x8.
fn respecify(rb: GLuint, samples: GLsizei, format: GLenum) {
    // SAFETY: a renderbuffer this test made.
    unsafe {
        glBindRenderbuffer(GL_RENDERBUFFER, rb);
        glRenderbufferStorageMultisample(GL_RENDERBUFFER, samples, format, 8, 8);
    }
    assert_eq!(err(), GL_NO_ERROR);
}

fn status_of(target: GLenum) -> GLenum {
    // SAFETY: plain GL call.
    let s = unsafe { glCheckFramebufferStatus(target) };
    assert_eq!(err(), GL_NO_ERROR);
    s
}

fn status() -> GLenum {
    status_of(GL_FRAMEBUFFER)
}

const COMPLETE: GLenum = GL_FRAMEBUFFER_COMPLETE;
const INCOMPLETE_MS: GLenum = GL_FRAMEBUFFER_INCOMPLETE_MULTISAMPLE;
const C0: GLenum = GL_COLOR_ATTACHMENT0;

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

/// Draw one triangle over the bound framebuffer, finish, and give the GL
/// error of the draw and how many application draws the back end was given.
fn draw_triangle() -> (GLenum, usize) {
    static TRI: [f32; 12] = [-1.0, -1.0, 0.0, 1.0, 3.0, -1.0, 0.0, 1.0, -1.0, 3.0, 0.0, 1.0];
    SEEN.lock().unwrap().clear();
    // SAFETY: TRI is static and outlives the draw.
    let e = unsafe {
        glBindBuffer(GL_ARRAY_BUFFER, 0);
        glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, TRI.as_ptr() as *const c_void);
        glEnableVertexAttribArray(0);
        glDrawArrays(GL_TRIANGLES, 0, 3);
        let e = glGetError();
        glFinish();
        e
    };
    let n = SEEN.lock().unwrap().iter().filter(|d| **d).count();
    (e, n)
}

/* ---- ES 3.0.6 4.4.4: the sample counts --------------------------------- */

/// 4x colour and 4x depth24_stencil8: every attached renderbuffer has the
/// same RENDERBUFFER_SAMPLES.
#[test]
fn four_x_colour_and_four_x_depth_stencil_is_complete() {
    let _g = context_ms4();
    let mut n = Names::default();
    n.fbo();
    attach_rb(C0, n.rb(4, GL_RGBA8));
    attach_rb(GL_DEPTH_STENCIL_ATTACHMENT, n.rb(4, GL_DEPTH24_STENCIL8));
    assert_eq!(status(), COMPLETE);
}

/// The counts differ (4 and 0): INCOMPLETE_MULTISAMPLE, for depth, stencil
/// and packed depth/stencil.
#[test]
fn four_x_colour_and_zero_sample_depth_is_incomplete() {
    let _g = context_ms4();
    let mut n = Names::default();
    n.fbo();
    attach_rb(C0, n.rb(4, GL_RGBA8));
    attach_rb(GL_DEPTH_ATTACHMENT, n.rb(0, GL_DEPTH_COMPONENT16));
    assert_eq!(status(), INCOMPLETE_MS, "depth16 of 0 samples");
    attach_rb(GL_DEPTH_ATTACHMENT, 0);
    assert_eq!(status(), COMPLETE, "depth detached");
    attach_rb(GL_DEPTH_STENCIL_ATTACHMENT, n.rb(0, GL_DEPTH24_STENCIL8));
    assert_eq!(status(), INCOMPLETE_MS, "depth24_stencil8 of 0 samples");
    attach_rb(GL_DEPTH_STENCIL_ATTACHMENT, 0);
    attach_rb(GL_STENCIL_ATTACHMENT, n.rb(0, GL_STENCIL_INDEX8));
    assert_eq!(status(), INCOMPLETE_MS, "stencil8 of 0 samples");
}

/// 0x colour with 4x depth is as incomplete as the reverse.
#[test]
fn zero_sample_colour_and_four_x_depth_is_incomplete() {
    let _g = context_ms4();
    let mut n = Names::default();
    n.fbo();
    attach_rb(C0, n.rb(0, GL_RGBA8));
    attach_rb(GL_DEPTH_STENCIL_ATTACHMENT, n.rb(4, GL_DEPTH24_STENCIL8));
    assert_eq!(status(), INCOMPLETE_MS);
}

/// A texture counts as samples 0: a 4x renderbuffer next to it is
/// INCOMPLETE_MULTISAMPLE, on another colour point or as the depth.
#[test]
fn four_x_colour_and_a_texture_is_incomplete() {
    let _g = context_ms4();
    let mut n = Names::default();
    n.fbo();
    attach_rb(C0, n.rb(4, GL_RGBA8));
    let tex = texture();
    attach_tex(GL_COLOR_ATTACHMENT1, tex);
    assert_eq!(status(), INCOMPLETE_MS, "4x colour 0 and a texture on colour 1");
    attach_tex(GL_COLOR_ATTACHMENT1, 0);
    assert_eq!(status(), COMPLETE);
    /* a texture colour with a 4x depth renderbuffer */
    attach_rb(C0, 0);
    attach_tex(C0, tex);
    attach_rb(GL_DEPTH_STENCIL_ATTACHMENT, n.rb(4, GL_DEPTH24_STENCIL8));
    assert_eq!(status(), INCOMPLETE_MS, "texture colour with a 4x depth/stencil");
}

/// MRT, two colour attachments (HOST_DRAW_TARGETS 4): equal counts complete,
/// unequal INCOMPLETE_MULTISAMPLE.
#[test]
fn two_colour_attachments() {
    let _g = context_ms4();
    let mut n = Names::default();
    n.fbo();
    let a = n.rb(4, GL_RGBA8);
    let b = n.rb(4, GL_RGBA8);
    attach_rb(C0, a);
    attach_rb(GL_COLOR_ATTACHMENT1, b);
    assert_eq!(status(), COMPLETE, "4 and 4");
    attach_rb(GL_COLOR_ATTACHMENT1, n.rb(0, GL_RGBA8));
    assert_eq!(status(), INCOMPLETE_MS, "4 and 0");
    attach_rb(C0, n.rb(0, GL_RGBA8));
    assert_eq!(status(), COMPLETE, "0 and 0");
}

/// Four colour attachments: all 4x complete; any one differing, wherever it
/// sits, is INCOMPLETE_MULTISAMPLE.
#[test]
fn four_colour_attachments() {
    let _g = context_ms4();
    let mut n = Names::default();
    n.fbo();
    let rbs: Vec<GLuint> = (0..4).map(|_| n.rb(4, GL_RGBA8)).collect();
    for (i, rb) in rbs.iter().enumerate() {
        attach_rb(C0 + i as GLenum, *rb);
    }
    assert_eq!(status(), COMPLETE, "4 x 4");
    let zero = n.rb(0, GL_RGBA8);
    for i in 0..4 {
        attach_rb(C0 + i, zero);
        assert_eq!(status(), INCOMPLETE_MS, "colour {i} has 0, the others 4");
        attach_rb(C0 + i, rbs[i as usize]);
        assert_eq!(status(), COMPLETE, "colour {i} back to 4");
    }
    attach_rb(GL_DEPTH_STENCIL_ATTACHMENT, n.rb(4, GL_DEPTH24_STENCIL8));
    assert_eq!(status(), COMPLETE, "4 x 4 and depth/stencil 4");
    attach_tex(GL_COLOR_ATTACHMENT3, texture());
    assert_eq!(status(), INCOMPLETE_MS, "three 4x renderbuffers and a texture");
}

/// ES 3.0.6 4.4.4: separate depth and stencil renderbuffers are not the one
/// image, which an ES 3.0 implementation answers with
/// GL_FRAMEBUFFER_UNSUPPORTED, 4x or not; the same renderbuffer at both
/// points is complete. The sample counts agree here, so
/// INCOMPLETE_MULTISAMPLE is not the answer.
#[test]
fn separate_four_x_depth_and_stencil() {
    let _g = context_ms4();
    let mut n = Names::default();
    n.fbo();
    attach_rb(C0, n.rb(4, GL_RGBA8));
    attach_rb(GL_DEPTH_ATTACHMENT, n.rb(4, GL_DEPTH_COMPONENT16));
    attach_rb(GL_STENCIL_ATTACHMENT, n.rb(4, GL_STENCIL_INDEX8));
    assert_eq!(status(), GL_FRAMEBUFFER_UNSUPPORTED, "separate 4x depth and stencil");
    let ds = n.rb(4, GL_DEPTH24_STENCIL8);
    attach_rb(GL_DEPTH_ATTACHMENT, ds);
    attach_rb(GL_STENCIL_ATTACHMENT, ds);
    assert_eq!(status(), COMPLETE, "one 4x depth24_stencil8 at both points");
}

/// A 4x depth only, and a 4x stencil only, each with a 4x colour.
#[test]
fn four_x_depth_only_and_stencil_only() {
    let _g = context_ms4();
    let mut n = Names::default();
    n.fbo();
    attach_rb(C0, n.rb(4, GL_RGBA8));
    attach_rb(GL_DEPTH_ATTACHMENT, n.rb(4, GL_DEPTH_COMPONENT16));
    assert_eq!(status(), COMPLETE, "4x depth only");
    attach_rb(GL_DEPTH_ATTACHMENT, 0);
    attach_rb(GL_STENCIL_ATTACHMENT, n.rb(4, GL_STENCIL_INDEX8));
    assert_eq!(status(), COMPLETE, "4x stencil only");
}

/// A request of 2 and a request of 4 both store 4 (ES 3.0.6 4.4.2.1 lets the
/// implementation give more samples than asked): complete together.
#[test]
fn requests_of_two_and_four_are_complete_together() {
    let _g = context_ms4();
    let mut n = Names::default();
    n.fbo();
    let two = n.rb(2, GL_RGBA8);
    let four = n.rb(4, GL_RGBA8);
    attach_rb(C0, two);
    attach_rb(GL_COLOR_ATTACHMENT1, four);
    assert_eq!(status(), COMPLETE, "colour 2 and 4");
    attach_rb(GL_DEPTH_STENCIL_ATTACHMENT, n.rb(3, GL_DEPTH24_STENCIL8));
    assert_eq!(status(), COMPLETE, "and a depth/stencil requested with 3");
    attach_rb(GL_COLOR_ATTACHMENT1, n.rb(0, GL_RGBA8));
    assert_eq!(status(), INCOMPLETE_MS, "2 and 0");
}

/// Respecifying one attachment's samples after the status was checked changes
/// the status, both ways.
#[test]
fn respecifying_changes_the_status() {
    let _g = context_ms4();
    let mut n = Names::default();
    n.fbo();
    let a = n.rb(4, GL_RGBA8);
    let d = n.rb(4, GL_DEPTH24_STENCIL8);
    attach_rb(C0, a);
    attach_rb(GL_DEPTH_STENCIL_ATTACHMENT, d);
    assert_eq!(status(), COMPLETE);
    respecify(d, 0, GL_DEPTH24_STENCIL8);
    assert_eq!(status(), INCOMPLETE_MS, "depth respecified with 0");
    respecify(d, 2, GL_DEPTH24_STENCIL8);
    assert_eq!(status(), COMPLETE, "depth respecified with 2 (stores 4)");
    respecify(a, 0, GL_RGBA8);
    assert_eq!(status(), INCOMPLETE_MS, "colour respecified with 0");
    respecify(d, 0, GL_DEPTH24_STENCIL8);
    assert_eq!(status(), COMPLETE, "both 0");
    respecify(a, 4, GL_RGBA8);
    respecify(d, 4, GL_DEPTH24_STENCIL8);
    assert_eq!(status(), COMPLETE, "both 4 again");
}

/// GL_READ_FRAMEBUFFER names the read binding: its status is its own, while
/// GL_DRAW_FRAMEBUFFER (and GL_FRAMEBUFFER) report the draw one.
#[test]
fn read_framebuffer_status() {
    let _g = context_ms4();
    let mut n = Names::default();
    let good = n.fbo();
    attach_rb(C0, n.rb(4, GL_RGBA8));
    attach_rb(GL_DEPTH_STENCIL_ATTACHMENT, n.rb(4, GL_DEPTH24_STENCIL8));
    let bad = n.fbo();
    attach_rb(C0, n.rb(4, GL_RGBA8));
    attach_rb(GL_DEPTH_ATTACHMENT, n.rb(0, GL_DEPTH_COMPONENT16));
    // SAFETY: names this test made.
    unsafe {
        glBindFramebuffer(GL_READ_FRAMEBUFFER, bad);
        glBindFramebuffer(GL_DRAW_FRAMEBUFFER, good);
    }
    assert_eq!(status_of(GL_READ_FRAMEBUFFER), INCOMPLETE_MS, "read: bad");
    assert_eq!(status_of(GL_DRAW_FRAMEBUFFER), COMPLETE, "draw: good");
    assert_eq!(status_of(GL_FRAMEBUFFER), COMPLETE, "GL_FRAMEBUFFER is the draw one");
    // SAFETY: names this test made.
    unsafe {
        glBindFramebuffer(GL_READ_FRAMEBUFFER, good);
        glBindFramebuffer(GL_DRAW_FRAMEBUFFER, bad);
    }
    assert_eq!(status_of(GL_READ_FRAMEBUFFER), COMPLETE, "read: good");
    assert_eq!(status_of(GL_DRAW_FRAMEBUFFER), INCOMPLETE_MS, "draw: bad");
}

/// ES 3.0.6 4.4.4: a draw while the framebuffer is not complete is
/// GL_INVALID_FRAMEBUFFER_OPERATION and the back end is not handed a draw;
/// once complete it is.
#[test]
fn draw_on_an_incomplete_four_x_framebuffer() {
    let _g = context_ms4();
    let mut n = Names::default();
    n.fbo();
    attach_rb(C0, n.rb(4, GL_RGBA8));
    attach_rb(GL_DEPTH_ATTACHMENT, n.rb(0, GL_DEPTH_COMPONENT16));
    program();
    assert_eq!(status(), INCOMPLETE_MS);
    assert_eq!(draw_triangle(), (GL_INVALID_FRAMEBUFFER_OPERATION, 0), "4x colour with a 0 depth");
    attach_rb(GL_DEPTH_ATTACHMENT, 0);
    assert_eq!(status(), COMPLETE);
    assert_eq!(draw_triangle(), (GL_NO_ERROR, 1), "complete again");
    attach_tex(GL_COLOR_ATTACHMENT1, texture());
    assert_eq!(draw_triangle(), (GL_INVALID_FRAMEBUFFER_OPERATION, 0), "4x colour with a texture");
}
