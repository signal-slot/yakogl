// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! An inactive attribute takes no location on the Mali back end (#11307).
//!
//! An attribute the compiler and linker find is never read is not active
//! (ES 2.0 section 2.10.4): glGetAttribLocation answers -1 for it,
//! glGetActiveAttrib and GL_ACTIVE_ATTRIBUTES leave it out, and it takes none
//! of the 16 locations, so the active attributes sit where they were bound or
//! where they would sit without it. dEQP-GLES2
//! attribute_location.bind_aliasing.max_inactive_<type> relies on it: the
//! NanoPC-T6 tree at 8ff01055 failed its 7 cases at link with "attribute
//! 'a_8' has location 16, beyond the 16 vertex inputs" (float, vec2, vec3 and
//! vec4; 'a_4' at 16 for mat2, 'a_3' at 18 for mat3, 'a_2' at 16 for mat4;
//! docs/es2-cts-failures.md M.1 row 18). This test links those shaders and
//! smaller ones of the same kind through the GL entry points, into the real
//! Mali back end over a mock of its OSAL.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_attr_inactive_loc
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

/// Link `vs` + `fs` with `binds` and return the program.
fn link(label: &str, vs: &str, fs: &str, binds: &[(&str, GLuint)]) -> GLuint {
    let (v, f) = (compile(GL_VERTEX_SHADER, vs), compile(GL_FRAGMENT_SHADER, fs));
    // SAFETY: plain GL calls; the names are NUL-terminated.
    unsafe {
        let p = glCreateProgram();
        glAttachShader(p, v);
        glAttachShader(p, f);
        for (n, l) in binds {
            let n = CString::new(*n).unwrap();
            glBindAttribLocation(p, *l, n.as_ptr());
        }
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        let log = LOG.lock().unwrap_or_else(|e| e.into_inner()).clone();
        assert_eq!(ok, 1, "{label}: the program does not link; log {log:?}");
        assert_eq!(glGetError(), GL_NO_ERROR, "{label}: GL error after the link");
        p
    }
}

fn location(p: GLuint, name: &str) -> GLint {
    let name = CString::new(name).unwrap();
    // SAFETY: plain GL call; the name is NUL-terminated.
    unsafe { glGetAttribLocation(p, name.as_ptr()) }
}

/// `(name, type, location)` of each active attribute, sorted by name: the
/// GL_ACTIVE_ATTRIBUTES entries of glGetActiveAttrib.
fn active(p: GLuint) -> Vec<(String, GLenum, GLint)> {
    // SAFETY: plain GL calls; `name` holds 64 bytes.
    let mut v: Vec<_> = unsafe {
        let mut n: GLint = 0;
        glGetProgramiv(p, GL_ACTIVE_ATTRIBUTES, &mut n);
        (0..n as GLuint)
            .map(|i| {
                let (mut len, mut size, mut ty) = (0, 0, 0);
                let mut name = [0 as c_char; 64];
                glGetActiveAttrib(p, i, 64, &mut len, &mut size, &mut ty, name.as_mut_ptr());
                assert_eq!(glGetError(), GL_NO_ERROR, "glGetActiveAttrib({i})");
                assert_eq!(size, 1, "attribute {i}");
                let l = glGetAttribLocation(p, name.as_ptr());
                (CStr::from_ptr(name.as_ptr()).to_string_lossy().into_owned(), ty, l)
            })
            .collect()
    };
    v.sort();
    v
}

fn want(n: &[(&str, GLenum, GLint)]) -> Vec<(String, GLenum, GLint)> {
    let mut v: Vec<_> = n.iter().map(|(s, t, l)| (s.to_string(), *t, *l)).collect();
    v.sort();
    v
}

/// Draw one triangle with `p`, position array at `pos` and colour array at
/// `color`, and assert it drew.
fn draw(label: &str, p: GLuint, pos: GLuint, color: GLuint) {
    let before = backend::draws_refused();
    // SAFETY: plain GL calls; `FULL` and `ATTR` are static and outlive the draw.
    let e = unsafe {
        glUseProgram(p);
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

/* ---- the dEQP cases' shaders -------------------------------------------- */

/// The types of the 7 cases: name, locations taken, glGetActiveAttrib type.
const TYPES: [(&str, u32, GLenum); 7] = [
    ("float", 1, GL_FLOAT),
    ("vec2", 1, GL_FLOAT_VEC2),
    ("vec3", 1, GL_FLOAT_VEC3),
    ("vec4", 1, GL_FLOAT_VEC4),
    ("mat2", 2, GL_FLOAT_MAT2),
    ("mat3", 3, GL_FLOAT_MAT3),
    ("mat4", 4, GL_FLOAT_MAT4),
];

/// One attribute of a dEQP shader: name, the location the case binds it to,
/// whether the shader reads it.
type Attr = (String, GLuint, bool);

/// dEQP-GLES2 attribute_location.bind_aliasing.max_inactive_<type>'s vertex
/// shader (glsAttributeLocationTests.cpp, `BindInactiveAliasingAttributeTest`):
/// for each location run of the type, from the top down, `a_k` read under
/// `if (u_A != 0.0)` and `a_(k+16)` under `if (0 != 0)`, both bound to the
/// run. 32 declared attributes for the one-location types, 16 locations.
fn max_inactive(ty: &str, columns: u32) -> (String, Vec<Attr>) {
    let to_vec4 = |n: &str| match ty {
        "vec2" => format!("vec4({n}.xy, {n}.yx)"),
        "vec3" => format!("vec4({n}.xyz, {n}.x)"),
        _ => format!("vec4({n})"),
    };
    let mut attrs = Vec::new();
    let mut loc = 16 - columns as i32;
    let mut k = 0;
    while loc >= 0 {
        attrs.push((format!("a_{k}"), loc as GLuint, true));
        attrs.push((format!("a_{}", k + 16), loc as GLuint, false));
        loc -= columns as i32;
        k += 1;
    }
    let mut s = String::from("#version 100\nvarying mediump vec4 v_color;\n");
    for (n, _, _) in &attrs {
        s += &format!("attribute mediump {ty} {n};\n");
    }
    s += "\nuniform mediump float u_A;\n\nvoid main (void)\n{\n\tmediump vec4 color = vec4(0.0);\n\n";
    for (n, _, active) in &attrs {
        let cond = if *active { "u_A != 0.0" } else { "0 != 0" };
        s += &format!("\tif ({cond})\n\t{{\n\t\tcolor += {};\n\t}}\n", to_vec4(n));
    }
    s += "\n\tv_color = color;\n\tgl_Position = color;\n}\n";
    (s, attrs)
}

/// The fragment shader of the attribute_location cases.
const DEQP_FS: &str =
    "#version 100\n\nvarying mediump vec4 v_color;\nvoid main (void)\n{\n\tgl_FragColor = v_color;\n}\n";

/// The 7 cases as dEQP runs them: every pair bound to one run, the link
/// passes, the attribute under `if (0 != 0)` has no location and is not
/// listed, and the read one is listed at its binding with its type.
/// The 8ff01055 tree refused the link (the header's line).
#[test]
fn deqp_max_inactive_bound_pairs_link_and_the_unread_one_has_no_location() {
    let _g = context();
    for (ty, columns, gl_ty) in TYPES {
        let (vs, attrs) = max_inactive(ty, columns);
        let binds: Vec<(&str, GLuint)> = attrs.iter().map(|(n, l, _)| (n.as_str(), *l)).collect();
        let p = link(ty, &vs, DEQP_FS, &binds);
        let mut n: GLint = 0;
        // SAFETY: plain GL call.
        unsafe { glGetProgramiv(p, GL_ACTIVE_ATTRIBUTES, &mut n) };
        assert_eq!(n, 16 / columns as GLint, "{ty}: GL_ACTIVE_ATTRIBUTES");
        let read: Vec<_> = attrs.iter().filter(|a| a.2).map(|(n, l, _)| (n.as_str(), gl_ty, *l as GLint)).collect();
        assert_eq!(active(p), want(&read), "{ty}: the glGetActiveAttrib list");
        for (name, _, _) in attrs.iter().filter(|a| !a.2) {
            assert_eq!(location(p, name), -1, "{ty}: glGetAttribLocation({name})");
        }
        // SAFETY: plain GL call.
        unsafe { glDeleteProgram(p) };
    }
}

/// The same shaders with no binding: the linker picks the locations. The
/// unread attributes take none, so the read ones fit the 16 locations, each
/// on its own run.
#[test]
fn deqp_max_inactive_unbound_fits_the_16_locations() {
    let _g = context();
    for (ty, columns, gl_ty) in TYPES {
        let (vs, attrs) = max_inactive(ty, columns);
        let p = link(ty, &vs, DEQP_FS, &[]);
        let list = active(p);
        let names: Vec<&str> = list.iter().map(|a| a.0.as_str()).collect();
        let mut read: Vec<&str> = attrs.iter().filter(|a| a.2).map(|a| a.0.as_str()).collect();
        read.sort();
        assert_eq!(names, read, "{ty}: the glGetActiveAttrib list");
        let mut taken = [false; 16];
        for (name, t, l) in &list {
            assert_eq!(*t, gl_ty, "{ty}: the type of {name}");
            assert!(*l >= 0 && *l as u32 + columns <= 16, "{ty}: {name} at {l}");
            for c in 0..columns {
                let slot = &mut taken[(*l as u32 + c) as usize];
                assert!(!*slot, "{ty}: {name} at {l} shares location {}", *l as u32 + c);
                *slot = true;
            }
        }
        for (name, _, _) in attrs.iter().filter(|a| !a.2) {
            assert_eq!(location(p, name), -1, "{ty}: glGetAttribLocation({name})");
        }
        // SAFETY: plain GL call.
        unsafe { glDeleteProgram(p) };
    }
}

/* ---- smaller shapes ------------------------------------------------------ */

const FS: &str = "#version 100\nvarying mediump vec4 v_color;\nvoid main() { gl_FragColor = v_color; }\n";

/// `a_pos`, `a_dead` and `a_color` declared in this order; `body` is main's
/// statements after `gl_Position = a_pos; v_color = a_color;`.
fn three(body: &str) -> String {
    format!(
        "#version 100\nattribute mediump vec4 a_pos;\nattribute mediump vec4 a_dead;\n\
         attribute mediump vec4 a_color;\nvarying mediump vec4 v_color;\nuniform mediump float u_A;\n\
         void main() {{\n    gl_Position = a_pos;\n    v_color = a_color;\n{body}}}\n"
    )
}

/// `a_dead` is not active in `three(body)`: no location, not listed, and
/// `a_pos` and `a_color` sit at 0 and 1 as they do in the shader that does
/// not declare it; bound, they sit at their bindings and `a_dead`'s binding
/// to `a_color`'s location is no alias. Both programs draw.
fn dead_is_inactive(label: &str, body: &str) {
    let _g = context();
    let vs = three(body);
    let p = link(label, &vs, FS, &[]);
    assert_eq!(location(p, "a_dead"), -1, "{label}: glGetAttribLocation(a_dead)");
    assert_eq!(
        active(p),
        want(&[("a_pos", GL_FLOAT_VEC4, 0), ("a_color", GL_FLOAT_VEC4, 1)]),
        "{label}: the glGetActiveAttrib list"
    );
    draw(label, p, 0, 1);

    let b = link(label, &vs, FS, &[("a_pos", 5), ("a_dead", 2), ("a_color", 2)]);
    assert_eq!(location(b, "a_dead"), -1, "{label}, bound: glGetAttribLocation(a_dead)");
    assert_eq!(
        active(b),
        want(&[("a_pos", GL_FLOAT_VEC4, 5), ("a_color", GL_FLOAT_VEC4, 2)]),
        "{label}, bound: the glGetActiveAttrib list"
    );
    draw(label, b, 5, 2);
    // SAFETY: plain GL calls.
    unsafe {
        glUseProgram(0);
        glDeleteProgram(p);
        glDeleteProgram(b);
    }
}

/// Declared and never named again.
#[test]
fn a_declared_attribute_nothing_names_has_no_location() {
    dead_is_inactive("unnamed", "");
}

/// The dEQP cases' condition, one attribute.
#[test]
fn an_attribute_under_if_0_ne_0_has_no_location() {
    dead_is_inactive("0 != 0", "    if (0 != 0) { v_color += a_dead; }\n");
}

#[test]
fn an_attribute_under_if_false_has_no_location() {
    dead_is_inactive("false", "    if (false) { v_color += a_dead; }\n");
}

/// The side a constant-true condition does not take.
#[test]
fn an_attribute_in_the_else_of_if_true_has_no_location() {
    dead_is_inactive("else", "    if (true) { v_color += vec4(u_A); } else { v_color += a_dead; }\n");
}

/// The same with `1 == 1`, the mirror of the dEQP cases' `0 != 0`. Observed
/// on 7462e3d5: glCompileShader fails, "the code generator refused this
/// vertex shader: Mali(Valhall: a constant operation on or to a value that is
/// not a finite normal float is not lowered yet)", and so does the shader
/// with no `else` and no `a_dead` (`if (1 == 1) { v_color += vec4(u_A); }`):
/// the compiler's fault, before any location is given.
#[test]
#[ignore = "issue #11307: `if (1 == 1) { .. } else { v_color += a_dead; }` does not compile (observed: compile status 0, Valhall refuses a constant operation on a value that is not a finite normal float)"]
fn an_attribute_in_the_else_of_if_1_eq_1_has_no_location() {
    dead_is_inactive("else of 1 == 1", "    if (1 == 1) { v_color += vec4(u_A); } else { v_color += a_dead; }\n");
}

/// A never-taken side inside a side that is taken at run time.
#[test]
fn an_attribute_under_a_nested_never_taken_if_has_no_location() {
    dead_is_inactive("nested", "    if (u_A != 0.0) { if (0 != 0) { v_color += a_dead; } }\n");
}

/// The condition is a `const bool`, a constant expression (GLSL ES 1.00
/// section 5.10). Observed on 7462e3d5: `a_dead` is active at location 1 and
/// `a_color` moves to 2.
#[test]
#[ignore = "issue #11307: an attribute under `const bool on = false; if (on)` is active (observed: glGetAttribLocation(a_dead) = 1, not -1, and a_color at 2, not 1)"]
fn an_attribute_under_a_const_bool_condition_has_no_location() {
    dead_is_inactive("const bool", "    const bool on = false;\n    if (on) { v_color += a_dead; }\n");
}

/// The other half of the pin: the same shape with a run-time condition keeps
/// the attribute active, at the location after the one declared before it.
#[test]
fn an_attribute_under_a_run_time_condition_is_active() {
    let _g = context();
    let p = link("run time", &three("    if (u_A != 0.0) { v_color += a_dead; }\n"), FS, &[]);
    assert_eq!(
        active(p),
        want(&[("a_pos", GL_FLOAT_VEC4, 0), ("a_dead", GL_FLOAT_VEC4, 1), ("a_color", GL_FLOAT_VEC4, 2)])
    );
    // SAFETY: plain GL call.
    unsafe { glDeleteProgram(p) };
}
