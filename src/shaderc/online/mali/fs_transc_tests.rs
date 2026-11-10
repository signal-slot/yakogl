// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! `sin`, `cos` and `log2` in a mediump fragment shader (gl_test case 77,
//! docs/notes/mali-transcendentals.md) against Mesa 26.2.1's drm-shim
//! program fsarith_77_transc. Its fourth lane, `exp2`, stays refused: Mesa's
//! `F32_TO_S32` there has no rounding mode (word 33), an op bir.rs lacks, so
//! the words up to that lane's are compared, from a program whose fourth
//! lane is a `cos` (the same lanes before it, the same registers).

use super::tests::{compile, region};
use super::MaliError;
use crate::shaderc::online::codegen::{self, CompileError};
use crate::shaderc::SHC_STAGE_FRAGMENT;

macro_rules! td {
    ($f:literal) => {
        include_str!(concat!("../../../../tools/mali/testdata/", $f))
    };
}

const TRANSC_FS: &str = td!("fsarith_77_transc.frag");
const TRANSC_JSON: &str = td!("fsarith_77_transc.expect.json");
const VTRANSC_FS: &str = td!("fsarith_77_vtransc.frag");

/// Case 77's colour with `cos(u_a.w)` for `exp2(u_a.w)`.
const TRANSC_COS_W: &str = "precision mediump float; uniform vec4 u_a; void main() \
     { gl_FragColor = vec4(sin(u_a.x), cos(u_a.y), log2(u_a.z) * 0.25, cos(u_a.w)); }";

/// The Mali gap a fragment shader meets first, None if it compiles.
fn verdict(src: &str) -> Option<MaliError> {
    match compile(SHC_STAGE_FRAGMENT, src) {
        Ok(_) => None,
        Err(CompileError::BackEnd(codegen::BackEndError::Mali(e))) => Some(e),
        Err(e) => panic!("{src}: not a Mali gap: {e:?}"),
    }
}

/// The compiled fragment words of `fs` without the zero padding (a program
/// never ends in an all-zero word).
fn fs_words(fs: &str) -> Vec<u64> {
    let mut w = compile(SHC_STAGE_FRAGMENT, fs).expect("the fragment shader compiles").code;
    while w.last() == Some(&0) {
        w.pop();
    }
    w
}

fn text(w: u64) -> String {
    let mut s = String::new();
    super::isa::describe_word(w, &mut s).expect("a Valhall word");
    s
}

/// The mnemonic of each word, its flow (`.discard`, `.wait*`) dropped.
fn mnemonics(words: &[u64]) -> Vec<String> {
    words
        .iter()
        .map(|&w| {
            let t = text(w);
            let m = t.split_whitespace().next().unwrap_or("");
            m.split('.').filter(|p| *p != "discard" && !p.starts_with("wait")).collect::<Vec<_>>().join(".")
        })
        .collect()
}

/// Byte-equal word by word; a mismatch lists every differing index with
/// both disassemblies.
fn assert_words(ours: &[u64], theirs: &[u64]) {
    let diff: Vec<String> = (0..ours.len().max(theirs.len()))
        .filter(|&i| ours.get(i) != theirs.get(i))
        .map(|i| {
            let side = |w: Option<&u64>| w.map_or("(none)".into(), |&w| format!("{w:016x} {}", text(w)));
            format!("[{i:2}] ours {}\n     ref  {}", side(ours.get(i)), side(theirs.get(i)))
        })
        .collect();
    assert!(diff.is_empty(), "{} words ours, {} reference:\n{}", ours.len(), theirs.len(), diff.join("\n"));
}

/// Words 0-30 of fsarith_77_transc: the sin lane (0-10), the cos lane
/// (11-21), the log2 lane and its fp16 product (22-29), then the fourth
/// lane's `F16_TO_F32` (30), byte for byte.
#[test]
fn the_sin_cos_and_log2_lanes_are_byte_equal_to_mesa_26_2_1() {
    let r = region(TRANSC_JSON, "fs.fragment").expect("fs.fragment");
    let ours = fs_words(TRANSC_COS_W);
    assert_words(&ours[..31], &r.words[..31]);
}

/// The fourth lane of that program is the cos lane's rows again (11-21,
/// two registers up), then ATEST on its narrowed half and the two words
/// packed after it, as the reference's epilogue (36-41).
#[test]
fn a_cos_in_the_fourth_lane_is_the_cos_rows_and_the_epilogue() {
    let r = region(TRANSC_JSON, "fs.fragment").expect("fs.fragment");
    let ours = mnemonics(&fs_words(TRANSC_COS_W));
    let rows = mnemonics(&r.words);
    assert_eq!(&ours[30..41], &rows[11..22]);
    assert_eq!(&ours[41..47], &rows[36..42]);
    assert_eq!(ours.len(), 47);
}

/// `exp2` is refused where it stands (the gap this file's header names),
/// so case 77's first program still meets the same refusal.
#[test]
fn exp2_is_still_refused() {
    assert_eq!(verdict(TRANSC_FS), Some(MaliError::Arithmetic));
    let e = "precision mediump float; uniform vec4 u_a; void main() \
             { gl_FragColor = vec4(sin(u_a.x), exp2(u_a.y), 0.5, 1.0); }";
    assert_eq!(verdict(e), Some(MaliError::Arithmetic));
    /* vtransc: `inversesqrt` at fp16 (`FRSQ.f16`) has no op either */
    assert_eq!(verdict(VTRANSC_FS), Some(MaliError::Arithmetic));
}

/// The corpus lines of case 77 are the references' own sources.
#[test]
fn the_corpus_transcendental_sources_are_the_references() {
    assert_eq!(include_str!("../../../../tools/shaders/es100/transc.frag"), TRANSC_FS);
    assert_eq!(include_str!("../../../../tools/shaders/es100/vtransc.frag"), VTRANSC_FS);
    assert_eq!(td!("alu_transc.frag"), TRANSC_FS);
    assert_eq!(td!("alu_transc.expect.json"), TRANSC_JSON);
}

/// `log2(z)` is narrowed and its product stays fp16 (words 27-29: `FMA.f32`,
/// the `FADD.f32 .h0`, then `FMA.v2f16`), not an `FMA.f32` by 0.25.
#[test]
fn a_transcendental_result_is_narrowed_before_an_fp16_product() {
    let src = "precision mediump float; uniform vec4 u_a; void main() \
               { gl_FragColor = vec4(log2(u_a.z) * 0.25, sin(u_a.x), 0.5, 1.0); }";
    let ours = mnemonics(&fs_words(src));
    let at = ours.iter().position(|m| m == "FLOGD.f32").expect("FLOGD");
    assert_eq!(&ours[at + 1..at + 4], ["FMA.f32", "FADD.f32", "FMA.v2f16"]);
}

/// An operand other than a uniform's component has no reference, nor has a
/// lane paired with a constant.
#[test]
fn other_operands_are_refused() {
    let v = "precision mediump float; varying vec4 v_c; void main() \
             { gl_FragColor = vec4(sin(v_c.x), cos(v_c.y), 0.5, 1.0); }";
    assert_eq!(verdict(v), Some(MaliError::Arithmetic));
    let u = "precision mediump float; uniform vec4 u_a; void main() \
             { gl_FragColor = vec4(sin(u_a.x * u_a.y), cos(u_a.y), 0.5, 1.0); }";
    assert_eq!(verdict(u), Some(MaliError::Arithmetic));
    let k = "precision mediump float; uniform vec4 u_a; void main() \
             { gl_FragColor = vec4(sin(u_a.x), 0.5, 0.5, 1.0); }";
    assert_eq!(verdict(k), Some(MaliError::ColourForm));
}
