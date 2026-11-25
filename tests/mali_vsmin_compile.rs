// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! dEQP's smallest vertex shader on the Mali online compiler (#11328).
//!
//! dEQP-GLES2 builds `void main (void) { gl_Position = vec4(0.0); }` with
//! the smallest fragment shader for its negative_api, state_query and
//! shader_api cases (docs/es2-cts-failures.md M.1 row 7). The NanoPC-T6 tree
//! at 8ff01055 refused it at compile, so those 8 cases fail. The first test
//! pins the outcome at the tip (the shader compiles and links, the draw is
//! refused by the back end); the ignored one asserts that it compiles, links
//! with a trivial fragment shader and draws, for the fix to un-ignore.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_vsmin_compile
//! ```

#![cfg(all(feature = "host-test", feature = "shaderc-online", feature = "gpu-mali", not(feature = "report-es2")))]

use core::ffi::{c_char, c_void, CStr};
use std::ffi::CString;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend;
use yakogl::gl::buffer::glBindBuffer;
use yakogl::gl::consts::*;
use yakogl::gl::draw::*;
use yakogl::gl::framebuffer::*;
use yakogl::gl::program::*;
use yakogl::gl::shader::*;
use yakogl::gl::state::*;
use yakogl::gl::types::*;
use yakogl::gl::vertex::{glDisableVertexAttribArray, glEnableVertexAttribArray, glVertexAttribPointer};
use yakogl::osal::mali::{OsalMaliGpu, OsalMaliHeap, OSAL_QUERY_MALI_GPU, OSAL_QUERY_MALI_TILER_HEAP};
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};
use yakogl::util::ENOTSUP;

/* ---- the Mali over a mock OSAL ------------------------------------------ */

/// The OSAL's answers: the board's (panthor-frame on the T6).
const GPU: OsalMaliGpu = OsalMaliGpu::T6;
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

/// The log lines since the last `context()`.
static LOG: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// The OSAL's log hook: the library only calls it as `log("%s", line)`, so a
/// two-pointer function takes its place (the same calling convention for
/// these arguments).
unsafe extern "C" fn m_log(_fmt: *const c_char, line: *const c_char) {
    let line = core::ffi::CStr::from_ptr(line).to_string_lossy().into_owned();
    eprintln!("osal log: {line}");
    LOG.lock().unwrap_or_else(|e| e.into_inner()).push(line);
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

/// One context for the whole file; the tests take turns on it.
static LOCK: Mutex<()> = Mutex::new(());
static ONCE: Once = Once::new();

/// The GL context and an 8x8 RGBA8 framebuffer object bound.
fn context() -> MutexGuard<'static, ()> {
    let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    ONCE.call_once(|| {
        backend::HOST_DRAW_TARGETS.store(1, Ordering::Relaxed);
        // SAFETY: static tables and one context; the lock above serialises
        // the tests.
        unsafe {
            osal_set_ops(&OSAL);
            backend::g_backend = &yakogl::mali::backend::yakogl_backend_mali;
            assert_eq!(yakogl::mali::backend::mali_init(), 0);
            yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
            assert!(!yakogl::gl::context::gl_context_create(8, 8).is_null());
            let (mut fb, mut rb) = (0, 0);
            glGenFramebuffers(1, &mut fb);
            glBindFramebuffer(GL_FRAMEBUFFER, fb);
            glGenRenderbuffers(1, &mut rb);
            glBindRenderbuffer(GL_RENDERBUFFER, rb);
            glRenderbufferStorage(GL_RENDERBUFFER, GL_RGBA8, 8, 8);
            glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_RENDERBUFFER, rb);
            assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE);
        }
    });
    LOG.lock().unwrap_or_else(|e| e.into_inner()).clear();
    // SAFETY: plain GL calls.
    unsafe {
        glClearColor(0.5, 0.25, 0.75, 0.4);
        glClear(GL_COLOR_BUFFER_BIT);
        while glGetError() != GL_NO_ERROR {}
    }
    g
}

/// Covers the whole target.
static FULL: [f32; 12] = [-1.0, -1.0, 0.0, 1.0, 3.0, -1.0, 0.0, 1.0, -1.0, 3.0, 0.0, 1.0];

/// dEQP's smallest vertex shader (the negative_api, state_query and
/// shader_api cases build it with the smallest fragment shader).
const VS: &str = "void main (void) { gl_Position = vec4(0.0); }";
/// A trivial fragment shader.
const FS: &str = "void main (void) { gl_FragColor = vec4(1.0); }";

/// Compile `text`: the shader, the status and the info log.
fn compile(ty: GLenum, text: &str) -> (GLuint, GLint, String) {
    let src = CString::new(text).unwrap();
    // SAFETY: plain GL calls; one NUL-terminated source; `buf` holds 512 bytes.
    unsafe {
        let s = glCreateShader(ty);
        let p: [*const c_char; 1] = [src.as_ptr()];
        glShaderSource(s, 1, p.as_ptr(), core::ptr::null());
        glCompileShader(s);
        let mut ok: GLint = 0;
        glGetShaderiv(s, GL_COMPILE_STATUS, &mut ok);
        let mut buf = [0 as c_char; 512];
        let mut len: GLsizei = 0;
        glGetShaderInfoLog(s, 512, &mut len, buf.as_mut_ptr());
        (s, ok, CStr::from_ptr(buf.as_ptr()).to_string_lossy().into_owned())
    }
}

/// Link `v` and `f`, draw one triangle: (linked, draws refused, GL error).
fn link_and_draw(v: GLuint, f: GLuint) -> (bool, u64, GLenum) {
    // SAFETY: plain GL calls; `FULL` is static and outlives the draw.
    unsafe {
        let p = glCreateProgram();
        glAttachShader(p, v);
        glAttachShader(p, f);
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        glUseProgram(p);
        glBindBuffer(GL_ARRAY_BUFFER, 0);
        glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, FULL.as_ptr() as *const c_void);
        glEnableVertexAttribArray(0);
        while glGetError() != GL_NO_ERROR {}
        let before = backend::draws_refused();
        glDrawArrays(GL_TRIANGLES, 0, 3);
        glFinish();
        let e = glGetError();
        glDisableVertexAttribArray(0);
        let refused = backend::draws_refused() - before;
        glUseProgram(0);
        glDeleteProgram(p);
        (ok == 1, refused as u64, e)
    }
}

/// The outcome at the tip (59b8bac6, host): the vertex shader compiles (info
/// log empty; the constant gl_Position fix of #11328, 159f1d14) and links
/// with the trivial fragment shader, but a draw with it is refused by the
/// back end: "mali frame: a draw with no vertex attribute, more than 16, or
/// one of other than 1 to 4 32-bit floats is not built yet" (-95, GL error
/// GL_INVALID_OPERATION from glDrawArrays). dEQP only builds the program, so
/// the 8 cases of M.1 row 7 depend on the compile and link alone.
#[test]
fn today_the_smallest_vertex_shader_compiles_and_links_but_its_draw_is_refused() {
    let _g = context();
    let (v, ok, log) = compile(GL_VERTEX_SHADER, VS);
    assert_eq!((ok, log.as_str()), (1, ""), "the vertex shader's compile status and info log");
    let (f, ok, log) = compile(GL_FRAGMENT_SHADER, FS);
    assert_eq!((ok, log.as_str()), (1, ""), "the fragment shader's compile status and info log");
    let (linked, refused, err) = link_and_draw(v, f);
    assert!(linked, "the program links");
    assert_eq!(refused, 1, "the draw is refused by the back end");
    assert_eq!(err, GL_INVALID_OPERATION, "glDrawArrays' error");
    let osal = LOG.lock().unwrap_or_else(|e| e.into_inner()).clone();
    assert!(
        osal.iter().any(|l| l.contains("a draw with no vertex attribute, more than 16, or one of other than 1 to 4")),
        "the refusal line; osal log {osal:?}"
    );
}

/// What a full fix delivers: it compiles, links with a trivial fragment
/// shader and draws one triangle without a refusal. Un-ignore it and delete
/// the pin above when the back end builds a draw with no vertex attribute.
#[test]
#[ignore = "issue #11328: compile and link work at the tip, the draw of a shader with no vertex attribute is refused (-95, GL_INVALID_OPERATION)"]
fn the_smallest_vertex_shader_compiles_links_and_draws() {
    let _g = context();
    let (v, ok, log) = compile(GL_VERTEX_SHADER, VS);
    assert_eq!(ok, 1, "the vertex shader does not compile; info log {log:?}");
    let (f, ok, log) = compile(GL_FRAGMENT_SHADER, FS);
    assert_eq!(ok, 1, "the fragment shader does not compile; info log {log:?}");
    let (linked, refused, err) = link_and_draw(v, f);
    let osal = LOG.lock().unwrap_or_else(|e| e.into_inner()).clone();
    assert!(linked, "the program does not link; osal log {osal:?}");
    assert_eq!(refused, 0, "the draw was refused; osal log {osal:?}");
    assert_eq!(err, GL_NO_ERROR, "GL error after the draw");
}
