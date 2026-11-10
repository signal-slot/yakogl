// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! dEQP-GLES3 `functional.shaders.uniform_block.valid.long_instance_name_{vertex,fragment}`
//! on the PowerVR online back end, through the GL API on the host: a block
//! `UniformBlock { vec4 uniformMember; }` with an instance name of 1,024
//! characters. The member's uniform row is named `<1,024 a's>.uniformMember`,
//! which the 64-byte `ShcUniform::name` field cuts at 63 a's; the record's
//! name pool (`shaderc::online::backend::UniformNames`) keeps the whole
//! name, the block table finds the member's row through it, and the link
//! accepts the stage. Both programs link, and every name GL hands out is
//! the short one: the block is `UniformBlock`, its member
//! `UniformBlock.uniformMember`; the instance name reaches no query.
//!
//! The file also pins what the pool does not lift: a default-block uniform
//! whose name fills the field is still refused at link, because the GL core
//! reads that row's field for `glGetActiveUniform` and
//! `glGetUniformLocation` (task #47's last step).
//!
//! The host cannot render: the verdicts here are the compile, the link, the
//! reflection and the uniform dwords a draw hands the back end (the member's
//! row filled from the bound buffer), HOST-ONLY like
//! `tools/cts/host-deqp-run.sh`'s.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test es3_pvr_longname
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-powervr", not(feature = "report-es2")))]
use core::ffi::{c_char, c_void, CStr};
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
/// One stage as the last draw handed it over: each uniform row's dword
/// offset and dwords, and the stage's uniform dwords as the GL core filled
/// them from the bound buffers.
#[derive(Clone, Default, Debug)]
struct Stage {
    rows: Vec<(u16, u16)>,
    dwords: Vec<u32>,
}

static DRAWN: Mutex<Option<(Stage, Stage)>> = Mutex::new(None);

unsafe fn stage_of(sh: *const yakogl::shaderc::ShcShader, cpu: *const c_void, dwords: u16) -> Stage {
    let Some(sh) = sh.as_ref() else { return Stage::default() };
    let rows = sh.uniform_table().iter().map(|u| (u.dword_off, u.dwords)).collect();
    let dwords = if cpu.is_null() {
        Vec::new()
    } else {
        core::slice::from_raw_parts(cpu as *const u32, usize::from(dwords)).to_vec()
    };
    Stage { rows, dwords }
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

/* ---- the two cases, as data/gles3/shaders/uniform_block.test spells them -- */

/// "Total of 1024 characters" (the .test file's own comment on the name).
fn instance_name() -> String {
    "a".repeat(1024)
}

const VS_PLAIN: &str =
    "#version 300 es\nprecision highp float;\n\nin vec4 a_pos;\n\nvoid main()\n{\n    gl_Position = a_pos;\n}\n";

const FS_PLAIN: &str = "#version 300 es\nprecision highp float;\n\nlayout(location=0) out vec4 o_color;\n\n\
                        void main()\n{\n    o_color = vec4(1.0);\n}\n";

/// `long_instance_name_vertex`'s vertex shader.
fn vs_long() -> String {
    let a = instance_name();
    format!(
        "#version 300 es\nprecision highp float;\n\n\n\nuniform UniformBlock\n{{\n    vec4 uniformMember;\n}} {a};\n\
         // Total of 1024 characters\n\nvoid main()\n{{\n    gl_Position = {a}.uniformMember;\n}}\n"
    )
}

/// `long_instance_name_fragment`'s fragment shader.
fn fs_long() -> String {
    let a = instance_name();
    format!(
        "#version 300 es\nprecision highp float;\n\nlayout(location=0) out vec4 o_color;\n\n\
         uniform UniformBlock\n{{\n    vec4 uniformMember;\n}} {a};\n// Total of 1024 characters\n\n\
         void main()\n{{\n    o_color = {a}.uniformMember;\n}}\n"
    )
}

/* ---- the GL side ------------------------------------------------------- */

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

fn programiv(p: GLuint, pname: GLenum) -> GLint {
    let mut v: GLint = -1;
    // SAFETY: plain GL call; one value.
    unsafe { glGetProgramiv(p, pname, &mut v) };
    v
}

/// `glGetActiveUniform(index)`: name, size, type.
fn active_uniform(p: GLuint, index: u32) -> (String, GLint, GLenum) {
    let mut buf = [0 as c_char; 256];
    let (mut len, mut size, mut ty): (GLsizei, GLint, GLenum) = (0, 0, 0);
    // SAFETY: plain GL call; every pointer has the room it announces.
    unsafe { glGetActiveUniform(p, index, buf.len() as GLsizei, &mut len, &mut size, &mut ty, buf.as_mut_ptr()) };
    let name = CStr::from_bytes_until_nul(c_bytes(&buf)).unwrap().to_string_lossy().into_owned();
    assert_eq!(len as usize, name.len(), "the length written is the name's");
    (name, size, ty)
}

fn c_bytes(c: &[c_char]) -> &[u8] {
    // SAFETY: c_char and u8 have the same layout.
    unsafe { core::slice::from_raw_parts(c.as_ptr().cast::<u8>(), c.len()) }
}

/// `glGetActiveUniformBlockName(index)`.
fn block_name(p: GLuint, index: u32) -> String {
    let mut buf = [0 as c_char; 256];
    let mut len: GLsizei = 0;
    // SAFETY: plain GL call; every pointer has the room it announces.
    unsafe { glGetActiveUniformBlockName(p, index, buf.len() as GLsizei, &mut len, buf.as_mut_ptr()) };
    let name = CStr::from_bytes_until_nul(c_bytes(&buf)).unwrap().to_string_lossy().into_owned();
    assert_eq!(len as usize, name.len(), "the length written is the name's");
    name
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

fn uniform_location(p: GLuint, name: &str) -> GLint {
    let c = CString::new(name).unwrap();
    // SAFETY: plain GL call; a NUL-terminated name.
    unsafe { glGetUniformLocation(p, c.as_ptr()) }
}

/// What dEQP's `ShaderCase` checks of a `build_successful` case, and the
/// reflection of the block behind it: the names GL hands out are the
/// block's, never the instance's.
fn check_long_instance_program(p: GLuint, case: &str) {
    assert_eq!(programiv(p, GL_ACTIVE_UNIFORM_BLOCKS), 1, "{case}: one block");
    assert_eq!(block_name(p, 0), "UniformBlock", "{case}: the block's own name");
    let c = CString::new("UniformBlock").unwrap();
    // SAFETY: plain GL call; a NUL-terminated name.
    assert_eq!(unsafe { glGetUniformBlockIndex(p, c.as_ptr()) }, 0, "{case}");
    assert_eq!(programiv(p, GL_ACTIVE_UNIFORMS), 1, "{case}: the member is the one active uniform");
    assert_eq!(
        active_uniform(p, 0),
        ("UniformBlock.uniformMember".to_string(), 1, GL_FLOAT_VEC4),
        "{case}: named `Block.member`, not by the instance"
    );
    assert_eq!(uniform_index(p, "UniformBlock.uniformMember"), 0, "{case}");
    let a = instance_name();
    assert_eq!(
        uniform_index(p, &format!("{a}.uniformMember")),
        GL_INVALID_INDEX,
        "{case}: no GL name has the instance"
    );
    assert_eq!(uniform_location(p, "UniformBlock.uniformMember"), -1, "{case}: a block member has no location");
    let mut off: GLint = -1;
    let idx: [GLuint; 1] = [0];
    // SAFETY: plain GL call; one index, room for one value.
    unsafe { glGetActiveUniformsiv(p, 1, idx.as_ptr(), GL_UNIFORM_OFFSET, &mut off) };
    assert_eq!(off, 0, "{case}: the member's std140 offset");
    let mut bi: GLint = -1;
    // SAFETY: plain GL call; one index, room for one value.
    unsafe { glGetActiveUniformsiv(p, 1, idx.as_ptr(), GL_UNIFORM_BLOCK_INDEX, &mut bi) };
    assert_eq!(bi, 0, "{case}: in block 0");
    assert_eq!(err(), GL_NO_ERROR, "{case}");
}

/// Draw `p` with the block's buffer holding vec4(1, 2, 3, 4), and return the
/// declaring stage (`vertex`: the vertex shader's) as the draw handed it to
/// the back end. As far as the host can check a draw: the GL core fills the
/// member's row from the bound buffer by the member's row index
/// (`program::fill_uniform_blocks`), which is the row the name pool resolved;
/// what the GPU then computes is the board's.
fn draw_with_member(p: GLuint, vertex: bool) -> Stage {
    static POS: [f32; 12] = [-1.0, -1.0, 0.0, 1.0, 1.0, -1.0, 0.0, 1.0, -1.0, 1.0, 0.0, 1.0];
    let data: [f32; 4] = [1.0, 2.0, 3.0, 4.0];
    // SAFETY: plain GL calls on a linked program of this context; every
    // pointer stays alive through the draw.
    unsafe {
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
        glBindBufferBase(GL_UNIFORM_BUFFER, 0, 0);
        glDeleteBuffers(1, &buf);
        glUseProgram(0);
    }
    let (vs, fs) = DRAWN.lock().unwrap_or_else(|e| e.into_inner()).take().expect("the back end drew");
    if vertex {
        vs
    } else {
        fs
    }
}

/// The draw filled the member's row of the declaring stage from the bound
/// buffer: the row the pool resolved is the one the GL core writes.
fn check_draw_fills_the_member(p: GLuint, vertex: bool, case: &str) {
    let stage = draw_with_member(p, vertex);
    assert_eq!(stage.rows.len(), 1, "{case}: the declaring stage's one uniform row is the member's: {stage:?}");
    let (off, dwords) = stage.rows[0];
    assert_eq!(dwords, 4, "{case}: a vec4: {stage:?}");
    let got: Vec<f32> =
        stage.dwords[usize::from(off)..usize::from(off) + 4].iter().map(|&d| f32::from_bits(d)).collect();
    assert_eq!(got, [1.0, 2.0, 3.0, 4.0], "{case}: the member's row holds the bound buffer's vec4");
}

/// `long_instance_name_vertex`: the block read in the vertex shader.
#[test]
fn long_instance_name_vertex_links_and_reports_the_blocks_names() {
    let _g = context();
    let p = program(&vs_long(), FS_PLAIN).unwrap_or_else(|log| panic!("long_instance_name_vertex: link failed: {log}"));
    check_long_instance_program(p, "long_instance_name_vertex");
    check_draw_fills_the_member(p, true, "long_instance_name_vertex");
    // SAFETY: a program of this context.
    unsafe { glDeleteProgram(p) };
}

/// `long_instance_name_fragment`: the block read in the fragment shader.
#[test]
fn long_instance_name_fragment_links_and_reports_the_blocks_names() {
    let _g = context();
    let p =
        program(VS_PLAIN, &fs_long()).unwrap_or_else(|log| panic!("long_instance_name_fragment: link failed: {log}"));
    check_long_instance_program(p, "long_instance_name_fragment");
    check_draw_fills_the_member(p, false, "long_instance_name_fragment");
    // SAFETY: a program of this context.
    unsafe { glDeleteProgram(p) };
}

/// Two members behind one long instance name: their rows share the 63 bytes
/// the field holds, and GL still sees two members at their own offsets.
#[test]
fn two_members_of_a_long_named_block_are_two_active_uniforms() {
    let _g = context();
    let a = "a".repeat(1020);
    let fs = format!(
        "#version 300 es\nprecision highp float;\nlayout(location = 0) out vec4 o;\n\
         uniform B {{ vec4 x; vec4 y; }} {a};\nvoid main() {{ o = {a}.x + {a}.y; }}\n"
    );
    let p = program(VS_PLAIN, &fs).unwrap_or_else(|log| panic!("link failed: {log}"));
    assert_eq!(programiv(p, GL_ACTIVE_UNIFORMS), 2);
    let (n0, n1) = (active_uniform(p, 0).0, active_uniform(p, 1).0);
    assert_eq!((n0.as_str(), n1.as_str()), ("B.x", "B.y"));
    let idx: [GLuint; 2] = [0, 1];
    let mut off: [GLint; 2] = [-1; 2];
    // SAFETY: plain GL call; two indices, room for two values.
    unsafe { glGetActiveUniformsiv(p, 2, idx.as_ptr(), GL_UNIFORM_OFFSET, off.as_mut_ptr()) };
    assert_eq!(off, [0, 16]);
    assert_eq!(err(), GL_NO_ERROR);
    // SAFETY: a program of this context.
    unsafe { glDeleteProgram(p) };
}

/// Not lifted: a default-block uniform whose name fills the field. GL reads
/// that row's field for its name, so the link refuses it as before, naming
/// the field.
#[test]
fn a_default_block_uniform_of_63_characters_is_still_refused_at_link() {
    let _g = context();
    let u = "u".repeat(63);
    let fs = format!(
        "#version 300 es\nprecision highp float;\nlayout(location = 0) out vec4 o;\n\
         uniform vec4 {u};\nvoid main() {{ o = {u}; }}\n"
    );
    let log = program(VS_PLAIN, &fs).map(|p| {
        // SAFETY: a program of this context.
        unsafe { glDeleteProgram(p) };
    });
    let log = log.expect_err("a 63-character default-block uniform does not link");
    assert!(log.contains("64-byte name field"), "the log names the field: {log}");
    assert!(log.contains("fragment uniform"), "and the stage and kind: {log}");
    // a refused link is no GL error
    assert_eq!(err(), GL_NO_ERROR);
}

/// And a name one under the field's limit links as it always did: the
/// field holds it whole and the GL name is the field's.
#[test]
fn a_default_block_uniform_of_62_characters_links_as_before() {
    let _g = context();
    let u = "u".repeat(62);
    let fs = format!(
        "#version 300 es\nprecision highp float;\nlayout(location = 0) out vec4 o;\n\
         uniform vec4 {u};\nvoid main() {{ o = {u}; }}\n"
    );
    let p = program(VS_PLAIN, &fs).unwrap_or_else(|log| panic!("link failed: {log}"));
    assert_eq!(programiv(p, GL_ACTIVE_UNIFORMS), 1);
    assert_eq!(active_uniform(p, 0).0, u);
    assert!(uniform_location(p, &u) >= 0);
    assert_eq!(err(), GL_NO_ERROR);
    // SAFETY: a program of this context.
    unsafe { glDeleteProgram(p) };
}
