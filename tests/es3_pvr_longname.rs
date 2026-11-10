// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Uniform-block members whose internal row name is longer than the 64-byte
//! `ShcUniform::name` stores, on the PowerVR online back end
//! (docs/notes/reflection-long-names.md): dEQP-GLES3
//! `functional.shaders.uniform_block.valid.long_instance_name_vertex` and
//! `..._fragment` declare `uniform UniformBlock { vec4 uniformMember; }`
//! with an instance name of 1,024 `a`s, so the row the shader reads is
//! `<1024 a's>.uniformMember`, 1,038 characters, which the field cuts to 63.
//! The two programs link, the GL-visible names are the block type's
//! (`UniformBlock`, `UniformBlock.uniformMember`), and the draw fills the
//! member's row from the bound buffer: the member's row was resolved by its
//! whole name (the compiler's name pool), not by the cut field.
//!
//! What still refuses until the GL core reads default-block names from the
//! pool: a default-block uniform of 64 or more characters, alone, and two
//! such uniforms cut alike, one in each stage (they would be merged into one
//! location under the cut name).
//!
//! HOST-ONLY: the draw is the GL core's; what the GPU computes is the board's.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test es3_pvr_longname
//! ```
#![cfg(all(feature = "host-test", feature = "shaderc-online", feature = "gpu-powervr", not(feature = "report-es2")))]

use core::ffi::{c_char, c_void};
use std::ffi::CString;
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{GlesDraw, GlesHeap, YakoglBackend};
use yakogl::gl::buffer::*;
use yakogl::gl::consts::*;
use yakogl::gl::draw::*;
use yakogl::gl::es3::*;
use yakogl::gl::program::*;
use yakogl::gl::shader::*;
use yakogl::gl::state::*;
use yakogl::gl::types::*;
use yakogl::gl::vertex::*;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};

/* ---- a context on the host, as tests/es3_ubo.rs makes one --------------- */

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

/// One stage as the last draw handed it over: each uniform row's dword
/// offset and dwords, and the stage's uniform dwords.
#[derive(Clone, Default, Debug)]
struct Stage {
    rows: Vec<(u16, u16)>,
    block: Vec<u32>,
}

static DRAWN: Mutex<Option<(Stage, Stage)>> = Mutex::new(None);

unsafe fn stage_of(sh: *const yakogl::shaderc::ShcShader, cpu: *const c_void, dwords: u16) -> Stage {
    let Some(sh) = sh.as_ref() else { return Stage::default() };
    let rows = sh.uniform_table().iter().map(|u| (u.dword_off, u.dwords)).collect();
    let block = if cpu.is_null() {
        Vec::new()
    } else {
        core::slice::from_raw_parts(cpu as *const u32, usize::from(dwords)).to_vec()
    };
    Stage { rows, block }
}

unsafe extern "C" fn b_draw(d: *const GlesDraw) -> i32 {
    let d = &*d;
    let vs = stage_of(d.vs, d.vs_uniform_cpu, d.vs_uniform_dwords);
    let fs = stage_of(d.fs, d.fs_uniform_cpu, d.fs_uniform_dwords);
    *DRAWN.lock().unwrap_or_else(|e| e.into_inner()) = Some((vs, fs));
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
    name: c"es3-pvr-longname".as_ptr() as *const c_char,
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

/* ---- the programs ------------------------------------------------------ */

/// The instance name of both dEQP cases: "Total of 1024 characters".
fn instance() -> String {
    "a".repeat(1024)
}

/// `long_instance_name_vertex` as `data/gles3/shaders/uniform_block.test`
/// writes it: the block, and its member read, in the vertex shader.
fn long_instance_vertex() -> (String, String) {
    let i = instance();
    (
        format!(
            "#version 300 es\nprecision highp float;\n\n\n\n\
             uniform UniformBlock\n{{\n    vec4 uniformMember;\n}} {i};\n// Total of 1024 characters\n\n\
             void main()\n{{\n    gl_Position = {i}.uniformMember;\n}}\n"
        ),
        "#version 300 es\nprecision highp float;\n\nlayout(location=0) out vec4 o_color;\n\n\
         void main()\n{\n    o_color = vec4(1.0);\n}\n"
            .into(),
    )
}

/// `long_instance_name_fragment`: the block, and its member read, in the
/// fragment shader.
fn long_instance_fragment() -> (String, String) {
    let i = instance();
    (
        "#version 300 es\nprecision highp float;\n\nin vec4 a_pos;\n\n\
         void main()\n{\n    gl_Position = a_pos;\n}\n"
            .into(),
        format!(
            "#version 300 es\nprecision highp float;\n\nlayout(location=0) out vec4 o_color;\n\n\
             uniform UniformBlock\n{{\n    vec4 uniformMember;\n}} {i};\n// Total of 1024 characters\n\n\
             void main()\n{{\n    o_color = {i}.uniformMember;\n}}\n"
        ),
    )
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
        let mut log = [0 as c_char; 1024];
        glGetShaderInfoLog(s, 1024, core::ptr::null_mut(), log.as_mut_ptr());
        assert_eq!(ok, 1, "compile: {}", core::ffi::CStr::from_ptr(log.as_ptr()).to_string_lossy());
        s
    }
}

/// The program of `vs` and `fs`, linked or not, and its info log.
fn program(vs: &str, fs: &str) -> (GLuint, bool, String) {
    let (v, f) = (shader(GL_VERTEX_SHADER, vs), shader(GL_FRAGMENT_SHADER, fs));
    // SAFETY: plain GL calls on handles this test owns; a log buffer of the
    // size passed.
    unsafe {
        let p = glCreateProgram();
        glAttachShader(p, v);
        glAttachShader(p, f);
        glLinkProgram(p);
        glDeleteShader(v);
        glDeleteShader(f);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        let mut log = [0 as c_char; 1024];
        glGetProgramInfoLog(p, 1024, core::ptr::null_mut(), log.as_mut_ptr());
        (p, ok == 1, core::ffi::CStr::from_ptr(log.as_ptr()).to_string_lossy().into_owned())
    }
}

/// The program's API names: the blocks' and the active uniforms'.
///
/// # Safety
/// `p` is a linked program of the current context.
unsafe fn api_names(p: GLuint) -> (Vec<String>, Vec<String>) {
    let name =
        |buf: &[c_char], n: GLsizei| String::from_utf8(buf[..n as usize].iter().map(|&c| c as u8).collect()).unwrap();
    let (mut nb, mut nu) = (0, 0);
    glGetProgramiv(p, GL_ACTIVE_UNIFORM_BLOCKS, &mut nb);
    glGetProgramiv(p, GL_ACTIVE_UNIFORMS, &mut nu);
    let mut buf = [0 as c_char; 2048];
    let blocks = (0..nb as u32)
        .map(|k| {
            let mut n: GLsizei = 0;
            glGetActiveUniformBlockName(p, k, buf.len() as GLsizei, &mut n, buf.as_mut_ptr());
            name(&buf, n)
        })
        .collect();
    let uniforms = (0..nu as u32)
        .map(|k| {
            let (mut n, mut size, mut ty): (GLsizei, GLint, GLenum) = (0, 0, 0);
            glGetActiveUniform(p, k, buf.len() as GLsizei, &mut n, &mut size, &mut ty, buf.as_mut_ptr());
            assert_eq!((size, ty), (1, GL_FLOAT_VEC4), "uniform {k}");
            name(&buf, n)
        })
        .collect();
    (blocks, uniforms)
}

/// Draw `p` with the block's buffer holding vec4(1, 2, 3, 4): the stage
/// that declares the block as the draw handed it over.
///
/// # Safety
/// `p` is a linked program of the current context.
unsafe fn draw_with_member(p: GLuint, vertex: bool) -> Stage {
    static POS: [f32; 12] = [-1.0, -1.0, 0.0, 1.0, 1.0, -1.0, 0.0, 1.0, -1.0, 1.0, 0.0, 1.0];
    let data: [f32; 4] = [1.0, 2.0, 3.0, 4.0];
    glUseProgram(p);
    let k = glGetUniformBlockIndex(p, c"UniformBlock".as_ptr());
    glUniformBlockBinding(p, k, 0);
    let mut buf: GLuint = 0;
    glGenBuffers(1, &mut buf);
    glBindBuffer(GL_UNIFORM_BUFFER, buf);
    glBufferData(GL_UNIFORM_BUFFER, 16, data.as_ptr() as *const c_void, GL_STATIC_DRAW);
    glBindBufferBase(GL_UNIFORM_BUFFER, 0, buf);
    let loc = glGetAttribLocation(p, c"a_pos".as_ptr());
    glBindBuffer(GL_ARRAY_BUFFER, 0);
    if loc >= 0 {
        glVertexAttribPointer(loc as u32, 4, GL_FLOAT, 0, 0, POS.as_ptr() as *const c_void);
        glEnableVertexAttribArray(loc as u32);
    }
    *DRAWN.lock().unwrap_or_else(|e| e.into_inner()) = None;
    glDrawArrays(GL_TRIANGLES, 0, 3);
    assert_eq!(glGetError(), GL_NO_ERROR, "the draw");
    if loc >= 0 {
        glDisableVertexAttribArray(loc as u32);
    }
    glDeleteBuffers(1, &buf);
    glUseProgram(0);
    let (vs, fs) = DRAWN.lock().unwrap_or_else(|e| e.into_inner()).take().expect("the back end drew");
    if vertex {
        vs
    } else {
        fs
    }
}

/// One of the two dEQP cases: it links, its API names are the block type's,
/// and the draw fills the member's row of the declaring stage.
fn long_instance_case(vs: &str, fs: &str, vertex: bool) {
    let _g = context();
    let (p, linked, log) = program(vs, fs);
    assert!(linked, "the program must link (build_successful): {log}");
    // SAFETY: a linked program of the current context.
    unsafe {
        let (blocks, uniforms) = api_names(p);
        assert_eq!(blocks, ["UniformBlock"], "the block's GL name is its type name");
        assert_eq!(uniforms, ["UniformBlock.uniformMember"], "the member's GL name has the type name's prefix");
        assert_ne!(glGetUniformBlockIndex(p, c"UniformBlock".as_ptr()), GL_INVALID_INDEX);
        let name = c"UniformBlock.uniformMember";
        let mut index: GLuint = GL_INVALID_INDEX;
        glGetUniformIndices(p, 1, [name.as_ptr()].as_ptr(), &mut index);
        assert_eq!(index, 0, "glGetUniformIndices finds the member by its GL name");
        let mut max: GLint = 0;
        glGetProgramiv(p, GL_ACTIVE_UNIFORM_MAX_LENGTH, &mut max);
        assert_eq!(max, "UniformBlock.uniformMember".len() as GLint + 1);
        assert_eq!(glGetError(), GL_NO_ERROR);

        let stage = draw_with_member(p, vertex);
        assert_eq!(stage.rows.len(), 1, "the declaring stage has the member's row: {stage:?}");
        let (off, dwords) = stage.rows[0];
        assert_eq!(dwords, 4, "{stage:?}");
        let got: Vec<f32> =
            stage.block[usize::from(off)..usize::from(off) + 4].iter().map(|&d| f32::from_bits(d)).collect();
        assert_eq!(got, [1.0, 2.0, 3.0, 4.0], "the member's row holds the bound buffer's vec4");
        glDeleteProgram(p);
    }
}

#[test]
fn long_instance_name_vertex_links_with_the_type_names() {
    let (vs, fs) = long_instance_vertex();
    long_instance_case(&vs, &fs, true);
}

#[test]
fn long_instance_name_fragment_links_with_the_type_names() {
    let (vs, fs) = long_instance_fragment();
    long_instance_case(&vs, &fs, false);
}

/// The compiler keeps the member row's whole name in its name pool: the
/// accessor the GL core will read default-block names through hands it back,
/// and nothing for a row whose field is whole.
#[test]
fn the_name_pool_holds_the_whole_row_name() {
    let _g = context();
    let (vs, _) = long_instance_vertex();
    let c = CString::new(vs).unwrap();
    let mut log = [0 as c_char; 512];
    let mut sh = yakogl::shaderc::ShcShader::ZERO;
    let backend = &yakogl::shaderc::online::SHC_ONLINE;
    // SAFETY: the back end's compile entry with a NUL-terminated source, a
    // log of the stated size and a record to fill; released below.
    unsafe {
        let rc = (backend.compile.unwrap())(
            yakogl::shaderc::SHC_STAGE_VERTEX,
            c.as_ptr(),
            &mut sh,
            log.as_mut_ptr(),
            log.len(),
        );
        assert_eq!(rc, 0, "{}", core::ffi::CStr::from_ptr(log.as_ptr()).to_string_lossy());
    }
    let whole = format!("{}.uniformMember", instance());
    assert_eq!(sh.uniform_table().len(), 1);
    assert_eq!(yakogl::util::name_bytes(&sh.uniform_table()[0].name), &whole.as_bytes()[..63], "the field: cut");
    assert_eq!(yakogl::shaderc::uniform_row_name(&sh, 0), Some(whole.as_str()), "the pool: whole");
    assert_eq!(yakogl::shaderc::uniform_row_name(&sh, 1), None, "no row 1");
    let blocks = yakogl::shaderc::uniform_blocks(&sh);
    assert_eq!(blocks.members.len(), 1);
    assert_eq!((blocks.members[0].name, blocks.members[0].row), ("UniformBlock.uniformMember", 0));
    yakogl::shaderc::release_shader(&sh);
}

/// A default-block uniform is named, matched and reported by its 64-byte
/// field in the GL core until that reads the pool: one of 64 or more
/// characters still refuses the link rather than answer to a cut name.
#[test]
fn a_long_default_block_uniform_still_refuses_the_link() {
    let _g = context();
    let name = format!("{}X", "u".repeat(70));
    let vs = format!(
        "#version 300 es\nin highp vec4 a_pos;\nuniform highp vec4 {name};\n\
         void main()\n{{\n    gl_Position = a_pos + {name};\n}}\n"
    );
    let fs = "#version 300 es\nlayout(location = 0) out mediump vec4 o;\nvoid main()\n{\n    o = vec4(1.0);\n}\n";
    let (p, linked, log) = program(&vs, fs);
    assert!(!linked, "a 71-character default-block uniform must not link under its 63-byte cut");
    assert!(log.contains("may have been truncated"), "{log}");
    // SAFETY: a program of the current context.
    unsafe { glDeleteProgram(p) };
}

/// Two different default-block uniforms cut to the same 63 bytes, one in
/// each stage: linked, the GL core would merge them into one location under
/// the cut name. Refused.
#[test]
fn two_long_uniforms_cut_alike_one_per_stage_refuse_the_link() {
    let _g = context();
    let stem = "u".repeat(70);
    let vs = format!(
        "#version 300 es\nin highp vec4 a_pos;\nuniform highp vec4 {stem}X;\n\
         void main()\n{{\n    gl_Position = a_pos + {stem}X;\n}}\n"
    );
    let fs = format!(
        "#version 300 es\nlayout(location = 0) out mediump vec4 o;\nuniform mediump vec4 {stem}Y;\n\
         void main()\n{{\n    o = {stem}Y;\n}}\n"
    );
    let (p, linked, log) = program(&vs, &fs);
    assert!(!linked, "two uniforms cut alike must not link as one: {log}");
    assert!(log.contains("may have been truncated"), "{log}");
    // SAFETY: a program of the current context.
    unsafe { glDeleteProgram(p) };
}
