// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The GC7000UL's (i.MX8MP) vertex output: what the 15-vector case needs of
//! it today, and the 64-component case ES 3.0 asks
//! (GL_MAX_VERTEX_OUTPUT_COMPONENTS, reported 60 from gl/context.rs
//! MAX_VARYING_VECTORS * 4).
//!
//! Live half: a `#version 300 es` program of fifteen vec4 varyings (three of
//! them flat) beside gl_Position links and draws through the GL on a
//! stand-in back end. There is no GC7000 to read pixels back from on the
//! host, so the pixels are the lane interpreter's (tests/common `run`) on the
//! very records the draw was handed: the vertex program is run for the three
//! vertices, each output is checked against the shader's own arithmetic at
//! the register the record names, the fragment program is run at the
//! triangle's centre with each smooth input the mean of the three vertices
//! and each flat one the last (provoking) vertex, and its colour output is
//! read back. That checks the compiler's registers and the two programs'
//! agreement, not the varying hardware (the interpreter cannot).
//!
//! Ignored half: GL_MAX_VERTEX_OUTPUT_COMPONENTS is 64 and a vertex shader
//! writing 64 output components (fifteen vec4, a vec3 and gl_PointSize, the
//! fragment shader reading one) links and draws. plan #359 gc-vout64-study:
//! 64 is reachable, position takes no varying row, and the cap is the one
//! line in gl/context.rs.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online --test es3_gc_vout64
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online --test es3_gc_vout64 -- --ignored
//! ```
#![cfg(all(feature = "host-test", feature = "shaderc-online", feature = "gpu-vivante", not(feature = "report-es2")))]
use core::ffi::{c_char, c_void};
use std::ffi::CString;
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{self, GlesDraw, GlesHeap, YakoglBackend};
use yakogl::gl::consts::*;
use yakogl::gl::draw::glDrawArrays;
use yakogl::gl::program::*;
use yakogl::gl::shader::*;
use yakogl::gl::state::{glFinish, glGetError};
use yakogl::gl::types::*;
use yakogl::gl::vertex::{glEnableVertexAttribArray, glVertexAttribPointer};
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};
use yakogl::shaderc::online::vivante::decode;
use yakogl::shaderc::online::vivante::isa::Instr;
use yakogl::shaderc::{ShcShader, ShcVivIo};

mod common;
use common::{run, Run};

/// ES 3.0's table 6.31 fragment side: fifteen vec4.
const N: usize = 15;
/// The varyings qualified `flat` in the live program.
const FLAT: [usize; 3] = [4, 9, 14];

/* ---- the program ----------------------------------------------------------------- */

/// A vertex shader writing `n` vec4 varyings `v0..v<n>` (each a different
/// swizzle of one attribute, scaled and offset, so no two are the same
/// value and all stay inside [0, 1)), and a fragment shader that reads the
/// first `reads` of them and outputs `v<pick>` (or the sum when `pick` is
/// None). `point_size` adds a gl_PointSize write and a vec3 varying `w`
/// (three more output components: 15 * 4 + 3 + 1 = 64).
fn sources(n: usize, reads: usize, pick: Option<usize>, point_size: bool) -> (String, String) {
    let mut vs = String::from("#version 300 es\nin highp vec4 a_pos;\nin highp vec4 a_c;\n");
    let mut fs = String::from("#version 300 es\nprecision highp float;\nlayout(location = 0) out highp vec4 o;\n");
    for i in 0..n {
        let q = if n == N && FLAT.contains(&i) { "flat" } else { "" };
        vs += &format!("{q} out highp vec4 v{i};\n");
        if i < reads {
            fs += &format!("{q} in highp vec4 v{i};\n");
        }
    }
    if point_size {
        vs += "out highp vec3 w;\n";
    }
    vs += "void main() {\n    gl_Position = a_pos;\n";
    if point_size {
        vs += "    gl_PointSize = 1.0;\n    w = a_c.xyz;\n";
    }
    for i in 0..n {
        let sw = ["xyzw", "yzwx", "zwxy", "wxyz"][i % 4];
        vs += &format!("    v{i} = a_c.{sw} * 0.5 + {:.6};\n", i as f32 / 40.0);
    }
    vs += "}\n";
    fs += "void main() {\n";
    match pick {
        Some(k) => fs += &format!("    o = v{k};\n"),
        None => {
            fs += "    highp vec4 s = vec4(0.0);\n";
            for i in 0..reads {
                fs += &format!("    s += v{i};\n");
            }
            fs += "    o = s * 0.0625;\n";
        }
    }
    fs += "}\n";
    (vs, fs)
}

/// The three vertices: positions (a triangle that covers the 8 x 8 target)
/// and the attribute each varying is made of.
static POS: [f32; 12] = [-1.0, -1.0, 0.0, 1.0, 3.0, -1.0, 0.0, 1.0, -1.0, 3.0, 0.0, 1.0];
static COL: [f32; 12] = [0.10, 0.20, 0.30, 0.40, 0.50, 0.60, 0.70, 0.80, 0.90, 0.15, 0.25, 0.35];

/// What the shader computes for varying `i`, component `c`, at vertex `j`.
fn vertex_value(i: usize, c: usize, j: usize) -> f32 {
    let sw = [[0, 1, 2, 3], [1, 2, 3, 0], [2, 3, 0, 1], [3, 0, 1, 2]][i % 4];
    COL[4 * j + sw[c]] * 0.5 + i as f32 / 40.0
}

/* ---- the GL, on a stand-in back end ---------------------------------------------------- */

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

/// What the back end was handed for one application draw, and the pixel the
/// lane interpreter makes of its two programs at the triangle's centre.
#[derive(Clone, Debug)]
struct Seen {
    vs_outputs: usize,
    fs_inputs: usize,
    /// Per vertex, the value of each vertex output by name.
    vs_out: Vec<std::collections::BTreeMap<String, [f32; 4]>>,
    pos: Vec<[f32; 4]>,
    /// The fragment program's colour output.
    pixel: Result<[f32; 4], String>,
}

static SEEN: Mutex<Vec<Seen>> = Mutex::new(Vec::new());

fn io_name(io: &ShcVivIo) -> String {
    io.name().iter().map(|c| *c as u8 as char).collect()
}

fn consts_of(s: &ShcShader) -> Vec<[u32; 4]> {
    let mut consts = vec![[0u32; 4]; 512];
    for (k, (&kind, &data)) in s.viv.const_contents().iter().zip(s.viv.const_data()).enumerate() {
        assert!(
            matches!(kind, yakogl::shaderc::SHC_VIV_CONST_LITERAL | yakogl::shaderc::SHC_VIV_CONST_UNUSED),
            "a constant of kind {kind} in a shader with no uniform"
        );
        consts[k / 4][k % 4] = data;
    }
    consts
}

fn code_of(s: &ShcShader) -> Vec<Instr> {
    // SAFETY: the record's code, `usc_dw` dwords, alive while the program is.
    let words = unsafe { core::slice::from_raw_parts(s.usc, s.usc_dw as usize) };
    words.chunks(4).map(|c| decode::decode_instr(&c.try_into().unwrap()).expect("decodes")).collect()
}

fn run_regs(s: &ShcShader, regs: Vec<[u32; 4]>) -> Result<Vec<[u32; 4]>, String> {
    match run(&code_of(s), &regs, &consts_of(s)) {
        Run::Done { regs, .. } => Ok(regs),
        Run::Unsupported(why) => Err(format!("the interpreter cannot run it: {why}")),
    }
}

fn f4(r: [u32; 4]) -> [f32; 4] {
    r.map(f32::from_bits)
}

/// The two programs on the lane interpreter: the vertex program for the
/// three vertices, then the fragment program at the centre, each smooth
/// input the mean of the three vertices and each flat one the last
/// (provoking) vertex's.
fn model(vs: &ShcShader, fs: &ShcShader) -> Seen {
    let vin = &vs.viv.in_[..vs.viv.num_in as usize];
    let vout = &vs.viv.out[..vs.viv.num_out as usize];
    let reg_of = |n: &str| vin.iter().find(|x| io_name(x) == n).unwrap_or_else(|| panic!("no attribute {n}")).reg;
    let (pos_r, col_r) = (reg_of("a_pos"), reg_of("a_c"));
    let mut outs = Vec::new();
    let mut poss = Vec::new();
    for j in 0..3 {
        let mut regs = vec![[0u32; 4]; 128];
        regs[pos_r as usize] = core::array::from_fn(|c| POS[4 * j + c].to_bits());
        regs[col_r as usize] = core::array::from_fn(|c| COL[4 * j + c].to_bits());
        let r = run_regs(vs, regs).unwrap_or_default();
        let mut m = std::collections::BTreeMap::new();
        if !r.is_empty() {
            for o in vout {
                m.insert(io_name(o), f4(r[o.reg as usize]));
            }
            poss.push(f4(r[vs.viv.vs_pos_out_reg as usize]));
        }
        outs.push(m);
    }
    let fin = &fs.viv.in_[..fs.viv.num_in as usize];
    let pixel = if outs.iter().any(|m| m.is_empty()) {
        Err("the vertex program did not run".to_string())
    } else {
        let mut regs = vec![[0u32; 4]; 128];
        for i in fin {
            let n = io_name(i);
            let at = |j: usize, c: usize| outs[j].get(&n).unwrap_or_else(|| panic!("vertex stage lacks {n}"))[c];
            regs[i.reg as usize] = core::array::from_fn(|c| {
                if i.interp == 2 { at(2, c) } else { (at(0, c) + at(1, c) + at(2, c)) / 3.0 }.to_bits()
            });
        }
        run_regs(fs, regs).map(|r| f4(r[fs.viv.ps_color_out_reg as usize]))
    };
    Seen { vs_outputs: vout.len(), fs_inputs: fin.len(), vs_out: outs, pos: poss, pixel }
}

unsafe extern "C" fn b_draw(d: *const GlesDraw) -> i32 {
    let d = &*d;
    if d.index_count == 0 || d.vs.is_null() || d.fs.is_null() {
        return 0;
    }
    SEEN.lock().unwrap().push(model(&*d.vs, &*d.fs));
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
    name: c"es3-gc-vout64".as_ptr() as *const c_char,
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

/// One context for the whole file; the tests take turns on it and on the
/// compiler (one arena, not reentrant).
static LOCK: Mutex<()> = Mutex::new(());
static ONCE: Once = Once::new();

fn context() -> MutexGuard<'static, ()> {
    let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    ONCE.call_once(|| {
        // SAFETY: both tables are statics; the lock above serialises the tests.
        unsafe {
            osal_set_ops(&OSAL);
            backend::g_backend = &BACKEND;
            yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
            assert!(!yakogl::gl::context::gl_context_create(8, 8).is_null());
        }
    });
    // SAFETY: plain GL calls.
    unsafe {
        glFinish();
        while glGetError() != GL_NO_ERROR {}
    }
    SEEN.lock().unwrap().clear();
    g
}

fn geti(p: GLenum) -> GLint {
    let mut v: GLint = -1;
    // SAFETY: plain GL call with a real output.
    unsafe { yakogl::gl::state::glGetIntegerv(p, &mut v) };
    v
}

fn text_of(log: &[c_char]) -> String {
    log.iter().take_while(|c| **c != 0).map(|c| *c as u8 as char).collect()
}

fn shader(ty: GLenum, text: &str) -> GLuint {
    let text = CString::new(text).unwrap();
    // SAFETY: plain GL calls; one NUL-terminated source.
    unsafe {
        let s = glCreateShader(ty);
        let p: [*const c_char; 1] = [text.as_ptr()];
        glShaderSource(s, 1, p.as_ptr(), core::ptr::null());
        glCompileShader(s);
        let mut ok: GLint = 0;
        glGetShaderiv(s, GL_COMPILE_STATUS, &mut ok);
        if ok != 1 {
            let mut log = [0 as c_char; 1024];
            glGetShaderInfoLog(s, log.len() as GLsizei, core::ptr::null_mut(), log.as_mut_ptr());
            panic!("did not compile: {}\n{text:?}", text_of(&log));
        }
        s
    }
}

/// The two sources linked: Ok(program) or Err(info log).
fn program((vs, fs): &(String, String)) -> Result<GLuint, String> {
    let (v, f) = (shader(GL_VERTEX_SHADER, vs), shader(GL_FRAGMENT_SHADER, fs));
    // SAFETY: plain GL calls.
    unsafe {
        let p = glCreateProgram();
        glAttachShader(p, v);
        glAttachShader(p, f);
        glBindAttribLocation(p, 0, c"a_pos".as_ptr());
        glBindAttribLocation(p, 1, c"a_c".as_ptr());
        glLinkProgram(p);
        glDeleteShader(v);
        glDeleteShader(f);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        if ok == 1 {
            return Ok(p);
        }
        let mut log = [0 as c_char; 1024];
        glGetProgramInfoLog(p, log.len() as GLsizei, core::ptr::null_mut(), log.as_mut_ptr());
        glDeleteProgram(p);
        Err(text_of(&log))
    }
}

/// Draws the triangle with `p`; the draw's record is in SEEN afterwards.
fn draw(p: GLuint) {
    // SAFETY: the arrays are statics and outlive the draw.
    unsafe {
        glUseProgram(p);
        yakogl::gl::buffer::glBindBuffer(GL_ARRAY_BUFFER, 0);
        glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, POS.as_ptr() as *const c_void);
        glEnableVertexAttribArray(0);
        glVertexAttribPointer(1, 4, GL_FLOAT, 0, 0, COL.as_ptr() as *const c_void);
        glEnableVertexAttribArray(1);
        glDrawArrays(GL_TRIANGLES, 0, 3);
        glFinish();
    }
}

/* ---- the live half ------------------------------------------------------------------ */

/// Fifteen vec4 varyings (three flat) beside gl_Position: one program per
/// varying read back, each drawn on its own. Each vertex output and each
/// pixel is held to the arithmetic.
#[test]
fn fifteen_vec4_varyings_draw_and_read_back() {
    let _g = context();
    let refused_before = backend::draws_refused();
    for k in 0..N {
        let src = sources(N, N, Some(k), false);
        let p = program(&src).unwrap_or_else(|e| panic!("v{k}: fifteen vec4 do not link: {e}"));
        draw(p);
        // SAFETY: plain GL calls.
        unsafe {
            assert_eq!(glGetError(), GL_NO_ERROR, "the draw of v{k}");
            glUseProgram(0);
            glDeleteProgram(p);
        }
        let seen = SEEN.lock().unwrap().pop().unwrap_or_else(|| panic!("v{k}: the back end saw no draw"));
        SEEN.lock().unwrap().clear();
        assert_eq!(seen.vs_outputs, N, "v{k}: the vertex record lists the fifteen (position is no out row)");
        assert_eq!(seen.fs_inputs, 1, "v{k}: the fragment record lists the one it reads");
        for (j, out) in seen.vs_out.iter().enumerate() {
            for i in 0..N {
                let got = out.get(&format!("v{i}")).unwrap_or_else(|| panic!("v{k}: no output v{i}"));
                for c in 0..4 {
                    let want = vertex_value(i, c, j);
                    assert!(
                        (got[c] - want).abs() < 1e-6,
                        "v{k}: vertex {j} varying {i}[{c}] = {}, want {want}",
                        got[c]
                    );
                }
            }
            for c in 0..4 {
                assert_eq!(seen.pos[j][c], POS[4 * j + c], "v{k}: gl_Position[{c}] of vertex {j}");
            }
        }
        let px = seen.pixel.clone().unwrap_or_else(|e| panic!("v{k}: the fragment program: {e}"));
        for c in 0..4 {
            if FLAT.contains(&k) {
                assert_eq!(px[c], vertex_value(k, c, 2), "v{k} (flat)[{c}]: the last vertex's value exactly");
            } else {
                let want = (0..3).map(|j| vertex_value(k, c, j)).sum::<f32>() / 3.0;
                assert!((px[c] - want).abs() < 1e-5, "v{k}[{c}]: {} vs the centre's {want}", px[c]);
            }
        }
    }
    assert_eq!(backend::draws_refused(), refused_before, "no draw was refused");
}

/// All fifteen read in one fragment shader (the case a draw really has): the
/// records carry fifteen and the sum reaches the pixel.
#[test]
fn fifteen_vec4_varyings_read_by_one_fragment_shader() {
    let _g = context();
    let p = program(&sources(N, N, None, false)).unwrap_or_else(|e| panic!("fifteen vec4 do not link: {e}"));
    draw(p);
    // SAFETY: plain GL calls.
    unsafe {
        assert_eq!(glGetError(), GL_NO_ERROR);
        glUseProgram(0);
        glDeleteProgram(p);
    }
    let seen = SEEN.lock().unwrap().pop().expect("the back end saw the draw");
    assert_eq!((seen.vs_outputs, seen.fs_inputs), (N, N));
    let px = seen.pixel.expect("the fragment program runs");
    for c in 0..4 {
        let sum: f32 = (0..N)
            .map(|i| {
                if FLAT.contains(&i) {
                    vertex_value(i, c, 2)
                } else {
                    (0..3).map(|j| vertex_value(i, c, j)).sum::<f32>() / 3.0
                }
            })
            .sum();
        let want = sum * 0.0625;
        assert!((px[c] - want).abs() < 1e-4, "sum[{c}]: {} vs {want}", px[c]);
    }
}

/* ---- the 64-component half ---------------------------------------------------------- */

/// The raise to GL_MAX_VERTEX_OUTPUT_COMPONENTS 64: the limit reads 64 and a
/// vertex shader writing 64 output components (fifteen vec4, a vec3 and
/// gl_PointSize; the fragment shader reads one) links and draws without a
/// refusal. The study found 64 reachable on the GC7000UL (plan #359
/// gc-vout64-study): position is not a varying row, so the 60 is only the cap.
#[test]
#[ignore = "needs the GC raise to MAX_VERTEX_OUTPUT_COMPONENTS 64"]
fn sixty_four_vertex_output_components_link_and_draw() {
    let _g = context();
    assert_eq!(geti(GL_MAX_VERTEX_OUTPUT_COMPONENTS), 64, "ES 3.0 table 6.31 asks 64");
    let refused_before = backend::draws_refused();
    let p = program(&sources(N, 1, Some(0), true)).unwrap_or_else(|e| panic!("64 output components do not link: {e}"));
    draw(p);
    // SAFETY: plain GL calls.
    unsafe {
        assert_eq!(glGetError(), GL_NO_ERROR, "the draw");
        glUseProgram(0);
        glDeleteProgram(p);
    }
    assert_eq!(backend::draws_refused(), refused_before, "the draw was not refused");
    let seen = SEEN.lock().unwrap().pop().expect("the back end saw the draw");
    assert_eq!(seen.vs_outputs, N + 1, "fifteen vec4 and w (position and gl_PointSize have registers of their own)");
    assert_eq!(seen.fs_inputs, 1, "the fragment shader reads one");
}
