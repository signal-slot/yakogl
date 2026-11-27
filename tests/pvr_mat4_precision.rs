// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
// SPDX-FileCopyrightText: 2014 The Android Open Source Project
//! dEQP-GLES3 `shaders.builtin_functions.precision.{matrixcompmult,
//! outerproduct,transpose}.{lowp,mediump,highp}_{vertex,fragment}.{mat4,
//! mat4x2,mat4x3}` (54 cases) on the PowerVR online back end, through the GL
//! API on the host.
//!
//! dEQP's shader executor (`FragmentOutExecutor` in VK-GL-CTS 93bca01
//! `modules/glshared/glsShaderExecUtil.cpp`, Apache-2.0, Copyright 2014 The
//! Android Open Source Project) runs both the vertex and the fragment case
//! through a fragment shader that writes each column of a matrix result to
//! its own integer output, `layout(location = c) out highp uvecN o_out0_c =
//! floatBitsToUint(out0[c])`. A result with four columns (`mat4`, `mat4x2`,
//! `mat4x3`; dEQP names the case by the return type) writes locations 0-3.
//! The cause of the 54 refusals was not the PowerVR compiler: the link
//! (src/gl/program.rs, the output-location check) refused them with
//!
//! ```text
//! link error: fragment output at location 3 ends at 4, beyond the 3 draw buffers
//! ```
//!
//! while `backend::DRAW_TARGETS` was 3 on the PowerVR (RC14). With four draw
//! targets every one of the 54 programs compiles and links, and the chain
//! builds each fragment shader; the three-column and two-column shapes of the
//! same functions linked all along. The tests set the count through the host
//! stand-in `backend::HOST_DRAW_TARGETS`, so they hold whatever the build's
//! own count is.
//!
//! The render check runs the fragment case's built USC bytes through the host
//! byte model (tests/usc_exec `run_binary`) under the R32UI variant (each
//! column's first component in pixel outputs 0-3) and compares the bits with
//! the CPU's. The four-channel RGBA32UI variant dEQP's executor draws into is
//! refused by name ("integer colour outputs past the pixel's four output
//! registers": the tile-buffer spill, a later item), and so is every
//! three-column shape under it, so that variant is not checked here.
//!
//! HOST-ONLY: compile, link and the byte model; nothing is drawn on a GPU.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test pvr_mat4_precision
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-powervr", not(feature = "report-es2")))]
use core::ffi::{c_char, c_void, CStr};
use std::ffi::CString;
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{GlesHeap, YakoglBackend};
use yakogl::gl::consts::*;
use yakogl::gl::program::*;
use yakogl::gl::shader::*;
use yakogl::gl::state::*;
use yakogl::gl::types::*;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};
use yakogl::shaderc::online::powervr::chain::compile_fragment_for;
use yakogl::shaderc::online::sema::Stage;
use yakogl::shaderc::online::{lower, store};
use yakogl::shaderc::{shc_blend_key_int_rt, ShcIntRt};

mod usc_exec;

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
    name: c"pvr-mat4-precision".as_ptr() as *const c_char,
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
    max_samples: 0,
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

/// glCompileShader: the shader, or its info log.
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

/// Both stages compiled and linked: the program, or the failing step's log.
fn program(vs_src: &str, fs_src: &str) -> Result<GLuint, String> {
    let vs = shader(GL_VERTEX_SHADER, vs_src).map_err(|e| format!("vertex compile: {e}"))?;
    let fs = shader(GL_FRAGMENT_SHADER, fs_src).map_err(|e| format!("fragment compile: {e}"))?;
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
        Err(format!("link: {}", CStr::from_ptr(log.as_ptr()).to_string_lossy()))
    }
}

/// The draw targets the GL core allows while `f` runs (the host stand-in;
/// 0 afterwards, the build's own count).
fn with_targets<R>(n: u8, f: impl FnOnce() -> R) -> R {
    yakogl::backend::HOST_DRAW_TARGETS.store(n, std::sync::atomic::Ordering::Relaxed);
    let r = f();
    yakogl::backend::HOST_DRAW_TARGETS.store(0, std::sync::atomic::Ordering::Relaxed);
    r
}

/// Columns and rows of a GLSL matrix or vector type (a vector: one column).
fn shape(t: &str) -> (usize, usize) {
    match t {
        "vec2" => (1, 2),
        "vec3" => (1, 3),
        "vec4" => (1, 4),
        "mat2x4" => (2, 4),
        "mat3x4" => (3, 4),
        "mat4" => (4, 4),
        "mat4x2" => (4, 2),
        "mat4x3" => (4, 3),
        _ => panic!("no shape for {t}"),
    }
}

/// One of the 9 case shapes: the function's name, the argument types, the
/// return type (the case's name) and the statement.
struct Case {
    func: &'static str,
    ins: &'static [&'static str],
    out: &'static str,
    stmt: &'static str,
}

const CASES: &[Case] = &[
    Case { func: "matrixcompmult", ins: &["mat4", "mat4"], out: "mat4", stmt: "out0 = matrixCompMult(in0, in1);" },
    Case {
        func: "matrixcompmult",
        ins: &["mat4x2", "mat4x2"],
        out: "mat4x2",
        stmt: "out0 = matrixCompMult(in0, in1);",
    },
    Case {
        func: "matrixcompmult",
        ins: &["mat4x3", "mat4x3"],
        out: "mat4x3",
        stmt: "out0 = matrixCompMult(in0, in1);",
    },
    Case { func: "outerproduct", ins: &["vec4", "vec4"], out: "mat4", stmt: "out0 = outerProduct(in0, in1);" },
    Case { func: "outerproduct", ins: &["vec2", "vec4"], out: "mat4x2", stmt: "out0 = outerProduct(in0, in1);" },
    Case { func: "outerproduct", ins: &["vec3", "vec4"], out: "mat4x3", stmt: "out0 = outerProduct(in0, in1);" },
    Case { func: "transpose", ins: &["mat4"], out: "mat4", stmt: "out0 = transpose(in0);" },
    Case { func: "transpose", ins: &["mat2x4"], out: "mat4x2", stmt: "out0 = transpose(in0);" },
    Case { func: "transpose", ins: &["mat3x4"], out: "mat4x3", stmt: "out0 = transpose(in0);" },
];

const PRECISIONS: [&str; 3] = ["lowp", "mediump", "highp"];

/// The executor's integer outputs of `out0`: their declarations and the
/// per-column writes.
fn outputs(out: &str) -> (String, String) {
    let (cols, rows) = shape(out);
    let ty = if rows == 1 { "uint".to_string() } else { format!("uvec{rows}") };
    let mut decl = String::new();
    let mut write = String::new();
    for c in 0..cols {
        decl += &format!("layout(location = {c}) out highp {ty} o_out0_{c};\n");
        write += &format!("    o_out0_{c} = floatBitsToUint(out0[{c}]);\n");
    }
    (decl, write)
}

/// The `_vertex` case: the vertex shader evaluates, a flat varying carries
/// the result, the fragment shader writes its columns.
fn vertex_case(p: &str, c: &Case) -> (String, String) {
    let mut vs = String::from("#version 300 es\nin highp vec4 a_position;\n");
    for (i, t) in c.ins.iter().enumerate() {
        vs += &format!("in {p} {t} a_in{i};\n");
    }
    vs += &format!("flat out {p} {} vtx_out_out0;\nvoid main (void)\n{{\n", c.out);
    vs += "    gl_Position = a_position;\n    gl_PointSize = 1.0;\n";
    for (i, t) in c.ins.iter().enumerate() {
        vs += &format!("    {p} {t} in{i} = a_in{i};\n");
    }
    vs += &format!("    {p} {} out0;\n    {}\n    vtx_out_out0 = out0;\n}}\n", c.out, c.stmt);
    let (decl, write) = outputs(c.out);
    let fs = format!(
        "#version 300 es\nflat in {p} {o} vtx_out_out0;\n{decl}void main (void)\n{{\n    {p} {o} out0 = vtx_out_out0;\n{write}}}\n",
        o = c.out
    );
    (vs, fs)
}

/// The `_fragment` case: flat varyings carry the arguments, the fragment
/// shader evaluates and writes the columns.
fn fragment_case(p: &str, c: &Case) -> (String, String) {
    let mut vs = String::from("#version 300 es\nin highp vec4 a_position;\n");
    for (i, t) in c.ins.iter().enumerate() {
        vs += &format!("in {p} {t} a_in{i};\nflat out {p} {t} vtx_out_in{i};\n");
    }
    vs += "void main (void)\n{\n    gl_Position = a_position;\n    gl_PointSize = 1.0;\n";
    for i in 0..c.ins.len() {
        vs += &format!("    vtx_out_in{i} = a_in{i};\n");
    }
    vs += "}\n";
    let (decl, write) = outputs(c.out);
    let mut fs = String::from("#version 300 es\n");
    for (i, t) in c.ins.iter().enumerate() {
        fs += &format!("flat in {p} {t} vtx_out_in{i};\n");
    }
    fs += &decl;
    fs += "void main (void)\n{\n";
    for (i, t) in c.ins.iter().enumerate() {
        fs += &format!("    {p} {t} in{i} = vtx_out_in{i};\n");
    }
    fs += &format!("    {p} {} out0;\n    {}\n{write}}}\n", c.out, c.stmt);
    (vs, fs)
}

/// Every one of the 54 programs, by its dEQP name, with what linking it
/// gave.
fn link_all() -> Vec<(String, Result<(), String>)> {
    let mut got = Vec::new();
    for p in PRECISIONS {
        for c in CASES {
            for (stage, (vs, fs)) in [("vertex", vertex_case(p, c)), ("fragment", fragment_case(p, c))] {
                let name = format!("precision.{}.{p}_{stage}.{}", c.func, c.out);
                // SAFETY: plain GL call on the program this test made.
                let r = program(&vs, &fs).map(|prog| unsafe { glDeleteProgram(prog) });
                got.push((name, r));
            }
        }
    }
    got
}

/// With four draw targets every one of the 54 programs compiles and links,
/// and nothing is left in the GL error state.
#[test]
fn the_54_cases_compile_and_link_at_four_targets() {
    let _g = context();
    let got = with_targets(4, link_all);
    assert_eq!(got.len(), 54);
    let bad: Vec<_> = got.iter().filter(|(_, r)| r.is_err()).collect();
    assert!(bad.is_empty(), "{} of 54 refused: {bad:#?}", bad.len());
    // SAFETY: plain GL call.
    assert_eq!(unsafe { glGetError() }, GL_NO_ERROR);
}

/// The cause, pinned: with three draw targets (what the PowerVR reported
/// until DRAW_TARGETS became 4) the link refuses each of the 54 for its
/// fourth output, and for nothing else.
#[test]
fn three_targets_refuse_the_fourth_column_by_name() {
    let _g = context();
    let got = with_targets(3, link_all);
    for (name, r) in &got {
        let e = r.as_ref().expect_err(name);
        assert!(e.contains("fragment output at location 3 ends at 4, beyond the 3 draw buffers"), "{name}: {e}");
    }
}

/// A fragment shader through the PowerVR chain under the variant `key`:
/// the pixel output dwords and the USC bytes, or the refusal.
fn build(src: &str, key: u32) -> Result<(u8, Vec<u8>), String> {
    store::with_scratch(|a| {
        let _keep = store::begin_keep().expect("keep");
        let text = a.alloc_str(src).unwrap();
        let l = lower::compile_to_ir(a, Stage::Fragment, text).map_err(|e| format!("front end: {}", e.msg))?;
        let rec = compile_fragment_for(&l.module, &l.reflection, a, key).map_err(|e| format!("chain: {e:?}"))?;
        // SAFETY: the record's code lives in `keep`, `usc_dw` dwords.
        let code = unsafe { core::slice::from_raw_parts(rec.usc, rec.usc_dw as usize) };
        Ok((rec.num_outputs, code.iter().flat_map(|w| w.to_le_bytes()).collect()))
    })
    .unwrap()
}

/// The R32UI variant: one 32-bit unsigned channel a target.
fn r32ui() -> u32 {
    shc_blend_key_int_rt(ShcIntRt { channels: 1, bits: 32, signed: false })
}

/// Every fragment shader of the 54 builds a program under the link's
/// variant (key 0) and under R32UI, with one pixel output a column.
#[test]
fn every_fragment_shader_builds_in_the_chain() {
    let _g = context();
    for p in PRECISIONS {
        for c in CASES {
            for (stage, (_, fs)) in [("vertex", vertex_case(p, c)), ("fragment", fragment_case(p, c))] {
                let name = format!("{}.{p}_{stage}.{}", c.func, c.out);
                let (_, img) = build(&fs, 0).unwrap_or_else(|e| panic!("{name}, key 0: {e}"));
                assert!(!img.is_empty(), "{name}: an empty program");
                let (n, img) = build(&fs, r32ui()).unwrap_or_else(|e| panic!("{name}, R32UI: {e}"));
                assert_eq!(n, 4, "{name}: four pixel output dwords");
                assert!(!img.is_empty(), "{name}: an empty program");
            }
        }
    }
}

/// The CPU's `out0` for one case, column-major, from the arguments
/// (column-major each).
fn reference(c: &Case, args: &[Vec<f32>]) -> Vec<f32> {
    let (cols, rows) = shape(c.out);
    let mut out = vec![0f32; cols * rows];
    for col in 0..cols {
        for row in 0..rows {
            out[col * rows + row] = match c.func {
                "matrixcompmult" => args[0][col * rows + row] * args[1][col * rows + row],
                /* outerProduct(c, r): column `col` is c * r[col] */
                "outerproduct" => args[0][row] * args[1][col],
                /* transpose: out[col][row] = in[row][col]; the argument has
                 * `rows` columns of `cols` rows */
                _ => args[0][row * cols + col],
            };
        }
    }
    out
}

/// The fragment cases at highp: under R32UI each pixel output holds the
/// bits of its column's first component, which the byte model must give as
/// the CPU does. The arguments are small integers and halves, so every
/// product is exact.
#[test]
fn highp_fragment_cases_match_the_cpu_in_the_byte_model() {
    let _g = context();
    for c in CASES {
        let (_, fs) = fragment_case("highp", c);
        let (n, img) = build(&fs, r32ui()).unwrap_or_else(|e| panic!("{}.{}: {e}", c.func, c.out));
        assert_eq!(n, 4);
        let args: Vec<Vec<f32>> = c
            .ins
            .iter()
            .enumerate()
            .map(|(i, t)| {
                let (cols, rows) = shape(t);
                (0..cols * rows).map(|k| (k as f32 + 1.0) * if i == 0 { 0.5 } else { -1.0 } + i as f32).collect()
            })
            .collect();
        /* The flat varyings follow one another from coefficient 4 (after the
         * W plane), one 4-wide plane a component, column-major. */
        let mut inp = usc_exec::sentinel_inputs(1.0e30);
        let mut at = 4;
        for a in &args {
            for v in a {
                inp.coeff[at..at + 4].fill(v.to_bits());
                at += 4;
            }
        }
        let want = reference(c, &args);
        let (_, rows) = shape(c.out);
        let out =
            usc_exec::run_binary(&img, &inp, true).unwrap_or_else(|e| panic!("{}.{}: byte model: {e}", c.func, c.out));
        for col in 0..4u32 {
            let got = *out
                .get(&col)
                .unwrap_or_else(|| panic!("{}.{}: nothing in pixel output {col}: {out:?}", c.func, c.out));
            let w = want[col as usize * rows];
            assert_eq!(f32::from_bits(got), w, "{}.{}: column {col}", c.func, c.out);
        }
    }
}
