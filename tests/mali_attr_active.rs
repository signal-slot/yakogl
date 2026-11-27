// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The active attributes of a program on the Mali back end (#11239).
//!
//! An attribute the vertex shader statically reads is an active attribute of
//! the program (ES 2.0 section 2.10.4), also when it feeds only a varying the
//! fragment shader does not read. gl_test case 1 (`uniform`) links such a
//! pair and asks the program for its attributes; this test asks the same
//! questions through the GL entry points, into the real Mali back end over a
//! mock of its OSAL. An attribute read only in the side of an `if` its
//! constant condition does not take is not active and takes no location
//! (#11307, dEQP-GLES2 attribute_location.bind_aliasing.max_inactive_*).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_attr_active
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
/// One vec4 per vertex.
static ATTR: [f32; 12] = [0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9, 0.3, 0.2, 0.1];

/// gl_test's pass-through vertex shader and its uniform fragment shader,
/// which does not read `v_color` (driver/tools/pvrlab/gl_test_shaders_es100.h).
const VS: &str = "#version 100\nattribute vec4 a_pos;\nattribute vec4 a_color;\nvarying vec4 v_color;\n\
    void main() {\n    gl_Position = a_pos;\n    v_color = a_color;\n}\n";
const FS: &str = "#version 100\nprecision mediump float;\nuniform vec4 u_color;\n\
    void main() { gl_FragColor = u_color; }\n";

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
        let mut log = [0 as c_char; 1024];
        glGetShaderInfoLog(s, 1024, core::ptr::null_mut(), log.as_mut_ptr());
        let log = CStr::from_ptr(log.as_ptr()).to_string_lossy();
        assert_eq!(ok, 1, "the shader does not compile: {log}\n{text}");
        s
    }
}

/// Link `vs` + `fs` with `binds` and return the program.
fn link(label: &str, vs: &str, fs: &str, binds: &[(&CStr, GLuint)]) -> GLuint {
    let (v, f) = (compile(GL_VERTEX_SHADER, vs), compile(GL_FRAGMENT_SHADER, fs));
    // SAFETY: plain GL calls.
    unsafe {
        let p = glCreateProgram();
        glAttachShader(p, v);
        glAttachShader(p, f);
        for (n, l) in binds {
            glBindAttribLocation(p, *l, n.as_ptr());
        }
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        let mut log = [0 as c_char; 1024];
        glGetProgramInfoLog(p, 1024, core::ptr::null_mut(), log.as_mut_ptr());
        let log = CStr::from_ptr(log.as_ptr()).to_string_lossy();
        assert_eq!(ok, 1, "{label}: the program does not link: {log}");
        p
    }
}

/// `(name, location)` of each active attribute, as glGetActiveAttrib lists them.
fn active(p: GLuint) -> Vec<(String, GLint)> {
    // SAFETY: plain GL calls; `name` holds 64 bytes.
    unsafe {
        let mut n: GLint = 0;
        glGetProgramiv(p, GL_ACTIVE_ATTRIBUTES, &mut n);
        (0..n as GLuint)
            .map(|i| {
                let (mut len, mut size, mut ty) = (0, 0, 0);
                let mut name = [0 as c_char; 64];
                glGetActiveAttrib(p, i, 64, &mut len, &mut size, &mut ty, name.as_mut_ptr());
                assert_eq!(glGetError(), GL_NO_ERROR, "glGetActiveAttrib({i})");
                assert_eq!((size, ty), (1, GL_FLOAT_VEC4), "attribute {i}");
                let l = glGetAttribLocation(p, name.as_ptr());
                (CStr::from_ptr(name.as_ptr()).to_string_lossy().into_owned(), l)
            })
            .collect()
    }
}

/// Draw one triangle with `p`, position array at `pos`, and assert it drew.
fn draw(label: &str, p: GLuint, pos: GLuint, color: GLuint) {
    let before = backend::draws_refused();
    // SAFETY: plain GL calls; `FULL` and `ATTR` are static and outlive the draw.
    let e = unsafe {
        glUseProgram(p);
        glUniform4f(glGetUniformLocation(p, c"u_color".as_ptr()), 1.0, 0.5, 0.0, 1.0);
        glBindBuffer(GL_ARRAY_BUFFER, 0);
        glVertexAttribPointer(pos, 4, GL_FLOAT, 0, 0, FULL.as_ptr() as *const c_void);
        glEnableVertexAttribArray(pos);
        glVertexAttribPointer(color, 4, GL_FLOAT, 0, 0, ATTR.as_ptr() as *const c_void);
        glEnableVertexAttribArray(color);
        while glGetError() != GL_NO_ERROR {}
        glDrawArrays(GL_TRIANGLES, 0, 3);
        glFinish();
        let e = glGetError();
        glDisableVertexAttribArray(pos);
        glDisableVertexAttribArray(color);
        e
    };
    let refused = backend::draws_refused() - before;
    let log = LOG.lock().unwrap_or_else(|e| e.into_inner()).clone();
    let refusals: Vec<_> = log.iter().filter(|l| l.contains("refus")).collect();
    assert_eq!(refused, 0, "{label}: the draw was refused; refusal lines {refusals:?}");
    assert_eq!(e, GL_NO_ERROR, "{label}: GL error after the draw");
}

fn both(n: &[(&str, GLint)]) -> Vec<(String, GLint)> {
    n.iter().map(|(s, l)| (s.to_string(), *l)).collect()
}

/// What gl_test case 1 asks: `a_color` feeds only `v_color`, which the
/// fragment shader does not read, and is still active at its location.
#[test]
fn an_attribute_that_feeds_an_unread_varying_is_active() {
    let _g = context();
    let p = link("plain", VS, FS, &[]);
    assert_eq!(active(p), both(&[("a_pos", 0), ("a_color", 1)]));
    // SAFETY: plain GL calls.
    assert_eq!(unsafe { glGetAttribLocation(p, c"nope".as_ptr()) }, -1);
    draw("plain", p, 0, 1);
}

/// The case's second program: `a_pos` bound to 3, `a_color` where the case
/// reads it (1).
#[test]
fn a_bound_position_keeps_the_other_attribute_active() {
    let _g = context();
    let p = link("bound", VS, FS, &[(c"a_pos", 3)]);
    assert_eq!(active(p), both(&[("a_pos", 3), ("a_color", 1)]));
    draw("bound", p, 3, 1);
}

/// When a dEQP attribute_location case reads an attribute
/// (glsAttributeLocationTests.cpp, `generateOutputCode`): under
/// `if (u_A != 0.0)`, under `if (u_A == 0.0)`, or under `if (0 != 0)`.
#[derive(Clone, Copy, PartialEq)]
enum Cond {
    A,
    NotA,
    Never,
}

/// One declared attribute of a case: name, type, the location it is bound
/// to, and when it is read.
struct Attr {
    name: String,
    ty: &'static str,
    at: GLuint,
    cond: Cond,
}

/// The vertex shader dEQP's `generateVertexShaderTemplate` writes for
/// `attrs` (ES 1.00).
fn deqp_vs(attrs: &[Attr]) -> String {
    let to_vec4 = |a: &Attr| match a.ty {
        "vec2" => format!("vec4({n}.xy, {n}.yx)", n = a.name),
        "vec3" => format!("vec4({n}.xyz, {n}.x)", n = a.name),
        _ => format!("vec4({})", a.name),
    };
    let mut s = String::from("#version 100\nvarying mediump vec4 v_color;\n");
    for a in attrs {
        s += &format!("attribute mediump {} {};\n", a.ty, a.name);
    }
    s += "\n";
    if attrs.iter().any(|a| a.cond != Cond::Never) {
        s += "uniform mediump float u_A;\n";
    }
    s += "\nvoid main (void)\n{\n\tmediump vec4 color = vec4(0.0);\n\n";
    for a in attrs {
        let cond = match a.cond {
            Cond::A => "u_A != 0.0",
            Cond::NotA => "u_A == 0.0",
            Cond::Never => "0 != 0",
        };
        s += &format!("\tif ({cond})\n\t{{\n\t\tcolor += {};\n\t}}\n", to_vec4(a));
    }
    s += "\n\tv_color = color;\n\tgl_Position = color;\n}\n";
    s
}

/// The location runs of `ty` dEQP's `BindMaxAliasingAttributeTest` and
/// `BindInactiveAliasingAttributeTest` use: from the top of the 16 down,
/// `a_k` and `a_(k+16)` bound to the k-th run, `a_k` read when `first`
/// holds and `a_(k+16)` when `second` does. 32 declared names at most, on 16
/// locations.
fn max_pairs(ty: &'static str, columns: u32, first: Cond, second: Cond) -> Vec<Attr> {
    let mut attrs = Vec::new();
    let mut loc = 16 - columns as i32;
    let mut k = 0;
    while loc >= 0 {
        attrs.push(Attr { name: format!("a_{k}"), ty, at: loc as GLuint, cond: first });
        attrs.push(Attr { name: format!("a_{}", k + 16), ty, at: loc as GLuint, cond: second });
        loc -= columns as i32;
        k += 1;
    }
    attrs
}

/// The types of the attribute_location cases and their columns.
const TYPES: [(&str, u32); 7] =
    [("float", 1), ("vec2", 1), ("vec3", 1), ("vec4", 1), ("mat2", 2), ("mat3", 3), ("mat4", 4)];

/// Link `attrs`' vertex shader with dEQP's fragment shader and their
/// bindings, as dEQP's `runTest` does.
fn link_bound(label: &str, attrs: &[Attr]) -> GLuint {
    let names: Vec<CString> = attrs.iter().map(|a| CString::new(a.name.as_str()).unwrap()).collect();
    let binds: Vec<(&CStr, GLuint)> = names.iter().zip(attrs).map(|(n, a)| (n.as_c_str(), a.at)).collect();
    link(label, &deqp_vs(attrs), DEQP_FS, &binds)
}

/// The fragment shader of the attribute_location cases.
const DEQP_FS: &str =
    "#version 100\n\nvarying mediump vec4 v_color;\nvoid main (void)\n{\n\tgl_FragColor = v_color;\n}\n";

/// An attribute read only under `if (0 != 0)` is not active: it takes no
/// location, so binding it to an active one's location is no alias and the
/// 16 active ones fit the 16 locations (#11307). What the dEQP case checks:
/// the link passes and every active attribute is listed at its binding.
#[test]
fn an_attribute_read_only_in_a_never_taken_branch_takes_no_location() {
    let _g = context();
    for (ty, columns) in TYPES {
        let attrs = max_pairs(ty, columns, Cond::A, Cond::Never);
        let p = link_bound(ty, &attrs);
        // SAFETY: plain GL calls; the names are NUL-terminated.
        unsafe {
            let mut n: GLint = 0;
            glGetProgramiv(p, GL_ACTIVE_ATTRIBUTES, &mut n);
            assert_eq!(n, 16 / columns as GLint, "{ty}: the active attributes");
            for a in &attrs {
                let name = CString::new(a.name.as_str()).unwrap();
                let l = glGetAttribLocation(p, name.as_ptr());
                let want = if a.cond == Cond::A { a.at as GLint } else { -1 };
                assert_eq!(l, want, "{ty}: glGetAttribLocation({name:?})");
            }
            glDeleteProgram(p);
        }
    }
}

/// dEQP-GLES2 attribute_location.bind_aliasing.cond_<type> and
/// cond_<type>_offset_1 (`BindAliasingAttributeTest`): `a_0` of the type
/// read when `u_A != 0.0`, a vec4 `a_1` read when `u_A == 0.0`, `a_0` bound
/// to 1 and `a_1` to 1 + offset; and max_cond_<type>
/// (`BindMaxAliasingAttributeTest`): the 16 locations taken twice over,
/// `a_k` read when `u_A != 0.0` and `a_(k+16)` when `u_A == 0.0` (#11388).
/// Both names of a location are active (ES 2.0 section 2.10.4 allows the
/// alias when no path reads both): the link passes, every attribute is
/// listed at its binding, and a draw with each side of `u_A` is not refused.
#[test]
fn attributes_bound_to_one_location_link_and_draw() {
    let _g = context();
    let mut cases = Vec::new();
    for (ty, columns) in TYPES {
        let offsets: &[GLuint] = if columns > 1 { &[0, 1] } else { &[0] };
        for &offset in offsets {
            let attrs = vec![
                Attr { name: "a_0".into(), ty, at: 1, cond: Cond::A },
                Attr { name: "a_1".into(), ty: "vec4", at: 1 + offset, cond: Cond::NotA },
            ];
            let label = if offset == 0 { format!("cond_{ty}") } else { format!("cond_{ty}_offset_{offset}") };
            cases.push((label, columns, attrs));
        }
        cases.push((format!("max_cond_{ty}"), columns, max_pairs(ty, columns, Cond::A, Cond::NotA)));
    }
    let failed: Vec<String> = cases
        .iter()
        .filter_map(|(label, columns, attrs)| {
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| link_and_draw(label, *columns, attrs)))
                .err()
                .map(|e| e.downcast_ref::<String>().cloned().unwrap_or_else(|| format!("{label}: failed")))
        })
        .collect();
    assert!(failed.is_empty(), "{} of {} cases:\n{}", failed.len(), cases.len(), failed.join("\n"));
}

/// One case of [`attributes_bound_to_one_location_link_and_draw`].
fn link_and_draw(label: &str, columns: u32, attrs: &[Attr]) {
    {
        let p = link_bound(label, attrs);
        // SAFETY: plain GL calls; the names are NUL-terminated; `FULL` is
        // static and outlives the draws.
        unsafe {
            let mut n: GLint = 0;
            glGetProgramiv(p, GL_ACTIVE_ATTRIBUTES, &mut n);
            assert_eq!(n, attrs.len() as GLint, "{label}: the active attributes");
            for a in attrs {
                let name = CString::new(a.name.as_str()).unwrap();
                assert_eq!(glGetAttribLocation(p, name.as_ptr()), a.at as GLint, "{label}: {name:?}");
            }
            /* an array at every column of every binding */
            let mut used = [false; 16];
            for a in attrs {
                let cols = if a.ty == "vec4" { 1 } else { columns };
                for c in a.at..a.at + cols {
                    used[c as usize] = true;
                }
            }
            glUseProgram(p);
            glBindBuffer(GL_ARRAY_BUFFER, 0);
            for (l, _) in used.iter().enumerate().filter(|(_, u)| **u) {
                glVertexAttribPointer(l as GLuint, 4, GL_FLOAT, 0, 0, FULL.as_ptr() as *const c_void);
                glEnableVertexAttribArray(l as GLuint);
            }
            for side in [1.0, 0.0] {
                glUniform1f(glGetUniformLocation(p, c"u_A".as_ptr()), side);
                while glGetError() != GL_NO_ERROR {}
                let before = backend::draws_refused();
                LOG.lock().unwrap_or_else(|e| e.into_inner()).clear();
                glDrawArrays(GL_TRIANGLES, 0, 3);
                glFinish();
                let refused = backend::draws_refused() - before;
                let log = LOG.lock().unwrap_or_else(|e| e.into_inner()).clone();
                let lines: Vec<_> = log.iter().filter(|l| l.contains("refus")).collect();
                assert_eq!(refused, 0, "{label}, u_A = {side}: the draw was refused; {lines:?}");
                assert_eq!(glGetError(), GL_NO_ERROR, "{label}, u_A = {side}: GL error after the draw");
            }
            for (l, _) in used.iter().enumerate().filter(|(_, u)| **u) {
                glDisableVertexAttribArray(l as GLuint);
            }
            glDeleteProgram(p);
        }
    }
}
