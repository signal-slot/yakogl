// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! `glsl2usc --dump-translation` (hidden): what the front end derives from a
//! source without running the compiler: the translated GLSL, the GL names,
//! the reflection and, per variant spec, the blend key and the compiler
//! environment. The same report is produced from the Python original by a
//! verification script, so the two front ends can be compared byte for byte
//! over the whole shader set.

use crate::glsl::{self, GlName};
use crate::glsl2usc::{blend_key, reflect_source, stage_of, variant_env};
use crate::py;
use crate::{Error, R};
use std::fmt::Write;

fn err_line(e: &Error) -> String {
    match e {
        Error::Exit(m) => format!("ERROR: {m}\n"),
        _ => "FATAL\n".into(),
    }
}

fn body(path: &str, o: &mut String) -> R<()> {
    let stage = stage_of(path)?;
    writeln!(o, "stage: {stage}").unwrap();
    let sidecar = format!("{path}.prebuilt.json");
    let over = format!("{path}.override");
    let comp = if !py::exists(&sidecar) && py::exists(&over) { over } else { path.to_string() };
    let refl = reflect_source(&py::read_bytes(&comp)?, stage)?;
    match refl.es {
        Some(v) => writeln!(o, "es: {v}").unwrap(),
        None => writeln!(o, "es: None").unwrap(),
    }
    writeln!(o, "--- translated").unwrap();
    o.push_str(&refl.glsl);
    writeln!(o, "\n--- end").unwrap();
    let mut members: Vec<_> = refl.gl_names.members.iter().collect();
    members.sort_by(|a, b| a.0.cmp(&b.0));
    for (k, v) in members {
        match v {
            GlName::Name(n) => writeln!(o, "name {k} -> {n}").unwrap(),
            GlName::Inner(n, i) => writeln!(o, "name {k} -> {n} inner {i}").unwrap(),
        }
    }
    let mut samplers: Vec<_> = refl.gl_names.samplers.iter().collect();
    samplers.sort();
    for (k, v) in samplers {
        writeln!(o, "sampler {k} -> 0x{v:x}").unwrap();
    }
    let (uniforms, block_dw) = glsl::reflect_uniforms(&refl.glsl, &refl.gl_names)?;
    writeln!(o, "block_dw: {block_dw}").unwrap();
    for u in &uniforms {
        writeln!(
            o,
            "uniform {} off={} dwords={} type=0x{:x} len={} binding={} inner={}",
            u.name, u.dword_off, u.dwords, u.gl_type, u.array_len, u.binding, u.inner
        )
        .unwrap();
    }
    let mut v: Vec<_> = glsl::reflect_varyings(&refl.glsl, stage)?.into_iter().collect();
    v.sort();
    for (loc, n) in v {
        writeln!(o, "varying {loc} {n}").unwrap();
    }
    let mut f: Vec<_> = glsl::reflect_flat_varyings(&refl.glsl, stage)?.into_iter().collect();
    f.sort();
    writeln!(o, "flat: {f:?}").unwrap();
    if stage == "vs" {
        let mut a: Vec<_> = glsl::reflect_attribs(&refl.glsl)?.into_iter().collect();
        a.sort();
        for (loc, (n, t)) in a {
            writeln!(o, "attrib {loc} {n} 0x{t:x}").unwrap();
        }
    }
    Ok(())
}

pub fn dump(path: &str, specs: &[String]) -> String {
    let mut o = format!("=== {path}\n");
    if let Err(e) = body(path, &mut o) {
        o.push_str(&err_line(&e));
    }
    for spec in specs {
        write!(o, "variant {spec}:").unwrap();
        match blend_key(spec) {
            Ok(k) => write!(o, " key=0x{k:x}").unwrap(),
            Err(e) => write!(o, " key {}", err_line(&e).trim_end()).unwrap(),
        }
        match variant_env(spec) {
            Ok(env) => {
                for (k, v) in env {
                    write!(o, " {k}={}", v.as_deref().unwrap_or("-")).unwrap();
                }
                o.push('\n');
            }
            Err(e) => write!(o, " env {}", err_line(&e)).unwrap(),
        }
    }
    o
}
