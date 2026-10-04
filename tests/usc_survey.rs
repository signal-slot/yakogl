// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The PowerVR corpus through the front end and USC selection, by reason.
//!
//! The Vivante side reports "137 of 142" and a work list; this is the same
//! report for the USC half, and it exists so that the second back end's
//! progress is a number somebody else can check rather than a claim.
//!
//! Two rungs, because they fail differently and a single figure would hide
//! which:
//!
//! * **lowered** -- the front end accepted the source and produced IR. This
//!   is a property of es100-frontend's half and is the same number for both
//!   machines.
//! * **selected** -- USC selection produced groups. Everything short of that
//!   is a named refusal, and the histogram below is the work list.
//!
//! Printed, not asserted, for the same reason the Vivante surveys are: a
//! score is a quality, and a test that pins one stops it improving. What IS
//! asserted is that nothing panics and that every refusal carries a reason --
//! an unnamed refusal is a gap nobody can act on.

#![cfg(all(feature = "shaderc-online", not(feature = "no-shader-table")))]

use std::collections::BTreeMap;
use yakogl::shaderc::online::arena::Arena;
use yakogl::shaderc::online::powervr::select::{select, SelectError};
use yakogl::shaderc::online::{lower, sema};
use yakogl::shaderc::table::SHC_SOURCE_TABLE;

/// Its own arena rather than `store`'s: this walks the whole corpus and has
/// no interest in the product's budgets, and `store`'s scratch is shared with
/// tests that run beside it.
const SCRATCH: usize = 4 << 20;

#[test]
fn the_powervr_corpus_through_selection() {
    let mut lowered = 0usize;
    let mut selected = 0usize;
    let mut total = 0usize;
    let mut groups_total = 0usize;
    let mut refusals: BTreeMap<String, usize> = BTreeMap::new();
    let mut rejects: BTreeMap<String, usize> = BTreeMap::new();

    for e in SHC_SOURCE_TABLE.iter() {
        let stage = if e.name.contains(".vert") { sema::Stage::Vertex } else { sema::Stage::Fragment };
        let text = e.text.strip_suffix('\0').unwrap_or(e.text);
        total += 1;

        let mut mem = vec![0u8; SCRATCH];
        let a = Arena::new(&mut mem);
        let src = a.alloc_str(text).expect("the source fits");
        match lower::compile_to_ir(&a, stage, src) {
            Err(err) => {
                *rejects.entry(String::from(err.msg)).or_insert(0) += 1;
            }
            Ok(l) => {
                lowered += 1;
                match select(&l.module, &a) {
                    Ok(s) => {
                        selected += 1;
                        groups_total += s.groups.len();
                    }
                    Err(SelectError::Unsupported(why)) => {
                        *refusals.entry(String::from(why)).or_insert(0) += 1;
                    }
                    Err(other) => {
                        *refusals.entry(format!("{other:?}")).or_insert(0) += 1;
                    }
                }
            }
        }
    }

    println!("\n=== the USC half, over the corpus ===\n");
    println!("  sources                    {total}");
    println!("  lowered to IR              {lowered}");
    println!("  selected into USC groups   {selected}");
    println!("  groups emitted             {groups_total}");

    println!("\n  why selection refused -- the work list");
    let mut rows: Vec<_> = refusals.iter().collect();
    rows.sort_by(|a, b| b.1.cmp(a.1));
    for (why, n) in rows {
        println!("    {n:>5}  {why}");
    }

    println!("\n  why the front end refused (es100-frontend's half, for context)");
    let mut rows: Vec<_> = rejects.iter().collect();
    rows.sort_by(|a, b| b.1.cmp(a.1));
    for (why, n) in rows.iter().take(8) {
        println!("    {n:>5}  {why}");
    }

    /* The one thing asserted: every refusal names something. An empty reason
     * is a gap nobody can act on, and it would read as progress here. */
    assert!(refusals.keys().all(|k| !k.trim().is_empty()), "a refusal with no reason");
    assert!(total > 0, "the source table is empty; this survey would report nothing and look fine");
}

/// The USC-1 vehicle: the `hello` pair, group by group.
///
/// `hello.vert` is `gl_Position = transformationMatrix * myVertex` and
/// `hello.frag` is a literal colour -- the pair the PowerVR table has a
/// shipped blob for, and the one the milestone diffs against. Printing the
/// groups here gives shaderc-remote's allocator and pvrgpu-platform's builder
/// the exact input they have to consume, rather than a description of it.
///
/// It asserts only that both stages select. What the groups ARE is the
/// milestone's question, and it is settled against the shipped blob rather
/// than against an expectation written here.
#[test]
fn the_usc1_vehicle_selects_and_here_are_its_groups() {
    for (file, stage) in [("hello.vert", sema::Stage::Vertex), ("hello.frag", sema::Stage::Fragment)] {
        let path = format!("tools/shaders/es100/{file}");
        let Ok(text) = std::fs::read_to_string(&path) else {
            println!("  {file}: missing ({path})");
            continue;
        };
        let mut mem = vec![0u8; SCRATCH];
        let a = Arena::new(&mut mem);
        let src = a.alloc_str(&text).expect("the source fits");
        let l = lower::compile_to_ir(&a, stage, src).unwrap_or_else(|e| panic!("{file}: {}", e.msg));
        let s = select(&l.module, &a).unwrap_or_else(|e| panic!("{file}: {e:?}"));

        println!("\n=== {file}: {} groups, {} vregs ===", s.groups.len(), s.vregs);
        for (i, g) in s.groups.iter().enumerate() {
            let srcs: Vec<String> = g.srcs.iter().flatten().map(|x| format!("{x:?}")).collect();
            println!("  [{i:>2}] {:?} {:?} -> {:?}", g.op, g.ty, g.dst);
            for x in srcs {
                println!("         {x}");
            }
        }
        for (slot, comps) in s.inputs {
            println!("  in  {slot:?} {comps:?}");
        }
        for (slot, comps) in s.outputs {
            println!("  out {slot:?}");
            for c in comps.iter().flatten() {
                println!("         {c:?}");
            }
        }
    }
}
