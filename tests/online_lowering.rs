// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! How far the online front end gets on the offline table's GLSL ES 1.00
//! sources, and -- for every source it does not get through -- the *first*
//! construct it stops on.
//!
//! `tests/online_reflection.rs` measures the reflection, which needs the
//! preprocessor, the parser and the type checker. This measures the whole
//! front end including the lowering to IR, which is what the code generator
//! consumes: a source counts here only if `compile_to_ir` returns a module.
//!
//! The point of the test is the *list*, not the number. Work on a front end
//! that is deliberately incomplete is only as good as its ordering, and the
//! ordering has to come from a measurement: the histogram below says which
//! missing construct stands in front of how many real shaders, so the next
//! thing built is the one that unblocks the most sources rather than the one
//! that is easiest to remember. Run it with the output visible:
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online \
//!     --test online_lowering -- --nocapture
//! ```
//!
//! The table compiled in is whichever GPU the build is for, so the GC7000 row
//! and the PowerVR row are separate measurements of the same front end over
//! different corpora. Each has its own floor, and the floor is a ratchet: it
//! may only be raised. A source that stops being lowered is a regression even
//! if the count as a whole went up.
// The corpus lives in the offline table, so this measures nothing in a build
// that carries none (`no-shader-table`, the image configuration).
#![cfg(not(feature = "no-shader-table"))]
#![cfg(feature = "shaderc-online")]

use std::collections::BTreeMap;

use yakogl::shaderc::online::lower::compile_to_ir;
use yakogl::shaderc::online::{arena::Arena, sema::Stage};
use yakogl::shaderc::table;

/// Scratch for one compile: generous, so "out of scratch" can never be read
/// as a missing language feature.
const SCRATCH: usize = 4 << 20;

/// The ratchet. Raise it when a pass lands; never lower it.
///
/// The two corpora are measured separately: they are different shader sets
/// and the constructs they stop on already differ -- only the PowerVR set
/// writes `gl_FragData[0]` -- so one floor for both would let a regression in
/// the larger corpus hide behind the smaller one.
///
/// The denominators move when the tables are regenerated, so a floor is a
/// count of sources and never a fraction: the GC7000 set was 125 sources
/// before the ES 1.00 `gl_test` twins were added to the table and 142 after,
/// and a ratio would have appeared to fall while nothing regressed.
/// The two now stand at different numbers, which is the split earning itself:
/// the GC7000 table has the ES 1.00 `gl_test` twins and the PowerVR table does
/// not yet (it is rebuilt on the board, task #39), so seven of the ten are
/// sources the PowerVR corpus simply does not contain.
#[cfg(feature = "gpu-vivante")]
const FLOOR: usize = 139;
#[cfg(not(feature = "gpu-vivante"))]
const FLOOR: usize = 235;

/// Is this GLSL ES 1.00? A source with no `#version` is, by section 3.3.
fn is_es100(text: &str) -> bool {
    match text.find("#version") {
        None => true,
        Some(i) => text[i..].trim_start_matches("#version").trim_start().starts_with("100"),
    }
}

#[test]
fn every_es100_table_source_is_lowered_or_named() {
    let mut scratch = vec![0u8; SCRATCH];
    let (mut es100, mut lowered, mut no_blob) = (0usize, 0usize, 0usize);
    /* reason -> (how many sources, the tokens it stopped on, one example) */
    let mut buckets: BTreeMap<String, (usize, BTreeMap<String, usize>, String)> = BTreeMap::new();
    let mut ok_names: Vec<&str> = Vec::new();

    for src in table::SHC_SOURCE_TABLE.iter() {
        let text = src.text.trim_end_matches('\0');
        if !is_es100(text) {
            continue;
        }
        /* The stage is not in the source table, so it comes from the blob the
         * source belongs to, found the way the run-time lookup finds it. A
         * source with no blob is not part of this corpus. */
        let key = yakogl::shaderc::sha256::sha256(text.as_bytes());
        let Some(blob) = table::SHC_BLOB_TABLE.iter().find(|b| b.sha256 == key && b.blend_key == 0) else {
            no_blob += 1;
            continue;
        };
        es100 += 1;
        let stage = if blob.stage == yakogl::shaderc::SHC_STAGE_FRAGMENT { Stage::Fragment } else { Stage::Vertex };

        let a = Arena::new(&mut scratch);
        match compile_to_ir(&a, stage, text) {
            Ok(l) => {
                /* THE ONE `Return` IS THE LAST NODE OF THE BODY.
                 *
                 * Selection treats `Node::Return` as the end of the shader --
                 * the GC7000 marks its last instruction rather than branching
                 * -- so a `Return` anywhere else is not an early exit, it is
                 * nothing, and everything after it runs. That is a wrong
                 * picture with no failure anywhere: `texture_copy_unclipped.
                 * frag` had one at node 125 of 104..142, compiled clean, and
                 * counted as a success.
                 *
                 * gles2-core asserts the same thing on their side. This is
                 * the front end's half, over every source in the table,
                 * because the shape that escaped the rewrite was one no
                 * hand-written test had: a bare `return;` at the top level of
                 * `main` with statements after it. */
                let nodes = l.module.nodes_of(l.module.body);
                for (i, n) in nodes.iter().enumerate() {
                    assert!(
                        !matches!(n, yakogl::shaderc::online::ir::Node::Return) || i + 1 == nodes.len(),
                        "{}: Node::Return at {i} of {} -- selection ends the shader there and runs the rest",
                        src.name,
                        nodes.len()
                    );
                }
                lowered += 1;
                ok_names.push(src.name);
                if std::env::var("YAKOGL_SHOW_SIZES").is_ok() {
                    println!(
                        "  SIZE {:<40} locals {:>4}  instrs {:>5}  nodes {:>5}",
                        src.name,
                        l.module.locals.len(),
                        l.module.instrs.len(),
                        l.module.nodes.len()
                    );
                }
            }
            Err(e) => {
                /* Every refusal has to say WHERE and WHICH. Without the line
                 * a rejection cannot be located in the source at all, and
                 * without the token every refusal of a kind is the same
                 * string -- one dEQP run produced 1,841 byte-identical
                 * `0:0: assignment to this is not lowered yet`. Checked over
                 * the real corpus rather than on one hand-written source,
                 * because a path that forgets the line is exactly the path
                 * no hand-written test covers. */
                assert!(e.line > 0, "{}: refusal with no line: {:?} {}", src.name, e.tok, e.msg);
                assert!(!e.tok.is_empty(), "{}: refusal with no token: line {} {}", src.name, e.line, e.msg);
                let b = buckets.entry(e.msg.to_string()).or_insert_with(|| (0, BTreeMap::new(), String::new()));
                b.0 += 1;
                *b.1.entry(e.tok.to_string()).or_insert(0) += 1;
                if b.2.is_empty() {
                    b.2 = format!("{} line {}", src.name, e.line);
                }
            }
        }
    }

    let mut rows: Vec<_> = buckets.iter().collect();
    rows.sort_by_key(|(msg, (n, _, _))| (usize::MAX - *n, msg.as_str()));

    println!("\nonline front end: {lowered} of {es100} ES 1.00 table sources lowered to IR");
    if no_blob > 0 {
        println!("  ({no_blob} ES 1.00 sources have no blob in this table and are not part of the corpus)");
    }
    for name in &ok_names {
        println!("  LOWERED {name}");
    }
    println!("\n  the {} refused, by the first construct each stops on:", es100 - lowered);
    for (msg, (n, toks, example)) in rows {
        let mut t: Vec<_> = toks.iter().collect();
        t.sort_by_key(|(tok, n)| (usize::MAX - **n, tok.as_str()));
        let shown: Vec<String> = t.iter().take(6).map(|(tok, n)| format!("{tok} x{n}")).collect();
        println!("  {n:>4}  {msg}");
        println!("        on: {}{}", shown.join(", "), if t.len() > 6 { ", ..." } else { "" });
        println!("        e.g. {example}");
    }
    println!();

    assert!(
        lowered >= FLOOR,
        "the online front end lowered {lowered} of {es100} ES 1.00 table sources, below the floor of {FLOOR}: \
         a source that used to reach the IR no longer does"
    );
}

/// What the `if` arms in the corpus actually contain, for the back end's
/// real-branch selection.
///
/// The code generator refuses an `if` whose arm has a texture fetch or a
/// `discard`, because if-conversion executes both arms and neither may be
/// speculated: a fetch outside the taken path costs bandwidth and may sample
/// an incomplete texture, and a speculated `discard` kills a fragment that
/// should have lived. Fifteen of the corpus's fragment shaders are in that
/// bucket, all of one family.
///
/// Building real branches against the general case is much more work than
/// building them against the shapes that occur, so this measures the shapes:
/// what is in each arm, and -- the question that decides whether a merge is
/// needed at the join -- whether a value an arm defines is read after it.
///
/// Prints rather than asserts. It is a description of the corpus, and a
/// number that moved would be information, not a failure.
///
/// ```text
/// cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online \
///     --test online_lowering the_shapes_of_if_arms -- --nocapture
/// ```
#[test]
fn the_shapes_of_if_arms_the_back_end_cannot_flatten() {
    use yakogl::shaderc::online::ir::{Node, Op, Range as IrRange};

    let mut scratch = vec![0u8; SCRATCH];
    /* Totals over the corpus, and the per-source lines for the ones that
     * have an arm the back end cannot flatten. */
    let mut sources_with_if = 0usize;
    let mut lines: Vec<String> = Vec::new();

    /// The counts the walk accumulates, in one place so the recursion
    /// takes one reference rather than four.
    #[derive(Default)]
    struct Tally {
        ifs: usize,
        with_tex: usize,
        with_discard: usize,
        live_out: usize,
    }

    let mut tally = Tally::default();

    for src in table::SHC_SOURCE_TABLE.iter() {
        let text = src.text.trim_end_matches('\0');
        if !is_es100(text) {
            continue;
        }
        let key = yakogl::shaderc::sha256::sha256(text.as_bytes());
        let Some(blob) = table::SHC_BLOB_TABLE.iter().find(|b| b.sha256 == key && b.blend_key == 0) else {
            continue;
        };
        let stage = if blob.stage == yakogl::shaderc::SHC_STAGE_FRAGMENT { Stage::Fragment } else { Stage::Vertex };
        let a = Arena::new(&mut scratch);
        let Ok(l) = compile_to_ir(&a, stage, text) else { continue };
        let m = &l.module;

        /* Walk a run, describing every `If` in it and recursing into the
         * arms: an arm may hold an `if` of its own. */
        fn walk(
            m: &yakogl::shaderc::online::ir::Module<'_>,
            r: IrRange,
            name: &str,
            out: &mut Vec<String>,
            t: &mut Tally,
        ) {
            let nodes = m.nodes_of(r);
            for (i, n) in nodes.iter().enumerate() {
                let Node::If { then_, else_, .. } = *n else { continue };
                t.ifs += 1;
                /* What the arms hold, and which values they define. */
                let mut tex = 0;
                let mut disc = 0;
                let mut defined: Vec<u32> = Vec::new();
                let mut arm_nodes = 0;
                for arm in [then_, else_] {
                    for an in m.nodes_of(arm) {
                        arm_nodes += 1;
                        if let Node::Instr(id) = *an {
                            match m.instr(id) {
                                Some(Op::Tex { .. }) => tex += 1,
                                Some(Op::Discard) => disc += 1,
                                _ => {}
                            }
                            defined.push(id.0);
                        }
                    }
                }
                /* Is a value an arm defined read after the join? That is the
                 * question that decides whether a real branch needs a merge
                 * at all: with the front end putting every cross-arm value in
                 * a slot, the answer should be no. */
                let mut escapes = false;
                for later in &nodes[i + 1..] {
                    if let Node::Instr(id) = *later {
                        if let Some(op) = m.instr(id) {
                            let mut uses = |v: u32| {
                                if defined.contains(&v) {
                                    escapes = true;
                                }
                            };
                            match op {
                                Op::Alu(_, srcs) => srcs.iter().for_each(|s| uses(s.val.0)),
                                Op::StoreOutput { src, .. } => uses(src.val.0),
                                Op::StoreLocal { src, index, .. } => {
                                    uses(src.val.0);
                                    if let Some(x) = index {
                                        uses(x.val.0);
                                    }
                                }
                                Op::LoadLocal { index: Some(x), .. } | Op::LoadUniformIdx { index: x, .. } => {
                                    uses(x.val.0)
                                }
                                op @ (Op::Tex { .. } | Op::TexSize { .. }) => {
                                    let (srcs, n) = op.tex_srcs();
                                    srcs[..n].iter().flatten().for_each(|s| uses(s.val.0));
                                }
                                _ => {}
                            }
                        }
                    }
                }
                if tex > 0 {
                    t.with_tex += 1;
                }
                if disc > 0 {
                    t.with_discard += 1;
                }
                if escapes {
                    t.live_out += 1;
                }
                if tex > 0 || disc > 0 {
                    out.push(format!(
                        "  {name:<44} arm nodes {arm_nodes:>3}  tex {tex}  discard {disc}  \
                         value read after the join: {}",
                        if escapes { "YES" } else { "no" }
                    ));
                }
                for arm in [then_, else_] {
                    walk(m, arm, name, out, t);
                }
            }
        }

        let before = lines.len();
        walk(m, m.body, src.name, &mut lines, &mut tally);
        if lines.len() > before {
            sources_with_if += 1;
        }
    }

    println!("\nif arms with a texture fetch or a discard, by source:");
    for l in &lines {
        println!("{l}");
    }
    println!(
        "\n  {sources_with_if} sources, {} `if` nodes in the corpus, {} arms with a fetch, \
         {} with a discard, {} whose arm defines a value read after the join",
        tally.ifs, tally.with_tex, tally.with_discard, tally.live_out
    );
}
