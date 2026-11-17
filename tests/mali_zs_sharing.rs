// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The depth/stencil objects the 35 cause-5 dEQP-GLES2 cases share between
//! framebuffers draw without a refusal on the Mali, through the GL API on the
//! host (zs-pin, plan #373 cause 5).
//!
//! `fbo.render.{shared_depthbuffer,recreate_depthbuffer,recreate_stencilbuffer}.
//! *_depth_component16_stencil_index8` were refused `NOT_BUILT_ZS_ELSEWHERE`
//! ("a depth or stencil object drawn in another framebuffer's depth/stencil
//! pair is not built yet") until ce1fa529 (docs/notes/mali-zs-sharing.md).
//! This test replays their GL sequences from es2fFboRenderTest.cpp
//! (`SharedDepthbufferTest`, `RecreateBuffersTest<GL_DEPTH_BUFFER_BIT>` and
//! `<GL_STENCIL_BUFFER_BIT>`, rebind and no_rebind; the calls are those of
//! tools/mali/testdata/pan_zs_ref/zs_seq.py, which made Mesa's traces) into
//! the real Mali back end over a mock of its OSAL: an RGB565 colour, a
//! GL_DEPTH_COMPONENT16 and a separate GL_STENCIL_INDEX8 renderbuffer, a
//! scissored stencil clear, depth- and stencil-tested draws. A refused draw
//! reaches the GL core as `-ENOTSUP` (`GL_INVALID_OPERATION`, one more in
//! `draws_refused`). Every draw must leave `GL_NO_ERROR` and `draws_refused`
//! unchanged, and the framebuffers must be complete. The sequences include a
//! pending scissored stencil clear that opens a new frame after the stencil
//! renderbuffer was deleted (`mali_attachment_gone`). No pixels are checked
//! (src/mali/backend_tests.rs does).
//!
//! The file builds only with `report-es2`: the GL core reports a separate
//! D16 + S8 renderbuffer pair complete only in an ES 2.0 build (ES 3.0 4.4.4.2
//! makes it GL_FRAMEBUFFER_UNSUPPORTED; src/gl/framebuffer.rs, the test
//! `status_complete_with_separate_depth_and_stencil_renderbuffers`), and the
//! 35 cases are the pair's. The depth-only and stencil-only variants of the
//! same groups are replayed in the same build, so one feature set covers
//! them all. The gate row needs
//! `host-test,gpu-mali,shaderc-online,no-shader-table,report-es2`.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table,report-es2 --test mali_zs_sharing
//! ```

#![cfg(all(feature = "host-test", feature = "shaderc-online", feature = "gpu-mali", feature = "report-es2"))]

use core::ffi::{c_char, c_void};
use std::ffi::CString;
use std::sync::atomic::{AtomicI64, AtomicU32, AtomicU64, Ordering};
use yakogl::backend;
use yakogl::gl::buffer::glBindBuffer;
use yakogl::gl::consts::*;
use yakogl::gl::draw::*;
use yakogl::gl::framebuffer::*;
use yakogl::gl::program::*;
use yakogl::gl::shader::*;
use yakogl::gl::state::*;
use yakogl::gl::types::*;
use yakogl::gl::vertex::{glEnableVertexAttribArray, glVertexAttribPointer};
use yakogl::osal::mali::{OsalMaliGpu, OsalMaliHeap, OSAL_QUERY_MALI_GPU, OSAL_QUERY_MALI_TILER_HEAP};
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};
use yakogl::util::ENOTSUP;

/* ---- the Mali over a mock OSAL ------------------------------------------ */

/// The OSAL's answers: the board's (panthor-frame on the T6).
const GPU: OsalMaliGpu =
    OsalMaliGpu { gpu_id: 0xa867_0005, tiler_features: 0x809, cs_reg_count: 96, unpreserved_cs_reg_count: 4 };
const HEAP: OsalMaliHeap =
    OsalMaliHeap { ctx_gpu: 0x8000_0001_5000, first_chunk_gpu: 0x8000_00a0_0000, chunk_size: 2 << 20, reserved: 0 };

/// Buffers made so far: each one's GPU VA is its number times 16 MiB, far
/// from any CPU address. Nothing is given back (a few MiB for the run).
static ALLOCS: AtomicU64 = AtomicU64::new(0);
static FENCES: AtomicI64 = AtomicI64::new(0);

unsafe extern "C" fn m_alloc(_heap: u64, size: usize, _flags: u32, out: *mut OsalGpuMem) -> i32 {
    let n = ALLOCS.fetch_add(1, Ordering::Relaxed) + 1;
    assert!(size <= 16 << 20);
    let cpu = Box::leak(vec![0u64; size.div_ceil(8)].into_boxed_slice()).as_mut_ptr().cast::<c_void>();
    *out = OsalGpuMem { cpu, gpu: 0x40_0000_0000 + n * (16 << 20), phys: 0, size, impl_: n as *mut c_void };
    0
}
unsafe extern "C" fn m_free(mem: *mut OsalGpuMem) -> i32 {
    *mem = OsalGpuMem::ZERO;
    0
}
unsafe extern "C" fn m_cache(_cpu: *mut c_void, _n: usize) {}
unsafe extern "C" fn m_init(_arg: *mut c_void) -> i32 {
    0
}
unsafe extern "C" fn m_submit(_kick: *const c_void, _len: usize) -> i64 {
    FENCES.fetch_add(1, Ordering::Relaxed) + 1
}
unsafe extern "C" fn m_wait(_fence: i64, _timeout_us: i64) -> i32 {
    0
}
unsafe extern "C" fn m_completed() -> i64 {
    FENCES.load(Ordering::Relaxed)
}
unsafe extern "C" fn m_query(what: u32, out: *mut c_void, sz: usize) -> i32 {
    match what {
        OSAL_QUERY_MALI_GPU if sz == core::mem::size_of::<OsalMaliGpu>() => *out.cast::<OsalMaliGpu>() = GPU,
        OSAL_QUERY_MALI_TILER_HEAP if sz == core::mem::size_of::<OsalMaliHeap>() => *out.cast::<OsalMaliHeap>() = HEAP,
        _ => return -ENOTSUP,
    }
    0
}

/// The OSAL's log hook: the library only calls it as `log("%s", line)`, so a
/// two-pointer function takes its place (the same calling convention for
/// these arguments).
unsafe extern "C" fn m_log(_fmt: *const c_char, line: *const c_char) {
    eprintln!("osal log: {}", core::ffi::CStr::from_ptr(line).to_string_lossy());
}

static OSAL: OsalOps = OsalOps {
    init: Some(m_init),
    shutdown: None,
    gpu_alloc: Some(m_alloc),
    gpu_free: Some(m_free),
    mmu_map: None,
    map_phys: None,
    gpu_write: None,
    gpu_read: None,
    cache_flush: Some(m_cache),
    cache_invd: Some(m_cache),
    submit: Some(m_submit),
    wait: Some(m_wait),
    completed: Some(m_completed),
    reg_read: None,
    reg_write: None,
    query: Some(m_query),
    // SAFETY: see `m_log`.
    log: Some(unsafe {
        core::mem::transmute::<
            unsafe extern "C" fn(*const c_char, *const c_char),
            unsafe extern "C" fn(*const c_char, ...),
        >(m_log)
    }),
    wait_vblank: None,
    present: None,
    lock: None,
    unlock: None,
    thread_id: None,
    display_busy: None,
};

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

const N: GLsizei = 32;

fn setup() {
    backend::HOST_DRAW_TARGETS.store(1, Ordering::Relaxed);
    // SAFETY: static tables and one context; this is the only test of the
    // file.
    unsafe {
        osal_set_ops(&OSAL);
        backend::g_backend = &yakogl::mali::backend::yakogl_backend_mali;
        assert_eq!(yakogl::mali::backend::mali_init(), 0);
        yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
        assert!(!yakogl::gl::context::gl_context_create(N as u32, N as u32).is_null());
        let v = shader(GL_VERTEX_SHADER, "attribute vec4 p;\nvoid main() { gl_Position = p; }\n");
        let f = shader(
            GL_FRAGMENT_SHADER,
            "precision mediump float;\nuniform vec4 c;\nvoid main() { gl_FragColor = c; }\n",
        );
        let p = glCreateProgram();
        glAttachShader(p, v);
        glAttachShader(p, f);
        glBindAttribLocation(p, 0, c"p".as_ptr());
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        assert_eq!(ok, 1);
        glUseProgram(p);
        PROGRAM.store(p, Ordering::Relaxed);
        glViewport(0, 0, N, N);
        while glGetError() != GL_NO_ERROR {}
    }
}

static PROGRAM: AtomicU32 = AtomicU32::new(0);

/// One GL step's check: no error.
fn ok(what: &str) {
    // SAFETY: plain GL call.
    let e = unsafe { glGetError() };
    assert_eq!(e, GL_NO_ERROR, "{what}: glGetError 0x{e:x}");
}

fn complete(what: &str) {
    // SAFETY: plain GL call.
    let s = unsafe { glCheckFramebufferStatus(GL_FRAMEBUFFER) };
    assert_eq!(s, GL_FRAMEBUFFER_COMPLETE, "{what}: status 0x{s:x}");
}

fn clear(bits: GLbitfield, what: &str) {
    // SAFETY: plain GL call.
    unsafe { glClear(bits) };
    ok(what);
}

/// sglr::drawQuad's two triangles of the corners' z `z0` and `z1` (the
/// middle corners' mean) with colour `c`; the draw must not be refused.
fn quad(c: [f32; 4], z0: f32, z1: f32, what: &str) {
    let mid = 0.5 * (z0 + z1);
    let p: [f32; 16] = [-1.0, -1.0, z0, 1.0, -1.0, 1.0, mid, 1.0, 1.0, -1.0, mid, 1.0, 1.0, 1.0, z1, 1.0];
    static IDX: [u16; 6] = [0, 2, 1, 2, 1, 3];
    let before = backend::draws_refused();
    // SAFETY: `p` and `IDX` outlive the draw, which `glFinish` completes.
    let e = unsafe {
        glBindBuffer(GL_ARRAY_BUFFER, 0);
        let loc = glGetUniformLocation(PROGRAM.load(Ordering::Relaxed), c"c".as_ptr());
        glUniform4f(loc, c[0], c[1], c[2], c[3]);
        glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, p.as_ptr() as *const c_void);
        glEnableVertexAttribArray(0);
        glDrawElements(GL_TRIANGLES, 6, GL_UNSIGNED_SHORT, IDX.as_ptr() as *const c_void);
        glFinish();
        glGetError()
    };
    assert_eq!(e, GL_NO_ERROR, "{what}: glGetError 0x{e:x}");
    assert_eq!(backend::draws_refused(), before, "{what}: a draw was refused");
}

fn renderbuffer(fmt: GLenum) -> GLuint {
    let mut rb = 0;
    // SAFETY: room for one name.
    unsafe {
        glGenRenderbuffers(1, &mut rb);
        glBindRenderbuffer(GL_RENDERBUFFER, rb);
        glRenderbufferStorage(GL_RENDERBUFFER, fmt, N, N);
    }
    ok("renderbuffer");
    rb
}

fn attach(point: GLenum, rb: GLuint) {
    // SAFETY: plain GL call.
    unsafe { glFramebufferRenderbuffer(GL_FRAMEBUFFER, point, GL_RENDERBUFFER, rb) };
    ok("attach");
}

fn bind(f: GLuint) {
    // SAFETY: plain GL call.
    unsafe { glBindFramebuffer(GL_FRAMEBUFFER, f) };
    ok("bind");
}

/// es2fFboRenderTest.cpp's Framebuffer: an RGB565 colour renderbuffer, then
/// the depth and the stencil renderbuffer when asked; left unbound.
struct Fbo {
    fbo: GLuint,
    depth: GLuint,
    stencil: GLuint,
}

fn fbo(depth: bool, stencil: bool) -> Fbo {
    let mut f = 0;
    // SAFETY: room for one name.
    unsafe {
        glGenFramebuffers(1, &mut f);
        glBindFramebuffer(GL_FRAMEBUFFER, f);
    }
    attach(GL_COLOR_ATTACHMENT0, renderbuffer(GL_RGB565));
    let mut r = Fbo { fbo: f, depth: 0, stencil: 0 };
    if depth {
        r.depth = renderbuffer(GL_DEPTH_COMPONENT16);
        attach(GL_DEPTH_ATTACHMENT, r.depth);
    }
    if stencil {
        r.stencil = renderbuffer(GL_STENCIL_INDEX8);
        attach(GL_STENCIL_ATTACHMENT, r.stencil);
    }
    bind(0);
    r
}

fn scissor_clear_stencil(rect: [GLint; 4], value: GLint) {
    // SAFETY: plain GL calls.
    unsafe {
        glEnable(GL_SCISSOR_TEST);
        glScissor(rect[0], rect[1], rect[2], rect[3]);
        glClearStencil(value);
    }
    clear(GL_STENCIL_BUFFER_BIT, "scissored stencil clear");
    // SAFETY: plain GL call.
    unsafe { glDisable(GL_SCISSOR_TEST) };
}

const ALL: GLbitfield = GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT | GL_STENCIL_BUFFER_BIT;
const GREEN: [f32; 4] = [0.0, 1.0, 0.0, 1.0];
const WHITE: [f32; 4] = [1.0; 4];

/// SharedDepthbufferTest::render: A has depth (and stencil), B has its own
/// colour (and stencil) and A's depth renderbuffer; drawn alternately.
fn shared_depthbuffer(stencil: bool) {
    let a = fbo(true, stencil);
    bind(a.fbo);
    complete("shared A");
    let b = fbo(false, stencil);
    bind(b.fbo);
    attach(GL_DEPTH_ATTACHMENT, a.depth);
    complete("shared B");
    // SAFETY: plain GL calls.
    unsafe {
        glClearColor(1.0, 0.0, 0.0, 1.0);
        glClearStencil(1);
    }
    clear(ALL, "clear B");
    // SAFETY: plain GL call.
    unsafe { glEnable(GL_DEPTH_TEST) };
    bind(a.fbo);
    // SAFETY: plain GL call.
    unsafe { glClearColor(0.0, 0.0, 0.0, 1.0) };
    clear(ALL, "clear A");
    quad(WHITE, 0.0, 0.0, "A");
    bind(b.fbo);
    quad(WHITE, -1.0, 1.0, "B, A's depth");
    // SAFETY: plain GL call.
    unsafe { glDisable(GL_DEPTH_TEST) };
    if stencil {
        scissor_clear_stencil([10, 10, 12, 25], 0);
        // SAFETY: plain GL calls.
        unsafe {
            glEnable(GL_STENCIL_TEST);
            glStencilFunc(GL_EQUAL, 0, 0xff);
        }
        quad(GREEN, 0.0, 0.0, "B, stencil-tested");
        // SAFETY: plain GL call.
        unsafe { glDisable(GL_STENCIL_TEST) };
    }
    // A is drawn again after B wrote the shared depth.
    bind(a.fbo);
    // SAFETY: plain GL call.
    unsafe { glEnable(GL_DEPTH_TEST) };
    quad(WHITE, 0.0, 0.0, "A again");
    // SAFETY: plain GL call.
    unsafe { glDisable(GL_DEPTH_TEST) };
    bind(0);
}

/// RecreateBuffersTest<`depth_buffers`>::render: draw, clear the stencil
/// under a scissor, delete the depth or stencil renderbuffer and make it
/// anew (`rebind`: attach it again), clear it, draw.
fn recreate(depth_buffers: bool, rebind: bool, depth: bool, stencil: bool) {
    let f = fbo(depth, stencil);
    bind(f.fbo);
    complete("recreate");
    // SAFETY: plain GL call.
    unsafe { glClearColor(1.0, 0.0, 0.0, 1.0) };
    clear(ALL, "clear");
    // SAFETY: plain GL call.
    unsafe { glEnable(GL_DEPTH_TEST) };
    quad(WHITE, 0.0, 0.0, "first draw");
    if stencil {
        scissor_clear_stencil([N / 4, N / 4, N / 2, N / 2], 1);
    }
    if !rebind {
        bind(0);
    }
    let (point, fmt, name) = if depth_buffers {
        (GL_DEPTH_ATTACHMENT, GL_DEPTH_COMPONENT16, f.depth)
    } else {
        (GL_STENCIL_ATTACHMENT, GL_STENCIL_INDEX8, f.stencil)
    };
    // SAFETY: plain GL calls.
    unsafe {
        glDeleteRenderbuffers(1, &name);
        glBindRenderbuffer(GL_RENDERBUFFER, name);
        glRenderbufferStorage(GL_RENDERBUFFER, fmt, N, N);
    }
    ok("recreate storage");
    if rebind {
        attach(point, name);
    } else {
        bind(f.fbo);
    }
    complete("recreated");
    // SAFETY: plain GL calls.
    unsafe {
        glClearColor(0.0, 0.0, 1.0, 0.0);
        glClearStencil(0);
    }
    clear(if depth_buffers { GL_DEPTH_BUFFER_BIT } else { GL_STENCIL_BUFFER_BIT }, "clear recreated");
    if stencil {
        // SAFETY: plain GL calls.
        unsafe {
            glEnable(GL_STENCIL_TEST);
            glStencilFunc(GL_EQUAL, 0, 0xff);
        }
    }
    quad(GREEN, 1.0, -1.0, "draw after recreate");
    // SAFETY: plain GL calls.
    unsafe {
        glDisable(GL_STENCIL_TEST);
        glDisable(GL_DEPTH_TEST);
    }
    bind(0);
}

#[test]
fn shared_and_recreated_depth_stencil_draw_unrefused() {
    setup();
    // The 35 cases are the ones with the depth/stencil pair; the variants
    // with one of the two come with the same groups.
    for stencil in [true, false] {
        shared_depthbuffer(stencil);
    }
    for depth_buffers in [true, false] {
        for rebind in [true, false] {
            for (depth, stencil) in [(true, true), (depth_buffers, !depth_buffers)] {
                recreate(depth_buffers, rebind, depth, stencil);
            }
        }
    }
    assert_eq!(backend::draws_refused(), 0);
}
