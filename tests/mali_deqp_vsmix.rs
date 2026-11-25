// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Byte-compare of every `tools/mali/testdata/deqp_vsmix_*` reference (Mesa
//! 26.2.1 on drm-shim, plan #402 item `ref-vsmix3`,
//! `docs/notes/mali-deqp-vsmix.md`) against the Mali online compiler, in
//! the form of `tests/mali_deqp_vsunif.rs`. They are the vertex-shader loop
//! cases of `dEQP-GLES2.functional.shaders.loops.*` that unrolling unmasks
//! into another refusal. Plan #402 `unmask-forecast` named two such
//! refusals: an int uniform, "a uniform other than a float, vec2, vec3,
//! vec4 or mat4" (vsunif), and "a constant operation on or to a value
//! that is not a finite normal float" (vsconst). Each reference is refused
//! today with "a loop in a vertex shader" (`MaliError::LoopVertex`). It is
//! `#[ignore]`d with that refusal. `refusals_pinned` (not ignored) holds
//! the exact text, so the lowering that takes the loop moves its pin in the
//! same commit.
//!
//! The shapes, HOST-ONLY at `22c5fc5f` (`tools/mali/deqp-host-measure.sh
//! --cases` over the 196 cases whose `refusals.tsv` text in the four slices
//! `tools/cts/log/2026-11-24-t6-gles2-S{1..4}-8ff01055/` holds "a loop in a
//! vertex shader", VK-GL-CTS `05ca62dc`). 145 `shaders.loops.*` cases still
//! stop at the loop on the host. 72 of them read a `uniform mediump int`
//! beyond its declaration. None holds a float literal that is not finite and
//! normal. Each of the 72 programs was drawn on Mesa's drm-shim
//! (`tools/mali/drm_shim_ref.sh run`). Mesa keeps the loop in 60 of them
//! (step 2, `deqp_vsloop2_*` and `deqp_loops2_*`). It deletes the loop and
//! the int uniform in 8. It unrolls the loop to one guarded pass and still
//! reads the int uniform in 4. Only those 4 are "unrolling unmasks vsunif".
//! Their shapes are below. Fail is the board's status:
//!
//! ```text
//! shape                     cases  Fail  example case (dEQP-GLES2.functional.shaders.loops.)  reference
//! uni_unconditional_break       2     2  for_uniform_iterations.unconditional_break_vertex    covered by deqp_loops2_vs_for_uni_unconditional_break
//!                                        (while_uniform_iterations: the same Mesa bytes)
//! dyn_unconditional_break       2     2  for_dynamic_iterations.unconditional_break_vertex    deqp_vsmix_dyn_unconditional_break
//!                                        (while_dynamic_iterations: the same Mesa bytes)
//! total                         4     4
//! ```
//!
//! The reference is the case's program, verbatim from the .qpa, drawn with
//! `tools/mali/deqp_ref.sh <vert> <frag> <prefix> --instrument shim`
//! (gl_ref.py `--uniform-values`).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_deqp_vsmix
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-mali"))]
mod mali_ref;

/// The refusal every reference here is refused with today.
const LOOP_VERTEX: &str = "Valhall: a loop in a vertex shader is not lowered yet";

/// Every refused reference with its whole `MaliError` message, without the
/// `deqp_vsmix_` prefix.
const REFUSED: &[(&str, &str)] = &[("dyn_unconditional_break", LOOP_VERTEX)];

fn reference(name: &str) -> mali_ref::Reference {
    mali_ref::Reference::load(&format!("deqp_vsmix_{name}"))
}

fn assert_matches(name: &str) {
    let r = reference(name);
    let pair =
        mali_ref::compile_pair(&r.vs, &r.fs).unwrap_or_else(|e| panic!("deqp_vsmix_{name}: the pair is refused: {e}"));
    let d = mali_ref::differences(&r, &pair);
    assert!(d.is_empty(), "deqp_vsmix_{name}: differs from its reference:\n{}", d.join("\n"));
}

#[test]
fn refusals_pinned() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tools/mali/testdata");
    let mut on_disk: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .filter_map(|e| {
            let f = e.unwrap().file_name().into_string().unwrap();
            f.strip_prefix("deqp_vsmix_")?.strip_suffix(".expect.json").map(str::to_string)
        })
        .collect();
    on_disk.sort();
    let mut listed: Vec<String> = REFUSED.iter().map(|(n, _)| n.to_string()).collect();
    listed.sort();
    assert_eq!(on_disk, listed, "every deqp_vsmix_* reference has a test and a pin");
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

/// `for (mediump int i = 0; i < one*ui_three; i++) { res = res.yzwx; break; }`
/// with `mediump int one = int(a_one + 0.5)`. Mesa unrolls it to one pass
/// under `0 < one*ui_three`: `F16_TO_S32` of the fp16 `a_one + 0.5`, an
/// `IMUL.i32` by `ui_three` (`u5.w1`), an `ICMP_OR.s32.ge` and three
/// `MUX.v2i16` of the fp16 lanes (25 words of `vs.varying`). HOST-ONLY at
/// `22c5fc5f`: the hand-unrolled `if (0 < one*ui_three)` form compiles, and
/// the first difference is instruction 1 (ours `FADD_IMM.f32` and
/// `F32_TO_S32`, Mesa's fp16 `FADD.f32 r0.h0` and `F16_TO_S32`; 19 words
/// ours). So past the loop it meets no int-uniform refusal. The work left is
/// the mediump narrowing.
#[test]
#[ignore = "refused: a loop in a vertex shader (one guarded pass of an attribute-scaled int-uniform bound), lowered by lower-vsloop"]
fn dyn_unconditional_break() {
    assert_matches("dyn_unconditional_break");
}
