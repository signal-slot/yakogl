// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! `yakogl-tools ir-dump`: the node shape the online front end gives a
//! shader, printed for a human.
//!
//! Three of us had rebuilt this by hand as a throwaway test before it was
//! worth writing down: gles2-core diffing the emitted words against the
//! reference, cts-runner bisecting a GPU wedge, and me comparing the vertex
//! and fragment lowerings of one function. The question each time was the
//! same -- "what SHAPE does the front end give this?" -- and the answer is
//! the structured control flow, not the instruction list.
//!
//! It prints `nodes`, not `instrs`. That distinction is the reason the tool
//! exists rather than a `{:?}` on the module: `instrs` is a POOL whose index
//! is allocation order, and reading it as execution order is a mistake this
//! compiler has made twice, once in the lowering and once in a test of it.
//! The walk here follows `Module::nodes_of`, which is the order the shader
//! runs in, and prints each instruction under the node that runs it.
//!
//! READ-ONLY. It compiles a file and writes to stdout; it never touches the
//! tree, the table or a board.

use crate::{Error, R};
use yakogl::shaderc::online::{arena::Arena, ir, lower, sema::Stage};

const USAGE: &str = "usage: yakogl-tools ir-dump [--stage vert|frag|both] [--scratch KIB] <file.glsl>...

Prints the IR node shape the online front end produces: every If with its
arms and their ranges, every Loop, every Return, and the instruction under
the node that runs it.

The stage defaults to the file's extension (.vert/.frag/.vs/.fs) and to
BOTH when the extension does not say, because the interesting question is
usually whether the two stages differ.";

pub fn main(args: &[String]) -> R<()> {
    let mut stage: Option<Stage> = None;
    let mut both_forced = false;
    let mut scratch_kib = 1024usize;
    let mut files: Vec<&str> = Vec::new();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "-h" | "--help" => {
                println!("{USAGE}");
                return Ok(());
            }
            "--stage" => match it.next().map(String::as_str) {
                Some("vert") => stage = Some(Stage::Vertex),
                Some("frag") => stage = Some(Stage::Fragment),
                Some("both") => both_forced = true,
                other => return Err(Error::Usage(format!("{USAGE}\n\nbad --stage {other:?}"))),
            },
            "--scratch" => match it.next().and_then(|v| v.parse::<usize>().ok()).filter(|v| *v > 0) {
                Some(v) => scratch_kib = v,
                None => return Err(Error::Usage(format!("{USAGE}\n\n--scratch wants a positive number of KiB"))),
            },
            other if other.starts_with('-') => {
                return Err(Error::Usage(format!("{USAGE}\n\nunknown option '{other}'")));
            }
            other => files.push(other),
        }
    }
    if files.is_empty() {
        return Err(Error::Usage(USAGE.to_string()));
    }

    let mut buf = vec![0u8; scratch_kib * 1024];
    for f in files {
        let src = std::fs::read_to_string(f).map_err(|e| Error::Exit(format!("ir-dump: {f}: {e}")))?;
        let stages: &[Stage] = match (stage, both_forced, stage_of_name(f)) {
            (Some(s), false, _) => &[s][..],
            (_, true, _) => &[Stage::Vertex, Stage::Fragment][..],
            (None, false, Some(s)) => match s {
                Stage::Vertex => &[Stage::Vertex][..],
                Stage::Fragment => &[Stage::Fragment][..],
            },
            (None, false, None) => &[Stage::Vertex, Stage::Fragment][..],
        };
        for s in stages {
            dump_one(f, *s, &src, &mut buf);
        }
    }
    Ok(())
}

/// The stage a file name says, when it says one.
fn stage_of_name(f: &str) -> Option<Stage> {
    let lower = f.to_ascii_lowercase();
    if lower.ends_with(".vert") || lower.ends_with(".vs") {
        return Some(Stage::Vertex);
    }
    if lower.ends_with(".frag") || lower.ends_with(".fs") {
        return Some(Stage::Fragment);
    }
    None
}

fn name_of(s: Stage) -> &'static str {
    match s {
        Stage::Vertex => "vertex",
        Stage::Fragment => "fragment",
    }
}

fn dump_one(file: &str, stage: Stage, src: &str, buf: &mut [u8]) {
    println!("\n===== {file}  [{}] =====", name_of(stage));
    let a = Arena::new(buf);
    let l = match lower::compile_to_ir(&a, stage, src) {
        Ok(l) => l,
        Err(e) => {
            println!("  REFUSED line {} at {:?}: {}", e.line, e.tok, e.msg);
            return;
        }
    };
    let m = &l.module;
    let mut c = Counts::default();
    println!("  body = {{first:{}, len:{}}}", m.body.first, m.body.len);
    walk(m, m.body, 1, &mut c);
    println!(
        "  TOTALS  instrs {}  nodes {}  locals {}  |  Instr {}  If {}  Loop {}  Return {}",
        m.instrs.len(),
        m.nodes.len(),
        m.locals.len(),
        c.instr,
        c.ifs,
        c.loops,
        c.rets
    );
    for i in m.inputs.iter() {
        println!("  in   {:?} {:?} comps {}", i.name, i.kind, i.components);
    }
    for o in m.outputs.iter() {
        println!("  out  {:?} {:?} comps {}", o.name, o.kind, o.components);
    }
}

#[derive(Default)]
struct Counts {
    instr: usize,
    ifs: usize,
    loops: usize,
    rets: usize,
}

/// Walk in EXECUTION order, which is what `nodes` gives and `instrs` does
/// not: an instruction's pool index is where it was allocated.
fn walk(m: &ir::Module, r: ir::Range, depth: usize, c: &mut Counts) {
    let pad = "  ".repeat(depth);
    for (i, n) in m.nodes_of(r).iter().enumerate() {
        match n {
            ir::Node::Instr(id) => {
                c.instr += 1;
                match m.instrs.get(id.0 as usize) {
                    Some(op) => println!("{pad}[{i}] instr[{}] {op:?}", id.0),
                    None => println!("{pad}[{i}] instr[{}] <out of range>", id.0),
                }
            }
            ir::Node::If { cond, then_, else_ } => {
                c.ifs += 1;
                println!(
                    "{pad}[{i}] If cond=v{} sw{:?}  then={{first:{},len:{}}}  else={{first:{},len:{}}}",
                    cond.val.0, cond.swizzle, then_.first, then_.len, else_.first, else_.len
                );
                println!("{pad}  then:");
                walk(m, *then_, depth + 2, c);
                println!("{pad}  else:");
                walk(m, *else_, depth + 2, c);
            }
            ir::Node::Loop { body } => {
                c.loops += 1;
                println!("{pad}[{i}] Loop body={{first:{},len:{}}}", body.first, body.len);
                walk(m, *body, depth + 2, c);
            }
            ir::Node::Return => {
                c.rets += 1;
                println!("{pad}[{i}] Return");
            }
            other => println!("{pad}[{i}] {other:?}"),
        }
    }
}
