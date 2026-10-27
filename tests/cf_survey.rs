// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! How much control flow the corpus actually needs, and of what kind.
//!
//! Control flow is the whole of the GC7000 back end's remaining refusals, and
//! "control flow" is not one job. A compiler can serve an `if` three ways --
//! if-convert it into a `select`, unroll a loop, or emit real branches and
//! back-patch them -- and they are wildly different amounts of work. This
//! counts which of them the corpus is actually asking for, so the order is
//! chosen from a measurement rather than from the order they appear in a
//! textbook.
//!
//! ```sh
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online \
//!     --test cf_survey -- --nocapture
//! ```
//!
//! Prints rather than asserts: it is a work list, and every number in it is a
//! fact about the corpus that no assertion would make truer.

#![cfg(all(feature = "shaderc-online", feature = "gpu-vivante"))]

use std::collections::BTreeMap;
use yakogl::shaderc::mesa_reference::{SHC_BLOB_TABLE, SHC_SOURCE_TABLE};
use yakogl::shaderc::online::lower::compile_to_ir;
use yakogl::shaderc::online::{arena::Arena, ir, sema::Stage};

/// Generous, so "out of scratch" can never be read as a missing feature.
const SCRATCH: usize = 4 << 20;

/// Does this range hold anything that must not run speculatively?
///
/// The reference compiler if-converts an `if` -- evaluating both arms and
/// choosing with a `select` -- unless an arm holds something it may not
/// execute on the wrong side of the condition. Measured: a texture fetch makes
/// it emit a real `branch_unary`, and arms of seven ALU operations each do
/// not. A `discard` is in the same class by construction.
fn must_not_speculate(m: &ir::Module<'_>, r: ir::Range) -> bool {
    m.nodes_of(r).iter().any(|n| match n {
        ir::Node::Instr(id) => {
            matches!(m.instr(*id), Some(ir::Op::Tex { .. } | ir::Op::Discard))
        }
        ir::Node::If { then_, else_, .. } => must_not_speculate(m, *then_) || must_not_speculate(m, *else_),
        ir::Node::Loop { body } => must_not_speculate(m, *body),
        _ => false,
    })
}

#[test]
fn what_kind_of_control_flow_does_the_corpus_need() {
    let mut scratch = vec![0u8; SCRATCH];
    let mut kinds: BTreeMap<&str, usize> = BTreeMap::new();
    let mut per_source: BTreeMap<usize, usize> = BTreeMap::new();
    let (mut with_cf, mut if_only, mut needs_loops, mut cannot_convert) = (0, 0, 0, 0);
    let (mut loop_names, mut unsafe_names): (Vec<&str>, Vec<&str>) = (Vec::new(), Vec::new());

    for src in SHC_SOURCE_TABLE.iter() {
        let text = src.text.trim_end_matches('\0');
        let key = yakogl::shaderc::sha256::sha256(text.as_bytes());
        let Some(blob) = SHC_BLOB_TABLE.iter().find(|b| b.sha256 == key && b.blend_key == 0) else { continue };
        let stage = if blob.stage == yakogl::shaderc::SHC_STAGE_FRAGMENT { Stage::Fragment } else { Stage::Vertex };
        let a = Arena::new(&mut scratch);
        let Ok(m) = compile_to_ir(&a, stage, text) else { continue };

        let (mut n_if, mut n_loop, mut n_brk, mut n_cont) = (0usize, 0usize, 0usize, 0usize);
        for n in m.module.nodes {
            match n {
                ir::Node::If { .. } => n_if += 1,
                ir::Node::Loop { .. } => n_loop += 1,
                ir::Node::Break => n_brk += 1,
                ir::Node::Continue => n_cont += 1,
                _ => {}
            }
        }
        if n_if + n_loop + n_brk + n_cont == 0 {
            continue;
        }
        with_cf += 1;
        *kinds.entry("if").or_default() += n_if;
        *kinds.entry("loop").or_default() += n_loop;
        *kinds.entry("break").or_default() += n_brk;
        *kinds.entry("continue").or_default() += n_cont;
        *per_source.entry(n_if).or_default() += 1;

        if n_loop == 0 && n_brk == 0 && n_cont == 0 {
            if_only += 1;
        } else {
            needs_loops += 1;
            loop_names.push(src.name);
        }
        /* Only what sits INSIDE an arm blocks conversion. A texture fetch in
         * the straight-line body is not speculated by anything and does not
         * count -- asking the question of the whole body instead of the arms
         * turned 12 sources into 27. */
        let blocked = m.module.nodes.iter().any(|n| match n {
            ir::Node::If { then_, else_, .. } => {
                must_not_speculate(&m.module, *then_) || must_not_speculate(&m.module, *else_)
            }
            _ => false,
        });
        if blocked {
            cannot_convert += 1;
            unsafe_names.push(src.name);
        }
    }

    println!("\n=== what kind of control flow the corpus needs ===\n");
    println!("  sources with any control flow:        {with_cf}");
    println!("    `if` only, no loop or break:        {if_only}");
    println!("    need a loop or a break:             {needs_loops}");
    println!("    hold a texture or discard in an arm: {cannot_convert}   (cannot be if-converted)");
    println!("  node totals:            {kinds:?}");
    println!("  `if` count per source:  {per_source:?}");
    println!("\n  need real branches (texture or discard inside an arm):");
    for n in &unsafe_names {
        println!("    {n}");
    }
    println!("\n  need loops:");
    for n in &loop_names {
        println!("    {n}");
    }
}
