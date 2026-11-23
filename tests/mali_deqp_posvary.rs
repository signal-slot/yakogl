// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Byte-compare of the Mali online compiler against the Mesa 26.2.1
//! references `tools/mali/testdata/deqp_posvary_*` (plan #387, item
//! posvary-ref, `docs/notes/mali-deqp-posvary.md`): the dEQP-GLES2 programs
//! whose vertex shader the compiler refuses with one of three texts at the
//! tip (HOST-ONLY, `tools/mali/deqp-host-measure.sh`, 42 cases):
//!
//! * `a varying lane computed from no attribute ...` (32 cases): a uniform
//!   expression stored in a varying beside lanes that come from attributes,
//!   and a loop whose only inputs are uniforms;
//! * `a gl_Position lane left unwritten` (6 cases): the text is misnamed, no
//!   gl_Position lane is unwritten (note section 4);
//! * `a varying lane left unwritten` (4 cases): a varying the vertex shader
//!   declares and never writes.
//!
//! Eleven references are byte-equal ([`MATCHED`]): three `unwritten`, the
//! six `fract_*` (plan #387 item negzero-lower, steps P2 of
//! `docs/notes/mali-posvary-lowering.md`) and the two `tex_uniform_w*`
//! (step P4: a convergent lane in the interpolated vec4's free word). Every
//! other test is `#[ignore]`d with its
//! refusal ([`REFUSED`]) and [`refusals_pinned`] asserts the whole
//! `MaliError` message of every one, exactly, plus that every
//! `deqp_posvary_*` reference on disk is listed: the lowering that lifts one
//! moves it to the byte-compare in the same commit (`assert_matches`, the
//! form of `tests/mali_deqp_intval.rs`). The draws were made with
//! `--uniform-values` (`tools/mali/drm_shim_ref.sh run` and
//! `pan_ref_diff.py extract --draw`, the method of the note's section 3).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_deqp_posvary
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-mali"))]
mod mali_ref;

const NO_ATTR: &str = "Valhall: a varying lane computed from no attribute (a constant or a uniform alone, which Mesa's link moves or rewrites) is not lowered yet";
const CONTROL_FLOW: &str = "Valhall: control flow (if, a loop, a return before the end) is not lowered yet";
const ARITH: &str = "Valhall: arithmetic other than +, -, * and a dot or matrix product (in a fragment shader: +, -, * and abs) is not lowered yet";
const CONST_PRODUCT: &str = "Valhall: a constant product in a shape Mesa's algebraic rules move or fold (a constant times a product with a constant, a negation of a negation, a constant in a fused multiply-add) is not lowered yet";

/// The references the compiler matches byte for byte, without the
/// `deqp_posvary_` prefix: a declared varying the vertex shader never
/// writes is an undef the link takes into the fragment shader's loads
/// (`link::VaryingConsts::push_undef`), no slot stored for it; `fract` of a
/// local copy, fp32 then narrowed into the mediump colour (highp) or fp16
/// throughout (mediump, lowp; nir.rs `vs_ffract16`); a uniform alpha beside
/// texel lanes, interpolated in the vec4's free word (nir.rs `fill`).
const MATCHED: &[&str] = &[
    "unwritten_not_declared",
    "unwritten_declared_unused",
    "unwritten_negative_vs",
    "fract_highp_vec2",
    "fract_highp_vec3",
    "fract_mediump_vec2",
    "fract_mediump_vec3",
    "fract_lowp_vec2",
    "fract_lowp_vec3",
    "tex_uniform_w",
    "tex_uniform_w_array",
];

/// Every reference with its whole `MaliError` message, without the
/// `deqp_posvary_` prefix.
const REFUSED: &[(&str, &str)] = &[
    /* uniform copy beside an attribute vec4 (shape `flat_sep`): the vertex
     * shader alone compiles since P3 (the lane flat after the full vec4); its
     * fragment shader alone is refused first, a constant product */
    ("flat_sep_rnd13", CONST_PRODUCT),
    /* uniform + constant arithmetic beside an attribute vec4, a flat
     * varying of its own in Mesa: the vertex shader compiles since P3, the
     * fragment shader alone is refused first (arithmetic) */
    ("flat_sep_kau_bs26", ARITH),
    /* the uniform vec4 laid out before the interpolated float: Mesa's link
     * moves its lanes after it (words 1-4), which the per-varying layout
     * cannot hold (nir.rs `fill`) */
    ("flat_sep_vec4_bs51", NO_ATTR),
    /* uniform lanes inside a vec4 with attribute lanes (shape `flat_lane`),
     * before them: Mesa moves them after the interpolated lanes */
    ("flat_lane_vec4_v86", NO_ATTR),
    ("flat_lane_vec4_c30", NO_ATTR),
    /* a loop over uniform bounds and constant arrays (shape `loop_phi`) */
    ("loop_phi_array", NO_ATTR),
    /* a declared varying the vertex shader does not write, read under a
     * uniform `if` (shape `unwritten`): the program's fragment shader is
     * Mesa's (the undef folds the select), but the fragment shader alone,
     * which selects its colour under a boolean uniform, is refused */
    ("unwritten_declared_dead_read", CONTROL_FLOW),
];

macro_rules! refused_ref {
    ($($name:ident, $why:literal;)*) => {$(
        #[test]
        #[ignore = $why]
        fn $name() {
            mali_ref::assert_matches(concat!("deqp_posvary_", stringify!($name)));
        }
    )*};
}

refused_ref! {
    flat_sep_rnd13, "refused: the fragment shader alone, a constant product (flat_sep)";
    flat_sep_kau_bs26, "refused: the fragment shader alone, arithmetic (flat_sep)";
    flat_sep_vec4_bs51, "refused: the uniform lanes laid out before the interpolated one, which Mesa moves after it (flat_sep)";
    flat_lane_vec4_v86, "refused: uniform lanes inside a vec4 with attribute lanes (flat_lane)";
    flat_lane_vec4_c30, "refused: uniform lanes inside a vec4 with attribute lanes (flat_lane)";
    loop_phi_array, "refused: a loop over uniform bounds stored in a varying (loop_phi)";
    unwritten_declared_dead_read, "refused: the fragment shader alone selects its colour under a boolean uniform (control flow)";
}

/// A declared varying never written (shape `unwritten`) that the fragment
/// shader does not declare.
#[test]
fn unwritten_not_declared() {
    mali_ref::assert_matches("deqp_posvary_unwritten_not_declared");
}

/// A declared varying never written, declared and not read by the fragment
/// shader.
#[test]
fn unwritten_declared_unused() {
    mali_ref::assert_matches("deqp_posvary_unwritten_declared_unused");
}

/// A declared varying never written, the vertex half of a negative case.
#[test]
fn unwritten_negative_vs() {
    mali_ref::assert_matches("deqp_posvary_unwritten_negative_vs");
}

macro_rules! matched_ref {
    ($($(#[$doc:meta])* $name:ident;)*) => {$(
        $(#[$doc])*
        #[test]
        fn $name() {
            mali_ref::assert_matches(concat!("deqp_posvary_", stringify!($name)));
        }
    )*};
}

matched_ref! {
    /// `fract` of a highp local copy (`a_in0.wy`): `FROUND.f32.rtn` and
    /// `FADD.f32` per lane, narrowed where the mediump colour's fp16
    /// `FMA.v2f16` reads it (`FADD.f32 r0.h0, r0^, 0x0.neg`).
    fract_highp_vec2;
    /// The same of `a_in0.zxy`.
    fract_highp_vec3;
    /// `fract` of a mediump local copy at fp16: each lane narrowed alone,
    /// `FROUND.v2f16.rtn r2, r1.h00` per lane, their `MKVEC.v2i16`, then
    /// `FADD.v2f16 r1, r1^.h00, r2.neg.h00` per lane.
    fract_mediump_vec2;
    /// The same of `a_in0.zxy`: x and y one `V2F32_TO_V2F16` and one
    /// `FROUND.v2f16.rtn r3, r0`, z alone, the load `.v4`.
    fract_mediump_vec3;
    /// lowp: the words of mediump.
    fract_lowp_vec2;
    /// lowp: the words of mediump.
    fract_lowp_vec3;
    /// Texel rgb and a uniform alpha: the alpha is a convergent lane in
    /// the interpolated vec4's free word, stored as it is (no `FMA x, 0,
    /// x`), one `.v4` load.
    tex_uniform_w;
    /// The same through a sampler array.
    tex_uniform_w_array;
}

/// Each reference is refused with exactly its text in [`REFUSED`] (the
/// whole `MaliError` message, not a substring), and the lists ([`REFUSED`],
/// [`MATCHED`]) cover every `deqp_posvary_*` reference on disk.
#[test]
fn refusals_pinned() {
    let mut on_disk: Vec<String> = std::fs::read_dir("tools/mali/testdata")
        .expect("tools/mali/testdata")
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter_map(|n| n.strip_prefix("deqp_posvary_")?.strip_suffix(".expect.json").map(str::to_string))
        .collect();
    on_disk.sort();
    let mut listed: Vec<&str> = REFUSED.iter().map(|(n, _)| *n).chain(MATCHED.iter().copied()).collect();
    listed.sort_unstable();
    assert_eq!(on_disk, listed, "a deqp_posvary_* reference has no test here");
    let mut changed = Vec::new();
    for (name, refused) in REFUSED {
        let r = mali_ref::Reference::load(&format!("deqp_posvary_{name}"));
        match mali_ref::compile_pair(&r.vs, &r.fs) {
            Ok(_) => changed.push(format!("{name}: now compiles; move it to a byte-compare test")),
            Err(e) => {
                if e != *refused {
                    changed.push(format!("{name}: the refusal changed to {e:?}"));
                }
            }
        }
    }
    assert!(changed.is_empty(), "{}", changed.join("\n"));
}
