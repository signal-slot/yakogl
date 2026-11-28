// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Byte-compare of the Mali online compiler against the Mesa 26.2.1
//! references `tools/mali/testdata/deqp_newtext_*` (plan #387, item
//! newtexts-refs, `docs/notes/mali-newtext-lowering.md`): the shapes of the
//! small refusal texts that no step owned at `e9033663` (section 19.1 of
//! `docs/notes/mali-deqp-families.md`), measured again at `070608e37`: the
//! link refusal "varying at different offsets", `Defect(LaneCycle)`,
//! `UboFallback`, `FragmentTooLarge`, `TexLanes`, `TooManyValues`,
//! `PrecisionUnknown`, `TexHighp` and `Fp16Inexact`. A shape an earlier
//! reference already draws has no reference here (the note's section 3).
//! `tests/mali_ref` does the compare.
//!
//! Each pair is verbatim from the `.qpa` of `deqp-gles2` (VK-GL-CTS 93bca01)
//! that `tools/mali/deqp-host-measure.sh` logged; each reference was drawn
//! with `tools/mali/deqp_ref.sh --instrument shim` (Mesa 26.2.1's G610
//! drm-shim, `--uniform-values`), except the two of [`ZERO_UNIFORMS`]
//! (`--zero-uniforms`: a sampler in a struct, which gl_ref.py does not set).
//!
//! A reference the compiler refuses is `#[ignore]`d and `refusals_pinned`
//! holds the whole refusal text ([`REFUSED`]). The lowering step that lifts one
//! moves it to [`LOWERED`] (byte-equal) or [`DIFFERS`] (compiles with other
//! words than Mesa's) in the same commit, drops its `#[ignore]` and updates the
//! note. `references_parse` checks every file and the fragment word count the
//! note quotes ([`FS_WORDS`]).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_deqp_newtext
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-mali"))]
mod mali_ref;

/// The references a lowering step made byte-equal, without the
/// `deqp_newtext_` prefix.
const LOWERED: &[&str] = &[];

/// The references that compile with other words than Mesa's (hazards).
const DIFFERS: &[&str] = &[];

/// The references drawn with every uniform 0 (`deqp_ref.sh --zero-uniforms`).
const ZERO_UNIFORMS: &[&str] = &["texlanes_struct_sampler_nested", "values_random79"];

/// The link's refusal (`link.rs` `MALI_LAYOUTS_DIFFER`) as `compile_pair`
/// returns it; the varying it names follows.
macro_rules! link {
    () => {
        "online shaderc: a Mali link whose stages place a varying at different offsets is not lowered yet"
    };
}

/// Every ignored reference with its whole refusal (a `MaliError` message, or
/// the link's line with the varying it names).
const REFUSED: &[(&str, &str)] = &[
    ("fp16_sub_highp_mat3", TEXT_FP16),
    ("ftl_nested_initial", TEXT_FTL),
    ("ftl_random20", TEXT_FTL),
    ("ftl_struct_in_array_both", TEXT_FTL),
    ("ftl_struct_in_array_fs", TEXT_FTL),
    ("lanecycle_tex83", TEXT_LANECYCLE),
    ("link_bexpr53", concat!(link!(), " ('a')")),
    ("link_bexpr85", concat!(link!(), " ('e')")),
    ("link_bexpr9", concat!(link!(), " ('f')")),
    ("link_cmp17", concat!(link!(), " ('c')")),
    ("link_cmp41", concat!(link!(), " ('j')")),
    ("link_cmp45", concat!(link!(), " ('k')")),
    ("link_cond41", concat!(link!(), " ('g')")),
    ("link_cond47", concat!(link!(), " ('c')")),
    ("link_conv17", concat!(link!(), " ('c')")),
    ("link_conv32", concat!(link!(), " ('c')")),
    ("link_conv53", concat!(link!(), " ('a')")),
    ("link_seq_side_highp_vec4", concat!(link!(), " ('v_in2')")),
    ("link_swz17", concat!(link!(), " ('c')")),
    ("link_tex4", concat!(link!(), " ('b')")),
    ("precision_trig51", TEXT_PREC),
    ("texhighp_sampler_value", TEXT_TEXHIGHP),
    ("texlanes_struct_sampler_nested", TEXT_TEXLANES),
    ("texlanes_tex44", TEXT_TEXLANES),
    ("texlanes_tex55", TEXT_TEXLANES),
    ("ubo_light_10omni", TEXT_UBO),
    ("ubo_light_12omni", TEXT_UBO),
    ("ubo_light_14omni", TEXT_UBO),
    ("ubo_light_16dir", TEXT_UBO),
    ("values_random79", TEXT_VALUES),
];

const TEXT_FP16: &str = "Valhall: a constant fp16 does not hold (past its normal range, or a highp colour or point size constant not exactly) is not lowered yet";
const TEXT_FTL: &str =
    "Valhall: a fragment colour of more than 255 vectorised operations and operands is not lowered yet";
const TEXT_LANECYCLE: &str = "Valhall back end defect: an invariant of the pipeline did not hold (LaneCycle)";
const TEXT_PREC: &str = "Valhall: a value whose precision class the front end does not know (a folded constant a full-width built-in reads, a constant shared at two widths) is not lowered yet";
const TEXT_TEXHIGHP: &str = "Valhall: a highp sampler (Mesa keeps its look-up 32-bit) is not lowered yet";
const TEXT_TEXLANES: &str =
    "Valhall: a texture result's lanes other than its own pairs in order (a swizzle, a lane alone) is not lowered yet";
const TEXT_UBO: &str = "Valhall: a uniform past the FAU's 128 words (Mesa loads it from the UBO) is not lowered yet";
const TEXT_VALUES: &str =
    "Valhall: a shader of more than 2048 IR values (or 1536 instructions before selection) is not lowered yet";

/// The words of each reference's `fs.fragment` region (Mesa's fragment
/// program, the tail and the blend branch included).
const FS_WORDS: &[(&str, usize)] = &[
    ("fp16_sub_highp_mat3", 7),
    ("ftl_nested_initial", 152),
    ("ftl_random20", 153),
    ("ftl_struct_in_array_both", 131),
    ("ftl_struct_in_array_fs", 128),
    ("lanecycle_tex83", 10),
    ("link_bexpr53", 7),
    ("link_bexpr85", 12),
    ("link_bexpr9", 11),
    ("link_cmp17", 9),
    ("link_cmp41", 11),
    ("link_cmp45", 9),
    ("link_cond41", 9),
    ("link_cond47", 9),
    ("link_conv17", 9),
    ("link_conv32", 8),
    ("link_conv53", 7),
    ("link_seq_side_highp_vec4", 25),
    ("link_swz17", 9),
    ("link_tex4", 19),
    ("precision_trig51", 52),
    ("texhighp_sampler_value", 11),
    ("texlanes_struct_sampler_nested", 20),
    ("texlanes_tex44", 11),
    ("texlanes_tex55", 11),
    ("ubo_light_10omni", 8),
    ("ubo_light_12omni", 8),
    ("ubo_light_14omni", 8),
    ("ubo_light_16dir", 8),
    ("values_random79", 825),
];

/// The reference, with `uniform_values` as it was drawn ([`ZERO_UNIFORMS`]).
fn load(name: &str) -> mali_ref::Reference {
    let r = mali_ref::Reference::load(&format!("deqp_newtext_{name}"));
    mali_ref::Reference { uniform_values: !ZERO_UNIFORMS.contains(&name), ..r }
}

/// Every difference of the compiled pair against the reference; gl_ref.py
/// set the struct members of every `--uniform-values` draw here.
fn differences(r: &mali_ref::Reference, pair: &mali_ref::Pair) -> Vec<String> {
    mali_ref::differences_members(r, pair, true)
}

fn assert_matches(name: &str) {
    let r = load(name);
    let pair = mali_ref::compile_pair(&r.vs, &r.fs).unwrap_or_else(|e| panic!("{name}: the pair is refused: {e}"));
    let d = differences(&r, &pair);
    assert!(d.is_empty(), "{name}: differs from its reference:\n{}", d.join("\n"));
}

/// `shaders.matrix.sub.const_highp_mat3_mat3_vertex`: Mesa's link moves the
/// constant `v_color` into the fragment shader; its x is a sum below fp16's
/// normal range, which Mesa makes -0.0 (`0x8000`).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn fp16_sub_highp_mat3() {
    assert_matches("fp16_sub_highp_mat3");
}

/// `uniform_api.value.initial.*.nested_structs_arrays.mat4_mat2_fragment`:
/// compares of nested struct arrays of mat4 and mat2, more pair nodes than 255.
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn ftl_nested_initial() {
    assert_matches("ftl_nested_initial");
}

/// `uniform_api.random.20`: a fragment colour of more than 255 pair nodes.
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn ftl_random20() {
    assert_matches("ftl_random20");
}

/// `uniform_api.*.struct_in_array.mat4_mat2_both`: the compares in both stages.
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn ftl_struct_in_array_both() {
    assert_matches("ftl_struct_in_array_both");
}

/// `uniform_api.*.struct_in_array.mat4_mat2_fragment`.
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn ftl_struct_in_array_fs() {
    assert_matches("ftl_struct_in_array_fs");
}

/// `shaders.random.texture.fragment.83`: the `lane_tree`/`vectorise` cycle
/// (fix C of `docs/notes/mali-deqp-defects.md`) beside a look-up.
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn lanecycle_tex83() {
    assert_matches("lanecycle_tex83");
}

/// `shaders.random.basic_expression.fragment.53`: the vertex outputs the
/// fragment code never reads (an overwritten colour) are dropped by Mesa's link.
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn link_bexpr53() {
    assert_matches("link_bexpr53");
}

/// `shaders.random.basic_expression.fragment.85`.
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn link_bexpr85() {
    assert_matches("link_bexpr85");
}

/// `shaders.random.basic_expression.fragment.9`: seven varyings, only `f` read.
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn link_bexpr9() {
    assert_matches("link_bexpr9");
}

/// `shaders.random.comparison_ops.fragment.17`: `e` dead, `c` at offset 0.
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn link_cmp17() {
    assert_matches("link_cmp17");
}

/// `shaders.random.comparison_ops.fragment.41`.
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn link_cmp41() {
    assert_matches("link_cmp41");
}

/// `shaders.random.comparison_ops.fragment.45`.
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn link_cmp45() {
    assert_matches("link_cmp45");
}

/// `shaders.random.conditionals.fragment.41`.
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn link_cond41() {
    assert_matches("link_cond41");
}

/// `shaders.random.conditionals.fragment.47`.
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn link_cond47() {
    assert_matches("link_cond47");
}

/// `shaders.random.scalar_conversion.fragment.17`.
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn link_conv17() {
    assert_matches("link_conv17");
}

/// `shaders.random.scalar_conversion.fragment.32`.
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn link_conv32() {
    assert_matches("link_conv32");
}

/// `shaders.random.scalar_conversion.fragment.53`.
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn link_conv53() {
    assert_matches("link_conv53");
}

/// `shaders.operator.sequence.side_effects.highp_vec4_fragment`: `v_in0` is
/// not read, `v_in1` and `v_in2` at offsets 0 and 16.
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn link_seq_side_highp_vec4() {
    assert_matches("link_seq_side_highp_vec4");
}

/// `shaders.random.swizzle.fragment.17`.
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn link_swz17() {
    assert_matches("link_swz17");
}

/// `shaders.random.texture.fragment.4`.
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn link_tex4() {
    assert_matches("link_tex4");
}

/// `shaders.random.trigonometric.fragment.51`: `asin` of folded constants
/// and a varying sum.
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn precision_trig51() {
    assert_matches("precision_trig51");
}

/// `state_query.shader.uniform_value_sampler`: `vec4(texture2D(highp s, ..).x)`;
/// Mesa's look-up here is 16-bit (`TEX_SINGLE ... .16.r`).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn texhighp_sampler_value() {
    assert_matches("texhighp_sampler_value");
}

/// `shaders.struct.uniform.sampler_nested_fragment`: a swizzled look-up of a
/// sampler in a nested struct.
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn texlanes_struct_sampler_nested() {
    assert_matches("texlanes_struct_sampler_nested");
}

/// `shaders.random.texture.fragment.44`: `texture2D(a, b).qspt`, Mesa's
/// `MKVEC.v2i16` of the halves.
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn texlanes_tex44() {
    assert_matches("texlanes_tex44");
}

/// `shaders.random.texture.fragment.55`.
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn texlanes_tex55() {
    assert_matches("texlanes_tex55");
}

/// `light_amount.10omni`: past the FAU, Mesa loads from UBO 0 (`LD_PKA`).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn ubo_light_10omni() {
    assert_matches("ubo_light_10omni");
}

/// `light_amount.12omni`.
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn ubo_light_12omni() {
    assert_matches("ubo_light_12omni");
}

/// `light_amount.14omni`.
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn ubo_light_14omni() {
    assert_matches("ubo_light_14omni");
}

/// `light_amount.16dir`.
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn ubo_light_16dir() {
    assert_matches("ubo_light_16dir");
}

/// `uniform_api.random.79`: a vertex shader of more than 2048 IR values.
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn values_random79() {
    assert_matches("values_random79");
}

fn on_disk() -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir("tools/mali/testdata")
        .expect("tools/mali/testdata")
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter_map(|n| n.strip_prefix("deqp_newtext_")?.strip_suffix(".expect.json").map(str::to_string))
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

/// Every reference on disk parses: both shaders and their `.license` are
/// there and not empty, the trace is the reference's own (`deqp_ref.sh`
/// names it after the prefix), `vs.position` and `fs.fragment` have 64-bit
/// hex words, and `fs.fragment` has the count of [`FS_WORDS`].
#[test]
fn references_parse() {
    let names = on_disk();
    let mut listed: Vec<&str> = FS_WORDS.iter().map(|(n, _)| *n).collect();
    listed.sort_unstable();
    assert_eq!(names, listed, "FS_WORDS does not list every deqp_newtext_* reference");
    let mut bad = Vec::new();
    for (name, fs_words) in FS_WORDS {
        let r = load(name);
        if !r.vs.contains("void main") || !r.fs.contains("void main") {
            bad.push(format!("{name}: a shader has no main"));
        }
        for ext in ["vert", "frag"] {
            let p = format!("tools/mali/testdata/deqp_newtext_{name}.{ext}.license");
            match std::fs::read_to_string(&p) {
                Ok(t) if t.starts_with("SPDX-License-Identifier: Apache-2.0\n") => {}
                _ => bad.push(format!("{name}: {p} is missing or has no SPDX line")),
            }
        }
        if !r.json.contains(&format!("\"trace\": \"deqp_newtext_{name}.trace\"")) {
            bad.push(format!("{name}: the trace is not deqp_newtext_{name}.trace"));
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
/// (the whole message, not a substring), each of [`DIFFERS`] compiles and
/// differs, each of [`LOWERED`] is byte-equal, and the three lists cover
/// every `deqp_newtext_*` reference on disk.
#[test]
fn refusals_pinned() {
    let on_disk = on_disk();
    let mut listed: Vec<&str> = LOWERED.iter().chain(DIFFERS).copied().chain(REFUSED.iter().map(|(n, _)| *n)).collect();
    listed.sort_unstable();
    assert_eq!(on_disk, listed, "a deqp_newtext_* reference has no outcome here");
    let mut changed = Vec::new();
    for (name, refused) in REFUSED {
        let r = load(name);
        match mali_ref::compile_pair(&r.vs, &r.fs) {
            Ok(pair) => {
                let to = if differences(&r, &pair).is_empty() { "LOWERED" } else { "DIFFERS" };
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
        let r = load(name);
        match mali_ref::compile_pair(&r.vs, &r.fs) {
            Ok(pair) => {
                if differences(&r, &pair).is_empty() {
                    changed.push(format!("{name}: now byte-equal; move it to LOWERED"));
                }
            }
            Err(e) => changed.push(format!("{name}: now refused: {e}")),
        }
    }
    for name in LOWERED {
        let r = load(name);
        match mali_ref::compile_pair(&r.vs, &r.fs) {
            Ok(pair) => {
                let d = differences(&r, &pair);
                if !d.is_empty() {
                    changed.push(format!("{name}: no longer byte-equal: {}", d[0]));
                }
            }
            Err(e) => changed.push(format!("{name}: now refused: {e}")),
        }
    }
    assert!(changed.is_empty(), "{}", changed.join("\n"));
}
