// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The Mesa 26.2.1 references of the two open steps of trains 1 and 2 that
//! `docs/notes/mali-cfdead-lowering.md` designs (plan #387, item
//! cfdead-refs): **cf-dead** (an `if` whose condition folds to a constant,
//! or whose sides write only values nothing reads after it, is gone before
//! the walk) and the **reordered `gl_FragColor` lanes** (a colour word whose
//! two halves are lanes of one varying load in another order than its
//! pairs, `nir_fs.rs` `vectorise`). Twelve dEQP-GLES2 pairs, eight `if`
//! shapes (`fs_*`, `vs_*`) and four lane shapes (`rl_*`). None of these
//! twelve pairs (vertex and fragment shader together) is the pair of another
//! reference in `tools/mali/testdata`. The cf-dead shapes that other tests
//! already pin (`deqp_ctrlflow_*`, `deqp_posvary_unwritten_declared_dead_read`,
//! `deqp_intval3_rsg_condcomb80_fs`, `deqp_fsint2_rnd_cond9`) and the lane
//! ones (`deqp_fclane_vec3_local`, `deqp_idxloc_fs_mat3_dl_sl`) are listed
//! in section 2 of the note and stay with their own tests. Two
//! fragment shaders are shared: the `vs_*` cases of `random.conditionals.vertex`
//! write the colour from `v_color` as every such case does, and
//! `vs_dead_local`'s (`combined.34`) is the fragment shader of
//! `deqp_constprod_chainlit_fs` (`random.conditionals.fragment.34`, the
//! same generator seed). The vertex shader is the refusing stage of each `vs_*`.
//!
//! Each pair is verbatim from the `.qpa` of `tools/mali/deqp-host-measure.sh`
//! (VK-GL-CTS 93bca01) and was drawn on Mesa's drm-shim with
//! `tools/mali/deqp_ref.sh --instrument shim` (`--uniform-values`); the
//! `.license` files name each case. No reference compiles at the tip:
//! [`refusals_pinned`] asserts each pair's full first refusal. The lowering
//! that lifts one moves its entry from [`REFUSALS`] to [`MATCHED`] in the same
//! commit, and [`compares`] (ignored now) becomes its byte-compare.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_deqp_cfdead
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-mali"))]
mod mali_ref;

/// `MaliError::ControlFlow`.
const CONTROL_FLOW: &str = "Valhall: control flow (if, a loop, a return before the end) is not lowered yet";
/// `MaliError::ColourForm`.
const COLOUR_FORM: &str =
    "Valhall: a gl_FragColor lane other than the varying's own lane or a constant is not lowered yet";

/// Every reference (without the `deqp_cfdead_` prefix) still refused, the
/// dEQP case (under `dEQP-GLES2.functional.`) it is verbatim from, and its
/// full first refusal at the tip. The site measured at `297d118c` is in the
/// comment (section 2 of the note).
const REFUSALS: &[(&str, &str, &str)] = &[
    /* cf-dead, fragment shaders */
    // nir_fs.rs:5578 `walk_if16`: an ivec4 compare, the sides set locals only
    ("fs_int_dead_local", "shaders.random.conditionals.fragment.90", CONTROL_FLOW),
    // nir_fs.rs:5578: an ivec3 compare, the colour stored again after the if
    ("fs_colour_stored_again", "shaders.random.conditionals.fragment.45", CONTROL_FLOW),
    // nir_fs.rs:5603: `int c = d; if (d >= c)`, true; the colours disagree
    ("fs_self_compare", "fragment_ops.interaction.basic_shader.58", CONTROL_FLOW),
    // nir_fs.rs:5619: a 16-bit float compare under a bool, a dead local side
    ("fs_float_cond_dead", "shaders.random.conditionals.fragment.60", CONTROL_FLOW),
    /* cf-dead, vertex shaders */
    // nir.rs:4957 `vs_side_at`: a constant-condition if under another
    ("vs_const_nested", "shaders.random.conditionals.combined.22", CONTROL_FLOW),
    // nir.rs:4947: a side past the peephole limit that sets dead values
    ("vs_dead_local", "shaders.random.conditionals.combined.34", CONTROL_FLOW),
    // nir.rs:4957: a nested if writing `v_color`, stored again after it
    ("vs_varying_overwritten", "shaders.random.conditionals.vertex.74", CONTROL_FLOW),
    // nir.rs:6198: a nested if writing varyings the fragment shader never reads
    ("vs_unread_nested", "shaders.random.conditionals.combined.56", CONTROL_FLOW),
    /* reordered gl_FragColor lanes: nir_fs.rs:11057 `vectorise` */
    // `v_in0.yzwx + v_in1.wzyx`
    ("rl_add_vec4", "shaders.operator.binary_operator.add.mediump_vec4_fragment", COLOUR_FORM),
    // `-v_in0.yzwx`
    ("rl_minus_vec4", "shaders.operator.unary_operator.minus.mediump_vec4_fragment", COLOUR_FORM),
    // two float varyings of one packed load, (b, a)
    ("rl_swizzle", "shaders.random.swizzle.fragment.35", COLOUR_FORM),
    // `res[0][0], res[1][0]` of a mat2 varying: lanes 0 and 2 of one load
    ("rl_mat2_sub", "shaders.matrix.sub.dynamic_mediump_mat2_mat2_fragment", COLOUR_FORM),
];

/// References whose pair compiles and matches Mesa's words byte for byte
/// (`mali_ref::assert_matches`). Empty until a lowering lifts a refusal.
const MATCHED: &[&str] = &[];

fn name(short: &str) -> String {
    format!("deqp_cfdead_{short}")
}

#[test]
fn refusals_pinned() {
    for (short, case, text) in REFUSALS {
        let r = mali_ref::Reference::load(&name(short));
        match mali_ref::compile_pair(&r.vs, &r.fs) {
            Ok(_) => panic!("{short} ({case}) now compiles: move it to MATCHED with its byte compare"),
            Err(e) => assert_eq!(e, *text, "{short} ({case}): the refusal moved"),
        }
    }
}

/// The byte-compares that become the gate once a refusal is lifted
/// (`--ignored`). Each fails at the tip with its refusal.
#[test]
#[ignore = "refused at the tip: the cf-dead and reordered-lanes steps of docs/notes/mali-cfdead-lowering.md"]
fn compares() {
    for (short, ..) in REFUSALS {
        mali_ref::assert_matches(&name(short));
    }
}

#[test]
fn matched_compare() {
    for short in MATCHED {
        mali_ref::assert_matches(&name(short));
    }
}

/// Every `deqp_cfdead_*` reference on disk is listed exactly once, and each
/// has its six files (the pair, its `.license` files, `expect.json` and
/// `.nir.sha`).
#[test]
fn references_listed() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tools/mali/testdata");
    let mut on_disk: Vec<String> = std::fs::read_dir(&dir)
        .expect("tools/mali/testdata")
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter_map(|f| f.strip_prefix("deqp_cfdead_")?.strip_suffix(".expect.json").map(str::to_string))
        .collect();
    on_disk.sort();
    let mut listed: Vec<String> =
        REFUSALS.iter().map(|(n, ..)| n.to_string()).chain(MATCHED.iter().map(|n| n.to_string())).collect();
    listed.sort();
    assert_eq!(on_disk, listed, "a deqp_cfdead_* reference is not listed, or the other way round");
    for n in &listed {
        for ext in ["vert", "frag", "vert.license", "frag.license", "nir.sha"] {
            assert!(dir.join(format!("deqp_cfdead_{n}.{ext}")).is_file(), "deqp_cfdead_{n}.{ext} is missing");
        }
    }
}
