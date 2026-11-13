// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Uniform and uniform-block names of 63 or more characters, through the GL
//! API on the host, on each online back end: every name is linked, matched
//! and reported whole (docs/notes/reflection-long-names.md).
//!
//! The 64-byte `ShcUniform::name` field stores 63 characters; the online
//! record keeps the whole name of a row the field cut (its name pool) and
//! the GL core reads every uniform row's name through it
//! (`shaderc::uniform_name`). Pinned here:
//!
//! * dEQP-GLES3 `uniform_block.valid.long_instance_name_{vertex,fragment}`
//!   (a 1,024-character instance name) link, and the names GL hands out are
//!   the block type's, `GL_ACTIVE_UNIFORM_BLOCK_MAX_NAME_LENGTH` included;
//! * a default-block uniform of 63, 64, 200 and 1,024 characters links, is
//!   reported whole by glGetActiveUniform (with the copy-out edges of
//!   `bufSize`), counted by `GL_ACTIVE_UNIFORM_MAX_LENGTH` and
//!   `GL_UNIFORM_NAME_LENGTH`, found by glGetUniformLocation and
//!   glGetUniformIndices, and holds the value glUniform4f wrote;
//! * two long names that differ only past the field's 63 characters are two
//!   uniforms, in one stage and across the two stages, and one long name both
//!   stages declare is one;
//! * an array and an array of structs behind long names, and a block whose
//!   type name is 1,024 characters;
//! * a 1,025-character identifier is still refused (GLSL ES 3.00 3.8).
//!
//! The host renders nothing here: the verdicts are the compile, the link and
//! the GL queries.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test es3_uniform_long_name
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online --test es3_uniform_long_name
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_uniform_long_name
//! ```
#![cfg(all(
    feature = "shaderc-online",
    any(feature = "gpu-powervr", feature = "gpu-vivante", feature = "gpu-v3d"),
    not(feature = "report-es2")
))]
use core::ffi::{c_char, c_void, CStr};
use std::ffi::CString;
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{GlesDraw, GlesHeap, YakoglBackend};
use yakogl::gl::consts::*;
use yakogl::gl::program::*;
use yakogl::gl::shader::*;
use yakogl::gl::state::*;
use yakogl::gl::types::*;
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
unsafe extern "C" fn b_draw(_d: *const GlesDraw) -> i32 {
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
    name: c"es3-uniform-long-name".as_ptr() as *const c_char,
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
    ONCE.call_once(|| {
        // SAFETY: both tables are statics; the lock above serialises the tests.
        unsafe {
            osal_set_ops(&OSAL);
            yakogl::backend::g_backend = &BACKEND;
            yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
            assert!(!yakogl::gl::context::gl_context_create(64, 64).is_null());
        }
    });
    // SAFETY: plain GL calls on the one context.
    unsafe {
        glUseProgram(0);
        while glGetError() != GL_NO_ERROR {}
    }
    g
}

fn err() -> GLenum {
    // SAFETY: plain GL call.
    unsafe { glGetError() }
}

/* ---- the GL side ------------------------------------------------------- */

const VS_PLAIN: &str =
    "#version 300 es\nprecision highp float;\nin vec4 a_pos;\nvoid main() { gl_Position = a_pos; }\n";

const FS_PLAIN: &str = "#version 300 es\nprecision highp float;\nlayout(location = 0) out vec4 o;\n\
                        void main() { o = vec4(1.0); }\n";

/// A fragment shader whose colour is `body`, after `decls`.
fn fs(decls: &str, body: &str) -> String {
    format!(
        "#version 300 es\nprecision highp float;\nlayout(location = 0) out vec4 o;\n{decls}\n\
         void main() {{ o = {body}; }}\n"
    )
}

/// A vertex shader whose position is `body`, after `decls`.
fn vs(decls: &str, body: &str) -> String {
    format!("#version 300 es\nprecision highp float;\n{decls}\nvoid main() {{ gl_Position = {body}; }}\n")
}

fn shader(ty: GLenum, text: &str) -> Result<GLuint, String> {
    let c = CString::new(text).unwrap();
    // SAFETY: plain GL calls; one NUL-terminated source and a log buffer.
    unsafe {
        let s = glCreateShader(ty);
        let p: [*const c_char; 1] = [c.as_ptr()];
        glShaderSource(s, 1, p.as_ptr(), core::ptr::null());
        glCompileShader(s);
        let mut ok: GLint = 0;
        glGetShaderiv(s, GL_COMPILE_STATUS, &mut ok);
        if ok == 1 {
            return Ok(s);
        }
        let mut log = [0 as c_char; 1024];
        glGetShaderInfoLog(s, 1024, core::ptr::null_mut(), log.as_mut_ptr());
        glDeleteShader(s);
        Err(CStr::from_ptr(log.as_ptr()).to_string_lossy().into_owned())
    }
}

/// Both stages compiled and linked: the program, or the link's info log.
/// A compile refusal is a failure of the test, not a link verdict.
fn program(vs_src: &str, fs_src: &str) -> Result<GLuint, String> {
    let vs = shader(GL_VERTEX_SHADER, vs_src).unwrap_or_else(|e| panic!("vertex compile: {e}"));
    let fs = shader(GL_FRAGMENT_SHADER, fs_src).unwrap_or_else(|e| panic!("fragment compile: {e}"));
    // SAFETY: plain GL calls on handles this test owns.
    unsafe {
        let p = glCreateProgram();
        glAttachShader(p, vs);
        glAttachShader(p, fs);
        glLinkProgram(p);
        glDeleteShader(vs);
        glDeleteShader(fs);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        if ok == 1 {
            return Ok(p);
        }
        let mut log = [0 as c_char; 2048];
        glGetProgramInfoLog(p, 2048, core::ptr::null_mut(), log.as_mut_ptr());
        glDeleteProgram(p);
        Err(CStr::from_ptr(log.as_ptr()).to_string_lossy().into_owned())
    }
}

fn linked(vs_src: &str, fs_src: &str) -> GLuint {
    program(vs_src, fs_src).unwrap_or_else(|log| panic!("link failed: {log}"))
}

fn delete(p: GLuint) {
    // SAFETY: a program of this context.
    unsafe { glDeleteProgram(p) };
}

fn programiv(p: GLuint, pname: GLenum) -> GLint {
    let mut v: GLint = -1;
    // SAFETY: plain GL call; one value.
    unsafe { glGetProgramiv(p, pname, &mut v) };
    v
}

fn c_bytes(c: &[c_char]) -> &[u8] {
    // SAFETY: c_char and u8 have the same layout.
    unsafe { core::slice::from_raw_parts(c.as_ptr().cast::<u8>(), c.len()) }
}

/// `glGetActiveUniform(index)` into a buffer of 4,096 bytes: name, size, type.
fn active_uniform(p: GLuint, index: u32) -> (String, GLint, GLenum) {
    let mut buf = vec![0 as c_char; 4096];
    let (mut len, mut size, mut ty): (GLsizei, GLint, GLenum) = (-1, 0, 0);
    // SAFETY: plain GL call; every pointer has the room it announces.
    unsafe { glGetActiveUniform(p, index, buf.len() as GLsizei, &mut len, &mut size, &mut ty, buf.as_mut_ptr()) };
    let name = CStr::from_bytes_until_nul(c_bytes(&buf)).unwrap().to_string_lossy().into_owned();
    assert_eq!(len as usize, name.len(), "the length written is the name's");
    (name, size, ty)
}

/// Every active uniform's name, in index order.
fn active_names(p: GLuint) -> Vec<String> {
    (0..programiv(p, GL_ACTIVE_UNIFORMS) as u32).map(|i| active_uniform(p, i).0).collect()
}

/// `glGetActiveUniform(index)` into a buffer of `buf_size` bytes (with
/// sentinels past it): what was written, and `length`.
fn active_uniform_cut(p: GLuint, index: u32, buf_size: GLsizei) -> (Vec<u8>, GLsizei) {
    let mut buf = vec![0x55 as c_char; buf_size as usize + 8];
    let (mut len, mut size, mut ty): (GLsizei, GLint, GLenum) = (-1, 0, 0);
    // SAFETY: plain GL call; `buf` has `buf_size` bytes and more.
    unsafe { glGetActiveUniform(p, index, buf_size, &mut len, &mut size, &mut ty, buf.as_mut_ptr()) };
    assert!(c_bytes(&buf[buf_size as usize..]).iter().all(|&b| b == 0x55), "nothing past bufSize");
    (c_bytes(&buf[..buf_size as usize]).to_vec(), len)
}

/// `glGetActiveUniformsiv(index, pname)`.
fn uniformiv(p: GLuint, index: u32, pname: GLenum) -> GLint {
    let mut v: GLint = -7;
    // SAFETY: plain GL call; one index, room for one value.
    unsafe { glGetActiveUniformsiv(p, 1, &index, pname, &mut v) };
    v
}

/// `glGetActiveUniformBlockName(index)` into a buffer of 4,096 bytes.
fn block_name(p: GLuint, index: u32) -> String {
    let mut buf = vec![0 as c_char; 4096];
    let mut len: GLsizei = -1;
    // SAFETY: plain GL call; every pointer has the room it announces.
    unsafe { glGetActiveUniformBlockName(p, index, buf.len() as GLsizei, &mut len, buf.as_mut_ptr()) };
    let name = CStr::from_bytes_until_nul(c_bytes(&buf)).unwrap().to_string_lossy().into_owned();
    assert_eq!(len as usize, name.len(), "the length written is the name's");
    name
}

fn block_index(p: GLuint, name: &str) -> GLuint {
    let c = CString::new(name).unwrap();
    // SAFETY: plain GL call; a NUL-terminated name.
    unsafe { glGetUniformBlockIndex(p, c.as_ptr()) }
}

/// `glGetUniformIndices` of one name.
fn uniform_index(p: GLuint, name: &str) -> GLuint {
    let c = CString::new(name).unwrap();
    let names: [*const c_char; 1] = [c.as_ptr()];
    let mut idx: [GLuint; 1] = [7];
    // SAFETY: plain GL call; one name, room for one index.
    unsafe { glGetUniformIndices(p, 1, names.as_ptr(), idx.as_mut_ptr()) };
    idx[0]
}

fn location(p: GLuint, name: &str) -> GLint {
    let c = CString::new(name).unwrap();
    // SAFETY: plain GL call; a NUL-terminated name.
    unsafe { glGetUniformLocation(p, c.as_ptr()) }
}

/// glUniform4f at `loc` of `p`, then glGetUniformfv of it.
fn set_and_get(p: GLuint, loc: GLint, v: [f32; 4]) -> [f32; 4] {
    let mut out = [f32::NAN; 4];
    // SAFETY: plain GL calls; room for one vec4.
    unsafe {
        glUseProgram(p);
        glUniform4f(loc, v[0], v[1], v[2], v[3]);
        glGetUniformfv(p, loc, out.as_mut_ptr());
        glUseProgram(0);
    }
    out
}

fn get(p: GLuint, loc: GLint) -> [f32; 4] {
    let mut out = [f32::NAN; 4];
    // SAFETY: plain GL call; room for one vec4.
    unsafe { glGetUniformfv(p, loc, out.as_mut_ptr()) };
    out
}

/* ---- dEQP-GLES3 uniform_block.valid.long_instance_name_* ------------------- */

/// The .test file's own instance name: "Total of 1024 characters".
fn instance_name() -> String {
    "a".repeat(1024)
}

fn check_long_instance_program(p: GLuint, case: &str) {
    assert_eq!(programiv(p, GL_ACTIVE_UNIFORM_BLOCKS), 1, "{case}: one block");
    assert_eq!(block_name(p, 0), "UniformBlock", "{case}: the block's type name");
    assert_eq!(block_index(p, "UniformBlock"), 0, "{case}");
    assert_eq!(programiv(p, GL_ACTIVE_UNIFORM_BLOCK_MAX_NAME_LENGTH), 13, "{case}: `UniformBlock` and a NUL");
    assert_eq!(active_names(p), ["UniformBlock.uniformMember"], "{case}: named by the type, not the instance");
    assert_eq!(programiv(p, GL_ACTIVE_UNIFORM_MAX_LENGTH), 27, "{case}: the longest reported name and a NUL");
    assert_eq!(uniform_index(p, "UniformBlock.uniformMember"), 0, "{case}");
    assert_eq!(location(p, &format!("{}.uniformMember", instance_name())), -1, "{case}: no query knows the instance");
    assert_eq!(err(), GL_NO_ERROR, "{case}");
}

#[test]
fn long_instance_name_vertex() {
    let _g = context();
    let a = instance_name();
    let v = vs(&format!("uniform UniformBlock\n{{\n    vec4 uniformMember;\n}} {a};\n"), &format!("{a}.uniformMember"));
    let p = linked(&v, FS_PLAIN);
    check_long_instance_program(p, "long_instance_name_vertex");
    delete(p);
}

#[test]
fn long_instance_name_fragment() {
    let _g = context();
    let a = instance_name();
    let f = fs(&format!("uniform UniformBlock\n{{\n    vec4 uniformMember;\n}} {a};\n"), &format!("{a}.uniformMember"));
    let p = linked(VS_PLAIN, &f);
    check_long_instance_program(p, "long_instance_name_fragment");
    delete(p);
}

/// A block whose type name is the longest identifier there is: the block is
/// reported whole, and so is its length.
#[test]
fn a_block_type_name_of_1024_characters_is_reported_whole() {
    let _g = context();
    let b = "B".repeat(1024);
    let p = linked(VS_PLAIN, &fs(&format!("uniform {b} {{ vec4 m; }};"), "m"));
    assert_eq!(block_name(p, 0), b);
    assert_eq!(programiv(p, GL_ACTIVE_UNIFORM_BLOCK_MAX_NAME_LENGTH), 1025);
    assert_eq!(block_index(p, &b), 0);
    /* no instance name: the member is named on its own (ES 3.0 2.12.6) */
    assert_eq!(active_names(p), ["m"]);
    assert_eq!(programiv(p, GL_ACTIVE_UNIFORM_MAX_LENGTH), 2);
    assert_eq!(uniform_index(p, "m"), 0);
    assert_eq!(err(), GL_NO_ERROR);
    delete(p);
}

/* ---- default-block uniforms -------------------------------------------- */

/// One default-block vec4 of `n` characters in the fragment shader: linked,
/// reported, measured, found and written whole.
fn check_default_uniform(n: usize) {
    let u: String = (0..n).map(|i| (b'a' + (i % 26) as u8) as char).collect();
    let p = linked(VS_PLAIN, &fs(&format!("uniform vec4 {u};"), &u));
    assert_eq!(programiv(p, GL_ACTIVE_UNIFORMS), 1, "{n}");
    assert_eq!(active_uniform(p, 0), (u.clone(), 1, GL_FLOAT_VEC4), "{n}: the whole name");
    assert_eq!(programiv(p, GL_ACTIVE_UNIFORM_MAX_LENGTH), n as GLint + 1, "{n}");
    assert_eq!(uniformiv(p, 0, GL_UNIFORM_NAME_LENGTH), n as GLint + 1, "{n}");
    assert_eq!(uniform_index(p, &u), 0, "{n}");
    let loc = location(p, &u);
    assert!(loc >= 0, "{n}: glGetUniformLocation finds it");
    assert_eq!(location(p, &u[..n - 1]), -1, "{n}: a prefix is another name");
    assert_eq!(set_and_get(p, loc, [1.0, 2.0, 3.0, 4.0]), [1.0, 2.0, 3.0, 4.0], "{n}");
    assert_eq!(err(), GL_NO_ERROR, "{n}");
    delete(p);
}

#[test]
fn a_default_block_uniform_of_63_characters_links_and_reports_whole() {
    let _g = context();
    check_default_uniform(63);
}

#[test]
fn a_default_block_uniform_of_64_characters_links_and_reports_whole() {
    let _g = context();
    check_default_uniform(64);
}

#[test]
fn a_default_block_uniform_of_200_characters_links_and_reports_whole() {
    let _g = context();
    check_default_uniform(200);
}

#[test]
fn a_default_block_uniform_of_1024_characters_links_and_reports_whole() {
    let _g = context();
    check_default_uniform(1024);
}

/// The copy-out contract on a name longer than the field: at most
/// `bufSize - 1` bytes and a NUL, `length` the bytes written without it.
#[test]
fn glgetactiveuniform_cuts_a_long_name_at_bufsize_only() {
    let _g = context();
    let u = "q".repeat(100);
    let p = linked(VS_PLAIN, &fs(&format!("uniform vec4 {u};"), &u));
    let whole = [u.as_bytes(), b"\0"].concat();
    assert_eq!(active_uniform_cut(p, 0, 101), (whole.clone(), 100), "len + 1: whole and a NUL");
    assert_eq!(active_uniform_cut(p, 0, 100), ([&u.as_bytes()[..99], b"\0"].concat(), 99), "len: cut by one");
    assert_eq!(active_uniform_cut(p, 0, 64), ([&u.as_bytes()[..63], b"\0"].concat(), 63), "the field's size");
    assert_eq!(active_uniform_cut(p, 0, 1), (b"\0".to_vec(), 0), "1: empty and a NUL");
    assert_eq!(active_uniform_cut(p, 0, 0), (Vec::new(), 0), "0: nothing written");
    let mut len: GLsizei = -1;
    let (mut size, mut ty): (GLint, GLenum) = (0, 0);
    // SAFETY: plain GL call; a NULL name buffer with a non-zero bufSize.
    unsafe { glGetActiveUniform(p, 0, 200, &mut len, &mut size, &mut ty, core::ptr::null_mut()) };
    assert_eq!((len, size, ty), (0, 1, GL_FLOAT_VEC4), "NULL buffer: nothing written, length 0");
    assert_eq!(err(), GL_NO_ERROR);
    delete(p);
}

/// Two names alike in the field's 63 characters, different past them, in
/// one stage: two uniforms, each with its own value.
#[test]
fn two_long_names_that_differ_past_63_characters_are_two_uniforms() {
    let _g = context();
    let stem = "s".repeat(70);
    let (x, y) = (format!("{stem}_x"), format!("{stem}_y"));
    let p = linked(VS_PLAIN, &fs(&format!("uniform vec4 {x};\nuniform vec4 {y};"), &format!("{x} + {y}")));
    let mut names = active_names(p);
    names.sort();
    assert_eq!(names, [x.clone(), y.clone()]);
    let (lx, ly) = (location(p, &x), location(p, &y));
    assert!(lx >= 0 && ly >= 0 && lx != ly, "two locations: {lx} {ly}");
    set_and_get(p, lx, [1.0, 2.0, 3.0, 4.0]);
    set_and_get(p, ly, [5.0, 6.0, 7.0, 8.0]);
    assert_eq!(get(p, lx), [1.0, 2.0, 3.0, 4.0]);
    assert_eq!(get(p, ly), [5.0, 6.0, 7.0, 8.0]);
    assert_eq!(err(), GL_NO_ERROR);
    delete(p);
}

/// The same across the stages: the vertex shader's and the fragment
/// shader's long names are two uniforms when they differ past 63
/// characters, and one when they are the same name.
#[test]
fn long_names_across_the_stages_merge_by_the_whole_name() {
    let _g = context();
    let stem = "t".repeat(80);
    let (x, y) = (format!("{stem}_x"), format!("{stem}_y"));
    let p = linked(&vs(&format!("uniform vec4 {x};"), &x), &fs(&format!("uniform vec4 {y};"), &y));
    let mut names = active_names(p);
    names.sort();
    assert_eq!(names, [x.clone(), y.clone()], "different names: two uniforms");
    assert_ne!(location(p, &x), location(p, &y));
    delete(p);

    let p = linked(&vs(&format!("uniform vec4 {x};"), &x), &fs(&format!("uniform vec4 {x};"), &x));
    assert_eq!(active_names(p), std::slice::from_ref(&x), "one name in both stages: one uniform");
    let l = location(p, &x);
    assert_eq!(set_and_get(p, l, [9.0, 8.0, 7.0, 6.0]), [9.0, 8.0, 7.0, 6.0]);
    assert_eq!(err(), GL_NO_ERROR);
    delete(p);
}

/// An array behind a long name is `name[0]` of its length, and each element
/// has a location.
#[test]
fn an_array_behind_a_long_name_is_reported_with_its_zero() {
    let _g = context();
    let u = "r".repeat(100);
    let p = linked(VS_PLAIN, &fs(&format!("uniform vec4 {u}[3];"), &format!("{u}[0] + {u}[1] + {u}[2]")));
    let whole = format!("{u}[0]");
    assert_eq!(active_uniform(p, 0), (whole.clone(), 3, GL_FLOAT_VEC4));
    assert_eq!(programiv(p, GL_ACTIVE_UNIFORM_MAX_LENGTH), whole.len() as GLint + 1);
    assert_eq!(uniform_index(p, &u), 0, "\"a\" is \"a[0]\"");
    assert_eq!(uniform_index(p, &whole), 0);
    let (l0, l2) = (location(p, &u), location(p, &format!("{u}[2]")));
    assert!(l0 >= 0 && l2 >= 0 && l0 != l2);
    assert_eq!(location(p, &format!("{u}[3]")), -1, "past the array");
    set_and_get(p, l2, [3.0, 3.0, 3.0, 3.0]);
    assert_eq!(get(p, l2), [3.0, 3.0, 3.0, 3.0]);
    assert_eq!(get(p, l0), [0.0, 0.0, 0.0, 0.0], "element 2's value is its own");
    assert_eq!(err(), GL_NO_ERROR);
    delete(p);
}

/// An array of structs whose member's name is long: one active uniform per
/// element, each named and found whole.
#[test]
fn an_array_of_structs_with_a_long_member_name() {
    let _g = context();
    let m = "m".repeat(90);
    let p =
        linked(VS_PLAIN, &fs(&format!("struct S {{ vec4 {m}; }};\nuniform S s[2];"), &format!("s[0].{m} + s[1].{m}")));
    let (e0, e1) = (format!("s[0].{m}"), format!("s[1].{m}"));
    let mut names = active_names(p);
    names.sort();
    assert_eq!(names, [e0.clone(), e1.clone()]);
    let (l0, l1) = (location(p, &e0), location(p, &e1));
    assert!(l0 >= 0 && l1 >= 0 && l0 != l1, "{l0} {l1}");
    set_and_get(p, l1, [4.0, 3.0, 2.0, 1.0]);
    assert_eq!(get(p, l1), [4.0, 3.0, 2.0, 1.0]);
    assert_eq!(err(), GL_NO_ERROR);
    delete(p);
}

/// The language's limit stays: GLSL ES 3.00 section 3.8 allows identifiers of
/// at most 1,024 characters.
#[test]
fn an_identifier_of_1025_characters_is_still_refused() {
    let _g = context();
    let u = "z".repeat(1025);
    assert!(shader(GL_FRAGMENT_SHADER, &fs(&format!("uniform vec4 {u};"), &u)).is_err());
}
