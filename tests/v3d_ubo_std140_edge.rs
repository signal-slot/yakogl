// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The std140 layout code and the 16384-byte block bound at the very end of a
//! full block on the V3D build. `tests/v3d_ubo_large_sim.rs` reads the first
//! and last dword of a block of plain vectors; this file packs a block to
//! exactly 16384 bytes whose tail is a `column_major mat4 cm[2]` (128 bytes),
//! a `row_major mat4 rm[3]` (192 bytes) and a `vec4 last` (16 bytes), the
//! 1003 vec4 before them being `h0..h3` (250, 250, 250, 253 entries:
//! `UniformRow::array_len` is a u8, so no member can hold 1003; a plan item
//! studies that limit). Then:
//!
//! - the sim: a fragment shader reads chosen matrix elements and `last` as
//!   `gl_FragDepth`. The program is linked and drawn through the GL API
//!   (glBufferData, glBindBufferBase, glDrawArrays) on a mock back end that
//!   keeps the fragment stage's block as the draw hands it over
//!   (`fill_uniform_blocks` in src/gl/program.rs has copied the bound buffer
//!   into it, row-major members included); the QPU interpreter then runs the
//!   same shader on a uniform stream built from that block (the harness of
//!   `v3d_ubo_buffer_range`). The buffer's dword k holds k + 0.5, and the
//!   value must be the dword std140 puts the element in (column-major:
//!   `m[c][r]` at column * 16 + row * 4 bytes; row-major: at row * 16 +
//!   column * 4);
//! - the queries: `UNIFORM_OFFSET`, `UNIFORM_MATRIX_STRIDE`,
//!   `UNIFORM_IS_ROW_MAJOR`, `UNIFORM_ARRAY_STRIDE`, the block's data size;
//! - the link: the same block plus one more vec4 (16400 bytes) must fail to
//!   link; it links today (pinned, with the `#[ignore]`d expectation).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test v3d_ubo_std140_edge
//! ```
#![cfg(all(feature = "host-test", feature = "shaderc-online", feature = "gpu-v3d", not(feature = "report-es2")))]

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
use yakogl::shaderc::online::arena::Arena;
use yakogl::shaderc::online::v3d::chain::{compile_program, Prim, Target};
use yakogl::shaderc::online::v3d::select::TLB_Z_PER_PIXEL;
use yakogl::shaderc::online::v3d::sim::{self, Env, FsInputs, Inputs, Varying, LANES};
use yakogl::shaderc::online::{lower, sema};

/* ---- a context on the host: the smallest OSAL and back end that make one -- */

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

/// The fragment stage's block of the last draw, as handed to the back end.
static DRAWN: Mutex<Option<Vec<u32>>> = Mutex::new(None);

unsafe extern "C" fn b_draw(d: *const GlesDraw) -> i32 {
    let d = &*d;
    let block = if d.fs_uniform_cpu.is_null() {
        Vec::new()
    } else {
        core::slice::from_raw_parts(d.fs_uniform_cpu as *const u32, usize::from(d.fs_uniform_dwords)).to_vec()
    };
    *DRAWN.lock().unwrap_or_else(|e| e.into_inner()) = Some(block);
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
    name: c"v3d-ubo-std140-edge".as_ptr() as *const c_char,
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
    /* as the V3D's table: the GL core hands over the host array */
    uniforms_host_only: true,
    ds_write: None,
    stencil_resolve: None,
};

/// The compiler keeps its output in one arena that every compile resets, and
/// the GL library is single-threaded by contract: one lock for both halves.
static LIBRARY: Mutex<()> = Mutex::new(());
static CONTEXT: Once = Once::new();

fn context() -> MutexGuard<'static, ()> {
    let guard = LIBRARY.lock().unwrap_or_else(|e| e.into_inner());
    CONTEXT.call_once(|| {
        // SAFETY: both tables are statics, and the guard above is held.
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
    guard
}

/* ---- the block --------------------------------------------------------- */

const VS: &str = "#version 300 es\nin highp vec4 a_position;\nin highp vec2 a_coord;\nout highp vec2 v_coord;\nvoid main (void)\n{\n    gl_Position = a_position;\n    v_coord = a_coord;\n}\n";

/// Entries of the filler members h0..h3: 1003 vec4 = 16048 bytes.
const HEAD: [usize; 4] = [250, 250, 250, 253];
/// Byte offsets of the tail (16048 + 128 + 192 + 16 = 16384).
const CM_AT: usize = 16048;
const RM_AT: usize = 16176;
const LAST_AT: usize = 16368;

/// The block declaration: `extra` more vec4 after `last` (0 for the full
/// 16384 bytes).
fn block(extra: usize) -> String {
    let head: String = HEAD.iter().enumerate().map(|(i, n)| format!("highp vec4 h{i}[{n}]; ")).collect();
    let more = if extra > 0 { format!("highp vec4 extra[{extra}]; ") } else { String::new() };
    format!(
        "layout(std140) uniform Big {{ {head}layout(column_major) highp mat4 cm[2]; layout(row_major) highp mat4 rm[3]; highp vec4 last; {more}}};\n"
    )
}

/// A fragment shader writing `expr` as the depth.
fn fs(extra: usize, expr: &str) -> String {
    format!(
        "#version 300 es\n{}in highp vec2 v_coord;\nlayout(location = 0) out mediump vec4 o_color;\nvoid main (void)\n{{\n    o_color = vec4(1.0);\n    gl_FragDepth = {expr};\n}}\n",
        block(extra)
    )
}

/// The distinct value of block dword `k`.
fn value(k: usize) -> f32 {
    k as f32 + 0.5
}

/// Link `fs`, bind a 16384-byte buffer (dword k = `value(k)`) to the block
/// `Big` with glBindBufferBase and draw once: the fragment block the draw
/// handed to the back end.
fn drawn_block(fs: &str) -> Result<Vec<u32>, String> {
    let (v, f) = (shader(GL_VERTEX_SHADER, VS), shader(GL_FRAGMENT_SHADER, fs));
    if !compiled(v) || !compiled(f) {
        return Err("shader compile".into());
    }
    // SAFETY: plain GL calls on the one context; every pointer outlives its call.
    unsafe {
        let p = glCreateProgram();
        glAttachShader(p, v);
        glAttachShader(p, f);
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        if ok != 1 {
            return Err("link".into());
        }
        glUseProgram(p);
        let idx = glGetUniformBlockIndex(p, c"Big".as_ptr());
        if idx == GL_INVALID_INDEX {
            return Err("no block Big".into());
        }
        glUniformBlockBinding(p, idx, 0);
        let data: Vec<u32> = (0..4096).map(|k| value(k).to_bits()).collect();
        let mut b = 0;
        glGenBuffers(1, &mut b);
        glBindBuffer(GL_UNIFORM_BUFFER, b);
        glBufferData(GL_UNIFORM_BUFFER, 16384, data.as_ptr() as *const c_void, GL_STATIC_DRAW);
        glBindBufferBase(GL_UNIFORM_BUFFER, 0, b);
        static POS: [f32; 12] = [-1.0, -1.0, 0.0, 1.0, -1.0, 1.0, 0.0, 1.0, 1.0, -1.0, 0.0, 1.0];
        let loc = glGetAttribLocation(p, c"a_position".as_ptr());
        glBindBuffer(GL_ARRAY_BUFFER, 0);
        glVertexAttribPointer(loc as u32, 4, GL_FLOAT, 0, 0, POS.as_ptr() as *const c_void);
        glEnableVertexAttribArray(loc as u32);
        *DRAWN.lock().unwrap_or_else(|e| e.into_inner()) = None;
        glDrawArrays(GL_TRIANGLES, 0, 3);
        glDisableVertexAttribArray(loc as u32);
        let e = glGetError();
        if e != GL_NO_ERROR {
            return Err(format!("GL error {e:#x} at the draw"));
        }
        glUseProgram(0);
        glDeleteProgram(p);
        glDeleteBuffers(1, &b);
    }
    DRAWN.lock().unwrap_or_else(|e| e.into_inner()).take().ok_or_else(|| "the draw did not reach the back end".into())
}

/// The depth lane 0 writes for `fs`: drawn through GL, read by the sim.
fn run(fs: &str) -> Result<f32, String> {
    let _g = context();
    let block = drawn_block(fs)?;
    sim_depth(fs, &block)
}

/// The depth lane 0 writes for `fs` when the QPU reads `block`.
fn sim_depth(fs: &str, block: &[u32]) -> Result<f32, String> {
    let mut mem = vec![0u8; 64 << 20];
    let a = Arena::new(&mut mem);
    let vs = a.alloc_str(VS).expect("arena");
    let fs_ = a.alloc_str(fs).expect("arena");
    let v = lower::compile_to_ir(&a, sema::Stage::Vertex, vs).map_err(|e| e.msg.to_string()).expect("the VS lowers");
    let f = lower::compile_to_ir(&a, sema::Stage::Fragment, fs_).map_err(|e| format!("lower: {}", e.msg))?;
    let target = Target { vpm_size: 65536 };
    let p =
        compile_program(&v.module, &f.module, Prim::Triangles, target, &a).map_err(|e| format!("compile: {e:?}"))?;
    let mut stream = Vec::new();
    for u in p.fs.code.uniforms.iter() {
        stream.push(match u.contents {
            0 => u.data,
            1 => *block.get(u.data as usize).ok_or(format!(
                "stream dword {} past the draw's block of {}",
                u.data,
                block.len()
            ))?,
            c => return Err(format!("uniform contents {c} (UNIFA / UBO address) not modelled by this test")),
        });
    }
    let zero = [0u32; LANES];
    let one = [1.0f32.to_bits(); LANES];
    let varyings = p.fs.input_slots.iter().map(|_| Varying { vary: zero, c: zero }).collect();
    let inputs = FsInputs { w: one, w_centroid: one, z: zero, x: zero, y: zero, back_facing: [false; LANES], varyings };
    let env = Env { threads: p.fs.threads, ..Env::default() };
    let o = sim::run(p.fs.code.words, &stream, &Inputs::Fragment(Box::new(inputs)), &env)
        .map_err(|e| format!("sim: {e}"))?;
    let w = o.tlb.iter().find(|w| w.config == Some(TLB_Z_PER_PIXEL)).ok_or("no depth write")?;
    Ok(f32::from_bits(w.value[0].ok_or("undefined depth")?))
}

fn read(expr: &str) -> f32 {
    let src = fs(0, expr);
    run(&src).unwrap_or_else(|why| panic!("{expr}: {why}\n{src}"))
}

/* ---- (a) host sim reads ------------------------------------------------- */

/// The row-major elements the compiler reads where std140 puts them: the
/// diagonal ones, where the row-major and column-major dword coincide.
#[test]
fn sim_reads_row_major_diagonal_elements() {
    for (i, d) in [(2usize, 3usize), (0, 0), (1, 1), (2, 2)] {
        let at = RM_AT + i * 64 + d * 16 + d * 4;
        assert_eq!(read(&format!("rm[{i}][{d}][{d}]")), value(at / 4), "rm[{i}][{d}][{d}] at byte {at}");
    }
}

/// The std140 layout: row-major `m[c][r]` at `row * 16 + column * 4`, as the
/// GL core's copy of the bound buffer into the block puts it.
#[test]
fn sim_reads_row_major_off_diagonal_elements_as_std140_says() {
    for (i, c, r) in [(2usize, 3usize, 0usize), (2, 0, 3), (0, 1, 2), (1, 2, 1)] {
        let at = RM_AT + i * 64 + r * 16 + c * 4;
        assert_eq!(read(&format!("rm[{i}][{c}][{r}]")), value(at / 4), "rm[{i}][{c}][{r}] at byte {at}");
    }
}

#[test]
fn sim_reads_column_major_matrix_elements() {
    // column-major: m[c][r] at CM_AT + i * 64 + c * 16 + r * 4
    for (i, c, r) in [(1usize, 3usize, 3usize), (1, 0, 3), (1, 3, 0), (0, 2, 1), (0, 0, 0)] {
        let at = CM_AT + i * 64 + c * 16 + r * 4;
        assert_eq!(read(&format!("cm[{i}][{c}][{r}]")), value(at / 4), "cm[{i}][{c}][{r}] at byte {at}");
    }
}

#[test]
fn sim_reads_the_last_vec4_of_the_block() {
    for (c, comp) in "xyzw".chars().enumerate() {
        assert_eq!(read(&format!("last.{comp}")), value(LAST_AT / 4 + c), "last.{comp}");
    }
    assert_eq!(value(LAST_AT / 4 + 3), 4095.5, "the last dword of 16384 bytes");
    let full = run(&fs(0, "last.w")).expect("compiles");
    assert_eq!(full, 4095.5);
}

/* ---- (b), (c) GL queries and link ---------------------------------------- */

fn shader(ty: GLenum, text: &str) -> GLuint {
    let c = CString::new(text).unwrap();
    // SAFETY: plain GL calls; one NUL-terminated source.
    unsafe {
        let s = glCreateShader(ty);
        let p: [*const c_char; 1] = [c.as_ptr()];
        glShaderSource(s, 1, p.as_ptr(), core::ptr::null());
        glCompileShader(s);
        s
    }
}

fn compiled(s: GLuint) -> bool {
    let mut ok: GLint = 0;
    // SAFETY: plain GL call.
    unsafe { glGetShaderiv(s, GL_COMPILE_STATUS, &mut ok) };
    ok == 1
}

/// Compile and link the block (`extra` vec4 past `last`): the program and
/// its link status, or `None` where a shader did not compile.
fn link(extra: usize) -> Option<(GLuint, bool)> {
    let v = shader(GL_VERTEX_SHADER, VS);
    let f = shader(GL_FRAGMENT_SHADER, &fs(extra, "last.x"));
    if !compiled(v) || !compiled(f) {
        return None;
    }
    // SAFETY: plain GL calls on the one context.
    unsafe {
        let p = glCreateProgram();
        glAttachShader(p, v);
        glAttachShader(p, f);
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        Some((p, ok == 1))
    }
}

fn uniform_index(p: GLuint, name: &str) -> GLuint {
    let c = CString::new(name).unwrap();
    let names: [*const c_char; 1] = [c.as_ptr()];
    let mut idx: GLuint = GL_INVALID_INDEX;
    // SAFETY: one name, one index.
    unsafe { glGetUniformIndices(p, 1, names.as_ptr(), &mut idx) };
    assert_ne!(idx, GL_INVALID_INDEX, "{name} is an active uniform");
    idx
}

fn member(p: GLuint, name: &str, pname: GLenum) -> GLint {
    let idx = uniform_index(p, name);
    let mut v: GLint = -1;
    // SAFETY: one index, one value.
    unsafe { glGetActiveUniformsiv(p, 1, &idx, pname, &mut v) };
    v
}

#[test]
fn std140_queries_of_the_tail_members() {
    let _g = context();
    let (p, ok) = link(0).expect("the shaders of the 16384-byte block compile");
    assert!(ok, "the 16384-byte block links");
    let mut size: GLint = 0;
    // SAFETY: one block, one value.
    unsafe { glGetActiveUniformBlockiv(p, 0, GL_UNIFORM_BLOCK_DATA_SIZE, &mut size) };
    assert_eq!(size, 16384, "the block's data size");
    // (member, offset, matrix stride, row major, array stride, array size)
    for (name, off, ms, rm, ast, n) in [
        ("cm[0]", CM_AT, 16, 0, 64, 2),
        ("rm[0]", RM_AT, 16, 1, 64, 3),
        ("last", LAST_AT, 0, 0, 0, 1),
        ("h3[0]", 12000, 0, 0, 16, 253),
    ] {
        assert_eq!(member(p, name, GL_UNIFORM_OFFSET), off as GLint, "{name} offset");
        assert_eq!(member(p, name, GL_UNIFORM_MATRIX_STRIDE), ms, "{name} matrix stride");
        assert_eq!(member(p, name, GL_UNIFORM_IS_ROW_MAJOR), rm, "{name} is_row_major");
        assert_eq!(member(p, name, GL_UNIFORM_ARRAY_STRIDE), ast, "{name} array stride");
        assert_eq!(member(p, name, GL_UNIFORM_SIZE), n, "{name} array size");
    }
}

/// Defect, pinned: the 16400-byte block compiles and links today.
#[test]
fn a_block_one_vec4_past_16384_bytes_links_today() {
    let _g = context();
    let (_, linked) = link(1).expect("the shaders compile");
    assert!(linked);
}

/// The ES 3.0 expectation (`MAX_UNIFORM_BLOCK_SIZE` is 16384): link failure.
#[test]
#[ignore = "defect: a 16400-byte std140 block links on the V3D build (see a_block_one_vec4_past_16384_bytes_links_today)"]
fn a_block_one_vec4_past_16384_bytes_does_not_link() {
    let _g = context();
    // refused at compile or at link, never accepted
    match link(1) {
        None => {}
        Some((_, linked)) => assert!(!linked, "a 16400-byte block must not link"),
    }
}
