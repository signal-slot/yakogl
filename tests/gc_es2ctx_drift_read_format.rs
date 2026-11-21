// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GC7000 (i.MX8MP), ES 2.0 context: the implementation read format and the
//! binary-format queries of 654b0620 (GL_IMPLEMENTATION_COLOR_READ_FORMAT/TYPE
//! for RGB10_A2 read buffers) and 5e94999a (glLinkProgram captures the program
//! binary; GL_NUM_PROGRAM_BINARY_FORMATS answers 1 in an ES 3.0 build).
//! The expected answers come from ES 2.0.25 and the extensions
//! GL_EXTENSIONS lists, not from the current output:
//!
//! - GL_IMPLEMENTATION_COLOR_READ_FORMAT/TYPE (ES 2.0 4.3.1, 6.1.x) is a pair
//!   glReadPixels accepts on the read buffer; RGBA/UNSIGNED_BYTE always works.
//!   Pinned for the default framebuffer and for FBOs with RGBA4, RGB5_A1 and
//!   RGB565 renderbuffers (RGBA8 only if GL_OES_rgb8_rgba8 is listed)
//!   (functional.read_pixels.*, functional.state_query.*).
//! - GL_NUM_SHADER_BINARY_FORMATS is 0 (no shader binary format is listed) and
//!   GL_SHADER_BINARY_FORMATS then writes nothing.
//! - GL_NUM_PROGRAM_BINARY_FORMATS and GL_PROGRAM_BINARY_FORMATS are
//!   GL_INVALID_ENUM unless GL_OES_get_program_binary is listed; when it is,
//!   a linked GLSL ES 1.00 program has a binary that round-trips.
//! - glLinkProgram keeps answering GL_LINK_STATUS and the info log: a normal
//!   program links with GL_LINK_STATUS 1, a failing one (a varying the
//!   fragment shader reads and the vertex shader does not write) is 0 with a
//!   non-empty log, and cannot be used.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online,report-es2 --test gc_es2ctx_drift_read_format
//! ```

#![cfg(all(feature = "host-test", feature = "gpu-vivante", feature = "shaderc-online", feature = "report-es2"))]

use core::ffi::{c_void, CStr};
use std::collections::BTreeMap;
use std::ffi::CString;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Mutex;
use yakogl::gl::consts::*;
use yakogl::gl::es3::*;
use yakogl::gl::framebuffer::*;
use yakogl::gl::program::*;
use yakogl::gl::shader::*;
use yakogl::gl::state::*;
use yakogl::gl::types::*;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};
use yakogl::vivante::kick::{GcKick, GC_KICK_MAGIC};

/* ---- the stand-in OSAL: host memory, kicks accepted and completed at once ---------- */

static BUMP: Mutex<BTreeMap<u64, u64>> = Mutex::new(BTreeMap::new());
static FENCE: AtomicI64 = AtomicI64::new(0);
/// Serialises the tests: the OSAL pointer, the back end and `g_backend` are process-global.
static SERIAL: Mutex<()> = Mutex::new(());

unsafe extern "C" fn p_gpu_alloc(heap_base: u64, size: usize, _flags: u32, out: *mut OsalGpuMem) -> i32 {
    if size == 0 || size > 1 << 30 {
        return -12;
    }
    let mem = Box::leak(vec![0u64; size.div_ceil(8)].into_boxed_slice());
    let mut b = BUMP.lock().unwrap_or_else(|e| e.into_inner());
    let at = b.entry(heap_base).or_insert(0);
    let gpu = heap_base + 0x1000 + *at;
    *at += (size as u64).div_ceil(4096) * 4096;
    let cpu = mem.as_mut_ptr() as *mut c_void;
    // SAFETY: the caller passes a writable descriptor.
    unsafe { *out = OsalGpuMem { cpu, gpu, phys: gpu, size, impl_: cpu } };
    0
}
unsafe extern "C" fn p_gpu_free(mem: *mut OsalGpuMem) -> i32 {
    // SAFETY: the caller passes a live descriptor; the memory is leaked on purpose.
    unsafe { *mem = OsalGpuMem::ZERO };
    0
}
unsafe extern "C" fn p_cache(_cpu: *mut c_void, _n: usize) {}
unsafe extern "C" fn p_submit(kick: *const c_void, len: usize) -> i64 {
    if len != core::mem::size_of::<GcKick>() {
        return -22;
    }
    // SAFETY: the back end passes a live GcKick of that size.
    let k = unsafe { *(kick as *const GcKick) };
    if k.magic != GC_KICK_MAGIC {
        return -22;
    }
    FENCE.fetch_add(1, Ordering::SeqCst) + 1
}
unsafe extern "C" fn p_wait(_fence: i64, _timeout_us: i64) -> i32 {
    0
}
unsafe extern "C" fn p_completed() -> i64 {
    FENCE.load(Ordering::SeqCst)
}
unsafe extern "C" fn p_query(what: u32, out: *mut c_void, out_sz: usize) -> i32 {
    use yakogl::osal::{OSAL_QUERY_DRAW_STATS, OSAL_QUERY_GC_FRAME_OPEN};
    if (what == OSAL_QUERY_GC_FRAME_OPEN && out_sz == 4 && !out.is_null()) || what == OSAL_QUERY_DRAW_STATS {
        return 0;
    }
    -22
}

static OSAL: OsalOps = OsalOps {
    init: None,
    shutdown: None,
    gpu_alloc: Some(p_gpu_alloc),
    gpu_free: Some(p_gpu_free),
    mmu_map: None,
    map_phys: None,
    gpu_write: None,
    gpu_read: None,
    cache_flush: Some(p_cache),
    cache_invd: Some(p_cache),
    submit: Some(p_submit),
    wait: Some(p_wait),
    completed: Some(p_completed),
    reg_read: None,
    reg_write: None,
    query: Some(p_query),
    log: None,
    wait_vblank: None,
    present: None,
    lock: None,
    unlock: None,
    thread_id: None,
    display_busy: None,
};

/// Run `case` with the Vivante back end up on the stand-in OSAL and a fresh 64x64 ES 2.0 context.
fn on_gc7000(case: impl FnOnce()) {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    // SAFETY: the guard serialises the tests; the tables are statics.
    let ctx = unsafe {
        osal_set_ops(&OSAL);
        yakogl::backend::g_backend = &yakogl::vivante::backend::yakogl_backend_vivante;
        yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
        assert_eq!(yakogl::vivante::backend::gc_init(), 0, "gc_init");
        let c = yakogl::gl::context::gl_context_create(64, 64);
        assert!(!c.is_null(), "context pool exhausted");
        while glGetError() != GL_NO_ERROR {}
        c
    };
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(case));
    // SAFETY: the context made above.
    unsafe {
        yakogl::gl::context::gl_context_destroy(ctx);
        yakogl::vivante::backend::gc_shutdown();
    }
    if let Err(e) = r {
        std::panic::resume_unwind(e);
    }
}

/* ---- GL plumbing -------------------------------------------------------------------- */

fn err() -> GLenum {
    // SAFETY: plain GL call.
    unsafe { glGetError() }
}

fn extensions() -> Vec<String> {
    // SAFETY: plain GL call; the string is NUL-terminated and static.
    let s = unsafe { CStr::from_ptr(glGetString(GL_EXTENSIONS) as *const _) };
    s.to_str().unwrap().split(' ').filter(|e| !e.is_empty()).map(String::from).collect()
}

fn listed(name: &str) -> bool {
    extensions().iter().any(|e| e == name)
}

/// glGetIntegerv into a sentinel-filled array: (error, values written).
fn geti(pname: GLenum) -> (GLenum, Vec<GLint>) {
    const UNSET: GLint = 0x5a5a_5a5a;
    let mut v = [UNSET; 8];
    // SAFETY: plain GL call; eight slots.
    unsafe { glGetIntegerv(pname, v.as_mut_ptr()) };
    let e = err();
    (e, v.iter().copied().take_while(|&x| x != UNSET).collect())
}

fn shader(ty: GLenum, text: &str) -> GLuint {
    let text = CString::new(text).unwrap();
    // SAFETY: plain GL calls; one NUL-terminated source.
    unsafe {
        let s = glCreateShader(ty);
        let p = [text.as_ptr()];
        glShaderSource(s, 1, p.as_ptr(), core::ptr::null());
        glCompileShader(s);
        let mut ok: GLint = 0;
        glGetShaderiv(s, GL_COMPILE_STATUS, &mut ok);
        assert_eq!(ok, 1, "shader compiles:\n{text:?}");
        s
    }
}

/// The program of `vs`/`fs` after glLinkProgram: (name, GL_LINK_STATUS, info log).
fn link(vs: &str, fs: &str) -> (GLuint, GLint, String) {
    let (v, f) = (shader(GL_VERTEX_SHADER, vs), shader(GL_FRAGMENT_SHADER, fs));
    // SAFETY: plain GL calls; the log buffer is 512 bytes.
    unsafe {
        let p = glCreateProgram();
        glAttachShader(p, v);
        glAttachShader(p, f);
        glLinkProgram(p);
        let mut ok: GLint = -1;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        let mut buf = [0u8; 512];
        let mut n: GLsizei = 0;
        glGetProgramInfoLog(p, 512, &mut n, buf.as_mut_ptr() as *mut _);
        (p, ok, String::from_utf8_lossy(&buf[..n as usize]).into_owned())
    }
}

const VS100: &str = "attribute vec4 p;\nvarying vec4 v;\nvoid main() { v = p; gl_Position = p; }\n";
const FS100: &str = "precision mediump float;\nvarying vec4 v;\nvoid main() { gl_FragColor = v; }\n";
/* reads a varying the vertex shader does not write: ES 2.0 2.10.4 link error */
const FS_BAD: &str = "precision mediump float;\nvarying vec4 nowhere;\nvoid main() { gl_FragColor = nowhere; }\n";

/* ---- the implementation read format ------------------------------------------------- */

fn read_pair() -> (GLint, GLint) {
    let (mut f, mut t) = (0, 0);
    // SAFETY: plain GL calls; one slot each.
    unsafe {
        glGetIntegerv(GL_IMPLEMENTATION_COLOR_READ_FORMAT, &mut f);
        glGetIntegerv(GL_IMPLEMENTATION_COLOR_READ_TYPE, &mut t);
    }
    assert_eq!(err(), GL_NO_ERROR, "the read format/type queries are ES 2.0 state");
    (f, t)
}

fn read_ok(f: GLenum, t: GLenum) -> bool {
    let mut px = [0u8; 8 * 8 * 8];
    // SAFETY: plain GL call; 8x8 pixels of at most 8 bytes each.
    unsafe { glReadPixels(0, 0, 8, 8, f, t, px.as_mut_ptr() as *mut c_void) };
    err() == GL_NO_ERROR
}

/// The pair is one of ES 2.0's (table 3.4: RGBA or RGB with a type that fits
/// it) and glReadPixels takes it on the current read buffer; RGBA/UNSIGNED_BYTE too.
fn pin_read_format(what: &str) {
    let (f, t) = read_pair();
    let (f, t) = (f as GLenum, t as GLenum);
    assert!(
        matches!(
            (f, t),
            (GL_RGBA, GL_UNSIGNED_BYTE)
                | (GL_RGBA, GL_UNSIGNED_SHORT_4_4_4_4)
                | (GL_RGBA, GL_UNSIGNED_SHORT_5_5_5_1)
                | (GL_RGB, GL_UNSIGNED_BYTE)
                | (GL_RGB, GL_UNSIGNED_SHORT_5_6_5)
        ),
        "{what}: 0x{f:x}/0x{t:x} is not an ES 2.0 read pair"
    );
    assert!(read_ok(f, t), "{what}: glReadPixels refuses the implementation pair 0x{f:x}/0x{t:x}");
    assert!(read_ok(GL_RGBA, GL_UNSIGNED_BYTE), "{what}: RGBA/UNSIGNED_BYTE always works");
}

fn rbo_fbo(format: GLenum) -> (GLuint, GLuint, GLenum) {
    // SAFETY: plain GL calls on the one context.
    unsafe {
        let (mut rb, mut fb) = (0, 0);
        glGenRenderbuffers(1, &mut rb);
        glBindRenderbuffer(GL_RENDERBUFFER, rb);
        glRenderbufferStorage(GL_RENDERBUFFER, format, 16, 16);
        let e = err();
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_RENDERBUFFER, rb);
        assert_eq!(e, GL_NO_ERROR, "storage 0x{format:x}");
        (rb, fb, glCheckFramebufferStatus(GL_FRAMEBUFFER))
    }
}

#[test]
fn read_format_default_framebuffer() {
    on_gc7000(|| {
        pin_read_format("default framebuffer");
        assert_eq!(read_pair(), (GL_RGBA as GLint, GL_UNSIGNED_BYTE as GLint));
    });
}

#[test]
fn read_format_fbo_colour_formats() {
    on_gc7000(|| {
        let mut formats = vec![GL_RGBA4, GL_RGB5_A1, GL_RGB565];
        if listed("GL_OES_rgb8_rgba8") {
            formats.push(GL_RGBA8_OES);
        }
        for f in formats {
            let (rb, fb, status) = rbo_fbo(f);
            // SAFETY: plain GL calls.
            unsafe {
                // ES 2.0 4.4.5: RGBA4, RGB5_A1 and RGB565 are colour-renderable
                assert_eq!(status, GL_FRAMEBUFFER_COMPLETE, "0x{f:x} renderbuffer FBO");
                pin_read_format(&format!("FBO 0x{f:x}"));
                glBindFramebuffer(GL_FRAMEBUFFER, 0);
                glDeleteFramebuffers(1, &fb);
                glDeleteRenderbuffers(1, &rb);
                assert_eq!(err(), GL_NO_ERROR);
            }
        }
        pin_read_format("default framebuffer again");
    });
}

#[test]
fn rgb10_a2_is_no_es2_format() {
    on_gc7000(|| {
        // 654b0620's RGB10_A2 pair belongs to ES 3.0: ES 2.0 has no such storage
        // and glReadPixels refuses the packed type.
        // SAFETY: plain GL calls.
        unsafe {
            let mut rb = 0;
            glGenRenderbuffers(1, &mut rb);
            glBindRenderbuffer(GL_RENDERBUFFER, rb);
            glRenderbufferStorage(GL_RENDERBUFFER, GL_RGB10_A2, 16, 16);
            assert_eq!(err(), GL_INVALID_ENUM, "RGB10_A2 is not an ES 2.0 renderbuffer format");
            assert!(!read_ok(GL_RGBA, GL_UNSIGNED_INT_2_10_10_10_REV));
            let (f, t) = read_pair();
            assert_ne!(t as GLenum, GL_UNSIGNED_INT_2_10_10_10_REV, "format 0x{f:x}");
        }
    });
}

/* ---- binary formats ----------------------------------------------------------------- */

#[test]
fn shader_binary_formats_are_none() {
    on_gc7000(|| {
        // no shader binary extension is listed, so the count is 0 and the list is empty
        assert!(!extensions().iter().any(|e| e.contains("shader_binary")), "{:?}", extensions());
        assert_eq!(geti(GL_NUM_SHADER_BINARY_FORMATS), (GL_NO_ERROR, vec![0]));
        let mut c: GLboolean = 0;
        // SAFETY: plain GL call.
        unsafe { glGetBooleanv(GL_SHADER_COMPILER, &mut c) };
        assert_eq!((c, err()), (1, GL_NO_ERROR), "an online compiler is there");
    });
}

/// ES 2.0.25 6.1.3 / table 6.x: GL_SHADER_BINARY_FORMATS is core state, a list
/// of GL_NUM_SHADER_BINARY_FORMATS (here 0) values: glGetIntegerv writes nothing
/// and sets no error. DEFECT: `get_integerv` (src/gl/state.rs) has no arm for
/// 0x8DF8 and answers GL_INVALID_ENUM (1280). Ignored until fixed.
#[test]
#[ignore = "defect: glGetIntegerv(GL_SHADER_BINARY_FORMATS) is GL_INVALID_ENUM, ES 2.0 says an empty list and no error"]
fn shader_binary_formats_list_is_empty() {
    on_gc7000(|| {
        assert_eq!(geti(GL_SHADER_BINARY_FORMATS), (GL_NO_ERROR, vec![]));
    });
}

#[test]
fn program_binary_formats_follow_the_extension() {
    on_gc7000(|| {
        if !listed("GL_OES_get_program_binary") {
            // the enums exist only with the extension: INVALID_ENUM, nothing written
            assert_eq!(geti(GL_NUM_PROGRAM_BINARY_FORMATS), (GL_INVALID_ENUM, vec![]));
            assert_eq!(geti(GL_PROGRAM_BINARY_FORMATS), (GL_INVALID_ENUM, vec![]));
            let (p, ok, _) = link(VS100, FS100);
            assert_eq!(ok, 1);
            let mut n: GLint = 0x5a5a;
            // SAFETY: plain GL call.
            unsafe { glGetProgramiv(p, GL_PROGRAM_BINARY_LENGTH_OES, &mut n) };
            assert_eq!((n, err()), (0x5a5a, GL_INVALID_ENUM), "GL_PROGRAM_BINARY_LENGTH_OES needs the extension");
            return;
        }
        // GL_OES_get_program_binary: glGetProgramBinaryOES and glProgramBinaryOES are
        // the entry points glGetProgramBinary and glProgramBinary here
        let (e, nfmt) = geti(GL_NUM_PROGRAM_BINARY_FORMATS_OES);
        assert_eq!(e, GL_NO_ERROR);
        assert!(nfmt[0] >= 1);
        let (e, fmts) = geti(GL_PROGRAM_BINARY_FORMATS_OES);
        assert_eq!((e, fmts.len() as GLint), (GL_NO_ERROR, nfmt[0]));
        let (p, ok, _) = link(VS100, FS100);
        assert_eq!(ok, 1);
        // SAFETY: plain GL calls; the buffer is as long as the length asked.
        unsafe {
            let mut n: GLint = 0;
            glGetProgramiv(p, GL_PROGRAM_BINARY_LENGTH_OES, &mut n);
            assert!(n > 0 && err() == GL_NO_ERROR);
            let mut buf = vec![0u8; n as usize];
            let (mut got, mut fmt): (GLsizei, GLenum) = (0, 0);
            glGetProgramBinary(p, n, &mut got, &mut fmt, buf.as_mut_ptr() as *mut c_void);
            assert_eq!((got, err()), (n, GL_NO_ERROR));
            assert!(fmts.contains(&(fmt as GLint)));
            let q = glCreateProgram();
            glProgramBinary(q, fmt, buf.as_ptr() as *const c_void, n);
            let mut lk: GLint = 0;
            glGetProgramiv(q, GL_LINK_STATUS, &mut lk);
            assert_eq!((lk, err()), (1, GL_NO_ERROR), "the binary links again");
        }
    });
}

#[test]
fn link_status_and_log() {
    on_gc7000(|| {
        let (p, ok, log) = link(VS100, FS100);
        assert_eq!(ok, 1, "a normal program links: {log}");
        assert_eq!(err(), GL_NO_ERROR);

        let (b, ok, log) = link(VS100, FS_BAD);
        assert_eq!(ok, 0, "a fragment varying nothing writes is a link error (ES 2.0 2.10.4)");
        assert!(!log.is_empty(), "the failed link leaves an info log");
        assert_eq!(err(), GL_NO_ERROR, "a failed link is no GL error");
        // a failed program cannot be used (ES 2.0 2.10.3), a good one can
        // SAFETY: plain GL calls.
        unsafe {
            glUseProgram(b);
            assert_eq!(err(), GL_INVALID_OPERATION);
            glUseProgram(p);
            assert_eq!(err(), GL_NO_ERROR);
            glUseProgram(0);
            glLinkProgram(b);
            let mut ok2: GLint = -1;
            glGetProgramiv(b, GL_LINK_STATUS, &mut ok2);
            assert_eq!((ok2, err()), (0, GL_NO_ERROR));
        }
    });
}
