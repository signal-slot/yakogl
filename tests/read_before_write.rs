// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Every register a selected program reads is written before it is read.
//!
//! The property, over the whole corpus, on BOTH back ends. It exists because
//! it did not: on 2026-09-22 the GC7000 if-conversion emitted
//!
//! ```text
//!   mov    t7, u1          ; v = a
//!   mov    t7, attr0       ; the else arm, over the top of a
//!   select t7, c, t8, t7   ; t8 IS NEVER WRITTEN
//! ```
//!
//! for `v = a; if (c) { } else { v = b; }`, in **30 of the 137 sources that
//! compile** -- the whole FemtoVG fill, gradient, image and stencil set that
//! the UI draws with -- and the USC selection had the same defect. It
//! compiles, it draws, and on one side of the condition it uses whatever that
//! register happened to hold.
//!
//! Nothing caught it. Every host test passed, `gl_test` returned 25/29 with
//! the same four failures on every board run, and the suite was silent rather
//! than right: not one `gl_test` ES 1.00 shader contains an `if` at all.
//!
//! So this is the check that would have caught it, phrased as the property
//! rather than as the symptom, and it runs in `check.sh`.
//!
//! # What counts as written
//!
//! A virtual register is defined when an earlier instruction (or group)
//! writes it. On the GC7000 an io INPUT is not a virtual register at all
//! (`SelSrc::Input`), so it needs no definition; on the USC a varying IS a
//! virtual register, filled by an iterator the builder emits before the
//! program runs, so `Selected::inputs` seeds the written set there.
//!
//! # TWO QUESTIONS, and they have different answers
//!
//! Do not read the two counts as one number:
//!
//! * **"read and never written at all"** -- a register that appears as no
//!   instruction's destination anywhere in the program. It is asserted to
//!   be 0.
//! * **"read before anything wrote it, in emitted order"** -- this file's
//!   walk. 608 on the GC7000 and 294 on the USC, because a slot's register IS
//!   written, later, by the join. Those are the seeds described below.
//!
//! Both are true at once, and the assertion that matters is neither: it is
//! that no value nothing produced reaches an OUTPUT.
//!
//! # What the survivors are, and why they are not the same defect
//!
//! 608 reads on the GC7000 and 294 on the USC still read a register nothing
//! wrote, and **none of them reaches an output**. They are one shape: an
//! if-conversion seed copying a slot before that slot's first store, and the
//! join that selects it. It happens for a variable DECLARED INSIDE an arm --
//! `if (r > 0.0) { vec2 pt = ...; float d = f(pt); return g(d); }` in
//! femtovg's `scissorMask` is the pattern -- where the arm writes the real
//! value before reading it, so the copied garbage is overwritten and never
//! observed.
//!
//! That is what the taint below demonstrates rather than what anyone argued:
//! a register redefined from clean sources loses the taint, and by the time
//! any value leaves the program none of it depends on an undefined read. The
//! seeds are waste, not error; the waste is `reads_or_partly_writes` not
//! asking whether the read comes before the arm's first write.
//!
//! # The population
//!
//! A source the front end or the back end refuses has no program to check,
//! which is not a pass. Both halves therefore print `checked + refused` and
//! assert that the sum is the whole corpus, so a run that quietly checked six
//! shaders cannot look like a clean sweep.
//!
//! ```sh
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online \
//!     --test read_before_write -- --nocapture
//! ```
#![cfg(all(feature = "shaderc-online", not(feature = "no-shader-table")))]

use std::string::String;
use std::vec::Vec;
use yakogl::shaderc::online::arena::Arena;
use yakogl::shaderc::online::{lower, sema};
use yakogl::shaderc::table::SHC_SOURCE_TABLE;

/// Generous, so "out of scratch" cannot be read as a refusal.
const SCRATCH: usize = 4 << 20;

fn stage_of(name: &str) -> sema::Stage {
    if name.contains(".vert") {
        sema::Stage::Vertex
    } else {
        sema::Stage::Fragment
    }
}

/// The GC7000 half: `SelInstr`s, `SelSrc::Virtual` reads, `SelDst::Virtual`
/// writes.
#[cfg(feature = "gpu-vivante")]
fn vivante_half() {
    use yakogl::shaderc::online::select::select;
    use yakogl::shaderc::online::vivante::alloc::{SelDst, SelSrc};

    let (mut checked, mut refused, mut instrs) = (0usize, 0usize, 0usize);
    let mut undefined_reads = 0usize;
    let mut branchy: Vec<&str> = Vec::new();
    let mut never_written: Vec<String> = Vec::new();
    let mut bad: Vec<String> = Vec::new();

    for e in SHC_SOURCE_TABLE.iter() {
        let text = e.text.strip_suffix('\0').unwrap_or(e.text);
        let mut mem = vec![0u8; SCRATCH];
        let a = Arena::new(&mut mem);
        let Ok(src) = a.alloc_str(text) else { continue };
        let Ok(l) = lower::compile_to_ir(&a, stage_of(e.name), src) else {
            refused += 1;
            continue;
        };
        let Ok(s) = select(&l.module, &a) else {
            refused += 1;
            continue;
        };
        checked += 1;
        instrs += s.instrs.len();
        if s.instrs.iter().any(|i| i.target.is_some()) {
            branchy.push(e.name);
        }

        /* The other question: a register that never appears as a
         * destination ANYWHERE in the program. Pinned at 0. */
        for ins in s.instrs.iter() {
            for r in ins.srcs.iter().flatten() {
                if let SelSrc::Virtual { v, .. } = r {
                    let defined =
                        s.instrs.iter().any(|o| matches!(o.dst, Some(SelDst::Virtual { v: d, .. }) if d.0 == v.0));
                    if !defined {
                        never_written.push(format!("  {}: v{} is read and never written at all", e.name, v.0));
                    }
                }
            }
        }

        let mut written: Vec<u32> = Vec::new();
        /* Registers whose value depends on a read of something nothing
         * wrote. A `select` with one tainted operand is tainted: its result
         * may be that operand. */
        let mut tainted: Vec<u32> = Vec::new();
        for (i, ins) in s.instrs.iter().enumerate() {
            let mut reads_undefined = false;
            for r in ins.srcs.iter().flatten() {
                if let SelSrc::Virtual { v, .. } = r {
                    if !written.contains(&v.0) {
                        undefined_reads += 1;
                        reads_undefined = true;
                    } else if tainted.contains(&v.0) {
                        reads_undefined = true;
                    }
                }
            }
            match ins.dst {
                Some(SelDst::Virtual { v, .. }) => {
                    written.push(v.0);
                    /* A REDEFINITION FROM CLEAN SOURCES CLEARS THE TAINT.
                     * Without this the arm-local seeds poison everything
                     * downstream of them: the seed copies a slot before its
                     * first store, the arm then writes the real value into
                     * the same register, and the garbage is gone. Counting it
                     * as tainted anyway reported 14 sources writing outputs
                     * from undefined values that do not. */
                    tainted.retain(|t| *t != v.0);
                    if reads_undefined {
                        tainted.push(v.0);
                    }
                }
                /* An io register, or an instruction with no destination at
                 * all -- a `texkill`, whose effect is the fragment. Either
                 * way the value has left the program. */
                _ if reads_undefined => bad.push(format!(
                    "  {}: instruction {i} ({}) writes an OUTPUT from a value nothing produced",
                    e.name,
                    ins.op.op().name
                )),
                _ => {}
            }
        }
    }

    println!("\n=== GC7000: {checked} sources checked, {refused} refused, {instrs} instructions ===");
    println!("  reads of a register nothing wrote: {undefined_reads}, of which reaching an output: {}", bad.len());
    /* WHERE THIS IS OPTIMISTIC, said rather than left to be discovered: the
     * walk is over the emitted list in order, so a register written inside
     * ONE arm of a real branch counts as written for everything after the
     * join. For an if-converted program (both arms always run) that is
     * exact; for a program with a `branch` it can miss a read that is
     * undefined on one path. Those sources are counted, not excluded. */
    println!("  of the {checked} checked, {} contain a real branch, where the walk is OPTIMISTIC:", branchy.len());
    for n in &branchy {
        println!("    optimistic: {n}");
    }
    assert_eq!(
        checked + refused,
        SHC_SOURCE_TABLE.len(),
        "the population must be the whole corpus: {checked} checked + {refused} refused of {}",
        SHC_SOURCE_TABLE.len()
    );
    assert!(checked > 100, "only {checked} sources reached selection; this checked almost nothing");
    assert!(
        bad.is_empty(),
        "a value nothing produced reaches an output -- the picture depends on what a register happened to hold:\n{}",
        bad.join("\n")
    );
    assert!(
        never_written.is_empty(),
        "registers read and never written at all (must be 0):\n{}",
        never_written.join("\n")
    );
}

#[cfg(not(feature = "gpu-vivante"))]
fn vivante_half() {
    println!("\n=== GC7000: not this build's back end ===");
}

/// The USC half: `SelGroup`s, and a varying IS a virtual register.
fn powervr_half() {
    use yakogl::shaderc::online::powervr::sel::{SelDst, SelGroup, SelOp, SelSrc};
    use yakogl::shaderc::online::powervr::select::select;
    /* The virtual registers a group defines: one, except a sample, whose
     * destination names the first of the FOUR its fetch writes
     * (`SelOp::Sample`). Counting one read lanes 1..3 of every texture2D as
     * values nothing produced. */
    let defs = |g: &SelGroup| -> std::ops::Range<u32> {
        match (g.op, g.dst) {
            (SelOp::Sample { .. }, Some(SelDst::Virtual(v))) => v.0..v.0 + 4,
            (_, Some(SelDst::Virtual(v))) => v.0..v.0 + 1,
            _ => 0..0,
        }
    };

    let (mut checked, mut refused, mut groups) = (0usize, 0usize, 0usize);
    let mut undefined_reads = 0usize;
    let mut never_written: Vec<String> = Vec::new();
    let mut bad: Vec<String> = Vec::new();

    for e in SHC_SOURCE_TABLE.iter() {
        let text = e.text.strip_suffix('\0').unwrap_or(e.text);
        let mut mem = vec![0u8; SCRATCH];
        let a = Arena::new(&mut mem);
        let Ok(src) = a.alloc_str(text) else { continue };
        let Ok(l) = lower::compile_to_ir(&a, stage_of(e.name), src) else {
            refused += 1;
            continue;
        };
        let Ok(s) = select(&l.module, &a) else {
            refused += 1;
            continue;
        };
        checked += 1;
        groups += s.groups.len();
        for g in s.groups.iter() {
            for r in g.srcs.iter().flatten() {
                if let SelSrc::Virtual { v, .. } = r {
                    let defined = s.groups.iter().any(|o| defs(o).contains(&v.0))
                        || s.inputs.iter().any(|(_, c)| c.iter().flatten().any(|iv| iv.0 == v.0));
                    if !defined {
                        never_written.push(format!("  {}: v{} is read and never written at all", e.name, v.0));
                    }
                }
            }
        }

        /* A fragment varying is read out of a register an iterator fills
         * before the first group runs, so it is written without a group
         * writing it. */
        let mut written: Vec<u32> = Vec::new();
        for (_, comps) in s.inputs.iter() {
            for v in comps.iter().flatten() {
                written.push(v.0);
            }
        }
        let mut tainted: Vec<u32> = Vec::new();
        for (i, g) in s.groups.iter().enumerate() {
            let mut reads_undefined = false;
            for r in g.srcs.iter().flatten() {
                if let SelSrc::Virtual { v, .. } = r {
                    if !written.contains(&v.0) {
                        undefined_reads += 1;
                        reads_undefined = true;
                    } else if tainted.contains(&v.0) {
                        reads_undefined = true;
                    }
                }
            }
            match g.dst {
                Some(SelDst::Virtual(_)) => {
                    for v in defs(g) {
                        written.push(v);
                        /* A redefinition from clean sources clears the taint;
                         * see the GC7000 half. */
                        tainted.retain(|t| *t != v);
                        if reads_undefined {
                            tainted.push(v);
                        }
                    }
                }
                _ if reads_undefined => bad.push(format!(
                    "  {}: group {i} ({:?}) writes a FIXED register from a value nothing produced",
                    e.name, g.op
                )),
                _ => {}
            }
        }
        /* The outputs are not groups: the builder writes them from the
         * operands this list carries, so a tainted value reaches the picture
         * there too. */
        for (slot, comps) in s.outputs.iter() {
            for r in comps.iter().flatten() {
                if let SelSrc::Virtual { v, .. } = r {
                    if !written.contains(&v.0) || tainted.contains(&v.0) {
                        bad.push(format!("  {}: output {slot:?} takes v{}, a value nothing produced", e.name, v.0));
                    }
                }
            }
        }
    }

    println!("=== USC: {checked} sources checked, {refused} refused, {groups} groups ===");
    println!("  reads of a register nothing wrote: {undefined_reads}, of which reaching an output: {}", bad.len());
    assert_eq!(
        checked + refused,
        SHC_SOURCE_TABLE.len(),
        "the population must be the whole corpus: {checked} checked + {refused} refused of {}",
        SHC_SOURCE_TABLE.len()
    );
    assert!(checked > 50, "only {checked} sources reached selection; this checked almost nothing");
    assert!(bad.is_empty(), "a value nothing produced reaches an output:\n{}", bad.join("\n"));
    assert!(never_written.is_empty(), "registers read and never written at all:\n{}", never_written.join("\n"));
}

/// Both halves, in ONE test: each compiles the corpus, and a binary that
/// resets the compiler's state runs one test (`src/test_rules.rs`).
#[test]
fn every_register_a_program_reads_is_written_first() {
    vivante_half();
    powervr_half();
}
