// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The names an ES 3.0 program with uniform blocks reports, through the GL
//! API on the host, for names that fit the 64-byte reflection field (1, 31
//! and 62 characters) and for names it cannot hold whole (63, 64 and 200
//! characters), which the online compiler's name pool keeps and the GL core
//! reports whole (`docs/notes/reflection-long-names.md`). Default-block
//! uniforms of those lengths are checked beside the blocks.
//!
//! What is pinned, per program:
//! - `GL_ACTIVE_UNIFORM_BLOCK_MAX_NAME_LENGTH` is the longest block name + 1
//!   and `GL_UNIFORM_BLOCK_NAME_LENGTH` each block's name + 1 (ES 3.0 6.1.12);
//! - `glGetActiveUniformBlockName` and `glGetActiveUniform` hand out the
//!   whole name with `length` the bytes written without the NUL, and cut to
//!   `bufSize - 1` bytes plus a NUL for a smaller buffer, writing nothing
//!   past it (ES 3.0 2.12.6);
//! - `GL_ACTIVE_UNIFORM_MAX_LENGTH` is the longest active uniform name + 1
//!   and `GL_UNIFORM_NAME_LENGTH` each one's;
//! - a block member is named by the block's NAME, never its instance name:
//!   `Block.member` for a block with an instance name, `member` for one
//!   without (GLSL ES 3.00 4.3.7, ES 3.0 2.12.6), and the name each query
//!   reports finds the same index again.
//!
//! The blocks are declared in the vertex shader, then in the fragment
//! shader, the two stages dEQP's `uniform_block.valid.long_instance_name_*`
//! puts its block in.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test es3_ubo_name_queries
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online --test es3_ubo_name_queries
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_ubo_name_queries
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
    name: c"es3-ubo-names".as_ptr() as *const c_char,
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

/// An identifier of exactly `len` characters: `head` and then `x`s. Every
/// name of a program starts with a letter of its own, so no two collide.
fn ident(head: char, len: usize) -> String {
    assert!(len >= 1);
    let mut s = String::with_capacity(len);
    s.push(head);
    s.extend(core::iter::repeat_n('x', len - 1));
    s
}

/// One `vec4` block of one member, and what the API must call them.
struct Block {
    name: String,
    instance: Option<String>,
    member: String,
}

impl Block {
    fn decl(&self) -> String {
        match &self.instance {
            Some(i) => format!("uniform {} {{ vec4 {}; }} {};\n", self.name, self.member, i),
            None => format!("uniform {} {{ vec4 {}; }};\n", self.name, self.member),
        }
    }
    /// The member as the shader reads it: through the instance name.
    fn read(&self) -> String {
        match &self.instance {
            Some(i) => format!("{i}.{}", self.member),
            None => self.member.clone(),
        }
    }
    /// The member as the API names it: through the block name.
    fn api(&self) -> String {
        match &self.instance {
            Some(_) => format!("{}.{}", self.name, self.member),
            None => self.member.clone(),
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum Stage {
    Vertex,
    Fragment,
}

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

/// A linked program whose `stage` declares and reads `blocks`.
/// `defaults`: `vec4` default-block uniforms it declares and reads too.
fn program(blocks: &[Block], defaults: &[String], stage: Stage) -> GLuint {
    let decls: String =
        blocks.iter().map(Block::decl).chain(defaults.iter().map(|d| format!("uniform vec4 {d};\n"))).collect();
    let sum: String =
        blocks.iter().map(|b| format!(" + {}", b.read())).chain(defaults.iter().map(|d| format!(" + {d}"))).collect();
    let (vs_src, fs_src) = match stage {
        Stage::Vertex => (
            format!("#version 300 es\nin vec4 a_pos;\n{decls}void main() {{ gl_Position = a_pos{sum}; }}\n"),
            "#version 300 es\nprecision mediump float;\nout vec4 o;\nvoid main() { o = vec4(1.0); }\n".to_string(),
        ),
        Stage::Fragment => (
            "#version 300 es\nin vec4 a_pos;\nvoid main() { gl_Position = a_pos; }\n".to_string(),
            format!(
                "#version 300 es\nprecision mediump float;\n{decls}out vec4 o;\nvoid main() {{ o = vec4(0.0){sum}; }}\n"
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
            let mut log = [0 as c_char; 1024];
            glGetProgramInfoLog(p, 1024, core::ptr::null_mut(), log.as_mut_ptr());
            panic!("link failed: {}\n{vs_src}\n{fs_src}", CStr::from_ptr(log.as_ptr()).to_string_lossy());
        }
        p
    }
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

/// Runs `q(bufSize, length, buf)` with a buffer of `buf_size` bytes and a
/// spare tail it must not write.
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

/// The whole name, a buffer one byte short, two bytes, one byte and none:
/// each answer as ES 3.0 2.12.6 gives it for a name `want`.
fn check_copy_out(what: &str, want: &str, q: impl Fn(GLsizei, *mut GLsizei, *mut GLchar)) {
    let n = want.len();
    for buf_size in [n + 1 + 5, n + 1, n, 2, 1, 0] {
        let out = copy_out(buf_size, &q);
        let kept = n.min(buf_size.saturating_sub(1));
        assert_eq!(
            String::from_utf8_lossy(&out.name),
            &want[..kept],
            "{what} '{want}' ({n} characters), bufSize {buf_size}: the name"
        );
        assert_eq!(out.length as usize, kept, "{what} '{want}', bufSize {buf_size}: length (without the NUL)");
        assert!(out.untouched_after, "{what} '{want}', bufSize {buf_size}: a byte past the NUL was written");
        assert_eq!(err(), GL_NO_ERROR, "{what} '{want}', bufSize {buf_size}");
    }
}

/* ---- the checks ------------------------------------------------------------ */

fn program_iv(p: GLuint, pname: GLenum) -> GLint {
    let mut v: GLint = -1;
    // SAFETY: plain GL call; one value.
    unsafe { glGetProgramiv(p, pname, &mut v) };
    v
}

/// Every name query of the note's list, for a program declaring `blocks`
/// in `stage`.
fn check_names(blocks: &[Block], stage: Stage) {
    let _g = context();
    let p = program(blocks, &[], stage);
    let ctx = format!("{stage:?} shader");

    /* the blocks */
    assert_eq!(program_iv(p, GL_ACTIVE_UNIFORM_BLOCKS), blocks.len() as GLint, "{ctx}: active blocks");
    let longest_block = blocks.iter().map(|b| b.name.len()).max().unwrap();
    assert_eq!(
        program_iv(p, GL_ACTIVE_UNIFORM_BLOCK_MAX_NAME_LENGTH),
        longest_block as GLint + 1,
        "{ctx}: GL_ACTIVE_UNIFORM_BLOCK_MAX_NAME_LENGTH"
    );
    let mut block_of = Vec::new();
    for b in blocks {
        let c = CString::new(b.name.as_str()).unwrap();
        // SAFETY: plain GL call; a NUL-terminated name.
        let k = unsafe { glGetUniformBlockIndex(p, c.as_ptr()) };
        assert_ne!(k, GL_INVALID_INDEX, "{ctx}: glGetUniformBlockIndex('{}')", b.name);
        let mut len: GLint = -1;
        // SAFETY: plain GL call; one value.
        unsafe { glGetActiveUniformBlockiv(p, k, GL_UNIFORM_BLOCK_NAME_LENGTH, &mut len) };
        assert_eq!(len, b.name.len() as GLint + 1, "{ctx}: GL_UNIFORM_BLOCK_NAME_LENGTH of '{}'", b.name);
        check_copy_out("glGetActiveUniformBlockName", &b.name, |size, length, buf| {
            // SAFETY: `buf` has room for `size` bytes, `length` for one value.
            unsafe { glGetActiveUniformBlockName(p, k, size, length, buf) }
        });
        // a NULL `length` is allowed and changes nothing of the name
        let out = copy_out(b.name.len() + 1, |size, _, buf| {
            // SAFETY: as above, without `length`.
            unsafe { glGetActiveUniformBlockName(p, k, size, core::ptr::null_mut(), buf) }
        });
        assert_eq!(String::from_utf8_lossy(&out.name), b.name, "{ctx}: block name with a NULL length");
        block_of.push(k);
    }

    /* the members */
    let n = program_iv(p, GL_ACTIVE_UNIFORMS);
    assert_eq!(n, blocks.len() as GLint, "{ctx}: active uniforms (one member per block)");
    let longest_member = blocks.iter().map(|b| b.api().len()).max().unwrap();
    assert_eq!(
        program_iv(p, GL_ACTIVE_UNIFORM_MAX_LENGTH),
        longest_member as GLint + 1,
        "{ctx}: GL_ACTIVE_UNIFORM_MAX_LENGTH"
    );
    for (b, &k) in blocks.iter().zip(&block_of) {
        let api = b.api();
        let c = CString::new(api.as_str()).unwrap();
        let names = [c.as_ptr()];
        let mut i: GLuint = GL_INVALID_INDEX;
        // SAFETY: plain GL call; one NUL-terminated name and one index.
        unsafe { glGetUniformIndices(p, 1, names.as_ptr(), &mut i) };
        assert_ne!(i, GL_INVALID_INDEX, "{ctx}: glGetUniformIndices('{api}')");
        if let Some(inst) = &b.instance {
            // the instance name is the shader's, never the API's
            let c = CString::new(format!("{inst}.{}", b.member)).unwrap();
            let names = [c.as_ptr()];
            let mut j: GLuint = 0;
            // SAFETY: as above.
            unsafe { glGetUniformIndices(p, 1, names.as_ptr(), &mut j) };
            assert_eq!(j, GL_INVALID_INDEX, "{ctx}: '{inst}.{}' is not an API name", b.member);
        }
        let iv = |pname| {
            let mut v: GLint = -1;
            // SAFETY: plain GL call; one index and one value.
            unsafe { glGetActiveUniformsiv(p, 1, &i, pname, &mut v) };
            v
        };
        assert_eq!(iv(GL_UNIFORM_NAME_LENGTH), api.len() as GLint + 1, "{ctx}: GL_UNIFORM_NAME_LENGTH of '{api}'");
        assert_eq!(iv(GL_UNIFORM_BLOCK_INDEX), k as GLint, "{ctx}: GL_UNIFORM_BLOCK_INDEX of '{api}'");
        check_copy_out("glGetActiveUniform", &api, |size, length, buf| {
            let (mut sz, mut ty): (GLint, GLenum) = (0, 0);
            // SAFETY: `buf` has room for `size` bytes; one value each else.
            unsafe { glGetActiveUniform(p, i, size, length, &mut sz, &mut ty, buf) };
            assert_eq!((sz, ty), (1, GL_FLOAT_VEC4), "{ctx}: size and type of '{api}'");
        });
    }

    // SAFETY: plain GL call on the program this test made.
    unsafe { glDeleteProgram(p) };
    assert_eq!(err(), GL_NO_ERROR, "{ctx}");
}

/// Blocks without an instance name: block names of 1, 31 and 62
/// characters, members reported bare, of 1, 31 and 62 characters.
fn anonymous() -> Vec<Block> {
    let v = vec![
        Block { name: ident('A', 1), instance: None, member: ident('m', 1) },
        Block { name: ident('B', 31), instance: None, member: ident('n', 31) },
        Block { name: ident('C', 62), instance: None, member: ident('o', 62) },
    ];
    for (b, len) in v.iter().zip([1, 31, 62]) {
        assert_eq!((b.name.len(), b.api().len()), (len, len));
    }
    v
}

/// Blocks with an instance name: block names of 1, 31 and 29 characters,
/// members reported `Block.member`, of 62, 62 and 31 characters -- and
/// read through instance names that are none of those.
fn named() -> Vec<Block> {
    let v = vec![
        Block { name: ident('D', 1), instance: Some(ident('i', 1)), member: ident('p', 60) },
        Block { name: ident('E', 31), instance: Some(ident('j', 31)), member: ident('q', 30) },
        Block { name: ident('F', 29), instance: Some(ident('k', 62 - 30)), member: ident('r', 1) },
    ];
    for (b, (name, api)) in v.iter().zip([(1, 62), (31, 62), (29, 31)]) {
        assert_eq!((b.name.len(), b.api().len()), (name, api));
    }
    v
}

#[test]
fn anonymous_blocks_in_the_vertex_shader() {
    check_names(&anonymous(), Stage::Vertex);
}

#[test]
fn anonymous_blocks_in_the_fragment_shader() {
    check_names(&anonymous(), Stage::Fragment);
}

#[test]
fn named_instances_in_the_vertex_shader() {
    check_names(&named(), Stage::Vertex);
}

#[test]
fn named_instances_in_the_fragment_shader() {
    check_names(&named(), Stage::Fragment);
}

/// One block per name length, alone in its program: the maximum is that
/// block's own length + 1, not the longest of a set. A 62-character block
/// with an instance name would report its member as 64 characters, one of
/// the cases of 63 and more below.
#[test]
fn one_block_each_length() {
    for stage in [Stage::Vertex, Stage::Fragment] {
        for len in [1, 31, 62] {
            check_names(&[Block { name: ident('H', len), instance: None, member: ident('u', len) }], stage);
        }
        for (len, member) in [(1, 1), (31, 30), (60, 1)] {
            let b = Block { name: ident('G', len), instance: Some(ident('s', 1)), member: ident('t', member) };
            assert!(b.api().len() <= 62);
            check_names(&[b], stage);
        }
    }
}

/* ---- names of 63 characters and more --------------------------------------- */

/// Names the 64-byte reflection field cannot hold whole (63 fills it):
/// block names, bare members and `Block.member` names of 63, 64 and 200
/// characters, each alone in its program and three together. The record's
/// name pool keeps the rows' whole names, and every query hands them out
/// whole (`docs/notes/reflection-long-names.md`).
#[test]
fn block_and_member_names_of_63_64_and_200_characters() {
    for stage in [Stage::Vertex, Stage::Fragment] {
        for len in [63, 64, 200] {
            check_names(&[Block { name: ident('H', len), instance: None, member: ident('u', len) }], stage);
            let b = Block { name: ident('G', len), instance: Some(ident('s', len)), member: ident('t', len) };
            check_names(&[b], stage);
        }
        let anonymous = [
            Block { name: ident('A', 63), instance: None, member: ident('m', 63) },
            Block { name: ident('B', 64), instance: None, member: ident('n', 64) },
            Block { name: ident('C', 200), instance: None, member: ident('o', 200) },
        ];
        check_names(&anonymous, stage);
        let named = [
            Block { name: ident('D', 1), instance: Some(ident('i', 1)), member: ident('p', 61) },
            Block { name: ident('E', 31), instance: Some(ident('j', 200)), member: ident('q', 32) },
            Block { name: ident('F', 62), instance: Some(ident('k', 62)), member: ident('r', 137) },
        ];
        for (b, api) in named.iter().zip([63, 64, 200]) {
            assert_eq!(b.api().len(), api);
        }
        check_names(&named, stage);
    }
}

/// Every name query of a default-block `vec4` per name of `names`, declared
/// in `stage`: `GL_ACTIVE_UNIFORM_MAX_LENGTH`, `glGetUniformIndices`,
/// `GL_UNIFORM_NAME_LENGTH`, no block, `glGetActiveUniform` at the buffer
/// sizes of [`check_copy_out`], and a location of its own.
fn check_default_names(names: &[String], stage: Stage) {
    let _g = context();
    let p = program(&[], names, stage);
    let ctx = format!("{stage:?} shader");
    assert_eq!(program_iv(p, GL_ACTIVE_UNIFORMS), names.len() as GLint, "{ctx}: active uniforms");
    let longest = names.iter().map(String::len).max().unwrap();
    assert_eq!(
        program_iv(p, GL_ACTIVE_UNIFORM_MAX_LENGTH),
        longest as GLint + 1,
        "{ctx}: GL_ACTIVE_UNIFORM_MAX_LENGTH"
    );
    let mut locations = Vec::new();
    for name in names {
        let c = CString::new(name.as_str()).unwrap();
        let ptrs = [c.as_ptr()];
        let mut i: GLuint = GL_INVALID_INDEX;
        // SAFETY: plain GL call; one NUL-terminated name and one index.
        unsafe { glGetUniformIndices(p, 1, ptrs.as_ptr(), &mut i) };
        assert_ne!(i, GL_INVALID_INDEX, "{ctx}: glGetUniformIndices('{name}')");
        let iv = |pname| {
            let mut v: GLint = -2;
            // SAFETY: plain GL call; one index and one value.
            unsafe { glGetActiveUniformsiv(p, 1, &i, pname, &mut v) };
            v
        };
        assert_eq!(iv(GL_UNIFORM_NAME_LENGTH), name.len() as GLint + 1, "{ctx}: GL_UNIFORM_NAME_LENGTH");
        assert_eq!(iv(GL_UNIFORM_BLOCK_INDEX), -1, "{ctx}: a default-block uniform is in no block");
        check_copy_out("glGetActiveUniform", name, |size, length, buf| {
            let (mut sz, mut ty): (GLint, GLenum) = (0, 0);
            // SAFETY: `buf` has room for `size` bytes; one value each else.
            unsafe { glGetActiveUniform(p, i, size, length, &mut sz, &mut ty, buf) };
            assert_eq!((sz, ty), (1, GL_FLOAT_VEC4), "{ctx}: size and type of '{name}'");
        });
        // SAFETY: plain GL call; a NUL-terminated name.
        let loc = unsafe { glGetUniformLocation(p, c.as_ptr()) };
        assert!(loc >= 0, "{ctx}: glGetUniformLocation('{name}')");
        assert!(!locations.contains(&loc), "{ctx}: '{name}' has a location of its own");
        locations.push(loc);
    }
    // SAFETY: plain GL call on the program this test made.
    unsafe { glDeleteProgram(p) };
    assert_eq!(err(), GL_NO_ERROR, "{ctx}");
}

/// Default-block uniforms of 62 (one under the field's limit), 63, 64 and
/// 200 characters, each alone and all in one program.
#[test]
fn default_block_uniforms_of_63_64_and_200_characters() {
    for stage in [Stage::Vertex, Stage::Fragment] {
        for len in [62, 63, 64, 200] {
            check_default_names(&[ident('d', len)], stage);
        }
        check_default_names(&[ident('e', 62), ident('f', 63), ident('g', 64), ident('h', 200)], stage);
    }
}
