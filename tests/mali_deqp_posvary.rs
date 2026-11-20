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
//! No reference is lowered yet, so each test is `#[ignore]`d with its
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
const POS_LANE: &str = "Valhall: a gl_Position lane left unwritten is not lowered yet";
const VARY_UNWRITTEN: &str = "Valhall: a varying lane left unwritten is not lowered yet";

/// Every reference with its whole `MaliError` message, without the
/// `deqp_posvary_` prefix.
const REFUSED: &[(&str, &str)] = &[
    /* uniform copy beside an attribute vec4: Mesa moves it into the
     * fragment shader (note section 5, shape `flat_sep`) */
    ("flat_sep_rnd13", NO_ATTR),
    /* uniform + constant arithmetic beside an attribute vec4: stays a flat
     * varying in Mesa */
    ("flat_sep_kau_bs26", NO_ATTR),
    ("flat_sep_vec4_bs51", NO_ATTR),
    /* uniform lanes inside a vec4 with attribute lanes (shape `flat_lane`) */
    ("flat_lane_vec4_v86", NO_ATTR),
    ("flat_lane_vec4_c30", NO_ATTR),
    /* texture look-up rgb and a uniform alpha (shape `tex_uniform_w`);
     * the struct sampler of dEQP is flattened, gl_ref.py cannot set it */
    ("tex_uniform_w", NO_ATTR),
    ("tex_uniform_w_array", NO_ATTR),
    /* a loop over uniform bounds and constant arrays (shape `loop_phi`) */
    ("loop_phi_array", NO_ATTR),
    /* a declared varying the vertex shader does not write (shape
     * `unwritten`) */
    ("unwritten_not_declared", VARY_UNWRITTEN),
    ("unwritten_declared_unused", VARY_UNWRITTEN),
    ("unwritten_declared_dead_read", VARY_UNWRITTEN),
    ("unwritten_negative_vs", VARY_UNWRITTEN),
    /* `fract` of a local copy of an attribute swizzle, written into the
     * lanes of a mediump colour (shape `fract_local`) */
    ("fract_highp_vec2", POS_LANE),
    ("fract_highp_vec3", POS_LANE),
    ("fract_mediump_vec2", POS_LANE),
    ("fract_mediump_vec3", POS_LANE),
    ("fract_lowp_vec2", POS_LANE),
    ("fract_lowp_vec3", POS_LANE),
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
    flat_sep_rnd13, "refused: a uniform-only varying beside attribute lanes (flat_sep)";
    flat_sep_kau_bs26, "refused: a uniform-only varying beside attribute lanes (flat_sep)";
    flat_sep_vec4_bs51, "refused: a uniform-only varying beside attribute lanes (flat_sep)";
    flat_lane_vec4_v86, "refused: uniform lanes inside a vec4 with attribute lanes (flat_lane)";
    flat_lane_vec4_c30, "refused: uniform lanes inside a vec4 with attribute lanes (flat_lane)";
    tex_uniform_w, "refused: texture rgb and a uniform alpha in one vec4 (tex_uniform_w)";
    tex_uniform_w_array, "refused: texture rgb and a uniform alpha in one vec4 (tex_uniform_w)";
    loop_phi_array, "refused: a loop over uniform bounds stored in a varying (loop_phi)";
    unwritten_not_declared, "refused: a declared varying never written (unwritten)";
    unwritten_declared_unused, "refused: a declared varying never written (unwritten)";
    unwritten_declared_dead_read, "refused: a declared varying never written (unwritten)";
    unwritten_negative_vs, "refused: a declared varying never written (unwritten), the vertex half of a negative case";
    fract_highp_vec2, "refused as a gl_Position lane left unwritten: fract of a local copy (fract_local)";
    fract_highp_vec3, "refused as a gl_Position lane left unwritten: fract of a local copy (fract_local)";
    fract_mediump_vec2, "refused as a gl_Position lane left unwritten: fract of a local copy (fract_local)";
    fract_mediump_vec3, "refused as a gl_Position lane left unwritten: fract of a local copy (fract_local)";
    fract_lowp_vec2, "refused as a gl_Position lane left unwritten: fract of a local copy (fract_local)";
    fract_lowp_vec3, "refused as a gl_Position lane left unwritten: fract of a local copy (fract_local)";
}

/// Each reference is refused with exactly its text in [`REFUSED`] (the
/// whole `MaliError` message, not a substring), and the list covers every
/// `deqp_posvary_*` reference on disk.
#[test]
fn refusals_pinned() {
    let mut on_disk: Vec<String> = std::fs::read_dir("tools/mali/testdata")
        .expect("tools/mali/testdata")
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter_map(|n| n.strip_prefix("deqp_posvary_")?.strip_suffix(".expect.json").map(str::to_string))
        .collect();
    on_disk.sort();
    let mut listed: Vec<&str> = REFUSED.iter().map(|(n, _)| *n).collect();
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
