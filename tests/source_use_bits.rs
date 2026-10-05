// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The `SRC{n}_USE` bits we emit are the reference's, operation by operation.
//!
//! Most operations pin their use bits in the ISA table; `texldb` and `texldl`
//! carry all three as fields (and `texld` its SRC2), so for them the bit is
//! the builder's to set. It never was: every texldl and texldb went out with
//! SRC0_USE = SRC1_USE = 0, a look-up with no coordinate and no level, while
//! all 78 texldl of the reference have both set. dEQP-GLES2's
//! texture_functions failed on the i.MX8MP for it (2026-09-24), even the
//! vertex texture2dprojlod cases, whose texture has one level. The form
//! vocabulary (tests/encoding_vocabulary.rs) cannot see a use bit.
//!
//! One test in this binary: it resets the compiler's keep arena.

#![cfg(all(feature = "gpu-vivante", feature = "shaderc-online", not(feature = "no-shader-table")))]

use core::ffi::c_char;
use std::collections::{BTreeMap, BTreeSet};
use yakogl::shaderc::mesa_reference::{SHC_BLOB_TABLE, SHC_SOURCE_TABLE};
use yakogl::shaderc::online::vivante::decode::for_each_instr;
use yakogl::shaderc::online::vivante::isa::Instr;
use yakogl::shaderc::{ShcShader, SHC_STAGE_FRAGMENT, SHC_STAGE_VERTEX};

/// SRC0_USE, SRC1_USE, SRC2_USE; None where the bit is pinned.
type UseBits = [Option<u32>; 3];
/// Per operation, each pattern with its count and a source that emits it.
type Patterns = BTreeMap<&'static str, BTreeMap<UseBits, (usize, String)>>;

/// The use bits of an instruction whose operation carries any as fields:
/// (name, [SRC0_USE, SRC1_USE, SRC2_USE]), None where a bit is pinned.
fn use_bits(ins: &Instr) -> Option<(&'static str, [Option<u32>; 3])> {
    let u = [ins.get("SRC0_USE"), ins.get("SRC1_USE"), ins.get("SRC2_USE")];
    u.iter().any(|b| b.is_some()).then(|| (ins.op().name, u))
}

fn compile(stage: u32, text: &str) -> Option<ShcShader> {
    let compile = yakogl::shaderc::online::SHC_ONLINE.compile.expect("the online back end compiles");
    yakogl::shaderc::online::backend::forget_compiled();
    let mut out = ShcShader::ZERO;
    let mut log = [0 as c_char; 512];
    let src = format!("{text}\0");
    // SAFETY: a NUL-terminated source, a real output, and a log of the size announced.
    let rc = unsafe { compile(stage, src.as_ptr() as *const c_char, &mut out, log.as_mut_ptr(), log.len()) };
    (rc == 0).then_some(out)
}

/// dEQP-GLES2 texture_functions' shapes: a vertex texture2DLod and a
/// fragment texture2D with a bias, the two instructions the table holds too
/// few of (texldb: none).
const VS_LOD: &str = "attribute highp vec4 a_position; attribute highp vec4 a_in0; attribute highp float a_in1;
uniform sampler2D u_sampler; uniform mediump vec4 u_scale; uniform mediump vec4 u_bias;
varying mediump vec4 v_color;
void main() { gl_Position = a_position; v_color = texture2DLod(u_sampler, a_in0.xy, a_in1) * u_scale + u_bias; }";
const FS_BIAS: &str = "precision mediump float; uniform sampler2D u_sampler; uniform vec4 u_scale; uniform vec4 u_bias;
varying highp vec4 v_texCoord; varying mediump float v_lodBias;
void main() { gl_FragColor = texture2D(u_sampler, v_texCoord.xy, v_lodBias) * u_scale + u_bias; }";

#[test]
fn every_source_use_bit_we_emit_is_the_references() {
    let mut reference: BTreeMap<&str, BTreeSet<[Option<u32>; 3]>> = BTreeMap::new();
    for row in SHC_BLOB_TABLE.iter() {
        for_each_instr(row.shader.usc_words(), |_, ins| {
            if let Some((n, u)) = use_bits(ins) {
                reference.entry(n).or_default().insert(u);
            }
            Ok(())
        })
        .expect("the reference decodes");
    }
    /* The reference's own pattern, pinned so a regenerated table that
     * changed it is seen: coordinate and level read, no SRC2. */
    assert_eq!(
        reference.get("texldl").map(|s| s.iter().copied().collect::<Vec<_>>()),
        Some(vec![[Some(1), Some(1), Some(0)]]),
        "the reference's texldl use bits moved"
    );

    let mut ours: Patterns = BTreeMap::new();
    let mut counted = 0usize;
    for e in SHC_SOURCE_TABLE.iter() {
        let st = if e.name.contains(".vert") { SHC_STAGE_VERTEX } else { SHC_STAGE_FRAGMENT };
        let Some(out) = compile(st, e.text.trim_end_matches('\0')) else { continue };
        for_each_instr(out.usc_words(), |_, ins| {
            if let Some((n, u)) = use_bits(ins) {
                counted += 1;
                ours.entry(n).or_default().entry(u).or_insert((0, e.name.to_string())).0 += 1;
            }
            Ok(())
        })
        .expect("our own code decodes");
    }
    let foreign: Vec<String> = ours
        .iter()
        .flat_map(|(n, pats)| {
            let r = reference.get(n);
            pats.iter()
                .filter(move |(p, _)| r.is_some_and(|r| !r.contains(*p)))
                .map(move |(p, (c, ex))| format!("{n} {p:?} x{c}, e.g. {ex}"))
        })
        .collect();
    println!("{counted} instructions with use fields; ours {ours:?}");
    /* The population, per build: the table's texldl all come from GLSL ES
     * 3.00 sources (es300/cube.frag and its kin), which a report-es2 build
     * refuses in the front end (pp.rs out_of_scope); texld reaches the check
     * in both. dEQP's ES 1.00 shapes below check texldl and texldb in both. */
    let es2 = cfg!(feature = "report-es2");
    assert!(ours.get("texld").is_some_and(|p| !p.is_empty()), "no texld reached the check");
    assert_eq!(
        ours.get("texldl").is_some_and(|p| !p.is_empty()),
        !es2,
        "the table's texldl population moved (report-es2: {es2})"
    );
    assert!(foreign.is_empty(), "use bits the reference never emits: {foreign:#?}");

    /* texldb has no reference instance at all, so dEQP's shapes are checked
     * against the rule itself: both operands read, SRC2 not. */
    for (what, stage, text, op) in [
        ("VS texture2DLod", SHC_STAGE_VERTEX, VS_LOD, "texldl"),
        ("FS texture2D bias", SHC_STAGE_FRAGMENT, FS_BIAS, "texldb"),
    ] {
        let out = compile(stage, text).unwrap_or_else(|| panic!("{what} compiles"));
        let mut seen = 0;
        for_each_instr(out.usc_words(), |_, ins| {
            if let Some((n, u)) = use_bits(ins) {
                if n == op {
                    seen += 1;
                    assert_eq!(
                        u,
                        [Some(1), Some(1), Some(0)],
                        "{what}: {op} must read its coordinate and its level or bias"
                    );
                }
            }
            Ok(())
        })
        .expect("our own code decodes");
        assert_eq!(seen, 1, "{what}: one {op} expected");
    }
}
