// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Who sets the scratch high-water mark, and how much of it is whose.
//!
//! M5 sizes a Kconfig static from this number and that static is `.bss` in the
//! product's image, so a figure that is wrong in either direction costs
//! something real: too low and some shader nobody predicted fails on the
//! board, too high and the user pays for the memory forever.
//!
//! The number has moved twice in one day, in both directions, which is why
//! this exists rather than a line in a commit message:
//!
//! | measured | peak | corpus | what changed |
//! |---|---|---|---|
//! | 13:32 | 931,496 | 307 rows | the original figure in `store.rs` |
//! | ~16:00 | 447,496 | 324 rows | the front end rewritten a dozen times since |
//! | after if-conversion | 515,472 | 324 rows | more sources reach selection and assembly |
//!
//! **`SCRATCH_PEAK` is the deepest single compile the compiler happened to
//! perform, not a bound on anything.** It rises when the compiler covers more
//! and falls when the front end gets leaner, so it tracks the compiler rather
//! than the hardware or the corpus. Two readings are only comparable with
//! their corpus size and their date, which is why both are printed here.
//!
//! ```sh
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online \
//!     --test scratch_survey -- --nocapture
//! ```

#![cfg(all(feature = "shaderc-online", feature = "gpu-vivante"))]

use core::ffi::c_char;
use yakogl::shaderc::online::lower::compile_to_ir;
use yakogl::shaderc::online::{arena::Arena, sema::Stage, store};
use yakogl::shaderc::table::{SHC_BLOB_TABLE, SHC_SOURCE_TABLE};
use yakogl::shaderc::{ShcShader, SHC_STAGE_FRAGMENT, SHC_STAGE_VERTEX};

fn row_for(text: &str) -> Option<&'static yakogl::shaderc::ShcBlob> {
    let key = yakogl::shaderc::sha256::sha256(text.as_bytes());
    SHC_BLOB_TABLE.iter().find(|b| b.sha256 == key && b.blend_key == 0)
}

#[test]
fn who_sets_the_scratch_high_water() {
    /* Pass 1: the whole compile. Reading the monotone peak before and after
     * each source names the one that raised it, which a single final number
     * cannot. */
    let mut records: Vec<(usize, &str, String)> = Vec::new();
    let mut before = store::scratch_peak();
    let (mut n, mut ok) = (0usize, 0usize);
    for e in SHC_SOURCE_TABLE.iter() {
        let txt = e.text.trim_end_matches('\0');
        let Some(b) = row_for(txt) else { continue };
        let stage = if b.stage == SHC_STAGE_FRAGMENT { SHC_STAGE_FRAGMENT } else { SHC_STAGE_VERTEX };
        n += 1;
        yakogl::shaderc::online::backend::forget_compiled();
        let mut out = ShcShader::ZERO;
        let mut log = [0 as c_char; 512];
        let mut s = String::from(txt);
        s.push('\0');
        let compile = yakogl::shaderc::online::SHC_ONLINE.compile.expect("the online back end compiles");
        // SAFETY: a NUL-terminated source, a real output, a log of the size announced.
        let rc = unsafe { compile(stage, s.as_ptr() as *const c_char, &mut out, log.as_mut_ptr(), log.len()) };
        if rc == 0 {
            ok += 1;
        }
        let after = store::scratch_peak();
        if after > before {
            let why = if rc == 0 {
                format!("compiled, {} instructions", out.usc_words().len() / 4)
            } else {
                let m: Vec<u8> = log.iter().take_while(|c| **c != 0).map(|c| *c as u8).collect();
                format!("refused: {}", String::from_utf8_lossy(&m))
            };
            records.push((after, e.name, why));
            before = after;
        }
    }

    /* Pass 2: lowering alone, so the peak can be split. A refusal late in the
     * back end still carries the whole front end's arena, so "who owns this
     * number" is not answerable from the total. */
    let mut scratch = vec![0u8; 4 << 20];
    let mut fe: Vec<(usize, &str)> = Vec::new();
    for src in SHC_SOURCE_TABLE.iter() {
        let text = src.text.trim_end_matches('\0');
        let Some(b) = row_for(text) else { continue };
        let stage = if b.stage == SHC_STAGE_FRAGMENT { Stage::Fragment } else { Stage::Vertex };
        let a = Arena::new(&mut scratch);
        if compile_to_ir(&a, stage, text).is_ok() {
            fe.push((a.used(), src.name));
        }
    }
    fe.sort_unstable();
    fe.reverse();

    let peak = store::scratch_peak();
    println!("\n=== the scratch high-water mark ===\n");
    println!("  corpus {n} sources, {ok} compiled through the whole path");
    println!("  peak   {peak} bytes of {}\n", store::SCRATCH_BYTES);
    println!("  raised by, in order:");
    for (p, name, why) in &records {
        println!("    {p:>9}  {name:<46} {why}");
    }
    println!("\n  lowering alone, deepest first:");
    for (used, name) in fe.iter().take(5) {
        println!("    {used:>9}  {name}");
    }
    if let (Some((deep, _)), true) = (fe.first(), peak > 0) {
        println!("\n  the front end is {}% of the deepest compile.", deep * 100 / peak);
    }
}
