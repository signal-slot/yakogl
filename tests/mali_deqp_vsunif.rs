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
//! Since `5e1feec9` (plan #387 `intval-vs16`, step vs16-2) the vertex walk
//! no longer refuses these with `UniformType`, so the pins were repinned
//! (HOST-ONLY, at `696d5be3`): `random_dead_int` and `struct_local` are
//! byte-equal ([`MATCHED`]); `vec_subscript`, `tmp_array` and
//! `struct_dynidx` compile but differ ([`DIFFERS`]; the first difference is
//! in the comment of each test); `struct_cmp`, `struct_int` and
//! `varying_array` are refused with other texts ([`REFUSED`]). Plan #402's
//! `lower-vsunif` makes `struct_int` byte-equal ([`MATCHED`]): a nested
//! struct's member (`s.b.b`) is fp32 in the block whatever its precision,
//! as a plain struct's member already was.
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
//! Plan #387 item `vsuniftype-design` (`docs/notes/mali-vsunif-lowering.md`)
//! adds twelve references ([`DESIGN`]), taken from the 389 cases the Verdin
//! iMX95 (Mali-G310) run at `e63ab28e` refused with the same text. At the
//! tip `8133663c` none of the 389 meets it any more; the twelve are the
//! shapes behind what they meet now, or compile into, that no other design
//! owns: the vertex vector and struct `==` (`local_equal`,
//! `local_nested_equal`), the dynamically indexed local writes
//! (`vec2_dynwrite`, `tmp_dynwrite`, `mat2_dynwrite`), Mesa's fp16 trees
//! for mediump vertex values (`vec2_subscript`, `mat2_subscript`,
//! `local_cond_assign`), the scratch-memory array (`tmp_float_const`) and
//! four struct shapes that compile byte-equal (`local_array_member`,
//! `local_parameter`, `unif_struct_array`). All twelve are drawn on
//! Mesa 26.2.1's G610 drm-shim and are identical on the G310v2
//! (`tools/mali/g310_check.sh`). `references_load` checks that every
//! reference on disk loads and has its regions' words.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_deqp_vsunif
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-mali"))]
mod mali_ref;

/// `struct_cmp`: the struct compare is not lowered (the refusal every
/// reference here was taken for, `UniformType`, is gone since `5e1feec9`).
const ARITHMETIC: &str = "Valhall: arithmetic other than +, -, * and a dot or matrix product (in a fragment shader: +, -, * and abs) is not lowered yet";

/// `varying_array`: the dynamically written varying array leaves lanes unwritten.
const VARYING_LANE: &str = "Valhall: a varying lane left unwritten is not lowered yet";

/// The references the compiler makes byte-equal, without the
/// `deqp_vsunif_` prefix.
const MATCHED: &[&str] = &["random_dead_int", "struct_int", "struct_local"];

/// The references that compile with other words than Mesa's.
const DIFFERS: &[&str] = &["struct_dynidx", "tmp_array", "vec_subscript"];

/// Every refused reference with its whole `MaliError` message.
const REFUSED: &[(&str, &str)] = &[("struct_cmp", ARITHMETIC), ("varying_array", VARYING_LANE)];

/// The references with a struct uniform, drawn with its members set.
const STRUCT: &[&str] = &["struct_cmp", "struct_dynidx", "struct_int", "unif_struct_array"];

/// `local_equal`, `local_nested_equal`: a vertex `==` of mediump vec3s and
/// of ints (`nir.rs` `vs_reduce`, its last arm).
const ARITHMETIC_D: &str = "Valhall: arithmetic other than +, -, * and a dot or matrix product (in a fragment shader: +, -, * and abs) is not lowered yet";

/// The dynamically indexed local writes (`tmp[ui_k] = ..`).
const VARYING_LANE_D: &str = "Valhall: a varying lane left unwritten is not lowered yet";

/// The references of plan #387 item `vsuniftype-design`, each with its
/// state at `8133663c`: `Ok(true)` byte-equal, `Ok(false)` compiles with
/// other words than Mesa's, `Err(text)` refused with the whole text. The
/// step of `docs/notes/mali-vsunif-lowering.md` that changes a state moves
/// it here and drops the test's `#[ignore]` in the same commit.
const DESIGN: &[(&str, Result<bool, &str>)] = &[
    ("local_array_member", Ok(true)),
    ("local_cond_assign", Ok(false)),
    ("local_equal", Err(ARITHMETIC_D)),
    ("local_nested_equal", Err(ARITHMETIC_D)),
    ("local_parameter", Ok(true)),
    ("mat2_dynwrite", Err(VARYING_LANE_D)),
    ("mat2_subscript", Ok(false)),
    ("tmp_dynwrite", Err(VARYING_LANE_D)),
    ("tmp_float_const", Ok(false)),
    ("unif_struct_array", Ok(true)),
    ("vec2_dynwrite", Err(VARYING_LANE_D)),
    ("vec2_subscript", Ok(false)),
];

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
        .chain(DESIGN.iter().map(|(n, _)| *n))
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

/// Each [`DESIGN`] reference is in its pinned state at the tip.
#[test]
fn design_pinned() {
    let mut changed = Vec::new();
    for (name, want) in DESIGN {
        let now = match differences(name) {
            Ok(d) => Ok(d.is_empty()),
            Err(e) => Err(e),
        };
        let same = match (&now, want) {
            (Ok(a), Ok(b)) => a == b,
            (Err(a), Err(b)) => a == b,
            _ => false,
        };
        if !same {
            changed.push(format!("{name}: pinned {want:?}, now {now:?}"));
        }
    }
    assert!(changed.is_empty(), "{}", changed.join("\n"));
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

/// Every [`DESIGN`] reference loads: both shaders have a `main`, the trace
/// is the reference's own, and `vs.position`, `vs.varying` and
/// `fs.fragment` have 64-bit hex words.
#[test]
fn references_load() {
    let mut bad = Vec::new();
    for (name, _) in DESIGN {
        let r = reference(name);
        if !r.vs.contains("void main") || !r.fs.contains("void main") {
            bad.push(format!("{name}: a shader has no main"));
        }
        if !r.json.contains(&format!("\"trace\": \"deqp_vsunif_{name}.trace\"")) {
            bad.push(format!("{name}: the trace is not deqp_vsunif_{name}.trace"));
        }
        for region in ["vs.position", "vs.varying", "fs.fragment"] {
            match region_words(&r.json, region) {
                None => bad.push(format!("{name}: no {region}")),
                Some(Err(e)) => bad.push(format!("{name}: {e}")),
                Some(Ok(0)) => bad.push(format!("{name}: {region} has no words")),
                Some(Ok(_)) => {}
            }
        }
    }
    assert!(bad.is_empty(), "{}", bad.join("\n"));
}

#[test]
fn differences_pinned() {
    for name in DIFFERS {
        let d = differences(name).unwrap_or_else(|e| panic!("{name}: refused: {e}"));
        assert!(!d.is_empty(), "{name}: now byte-equal; move it to MATCHED");
    }
}

#[test]
fn random_dead_int() {
    assert_matches("random_dead_int");
}

#[test]
fn struct_local() {
    assert_matches("struct_local");
}

/// Compiles. The last FAU word of `vs.position` differs (`0x3e000000` ours,
/// `0x30003000` Mesa's fp16 pair). The first differing instruction is `[0]`:
/// ours `LD_ATTR_IMM` lacks the `.wait0` Mesa's has; then Mesa narrows with
/// `V2F32_TO_V2F16` and an fp16 `MUX.v2i16` tree where ours is an f32
/// `ICMP_OR.u32.eq` / `CSEL.u32.eq` tree.
#[test]
#[ignore = "compiles but differs: vs.position FAU word 15, instruction 0 (LD_ATTR_IMM without .wait0; fp16 MUX.v2i16 tree against our f32 CSEL tree)"]
fn tmp_array() {
    assert_matches("tmp_array");
}

/// Compiles. The FAU words of `vs.position` differ. The first differing
/// instruction is `[1]`: ours `FMA.f32` of the attribute, Mesa's
/// `V2F32_TO_V2F16` and a `MUX.v2i16` tree on `ICMP_OR.s32.lt`. The varying
/// is 34 words ours, 44 Mesa's.
#[test]
#[ignore = "compiles but differs: vs.position FAU words, instruction 1 (FMA.f32 against V2F32_TO_V2F16 and a MUX.v2i16 tree)"]
fn vec_subscript() {
    assert_matches("vec_subscript");
}

/// Compiles. The FAU words of `vs.position` differ. The first differing
/// instruction is `[0]`: ours `MOV.i32 r0, 0x0`, Mesa's `MOV.i32 r0,
/// 0x3020100`; then Mesa addresses the struct array with `LSHIFT_OR.i32`
/// and `IADD_IMM` where ours is an `ICMP_OR.u32.eq` / `CSEL.u32.eq` select
/// tree. The varying is 42 words ours, 37 Mesa's.
#[test]
#[ignore = "compiles but differs: vs.position FAU words, instruction 0 (MOV 0x0 against 0x3020100; select tree against LSHIFT_OR / IADD_IMM addressing)"]
fn struct_dynidx() {
    assert_matches("struct_dynidx");
}

#[test]
#[ignore = "refused: arithmetic other than +, -, * and a dot or matrix product (struct uniforms compared whole, an int member among them), lowered by lower-vsunif"]
fn struct_cmp() {
    assert_matches("struct_cmp");
}

#[test]
fn struct_int() {
    assert_matches("struct_int");
}

#[test]
#[ignore = "refused: a varying lane left unwritten (the int index of a varying array's dynamic write), lowered by lower-vsunif"]
fn varying_array() {
    assert_matches("varying_array");
}

/// `shaders.struct.local.array_member_vertex`: a local struct's array
/// member read at constant indices, an int uniform among the values.
#[test]
fn local_array_member() {
    assert_matches("local_array_member");
}

/// `shaders.struct.local.conditional_assignment_vertex`. Compiles. The
/// first differing instruction of `vs.varying` is `[0]`: ours `F16_TO_F32`
/// of the mediump uniform `uf_one` and an f32 `FCMP_OR`, Mesa's
/// `FCMP_OR.v2f16.lt.m1` of its fp16 half and `S16_TO_S32` (step V3).
#[test]
#[ignore = "compiles but differs: vs.varying instruction 0 (F16_TO_F32 and FCMP_OR.f32 against FCMP_OR.v2f16 of the fp16 uniform; step V3)"]
fn local_cond_assign() {
    assert_matches("local_cond_assign");
}

/// `shaders.struct.local.basic_equal_vertex`: `a == b` of local structs
/// `{mediump float; mediump vec3; int}` (step V1).
#[test]
#[ignore = "refused: arithmetic (the vertex vec3 and int == of a struct compare; step V1)"]
fn local_equal() {
    assert_matches("local_equal");
}

/// `shaders.struct.local.nested_equal_vertex`: `==` of nested local
/// structs (step V1).
#[test]
#[ignore = "refused: arithmetic (the vertex vec3 and int == of a nested struct compare; step V1)"]
fn local_nested_equal() {
    assert_matches("local_nested_equal");
}

/// `shaders.struct.local.parameter_vertex`: a local struct passed to a
/// function, an int uniform member.
#[test]
fn local_parameter() {
    assert_matches("local_parameter");
}

/// `shaders.indexing.matrix_subscript.mat2_dynamic_write_static_read_vertex`:
/// `tmp[ui_zero] = ..` of a local mediump mat2 (step V2).
#[test]
#[ignore = "refused: a varying lane left unwritten (a local mat2 column written at a run-time index; step V2)"]
fn mat2_dynwrite() {
    assert_matches("mat2_dynwrite");
}

/// `shaders.indexing.matrix_subscript.mat2_static_write_dynamic_read_vertex`.
/// Compiles. The first differing instruction of `vs.varying` is `[1]`: ours
/// `FMA.f32`, Mesa's `V2F32_TO_V2F16` and a `MUX.v2i16` tree on
/// `ICMP_OR.s32.lt.m1` (step V3); 27 words ours, 30 Mesa's.
#[test]
#[ignore = "compiles but differs: vs.varying instruction 1 (FMA.f32 against V2F32_TO_V2F16 and a MUX.v2i16 tree; step V3)"]
fn mat2_subscript() {
    assert_matches("mat2_subscript");
}

/// `shaders.indexing.tmp_array.vec4_dynamic_write_static_read_vertex`:
/// `arr[ui_k] = ..` of a local `mediump vec4 arr[4]` (step V2).
#[test]
#[ignore = "refused: a varying lane left unwritten (a local array written at run-time indices; step V2)"]
fn tmp_dynwrite() {
    assert_matches("tmp_dynwrite");
}

/// `shaders.indexing.tmp_array.float_const_write_dynamic_read_vertex`.
/// Compiles. Mesa keeps the 40-element `mediump float` array in thread-local
/// storage (`STORE.i16` to `thread_local_pointer`, loads at run-time
/// offsets; secondary offset 896): `vs.position` is 24 words ours, 104
/// Mesa's (step V4).
#[test]
#[ignore = "compiles but differs: Mesa's thread-local storage of the 40-element local array (vs.position 24 words ours, 104 Mesa's; step V4)"]
fn tmp_float_const() {
    assert_matches("tmp_float_const");
}

/// `shaders.struct.uniform.struct_array_vertex`: `uniform S s[3]` read at
/// constant indices, with its members' values.
#[test]
fn unif_struct_array() {
    assert_matches("unif_struct_array");
}

/// `shaders.indexing.vector_subscript.vec2_dynamic_subscript_write_component_read_vertex`:
/// `tmp[ui_zero] = ..` of a local mediump vec2 (step V2).
#[test]
#[ignore = "refused: a varying lane left unwritten (a local vec2 written at a run-time subscript; step V2)"]
fn vec2_dynwrite() {
    assert_matches("vec2_dynwrite");
}

/// `shaders.indexing.vector_subscript.vec2_component_write_dynamic_subscript_read_vertex`.
/// Compiles. The first differing instruction of `vs.varying` is `[1]`: ours
/// `FMA.f32`, Mesa's `V2F32_TO_V2F16` and the fp16 select (step V3); 9
/// words ours, 20 Mesa's.
#[test]
#[ignore = "compiles but differs: vs.varying instruction 1 (FMA.f32 against V2F32_TO_V2F16 and an fp16 select; step V3)"]
fn vec2_subscript() {
    assert_matches("vec2_subscript");
}
