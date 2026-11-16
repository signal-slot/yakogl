// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Every std140 matrix shape, `column_major` and `row_major`, in a uniform
//! block that is exactly 16384 bytes on the V3D build. The block holds
//! `mat2 mat3 mat4 mat2x3 mat2x4 mat3x2 mat3x4 mat4x2 mat4x3`, each once as
//! `column_major` (`cm22`, `cm33`, ...) and once as `row_major` (`rm22`, ...),
//! 864 bytes in all, behind filler `vec4 h0..h3` (970 vec4, 15520 bytes;
//! `UniformRow::array_len` is a u8) so that the last matrix ends at byte 16384.
//! `matCxR` has C columns and R rows. std140 strides the major axis to 16
//! bytes: a column-major matrix is C columns of 16 bytes, a row-major one R
//! rows of 16 bytes (a mat3 is 3 columns strided 4 dwords, see
//! `tests/v3d_ubo_struct_members.rs`). Then:
//!
//! - the queries: `UNIFORM_OFFSET`, `UNIFORM_MATRIX_STRIDE` (16) and
//!   `UNIFORM_IS_ROW_MAJOR` of each member by `glGetActiveUniformsiv`;
//! - the sim: a fragment shader reads `m[0][0]` and `m[C-1][R-1]` of each
//!   matrix as `gl_FragDepth`. The block's data goes through the GL path:
//!   `glBufferData` of a buffer whose dword k holds k + 0.5, `glBindBufferBase`,
//!   a draw on a mock back end that keeps the fragment block the GL core
//!   hands over (`fill_uniform_blocks` / `copy_block_member` in
//!   src/gl/program.rs, the std140 copy into the default block), and the QPU
//!   interpreter running the compiled shader on a uniform stream built from
//!   that block. (A harness that maps buffer dwords to default-block dwords
//!   at the compiler level skips that copy and reads the matrices wrong.)
//!   Column-major: element `m[c][r]` at byte `c * 16 + r * 4` of the matrix;
//!   row-major: at `r * 16 + c * 4`.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test v3d_ubo_matrix_layouts
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
    name: c"v3d-ubo-matrix-layouts".as_ptr() as *const c_char,
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
    guard
}

/* ---- the block --------------------------------------------------------- */

const VS: &str = "#version 300 es\nin highp vec4 a_position;\nvoid main (void)\n{\n    gl_Position = a_position;\n}\n";

/// Entries of the filler members h0..h3: 970 vec4 = 15520 bytes.
const HEAD: [usize; 4] = [250, 250, 250, 220];
const HEAD_BYTES: usize = 15520;

/// (type, name suffix, columns, rows) of the nine shapes, in declaration order.
const SHAPES: [(&str, &str, usize, usize); 9] = [
    ("mat2", "22", 2, 2),
    ("mat3", "33", 3, 3),
    ("mat4", "44", 4, 4),
    ("mat2x3", "23", 2, 3),
    ("mat2x4", "24", 2, 4),
    ("mat3x2", "32", 3, 2),
    ("mat3x4", "34", 3, 4),
    ("mat4x2", "42", 4, 2),
    ("mat4x3", "43", 4, 3),
];

/// One member: its name, shape and layout, and its byte offset in the block.
struct Member {
    name: String,
    ty: &'static str,
    cols: usize,
    rows: usize,
    row_major: bool,
    at: usize,
}

/// The matrices in declaration order (all column-major, then all row-major)
/// with their std140 offsets, and the end of the last one.
fn members() -> (Vec<Member>, usize) {
    let mut at = HEAD_BYTES;
    let mut v = Vec::new();
    for row_major in [false, true] {
        for &(ty, suffix, cols, rows) in &SHAPES {
            let name = format!("{}{suffix}", if row_major { "rm" } else { "cm" });
            v.push(Member { name, ty, cols, rows, row_major, at });
            at += if row_major { rows } else { cols } * 16;
        }
    }
    (v, at)
}

fn block() -> String {
    let head: String = HEAD.iter().enumerate().map(|(i, n)| format!("highp vec4 h{i}[{n}]; ")).collect();
    let (ms, _) = members();
    let mats: String = ms
        .iter()
        .map(|m| {
            let layout = if m.row_major { "row_major" } else { "column_major" };
            format!("layout({layout}) highp {} {}; ", m.ty, m.name)
        })
        .collect();
    format!("layout(std140) uniform Big {{ {head}{mats}}};\n")
}

/// A fragment shader writing `expr` as the depth.
fn fs(expr: &str) -> String {
    format!(
        "#version 300 es\n{}layout(location = 0) out mediump vec4 o_color;\nvoid main (void)\n{{\n    o_color = vec4(1.0);\n    gl_FragDepth = {expr};\n}}\n",
        block()
    )
}

/// The distinct value of block dword `k`.
fn value(k: usize) -> f32 {
    k as f32 + 0.5
}

/// The depth lane 0 writes for `fs` when the QPU reads `block`, the fragment
/// block of a draw.
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

/// The depth lane 0 writes for `expr` of the block: a GL draw of a program
/// whose block `Big` is bound to a buffer of `value(k)` dwords, then the sim
/// on the fragment block the draw handed over.
fn read(expr: &str) -> f32 {
    let src = fs(expr);
    let _g = context();
    // SAFETY: plain GL calls on handles this function owns.
    let block = unsafe {
        let (p, ok) = link_src(&src).unwrap_or_else(|| panic!("{expr}: the shaders compile\n{src}"));
        assert!(ok, "{expr}: link\n{src}");
        glUseProgram(p);
        let data: Vec<u32> = (0..16384 / 4).map(|k| value(k).to_bits()).collect();
        let mut buf = 0;
        glGenBuffers(1, &mut buf);
        glBindBuffer(GL_UNIFORM_BUFFER, buf);
        glBufferData(GL_UNIFORM_BUFFER, 16384, data.as_ptr() as *const c_void, GL_STATIC_DRAW);
        glBindBufferBase(GL_UNIFORM_BUFFER, 0, buf);
        assert_eq!(glGetError(), GL_NO_ERROR, "{expr}: the buffer");
        let block = draw(p);
        glUseProgram(0);
        glDeleteProgram(p);
        glDeleteBuffers(1, &buf);
        block
    };
    sim_depth(&src, &block).unwrap_or_else(|why| panic!("{expr}: {why}\n{src}"))
}

/// One draw of the current program; the fragment block it handed over.
///
/// # Safety
/// Plain GL calls on the current context.
unsafe fn draw(p: GLuint) -> Vec<u32> {
    static POS: [f32; 12] = [-1.0, -1.0, 0.0, 1.0, -1.0, 1.0, 0.0, 1.0, 1.0, -1.0, 0.0, 1.0];
    let loc = glGetAttribLocation(p, c"a_position".as_ptr());
    glBindBuffer(GL_ARRAY_BUFFER, 0);
    glVertexAttribPointer(loc as u32, 4, GL_FLOAT, 0, 0, POS.as_ptr() as *const c_void);
    glEnableVertexAttribArray(loc as u32);
    *DRAWN.lock().unwrap() = None;
    glDrawArrays(GL_TRIANGLES, 0, 3);
    glDisableVertexAttribArray(loc as u32);
    assert_eq!(glGetError(), GL_NO_ERROR, "the draw");
    DRAWN.lock().unwrap().take().expect("the draw reached the back end")
}

/* ---- GL ------------------------------------------------------------------ */

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

/// Compile and link the block: the program and its link status, or `None`
/// where a shader did not compile.
fn link() -> Option<(GLuint, bool)> {
    link_src(&fs("cm22[0][0]"))
}

/// The same for the fragment source `src`.
fn link_src(src: &str) -> Option<(GLuint, bool)> {
    let v = shader(GL_VERTEX_SHADER, VS);
    let f = shader(GL_FRAGMENT_SHADER, src);
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

/* ---- tests ---------------------------------------------------------------- */

#[test]
fn the_block_is_16384_bytes_and_the_queries_say_std140() {
    let (ms, end) = members();
    assert_eq!(end, 16384, "the last matrix ends at byte 16384");
    let _g = context();
    let (p, ok) = link().expect("the shaders compile");
    assert!(ok, "the 16384-byte block links");
    let mut size: GLint = 0;
    // SAFETY: one block, one value.
    unsafe { glGetActiveUniformBlockiv(p, 0, GL_UNIFORM_BLOCK_DATA_SIZE, &mut size) };
    assert_eq!(size, 16384, "the block's data size");
    for m in &ms {
        let what = format!("{} ({} {})", m.name, if m.row_major { "row_major" } else { "column_major" }, m.ty);
        assert_eq!(member(p, &m.name, GL_UNIFORM_OFFSET), m.at as GLint, "{what} offset");
        assert_eq!(member(p, &m.name, GL_UNIFORM_MATRIX_STRIDE), 16, "{what} matrix stride");
        assert_eq!(member(p, &m.name, GL_UNIFORM_IS_ROW_MAJOR), GLint::from(m.row_major), "{what} is_row_major");
        assert_eq!(member(p, &m.name, GL_UNIFORM_SIZE), 1, "{what} array size");
    }
}

/// The two elements read per matrix: `m[0][0]` and `m[C-1][R-1]`.
fn probes(m: &Member) -> [(usize, usize); 2] {
    [(0, 0), (m.cols - 1, m.rows - 1)]
}

/// The block dword std140 puts `m[c][r]` in: column-major at `c * 16 + r * 4`
/// bytes of the matrix, row-major at `r * 16 + c * 4`.
fn std140_dword(m: &Member, c: usize, r: usize) -> usize {
    m.at / 4 + if m.row_major { r * 4 + c } else { c * 4 + r }
}

/// The std140 expectation, first and last element of each matrix.
#[test]
fn sim_reads_each_matrix_as_std140_says() {
    let (ms, _) = members();
    for m in &ms {
        for (c, r) in probes(m) {
            let k = std140_dword(m, c, r);
            assert_eq!(read(&format!("{}[{c}][{r}]", m.name)), value(k), "{}[{c}][{r}] at dword {k}", m.name);
        }
    }
}
