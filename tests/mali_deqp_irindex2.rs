// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Byte-compare of the Mali online compiler against the Mesa 26.2.1
//! references `tools/mali/testdata/deqp_irindex2_*` (plan #402, item
//! ref-irindex2): the shapes of the `Defect(IrIndex)`, `Defect(LaneCycle)`
//! and `Defect(Encode)` cases of the 8ff01055 whole-tree run
//! (`tools/cts/log/2026-11-24-t6-gles2-S{1..4}-8ff01055/refusals.tsv`: 206 +
//! 9 + 6 = 221 cases) that the nine `deqp_irindex_*` references
//! (`tests/mali_deqp_irindex.rs`, `docs/notes/mali-deqp-irindex.md`) and the
//! four `deqp_defects_*` references (`tests/mali_deqp_defects.rs`) do not
//! cover.
//!
//! HOST-ONLY re-measure at the tip 51af40b3 (`tools/mali/deqp-host-measure.sh`,
//! features host-test,gpu-mali,shaderc-online,no-shader-table, `--cases` the
//! 221 cases), every site from a scratch copy of the tip whose
//! `Defect::IrIndex` constructions print their `file:line` and a backtrace.
//! What the 221 cases meet at the tip, by shape:
//!
//! | shape (first refusal at the tip) | cases | site (file:line, 51af40b3) | reference |
//! |---|--:|---|---|
//! | `bvec2(v)` pair, the second lane unread (conversions `bvecN_to_*`, `ivecN_to_int`) | 64 | `nir_fs.rs:9642` `index` from `src32` | `deqp_irindex_pair_hi_unread` |
//! | the same, the first lane unread (swizzles `vector_swizzles`) | 60 | `nir_fs.rs:9642` `index` from `load_lane` | `deqp_irindex_pair_lo_unread` |
//! | mediump/lowp mat4 column pair (`shaders.matrix.*dynamic_*mat4*`) | 20 | `nir_fs.rs:9642` `index` from `split_lanes` | `deqp_irindex_mat4_pair_unread` |
//! | a matrix or vector built from conversions: a varying's load never emitted | 18 | `nir_fs.rs:12702` `src32` | `deqp_irindex_load_mat3` |
//! | two-lane `B2f.16` (`bvecN` into `vec`/`mat`) | 15 | `emit.rs:489` `emit` from `row` | `deqp_irindex_b2f16_pair` |
//! | an fp16 fabs of a uniform (`preprocessor.*`, `datatypes.float_mat4`) | 5 | `nir_fs.rs:12709` `src32` | `deqp_irindex_val16_abs_fold` |
//! | `Defect(LaneCycle)` (`shaders.random.*`) | 8 | `nir_fs::lane_tree` | `deqp_defects_lanecycle_fs` |
//! | `Defect(Encode)` (`attribute_location.bind_aliasing.max_cond_*`) | 6 | vertex shader encode | `deqp_defects_encode_max_cond_float` |
//! | highp vec3 compare (past the defect: mixes highp and mediump) | 6 | `nir_fs.rs:14139` | `deqp_irindex_load_vec3_cmp` |
//! | `P::Op`'s value is not there when an operand reads it (a constant-folded mediump expression) | 2 | `nir_fs.rs:15732` `operand` from `op`, `select` | **`deqp_irindex2_op_val`** |
//! | not a defect now: algebraic-rewrite refusal (`matrix.mul.*`, `random.4`) | 8 | `nir_fs.rs:7678` (1), no site (7) | `deqp_irindex_mat3_uniform_conv` (the uniform one); the rest are the misc family |
//! | no refusal on the host (compile_pair succeeds) | 9 | `motion.rs:174` only builds the value | none |
//!
//! Only `op_val` is uncovered, so it is the only reference drawn: the
//! `dEQP-GLES2.functional.shaders.random.scalar_conversion.fragment.56` pair
//! (its `combined.56` partner has the same fragment shader), whose first
//! refusal is `Defect(IrIndex)` at the `operand` site that `sqrt_op_value`
//! met at 3497799a (that reference is past it now, [`MIXED`] in
//! `tests/mali_deqp_irindex.rs`). Its shape: a mediump fragment shader whose
//! colour reads constants, two int uniforms and a dead ivec3 assignment.
//!
//! The reference was drawn with
//! `MESA_SHIM_WORK="$CONNECTED_DEPS_DIR/mesa-shim" tools/mali/deqp_ref.sh
//! <case>.vert <case>.frag tools/mali/testdata/deqp_irindex2_op_val --instrument shim`;
//! `tests/mali_ref` does the compare. The fix (plan #402 item fix-irindex)
//! moves a shape to [`LOWERED`] (or [`DIFFERS`]) in the same commit and drops
//! the `#[ignore]`.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_deqp_irindex2
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-mali"))]
mod mali_ref;

/// The references the fix made byte-equal, without the `deqp_irindex2_`
/// prefix: each has a byte-compare test below that is not ignored.
const LOWERED: &[&str] = &[];

/// The references that compile with other words than Mesa's.
const DIFFERS: &[&str] = &[];

const IR_INDEX: &str = "Valhall back end defect: an invariant of the pipeline did not hold (IrIndex)";

/// Every ignored reference with its whole `MaliError` message.
const REFUSED: &[(&str, &str)] = &[("op_val", IR_INDEX)];

#[test]
#[ignore = "Defect(IrIndex): nir_fs.rs:15732 (operand, from op and select), a P::Op operand has no value when read"]
fn op_val() {
    mali_ref::assert_matches("deqp_irindex2_op_val");
}

/// Each refused reference is refused with exactly its text in [`REFUSED`]
/// (the whole `MaliError` message, not a substring), each of [`DIFFERS`]
/// compiles and differs, and the three lists cover every `deqp_irindex2_*`
/// reference on disk. The fix flips this test: a shape that compiles must
/// move to [`LOWERED`] or [`DIFFERS`].
#[test]
fn refusals_pinned() {
    let mut on_disk: Vec<String> = std::fs::read_dir("tools/mali/testdata")
        .expect("tools/mali/testdata")
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter_map(|n| n.strip_prefix("deqp_irindex2_")?.strip_suffix(".expect.json").map(str::to_string))
        .collect();
    on_disk.sort();
    let mut listed: Vec<&str> = LOWERED.iter().chain(DIFFERS).copied().chain(REFUSED.iter().map(|(n, _)| *n)).collect();
    listed.sort_unstable();
    assert_eq!(on_disk, listed, "a deqp_irindex2_* reference has no test here");
    let mut changed = Vec::new();
    for (name, refused) in REFUSED {
        let r = mali_ref::Reference::load(&format!("deqp_irindex2_{name}"));
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
        let r = mali_ref::Reference::load(&format!("deqp_irindex2_{name}"));
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
