// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Byte-compare of the Mali online compiler against the Mesa 26.2.1
//! `deqp_varylink2_*` references (`tools/mali/testdata/`): the dEQP-GLES2
//! shapes that still meet one of the varying layout refusals (`two varying
//! lanes of equal value`, `a varying lane computed from no attribute`, `a
//! gl_Position lane left unwritten`) first at cc89a1cf, after the lowering of
//! `docs/notes/mali-deqp-varylink.md`. Counts, Mesa's link per shape and the
//! proposed lowering are in `docs/notes/mali-deqp-varylink2.md`.
//!
//! `fract_vec2` is byte-equal (plan #387 item negzero-lower, step V1 = P2
//! of `docs/notes/mali-posvary-lowering.md`), and `swz_mixed`,
//! `mediump_vec4_yyw`, `mediump_vec3_bbg` and `mediump_vec4_aaag` (nir_fs.rs
//! `convert_inputs16`: equal lanes beside a distinct lane, the distinct
//! lanes paired by their place in the compacted slot). `prec_mixed`
//! compiles with other words than Mesa's ([`DIFFERS`]). Every other
//! reference is refused at the tip: each test is `#[ignore]`d with its
//! refusal quoted.
//! `refusals_pinned` runs now and asserts each reference's exact current
//! refusal, so the lowering that reaches one moves its pin and
//! removes its ignore. Shape U (`a varying lane left unwritten`) and the
//! struct sampler lane are pinned by `tests/mali_deqp_varylink.rs`
//! (`deqp_varylink_unwritten`) and `varymerge_structsampler`, not here.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_deqp_varylink2
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-mali"))]
mod mali_ref;

const EQUAL: &str = "Valhall: two varying lanes of equal value (Mesa's link merges them) is not lowered yet";
const COLOUR_FORM: &str =
    "Valhall: a gl_FragColor lane other than the varying's own lane or a constant is not lowered yet";
const FS_LOAD: &str = "Valhall: a fragment varying load or conversion of a form no reference draws (a colour lane of unmeasured precision beside a mediump varying, an aligned pair converted apart, lanes converted out of Mesa's order) is not lowered yet";
const NO_ATTRIBUTE: &str = "Valhall: a varying lane computed from no attribute (a constant or a uniform alone, which Mesa's link moves or rewrites) is not lowered yet";

/// Each reference and the refusal it meets at the tip.
const PINS: [(&str, &str); 5] = [
    ("mat2", EQUAL),
    ("loop_bcast", EQUAL),
    ("vec_uniform", COLOUR_FORM),
    ("scalar_uniform", FS_LOAD),
    ("loop_uniform", NO_ATTRIBUTE),
];

/// The references that compile with other words than Mesa's:
/// `prec_mixed`'s fragment sum `var.x + var.y + var.z + var.w` of the merged
/// `z` and `w` is two `FADD.v2f16` of `z` where Mesa makes one `FMA.v2f16
/// z, 2.0, x + y` (a rewrite of `(a + b) + b` no rule of ours makes).
const DIFFERS: &[&str] = &["prec_mixed"];

/// `out0 = in0.grg`: Mesa merges lane 2 into lane 0 at the link and stores
/// two lanes (`VAR0.xy`); the fragment shader converts lanes 0 and 1 as a
/// pair and reads lane 2 as lane 0.
#[test]
fn swz_mixed() {
    mali_ref::assert_matches("deqp_varylink2_swz_mixed");
}

/// `out0 = in0.yyw`: Mesa merges lane 1 into lane 0 and compacts the slot,
/// storing y and w as `VAR0.xy`; the fragment shader converts lanes 0 and 2
/// as one pair (`V2F32_TO_V2F16`) and reads lane 1 as lane 0.
#[test]
fn mediump_vec4_yyw() {
    mali_ref::assert_matches("deqp_varylink2_mediump_vec4_yyw");
}

/// `out0 = in0.bbg`: lane 1 merged into lane 0, b and g stored as
/// `VAR0.xy`, lanes 0 and 2 converted as one pair.
#[test]
fn mediump_vec3_bbg() {
    mali_ref::assert_matches("deqp_varylink2_mediump_vec3_bbg");
}

/// `out0 = in0.aaag`: lanes 1 and 2 merged into lane 0, a and g stored as
/// `VAR0.xy`, lanes 0 and 3 converted as one pair.
#[test]
fn mediump_vec4_aaag() {
    mali_ref::assert_matches("deqp_varylink2_mediump_vec4_aaag");
}

/// `var = vec4(in0, 2.0*in0, -in0, -in0)`: Mesa merges `w` into `z` and
/// stores three lanes (`VAR0.xyz`).
#[test]
#[ignore = "compiles with other words than Mesa's: two FADD.v2f16 of the merged lane where Mesa makes FMA.v2f16 z, 2.0, x + y; pinned by refusals_pinned"]
fn prec_mixed() {
    mali_ref::assert_matches("deqp_varylink2_prec_mixed");
}

/// `out0 = mat2(in0)`: the matrix is one packed vec4; Mesa merges nothing
/// and stores all four lanes, the two 0.0 included.
#[test]
#[ignore = "refused at the link: two varying lanes of equal value (Mesa's link merges them); pinned by refusals_pinned"]
fn mat2() {
    mali_ref::assert_matches("deqp_varylink2_mat2");
}

/// `v_color = vec3(res)` of a run-time loop's sum: Mesa stores the one
/// converted value three times (`VAR0.xyz`).
#[test]
#[ignore = "refused: two varying lanes of equal value (Mesa's link merges them), the vertex shader alone; pinned by refusals_pinned"]
fn loop_bcast() {
    mali_ref::assert_matches("deqp_varylink2_loop_bcast");
}

/// `v_color = vec4(i + h, g, f, b)` with `g` a uniform: Mesa stores `g` as a
/// plain interpolated lane of the one smooth vec4, at word 3 (nir.rs
/// `fill`). The vertex shader compiles; the program's fragment shader reads
/// the load's lanes as `(x, w, y, z)`, a colour form not lowered.
#[test]
#[ignore = "refused: the program's fragment shader, a colour of the load's lanes out of order; pinned by refusals_pinned"]
fn vec_uniform() {
    mali_ref::assert_matches("deqp_varylink2_vec_uniform");
}

/// `a = 3.0 * n` beside interpolated varyings: the fragment shader reads
/// only `a`, which Mesa keeps as one flat lane (`ffma(x, 0.0, x)`). The
/// vertex shader compiles; the fragment shader alone is refused first.
#[test]
#[ignore = "refused: the fragment shader alone, a varying conversion form; pinned by refusals_pinned"]
fn scalar_uniform() {
    mali_ref::assert_matches("deqp_varylink2_scalar_uniform");
}

/// `v_color = vec4(res, 0.0, 0.0, 1.0)` with `res` a loop over constants:
/// Mesa stores all four lanes, smooth.
#[test]
#[ignore = "refused: a varying lane computed from no attribute, the vertex shader alone; pinned by refusals_pinned"]
fn loop_uniform() {
    mali_ref::assert_matches("deqp_varylink2_loop_uniform");
}

/// `color.yz = fract(in0); color * 0.80 + 0.10`: `fract` of the highp copy
/// at fp32, narrowed where the mediump colour's `FMA.v2f16` reads it, and
/// the four lanes stored.
#[test]
fn fract_vec2() {
    mali_ref::assert_matches("deqp_varylink2_fract_vec2");
}

/// The refusal each reference meets at the tip; one message lists every
/// reference that moved.
#[test]
fn refusals_pinned() {
    let mut moved: Vec<String> = PINS
        .iter()
        .filter_map(|(name, text)| mali_ref::refusal_change(&format!("deqp_varylink2_{name}"), text))
        .collect();
    for name in DIFFERS {
        let r = mali_ref::Reference::load(&format!("deqp_varylink2_{name}"));
        match mali_ref::compile_pair(&r.vs, &r.fs) {
            Ok(pair) if mali_ref::differences(&r, &pair).is_empty() => {
                moved.push(format!("{name}: now matches; remove its #[ignore] and its DIFFERS entry"))
            }
            Ok(_) => {}
            Err(e) => moved.push(format!("{name}: expected to compile with other words, refused: {e}")),
        }
    }
    assert!(moved.is_empty(), "pins moved:\n{}", moved.join("\n"));
}
