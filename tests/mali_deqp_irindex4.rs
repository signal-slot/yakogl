// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Byte-compare of the Mali online compiler against the Mesa 26.2.1
//! references `tools/mali/testdata/deqp_irindex4_*` (plan #402, item
//! ref-irindex4). They cover the two `Defect(IrIndex)` shapes U1 and U2,
//! which `deqp_irindex_*`, `deqp_irindex2_*`, `deqp_irindex3_*` and
//! `deqp_defects*` do not cover (docs/notes/mali-deqp-irindex4.md).
//!
//! Shape table. Every row was measured HOST-ONLY at c28470db with
//! `tests/mali_ref::compile_pair`. The "8ff01055" column is the case's text in
//! `tools/cts/log/2026-11-24-t6-gles2-S{1..4}-8ff01055/refusals.tsv`.
//!
//! | shape | site at c28470db | dEQP cases (fragment) | 8ff01055 | reference |
//! |---|---|---|---|---|
//! | U1: a `?` on `index` in a `vectorise` match guard fails the shader instead of making the arm not apply | `nir_fs.rs:11014` (from `vectorise`'s product arm `:10965`, `pair_node` `:11791`) | `matrix.div.dynamic_{lowp,mediump}_mat4_mat4`, `matrix.div_assign.dynamic_{lowp,mediump}_mat4` (4) | "arithmetic other than +, -, *" | **`deqp_irindex4_u1_div_mediump`**, **`deqp_irindex4_u1_divassign_mediump`** |
//! | U2: the value of a 16-bit `B2f` lane the colour reads is not there | `nir_fs.rs:13498` (`src32`'s 16-bit arm; `defect` built at `:13485`) | `operator.bool_compare.not.{bvec3,bvec4}` (2) | IrIndex | **`deqp_irindex4_u2_not_bvec3`**, **`deqp_irindex4_u2_not_bvec4`** |
//!
//! The lowp U1 cases draw the same Mesa words as the mediump ones, apart from
//! the precision names in their sources, so they have no reference of their
//! own. `div` and `div_assign` also draw the same words. Both are kept because
//! they are different dEQP programs.
//!
//! The other 20 IrIndex cases of dEQP's matrix fragment set at c28470db
//! (`add`, `sub`, `add_assign`, `sub_assign`, `pre_*`, `post_*` of a
//! `dynamic_{lowp,mediump}_mat4`) fail in `split_lanes` (`nir_fs.rs:10491`).
//! That is the root cause 5.2 of docs/notes/mali-deqp-irindex.md, already
//! drawn by `deqp_irindex_mat4_pair_unread`. The IrIndex conversions
//! (`matrix_to_matrix.mat3_*`, `vector_combine.vec3_vec3_vec3_to_mat3`) and
//! `functions.datatypes.float_mat3` are its 5.3 (`load_mat3`).
//!
//! RECONSTRUCTED SOURCES. This machine had no host dEQP build, so no `.qpa`
//! was available. Each pair was written by a script that follows dEQP's
//! generator (`ShaderMatrixCase::init`, `ShaderOperatorCase`; VK-GL-CTS
//! 05ca62d). The same script reproduces `deqp_irindex_b2f16_pair`, which was
//! taken from a `.qpa`, byte for byte. Their `.license` files say so.
//!
//! [`MINIMAL`] holds the smallest pair per shape: a shader that refuses next to
//! one that compiles. `minimal_pairs` pins both.
//!
//! The references were drawn with `tools/mali/deqp_ref.sh <in>.vert <in>.frag
//! tools/mali/testdata/deqp_irindex4_<shape> --instrument shim`
//! (`$CONNECTED_DEPS_DIR/mesa-shim`), and their `.nir.sha` with
//! `tools/mali/nir_refs.sh 'deqp_irindex4_*'`. `tests/mali_ref` does the
//! compare. The fix (plan #402 item fix-irindex) moves a shape to [`LOWERED`]
//! (or [`DIFFERS`]) in the same commit, drops its `#[ignore]` and flips its
//! row of [`MINIMAL`].
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_deqp_irindex4
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-mali"))]
mod mali_ref;

/// The references the fix made byte-equal, without the `deqp_irindex4_`
/// prefix: each has a byte-compare test below that is not ignored.
const LOWERED: &[&str] = &[];

/// The references that compile with other words than Mesa's.
const DIFFERS: &[&str] = &[];

const IR_INDEX: &str = "Valhall back end defect: an invariant of the pipeline did not hold (IrIndex)";

/// Every ignored reference with its whole `MaliError` message.
const REFUSED: &[(&str, &str)] = &[
    ("u1_div_mediump", IR_INDEX),
    ("u1_divassign_mediump", IR_INDEX),
    ("u2_not_bvec3", IR_INDEX),
    ("u2_not_bvec4", IR_INDEX),
];

#[test]
#[ignore = "Defect(IrIndex) U1: nir_fs.rs:11014, the `?` in vectorise's guard (mediump mat4 / uniform mat4)"]
fn u1_div_mediump() {
    mali_ref::assert_matches("deqp_irindex4_u1_div_mediump");
}

#[test]
#[ignore = "Defect(IrIndex) U1: nir_fs.rs:11014, the `?` in vectorise's guard (mediump mat4 /= uniform mat4)"]
fn u1_divassign_mediump() {
    mali_ref::assert_matches("deqp_irindex4_u1_divassign_mediump");
}

#[test]
#[ignore = "Defect(IrIndex) U2: nir_fs.rs:13498, a 16-bit B2f lane of the colour has no value (vec3(not(bvec3)))"]
fn u2_not_bvec3() {
    mali_ref::assert_matches("deqp_irindex4_u2_not_bvec3");
}

#[test]
#[ignore = "Defect(IrIndex) U2: nir_fs.rs:13498, a 16-bit B2f lane of the colour has no value (vec4(not(bvec4)))"]
fn u2_not_bvec4() {
    mali_ref::assert_matches("deqp_irindex4_u2_not_bvec4");
}

/// Each refused reference is refused with exactly its text in [`REFUSED`]
/// (the whole `MaliError` message, not a substring), each of [`DIFFERS`]
/// compiles and differs, and the three lists cover every `deqp_irindex4_*`
/// reference on disk. The fix flips this test: a shape that compiles must
/// move to [`LOWERED`] or [`DIFFERS`].
#[test]
fn refusals_pinned() {
    let mut on_disk: Vec<String> = std::fs::read_dir("tools/mali/testdata")
        .expect("tools/mali/testdata")
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter_map(|n| n.strip_prefix("deqp_irindex4_")?.strip_suffix(".expect.json").map(str::to_string))
        .collect();
    on_disk.sort();
    let mut listed: Vec<&str> = LOWERED.iter().chain(DIFFERS).copied().chain(REFUSED.iter().map(|(n, _)| *n)).collect();
    listed.sort_unstable();
    assert_eq!(on_disk, listed, "a deqp_irindex4_* reference has no test here");
    let mut changed = Vec::new();
    for (name, refused) in REFUSED {
        let r = mali_ref::Reference::load(&format!("deqp_irindex4_{name}"));
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
        let r = mali_ref::Reference::load(&format!("deqp_irindex4_{name}"));
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

/// The vertex shader of every [`MINIMAL`] pair.
const MINIMAL_VS: &str = "attribute highp vec4 a_position;
attribute mediump vec4 a_m;
varying mediump vec4 m;
void main() { gl_Position = a_position; m = a_m; }
";

/// Per shape: the smallest fragment shader that refuses (with its whole
/// message), next to one that compiles. Each pair differs only in what the
/// shape needs (docs/notes/mali-deqp-irindex4.md).
const MINIMAL: &[(&str, &str, &str, &str)] = &[
    (
        "U1",
        "precision mediump float; varying vec4 m; uniform vec4 u;
         void main() { gl_FragColor = vec4((m * u).zwx, 1.0); }",
        IR_INDEX,
        "precision mediump float; varying vec4 m; uniform vec4 u;
         void main() { gl_FragColor = vec4((m * u).xyz, 1.0); }",
    ),
    (
        "U2",
        "precision mediump float; varying vec4 m;
         void main() { bool b = m.x > 0.0; bool c = m.y > 0.0; gl_FragColor = vec4(float(b), float(c), 0.0, 1.0); }",
        IR_INDEX,
        "precision mediump float; varying vec4 m;
         void main() { bool b = m.x > 0.0; gl_FragColor = vec4(float(b), 0.0, 0.0, 1.0); }",
    ),
];

/// Each shape's minimal pair: the first shader is refused with exactly its
/// text, the second compiles. The fix flips the first; the row then goes.
#[test]
fn minimal_pairs() {
    let mut changed = Vec::new();
    for (shape, refuses, text, compiles) in MINIMAL {
        match mali_ref::compile_pair(MINIMAL_VS, refuses) {
            Ok(_) => changed.push(format!("{shape}: the refusing shader now compiles")),
            Err(e) => {
                if e != *text {
                    changed.push(format!("{shape}: the refusal changed to {e:?}"));
                }
            }
        }
        if let Err(e) = mali_ref::compile_pair(MINIMAL_VS, compiles) {
            changed.push(format!("{shape}: the compiling shader is now refused: {e}"));
        }
    }
    assert!(changed.is_empty(), "{}", changed.join("\n"));
}
