// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#![cfg(all(feature = "shaderc-online", feature = "gpu-vivante"))]
//! Which corpus sources can be drawn BOTH ways, and which cannot?
//!
//! The twin-draw rung (team-lead, 2026-09-22) draws every corpus source twice
//! into the same scene -- once through the online compiler, once through the
//! offline table's blob for the same source -- and compares pixels. It exists
//! because `gl_test` scored 25/29 all day while 30 of 137 sources were
//! miscompiled: a suite score is bounded by its coverage, and the corpus is
//! much wider coverage than 29 hand-written cases.
//!
//! This is the host half. It cannot compare pixels -- that needs the GPU --
//! but it can answer the question the board run depends on: for how many
//! sources do BOTH sides exist? A source with no table blob has nothing to be
//! compared against, and a source the online compiler refuses has nothing to
//! compare. Neither is a defect; both are rows the rung must skip, and a rung
//! that silently skipped them would report "0 mismatches" over a set nobody
//! had counted.
use core::ffi::c_char;
use std::collections::BTreeMap;
use yakogl::shaderc::table::{SHC_BLOB_TABLE, SHC_SOURCE_TABLE};
use yakogl::shaderc::{ShcShader, SHC_STAGE_FRAGMENT, SHC_STAGE_VERTEX};

fn online(stage: u32, src: &str) -> Result<ShcShader, String> {
    let mut out = ShcShader::ZERO;
    let mut log = [0 as c_char; 512];
    let mut s = String::from(src);
    if !s.ends_with('\0') {
        s.push('\0');
    }
    let f = yakogl::shaderc::online::SHC_ONLINE.compile.unwrap();
    let rc = unsafe { f(stage, s.as_ptr() as *const c_char, &mut out, log.as_mut_ptr(), log.len()) };
    if rc == 0 {
        Ok(out)
    } else {
        let m: Vec<u8> = log.iter().take_while(|c| **c != 0).map(|c| *c as u8).collect();
        Err(String::from_utf8_lossy(&m).into_owned())
    }
}

/// The FemtoVG set is the one Slint draws with, and the one the if-conversion
/// miscompile hit. Selected by name here; gles2-core owns the authoritative
/// list of the 30 their fix touched, and this is stated as a NAME MATCH so
/// nobody reads it as that list.
fn is_femtovg(name: &str) -> bool {
    ["fill", "gradient", "image", "stencil", "texturefill", "glyph"].iter().any(|k| name.contains(k))
}

#[test]
fn twinnable_corpus() {
    let (mut both, mut no_blob, mut refused, mut neither) = (0usize, 0usize, 0usize, 0usize);
    let mut refusals: BTreeMap<String, usize> = BTreeMap::new();
    let (mut fv_both, mut fv_total) = (0usize, 0usize);
    let mut no_blob_names: Vec<&str> = Vec::new();
    // The FemtoVG set is what the product draws with, so a source the online
    // path cannot compile is a PRODUCT gap before it is a rung gap. Each one
    // is named with its own refusal rather than counted.
    let mut fv_blocked: Vec<(&str, String)> = Vec::new();

    for e in SHC_SOURCE_TABLE.iter() {
        let stage = if e.name.contains(".vert") { SHC_STAGE_VERTEX } else { SHC_STAGE_FRAGMENT };
        // Reset the compiler's keep arena between sources.
        //
        // WITHOUT THIS the harness measured itself. The arena is 512 KiB of
        // .bss holding every compiled shader's code, reflection and names for
        // the life of the process, never reset because the GL core's
        // ShcShader points into it. Walking 324 sources in one process fills
        // it after about 65 at ~8 KiB each, and every source after that is
        // refused with Abi(OutOfMemory) -- which I reported as "the online
        // compiler cannot build 14 of the 16 FemtoVG vertex shaders", a
        // product claim that was false. The keep arena is the resource
        // (2e6914b): conicalgradient.vert and lineargradient.vert both
        // compile, and 69 of the 71 refused compile once the arena is reset.
        //
        // A board rung sized from the un-reset count would have tested a
        // fraction of what the compiler covers.
        yakogl::shaderc::online::backend::forget_compiled();
        let txt = e.text.strip_suffix('\0').unwrap_or(e.text);
        let digest = yakogl::shaderc::sha256::sha256(txt.as_bytes());
        let has_blob = SHC_BLOB_TABLE.iter().any(|b| b.blend_key == 0 && b.sha256 == digest);
        let compiled = online(stage, e.text);

        if is_femtovg(e.name) {
            fv_total += 1;
            match (&compiled, has_blob) {
                (Ok(_), true) => fv_both += 1,
                (Ok(_), false) => fv_blocked.push((e.name, "no blob in the table".into())),
                (Err(why), _) => fv_blocked.push((e.name, why.trim().to_string())),
            }
        }
        match (has_blob, compiled) {
            (true, Ok(_)) => both += 1,
            (false, Ok(_)) => {
                no_blob += 1;
                no_blob_names.push(e.name);
            }
            (true, Err(why)) => {
                refused += 1;
                // A table source the CEILING refuses is worth naming: it means
                // the offline table holds a blob above the bound we now
                // enforce, which is exactly the wedge region.
                if why.contains("LoadBalancingTempsUnmeasured") {
                    // The ceiling's own criterion is "the highest temps any
                    // shipped BLOB in each band carries". So print the blob's
                    // temps beside ours: if the blob is above the ceiling, the
                    // band contradicts the rule it was derived from.
                    let blob_temps =
                        SHC_BLOB_TABLE.iter().find(|b| b.blend_key == 0 && b.sha256 == digest).map(|b| b.shader.temps);
                    println!(
                        "  CEILING refuses a table source: {} -- {} | the table's blob for it: {:?} temps",
                        e.name,
                        why.trim(),
                        blob_temps
                    );
                }
                // Group by the refusal's KIND. Splitting on punctuation left
                // rows reading "12 })", which is how a count becomes
                // unattributable -- the same truncation that hid the ceiling
                // behind "online shaderc: the" for most of today.
                let key = if why.contains("LoadBalancingTempsUnmeasured") {
                    "the temps CEILING (abi.rs vs_load_balancing)".to_string()
                } else if why.contains("OutOfMemory") {
                    // NOT the ceiling, and NOT an ABI space: it is the
                    // compiler's keep arena, and with the reset above it
                    // should not appear at all. If it still does, the source
                    // needs more than the whole 512 KiB by itself.
                    "keep arena full even after a reset (one shader over 512 KiB)".to_string()
                } else if why.contains("ES 3.00") {
                    "GLSL ES 3.00 source, correctly refused by an ES 2.0 compiler".to_string()
                } else if why.contains("desktop GLSL") {
                    // es100-frontend named these: all 13 are `#version 450`.
                    // They exist because the OFFLINE generators accept GLSL
                    // 450 with explicit locations as well as GLSL ES 1.00, so
                    // an ES compiler refusing them is the point of having two
                    // spellings, not a gap.
                    "desktop GLSL 450, correctly refused by an ES compiler".to_string()
                } else if why.contains("needs 'es'") {
                    // `#version 300` without `es`. There is no desktop GLSL
                    // 300/310/320, so this is a typo in the source and calling
                    // it "desktop" would send the author after the wrong
                    // problem.
                    "an ES #version missing its `es` (a typo, not an unsupported version)".to_string()
                } else if why.contains("shading language version") {
                    "a version number that is neither ES nor desktop".to_string()
                } else {
                    why.split(':').next_back().unwrap_or(&why).trim().to_string()
                };
                *refusals.entry(key).or_insert(0) += 1;
            }
            (false, Err(_)) => neither += 1,
        }
    }

    let total = SHC_SOURCE_TABLE.len();
    println!("corpus {total} sources");
    println!("  TWINNABLE (blob + online compile): {both}");
    println!("  no table blob, online compiles:    {no_blob}");
    println!("  blob exists, online refuses:       {refused}");
    println!("  neither:                           {neither}");
    println!("  FemtoVG-named: {fv_both} of {fv_total} twinnable");
    // Is the split along the STAGE? Every blocked one seen so far is a vertex
    // shader, and "the online path compiles no FemtoVG vertex shader" is a
    // different and much larger statement than "14 sources are blocked".
    let fv_vert = SHC_SOURCE_TABLE.iter().filter(|e| is_femtovg(e.name) && e.name.contains(".vert")).count();
    let fv_frag = fv_total - fv_vert;
    let blocked_vert = fv_blocked.iter().filter(|(n, _)| n.contains(".vert")).count();
    println!(
        "  FemtoVG stages: {fv_vert} vertex, {fv_frag} fragment; blocked: {} of which {blocked_vert} vertex",
        fv_blocked.len()
    );
    for (why, n) in &refusals {
        println!("  refusal x{n}: {why}");
    }
    if !fv_blocked.is_empty() {
        println!("  the FemtoVG sources that CANNOT be twinned, each with its reason:");
        for (name, why) in &fv_blocked {
            println!("    {name}: {why}");
        }
    }
    if !no_blob_names.is_empty() {
        println!("  sources with no blob (first 8): {:?}", &no_blob_names[..no_blob_names.len().min(8)]);
    }

    // HOW MANY CAN ACTUALLY BE DRAWN? A draw needs both stages, and the table
    // lists shaders one at a time, so a source is only twin-drawable if its
    // same-base-name partner is twinnable too (`lineargradient.vert` with
    // `lineargradient.frag`). That is a smaller set than the twinnable count,
    // and the board rung's real denominator, so it is measured here rather
    // than discovered when the walk is written.
    let twinnable_names: Vec<&str> = SHC_SOURCE_TABLE
        .iter()
        .filter(|e| {
            let txt = e.text.strip_suffix('\0').unwrap_or(e.text);
            let d = yakogl::shaderc::sha256::sha256(txt.as_bytes());
            let has = SHC_BLOB_TABLE.iter().any(|b| b.blend_key == 0 && b.sha256 == d);
            let stage = if e.name.contains(".vert") { SHC_STAGE_VERTEX } else { SHC_STAGE_FRAGMENT };
            yakogl::shaderc::online::backend::forget_compiled();
            has && online(stage, e.text).is_ok()
        })
        .map(|e| e.name)
        .collect();
    let base = |n: &str| -> String { n.trim_end_matches(".vert").trim_end_matches(".frag").to_string() };
    let mut paired = 0usize;
    let mut unpaired: Vec<&str> = Vec::new();
    for n in &twinnable_names {
        let b = base(n);
        let want_vert = n.ends_with(".frag");
        let partner = twinnable_names.iter().any(|m| *m != *n && base(m) == b && (m.ends_with(".vert") == want_vert));
        if partner {
            paired += 1;
        } else {
            unpaired.push(n);
        }
    }
    println!("  DRAWABLE PAIRS: {paired} of {} twinnable sources have a twinnable partner", twinnable_names.len());
    println!("  = {} complete programs; {} sources have no twinnable partner", paired / 2, unpaired.len());
    if !unpaired.is_empty() {
        println!("  first unpaired: {:?}", &unpaired[..unpaired.len().min(6)]);
    }

    // The rung's population must be stated, not discovered at run time. This
    // is the same rule pvrgpu-platform's USC gate arrived at after reporting
    // 72.4 % while skipping 159,305 groups: a gate asserts its population.
    assert_eq!(both + no_blob + refused + neither, total, "every source accounted for");
    assert!(both > 0, "nothing is twinnable: the rung would measure an empty set");
}
