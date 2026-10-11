// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The vertex and fragment shaders dEQP-GLES3's `transform_feedback` group
//! generates, rebuilt on the host and put through the online compiler.
//!
//! On the AM62P GLES3 run (tools/cts/RESULTS.md §1r) 48 mustpass cases of the
//! group died with `Compile failed: '' at es3fTransformFeedbackTests.cpp`:
//! `interpolation.centroid.*` (18) and 30 of the 60
//! `random_full_array_capture.*`. The `''` is NOT the info log. It is the
//! empty `expr` of `tcu::TestError("Compile failed", "", __FILE__, __LINE__)`
//! (es3fTransformFeedbackTests.cpp:1043), which `formatError`
//! (framework/common/tcuDefs.cpp:58) prints between quotes. The info log goes
//! to the `.qpa` only. What refused was the PowerVR fragment chain, on a
//! `centroid` input (`ChainError::FragmentInputsNotBuilt`), and its log named
//! that already.
//!
//! The generator below follows VK-GL-CTS 93bca01
//! (modules/gles3/functional/es3fTransformFeedbackTests.cpp): `genShaderSources`
//! for the text and the case tree of `TransformFeedbackTests::init`.
//! `RandomCase::init` is ported draw for draw, on a port of `de::Random`
//! (framework/delibs/debase/deRandom.c, decpp/deRandom.hpp) and
//! `deInt32Hash` (debase/deInt32.h), so the 60 `random_full_array_capture`
//! programs here are the board's. The fact that exactly 30 of them declare a
//! `centroid` varying is checked below, and is what ties the port to the
//! board's count. `random.*` has the same seeds and therefore the same
//! shaders. It is not in tools/cts/gles3-mustpass.txt.
//!
//! dEQP's generator declares no struct varyings (`ProgramSpec::createStruct`
//! has no caller). A few struct, flat-integer and centroid-matrix programs
//! of the same shape are added so that those constructs stay covered.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test es3_shaderc_infolog
//! ```
//!
//! GLSL ES 3.00 is refused by a `report-es2` build (pp.rs out_of_scope), so
//! the file is empty there.
#![cfg(all(
    feature = "shaderc-online",
    not(feature = "report-es2"),
    any(feature = "gpu-powervr", feature = "gpu-vivante", feature = "gpu-v3d")
))]

use core::ffi::c_char;
use yakogl::shaderc::{ShcShader, SHC_STAGE_FRAGMENT, SHC_STAGE_VERTEX};

// ---- dEQP's random numbers -------------------------------------------------

/// `de::Random` (xorshift128, deRandom.c).
struct DeRandom {
    x: u32,
    y: u32,
    z: u32,
    w: u32,
}

impl DeRandom {
    fn new(seed: u32) -> DeRandom {
        DeRandom {
            x: (seed as i32).wrapping_neg() as u32 ^ 123_456_789,
            y: 362_436_069u32.wrapping_mul(seed),
            z: 521_288_629 ^ (seed >> 7),
            w: 88_675_123 ^ (seed << 3),
        }
    }

    fn get_u32(&mut self) -> u32 {
        let w = self.w;
        let t = self.x ^ (self.x << 11);
        self.x = self.y;
        self.y = self.z;
        self.z = w;
        self.w = (w ^ (w >> 19)) ^ (t ^ (t >> 8));
        self.w
    }

    fn get_float(&mut self) -> f32 {
        (self.get_u32() & 0x0FFF_FFFF) as f32 / (0x0FFF_FFFFu32 + 1) as f32
    }

    /// `getInt(min, max)`, inclusive.
    fn get_int(&mut self, min: i32, max: i32) -> i32 {
        min + (self.get_u32() % ((max as u32).wrapping_sub(min as u32).wrapping_add(1))) as i32
    }

    /// `choose<T>(first, last)`: reservoir sampling of one, which draws once
    /// per candidate after the first.
    fn choose<T: Copy>(&mut self, items: &[T]) -> T {
        let mut val = items[0];
        for (ndx, it) in items.iter().enumerate().skip(1) {
            if self.get_int(0, ndx as i32) < 1 {
                val = *it;
            }
        }
        val
    }
}

/// `deInt32Hash`.
fn de_int32_hash(a: u32) -> u32 {
    let mut key = a;
    key = (key ^ 61) ^ (key >> 16);
    key = key.wrapping_add(key << 3);
    key ^= key >> 4;
    key = key.wrapping_mul(0x27d4_eb2d);
    key ^ (key >> 15)
}

// ---- the program spec ------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Debug)]
enum Interp {
    Smooth,
    Flat,
    Centroid,
}

impl Interp {
    /// `getInterpolationName`.
    fn name(self) -> &'static str {
        match self {
            Interp::Smooth => "smooth",
            Interp::Flat => "flat",
            Interp::Centroid => "centroid",
        }
    }
}

/// A basic GLSL type: its name, whether it is float, its columns (1 for a
/// scalar or vector) and the components per column.
#[derive(Clone, Copy, PartialEq, Debug)]
struct Ty {
    name: &'static str,
    float: bool,
    cols: usize,
    rows: usize,
}

const fn ty(name: &'static str, float: bool, cols: usize, rows: usize) -> Ty {
    Ty { name, float, cols, rows }
}

const FLOAT: Ty = ty("float", true, 1, 1);
const VEC2: Ty = ty("vec2", true, 1, 2);
const VEC3: Ty = ty("vec3", true, 1, 3);
const VEC4: Ty = ty("vec4", true, 1, 4);
const MAT2: Ty = ty("mat2", true, 2, 2);
const MAT2X3: Ty = ty("mat2x3", true, 2, 3);
const MAT2X4: Ty = ty("mat2x4", true, 2, 4);
const MAT3X2: Ty = ty("mat3x2", true, 3, 2);
const MAT3: Ty = ty("mat3", true, 3, 3);
const MAT3X4: Ty = ty("mat3x4", true, 3, 4);
const MAT4X2: Ty = ty("mat4x2", true, 4, 2);
const MAT4X3: Ty = ty("mat4x3", true, 4, 3);
const MAT4: Ty = ty("mat4", true, 4, 4);
const INT: Ty = ty("int", false, 1, 1);
const IVEC2: Ty = ty("ivec2", false, 1, 2);
const IVEC3: Ty = ty("ivec3", false, 1, 3);
const IVEC4: Ty = ty("ivec4", false, 1, 4);
const UINT: Ty = ty("uint", false, 1, 1);
const UVEC2: Ty = ty("uvec2", false, 1, 2);
const UVEC3: Ty = ty("uvec3", false, 1, 3);
const UVEC4: Ty = ty("uvec4", false, 1, 4);

/// `basicTypes` of `TransformFeedbackTests::init`, in its order.
const BASIC_TYPES: [Ty; 21] = [
    FLOAT, VEC2, VEC3, VEC4, MAT2, MAT2X3, MAT2X4, MAT3X2, MAT3, MAT3X4, MAT4X2, MAT4X3, MAT4, INT, IVEC2, IVEC3,
    IVEC4, UINT, UVEC2, UVEC3, UVEC4,
];

/// `RandomCase::init`'s `typeCandidates`, in its order (the vectors, then the
/// matrices: the case slices it by how many vectors are left).
const TYPE_CANDIDATES: [Ty; 21] = [
    FLOAT, VEC2, VEC3, VEC4, INT, IVEC2, IVEC3, IVEC4, UINT, UVEC2, UVEC3, UVEC4, MAT2, MAT2X3, MAT2X4, MAT3X2, MAT3,
    MAT3X4, MAT4X2, MAT4X3, MAT4,
];

const PRECISIONS: [&str; 3] = ["lowp", "mediump", "highp"];

/// `RandomCase::init`'s `interpModes`, in its order.
const RANDOM_INTERPS: [Interp; 3] = [Interp::Flat, Interp::Smooth, Interp::Centroid];

const GL_POINTS: u32 = 0x0000;
const GL_LINES: u32 = 0x0001;
const GL_TRIANGLES: u32 = 0x0004;
const GL_INTERLEAVED_ATTRIBS: u32 = 0x8C8C;
const GL_SEPARATE_ATTRIBS: u32 = 0x8C8D;

const PRIMS: [(&str, u32); 3] = [("points", GL_POINTS), ("lines", GL_LINES), ("triangles", GL_TRIANGLES)];
const MODES: [(&str, u32); 2] = [("separate", GL_SEPARATE_ATTRIBS), ("interleaved", GL_INTERLEAVED_ATTRIBS)];

/// A varying's type: a basic type, or a struct of basic-type members.
#[derive(Clone, Debug)]
enum VTy {
    Basic(Ty),
    Struct(&'static str, Vec<(&'static str, Ty)>),
}

#[derive(Clone, Debug)]
struct Varying {
    name: String,
    ty: VTy,
    prec: &'static str,
    /// 0 = not an array.
    array: usize,
    interp: Interp,
}

impl Varying {
    fn basic(name: &str, t: Ty, prec: &'static str, array: usize, interp: Interp) -> Varying {
        Varying { name: name.into(), ty: VTy::Basic(t), prec, array, interp }
    }
}

/// The column type of `t` (`mat3x2`'s columns are `vec2`); `t` itself for a
/// vector or scalar.
fn column_name(t: Ty) -> String {
    if t.cols == 1 {
        t.name.to_string()
    } else {
        format!("vec{}", t.rows)
    }
}

/// One step of `glu::VectorTypeIterator` over a varying: the path printed
/// after the name (`[e]`, `.m`, `[c]`), the attribute's name
/// (`getAttributeName`), its declared type, and its scalar count.
struct VecPath {
    path: String,
    attr: String,
    ty: String,
    scalars: usize,
}

fn vectors_of(v: &Varying) -> Vec<VecPath> {
    let base = v.name.strip_prefix("v_").unwrap_or(&v.name).to_string();
    let elems = v.array.max(1);
    let mut out = Vec::new();
    let push_basic = |out: &mut Vec<VecPath>, path: &str, attr: &str, t: Ty| {
        for c in 0..t.cols {
            let (p, a) =
                if t.cols > 1 { (format!("{path}[{c}]"), format!("{attr}_c{c}")) } else { (path.into(), attr.into()) };
            out.push(VecPath { path: p, attr: a, ty: column_name(t), scalars: t.rows });
        }
    };
    for e in 0..elems {
        let (path, attr) = if v.array != 0 {
            (format!("[{e}]"), format!("a_{base}_e{e}"))
        } else {
            (String::new(), format!("a_{base}"))
        };
        match &v.ty {
            VTy::Basic(t) => push_basic(&mut out, &path, &attr, *t),
            VTy::Struct(_, members) => {
                for (m, (mname, t)) in members.iter().enumerate() {
                    push_basic(&mut out, &format!("{path}.{mname}"), &format!("{attr}_m{m}"), *t);
                }
            }
        }
    }
    out
}

/// `glu::declare` of a varying's type and name.
fn declare(v: &Varying) -> String {
    let arr = if v.array != 0 { format!("[{}]", v.array) } else { String::new() };
    match &v.ty {
        VTy::Basic(t) => format!("{} {} {}{}", v.prec, t.name, v.name, arr),
        VTy::Struct(sname, _) => format!("{} {}{}", sname, v.name, arr),
    }
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
        let prec = v.prec;
        for p in vectors_of(v) {
            vtx += &format!("in {prec} {} {};\n", p.ty, p.attr);
        }
    }
    for (dir, s) in [("out", &mut vtx), ("in", &mut frag)] {
        /* The named structs, then the varyings. */
        let mut seen: Vec<&str> = Vec::new();
        for v in varyings {
            if let VTy::Struct(sname, members) = &v.ty {
                if !seen.contains(sname) {
                    seen.push(sname);
                    s.push_str(&format!("struct {sname}\n{{\n"));
                    for (mname, t) in members {
                        s.push_str(&format!("\t{} {} {};\n", v.prec, t.name, mname));
                    }
                    s.push_str("};\n");
                }
            }
        }
        for v in varyings {
            s.push_str(&format!("{} {} {};\n", v.interp.name(), dir, declare(v)));
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
        for p in vectors_of(v) {
            vtx += &format!("\t{}{} = {};\n", v.name, p.path, p.attr);
            frag += &match p.scalars {
                2 => format!("\tres += vec2({}{}).xxyy;\n", v.name, p.path),
                3 => format!("\tres += vec3({}{}).xyzx;\n", v.name, p.path),
                _ => format!("\tres += vec4({}{});\n", v.name, p.path),
            };
        }
    }
    frag += "\to_color = res * u_scale + u_bias;\n}\n";
    vtx += "}\n";
    (vtx, frag)
}

/// `RandomCase::init` up to the varyings (the transform-feedback selection
/// that follows draws from the same generator but changes no shader text).
/// Returns the varyings and `usePointSize`.
fn random_varyings(buffer_mode: u32, seed: u32) -> (Vec<Varying>, bool) {
    const MAX_ATTRIBUTE_VECTORS: i32 = 16;
    let separate = buffer_mode == GL_SEPARATE_ATTRIBS;
    let mut rnd = DeRandom::new(seed);
    let _use_position = rnd.get_float() < 0.7;
    let use_point_size = rnd.get_float() < 0.1;
    let to_use = rnd.get_int(1, MAX_ATTRIBUTE_VECTORS - 1 - use_point_size as i32);
    let (mut have, mut ndx) = (0i32, 0);
    let mut out = Vec::new();
    while have < to_use {
        let max_vecs = if separate { 2.min(to_use - have) } else { to_use - have };
        let end = match max_vecs {
            4.. => 21,
            3 => 18,
            2 if separate => 13,
            2 => 15,
            _ => 12,
        };
        let t = rnd.choose(&TYPE_CANDIDATES[..end]);
        let prec = rnd.choose(&PRECISIONS);
        let interp = if t.float { rnd.choose(&RANDOM_INTERPS) } else { Interp::Flat };
        let num_vecs = t.cols as i32;
        let num_comps = (t.cols * t.rows) as i32;
        let max_array_len = 1.max(if separate { 4 / num_comps } else { max_vecs / num_vecs });
        let use_array = rnd.get_float() < 0.3;
        let array_len = if use_array { rnd.get_int(1, max_array_len) } else { 1 };
        out.push(Varying::basic(
            &format!("v_var{ndx}"),
            t,
            prec,
            if use_array { array_len as usize } else { 0 },
            interp,
        ));
        have += array_len * num_vecs;
        ndx += 1;
    }
    (out, use_point_size)
}

/// A case: its dEQP name (below `functional.transform_feedback.`), whether
/// it declares a `centroid` varying, and the two stages' sources.
struct Case {
    name: String,
    centroid: bool,
    /// Varying vectors (an array element and a matrix column count one each).
    vectors: usize,
    vtx: String,
    frag: String,
}

fn case(name: String, varyings: &[Varying], point_size_used: bool, prim: u32) -> Case {
    let (vtx, frag) = gen(varyings, point_size_used, prim == GL_POINTS);
    let centroid = varyings.iter().any(|v| v.interp == Interp::Centroid);
    let vectors = varyings.iter().map(|v| vectors_of(v).len()).sum();
    Case { name, centroid, vectors, vtx, frag }
}

/// `TransformFeedbackTests::init`, plus the extra struct shapes.
fn corpus() -> Vec<Case> {
    let mut cases = Vec::new();
    for (prim, p) in PRIMS {
        for (mode, _) in MODES {
            cases.push(case(format!("position.{prim}_{mode}"), &[], false, p));
            cases.push(case(format!("point_size.{prim}_{mode}"), &[], true, p));
        }
    }
    for (mode, m) in MODES {
        for (prim, p) in PRIMS {
            for t in BASIC_TYPES {
                let interp = if t.float { Interp::Smooth } else { Interp::Flat };
                for prec in PRECISIONS {
                    let n = format!("{mode}.{prim}.{prec}_{}", t.name);
                    let two = |a: usize, b: usize| {
                        [Varying::basic("v_varA", t, prec, a, interp), Varying::basic("v_varB", t, prec, b, interp)]
                    };
                    cases.push(case(format!("basic_types.{n}"), &two(0, 0), false, p));
                    let (a, b) = if t.cols > 1 || m == GL_SEPARATE_ATTRIBS { (1, 2) } else { (3, 4) };
                    cases.push(case(format!("array.{n}"), &two(a, b), false, p));
                    cases.push(case(format!("array_element.{n}"), &two(3, 4), false, p));
                }
            }
        }
    }
    for interp in [Interp::Smooth, Interp::Flat, Interp::Centroid] {
        for prec in PRECISIONS {
            for (prim, p) in PRIMS {
                for (mode, _) in MODES {
                    let vs = [
                        Varying::basic("v_varA", VEC4, prec, 0, interp),
                        Varying::basic("v_varB", VEC4, prec, 0, interp),
                    ];
                    cases.push(case(
                        format!("interpolation.{}.{prec}_vec4_{prim}_{mode}", interp.name()),
                        &vs,
                        false,
                        p,
                    ));
                }
            }
        }
    }
    for group in ["random", "random_full_array_capture"] {
        for (mode, m) in MODES {
            for (prim, p) in PRIMS {
                for ndx in 0..10u32 {
                    let seed = de_int32_hash(m) ^ de_int32_hash(p) ^ de_int32_hash(ndx);
                    let (vs, point_size) = random_varyings(m, seed);
                    cases.push(case(format!("{group}.{mode}.{prim}.{}", ndx + 1), &vs, point_size, p));
                }
            }
        }
    }
    /* Not dEQP's: struct varyings (`out S v`), and flat integers and
     * centroid matrices beside them. */
    let s1 = VTy::Struct("S", vec![("a", FLOAT), ("b", VEC3), ("c", MAT2)]);
    let s2 = VTy::Struct("T", vec![("i", IVEC2), ("u", UINT)]);
    let extra: [(&str, Vec<Varying>); 4] = [
        (
            "struct.smooth",
            vec![Varying { name: "v_s".into(), ty: s1.clone(), prec: "mediump", array: 0, interp: Interp::Smooth }],
        ),
        (
            "struct.centroid",
            vec![Varying { name: "v_s".into(), ty: s1, prec: "highp", array: 0, interp: Interp::Centroid }],
        ),
        (
            "struct.flat_int",
            vec![Varying { name: "v_t".into(), ty: s2, prec: "highp", array: 0, interp: Interp::Flat }],
        ),
        (
            "mixed.centroid_mat_flat_int",
            vec![
                Varying::basic("v_m", MAT3X4, "highp", 2, Interp::Centroid),
                Varying::basic("v_i", IVEC3, "mediump", 2, Interp::Flat),
                Varying::basic("v_f", VEC2, "lowp", 0, Interp::Smooth),
            ],
        ),
    ];
    for (name, vs) in extra {
        cases.push(case(format!("extra.{name}"), &vs, false, GL_TRIANGLES));
    }
    cases
}

// ---- compile and link, as glCompileShader / glLinkProgram reach them -------

fn text_of(log: &[c_char]) -> String {
    log.iter().take_while(|c| **c != 0).map(|c| *c as u8 as char).collect()
}

fn compile(stage: u32, src: &str) -> Result<ShcShader, (i32, String)> {
    let mut out = ShcShader::ZERO;
    let mut log = [0 as c_char; 1024];
    let text = format!("{src}\0");
    let compile = yakogl::shaderc::online::SHC_ONLINE.compile.expect("the online back end compiles");
    // SAFETY: a NUL-terminated source, a real output, and a log of the size announced.
    let rc = unsafe { compile(stage, text.as_ptr() as *const c_char, &mut out, log.as_mut_ptr(), log.len()) };
    if rc == 0 {
        Ok(out)
    } else {
        Err((rc, text_of(&log)))
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Outcome {
    Linked,
    /// The stage, the errno, and the info log.
    CompileFail(&'static str, i32, String),
    LinkFail(i32, String),
}

/// glCompileShader on both stages (dEQP compiles both before it looks), then
/// glLinkProgram.
fn build(vs: &str, fs: &str) -> Outcome {
    yakogl::shaderc::online::backend::forget_compiled();
    let vso = compile(SHC_STAGE_VERTEX, vs);
    let fso = compile(SHC_STAGE_FRAGMENT, fs);
    let (vso, mut fso) = match (vso, fso) {
        (Err((rc, m)), _) => return Outcome::CompileFail("vertex", rc, m),
        (_, Err((rc, m))) => return Outcome::CompileFail("fragment", rc, m),
        (Ok(v), Ok(f)) => (v, f),
    };
    let Some(link) = yakogl::shaderc::online::SHC_ONLINE.link else {
        return Outcome::Linked;
    };
    let mut log = [0 as c_char; 1024];
    // SAFETY: two distinct compiled shaders and a log of the size announced.
    let rc = unsafe { link(&vso, &mut fso, log.as_mut_ptr(), log.len()) };
    if rc == 0 {
        Outcome::Linked
    } else {
        Outcome::LinkFail(rc, text_of(&log))
    }
}

// ---- the tests ---------------------------------------------------------------

/// The functional.transform_feedback cases of tools/cts/gles3-mustpass.txt,
/// below that prefix.
fn mustpass_list() -> Vec<String> {
    let list = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/tools/cts/gles3-mustpass.txt"))
        .expect("tools/cts/gles3-mustpass.txt");
    list.lines()
        .filter_map(|l| l.strip_prefix("dEQP-GLES3.functional.transform_feedback."))
        .map(str::to_string)
        .collect()
}

/// The port of `de::Random` and of the case tree is dEQP's: every one of the
/// mustpass's 882 functional.transform_feedback cases is rebuilt here, and 30
/// of its 60 `random_full_array_capture` programs declare a `centroid`
/// varying, which is the board's `Compile failed` count for that group.
fn the_reconstruction_is_deqps() {
    let cases = corpus();
    let list = mustpass_list();
    assert_eq!(list.len(), 882, "functional.transform_feedback cases in gles3-mustpass.txt");
    for name in &list {
        assert!(cases.iter().any(|c| &c.name == name), "{name} is not rebuilt: the case tree is not dEQP's");
    }
    let listed = |prefix: &str| -> Vec<&Case> {
        cases.iter().filter(|c| c.centroid && c.name.starts_with(prefix) && list.contains(&c.name)).collect()
    };
    let random = listed("random_full_array_capture.");
    assert_eq!(random.len(), 30, "{:?}", random.iter().map(|c| &c.name).collect::<Vec<_>>());
    assert_eq!(listed("interpolation.").len(), 18);
    /* No other mustpass case declares one. */
    assert_eq!(listed("").len(), 48);
}

/// A refused program: its case, whether it is in the mustpass, its varying
/// vectors, and `<stage or link>: <info log>`.
struct Refusal {
    name: String,
    mustpass: bool,
    vectors: usize,
    log: String,
}

/// One test: the binary resets the compiler (`forget_compiled`), so its
/// tests may not run in parallel (src/test_rules.rs).
#[test]
fn every_refusal_has_an_info_log() {
    the_reconstruction_is_deqps();
    let list = mustpass_list();
    let mut linked = Vec::new();
    let mut refused = Vec::new();
    for c in corpus() {
        let (what, rc, log) = match build(&c.vtx, &c.frag) {
            Outcome::Linked => {
                linked.push(c.name);
                continue;
            }
            Outcome::CompileFail(stage, rc, log) => (stage, rc, log),
            Outcome::LinkFail(rc, log) => ("link", rc, log),
        };
        /* The point of the file: a refusal says why. */
        assert!(rc < 0, "{} {what}: a refusal answers an errno, not {rc}", c.name);
        assert!(!log.trim().is_empty(), "{} {what}: refused (rc {rc}) with an empty info log", c.name);
        assert!(!log.trim_end().ends_with(':'), "{} {what}: a log with no message: {log:?}", c.name);
        let mustpass = list.contains(&c.name);
        refused.push(Refusal { mustpass, vectors: c.vectors, log: format!("{what}: {log}"), name: c.name });
    }
    let refused_mustpass = refused.iter().filter(|r| r.mustpass).count();
    println!("{} programs link, {} refused ({refused_mustpass} of them mustpass)", linked.len(), refused.len());
    for r in &refused {
        println!("  {} ({} vectors): {}", r.name, r.vectors, r.log);
    }

    /* Pinned on every GPU: what links whatever the back end. */
    for must in [
        "position.points_separate",
        "point_size.triangles_interleaved",
        "interpolation.flat.mediump_vec4_lines_interleaved",
        "interpolation.smooth.highp_vec4_points_interleaved",
        "basic_types.separate.points.highp_mat4",
        "basic_types.interleaved.triangles.lowp_uvec4",
        "array.interleaved.lines.mediump_mat3x2",
        "array.interleaved.lines.highp_ivec3",
        "array_element.separate.triangles.lowp_vec3",
        "extra.struct.smooth",
        "extra.struct.flat_int",
    ] {
        assert!(linked.iter().any(|n| n == must), "{must} must compile and link");
    }

    /* PowerVR (the AM62P): every mustpass program links, the 48 of §1r
     * included (`centroid` was refused with Usc(FragmentInputsNotBuilt)
     * until chain.rs iterated it, e58745272). The one refusal left is the
     * fragment iterator tables' 16 vectors, named as such. Only the
     * `array_element.*` matrices exceed it (3 + 4 elements of 3 or 4
     * columns); that group is not in the mustpass. */
    #[cfg(feature = "gpu-powervr")]
    {
        assert_eq!(refused_mustpass, 0, "every mustpass program links on the PowerVR");
        for r in &refused {
            assert!(r.vectors > 16, "{} ({} vectors) fits the iterator tables, yet: {}", r.name, r.vectors, r.log);
            assert!(r.log.contains("TooManyFragmentVaryings"), "{}: the log must name the limit: {}", r.name, r.log);
        }
        for must in [
            "interpolation.centroid.lowp_vec4_points_separate",
            "extra.struct.centroid",
            "extra.mixed.centroid_mat_flat_int",
        ] {
            assert!(linked.iter().any(|n| n == must), "{must} must compile and link");
        }
        assert_eq!(refused.len(), 108);
    }
    /* GC7000 (the i.MX8MP): its own limits, named (vertex temps
     * `LoadBalancingTempsUnmeasured`, fragment `TooManyIo`). Two mustpass
     * programs are among them. Ratchets: lower them as limits go. */
    #[cfg(feature = "gpu-vivante")]
    {
        assert!(refused_mustpass <= 2, "{refused_mustpass} mustpass programs refused, ceiling 2");
        assert!(refused.len() <= 124, "{} programs refused, ceiling 124", refused.len());
    }
    /* V3D (the Pi 4): `centroid` is refused at link, by name
     * (`NotYet("centroid varying")`, src/shaderc/online/v3d), and so are
     * flat varyings past input 31 and more than its varyings. Ratchets. */
    #[cfg(feature = "gpu-v3d")]
    {
        assert!(refused_mustpass <= 55, "{refused_mustpass} mustpass programs refused, ceiling 55");
        assert!(refused.len() <= 202, "{} programs refused, ceiling 202", refused.len());
    }
}
