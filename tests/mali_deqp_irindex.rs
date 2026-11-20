// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Byte-compare of the Mali online compiler against the Mesa 26.2.1
//! references `tools/mali/testdata/deqp_irindex_*` (plan #387, item
//! irindex-ref, `docs/notes/mali-deqp-irindex.md`): one reference per shape
//! of the dEQP-GLES2 fragment shaders whose first refusal is the back end's
//! `Defect(IrIndex)` (187 cases at `3497799a`, HOST-ONLY count). Each pair is
//! a representative case's, verbatim from the host measure's `.qpa`;
//! `tests/mali_ref` does the compare. The references were drawn with
//! `tools/mali/deqp_ref.sh --instrument shim` (the command is in the note).
//!
//! Every reference meets the defect at the tip, so each byte-compare is
//! `#[ignore]`d with the site and the cause, and `refusals_pinned` (not
//! ignored) asserts the defect exactly ([`REFUSED`]). The fix (item
//! irindex-fix) moves a shape to [`LOWERED`] (or to [`DIFFERS`] when it
//! compiles with other words than Mesa's, or to another refusal text in
//! [`REFUSED`]) in the same commit and drops the `#[ignore]`.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_deqp_irindex
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-mali"))]
mod mali_ref;

/// The references the fix made byte-equal, without the `deqp_irindex_`
/// prefix: each has a byte-compare test below that is not ignored.
const LOWERED: &[&str] = &[];

/// The references that compile with other words than Mesa's.
const DIFFERS: &[&str] = &[];

const IR_INDEX: &str = "Valhall back end defect: an invariant of the pipeline did not hold (IrIndex)";

/// Every ignored reference with its whole `MaliError` message: all nine
/// meet `Defect(IrIndex)` at `3497799a` (the note's section 2 names the
/// site of each).
const REFUSED: &[(&str, &str)] = &[
    ("b2f16_pair", IR_INDEX),
    ("load_mat3", IR_INDEX),
    ("load_vec3_cmp", IR_INDEX),
    ("mat3_uniform_conv", IR_INDEX),
    ("mat4_pair_unread", IR_INDEX),
    ("pair_hi_unread", IR_INDEX),
    ("pair_lo_unread", IR_INDEX),
    ("sqrt_op_value", IR_INDEX),
    ("val16_abs_fold", IR_INDEX),
];

#[test]
#[ignore = "Defect(IrIndex): emit.rs:487, select_highp vectorises bvec2(v) -> vec2 into a two-lane B2f.16 that emit has no encoding for"]
fn b2f16_pair() {
    mali_ref::assert_matches("deqp_irindex_b2f16_pair");
}

#[test]
#[ignore = "Defect(IrIndex): nir_fs.rs:10690 (src32), a V2F32_TO_V2F16 pair reads a mat3 varying's load that was never emitted"]
fn load_mat3() {
    mali_ref::assert_matches("deqp_irindex_load_mat3");
}

#[test]
#[ignore = "Defect(IrIndex): nir_fs.rs:10690 (src32), a V2F32_TO_V2F16 pair reads a swizzled highp vec4 varying's load that was never emitted"]
fn load_vec3_cmp() {
    mali_ref::assert_matches("deqp_irindex_load_vec3_cmp");
}

#[test]
#[ignore = "Defect(IrIndex): nir_fs.rs:13112 (conv_of), mat3 uniform * float varying reads a varying conversion the walk never made"]
fn mat3_uniform_conv() {
    mali_ref::assert_matches("deqp_irindex_mat3_uniform_conv");
}

#[test]
#[ignore = "Defect(IrIndex): nir_fs.rs:8146 (index, from vectorise), a mediump mat4 column's lane pair holds a lane the colour never reads"]
fn mat4_pair_unread() {
    mali_ref::assert_matches("deqp_irindex_mat4_pair_unread");
}

#[test]
#[ignore = "Defect(IrIndex): nir_fs.rs:8146 (index, from src32), the bvec2(v) V2F32_TO_V2F16 pair reads v.y, which the colour never reads"]
fn pair_hi_unread() {
    mali_ref::assert_matches("deqp_irindex_pair_hi_unread");
}

#[test]
#[ignore = "Defect(IrIndex): nir_fs.rs:8146 (index, from load_lane), the bvec2(v) V2F32_TO_V2F16 pair reads v.x, which the colour never reads"]
fn pair_lo_unread() {
    mali_ref::assert_matches("deqp_irindex_pair_lo_unread");
}

#[test]
#[ignore = "Defect(IrIndex): nir_fs.rs:13198 (operand), the mediump colour walk reads an operation's value it has not made (sqrt of a swizzled highp varying)"]
fn sqrt_op_value() {
    mali_ref::assert_matches("deqp_irindex_sqrt_op_value");
}

#[test]
#[ignore = "Defect(IrIndex): nir_fs.rs:10697 (src32), an fp16 fabs of a uniform has no value when read (since 0fd576ea; compiled at ab9cde96)"]
fn val16_abs_fold() {
    mali_ref::assert_matches("deqp_irindex_val16_abs_fold");
}

/// Each refused reference is refused with exactly its text in [`REFUSED`]
/// (the whole `MaliError` message, not a substring: `Defect(IrIndex)` for
/// all nine today), each of [`DIFFERS`] compiles and differs, and the three
/// lists cover every `deqp_irindex_*` reference on disk. The fix flips this
/// test: a shape that compiles must move to [`LOWERED`] or [`DIFFERS`].
#[test]
fn refusals_pinned() {
    let mut on_disk: Vec<String> = std::fs::read_dir("tools/mali/testdata")
        .expect("tools/mali/testdata")
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter_map(|n| n.strip_prefix("deqp_irindex_")?.strip_suffix(".expect.json").map(str::to_string))
        .collect();
    on_disk.sort();
    let mut listed: Vec<&str> = LOWERED.iter().chain(DIFFERS).copied().chain(REFUSED.iter().map(|(n, _)| *n)).collect();
    listed.sort_unstable();
    assert_eq!(on_disk, listed, "a deqp_irindex_* reference has no test here");
    let mut changed = Vec::new();
    for (name, refused) in REFUSED {
        let r = mali_ref::Reference::load(&format!("deqp_irindex_{name}"));
        match mali_ref::compile_pair(&r.vs, &r.fs) {
            Ok(_) => changed.push(format!("{name}: now compiles; move it to LOWERED or DIFFERS")),
            Err(e) => {
                if e != *refused {
                    changed.push(format!("{name}: the refusal changed to {e:?}"));
                }
            }
        }
    }
    for name in DIFFERS {
        let r = mali_ref::Reference::load(&format!("deqp_irindex_{name}"));
        match mali_ref::compile_pair(&r.vs, &r.fs) {
            Ok(pair) => {
                if mali_ref::differences(&r, &pair).is_empty() {
                    changed.push(format!("{name}: now byte-equal; move it to LOWERED"));
                }
            }
            Err(e) => changed.push(format!("{name}: now refused: {e}")),
        }
    }
    assert!(changed.is_empty(), "{}", changed.join("\n"));
}
