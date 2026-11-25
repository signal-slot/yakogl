// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Byte-compare of the Mali online compiler against the Mesa 26.2.1
//! references `tools/mali/testdata/deqp_irindex3_*` (plan #402, item
//! ref-irindex3): matrix shapes of the `Defect(IrIndex)` cases of the
//! 8ff01055 whole-tree run
//! (`tools/cts/log/2026-11-24-t6-gles2-S{1..4}-8ff01055/refusals.tsv`) that
//! `deqp_irindex_*` (`tests/mali_deqp_irindex.rs`), `deqp_irindex2_*`
//! (`tests/mali_deqp_irindex2.rs`) and `deqp_defects*` do not draw.
//!
//! The refusals.tsv has no trigonometry case among its IrIndex, LaneCycle and
//! Encode rows: the only non-matrix, non-swizzle, non-conversion rows are
//! `shaders.random.*` (basic_expression 4 and 58, comparison_ops 29, covered
//! by `deqp_irindex2_op_val`, `deqp_irindex_*` and `deqp_defects_lanecycle_fs`),
//! so no trigonometry reference is drawn.
//!
//! Shape table (the matrix.* IrIndex rows are 40 of the 206 cases; the
//! first-refusal column was measured on the host at the tip 53b04621 with
//! `tests/mali_ref::compile_pair`; LaneCycle did not fire on any of these
//! programs):
//!
//! | shape | matrix.* cases (approximate, counted by eye from refusals.tsv) | reference | tip refusal |
//! |---|--:|---|---|
//! | mat4 + mat4, both varyings (`add/sub.dynamic_{lowp,mediump,highp}_mat4_mat4`) | 9 | **`deqp_irindex3_mat4_mat4`** | IrIndex |
//! | `res += mat4` (`add_assign/sub_assign.dynamic_*_mat4`) | 6 | **`deqp_irindex3_mat4_addassign`** | IrIndex |
//! | `++m` of a mat4 (`pre/post_increment/decrement.dynamic_{lowp,mediump}_mat4`) | 8 | **`deqp_irindex3_mat4_preinc`** | IrIndex |
//! | mat3 + float (`add/sub.dynamic_*_mat3_float`) | 4 | none: compiles at the tip | none |
//! | mat3 * float (`mul.*_mat3_float`) | 7 | none: refused as the algebraic rewrite (`matrix.mul.*`) | not IrIndex |
//! | mat4 + float (`add/sub/mul.dynamic_*_mat4_float`) | 10 | `deqp_irindex_mat4_pair_unread` | covered |
//!
//! HAND-WRITTEN SOURCES: the host-only dEQP build (VK-GL-CTS) was not
//! available on this machine, so the three shader pairs are written in the
//! form of dEQP's matrix shaders (a `mediump` varying pair per operand, a
//! sum of the column swizzles to `gl_FragColor`), not taken verbatim from a
//! `.qpa`; their `.license` files say so. The dEQP cases they stand for
//! should be re-confirmed by the item that fixes IrIndex.
//!
//! The references were drawn with `tools/mali/deqp_ref.sh <in>.vert <in>.frag
//! tools/mali/testdata/deqp_irindex3_<shape> --instrument shim`
//! (`$CONNECTED_DEPS_DIR/mesa-shim`); `tests/mali_ref` does the compare. The
//! fix (plan #402 item fix-irindex) moves a shape to [`LOWERED`] (or
//! [`DIFFERS`]) in the same commit and drops its `#[ignore]`.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_deqp_irindex3
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-mali"))]
mod mali_ref;

/// The references the fix made byte-equal, without the `deqp_irindex3_`
/// prefix: each has a byte-compare test below that is not ignored.
const LOWERED: &[&str] = &[];

/// The references that compile with other words than Mesa's.
const DIFFERS: &[&str] = &[];

const IR_INDEX: &str = "Valhall back end defect: an invariant of the pipeline did not hold (IrIndex)";

/// Every ignored reference with its whole `MaliError` message.
const REFUSED: &[(&str, &str)] = &[("mat4_mat4", IR_INDEX), ("mat4_addassign", IR_INDEX), ("mat4_preinc", IR_INDEX)];

#[test]
#[ignore = "Defect(IrIndex): a mat4 + mat4 of two mediump varyings"]
fn mat4_mat4() {
    mali_ref::assert_matches("deqp_irindex3_mat4_mat4");
}

#[test]
#[ignore = "Defect(IrIndex): mat4 += mat4 of mediump varyings"]
fn mat4_addassign() {
    mali_ref::assert_matches("deqp_irindex3_mat4_addassign");
}

#[test]
#[ignore = "Defect(IrIndex): ++ of a mediump mat4 varying"]
fn mat4_preinc() {
    mali_ref::assert_matches("deqp_irindex3_mat4_preinc");
}

/// Each refused reference is refused with exactly its text in [`REFUSED`]
/// (the whole `MaliError` message, not a substring), each of [`DIFFERS`]
/// compiles and differs, and the three lists cover every `deqp_irindex3_*`
/// reference on disk. The fix flips this test: a shape that compiles must
/// move to [`LOWERED`] or [`DIFFERS`].
#[test]
fn refusals_pinned() {
    let mut on_disk: Vec<String> = std::fs::read_dir("tools/mali/testdata")
        .expect("tools/mali/testdata")
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter_map(|n| n.strip_prefix("deqp_irindex3_")?.strip_suffix(".expect.json").map(str::to_string))
        .collect();
    on_disk.sort();
    let mut listed: Vec<&str> = LOWERED.iter().chain(DIFFERS).copied().chain(REFUSED.iter().map(|(n, _)| *n)).collect();
    listed.sort_unstable();
    assert_eq!(on_disk, listed, "a deqp_irindex3_* reference has no test here");
    let mut changed = Vec::new();
    for (name, refused) in REFUSED {
        let r = mali_ref::Reference::load(&format!("deqp_irindex3_{name}"));
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
        let r = mali_ref::Reference::load(&format!("deqp_irindex3_{name}"));
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
