// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! glmark2's vertex constant-product family (ARITH_CONST) on the Mali:
//! the four Mesa 26.2.1 drm-shim references `tools/mali/testdata/
//! gm2_arithconst_1..4` (docs/notes/mali-vs-constant-products.md section 5)
//! built as glCompileShader and glLinkProgram build them
//! (`tests/mali_ref/mod.rs`) and compared byte for byte.
//!
//! The references were drawn without gl_ref.py's `--uniform-values`, so
//! every uniform word of their FAU is 0: [`assert_matches_unset`] compares
//! the FAU words with the uniforms 0 and everything else as
//! [`mali_ref::differences`] does.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_arith_const
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-mali"))]
mod mali_ref;

use yakogl::shaderc::online::mali::bir::Sysval;
use yakogl::shaderc::online::mali::fau::FauWord;

/// gl_ref.py's default draw: a 64x64 target, scale and offset (32, 32, 0.5).
const VIEWPORT: [u32; 3] = [0x4200_0000, 0x4200_0000, 0x3f00_0000];

/// The FAU words of `fau` for a draw that sets no uniform (all 0).
fn fau_unset(fau: &[FauWord]) -> Vec<u32> {
    let mut v: Vec<u32> = fau
        .iter()
        .map(|w| match *w {
            FauWord::Sysval(Sysval::ViewportScale | Sysval::ViewportOffset, c) => VIEWPORT[c as usize],
            FauWord::Constant(c) => c,
            FauWord::Uniform { .. } | FauWord::UniformHalves { .. } | FauWord::BlendConstant(_) => 0,
            other => panic!("a FAU word this test does not model: {other:?}"),
        })
        .collect();
    if v.len() % 2 == 1 {
        v.push(0);
    }
    v
}

/// The reference's FAU words of `region`, as hex text.
fn fau_ref(json: &str, region: &str) -> Vec<u32> {
    let r = &json[json.find(&format!("\"{region}\"")).unwrap_or_else(|| panic!("{region}"))..];
    let f = &r[r.find("\"fau\":").expect("a fau list")..];
    let list = &f[f.find('[').unwrap() + 1..f.find(']').unwrap()];
    list.split(',').map(|x| u32::from_str_radix(x.trim().trim_matches('"'), 16).expect("a hex word")).collect()
}

/// `name`'s pair compiles and matches its reference in every region, the
/// FAU words with every uniform 0.
fn assert_matches_unset(name: &str) {
    let r = mali_ref::Reference::load(name);
    let pair = mali_ref::compile_pair(&r.vs, &r.fs).unwrap_or_else(|e| panic!("{name}: the pair is refused: {e}"));
    let mut d: Vec<String> =
        mali_ref::differences(&r, &pair).into_iter().filter(|m| !m.contains(": FAU words ours")).collect();
    let mut regions = vec![("vs.position", &pair.vs), ("fs.fragment", &pair.fs)];
    if r.json.contains("\"vs.varying\"") {
        regions.push(("vs.varying", &pair.vs));
    }
    for (region, stage) in regions {
        let (ours, theirs) = (fau_unset(&stage.fau), fau_ref(&r.json, region));
        if ours != theirs {
            d.push(format!("{region}: FAU words ours {ours:08x?}, reference {theirs:08x?}"));
        }
    }
    assert!(d.is_empty(), "{name}: differs from its reference:\n{}", d.join("\n"));
}

/// S2 of docs/notes/mali-vs-constant-products.md: `M * vec4(p.xy, 0.0,
/// 1.0)` (glmark2's shadow b9d1, `LightMatrix * pos4` and `MVP * pos4`):
/// the `z` term gone, column 3 moved into the first term, every lane's
/// `FMA c0, x, c3` before every lane's `FMA c1, y, ..`, in the position and
/// the varying program; the constant `Color` folded into the fragment
/// shader.
#[test]
fn gm2_arithconst_1_matches_its_reference() {
    assert_matches_unset("gm2_arithconst_1");
}

/// `proj * view * vec4(xy, 0.0, 1.0)` (glmark2's ideas a09e): `view * p`
/// first as S2, then `proj` times that, four terms.
#[test]
fn gm2_arithconst_2_matches_its_reference() {
    assert_matches_unset("gm2_arithconst_2");
}

/// `normalize(LightDir.xyz)` of a `const vec4` folded as Mesa's front end
/// folds a built-in call on constants (`dot_f`, then `1.0F / sqrtf`, then
/// the products), `dot(N, L)` with `N`'s reciprocal square root factored
/// out of the constant dot product, the three equal colour lanes one
/// stored lane.
#[test]
fn gm2_arithconst_3_matches_its_reference() {
    assert_matches_unset("gm2_arithconst_3");
}

/// As case 3 with the 0.0 lanes of `Albedo` and of the folded `LightDir`:
/// the dot product's `x` term gone, and the two colour lanes `diffuse *
/// 0.0` stored computed (0) as Mesa's link sees them.
#[test]
fn gm2_arithconst_4_matches_its_reference() {
    assert_matches_unset("gm2_arithconst_4");
}

/// The fold on its own (docs/notes/mali-deqp-constop.md's S1b): a dot
/// product of two constant vectors and `normalize` of a constant vector,
/// varyings of constants the link folds into the fragment shader
/// (vsfold_dotconst, vsfold_normconst, drawn with `--uniform-values`).
#[test]
fn a_vertex_dot_and_normalize_of_constants_match_their_references() {
    mali_ref::assert_matches("vsfold_dotconst");
    mali_ref::assert_matches("vsfold_normconst");
}

/// Equal varying lanes of one value: the vertex shader alone is built
/// without them, as without its constant lanes, so glCompileShader takes
/// it and the program is Mesa's (fsload_dup, fsload_dup_swz; case 3's
/// colour lanes).
#[test]
fn equal_varying_lanes_compile_alone_and_match_their_references() {
    mali_ref::assert_matches("fsload_dup");
    mali_ref::assert_matches("fsload_dup_swz");
}

/// A constant fold whose operand or result is not a finite normal float
/// stays refused: `normalize(vec3(0.0))`'s `1.0 / sqrt(0.0)`.
#[test]
fn a_normalize_of_a_zero_constant_stays_refused() {
    let vs = "attribute vec4 p; varying vec4 v; void main() { v = vec4(normalize(vec3(0.0)), 1.0); gl_Position = p; }";
    let fs = "precision mediump float; varying vec4 v; void main() { gl_FragColor = v; }";
    mali_ref::assert_refused(vs, fs, "not a finite normal float");
}
