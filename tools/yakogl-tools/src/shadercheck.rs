// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! `yakogl-tools shader-check`: the GLSL ES front end as a batch service.
//!
//! One JSON object per line on stdin, one per line on stdout. It exists so
//! that the two things which must agree about whether a shader compiles — the
//! offline table generator and whatever runs the shaders at run time — call
//! the same code, and so that a harness can ask about thousands of shaders
//! without paying for a process each (`tools/es100_oracle.py` asks about
//! ~5,800 of them).
//!
//! Requests:
//!
//! ```json
//! {"op": "compile", "stage": "vert" | "frag", "source": "<GLSL ES text>"}
//! {"op": "link", "vert": "<GLSL ES text>", "frag": "<GLSL ES text>"}
//! ```
//!
//! Replies:
//!
//! ```json
//! {"ok": true}
//! {"ok": false, "phase": "validate" | "preprocess" | "translate" | "codegen" | "link",
//!  "error": "<the info log, as glGetShaderInfoLog would give it>"}
//! {"error": "<this tool is broken, not the shader>"}
//! ```
//!
//! `ok: false` means *the application's shader is wrong*; a bare `error` means
//! the service is. A caller that cannot tell them apart reports a missing
//! compiler as a conformance failure, so the two are never merged.
//!
//! Modes:
//!
//! * default — the front end only: validate, preprocess, translate, and check
//!   the translation with `glslangValidator -V`. No `pco_shc`, so it runs
//!   anywhere and answers in milliseconds.
//! * `--codegen` — also run `pco_shc`, i.e. the whole offline pipeline.
//! * `--legacy` — the pipeline as it was before the ES front end existed:
//!   translate first, then judge the *rewritten GLSL 450*. Only useful for
//!   measuring what the front end changed; see `tools/es100_oracle.py`.

use crate::es100::{self, Diag, Phase, Scratch};
use crate::glsl;
use crate::glsl2usc;
use crate::{Error, R};
use serde_json::{json, Value};
use std::io::{BufRead, Write};

const USAGE: &str = "usage: yakogl-tools shader-check [--codegen] [--legacy]";

const HELP: &str = "
The GLSL ES front end as a batch service: one JSON request per line on stdin,
one reply per line on stdout (see the module documentation in shadercheck.rs).

options:
  -h, --help   show this help message and exit
  --codegen    also run pco_shc (the whole offline pipeline), not just the
               front end
  --legacy     the pipeline as it was before the ES front end: translate first
               and judge the rewritten GLSL 450. For measurement only.
";

struct Mode {
    codegen: bool,
    legacy: bool,
}

/// One stage's verdict: Ok(()) or the info log that refused it.
fn compile_one(scratch: &Scratch, mode: &Mode, stage: &str, source: &str) -> R<Result<(), Diag>> {
    let mut text = source.to_string();
    if !mode.legacy {
        // 1. GLSL ES rules, on the application's own bytes.
        if let Err(d) = es100::validate(scratch, stage, &text)? {
            return Ok(Err(d));
        }
        // 2. The ES preprocessor, so the rewriting below sees only live code.
        match es100::preprocess(scratch, stage, &text)? {
            Ok(t) => text = t,
            Err(d) => return Ok(Err(d)),
        }
    }
    // 3. The rewrite into the GLSL 450 the PCO pipeline compiles.
    //
    // The translator's own refusals (a construct the rewriting cannot express,
    // a character set it will not take) are diagnosed input, not a crash: they
    // mean this implementation will not compile the shader, so they are a
    // compile failure with a reason, not a broken service.
    let short = glsl2usc::stage_short(stage)?;
    match glsl::es_version(&text).and_then(|es| match es {
        Some(es) => glsl::translate(&text, short, es).map(|(t, _)| Some(t)),
        None => Ok(None),
    }) {
        Ok(Some(t)) => text = t,
        Ok(None) => {}
        Err(Error::Exit(m)) => return Ok(Err(Diag { phase: Phase::Translate, log: format!("{m}\n") })),
        Err(e) => return Err(e),
    }
    // 4. What the rewrite produced must still be a legal shader.
    if let Err(d) = es100::spirv_check(scratch, stage, &text)? {
        return Ok(Err(d));
    }
    if mode.codegen {
        if let Err(d) = es100::codegen_check(scratch, stage, &text)? {
            return Ok(Err(d));
        }
    }
    Ok(Ok(()))
}

/// The cross-stage verdict.
///
/// Before the ES front end this was only what the offline backend does at
/// `glLinkProgram`: every fragment varying must be written by the vertex
/// shader, matched by name (`shc_link_by_name`). That is the rule the GPU
/// needs; it is not the whole of what GL ES 2.0 calls a link error.
fn link_two(scratch: &Scratch, mode: &Mode, vert: &str, frag: &str) -> R<Result<(), Diag>> {
    if !mode.legacy {
        return es100::link(scratch, vert, frag);
    }
    let (vs, fs) = match (translated(vert, "vs"), translated(frag, "fs")) {
        (Ok(v), Ok(f)) => (v, f),
        (Err(Error::Exit(m)), _) | (_, Err(Error::Exit(m))) => {
            return Ok(Err(Diag { phase: Phase::Translate, log: format!("{m}\n") }))
        }
        (Err(e), _) | (_, Err(e)) => return Err(e),
    };
    let written = glsl::reflect_varyings(&vs, "vs")?;
    let read = glsl::reflect_varyings(&fs, "fs")?;
    let names: std::collections::HashSet<&String> = written.values().collect();
    for name in read.values() {
        if !name.is_empty() && !names.contains(name) {
            return Ok(Err(Diag {
                phase: Phase::Link,
                log: format!("'{name}' is not written by the vertex shader\n"),
            }));
        }
    }
    Ok(Ok(()))
}

/// The GLSL 450 a source becomes (or the source itself when it is already 450).
fn translated(source: &str, short_stage: &str) -> R<String> {
    match glsl::es_version(source)? {
        Some(es) => Ok(glsl::translate(source, short_stage, es)?.0),
        None => Ok(source.to_string()),
    }
}

fn reply_err(d: &Diag) -> Value {
    json!({ "ok": false, "phase": d.phase.as_str(), "error": d.log })
}

/// Answer one request. `Err` here is the *service* failing, which the caller
/// must not mistake for a bad shader.
fn handle(scratch: &Scratch, mode: &Mode, req: &Value) -> R<Value> {
    let op = req.get("op").and_then(Value::as_str).unwrap_or("compile");
    match op {
        "compile" => {
            let stage = req
                .get("stage")
                .and_then(Value::as_str)
                .ok_or_else(|| Error::Fatal("compile request has no 'stage'".into()))?;
            let source = req
                .get("source")
                .and_then(Value::as_str)
                .ok_or_else(|| Error::Fatal("compile request has no 'source'".into()))?;
            Ok(match compile_one(scratch, mode, stage, source)? {
                Ok(()) => json!({ "ok": true }),
                Err(d) => reply_err(&d),
            })
        }
        "link" => {
            let vert = req
                .get("vert")
                .and_then(Value::as_str)
                .ok_or_else(|| Error::Fatal("link request has no 'vert'".into()))?;
            let frag = req
                .get("frag")
                .and_then(Value::as_str)
                .ok_or_else(|| Error::Fatal("link request has no 'frag'".into()))?;
            Ok(match link_two(scratch, mode, vert, frag)? {
                Ok(()) => json!({ "ok": true }),
                Err(d) => reply_err(&d),
            })
        }
        other => Err(Error::Fatal(format!("unknown op '{other}'"))),
    }
}

pub fn main(argv: &[String]) -> R<()> {
    let mut mode = Mode { codegen: false, legacy: false };
    for a in argv {
        match a.as_str() {
            "--codegen" => mode.codegen = true,
            "--legacy" => mode.legacy = true,
            "-h" | "--help" => {
                println!("{USAGE}{HELP}");
                return Ok(());
            }
            other => return Err(Error::Usage(format!("{USAGE}\nshader-check: unknown option '{other}'"))),
        }
    }
    let scratch = Scratch::new()?;
    let stdin = std::io::stdin();
    let mut out = std::io::stdout().lock();
    for line in stdin.lock().lines() {
        let line = line.map_err(|e| Error::Fatal(format!("stdin: {e}")))?;
        if line.trim().is_empty() {
            continue;
        }
        let reply = match serde_json::from_str::<Value>(&line) {
            Ok(req) => match handle(&scratch, &mode, &req) {
                Ok(v) => v,
                /* A broken service, not a broken shader: no "ok" field. */
                Err(Error::Exit(m)) | Err(Error::Fatal(m)) | Err(Error::Usage(m)) => json!({ "error": m }),
            },
            Err(e) => json!({ "error": format!("malformed request: {e}") }),
        };
        writeln!(out, "{reply}").map_err(|e| Error::Fatal(format!("stdout: {e}")))?;
        out.flush().map_err(|e| Error::Fatal(format!("stdout: {e}")))?;
    }
    Ok(())
}
