// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! `deqp_colourpair_*` on the Mali compiler: the byte-compares of the Mesa
//! 26.2.1 drm-shim programs for the colour-pair shapes — `gl_FragColor` as a
//! pair of a constant then a varying lane, or of a varying lane and 0.0 —
//! against `tools/mali/testdata/deqp_colourpair_*.{vert,frag,expect.json}`
//! (`tests/mali_ref`, the `gm2_*` format). The shapes and what each
//! reference pins: section 10 and section 16 of
//! `docs/notes/mali-deqp-families.md` and `docs/notes/mali-deqp-colourpair.md`.
//!
//! `ck_lane` and `lane_zero_load` are byte-equal since plan #377's
//! lower-intval-2: Mesa keeps the constant lanes of a mediump `color` beside
//! a computed one in the vertex shader (the vertex walk's `kept` lanes,
//! `docs/notes/mali-deqp-intval.md` section 8), so their fragment shaders
//! have no colour pair at all. `ck_lane_mediump` and `rnd_vs_mediump_kept`
//! compile since then too and differ in the vertex program alone ([`DIFFERS`]:
//! Mesa adds the mediump attributes' lanes at fp16, the vertex walk at fp32).
//!
//! Ten more are byte-equal since plan #377's lower-varylink
//! (`docs/notes/mali-deqp-colourpair.md`, "Lowering"): the pairs that are a
//! word of the colour, `MKVEC.v2i16 <constant>, lane` (`ck_lane_highpcolour`,
//! `rnd_fs_ck_dup`, `rnd_fs_ck_lz`, `rnd_fs_kword_ck`, `rnd_fs_zword_zk`,
//! `rnd_vs_ck_lz`, `rnd_vs_zk_lz`) and `U16_TO_U32` of a lane and 0.0
//! (`default_attrib`), and the two whose vertex shader's lanes the link
//! keeps as components of a vector operation (`rnd_vs_uniform_flat`,
//! `rnd_vs_vecop_fs_uniform`).
//!
//! `rnd_vs_vecop_kept` was refused at the partial-varying link until the
//! program's vertex shader was built for what its fragment shader reads
//! (`docs/notes/mali-product-route.md`): byte-equal since.
//!
//! Three more are byte-equal since plan #377's spike-colourpair-rest
//! (`docs/notes/mali-deqp-colourpair.md`, "Lowering"): a constant then a
//! load's lane as a product's operand (`rnd_fs_ck_product`), a uniform's
//! sum and 0.0 and a constant then a uniform's component as words of the
//! colour (`rnd_fs_uniform_tz_ck`), and a mixed constructor's constant
//! lanes across two 16-bit pairs, each its own lane's rule
//! (`rnd_fs_zk_tree`). `rnd_fs_ck_madd` compiles to Mesa's operations and
//! values since then, with other words ([`FS_DIFFERS`]): Mesa makes the
//! product's two constant pairs with `IADD_IMM`, we push them to the FAU.
//!
//! No reference is refused now ([`REFUSALS`] is empty); `refusals_pinned`
//! asserts each ignored reference's state today — a refusal's
//! `MaliError::message` without its `Valhall: ` prefix and its ` is not
//! lowered yet` suffix, or the stage that differs — so a state that moves
//! breaks its pin, and its test is what starts running.
//!
//! What each byte-compare covers once lowered: the base references compare
//! every region (their lowering is a link rule, the vertex varying program
//! included). The `rnd_*` references (the `shaders.random.*` and
//! `indexing.tmp_array` pairs of section 16) compare `fs.fragment` for all,
//! and the vertex program as well for the `rnd_vs_*` ones: the `rnd_fs_*`
//! vertex programs are plain attribute forwards, no colour-pair work.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_deqp_colourpair
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-mali"))]
mod mali_ref;

/// Every refused reference and its pinned first refusal at this tip, the
/// table behind [`refusals_pinned`], in `MaliError::message`'s middle form
/// (without the `Valhall: ` prefix and the ` is not lowered yet` suffix):
/// none since spike-colourpair-rest. A lowering that refuses one again
/// adds it here, its test `#[ignore]`d with the same text
/// (`MaliError::ColourPair`'s "a colour pair of a constant then a varying
/// lane", `MaliError::ColourZeroHalf`'s "a colour pair of a varying lane and
/// 0.0 (Mesa's U16_TO_U32)").
const REFUSALS: &[(&str, &str)] = &[];

/// The references that compile with other words than Mesa's, in the vertex
/// program alone: the mediump attributes' lanes are added at fp16 in Mesa
/// (`FADD.v2f16`, `F16_TO_F32`), at fp32 in the vertex walk (the mixprec
/// family's vertex side). Their constant colour lanes are stored as Mesa
/// stores them, and their fragment programs are Mesa's.
const DIFFERS: &[&str] = &["deqp_colourpair_ck_lane_mediump", "deqp_colourpair_rnd_vs_mediump_kept"];

/// The references that compile with other words than Mesa's in the
/// fragment program, the same operations on the same values:
/// `rnd_fs_ck_madd`'s products by the constant pairs `(0.25, 1.0)` and
/// `(-1.0, -2.0)`, which Mesa makes with `IADD_IMM.i32 r1, 0x0,
/// #0x3C003400` and `#0xC000BC00` (the FAU holds the colour's `-3.0` and
/// `-1.5` alone) and we push to the FAU (`MOV.i32 r1, u3.w0`). Why Mesa
/// does not push them is not established (docs/notes/mali-deqp-colourpair.md).
const FS_DIFFERS: &[&str] = &["deqp_colourpair_rnd_fs_ck_madd"];

/// Byte-compares `fs.fragment` of a reference, leaving the vertex regions
/// out: the vertex program of a `rnd_fs_*` reference is a plain attribute
/// forward that no colour-pair lowering touches (section 16).
fn assert_fs_matches(name: &str) {
    let r = mali_ref::Reference::load(name);
    let pair = mali_ref::compile_pair(&r.vs, &r.fs).unwrap_or_else(|e| panic!("{name}: the pair is refused: {e}"));
    let d: Vec<String> = mali_ref::differences(&r, &pair).into_iter().filter(|m| m.starts_with("fs")).collect();
    assert!(d.is_empty(), "{name}: differs from its reference:\n{}", d.join("\n"));
}

/* ---- the base references: every region --------------------------------- */

#[test]
fn ck_lane() {
    mali_ref::assert_matches("deqp_colourpair_ck_lane");
}

#[test]
#[ignore = "differs: Mesa adds the mediump attributes' lanes at fp16, the vertex walk at fp32 (mixprec's vertex side); pinned by refusals_pinned"]
fn ck_lane_mediump() {
    mali_ref::assert_matches("deqp_colourpair_ck_lane_mediump");
}

#[test]
fn ck_lane_highpcolour() {
    mali_ref::assert_matches("deqp_colourpair_ck_lane_highpcolour");
}

#[test]
fn default_attrib() {
    mali_ref::assert_matches("deqp_colourpair_default_attrib");
}

#[test]
fn lane_zero_load() {
    mali_ref::assert_matches("deqp_colourpair_lane_zero_load");
}

/* ---- the rnd_fs_* references: fs.fragment only ------------------------- */

#[test]
fn rnd_fs_ck_dup() {
    assert_fs_matches("deqp_colourpair_rnd_fs_ck_dup");
}

#[test]
fn rnd_fs_ck_lz() {
    assert_fs_matches("deqp_colourpair_rnd_fs_ck_lz");
}

#[test]
#[ignore = "differs: Mesa makes the product's constant pairs with IADD_IMM, we push them to the FAU; pinned by refusals_pinned"]
fn rnd_fs_ck_madd() {
    assert_fs_matches("deqp_colourpair_rnd_fs_ck_madd");
}

#[test]
fn rnd_fs_ck_product() {
    assert_fs_matches("deqp_colourpair_rnd_fs_ck_product");
}

#[test]
fn rnd_fs_kword_ck() {
    assert_fs_matches("deqp_colourpair_rnd_fs_kword_ck");
}

#[test]
fn rnd_fs_uniform_tz_ck() {
    assert_fs_matches("deqp_colourpair_rnd_fs_uniform_tz_ck");
}

#[test]
fn rnd_fs_zk_tree() {
    assert_fs_matches("deqp_colourpair_rnd_fs_zk_tree");
}

#[test]
fn rnd_fs_zword_zk() {
    assert_fs_matches("deqp_colourpair_rnd_fs_zword_zk");
}

/* ---- the rnd_vs_* references: the vertex program as well --------------- */

#[test]
fn rnd_vs_ck_lz() {
    mali_ref::assert_matches("deqp_colourpair_rnd_vs_ck_lz");
}

#[test]
#[ignore = "differs: Mesa computes the mediump expression at fp16, the vertex walk at fp32 (mixprec's vertex side); pinned by refusals_pinned"]
fn rnd_vs_mediump_kept() {
    mali_ref::assert_matches("deqp_colourpair_rnd_vs_mediump_kept");
}

#[test]
fn rnd_vs_uniform_flat() {
    mali_ref::assert_matches("deqp_colourpair_rnd_vs_uniform_flat");
}

#[test]
fn rnd_vs_vecop_fs_uniform() {
    mali_ref::assert_matches("deqp_colourpair_rnd_vs_vecop_fs_uniform");
}

#[test]
fn rnd_vs_vecop_kept() {
    mali_ref::assert_matches("deqp_colourpair_rnd_vs_vecop_kept");
}

#[test]
fn rnd_vs_zk_lz() {
    mali_ref::assert_matches("deqp_colourpair_rnd_vs_zk_lz");
}

/* ---- the pins ------------------------------------------------------------- */

/// The exact current refusal of every ignored reference: the code
/// generator's message must be `Valhall: <pin> is not lowered yet` (a
/// refusal that is not the code generator's is pinned as the whole log).
/// A pin that no longer matches is a refusal that moved — lowered, or moved
/// to another family — and the test names both texts.
#[test]
fn refusals_pinned() {
    let mut moved = Vec::new();
    for (name, pin) in REFUSALS {
        let r = mali_ref::Reference::load(name);
        match mali_ref::compile_pair(&r.vs, &r.fs) {
            Ok(_) => moved.push(format!("{name}: the pair compiles; its test is ready to un-ignore")),
            Err(e) => {
                let want = format!("Valhall: {pin} is not lowered yet");
                if e != want && e != *pin {
                    moved.push(format!("{name}: refusal {e:?}, pinned {pin:?}"));
                }
            }
        }
    }
    for name in DIFFERS {
        let r = mali_ref::Reference::load(name);
        match mali_ref::compile_pair(&r.vs, &r.fs) {
            Ok(pair) => {
                let d = mali_ref::differences(&r, &pair);
                if d.is_empty() {
                    moved.push(format!("{name}: now byte-equal; un-ignore its test"));
                } else if d.iter().any(|m| m.starts_with("fs")) {
                    moved.push(format!("{name}: its fragment program differs now:\n{}", d.join("\n")));
                }
            }
            Err(e) => moved.push(format!("{name}: now refused: {e}")),
        }
    }
    for name in FS_DIFFERS {
        let r = mali_ref::Reference::load(name);
        match mali_ref::compile_pair(&r.vs, &r.fs) {
            Ok(pair) => {
                if !mali_ref::differences(&r, &pair).iter().any(|m| m.starts_with("fs")) {
                    moved.push(format!("{name}: its fragment program is byte-equal now; un-ignore its test"));
                }
            }
            Err(e) => moved.push(format!("{name}: now refused: {e}")),
        }
    }
    assert!(moved.is_empty(), "the pinned refusals moved:\n{}", moved.join("\n"));
}
