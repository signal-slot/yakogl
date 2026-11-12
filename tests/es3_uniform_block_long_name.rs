// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Reflection names of 63 characters and more, reported whole through the GL
//! API on the host (docs/notes/reflection-long-names.md). The 64-byte
//! `ShcUniform::name` field stores 63 characters; the online compiler's
//! record keeps the whole name of a row the field cut, and the GL core names,
//! finds and reports every uniform row by it (`shaderc::uniform_row_name`).
//!
//! What is pinned:
//! - dEQP-GLES3 `functional.shaders.uniform_block.valid.long_instance_name_vertex`
//!   and `_fragment` (a block behind a 1,024-character instance name) link,
//!   and the API names are the block's: `UniformBlock`,
//!   `UniformBlock.uniformMember`;
//! - block and instance names of 70 and 200 characters: each block name
//!   whole from `glGetActiveUniformBlockName`, `GL_UNIFORM_BLOCK_NAME_LENGTH`
//!   and `GL_ACTIVE_UNIFORM_BLOCK_MAX_NAME_LENGTH` (longest + 1), and the
//!   members named by the block name, never the instance;
//! - default-block uniforms of 63, 64 and 200 characters (the first two
//!   share a 62-character prefix, so the field holds the same 63 bytes for
//!   both): each whole from `glGetActiveUniform`, found by
//!   `glGetUniformLocation` and `glGetUniformIndices` at a location of its
//!   own, `GL_ACTIVE_UNIFORM_MAX_LENGTH` the longest + 1 with the NUL;
//! - truncation to `bufSize`: `bufSize - 1` bytes and a NUL, `length`
//!   without the NUL, nothing written past it, for `bufSize` around the
//!   length, at the old field's 63 and 64, and 2, 1 and 0;
//! - a 1,025-character identifier is still a compile error (GLSL ES 3.00
//!   section 3.8: the language's limit, the only one).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test es3_uniform_block_long_name
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online --test es3_uniform_block_long_name
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online --test es3_uniform_block_long_name
//! ```

// Uniform blocks need the compiler and an ES 3.0 report; the other
// configurations build the file empty.
#![cfg(all(feature = "host-test", feature = "shaderc-online", not(feature = "report-es2")))]

use core::ffi::{c_char, c_void};
use std::ffi::{CStr, CString};
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
    name: c"es3-ublock-long-name".as_ptr() as *const c_char,
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

/* ---- names and programs ---------------------------------------------------- */

/// An identifier of exactly `len` characters: `head` and then `x`s.
fn ident(head: &str, len: usize) -> String {
    assert!(len >= head.len());
    let mut s = String::from(head);
    s.extend(core::iter::repeat_n('x', len - head.len()));
    s
}

const VS_PLAIN: &str = "#version 300 es\nin vec4 a_pos;\nvoid main() { gl_Position = a_pos; }\n";
const FS_PLAIN: &str = "#version 300 es\nprecision mediump float;\nout vec4 o;\nvoid main() { o = vec4(1.0); }\n";

/// A compiled shader, or the compile's info log.
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

/// Both stages compiled and linked; a compile or link failure fails the test.
fn program(vs_src: &str, fs_src: &str) -> GLuint {
    let vs = shader(GL_VERTEX_SHADER, vs_src).unwrap_or_else(|e| panic!("vertex compile: {e}\n{vs_src}"));
    let fs = shader(GL_FRAGMENT_SHADER, fs_src).unwrap_or_else(|e| panic!("fragment compile: {e}\n{fs_src}"));
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
        if ok != 1 {
            let mut log = [0 as c_char; 2048];
            glGetProgramInfoLog(p, 2048, core::ptr::null_mut(), log.as_mut_ptr());
            panic!("link failed: {}", CStr::from_ptr(log.as_ptr()).to_string_lossy());
        }
        p
    }
}

fn program_iv(p: GLuint, pname: GLenum) -> GLint {
    let mut v: GLint = -1;
    // SAFETY: plain GL call; one value.
    unsafe { glGetProgramiv(p, pname, &mut v) };
    v
}

fn delete(p: GLuint) {
    // SAFETY: a program of this context.
    unsafe { glDeleteProgram(p) };
}

/* ---- the copy-out contract ------------------------------------------------- */

/// What a name query wrote into a buffer of `buf_size` bytes (filled with
/// 0x7f first): the bytes up to the NUL, `length`, and whether every byte
/// past the NUL is still untouched.
struct Out {
    name: Vec<u8>,
    length: GLsizei,
    untouched_after: bool,
}

fn copy_out(buf_size: usize, q: impl FnOnce(GLsizei, *mut GLsizei, *mut GLchar)) -> Out {
    let mut buf = vec![0x7f as c_char; buf_size + 8];
    let mut length: GLsizei = -1;
    q(buf_size as GLsizei, &mut length, buf.as_mut_ptr());
    let bytes: Vec<u8> = buf.iter().map(|c| *c as u8).collect();
    match bytes[..buf_size].iter().position(|c| *c == 0) {
        Some(n) => {
            Out { name: bytes[..n].to_vec(), length, untouched_after: bytes[n + 1..].iter().all(|c| *c == 0x7f) }
        }
        // no NUL inside the buffer: only right for bufSize 0
        None => Out { name: Vec::new(), length, untouched_after: bytes.iter().all(|c| *c == 0x7f) },
    }
}

/// The whole name with room to spare, exactly, one byte short, cut at the
/// old field's 63 and 64, and two, one and no bytes: each answer as ES 3.0
/// 2.12.6 gives it for a name `want`.
fn check_copy_out(what: &str, want: &str, q: impl Fn(GLsizei, *mut GLsizei, *mut GLchar)) {
    let n = want.len();
    for buf_size in [n + 1 + 5, n + 1, n, 65, 64, 63, 2, 1, 0] {
        let out = copy_out(buf_size, &q);
        let kept = n.min(buf_size.saturating_sub(1));
        assert_eq!(String::from_utf8_lossy(&out.name), &want[..kept], "{what} ({n} characters), bufSize {buf_size}");
        assert_eq!(out.length as usize, kept, "{what}, bufSize {buf_size}: length (without the NUL)");
        assert!(out.untouched_after, "{what}, bufSize {buf_size}: a byte past the NUL was written");
        assert_eq!(err(), GL_NO_ERROR, "{what}, bufSize {buf_size}");
    }
}

/// `glGetActiveUniform(index)`'s whole name, size and type.
fn active_uniform(p: GLuint, index: u32) -> (String, GLint, GLenum) {
    let mut buf = vec![0 as c_char; 2048];
    let (mut len, mut size, mut ty): (GLsizei, GLint, GLenum) = (-1, 0, 0);
    // SAFETY: plain GL call; every pointer has the room it announces.
    unsafe { glGetActiveUniform(p, index, buf.len() as GLsizei, &mut len, &mut size, &mut ty, buf.as_mut_ptr()) };
    // SAFETY: the call NUL-terminated the buffer.
    let name = unsafe { CStr::from_ptr(buf.as_ptr()) }.to_string_lossy().into_owned();
    assert_eq!(len as usize, name.len(), "the length written is the name's");
    (name, size, ty)
}

/// Every active uniform's name, in index order.
fn active_uniform_names(p: GLuint) -> Vec<String> {
    (0..program_iv(p, GL_ACTIVE_UNIFORMS) as u32).map(|i| active_uniform(p, i).0).collect()
}

/// `glGetActiveUniformBlockName(index)`, whole.
fn block_name(p: GLuint, index: u32) -> String {
    let mut buf = vec![0 as c_char; 2048];
    let mut len: GLsizei = -1;
    // SAFETY: plain GL call; every pointer has the room it announces.
    unsafe { glGetActiveUniformBlockName(p, index, buf.len() as GLsizei, &mut len, buf.as_mut_ptr()) };
    // SAFETY: the call NUL-terminated the buffer.
    let name = unsafe { CStr::from_ptr(buf.as_ptr()) }.to_string_lossy().into_owned();
    assert_eq!(len as usize, name.len(), "the length written is the name's");
    name
}

fn block_index(p: GLuint, name: &str) -> GLuint {
    let c = CString::new(name).unwrap();
    // SAFETY: plain GL call; a NUL-terminated name.
    unsafe { glGetUniformBlockIndex(p, c.as_ptr()) }
}

fn block_iv(p: GLuint, index: u32, pname: GLenum) -> GLint {
    let mut v: GLint = -1;
    // SAFETY: plain GL call; one value.
    unsafe { glGetActiveUniformBlockiv(p, index, pname, &mut v) };
    v
}

fn uniform_index(p: GLuint, name: &str) -> GLuint {
    let c = CString::new(name).unwrap();
    let names: [*const c_char; 1] = [c.as_ptr()];
    let mut idx: [GLuint; 1] = [7777];
    // SAFETY: plain GL call; one name, room for one index.
    unsafe { glGetUniformIndices(p, 1, names.as_ptr(), idx.as_mut_ptr()) };
    idx[0]
}

fn uniform_iv(p: GLuint, index: u32, pname: GLenum) -> GLint {
    let idx: [GLuint; 1] = [index];
    let mut v: GLint = -1;
    // SAFETY: plain GL call; one index, room for one value.
    unsafe { glGetActiveUniformsiv(p, 1, idx.as_ptr(), pname, &mut v) };
    v
}

fn location(p: GLuint, name: &str) -> GLint {
    let c = CString::new(name).unwrap();
    // SAFETY: plain GL call; a NUL-terminated name.
    unsafe { glGetUniformLocation(p, c.as_ptr()) }
}

/* ---- dEQP-GLES3 uniform_block.valid.long_instance_name_* ------------------ */

/// The two cases as data/gles3/shaders/uniform_block.test spells them: a
/// block behind a 1,024-character instance name ("Total of 1024
/// characters"), read in the vertex, resp. the fragment shader. The case is
/// `build_successful`; the names are what the API must hand out.
fn long_instance_name(vertex: bool) {
    let _g = context();
    let a = "a".repeat(1024);
    let block = format!("uniform UniformBlock\n{{\n    vec4 uniformMember;\n}} {a};\n// Total of 1024 characters\n");
    let p = if vertex {
        let vs = format!(
            "#version 300 es\nprecision highp float;\n\n{block}\nvoid main()\n{{\n    gl_Position = {a}.uniformMember;\n}}\n"
        );
        program(&vs, FS_PLAIN)
    } else {
        let fs = format!(
            "#version 300 es\nprecision highp float;\n\nlayout(location=0) out vec4 o_color;\n\n{block}\n\
             void main()\n{{\n    o_color = {a}.uniformMember;\n}}\n"
        );
        program(VS_PLAIN, &fs)
    };
    let case = if vertex { "long_instance_name_vertex" } else { "long_instance_name_fragment" };
    assert_eq!(program_iv(p, GL_ACTIVE_UNIFORM_BLOCKS), 1, "{case}");
    assert_eq!(block_name(p, 0), "UniformBlock", "{case}: the block's own name");
    assert_eq!(block_index(p, "UniformBlock"), 0, "{case}");
    assert_eq!(program_iv(p, GL_ACTIVE_UNIFORM_BLOCK_MAX_NAME_LENGTH), 13, "{case}: `UniformBlock` and the NUL");
    assert_eq!(active_uniform_names(p), ["UniformBlock.uniformMember"], "{case}: named by the block, not the instance");
    assert_eq!(program_iv(p, GL_ACTIVE_UNIFORM_MAX_LENGTH), 27, "{case}");
    assert_eq!(uniform_index(p, &format!("{a}.uniformMember")), GL_INVALID_INDEX, "{case}: no name has the instance");
    assert_eq!(err(), GL_NO_ERROR, "{case}");
    delete(p);
}

#[test]
fn long_instance_name_vertex_links() {
    long_instance_name(true);
}

#[test]
fn long_instance_name_fragment_links() {
    long_instance_name(false);
}

/* ---- block and instance names of 70 and 200 characters -------------------- */

/// Two blocks, one named with 70 characters behind a 200-character instance
/// and one named with 200 behind a 70-character instance, read in `stage`.
fn long_blocks(vertex: bool) {
    let _g = context();
    let (b70, i200) = (ident("B", 70), ident("i", 200));
    let (b200, i70) = (ident("C", 200), ident("j", 70));
    let decls = format!("uniform {b70} {{ vec4 m; }} {i200};\nuniform {b200} {{ vec4 m; }} {i70};\n");
    let sum = format!("{i200}.m + {i70}.m");
    let p = if vertex {
        program(&format!("#version 300 es\n{decls}void main() {{ gl_Position = {sum}; }}\n"), FS_PLAIN)
    } else {
        let fs =
            format!("#version 300 es\nprecision mediump float;\n{decls}out vec4 o;\nvoid main() {{ o = {sum}; }}\n");
        program(VS_PLAIN, &fs)
    };
    let ctx = if vertex { "vertex" } else { "fragment" };
    assert_eq!(program_iv(p, GL_ACTIVE_UNIFORM_BLOCKS), 2, "{ctx}");
    assert_eq!(program_iv(p, GL_ACTIVE_UNIFORM_BLOCK_MAX_NAME_LENGTH), 201, "{ctx}: 200 characters and the NUL");
    for want in [&b70, &b200] {
        let k = block_index(p, want);
        assert_ne!(k, GL_INVALID_INDEX, "{ctx}: {} characters find the block", want.len());
        assert_eq!(&block_name(p, k), want, "{ctx}: the whole block name");
        assert_eq!(block_iv(p, k, GL_UNIFORM_BLOCK_NAME_LENGTH), want.len() as GLint + 1, "{ctx}");
        check_copy_out(&format!("{ctx}: block name"), want, |n, l, b| {
            // SAFETY: plain GL call; `b` holds `n` bytes, `l` one GLsizei.
            unsafe { glGetActiveUniformBlockName(p, k, n, l, b) }
        });
        /* the member, by the block name: 72 and 202 characters */
        let member = format!("{want}.m");
        let u = uniform_index(p, &member);
        assert_ne!(u, GL_INVALID_INDEX, "{ctx}: the member `{}...m` ({} characters)", &want[..4], member.len());
        assert_eq!(active_uniform(p, u).0, member, "{ctx}");
        assert_eq!(uniform_iv(p, u, GL_UNIFORM_NAME_LENGTH), member.len() as GLint + 1, "{ctx}");
        assert_eq!(uniform_iv(p, u, GL_UNIFORM_BLOCK_INDEX), k as GLint, "{ctx}");
        check_copy_out(&format!("{ctx}: member name"), &member, |n, l, b| {
            // SAFETY: plain GL call; every pointer has the room it announces.
            unsafe { glGetActiveUniform(p, u, n, l, core::ptr::null_mut(), core::ptr::null_mut(), b) }
        });
    }
    for inst in [&i200, &i70] {
        assert_eq!(block_index(p, inst), GL_INVALID_INDEX, "{ctx}: an instance name names no block");
        assert_eq!(uniform_index(p, &format!("{inst}.m")), GL_INVALID_INDEX, "{ctx}: nor a member");
    }
    assert_eq!(program_iv(p, GL_ACTIVE_UNIFORM_MAX_LENGTH), 203, "{ctx}: `<200>.m` and the NUL");
    assert_eq!(err(), GL_NO_ERROR, "{ctx}");
    delete(p);
}

#[test]
fn block_and_instance_names_of_70_and_200_characters_in_the_vertex_shader() {
    long_blocks(true);
}

#[test]
fn block_and_instance_names_of_70_and_200_characters_in_the_fragment_shader() {
    long_blocks(false);
}

/* ---- default-block uniforms of 63, 64 and 200 characters ------------------ */

/// The three names: 63 and 64 characters on one 62-character prefix (the
/// 64-byte field holds the same 63 bytes of the 64-character name as the
/// whole 63-character one), and 200.
fn long_uniforms() -> [String; 3] {
    let prefix = ident("u", 62);
    [format!("{prefix}a"), format!("{prefix}bb"), ident("w", 200)]
}

fn check_long_uniforms(p: GLuint, ctx: &str) {
    let names = long_uniforms();
    let mut active = active_uniform_names(p);
    active.sort();
    let mut want: Vec<String> = names.to_vec();
    want.sort();
    assert_eq!(active, want, "{ctx}: every name whole");
    assert_eq!(program_iv(p, GL_ACTIVE_UNIFORM_MAX_LENGTH), 201, "{ctx}: 200 characters and the NUL");
    assert_eq!(program_iv(p, GL_ACTIVE_UNIFORM_BLOCKS), 0, "{ctx}");
    assert_eq!(program_iv(p, GL_ACTIVE_UNIFORM_BLOCK_MAX_NAME_LENGTH), 0, "{ctx}: no block");
    let mut locs = Vec::new();
    for (k, n) in names.iter().enumerate() {
        let u = uniform_index(p, n);
        assert_ne!(u, GL_INVALID_INDEX, "{ctx}: {} characters", n.len());
        assert_eq!(active_uniform(p, u), (n.clone(), 1, GL_FLOAT_VEC4), "{ctx}: {} characters", n.len());
        assert_eq!(uniform_iv(p, u, GL_UNIFORM_NAME_LENGTH), n.len() as GLint + 1, "{ctx}");
        assert_eq!(uniform_iv(p, u, GL_UNIFORM_BLOCK_INDEX), -1, "{ctx}: a default-block uniform");
        check_copy_out(&format!("{ctx}: uniform of {} characters", n.len()), n, |sz, l, b| {
            // SAFETY: plain GL call; every pointer has the room it announces.
            unsafe { glGetActiveUniform(p, u, sz, l, core::ptr::null_mut(), core::ptr::null_mut(), b) }
        });
        let l = location(p, n);
        assert!(l >= 0, "{ctx}: {} characters have a location", n.len());
        locs.push(l);
        /* each location is its own storage: write k, read it back */
        let v = [k as f32, 10.0, 20.0, 30.0];
        let mut got = [0f32; 4];
        // SAFETY: plain GL calls on the current program; room for a vec4.
        unsafe {
            glUseProgram(p);
            glUniform4f(l, v[0], v[1], v[2], v[3]);
            glGetUniformfv(p, l, got.as_mut_ptr());
        }
        assert_eq!(got, v, "{ctx}: {} characters", n.len());
    }
    let mut distinct = locs.clone();
    distinct.sort();
    distinct.dedup();
    assert_eq!(distinct.len(), 3, "{ctx}: three uniforms, three locations: {locs:?}");
    for (k, l) in locs.iter().enumerate() {
        let mut got = [0f32; 4];
        // SAFETY: plain GL call on a linked program; room for a vec4.
        unsafe { glGetUniformfv(p, *l, got.as_mut_ptr()) };
        assert_eq!(got[0], k as f32, "{ctx}: no write reached another uniform");
    }
    /* the field's 63 bytes name neither of the two that share them past the
     * 63-character one, and a name one past the 200 is no uniform */
    assert_eq!(location(p, &format!("{}b", ident("u", 62))), -1, "{ctx}: the 64-character name's first 63");
    assert_eq!(location(p, &format!("{}x", names[2])), -1, "{ctx}");
    assert_eq!(location(p, &names[2][..63]), -1, "{ctx}: the 200-character name cut at the old field");
    // SAFETY: plain GL call.
    unsafe { glUseProgram(0) };
    assert_eq!(err(), GL_NO_ERROR, "{ctx}");
}

#[test]
fn uniforms_of_63_64_and_200_characters_in_the_fragment_shader() {
    let _g = context();
    let [a, b, c] = long_uniforms();
    let fs = format!(
        "#version 300 es\nprecision mediump float;\nuniform vec4 {a};\nuniform vec4 {b};\nuniform vec4 {c};\n\
         out vec4 o;\nvoid main() {{ o = {a} + {b} + {c}; }}\n"
    );
    let p = program(VS_PLAIN, &fs);
    check_long_uniforms(p, "fragment");
    delete(p);
}

#[test]
fn uniforms_of_63_64_and_200_characters_in_the_vertex_shader() {
    let _g = context();
    let [a, b, c] = long_uniforms();
    let vs = format!(
        "#version 300 es\nin vec4 a_pos;\nuniform vec4 {a};\nuniform vec4 {b};\nuniform vec4 {c};\n\
         void main() {{ gl_Position = a_pos + {a} + {b} + {c}; }}\n"
    );
    let p = program(&vs, FS_PLAIN);
    check_long_uniforms(p, "vertex");
    delete(p);
}

/// One 200-character uniform declared in both stages is one active uniform:
/// the link matches the two stages' rows by the whole name.
#[test]
fn a_200_character_uniform_of_both_stages_is_one_uniform() {
    let _g = context();
    let c = ident("w", 200);
    let vs =
        format!("#version 300 es\nin vec4 a_pos;\nuniform vec4 {c};\nvoid main() {{ gl_Position = a_pos + {c}; }}\n");
    /* highp, as the vertex shader's default: a shared uniform's precision must match */
    let fs = format!(
        "#version 300 es\nprecision highp float;\nuniform vec4 {c};\nout vec4 o;\nvoid main() {{ o = {c}; }}\n"
    );
    let p = program(&vs, &fs);
    assert_eq!(active_uniform_names(p), [c.clone()]);
    assert!(location(p, &c) >= 0);
    assert_eq!(err(), GL_NO_ERROR);
    delete(p);
}

/// An array of 64 characters is reported `name[0]` (67 characters), and
/// both `name` and `name[1]` find their elements.
#[test]
fn an_array_of_64_characters_is_reported_with_its_subscript() {
    let _g = context();
    let n = ident("v", 64);
    let fs = format!(
        "#version 300 es\nprecision mediump float;\nuniform vec4 {n}[2];\nout vec4 o;\nvoid main() {{ o = {n}[0] + {n}[1]; }}\n"
    );
    let p = program(VS_PLAIN, &fs);
    let gl_name = format!("{n}[0]");
    assert_eq!(active_uniform(p, 0), (gl_name.clone(), 2, GL_FLOAT_VEC4));
    assert_eq!(program_iv(p, GL_ACTIVE_UNIFORM_MAX_LENGTH), 68);
    assert_eq!(uniform_index(p, &n), 0, "`a` is also `a[0]`");
    assert_eq!(uniform_index(p, &gl_name), 0);
    let (l0, l1) = (location(p, &n), location(p, &format!("{n}[1]")));
    assert!(l0 >= 0 && l1 >= 0 && l0 != l1, "{l0} {l1}");
    assert_eq!(location(p, &gl_name), l0);
    assert_eq!(location(p, &format!("{n}[2]")), -1, "past the array");
    assert_eq!(err(), GL_NO_ERROR);
    delete(p);
}

/* ---- the language's limit stays -------------------------------------------- */

/// GLSL ES 3.00 section 3.8: an identifier of more than 1,024 characters is
/// a compile error. 1,024 compiles (the dEQP cases above).
#[test]
fn an_identifier_of_1025_characters_is_still_a_compile_error() {
    let _g = context();
    let n = ident("u", 1025);
    let fs = format!(
        "#version 300 es\nprecision mediump float;\nuniform vec4 {n};\nout vec4 o;\nvoid main() {{ o = {n}; }}\n"
    );
    assert!(shader(GL_FRAGMENT_SHADER, &fs).is_err(), "1,025 characters must not compile");
    assert_eq!(err(), GL_NO_ERROR);
}
