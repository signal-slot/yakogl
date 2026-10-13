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
            /* The source's own undefined value, not ours: Qt's tabled
             * particle shader writes `tt.x = t;` and never `tt.y`, and its
             * fragment shader samples a colour table at `tt`. */
            if name == "imageparticle_tabled.vert" && lanes == 0b0010 {
                source_undefined += 1;
                continue;
            }
            /* Also the source's own: dEQP's vector_subscript case
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
    /* 158 on main; 271 once the GLSL ES 3.00 front end (steps 1 to 2b) lets
     * the table's ES 3.00 sources through; 272 for step 4, because
     * qt3d/skyboxcube_runtime_tonemapping_300.frag, the one table source with a `switch`, gets through once ES3 step 4 lowers it (no undefined read). */
    /* +18 with the cube level/bias arm (f3eeea0), 0 undefined reads among them */
    /* Per build: a report-es2 build (the i.MX8MP's dEQP-GLES2 image) refuses
     * GLSL ES 3.00 in the front end (pp.rs out_of_scope), so the population
     * is the one before the ES 3.00 front end; both measured 2026-09-24. */
    /* +1 in both builds: es100/affectand.frag, dEQP's affect_and (2026-09-24).
     * Draw buffers (MRT step 1: colour outputs at locations 1..3 select)
     * add 19 to the ES 3.00 build, 0 undefined reads among them. */
    let es2 = cfg!(feature = "report-es2");
    /* +6 in both builds (2026-10-02): the six FemtoVG NO_SCISSOR programs
     * (tools/shaders/femtovg/<name>.nodiscard.noscissor.frag, appended to the
     * GC7000 table), 0 undefined reads among them. */
    /* +9 in the ES 3.00 build (2026-10-04): the GC7000 now selects
     * gl_VertexID, texelFetch / textureSize / 3D and array look-ups, dsx and
     * dsy (SRC2) and `%` (builtins::imod), so more table sources get
     * through; 0 undefined reads among them. */
    /* 2026-10-12 (issue #22): 220 rows appended to the GC7000 table
     * (tools/vivante/README.md, "The table against its sources"); one of
     * them, es100/dynidx.frag, is undefined in its own text. */
    assert_eq!(checked, if es2 { 366 } else { 551 }, "the population moved: {checked} programs selected");
    assert_eq!(
        source_undefined, 2,
        "the source-level undefined lanes (imageparticle_tabled's tt.y, dynidx.frag's tmp) moved"
    );
    assert!(bad.is_empty(), "{} reads of lanes nothing computes", bad.len());
}
