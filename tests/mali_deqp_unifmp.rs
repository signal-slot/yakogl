// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Byte-compare of the Mali online compiler against the Mesa 26.2.1
//! references `tools/mali/testdata/deqp_unifmp_*` (plan #387, item
//! unifmp-design; `docs/notes/mali-unifmp-lowering.md`): the mediump
//! fragment-shader uniform refusal (`MaliError::FsUniform`), 472 console
//! lines of the Verdin iMX95 (Mali-G310) dEQP-GLES2 tree. Each pair is a
//! program of a case that refuses with this text first at the tip 8133663c
//! (HOST-ONLY, `tools/mali/deqp-host-measure.sh`), at one of the refusal
//! sites that no earlier reference (`deqp_fsunif*`, `deqp_unif2_fs_*`,
//! `deqp_uniarray_api_float`, `deqp_misc2_invariant_cse`,
//! `deqp_intval3_rsg_swizzle12_fs`, the linkage struct references) draws, or
//! a matrix or random-expression form of a site they draw in another form.
//! Every reference was drawn with `tools/mali/deqp_ref.sh --instrument shim`
//! (Mesa 26.2.1's G610 drm-shim, `--uniform-values`; `struct_sampler` with
//! `--zero-uniforms`, a struct with a sampler member). The G310's words are
//! the G610's (`docs/notes/mali-g310-vs-g610.md`).
//!
//! A reference the compiler refuses is `#[ignore]`d and `refusals_pinned`
//! holds the whole refusal text ([`REFUSED`]). The lowering step that lifts
//! one moves it to [`LOWERED`] (byte-equal) or [`DIFFERS`] (compiles with
//! other words than Mesa's) in the same commit, drops its `#[ignore]` and
//! updates the note. `references_parse` checks every file and the fragment
//! word count the note quotes ([`FS_WORDS`]).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_deqp_unifmp
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-mali"))]
mod mali_ref;

/// The references a lowering step made byte-equal, without the
/// `deqp_unifmp_` prefix.
/// `rsg_sconv12` since plan #377 spike-uniform-rest3: two float uniforms'
/// halves of two words as one `MKVEC.v2i16` (nir_fs.rs `vectorise`).
/// `interaction_16` since plan #387 mali-train-3a (unifmp U2): one uniform
/// half beside a varying's or a computed lane as a word of the colour
/// (nir_fs.rs `vectorise`).
/// `interaction_88` since plan #377 spike-fsunif-mp: one int uniform's
/// float in both halves of a colour word (nir_fs.rs `Leaf::IntSplat`).
const LOWERED: &[&str] = &["rsg_sconv12", "interaction_16", "interaction_88"];

/// The references that compile with other words than Mesa's (hazards).
/// `inv_mediump_cse2` since unifmp U2: its fragment program is byte-equal
/// (`inv_mediump_cse2_fragment`), its vertex shader is not. That shader
/// compiled before the step too (the program stopped at the fragment
/// shader): its mediump arithmetic on a mediump attribute is built at fp32
/// (`FADD.f32 r4, r0, r1`), Mesa's at fp16 (`V2F32_TO_V2F16` of the
/// attribute, then `FADD.v2f16`, `FMA.v2f16`).
const DIFFERS: &[&str] = &["inv_mediump_cse2"];

/// Every ignored reference with its whole `MaliError` message.
const REFUSED: &[(&str, &str)] = &[
    ("highp_mat3_mul", TEXT_FSU),
    ("highp_vec3_mat3", TEXT_FSU),
    ("mat4_sub_mat4", TEXT_FSU),
    ("rsg_expr10", TEXT_FSU),
    ("struct_array", TEXT_FSU),
    ("struct_nested", TEXT_FSU),
    ("struct_sampler", TEXT_FSU),
    ("vec4_mat4_mul", TEXT_FSU),
];

/// The references whose struct members gl_ref.py set (`--uniform-values`):
/// their FAU words are compared with the members' values
/// (`mali_ref::differences_members(.., true)`; none is in
/// `mali_ref::STRUCT_VALUES`).
const MEMBERS_SET: &[&str] = &["struct_array", "struct_nested"];

const TEXT_FSU: &str = "Valhall: a mediump fragment shader uniform other than a float, vec2, vec3, vec4, vec4 array or int read in its own fp16 lane pairs or as one lane, or one declared highp, is not lowered yet";

/// The words of each reference's `fs.fragment` region (Mesa's fragment
/// program, the tail and the blend branch included).
const FS_WORDS: &[(&str, usize)] = &[
    ("highp_mat3_mul", 48),
    ("highp_vec3_mat3", 21),
    ("interaction_16", 11),
    ("interaction_88", 8),
    ("inv_mediump_cse2", 15),
    ("mat4_sub_mat4", 23),
    ("rsg_expr10", 18),
    ("rsg_sconv12", 7),
    ("struct_array", 14),
    ("struct_nested", 13),
    ("struct_sampler", 18),
    ("vec4_mat4_mul", 27),
];

/// The pair compiles and is byte-equal to its reference (members compared
/// with their values for [`MEMBERS_SET`]).
fn assert_matches(name: &str) {
    let full = format!("deqp_unifmp_{name}");
    if !MEMBERS_SET.contains(&name) {
        return mali_ref::assert_matches(&full);
    }
    let r = mali_ref::Reference::load(&full);
    let pair = mali_ref::compile_pair(&r.vs, &r.fs).unwrap_or_else(|e| panic!("{full}: the pair is refused: {e}"));
    let d = mali_ref::differences_members(&r, &pair, true);
    assert!(d.is_empty(), "{full}: differs from its reference:\n{}", d.join("\n"));
}

/// `shaders.matrix.mul.uniform_highp_mat3_mat3_fragment`: two `highp mat3`
/// uniforms multiplied, the colour mediump (fp32 words, a mat3's padded
/// columns; site `nir_fs.rs:10318`).
#[test]
#[ignore = "refused (FsUniform: two fp32 uniform words not consecutive); pinned by refusals_pinned"]
fn highp_mat3_mul() {
    assert_matches("highp_mat3_mul");
}

/// `shaders.matrix.mul.uniform_highp_vec3_mat3_fragment`: a highp varying
/// times a `highp mat3` (fp32 FMA chains, each lane narrowed alone).
#[test]
#[ignore = "refused (FsUniform: two fp32 uniform words not consecutive); pinned by refusals_pinned"]
fn highp_vec3_mat3() {
    assert_matches("highp_vec3_mat3");
}

/// `fragment_ops.interaction.basic_shader.16`: `vec4(a, bool(d), float(d),
/// float(a))` of a `uniform mediump float a` (one half beside a computed
/// lane; site `nir_fs.rs:10805`).
#[test]
fn interaction_16() {
    assert_matches("interaction_16");
}

/// `fragment_ops.interaction.basic_shader.88`: `vec4(a, 0, b, a).gbar` of a
/// `uniform mediump int a` (the int's float in both lanes of the colour's
/// second word; site `nir_fs.rs:10360`).
#[test]
fn interaction_88() {
    assert_matches("interaction_88");
}

/// `shaders.invariance.mediump.common_subexpression_2`: `vec4(u_color.r,
/// u_color.g, blue, u_color.a)` (a uniform half beside a computed lane).
#[test]
#[ignore = "compiles, not Mesa's: the vertex shader's mediump arithmetic is fp32 here, fp16 in Mesa (DIFFERS; the fragment program is byte-equal, inv_mediump_cse2_fragment)"]
fn inv_mediump_cse2() {
    assert_matches("inv_mediump_cse2");
}

/// `inv_mediump_cse2`'s fragment program alone is byte-equal: the uniform's
/// aligned pair as the first word, `MKVEC.v2i16 r2, r0^.h0, u2.w1.h1` of
/// the sum and the uniform's high half as the second, ATEST reading
/// `u2.w1.h1` (the vertex regions differ, [`DIFFERS`]).
#[test]
fn inv_mediump_cse2_fragment() {
    let r = mali_ref::Reference::load("deqp_unifmp_inv_mediump_cse2");
    let pair = mali_ref::compile_pair(&r.vs, &r.fs).unwrap_or_else(|e| panic!("refused: {e}"));
    let d: Vec<String> = mali_ref::differences(&r, &pair).into_iter().filter(|d| d.starts_with("fs")).collect();
    assert!(d.is_empty(), "the fragment program differs from deqp_unifmp_inv_mediump_cse2:\n{}", d.join("\n"));
}

/// `shaders.matrix.sub.uniform_mediump_mat4_mat4_fragment`: two mediump
/// `mat4`s subtracted, lanes recombined across the fp16 pairs (site
/// `nir_fs.rs:10792`).
#[test]
#[ignore = "refused (FsUniform: two fp16 uniform halves not an aligned pair); pinned by refusals_pinned"]
fn mat4_sub_mat4() {
    assert_matches("mat4_sub_mat4");
}

/// `shaders.random.basic_expression.fragment.10`: a mediump float uniform's
/// half beside a constant's and a varying's (`MKVEC.v2i16 u2.w0.h0, u2.w1.h0`).
#[test]
#[ignore = "refused (FsUniform: a uniform half beside a computed lane); pinned by refusals_pinned"]
fn rsg_expr10() {
    assert_matches("rsg_expr10");
}

/// `shaders.random.scalar_conversion.fragment.12`: `vec4(c, b, 0, a)` of two
/// mediump float uniforms in swapped order (`MKVEC.v2i16 u2.w1.h0, u2.w0.h0`).
#[test]
fn rsg_sconv12() {
    assert_matches("rsg_sconv12");
}

/// `shaders.struct.uniform.struct_array_fragment`: `uniform S s[3]` of
/// `{ mediump float a; mediump int b; }` (fp32 member words narrowed alone).
#[test]
#[ignore = "refused (FsUniform: two fp32 uniform words not consecutive); pinned by refusals_pinned"]
fn struct_array() {
    assert_matches("struct_array");
}

/// `shaders.struct.uniform.nested_fragment`: a struct with a nested struct
/// member (`int`, `vec2`) and an `int`.
#[test]
#[ignore = "refused (FsUniform: two fp32 uniform words not consecutive); pinned by refusals_pinned"]
fn struct_nested() {
    assert_matches("struct_nested");
}

/// `shaders.struct.uniform.sampler_fragment`: a struct with a `sampler2D`
/// member; every uniform word is 0 (`--zero-uniforms`; site
/// `nir_fs.rs:10335`).
#[test]
#[ignore = "refused (FsUniform: an fp32 uniform word beside a lane that is no fp32 word); pinned by refusals_pinned"]
fn struct_sampler() {
    assert_matches("struct_sampler");
}

/// `shaders.matrix.mul.uniform_mediump_vec4_mat4_fragment`: a varying times
/// a mediump `mat4` (`FMA.v2f16` of single halves `.h00`/`.h11`).
#[test]
#[ignore = "refused (FsUniform: two fp16 uniform halves not an aligned pair); pinned by refusals_pinned"]
fn vec4_mat4_mul() {
    assert_matches("vec4_mat4_mul");
}

fn on_disk() -> Vec<String> {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tools/mali/testdata");
    let mut v: Vec<String> = std::fs::read_dir(dir)
        .expect("tools/mali/testdata")
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter_map(|n| n.strip_prefix("deqp_unifmp_")?.strip_suffix(".expect.json").map(str::to_string))
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

/// Every reference on disk loads and parses: both shaders are there with a
/// `main`, both have a `.license`, the trace is the reference's own
/// (`deqp_ref.sh` names it after the prefix), `vs.position` and
/// `fs.fragment` have 64-bit hex words, and `fs.fragment` has the count of
/// [`FS_WORDS`].
#[test]
fn references_parse() {
    let names = on_disk();
    let mut listed: Vec<&str> = FS_WORDS.iter().map(|(n, _)| *n).collect();
    listed.sort_unstable();
    assert_eq!(names, listed, "FS_WORDS does not list every deqp_unifmp_* reference");
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tools/mali/testdata");
    let mut bad = Vec::new();
    for (name, fs_words) in FS_WORDS {
        let r = mali_ref::Reference::load(&format!("deqp_unifmp_{name}"));
        if !r.vs.contains("void main") || !r.fs.contains("void main") {
            bad.push(format!("{name}: a shader has no main"));
        }
        for ext in ["vert", "frag"] {
            if !dir.join(format!("deqp_unifmp_{name}.{ext}.license")).is_file() {
                bad.push(format!("{name}: no .{ext}.license"));
            }
        }
        if !r.json.contains(&format!("\"trace\": \"deqp_unifmp_{name}.trace\"")) {
            bad.push(format!("{name}: the trace is not deqp_unifmp_{name}.trace"));
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

/// The differences of a compiled pair from its reference, members compared
/// with their values for [`MEMBERS_SET`].
fn differences(name: &str, r: &mali_ref::Reference, pair: &mali_ref::Pair) -> Vec<String> {
    if MEMBERS_SET.contains(&name) {
        mali_ref::differences_members(r, pair, true)
    } else {
        mali_ref::differences(r, pair)
    }
}

/// Each refused reference is refused with exactly its text in [`REFUSED`]
/// (the whole `MaliError` message, not a substring), each of [`DIFFERS`]
/// compiles and differs, each of [`LOWERED`] is byte-equal, and the three
/// lists cover every `deqp_unifmp_*` reference on disk.
#[test]
fn refusals_pinned() {
    let on_disk = on_disk();
    let mut listed: Vec<&str> = LOWERED.iter().chain(DIFFERS).copied().chain(REFUSED.iter().map(|(n, _)| *n)).collect();
    listed.sort_unstable();
    assert_eq!(on_disk, listed, "a deqp_unifmp_* reference has no outcome here");
    let mut changed = Vec::new();
    for (name, refused) in REFUSED {
        let r = mali_ref::Reference::load(&format!("deqp_unifmp_{name}"));
        match mali_ref::compile_pair(&r.vs, &r.fs) {
            Ok(pair) => {
                let to = if differences(name, &r, &pair).is_empty() { "LOWERED" } else { "DIFFERS" };
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
        let r = mali_ref::Reference::load(&format!("deqp_unifmp_{name}"));
        match mali_ref::compile_pair(&r.vs, &r.fs) {
            Ok(pair) => {
                if differences(name, &r, &pair).is_empty() {
                    changed.push(format!("{name}: now byte-equal; move it to LOWERED"));
                }
            }
            Err(e) => changed.push(format!("{name}: now refused: {e}")),
        }
    }
    for name in LOWERED {
        let r = mali_ref::Reference::load(&format!("deqp_unifmp_{name}"));
        match mali_ref::compile_pair(&r.vs, &r.fs) {
            Ok(pair) => {
                let d = differences(name, &r, &pair);
                if !d.is_empty() {
                    changed.push(format!("{name}: no longer byte-equal: {}", d[0]));
                }
            }
            Err(e) => changed.push(format!("{name}: now refused: {e}")),
        }
    }
    assert!(changed.is_empty(), "{}", changed.join("\n"));
}
