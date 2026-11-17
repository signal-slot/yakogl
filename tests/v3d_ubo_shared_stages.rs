// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! One uniform block of 16384 bytes declared in both stages of a V3D program.
//! `Big` is 8 members of `highp vec4 m[128]` (`UniformRow::array_len` is a u8,
//! so a member holds at most 255 vec4), declared the same in the vertex and
//! the fragment shader.
//!
//! * GL level (the host context of `es3_limits_v3d`, the real shader
//!   compiler): the program links, `GL_ACTIVE_UNIFORM_BLOCKS` counts the block
//!   once, its data size is 16384, and it is referenced by both stages.
//! * Sim level (the QPU interpreter, as `v3d_ubo_large_sim` and
//!   `v3d_ubo_large_vs`): from one buffer whose dword k holds k + 0.5, the
//!   vertex stage and the fragment stage each read dword 0 and dword 4095.
//! * A block of one name with other member layouts in the two stages: the
//!   link answer of today is pinned (see the tests).
//!
//! No pixel is drawn: the host rig's back end has no job builder. The uniform
//! words a draw hands the job are pinned in `v3d_uniform_host`.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test v3d_ubo_shared_stages
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-v3d", not(feature = "report-es2")))]

use core::ffi::{c_char, c_void};
use std::ffi::CString;
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{GlesHeap, YakoglBackend};
use yakogl::gl::consts::*;
use yakogl::gl::program::*;
use yakogl::gl::shader::*;
use yakogl::gl::types::*;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};
use yakogl::shaderc::online::arena::Arena;
use yakogl::shaderc::online::v3d::chain::{compile_program, Prim, Target};
use yakogl::shaderc::online::v3d::select::TLB_Z_PER_PIXEL;
use yakogl::shaderc::online::v3d::sim::{self, Env, FsInputs, Inputs, Varying, VsInputs, LANES};
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
    name: c"v3d-ubo-shared-stages".as_ptr() as *const c_char,
    native_attrib_formats: 0,
    rt_rgba8888_is_bgra: false,
    mip_level_align: 0,
    compressed_blocks_in_rows: false,
    native_layered_textures: false,
    front_facing_shader_variant: false,
    etc1_decoded_at_upload: false,
    init: Some(b_init),
    shutdown: None,
    draw: None,
    flush: None,
    finish: None,
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

static LIBRARY: Mutex<()> = Mutex::new(());
static CONTEXT: Once = Once::new();

/// The context, made once; the guard keeps the tests of this file off the
/// library (single-threaded by contract) one at a time.
fn context() -> MutexGuard<'static, ()> {
    let guard = LIBRARY.lock().unwrap_or_else(|e| e.into_inner());
    CONTEXT.call_once(|| {
        // SAFETY: both tables are statics, and the guard above is held.
        unsafe {
            osal_set_ops(&OSAL);
            yakogl::backend::g_backend = &BACKEND;
            assert!(!yakogl::gl::context::gl_context_create(8, 8).is_null());
        }
    });
    guard
}

/* ---- shaders ---------------------------------------------------------------- */

/// `k` members `m0 ..` of `highp vec4 m[len]`.
fn members(k: usize, len: usize) -> String {
    (0..k).map(|m| format!("highp vec4 m{m}[{len}]; ")).collect()
}

/// The vertex shader: block `Big` of `k` x vec4[`len`]; `v` is `expr`.
fn vs(k: usize, len: usize, expr: &str) -> String {
    format!(
        "#version 300 es\nuniform Big {{ {} }};\nin highp vec4 a_position;\nout highp vec4 v;\n\
         void main (void)\n{{\n    gl_Position = a_position;\n    v = {expr};\n}}\n",
        members(k, len)
    )
}

/// The fragment shader: block `Big` of `k` x vec4[`len`]; `gl_FragDepth` is `expr`.
fn fs(k: usize, len: usize, expr: &str) -> String {
    format!(
        "#version 300 es\nuniform Big {{ {} }};\nin highp vec4 v;\n\
         layout(location = 0) out mediump vec4 o_color;\n\
         void main (void)\n{{\n    o_color = v;\n    gl_FragDepth = {expr};\n}}\n",
        members(k, len)
    )
}

/// Both ends of the block as a `vec4`: dword 0 and dword 4095.
const BOTH_V: &str = "vec4(m0[0].x, m7[127].w, 0.0, 0.0)";
const BOTH_F: &str = "m0[0].x + m7[127].w";

/* ---- GL level ----------------------------------------------------------------- */

fn log_of(buf: &[c_char]) -> String {
    // SAFETY: the buffer is zero-initialised and the driver NUL-terminates.
    unsafe { core::ffi::CStr::from_ptr(buf.as_ptr()) }.to_string_lossy().into_owned()
}

fn compile(ty: GLenum, src: &str) -> Result<GLuint, String> {
    let c = CString::new(src).unwrap();
    // SAFETY: plain GL calls on a CString that outlives them; the log has room.
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
        glGetShaderInfoLog(s, 1023, core::ptr::null_mut(), log.as_mut_ptr());
        Err(log_of(&log))
    }
}

/// Link the two sources: the program, or the compile / link log.
fn link(vs: &str, fs: &str) -> Result<GLuint, String> {
    let v = compile(GL_VERTEX_SHADER, vs).map_err(|e| format!("vs: {e}"))?;
    let f = compile(GL_FRAGMENT_SHADER, fs).map_err(|e| format!("fs: {e}"))?;
    // SAFETY: plain GL calls on objects made above; the log has room.
    unsafe {
        let p = glCreateProgram();
        glAttachShader(p, v);
        glAttachShader(p, f);
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        if ok == 1 {
            return Ok(p);
        }
        let mut log = [0 as c_char; 1024];
        glGetProgramInfoLog(p, 1023, core::ptr::null_mut(), log.as_mut_ptr());
        Err(log_of(&log))
    }
}

fn program_iv(p: GLuint, pname: GLenum) -> GLint {
    let mut x: GLint = -1;
    // SAFETY: one value out.
    unsafe { glGetProgramiv(p, pname, &mut x) };
    x
}

fn block_iv(p: GLuint, index: GLuint, pname: GLenum) -> GLint {
    let mut x: GLint = -1;
    // SAFETY: one value out.
    unsafe { glGetActiveUniformBlockiv(p, index, pname, &mut x) };
    x
}

#[test]
fn links_with_one_block_of_16384_bytes_in_both_stages() {
    let _g = context();
    let p = link(&vs(8, 128, BOTH_V), &fs(8, 128, BOTH_F)).unwrap_or_else(|e| panic!("link: {e}"));
    assert_eq!(program_iv(p, GL_ACTIVE_UNIFORM_BLOCKS), 1, "the shared block is counted once");
    assert_eq!(block_iv(p, 0, GL_UNIFORM_BLOCK_DATA_SIZE), 16384);
    assert_eq!(block_iv(p, 0, GL_UNIFORM_BLOCK_ACTIVE_UNIFORMS), 8);
}

#[test]
fn block_is_referenced_by_both_stages() {
    let _g = context();
    let p = link(&vs(8, 128, BOTH_V), &fs(8, 128, BOTH_F)).unwrap_or_else(|e| panic!("link: {e}"));
    assert_eq!(block_iv(p, 0, GL_UNIFORM_BLOCK_REFERENCED_BY_VERTEX_SHADER), 1);
    assert_eq!(block_iv(p, 0, GL_UNIFORM_BLOCK_REFERENCED_BY_FRAGMENT_SHADER), 1);
}

/// A block of one name with other member layouts in the two stages (8 x
/// vec4[128] against 16 x vec4[64], both 16384 bytes). ES 3.0 asks for a link
/// error; today the link fails first on the members' names, which the online
/// compiler treats as plain uniforms shared across the stages.
#[test]
fn different_member_layouts_same_size_link_answer() {
    let _g = context();
    let r = link(&vs(8, 128, BOTH_V), &fs(16, 64, "m0[0].x + m15[63].w"));
    let e = r.expect_err("the link must fail");
    assert!(e.contains("'m0' has another type or precision in each stage"), "{e}");
}

/// The same name at 16384 bytes in the vertex stage and 8192 in the fragment.
#[test]
fn different_block_sizes_link_answer() {
    let _g = context();
    let r = link(&vs(8, 128, BOTH_V), &fs(8, 64, "m0[0].x + m7[63].w"));
    let e = r.expect_err("the link must fail");
    assert!(e.contains("'m0' has another type or precision in each stage"), "{e}");
}

/// Other member names under the same block name and size: the block check
/// of the online compiler rejects it.
#[test]
fn different_member_names_link_answer() {
    let _g = context();
    let fs = fs(8, 128, BOTH_F).replace("m0", "n0").replace("m7", "n7");
    let r = link(&vs(8, 128, BOTH_V), &fs);
    let e = r.expect_err("the link must fail");
    assert!(e.contains("uniform block 'Big' is declared differently"), "{e}");
}

/* ---- sim level ----------------------------------------------------------------- */

/// The compiler keeps its output in one arena that every compile resets.
static COMPILING: Mutex<()> = Mutex::new(());

const TARGET: Target = Target { vpm_size: 65536 };

/// The distinct value of block dword `k`.
fn value(k: usize) -> f32 {
    k as f32 + 0.5
}

/// The words of a stage's uniform stream, from a block whose dword k is k + 0.5.
fn stream(uniforms: impl Iterator<Item = (u8, u32)>, dwords: usize) -> Result<Vec<u32>, String> {
    uniforms
        .map(|(contents, data)| match contents {
            0 | 40 | 41 => Ok(data),
            1 if (data as usize) < dwords => Ok(value(data as usize).to_bits()),
            1 => Err(format!("stream dword {data} past the block")),
            2..=5 => Ok(1.0f32.to_bits()),
            c => Err(format!("uniform contents {c} (UBO address) not modelled by this test")),
        })
        .collect()
}

/// Both stages of one program, compiled once: (the vertex stage's `v` of lane
/// 0, the fragment stage's depth of lane 0).
fn run(vs_src: &str, fs_src: &str) -> Result<([f32; 4], f32), String> {
    let _g = COMPILING.lock().unwrap_or_else(|e| e.into_inner());
    let mut mem = vec![0u8; 64 << 20];
    let a = Arena::new(&mut mem);
    let vs_ = a.alloc_str(vs_src).expect("arena");
    let fs_ = a.alloc_str(fs_src).expect("arena");
    let v = lower::compile_to_ir(&a, sema::Stage::Vertex, vs_).map_err(|e| format!("lower vs: {}", e.msg))?;
    let f = lower::compile_to_ir(&a, sema::Stage::Fragment, fs_).map_err(|e| format!("lower fs: {}", e.msg))?;
    let p =
        compile_program(&v.module, &f.module, Prim::Triangles, TARGET, &a).map_err(|e| format!("compile: {e:?}"))?;

    // vertex stage
    let vstream = stream(
        p.vs.code.uniforms.iter().map(|u| (u.contents as u8, u.data)),
        usize::from(v.reflection.uniform_dwords),
    )?;
    let mut rows: Vec<[u32; LANES]> = Vec::new();
    let n = p.attributes.iter().filter(|e| e.1.is_some()).count();
    for &size in p.vs.vattr_sizes.iter().take(n) {
        for c in 0..size as usize {
            rows.push([[0.0f32, 0.0, 0.0, 1.0][c.min(3)].to_bits(); LANES]);
        }
    }
    let env = Env { threads: p.vs.threads, ..Env::default() };
    let o = sim::run(p.vs.code.words, &vstream, &Inputs::Vertex(VsInputs { vpm: rows }), &env)
        .map_err(|e| format!("vs sim: {e}"))?;
    let out: Vec<[u32; LANES]> =
        o.vpm_out.iter().map(|r| r.ok_or("an output row is not written")).collect::<Result<_, _>>()?;
    let vv = &out[out.len() - 4..];
    let vout: [f32; 4] = core::array::from_fn(|c| f32::from_bits(vv[c][0]));

    // fragment stage
    let fstream = stream(
        p.fs.code.uniforms.iter().map(|u| (u.contents as u8, u.data)),
        usize::from(f.reflection.uniform_dwords),
    )?;
    let zero = [0u32; LANES];
    let one = [1.0f32.to_bits(); LANES];
    let varyings = p.fs.input_slots.iter().map(|_| Varying { vary: zero, c: zero }).collect();
    let inputs = FsInputs { w: one, w_centroid: one, z: zero, x: zero, y: zero, back_facing: [false; LANES], varyings };
    let env = Env { threads: p.fs.threads, ..Env::default() };
    let o = sim::run(p.fs.code.words, &fstream, &Inputs::Fragment(Box::new(inputs)), &env)
        .map_err(|e| format!("fs sim: {e}"))?;
    let w = o.tlb.iter().find(|w| w.config == Some(TLB_Z_PER_PIXEL)).ok_or("no depth write")?;
    Ok((vout, f32::from_bits(w.value[0].ok_or("undefined depth")?)))
}

#[test]
fn both_stages_read_the_first_dword() {
    let (v, d) = run(&vs(8, 128, "m0[0]"), &fs(8, 128, "m0[0].x")).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(v, [value(0), value(1), value(2), value(3)], "vertex entry 0");
    assert_eq!(d, value(0), "fragment dword 0");
}

#[test]
fn both_stages_read_the_last_dword() {
    let (v, d) = run(&vs(8, 128, "m7[127]"), &fs(8, 128, "m7[127].w")).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(v, [value(4092), value(4093), value(4094), value(4095)], "vertex entry 1023");
    assert_eq!(d, value(4095), "fragment dword 4095");
}

#[test]
fn both_stages_read_first_and_last_in_one_program() {
    let (v, d) = run(&vs(8, 128, BOTH_V), &fs(8, 128, BOTH_F)).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(v, [value(0), value(4095), 0.0, 0.0]);
    assert_eq!(d, value(0) + value(4095));
}

/// The stages read other members of the one block: the vertex stage the
/// middle of member 3, the fragment stage dword 1536 (entry 384 = m3[0]).
#[test]
fn stages_read_different_members_of_the_block() {
    let (v, d) = run(&vs(8, 128, "m3[64]"), &fs(8, 128, "m3[0].x")).unwrap_or_else(|e| panic!("{e}"));
    let e = 3 * 128 + 64;
    assert_eq!(v, core::array::from_fn(|c| value(e * 4 + c)));
    assert_eq!(d, value(384 * 4));
}
