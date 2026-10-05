// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Every branch the online compiler emits reads its condition through one
//! lane, splatted -- the only form in the reference table (208 of 208
//! `branch_unary`). Any other swizzle makes lanes nobody computed part of the
//! test.
//!
//! Over the whole corpus plus the gl_test shapes that branch, on the bytes the
//! product emits. One test in this binary: it resets the keep arena.

#![cfg(all(feature = "gpu-vivante", feature = "shaderc-online", not(feature = "no-shader-table")))]

use core::ffi::c_char;
use yakogl::shaderc::mesa_reference::SHC_SOURCE_TABLE;
use yakogl::shaderc::{ShcShader, SHC_STAGE_FRAGMENT, SHC_STAGE_VERTEX};

#[test]
fn every_branch_reads_one_lane_splatted() {
    let mut sources: Vec<(String, u32, String)> = SHC_SOURCE_TABLE
        .iter()
        .map(|e| {
            let st = if e.name.contains(".vert") { SHC_STAGE_VERTEX } else { SHC_STAGE_FRAGMENT };
            (e.name.to_string(), st, e.text.trim_end_matches('\0').to_string())
        })
        .collect();
    for (n, t) in [
        ("es100/loopcarry.frag", include_str!("../tools/shaders/es100/loopcarry.frag")),
        ("es100/deadloop.frag", include_str!("../tools/shaders/es100/deadloop.frag")),
        ("es100/branchlane.frag", include_str!("../tools/shaders/es100/branchlane.frag")),
        ("es100/ifloop.frag", include_str!("../tools/shaders/es100/ifloop.frag")),
        ("es100/contloops.frag", include_str!("../tools/shaders/es100/contloops.frag")),
        ("es100/branchquad.frag", include_str!("../tools/shaders/es100/branchquad.frag")),
        ("es100/ifoutput.frag", include_str!("../tools/shaders/es100/ifoutput.frag")),
        ("es100/ifoutput2.frag", include_str!("../tools/shaders/es100/ifoutput2.frag")),
        ("es100/intlit.frag", include_str!("../tools/shaders/es100/intlit.frag")),
    ] {
        sources.push((n.to_string(), SHC_STAGE_FRAGMENT, t.to_string()));
    }
    let compile = yakogl::shaderc::online::SHC_ONLINE.compile.expect("the online back end compiles");
    let (mut compiled, mut with_branch, mut branches) = (0usize, 0usize, 0usize);
    let mut conversions = 0usize;
    let mut bad = Vec::new();
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
        let mut here = 0usize;
        yakogl::shaderc::online::vivante::decode::for_each_instr(out.usc_words(), |i, ins| {
            /* A conversion is typed by its integer side: all 95 f2i/i2f in
             * the reference table print `.s32` or `.u32`. An F32-typed f2i
             * made `int(u_in.x) == 2` false on silicon (gl_test 72). */
            if matches!(ins.op().name, "f2i" | "i2f") {
                let mut t = String::new();
                yakogl::shaderc::online::vivante::listing::instr_text(&mut t, i, ins).expect("lists");
                if !(t.contains(".s32") || t.contains(".u32")) {
                    bad.push(format!("{name} instruction {i}: a conversion without an integer type: {}", t.trim()));
                }
                conversions += 1;
            }
            if ins.op().name == "branch_unary" {
                here += 1;
                let s = ins.src(0).expect("a branch_unary reads its condition");
                if !s.swizzle.iter().all(|c| *c == s.swizzle[0]) {
                    bad.push(format!("{name} instruction {i}: condition swizzle {:?}", s.swizzle));
                }
            }
            Ok(())
        })
        .expect("our own code decodes");
        branches += here;
        with_branch += (here > 0) as usize;
    }
    println!("{compiled} programs compile, {with_branch} with a branch_unary, {branches} branch_unary in all, {conversions} f2i/i2f");
    /* The population: the three gl_test shapes branch, so zero here means the
     * check stopped seeing branches, not that they are all right. */
    assert!(with_branch >= 3, "only {with_branch} programs reached the check with a branch");
    assert!(conversions > 0, "no f2i/i2f reached the check; intlit.frag has one");
    assert!(bad.is_empty(), "{} encodings differ from the reference's rule: {bad:#?}", bad.len());
}
