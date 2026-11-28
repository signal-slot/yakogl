// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Byte-compare of the Mali online compiler against the Mesa 26.2.1
//! references `tools/mali/testdata/deqp_vscprod_*` (plan #387, item
//! vsconstprod-design, `docs/notes/mali-vscprod-lowering.md`): the vertex
//! shaders that the constant-product refusal (`MaliError::ArithConstant`)
//! stops at `Nir::alu`'s `is_const_product` guard ("a constant times a
//! product with a constant", constprod step K3). That guard is the first
//! refusal of 173 of the 249 dEQP-GLES2 cases whose first refusal at the tip
//! is this text in a vertex shader (HOST-ONLY measure of the Verdin iMX95
//! run's refused cases). The pairs are verbatim from the `.qpa` of
//! `tools/mali/deqp-host-measure.sh`; every reference was drawn with
//! `tools/mali/deqp_ref.sh --instrument shim` (Mesa 26.2.1's G610 drm-shim,
//! `--uniform-values`). `tests/mali_ref` does the compare.
//!
//! A reference the compiler refuses is `#[ignore]`d and `refusals_pinned`
//! holds the whole refusal text ([`REFUSED`]). The lowering step that lifts
//! one moves it to [`LOWERED`] (byte-equal) or [`DIFFERS`] (compiles with
//! other words than Mesa's) in the same commit, drops its `#[ignore]` and
//! updates the note. `references_parse` checks every file and the word the
//! note quotes for each form ([`MESA_WORDS`]).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_deqp_vscprod
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-mali"))]
mod mali_ref;

/// The references a lowering step made byte-equal, without the
/// `deqp_vscprod_` prefix.
const LOWERED: &[&str] = &[];

/// The references that compile with other words than Mesa's (hazards).
const DIFFERS: &[&str] = &[];

/// Every ignored reference with its whole `MaliError` message.
const REFUSED: &[(&str, &str)] = &[
    ("attrib_mat4", TEXT_CPROD),
    ("degrees_mediump_vec3", TEXT_CPROD),
    ("idx_matrix_mat2_static", TEXT_CPROD),
    ("idx_tmp_float_static", TEXT_CPROD),
    ("idx_tmp_vec3_dynwrite", TEXT_CPROD),
    ("idx_tmp_vec4_staticloop", TEXT_CPROD),
    ("idx_varying_vec2_static", TEXT_CPROD),
    ("idx_vector_vec4_static", TEXT_CPROD),
    ("inv_cse2_mediump", TEXT_CPROD),
    ("inv_loop0_mediump", TEXT_CPROD),
    ("inv_loop3_lowp", TEXT_CPROD),
    ("radians_highp_float", TEXT_CPROD),
    ("rnd_expr24", TEXT_CPROD),
    ("rnd_sconv4", TEXT_CPROD),
];

const TEXT_CPROD: &str = "Valhall: a constant product in a shape Mesa's algebraic rules move or fold (a constant times a product with a constant, a negation of a negation, a constant in a fused multiply-add) is not lowered yet";

/// Per reference: the region, one line of Mesa's disassembly that decides
/// the form (section 2 of the note) and how often it stands there.
const MESA_WORDS: &[(&str, &str, &str, usize)] = &[
    /* `(p.xy + p.zz) / 2.0` of halves: the two 0.5s meet as 0.25, first */
    ("attrib_mat4", "vs.varying", "FMA.f32 r0, 0x34000000.h1, r0^, 0x0.neg", 1),
    /* degrees() then `* 0.04 + 0.50` through a vec4 build: both kept */
    ("degrees_mediump_vec3", "vs.varying", "FMA.v2f16 r1, r1^, u6.w0, 0x38000000.h11", 1),
    /* the second iteration's product is dead: one product left */
    ("idx_matrix_mat2_static", "vs.varying", "FMA.v2f16 r2, r0.h10, 0x38000000.h11, 0x0.neg", 1),
    /* `(c * 0.5) * 0.5` is `0.25 * c`, the folded constant first */
    ("idx_tmp_float_static", "vs.varying", "FMA.v2f16 r1, 0x34000000.h11, r0.h00, r1^.h00", 1),
    ("idx_tmp_vec3_dynwrite", "vs.varying", "FMA.v2f16 r2, 0x34000000.h11, r0, r2^", 1),
    ("idx_tmp_vec4_staticloop", "vs.varying", "FMA.v2f16 r2, 0x34000000.h11, r0, r2^", 1),
    ("idx_varying_vec2_static", "vs.varying", "FMA.v2f16 r1, 0x34000000.h11, r0, 0x0.neg", 1),
    /* the folded pair (0.25, 0.125) as one FAU word, 30003400 */
    ("idx_vector_vec4_static", "vs.varying", "FMA.v2f16 r1, u6.w0, r1^, 0x0.neg", 1),
    /* invariant gl_Position: `c / 1e2 / 1e2` stays two products by 0.01 */
    ("inv_cse2_mediump", "vs.position", "FMA.v2f16 r2, r2^, u3.w1, 0x0.neg", 2),
    /* invariant gl_Position: `value *= 1.9` stays one product per iteration */
    ("inv_loop0_mediump", "vs.position", "FMA.v2f16 r2, r2^, u3.w0, 0x0.neg", 12),
    ("inv_loop3_lowp", "vs.position", "FMA.v2f16 r3, r3^, u3.w0, 0x0.neg", 3),
    /* radians() at fp32, narrowed, then `* 25.0 + 0.50`: both kept */
    ("radians_highp_float", "vs.varying", "FMA.v2f16 r0, r0^, u6.w0, 0x38000000.h11", 1),
    ("rnd_expr24", "vs.varying", "FMA.f32 r0, r0^, 0x38000000.h1, r2^", 1),
    ("rnd_sconv4", "vs.varying", "FMA.v2f16 r1, u7.w1, r2^, r1^", 1),
];

/// `vec4(a_value[1] / 10.0 + 1.0) / 2.0` and its halves summed and halved
/// (`default_vertex_attrib.mat4.vertex_attrib_1f`).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn attrib_mat4() {
    mali_ref::assert_matches("deqp_vscprod_attrib_mat4");
}

/// `degrees(in0) * 0.04 + 0.50` of a mediump vec3
/// (`operator.angle_and_trigonometry.degrees.mediump_vec3_vertex`).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn degrees_mediump_vec3() {
    mali_ref::assert_matches("deqp_vscprod_degrees_mediump_vec3");
}

/// `coords = coords.yzwx * 0.5` over a mat2's two columns
/// (`indexing.matrix_subscript.mat2_static_loop_write_static_read_vertex`).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn idx_matrix_mat2_static() {
    mali_ref::assert_matches("deqp_vscprod_idx_matrix_mat2_static");
}

/// `coords = coords * 0.5` written to a float[4] in a static loop
/// (`indexing.tmp_array.float_static_loop_write_static_read_vertex`).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn idx_tmp_float_static() {
    mali_ref::assert_matches("deqp_vscprod_idx_tmp_float_static");
}

/// The same chain in a loop bounded by a uniform
/// (`indexing.tmp_array.vec3_dynamic_loop_write_static_read_vertex`).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn idx_tmp_vec3_dynwrite() {
    mali_ref::assert_matches("deqp_vscprod_idx_tmp_vec3_dynwrite");
}

/// The chain over vec4 elements, read back in a static loop
/// (`indexing.tmp_array.vec4_static_loop_write_static_loop_read_vertex`).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn idx_tmp_vec4_staticloop() {
    mali_ref::assert_matches("deqp_vscprod_idx_tmp_vec4_staticloop");
}

/// The chain written to a varying vec2 array
/// (`indexing.varying_array.vec2_static_loop_write_static_read`).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn idx_varying_vec2_static() {
    mali_ref::assert_matches("deqp_vscprod_idx_varying_vec2_static");
}

/// `tmp[i] = coords.x; coords = coords.yzwx * 0.5` into a vec4
/// (`indexing.vector_subscript.vec4_static_loop_subscript_write_direct_read_vertex`).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn idx_vector_vec4_static() {
    mali_ref::assert_matches("deqp_vscprod_idx_vector_vec4_static");
}

/// `1.0e2 * (x + y)`, squared, `/ 1.0e2 / 1.0e2`, under `invariant
/// gl_Position` (`invariance.mediump.common_subexpression_2`).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn inv_cse2_mediump() {
    mali_ref::assert_matches("deqp_vscprod_inv_cse2_mediump");
}

/// `value *= 1.9` 13 times under `invariant gl_Position`
/// (`invariance.mediump.loop_0`).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn inv_loop0_mediump() {
    mali_ref::assert_matches("deqp_vscprod_inv_loop0_mediump");
}

/// `value *= 1.1` 6 times, lowp, summed into gl_Position
/// (`invariance.lowp.loop_3`).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn inv_loop3_lowp() {
    mali_ref::assert_matches("deqp_vscprod_inv_loop3_lowp");
}

/// `radians(in0) * 25.0 + 0.50` of a highp float
/// (`operator.angle_and_trigonometry.radians.highp_float_vertex`).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn radians_highp_float() {
    mali_ref::assert_matches("deqp_vscprod_radians_highp_float");
}

/// A random expression with products of constant vectors
/// (`random.basic_expression.vertex.24`).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn rnd_expr24() {
    mali_ref::assert_matches("deqp_vscprod_rnd_expr24");
}

/// A random scalar-conversion shader with products of constant vectors
/// (`random.scalar_conversion.vertex.4`).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn rnd_sconv4() {
    mali_ref::assert_matches("deqp_vscprod_rnd_sconv4");
}

fn on_disk() -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir("tools/mali/testdata")
        .expect("tools/mali/testdata")
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter_map(|n| n.strip_prefix("deqp_vscprod_")?.strip_suffix(".expect.json").map(str::to_string))
        .collect();
    v.sort();
    v
}

/// The text of `region` in an expect.json (up to the next region), or None.
fn region(json: &str, region: &str) -> Option<String> {
    let at = json.find(&format!("\"{region}\": {{"))?;
    let rest = &json[at + 1..];
    let end = rest.find("\n  \"").unwrap_or(rest.len());
    Some(rest[..end].to_string())
}

/// The number of 64-bit words of a region's `words` list; a word that is
/// not 16 hex digits is an error.
fn region_words(text: &str) -> Result<usize, String> {
    let words = match text.find("\"words\": [") {
        Some(w) => &text[w + "\"words\": [".len()..],
        None => return Err("no words".to_string()),
    };
    let list = &words[..words.find(']').unwrap_or(0)];
    let mut n = 0;
    for w in list.split(',').map(|w| w.trim().trim_matches('"')).filter(|w| !w.is_empty()) {
        if w.len() != 16 || !w.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(format!("{w:?} is not a 64-bit hex word"));
        }
        n += 1;
    }
    Ok(n)
}

/// Every reference on disk parses: both shaders are there and not empty,
/// the trace is the reference's own, `vs.position` and `fs.fragment` have
/// 64-bit hex words, and the line [`MESA_WORDS`] names stands in its region
/// as often as it says.
#[test]
fn references_parse() {
    let names = on_disk();
    let mut listed: Vec<&str> = MESA_WORDS.iter().map(|(n, ..)| *n).collect();
    listed.sort_unstable();
    assert_eq!(names, listed, "MESA_WORDS does not list every deqp_vscprod_* reference");
    let mut bad = Vec::new();
    for (name, reg, line, count) in MESA_WORDS {
        let r = mali_ref::Reference::load(&format!("deqp_vscprod_{name}"));
        if !r.vs.contains("void main") || !r.fs.contains("void main") {
            bad.push(format!("{name}: a shader has no main"));
        }
        if !r.json.contains(&format!("\"trace\": \"deqp_vscprod_{name}.trace\"")) {
            bad.push(format!("{name}: the trace is not deqp_vscprod_{name}.trace"));
        }
        for want in ["vs.position", "fs.fragment"] {
            match region(&r.json, want).map(|t| region_words(&t)) {
                None => bad.push(format!("{name}: no {want}")),
                Some(Err(e)) => bad.push(format!("{name}: {want}: {e}")),
                Some(Ok(0)) => bad.push(format!("{name}: {want} has no words")),
                Some(Ok(_)) => {}
            }
        }
        match region(&r.json, reg) {
            None => bad.push(format!("{name}: no {reg}")),
            Some(t) => {
                let n = t.matches(&format!("\"{line}\"")).count();
                if n != *count {
                    bad.push(format!("{name}: {reg} has {line:?} {n} times, MESA_WORDS says {count}"));
                }
            }
        }
    }
    assert!(bad.is_empty(), "{}", bad.join("\n"));
}

/// Each refused reference is refused with exactly its text in [`REFUSED`]
/// (the whole `MaliError` message, not a substring), each of [`DIFFERS`]
/// compiles and differs, each of [`LOWERED`] is byte-equal, and the three
/// lists cover every `deqp_vscprod_*` reference on disk.
#[test]
fn refusals_pinned() {
    let on_disk = on_disk();
    let mut listed: Vec<&str> = LOWERED.iter().chain(DIFFERS).copied().chain(REFUSED.iter().map(|(n, _)| *n)).collect();
    listed.sort_unstable();
    assert_eq!(on_disk, listed, "a deqp_vscprod_* reference has no outcome here");
    let mut changed = Vec::new();
    for (name, refused) in REFUSED {
        let r = mali_ref::Reference::load(&format!("deqp_vscprod_{name}"));
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
        let r = mali_ref::Reference::load(&format!("deqp_vscprod_{name}"));
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
        let r = mali_ref::Reference::load(&format!("deqp_vscprod_{name}"));
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
