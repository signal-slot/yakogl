// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Uniform and uniform-block names of 63 characters and more, through the
//! GL API on the host, on each GPU's online back end: they link, and every
//! query hands them out whole (`docs/notes/reflection-long-names.md`).
//!
//! The 64-byte `ShcUniform::name` field holds 63 characters; the online
//! compiler's record keeps the whole name of each row the field cut (its
//! name pool, `shaderc::online::backend::UniformNames`), and the GL core
//! takes every uniform row's name from there (`program::row_name`). So a
//! default-block uniform of 63, 64 or 200 characters is reported whole by
//! `glGetActiveUniform` at every `bufSize` (cut only by the caller's
//! buffer, ES 3.0 2.12.6), measured whole by `GL_UNIFORM_NAME_LENGTH` and
//! `GL_ACTIVE_UNIFORM_MAX_LENGTH`, and found by `glGetUniformIndices` and
//! `glGetUniformLocation` -- names that share the field's 63 bytes too,
//! each at its own location. A block name of 70 characters is reported
//! whole by `glGetActiveUniformBlockName` and
//! `GL_ACTIVE_UNIFORM_BLOCK_MAX_NAME_LENGTH`, and its members' names
//! (`Block.member`) by the active-uniform queries.
//!
//! The host cannot render: the verdicts are the link, the queries and the
//! values `glGetUniformfv` reads back.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test es3_uniform_block_long_name
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online --test es3_uniform_block_long_name
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_uniform_block_long_name
//! ```

// The names need the compiler's name pool and an ES 3.0 report; the other
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
    name: c"es3-uniform-block-long-name".as_ptr() as *const c_char,
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

/* ---- the programs ---------------------------------------------------------- */

/// An identifier of exactly `len` characters: `head` and then `x`s.
fn ident(head: char, len: usize) -> String {
    assert!(len >= 1);
    let mut s = String::with_capacity(len);
    s.push(head);
    s.extend(core::iter::repeat_n('x', len - 1));
    s
}

/// `prefix` and then `tail` as many `a`s as make `len` characters.
fn with_prefix(prefix: &str, len: usize) -> String {
    assert!(len > prefix.len());
    format!("{prefix}{}", "a".repeat(len - prefix.len()))
}

#[derive(Clone, Copy, Debug)]
enum Stage {
    Vertex,
    Fragment,
}

const STAGES: [Stage; 2] = [Stage::Vertex, Stage::Fragment];

fn shader(ty: GLenum, text: &str) -> GLuint {
    let c = CString::new(text).unwrap();
    // SAFETY: plain GL calls; one NUL-terminated source and a log buffer.
    unsafe {
        let s = glCreateShader(ty);
        let p: [*const c_char; 1] = [c.as_ptr()];
        glShaderSource(s, 1, p.as_ptr(), core::ptr::null());
        glCompileShader(s);
        let mut ok: GLint = 0;
        glGetShaderiv(s, GL_COMPILE_STATUS, &mut ok);
        if ok != 1 {
            let mut log = [0 as c_char; 1024];
            glGetShaderInfoLog(s, 1024, core::ptr::null_mut(), log.as_mut_ptr());
            panic!("compile failed: {}\n{text}", CStr::from_ptr(log.as_ptr()).to_string_lossy());
        }
        s
    }
}

/// A linked program whose `stage` declares `decls` and adds up `reads`
/// (each a `vec4` expression) into its output.
fn program(decls: &str, reads: &[String], stage: Stage) -> GLuint {
    let sum: String = reads.iter().map(|r| format!(" + {r}")).collect();
    let (vs_src, fs_src) = match stage {
        Stage::Vertex => (
            format!("#version 300 es\nin vec4 a_pos;\n{decls}void main() {{ gl_Position = a_pos{sum}; }}\n"),
            "#version 300 es\nprecision mediump float;\nout vec4 o;\nvoid main() { o = vec4(1.0); }\n".to_string(),
        ),
        Stage::Fragment => (
            "#version 300 es\nin vec4 a_pos;\nvoid main() { gl_Position = a_pos; }\n".to_string(),
            format!(
                "#version 300 es\nprecision highp float;\n{decls}out vec4 o;\nvoid main() {{ o = vec4(0.0){sum}; }}\n"
            ),
        ),
    };
    let vs = shader(GL_VERTEX_SHADER, &vs_src);
    let fs = shader(GL_FRAGMENT_SHADER, &fs_src);
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
            panic!("{stage:?}: link failed: {}", CStr::from_ptr(log.as_ptr()).to_string_lossy());
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

/* ---- the copy-out contract ------------------------------------------------- */

/// For EVERY `bufSize` from 0 to the whole name and its NUL and a few more:
/// `q(bufSize, length, buf)` writes the first `bufSize - 1` bytes of `want`
/// and a NUL, `length` is the bytes written without the NUL, and no byte
/// past the NUL is touched (ES 3.0 2.12.6).
fn check_every_buf_size(what: &str, want: &str, q: impl Fn(GLsizei, *mut GLsizei, *mut GLchar)) {
    let n = want.len();
    for buf_size in 0..=n + 4 {
        let mut buf = vec![0x7f as c_char; buf_size + 8];
        let mut length: GLsizei = -1;
        q(buf_size as GLsizei, &mut length, buf.as_mut_ptr());
        let bytes: Vec<u8> = buf.iter().map(|c| *c as u8).collect();
        let kept = n.min(buf_size.saturating_sub(1));
        if buf_size == 0 {
            assert!(bytes.iter().all(|c| *c == 0x7f), "{what} ({n}), bufSize 0 writes nothing");
        } else {
            assert_eq!(&bytes[..kept], &want.as_bytes()[..kept], "{what} ({n}), bufSize {buf_size}: the bytes");
            assert_eq!(bytes[kept], 0, "{what} ({n}), bufSize {buf_size}: the NUL");
            assert!(bytes[kept + 1..].iter().all(|c| *c == 0x7f), "{what} ({n}), bufSize {buf_size}: past the NUL");
        }
        assert_eq!(length as usize, kept, "{what} ({n}), bufSize {buf_size}: length");
        assert_eq!(err(), GL_NO_ERROR, "{what} ({n}), bufSize {buf_size}");
    }
}

/* ---- default-block uniforms ------------------------------------------------ */

/// Every name query and the location of each `vec4` default-block uniform
/// `names` declared in `stage`: `glGetActiveUniform` (at every buffer
/// size), `GL_UNIFORM_NAME_LENGTH`, `glGetUniformIndices`,
/// `GL_ACTIVE_UNIFORM_MAX_LENGTH`, and a location per name whose value
/// reads back as the one set there and no other uniform's.
fn check_default_uniforms(names: &[String], stage: Stage) {
    let _g = context();
    let decls: String = names.iter().map(|n| format!("uniform vec4 {n};\n")).collect();
    let p = program(&decls, names, stage);
    let ctx = format!("{stage:?} shader");
    assert_eq!(program_iv(p, GL_ACTIVE_UNIFORMS), names.len() as GLint, "{ctx}: active uniforms");
    let longest = names.iter().map(String::len).max().unwrap();
    assert_eq!(program_iv(p, GL_ACTIVE_UNIFORM_MAX_LENGTH), longest as GLint + 1, "{ctx}: MAX_LENGTH");
    // SAFETY: plain GL call on the program this test made.
    unsafe { glUseProgram(p) };
    let mut locations = Vec::new();
    for (j, name) in names.iter().enumerate() {
        let c = CString::new(name.as_str()).unwrap();
        let ptrs = [c.as_ptr()];
        let mut i: GLuint = GL_INVALID_INDEX;
        // SAFETY: plain GL call; one NUL-terminated name and one index.
        unsafe { glGetUniformIndices(p, 1, ptrs.as_ptr(), &mut i) };
        assert_ne!(i, GL_INVALID_INDEX, "{ctx}: glGetUniformIndices of the {}-character name", name.len());
        let mut len: GLint = -1;
        // SAFETY: plain GL call; one index and one value.
        unsafe { glGetActiveUniformsiv(p, 1, &i, GL_UNIFORM_NAME_LENGTH, &mut len) };
        assert_eq!(len, name.len() as GLint + 1, "{ctx}: GL_UNIFORM_NAME_LENGTH");
        check_every_buf_size(&format!("{ctx}: glGetActiveUniform"), name, |size, length, buf| {
            let (mut sz, mut ty): (GLint, GLenum) = (0, 0);
            // SAFETY: `buf` has room for `size` bytes; one value each else.
            unsafe { glGetActiveUniform(p, i, size, length, &mut sz, &mut ty, buf) };
            assert_eq!((sz, ty), (1, GL_FLOAT_VEC4), "{ctx}: size and type");
        });
        // SAFETY: plain GL calls; a NUL-terminated name.
        let loc = unsafe { glGetUniformLocation(p, c.as_ptr()) };
        assert!(loc >= 0, "{ctx}: the {}-character name has a location", name.len());
        // SAFETY: a location of the program in use.
        unsafe { glUniform4f(loc, j as f32 + 1.0, 0.5, 0.25, -(j as f32)) };
        assert_eq!(err(), GL_NO_ERROR, "{ctx}: glUniform4f of the {}-character name", name.len());
        locations.push(loc);
    }
    /* every location is its own uniform: each reads back its own value */
    for (j, (&loc, name)) in locations.iter().zip(names).enumerate() {
        let mut v = [0f32; 4];
        // SAFETY: plain GL call; room for a vec4.
        unsafe { glGetUniformfv(p, loc, v.as_mut_ptr()) };
        assert_eq!(v, [j as f32 + 1.0, 0.5, 0.25, -(j as f32)], "{ctx}: the {}-character uniform", name.len());
    }
    /* a name the field would have cut to is no uniform */
    for name in names.iter().filter(|n| n.len() > 63) {
        let c = CString::new(&name[..63]).unwrap();
        if names.iter().any(|n| n.as_str() == &name[..63]) {
            continue;
        }
        // SAFETY: plain GL call; a NUL-terminated name.
        assert_eq!(unsafe { glGetUniformLocation(p, c.as_ptr()) }, -1, "{ctx}: the 63-byte cut is no name");
    }
    // SAFETY: plain GL calls on the program this test made.
    unsafe {
        glUseProgram(0);
        glDeleteProgram(p);
    }
    assert_eq!(err(), GL_NO_ERROR, "{ctx}");
}

/// Default-block uniforms of 63, 64 and 200 characters in one program.
#[test]
fn default_block_uniforms_of_63_64_and_200_characters() {
    for stage in STAGES {
        check_default_uniforms(&[ident('d', 63), ident('e', 64), ident('f', 200)], stage);
    }
}

/// Each alone in its program, so the maximum length is its own.
#[test]
fn one_default_block_uniform_each_length() {
    for stage in STAGES {
        for len in [62, 63, 64, 200] {
            check_default_uniforms(&[ident('g', len)], stage);
        }
    }
}

/// Names sharing a 62-character prefix, the field's 63 bytes and more:
/// the 64- and 200-character names are cut to the 63-character one's
/// bytes, so only the whole names tell the three rows apart.
#[test]
fn default_block_uniforms_sharing_a_62_character_prefix() {
    let prefix = ident('p', 62);
    for stage in STAGES {
        let names = [with_prefix(&prefix, 63), with_prefix(&prefix, 64), with_prefix(&prefix, 200)];
        assert!(names[1].starts_with(&names[0]) && names[2].starts_with(&names[1]));
        check_default_uniforms(&names, stage);
        /* and two that differ only past the field */
        let b = [format!("{prefix}ab"), format!("{prefix}ac")];
        check_default_uniforms(&b, stage);
    }
}

/// A long array: reported as `name[0]` of size 2, found as `name`,
/// `name[0]` and `name[1]`.
#[test]
fn a_default_block_array_of_100_characters() {
    for stage in STAGES {
        let _g = context();
        let name = ident('r', 100);
        let p = program(&format!("uniform vec4 {name}[2];\n"), &[format!("{name}[0] + {name}[1]")], stage);
        let gl = format!("{name}[0]");
        assert_eq!(program_iv(p, GL_ACTIVE_UNIFORM_MAX_LENGTH), gl.len() as GLint + 1);
        check_every_buf_size("glGetActiveUniform of an array", &gl, |size, length, buf| {
            let (mut sz, mut ty): (GLint, GLenum) = (0, 0);
            // SAFETY: `buf` has room for `size` bytes; one value each else.
            unsafe { glGetActiveUniform(p, 0, size, length, &mut sz, &mut ty, buf) };
            assert_eq!((sz, ty), (2, GL_FLOAT_VEC4));
        });
        for (n, want) in [(name.clone(), 0usize), (gl.clone(), 0), (format!("{name}[1]"), 1)] {
            let c = CString::new(n.as_str()).unwrap();
            let ptrs = [c.as_ptr()];
            let mut i: GLuint = GL_INVALID_INDEX;
            // SAFETY: plain GL calls; NUL-terminated names, one index.
            let loc = unsafe {
                glGetUniformIndices(p, 1, ptrs.as_ptr(), &mut i);
                glGetUniformLocation(p, c.as_ptr())
            };
            assert!(loc >= 0, "{stage:?}: location of element {want}");
            if n.ends_with("[1]") {
                assert_eq!(i, GL_INVALID_INDEX, "{stage:?}: an element past 0 is no active uniform's name");
            } else {
                assert_eq!(i, 0, "{stage:?}: '{}...' finds the array", &n[..8]);
            }
        }
        // SAFETY: plain GL call on the program this test made.
        unsafe { glDeleteProgram(p) };
        assert_eq!(err(), GL_NO_ERROR);
    }
}

/* ---- uniform blocks --------------------------------------------------------- */

/// A block of `block` characters with one `vec4` member, with an instance
/// name or without: `glGetUniformBlockIndex`, the block's name at every
/// buffer size, `GL_UNIFORM_BLOCK_NAME_LENGTH`,
/// `GL_ACTIVE_UNIFORM_BLOCK_MAX_NAME_LENGTH`, and the member's API name
/// (`Block.member`, or the bare member) through the active-uniform queries.
fn check_block(block: &str, instance: Option<&str>, member: &str, stage: Stage) {
    let _g = context();
    let (decl, read, api) = match instance {
        Some(i) => (
            format!("uniform {block} {{ vec4 {member}; }} {i};\n"),
            format!("{i}.{member}"),
            format!("{block}.{member}"),
        ),
        None => (format!("uniform {block} {{ vec4 {member}; }};\n"), member.to_string(), member.to_string()),
    };
    let p = program(&decl, &[read], stage);
    let ctx = format!("{stage:?} shader, block of {}", block.len());
    assert_eq!(program_iv(p, GL_ACTIVE_UNIFORM_BLOCKS), 1, "{ctx}");
    assert_eq!(program_iv(p, GL_ACTIVE_UNIFORM_BLOCK_MAX_NAME_LENGTH), block.len() as GLint + 1, "{ctx}");
    let c = CString::new(block).unwrap();
    // SAFETY: plain GL call; a NUL-terminated name.
    let k = unsafe { glGetUniformBlockIndex(p, c.as_ptr()) };
    assert_eq!(k, 0, "{ctx}: glGetUniformBlockIndex");
    let c = CString::new(&block[..63]).unwrap();
    // SAFETY: as above.
    assert_eq!(unsafe { glGetUniformBlockIndex(p, c.as_ptr()) }, GL_INVALID_INDEX, "{ctx}: a cut name is no block");
    let mut len: GLint = -1;
    // SAFETY: plain GL call; one value.
    unsafe { glGetActiveUniformBlockiv(p, k, GL_UNIFORM_BLOCK_NAME_LENGTH, &mut len) };
    assert_eq!(len, block.len() as GLint + 1, "{ctx}: GL_UNIFORM_BLOCK_NAME_LENGTH");
    check_every_buf_size(&format!("{ctx}: glGetActiveUniformBlockName"), block, |size, length, buf| {
        // SAFETY: `buf` has room for `size` bytes, `length` for one value.
        unsafe { glGetActiveUniformBlockName(p, k, size, length, buf) }
    });
    assert_eq!(program_iv(p, GL_ACTIVE_UNIFORMS), 1, "{ctx}: the member");
    assert_eq!(program_iv(p, GL_ACTIVE_UNIFORM_MAX_LENGTH), api.len() as GLint + 1, "{ctx}: MAX_LENGTH");
    let c = CString::new(api.as_str()).unwrap();
    let ptrs = [c.as_ptr()];
    let mut i: GLuint = GL_INVALID_INDEX;
    // SAFETY: plain GL call; one NUL-terminated name and one index.
    unsafe { glGetUniformIndices(p, 1, ptrs.as_ptr(), &mut i) };
    assert_eq!(i, 0, "{ctx}: glGetUniformIndices of the member");
    let mut bi: GLint = -1;
    // SAFETY: plain GL call; one index and one value.
    unsafe { glGetActiveUniformsiv(p, 1, &i, GL_UNIFORM_BLOCK_INDEX, &mut bi) };
    assert_eq!(bi, 0, "{ctx}: the member is the block's");
    let mut nl: GLint = -1;
    // SAFETY: as above.
    unsafe { glGetActiveUniformsiv(p, 1, &i, GL_UNIFORM_NAME_LENGTH, &mut nl) };
    assert_eq!(nl, api.len() as GLint + 1, "{ctx}: GL_UNIFORM_NAME_LENGTH");
    check_every_buf_size(&format!("{ctx}: glGetActiveUniform"), &api, |size, length, buf| {
        let (mut sz, mut ty): (GLint, GLenum) = (0, 0);
        // SAFETY: `buf` has room for `size` bytes; one value each else.
        unsafe { glGetActiveUniform(p, i, size, length, &mut sz, &mut ty, buf) };
        assert_eq!((sz, ty), (1, GL_FLOAT_VEC4), "{ctx}");
    });
    // SAFETY: plain GL call; a NUL-terminated name.
    assert_eq!(unsafe { glGetUniformLocation(p, c.as_ptr()) }, -1, "{ctx}: a block member has no location");
    // SAFETY: plain GL call on the program this test made.
    unsafe { glDeleteProgram(p) };
    assert_eq!(err(), GL_NO_ERROR, "{ctx}");
}

/// A block name of 70 characters, without an instance name and with one,
/// the member short and long.
#[test]
fn a_block_name_of_70_characters() {
    let block = ident('B', 70);
    for stage in STAGES {
        check_block(&block, None, "m", stage);
        check_block(&block, Some("i"), "m", stage);
        check_block(&block, Some(&ident('j', 70)), &ident('n', 70), stage);
        check_block(&block, None, &ident('o', 200), stage);
    }
}
