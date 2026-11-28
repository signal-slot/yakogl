// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Byte-compare of every `tools/mali/testdata/deqp_vsunif3_*` reference (Mesa
//! 26.2.1 drm-shim, plan #402 item `ref-vsunif3`,
//! `docs/notes/mali-deqp-vsunif3.md`) against the Mali online compiler: the
//! vertex-shader uniform-type shapes that no `deqp_vsunif_*`, `deqp_vsunif2_*`,
//! `deqp_unitype_*`, `deqp_unif2_*` or other `deqp_*` reference holds.
//!
//! HOST-ONLY at `14e7a7421` (`tools/mali/deqp-host-measure.sh`, the 463 cases
//! of `tools/cts/log/2026-11-24-t6-gles2-S{1..4}-8ff01055/refusals.tsv` whose
//! first refusal is the vertex "a uniform other than a float, vec2, vec3,
//! vec4 or mat4"): 2 still stop at a vertex uniform refusal, the rest moved
//! on (lower-vsunif and the earlier lowerings) or compile. One shape is
//! uncovered:
//!
//! | shape | reference | cases | refusal at the tip | covered by |
//! |---|---|---:|---|---|
//! | `struct_arr_dynidx`: `uniform S s[2]`, `S` holding a `T b[3]` of `mediump float` and `vec2 b[2]`, read at `uniform int` indices | `struct_arr_dynidx` | 1 NotSupported (`shaders.struct.uniform.nested_struct_array_dynamic_index_vertex`) | `UniformPrecision` | none: `deqp_unif2_arr_struct_array` reads the same struct at constant indices (the other case, `nested_struct_array_vertex`), `deqp_t3gap_vsunif_nested_struct_array_dynidx` is a local struct |
//!
//! The refused one is `#[ignore]`d with its refusal pinned
//! (`refusals_pinned` runs now).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_deqp_vsunif3
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-mali"))]
mod mali_ref;

/// `MaliError::UniformPrecision`, without the `Valhall: ` prefix and the
/// ` is not lowered yet` suffix.
const VS_PRECISION: &str = "a vertex shader uniform that is not highp (Mesa's fp16 uniform), other than a float, vec2, vec3, vec4 or mat4 that is neither an array nor a struct member,";

/// The references the lowering makes byte-equal. None yet.
const MATCHED: &[&str] = &[];

/// Every refused reference with its refusal.
const REFUSED: &[(&str, &str)] = &[("struct_arr_dynidx", VS_PRECISION)];

#[test]
fn refusals_pinned() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tools/mali/testdata");
    let mut on_disk: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .filter_map(|e| {
            let f = e.unwrap().file_name().into_string().unwrap();
            f.strip_prefix("deqp_vsunif3_")?.strip_suffix(".expect.json").map(str::to_string)
        })
        .collect();
    on_disk.sort();
    let mut listed: Vec<String> =
        REFUSED.iter().map(|(n, _)| *n).chain(MATCHED.iter().copied()).map(str::to_string).collect();
    listed.sort();
    assert_eq!(on_disk, listed, "every deqp_vsunif3_* reference has a test and a pin");
    let changed: Vec<String> = REFUSED
        .iter()
        .filter_map(|(name, text)| mali_ref::refusal_change(&format!("deqp_vsunif3_{name}"), text))
        .collect();
    assert!(changed.is_empty(), "{}", changed.join("\n"));
}

#[test]
#[ignore = "refused: a vertex shader uniform that is not highp (Mesa's fp16 uniform), other than a float, vec2, vec3, vec4 or mat4 that is neither an array nor a struct member (a nested struct array uniform read at uniform int indices), the struct array lowering of mali-deqp-vsunif3.md"]
fn struct_arr_dynidx() {
    mali_ref::assert_matches("deqp_vsunif3_struct_arr_dynidx");
}
