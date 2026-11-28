// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Byte-compare of the Mali online compiler against the Mesa 26.2.1
//! references `tools/mali/testdata/deqp_ctrlres_*` (plan #387, item
//! ctrlres-refs, `docs/notes/mali-ctrlres-lowering.md`): the `ControlFlow`
//! residue at the integration tip `349f989f`, the dEQP-GLES2 shapes of CT1
//! steps 3 and 4 (`docs/notes/mali-deqp-ctrlflow.md` section 5) and of the
//! new step CR1 (nested fragment `if`s) that no `deqp_ctrlflow_*`,
//! `deqp_cfdead_*` or `deqp_loops2_*` reference pins. Each pair is verbatim
//! from the `.qpa` of `tools/mali/deqp-host-measure.sh` (VK-GL-CTS
//! 93bca01), the dEQP case its `.license` names, drawn with
//! `tools/mali/deqp_ref.sh --instrument shim` (Mesa 26.2.1's G610
//! drm-shim, `--uniform-values`); `.nir.sha` is from `tools/mali/nir_refs.sh`.
//! `tests/mali_ref` does the compare.
//!
//! A reference the compiler refuses is `#[ignore]`d and `refusals_pinned`
//! holds the whole refusal text ([`REFUSED`]). One that compiles with other
//! words than Mesa's is in [`DIFFERS`], its byte-compare `#[ignore]`d too. One
//! that compiles byte-equal is in [`LOWERED`] and its test runs. The lowering
//! step that lifts a reference moves it in the same commit, drops its
//! `#[ignore]` and updates the note. `references_parse` checks every file and
//! the fragment word count of [`FS_WORDS`].
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_deqp_ctrlres
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-mali"))]
mod mali_ref;

/// The references that compile byte-equal at the tip, without the
/// `deqp_ctrlres_` prefix (none yet).
const LOWERED: &[&str] = &[];

/// The references that compile with other words than Mesa's (none yet).
const DIFFERS: &[&str] = &[];

/// Every ignored reference with its whole `MaliError` message.
const REFUSED: &[(&str, &str)] = &[
    ("fs_float_cond_bvec_else", CONTROL_FLOW),
    ("fs_float_ne_const_nested", CONTROL_FLOW),
    ("fs_int_elseif_nested", CONTROL_FLOW),
    ("fs_int_nested_bool", CONTROL_FLOW),
    ("fs_int_uniform_colours", CONTROL_FLOW),
    ("fs_int_uniform_dead_bvec", CONTROL_FLOW),
    ("fs_ivec4_eq_else_colour", CONTROL_FLOW),
    ("fs_ivec_nested", CONTROL_FLOW),
    ("fs_ivec_uniform_eq", CONTROL_FLOW),
    ("fs_struct_nested_select", CONTROL_FLOW),
    ("vs_nested_bool", CONTROL_FLOW),
    ("vs_nested_int_bool", CONTROL_FLOW),
    ("vs_nested_long", CONTROL_FLOW),
    ("vs_one_side_lane", CONTROL_FLOW),
    ("vs_side_over_limit", CONTROL_FLOW),
];

/// `MaliError::ControlFlow`.
const CONTROL_FLOW: &str = "Valhall: control flow (if, a loop, a return before the end) is not lowered yet";

/// The words of each reference's `fs.fragment` region (Mesa's fragment
/// program, the tail and the blend branch included).
const FS_WORDS: &[(&str, usize)] = &[
    ("fs_float_cond_bvec_else", 37),
    ("fs_float_ne_const_nested", 26),
    ("fs_int_elseif_nested", 33),
    ("fs_int_nested_bool", 25),
    ("fs_int_uniform_colours", 13),
    ("fs_int_uniform_dead_bvec", 15),
    ("fs_ivec4_eq_else_colour", 19),
    ("fs_ivec_nested", 22),
    ("fs_ivec_uniform_eq", 41),
    ("fs_struct_nested_select", 21),
    ("vs_nested_bool", 7),
    ("vs_nested_int_bool", 7),
    ("vs_nested_long", 7),
    ("vs_one_side_lane", 13),
    ("vs_side_over_limit", 9),
];

/// `dEQP-GLES2.functional.shaders.random.conditionals.fragment.92` (CT1 step 3; refused at `nir_fs.rs:5619`).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn fs_float_cond_bvec_else() {
    mali_ref::assert_matches("deqp_ctrlres_fs_float_cond_bvec_else");
}

/// `dEQP-GLES2.functional.shaders.random.conditionals.fragment.39` (CT1 step 3; refused at `nir_fs.rs:5578`).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn fs_float_ne_const_nested() {
    mali_ref::assert_matches("deqp_ctrlres_fs_float_ne_const_nested");
}

/// `dEQP-GLES2.functional.shaders.random.conditionals.fragment.94` (step CR1; refused at `nir_fs.rs:5578`).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn fs_int_elseif_nested() {
    mali_ref::assert_matches("deqp_ctrlres_fs_int_elseif_nested");
}

/// `dEQP-GLES2.functional.shaders.random.conditionals.fragment.88` (step CR1; refused at `nir_fs.rs:5603`).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn fs_int_nested_bool() {
    mali_ref::assert_matches("deqp_ctrlres_fs_int_nested_bool");
}

/// `dEQP-GLES2.functional.fragment_ops.interaction.basic_shader.41` (CT1 step 3; refused at `nir_fs.rs:5603`).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn fs_int_uniform_colours() {
    mali_ref::assert_matches("deqp_ctrlres_fs_int_uniform_colours");
}

/// `dEQP-GLES2.functional.fragment_ops.interaction.basic_shader.68` (CT1 step 3; refused at `nir_fs.rs:5654`).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn fs_int_uniform_dead_bvec() {
    mali_ref::assert_matches("deqp_ctrlres_fs_int_uniform_dead_bvec");
}

/// `dEQP-GLES2.functional.fragment_ops.interaction.basic_shader.4` (CT1 step 3; refused at `nir_fs.rs:5578`).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn fs_ivec4_eq_else_colour() {
    mali_ref::assert_matches("deqp_ctrlres_fs_ivec4_eq_else_colour");
}

/// `dEQP-GLES2.functional.shaders.random.conditionals.fragment.82` (step CR1; refused at `nir_fs.rs:5578`).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn fs_ivec_nested() {
    mali_ref::assert_matches("deqp_ctrlres_fs_ivec_nested");
}

/// `dEQP-GLES2.functional.shaders.random.conditionals.fragment.23` (CT1 step 3; refused at `nir_fs.rs:5578`).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn fs_ivec_uniform_eq() {
    mali_ref::assert_matches("deqp_ctrlres_fs_ivec_uniform_eq");
}

/// `dEQP-GLES2.functional.shaders.struct.local.nested_conditional_assignment_fragment` (CT1 step 3; refused at `nir_fs.rs:5654`).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn fs_struct_nested_select() {
    mali_ref::assert_matches("deqp_ctrlres_fs_struct_nested_select");
}

/// `dEQP-GLES2.functional.shaders.random.conditionals.vertex.6` (CT1 step 4; refused at `nir.rs:6220`).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn vs_nested_bool() {
    mali_ref::assert_matches("deqp_ctrlres_vs_nested_bool");
}

/// `dEQP-GLES2.functional.shaders.random.conditionals.vertex.79` (CT1 step 4; refused at `nir.rs:6220`).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn vs_nested_int_bool() {
    mali_ref::assert_matches("deqp_ctrlres_vs_nested_int_bool");
}

/// `dEQP-GLES2.functional.shaders.random.conditionals.vertex.82` (CT1 step 4; refused at `nir.rs:6220`).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn vs_nested_long() {
    mali_ref::assert_matches("deqp_ctrlres_vs_nested_long");
}

/// `dEQP-GLES2.functional.shaders.random.conditionals.combined.66` (CT1 step 4; refused at `nir.rs:5223`).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn vs_one_side_lane() {
    mali_ref::assert_matches("deqp_ctrlres_vs_one_side_lane");
}

/// `dEQP-GLES2.functional.shaders.random.conditionals.combined.30` (CT1 step 4; refused at `nir.rs:4969`).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn vs_side_over_limit() {
    mali_ref::assert_matches("deqp_ctrlres_vs_side_over_limit");
}

fn on_disk() -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir("tools/mali/testdata")
        .expect("tools/mali/testdata")
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter_map(|n| n.strip_prefix("deqp_ctrlres_")?.strip_suffix(".expect.json").map(str::to_string))
        .collect();
    v.sort();
    v
}

/// The words of `region` in an expect.json, or None when the region is
/// absent; a word that is not 16 hex digits is an error.
fn region_words(json: &str, region: &str) -> Option<Result<usize, String>> {
    let at = json.find(&format!("\"{region}\": {{"))?;
    let rest = &json[at..];
    let words = match rest.find("\"words\": [") {
        Some(w) => &rest[w + "\"words\": [".len()..],
        None => return Some(Err(format!("{region}: no words"))),
    };
    let list = &words[..words.find(']').unwrap_or(0)];
    let mut n = 0;
    for w in list.split(',').map(|w| w.trim().trim_matches('"')).filter(|w| !w.is_empty()) {
        if w.len() != 16 || !w.chars().all(|c| c.is_ascii_hexdigit()) {
            return Some(Err(format!("{region}: {w:?} is not a 64-bit hex word")));
        }
        n += 1;
    }
    Some(Ok(n))
}

/// Every reference on disk parses: both shaders are there and not empty,
/// the trace is the reference's own (`deqp_ref.sh` names it after the
/// prefix), `vs.position` and `fs.fragment` have 64-bit hex words, and
/// `fs.fragment` has the count of [`FS_WORDS`].
#[test]
fn references_parse() {
    let names = on_disk();
    let mut listed: Vec<&str> = FS_WORDS.iter().map(|(n, _)| *n).collect();
    listed.sort_unstable();
    assert_eq!(names, listed, "FS_WORDS does not list every deqp_ctrlres_* reference");
    let mut bad = Vec::new();
    for (name, fs_words) in FS_WORDS {
        let r = mali_ref::Reference::load(&format!("deqp_ctrlres_{name}"));
        if !r.vs.contains("void main") || !r.fs.contains("void main") {
            bad.push(format!("{name}: a shader has no main"));
        }
        if !r.json.contains(&format!("\"trace\": \"deqp_ctrlres_{name}.trace\"")) {
            bad.push(format!("{name}: the trace is not deqp_ctrlres_{name}.trace"));
        }
        for region in ["vs.position", "vs.varying", "fs.fragment"] {
            match region_words(&r.json, region) {
                None if region == "vs.varying" => {}
                None => bad.push(format!("{name}: no {region}")),
                Some(Err(e)) => bad.push(format!("{name}: {e}")),
                Some(Ok(0)) => bad.push(format!("{name}: {region} has no words")),
                Some(Ok(n)) => {
                    if region == "fs.fragment" && n != *fs_words {
                        bad.push(format!("{name}: fs.fragment has {n} words, FS_WORDS says {fs_words}"));
                    }
                }
            }
        }
    }
    assert!(bad.is_empty(), "{}", bad.join("\n"));
}

/// Each refused reference is refused with exactly its text in [`REFUSED`]
/// (the whole `MaliError` message, not a substring), each of [`DIFFERS`]
/// compiles and differs, each of [`LOWERED`] is byte-equal, and the three
/// lists cover every `deqp_ctrlres_*` reference on disk.
#[test]
fn refusals_pinned() {
    let on_disk = on_disk();
    let mut listed: Vec<&str> = LOWERED.iter().chain(DIFFERS).copied().chain(REFUSED.iter().map(|(n, _)| *n)).collect();
    listed.sort_unstable();
    assert_eq!(on_disk, listed, "a deqp_ctrlres_* reference has no outcome here");
    let mut changed = Vec::new();
    for (name, refused) in REFUSED {
        let r = mali_ref::Reference::load(&format!("deqp_ctrlres_{name}"));
        match mali_ref::compile_pair(&r.vs, &r.fs) {
            Ok(pair) => {
                let to = if mali_ref::differences(&r, &pair).is_empty() { "LOWERED" } else { "DIFFERS" };
                changed.push(format!("{name}: now compiles; move it to {to}"));
            }
            Err(e) => {
                if e != *refused {
                    changed.push(format!("{name}: the refusal changed to {e:?}"));
                }
            }
        }
    }
    for name in DIFFERS {
        let r = mali_ref::Reference::load(&format!("deqp_ctrlres_{name}"));
        match mali_ref::compile_pair(&r.vs, &r.fs) {
            Ok(pair) => {
                if mali_ref::differences(&r, &pair).is_empty() {
                    changed.push(format!("{name}: now byte-equal; move it to LOWERED"));
                }
            }
            Err(e) => changed.push(format!("{name}: now refused: {e}")),
        }
    }
    for name in LOWERED {
        let r = mali_ref::Reference::load(&format!("deqp_ctrlres_{name}"));
        match mali_ref::compile_pair(&r.vs, &r.fs) {
            Ok(pair) => {
                let d = mali_ref::differences(&r, &pair);
                if !d.is_empty() {
                    changed.push(format!("{name}: no longer byte-equal: {}", d[0]));
                }
            }
            Err(e) => changed.push(format!("{name}: now refused: {e}")),
        }
    }
    assert!(changed.is_empty(), "{}", changed.join("\n"));
}
