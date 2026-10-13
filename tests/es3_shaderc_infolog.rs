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

    /* The GL API's side of the same claim (glTransformFeedbackVaryings, then
     * glLinkProgram on the captured names). It runs here and not as a second
     * #[test], because this binary resets the compiler (src/test_rules.rs). */
    #[cfg(feature = "host-test")]
    gl_api::through_the_gl_api();
}

// ---- the same group through the GL API ---------------------------------------

#[cfg(feature = "host-test")]
mod gl_api {
    //! The programs dEQP-GLES3's `transform_feedback` group builds, rebuilt
    //! on the host and put through the GL API: `glCompileShader` on both
    //! stages, then `glTransformFeedbackVaryings` and `glLinkProgram`, the way
    //! `createVertexCaptureProgram` does.
    //!
    //! The generator follows `genShaderSources` and the case classes of the
    //! same file: every varying is fed from per-vector attributes
    //! (`a_<name>_e<i>_c<j>`), written into the `out` varying one vector at a
    //! time, and summed in the fragment shader. `RandomCase` draws here from
    //! a generator of our own over the same candidate lists and weights, many
    //! more seeds than dEQP's, which is the point (the coverage, not the
    //! sequence; the file above holds dEQP's own sequence).

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
                let interp = if t.float {
                    [Interp::Flat, Interp::Smooth, Interp::Centroid][rng.range(0, 2)]
                } else {
                    Interp::Flat
                };
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

    pub fn through_the_gl_api() {
        let _g = context();
        let mut refused = Vec::new();
        let mut linked = Vec::new();
        for c in corpus() {
            match build(&c) {
                Ok(()) => linked.push(c.name),
                Err(r) => {
                    let (step, log) = match &r {
                        Refusal::Compile(GL_VERTEX_SHADER, log) => ("vertex compile", log),
                        Refusal::Compile(_, log) => ("fragment compile", log),
                        Refusal::Link(log) => ("link", log),
                    };
                    assert!(!log.trim().is_empty(), "{}: {step} refused with an empty info log", c.name);
                    assert!(!log.trim_end().ends_with(':'), "{}: {step} log with no message: {log:?}", c.name);
                    refused.push((c.name, c.vectors, r));
                }
            }
        }
        println!("GL API: {} programs link, {} refused", linked.len(), refused.len());
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
}
