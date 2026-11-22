// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! A depth-only framebuffer object (the shape of a shadow-map pass: a depth
//! attachment, no colour attachment) is complete and is cleared and drawn
//! through the GL entry points on the Mali back end (plan #367, item
//! depthonly; note docs/notes/mali-depth-only.md).
//!
//! Two cases, each into the real Mali back end over a mock of its OSAL: the
//! depth attachment is a `GL_DEPTH_COMPONENT` texture, then a
//! `GL_DEPTH_COMPONENT16` renderbuffer. Each asserts
//! `glCheckFramebufferStatus == GL_FRAMEBUFFER_COMPLETE`, then with
//! `glColorMask` all false clears the depth and draws one triangle with the
//! depth test on, asserting `GL_NO_ERROR` after every step and that the
//! frame reached the (mock) kernel as one job submission. No pixels are
//! checked. The shaders are original and compile on the Mali (the
//! `mali_draw_oom` control pair).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_depthonly_gl
//! ```

#![cfg(all(feature = "host-test", feature = "shaderc-online", feature = "gpu-mali", not(feature = "report-es2")))]

use core::ffi::{c_char, c_void};
use std::ffi::CString;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use yakogl::backend;
use yakogl::gl::buffer::glBindBuffer;
use yakogl::gl::consts::*;
use yakogl::gl::draw::*;
use yakogl::gl::framebuffer::*;
use yakogl::gl::program::*;
use yakogl::gl::shader::*;
use yakogl::gl::state::*;
use yakogl::gl::texture::*;
use yakogl::gl::types::*;
use yakogl::gl::vertex::{glEnableVertexAttribArray, glVertexAttribPointer};
use yakogl::osal::mali::{OsalMaliGpu, OsalMaliHeap, OSAL_QUERY_MALI_GPU, OSAL_QUERY_MALI_TILER_HEAP};
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};
use yakogl::util::ENOTSUP;

/* ---- the Mali over a mock OSAL ------------------------------------------ */

const GPU: OsalMaliGpu = OsalMaliGpu::T6;
const HEAP: OsalMaliHeap =
    OsalMaliHeap { ctx_gpu: 0x8000_0001_5000, first_chunk_gpu: 0x8000_00a0_0000, chunk_size: 2 << 20, reserved: 0 };

static ALLOCS: AtomicU64 = AtomicU64::new(0);
/// Job submissions so far (the fence counter).
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

/// The OSAL's log hook, called as `log("%s", line)`.
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

const VS: &str = "attribute vec4 a_pos;\nvoid main() { gl_Position = a_pos; }\n";
const FS: &str = "precision mediump float;\nvoid main() { gl_FragColor = vec4(1.0); }\n";

/// Covers the whole target, at depth 0.25.
static TRI: [f32; 12] = [-1.0, -1.0, 0.25, 1.0, 3.0, -1.0, 0.25, 1.0, -1.0, 3.0, 0.25, 1.0];

fn compile(ty: GLenum, text: &str) -> GLuint {
    let text = CString::new(text).unwrap();
    // SAFETY: plain GL calls; one NUL-terminated source.
    unsafe {
        let s = glCreateShader(ty);
        let p: [*const c_char; 1] = [text.as_ptr()];
        glShaderSource(s, 1, p.as_ptr(), core::ptr::null());
        glCompileShader(s);
        let mut ok: GLint = 0;
        glGetShaderiv(s, GL_COMPILE_STATUS, &mut ok);
        assert_eq!(ok, 1, "the Mali compiler refuses the shader");
        s
    }
}

fn no_error(step: &str) {
    // SAFETY: plain GL call.
    assert_eq!(unsafe { glGetError() }, GL_NO_ERROR, "{step}");
}

fn setup() {
    backend::HOST_DRAW_TARGETS.store(1, Ordering::Relaxed);
    // SAFETY: static tables and one context; this is the only test of the
    // file (the two cases run in one test function).
    unsafe {
        osal_set_ops(&OSAL);
        backend::g_backend = &yakogl::mali::backend::yakogl_backend_mali;
        assert_eq!(yakogl::mali::backend::mali_init(), 0);
        yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
        assert!(!yakogl::gl::context::gl_context_create(64, 64).is_null());
        while glGetError() != GL_NO_ERROR {}
    }
}

/// With the depth-only FBO bound: complete, colour mask off, clear depth,
/// draw one triangle; no error at any step, one job submitted.
fn depth_pass(case: &str) {
    // SAFETY: plain GL calls; `TRI` is static and outlives the draw.
    unsafe {
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE, "{case}");
        no_error("status");
        let v = compile(GL_VERTEX_SHADER, VS);
        let f = compile(GL_FRAGMENT_SHADER, FS);
        let p = glCreateProgram();
        glAttachShader(p, v);
        glAttachShader(p, f);
        glBindAttribLocation(p, 0, c"a_pos".as_ptr());
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        assert_eq!(ok, 1, "{case}: link");
        glUseProgram(p);
        no_error("program");
        glViewport(0, 0, 64, 64);
        glColorMask(0, 0, 0, 0);
        no_error("colour mask");
        glEnable(GL_DEPTH_TEST);
        glDepthFunc(GL_LESS);
        glDepthMask(1);
        glClearDepthf(1.0);
        glClear(GL_DEPTH_BUFFER_BIT);
        no_error("clear depth");
        let before = FENCES.load(Ordering::Relaxed);
        glBindBuffer(GL_ARRAY_BUFFER, 0);
        glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, TRI.as_ptr() as *const c_void);
        glEnableVertexAttribArray(0);
        let refused = backend::draws_refused();
        glDrawArrays(GL_TRIANGLES, 0, 3);
        no_error("draw");
        assert_eq!(backend::draws_refused(), refused, "{case}: the draw was refused");
        glFinish();
        no_error("finish");
        assert!(FENCES.load(Ordering::Relaxed) > before, "{case}: the depth frame was submitted");
        glColorMask(1, 1, 1, 1);
        glDisable(GL_DEPTH_TEST);
        no_error("restore");
    }
}

#[test]
fn a_depth_only_fbo_is_complete_and_drawn() {
    setup();
    // SAFETY: plain GL calls.
    unsafe {
        let mut fb = 0;
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);

        // Case 1: a GL_DEPTH_COMPONENT texture (the shadow map).
        let mut tex = 0;
        glGenTextures(1, &mut tex);
        glBindTexture(GL_TEXTURE_2D, tex);
        glTexImage2D(
            GL_TEXTURE_2D,
            0,
            (if cfg!(feature = "report-es2") { GL_DEPTH_COMPONENT } else { GL_DEPTH_COMPONENT24 }) as GLint,
            64,
            64,
            0,
            GL_DEPTH_COMPONENT,
            GL_UNSIGNED_INT,
            core::ptr::null(),
        );
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST as GLint);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST as GLint);
        no_error("depth texture");
        glFramebufferTexture2D(GL_FRAMEBUFFER, GL_DEPTH_ATTACHMENT, GL_TEXTURE_2D, tex, 0);
        no_error("attach texture");
        depth_pass("depth texture");

        // Case 2: a depth renderbuffer.
        glFramebufferTexture2D(GL_FRAMEBUFFER, GL_DEPTH_ATTACHMENT, GL_TEXTURE_2D, 0, 0);
        let mut rb = 0;
        glGenRenderbuffers(1, &mut rb);
        glBindRenderbuffer(GL_RENDERBUFFER, rb);
        glRenderbufferStorage(GL_RENDERBUFFER, GL_DEPTH_COMPONENT16, 64, 64);
        glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_DEPTH_ATTACHMENT, GL_RENDERBUFFER, rb);
        no_error("attach renderbuffer");
        depth_pass("depth renderbuffer");
    }
}
