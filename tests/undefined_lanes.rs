// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! No instruction selection emits reads a lane of a value that nothing
//! computes.
//!
//! Twice a one-lane IR value -- a condition -- was read through `.xyzw`: by
//! a branch (0828b91, gl_test 70 black on the GC7000) and by the
//! if-conversion's `select` (fb04d88, dEQP loops.*.conditional_body). Both
//! times the lanes past x decided with whatever the register held. The
//! class is "a live read of a lane no instruction writes", and that is what
//! this checks, over the whole corpus and the gl_test and dEQP shapes, on
//! what selection hands the allocator (`alloc::undefined_lane_reads`: the
//! allocator's own model of which lanes an instruction consumes, over its
//! lane-need fixpoint, so a lane of a result nobody needs does not count).

#![cfg(all(feature = "gpu-vivante", feature = "shaderc-online", not(feature = "no-shader-table")))]

use yakogl::shaderc::mesa_reference::SHC_SOURCE_TABLE;
use yakogl::shaderc::online::arena::Arena;
use yakogl::shaderc::online::{lower, select, sema, vivante::alloc};

#[test]
fn no_selected_instruction_reads_a_lane_nothing_writes() {
    let mut sources: Vec<(String, sema::Stage, String)> = SHC_SOURCE_TABLE
        .iter()
        .map(|e| {
            let st = if e.name.contains(".vert") { sema::Stage::Vertex } else { sema::Stage::Fragment };
            (e.name.to_string(), st, e.text.trim_end_matches('\0').to_string())
        })
        .collect();
    for (n, t) in [
        ("es100/loopcarry.frag", include_str!("../tools/shaders/es100/loopcarry.frag")),
        ("es100/deadloop.frag", include_str!("../tools/shaders/es100/deadloop.frag")),
        ("es100/branchlane.frag", include_str!("../tools/shaders/es100/branchlane.frag")),
        ("es100/branchquad.frag", include_str!("../tools/shaders/es100/branchquad.frag")),
        ("es100/ifloop.frag", include_str!("../tools/shaders/es100/ifloop.frag")),
        ("es100/contloops.frag", include_str!("../tools/shaders/es100/contloops.frag")),
        ("es100/ifoutput.frag", include_str!("../tools/shaders/es100/ifoutput.frag")),
        ("es100/ifoutput2.frag", include_str!("../tools/shaders/es100/ifoutput2.frag")),
        ("es100/intlit.frag", include_str!("../tools/shaders/es100/intlit.frag")),
        ("es100/idiv.frag", include_str!("../tools/shaders/es100/idiv.frag")),
        ("es100/transc.frag", include_str!("../tools/shaders/es100/transc.frag")),
        ("es100/vtransc.frag", include_str!("../tools/shaders/es100/vtransc.frag")),
        ("es100/matvary.frag", include_str!("../tools/shaders/es100/matvary.frag")),
        ("es100/immneg.frag", include_str!("../tools/shaders/es100/immneg.frag")),
        ("es100/affectand.frag", include_str!("../tools/shaders/es100/affectand.frag")),
    ] {
        sources.push((n.to_string(), sema::Stage::Fragment, t.to_string()));
    }
    for (n, t) in [
        ("es100/ioswap.vert", include_str!("../tools/shaders/es100/ioswap.vert")),
        ("es100/immneg.vert", include_str!("../tools/shaders/es100/immneg.vert")),
        ("es100/matvary.vert", include_str!("../tools/shaders/es100/matvary.vert")),
        ("es100/anyall.vert", include_str!("../tools/shaders/es100/anyall.vert")),
        ("es100/mvstruct.vert", include_str!("../tools/shaders/es100/mvstruct.vert")),
    ] {
        sources.push((n.to_string(), sema::Stage::Vertex, t.to_string()));
    }

    let (mut checked, mut source_undefined, mut bad) = (0usize, 0usize, Vec::new());
    for (name, stage, text) in &sources {
        let mut mem = vec![0u8; 8 << 20];
        let a = Arena::new(&mut mem);
        let Ok(src) = a.alloc_str(text) else { continue };
        let Ok(l) = lower::compile_to_ir(&a, *stage, src) else { continue };
        let Ok(sel) = select::select(&l.module, &a) else { continue };
        checked += 1;
        /* What each io register defines (inputs) and hands on (outputs): a
         * varying or attribute its io row's width, the point size one lane.
         * Position and colour are whole, and listed nowhere. */
        let inputs: Vec<(u8, u8)> = sel.io_in.iter().map(|r| (r.reg, r.components)).collect();
        let mut outputs: Vec<(u8, u8)> = sel.io_out.iter().map(|r| (r.reg, r.components)).collect();
        outputs.extend(sel.vs_pointsize_out_reg.map(|r| (r, 1)));
        for (i, n, lanes) in alloc::undefined_lane_reads(sel.instrs, &inputs, &outputs) {
            /* The source's own undefined value, not ours: dEQP's vector_subscript case
             * (es100/dynidx.frag) declares `tmp` with no initialiser and
             * writes it lane by lane at run-time indices, so the select that
             * merges the writes reads the lanes none of them reached. */
            if name == "dynidx.frag" {
                source_undefined += 1;
                continue;
            }
            bad.push(format!(
                "{name}: instruction {i} ({}) source {n} reads lanes {lanes:#06b} nothing writes; dst {:?}",
                sel.instrs[i].op.op().name,
                sel.instrs[i].dst
            ));
        }
    }
    println!(
        "checked {checked} programs, {source_undefined} undefined in the source, {} undefined lane reads",
        bad.len()
    );
    for b in bad.iter().take(1000) {
        println!("  {b}");
    }
    /* The population: the programs selected. Per build: a report-es2 build
     * refuses GLSL ES 3.00 in the front end (pp.rs out_of_scope). Measured
     * 2026-10-24; 551 / 366 while the Qt shaders were in the GC7000 table,
     * until #1509. */
    let es2 = cfg!(feature = "report-es2");
    assert_eq!(checked, if es2 { 196 } else { 239 }, "the population moved: {checked} programs selected");
    assert_eq!(source_undefined, 1, "the source-level undefined lanes (dynidx.frag's tmp) moved");
    assert!(bad.is_empty(), "{} reads of lanes nothing computes", bad.len());
}
