// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The GLSL ES front end: what an OpenGL ES 2.0 implementation owes an
//! application's shader before any of this project's own translation runs.
//!
//! `glsl2usc` used to hand the application's text straight to
//! [`crate::glsl::translate`], which rewrites GLSL ES into the Vulkan GLSL 450
//! the PCO pipeline compiles, and only then asked glslang whether it was legal.
//! Three things are wrong with that, and dEQP-GLES2 measures all three:
//!
//! 1. **The rules applied were the wrong language's.** `glslangValidator -V`
//!    compiles for SPIR-V, which refuses `#version 100` outright, so the ES
//!    text was judged as GLSL 450 after rewriting. GLSL 450 has implicit
//!    int -> float conversion and GLSL ES 1.00 does not, so `int i = 1.5;`
//!    compiled. Plain `glslangValidator -S vert|frag <file>` — no `-V`, no
//!    `-G` — is the ES front end (a file with no `#version` defaults to ES
//!    100), and that is what [`validate`] runs on the application's own bytes.
//! 2. **The preprocessor ran after the translation, not before it.** The
//!    rewriting in `glsl.rs` is regular expressions over the text, so a
//!    declaration inside `#if 0` was rewritten as if it were live, and a
//!    macro-built declaration was not seen at all. [`preprocess`] runs
//!    `glslangValidator -E` first and the translation works on *that* text.
//!    glslang replaces a directive with a blank line, so line numbers still
//!    count the application's own source. The table key stays the original
//!    bytes: nothing the application can observe changes.
//! 3. **Nothing linked the two stages.** A GL ES 2.0 link error (a varying the
//!    fragment shader uses and the vertex shader never declares, a varying or
//!    uniform whose type or precision differs between the stages) is
//!    cross-stage, and neither stage's compile can see it. [`link`] runs
//!    `glslangValidator -l` over both texts — the same front end again, rather
//!    than a second opinion about the same language — and then applies the two
//!    ES rules glslang does not: [`unmatched_varying`] and
//!    [`uniform_precision_conflict`]; and the varying budget of GLSL ES 1.00
//!    Appendix A 7 ([`varying_budget`]). The uniform budget is the GL link's
//!    (`src/gl/program.rs` `check_uniform_packing`), which has every back
//!    end's uniform reflection.
//!
//! The diagnostics are glslang's, unedited apart from the file-name echo, so
//! an info log reads `0:5: '=' : cannot convert ...`: string 0, line 5 of what
//! the application passed to `glShaderSource`.

use crate::py;
use crate::{Error, R};
use std::collections::HashMap;
use std::io::Read;
use std::process::{Command, Stdio};

/// Which step of the front end rejected a shader (the `phase` a compile
/// response reports; see `shadercheck.rs`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Phase {
    /// `glslangValidator -S <stage>`: the GLSL ES rules on the original text.
    Validate,
    /// `glslangValidator -E -S <stage>`: the ES preprocessor.
    Preprocess,
    /// The rewrite into GLSL 450 ([`crate::glsl::translate`]) or
    /// `glslangValidator -V` on its result: a shader this project cannot
    /// express, rather than one GLSL ES rejects.
    Translate,
    /// `pco_shc`: the machine-code back end.
    Codegen,
    /// `glslangValidator -l`: the cross-stage link.
    Link,
}

impl Phase {
    pub fn as_str(self) -> &'static str {
        match self {
            Phase::Validate => "validate",
            Phase::Preprocess => "preprocess",
            Phase::Translate => "translate",
            Phase::Codegen => "codegen",
            Phase::Link => "link",
        }
    }
}

/// A shader (or program) the front end refused: the phase and the info log,
/// ready to hand to `glGetShaderInfoLog` / `glGetProgramInfoLog`.
#[derive(Clone, Debug)]
pub struct Diag {
    pub phase: Phase,
    pub log: String,
}

/// `vs`/`fs` (this project's names) as the stage names glslang's `-S` takes.
pub fn glslang_stage(stage: &str) -> R<&'static str> {
    match stage {
        "vs" | "vert" | "vertex" => Ok("vert"),
        "fs" | "frag" | "fragment" => Ok("frag"),
        other => Err(Error::Fatal(format!("es100: unknown stage '{other}'"))),
    }
}

fn glslang_path() -> String {
    std::env::var("GLSLANG").unwrap_or_else(|_| "glslangValidator".into())
}

/// The limits this implementation reports, as glslang names them: the values
/// `src/gl/state.rs` answers `glGetIntegerv(GL_MAX_*)` with, so that the front
/// end counts varyings, attributes and uniforms against what the application
/// is told it has rather than against glslang's desktop defaults. GL ES 2.0
/// requires an implementation to reject a program that needs more than it has,
/// and dEQP asks.
fn limits_overrides() -> Vec<(&'static str, i64)> {
    use yakogl::gl::context::{
        CAP_COLOR_ATTACHMENTS, CAP_COMBINED_TEXTURE_UNITS, CAP_TEXTURE_UNITS, MAX_FRAGMENT_UNIFORM_VECTORS,
        MAX_UNIFORM_DWORDS, MAX_VERTEX_ATTRIBS, MAX_VERTEX_UNIFORM_VECTORS,
    };
    let vs_tex = yakogl::backend::GLES_VS_TEX_UNITS as i64;
    vec![
        ("MaxVertexAttribs", MAX_VERTEX_ATTRIBS as i64),
        ("MaxVaryingVectors", 8),
        ("MaxVaryingFloats", 8 * 4),
        ("MaxVertexUniformVectors", MAX_VERTEX_UNIFORM_VECTORS as i64),
        ("MaxFragmentUniformVectors", MAX_FRAGMENT_UNIFORM_VECTORS as i64),
        ("MaxVertexUniformComponents", MAX_UNIFORM_DWORDS as i64),
        ("MaxFragmentUniformComponents", MAX_UNIFORM_DWORDS as i64),
        ("MaxTextureImageUnits", CAP_TEXTURE_UNITS as i64),
        ("MaxVertexTextureImageUnits", vs_tex),
        ("MaxCombinedTextureImageUnits", CAP_COMBINED_TEXTURE_UNITS as i64),
        ("MaxDrawBuffers", CAP_COLOR_ATTACHMENTS as i64),
    ]
}

/// glslang's own default limits (`glslangValidator -c`), read once per run.
///
/// A configuration file *replaces* glslang's table rather than amending it, so
/// a file holding only the values that differ silently zeroes everything else
/// — and a built-in array declared `[gl_MaxDualSourceDrawBuffersEXT]` then has
/// length zero, which makes glslang fail to parse its own built-ins. The
/// defaults therefore have to be the starting point.
fn glslang_defaults() -> R<&'static str> {
    static DEFAULTS: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    if let Some(d) = DEFAULTS.get() {
        return Ok(d);
    }
    let (ok, text) = run_glslang(&["-c"])?;
    if !ok {
        return Err(Error::Exit("es100: glslangValidator -c (default limits) failed".into()));
    }
    Ok(DEFAULTS.get_or_init(|| text))
}

/// The configuration file the front end hands glslang: its defaults with
/// [`limits_overrides`] applied.
pub fn limits_conf() -> R<String> {
    let overrides = limits_overrides();
    let mut out = String::new();
    let mut seen = std::collections::HashSet::new();
    for line in glslang_defaults()?.lines() {
        let key = line.split_whitespace().next().unwrap_or("");
        match overrides.iter().find(|(k, _)| *k == key) {
            Some((k, v)) => {
                seen.insert(*k);
                out.push_str(&format!("{k} {v}\n"));
            }
            None => {
                out.push_str(line);
                out.push('\n');
            }
        }
    }
    /* A key this glslang does not know is still worth writing: it either
     * understands it or ignores it, and a silent drop would be worse. */
    for (k, v) in overrides.iter().filter(|(k, _)| !seen.contains(k)) {
        out.push_str(&format!("{k} {v}\n"));
    }
    Ok(out)
}

/// A scratch directory holding the texts glslang is given: the front end has
/// to hand the compiler files, and the file names end up in its messages.
pub struct Scratch {
    dir: tempfile::TempDir,
}

impl Scratch {
    pub fn new() -> R<Scratch> {
        let dir = tempfile::Builder::new()
            .prefix("es100.")
            .tempdir_in(std::env::temp_dir())
            .map_err(|e| Error::Fatal(format!("mkdtemp: {e}")))?;
        Ok(Scratch { dir })
    }

    fn path(&self, name: &str) -> String {
        py::join(&self.dir.path().to_string_lossy(), name)
    }

    /// Write `text` as `name` and return its path.
    fn put(&self, name: &str, text: &str) -> R<String> {
        let p = self.path(name);
        py::write_text(&p, text)?;
        Ok(p)
    }

    /// The limits file, written once per scratch directory.
    fn conf(&self) -> R<String> {
        let p = self.path("yakogl.conf");
        if !py::exists(&p) {
            py::write_text(&p, &limits_conf()?)?;
        }
        Ok(p)
    }
}

/// Run glslang and return (success, the merged output).
///
/// glslang writes its diagnostics to stdout, not stderr, and its exit status
/// distinguishes a compile failure (1/2) from a link failure (3); neither is
/// this tool's error, so both come back as `false` with the text.
fn run_glslang(args: &[&str]) -> R<(bool, String)> {
    let prog = glslang_path();
    let (mut rd, wr) = std::io::pipe().map_err(|e| Error::Fatal(format!("pipe: {e}")))?;
    let wr2 = wr.try_clone().map_err(|e| Error::Fatal(format!("pipe: {e}")))?;
    let mut cmd = Command::new(&prog);
    cmd.args(args).stdin(Stdio::null()).stdout(wr).stderr(wr2);
    let mut child = cmd.spawn().map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            /* Fatal, not Exit: a missing compiler is the service being
             * broken, not a verdict on the shader (see `not_found` in
             * glsl2usc.rs). */
            Error::Fatal(format!("es100: {prog} not found (GLSLANG=<path> or put it on PATH)"))
        } else {
            Error::Fatal(format!("es100: {prog}: {e}"))
        }
    })?;
    drop(cmd); // the write ends, so the read below sees EOF
    let mut out = Vec::new();
    rd.read_to_end(&mut out).map_err(|e| Error::Fatal(format!("read: {e}")))?;
    let st = child.wait().map_err(|e| Error::Fatal(format!("wait: {e}")))?;
    Ok((st.success(), py::lossy_text(&out)))
}

/// glslang's output as an info log: it echoes each input file's name on a line
/// of its own, and a scratch path means nothing to the application. Everything
/// else — including `0:<line>:`, which already counts lines of the text the
/// application passed — is left exactly as the compiler wrote it.
fn info_log(raw: &str, paths: &[&str]) -> String {
    let mut out = String::new();
    for line in raw.lines() {
        let t = line.trim_end();
        if t.is_empty() || paths.contains(&t) {
            continue;
        }
        out.push_str(t);
        out.push('\n');
    }
    out
}

/// GLSL ES rules on the text the application passed, before anything rewrites
/// it: `glslangValidator -S vert|frag <file>` with this implementation's
/// limits. No `-V`/`-G`, which would compile it as Vulkan/OpenGL SPIR-V and so
/// apply GLSL 450's rules (and reject `#version 100` outright).
pub fn validate(scratch: &Scratch, stage: &str, text: &str) -> R<Result<(), Diag>> {
    let gstage = glslang_stage(stage)?;
    let src = scratch.put(&format!("shader.{gstage}"), text)?;
    let conf = scratch.conf()?;
    let (ok, raw) = run_glslang(&["-S", gstage, &conf, &src])?;
    if !ok {
        return Ok(Err(Diag { phase: Phase::Validate, log: info_log(&raw, &[&src, &conf]) }));
    }
    if let Some(log) = extension_after_code(text) {
        return Ok(Err(Diag { phase: Phase::Validate, log }));
    }
    Ok(Ok(()))
}

/// GLSL ES 1.00 section 3.4: `#extension` must come before any
/// non-preprocessor token. glslang accepts one that does not, so the rule is
/// applied here (dEQP's
/// `shaders.preprocessor.extensions.after_non_preprocessing_tokens`).
fn extension_after_code(text: &str) -> Option<String> {
    let mut seen_code = false;
    let mut in_block_comment = false;
    for (n, raw_line) in text.lines().enumerate() {
        let mut line = String::new();
        let mut rest = raw_line;
        /* Comments, kept line by line so the number below is the
         * application's own. */
        loop {
            if in_block_comment {
                match rest.find("*/") {
                    Some(i) => {
                        rest = &rest[i + 2..];
                        in_block_comment = false;
                    }
                    None => break,
                }
            }
            match (rest.find("//"), rest.find("/*")) {
                (Some(l), b) if b.is_none_or(|b| l < b) => {
                    line.push_str(&rest[..l]);
                    break;
                }
                (_, Some(b)) => {
                    line.push_str(&rest[..b]);
                    rest = &rest[b + 2..];
                    in_block_comment = true;
                }
                _ => {
                    line.push_str(rest);
                    break;
                }
            }
        }
        let t = line.trim();
        if t.is_empty() {
            continue;
        }
        if let Some(d) = t.strip_prefix('#') {
            if d.trim_start().starts_with("extension") && seen_code {
                return Some(format!(
                    "ERROR: 0:{}: '#extension' : directive must occur before any non-preprocessor tokens\n",
                    n + 1
                ));
            }
            continue;
        }
        seen_code = true;
    }
    None
}

/// The ES preprocessor, run before the translation so that the rewriting in
/// `glsl.rs` only ever sees live code: `glslangValidator -E -S vert|frag`.
///
/// glslang leaves a blank line where a directive was, so the result has the
/// same line numbering as the input and a later diagnostic still points at the
/// application's own source.
pub fn preprocess(scratch: &Scratch, stage: &str, text: &str) -> R<Result<String, Diag>> {
    let gstage = glslang_stage(stage)?;
    let src = scratch.put(&format!("shader.{gstage}"), text)?;
    let conf = scratch.conf()?;
    let (ok, raw) = run_glslang(&["-E", "-S", gstage, &conf, &src])?;
    if !ok {
        return Ok(Err(Diag { phase: Phase::Preprocess, log: info_log(&raw, &[&src, &conf]) }));
    }
    /* -E prints the file name first, like every other mode. */
    Ok(Ok(drop_pragmas(&strip_name_echo(&raw, &src))))
}

/// Blank every `#pragma` line the preprocessor left behind, keeping the line
/// so that numbering still counts the application's source.
///
/// GLSL ES 1.00 section 3.4 says a `#pragma` the implementation does not
/// recognise is ignored, and its argument need not even be a GLSL token — it
/// may hold bytes outside GLSL's character set. glslang has already applied
/// the pragmas it knows; what is left is inert, and passing those bytes on to
/// the text rewriting in `glsl.rs` only makes it refuse a shader that the
/// language says must build (dEQP's
/// `shaders.preprocessor.pragmas.pragma_unrecognized_token`).
fn drop_pragmas(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for line in text.split_inclusive('\n') {
        if line.trim_start().starts_with("#pragma") {
            out.push('\n');
        } else {
            out.push_str(line);
        }
    }
    out
}

/// Drop the leading file-name echo from `-E` output (the text after it is the
/// preprocessed shader and must be kept byte for byte).
fn strip_name_echo(raw: &str, path: &str) -> String {
    match raw.strip_prefix(path) {
        Some(rest) => rest.strip_prefix('\n').unwrap_or(rest).to_string(),
        None => raw.to_string(),
    }
}

/// The cross-stage link GL ES 2.0 requires of `glLinkProgram`: the same front
/// end over both texts at once (`glslangValidator -l`), with this
/// implementation's limits. It is what catches a varying the vertex shader
/// never writes, a varying or uniform whose type or precision differs between
/// the stages, and a program that wants more varyings, attributes or uniform
/// vectors than there are.
pub fn link(scratch: &Scratch, vs_text: &str, fs_text: &str) -> R<Result<(), Diag>> {
    link_for(scratch, vs_text, fs_text, yakogl::gl::context::MAX_VARYING_VECTORS)
}

/// [`link`] for a GPU that reports `max_varying_vectors` as
/// GL_MAX_VARYING_VECTORS: this binary is built for no GPU (its constant is
/// the PowerVR's), so a caller checking for another GPU passes its value.
pub fn link_for(scratch: &Scratch, vs_text: &str, fs_text: &str, max_varying_vectors: usize) -> R<Result<(), Diag>> {
    let vs = scratch.put("link.vert", vs_text)?;
    let fs = scratch.put("link.frag", fs_text)?;
    let conf = scratch.conf()?;
    let (ok, raw) = run_glslang(&["-l", &conf, &vs, &fs])?;
    if !ok {
        return Ok(Err(Diag { phase: Phase::Link, log: info_log(&raw, &[&vs, &fs, &conf]) }));
    }
    /* Two GLSL ES 1.00 link rules glslang's own linker does not apply: it
     * models a fragment input with no vertex output as legal (GLSL 4.50 does),
     * and it compares the types of a uniform declared in both stages but not
     * its precision. GL ES 2.0 section 2.10.4 makes both link errors, and
     * dEQP's shaders.linkage asks. */
    if let Some(msg) = unmatched_varying(vs_text, fs_text) {
        return Ok(Err(Diag { phase: Phase::Link, log: msg }));
    }
    if let Some(msg) = uniform_precision_conflict(vs_text, fs_text) {
        return Ok(Err(Diag { phase: Phase::Link, log: msg }));
    }
    if let Some(msg) = varying_budget(vs_text, fs_text, max_varying_vectors) {
        return Ok(Err(Diag { phase: Phase::Link, log: msg }));
    }
    Ok(Ok(()))
}

/// Scratch for the in-process front end's reflection of one program: what
/// `online-check` gives one shader by default, ample for two.
const REFLECT_SCRATCH: usize = 4096 * 1024;

/// GLSL ES 1.00 Appendix A 7: the varyings both stages statically use (and
/// the fragment built-ins it names) must pack on `rows` rows. glslang applies
/// MaxVaryingVectors to GLSL ES 3.00's location-based varyings but not to ES
/// 1.00's named ones, so this asks the in-process compiler's front end for
/// the declarations and runs the rule the in-process link runs
/// (`yakogl::shaderc::online::reflect::varying_budget`): one rule, one
/// implementation, whichever back end the program is for.
///
/// A pair that front end cannot read (glslang has accepted it by now) is
/// not checked, and says so on stderr, which the oracle and the server log.
fn varying_budget(vs_text: &str, fs_text: &str, rows: usize) -> Option<String> {
    use yakogl::shaderc::online::{arena::Arena, reflect, sema::Stage};
    let mut buf = vec![0u8; REFLECT_SCRATCH];
    let a = Arena::new(&mut buf);
    let (vi, fi) =
        match (reflect::interface(&a, Stage::Vertex, vs_text), reflect::interface(&a, Stage::Fragment, fs_text)) {
            (Ok(v), Ok(f)) => (v, f),
            (Err(e), _) | (_, Err(e)) => {
                eprintln!(
                    "es100 link: the varying budget is not checked: the in-process front end refused the pair: {e:?}"
                );
                return None;
            }
        };
    reflect::varying_budget(&vi, &fi, rows).err().map(|u| format!("ERROR: Linking vertex and fragment stages: {u}\n"))
}

/* ---- the ES link rules glslang does not apply ---------------------------- */

/// A GLSL ES declaration this file has to look at: its name and, for the
/// cross-stage rules, the type and precision it was declared with.
#[derive(Clone, PartialEq, Eq, Debug)]
struct Decl {
    name: String,
    ty: String,
    precision: String,
}

/// GLSL ES 1.00 section 4.5.2: only `float`, `int` and the sampler types
/// carry a precision. `bool`, `bvec*` and a struct name do not, so comparing
/// one across the stages would invent a conflict that is not there.
fn has_precision(ty: &str) -> bool {
    matches!(ty, "float" | "vec2" | "vec3" | "vec4" | "mat2" | "mat3" | "mat4" | "int" | "ivec2" | "ivec3" | "ivec4")
        || ty.starts_with("sampler")
}

/// The type whose `precision` statement governs `ty` (`vec3` follows
/// `precision ... float;`, `ivec2` follows `precision ... int;`).
fn precision_base(ty: &str) -> &str {
    match ty {
        "int" | "ivec2" | "ivec3" | "ivec4" => "int",
        t if t.starts_with("sampler") => t,
        _ => "float",
    }
}

/// The default precision of `ty` in a stage that declared none, GLSL ES 1.00
/// section 4.5.3: a vertex shader has one for every type; a fragment shader
/// has `mediump int` and `lowp sampler2D` but no default for `float`, so a
/// fragment shader that uses one has said `precision ... float;` and
/// [`precision_at`] finds it before falling back here.
fn default_precision(stage_is_vertex: bool, ty: &str) -> &'static str {
    /* A sampler defaults to lowp in BOTH stages -- the vertex language has
     * `precision lowp sampler2D;` as well. Saying highp here because the
     * stage is the vertex one invents a conflict for every program that
     * samples in both stages, which is most of them. */
    if ty.starts_with("sampler") {
        return "lowp";
    }
    if stage_is_vertex {
        return "highp";
    }
    match precision_base(ty) {
        "int" => "mediump",
        _ => "mediump",
    }
}

/// Every `precision <p> <type>;` statement, in order of appearance.
fn precision_statements(code: &str) -> Vec<(usize, String, String)> {
    re(r"\bprecision\s+(lowp|mediump|highp)\s+([A-Za-z_][A-Za-z0-9_]*)\s*;")
        .captures_iter(code)
        .map(|m| (m.get(0).unwrap().start(), m[1].to_string(), m[2].to_string()))
        .collect()
}

/// The precision in force for `ty` at byte offset `at`: the last
/// `precision <p> <base>;` before it, or the stage's default.
fn precision_at(stmts: &[(usize, String, String)], at: usize, stage_is_vertex: bool, ty: &str) -> String {
    let base = precision_base(ty);
    let mut p = None;
    for (off, prec, pty) in stmts {
        if *off < at && pty == base {
            p = Some(prec.clone());
        }
    }
    p.unwrap_or_else(|| default_precision(stage_is_vertex, ty).to_string())
}

fn re(pat: &str) -> regex::Regex {
    crate::py::re(pat)
}

/// `struct <Name> { <members> };` -> the members, each with its precision
/// resolved at the point of declaration.
fn struct_members(code: &str, stage_is_vertex: bool) -> HashMap<String, Vec<Decl>> {
    let stmts = precision_statements(code);
    let mut out = HashMap::new();
    for m in re(r"(?s)\bstruct\s+([A-Za-z_][A-Za-z0-9_]*)\s*\{(.*?)\}").captures_iter(code) {
        let body = m.get(2).unwrap();
        let mut members = Vec::new();
        for f in re(r"(?:(lowp|mediump|highp)\s+)?([A-Za-z_][A-Za-z0-9_]*)\s+([A-Za-z_][A-Za-z0-9_]*)\s*;")
            .captures_iter(body.as_str())
        {
            let ty = f[2].to_string();
            let precision = match f.get(1) {
                Some(p) => p.as_str().to_string(),
                None => precision_at(&stmts, body.start(), stage_is_vertex, &ty),
            };
            members.push(Decl { name: f[3].to_string(), ty, precision });
        }
        out.insert(m[1].to_string(), members);
    }
    out
}

/// Every `uniform` a stage declares, flattened: a struct-typed uniform becomes
/// one entry per member, named as GL names it (`val.a`).
fn uniform_decls(text: &str, stage_is_vertex: bool) -> Vec<Decl> {
    let code = crate::glsl::strip_comments(text);
    let stmts = precision_statements(&code);
    let structs = struct_members(&code, stage_is_vertex);
    let mut out = Vec::new();
    for m in re(r"\buniform\s+(?:(lowp|mediump|highp)\s+)?([A-Za-z_][A-Za-z0-9_]*)\s+([A-Za-z_][A-Za-z0-9_]*)")
        .captures_iter(&code)
    {
        let ty = m[2].to_string();
        let name = m[3].to_string();
        match structs.get(&ty) {
            Some(members) => flatten(&structs, members, &name, &mut out),
            None => {
                let precision = match m.get(1) {
                    Some(p) => p.as_str().to_string(),
                    None => precision_at(&stmts, m.get(0).unwrap().start(), stage_is_vertex, &ty),
                };
                out.push(Decl { name, ty, precision });
            }
        }
    }
    out
}

/// A struct-typed uniform contributes one entry per leaf member, named as GL
/// names it (`val.inner.a`): a struct has no precision of its own.
fn flatten(structs: &HashMap<String, Vec<Decl>>, members: &[Decl], prefix: &str, out: &mut Vec<Decl>) {
    for d in members {
        let name = format!("{prefix}.{}", d.name);
        match structs.get(&d.ty) {
            Some(inner) => flatten(structs, inner, &name, out),
            None => out.push(Decl { name, ty: d.ty.clone(), precision: d.precision.clone() }),
        }
    }
}

/// Every `varying` a stage declares.
fn varying_names(text: &str) -> Vec<String> {
    let code = crate::glsl::strip_comments(text);
    re(r"\bvarying\s+(?:invariant\s+)?(?:(?:lowp|mediump|highp)\s+)?[A-Za-z_][A-Za-z0-9_]*\s+([A-Za-z_][A-Za-z0-9_]*)")
        .captures_iter(&code)
        .map(|m| m[1].to_string())
        .collect()
}

/// GL ES 2.0 section 2.10.4: a varying the fragment shader statically uses but
/// the vertex shader does not declare is a link error. A varying merely
/// *declared* in the fragment shader is not (`shaders.linkage.varying_1`
/// expects that to link), so the use has to be counted, not the declaration.
fn unmatched_varying(vs_text: &str, fs_text: &str) -> Option<String> {
    let declared = varying_names(vs_text);
    let fs_code = crate::glsl::strip_comments(fs_text);
    for name in varying_names(fs_text) {
        if declared.contains(&name) {
            continue;
        }
        /* Uses, not the declaration itself: the declaration is the one
         * occurrence preceded by the type in a `varying` statement. */
        let decl = re(&format!(
            r"\bvarying\s+(?:invariant\s+)?(?:(?:lowp|mediump|highp)\s+)?[A-Za-z_][A-Za-z0-9_]*\s+{}\b",
            crate::py::esc(&name)
        ));
        let uses = re(&format!(r"\b{}\b", crate::py::esc(&name))).find_iter(&fs_code).count();
        let decls = decl.find_iter(&fs_code).count();
        if uses > decls {
            return Some(format!(
                "ERROR: Linking vertex and fragment stages: varying '{name}' is used by the fragment shader \
                 but not declared by the vertex shader\n"
            ));
        }
    }
    None
}

/// GL ES 2.0 section 2.10.4: a uniform of the same name in both stages must
/// have the same type *and* the same precision.
fn uniform_precision_conflict(vs_text: &str, fs_text: &str) -> Option<String> {
    let vs = uniform_decls(vs_text, true);
    let fs = uniform_decls(fs_text, false);
    for a in &vs {
        let Some(b) = fs.iter().find(|b| b.name == a.name) else { continue };
        if a.ty != b.ty || (has_precision(&a.ty) && a.precision != b.precision) {
            return Some(format!(
                "ERROR: Linking vertex and fragment stages: uniform '{}' is {} {} in the vertex shader \
                 and {} {} in the fragment shader\n",
                a.name, a.precision, a.ty, b.precision, b.ty
            ));
        }
    }
    None
}

/// `glslangValidator -V` on the **rewritten** GLSL 450: whether this project's
/// translation of a shader it already accepted is itself a legal shader. A
/// failure here is a limitation of the rewriting, not of the application's
/// source, and the phase says so.
pub fn spirv_check(scratch: &Scratch, stage: &str, text450: &str) -> R<Result<(), Diag>> {
    let gstage = glslang_stage(stage)?;
    let src = scratch.put(&format!("rewritten.{gstage}"), text450)?;
    let spv = scratch.path("rewritten.spv");
    let (ok, raw) = run_glslang(&["-V", "-S", gstage, &src, "-o", &spv])?;
    if ok {
        return Ok(Ok(()));
    }
    Ok(Err(Diag { phase: Phase::Translate, log: info_log(&raw, &[&src]) }))
}

/// `pco_shc` on the SPIR-V of the rewritten text: the machine-code back end,
/// the last thing that can refuse a shader. Only the whole offline pipeline
/// runs this; the front end alone does not need a GPU compiler.
pub fn codegen_check(scratch: &Scratch, stage: &str, text450: &str) -> R<Result<(), Diag>> {
    let gstage = glslang_stage(stage)?;
    let short = if gstage == "vert" { "vs" } else { "fs" };
    let src = scratch.put(&format!("codegen.{gstage}"), text450)?;
    let spv = scratch.path("codegen.spv");
    let (ok, raw) = run_glslang(&["-V", "-S", gstage, &src, "-o", &spv])?;
    if !ok {
        return Ok(Err(Diag { phase: Phase::Translate, log: info_log(&raw, &[&src]) }));
    }
    let pco = crate::glsl2usc::pco_shc_path();
    let out = Command::new(&pco)
        .args([short, &spv])
        .stdin(Stdio::null())
        .output()
        .map_err(|e| Error::Exit(format!("es100: {pco}: {e} (PCO_SHC=<path>)")))?;
    if out.status.success() {
        return Ok(Ok(()));
    }
    let mut err = out.stdout;
    err.extend_from_slice(&out.stderr);
    Ok(Err(Diag { phase: Phase::Codegen, log: py::lossy_text(&err) }))
}

#[cfg(test)]
mod tests {
    //! These exercise the front end against the real glslang, which the whole
    //! tool needs anyway (`tools/README.md`, Requirements). A run without it
    //! on PATH says so rather than failing for the wrong reason.
    use super::*;

    /// Run a closure with a scratch directory, skipping when glslang is absent.
    fn with_scratch(f: impl FnOnce(&Scratch)) {
        if run_glslang(&["--version"]).is_err() {
            eprintln!("es100 tests skipped: no glslangValidator on PATH");
            return;
        }
        let s = Scratch::new().expect("scratch");
        f(&s);
    }

    const VS_OK: &str = "attribute highp vec4 a_pos;\nvoid main() { gl_Position = a_pos; }\n";
    const FS_OK: &str = "precision mediump float;\nvoid main() { gl_FragColor = vec4(1.0); }\n";

    #[test]
    fn es_rules_reject_an_implicit_conversion_glsl_450_would_allow() {
        with_scratch(|s| {
            let src =
                "precision mediump float;\nvoid main()\n{\n    int i = 1.5;\n    gl_FragColor = vec4(float(i));\n}\n";
            let d = validate(s, "fs", src).unwrap().expect_err("GLSL ES 1.00 has no implicit int <- float");
            assert_eq!(d.phase, Phase::Validate);
            assert!(d.log.contains("0:4:"), "the log points at the application's line 4: {}", d.log);
            assert!(d.log.contains("cannot convert"), "{}", d.log);
        });
    }

    #[test]
    fn a_source_without_a_version_directive_is_es_100_not_desktop_glsl() {
        with_scratch(|s| {
            /* 'attribute' is a keyword in GLSL ES 1.00; a desktop default
             * would take it as an ordinary identifier. */
            let src = "precision mediump float;\nvoid main()\n{\n    float attribute = 1.0;\n    gl_FragColor = vec4(attribute);\n}\n";
            assert!(validate(s, "fs", src).unwrap().is_err(), "'attribute' is a keyword in GLSL ES 1.00");
        });
    }

    #[test]
    fn a_legal_shader_validates_in_both_stages() {
        with_scratch(|s| {
            assert!(validate(s, "vs", VS_OK).unwrap().is_ok());
            assert!(validate(s, "fs", FS_OK).unwrap().is_ok());
        });
    }

    #[test]
    fn the_preprocessor_runs_before_the_translation_and_keeps_the_line_count() {
        with_scratch(|s| {
            let src = "#define C vec4(1.0, 0.0, 0.0, 1.0)\n#if 1\nprecision mediump float;\nvoid main() { gl_FragColor = C; }\n#endif\n";
            let out = preprocess(s, "fs", src).unwrap().expect("legal preprocessor input");
            assert!(out.contains("gl_FragColor = vec4(1.0, 0.0, 0.0, 1.0)"), "the macro is expanded: {out}");
            assert!(!out.contains("#define"), "and the directive is gone: {out}");
            assert_eq!(
                out.lines().count(),
                src.lines().count(),
                "a directive becomes a blank line, so line numbers still count the application's source:\n{out}"
            );
        });
    }

    /// GLSL ES 1.00 section 4.5.4: GL_FRAGMENT_PRECISION_HIGH is defined to 1,
    /// in both stages, on a system with fragment highp. This pipeline does
    /// not define it itself: glslang's ES preamble does (glslang 16.4.0
    /// defines it in both stages). This pins that glslang's answer is the
    /// library's own fact (yakogl::backend::FRAGMENT_HIGHP, which
    /// glGetShaderPrecisionFormat and the online preprocessor read), so the
    /// two shader back ends see the same macro.
    #[test]
    fn fragment_precision_high_is_the_librarys_fact_in_both_stages() {
        with_scratch(|s| {
            let src = "precision mediump float;\n#ifdef GL_FRAGMENT_PRECISION_HIGH\nint high = GL_FRAGMENT_PRECISION_HIGH;\n#else\nint none;\n#endif\nvoid main() {}\n";
            for stage in ["vs", "fs"] {
                let out = preprocess(s, stage, src).unwrap().expect("legal preprocessor input");
                let defined = out.contains("int high = 1;");
                assert!(defined || out.contains("int none;"), "{stage}: neither branch: {out}");
                assert_eq!(defined, yakogl::backend::FRAGMENT_HIGHP, "{stage}: glslang against the library: {out}");
            }
        });
    }

    #[test]
    fn a_preprocessor_error_is_reported_as_one() {
        with_scratch(|s| {
            let d = preprocess(s, "fs", "#if 1\nvoid main() {}\n").unwrap().expect_err("unterminated #if");
            assert_eq!(d.phase, Phase::Preprocess);
            assert!(!d.log.is_empty());
        });
    }

    #[test]
    fn dead_code_behind_an_if_zero_is_not_translated() {
        with_scratch(|s| {
            let src = "#if 0\nthis is not GLSL at all\n#endif\nprecision mediump float;\nvoid main() { gl_FragColor = vec4(1.0); }\n";
            assert!(validate(s, "fs", src).unwrap().is_ok(), "the ES preprocessor removes it");
            let out = preprocess(s, "fs", src).unwrap().expect("legal");
            assert!(!out.contains("not GLSL at all"), "{out}");
        });
    }

    #[test]
    fn a_varying_the_two_stages_type_differently_is_a_link_error() {
        with_scratch(|s| {
            let vs = "attribute highp vec4 a_pos;\nvarying mediump float var;\nvoid main() { var = 1.0; gl_Position = a_pos; }\n";
            let fs = "precision mediump float;\nvarying mediump vec2 var;\nvoid main() { gl_FragColor = vec4(var, 0.0, 1.0); }\n";
            assert!(validate(s, "vs", vs).unwrap().is_ok(), "each stage on its own is legal");
            assert!(validate(s, "fs", fs).unwrap().is_ok());
            let d = link(s, vs, fs).unwrap().expect_err("float vs vec2");
            assert_eq!(d.phase, Phase::Link);
            assert!(d.log.contains("Linking"), "{}", d.log);
        });
    }

    #[test]
    fn a_matching_pair_links() {
        with_scratch(|s| {
            assert!(link(s, VS_OK, FS_OK).unwrap().is_ok());
        });
    }

    #[test]
    fn the_limits_are_this_implementations_own() {
        if run_glslang(&["--version"]).is_err() {
            eprintln!("es100 tests skipped: no glslangValidator on PATH");
            return;
        }
        let conf = limits_conf().expect("glslang -c");
        assert!(conf.contains("MaxVertexAttribs 16"), "src/gl/context.rs MAX_VERTEX_ATTRIBS: {conf}");
        assert!(conf.contains("MaxVaryingVectors 8"), "src/gl/state.rs GL_MAX_VARYING_VECTORS: {conf}");
        /* this crate builds yakogl with `gpu-powervr`: the PowerVR's 16 */
        assert!(conf.contains("MaxVertexTextureImageUnits 16"), "src/backend.rs GLES_VS_TEX_UNITS: {conf}");
        assert!(conf.contains("MaxDrawBuffers 3"), "src/gl/state.rs GL_MAX_DRAW_BUFFERS: {conf}");
        assert!(
            conf.contains("MaxDualSourceDrawBuffersEXT"),
            "and glslang's other defaults survive: a config file replaces the whole table"
        );
    }

    /// A pair sharing the varyings `decls` (`"vec4 a"`, `"vec2 c[3]"`): the
    /// vertex shader writes each one (every element), the fragment shader
    /// reads each one.
    fn varying_pair(decls: &[&str]) -> (String, String) {
        let mut vs = String::from("attribute highp vec4 a_pos;\n");
        let mut fs = String::from("precision mediump float;\n");
        let (mut writes, mut reads) = (String::new(), String::new());
        for d in decls {
            vs.push_str(&format!("varying mediump {d};\n"));
            fs.push_str(&format!("varying mediump {d};\n"));
            let (ty, rest) = d.split_once(' ').unwrap();
            let (name, n) = match rest.split_once('[') {
                Some((name, n)) => (name, n.trim_end_matches(']').parse::<usize>().unwrap()),
                None => (rest, 0),
            };
            let value = match ty {
                "float" => "a_pos.x".to_string(),
                t => format!("{t}(a_pos.x)"),
            };
            let first = match ty {
                "float" => String::new(),
                t if t.starts_with("mat") => "[0][0]".into(),
                _ => ".x".into(),
            };
            if n == 0 {
                writes.push_str(&format!("    {name} = {value};\n"));
                reads.push_str(&format!(" + {name}{first}"));
            } else {
                for k in 0..n {
                    writes.push_str(&format!("    {name}[{k}] = {value};\n"));
                }
                reads.push_str(&format!(" + {name}[0]{first}"));
            }
        }
        vs.push_str(&format!("void main() {{\n{writes}    gl_Position = a_pos;\n}}\n"));
        fs.push_str(&format!("void main() {{\n    gl_FragColor = vec4(0.0{reads});\n}}\n"));
        (vs, fs)
    }

    const BUDGET: &str = "ERROR: Linking vertex and fragment stages: the program's varyings do not fit \
                          MAX_VARYING_VECTORS = 8 (Appendix A packing)\n";

    /// GLSL ES 1.00 Appendix A 7's own example, a..h, fills all but one cell
    /// of the eight rows (the table the appendix prints): it links, one float
    /// more still does, and a vec2 more does not.
    #[test]
    fn the_appendix_example_links_and_one_vec2_more_does_not() {
        const EXAMPLE: [&str; 8] =
            ["vec4 a", "mat3 b", "vec2 c[3]", "vec2 d[2]", "vec2 e", "float f[3]", "float g[2]", "float h"];
        with_scratch(|s| {
            let (vs, fs) = varying_pair(&EXAMPLE);
            assert_eq!(link(s, &vs, &fs).unwrap().map_err(|d| d.log), Ok(()));
            let (vs, fs) = varying_pair(&[&EXAMPLE[..], &["float i"]].concat());
            assert_eq!(link(s, &vs, &fs).unwrap().map_err(|d| d.log), Ok(()));
            let (vs, fs) = varying_pair(&[&EXAMPLE[..], &["vec2 i"]].concat());
            let d = link(s, &vs, &fs).unwrap().expect_err("no two cells side by side are left");
            assert_eq!((d.phase, d.log.as_str()), (Phase::Link, BUDGET));
        });
    }

    /// Eight vec4 fill GL_MAX_VARYING_VECTORS; a ninth does not fit under
    /// any packing. glslang applies MaxVaryingVectors to GLSL ES 3.00's
    /// location-based varyings but not to ES 1.00's named ones, so this was
    /// linked until the appendix's rule was applied here.
    #[test]
    fn nine_vec4_varyings_fail_and_eight_link() {
        with_scratch(|s| {
            let decls: Vec<String> = (0..9).map(|i| format!("vec4 v{i}")).collect();
            let decls: Vec<&str> = decls.iter().map(String::as_str).collect();
            let (vs, fs) = varying_pair(&decls[..8]);
            assert_eq!(link(s, &vs, &fs).unwrap().map_err(|d| d.log), Ok(()));
            let (vs, fs) = varying_pair(&decls);
            assert_eq!(link(s, &vs, &fs).unwrap().map_err(|d| d.log), Err(BUDGET.to_string()));
            /* the GC7000 reports 15, and its client says so in the request */
            assert_eq!(link_for(s, &vs, &fs, 15).unwrap().map_err(|d| d.log), Ok(()));
        });
    }

    /// Why a plain row count would be wrong: 32 float varyings pack four to
    /// a row, which real shaders rely on; the 33rd has no cell.
    #[test]
    fn float_varyings_share_rows() {
        with_scratch(|s| {
            let decls: Vec<String> = (0..33).map(|i| format!("float f{i}")).collect();
            let decls: Vec<&str> = decls.iter().map(String::as_str).collect();
            let (vs, fs) = varying_pair(&decls[..32]);
            assert_eq!(link(s, &vs, &fs).unwrap().map_err(|d| d.log), Ok(()));
            let (vs, fs) = varying_pair(&decls);
            assert_eq!(link(s, &vs, &fs).unwrap().map_err(|d| d.log), Err(BUDGET.to_string()));
        });
    }

    #[test]
    fn a_varying_the_fragment_shader_uses_but_the_vertex_shader_never_declares() {
        with_scratch(|s| {
            let vs = "attribute highp vec4 a_pos;\nvoid main() { gl_Position = a_pos; }\n";
            let fs =
                "precision mediump float;\nvarying mediump float var;\nvoid main() { gl_FragColor = vec4(var); }\n";
            let d = link(s, vs, fs).unwrap().expect_err("GL ES 2.0 section 2.10.4");
            assert!(d.log.contains("'var'"), "{}", d.log);
        });
    }

    #[test]
    fn a_varying_the_fragment_shader_only_declares_still_links() {
        with_scratch(|s| {
            /* shaders.linkage.varying_1: declared, never read. */
            let vs = "attribute highp vec4 a_pos;\nvoid main() { gl_Position = a_pos; }\n";
            let fs =
                "precision mediump float;\nvarying mediump float var;\nvoid main() { gl_FragColor = vec4(1.0); }\n";
            assert!(link(s, vs, fs).unwrap().is_ok());
        });
    }

    #[test]
    fn a_uniform_declared_at_two_precisions_is_a_link_error() {
        with_scratch(|s| {
            let vs = "attribute highp vec4 a_pos;\nstruct S { highp float a; };\nuniform S val;\nvoid main() { gl_Position = a_pos * val.a; }\n";
            let fs = "precision mediump float;\nstruct S { mediump float a; };\nuniform S val;\nvoid main() { gl_FragColor = vec4(val.a); }\n";
            let d = link(s, vs, fs).unwrap().expect_err("highp in one stage, mediump in the other");
            assert!(d.log.contains("val.a"), "{}", d.log);
        });
    }

    #[test]
    fn a_type_without_a_precision_never_conflicts() {
        with_scratch(|s| {
            /* bool has no precision in GLSL ES, so the stages' differing
             * defaults must not be read as a conflict. */
            let vs = "attribute highp vec4 a_pos;\nuniform bool u_f;\nvoid main() { gl_Position = u_f ? a_pos : vec4(0.0); }\n";
            let fs =
                "precision mediump float;\nuniform bool u_f;\nvoid main() { gl_FragColor = vec4(u_f ? 1.0 : 0.0); }\n";
            assert!(link(s, vs, fs).unwrap().is_ok());
        });
    }

    #[test]
    fn an_extension_directive_after_code_is_rejected() {
        assert!(extension_after_code("precision mediump float;\n#extension all : warn\nvoid main() {}\n")
            .is_some_and(|m| m.contains("0:2:")));
        assert!(extension_after_code("#extension all : warn\nprecision mediump float;\nvoid main() {}\n").is_none());
        assert!(
            extension_after_code("/* code?\nno */\n#extension all : warn\nvoid main() {}\n").is_none(),
            "a comment is not a non-preprocessor token, and a block comment may span lines"
        );
    }

    #[test]
    fn a_pragma_is_blanked_but_the_line_stays() {
        let out = drop_pragmas("#version 100\n#pragma unknown(\u{a4})\nvoid main() {}\n");
        assert_eq!(out, "#version 100\n\nvoid main() {}\n");
    }

    #[test]
    fn an_info_log_drops_the_file_name_echo_and_nothing_else() {
        let raw =
            "/scratch/es100.x/shader.frag\nERROR: 0:3: 'x' : undeclared identifier\nERROR: 1 compilation errors.\n";
        let log = info_log(raw, &["/scratch/es100.x/shader.frag"]);
        assert_eq!(log, "ERROR: 0:3: 'x' : undeclared identifier\nERROR: 1 compilation errors.\n");
    }

    #[test]
    fn stage_names_from_either_vocabulary() {
        assert_eq!(glslang_stage("vs").unwrap(), "vert");
        assert_eq!(glslang_stage("vertex").unwrap(), "vert");
        assert_eq!(glslang_stage("fs").unwrap(), "frag");
        assert_eq!(glslang_stage("fragment").unwrap(), "frag");
        assert!(glslang_stage("compute").is_err());
    }
}
