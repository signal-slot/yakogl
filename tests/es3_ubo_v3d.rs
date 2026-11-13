// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! std140 uniform blocks of the size GL_MAX_UNIFORM_BLOCK_SIZE reports,
//! drawn through the GL API on the V3D build and read by the V3D's own
//! compiled code (plan #359, acceptance criterion c2).
//!
//! The harness is tests/es3_ubo.rs' (the GL core into a stand-in back end
//! that records the block of dwords each draw hands over), and the V3D half
//! is the one the other V3D shader tests use: the same GLSL goes through
//! `compile_program` and the QPU interpreter (`sim::run`) with the uniform
//! stream resolved from the block the GL core handed the back end. The host
//! has no V3D pixels, so "readback" is what the program writes to the TLB
//! (fragment stage) or to the VPM (vertex stage, the value its flat varying
//! then carries): the vec4 the shader picked, one lane per pixel.
//!
//! Every case is `layout(std140) uniform Data0 { highp vec4 v0[64]; highp
//! vec4 v1[64]; ... };`: N = size / 16 vec4 in arrays of CHUNK (64) --
//! std140 lays consecutive arrays of vec4 out back to back, so the block is
//! the flat vec4 array of N elements. (A single `vec4 v[N]` with N > 255 does
//! not compile: `fit_u8` of the array length in src/shaderc/online/reflect.rs
//! refuses it, "too many of this kind of declaration"; 6144 bytes are 384
//! vec4 already. The 16384-byte block is a vec4[1024] in the dEQP cases
//! (`ubo.2_level_array`, `max_block_size`), so the front end has to carry it
//! before `ubo-v3d` can be accepted: the ignored
//! `a_single_vec4_array_of_1024_elements_compiles` pins that.) A distinct
//! float in every component (`value`, below 1024), bound with glUniformBlockBinding and
//! glBindBufferRange. The element is a constant index (the last element, N - 1,
//! always among them), not a `uniform int` set per draw as the task asked:
//! the GL core keeps block members in the stage's default uniform block, which
//! is MAX_UNIFORM_DWORDS (1536 dwords = 6144 bytes) long, so a block of the
//! reported size leaves no dword for a second uniform ("uniform dword 1536 of
//! a block of 1536 dwords"). Plan #359's ubo-v3d has to move the blocks out
//! of that budget (the memory loads of docs/notes/ubo-limits.md); the
//! ignored run-time-index test below is the check that it did.
//!
//! - the live tests use the size the build reports (6144 today) and follow
//!   the limit when it moves;
//! - the `#[ignore]`d tests state ES 3.0: exactly 16384 bytes (1024 vec4,
//!   the last one included) and two blocks of one program whose sizes sum
//!   past 16384. They need the V3D to report and carry 16384 (item
//!   `ubo-v3d` of plan #359); removing the `#[ignore]` is that item's
//!   acceptance.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_ubo_v3d
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_ubo_v3d -- --ignored
//! ```

#![cfg(all(feature = "host-test", feature = "gpu-v3d", feature = "shaderc-online", not(feature = "report-es2")))]

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
use yakogl::shaderc::online::v3d::chain::{compile_program, Prim, Target, V3dProgram};
use yakogl::shaderc::online::v3d::select::Uniform;
use yakogl::shaderc::online::v3d::sim::{self, Env, FsInputs, Inputs, VsInputs, LANES};
use yakogl::shaderc::online::{lower, sema};

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

/// The two stages' blocks of dwords as the last draw handed them over.
#[derive(Clone, Default)]
struct Drawn {
    vs: Vec<u32>,
    fs: Vec<u32>,
}

static DRAWN: Mutex<Option<Drawn>> = Mutex::new(None);

unsafe fn block_of(cpu: *const c_void, dwords: u16) -> Vec<u32> {
    if cpu.is_null() {
        Vec::new()
    } else {
        core::slice::from_raw_parts(cpu as *const u32, usize::from(dwords)).to_vec()
    }
}

unsafe extern "C" fn b_draw(d: *const GlesDraw) -> i32 {
    let d = &*d;
    let vs = block_of(d.vs_uniform_cpu, d.vs_uniform_dwords);
    let fs = block_of(d.fs_uniform_cpu, d.fs_uniform_dwords);
    *DRAWN.lock().unwrap_or_else(|e| e.into_inner()) = Some(Drawn { vs, fs });
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
    name: c"es3-ubo-v3d".as_ptr() as *const c_char,
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

/* ---- the GLSL of a case --------------------------------------------------- */

const VS_PLAIN: &str = "#version 300 es\nin highp vec4 a_position;\nvoid main() { gl_Position = a_position; }\n";

/// Vec4 per array of a block. Two limits of today's compiler set it: an
/// array past 255 elements is refused by the front end, and an array read at
/// a run-time index is selected from element by element, 256 dwords at most
/// (`MAX_SELECTED`, v3d/prepare.rs, "no ldunifa yet"): 64 vec4.
const CHUNK: usize = 64;

/// `n` vec4 as arrays `<prefix>0 .. <prefix>k` of at most CHUNK.
fn chunked(prefix: &str, n: usize) -> String {
    (0..n.div_ceil(CHUNK)).map(|c| format!("highp vec4 {prefix}{c}[{}]; ", (n - c * CHUNK).min(CHUNK))).collect()
}

/// Element `i` of the chunked arrays, both indices constant.
fn const_at(prefix: &str, i: usize) -> String {
    format!("{prefix}{}[{}]", i / CHUNK, i % CHUNK)
}

/// The vertex stage reads the block and hands `pick` on in a flat varying.
fn vs_picking(decl: &str, pick: &str) -> String {
    format!(
        "#version 300 es\n{decl}in highp vec4 a_position;\nflat out highp vec4 v_val;\n\
         void main() {{ gl_Position = a_position; v_val = {pick}; }}\n"
    )
}

const FS_FLAT_IN: &str = "#version 300 es\nprecision highp float;\nflat in highp vec4 v_val;\n\
     layout(location = 0) out highp vec4 o;\nvoid main() { o = v_val; }\n";

fn fs_picking(decl: &str, pick: &str) -> String {
    format!(
        "#version 300 es\nprecision highp float;\n{decl}\
         layout(location = 0) out highp vec4 o;\nvoid main() {{ o = {pick}; }}\n"
    )
}

/// Component `k` of element `i` of block `b`: an integer below 1024, so
/// that the fragment colour (packed as f16 pairs, exact for integers up to
/// 2048) and the sum of two blocks' values are exact. The four components
/// differ from one another and from every other element's (k0 is the
/// element, k1 its mirror, k2 and k3 spread it), and the two blocks differ.
fn value(b: usize, i: usize, k: usize) -> f32 {
    (match k {
        0 => i,
        1 => 1023 - i,
        2 => (i * 7 + b * 300) % 1024,
        _ => (i * 13 + b * 500) % 1024,
    }) as f32
}

/// The block's bytes: `n` vec4.
fn block_bytes(b: usize, n: usize) -> Vec<u8> {
    (0..n * 4).flat_map(|c| value(b, c / 4, c % 4).to_le_bytes()).collect()
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
        let mut n: GLsizei = 0;
        glGetShaderInfoLog(s, 1024, &mut n, log.as_mut_ptr());
        glDeleteShader(s);
        Err(core::ffi::CStr::from_ptr(log.as_ptr()).to_string_lossy().into_owned())
    }
}

/* ---- the GL side: link, bind, upload, draw --------------------------------- */

/// Link `vs`/`fs`, bind block `Data<b>` to a range of `sizes[b]` bytes of
/// one buffer (at the buffer offset alignment), set `u_idx` if any, and draw. The
/// block each stage was handed.
fn gl_draw(vs: &str, fs: &str, sizes: &[usize], u_idx: i32) -> Result<Drawn, String> {
    let _g = context();
    let vs = shader(GL_VERTEX_SHADER, vs).map_err(|e| format!("vertex shader: {e}"))?;
    let fs = shader(GL_FRAGMENT_SHADER, fs).map_err(|e| format!("fragment shader: {e}"))?;
    // SAFETY: plain GL calls on handles this case owns; every pointer
    // outlives its call and holds what is asked.
    unsafe {
        let p = glCreateProgram();
        glAttachShader(p, vs);
        glAttachShader(p, fs);
        glLinkProgram(p);
        glDeleteShader(vs);
        glDeleteShader(fs);
        let res = gl_draw_linked(p, sizes, u_idx);
        glUseProgram(0);
        glDeleteProgram(p);
        while glGetError() != GL_NO_ERROR {}
        res
    }
}

/// # Safety
/// `p` is a program of the current context; plain GL calls.
unsafe fn gl_draw_linked(p: GLuint, sizes: &[usize], u_idx: i32) -> Result<Drawn, String> {
    let mut ok: GLint = 0;
    glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
    if ok != 1 {
        let mut log = [0 as c_char; 1024];
        glGetProgramInfoLog(p, 1024, core::ptr::null_mut(), log.as_mut_ptr());
        return Err(format!("link: {}", core::ffi::CStr::from_ptr(log.as_ptr()).to_string_lossy()));
    }
    glUseProgram(p);
    let mut align: GLint = 0;
    glGetIntegerv(GL_UNIFORM_BUFFER_OFFSET_ALIGNMENT, &mut align);
    let mut all = Vec::new();
    let mut offs = Vec::new();
    for (b, &n) in sizes.iter().enumerate() {
        if align > 0 {
            all.resize(all.len().next_multiple_of(align as usize), 0u8);
        }
        offs.push(all.len());
        all.extend_from_slice(&block_bytes(b, n / 16));
    }
    let mut buf = 0u32;
    glGenBuffers(1, &mut buf);
    glBindBuffer(GL_UNIFORM_BUFFER, buf);
    glBufferData(GL_UNIFORM_BUFFER, all.len() as GLsizeiptr, all.as_ptr() as *const c_void, GL_STATIC_DRAW);
    for (b, &n) in sizes.iter().enumerate() {
        let name = CString::new(format!("Data{b}")).unwrap();
        let k = glGetUniformBlockIndex(p, name.as_ptr());
        if k == GL_INVALID_INDEX {
            return Err(format!("no block {}", name.to_string_lossy()));
        }
        let mut got: GLint = -1;
        glGetActiveUniformBlockiv(p, k, GL_UNIFORM_BLOCK_DATA_SIZE, &mut got);
        if got as usize != n {
            return Err(format!("block {b}: GL_UNIFORM_BLOCK_DATA_SIZE {got}, wanted {n}"));
        }
        glUniformBlockBinding(p, k, b as u32);
        glBindBufferRange(GL_UNIFORM_BUFFER, b as u32, buf, offs[b] as GLintptr, n as GLsizeiptr);
    }
    let loc = glGetUniformLocation(p, c"u_idx".as_ptr());
    if loc >= 0 {
        glUniform1i(loc, u_idx);
    }
    if err() != GL_NO_ERROR {
        return Err("block binding or upload failed".into());
    }
    static POS: [f32; 16] = [-1.0, -1.0, 0.0, 1.0, -1.0, 1.0, 0.0, 1.0, 1.0, -1.0, 0.0, 1.0, 1.0, 1.0, 0.0, 1.0];
    static IDX: [u16; 6] = [0, 1, 2, 2, 1, 3];
    let a = glGetAttribLocation(p, c"a_position".as_ptr());
    glBindBuffer(GL_ARRAY_BUFFER, 0);
    glVertexAttribPointer(a as u32, 4, GL_FLOAT, 0, 0, POS.as_ptr() as *const c_void);
    glEnableVertexAttribArray(a as u32);
    *DRAWN.lock().unwrap() = None;
    glDrawElements(GL_TRIANGLES, 6, GL_UNSIGNED_SHORT, IDX.as_ptr() as *const c_void);
    glDisableVertexAttribArray(a as u32);
    glDeleteBuffers(1, &buf);
    if err() != GL_NO_ERROR {
        return Err("draw failed".into());
    }
    DRAWN.lock().unwrap().take().ok_or_else(|| "no draw reached the back end".to_string())
}

/* ---- the V3D side: compile, resolve the stream, run on the interpreter ----- */

/// The compiler keeps its output in one arena that every compile resets.
static COMPILING: Mutex<()> = Mutex::new(());
const TARGET: Target = Target { vpm_size: 65536 };

/// The uniform stream of a compiled stage over the block the GL core handed
/// the back end: contents 1 reads dword `data` of it, 0 is a constant.
fn stream(uniforms: &[Uniform], block: &[u32]) -> Result<Vec<u32>, String> {
    uniforms
        .iter()
        .map(|u| match u.contents {
            0 => Ok(u.data),
            /* the viewport scale and offset (contents 2-5), unity here */
            2..=5 => Ok(1.0f32.to_bits()),
            1 => block
                .get(u.data as usize)
                .copied()
                .ok_or_else(|| format!("uniform dword {} of a block of {} dwords", u.data, block.len())),
            c => Err(format!("uniform contents {c} (data {:#x}) not modelled by this test", u.data)),
        })
        .collect()
}

fn with_program<R>(vs: &str, fs: &str, f: impl FnOnce(&V3dProgram<'_>) -> R) -> Result<R, String> {
    let _g = COMPILING.lock().unwrap_or_else(|e| e.into_inner());
    let mut mem = vec![0u8; 64 << 20];
    let a = Arena::new(&mut mem);
    let v = a.alloc_str(vs).expect("arena");
    let fr = a.alloc_str(fs).expect("arena");
    let v = lower::compile_to_ir(&a, sema::Stage::Vertex, v).map_err(|e| format!("VS lowers: {}", e.msg))?;
    let fr = lower::compile_to_ir(&a, sema::Stage::Fragment, fr).map_err(|e| format!("FS lowers: {}", e.msg))?;
    let p = compile_program(&v.module, &fr.module, Prim::Triangles, TARGET, &a)
        .map_err(|e| format!("the program compiles: {e:?}"))?;
    Ok(f(&p))
}

/// The four components of lane 0's colour write: the program packs a highp
/// vec4 colour as two words of f16 pairs (x, y) and (z, w).
fn fs_colour(p: &V3dProgram<'_>, block: &[u32]) -> Result<[f32; 4], String> {
    let s = stream(p.fs.code.uniforms, block)?;
    let one = [1.0f32.to_bits(); LANES];
    let zero = [0u32; LANES];
    let varyings = p.fs.input_slots.iter().map(|_| sim::Varying { vary: zero, c: zero }).collect();
    let inputs = FsInputs { w: one, w_centroid: one, z: zero, x: zero, y: zero, back_facing: [false; LANES], varyings };
    let env = Env { threads: p.fs.threads, ..Env::default() };
    let o = sim::run(p.fs.code.words, &s, &Inputs::Fragment(Box::new(inputs)), &env)?;
    let w: Vec<u32> = o.tlb.iter().filter(|w| w.config.is_none()).filter_map(|w| w.value[0]).collect();
    if w.len() != 2 {
        return Err(format!("{} colour words written: {:#x?}", w.len(), o.tlb));
    }
    Ok(core::array::from_fn(|c| {
        let h = if c % 2 == 0 { w[c / 2] & 0xffff } else { w[c / 2] >> 16 };
        sim::half_to_f32(h as u16)
    }))
}

/// The vec4 the vertex program writes after its position rows (lane 0).
fn vs_vec4(p: &V3dProgram<'_>, block: &[u32]) -> Result<[f32; 4], String> {
    let s = stream(p.vs.code.uniforms, block)?;
    let rec = p.attributes.iter().find(|e| e.0 == 0).and_then(|e| e.1).ok_or("a_position is read")? as usize;
    let rows: Vec<[u32; LANES]> =
        (0..usize::from(p.vs.vattr_sizes[rec])).map(|c| [[-1.0f32, -1.0, 0.0, 1.0][c].to_bits(); LANES]).collect();
    let env = Env { threads: p.vs.threads, ..Env::default() };
    let o = sim::run(p.vs.code.words, &s, &Inputs::Vertex(VsInputs { vpm: rows }), &env)?;
    let out: Vec<[u32; LANES]> = o.vpm_out.iter().filter_map(|r| *r).collect();
    let n = out.len();
    if n < 4 {
        return Err(format!("{n} VPM rows written"));
    }
    Ok(core::array::from_fn(|c| f32::from_bits(out[n - 4 + c][0])))
}

#[derive(Clone, Copy, PartialEq)]
enum Stage {
    Vertex,
    Fragment,
}

/// Draw `vs`/`fs` (`u_idx` set when the program has one) and read the picked vec4 back in `stage`.
fn read_back(vs: &str, fs: &str, stage: Stage, sizes: &[usize], u_idx: i32) -> Result<[f32; 4], String> {
    let drawn = gl_draw(vs, fs, sizes, u_idx)?;
    with_program(vs, fs, |p| match stage {
        Stage::Fragment => fs_colour(p, &drawn.fs),
        Stage::Vertex => vs_vec4(p, &drawn.vs),
    })?
}

/// One block of `bytes` (vec4 arrays), each of `picks` read in `stage`
/// by constant index and compared with `value`.
fn check_one_block(bytes: usize, stage: Stage, picks: &[usize]) -> Result<(), String> {
    let decl = format!("layout(std140) uniform Data0 {{ {}}};\n", chunked("v", bytes / 16));
    for &i in picks {
        let pick = const_at("v", i);
        let (vs, fs) = match stage {
            Stage::Fragment => (VS_PLAIN.to_string(), fs_picking(&decl, &pick)),
            Stage::Vertex => (vs_picking(&decl, &pick), FS_FLAT_IN.to_string()),
        };
        let got = read_back(&vs, &fs, stage, &[bytes], 0)?;
        let want: [f32; 4] = core::array::from_fn(|k| value(0, i, k));
        if got != want {
            return Err(format!("element {i} of {}: read {got:?}, wanted {want:?}", bytes / 16));
        }
    }
    Ok(())
}

fn reported_block_size() -> usize {
    let _g = context();
    let mut n: GLint = 0;
    // SAFETY: plain GL call; one value.
    unsafe { glGetIntegerv(GL_MAX_UNIFORM_BLOCK_SIZE, &mut n) };
    n as usize
}

/// The elements a case reads: the first, the second, the middle, the last
/// but one and the last.
fn picks_of(n: usize) -> Vec<usize> {
    vec![0, 1, n / 2, n - 2, n - 1]
}

/* ---- live: the size the build reports ---------------------------------------- */

#[test]
fn fragment_reads_a_block_of_the_reported_size_last_vec4_included() {
    let bytes = reported_block_size();
    assert!(bytes >= 1024 && bytes % 16 == 0, "reported {bytes}");
    check_one_block(bytes, Stage::Fragment, &picks_of(bytes / 16)).unwrap_or_else(|e| panic!("{bytes} bytes: {e}"));
}

#[test]
fn vertex_passes_a_block_of_the_reported_size_through_a_flat_varying() {
    let bytes = reported_block_size();
    check_one_block(bytes, Stage::Vertex, &picks_of(bytes / 16)).unwrap_or_else(|e| panic!("{bytes} bytes: {e}"));
}

/* ---- ES 3.0: 16384 bytes ------------------------------------------------------- */

const ES3_BLOCK: usize = 16384;

#[test]
#[ignore = "needs ubo-v3d: 16384-byte blocks"]
fn the_limit_is_at_least_the_es3_minimum() {
    assert!(reported_block_size() >= ES3_BLOCK);
}

#[test]
#[ignore = "needs ubo-v3d: 16384-byte blocks"]
fn fragment_reads_a_16384_byte_block_last_vec4_included() {
    check_one_block(ES3_BLOCK, Stage::Fragment, &[0, 383, 384, 511, 1022, 1023]).unwrap();
}

#[test]
#[ignore = "needs ubo-v3d: 16384-byte blocks"]
fn vertex_passes_a_16384_byte_block_through_a_flat_varying() {
    check_one_block(ES3_BLOCK, Stage::Vertex, &[0, 383, 384, 511, 1022, 1023]).unwrap();
}

/// Two blocks, 8192 + 12288 bytes (their sum is past 16384): the first
/// element of each, the last of each and the two crossed, summed in the
/// shader (`v[i] + w[j]`, indices constant) so a wrong block shows.
fn two_blocks(stage: Stage) {
    let (a, b) = (8192usize, 12288usize);
    let (na, nb) = (a / 16, b / 16);
    let decl = format!(
        "layout(std140) uniform Data0 {{ {}}};\nlayout(std140) uniform Data1 {{ {}}};\n",
        chunked("v", na),
        chunked("w", nb)
    );
    for (i, j) in [(0, 0), (na - 1, nb - 1), (na - 1, 0), (0, nb - 1)] {
        let pick = format!("{} + {}", const_at("v", i), const_at("w", j));
        let (vs, fs) = match stage {
            Stage::Fragment => (VS_PLAIN.to_string(), fs_picking(&decl, &pick)),
            Stage::Vertex => (vs_picking(&decl, &pick), FS_FLAT_IN.to_string()),
        };
        let got = read_back(&vs, &fs, stage, &[a, b], 0).unwrap_or_else(|e| panic!("v[{i}] + w[{j}]: {e}"));
        let want: [f32; 4] = core::array::from_fn(|k| value(0, i, k) + value(1, j, k));
        assert_eq!(got, want, "v[{i}] + w[{j}] of {na} + {nb} vec4");
    }
}

#[test]
#[ignore = "needs ubo-v3d: 16384-byte blocks"]
fn two_blocks_past_16384_bytes_in_one_program_fragment() {
    two_blocks(Stage::Fragment);
}

#[test]
#[ignore = "needs ubo-v3d: 16384-byte blocks"]
fn two_blocks_past_16384_bytes_in_one_program_vertex() {
    two_blocks(Stage::Vertex);
}

/// The block the dEQP cases use: one `vec4 v[1024]`. Refused by the front
/// end today (`array_len: u8`, reflect.rs `fit_u8`), whatever the limit.
#[test]
#[ignore = "needs ubo-v3d: the front end carries arrays past 255 elements"]
fn a_single_vec4_array_of_1024_elements_compiles() {
    let decl = "layout(std140) uniform Data0 { highp vec4 v[1024]; };\nuniform int u_idx;\n";
    let fs = fs_picking(decl, "v[u_idx]");
    let got = read_back(VS_PLAIN, &fs, Stage::Fragment, &[ES3_BLOCK], 1023).unwrap();
    assert_eq!(got, core::array::from_fn(|k| value(0, 1023, k)));
}

/// A run-time index into one array of 128 vec4 (512 dwords): the selection
/// chain stops at 256 dwords; a block read through `ldunifa` is the V3D's
/// way past it (Mesa reads a UBO at a uniform index that way).
#[test]
#[ignore = "needs ubo-v3d: a run-time index past 64 vec4 (ldunifa)"]
fn a_run_time_index_selects_from_an_array_of_128_vec4() {
    let decl = "layout(std140) uniform Data0 { highp vec4 v[128]; };\nuniform int u_idx;\n";
    let fs = fs_picking(decl, "v[u_idx]");
    let got = read_back(VS_PLAIN, &fs, Stage::Fragment, &[2048], 127).unwrap();
    assert_eq!(got, core::array::from_fn(|k| value(0, 127, k)));
}
