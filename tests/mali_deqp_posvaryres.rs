// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Byte-compare of the Mali online compiler against the Mesa 26.2.1
//! references `tools/mali/testdata/deqp_posvaryres_*` (plan #387, item
//! posvaryres-refs, `docs/notes/mali-posvaryres-lowering.md`): the
//! dEQP-GLES2 programs whose first refusal at the integration tip
//! `349f989f` is one of the three vertex varying texts (HOST-ONLY,
//! `tools/mali/deqp-host-measure.sh`, 90 cases):
//!
//! * `a varying lane left unwritten` (`VaryingForm`, 52 cases): a store at
//!   a run-time index into a local or a varying array (vsunif V2), and a
//!   store at a loop counter's index (`rgb[i] = ..`, step PV2);
//! * `a varying lane computed from no attribute` (`VaryingNotFromAttribute`,
//!   35 cases): the vertex shader compiled alone, with no fragment reads,
//!   refused for a layout only the link decides (step PV1), a convergent
//!   lane before an interpolated one in the linked layout (PV4), more than
//!   16 constant lanes (PV5) and the loop phi of posvary P5;
//! * `a gl_Position lane left unwritten` (`PositionLane`, 3 cases): the
//!   run-time-index store read by a `dot` (PV3, with V2).
//!
//! Each reference is the case's refusing program verbatim from the .qpa,
//! drawn with `tools/mali/deqp_ref.sh <vert> <frag> <prefix> --instrument
//! shim` (`--uniform-values`); its `.nir.sha` is from `tools/mali/nir_refs.sh`,
//! which redrew every pair and found the same words. The case of each is in
//! its `.vert.license`; the note's section 3 maps every one of the 90 cases to
//! a step and a reference.
//!
//! A reference the compiler refuses is `#[ignore]`d and `refusals_pinned`
//! holds the whole refusal text ([`REFUSED`]). The lowering step that lifts
//! one moves it to [`LOWERED`] (byte-equal) or [`DIFFERS`] (compiles with
//! other words than Mesa's) in the same commit, drops its `#[ignore]` and
//! updates the note. `references_parse` checks every file and the fragment
//! word count the note quotes ([`FS_WORDS`]). The two struct-uniform
//! references ([`STRUCT`]) are compared with gl_ref.py's member values.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_deqp_posvaryres
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-mali"))]
mod mali_ref;

/// The references a lowering step made byte-equal, without the
/// `deqp_posvaryres_` prefix.
const LOWERED: &[&str] = &[];

/// The references that compile with other words than Mesa's (hazards).
const DIFFERS: &[&str] = &[];

/// Every ignored reference with its whole `MaliError` message, and the
/// step of the note that lifts it.
const REFUSED: &[(&str, &str)] = &[
    /* PV1: refused by the vertex shader compiled alone; the linked
     * compile is byte-equal at the tip (note section 4.1) */
    ("alone_bs19", NO_ATTR),
    ("alone_bs27", NO_ATTR),
    ("alone_bs60", NO_ATTR),
    ("alone_c53", NO_ATTR),
    ("alone_s87", NO_ATTR),
    ("constflat_s15", NO_ATTR),
    ("fill_three_vec4", NO_ATTR),
    /* PV1, then a fragment text of its own (note section 4.1) */
    ("constflat_c70", NO_ATTR),
    ("fill_flat_after_vec4", NO_ATTR),
    ("fill_two_vec4", NO_ATTR),
    ("lane_fill_flat_two_vec4", NO_ATTR),
    ("lane_fill_two_vec4", NO_ATTR),
    /* PV5: 19 constant lanes, past link::MAX_CONSTS */
    ("consts_full_s37", NO_ATTR),
    /* V2 (vsunif, train 3b): a store at a run-time index */
    ("mat4_dynw", VARYING_LANE),
    ("tmp_float_dynw", VARYING_LANE),
    ("tmp_vec4_dynw_loop", VARYING_LANE),
    ("vary_float_dynw", VARYING_LANE),
    ("vec4_dynw_dynr", VARYING_LANE),
    /* PV2: a store at a loop counter's index */
    ("struct_local_loop", VARYING_LANE),
    ("struct_unif_dynloop", VARYING_LANE),
    ("struct_unif_loop", VARYING_LANE),
    /* PV3: V2's store read by a dot, the undef operand */
    ("vec2_dynw_dot", POSITION_LANE),
    ("vec3_dynw_dot", POSITION_LANE),
    ("vec4_dynw_dot", POSITION_LANE),
    /* PV4: a convergent lane before an interpolated one, linked */
    ("vlane_cmp33", NO_ATTR),
    ("vlane_swz27", NO_ATTR),
    ("vlane_v18", NO_ATTR),
    ("vlane_v27", NO_ATTR),
    ("vlane_v46", NO_ATTR),
    ("vlane_v89", NO_ATTR),
];

const NO_ATTR: &str = "Valhall: a varying lane computed from no attribute (a constant or a uniform alone, which Mesa's link moves or rewrites) is not lowered yet";
const VARYING_LANE: &str = "Valhall: a varying lane left unwritten is not lowered yet";
const POSITION_LANE: &str = "Valhall: a gl_Position lane left unwritten is not lowered yet";

/// The references with a struct uniform, compared with its members set.
const STRUCT: &[&str] = &["struct_unif_dynloop", "struct_unif_loop"];

/// The words of each reference's `fs.fragment` region (Mesa's fragment
/// program, the tail and the blend branch included).
const FS_WORDS: &[(&str, usize)] = &[
    ("alone_bs19", 7),
    ("alone_bs27", 7),
    ("alone_bs60", 7),
    ("alone_c53", 7),
    ("alone_s87", 7),
    ("constflat_c70", 7),
    ("constflat_s15", 10),
    ("consts_full_s37", 7),
    ("fill_flat_after_vec4", 17),
    ("fill_three_vec4", 7),
    ("fill_two_vec4", 10),
    ("lane_fill_flat_two_vec4", 7),
    ("lane_fill_two_vec4", 10),
    ("mat4_dynw", 7),
    ("struct_local_loop", 7),
    ("struct_unif_dynloop", 7),
    ("struct_unif_loop", 7),
    ("tmp_float_dynw", 7),
    ("tmp_vec4_dynw_loop", 7),
    ("vary_float_dynw", 12),
    ("vec2_dynw_dot", 8),
    ("vec3_dynw_dot", 8),
    ("vec4_dynw_dot", 8),
    ("vec4_dynw_dynr", 8),
    ("vlane_cmp33", 11),
    ("vlane_swz27", 8),
    ("vlane_v18", 10),
    ("vlane_v27", 10),
    ("vlane_v46", 8),
    ("vlane_v89", 11),
];

fn differences(name: &str) -> Result<Vec<String>, String> {
    let r = mali_ref::Reference::load(&format!("deqp_posvaryres_{name}"));
    let pair = mali_ref::compile_pair(&r.vs, &r.fs)?;
    Ok(mali_ref::differences_members(&r, &pair, STRUCT.contains(&name)))
}

fn assert_matches(name: &str) {
    match differences(name) {
        Ok(d) => assert!(d.is_empty(), "deqp_posvaryres_{name}:\n{}", d.join("\n")),
        Err(e) => panic!("deqp_posvaryres_{name}: refused: {e}"),
    }
}

macro_rules! refused_ref {
    ($($(#[$doc:meta])* $name:ident, $why:literal;)*) => {$(
        $(#[$doc])*
        #[test]
        #[ignore = $why]
        fn $name() {
            assert_matches(stringify!($name));
        }
    )*};
}

refused_ref! {
    /// `fragment_ops.interaction.basic_shader.19`: Mesa stores one smooth
    /// vec4 and moves the uniform lanes the fragment shader reads (PV1).
    alone_bs19, "refused: the vertex shader alone (PV1)";
    /// `fragment_ops.interaction.basic_shader.27` (PV1).
    alone_bs27, "refused: the vertex shader alone (PV1)";
    /// `fragment_ops.interaction.basic_shader.60` (PV1).
    alone_bs60, "refused: the vertex shader alone (PV1)";
    /// `shaders.random.basic_expression.combined.53`: 8 interpolated and 4
    /// convergent lanes declared, a constant colour in Mesa (PV1).
    alone_c53, "refused: the vertex shader alone (PV1)";
    /// `shaders.random.scalar_conversion.combined.87` (PV1).
    alone_s87, "refused: the vertex shader alone (PV1)";
    /// `shaders.random.scalar_conversion.combined.15`: a constant lane kept
    /// flat beside an interpolated one, alone (PV1).
    constflat_s15, "refused: the vertex shader alone, a constant kept flat (PV1)";
    /// `fragment_ops.interaction.basic_shader.29`: nine interpolated lanes,
    /// the colour constant in Mesa (PV1).
    fill_three_vec4, "refused: the vertex shader alone (PV1)";
    /// `shaders.random.basic_expression.combined.70`: PV1, then a constant
    /// product in the fragment shader.
    constflat_c70, "refused: the vertex shader alone, a constant kept flat (PV1)";
    /// `shaders.random.basic_expression.combined.74`: PV1, then the
    /// fragment shader's mediump uniform.
    fill_flat_after_vec4, "refused: the vertex shader alone (PV1)";
    /// `shaders.random.basic_expression.combined.12`: PV1, then the
    /// fragment shader's mediump uniform; Mesa keeps one flat lane.
    fill_two_vec4, "refused: the vertex shader alone (PV1)";
    /// `fragment_ops.interaction.basic_shader.23`: PV1, then fragment
    /// arithmetic.
    lane_fill_flat_two_vec4, "refused: the vertex shader alone (PV1)";
    /// `shaders.random.scalar_conversion.combined.51`: PV1, then the
    /// linked fragment operand form.
    lane_fill_two_vec4, "refused: the vertex shader alone (PV1)";
    /// `shaders.random.scalar_conversion.combined.37`: 19 constant lanes,
    /// all propagated by Mesa (no varying program) (PV5).
    consts_full_s37, "refused: more constant lanes than the link's table (PV5)";
    /// `shaders.indexing.matrix_subscript.mat4_dynamic_write_static_read_vertex` (V2).
    mat4_dynw, "refused: a store at a run-time index (vsunif V2)";
    /// `shaders.indexing.tmp_array.float_dynamic_write_static_read_vertex` (V2).
    tmp_float_dynw, "refused: a store at a run-time index (vsunif V2)";
    /// `shaders.indexing.tmp_array.vec4_dynamic_write_static_loop_read_vertex` (V2).
    tmp_vec4_dynw_loop, "refused: a store at a run-time index (vsunif V2)";
    /// `shaders.indexing.varying_array.float_dynamic_write_static_read` (V2).
    vary_float_dynw, "refused: a store at a run-time index (vsunif V2)";
    /// `shaders.indexing.vector_subscript.vec4_dynamic_subscript_write_dynamic_subscript_read_vertex` (V2).
    vec4_dynw_dynr, "refused: a store at a run-time index (vsunif V2)";
    /// `shaders.struct.local.loop_struct_array_vertex`: `rgb[i] = ..` in a
    /// constant loop (PV2).
    struct_local_loop, "refused: a store at a loop counter's index (PV2)";
    /// `shaders.struct.uniform.dynamic_loop_struct_array_vertex`: the same
    /// under a uniform bound; Mesa guards three iterations (PV2).
    struct_unif_dynloop, "refused: a store at a loop counter's index (PV2)";
    /// `shaders.struct.uniform.loop_struct_array_vertex` (PV2).
    struct_unif_loop, "refused: a store at a loop counter's index (PV2)";
    /// `shaders.indexing.vector_subscript.vec2_dynamic_subscript_write_direct_read_vertex` (PV3).
    vec2_dynw_dot, "refused: a run-time-index store read by a dot (PV3)";
    /// `shaders.indexing.vector_subscript.vec3_dynamic_subscript_write_direct_read_vertex` (PV3).
    vec3_dynw_dot, "refused: a run-time-index store read by a dot (PV3)";
    /// `shaders.indexing.vector_subscript.vec4_dynamic_subscript_write_direct_read_vertex` (PV3).
    vec4_dynw_dot, "refused: a run-time-index store read by a dot (PV3)";
    /// `shaders.random.comparison_ops.vertex.33` (PV4).
    vlane_cmp33, "refused: a convergent lane before an interpolated one (PV4)";
    /// `shaders.random.swizzle.vertex.27` (PV4).
    vlane_swz27, "refused: a convergent lane before an interpolated one (PV4)";
    /// `shaders.random.basic_expression.vertex.18` (PV4).
    vlane_v18, "refused: a convergent lane before an interpolated one (PV4)";
    /// `shaders.random.basic_expression.vertex.27` (PV4).
    vlane_v27, "refused: a convergent lane before an interpolated one (PV4)";
    /// `shaders.random.basic_expression.vertex.46` (PV4).
    vlane_v46, "refused: a convergent lane before an interpolated one (PV4)";
    /// `shaders.random.basic_expression.vertex.89` (PV4).
    vlane_v89, "refused: a convergent lane before an interpolated one (PV4)";
}

fn on_disk() -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir("tools/mali/testdata")
        .expect("tools/mali/testdata")
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter_map(|n| n.strip_prefix("deqp_posvaryres_")?.strip_suffix(".expect.json").map(str::to_string))
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
/// the trace is the reference's own, `vs.position` and `fs.fragment` have
/// 64-bit hex words (`vs.varying` too when present: a program whose lanes
/// Mesa all propagates has none), and `fs.fragment` has the count of
/// [`FS_WORDS`].
#[test]
fn references_parse() {
    let names = on_disk();
    let mut listed: Vec<&str> = FS_WORDS.iter().map(|(n, _)| *n).collect();
    listed.sort_unstable();
    assert_eq!(names, listed, "FS_WORDS does not list every deqp_posvaryres_* reference");
    let mut bad = Vec::new();
    for (name, fs_words) in FS_WORDS {
        let r = mali_ref::Reference::load(&format!("deqp_posvaryres_{name}"));
        if !r.vs.contains("void main") || !r.fs.contains("void main") {
            bad.push(format!("{name}: a shader has no main"));
        }
        if !r.json.contains(&format!("\"trace\": \"deqp_posvaryres_{name}.trace\"")) {
            bad.push(format!("{name}: the trace is not deqp_posvaryres_{name}.trace"));
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
/// lists cover every `deqp_posvaryres_*` reference on disk.
#[test]
fn refusals_pinned() {
    let on_disk = on_disk();
    let mut listed: Vec<&str> = LOWERED.iter().chain(DIFFERS).copied().chain(REFUSED.iter().map(|(n, _)| *n)).collect();
    listed.sort_unstable();
    assert_eq!(on_disk, listed, "a deqp_posvaryres_* reference has no outcome here");
    let mut changed = Vec::new();
    for (name, refused) in REFUSED {
        match differences(name) {
            Ok(d) => {
                let to = if d.is_empty() { "LOWERED" } else { "DIFFERS" };
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
        match differences(name) {
            Ok(d) if d.is_empty() => changed.push(format!("{name}: now byte-equal; move it to LOWERED")),
            Ok(_) => {}
            Err(e) => changed.push(format!("{name}: now refused: {e}")),
        }
    }
    for name in LOWERED {
        match differences(name) {
            Ok(d) if !d.is_empty() => changed.push(format!("{name}: no longer byte-equal: {}", d[0])),
            Ok(_) => {}
            Err(e) => changed.push(format!("{name}: now refused: {e}")),
        }
    }
    assert!(changed.is_empty(), "{}", changed.join("\n"));
}
