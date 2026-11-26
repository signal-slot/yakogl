// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GL_MAX_VERTEX_OUTPUT_COMPONENTS 64 on the PowerVR (AM62P): ES 3.0's
//! minimum (table 6.33), reported since gl/context.rs's 64 arm covers
//! `gpu-powervr` beside `gpu-v3d` (plan #388, issue #313).
//!
//! The raise rests on what the compiler and the back end already carry: a
//! `ShcShader` holds 16 varying rows, the vertex output record is sized by
//! the varyings the vertex shader writes, and the coefficient program
//! iterates what the fragment shader reads at the offsets the link patched
//! in (src/pvr/backend.rs, the `PvrPdsCoeffVarying` loop). So a vertex
//! shader of sixteen vec4 outputs beside gl_Position, of which the fragment
//! shader reads fifteen (GL_MAX_FRAGMENT_INPUT_COMPONENTS stays 60), links
//! and draws.
//!
//! What this file checks, through the GL on a stand-in back end that keeps
//! a copy of every record a draw hands it:
//!
//! - glGetIntegerv(GL_MAX_VERTEX_OUTPUT_COMPONENTS) is 64, and the fragment
//!   input limit stays 60;
//! - a vertex shader reads gl_MaxVertexOutputVectors as 16 (and
//!   gl_MaxFragmentInputVectors as 15): its program, run on the host USC
//!   byte model (tests/usc_exec), writes those values;
//! - the sixteen-output program links and draws with no error and no refused
//!   draw, and the pixels are right: the vertex program the draw was handed
//!   runs on the byte model for the three vertices (each output checked
//!   against the shader's own arithmetic, the sixteenth included), each
//!   fragment input is iterated from the vertex output dwords its record
//!   names, as the back end's coefficient program does, at the triangle's
//!   centre (the mean of the three vertices), and the fragment program's
//!   RGBA8 pixel output is compared with the shader's value. Fifteen
//!   fragment shaders, each reading all fifteen inputs and writing mostly
//!   one of them, so an input taken from the wrong output shows in a pixel.
//!
//! HOST-ONLY: the rasteriser and the iterators are the board's; this pins
//! the compiler's registers, the link's offsets and the GL's limits.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test es3_pvr_vout64
//! ```
#![cfg(all(feature = "host-test", feature = "gpu-powervr", feature = "shaderc-online", not(feature = "report-es2")))]

mod usc_exec;

use core::ffi::{c_char, c_void};
use std::ffi::CString;
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{self, GlesDraw, GlesHeap, YakoglBackend};
use yakogl::gl::consts::*;
use yakogl::gl::context::MAX_VERTEX_OUTPUT_COMPONENTS;
use yakogl::gl::draw::glDrawArrays;
use yakogl::gl::program::*;
use yakogl::gl::shader::*;
use yakogl::gl::state::{glFinish, glGetError, glGetIntegerv};
use yakogl::gl::types::*;
use yakogl::gl::vertex::{glEnableVertexAttribArray, glVertexAttribPointer};
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};

/// Sixteen vec4 vertex outputs: 64 components, ES 3.0's minimum.
const OUTPUTS: usize = 16;
/// Fifteen of them read by the fragment shader (GL_MAX_FRAGMENT_INPUT_COMPONENTS 60).
const READ: usize = 15;

const _: () = assert!(MAX_VERTEX_OUTPUT_COMPONENTS == OUTPUTS * 4);

/* ---- a context on a stand-in back end ----------------------------------------- */

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

/// One varying row of a record: name, components, interpolation and the
/// vertex output dword it sits at.
#[derive(Clone, Debug)]
struct Var {
    name: String,
    components: u8,
    interp: u8,
    vtxout_off: u8,
}

/// What a draw handed the back end, copied out of the records.
#[derive(Clone, Debug)]
struct Seen {
    vs_image: Vec<u8>,
    fs_image: Vec<u8>,
    /// (location, first vertex input register) of each attribute.
    vs_attribs: Vec<(u8, u8)>,
    vs_varyings: Vec<Var>,
    vs_output_size: u8,
    fs_varyings: Vec<Var>,
    fs_uses_z: u8,
    fs_uses_w: u8,
    fs_uses_pntc: u8,
}

static SEEN: Mutex<Vec<Seen>> = Mutex::new(Vec::new());

fn name_of(n: &[c_char]) -> String {
    n.iter().take_while(|c| **c != 0).map(|c| *c as u8 as char).collect()
}

fn vars(s: &yakogl::shaderc::ShcShader) -> Vec<Var> {
    s.varyings[..s.num_varyings as usize]
        .iter()
        .map(|v| Var { name: name_of(&v.name), components: v.components, interp: v.interp, vtxout_off: v.vtxout_off })
        .collect()
}

unsafe extern "C" fn b_draw(d: *const GlesDraw) -> i32 {
    let d = &*d;
    if d.index_count == 0 || d.vs.is_null() || d.fs.is_null() {
        return 0;
    }
    let (vs, fs) = (&*d.vs, &*d.fs);
    let bytes = |s: &yakogl::shaderc::ShcShader| s.usc_words().iter().flat_map(|w| w.to_le_bytes()).collect();
    SEEN.lock().unwrap().push(Seen {
        vs_image: bytes(vs),
        fs_image: bytes(fs),
        vs_attribs: vs.attribs[..vs.num_attribs as usize].iter().map(|a| (a.location, a.vi_reg)).collect(),
        vs_varyings: vars(vs),
        vs_output_size: vs.vs_output_size,
        fs_varyings: vars(fs),
        fs_uses_z: fs.uses_z,
        fs_uses_w: fs.uses_w,
        fs_uses_pntc: fs.uses_pntc,
    });
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
    name: c"es3-pvr-vout64".as_ptr() as *const c_char,
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
    max_samples: 0,
};

/// One context for the whole file; the tests take turns on it (the online
/// compiler is not reentrant either).
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
    unsafe { glGetIntegerv(p, &mut v) };
    v
}

fn text_of(log: &[c_char]) -> String {
    name_of(log)
}

fn shader(ty: GLenum, text: &str) -> GLuint {
    let src = CString::new(text).unwrap();
    // SAFETY: plain GL calls; one NUL-terminated source.
    unsafe {
        let s = glCreateShader(ty);
        let p: [*const c_char; 1] = [src.as_ptr()];
        glShaderSource(s, 1, p.as_ptr(), core::ptr::null());
        glCompileShader(s);
        let mut ok: GLint = 0;
        glGetShaderiv(s, GL_COMPILE_STATUS, &mut ok);
        if ok != 1 {
            let mut log = [0 as c_char; 1024];
            glGetShaderInfoLog(s, log.len() as GLsizei, core::ptr::null_mut(), log.as_mut_ptr());
            panic!("did not compile: {}\n{text}", text_of(&log));
        }
        s
    }
}

/// `vs` and `fs` linked, a_pos at location 0 and a_c at 1.
fn program(vs: &str, fs: &str) -> GLuint {
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
        if ok != 1 {
            let mut log = [0 as c_char; 1024];
            glGetProgramInfoLog(p, log.len() as GLsizei, core::ptr::null_mut(), log.as_mut_ptr());
            panic!("did not link: {}\n{vs}\n{fs}", text_of(&log));
        }
        p
    }
}

/// The triangle's three vertices: positions and the a_c attribute.
static POS: [f32; 12] = [-1.0, -1.0, 0.0, 1.0, 3.0, -1.0, 0.0, 1.0, -1.0, 3.0, 0.0, 1.0];
static COL: [f32; 12] = [0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9, 1.0, 0.25, 0.6];

/// `p` drawn once over the triangle: what the back end was handed.
fn draw(p: GLuint) -> Seen {
    let refused_before = backend::draws_refused();
    SEEN.lock().unwrap().clear();
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
        assert_eq!(glGetError(), GL_NO_ERROR, "the draw");
        glUseProgram(0);
    }
    assert_eq!(backend::draws_refused(), refused_before, "the draw was not refused");
    let mut seen = SEEN.lock().unwrap().clone();
    assert_eq!(seen.len(), 1, "one draw reached the back end");
    seen.pop().unwrap()
}

/// The vertex program of `s` run on the byte model for vertex `k`: its
/// output dwords.
fn run_vertex(s: &Seen, k: usize) -> std::collections::BTreeMap<u32, u32> {
    let mut inp = usc_exec::sentinel_inputs(1.0e30);
    for &(loc, reg) in &s.vs_attribs {
        let src = match loc {
            0 => &POS,
            1 => &COL,
            _ => panic!("an attribute at location {loc}"),
        };
        for c in 0..4 {
            inp.vtxin[reg as usize + c] = src[4 * k + c].to_bits();
        }
    }
    usc_exec::run_binary(&s.vs_image, &inp, true).unwrap_or_else(|e| panic!("the vertex program on the model: {e}"))
}

fn dword(out: &std::collections::BTreeMap<u32, u32>, at: u32) -> f32 {
    f32::from_bits(*out.get(&at).unwrap_or_else(|| panic!("vertex output dword {at} not written: {out:?}")))
}

/// The swizzle and scale of output `i` (0..16), so no two outputs carry the
/// same value: `v_i = a_c.<sw> * scale(i) + offset(i)`.
const SWIZZLES: [[usize; 4]; 4] = [[0, 1, 2, 3], [1, 2, 3, 0], [2, 3, 0, 1], [3, 0, 1, 2]];
const SW_TEXT: [&str; 4] = ["xyzw", "yzwx", "zwxy", "wxyz"];
fn scale(i: usize) -> f32 {
    1.0 / (i as f32 + 2.0)
}
fn offset(i: usize) -> f32 {
    i as f32 / 32.0
}

/// The shader's own value of output `i` at vertex `k`.
fn output_value(i: usize, k: usize) -> [f32; 4] {
    let sw = SWIZZLES[i % 4];
    core::array::from_fn(|c| COL[4 * k + sw[c]] * scale(i) + offset(i))
}

/// The vertex shader of sixteen vec4 outputs v0..v15 (scale and offset
/// written as the GLSL literals of [`scale`] and [`offset`]).
fn vertex_shader() -> String {
    let mut vs = String::from("#version 300 es\nin highp vec4 a_pos;\nin highp vec4 a_c;\n");
    for i in 0..OUTPUTS {
        vs += &format!("out highp vec4 v{i};\n");
    }
    vs += "void main() {\n    gl_Position = a_pos;\n    gl_PointSize = 1.0;\n";
    for i in 0..OUTPUTS {
        vs += &format!("    v{i} = a_c.{} * (1.0 / {}.0) + vec4({}.0 / 32.0);\n", SW_TEXT[i % 4], i + 2, i);
    }
    vs += "}\n";
    vs
}

/// The weight of the sum of all fifteen inputs that fragment shader `k`
/// adds to its input `k`: small enough to keep the pixel near v_k, not zero,
/// so every input is read.
const MIX: f32 = 0.001;

/// Fragment shader `k`: reads v0..v14 (not v15), writes
/// `v_k + MIX * (v0 + ... + v14)`.
fn fragment_shader(k: usize) -> String {
    let mut fs = String::from("#version 300 es\nprecision highp float;\nlayout(location = 0) out highp vec4 o;\n");
    for i in 0..READ {
        fs += &format!("in highp vec4 v{i};\n");
    }
    fs += "void main() {\n    highp vec4 s = vec4(0.0);\n";
    for i in 0..READ {
        fs += &format!("    s += v{i};\n");
    }
    fs += &format!("    o = v{k} + s * {MIX:?};\n}}\n");
    fs
}

/// Where the fragment record's inputs start in the coefficient file: Z's
/// four when it iterates Z, W's four when it iterates W, gl_PointCoord's
/// eight, then four per component in list order (the layout the back end's
/// coefficient program writes; tests/usc_limits.rs `coeff_layout`).
fn first_varying_coeff(s: &Seen) -> (Option<usize>, usize) {
    let z = if s.fs_uses_z != 0 { 4 } else { 0 };
    let w = (s.fs_uses_w != 0).then_some(z);
    let pntc = if s.fs_uses_pntc != 0 { 8 } else { 0 };
    (w, z + if w.is_some() { 4 } else { 0 } + pntc)
}

#[test]
fn max_vertex_output_components_reads_64_and_fragment_inputs_60() {
    let _g = context();
    assert_eq!(geti(GL_MAX_VERTEX_OUTPUT_COMPONENTS), 64, "GL_MAX_VERTEX_OUTPUT_COMPONENTS: ES 3.0 table 6.33 asks 64");
    assert_eq!(geti(GL_MAX_FRAGMENT_INPUT_COMPONENTS), 60, "GL_MAX_FRAGMENT_INPUT_COMPONENTS stays 15 * 4");
    assert_eq!(geti(GL_MAX_VARYING_VECTORS), 15, "GL_MAX_VARYING_VECTORS stays 15");
    let mut v64: GLint64 = -1;
    // SAFETY: plain GL calls with real outputs.
    unsafe {
        yakogl::gl::state::glGetInteger64v(GL_MAX_VERTEX_OUTPUT_COMPONENTS, &mut v64);
        assert_eq!(v64, 64, "the getinteger64 form");
        assert_eq!(glGetError(), GL_NO_ERROR);
    }
}

#[test]
fn a_vertex_shader_reads_gl_max_vertex_output_vectors_as_16() {
    let _g = context();
    let vs = "#version 300 es\nin highp vec4 a_pos;\nout highp vec4 v0;\nvoid main() {\n    gl_Position = a_pos;\n    \
              v0 = vec4(float(gl_MaxVertexOutputVectors), float(gl_MaxFragmentInputVectors), 0.0, 1.0);\n}\n";
    let fs = "#version 300 es\nprecision highp float;\nin highp vec4 v0;\nlayout(location = 0) out highp vec4 o;\n\
              void main() { o = v0 / 32.0; }\n";
    let p = program(vs, fs);
    let s = draw(p);
    // SAFETY: plain GL call.
    unsafe { glDeleteProgram(p) };
    let v0 = s.vs_varyings.iter().find(|v| v.name == "v0").expect("v0 in the vertex record");
    let out = run_vertex(&s, 0);
    let got: Vec<f32> = (0..4).map(|c| dword(&out, v0.vtxout_off as u32 + c)).collect();
    assert_eq!(got, [16.0, 15.0, 0.0, 1.0], "gl_MaxVertexOutputVectors, gl_MaxFragmentInputVectors");
}

#[test]
fn sixteen_vec4_outputs_fifteen_read_link_draw_and_colour_the_pixel() {
    let _g = context();
    let vs = vertex_shader();
    for k in 0..READ {
        let p = program(&vs, &fragment_shader(k));
        let s = draw(p);
        // SAFETY: plain GL call.
        unsafe { glDeleteProgram(p) };

        /* the records: sixteen outputs in the vertex record, fifteen inputs
         * in the fragment record, each at the dwords of the vertex output
         * of its name */
        assert_eq!(s.vs_varyings.len(), OUTPUTS, "the vertex record lists the sixteen outputs: {:?}", s.vs_varyings);
        assert_eq!(s.fs_varyings.len(), READ, "the fragment record lists the fifteen inputs: {:?}", s.fs_varyings);
        assert!(
            s.vs_output_size as usize >= 4 + 4 * OUTPUTS,
            "the vertex output holds the position and the sixteen: {}",
            s.vs_output_size
        );
        for f in &s.fs_varyings {
            let v = s.vs_varyings.iter().find(|v| v.name == f.name).unwrap_or_else(|| panic!("{} not written", f.name));
            assert_eq!((f.vtxout_off, f.components), (v.vtxout_off, 4), "{}: linked to the vertex output", f.name);
            assert_eq!(f.interp, 0, "{}: smooth", f.name);
        }

        /* the vertex program on the model: each of the sixteen outputs, at
         * each vertex, is the shader's value */
        let outs: Vec<_> = (0..3).map(|vtx| run_vertex(&s, vtx)).collect();
        for v in &s.vs_varyings {
            let i: usize = v.name[1..].parse().unwrap_or_else(|_| panic!("an output {}", v.name));
            for (vtx, out) in outs.iter().enumerate() {
                for (c, want) in output_value(i, vtx).into_iter().enumerate() {
                    let got = dword(out, v.vtxout_off as u32 + c as u32);
                    assert!((got - want).abs() <= 1e-6, "{}.{c} at vertex {vtx}: {got}, want {want}", v.name);
                }
            }
        }
        for (vtx, out) in outs.iter().enumerate() {
            let pos: Vec<f32> = (0..4).map(|c| dword(out, c)).collect();
            assert_eq!(pos, POS[4 * vtx..4 * vtx + 4], "gl_Position at vertex {vtx}");
        }

        /* the fragment program at the triangle's centre: each input
         * iterated from the vertex output dwords its record names, as the
         * mean of the three vertices */
        let mut inp = usc_exec::sentinel_inputs(1.0e30);
        let (w, mut at) = first_varying_coeff(&s);
        if let Some(w) = w {
            inp.coeff[w] = 1.0f32.to_bits();
        }
        for f in &s.fs_varyings {
            for c in 0..f.components as usize {
                let mean = outs.iter().map(|o| dword(o, f.vtxout_off as u32 + c as u32)).sum::<f32>() / 3.0;
                inp.coeff[at + 4 * c] = mean.to_bits();
                inp.coeff[at + 4 * c + 2] = mean.to_bits();
            }
            at += 4 * f.components as usize;
        }
        let px = usc_exec::run_binary(&s.fs_image, &inp, true)
            .unwrap_or_else(|e| panic!("fragment shader {k} on the model: {e}"));
        let word = *px.get(&0).unwrap_or_else(|| panic!("fragment shader {k}: nothing in pixel output 0: {px:?}"));
        let got = word.to_le_bytes();

        let mean = |i: usize, c: usize| (0..3).map(|vtx| output_value(i, vtx)[c]).sum::<f32>() / 3.0;
        let want: [u8; 4] = core::array::from_fn(|c| {
            let sum: f32 = (0..READ).map(|i| mean(i, c)).sum();
            ((mean(k, c) + sum * MIX).clamp(0.0, 1.0) * 255.0).round() as u8
        });
        assert!(
            got.iter().zip(&want).all(|(g, w)| g.abs_diff(*w) <= 1),
            "fragment shader {k} (o = v{k} + 0.001 * sum): pixel {got:?}, want {want:?}"
        );
    }
}
