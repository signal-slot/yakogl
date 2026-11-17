// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Draws with an unread or inactive attribute between the attributes a vertex
//! shader reads, on the Mali back end (plan #373, `inv-attrgap`).
//!
//! The back end refuses such draws as `NOT_BUILT_ATTRIBUTE_RECORDS`
//! (src/mali/backend.rs, `vertex_attributes`): the frame's attribute table is
//! indexed by record 0..n and a program whose records skip a location is not
//! built. dEQP-GLES2 meets it as a `mat2`/`mat3` attribute with an unread
//! column (shaders.matrix, shaders.random, default_vertex_attrib, ...). The
//! unit pins are in src/mali/backend_tests.rs; this test links GLSL ES 1.00
//! pairs through `glLinkProgram` and draws one triangle of each into an 8x8
//! RGBA8 framebuffer object through `glDrawArrays`, into the real Mali back
//! end over a mock of its OSAL. Each case asserts `GL_NO_ERROR` after the
//! draw, that the draw was not refused (`draws_refused`) and that the OSAL
//! log holds no refusal line. No pixels are checked.
//!
//! Cases that draw today are active: the control, and an unread attribute
//! last or between read ones (the compiler already numbers those records
//! 0..n). The cases that need the fix are `#[ignore]`d with the reason: a
//! `mat2`/`mat3` column read beyond column 0, whose location the program's
//! reflection does not list as an attribute (the loop over `sc.attributes` in
//! src/shaderc/online/mali/mod.rs skips it). The item `attrgap-fix` removes
//! the ignores.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_attr_gap
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_attr_gap -- --ignored
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
use yakogl::gl::vertex::{
    glDisableVertexAttribArray, glEnableVertexAttribArray, glVertexAttrib4f, glVertexAttribPointer,
};
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
/// One vec4 per vertex.
static ATTR: [f32; 12] = [0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9, 0.3, 0.2, 0.1];

const FS_C: &str = "precision mediump float;\nvarying vec4 v_c;\nvoid main() { gl_FragColor = v_c; }\n";

fn compile(ty: GLenum, text: &str) -> GLuint {
    let src = CString::new(text).unwrap();
    // SAFETY: plain GL calls; one NUL-terminated source.
    unsafe {
        let s = glCreateShader(ty);
        let p: [*const c_char; 1] = [src.as_ptr()];
        glShaderSource(s, 1, p.as_ptr(), core::ptr::null());
        glCompileShader(s);
        let mut ok: GLint = 0;
        glGetShaderiv(s, GL_COMPILE_STATUS, &mut ok);
        assert_eq!(ok, 1, "the shader does not compile:\n{text}");
        s
    }
}

/// What a case binds and enables.
struct Case<'a> {
    vs: &'a str,
    /// `(name, location)` pairs for `glBindAttribLocation`.
    binds: &'a [(&'a CStr, GLuint)],
    /// Locations fed from a client array (4 floats: `FULL` at 0, else `ATTR`).
    arrays: &'a [GLuint],
    /// Locations given a default value with `glVertexAttrib4f`.
    defaults: &'a [GLuint],
}

/// Link, draw one triangle, and assert it drew unrefused.
fn draw(label: &str, c: &Case) {
    let (v, f) = (compile(GL_VERTEX_SHADER, c.vs), compile(GL_FRAGMENT_SHADER, FS_C));
    let before = backend::draws_refused();
    // SAFETY: plain GL calls; `FULL` and `ATTR` are static and outlive the draw.
    let e = unsafe {
        let p = glCreateProgram();
        glAttachShader(p, v);
        glAttachShader(p, f);
        for (n, l) in c.binds {
            glBindAttribLocation(p, *l, n.as_ptr());
        }
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        assert_eq!(ok, 1, "{label}: the program does not link");
        glUseProgram(p);
        glBindBuffer(GL_ARRAY_BUFFER, 0);
        for l in c.arrays {
            let data = if *l == 0 { &FULL } else { &ATTR };
            glVertexAttribPointer(*l, 4, GL_FLOAT, 0, 0, data.as_ptr() as *const c_void);
            glEnableVertexAttribArray(*l);
        }
        for l in c.defaults {
            glVertexAttrib4f(*l, 0.25, 0.5, 0.75, 1.0);
        }
        while glGetError() != GL_NO_ERROR {}
        glDrawArrays(GL_TRIANGLES, 0, 3);
        glFinish();
        let e = glGetError();
        for l in c.arrays {
            glDisableVertexAttribArray(*l);
        }
        e
    };
    let refused = backend::draws_refused() - before;
    let log = LOG.lock().unwrap_or_else(|e| e.into_inner()).clone();
    let refusals: Vec<_> = log.iter().filter(|l| l.contains("refus")).collect();
    assert_eq!(refused, 0, "{label}: the draw was refused; refusal lines {refusals:?}");
    assert!(refusals.is_empty(), "{label}: refusal lines {refusals:?}");
    assert_eq!(e, GL_NO_ERROR, "{label}: glGetError 0x{e:x}");
}

/* ---- the cases ------------------------------------------------------------ */

/// Control, no gap: `a_pos` at 0 and `a_attr` at 1, both read.
#[test]
fn control_no_gap_draws() {
    let _g = context();
    draw(
        "control",
        &Case {
            vs: "attribute vec4 a_pos;\nattribute vec4 a_attr;\nvarying vec4 v_c;\n\
                 void main() { gl_Position = a_pos; v_c = a_attr; }\n",
            binds: &[(c"a_pos", 0), (c"a_attr", 1)],
            arrays: &[0, 1],
            defaults: &[],
        },
    );
}

/// An attribute declared, unread, and last: nothing follows it, so the read
/// records are 0..n.
#[test]
fn unread_attribute_last_draws() {
    let _g = context();
    draw(
        "unread last",
        &Case {
            vs: "attribute vec4 a_pos;\nattribute vec4 a_attr;\nattribute vec4 a_unused;\nvarying vec4 v_c;\n\
                 void main() { gl_Position = a_pos; v_c = a_attr; }\n",
            binds: &[(c"a_pos", 0), (c"a_attr", 1), (c"a_unused", 2)],
            arrays: &[0, 1],
            defaults: &[],
        },
    );
}

/// (1) `a0` and `a2` read, `a1` declared and unread between them. Draws
/// today: the compiler numbers the records of the read locations 0..n
/// (Mesa's `vsattr_between`).
#[test]
fn unread_attribute_between_read_ones_draws() {
    let _g = context();
    draw(
        "a0 a1(unused) a2",
        &Case {
            vs: "attribute vec4 a0;\nattribute vec4 a1;\nattribute vec4 a2;\nvarying vec4 v_c;\n\
                 void main() { gl_Position = a0; v_c = a2; }\n",
            binds: &[(c"a0", 0), (c"a1", 1), (c"a2", 2)],
            arrays: &[0, 2],
            defaults: &[],
        },
    );
}

/// (2) A `mat2` attribute (locations 1 and 2) of which only column 1 is read
/// (Mesa's `vsattr_matcol`): column 0's location is the unread slot before it;
/// a third attribute follows at 3.
#[test]
#[ignore = "needs attrgap-fix (NOT_BUILT_ATTRIBUTE_RECORDS); attrgap-fix removes this ignore"]
fn mat2_attribute_one_column_read_draws() {
    let _g = context();
    draw(
        "mat2 one column",
        &Case {
            vs: "attribute vec4 a_pos;\nattribute mat2 a_m;\nattribute vec4 a_tail;\nvarying vec4 v_c;\n\
                 void main() { gl_Position = a_pos; v_c = vec4(a_m[1], a_tail.xy); }\n",
            binds: &[(c"a_pos", 0), (c"a_m", 1), (c"a_tail", 3)],
            arrays: &[0, 1, 3],
            defaults: &[],
        },
    );
}

/// (3) A `mat3` attribute (locations 1..3), no array, a default value set
/// with `glVertexAttrib4f` on each column; only column 2 is read, columns 0
/// and 1 are the unread slots before it.
#[test]
#[ignore = "needs attrgap-fix (NOT_BUILT_ATTRIBUTE_RECORDS); attrgap-fix removes this ignore"]
fn mat3_attribute_default_value_draws() {
    let _g = context();
    draw(
        "mat3 default",
        &Case {
            vs: "attribute vec4 a_pos;\nattribute mat3 a_m;\nvarying vec4 v_c;\n\
                 void main() { gl_Position = a_pos; v_c = vec4(a_m[2], 1.0); }\n",
            binds: &[(c"a_pos", 0), (c"a_m", 1)],
            arrays: &[0],
            defaults: &[1, 2, 3],
        },
    );
}
