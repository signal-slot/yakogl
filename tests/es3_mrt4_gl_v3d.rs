// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The GL core with the V3D back end under a four-target stand-in
//! (mrt4-gl-v3d), on the host.
//!
//! `testutil::backend_draws_targets(4)` is `backend::HOST_DRAW_TARGETS`: the
//! count GL_MAX_DRAW_BUFFERS, GL_MAX_COLOR_ATTACHMENTS and gl_MaxDrawBuffers
//! read (RC14 of docs/es3-cts-failures.md). Pinned here, with the V3D back
//! end's own `state::refusal` / `state::unsupported` / `state::job_state` run
//! on every draw the GL core hands over:
//!
//! - the limits answer 4, a draw-buffer list of 4 is accepted, of 5 is
//!   GL_INVALID_VALUE, GL_DRAW_BUFFER3 reads back;
//! - COLOR_ATTACHMENT3 attaches and reads (glReadBuffer), COLOR_ATTACHMENT4
//!   is GL_INVALID_OPERATION on a framebuffer object;
//! - four attachments are framebuffer-complete, a four-output shader links;
//! - a draw into four targets reaches the back end with `num_targets == 4`
//!   and each target's gpu address and format; `V3dJobState` carries no
//!   target count (the RT words are the C frame builder's, `v3d_frame.c`).
//!
//! The GLSL cases are es3_mrt4_glsl's.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_mrt4_gl_v3d
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
use yakogl::v3d::ffi::*;
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
#[derive(Clone)]
struct DrawCopy(GlesDraw);
// SAFETY: the pointers in the copy are read only by this thread, under LOCK.
unsafe impl Send for DrawCopy {}

#[derive(Clone)]
struct Seen {
    /// An application draw (vertices), not a clear alone.
    draws: bool,
    /// `targets[..num_targets]`: (gpu, format).
    targets: Vec<(u64, u8)>,
    /// The draw as the GL core built it (a copy; tests may change it and re-run `job_state`).
    draw: DrawCopy,
    /// `state::refusal` / `state::unsupported`.
    refused: Option<&'static str>,
    /// `state::job_state`, when the V3D takes the draw.
    job: Option<V3dJobState>,
}

static SEEN: Mutex<Vec<Seen>> = Mutex::new(Vec::new());

unsafe extern "C" fn b_draw(d: *const GlesDraw) -> i32 {
    let d = &*d;
    let n = (d.num_targets.max(1) as usize).min(GLES_MAX_TARGETS);
    let refused = state::refusal(d).or_else(|| state::unsupported(d));
    let job = if d.index_count != 0 { state::job_state(d).ok() } else { None };
    SEEN.lock().unwrap().push(Seen {
        draws: d.index_count != 0,
        targets: d.targets[..n].iter().map(|t| (t.gpu, t.format)).collect(),
        draw: DrawCopy(*d),
        refused,
        job,
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
    name: c"es3-mrt4-gl-v3d".as_ptr() as *const c_char,
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
        glDisable(GL_BLEND);
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

/* ---- the framebuffer, the program, the draw ---------------------------- */

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

/// A bound 8x8 framebuffer with one renderbuffer of each sized format on
/// colour attachments 0.., all of them draw buffers.
fn framebuffer(formats: &[GLenum]) -> Fbo {
    let (mut fb, mut rbs) = (0, Vec::new());
    // SAFETY: room for one name each.
    unsafe {
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        for (i, f) in formats.iter().enumerate() {
            let mut rb = 0;
            glGenRenderbuffers(1, &mut rb);
            glBindRenderbuffer(GL_RENDERBUFFER, rb);
            glRenderbufferStorage(GL_RENDERBUFFER, *f, 8, 8);
            assert_eq!(err(), GL_NO_ERROR, "storage {f:#x}");
            glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0 + i as GLenum, GL_RENDERBUFFER, rb);
            rbs.push(rb);
        }
        let bufs: Vec<GLenum> = (0..formats.len()).map(|i| GL_COLOR_ATTACHMENT0 + i as GLenum).collect();
        glDrawBuffers(bufs.len() as GLsizei, bufs.as_ptr());
        assert_eq!(err(), GL_NO_ERROR);
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE, "{formats:#x?}");
    }
    Fbo { fb, rbs }
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

/// The current program: one `out` per entry of `types` ("vec4", "ivec4",
/// "uvec4").
fn program(types: &[&str]) {
    let mut fs = String::from("#version 300 es\nprecision highp float;\nprecision highp int;\n");
    for (i, t) in types.iter().enumerate() {
        fs += &format!("layout(location = {i}) out {t} o{i};\n");
    }
    fs += "void main() {\n";
    for (i, t) in types.iter().enumerate() {
        fs += &match *t {
            "vec4" => format!("o{i} = vec4(0.25, 0.5, 0.75, 0.5);\n"),
            "ivec4" => format!("o{i} = ivec4(-1, 2, -3, 4);\n"),
            _ => format!("o{i} = uvec4(1u, 2u, 3u, 4u);\n"),
        };
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

/// Draw one triangle and hand back what the V3D's job state carries for it.
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

/// A bound 8x8 framebuffer with `n` RGBA8 renderbuffers on COLOR_ATTACHMENT0..
/// and no draw-buffer list set yet.
fn fbo_n(n: usize) -> Fbo {
    let (mut fb, mut rbs) = (0, Vec::new());
    // SAFETY: room for one name each.
    unsafe {
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        for i in 0..n {
            let mut rb = 0;
            glGenRenderbuffers(1, &mut rb);
            glBindRenderbuffer(GL_RENDERBUFFER, rb);
            glRenderbufferStorage(GL_RENDERBUFFER, GL_RGBA8, 8, 8);
            glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0 + i as GLenum, GL_RENDERBUFFER, rb);
            rbs.push(rb);
        }
    }
    assert_eq!(err(), GL_NO_ERROR);
    Fbo { fb, rbs }
}

fn geti(pname: GLenum) -> GLint {
    let mut v = -1;
    // SAFETY: one int.
    unsafe { glGetIntegerv(pname, &mut v) };
    v
}

fn four_buffers() {
    let b: Vec<GLenum> = (0..4).map(|i| GL_COLOR_ATTACHMENT0 + i).collect();
    // SAFETY: four entries.
    unsafe { glDrawBuffers(4, b.as_ptr()) };
}

#[test]
fn limits_answer_four() {
    let _g = context();
    assert_eq!(geti(GL_MAX_DRAW_BUFFERS), 4);
    assert_eq!(geti(GL_MAX_COLOR_ATTACHMENTS), 4);
    assert_eq!(err(), GL_NO_ERROR);
}

/// `a[gl_MaxDrawBuffers - 3]` compiles only for >= 4, `b[5 - ...]` only for
/// <= 4.
#[test]
fn gl_max_draw_buffers_is_four_in_a_shader() {
    let _g = context();
    let fs = "#version 300 es\nprecision highp float;\nlayout(location = 0) out vec4 o;\n\
              void main() { float a[gl_MaxDrawBuffers - 3]; float b[5 - gl_MaxDrawBuffers]; \
              a[0] = 1.0; b[0] = 1.0; o = vec4(a[0] + b[0]); }\n";
    shader(GL_FRAGMENT_SHADER, fs);
}

#[test]
fn draw_buffers_four_accepted_five_rejected() {
    let _g = context();
    let _f = fbo_n(4);
    four_buffers();
    assert_eq!(err(), GL_NO_ERROR);
    for i in 0..4 {
        assert_eq!(geti(GL_DRAW_BUFFER0 + i), (GL_COLOR_ATTACHMENT0 + i) as GLint, "GL_DRAW_BUFFER{i}");
    }
    let five: Vec<GLenum> = (0..5).map(|i| GL_COLOR_ATTACHMENT0 + i).collect();
    // SAFETY: five entries.
    unsafe { glDrawBuffers(5, five.as_ptr()) };
    assert_eq!(err(), GL_INVALID_VALUE);
    // the failed call left the list alone
    assert_eq!(geti(GL_DRAW_BUFFER3), GL_COLOR_ATTACHMENT3 as GLint);
}

#[test]
fn attachment_three_attaches_four_does_not() {
    let _g = context();
    let _f = fbo_n(4);
    let mut ty = 0;
    // SAFETY: one int.
    unsafe {
        glGetFramebufferAttachmentParameteriv(
            GL_FRAMEBUFFER,
            GL_COLOR_ATTACHMENT3,
            GL_FRAMEBUFFER_ATTACHMENT_OBJECT_TYPE,
            &mut ty,
        );
    }
    assert_eq!(err(), GL_NO_ERROR);
    assert_eq!(ty as GLenum, GL_RENDERBUFFER);
    let mut rb = 0;
    // SAFETY: plain GL calls on a bound framebuffer.
    unsafe {
        glGenRenderbuffers(1, &mut rb);
        glBindRenderbuffer(GL_RENDERBUFFER, rb);
        glRenderbufferStorage(GL_RENDERBUFFER, GL_RGBA8, 8, 8);
        glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT4, GL_RENDERBUFFER, rb);
    }
    assert_eq!(err(), GL_INVALID_ENUM);
    // SAFETY: a name this test made.
    unsafe { glDeleteRenderbuffers(1, &rb) };
}

#[test]
fn read_buffer_attachment_three() {
    let _g = context();
    let _f = fbo_n(4);
    // SAFETY: plain GL call.
    unsafe { glReadBuffer(GL_COLOR_ATTACHMENT3) };
    assert_eq!(err(), GL_NO_ERROR);
    assert_eq!(geti(GL_READ_BUFFER), GL_COLOR_ATTACHMENT3 as GLint);
    // SAFETY: plain GL call.
    unsafe { glReadBuffer(GL_COLOR_ATTACHMENT4) };
    assert_eq!(err(), GL_INVALID_OPERATION);
}

#[test]
fn four_attachments_are_complete() {
    let _g = context();
    let _f = fbo_n(4);
    four_buffers();
    // SAFETY: plain GL call.
    assert_eq!(unsafe { glCheckFramebufferStatus(GL_FRAMEBUFFER) }, GL_FRAMEBUFFER_COMPLETE);
    // mixed formats, draw buffers set by the helper (asserts completeness)
    drop(_f);
    let _m = framebuffer(&[GL_RGBA8, GL_RGBA32UI, GL_RG16I, GL_RGBA16F]);
}

#[test]
fn four_output_shader_links() {
    let _g = context();
    program(&["vec4", "vec4", "vec4", "vec4"]);
    assert_eq!(err(), GL_NO_ERROR);
}

/// What the back end is handed today: four targets, each with its own address
/// and RGBA8 format; the V3D's refusal checks pass and the job state builds.
#[test]
fn draw_hands_four_targets_to_the_backend() {
    let _g = context();
    let _f = fbo_n(4);
    four_buffers();
    program(&["vec4", "vec4", "vec4", "vec4"]);
    let s = draw();
    assert_eq!(s.draw.0.num_targets, 4);
    assert_eq!(s.targets.len(), 4);
    let mut gpus: Vec<u64> = s.targets.iter().map(|t| t.0).collect();
    assert!(gpus.iter().all(|&g| g != 0));
    gpus.sort();
    gpus.dedup();
    assert_eq!(gpus.len(), 4, "four distinct surfaces: {:?}", s.targets);
    assert!(s.targets.iter().all(|t| t.1 == s.targets[0].1));
    assert_eq!(s.refused, None);
    assert!(s.job.is_some(), "state::job_state builds the draw");
}
