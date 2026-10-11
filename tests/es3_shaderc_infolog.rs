// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The programs dEQP-GLES3's `transform_feedback` group builds, rebuilt on
//! the host and put through the GL API: `glCompileShader` on both stages,
//! then `glTransformFeedbackVaryings` and `glLinkProgram`, the way
//! `createVertexCaptureProgram` does.
//!
//! On the AM62P GLES3 run (tools/cts/RESULTS.md §1r) 48 cases of the group
//! (`interpolation` 18, `random_full_array_capture` 30) failed with
//! `Compile failed: '' at es3fTransformFeedbackTests.cpp`. The `''` is not
//! the info log: VK-GL-CTS 93bca01 throws `tcu::TestError("Compile failed",
//! "", ...)` (es3fTransformFeedbackTests.cpp:1043) with an empty description
//! whatever the log says. What refused them was the PowerVR fragment chain
//! on a `centroid in` varying, accepted since e587452; every
//! `interpolation.centroid.*` program now links here. This file holds both
//! halves: the programs that must link, and, for every refusal that is left,
//! an info log that names its reason.
//!
//! The generator follows `genShaderSources` and the case classes of the
//! same file: every varying is fed from per-vector attributes
//! (`a_<name>_e<i>_c<j>`), written into the `out` varying one vector at a
//! time, and summed in the fragment shader. `RandomCase` draws from a seeded
//! `de::Random`; here it draws from a generator of our own over the same
//! candidate lists and weights, many more seeds than dEQP's, which is the
//! point (the coverage, not the sequence).
//!
//! Needs a build that accepts `#version 300 es` (not `report-es2`).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test es3_shaderc_infolog
//! ```
#![cfg(all(feature = "host-test", feature = "shaderc-online", not(feature = "report-es2")))]

use core::ffi::{c_char, c_void};
use std::ffi::CString;
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{GlesDraw, GlesHeap, YakoglBackend};
use yakogl::gl::consts::*;
use yakogl::gl::es3::*;
use yakogl::gl::program::*;
use yakogl::gl::shader::*;
use yakogl::gl::state::*;
use yakogl::gl::types::*;
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
unsafe extern "C" fn b_draw(_d: *const GlesDraw) -> i32 {
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
    lock: None,
    unlock: None,
    thread_id: None,
};

static BACKEND: YakoglBackend = YakoglBackend {
    name: c"es3-shaderc-infolog".as_ptr() as *const c_char,
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
            assert!(!yakogl::gl::context::gl_context_create(8, 8).is_null());
        }
    });
    g
}

/* ---- the generator --------------------------------------------------------- */

#[derive(Clone, Copy, PartialEq, Debug)]
enum Interp {
    Smooth,
    Flat,
    Centroid,
}

impl Interp {
    fn name(self) -> &'static str {
        match self {
            Interp::Smooth => "smooth",
            Interp::Flat => "flat",
            Interp::Centroid => "centroid",
        }
    }
}

/// A basic GLSL type: its name, whether it is float, columns (1 = vector or
/// scalar), and components per column.
#[derive(Clone, Copy, Debug)]
struct Ty {
    name: &'static str,
    float: bool,
    cols: usize,
    rows: usize,
}

const fn ty(name: &'static str, float: bool, cols: usize, rows: usize) -> Ty {
    Ty { name, float, cols, rows }
}

/// `basicTypes` of the group, in dEQP's order.
const BASIC_TYPES: [Ty; 21] = [
    ty("float", true, 1, 1),
    ty("vec2", true, 1, 2),
    ty("vec3", true, 1, 3),
    ty("vec4", true, 1, 4),
    ty("mat2", true, 2, 2),
    ty("mat2x3", true, 2, 3),
    ty("mat2x4", true, 2, 4),
    ty("mat3x2", true, 3, 2),
    ty("mat3", true, 3, 3),
    ty("mat3x4", true, 3, 4),
    ty("mat4x2", true, 4, 2),
    ty("mat4x3", true, 4, 3),
    ty("mat4", true, 4, 4),
    ty("int", false, 1, 1),
    ty("ivec2", false, 1, 2),
    ty("ivec3", false, 1, 3),
    ty("ivec4", false, 1, 4),
    ty("uint", false, 1, 1),
    ty("uvec2", false, 1, 2),
    ty("uvec3", false, 1, 3),
    ty("uvec4", false, 1, 4),
];

/// `RandomCase::init`'s typeCandidates, in its order (float, int and uint
/// vectors, then the matrices): the first 12, 13, 15, 18 or 21 are drawn
/// from.
const RANDOM_TYPES: [usize; 21] = [0, 1, 2, 3, 13, 14, 15, 16, 17, 18, 19, 20, 4, 5, 6, 7, 8, 9, 10, 11, 12];

const PRECISIONS: [&str; 3] = ["lowp", "mediump", "highp"];

struct Varying {
    name: String,
    ty: Ty,
    prec: &'static str,
    /// 0 = not an array.
    array: usize,
    interp: Interp,
}

/// The column vector type of `t` (`mat3x2` has columns of `vec2`).
fn column_type(t: Ty) -> String {
    if t.rows == 1 {
        "float".into()
    } else {
        format!("vec{}", t.rows)
    }
}

/// One attribute per vector of the varying: the `[e][c]` path that indexes
/// the varying, the dEQP attribute name (`getAttributeName`), its type, and
/// the vector's size.
fn paths(v: &Varying) -> Vec<(String, String, String, usize)> {
    let base = v.name.strip_prefix("v_").unwrap_or(&v.name);
    let elems = v.array.max(1);
    let mut out = Vec::new();
    for e in 0..elems {
        for c in 0..v.ty.cols {
            let mut idx = String::new();
            let mut attr = format!("a_{base}");
            if v.array != 0 {
                idx += &format!("[{e}]");
                attr += &format!("_e{e}");
            }
            let col_ty = if v.ty.cols > 1 {
                idx += &format!("[{c}]");
                attr += &format!("_c{c}");
                column_type(v.ty)
            } else {
                v.ty.name.to_string()
            };
            out.push((idx, attr, col_ty, v.ty.rows));
        }
    }
    out
}

/// `genShaderSources`.
fn gen(varyings: &[Varying], point_size_used: bool, point_size_required: bool) -> (String, String) {
    let mut vtx = String::from("#version 300 es\nin highp vec4 a_position;\n");
    let mut frag = String::from(
        "#version 300 es\nlayout(location = 0) out mediump vec4 o_color;\nuniform highp vec4 u_scale;\nuniform highp vec4 u_bias;\n",
    );
    if point_size_used {
        vtx += "in highp float a_pointSize;\n";
    }
    for v in varyings {
        for (_, attr, col_ty, _) in paths(v) {
            vtx += &format!("in {} {} {};\n", v.prec, col_ty, attr);
        }
    }
    for (dir, s) in [("out", &mut vtx), ("in", &mut frag)] {
        for v in varyings {
            let arr = if v.array != 0 { format!("[{}]", v.array) } else { String::new() };
            s.push_str(&format!("{} {} {} {} {}{};\n", v.interp.name(), dir, v.prec, v.ty.name, v.name, arr));
        }
    }
    vtx += "\nvoid main (void)\n{\n\tgl_Position = a_position;\n";
    frag += "\nvoid main (void)\n{\n\thighp vec4 res = vec4(0.0);\n";
    if point_size_used {
        vtx += "\tgl_PointSize = a_pointSize;\n";
    } else if point_size_required {
        vtx += "\tgl_PointSize = 1.0;\n";
    }
    for v in varyings {
        for (idx, attr, _, rows) in paths(v) {
            vtx += &format!("\t{}{} = {};\n", v.name, idx, attr);
            frag += "\tres += ";
            match rows {
                1 | 4 => frag += &format!("vec4({}{})", v.name, idx),
                2 => frag += &format!("vec2({}{}).xxyy", v.name, idx),
                _ => frag += &format!("vec3({}{}).xyzx", v.name, idx),
            }
            frag += ";\n";
        }
    }
    frag += "\to_color = res * u_scale + u_bias;\n}\n";
    vtx += "}\n";
    (vtx, frag)
}

/// A case: its dEQP-shaped name, the stages, and what it captures how.
struct Case {
    name: String,
    /// Varying vectors (array elements and matrix columns each count one).
    vectors: usize,
    vtx: String,
    frag: String,
    tf: Vec<String>,
    separate: bool,
}

const PRIMS: [(&str, bool); 3] = [("points", true), ("lines", false), ("triangles", false)];
const MODES: [(&str, bool); 2] = [("separate", true), ("interleaved", false)];

fn case(name: String, vs: &[Varying], tf: &[&str], separate: bool, ps_used: bool, ps_req: bool) -> Case {
    let (vtx, frag) = gen(vs, ps_used, ps_req);
    let vectors = vs.iter().map(|v| v.array.max(1) * v.ty.cols).sum();
    Case { name, vectors, vtx, frag, tf: tf.iter().map(|s| s.to_string()).collect(), separate }
}

fn corpus() -> Vec<Case> {
    let mut cases = Vec::new();
    for (mode, separate) in MODES {
        for (prim, req) in PRIMS {
            /* PositionCase, PointSizeCase: no varyings of their own. */
            cases.push(case(format!("position.{mode}.{prim}"), &[], &["gl_Position"], separate, false, req));
            cases.push(case(format!("point_size.{mode}.{prim}"), &[], &["gl_PointSize"], separate, true, req));
            for t in BASIC_TYPES {
                let interp = if t.float { Interp::Smooth } else { Interp::Flat };
                for prec in PRECISIONS {
                    let mk = |name: &str, array: usize| Varying { name: name.into(), ty: t, prec, array, interp };
                    let n = format!("{mode}.{prim}.{prec}_{}", t.name);
                    let ab = ["v_varA", "v_varB"];
                    let both = [mk("v_varA", 0), mk("v_varB", 0)];
                    cases.push(case(format!("basic_types.{n}"), &both, &ab, separate, false, req));
                    /* BasicArrayCase: 1 and 2 elements for matrices and in
                     * separate mode, 3 and 4 otherwise. */
                    let (a, b) = if t.cols > 1 || separate { (1, 2) } else { (3, 4) };
                    let arrays = [mk("v_varA", a), mk("v_varB", b)];
                    cases.push(case(format!("array.{n}"), &arrays, &ab, separate, false, req));
                    /* ArrayElementCase: 3 and 4 always. */
                    let arrays = [mk("v_varA", 3), mk("v_varB", 4)];
                    let el = ["v_varA[1]", "v_varB[0]", "v_varB[3]"];
                    cases.push(case(format!("array_element.{n}"), &arrays, &el, separate, false, req));
                }
            }
            /* interpolation: two vec4 under each mode and precision. */
            for interp in [Interp::Smooth, Interp::Flat, Interp::Centroid] {
                for prec in PRECISIONS {
                    let mk = |name: &str| Varying { name: name.into(), ty: BASIC_TYPES[3], prec, array: 0, interp };
                    let n = format!("interpolation.{}.{prec}_vec4_{prim}_{mode}", interp.name());
                    cases.push(case(n, &[mk("v_varA"), mk("v_varB")], &["v_varA", "v_varB"], separate, false, req));
                }
            }
        }
    }
    /* random and random_full_array_capture: RandomCase::init over a
     * deterministic generator of our own. */
    let mut rng = Rng(0x2545_f491_4f6c_dd1d);
    for seed in 0..600 {
        let separate = seed % 2 == 0;
        let element_capture = (seed / 2) % 2 == 0;
        let (prim, req) = PRIMS[seed % 3];
        let max_tf = if separate { 4 } else { 16 };
        let use_position = rng.float() < 0.7;
        let use_point_size = rng.float() < 0.1;
        let want = rng.range(1, 15 - use_point_size as usize);
        let (mut have, mut ndx) = (0usize, 0usize);
        let mut vs = Vec::new();
        while have < want {
            let max_vecs = if separate { 2.min(want - have) } else { want - have };
            let end = match max_vecs {
                4.. => 21,
                3 => 18,
                2 if separate => 13,
                2 => 15,
                _ => 12,
            };
            let t = BASIC_TYPES[RANDOM_TYPES[rng.range(0, end - 1)]];
            let prec = PRECISIONS[rng.range(0, 2)];
            let interp =
                if t.float { [Interp::Flat, Interp::Smooth, Interp::Centroid][rng.range(0, 2)] } else { Interp::Flat };
            let vecs = t.cols;
            let max_len = 1.max(if separate { 4 / (t.cols * t.rows) } else { max_vecs / vecs });
            let array = if rng.float() < 0.3 { rng.range(1, max_len) } else { 0 };
            vs.push(Varying { name: format!("v_var{ndx}"), ty: t, prec, array, interp });
            have += array.max(1) * vecs;
            ndx += 1;
        }
        let mut cand: Vec<String> = Vec::new();
        if use_position {
            cand.push("gl_Position".into());
        }
        if use_point_size {
            cand.push("gl_PointSize".into());
        }
        for v in &vs {
            if v.array != 0 && element_capture && rng.float() >= 0.5 {
                cand.extend((0..v.array).map(|e| format!("{}[{e}]", v.name)));
            } else {
                cand.push(v.name.clone());
            }
        }
        /* rnd.choose of min(n, max) distinct candidates, then a shuffle. */
        let mut tf = Vec::new();
        while tf.len() < cand.len().min(max_tf) {
            let i = rng.range(0, cand.len() - 1);
            tf.push(cand.remove(i));
        }
        let group = if element_capture { "random" } else { "random_full_array_capture" };
        let tf: Vec<&str> = tf.iter().map(String::as_str).collect();
        let mode = if separate { "separate" } else { "interleaved" };
        let n = format!("{group}.{mode}.{prim}.{seed}");
        cases.push(case(n, &vs, &tf, separate, use_point_size, req));
    }
    cases
}

/// xorshift64*: deterministic, no dependency.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }
    fn float(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }
    /// Inclusive on both ends.
    fn range(&mut self, lo: usize, hi: usize) -> usize {
        lo + (self.next() % (hi - lo + 1) as u64) as usize
    }
}

/* ---- the GL side ------------------------------------------------------------ */

/// Which step refused a program, and the info log it gave.
#[derive(Debug)]
enum Refusal {
    Compile(GLenum, String),
    Link(String),
}

fn shader_log(s: GLuint) -> String {
    let mut log = vec![0u8; 4096];
    let mut len: GLsizei = 0;
    // SAFETY: a buffer of the size announced.
    unsafe { glGetShaderInfoLog(s, log.len() as GLsizei, &mut len, log.as_mut_ptr() as *mut c_char) };
    String::from_utf8_lossy(&log[..len.max(0) as usize]).into_owned()
}

fn program_log(p: GLuint) -> String {
    let mut log = vec![0u8; 4096];
    let mut len: GLsizei = 0;
    // SAFETY: a buffer of the size announced.
    unsafe { glGetProgramInfoLog(p, log.len() as GLsizei, &mut len, log.as_mut_ptr() as *mut c_char) };
    String::from_utf8_lossy(&log[..len.max(0) as usize]).into_owned()
}

/// `createVertexCaptureProgram`: both stages, the varyings, the link.
fn build(c: &Case) -> Result<(), Refusal> {
    // SAFETY: plain GL calls on the one context; every pointer outlives its call.
    unsafe {
        let mut shaders = Vec::new();
        let mut refused = None;
        for (ty, text) in [(GL_VERTEX_SHADER, &c.vtx), (GL_FRAGMENT_SHADER, &c.frag)] {
            let s = glCreateShader(ty);
            let text = CString::new(text.as_str()).unwrap();
            let p: [*const c_char; 1] = [text.as_ptr()];
            glShaderSource(s, 1, p.as_ptr(), core::ptr::null());
            glCompileShader(s);
            let mut ok: GLint = 0;
            glGetShaderiv(s, GL_COMPILE_STATUS, &mut ok);
            if ok != 1 && refused.is_none() {
                refused = Some(Refusal::Compile(ty, shader_log(s)));
            }
            shaders.push(s);
        }
        let mut result = refused.map_or(Ok(()), Err);
        if result.is_ok() {
            let p = glCreateProgram();
            for s in &shaders {
                glAttachShader(p, *s);
            }
            let names: Vec<CString> = c.tf.iter().map(|n| CString::new(n.as_str()).unwrap()).collect();
            let ptrs: Vec<*const c_char> = names.iter().map(|n| n.as_ptr()).collect();
            let mode = if c.separate { GL_SEPARATE_ATTRIBS } else { GL_INTERLEAVED_ATTRIBS };
            glTransformFeedbackVaryings(p, ptrs.len() as GLsizei, ptrs.as_ptr(), mode);
            glLinkProgram(p);
            let mut ok: GLint = 0;
            glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
            if ok != 1 {
                result = Err(Refusal::Link(program_log(p)));
            }
            glDeleteProgram(p);
        }
        for s in shaders {
            glDeleteShader(s);
        }
        while glGetError() != GL_NO_ERROR {}
        result
    }
}

#[test]
fn every_refusal_has_an_info_log() {
    let _g = context();
    let mut refused = Vec::new();
    let mut linked = Vec::new();
    for c in corpus() {
        match build(&c) {
            Ok(()) => linked.push(c.name),
            Err(r) => {
                let log = match &r {
                    Refusal::Compile(_, log) | Refusal::Link(log) => log,
                };
                assert!(!log.trim().is_empty(), "{}: refused with an empty info log: {r:?}", c.name);
                assert!(!log.trim_end().ends_with(':'), "{}: a log with no message: {r:?}", c.name);
                refused.push((c.name, c.vectors, r));
            }
        }
    }
    println!("{} programs link, {} refused", linked.len(), refused.len());
    for (name, vectors, r) in refused.iter().take(8) {
        println!("  {name} ({vectors} vectors): {r:?}");
    }

    /* Every refusal left is one of these, by its log. A new kind fails here
     * until it is looked at and listed. The GL core's are the limits dEQP
     * itself answers NotSupported for (`isProgramSupported`); the back ends'
     * "past 16" ones refuse more varying vectors than they hold, unpacked
     * (two vec2 columns of a mat3x2/mat4x2 array are not shared, so the
     * spec's packing would fit some of them); the rest are back-end gaps on
     * programs within every limit, kept named until their items lift them. */
    let known: &[(&str, bool)] = &[
        ("MAX_TRANSFORM_FEEDBACK_SEPARATE_COMPONENTS", false),
        ("MAX_VERTEX_ATTRIBS", false),
        #[cfg(feature = "gpu-powervr")]
        ("Usc(TooManyFragmentVaryings)", true),
        #[cfg(feature = "gpu-vivante")]
        ("Select(TooManyIo { got: ", true),
        #[cfg(feature = "gpu-vivante")]
        ("Abi(LoadBalancingTempsUnmeasured {", false),
        #[cfg(feature = "gpu-v3d")]
        ("Abi(TooManyVaryings { got: ", true),
        #[cfg(feature = "gpu-v3d")]
        ("varying past input 31", false),
    ];
    for (name, vectors, r) in &refused {
        let log = format!("{r:?}");
        let kind = known.iter().find(|(k, _)| log.contains(k));
        let Some((_, over_16)) = kind else { panic!("{name}: a refusal of a kind not listed: {log}") };
        if *over_16 {
            assert!(*vectors > 16, "{name}: {vectors} varying vectors fit, yet refused: {log}");
        }
    }

    /* Pinned: what links. `interpolation.centroid.*` holds the AM62P's 18
     * `Compile failed` cases (and the random_full_array_capture ones that
     * drew a centroid varying). */
    for must in [
        "position.separate.points",
        "position.interleaved.triangles",
        "point_size.separate.points",
        "point_size.interleaved.lines",
        "interpolation.centroid.lowp_vec4_points_separate",
        "interpolation.centroid.mediump_vec4_lines_interleaved",
        "interpolation.centroid.highp_vec4_triangles_separate",
        "interpolation.flat.highp_vec4_points_interleaved",
        "interpolation.smooth.lowp_vec4_triangles_separate",
        "basic_types.interleaved.points.highp_mat4",
        "basic_types.interleaved.triangles.lowp_uvec4",
        "array.interleaved.points.mediump_mat3x2",
        "array.interleaved.lines.highp_ivec3",
        "array.interleaved.lines.highp_mat4",
        "array_element.separate.triangles.lowp_vec3",
        "array_element.interleaved.points.highp_mat2x4",
    ] {
        assert!(linked.iter().any(|n| n == must), "{must} must link");
    }
    let interp = linked.iter().filter(|n| n.starts_with("interpolation.")).count();
    assert_eq!(interp, 54, "every interpolation.* program links");
    let random = linked.iter().filter(|n| n.starts_with("random")).count();
    /* Ratchets: raise, never lower. The GC7000's 62 and the V3D's 96 below
     * 600 are the back-end gaps of the list above. */
    let floor = if cfg!(feature = "gpu-powervr") {
        597
    } else if cfg!(feature = "gpu-vivante") {
        538
    } else {
        504
    };
    assert!(random >= floor, "{random} of 600 random programs link, floor {floor}");
}
