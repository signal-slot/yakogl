// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! ES 2.0 context: the link outcomes dEQP-GLES2 checks keep their answers
//! (guard for 830858db, "glLinkProgram links through link_with(LinkInputs),
//! tf link_resolve takes the request", and ae96bfc7, "glLinkProgram prints a
//! failed link's info log on the console, once").
//!
//! 830858db split glLinkProgram into `link_inputs` (the attached shader
//! objects), `link_reset` and `link_with`; ae96bfc7 moved the console print of
//! a failed link to one place. Both say "no behaviour change" for the GL
//! answers. This drives the real GL core in a `report-es2` build, with the
//! online compiler of the back end the feature set names, and asserts:
//!
//! (a) a link that works: GL_LINK_STATUS, an empty info log, no error, no
//!     console line, and GL_ATTACHED_SHADERS / GL_ACTIVE_ATTRIBUTES /
//!     GL_ACTIVE_UNIFORMS after each step from glCreateProgram on;
//! (b) glBindAttribLocation takes effect at the next link, not before;
//! (c) a detached shader leaves the linked program as it is until the next
//!     link, which then fails; another fragment shader attached, it links
//!     again with that shader's uniforms;
//! (d) a program in use that is linked again stays current and draws with
//!     the new executable; linked again without success it stays the current
//!     program with GL_LINK_STATUS FALSE (what ES 2.0 2.10.3 says of its old
//!     executable is the `#[ignore]`d test at the end);
//! (e) failed links (nothing attached, no vertex shader, no fragment shader,
//!     a shader that did not compile, an undefined varying, a uniform of two
//!     types, more attributes than GL_MAX_VERTEX_ATTRIBS): GL_LINK_STATUS
//!     FALSE, a non-empty info log, glUseProgram GL_INVALID_OPERATION, no
//!     error from glLinkProgram, and one console line carrying the info log;
//! (f) glValidateProgram: GL_VALIDATE_STATUS starts FALSE, follows the link
//!     status, and a link resets it;
//! (g) no transform-feedback state: the three program pnames are
//!     GL_INVALID_ENUM, and every link above ran with none requested.
//!
//! The stand-in back end executes nothing; a draw is asserted by the draws it
//! hands the back end. Both back ends run the same bodies, each in its own
//! module.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online,report-es2 --test gl_es2ctx_drift_link_with
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table,report-es2 --test gl_es2ctx_drift_link_with
//! ```
#![cfg(all(
    feature = "host-test",
    feature = "shaderc-online",
    feature = "report-es2",
    any(feature = "gpu-vivante", feature = "gpu-v3d")
))]

use core::ffi::{c_char, c_void, CStr};
use std::ffi::CString;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::*;
use yakogl::gl::buffer::*;
use yakogl::gl::consts::*;
use yakogl::gl::draw::*;
use yakogl::gl::program::*;
use yakogl::gl::shader::*;
use yakogl::gl::state::*;
use yakogl::gl::types::*;
use yakogl::gl::vertex::*;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};

/* ---- a context on the host: the smallest OSAL and back end that make one --- */

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

static DRAWS: AtomicUsize = AtomicUsize::new(0);

unsafe extern "C" fn b_draw(_d: *const GlesDraw) -> i32 {
    DRAWS.fetch_add(1, Ordering::SeqCst);
    0
}

/// The console: every line the library printed.
static CONSOLE: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// The log hook. The library only ever calls it as `("%s", line)`
/// (`osal::log_str`), so the two-pointer form reads the whole call.
unsafe extern "C" fn m_log(_fmt: *const c_char, line: *const c_char) {
    let line = CStr::from_ptr(line).to_string_lossy().into_owned();
    CONSOLE.lock().unwrap_or_else(|e| e.into_inner()).push(line);
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

static BACKEND: YakoglBackend = YakoglBackend {
    name: c"gl-es2ctx-drift-link-with".as_ptr() as *const c_char,
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

static LOCK: Mutex<()> = Mutex::new(());
static ONCE: Once = Once::new();

/// The one ES 2.0 context, the online compiler behind it; serialises the tests.
fn context() -> MutexGuard<'static, ()> {
    let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    ONCE.call_once(|| {
        // SAFETY: both tables are statics; the lock above serialises the tests.
        unsafe {
            osal_set_ops(&OSAL);
            g_backend = &BACKEND;
            yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
            assert!(!yakogl::gl::context::gl_context_create(16, 16).is_null());
        }
    });
    // SAFETY: plain GL calls on the one context.
    unsafe {
        glUseProgram(0);
        while glGetError() != GL_NO_ERROR {}
        let version = CStr::from_ptr(glGetString(GL_VERSION) as *const c_char).to_string_lossy().into_owned();
        assert!(version.starts_with("OpenGL ES 2.0"), "an ES 2.0 context: {version}");
    }
    g
}

fn err() -> GLenum {
    // SAFETY: plain GL call.
    unsafe { glGetError() }
}

/// The console's glLinkProgram lines since the last call.
fn link_lines() -> Vec<String> {
    let mut c = CONSOLE.lock().unwrap_or_else(|e| e.into_inner());
    c.drain(..).filter(|l| l.starts_with("glLinkProgram(")).collect()
}

/// Compile `text`: the shader and whether it compiled.
fn compile(ty: GLenum, text: &str) -> (GLuint, bool) {
    let text = CString::new(text).unwrap();
    // SAFETY: plain GL calls; one NUL-terminated source.
    unsafe {
        let s = glCreateShader(ty);
        let p: [*const c_char; 1] = [text.as_ptr()];
        glShaderSource(s, 1, p.as_ptr(), core::ptr::null());
        glCompileShader(s);
        let mut ok: GLint = 0;
        glGetShaderiv(s, GL_COMPILE_STATUS, &mut ok);
        (s, ok == 1)
    }
}

/// A shader that has to compile.
fn shader(ty: GLenum, text: &str) -> GLuint {
    let (s, ok) = compile(ty, text);
    assert!(ok, "compiles:\n{text}");
    assert_eq!(err(), GL_NO_ERROR);
    s
}

fn geti(p: GLuint, pname: GLenum) -> GLint {
    let mut v: GLint = -99;
    // SAFETY: one GLint out.
    unsafe { glGetProgramiv(p, pname, &mut v) };
    v
}

fn info_log(p: GLuint) -> String {
    let mut log = [0u8; 1024];
    let mut n: GLsizei = 0;
    // SAFETY: the buffer is 1024 bytes.
    unsafe { glGetProgramInfoLog(p, 1024, &mut n, log.as_mut_ptr().cast::<c_char>()) };
    String::from_utf8_lossy(&log[..n.max(0) as usize]).into_owned()
}

/// (GL_ATTACHED_SHADERS, GL_ACTIVE_ATTRIBUTES, GL_ACTIVE_UNIFORMS), and the
/// queries raise nothing.
fn counts(p: GLuint) -> (GLint, GLint, GLint) {
    let c = (geti(p, GL_ATTACHED_SHADERS), geti(p, GL_ACTIVE_ATTRIBUTES), geti(p, GL_ACTIVE_UNIFORMS));
    assert_eq!(err(), GL_NO_ERROR, "the three counts are ES 2.0 queries");
    c
}

fn linked(p: GLuint) -> bool {
    geti(p, GL_LINK_STATUS) == 1
}

/// glLinkProgram, which itself raises nothing whatever the outcome.
fn link(p: GLuint) -> bool {
    // SAFETY: plain GL call.
    unsafe { glLinkProgram(p) };
    assert_eq!(err(), GL_NO_ERROR, "glLinkProgram raises no error of its own");
    linked(p)
}

fn attach(p: GLuint, s: GLuint) {
    // SAFETY: plain GL call.
    unsafe { glAttachShader(p, s) };
    assert_eq!(err(), GL_NO_ERROR, "glAttachShader");
}

fn detach(p: GLuint, s: GLuint) {
    // SAFETY: plain GL call.
    unsafe { glDetachShader(p, s) };
    assert_eq!(err(), GL_NO_ERROR, "glDetachShader");
}

fn attached(p: GLuint) -> Vec<GLuint> {
    let mut names = [0 as GLuint; 4];
    let mut n: GLsizei = -1;
    // SAFETY: room for four names.
    unsafe { glGetAttachedShaders(p, 4, &mut n, names.as_mut_ptr()) };
    assert_eq!(err(), GL_NO_ERROR, "glGetAttachedShaders");
    names[..n.max(0) as usize].to_vec()
}

fn attrib(p: GLuint, name: &CStr) -> GLint {
    // SAFETY: a NUL-terminated name.
    unsafe { glGetAttribLocation(p, name.as_ptr()) }
}

fn uniform(p: GLuint, name: &CStr) -> GLint {
    // SAFETY: a NUL-terminated name.
    unsafe { glGetUniformLocation(p, name.as_ptr()) }
}

/// The names glGetActiveAttrib answers, sorted.
fn attrib_names(p: GLuint) -> Vec<String> {
    let mut out = Vec::new();
    for i in 0..geti(p, GL_ACTIVE_ATTRIBUTES).max(0) {
        let mut name = [0u8; 64];
        let (mut len, mut size, mut ty): (GLsizei, GLint, GLenum) = (0, 0, 0);
        // SAFETY: the name buffer is 64 bytes.
        unsafe {
            glGetActiveAttrib(p, i as GLuint, 64, &mut len, &mut size, &mut ty, name.as_mut_ptr().cast::<c_char>())
        };
        out.push(String::from_utf8_lossy(&name[..len.max(0) as usize]).into_owned());
    }
    assert_eq!(err(), GL_NO_ERROR, "glGetActiveAttrib");
    out.sort();
    out
}

fn use_program(p: GLuint) -> GLenum {
    // SAFETY: plain GL call.
    unsafe { glUseProgram(p) };
    err()
}

fn current_program() -> GLuint {
    let mut v: GLint = -1;
    // SAFETY: one GLint out.
    unsafe { glGetIntegerv(GL_CURRENT_PROGRAM, &mut v) };
    v as GLuint
}

fn delete(p: GLuint, shaders: &[GLuint]) {
    // SAFETY: plain GL calls.
    unsafe {
        glUseProgram(0);
        glDeleteProgram(p);
        for &s in shaders {
            glDeleteShader(s);
        }
        while glGetError() != GL_NO_ERROR {}
    }
}

fn program() -> GLuint {
    // SAFETY: plain GL call.
    let p = unsafe { glCreateProgram() };
    assert_ne!(p, 0);
    p
}

/// Draw one triangle with the current program, `a_pos` from a buffer at
/// attribute `pos`: the draws that reached the back end, and the error.
fn draw(pos: GLuint) -> (usize, GLenum) {
    let verts: [f32; 12] = [0.0, 0.0, 0.0, 1.0, 1.0, 0.0, 0.0, 1.0, 0.0, 1.0, 0.0, 1.0];
    let before = DRAWS.load(Ordering::SeqCst);
    // SAFETY: plain GL calls; glBufferData copies the vertex data.
    unsafe {
        let mut b = 0;
        glGenBuffers(1, &mut b);
        glBindBuffer(GL_ARRAY_BUFFER, b);
        glBufferData(GL_ARRAY_BUFFER, 48, verts.as_ptr().cast::<c_void>(), GL_STATIC_DRAW);
        glVertexAttribPointer(pos, 4, GL_FLOAT, 0, 0, core::ptr::null());
        glEnableVertexAttribArray(pos);
        glDrawArrays(GL_TRIANGLES, 0, 3);
        let e = glGetError();
        glFinish();
        glDisableVertexAttribArray(pos);
        glBindBuffer(GL_ARRAY_BUFFER, 0);
        glDeleteBuffers(1, &b);
        while glGetError() != GL_NO_ERROR {}
        (DRAWS.load(Ordering::SeqCst) - before, e)
    }
}

/// Two attributes, one vertex uniform, one varying.
const VS: &str = "attribute vec4 a_pos;\nattribute vec4 a_col;\nuniform mediump float u_scale;\nvarying mediump vec4 v_col;\nvoid main() { v_col = a_col; gl_Position = a_pos * u_scale; }\n";
/// One fragment uniform.
const FS: &str = "precision mediump float;\nuniform vec4 u_tint;\nvarying mediump vec4 v_col;\nvoid main() { gl_FragColor = v_col * u_tint; }\n";
/// Two fragment uniforms.
const FS2: &str = "precision mediump float;\nuniform vec4 u_tint;\nuniform float u_gain;\nvarying mediump vec4 v_col;\nvoid main() { gl_FragColor = v_col * u_tint * u_gain; }\n";

/// A linked program of `VS` and `FS`: (program, vertex shader, fragment shader).
fn linked_program() -> (GLuint, GLuint, GLuint) {
    let (v, f) = (shader(GL_VERTEX_SHADER, VS), shader(GL_FRAGMENT_SHADER, FS));
    let p = program();
    attach(p, v);
    attach(p, f);
    link_lines();
    assert!(link(p), "links: {}", info_log(p));
    assert!(link_lines().is_empty(), "a link that works prints nothing");
    (p, v, f)
}

/// What every failed link answers: GL_LINK_STATUS FALSE, a non-empty info
/// log (and its length), no executable to use, one console line with the log.
fn assert_failed_link(p: GLuint, what: &str) {
    link_lines();
    assert!(!link(p), "{what}: GL_LINK_STATUS is FALSE");
    let log = info_log(p);
    assert!(!log.trim().is_empty(), "{what}: the info log says why");
    assert_eq!(geti(p, GL_INFO_LOG_LENGTH), log.len() as GLint + 1, "{what}: GL_INFO_LOG_LENGTH counts the NUL");
    assert_eq!(err(), GL_NO_ERROR, "{what}: the queries raise nothing");
    let lines = link_lines();
    assert_eq!(lines.len(), 1, "{what}: one console line for the failed link: {lines:?}");
    assert!(lines[0].starts_with(&format!("glLinkProgram({p}): ")), "{what}: {lines:?}");
    /* the console line is the info log (cut at the console's line length) */
    let said = lines[0].split_once("): ").map_or("", |x| x.1).trim_end();
    assert!(!said.is_empty() && log.starts_with(said.lines().next().unwrap_or("")), "{what}: {said:?} vs {log:?}");
    let before = current_program();
    assert_eq!(use_program(p), GL_INVALID_OPERATION, "{what}: glUseProgram of a program that did not link");
    assert_eq!(current_program(), before, "{what}: the current program is unchanged");
    assert_eq!(attrib(p, c"a_pos"), -1, "{what}");
    assert_eq!(err(), GL_INVALID_OPERATION, "{what}: glGetAttribLocation of a program that did not link");
    assert_eq!(geti(p, GL_VALIDATE_STATUS), 0, "{what}: the link reset GL_VALIDATE_STATUS");
}

/* ---- the cases ----------------------------------------------------------- */

/// (a) the counts after each step, and what a working link leaves.
fn a_link_that_works() {
    let _g = context();
    let (v, f) = (shader(GL_VERTEX_SHADER, VS), shader(GL_FRAGMENT_SHADER, FS));
    let p = program();
    assert_eq!(counts(p), (0, 0, 0), "a new program");
    assert!(!linked(p));
    assert!(attached(p).is_empty());
    attach(p, v);
    assert_eq!(counts(p), (1, 0, 0), "the vertex shader attached");
    attach(p, f);
    assert_eq!(counts(p), (2, 0, 0), "both attached, not linked");
    assert_eq!(attached(p), [v, f]);
    assert_eq!(use_program(p), GL_INVALID_OPERATION, "not linked yet");

    link_lines();
    assert!(link(p), "links: {}", info_log(p));
    assert!(link_lines().is_empty(), "a link that works prints nothing");
    assert_eq!(info_log(p), "", "a link that works leaves no log");
    assert_eq!(counts(p), (2, 2, 2), "linked: a_pos, a_col; u_scale, u_tint");
    assert_eq!(attrib_names(p), ["a_col", "a_pos"]);
    assert!(uniform(p, c"u_scale") >= 0 && uniform(p, c"u_tint") >= 0);
    assert_eq!(uniform(p, c"u_gain"), -1);
    assert_eq!(err(), GL_NO_ERROR);

    /* the same link again answers the same */
    assert!(link(p));
    assert_eq!(counts(p), (2, 2, 2), "linked twice");
    assert!(link_lines().is_empty());

    assert_eq!(use_program(p), GL_NO_ERROR);
    assert_eq!(draw(attrib(p, c"a_pos") as GLuint), (1, GL_NO_ERROR), "the draw reaches the back end");
    delete(p, &[v, f]);
}

/// (b) glBindAttribLocation: the locations follow the binding at the next link.
fn relink_after_bind_attrib_location() {
    let _g = context();
    let (p, v, f) = linked_program();
    let (pos0, col0) = (attrib(p, c"a_pos"), attrib(p, c"a_col"));
    assert!(pos0 >= 0 && col0 >= 0 && pos0 != col0, "a_pos {pos0}, a_col {col0}");

    // SAFETY: plain GL calls; NUL-terminated names.
    unsafe {
        glBindAttribLocation(p, 7, c"a_pos".as_ptr());
        glBindAttribLocation(p, 3, c"a_col".as_ptr());
    }
    assert_eq!(err(), GL_NO_ERROR, "glBindAttribLocation");
    assert!(linked(p), "a binding does not unlink");
    assert_eq!((attrib(p, c"a_pos"), attrib(p, c"a_col")), (pos0, col0), "no effect before the link");

    assert!(link(p), "relinks: {}", info_log(p));
    assert_eq!((attrib(p, c"a_pos"), attrib(p, c"a_col")), (7, 3), "the locations follow the binding");
    assert_eq!(counts(p), (2, 2, 2));
    assert_eq!(attrib_names(p), ["a_col", "a_pos"]);
    assert_eq!(use_program(p), GL_NO_ERROR);
    assert_eq!(draw(7), (1, GL_NO_ERROR), "draws with a_pos at 7");

    /* bound again, linked again: the last binding wins */
    // SAFETY: as above.
    unsafe { glBindAttribLocation(p, 0, c"a_pos".as_ptr()) };
    assert!(link(p));
    assert_eq!((attrib(p, c"a_pos"), attrib(p, c"a_col")), (0, 3));

    /* a binding at GL_MAX_VERTEX_ATTRIBS is refused and changes nothing */
    // SAFETY: as above.
    unsafe { glBindAttribLocation(p, 16, c"a_pos".as_ptr()) };
    assert_eq!(err(), GL_INVALID_VALUE);
    assert!(link(p));
    assert_eq!((attrib(p, c"a_pos"), attrib(p, c"a_col")), (0, 3));
    assert_eq!(err(), GL_NO_ERROR);
    assert!(link_lines().is_empty(), "none of these links failed");
    delete(p, &[v, f]);
}

/// (c) a shader detached, then replaced.
fn relink_after_detach_and_replace() {
    let _g = context();
    let (p, v, f) = linked_program();
    let f2 = shader(GL_FRAGMENT_SHADER, FS2);

    detach(p, f);
    assert_eq!(counts(p), (1, 2, 2), "detaching leaves the linked program as it is");
    assert_eq!(attached(p), [v]);
    assert!(linked(p));
    assert_eq!(use_program(p), GL_NO_ERROR, "the executable is still there");
    assert_eq!(draw(attrib(p, c"a_pos") as GLuint), (1, GL_NO_ERROR));
    assert_eq!(use_program(0), GL_NO_ERROR);

    assert_failed_link(p, "the fragment shader detached");
    assert_eq!(counts(p), (1, 0, 0), "nothing is active after a failed link");
    assert_eq!(uniform(p, c"u_tint"), -1);
    while err() != GL_NO_ERROR {}

    attach(p, f2);
    assert_eq!(counts(p), (2, 0, 0), "attached, not linked yet");
    link_lines();
    assert!(link(p), "links with the other fragment shader: {}", info_log(p));
    assert!(link_lines().is_empty());
    assert_eq!(info_log(p), "", "the failed link's log is gone");
    assert_eq!(counts(p), (2, 2, 3), "u_scale, u_tint, u_gain");
    assert!(uniform(p, c"u_gain") >= 0);
    assert_eq!(use_program(p), GL_NO_ERROR);
    assert_eq!(draw(attrib(p, c"a_pos") as GLuint), (1, GL_NO_ERROR));

    /* the vertex shader detached and attached again links the same */
    assert_eq!(use_program(0), GL_NO_ERROR);
    detach(p, v);
    assert_failed_link(p, "the vertex shader detached");
    assert_eq!(counts(p), (1, 0, 0));
    attach(p, v);
    assert!(link(p));
    assert_eq!(counts(p), (2, 2, 3));
    delete(p, &[v, f, f2]);
}

/// (d) a program in use, linked again.
fn relink_of_a_program_in_use() {
    let _g = context();
    let (p, v, f) = linked_program();
    let f2 = shader(GL_FRAGMENT_SHADER, FS2);
    assert_eq!(use_program(p), GL_NO_ERROR);
    let pos = attrib(p, c"a_pos") as GLuint;
    assert_eq!(draw(pos), (1, GL_NO_ERROR));

    /* linked again as it is, while current */
    assert!(link(p));
    assert_eq!(current_program(), p);
    assert_eq!(draw(pos), (1, GL_NO_ERROR), "no glUseProgram needed after a relink that worked");

    /* another fragment shader: the new executable is the current one */
    detach(p, f);
    attach(p, f2);
    assert_eq!(draw(pos), (1, GL_NO_ERROR), "the old executable draws until the link");
    assert_eq!(counts(p), (2, 2, 2), "and answers the queries until the link");
    assert!(link(p), "{}", info_log(p));
    assert_eq!(current_program(), p);
    assert_eq!(counts(p), (2, 2, 3));
    let gain = uniform(p, c"u_gain");
    assert!(gain >= 0);
    // SAFETY: plain GL call on the current program.
    unsafe { glUniform1f(gain, 0.5) };
    assert_eq!(err(), GL_NO_ERROR, "the new uniform is set through the current program");
    assert_eq!(draw(pos), (1, GL_NO_ERROR), "draws with the new executable");

    /* a relink that fails: the program object is unlinked and stays current */
    detach(p, f2);
    assert!(!link(p));
    assert!(!info_log(p).trim().is_empty());
    assert_eq!(link_lines().len(), 1);
    assert_eq!(current_program(), p, "still the current program (ES 2.0 2.10.3)");
    assert_eq!(counts(p), (1, 0, 0));
    assert_eq!(err(), GL_NO_ERROR);
    /* it cannot be installed again, and another program can replace it */
    assert_eq!(use_program(p), GL_INVALID_OPERATION);
    assert_eq!(current_program(), p);
    assert_eq!(use_program(0), GL_NO_ERROR);
    assert_eq!(current_program(), 0);
    /* and it links again */
    attach(p, f);
    assert!(link(p));
    assert_eq!(counts(p), (2, 2, 2));
    assert_eq!(use_program(p), GL_NO_ERROR);
    assert_eq!(draw(attrib(p, c"a_pos") as GLuint), (1, GL_NO_ERROR));
    delete(p, &[v, f, f2]);
}

/// (e) links with a shader missing or not compiled.
fn failed_link_missing_shader() {
    let _g = context();
    let (v, f) = (shader(GL_VERTEX_SHADER, VS), shader(GL_FRAGMENT_SHADER, FS));

    let p = program();
    assert_failed_link(p, "nothing attached");
    assert_eq!(counts(p), (0, 0, 0));
    attach(p, f);
    assert_failed_link(p, "no vertex shader");
    assert_eq!(counts(p), (1, 0, 0));
    detach(p, f);
    attach(p, v);
    assert_failed_link(p, "no fragment shader");
    assert_eq!(counts(p), (1, 0, 0));
    /* the same failure twice: one line each time, the same answers */
    assert_failed_link(p, "no fragment shader, again");

    /* a shader that was never compiled, and one that did not compile */
    // SAFETY: plain GL call.
    let raw = unsafe { glCreateShader(GL_FRAGMENT_SHADER) };
    attach(p, raw);
    assert_failed_link(p, "a fragment shader never compiled");
    assert_eq!(counts(p), (2, 0, 0));
    detach(p, raw);
    let (bad, ok) = compile(GL_FRAGMENT_SHADER, "precision mediump float;\nvoid main() { gl_FragColor = nothing; }\n");
    assert!(!ok, "an undeclared name does not compile");
    attach(p, bad);
    assert_failed_link(p, "a fragment shader that did not compile");
    assert_eq!(counts(p), (2, 0, 0));
    detach(p, bad);

    /* with both it links, and the failures left nothing behind */
    attach(p, f);
    link_lines();
    assert!(link(p), "{}", info_log(p));
    assert!(link_lines().is_empty());
    assert_eq!(counts(p), (2, 2, 2));
    assert_eq!(use_program(p), GL_NO_ERROR);
    delete(p, &[v, f, raw, bad]);
}

/// Link `vs` and `fs`, which compile, and assert the link fails.
fn assert_link_refused(vs: &str, fs: &str, what: &str) {
    let (v, f) = (shader(GL_VERTEX_SHADER, vs), shader(GL_FRAGMENT_SHADER, fs));
    let p = program();
    attach(p, v);
    attach(p, f);
    assert_failed_link(p, what);
    assert_eq!(counts(p), (2, 0, 0), "{what}");
    delete(p, &[v, f]);
}

/// (e) a varying the fragment shader reads and the vertex shader lacks.
fn failed_link_undefined_varying() {
    let _g = context();
    let vs = "attribute vec4 a_pos;\nvoid main() { gl_Position = a_pos; }\n";
    let fs = "precision mediump float;\nvarying vec4 v_missing;\nvoid main() { gl_FragColor = v_missing; }\n";
    assert_link_refused(vs, fs, "an undefined varying");
    /* declared and not read, it is no error (GLSL ES 1.00 4.3.5) */
    let fs = "precision mediump float;\nvarying vec4 v_missing;\nvoid main() { gl_FragColor = vec4(1.0); }\n";
    let (v, f) = (shader(GL_VERTEX_SHADER, vs), shader(GL_FRAGMENT_SHADER, fs));
    let p = program();
    attach(p, v);
    attach(p, f);
    assert!(link(p), "a varying declared and not read links: {}", info_log(p));
    delete(p, &[v, f]);
}

/// (e) one uniform name, two types.
fn failed_link_uniform_type_mismatch() {
    let _g = context();
    let vs = "attribute vec4 a_pos;\nuniform mediump float u_x;\nvoid main() { gl_Position = a_pos * u_x; }\n";
    let fs = "uniform mediump vec4 u_x;\nvoid main() { gl_FragColor = u_x; }\n";
    assert_link_refused(vs, fs, "a uniform of two types");
    /* the same type in both stages is one uniform */
    let fs = "uniform mediump float u_x;\nvoid main() { gl_FragColor = vec4(u_x); }\n";
    let (v, f) = (shader(GL_VERTEX_SHADER, vs), shader(GL_FRAGMENT_SHADER, fs));
    let p = program();
    attach(p, v);
    attach(p, f);
    assert!(link(p), "a uniform shared by both stages links: {}", info_log(p));
    assert_eq!(counts(p), (2, 1, 1));
    delete(p, &[v, f]);
}

/// A vertex shader reading `n` vec4 attributes.
fn vs_with_attribs(n: usize) -> String {
    let mut s = String::new();
    for i in 0..n {
        s += &format!("attribute vec4 a{i};\n");
    }
    s += "void main() {\n  vec4 p = vec4(0.0);\n";
    for i in 0..n {
        s += &format!("  p += a{i};\n");
    }
    s + "  gl_Position = p;\n}\n"
}

/// (e) more active attributes than GL_MAX_VERTEX_ATTRIBS (ES 2.0 2.10.4).
fn failed_link_too_many_attributes() {
    let _g = context();
    let mut max: GLint = 0;
    // SAFETY: one GLint out.
    unsafe { glGetIntegerv(GL_MAX_VERTEX_ATTRIBS, &mut max) };
    assert_eq!(max, 16);
    let fs = "void main() { gl_FragColor = vec4(1.0); }\n";

    /* exactly the maximum links */
    let (v, f) = (shader(GL_VERTEX_SHADER, &vs_with_attribs(16)), shader(GL_FRAGMENT_SHADER, fs));
    let p = program();
    attach(p, v);
    attach(p, f);
    link_lines();
    assert!(link(p), "16 attributes link: {}", info_log(p));
    assert!(link_lines().is_empty());
    assert_eq!(counts(p), (2, 16, 0));
    delete(p, &[v, f]);

    /* one more is no program: the compile or the link refuses it */
    let (v, vok) = compile(GL_VERTEX_SHADER, &vs_with_attribs(17));
    let f = shader(GL_FRAGMENT_SHADER, fs);
    while err() != GL_NO_ERROR {}
    let p = program();
    attach(p, v);
    attach(p, f);
    assert_failed_link(p, "17 attributes");
    assert_eq!(counts(p), (2, 0, 0));
    /* which of the two refuses is the compiler's choice; it is pinned so a
     * move shows */
    assert!(vok, "the 17-attribute vertex shader compiles and the link refuses it");
    delete(p, &[v, f]);
}

/// (f) glValidateProgram.
fn validate_program() {
    let _g = context();
    let (v, f) = (shader(GL_VERTEX_SHADER, VS), shader(GL_FRAGMENT_SHADER, FS));
    let p = program();
    let validate = |p: GLuint| {
        // SAFETY: plain GL call.
        unsafe { glValidateProgram(p) };
        err()
    };
    assert_eq!(geti(p, GL_VALIDATE_STATUS), 0, "starts FALSE");
    assert_eq!(validate(p), GL_NO_ERROR, "validating an unlinked program is no error");
    assert_eq!(geti(p, GL_VALIDATE_STATUS), 0, "an unlinked program does not validate");

    attach(p, v);
    attach(p, f);
    assert!(link(p));
    assert_eq!(geti(p, GL_VALIDATE_STATUS), 0, "a link is not a validation");
    assert_eq!(validate(p), GL_NO_ERROR);
    assert_eq!(geti(p, GL_VALIDATE_STATUS), 1, "a linked program validates");
    assert!(linked(p), "validation leaves the link status");
    assert_eq!(counts(p), (2, 2, 2), "and the counts");

    assert!(link(p));
    assert_eq!(geti(p, GL_VALIDATE_STATUS), 0, "a link resets it");
    assert_eq!(validate(p), GL_NO_ERROR);
    assert_eq!(geti(p, GL_VALIDATE_STATUS), 1);

    detach(p, f);
    assert_failed_link(p, "validated, then the fragment shader detached");
    assert_eq!(validate(p), GL_NO_ERROR);
    assert_eq!(geti(p, GL_VALIDATE_STATUS), 0, "a program that did not link does not validate");

    /* dEQP-GLES2 negative_api.shader.validate_program */
    assert_eq!(validate(v), GL_INVALID_OPERATION, "a shader's name");
    let mut unused: GLuint = 1;
    // SAFETY: plain GL call.
    while unsafe { glIsProgram(unused) != 0 || glIsShader(unused) != 0 } {
        unused += 1;
    }
    assert_eq!(validate(unused), GL_INVALID_VALUE, "a name that is no object");
    assert!(link_lines().len() <= 1, "validation prints nothing");
    delete(p, &[v, f]);
}

/// (g) ES 2.0 has no transform-feedback program state (6.1.8 lists nine
/// pnames); the links above all ran without any.
fn no_transform_feedback_state() {
    let _g = context();
    let (p, v, f) = linked_program();
    for pname in
        [GL_TRANSFORM_FEEDBACK_VARYINGS, GL_TRANSFORM_FEEDBACK_BUFFER_MODE, GL_TRANSFORM_FEEDBACK_VARYING_MAX_LENGTH]
    {
        assert_eq!(geti(p, pname), -99, "{pname:#x} writes nothing");
        assert_eq!(err(), GL_INVALID_ENUM, "{pname:#x} is no ES 2.0 program query");
    }
    /* the nine that are */
    for pname in [
        GL_DELETE_STATUS,
        GL_LINK_STATUS,
        GL_VALIDATE_STATUS,
        GL_INFO_LOG_LENGTH,
        GL_ATTACHED_SHADERS,
        GL_ACTIVE_ATTRIBUTES,
        GL_ACTIVE_ATTRIBUTE_MAX_LENGTH,
        GL_ACTIVE_UNIFORMS,
        GL_ACTIVE_UNIFORM_MAX_LENGTH,
    ] {
        assert_ne!(geti(p, pname), -99, "{pname:#x} answers");
        assert_eq!(err(), GL_NO_ERROR, "{pname:#x}");
    }
    assert!(linked(p), "the refused queries leave the program linked");
    assert_eq!(use_program(p), GL_NO_ERROR);
    assert_eq!(draw(attrib(p, c"a_pos") as GLuint), (1, GL_NO_ERROR));
    delete(p, &[v, f]);
}

/// ES 2.0 2.10.3: "If a program object that is in use is re-linked
/// unsuccessfully, the link status will be set to FALSE, but existing
/// executable and associated state will remain part of the current rendering
/// state until a subsequent call to UseProgram removes it from use."
///
/// Not so here, and not a change of 830858db or ae96bfc7: glLinkProgram drops
/// the old link first (`link_reset`; the same lines stood in glLinkProgram at
/// 0fbe61a3, the i.MX8MP's last whole ES 2.0 tree), and a draw then asks for a
/// linked current program (draw.rs `current_drawable_program`), so the draw
/// is GL_INVALID_OPERATION and nothing reaches the back end. `#[ignore]`d
/// until the GL core keeps the executable; src is not this guard's to change.
fn failed_relink_keeps_the_executable_in_use() {
    let _g = context();
    let (p, v, f) = linked_program();
    assert_eq!(use_program(p), GL_NO_ERROR);
    let pos = attrib(p, c"a_pos") as GLuint;
    assert_eq!(draw(pos), (1, GL_NO_ERROR));
    detach(p, f);
    assert!(!link(p));
    assert_eq!(current_program(), p);
    assert_eq!(draw(pos), (1, GL_NO_ERROR), "the old executable still draws");
    delete(p, &[v, f]);
}

macro_rules! on_each_back_end {
    ($($(#[$m:meta])* $name:ident;)*) => {
        #[cfg(feature = "gpu-vivante")]
        mod vivante {
            $(
                #[test]
                $(#[$m])*
                fn $name() {
                    super::$name();
                }
            )*
        }

        #[cfg(feature = "gpu-v3d")]
        mod v3d {
            $(
                #[test]
                $(#[$m])*
                fn $name() {
                    super::$name();
                }
            )*
        }
    };
}

on_each_back_end! {
    a_link_that_works;
    relink_after_bind_attrib_location;
    relink_after_detach_and_replace;
    relink_of_a_program_in_use;
    failed_link_missing_shader;
    failed_link_undefined_varying;
    failed_link_uniform_type_mismatch;
    failed_link_too_many_attributes;
    validate_program;
    no_transform_feedback_state;
    #[ignore = "ES 2.0 2.10.3: a failed relink of the program in use drops its executable (as at 0fbe61a3)"]
    failed_relink_keeps_the_executable_in_use;
}
