// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Every instruction FORM the online compiler emits -- operation, condition,
//! type and rounding, as the listing spells them (`cmp.lt.s32`, `sin.rtz`) --
//! is one the reference compiler emits too.
//!
//! Two defects of 2026-09-23 were a form the reference never uses: `f2i`
//! typed F32 where all 44 of the reference's are `.s32`/`.u32` (it made
//! `int(x) == 2` false on silicon), and a plain `sin`/`cos`/`log` where
//! every one of the reference's is `.rtz` and paired with a `mul` (this
//! core's transcendental unit). A comparison of vocabularies catches that
//! class before a board does: a form here that the 324 reference programs
//! never contain is either a new, deliberate choice -- to be justified and
//! listed below -- or a defect.
//!
//! One test in this binary: it resets the compiler's keep arena.

#![cfg(all(feature = "gpu-vivante", feature = "shaderc-online", not(feature = "no-shader-table")))]

use core::ffi::c_char;
use std::collections::{BTreeMap, BTreeSet};
use yakogl::shaderc::online::vivante::isa::Instr;
use yakogl::shaderc::table::{SHC_BLOB_TABLE, SHC_SOURCE_TABLE};
use yakogl::shaderc::{ShcShader, SHC_STAGE_FRAGMENT, SHC_STAGE_VERTEX};

/// Forms we emit on purpose that the reference table does not contain, each
/// with the reason.
const DELIBERATE: &[(&str, &str)] = &[
    (
        "lshift.s32",
        "GLSL ES 3.00 `<<` on an int: Mesa's etnaviv table IOP(ishl, LSHIFT), S32 \
     (etnaviv_compiler_nir_emit.c, MIT). The reference never shifts left -- ES 1.00 \
     has no `<<` -- but emits the same instruction family as rshift.s32 / \
     rshift.u32 (144). Transcribed, not measured on silicon; the first ES 3.00 \
     source to use it is environmentmapprefilter_rgbe_300.frag.",
    ),
    (
        "cmp.lt.u32",
        "GLSL ES 3.00 `<` on a uint: Mesa's etnaviv table UOPC(ult32, CMP, LT), U32 \
     (select.rs op_type). ES 1.00 has no uint, so the reference has no unsigned \
     compare; the instruction is its cmp.lt at TYPE 6. Host-verified by \
     backend_differential's ES3 integer test (a mutation typing it S32 makes 54 of \
     72 disagree); not on silicon. First reached by e0880dcd5582d4de.frag once the \
     cube level/bias arm (f3eeea0) let it select.",
    ),
    (
        "load.u32",
        "a uniform array read at a run-time index (select.rs LoadUniformIdx): the \
     reference table comes from etna_shc, which runs no GL state tracker and so emits \
     movar + u[a.x], the form that read element 0 on the i.MX8MP; Mesa's GL path \
     ships this one to the GC7000 (cs_golden mode 25: `load.u32 t1, t1.xxxx, u0.zxxx`), \
     and backend_differential runs it against the IR.",
    ),
];

fn form(i: usize, ins: &Instr) -> String {
    let mut t = String::new();
    yakogl::shaderc::online::vivante::listing::instr_text(&mut t, i, ins).expect("lists");
    t.split(']').nth(1).unwrap_or("").split_whitespace().next().unwrap_or("").to_string()
}

#[test]
fn every_instruction_form_we_emit_is_one_the_reference_emits() {
    let mut reference = BTreeSet::new();
    for row in SHC_BLOB_TABLE.iter() {
        yakogl::shaderc::online::vivante::decode::for_each_instr(row.shader.usc_words(), |i, ins| {
            reference.insert(form(i, ins));
            Ok(())
        })
        .expect("the reference decodes");
    }
    let mut sources: Vec<(String, u32, String)> = SHC_SOURCE_TABLE
        .iter()
        .map(|e| {
            let st = if e.name.contains(".vert") { SHC_STAGE_VERTEX } else { SHC_STAGE_FRAGMENT };
            (e.name.to_string(), st, e.text.trim_end_matches('\0').to_string())
        })
        .collect();
    for (n, st, t) in [
        ("es100/transc.frag", SHC_STAGE_FRAGMENT, include_str!("../tools/shaders/es100/transc.frag")),
        ("es100/vtransc.frag", SHC_STAGE_FRAGMENT, include_str!("../tools/shaders/es100/vtransc.frag")),
        ("es100/intlit.frag", SHC_STAGE_FRAGMENT, include_str!("../tools/shaders/es100/intlit.frag")),
        ("es100/loopcarry.frag", SHC_STAGE_FRAGMENT, include_str!("../tools/shaders/es100/loopcarry.frag")),
        ("es100/ifoutput.frag", SHC_STAGE_FRAGMENT, include_str!("../tools/shaders/es100/ifoutput.frag")),
    ] {
        sources.push((n.to_string(), st, t.to_string()));
    }
    let compile = yakogl::shaderc::online::SHC_ONLINE.compile.expect("the online back end compiles");
    let mut ours: BTreeMap<String, (usize, String)> = BTreeMap::new();
    let mut compiled = 0usize;
    for (name, stage, text) in &sources {
        yakogl::shaderc::online::backend::forget_compiled();
        let mut out = ShcShader::ZERO;
        let mut log = [0 as c_char; 512];
        let src = format!("{text}\0");
        // SAFETY: a NUL-terminated source, a real output, and a log of the size announced.
        let rc = unsafe { compile(*stage, src.as_ptr() as *const c_char, &mut out, log.as_mut_ptr(), log.len()) };
        if rc != 0 {
            assert!(!name.starts_with("es100/"), "{name} is a gl_test shape and must compile");
            continue;
        }
        compiled += 1;
        yakogl::shaderc::online::vivante::decode::for_each_instr(out.usc_words(), |i, ins| {
            let e = ours.entry(form(i, ins)).or_insert((0, name.clone()));
            e.0 += 1;
            Ok(())
        })
        .expect("our own code decodes");
    }
    let foreign: Vec<String> = ours
        .iter()
        .filter(|(f, _)| !reference.contains(*f) && !DELIBERATE.iter().any(|(d, _)| d == f))
        .map(|(f, (n, ex))| format!("{f} x{n}, e.g. {ex}"))
        .collect();
    println!("{compiled} programs, {} forms emitted; the reference uses {} forms", ours.len(), reference.len());
    /* The population: the corpus and the shapes compile and use real forms. */
    assert!(compiled >= 140 && ours.len() >= 30, "only {compiled} programs / {} forms reached the check", ours.len());
    assert!(foreign.is_empty(), "forms the reference never emits: {foreign:#?}");
}
