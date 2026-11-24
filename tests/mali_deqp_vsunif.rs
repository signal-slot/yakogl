// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Byte-compare of every `tools/mali/testdata/deqp_vsunif_*` reference (Mesa
//! 26.2.1 on drm-shim, plan #402 item `ref-vsunif`,
//! `docs/notes/mali-deqp-vsunif.md`) against the Mali online compiler, in
//! the form of `tests/mali_deqp_unitype.rs`. They are the shapes behind the
//! vertex-shader refusal "a uniform other than a float, vec2, vec3, vec4 or
//! mat4" (`MaliError::UniformType`) that no earlier `deqp_*` reference
//! covers. The ones the compiler makes byte-equal run ([`MATCHED`]); every
//! refused one is `#[ignore]`d with its refusal and the family that lowers
//! it, and `refusals_pinned` (not ignored) holds each refusal's exact text,
//! so a lowering moves its pin in the same commit.
//!
//! The shapes, HOST-ONLY at `e65567ea` (`tools/mali/deqp-host-measure.sh
//! --cases` over the 463 cases whose `refusals.tsv` text in the four slices
//! `tools/cts/log/2026-11-24-t6-gles2-S{1..4}-8ff01055/` holds the refusal;
//! 375 of them are still refused by it on the host, 88 are not reproduced).
//! Fail and NotSupported are the board's status of the cases:
//!
//! ```text
//! shape               cases  Fail  NS  example case (dEQP-GLES2.functional.)                          reference
//! random_dead_int       111   111   0  shaders.random.basic_expression.vertex.13                      deqp_vsunif_random_dead_int
//! uapi_struct_int        79    79   0  uniform_api.value.assigned.by_value.render.array_in_struct.int_ivec4_both
//!                                                                                                     covered by deqp_unif2_vs_struct_uniform, deqp_vs16ref_uapi_struct_int_ivec4
//! loop_bound_product     44    44   0  shaders.loops.for_dynamic_iterations.basic_mediump_int_vertex  covered by deqp_loops2_vs_for_dyn_nested
//! vec_subscript          27     0  27  shaders.indexing.vector_subscript.vec4_direct_write_dynamic_subscript_read_vertex
//!                                                                                                     deqp_vsunif_vec_subscript
//! struct_local           26    19   7  shaders.struct.local.basic_vertex                              deqp_vsunif_struct_local
//! tmp_array              24     0  24  shaders.indexing.tmp_array.vec4_static_write_dynamic_read_vertex
//!                                                                                                     deqp_vsunif_tmp_array
//! varying_array          16     0  16  shaders.indexing.varying_array.vec4_dynamic_write_static_read  deqp_vsunif_varying_array
//! matrix_subscript       15     0  15  shaders.indexing.matrix_subscript.mat2_dynamic_write_dynamic_loop_read_vertex
//!                                                                                                     covered by deqp_unif2_vs_int_uniform
//! uapi_int               15    15   0  uniform_api.value.initial.render.basic.int_both                covered by deqp_vs16ref_uapi_int
//! loop_select             6     6   0  shaders.loops.for_uniform_iterations.select_iteration_count_vertex
//!                                                                                                     covered by deqp_loops_vs_select
//! loop_vector_counter     6     6   0  shaders.loops.for_uniform_iterations.vector_counter_vertex     covered by deqp_loops_vs_vector
//! struct_cmp              2     2   0  shaders.struct.uniform.equal_vertex                            deqp_vsunif_struct_cmp
//! struct_dynidx           2     0   2  shaders.struct.uniform.struct_array_dynamic_index_vertex       deqp_vsunif_struct_dynidx
//! struct_int              2     2   0  shaders.struct.uniform.nested_vertex                           deqp_vsunif_struct_int
//! total                 375   284  91
//! ```
//!
//! Each reference is the case's refusing program, verbatim from the .qpa,
//! drawn with `tools/mali/deqp_ref.sh <vert> <frag> <prefix> --instrument
//! shim` (gl_ref.py `--uniform-values`: struct members set one by one).
//! The struct references are compared with their members' values
//! ([`STRUCT`]: `mali_ref::differences_members(.., true)`), as
//! `mali_ref::STRUCT_VALUES` does for the references it lists.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_deqp_vsunif
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-mali"))]
mod mali_ref;

/// The refusal every reference here was taken for: a vertex shader `int`
/// uniform read other than as a uniform array's index, a loop head's
/// compare operand or a conversion to float (`nir.rs` `index_only`,
/// `int_compare_only`, `int_float_only`), or a struct uniform whose int
/// members are read so.
const UNIFORM_TYPE: &str = "Valhall: a uniform other than a float, vec2, vec3, vec4 or mat4 is not lowered yet";

/// The references the compiler makes byte-equal, without the
/// `deqp_vsunif_` prefix.
const MATCHED: &[&str] = &[];

/// The references that compile with other words than Mesa's.
const DIFFERS: &[&str] = &[];

/// Every refused reference with its whole `MaliError` message.
const REFUSED: &[(&str, &str)] = &[
    ("random_dead_int", UNIFORM_TYPE),
    ("struct_cmp", UNIFORM_TYPE),
    ("struct_dynidx", UNIFORM_TYPE),
    ("struct_int", UNIFORM_TYPE),
    ("struct_local", UNIFORM_TYPE),
    ("tmp_array", UNIFORM_TYPE),
    ("varying_array", UNIFORM_TYPE),
    ("vec_subscript", UNIFORM_TYPE),
];

/// The references with a struct uniform, drawn with its members set.
const STRUCT: &[&str] = &["struct_cmp", "struct_dynidx", "struct_int"];

fn reference(name: &str) -> mali_ref::Reference {
    mali_ref::Reference::load(&format!("deqp_vsunif_{name}"))
}

fn differences(name: &str) -> Result<Vec<String>, String> {
    let r = reference(name);
    let pair = mali_ref::compile_pair(&r.vs, &r.fs)?;
    Ok(mali_ref::differences_members(&r, &pair, STRUCT.contains(&name)))
}

fn assert_matches(name: &str) {
    let d = differences(name).unwrap_or_else(|e| panic!("deqp_vsunif_{name}: the pair is refused: {e}"));
    assert!(d.is_empty(), "deqp_vsunif_{name}: differs from its reference:\n{}", d.join("\n"));
}

#[test]
fn refusals_pinned() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tools/mali/testdata");
    let mut on_disk: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .filter_map(|e| {
            let f = e.unwrap().file_name().into_string().unwrap();
            f.strip_prefix("deqp_vsunif_")?.strip_suffix(".expect.json").map(str::to_string)
        })
        .collect();
    on_disk.sort();
    let mut listed: Vec<String> = REFUSED
        .iter()
        .map(|(n, _)| *n)
        .chain(MATCHED.iter().copied())
        .chain(DIFFERS.iter().copied())
        .map(str::to_string)
        .collect();
    listed.sort();
    assert_eq!(on_disk, listed, "every deqp_vsunif_* reference has a test and a pin");
    let mut changed = Vec::new();
    for (name, text) in REFUSED {
        let r = reference(name);
        match mali_ref::compile_pair(&r.vs, &r.fs) {
            Ok(_) => changed.push(format!("{name}: now compiles; remove its #[ignore] and its pin")),
            Err(e) if e == *text => {}
            Err(e) => changed.push(format!("{name}: the refusal changed to {e:?}")),
        }
    }
    assert!(changed.is_empty(), "{}", changed.join("\n"));
}

#[test]
fn differences_pinned() {
    for name in DIFFERS {
        let d = differences(name).unwrap_or_else(|e| panic!("{name}: refused: {e}"));
        assert!(!d.is_empty(), "{name}: now byte-equal; move it to MATCHED");
    }
}

#[test]
#[ignore = "refused: a uniform other than a float, vec2, vec3, vec4 or mat4 (an int uniform read only by dead code, which Mesa removes before it lays out the uniforms), lowered by lower-vsunif"]
fn random_dead_int() {
    assert_matches("random_dead_int");
}

#[test]
#[ignore = "refused: a uniform other than a float, vec2, vec3, vec4 or mat4 (the int index of a vec4's dynamic subscript read), lowered by lower-vsunif"]
fn vec_subscript() {
    assert_matches("vec_subscript");
}

#[test]
#[ignore = "refused: a uniform other than a float, vec2, vec3, vec4 or mat4 (the int index of a local vec4 array's dynamic read), lowered by lower-vsunif"]
fn tmp_array() {
    assert_matches("tmp_array");
}

#[test]
#[ignore = "refused: a uniform other than a float, vec2, vec3, vec4 or mat4 (the int index of a varying array's dynamic write), lowered by lower-vsunif"]
fn varying_array() {
    assert_matches("varying_array");
}

#[test]
#[ignore = "refused: a uniform other than a float, vec2, vec3, vec4 or mat4 (an int uniform stored in a local struct's int member, read as a float), lowered by lower-vsunif"]
fn struct_local() {
    assert_matches("struct_local");
}

#[test]
#[ignore = "refused: a uniform other than a float, vec2, vec3, vec4 or mat4 (struct uniforms compared whole, an int member among them), lowered by lower-vsunif"]
fn struct_cmp() {
    assert_matches("struct_cmp");
}

#[test]
#[ignore = "refused: a uniform other than a float, vec2, vec3, vec4 or mat4 (a nested struct uniform's int members added, then read as a float), lowered by lower-vsunif"]
fn struct_int() {
    assert_matches("struct_int");
}

#[test]
#[ignore = "refused: a uniform other than a float, vec2, vec3, vec4 or mat4 (a struct uniform array read at int uniform indices, its int members added), lowered by lower-vsunif"]
fn struct_dynidx() {
    assert_matches("struct_dynidx");
}
