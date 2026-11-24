// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! glmark2's ideas scene on the Mali (plan #367 lower-ideas), against the
//! Mesa 26.2.1 drm-shim references `tools/mali/testdata/gm2_ideas_*` and
//! `gm2_cf_earlyret_vs`, byte for byte (docs/notes/mali-ideas.md):
//!
//! - fs df64, the alpha-tested glyph: `gl_FragCoord.xy / 32.0` of a mediump
//!   shader as a two-lane computed coordinate (`FMA.v2f16` of the narrowed
//!   position by 1/32, each lane widened to the look-up's staging words),
//!   and `if (c.w < 0.5) discard;` on the fp16 texel's half;
//! - vs 51b2, `unitvec`'s early returns inlined as three nested `if`s: the
//!   outer condition folds on the constant second argument (`v2.w == 0.0`
//!   of `vec4(0.0, 0.0, 0.0, 1.0)`), its `if (false)` too, and the one left
//!   is a select of the two negations, negated once after `normalize`
//!   (docs/notes/mali-control-flow.md).
//!
//! fs c565 and 1aca (the same function of two varying or uniform vec4s, Mesa's
//! fp16 and fp32 selects, and 1aca's loop over a struct array) stay refused
//! at INT_VALUE: pinned below, so the item that lowers them updates this file.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_ideas
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-mali"))]
mod mali_ref;

/// ideas df64: `U16_TO_F32` and `FADD_IMM` of the position, its narrowing to
/// one word, `FMA.v2f16 r0, r0^, u3.w0, 0x0.neg` by (1/32, 1/32), `F16_TO_F32`
/// of each half to the staging words, then `DISCARD.f32.lt r1.h1,
/// 0x38000000.h1` of the texel's w.
#[test]
fn gm2_ideas_1_the_glyph_matches_its_reference() {
    mali_ref::assert_matches("gm2_ideas_1");
}

/// A fragment shader that reads every lane of vs 51b2's three varyings, so the
/// link keeps the layout Mesa's draw of gm2_cf_earlyret_vs has (v_eye whole at
/// 0x0, v_nrm at 0x10, v_dir at 0x1c): the reference's own fragment shader
/// stops at FS_VARYING_LOAD (below), a family of its own.
const FS_ALL_LANES: &str = "precision highp float; varying vec3 v_nrm; varying vec4 v_eye; varying vec3 v_dir; \
                            void main() { gl_FragColor = v_eye + vec4(v_nrm + v_dir, 1.0); }";

/// ideas vs 51b2 (gm2_cf_earlyret_vs is its text): both vertex programs and
/// their facts equal the reference's, `FRCP.f32` of `v_eye.w`,
/// `FCMP_OR.f32.eq.m1` against 0.0, three `CSEL.u32.eq` of `v_eye.xyz` and
/// its quotient, the dot product of the select, and `FADD.f32 r0, r0^.neg,
/// 0x0.neg` of each product with the `FRSQ` at its store.
#[test]
fn gm2_cf_earlyret_vs_the_vertex_programs_match_their_reference() {
    let r = mali_ref::Reference::load("gm2_cf_earlyret_vs");
    let pair = mali_ref::compile_pair(&r.vs, FS_ALL_LANES).unwrap_or_else(|e| panic!("refused: {e}"));
    let d: Vec<String> = mali_ref::differences(&r, &pair)
        .into_iter()
        .filter(|x| x.starts_with("vs") || x.starts_with("secondary_offset"))
        .collect();
    assert!(d.is_empty(), "gm2_cf_earlyret_vs's vertex programs differ:\n{}", d.join("\n"));
}

/// gm2_cf_earlyret_vs's own fragment shader, `vec4(v_nrm + v_dir, v_eye.w)`
/// of a mediump shader: a colour lane beside a mediump varying's sum, not this
/// item's (no plan item yet).
#[test]
fn gm2_cf_earlyret_vs_fragment_shader_stays_refused() {
    mali_ref::assert_refused_ref("gm2_cf_earlyret_vs", "a fragment varying load or conversion of a form");
}

/// ideas fs c565 and 1aca: `a.w == 0.0 && b.w == 0.0` of two run-time vec4s
/// is an `iand` of two masks, which Mesa's flattened function selects with at
/// fp16 (`LSHIFT_AND.v2i16`, `MUX.v2i16`) beside the fp32 `CSEL.u32.eq` of the
/// return value (gm2_ideas_2's fs.fragment). Not lowered: when they move,
/// make these [`mali_ref::assert_matches`].
#[test]
fn gm2_ideas_2_and_3_stay_refused_at_their_int_values() {
    let int = "an integer or boolean value is not lowered yet";
    let moved: Vec<String> =
        ["gm2_ideas_2", "gm2_ideas_3"].iter().filter_map(|n| mali_ref::refusal_change(n, int)).collect();
    assert!(moved.is_empty(), "{}", moved.join("\n"));
}

const VS_HEAD: &str = "attribute vec4 p; attribute vec4 a; varying vec4 v; ";
const FS_V: &str = "precision highp float; varying vec4 v; void main() { gl_FragColor = v; }";

/// An `if` that does not fold under another that does not: Mesa flattens the
/// inner one first, then the outer one when its sides stay within the
/// peephole limit; no reference draws it.
#[test]
fn two_kept_vertex_ifs_nested_stay_refused() {
    let vs = format!(
        "{VS_HEAD}void main() {{ gl_Position = p; v = a; \
         if (a.x > 0.0) {{ if (a.y > 0.0) v = -a; else v = a * 2.0; }} }}"
    );
    mali_ref::assert_refused(&vs, FS_V, "control flow (if, a loop");
}

/// A side over the peephole limit once its nested `if` folded stays an `if`
/// Mesa keeps: refused.
#[test]
fn a_side_over_the_peephole_limit_after_a_fold_stays_refused() {
    let vs = format!(
        "{VS_HEAD}void main() {{ gl_Position = p; vec4 r = a; \
         if (a.x > 0.0) {{ if (false) r = a; else r = ((((((((a * a + a) * a + a) * a + a) * a + a) * a + a) * a + a) * a + a) * a); }} \
         v = r; }}"
    );
    mali_ref::assert_refused(&vs, FS_V, "control flow (if, a loop");
}

/// The two-lane coordinate is gl_FragCoord's x and y and constants: one
/// with a uniform has no reference (TEX_COORD); the position's lanes swapped
/// stop before it, at their fp16 arithmetic.
#[test]
fn other_two_lane_frag_coord_coordinates_stay_refused() {
    let vs = "attribute vec4 p; void main() { gl_Position = p; }";
    let fs = "precision mediump float; uniform sampler2D s; uniform vec2 o; \
              void main() { gl_FragColor = texture2D(s, gl_FragCoord.xy / 32.0 + o); }";
    mali_ref::assert_refused(vs, fs, "a texture coordinate other than");
    let fs = "precision mediump float; uniform sampler2D s; \
              void main() { gl_FragColor = texture2D(s, gl_FragCoord.yx / 32.0); }";
    mali_ref::assert_refused(vs, fs, "fp16 arithmetic on gl_FragCoord");
}
