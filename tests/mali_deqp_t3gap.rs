// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Byte-compare of the `tools/mali/testdata/deqp_t3gap_*` references: the
//! Mesa 26.2.1 references that a step of `docs/notes/mali-lowering-schedule.md`
//! section 5.4 (Train 3) lacked (plan #387 item `train3-refgaps`,
//! `docs/notes/mali-train3-refs.md`). Each is drawn on Mesa 26.2.1's G610
//! drm-shim with `tools/mali/deqp_ref.sh <vert> <frag> <prefix> --instrument shim`.
//!
//! ```text
//! reference                    case (dEQP-GLES2.functional.shaders.)             step
//! vsunif_array_member_dynidx   struct.local.array_member_dynamic_index_vertex    vsunif V3
//! vsunif_struct_array_dynidx   struct.local.struct_array_dynamic_index_vertex    vsunif V3
//! vsunif_nested_struct_array_dynidx
//!                              struct.local.nested_struct_array_dynamic_index_vertex   vsunif V3
//! mixprec2_rnd_tex145          random.texture.fragment.145                     mixprec2 M2-3
//! ```
//!
//! [`STATE`] pins each reference's state at the tip: `Ok(true)` byte-equal,
//! `Ok(false)` compiles with other words than Mesa's, `Err(text)` refused with
//! the whole `MaliError` text. The step that changes a state moves it here
//! and drops the test's `#[ignore]` in the same commit.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_deqp_t3gap
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-mali"))]
mod mali_ref;

/// `mixprec2_rnd_tex145`: the mixed-precision text (M2-3's `Fneg` operand).
const MIXED: &str = "Valhall: a fragment shader that mixes highp and mediump or lowp float values (Mesa's fp32 and fp16) is not lowered yet";

const STATE: &[(&str, Result<bool, &str>)] = &[
    ("mixprec2_rnd_tex145", Err(MIXED)),
    ("vsunif_array_member_dynidx", Ok(false)),
    ("vsunif_nested_struct_array_dynidx", Ok(false)),
    ("vsunif_struct_array_dynidx", Ok(false)),
];

fn reference(name: &str) -> mali_ref::Reference {
    mali_ref::Reference::load(&format!("deqp_t3gap_{name}"))
}

fn differences(name: &str) -> Result<Vec<String>, String> {
    let r = reference(name);
    let pair = mali_ref::compile_pair(&r.vs, &r.fs)?;
    Ok(mali_ref::differences_members(&r, &pair, false))
}

fn assert_matches(name: &str) {
    let d = differences(name).unwrap_or_else(|e| panic!("deqp_t3gap_{name}: the pair is refused: {e}"));
    assert!(d.is_empty(), "deqp_t3gap_{name}: differs from its reference:\n{}", d.join("\n"));
}

/// Every reference on disk is pinned, and each is in its pinned state.
#[test]
fn state_pinned() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tools/mali/testdata");
    let mut on_disk: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .filter_map(|e| {
            let f = e.unwrap().file_name().into_string().unwrap();
            f.strip_prefix("deqp_t3gap_")?.strip_suffix(".expect.json").map(str::to_string)
        })
        .collect();
    on_disk.sort();
    let listed: Vec<String> = STATE.iter().map(|(n, _)| n.to_string()).collect();
    assert_eq!(on_disk, listed, "every deqp_t3gap_* reference has a pin (sorted)");
    let mut changed = Vec::new();
    for (name, want) in STATE {
        let now = differences(name).map(|d| d.is_empty());
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

/// Compiles. `vs.varying` is 29 words both; the first differing instruction
/// is `[0]`: Mesa starts with `MOV.i32 r0, 0x0` and `ICMP_OR.s32.lt.m1` on
/// the index uniform before the attribute load, ours loads first and builds
/// an `ICMP_OR.u32.eq.m1` / `CSEL.u32.eq` select tree (step V3).
#[test]
#[ignore = "compiles but differs: vs.varying instruction 0 (Mesa's ICMP_OR.s32.lt.m1 select tree order against our ICMP_OR.u32.eq / CSEL tree; step V3)"]
fn vsunif_array_member_dynidx() {
    assert_matches("vsunif_array_member_dynidx");
}

/// Compiles, and `vs.position` is 24 words ours against Mesa's 70: Mesa keeps
/// the nested struct array in thread-local storage (`STORE.*
/// thread_local_pointer`, step V4's mechanism); ours is a different
/// 24-word sequence.
#[test]
#[ignore = "compiles but differs: vs.position 24 words ours, 70 Mesa's (Mesa's thread-local storage of the nested struct array; step V4)"]
fn vsunif_nested_struct_array_dynidx() {
    assert_matches("vsunif_nested_struct_array_dynidx");
}

/// Compiles. `vs.varying` is 38 words both; the first differing instruction
/// is `[0]`, the same select-tree order as `array_member_dynidx` (step V3).
#[test]
#[ignore = "compiles but differs: vs.varying instruction 0 (Mesa's ICMP_OR.s32.lt.m1 select tree order against our ICMP_OR.u32.eq / CSEL tree; step V3)"]
fn vsunif_struct_array_dynidx() {
    assert_matches("vsunif_struct_array_dynidx");
}

/// Refused with [`MIXED`]: a 32-bit `Fneg` operand of the fp32 product at
/// the return site (`mali-mixprec2-lowering.md` M2-3; `random.texture.fragment.145`
/// has no `.qpa` of the tip's host measure in the tree, so the source is the
/// null-target `deqp-gles2` run of `docs/notes/mali-train3-refs.md`).
#[test]
#[ignore = "refused: a fragment shader that mixes highp and mediump or lowp float values (M2-3, the Fneg operand)"]
fn mixprec2_rnd_tex145() {
    assert_matches("mixprec2_rnd_tex145");
}
