// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! `yakogl-tools online-check`: the **in-process** front end as a batch
//! service, speaking the same JSON protocol as [`crate::shadercheck`].
//!
//! `shader-check` answers for the pipeline that ships today (glslang, then
//! the regular-expression rewrite, then `pco_shc`). This answers for
//! `src/shaderc/online/`, the compiler being built for the target. Both speak
//! the same protocol so that `tools/es100_oracle.py` can ask the same
//! thousands of dEQP shaders of either one and the answers are comparable:
//! the measurement is the point of the tool, not a side effect.
//!
//! It runs the whole front end -- preprocess, parse, semantic analysis,
//! reflection, lowering to IR -- and the `phase` in a rejection says which
//! one refused the shader.
//!
//! It must run all of it, and this stopped at semantic analysis until
//! 2026-09-22. That is not a smaller measurement, it is a wrong one: a
//! shader the analyser accepts and the lowering refuses is a rejection in
//! the real pipeline and was scored here as a pass. The whole
//! `shaders.conditionals` group went that way -- 1,527 dEQP cases declare
//! their result at file scope, the lowering had no globals, and
//! `tools/online_oracle.py` reported 0 false rejects throughout. Every
//! stage the compiler has belongs here, or the score is about the tool.
//!
//! What it must never do is guess. The online front end has no link step yet,
//! so `{"op": "link"}` gets `{"error": ...}` -- a *service* error, which the
//! oracle is written to treat as "this pipeline cannot answer", never as a
//! verdict on the shader. Answering `ok: true` there would turn every
//! `link_fail` case into a silent false pass and make the score look better
//! than the compiler is.

use crate::{Error, R};
use serde_json::{json, Value};
use std::io::{BufRead, Write};
use yakogl::shaderc::online::{arena::Arena, lower, parser, pp, reflect, sema, Error as FeError};

const USAGE: &str = "usage: yakogl-tools online-check [--scratch KIB]";

const HELP: &str = "
The in-process (online) GLSL ES front end as a batch service: one JSON
request per line on stdin, one reply per line on stdout, the same protocol
`shader-check` speaks.

    {\"op\": \"compile\", \"stage\": \"vert\" | \"frag\", \"source\": \"<GLSL ES>\"}
      -> {\"ok\": true} | {\"ok\": false, \"phase\": \"...\", \"error\": \"<info log>\"}
    {\"op\": \"link\", ...}
      -> {\"error\": ...}   the online front end has no link step yet

options:
  -h, --help      show this help message and exit
  --scratch KIB   the compiler's scratch buffer, in KiB (default 4096). The
                  library allocates nothing; this is the buffer it is lent.
";

/// The default scratch buffer. A target sizes its own from what the largest
/// shader it must compile actually used; 4 MiB is simply generous enough that
/// a host measurement never reports "out of scratch" as if it were a verdict
/// about the shader.
const DEFAULT_SCRATCH_KIB: usize = 4096;

pub fn main(args: &[String]) -> R<()> {
    let mut scratch_kib = DEFAULT_SCRATCH_KIB;
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                println!("{USAGE}\n{HELP}");
                return Ok(());
            }
            "--scratch" => {
                let Some(v) = it.next().and_then(|v| v.parse::<usize>().ok()).filter(|v| *v > 0) else {
                    return Err(Error::Usage(format!("{USAGE}\n\n--scratch takes a positive number of KiB")));
                };
                scratch_kib = v;
            }
            other => return Err(Error::Usage(format!("{USAGE}\n\nunknown option '{other}'"))),
        }
    }

    let mut scratch = vec![0u8; scratch_kib * 1024];
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    for line in stdin.lock().lines() {
        let line = line.map_err(|e| Error::Fatal(format!("stdin: {e}")))?;
        if line.trim().is_empty() {
            continue;
        }
        let reply = handle(&line, &mut scratch);
        writeln!(stdout, "{reply}").map_err(|e| Error::Fatal(format!("stdout: {e}")))?;
        stdout.flush().map_err(|e| Error::Fatal(format!("stdout: {e}")))?;
    }
    Ok(())
}

fn handle(line: &str, scratch: &mut [u8]) -> Value {
    let req: Value = match serde_json::from_str(line) {
        Ok(v) => v,
        Err(e) => return json!({ "error": format!("malformed request: {e}") }),
    };
    match req.get("op").and_then(|v| v.as_str()) {
        Some("compile") => {
            let Some(source) = req.get("source").and_then(|v| v.as_str()) else {
                return json!({ "error": "compile: no source" });
            };
            let stage = match req.get("stage").and_then(|v| v.as_str()) {
                Some("vert") => sema::Stage::Vertex,
                Some("frag") => sema::Stage::Fragment,
                other => return json!({ "error": format!("compile: unknown stage {other:?}") }),
            };
            compile(source, stage, scratch)
        }
        Some("link") => json!({ "error": "the online front end has no link step yet (M2 in progress)" }),
        other => json!({ "error": format!("unknown op {other:?}") }),
    }
}

/// One shader through the whole online front end, naming the stage that
/// refused it.
///
/// `stage` is the shader stage, where the rules start to differ: `attribute`
/// and `discard` belong to one each, and a `varying` is writable in the
/// vertex shader and read-only in the fragment one.
///
/// This repeats the body of
/// [`yakogl::shaderc::online::lower::compile_to_ir`] rather than calling it,
/// and only so that a rejection can say WHICH pass refused: the front end
/// returns one `Error` type from all five. Keep the two in step -- a pass
/// added there and not here is a pass this tool stops measuring, silently,
/// which is the defect this function was written to end.
fn compile(source: &str, stage: sema::Stage, scratch: &mut [u8]) -> Value {
    let a = Arena::new(scratch);
    let pre = match pp::preprocess(&a, source) {
        Ok(p) => p,
        Err(e) => return refused("preprocess", e),
    };
    let ast = match parser::parse(&a, pre.tokens).and_then(|t| parser::resolve_io(&a, t, stage)) {
        Ok(t) => t,
        Err(e) => return refused("parse", e),
    };
    let types = match sema::check(&a, &ast, stage) {
        Ok(t) => t,
        Err(e) => return refused("semantic", e),
    };
    let reflection = match reflect::reflect(&a, &ast) {
        Ok(r) => r,
        Err(e) => return refused("reflect", e),
    };
    match lower::lower(&a, &ast, stage, &reflection, types) {
        Ok(_) => json!({ "ok": true }),
        Err(e) => refused("lower", e),
    }
}

/// A rejection, in the shape an application scraping `glGetShaderInfoLog`
/// has always seen: string 0, then the line, then the token.
fn refused(phase: &str, e: FeError<'_>) -> Value {
    json!({
        "ok": false,
        "phase": phase,
        "error": if e.tok.is_empty() {
            format!("0:{}: {}", e.line, e.msg)
        } else {
            format!("0:{}: '{}' : {}", e.line, e.tok, e.msg)
        },
    })
}
