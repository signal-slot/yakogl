// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Mesa 26.2.1 references `tools/mali/testdata/deqp_boolpix_*` (plan #387,
//! item fclane-lower-2, issue #11306, `docs/notes/mali-deqp-boolpix.md`): the
//! dEQP-GLES2 shaders with bool or bvec values that the Mali online compiler
//! compiles and the NanoPC-T6 draws wrong (`Image comparison failed`, no
//! refusal; the whole-tree run at 8ff01055,
//! `tools/cts/log/2026-11-24-t6-gles2-S{1..4}-8ff01055`). Each reference was
//! drawn with
//! `tools/mali/deqp_ref.sh <vert> <frag> tools/mali/testdata/deqp_boolpix_<shape> --instrument shim`
//! (Mesa 26.2.1's G610 drm-shim, `--uniform-values`). `tests/mali_ref` does
//! the compare. A reference byte-equal at the tip is in [`LOWERED`] with a
//! plain test; one that differs is in [`DIFFERS`] with an `#[ignore]`d test,
//! and `differs_pinned` checks both lists. A fix that makes one byte-equal
//! moves it to [`LOWERED`] and drops its `#[ignore]` in the same commit.
//!
//! The cause tags are the note's section 3: BW the bool uniform's word (our
//! GL stores 1, Mesa's words read `~0`; not in the words, so a byte-equal
//! shape still draws wrong), D1 the odd-lane `b2i32` miscompile, LK a
//! mediump struct member pushed as fp16 halves but read as an fp32 word, and
//! the value-equal form differences next to them (V1, VL, A1, S1, FP, NF,
//! CF, O; the note's section 4).
//!
//! Where the shader text comes from: every pair is rebuilt from its `.test`
//! row (`data/gles2/shaders/{swizzles,conversions,functions,linkage}.test`
//! of VK-GL-CTS `93bca01861b0e3ef3c387027a9791e6d065f900c`) through the
//! ShaderLibrary template of that commit (`glsShaderLibraryCase.cpp`
//! `genVertexShader`, `genFragmentShader`, `specialize*Shader`,
//! `generate*Specialization`; `gluShaderLibrary.cpp` `genCompareFunctions`,
//! `removeExtraIndentation`), so the text is dEQP's verbatim.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_deqp_boolpix
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-mali"))]
mod mali_ref;

/// The references byte-equal to Mesa's words at the tip, without the
/// `deqp_boolpix_` prefix: each has a byte-compare test below.
const LOWERED: &[&str] = &[
    "bw_conv_bvec3_to_bvec3_fs",
    "bw_conv_float_to_bool_fs",
    "bw_conv_int_to_bvec2_vs",
    "bw_conv_ivec4_to_bvec4_vs",
    "bw_conv_vec4_to_bool_fs",
    "bw_swz_bvec3_zyx_fs",
    "bw_swz_bvec4_wzyx_fs",
];

/// The references that compile with other words or facts than Mesa's.
const DIFFERS: &[&str] = &[
    "bw_conv_bool_bool_to_bvec2_fs",
    "bw_conv_bool_to_bool_vs",
    "bw_conv_bvec3_float_to_bvec4_fs",
    "bw_conv_bvec4_to_bool_vs",
    "bw_conv_float_to_bvec4_vs",
    "bw_conv_vec2_bvec2_to_bvec4_vs",
    "bw_fn_bool_bvec3_fs",
    "bw_fn_global_in_bool_vs",
    "bw_fn_in_bool_vs",
    "bw_fn_out_bool_fs",
    "bw_fn_user_func_arg_bool_types_vs",
    "bw_swz_bvec2_grg_vs",
    "bw_swz_bvec2_r_vs",
    "bw_swz_bvec4_wzyx_vs",
    "d1_conv_bvec2_to_ivec2_fs",
    "d1_conv_bvec3_float_to_ivec4_fs",
    "d1_conv_bvec4_to_ivec4_fs",
    "lk_uniform_struct",
    "lk_uniform_struct_fragment_only_vec4",
    "lk_uniform_struct_partial",
    "lk_uniform_struct_use_case_rip",
    "lk_uniform_struct_vec4",
];

/// `dEQP-GLES2.functional.shaders.conversions.vector_combine.bool_bool_to_bvec2_fragment` (BW+FP): two scalar varyings narrowed one by one (`FADD.f32.discard r0.h0`) and compared in two one-lane `FCMP_OR.v2f16`, where Mesa pairs them (`V2F32_TO_V2F16`, one compare); the same values.
#[test]
#[ignore = "compiles with other words than Mesa's; pinned by differs_pinned"]
fn bw_conv_bool_bool_to_bvec2_fs() {
    mali_ref::assert_matches("deqp_boolpix_bw_conv_bool_bool_to_bvec2_fs");
}

/// `dEQP-GLES2.functional.shaders.conversions.scalar_to_scalar.bool_to_bool_vertex` (BW+V1): fragment equal; the vertex `bool(a_in0)` is `FCMP_OR.f32.ne.f1` where Mesa narrows with `FADD.f32 r0.h0, r0^, 0x0.neg`.
#[test]
#[ignore = "compiles with other words than Mesa's; pinned by differs_pinned"]
fn bw_conv_bool_to_bool_vs() {
    mali_ref::assert_matches("deqp_boolpix_bw_conv_bool_to_bool_vs");
}

/// `dEQP-GLES2.functional.shaders.conversions.vector_combine.bvec3_float_to_bvec4_fragment` (BW+FP): the four lanes paired otherwise (ours `.yz` + `.w`,`.x`; Mesa `.yx` + `.zw`), every lane compared against the same uniform word as Mesa's.
#[test]
#[ignore = "compiles with other words than Mesa's; pinned by differs_pinned"]
fn bw_conv_bvec3_float_to_bvec4_fs() {
    mali_ref::assert_matches("deqp_boolpix_bw_conv_bvec3_float_to_bvec4_fs");
}

/// `dEQP-GLES2.functional.shaders.conversions.vector_to_vector.bvec3_to_bvec3_fragment` (BW): byte-equal.
#[test]
fn bw_conv_bvec3_to_bvec3_fs() {
    mali_ref::assert_matches("deqp_boolpix_bw_conv_bvec3_to_bvec3_fs");
}

/// `dEQP-GLES2.functional.shaders.conversions.vector_to_scalar.bvec4_to_bool_vertex` (BW+VL): fragment equal; the vertex shader loads `a_in0.x` alone where Mesa loads the `vec4`.
#[test]
#[ignore = "compiles with other words than Mesa's; pinned by differs_pinned"]
fn bw_conv_bvec4_to_bool_vs() {
    mali_ref::assert_matches("deqp_boolpix_bw_conv_bvec4_to_bool_vs");
}

/// `dEQP-GLES2.functional.shaders.conversions.scalar_to_scalar.float_to_bool_fragment` (BW): byte-equal.
#[test]
fn bw_conv_float_to_bool_fs() {
    mali_ref::assert_matches("deqp_boolpix_bw_conv_float_to_bool_fs");
}

/// `dEQP-GLES2.functional.shaders.conversions.scalar_to_vector.float_to_bvec4_vertex` (BW+V1): fragment equal; the vertex `bvec4(in0)` of a float at fp32.
#[test]
#[ignore = "compiles with other words than Mesa's; pinned by differs_pinned"]
fn bw_conv_float_to_bvec4_vs() {
    mali_ref::assert_matches("deqp_boolpix_bw_conv_float_to_bvec4_vs");
}

/// `dEQP-GLES2.functional.shaders.conversions.scalar_to_vector.int_to_bvec2_vertex` (BW): byte-equal (a vertex `int` source).
#[test]
fn bw_conv_int_to_bvec2_vs() {
    mali_ref::assert_matches("deqp_boolpix_bw_conv_int_to_bvec2_vs");
}

/// `dEQP-GLES2.functional.shaders.conversions.vector_to_vector.ivec4_to_bvec4_vertex` (BW): byte-equal.
#[test]
fn bw_conv_ivec4_to_bvec4_vs() {
    mali_ref::assert_matches("deqp_boolpix_bw_conv_ivec4_to_bvec4_vs");
}

/// `dEQP-GLES2.functional.shaders.conversions.vector_combine.vec2_bvec2_to_bvec4_vertex` (BW+A1): fragment equal; the vertex shader loads attribute index 2 first where Mesa loads index 1 (audit A1).
#[test]
#[ignore = "compiles with other words than Mesa's; pinned by differs_pinned"]
fn bw_conv_vec2_bvec2_to_bvec4_vs() {
    mali_ref::assert_matches("deqp_boolpix_bw_conv_vec2_bvec2_to_bvec4_vs");
}

/// `dEQP-GLES2.functional.shaders.conversions.vector_to_scalar.vec4_to_bool_fragment` (BW): byte-equal.
#[test]
fn bw_conv_vec4_to_bool_fs() {
    mali_ref::assert_matches("deqp_boolpix_bw_conv_vec4_to_bool_fs");
}

/// `dEQP-GLES2.functional.shaders.functions.datatypes.bool_bvec3_fragment` (BW+S1): the same instructions as Mesa's, two in the other order (`ICMP_OR` of `a.x == a.y` before the `FCMP_OR` of `a.z`; audit S1).
#[test]
#[ignore = "compiles with other words than Mesa's; pinned by differs_pinned"]
fn bw_fn_bool_bvec3_fs() {
    mali_ref::assert_matches("deqp_boolpix_bw_fn_bool_bvec3_fs");
}

/// `dEQP-GLES2.functional.shaders.functions.array_arguments.global_in_bool_vertex` (BW+V1): fragment equal; the vertex `bvec` of the array at fp32 (`FCMP_OR.f32.ne.m1`).
#[test]
#[ignore = "compiles with other words than Mesa's; pinned by differs_pinned"]
fn bw_fn_global_in_bool_vs() {
    mali_ref::assert_matches("deqp_boolpix_bw_fn_global_in_bool_vs");
}

/// `dEQP-GLES2.functional.shaders.functions.qualifiers.in_bool_vertex` (BW+CF): `f != !f` is a constant `true`: Mesa folds it, its fragment shader reads the uniform (`MOV.i32.discard r0, u2.w0`) and keeps no varying; ours computes it in the vertex shader and compares the varying.
#[test]
#[ignore = "compiles with other words than Mesa's; pinned by differs_pinned"]
fn bw_fn_in_bool_vs() {
    mali_ref::assert_matches("deqp_boolpix_bw_fn_in_bool_vs");
}

/// `dEQP-GLES2.functional.shaders.functions.qualifiers.out_bool_fragment` (BW+NF): `in0 == false` as `FCMP_OR.v2f16.ne` and a 32-bit `ICMP_OR.u32.eq` against 0, where Mesa folds the `not` into `FCMP_OR.v2f16.eq`; then a 32-bit compare against the whole uniform word (Mesa: its low half).
#[test]
#[ignore = "compiles with other words than Mesa's; pinned by differs_pinned"]
fn bw_fn_out_bool_fs() {
    mali_ref::assert_matches("deqp_boolpix_bw_fn_out_bool_fs");
}

/// `dEQP-GLES2.functional.shaders.functions.overloading.user_func_arg_bool_types_vertex` (BW+V1): fragment equal; the vertex `bool()` at fp32.
#[test]
#[ignore = "compiles with other words than Mesa's; pinned by differs_pinned"]
fn bw_fn_user_func_arg_bool_types_vs() {
    mali_ref::assert_matches("deqp_boolpix_bw_fn_user_func_arg_bool_types_vs");
}

/// `dEQP-GLES2.functional.shaders.swizzles.vector_swizzles.mediump_bvec2_grg_vertex` (BW+V1): the fragment `isOk` is Mesa's words (`ICMP_OR.v2u16.eq` of the mask against `u3.w0.h00`, the bool uniform's low half); the vertex `bvec2(a_in0)` is `FCMP_OR.f32.ne.f1` where Mesa narrows (`V2F32_TO_V2F16`, `FCMP_OR.v2f16`).
#[test]
#[ignore = "compiles with other words than Mesa's; pinned by differs_pinned"]
fn bw_swz_bvec2_grg_vs() {
    mali_ref::assert_matches("deqp_boolpix_bw_swz_bvec2_grg_vs");
}

/// `dEQP-GLES2.functional.shaders.swizzles.vector_swizzles.mediump_bvec2_r_vertex` (BW+VL): fragment equal; the vertex shader loads only the lane it reads (`LD_ATTR_IMM.f32`) where Mesa loads the whole `vec2`.
#[test]
#[ignore = "compiles with other words than Mesa's; pinned by differs_pinned"]
fn bw_swz_bvec2_r_vs() {
    mali_ref::assert_matches("deqp_boolpix_bw_swz_bvec2_r_vs");
}

/// `dEQP-GLES2.functional.shaders.swizzles.vector_swizzles.mediump_bvec3_zyx_fragment` (BW): byte-equal: the words are Mesa's, the wrong pixels are the bool uniform's word (1 where Mesa's words read `0xffff`).
#[test]
fn bw_swz_bvec3_zyx_fs() {
    mali_ref::assert_matches("deqp_boolpix_bw_swz_bvec3_zyx_fs");
}

/// `dEQP-GLES2.functional.shaders.swizzles.vector_swizzles.mediump_bvec4_wzyx_fragment` (BW): byte-equal, as `bw_swz_bvec3_zyx_fs`.
#[test]
fn bw_swz_bvec4_wzyx_fs() {
    mali_ref::assert_matches("deqp_boolpix_bw_swz_bvec4_wzyx_fs");
}

/// `dEQP-GLES2.functional.shaders.swizzles.vector_swizzles.mediump_bvec4_wzyx_vertex` (BW+V1): fragment equal; the vertex `bvec4(a_in0)` at fp32 (`FCMP_OR.f32.ne.f1`), Mesa's `V2F32_TO_V2F16` pairs.
#[test]
#[ignore = "compiles with other words than Mesa's; pinned by differs_pinned"]
fn bw_swz_bvec4_wzyx_vs() {
    mali_ref::assert_matches("deqp_boolpix_bw_swz_bvec4_wzyx_vs");
}

/// `dEQP-GLES2.functional.shaders.conversions.vector_to_vector.bvec2_to_ivec2_fragment` (D1): the odd-lane miscompile: `ivec2(in0)` widens `.h00` for both lanes where Mesa widens `.h11` for `y` (`IADD.v2u16 r1, r0.h11, 0x0`).
#[test]
#[ignore = "compiles with other words than Mesa's; pinned by differs_pinned"]
fn d1_conv_bvec2_to_ivec2_fs() {
    mali_ref::assert_matches("deqp_boolpix_d1_conv_bvec2_to_ivec2_fs");
}

/// `dEQP-GLES2.functional.shaders.conversions.vector_combine.bvec3_float_to_ivec4_fragment` (D1+FP): D1 (no `IADD.v2u16 .h11` for the odd lane), after a pairing of the lanes in another order.
#[test]
#[ignore = "compiles with other words than Mesa's; pinned by differs_pinned"]
fn d1_conv_bvec3_float_to_ivec4_fs() {
    mali_ref::assert_matches("deqp_boolpix_d1_conv_bvec3_float_to_ivec4_fs");
}

/// `dEQP-GLES2.functional.shaders.conversions.vector_to_vector.bvec4_to_ivec4_fragment` (D1): D1: `IADD.v2u16 r1, r1^.h00` where Mesa has `r1.h11`.
#[test]
#[ignore = "compiles with other words than Mesa's; pinned by differs_pinned"]
fn d1_conv_bvec4_to_ivec4_fs() {
    mali_ref::assert_matches("deqp_boolpix_d1_conv_bvec4_to_ivec4_fs");
}

/// `dEQP-GLES2.functional.shaders.linkage.uniform_struct` (LK): the words are Mesa's, but our record pushes the struct members `val.a`, `val.b` as fp16 halves (FAU `00004c00`, `00005000`) where the words read fp32 words (Mesa `41800000`, `42000000`).
#[test]
#[ignore = "compiles with other words than Mesa's; pinned by differs_pinned"]
fn lk_uniform_struct() {
    mali_ref::assert_matches("deqp_boolpix_lk_uniform_struct");
}

/// `dEQP-GLES2.functional.shaders.linkage.uniform_struct_fragment_only_vec4` (LK+O): as `lk_uniform_struct_vec4`, the struct read in the fragment shader only.
#[test]
#[ignore = "compiles with other words than Mesa's; pinned by differs_pinned"]
fn lk_uniform_struct_fragment_only_vec4() {
    mali_ref::assert_matches("deqp_boolpix_lk_uniform_struct_fragment_only_vec4");
}

/// `dEQP-GLES2.functional.shaders.linkage.uniform_struct_partial` (LK): as `lk_uniform_struct`: `val.b` pushed as a half, read as fp32.
#[test]
#[ignore = "compiles with other words than Mesa's; pinned by differs_pinned"]
fn lk_uniform_struct_partial() {
    mali_ref::assert_matches("deqp_boolpix_lk_uniform_struct_partial");
}

/// `dEQP-GLES2.functional.shaders.linkage.uniform_struct_use_case_rip` (LK): as `lk_uniform_struct`: `val.quadraticAttenuation` pushed as a half, read as fp32.
#[test]
#[ignore = "compiles with other words than Mesa's; pinned by differs_pinned"]
fn lk_uniform_struct_use_case_rip() {
    mali_ref::assert_matches("deqp_boolpix_lk_uniform_struct_use_case_rip");
}

/// `dEQP-GLES2.functional.shaders.linkage.uniform_struct_vec4` (LK+O): LK, and the uniform-only varying form of the audit's `o_linkage_uniform_struct_vec4` (`V2F32_TO_V2F16 r0, u2.w1, u2.w0` where Mesa has `FADD.f32.discard r0.h0, u2.w0`).
#[test]
#[ignore = "compiles with other words than Mesa's; pinned by differs_pinned"]
fn lk_uniform_struct_vec4() {
    mali_ref::assert_matches("deqp_boolpix_lk_uniform_struct_vec4");
}

fn on_disk() -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/tools/mali/testdata"))
        .expect("tools/mali/testdata")
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter_map(|n| n.strip_prefix("deqp_boolpix_")?.strip_suffix(".expect.json").map(str::to_string))
        .collect();
    v.sort();
    v
}

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

/// Every reference on disk is in exactly one list, its pair and trace are
/// its own, its sources carry a `.license`, and each region has 64-bit hex
/// words.
#[test]
fn references_parse() {
    let names = on_disk();
    let mut listed: Vec<&str> = LOWERED.iter().chain(DIFFERS).copied().collect();
    listed.sort_unstable();
    let before = listed.len();
    listed.dedup();
    assert_eq!(before, listed.len(), "a deqp_boolpix_* shape is in both LOWERED and DIFFERS, or twice in one");
    assert_eq!(names, listed, "a deqp_boolpix_* reference on disk is not listed, or a listed one is missing");
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tools/mali/testdata");
    let mut bad = Vec::new();
    for name in &names {
        let r = mali_ref::Reference::load(&format!("deqp_boolpix_{name}"));
        if !r.vs.contains("void main") || !r.fs.contains("void main") {
            bad.push(format!("{name}: a shader has no main"));
        }
        if !r.json.contains(&format!("\"trace\": \"deqp_boolpix_{name}.trace\"")) {
            bad.push(format!("{name}: the trace is not deqp_boolpix_{name}.trace"));
        }
        for ext in ["vert", "frag"] {
            if !dir.join(format!("deqp_boolpix_{name}.{ext}.license")).is_file() {
                bad.push(format!("{name}: no .{ext}.license"));
            }
        }
        for region in ["vs.position", "vs.varying", "fs.fragment"] {
            match region_words(&r.json, region) {
                None if region == "vs.varying" => {}
                None => bad.push(format!("{name}: no {region}")),
                Some(Err(e)) => bad.push(format!("{name}: {e}")),
                Some(Ok(0)) => bad.push(format!("{name}: {region} has no words")),
                Some(Ok(_)) => {}
            }
        }
    }
    assert!(bad.is_empty(), "{}", bad.join("\n"));
}

/// Each DIFFERS shape still compiles and still is not Mesa's words and
/// facts; each LOWERED one is byte-equal. A fix that moves one changes the
/// lists in the same commit.
#[test]
fn differs_pinned() {
    let mut changed = Vec::new();
    for name in DIFFERS {
        let r = mali_ref::Reference::load(&format!("deqp_boolpix_{name}"));
        match mali_ref::compile_pair(&r.vs, &r.fs) {
            Ok(pair) => {
                if mali_ref::differences(&r, &pair).is_empty() {
                    changed.push(format!("{name}: now byte-equal; move it to LOWERED and drop its #[ignore]"));
                }
            }
            Err(e) => changed.push(format!("{name}: no longer compiles: {e}")),
        }
    }
    for name in LOWERED {
        let r = mali_ref::Reference::load(&format!("deqp_boolpix_{name}"));
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
