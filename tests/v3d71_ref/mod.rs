// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The Mesa side of P8's host comparison (docs/notes/pre-study-rpi5.md
//! section 8): one P8 shape (`tests/v3d71_common`) compiled by Mesa 26.2.1's
//! v3d compiler for V3D 7.1, through `v3d_shc --device 71`
//! (tools/v3d/README.md, H2).
//!
//! * [`find_v3d_shc`]: `V3D_SHC`, else `MESA_V3D_TREE/build-v3d/src/broadcom/v3d_shc`
//!   (the tree tools/v3d/README.md sets up). Neither set, or the named file
//!   missing, is a [`Gap`] naming the variable: never a pass.
//! * [`compile71`]: the shape's GLSL ES 1.00 to GLSL 4.60 the way
//!   tools/v3d/glsl2v3d.py does it (`translate_es100_gl` of
//!   tools/vivante/glsl2vivante.py, run through `python3 -I`), SPIR-V by
//!   `glslangValidator -G`, then `v3d_shc vs.spv fs.spv --device 71`. A tool
//!   that is not installed, or a v3d_shc built before `--device` existed, is
//!   a [`Gap`] too; a tool that runs and fails is an error.
//! * [`Mesa71::fs_stream`]: Mesa's FS uniform stream resolved to values (the
//!   shape's uniforms by `uniform_names` slot, the TMU words unrelocated, as
//!   tests/v3d_sim_mesa.rs `stream` does), so the shape's runner
//!   (`v3d71_common::run71_stream`) judges Mesa's program like ours.
//!
//! A test file includes it after `mod v3d71_common;` with `mod v3d71_ref;`
//! and the feature set `--no-default-features --features
//! host-test,gpu-v3d,shaderc-online,no-shader-table`.

#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::v3d71_common::Shape;

/// The reference cannot run here: the reason names what is missing. A test
/// prints it as `GAP ...` and asserts nothing else; it is not a pass.
#[derive(Debug, Clone)]
pub struct Gap(pub String);

/// Mesa's 7.1 program for one shape, read from v3d_shc's JSON.
#[derive(Debug, Clone)]
pub struct Mesa71 {
    pub fs: Stage,
    pub vs: Stage,
    pub cs: Stage,
    /// `gpu.ver` (71 for `--device 71`).
    pub ver: u32,
    /// `uniform_names`.
    pub uniform_names: Vec<UniformName>,
}

/// One entry of v3d_shc's `uniform_names`.
#[derive(Debug, Clone)]
pub struct UniformName {
    /// "vert" or "frag": the stage whose storage holds it.
    pub stage: String,
    pub name: String,
    /// The vec4 slot, or a sampler's binding.
    pub slot: u32,
    pub sampler: bool,
}

/// One stage of [`Mesa71`].
#[derive(Debug, Clone)]
pub struct Stage {
    pub code: Vec<u64>,
    /// (contents, data): Mesa's `enum quniform_contents` and its data word.
    pub uniforms: Vec<(u32, u32)>,
    pub threads: u32,
}

const TREE_REL: &str = "build-v3d/src/broadcom/v3d_shc";

/// Where v3d_shc is, or the gap naming what is not set or not built.
pub fn find_v3d_shc() -> Result<PathBuf, Gap> {
    if let Some(p) = std::env::var_os("V3D_SHC").filter(|p| !p.is_empty()) {
        let p = PathBuf::from(p);
        return if p.is_file() {
            Ok(p)
        } else {
            Err(Gap(format!("V3D_SHC names no file ({}): Mesa's 7.1 output was NOT compared", p.display())))
        };
    }
    if let Some(t) = std::env::var_os("MESA_V3D_TREE").filter(|p| !p.is_empty()) {
        let p = Path::new(&t).join(TREE_REL);
        return if p.is_file() {
            Ok(p)
        } else {
            Err(Gap(format!(
                "MESA_V3D_TREE has no {TREE_REL} (ninja -C build-v3d src/broadcom/v3d_shc, tools/v3d/README.md): \
                 Mesa's 7.1 output was NOT compared"
            )))
        };
    }
    Err(Gap(
        "neither V3D_SHC nor MESA_V3D_TREE is set (tools/v3d/README.md builds v3d_shc): Mesa's 7.1 output was NOT compared"
            .to_string(),
    ))
}

/// Run a tool; a tool that cannot be started is a gap naming it.
fn run(cmd: &mut Command, what: &str) -> Result<Result<std::process::Output, String>, Gap> {
    match cmd.output() {
        Ok(o) => Ok(Ok(o)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            Err(Gap(format!("{what} is not installed: Mesa's 7.1 output was NOT compared")))
        }
        Err(e) => Ok(Err(format!("{what}: {e}"))),
    }
}

/// tools/v3d/glsl2v3d.py's translation (glsl2vivante.translate_es100_gl) of
/// one ES 1.00 source, stage "vs" or "fs".
const TRANSLATE: &str = "import sys\n\
sys.path.insert(0, sys.argv[1])\n\
import glsl2vivante as gv\n\
sys.stdout.write(gv.translate_es100_gl(sys.stdin.read(), sys.argv[2]))\n";

fn translate(src: &str, stage: &str) -> Result<Result<String, String>, Gap> {
    use std::io::Write;
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/tools/vivante");
    let mut child = match Command::new("python3")
        .args(["-I", "-c", TRANSLATE, dir, stage])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
    {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(Gap("python3 is not installed: Mesa's 7.1 output was NOT compared".into()))
        }
        Err(e) => return Ok(Err(format!("python3: {e}"))),
    };
    child.stdin.take().expect("piped").write_all(src.as_bytes()).expect("writes the source");
    let o = child.wait_with_output().expect("python3 runs");
    if !o.status.success() {
        return Ok(Err(format!("translate {stage}: {}", String::from_utf8_lossy(&o.stderr))));
    }
    Ok(Ok(String::from_utf8(o.stdout).expect("utf-8")))
}

/// Mesa's 7.1 program for `shape`: Ok(Err) when a tool ran and failed (the
/// message names the step), Err(Gap) when the reference cannot run here.
pub fn compile71(shc: &Path, shape: &Shape) -> Result<Result<Mesa71, String>, Gap> {
    let work = Path::new(env!("CARGO_TARGET_TMPDIR")).join("v3d71_ref").join(shape.name);
    std::fs::create_dir_all(&work).expect("creates the scratch directory");
    let mut spv = Vec::new();
    for (stage, src, ext) in [("vs", shape.vs, "vert"), ("fs", shape.fs, "frag")] {
        let text = match translate(src, stage)? {
            Ok(t) => t,
            Err(e) => return Ok(Err(format!("{}: {e}", shape.name))),
        };
        let glsl = work.join(format!("{stage}.{ext}"));
        let out = work.join(format!("{stage}.spv"));
        std::fs::write(&glsl, text).expect("writes the GLSL");
        let glslang = std::env::var_os("GLSLANG").unwrap_or_else(|| "glslangValidator".into());
        let o = match run(Command::new(glslang).arg("-G").arg(&glsl).arg("-o").arg(&out), "glslangValidator")? {
            Ok(o) => o,
            Err(e) => return Ok(Err(e)),
        };
        if !o.status.success() {
            return Ok(Err(format!(
                "{}: glslangValidator -G {stage}: {}{}",
                shape.name,
                String::from_utf8_lossy(&o.stdout),
                String::from_utf8_lossy(&o.stderr)
            )));
        }
        spv.push(out);
    }
    let o = match run(Command::new(shc).arg(&spv[0]).arg(&spv[1]).args(["--device", "71"]), "v3d_shc")? {
        Ok(o) => o,
        Err(e) => return Ok(Err(e)),
    };
    let stderr = String::from_utf8_lossy(&o.stderr);
    std::fs::write(work.join("disasm.txt"), stderr.as_bytes()).expect("writes the listing");
    // A v3d_shc built before 3762ec20 has no 7.1 device: the reference is
    // not here (rebuild it), which is not a defect of anything this tests.
    if !o.status.success() && stderr.contains("unknown argument '--device'") {
        return Err(Gap(format!(
            "{} predates --device 71 (rebuild it from tools/v3d/v3d_shc.c, tools/v3d/README.md): \
             Mesa's 7.1 output was NOT compared",
            shc.display()
        )));
    }
    if !o.status.success() {
        let why: Vec<&str> = stderr.lines().filter(|l| !l.starts_with("---") && !l.starts_with(' ')).collect();
        return Ok(Err(format!("{}: v3d_shc --device 71 failed: {}", shape.name, why.join(" | "))));
    }
    let json = String::from_utf8(o.stdout).expect("utf-8 JSON");
    Ok(parse(&json).map_err(|e| format!("{}: v3d_shc JSON: {e}", shape.name)))
}

impl Mesa71 {
    /// Mesa's FS uniform stream for `shape`, resolved to values. Contents
    /// (v3d_compiler.h `enum quniform_contents`): 0 a constant (the TLB
    /// configuration among them); 1 `QUNIFORM_UNIFORM`, the dword slot * 4 +
    /// component of the FS's own uniform storage (`uniform_names`), which
    /// holds the shape's uniform values in declaration order; 40/41 the TMU
    /// configuration P0/P1, whose unit (bits 24..31) the interpreter reads, so
    /// the word goes in unrelocated (tests/v3d_sim_mesa.rs `stream`). Any
    /// other contents is an error naming it.
    pub fn fs_stream(&self, shape: &Shape) -> Result<Vec<u32>, String> {
        let slots: Vec<u32> =
            self.uniform_names.iter().filter(|u| u.stage == "frag" && !u.sampler).map(|u| u.slot).collect();
        let base = slots.iter().min().copied().unwrap_or(0) * 4;
        let mut out = Vec::new();
        for &(contents, data) in &self.fs.uniforms {
            out.push(match contents {
                0 => data,
                1 => {
                    let k = data.checked_sub(base).map(|k| k as usize);
                    let v = k.and_then(|k| shape.uniforms.get(k)).ok_or_else(|| {
                        format!("{}: QUNIFORM_UNIFORM dword {data} is not one of the shape's uniforms", shape.name)
                    })?;
                    v.to_bits()
                }
                40 | 41 if data >> 24 == 0 && shape.texture.is_some() => data,
                c => return Err(format!("{}: FS uniform contents {c} (data {data:#x}) is not modelled", shape.name)),
            });
        }
        Ok(out)
    }
}

// ---------------------------------------------------------------------------
// v3d_shc's JSON: a minimal reader (the crate has no JSON dependency).
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
enum Json {
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    Arr(Vec<Json>),
    Obj(Vec<(String, Json)>),
}

impl Json {
    fn get(&self, k: &str) -> Option<&Json> {
        match self {
            Json::Obj(m) => m.iter().find(|(n, _)| n == k).map(|(_, v)| v),
            _ => None,
        }
    }
    fn arr(&self) -> Option<&[Json]> {
        match self {
            Json::Arr(a) => Some(a),
            _ => None,
        }
    }
    fn str(&self) -> Option<&str> {
        match self {
            Json::Str(s) => Some(s),
            _ => None,
        }
    }
    fn num(&self) -> Option<f64> {
        match self {
            Json::Num(n) => Some(*n),
            _ => None,
        }
    }
}

struct Reader<'a> {
    s: &'a [u8],
    i: usize,
}

impl Reader<'_> {
    fn ws(&mut self) {
        while self.i < self.s.len() && self.s[self.i].is_ascii_whitespace() {
            self.i += 1;
        }
    }
    fn eat(&mut self, c: u8) -> Result<(), String> {
        self.ws();
        if self.s.get(self.i) == Some(&c) {
            self.i += 1;
            Ok(())
        } else {
            Err(format!("expected '{}' at byte {}", c as char, self.i))
        }
    }
    fn value(&mut self) -> Result<Json, String> {
        self.ws();
        match self.s.get(self.i).copied() {
            Some(b'{') => {
                self.i += 1;
                let mut m = Vec::new();
                self.ws();
                if self.s.get(self.i) == Some(&b'}') {
                    self.i += 1;
                    return Ok(Json::Obj(m));
                }
                loop {
                    self.ws();
                    let Json::Str(k) = self.value()? else { return Err(format!("key at byte {}", self.i)) };
                    self.eat(b':')?;
                    m.push((k, self.value()?));
                    self.ws();
                    match self.s.get(self.i) {
                        Some(b',') => self.i += 1,
                        Some(b'}') => {
                            self.i += 1;
                            return Ok(Json::Obj(m));
                        }
                        _ => return Err(format!("object at byte {}", self.i)),
                    }
                }
            }
            Some(b'[') => {
                self.i += 1;
                let mut a = Vec::new();
                self.ws();
                if self.s.get(self.i) == Some(&b']') {
                    self.i += 1;
                    return Ok(Json::Arr(a));
                }
                loop {
                    a.push(self.value()?);
                    self.ws();
                    match self.s.get(self.i) {
                        Some(b',') => self.i += 1,
                        Some(b']') => {
                            self.i += 1;
                            return Ok(Json::Arr(a));
                        }
                        _ => return Err(format!("array at byte {}", self.i)),
                    }
                }
            }
            Some(b'"') => {
                self.i += 1;
                let start = self.i;
                while self.i < self.s.len() && self.s[self.i] != b'"' {
                    if self.s[self.i] == b'\\' {
                        return Err(format!("escape at byte {} (v3d_shc writes none)", self.i));
                    }
                    self.i += 1;
                }
                let t = std::str::from_utf8(&self.s[start..self.i]).map_err(|e| e.to_string())?;
                self.eat(b'"')?;
                Ok(Json::Str(t.to_string()))
            }
            Some(b't') if self.s[self.i..].starts_with(b"true") => {
                self.i += 4;
                Ok(Json::Bool(true))
            }
            Some(b'f') if self.s[self.i..].starts_with(b"false") => {
                self.i += 5;
                Ok(Json::Bool(false))
            }
            Some(b'n') if self.s[self.i..].starts_with(b"null") => {
                self.i += 4;
                Ok(Json::Null)
            }
            Some(_) => {
                let start = self.i;
                while self.i < self.s.len() && matches!(self.s[self.i], b'-' | b'+' | b'.' | b'e' | b'E' | b'0'..=b'9')
                {
                    self.i += 1;
                }
                let t = std::str::from_utf8(&self.s[start..self.i]).map_err(|e| e.to_string())?;
                t.parse().map(Json::Num).map_err(|_| format!("value at byte {start}"))
            }
            None => Err("unexpected end".to_string()),
        }
    }
}

fn hex(s: &str) -> Result<u64, String> {
    u64::from_str_radix(s.trim_start_matches("0x"), 16).map_err(|e| format!("{s}: {e}"))
}

fn stage(j: &Json, name: &str) -> Result<Stage, String> {
    let st = j.get(name).ok_or_else(|| format!("no \"{name}\""))?;
    let code = st
        .get("code")
        .and_then(Json::arr)
        .ok_or_else(|| format!("{name}: no code"))?
        .iter()
        .map(|w| w.str().ok_or_else(|| format!("{name}: a code word is not a string")).and_then(hex))
        .collect::<Result<Vec<u64>, String>>()?;
    let uniforms = st
        .get("uniforms")
        .and_then(Json::arr)
        .ok_or_else(|| format!("{name}: no uniforms"))?
        .iter()
        .map(|u| {
            let c = u.get("contents").and_then(Json::num).ok_or_else(|| format!("{name}: uniform contents"))?;
            let d = u.get("data").and_then(Json::str).ok_or_else(|| format!("{name}: uniform data"))?;
            Ok((c as u32, hex(d)? as u32))
        })
        .collect::<Result<Vec<(u32, u32)>, String>>()?;
    let threads = st.get("threads").and_then(Json::num).ok_or_else(|| format!("{name}: no threads"))? as u32;
    Ok(Stage { code, uniforms, threads })
}

/// v3d_shc's JSON (tools/v3d/v3d_shc.c) to [`Mesa71`].
pub fn parse(text: &str) -> Result<Mesa71, String> {
    let mut r = Reader { s: text.as_bytes(), i: 0 };
    let j = r.value()?;
    let ver = j.get("gpu").and_then(|g| g.get("ver")).and_then(Json::num).ok_or("no gpu.ver")? as u32;
    let uniform_names = j
        .get("uniform_names")
        .and_then(Json::arr)
        .ok_or("no uniform_names")?
        .iter()
        .map(|u| {
            let st = u.get("stage").and_then(Json::str).ok_or("uniform_names: stage")?;
            let n = u.get("name").and_then(Json::str).ok_or("uniform_names: name")?;
            let sampler = u.get("binding").is_some();
            let s = u.get("slot").or_else(|| u.get("binding")).and_then(Json::num).ok_or("uniform_names: slot")?;
            Ok(UniformName { stage: st.to_string(), name: n.to_string(), slot: s as u32, sampler })
        })
        .collect::<Result<Vec<_>, &str>>()?;
    Ok(Mesa71 { fs: stage(&j, "fs")?, vs: stage(&j, "vs")?, cs: stage(&j, "cs")?, ver, uniform_names })
}
