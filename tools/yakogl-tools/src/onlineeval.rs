// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! `yakogl-tools online-eval`: compile a vertex and fragment pair with the
//! in-process front end and RUN it with the reference IR evaluator
//! (`src/shaderc/online/eval.rs`), answering with the fragment colour.
//!
//! `online-check` asks whether the front end accepts a shader. This asks
//! what the accepted shader COMPUTES, which is the question every
//! wrong-picture defect of 2026-09-23 slipped through: integer literals
//! lowered as floats, constructors that did not convert, scopes that did not
//! end, bool uniforms read as 0/1 -- each compiled, and each was found only
//! by running the module. `tools/online_eval_oracle.py` drives it with
//! dEQP's own input and reference values, so the corpus that scores
//! compile verdicts can also score results, with no GPU and no back end.
//!
//! It measures the FRONT END only: the module the lowering produced, run by
//! an evaluator that implements the IR contract (`ir.md`). A back end that
//! miscompiles a correct module is invisible here, by construction.
//!
//! One JSON request per line on stdin, one reply per line on stdout:
//!
//! ```text
//! {"vert": "<GLSL>", "frag": "<GLSL>",
//!  "attribs":  [["name", [c0, ...], "type"], ...],     floats; a matN is N columns
//!  "uniforms": [["name", "type", [c0, c1, ...]], ...]} components in GLSL order
//!   -> {"color": [r, g, b, a]}                          both stages ran
//!    | {"discarded": true}                               the fragment was discarded
//!    | {"refused": "<stage>: <info log>"}                the front end refused
//!    | {"unmeasured": "<why>"}                           the evaluator cannot say
//!    | {"undefined": "<where>"}                           a lane no instruction defined reached
//!                                                         an output, a branch or an index
//! ```
//!
//! `undefined` is a defect of the front end, never of the shader: a value's
//! lanes past its width (lane 1..3 of a dot product, of `any`, of `all`)
//! hold nothing on a scalar GPU, and a module that reads them draws garbage
//! there. The evaluator used to broadcast a reduction into every lane, which
//! hid `faceforward`'s unsplatted condition (13 dEQP cases wrong on the
//! AM62P, 2026-09-24). The oracle counts `undefined` as a wrong result.
//! it (`src/gl/program.rs`): a float's bits, an int's bits, a bool as 0 or
//! 1, a matrix column by column at the reflection's stride. A uniform name
//! the program does not declare in a stage is skipped for that stage (dEQP
//! sets every value on the program; each stage sees its own). A name that
//! neither stage declares is `unmeasured`, never silently ignored.
//!
//! `unmeasured` is its own outcome and is never folded into pass or fail:
//! a texture, a derivative, a uniform the evaluator cannot place.

use crate::{Error, R};
use serde_json::{json, Value};
use std::io::{BufRead, Write};
use yakogl::shaderc::online::arena::Arena;
use yakogl::shaderc::online::eval::{self, Inputs};
use yakogl::shaderc::online::ir::{IoKind, IoSlot, Module};
use yakogl::shaderc::online::lower::{compile_to_ir, Lowered};
use yakogl::shaderc::online::sema::Stage;

const USAGE: &str = "usage: yakogl-tools online-eval";

pub fn main(args: &[String]) -> R<()> {
    if let Some(a) = args.first() {
        if a == "-h" || a == "--help" {
            println!("{USAGE}\n\nsee the module documentation in tools/yakogl-tools/src/onlineeval.rs");
            return Ok(());
        }
        return Err(Error::Usage(format!("{USAGE}\n\nunknown option '{a}'")));
    }
    let mut vbuf = vec![0u8; 4 << 20];
    let mut fbuf = vec![0u8; 4 << 20];
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    for line in stdin.lock().lines() {
        let line = line.map_err(|e| Error::Fatal(format!("stdin: {e}")))?;
        if line.trim().is_empty() {
            continue;
        }
        let reply = match serde_json::from_str::<Value>(&line) {
            Ok(req) => handle(&req, &mut vbuf, &mut fbuf),
            Err(e) => json!({ "error": format!("malformed request: {e}") }),
        };
        writeln!(stdout, "{reply}").map_err(|e| Error::Fatal(format!("stdout: {e}")))?;
        stdout.flush().map_err(|e| Error::Fatal(format!("stdout: {e}")))?;
    }
    Ok(())
}

fn handle(req: &Value, vbuf: &mut [u8], fbuf: &mut [u8]) -> Value {
    let (Some(vs), Some(fs)) = (req.get("vert").and_then(Value::as_str), req.get("frag").and_then(Value::as_str))
    else {
        return json!({ "error": "a request needs vert and frag" });
    };
    let va = Arena::new(vbuf);
    let fa = Arena::new(fbuf);
    let v = match compile_to_ir(&va, Stage::Vertex, vs) {
        Ok(l) => l,
        Err(e) => return json!({ "refused": format!("vert: 0:{}: '{}' : {}", e.line, e.tok, e.msg) }),
    };
    let f = match compile_to_ir(&fa, Stage::Fragment, fs) {
        Ok(l) => l,
        Err(e) => return json!({ "refused": format!("frag: 0:{}: '{}' : {}", e.line, e.tok, e.msg) }),
    };
    match run(req, &v, &f) {
        Ok(r) => r,
        Err(why) if why.contains("unset lane:") => json!({ "undefined": why }),
        Err(why) => json!({ "unmeasured": why }),
    }
}

/// Run the pair: the vertex shader with the attributes, its varyings into
/// the fragment shader by NAME (each stage numbers its own), then the
/// fragment colour.
fn run(req: &Value, v: &Lowered<'_>, f: &Lowered<'_>) -> Result<Value, String> {
    let vm = &v.module;
    let fm = &f.module;
    let uniforms = req.get("uniforms").and_then(Value::as_array).cloned().unwrap_or_default();
    let vblock = block_of(vm, &uniforms)?;
    let fblock = block_of(fm, &uniforms)?;
    for u in &uniforms {
        let name = u.get(0).and_then(Value::as_str).unwrap_or("");
        if !vm.uniforms.iter().chain(fm.uniforms.iter()).any(|d| d.name == name) {
            return Err(format!("uniform '{name}' is declared by neither stage"));
        }
    }

    let mut vio: Vec<(IoSlot, [u32; 4])> = Vec::new();
    for a in req.get("attribs").and_then(Value::as_array).into_iter().flatten() {
        let name = a.get(0).and_then(Value::as_str).unwrap_or("");
        let Some(d) = vm.inputs.iter().find(|d| d.name == name && d.kind == IoKind::Attribute) else {
            /* dEQP sets every attribute it declared; one the compiler
             * dropped as unused is simply not read. */
            continue;
        };
        let comps: Vec<f32> = a
            .get(1)
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .map(|x| x.as_f64().map(|f| f as f32).ok_or("an attribute component is not a number"))
            .collect::<Result<_, _>>()?;
        /* A matN attribute is N consecutive locations, one column each
         * (ES 2.0 2.10.4), which only the TYPE can say: a mat2 has as many
         * components as a vec4. */
        let ty = a.get(2).and_then(Value::as_str).unwrap_or("");
        let (cols, rows) = match mat_shape(ty) {
            Some(cr) => cr,
            None if comps.len() <= 4 => (1, comps.len()),
            None => return Err(format!("attribute '{name}' with {} components and no matrix type", comps.len())),
        };
        if comps.len() != cols * rows {
            return Err(format!("attribute '{name}': {} components for a {ty}", comps.len()));
        }
        for c in 0..cols {
            let mut lanes = [0.0f32, 0.0, 0.0, 1.0];
            lanes[..rows].copy_from_slice(&comps[c * rows..(c + 1) * rows]);
            vio.push((IoSlot::Attrib(d.location + c as u8), eval::f32x4(lanes)));
        }
    }
    /* An attribute the request did not set reads the current generic value,
     * which GL initialises to (0, 0, 0, 1) (ES 2.0 section 2.7). */
    for d in vm.inputs.iter().filter(|d| d.kind == IoKind::Attribute) {
        if !vio.iter().any(|(s, _)| *s == IoSlot::Attrib(d.location)) {
            vio.push((IoSlot::Attrib(d.location), eval::f32x4([0.0, 0.0, 0.0, 1.0])));
        }
    }
    let vout = eval::run(vm, &Inputs { io: &vio, uniforms: &vblock }).map_err(|w| format!("vert: evaluator: {w}"))?;

    /* Varyings by name: a stored Varying(l) belongs to the VS output whose
     * location is the greatest one <= l with that kind, and lands at the
     * same offset from the FS input of the same name (a matN varying spans
     * N locations). The values are constant over the primitive in dEQP's
     * cases, so interpolation is the identity. */
    let mut fio: Vec<(IoSlot, [u32; 4])> = vec![
        (IoSlot::FragCoord, eval::f32x4([0.5, 0.5, 0.5, 1.0])),
        (IoSlot::FrontFacing, [u32::MAX; 4]),
        (IoSlot::PointCoord, eval::f32x4([0.5, 0.5, 0.0, 0.0])),
    ];
    for d in fm.inputs.iter().filter(|d| d.kind == IoKind::Varying) {
        let Some(src) = vm.outputs.iter().find(|o| o.kind == IoKind::Varying && o.name == d.name) else {
            return Err(format!("fragment varying '{}' has no vertex output", d.name));
        };
        let span = varying_span(vm, src.location);
        for k in 0..span {
            let Some(val) = vout.output(IoSlot::Varying(src.location + k)) else {
                return Err(format!("varying '{}' location {} is never written", d.name, src.location + k));
            };
            if let Some(lane) = undefined_lane(vout.output_defined(IoSlot::Varying(src.location + k)), src.components) {
                return Ok(
                    json!({ "undefined": format!("vert: varying '{}' location {} lane {lane}", d.name, src.location + k) }),
                );
            }
            fio.push((IoSlot::Varying(d.location + k), val));
        }
    }
    let fout = eval::run(fm, &Inputs { io: &fio, uniforms: &fblock }).map_err(|w| format!("frag: evaluator: {w}"))?;
    if fout.discarded {
        return Ok(json!({ "discarded": true }));
    }
    let Some(c) = fout.output_f32(IoSlot::FragColor(0)) else {
        return Err("gl_FragColor is never written".into());
    };
    let comps = fm.outputs.iter().find(|o| o.kind == IoKind::Colour).map_or(4, |o| o.components);
    if let Some(lane) = undefined_lane(fout.output_defined(IoSlot::FragColor(0)), comps) {
        return Ok(json!({ "undefined": format!("frag: the colour's lane {lane}") }));
    }
    Ok(json!({ "color": c }))
}

/// The first lane below `comps` that no instruction defined, if any.
fn undefined_lane(defined: Option<[bool; 4]>, comps: u8) -> Option<usize> {
    let d = defined?;
    (0..usize::from(comps).min(4)).find(|&k| !d[k])
}

/// How many consecutive locations the VS varying at `loc` spans: up to the
/// next declared varying's location, at most 4 (a mat4).
fn varying_span(m: &Module<'_>, loc: u8) -> u8 {
    let next = m.outputs.iter().filter(|o| o.kind == IoKind::Varying && o.location > loc).map(|o| o.location).min();
    match next {
        Some(n) => (n - loc).min(4),
        None => {
            /* The last varying: as many locations as the module stores to. */
            let mut span = 1u8;
            for op in m.instrs {
                if let yakogl::shaderc::online::ir::Op::StoreOutput { slot: IoSlot::Varying(l), .. } = *op {
                    if l >= loc && l - loc < 4 {
                        span = span.max(l - loc + 1);
                    }
                }
            }
            span
        }
    }
}

/// One stage's uniform block, written as the GL core writes it.
fn block_of(m: &Module<'_>, uniforms: &[Value]) -> Result<Vec<u32>, String> {
    let mut block = vec![0u32; usize::from(m.uniform_dwords) + 64];
    for u in uniforms {
        let name = u.get(0).and_then(Value::as_str).ok_or("a uniform without a name")?;
        let ty = u.get(1).and_then(Value::as_str).ok_or("a uniform without a type")?;
        let comps: Vec<f64> =
            u.get(2).and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_f64).collect();
        let Some(d) = m.uniforms.iter().find(|d| d.name == name) else { continue };
        if d.array_len > 1 {
            return Err(format!("uniform '{name}' is an array, which this tool does not place"));
        }
        let at = usize::from(d.dword_off);
        let bits = |x: f64| -> u32 {
            match ty.trim_start_matches(|c: char| c.is_ascii_digit()) {
                t if t.starts_with("int") || t.starts_with("ivec") => x as i32 as u32,
                /* ES 3.00's unsigned types: the value's own bits. */
                t if t.starts_with("uint") || t.starts_with("uvec") => x as u32,
                t if t.starts_with("bool") || t.starts_with("bvec") => u32::from(x != 0.0),
                _ => (x as f32).to_bits(),
            }
        };
        if let Some((nc, nr)) = mat_shape(ty) {
            /* Column by column at the reflection's stride: a column of three
             * or four rows is padded to four dwords, one of two is not. */
            let stride = usize::from(d.dwords) / nc;
            for c in 0..nc {
                for r in 0..nr {
                    let x = *comps.get(c * nr + r).ok_or("a matrix value with too few components")?;
                    block[at + c * stride + r] = bits(x);
                }
            }
        } else {
            for (k, x) in comps.iter().enumerate() {
                block[at + k] = bits(*x);
            }
        }
    }
    Ok(block)
}

/// `matN` or ES 3.00's `matCxR` as (columns, rows); `None` for any other type.
fn mat_shape(ty: &str) -> Option<(usize, usize)> {
    let s = ty.strip_prefix("mat")?;
    match s.split_once('x') {
        Some((c, r)) => Some((c.parse().ok()?, r.parse().ok()?)),
        None => s.parse().ok().map(|n| (n, n)),
    }
}
