// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GL_MAX_VARYING_VECTORS 15 and GL_MAX_VERTEX_OUTPUT_COMPONENTS 64 on the
//! V3D: ES 3.0's minimums, carried end to end by the online compiler and the
//! job builder (gl/context.rs MAX_VARYING_VECTORS says where each bound is).
//! The V3D answered 8 / 32 / 32 / 32 (a first cut) and its job builder
//! refused any draw of more than 24 fragment inputs, so the 8 it claimed
//! could not be drawn either.
//!
//! The first part asks the compiler: fifteen vec4 varyings compile, link and
//! lay out (60 inputs, the flat and centroid flags of inputs past 24 in
//! words 1 and 2 of the three the shader record takes, `ShcV3dFs::flags_hi`),
//! and the QPU interpreter (`sim::run`) runs the 60-input fragment program,
//! each input read from the slot the record says it is in. The second is the
//! GL: the four limits read 15 / 60 / 60 / 64, a program of fifteen vec4
//! varyings links and draws (no error, no refused draw, the V3D record the
//! draw hands the back end carries the 60 inputs), and an ES 1.00 program of
//! sixteen is refused by the name of the limit. The job builder's three
//! flag-word packets are tools/v3d/job_check.c part 13b, against Mesa's
//! decoder. Under `report-es2` the answer stays 8 (tests/es2_report_guard.rs);
//! this file is not built there.
//!
//! The dEQP cases this answers (API verdicts, HOST-ONLY, tools/cts/README.md
//! 2b): implementation_limits.max_varying_vectors, max_varying_components,
//! max_fragment_input_components, max_vertex_output_components and the
//! state_query.integers / integers64 forms of them, and the varying shader
//! groups.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_v3d_varyings
//! ```
#![cfg(all(feature = "host-test", feature = "shaderc-online", feature = "gpu-v3d", not(feature = "report-es2")))]
use core::ffi::{c_char, c_void};
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{self, GlesDraw, GlesHeap, YakoglBackend};
use yakogl::gl::consts::*;
use yakogl::gl::context::{MAX_VARYING_VECTORS, MAX_VERTEX_OUTPUT_COMPONENTS};
use yakogl::gl::types::*;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};
use yakogl::shaderc::online::arena::Arena;
use yakogl::shaderc::online::v3d::chain::{compile_program, Prim, Target, MAX_FS_INPUTS, VARYING_SLOT_VAR0};
use yakogl::shaderc::online::v3d::select::TLB_Z_PER_PIXEL;
use yakogl::shaderc::online::v3d::sim::{self, Env, FsInputs, Inputs, Varying, LANES};
use yakogl::shaderc::online::{lower, sema};
use yakogl::shaderc::{ShcShader, SHC_STAGE_FRAGMENT, SHC_STAGE_VERTEX};

/// The fifteen of ES 3.0's table 6.31 (GL_MAX_VARYING_VECTORS minimum).
const N: usize = 15;

/* ---- the bounds the caps rest on ---------------------------------------------- */

const _: () = assert!(MAX_VARYING_VECTORS == N);
/* gl_Position and the fifteen: ES 3.0's 64 (16 vec4) */
const _: () = assert!(MAX_VERTEX_OUTPUT_COMPONENTS == 64);
/* a fragment input is a scalar: the 60 are inside Mesa's V3D_MAX_FS_INPUTS */
const _: () = assert!(N * 4 <= MAX_FS_INPUTS);
/* the flag words hold 24 inputs each, three of them */
const _: () = assert!(MAX_FS_INPUTS <= 3 * 24);

/// The compiler keeps its output in one arena that every compile resets,
/// and is not reentrant: the direct compiles and the GL's (through the
/// context below) take turns on this one lock.
static COMPILING: Mutex<()> = Mutex::new(());

const TARGET: Target = Target { vpm_size: 65536 };

fn text_of(log: &[c_char]) -> String {
    log.iter().take_while(|c| **c != 0).map(|c| *c as u8 as char).collect()
}

/// Which GLSL ES the sources are written in.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Es {
    V100,
    V300,
}

/// A vertex shader writing `n` vec4 varyings `v0..v<n>` from one attribute
/// and a fragment shader summing them all. `flat_last` makes the last of them
/// flat and `centroid_at` the one of that index centroid (ES 3.00 only).
fn sources(es: Es, n: usize, flat_last: bool, centroid_at: Option<usize>) -> (String, String) {
    let (mut vs, mut fs, out, in_, color) = match es {
        Es::V300 => (
            String::from("#version 300 es\nin highp vec4 a_pos;\nin highp vec4 a_c;\n"),
            String::from("#version 300 es\nprecision highp float;\nlayout(location = 0) out highp vec4 o;\n"),
            "out",
            "in",
            "o",
        ),
        Es::V100 => (
            String::from("attribute highp vec4 a_pos;\nattribute highp vec4 a_c;\n"),
            String::from("precision highp float;\n"),
            "varying",
            "varying",
            "gl_FragColor",
        ),
    };
    for i in 0..n {
        let q = if flat_last && i + 1 == n {
            "flat"
        } else if centroid_at == Some(i) {
            "centroid"
        } else {
            ""
        };
        vs += &format!("{q} {out} highp vec4 v{i};\n");
        fs += &format!("{q} {in_} highp vec4 v{i};\n");
    }
    vs += "void main() {\n    gl_Position = a_pos;\n    gl_PointSize = 1.0;\n";
    for i in 0..n {
        let sw = ["xyzw", "yzwx", "zwxy", "wxyz"][i % 4];
        vs += &format!("    v{i} = a_c.{sw} * {}.0;\n", i + 1);
    }
    vs += "}\n";
    fs += "void main() {\n    highp vec4 s = vec4(0.0);\n";
    for i in 0..n {
        fs += &format!("    s += v{i};\n");
    }
    fs += &format!("    {color} = s;\n}}\n");
    (vs, fs)
}

fn compile(stage: u32, src: &str) -> Result<ShcShader, String> {
    let mut out = ShcShader::ZERO;
    let mut log = [0 as c_char; 1024];
    let text = format!("{src}\0");
    let compile = yakogl::shaderc::online::SHC_ONLINE.compile.expect("the online back end compiles");
    // SAFETY: a NUL-terminated source, a real output, and a log of the size announced.
    let rc = unsafe { compile(stage, text.as_ptr() as *const c_char, &mut out, log.as_mut_ptr(), log.len()) };
    if rc == 0 {
        Ok(out)
    } else {
        Err(text_of(&log))
    }
}

/// Both stages compiled and linked: (vertex record, fragment record).
fn build(vs: &str, fs: &str) -> Result<(ShcShader, ShcShader), String> {
    let _g = COMPILING.lock().unwrap_or_else(|e| e.into_inner());
    yakogl::shaderc::online::backend::forget_compiled();
    let vso = compile(SHC_STAGE_VERTEX, vs).map_err(|m| format!("vertex: {m}"))?;
    let mut fso = compile(SHC_STAGE_FRAGMENT, fs).map_err(|m| format!("fragment: {m}"))?;
    let link = yakogl::shaderc::online::SHC_ONLINE.link.expect("the online back end links");
    let mut log = [0 as c_char; 1024];
    // SAFETY: two distinct compiled shaders and a log of the size announced.
    let rc = unsafe { link(&vso, &mut fso, log.as_mut_ptr(), log.len()) };
    if rc != 0 {
        return Err(format!("link: {}", text_of(&log)));
    }
    Ok((vso, fso))
}

/// Bit `i % 24` of word `i / 24` of a record's flag arrays (`[word 0 field,
/// flags_hi]`).
fn flag(word0: u32, hi: [u32; 2], i: usize) -> bool {
    let w = [word0, hi[0], hi[1]][i / 24];
    w & (1 << (i % 24)) != 0
}

#[test]
fn fifteen_vec4_varyings_compile_link_and_fill_the_v3d_record() {
    let (vs, fs) = sources(Es::V300, N, false, None);
    let (v, f) = build(&vs, &fs).unwrap_or_else(|e| panic!("fifteen vec4 varyings do not build: {e}"));
    assert_eq!(f.v3d.linked, 1, "the V3D half is the fragment record's");
    let _ = v;
    assert_eq!(f.v3d.fs.num_inputs as usize, N * 4, "60 scalar inputs");
    assert_eq!(f.v3d.fs.flat_shade_flags, 0, "no flat input");
    assert_eq!(f.v3d.fs.flags_hi, [0; 6]);
    /* the vertex shader's VPM rows: 4 of header (clip, Xs, Ys, Zs, 1/W
     * take the first rows) and the 60, in sectors of 8; the shader record's
     * VPM output size is in sectors, and the 64 KiB VPM keeps at least two
     * batches */
    let (vs_rec, cs_rec) = (&f.v3d.vs, &f.v3d.cs);
    assert!(vs_rec.vpm_output_size >= 8 && vs_rec.vpm_output_size <= 10, "vertex output: {}", vs_rec.vpm_output_size);
    assert!(cs_rec.vpm_output_size >= 1, "coordinate shader output: {}", cs_rec.vpm_output_size);
    assert!(vs_rec.vcm_cache_size >= 2, "vcm cache {}", vs_rec.vcm_cache_size);
}

#[test]
fn flat_and_centroid_flags_past_input_24_land_in_the_high_words() {
    /* v14 (inputs 56..59) flat: word 2 bits 8..11; v7 (inputs 28..31)
     * centroid: word 1 bits 4..7 */
    let (vs, fs) = sources(Es::V300, N, true, Some(7));
    let (_, f) = build(&vs, &fs).unwrap_or_else(|e| panic!("fifteen with flat and centroid do not build: {e}"));
    let p = &f.v3d.fs;
    assert_eq!(p.num_inputs as usize, N * 4);
    for i in 0..N * 4 {
        let (want_flat, want_centroid) = ((56..60).contains(&i), (28..32).contains(&i));
        let hi_flat = [p.flags_hi[0], p.flags_hi[1]];
        let hi_centroid = [p.flags_hi[4], p.flags_hi[5]];
        assert_eq!(flag(p.flat_shade_flags, hi_flat, i), want_flat, "flat flag of input {i}");
        assert_eq!(flag(p.centroid_flags, hi_centroid, i), want_centroid, "centroid flag of input {i}");
        assert!(!flag(p.noperspective_flags, [p.flags_hi[2], p.flags_hi[3]], i), "non-perspective input {i}");
    }
    assert_eq!(p.flat_shade_flags, 0, "word 0 holds none of them");
    assert_eq!(p.flags_hi[1], 0xf00, "flat: word 2");
    assert_eq!(p.flags_hi[4], 0xf0, "centroid: word 1");
}

#[test]
fn a_flat_varying_early_in_the_list_keeps_word_0() {
    /* the old single word: inputs 0..3 flat */
    let (vs, fs) = sources(Es::V300, 2, false, None);
    let fs = fs.replace("in highp vec4 v0;", "flat in highp vec4 v0;");
    let vs = vs.replace("out highp vec4 v0;", "flat out highp vec4 v0;");
    let (_, f) = build(&vs, &fs).unwrap_or_else(|e| panic!("two varyings, the first flat, do not build: {e}"));
    assert_eq!(f.v3d.fs.num_inputs, 8);
    assert_eq!(f.v3d.fs.flat_shade_flags, 0xf);
    assert_eq!(f.v3d.fs.flags_hi, [0; 6]);
}

/// ES 1.00's `varying` count is checked against the limit at the link
/// (reflect::varying_budget, Appendix A packing): fifteen vec4 link -- the
/// corpus pair es100/varyings15 -- and sixteen are refused by the limit's
/// name.
#[test]
fn es100_fifteen_link_and_sixteen_are_refused_by_the_limit() {
    let (vs, fs) = sources(Es::V100, N, false, None);
    let (_, f) = build(&vs, &fs).unwrap_or_else(|e| panic!("fifteen ES 1.00 varyings do not build: {e}"));
    assert_eq!(f.v3d.fs.num_inputs as usize, N * 4);
    let (vs, fs) = sources(Es::V100, N + 1, false, None);
    let err = build(&vs, &fs).err().expect("sixteen ES 1.00 varyings are refused");
    assert!(
        err.starts_with("link:") && err.contains("MAX_VARYING_VECTORS = 15"),
        "refused at the link by the limit's name and value: {err}"
    );
}

/* ---- the 60-input fragment program, on the QPU interpreter ---------------------- */

/// The fragment shader of the sim test: `gl_FragDepth` is a weighted sum of
/// three inputs of the 60, one in each flag word, so the value read says
/// which slot each was fetched from.
fn probe_fs(flat14: bool) -> String {
    let q = if flat14 { "flat" } else { "" };
    let mut s = String::from("#version 300 es\nprecision highp float;\nlayout(location = 0) out highp vec4 o;\n");
    for i in 0..N {
        let qq = if i == 14 {
            q
        } else if i == 7 {
            "centroid"
        } else {
            ""
        };
        s += &format!("{qq} in highp vec4 v{i};\n");
    }
    s += "void main() {\n    highp vec4 s = vec4(0.0);\n";
    for i in 0..N {
        s += &format!("    s += v{i};\n");
    }
    s += "    o = s;\n    gl_FragDepth = v0.x + v7.z * 256.0 + v14.w * 65536.0;\n}\n";
    s
}

/// The depth lane 0 writes when input slot k (in `ldvary` order) carries
/// the constant k + 1.
fn run_probe(flat14: bool) -> f32 {
    let (vs_src, _) = sources(Es::V300, N, false, None);
    let _g = COMPILING.lock().unwrap_or_else(|e| e.into_inner());
    let mut mem = vec![0u8; 64 << 20];
    let a = Arena::new(&mut mem);
    let vs = a.alloc_str(&vs_src).expect("arena");
    let fs_text = probe_fs(flat14);
    let fs = a.alloc_str(&fs_text).expect("arena");
    let v = lower::compile_to_ir(&a, sema::Stage::Vertex, vs).map_err(|e| e.msg.to_string()).expect("the VS lowers");
    let f = lower::compile_to_ir(&a, sema::Stage::Fragment, fs).map_err(|e| e.msg.to_string()).expect("the FS lowers");
    let p = compile_program(&v.module, &f.module, Prim::Triangles, TARGET, &a).expect("60 inputs compile");
    assert_eq!(p.fs.num_inputs as usize, N * 4);
    assert_eq!(p.fs.input_slots.len(), N * 4);
    /* each input's slot carries (VARYING_SLOT_VAR0 + location, component) */
    let zero = [0u32; LANES];
    let one = [1.0f32.to_bits(); LANES];
    let varyings =
        (0..N * 4).map(|k| Varying { vary: zero, c: [((k + 1) as f32).to_bits(); LANES] }).collect::<Vec<_>>();
    let inputs = FsInputs { w: one, w_centroid: one, z: zero, x: zero, y: zero, back_facing: [false; LANES], varyings };
    let env = Env { threads: p.fs.threads, ..Env::default() };
    let stream: Vec<u32> = p.fs.code.uniforms.iter().map(|u| u.data).collect();
    let o = sim::run(p.fs.code.words, &stream, &Inputs::Fragment(Box::new(inputs)), &env).expect("the sim runs it");
    let w = o.tlb.iter().find(|w| w.config == Some(TLB_Z_PER_PIXEL)).expect("a depth write");
    let depth = f32::from_bits(w.value[0].expect("a defined depth"));
    /* the slot of each probed input, from the record */
    let at = |loc: u8, comp: u8| {
        p.fs.input_slots.iter().position(|&s| s == (VARYING_SLOT_VAR0 + loc, comp)).expect("the input is a slot")
    };
    let want = (at(0, 0) + 1) as f32 + (at(7, 2) + 1) as f32 * 256.0 + (at(14, 3) + 1) as f32 * 65536.0;
    assert_eq!(depth, want, "each input is read from the slot the record gives it (flat v14: {flat14})");
    depth
}

#[test]
fn the_interpreter_reads_each_of_the_60_inputs_from_its_slot() {
    run_probe(false);
}

#[test]
fn the_interpreter_runs_the_60_input_program_with_a_flat_input_in_the_last_word() {
    run_probe(true);
}

/* ---- the GL, on a stand-in back end -------------------------------------------- */

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

/// What the back end was handed for each application draw: the V3D record's
/// fragment inputs and the vertex shader's output size in VPM sectors.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Seen {
    fs_inputs: u32,
    vs_vpm_output: u32,
}

static SEEN: Mutex<Vec<Seen>> = Mutex::new(Vec::new());

unsafe extern "C" fn b_draw(d: *const GlesDraw) -> i32 {
    let d = &*d;
    if d.index_count == 0 || d.vs.is_null() || d.fs.is_null() {
        return 0;
    }
    let fs = &*d.fs;
    SEEN.lock().unwrap().push(Seen { fs_inputs: fs.v3d.fs.num_inputs, vs_vpm_output: fs.v3d.vs.vpm_output_size });
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
    name: c"es3-v3d-varyings".as_ptr() as *const c_char,
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

/// One context for the whole file; the tests take turns on it (and on the
/// compiler: `COMPILING`).
static ONCE: Once = Once::new();

fn context() -> MutexGuard<'static, ()> {
    use yakogl::gl::state::{glFinish, glGetError};
    let g = COMPILING.lock().unwrap_or_else(|e| e.into_inner());
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

fn getf(p: GLenum) -> GLfloat {
    let mut v: GLfloat = -1.0;
    // SAFETY: plain GL call with a real output.
    unsafe { yakogl::gl::state::glGetFloatv(p, &mut v) };
    v
}

fn geti64(p: GLenum) -> GLint64 {
    let mut v: GLint64 = -1;
    // SAFETY: plain GL call with a real output.
    unsafe { yakogl::gl::state::glGetInteger64v(p, &mut v) };
    v
}

mod gl {
    use super::*;
    use std::ffi::CString;
    use yakogl::gl::draw::glDrawArrays;
    use yakogl::gl::program::*;
    use yakogl::gl::shader::*;
    use yakogl::gl::state::glGetError;
    use yakogl::gl::vertex::{glEnableVertexAttribArray, glVertexAttribPointer};

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

    /// The program of `n` varyings, linked: Ok(program) or Err(info log).
    fn program(es: Es, n: usize) -> Result<GLuint, String> {
        let (vs, fs) = sources(es, n, false, None);
        let (v, f) = (shader(GL_VERTEX_SHADER, &vs), shader(GL_FRAGMENT_SHADER, &fs));
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

    fn draw_triangle() {
        static TRI: [f32; 12] = [-1.0, -1.0, 0.0, 1.0, 3.0, -1.0, 0.0, 1.0, -1.0, 3.0, 0.0, 1.0];
        static COL: [f32; 12] = [0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9, 1.0, 0.1, 0.2];
        // SAFETY: the arrays are statics and outlive the draw.
        unsafe {
            yakogl::gl::buffer::glBindBuffer(GL_ARRAY_BUFFER, 0);
            glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, TRI.as_ptr() as *const c_void);
            glEnableVertexAttribArray(0);
            glVertexAttribPointer(1, 4, GL_FLOAT, 0, 0, COL.as_ptr() as *const c_void);
            glEnableVertexAttribArray(1);
            glDrawArrays(GL_TRIANGLES, 0, 3);
            yakogl::gl::state::glFinish();
        }
    }

    #[test]
    fn the_four_limits_read_fifteen_vectors_sixty_and_sixty_four_components() {
        let _g = context();
        for (pname, want) in [
            (GL_MAX_VARYING_VECTORS, 15),
            (GL_MAX_VARYING_COMPONENTS, 60),
            (GL_MAX_FRAGMENT_INPUT_COMPONENTS, 60),
            (GL_MAX_VERTEX_OUTPUT_COMPONENTS, 64),
        ] {
            /* the three integer forms of one limit answer the same number
             * (dEQP state_query.integers: _getinteger, _getfloat,
             * _getinteger64 of the same pname) */
            assert_eq!(geti(pname), want, "{pname:#x} through glGetIntegerv");
            assert_eq!(getf(pname), want as f32, "{pname:#x} through glGetFloatv");
            assert_eq!(geti64(pname), want as i64, "{pname:#x} through glGetInteger64v");
        }
        // SAFETY: plain GL call.
        assert_eq!(unsafe { glGetError() }, GL_NO_ERROR);
    }

    #[test]
    fn a_program_of_fifteen_vec4_varyings_links_and_draws() {
        let _g = context();
        let refused_before = backend::draws_refused();
        let p = program(Es::V300, N).unwrap_or_else(|e| panic!("fifteen vec4 varyings do not link: {e}"));
        // SAFETY: plain GL call.
        unsafe { glUseProgram(p) };
        draw_triangle();
        // SAFETY: plain GL call.
        assert_eq!(unsafe { glGetError() }, GL_NO_ERROR, "the draw");
        assert_eq!(backend::draws_refused(), refused_before, "the draw was not refused");
        let seen = SEEN.lock().unwrap().clone();
        assert_eq!(seen.len(), 1, "one draw reached the back end");
        assert_eq!(seen[0].fs_inputs as usize, N * 4, "the V3D record the back end was handed carries the 60 inputs");
        assert!(seen[0].vs_vpm_output >= 8, "the vertex shader's VPM output holds the 60 rows: {seen:?}");
        // SAFETY: plain GL calls.
        unsafe {
            glUseProgram(0);
            glDeleteProgram(p);
        }
    }

    /// ES 1.00 through the GL: fifteen `varying vec4` link (gl_test case 97
    /// "varyings15") and a sixteenth is refused at the link, the info log
    /// naming the limit.
    #[test]
    fn es100_fifteen_varyings_link_and_a_sixteenth_is_refused_by_the_limit_s_name() {
        let _g = context();
        let p = program(Es::V100, N).unwrap_or_else(|e| panic!("fifteen ES 1.00 varyings do not link: {e}"));
        // SAFETY: plain GL call.
        unsafe { glDeleteProgram(p) };
        let log = match program(Es::V100, N + 1) {
            Ok(p) => {
                // SAFETY: plain GL call.
                unsafe { glDeleteProgram(p) };
                panic!("sixteen ES 1.00 varyings linked past MAX_VARYING_VECTORS = 15")
            }
            Err(log) => log,
        };
        assert!(log.contains("MAX_VARYING_VECTORS = 15"), "the info log names the limit: {log}");
        // SAFETY: plain GL call.
        assert_eq!(unsafe { glGetError() }, GL_NO_ERROR, "a failed link is not an error");
    }
}
