// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The depth test and face culling state glmark2-es2's model scenes set every
//! frame, through the GL API on the Mali back end on the host: `glEnable
//! (GL_DEPTH_TEST)`, `glDepthFunc` (LEQUAL, LESS), `glClearDepthf`,
//! `glDepthMask` on and off, `glEnable(GL_CULL_FACE)` with `glCullFace
//! (GL_BACK)` and `glFrontFace` (CCW, CW), on the default framebuffer and on a
//! colour texture + depth renderbuffer FBO.
//!
//! The Mali back end runs over a mock OSAL. No GPU runs, so no pixel is drawn
//! and `glReadPixels` cannot say which triangle is visible. What the test
//! checks is what decides it: the draw the GL core hands the back end (depth
//! test, function, write, cull winding, front face, the pending depth clear)
//! for two overlapping triangles at different depths and a back-facing one,
//! and that `mali_draw` accepts each of them. The depth-and-cull decisions of
//! the board run are the Mali hardware's from there.
//!
//! The shaders are simple ones the online compiler accepts.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_glmark2_depth_cull
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table,report-es2 --test mali_glmark2_depth_cull
//! ```

#![cfg(all(feature = "host-test", feature = "shaderc-online", feature = "gpu-mali"))]

use core::ffi::{c_char, c_void};
use std::ffi::CString;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{self, GlesDraw, YakoglBackend};
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

static FENCES: AtomicI64 = AtomicI64::new(0);
static NEXT_VA: AtomicI64 = AtomicI64::new(1);

unsafe extern "C" fn m_alloc(_heap: u64, size: usize, _flags: u32, out: *mut OsalGpuMem) -> i32 {
    assert!(size <= 16 << 20);
    let cpu = Box::leak(vec![0u64; size.div_ceil(8)].into_boxed_slice()).as_mut_ptr().cast::<c_void>();
    let gpu = 0x40_0000_0000 + (NEXT_VA.fetch_add(1, Ordering::Relaxed) as u64) * (16 << 20);
    *out = OsalGpuMem { cpu, gpu, phys: 0, size, impl_: cpu };
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

/// The OSAL's log hook: the library only calls it as `log("%s", line)`.
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
    // SAFETY: see `m_log`: the same calling convention for these arguments.
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

/// What the back end was given for one draw.
#[derive(Clone, Copy, Debug)]
struct Seen {
    index_count: u32,
    depth_id: u32,
    depth_test: bool,
    depth_func: backend::GlesCmp,
    depth_write: bool,
    cull: backend::GlesCull,
    front_cw: bool,
    do_clear_depth: bool,
    clear_depth: f32,
    mali: i32,
}

static SEEN: Mutex<Vec<Seen>> = Mutex::new(Vec::new());

unsafe extern "C" fn b_draw(d: *const GlesDraw) -> i32 {
    let dr = &*d;
    let mali = yakogl::mali::backend::mali_draw(d);
    SEEN.lock().unwrap().push(Seen {
        index_count: dr.index_count,
        depth_id: dr.depth_id,
        depth_test: dr.depth_test,
        depth_func: dr.depth_func,
        depth_write: dr.depth_write,
        cull: dr.cull,
        front_cw: dr.front_cw,
        do_clear_depth: dr.do_clear_depth,
        clear_depth: dr.clear_depth,
        mali,
    });
    mali
}

static LOCK: Mutex<()> = Mutex::new(());
static ONCE: Once = Once::new();

const N: GLsizei = 32;

fn context() -> MutexGuard<'static, ()> {
    let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    ONCE.call_once(|| {
        backend::HOST_DRAW_TARGETS.store(1, Ordering::Relaxed);
        // SAFETY: the Mali table copied bit for bit with only `draw`
        // replaced; the OSAL and the copy are leaked; the lock serialises
        // the tests.
        unsafe {
            osal_set_ops(&OSAL);
            let mut b: YakoglBackend = core::ptr::read(&yakogl::mali::backend::yakogl_backend_mali);
            b.draw = Some(b_draw);
            backend::g_backend = Box::leak(Box::new(b));
            assert_eq!(yakogl::mali::backend::mali_init(), 0);
            yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
            assert!(!yakogl::gl::context::gl_context_create(N as u32, N as u32).is_null());
            glViewport(0, 0, N, N);
        }
    });
    SEEN.lock().unwrap().clear();
    // SAFETY: plain GL calls: the state every test starts from.
    unsafe {
        while glGetError() != GL_NO_ERROR {}
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glDisable(GL_DEPTH_TEST);
        glDisable(GL_CULL_FACE);
        glDepthMask(1);
        glDepthFunc(GL_LESS);
        glFrontFace(GL_CCW);
        glCullFace(GL_BACK);
        glClearDepthf(1.0);
    }
    g
}

fn ok(what: &str) {
    // SAFETY: plain GL call.
    let e = unsafe { glGetError() };
    assert_eq!(e, GL_NO_ERROR, "{what}: glGetError 0x{e:x}");
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
        let mut log = [0u8; 512];
        let mut n: GLsizei = 0;
        glGetShaderInfoLog(s, log.len() as GLsizei, &mut n, log.as_mut_ptr().cast());
        assert_eq!(ok, 1, "{text:?} did not compile: {}", String::from_utf8_lossy(&log[..n as usize]));
        s
    }
}

fn program() -> GLuint {
    let v = shader(GL_VERTEX_SHADER, "attribute vec4 p;\nvoid main() { gl_Position = p; }\n");
    let f =
        shader(GL_FRAGMENT_SHADER, "precision mediump float;\nuniform vec4 c;\nvoid main() { gl_FragColor = c; }\n");
    // SAFETY: plain GL calls.
    unsafe {
        let p = glCreateProgram();
        glAttachShader(p, v);
        glAttachShader(p, f);
        glBindAttribLocation(p, 0, c"p".as_ptr());
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        assert_eq!(ok, 1, "link");
        glUseProgram(p);
        p
    }
}

/// One triangle at NDC depth `z`, wound counter-clockwise when `ccw`, with
/// colour `c`; the draw the back end saw.
fn tri(p: GLuint, z: f32, ccw: bool, c: [f32; 4], what: &str) -> Seen {
    let a = [-0.8, -0.8, z, 1.0f32];
    let b = [0.8, -0.8, z, 1.0];
    let t = [0.0, 0.8, z, 1.0];
    let v: Vec<f32> = if ccw { [a, b, t] } else { [a, t, b] }.concat();
    SEEN.lock().unwrap().clear();
    let before = backend::draws_refused();
    // SAFETY: `v` outlives the draw, which `glFinish` completes.
    let e = unsafe {
        glUniform4fv(glGetUniformLocation(p, c"c".as_ptr()), 1, c.as_ptr());
        glBindBuffer(GL_ARRAY_BUFFER, 0);
        glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, v.as_ptr() as *const c_void);
        glEnableVertexAttribArray(0);
        glDrawArrays(GL_TRIANGLES, 0, 3);
        glFinish();
        glGetError()
    };
    assert_eq!(e, GL_NO_ERROR, "{what}: glGetError 0x{e:x}");
    assert_eq!(backend::draws_refused(), before, "{what}: a draw was refused");
    let s = SEEN.lock().unwrap().iter().rev().find(|s| s.index_count == 3).copied();
    let s = s.unwrap_or_else(|| panic!("{what}: the draw did not reach the back end"));
    assert_eq!(s.mali, 0, "{what}: mali_draw refused");
    s
}

const RED: [f32; 4] = [1.0, 0.0, 0.0, 1.0];
const GREEN: [f32; 4] = [0.0, 1.0, 0.0, 1.0];

/// Which target: the default framebuffer, or a colour texture + depth
/// renderbuffer FBO (glmark2's render-to-texture scenes).
fn target(fbo: bool) -> Option<(GLuint, GLuint, GLuint)> {
    if !fbo {
        return None;
    }
    let (mut tex, mut rb, mut f) = (0, 0, 0);
    // SAFETY: room for one name each; null pixels.
    unsafe {
        glGenTextures(1, &mut tex);
        glBindTexture(GL_TEXTURE_2D, tex);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST as GLint);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST as GLint);
        glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA as GLint, N, N, 0, GL_RGBA, GL_UNSIGNED_BYTE, core::ptr::null());
        glGenRenderbuffers(1, &mut rb);
        glBindRenderbuffer(GL_RENDERBUFFER, rb);
        glRenderbufferStorage(GL_RENDERBUFFER, GL_DEPTH_COMPONENT16, N, N);
        glGenFramebuffers(1, &mut f);
        glBindFramebuffer(GL_FRAMEBUFFER, f);
        glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex, 0);
        glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_DEPTH_ATTACHMENT, GL_RENDERBUFFER, rb);
        let st = glCheckFramebufferStatus(GL_FRAMEBUFFER);
        assert_eq!(st, GL_FRAMEBUFFER_COMPLETE, "colour+depth FBO: status 0x{st:x}");
    }
    ok("fbo");
    Some((tex, rb, f))
}

/// Run `body` on the default framebuffer and on the FBO.
fn on_both(body: impl Fn(GLuint, bool)) {
    for fbo in [false, true] {
        let _t = target(fbo);
        let p = program();
        body(p, fbo);
        // SAFETY: plain GL call.
        unsafe { glBindFramebuffer(GL_FRAMEBUFFER, 0) };
        ok("restore");
    }
}

/// glmark2's per-frame model state: clear depth 1.0, test on, LEQUAL.
#[test]
fn depth_test_lequal_with_clear() {
    let _g = context();
    on_both(|p, fbo| {
        // SAFETY: plain GL calls.
        unsafe {
            glClearDepthf(1.0);
            glClear(GL_DEPTH_BUFFER_BIT | GL_COLOR_BUFFER_BIT);
            glEnable(GL_DEPTH_TEST);
            glDepthFunc(GL_LEQUAL);
        }
        // the far triangle, then the near one over it
        let far = tri(p, 0.5, true, RED, "far");
        let near = tri(p, -0.5, true, GREEN, "near");
        for (s, what) in [(far, "far"), (near, "near")] {
            assert!(s.depth_test, "{what} fbo={fbo}: depth test on");
            assert_eq!(s.depth_func, backend::GLES_CMP_LEQUAL, "{what} fbo={fbo}");
            assert!(s.depth_write, "{what} fbo={fbo}: depth write on");
            assert_ne!(s.depth_id, 0, "{what} fbo={fbo}: a depth buffer to test against");
        }
        assert!(far.do_clear_depth, "fbo={fbo}: the first draw carries the depth clear");
        assert_eq!(far.clear_depth, 1.0);
        assert!(!near.do_clear_depth, "fbo={fbo}: the clear is not repeated");
    });
}

#[test]
fn depth_test_less_and_clear_value() {
    let _g = context();
    on_both(|p, fbo| {
        // SAFETY: plain GL calls.
        unsafe {
            glClearDepthf(0.5);
            glClear(GL_DEPTH_BUFFER_BIT);
            glEnable(GL_DEPTH_TEST);
            glDepthFunc(GL_LESS);
        }
        let a = tri(p, 0.0, true, RED, "less");
        assert!(a.depth_test && a.depth_write, "fbo={fbo}");
        assert_eq!(a.depth_func, backend::GLES_CMP_LESS, "fbo={fbo}");
        assert!(a.do_clear_depth, "fbo={fbo}");
        assert_eq!(a.clear_depth, 0.5, "fbo={fbo}");
    });
}

/// Every compare function a scene may pick reaches the draw as itself.
#[test]
fn depth_funcs_reach_the_draw() {
    let _g = context();
    let all = [
        (GL_NEVER, backend::GLES_CMP_NEVER),
        (GL_LESS, backend::GLES_CMP_LESS),
        (GL_EQUAL, backend::GLES_CMP_EQUAL),
        (GL_LEQUAL, backend::GLES_CMP_LEQUAL),
        (GL_GREATER, backend::GLES_CMP_GREATER),
        (GL_NOTEQUAL, backend::GLES_CMP_NOTEQUAL),
        (GL_GEQUAL, backend::GLES_CMP_GEQUAL),
        (GL_ALWAYS, backend::GLES_CMP_ALWAYS),
    ];
    on_both(|p, fbo| {
        // SAFETY: plain GL call.
        unsafe { glEnable(GL_DEPTH_TEST) };
        for (gl, want) in all {
            // SAFETY: plain GL call.
            unsafe { glDepthFunc(gl) };
            let s = tri(p, 0.0, true, RED, "func");
            assert_eq!(s.depth_func, want, "fbo={fbo} func 0x{gl:x}");
        }
    });
}

/// glDepthMask(GL_FALSE): the test stays, the write goes (a transparent
/// pass over opaque geometry).
#[test]
fn depth_mask_off_keeps_the_test() {
    let _g = context();
    on_both(|p, fbo| {
        // SAFETY: plain GL calls.
        unsafe {
            glEnable(GL_DEPTH_TEST);
            glDepthFunc(GL_LEQUAL);
            glDepthMask(0);
        }
        let s = tri(p, 0.0, true, RED, "mask off");
        assert!(s.depth_test && !s.depth_write, "fbo={fbo}: test on, write off");
        // SAFETY: plain GL call.
        unsafe { glDepthMask(1) };
        let s = tri(p, 0.0, true, RED, "mask on");
        assert!(s.depth_test && s.depth_write, "fbo={fbo}: test on, write on");
    });
}

#[test]
fn depth_test_off_is_off() {
    let _g = context();
    on_both(|p, fbo| {
        // SAFETY: plain GL call.
        unsafe { glDisable(GL_DEPTH_TEST) };
        let s = tri(p, 0.0, true, RED, "off");
        assert!(!s.depth_test, "fbo={fbo}: depth test off");
    });
}

/// (glCullFace, glFrontFace) -> the winding of the culled faces: the back
/// faces of glmark2's models are clockwise under the default CCW front.
#[test]
fn cull_winding_table() {
    let _g = context();
    let table = [
        (GL_BACK, GL_CCW, backend::GLES_CULL_CW),
        (GL_BACK, GL_CW, backend::GLES_CULL_CCW),
        (GL_FRONT, GL_CCW, backend::GLES_CULL_CCW),
        (GL_FRONT, GL_CW, backend::GLES_CULL_CW),
    ];
    on_both(|p, fbo| {
        // SAFETY: plain GL call.
        unsafe { glEnable(GL_CULL_FACE) };
        for (mode, front, want) in table {
            // SAFETY: plain GL calls.
            unsafe {
                glCullFace(mode);
                glFrontFace(front);
            }
            /* a front-facing and a back-facing triangle for that front face */
            for ccw in [true, false] {
                let s = tri(p, 0.0, ccw, RED, "cull");
                assert_eq!(s.cull, want, "fbo={fbo} cull 0x{mode:x} front 0x{front:x}");
                assert_eq!(s.front_cw, front == GL_CW, "fbo={fbo} front 0x{front:x}");
            }
        }
        // SAFETY: plain GL call.
        unsafe { glDisable(GL_CULL_FACE) };
        let s = tri(p, 0.0, false, RED, "no cull");
        assert_eq!(s.cull, backend::GLES_CULL_NONE, "fbo={fbo}: culling off");
    });
}

/// Depth test and culling together (a model scene's frame): a back-facing
/// triangle behind a front-facing one, both state words in one draw.
#[test]
fn depth_and_cull_together() {
    let _g = context();
    on_both(|p, fbo| {
        // SAFETY: plain GL calls.
        unsafe {
            glClearDepthf(1.0);
            glClear(GL_DEPTH_BUFFER_BIT | GL_COLOR_BUFFER_BIT);
            glEnable(GL_DEPTH_TEST);
            glDepthFunc(GL_LEQUAL);
            glEnable(GL_CULL_FACE);
            glCullFace(GL_BACK);
            glFrontFace(GL_CCW);
        }
        let front = tri(p, 0.2, true, GREEN, "front");
        let back = tri(p, -0.2, false, RED, "back");
        assert_eq!(front.cull, backend::GLES_CULL_CW, "fbo={fbo}");
        assert_eq!(back.cull, backend::GLES_CULL_CW, "fbo={fbo}");
        for s in [front, back] {
            assert!(s.depth_test && s.depth_write, "fbo={fbo}");
            assert_eq!(s.depth_func, backend::GLES_CMP_LEQUAL, "fbo={fbo}");
        }
        assert_eq!(backend::draws_refused(), 0);
    });
}
