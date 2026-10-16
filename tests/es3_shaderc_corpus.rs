// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! How much of GLSL ES 3.00 the online shader compiler takes, measured over
//! the dEQP `data/gles3/shaders/*.test` corpus (docs/es3/shaderc-coverage.md).
//!
//! Two parts:
//!
//! * `inline_cases` always runs. A few short cases in the `.test` format,
//!   copied from VK-GL-CTS (Apache-2.0, Copyright (c) 2014 The Android Open
//!   Source Project and The Khronos Group Inc., `data/gles3/shaders/`), go
//!   through the same parser, template expansion and compile/link path as the
//!   corpus and must come out as the case says.
//! * `corpus` runs only when `DEQP_GLES3_TEST_DIR` names the directory of the
//!   `.test` files (a VK-GL-CTS checkout's `data/gles3/shaders`). Without it
//!   the test prints that it skipped. It measures, per file: expected-pass
//!   compiled, expected-pass refused (the first error), expected-fail
//!   correctly refused, expected-fail wrongly accepted. It asserts nothing
//!   about the numbers (they move as the compiler grows); it prints the table
//!   and, with `DEQP_GLES3_REPORT=<file>`, writes one tab-separated line per
//!   unit that is not an expected-pass success (file, unit, expectation,
//!   verdict, first line of the log).
//!
//! The case format and the `${...}` expansion follow dEQP's
//! `glsShaderLibrary` (framework/opengl/gluShaderLibrary.cpp and
//! modules/glshared/glsShaderLibraryCase.cpp): a `both` case is two programs
//! (`_vertex`, `_fragment`), a `values` block supplies the inputs and outputs
//! the generated stage and the comparison code are written from, and a case
//! with `vertex` and `fragment` is one program. The unit counted is one
//! program, i.e. one dEQP test case. Each is compiled stage by stage and
//! linked through `SHC_ONLINE`, which is what glCompileShader and
//! glLinkProgram reach.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test es3_shaderc_corpus
//! DEQP_GLES3_TEST_DIR=<VK-GL-CTS>/data/gles3/shaders \
//!     cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online \
//!     --test es3_shaderc_corpus -- --nocapture
//! ```
//!
//! GLSL ES 3.00 is refused by a `report-es2` build (pp.rs out_of_scope), so
//! the file is empty there.
#![cfg(all(
    feature = "shaderc-online",
    not(feature = "report-es2"),
    any(feature = "gpu-powervr", feature = "gpu-vivante", feature = "gpu-v3d")
))]
use core::ffi::c_char;
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::Path;
use std::sync::Mutex;
use yakogl::shaderc::{ShcShader, SHC_STAGE_FRAGMENT, SHC_STAGE_VERTEX};

/// The compiler keeps its output in one arena that every compile resets.
static ONE_AT_A_TIME: Mutex<()> = Mutex::new(());

// ---- the front end under test ---------------------------------------------

fn text_of(log: &[c_char]) -> String {
    log.iter().take_while(|c| **c != 0).map(|c| *c as u8 as char).collect()
}

fn compile(stage: u32, src: &str) -> Result<ShcShader, String> {
    let mut out = ShcShader::ZERO;
    let mut log = [0 as c_char; 1024];
    let text = format!("{src}\0");
    let compile = yakogl::shaderc::online::SHC_ONLINE.compile.expect("the online back end compiles");
    // SAFETY: a NUL-terminated source, a real output, and a log of the size announced.
    let rc = unsafe { compile(stage, text.as_ptr() as *const c_char, &mut out, log.as_mut_ptr(), log.len()) };
    if rc == 0 {
        Ok(out)
    } else {
        Err(text_of(&log))
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Outcome {
    Linked,
    CompileFail(String),
    LinkFail(String),
}

/// glCompileShader on both stages, then glLinkProgram.
fn build(vs: &str, fs: &str) -> Outcome {
    yakogl::shaderc::online::backend::forget_compiled();
    let vso = match compile(SHC_STAGE_VERTEX, vs) {
        Ok(o) => o,
        Err(m) => return Outcome::CompileFail(format!("vertex: {m}")),
    };
    let mut fso = match compile(SHC_STAGE_FRAGMENT, fs) {
        Ok(o) => o,
        Err(m) => return Outcome::CompileFail(format!("fragment: {m}")),
    };
    let Some(link) = yakogl::shaderc::online::SHC_ONLINE.link else {
        return Outcome::Linked;
    };
    let mut log = [0 as c_char; 1024];
    // SAFETY: two distinct compiled shaders and a log of the size announced.
    let rc = unsafe { link(&vso, &mut fso, log.as_mut_ptr(), log.len()) };
    if rc == 0 {
        Outcome::Linked
    } else {
        Outcome::LinkFail(text_of(&log))
    }
}

// ---- the .test format ------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Debug)]
enum Expect {
    Pass,
    CompileFail,
    LinkFail,
    CompileOrLinkFail,
}

#[derive(Default, Clone)]
struct Values {
    inputs: Vec<(String, String)>,
    outputs: Vec<(String, String)>,
    uniforms: Vec<(String, String)>,
}

struct Case {
    name: String,
    glsl300: bool,
    expect: Expect,
    values: Values,
    both: Option<String>,
    vertex: Vec<String>,
    fragment: Vec<String>,
}

/// gluShaderLibrary's `removeExtraIndentation`: the indentation of the source's
/// first line (a tab counts 4) is taken off every line, after the escapes are
/// resolved. It matters for a line continuation: `#def\` / `    ine` is the
/// directive `#define` only once the second line's indentation is gone
/// (dEQP `preprocessor.line_continuation`).
fn remove_extra_indentation(src: &str) -> String {
    let is_ws = |c: char| matches!(c, ' ' | '\t' | '\n' | '\r');
    let width = |c: char| if c == '\t' { 4 } else { 1 };
    let indent: usize = src.chars().take_while(|&c| is_ws(c)).map(width).sum();
    let mut out = String::with_capacity(src.len());
    let (mut at_start, mut omitted) = (true, 0usize);
    let mut it = src.chars().peekable();
    while let Some(c) = it.next() {
        if at_start && omitted < indent && (c == ' ' || c == '\t') {
            omitted += width(c);
        } else if c == '\n' || c == '\r' {
            if c == '\r' && it.peek() == Some(&'\n') {
                it.next();
                out.push('\n');
            } else {
                out.push(c);
            }
            at_start = true;
            omitted = 0;
        } else {
            out.push(c);
            at_start = false;
        }
    }
    out
}

/// Cursor over a .test file (gluShaderLibrary's tokenizer: words, `"..."`
/// strings, `""..""` shader sources, `#` comments to the end of the line).
struct Cursor<'a> {
    s: &'a str,
    p: usize,
}

impl<'a> Cursor<'a> {
    fn skip(&mut self) {
        let b = self.s.as_bytes();
        loop {
            while self.p < b.len() && b[self.p].is_ascii_whitespace() {
                self.p += 1;
            }
            if self.p < b.len() && b[self.p] == b'#' {
                while self.p < b.len() && b[self.p] != b'\n' {
                    self.p += 1;
                }
            } else {
                return;
            }
        }
    }

    /// The next word, `"string"` or `""source""`, unescaped as dEQP does.
    fn next(&mut self) -> Option<String> {
        self.skip();
        let b = self.s.as_bytes();
        if self.p >= b.len() {
            return None;
        }
        if b[self.p..].starts_with(b"\"\"") {
            self.p += 2;
            let mut out = Vec::new();
            while self.p < b.len() && !b[self.p..].starts_with(b"\"\"") {
                if b[self.p] == b'\\' {
                    out.push(match b[self.p + 1] {
                        b'n' => b'\n',
                        b't' => b'\t',
                        c => c,
                    });
                    self.p += 2;
                } else {
                    out.push(b[self.p]);
                    self.p += 1;
                }
            }
            self.p += 2;
            return Some(remove_extra_indentation(&String::from_utf8_lossy(&out)));
        }
        if b[self.p] == b'"' {
            let start = self.p + 1;
            self.p = start;
            while self.p < b.len() && b[self.p] != b'"' {
                self.p += if b[self.p] == b'\\' { 2 } else { 1 };
            }
            let t = self.s[start..self.p].to_string();
            self.p += 1;
            return Some(t);
        }
        let start = self.p;
        while self.p < b.len() && !b[self.p].is_ascii_whitespace() {
            self.p += 1;
        }
        Some(self.s[start..self.p].to_string())
    }

    /// The raw text of a `{ ... }` block (values blocks hold no nested braces).
    fn block(&mut self) -> String {
        self.skip();
        assert_eq!(self.s.as_bytes()[self.p], b'{', "a values block opens with {{");
        self.p += 1;
        let start = self.p;
        let end = self.s[start..].find('}').expect("a values block closes with }") + start;
        self.p = end + 1;
        self.s[start..end].to_string()
    }
}

fn parse_values(text: &str) -> Values {
    let mut v = Values::default();
    for stmt in text.split(';') {
        let Some((decl, _)) = stmt.split_once('=') else { continue };
        let words: Vec<&str> = decl.split_whitespace().collect();
        let [kind, rest @ ..] = words.as_slice() else { continue };
        let [.., ty, name] = rest else { continue };
        let row = (ty.to_string(), name.to_string());
        match *kind {
            "input" => v.inputs.push(row),
            "output" => v.outputs.push(row),
            "uniform" => v.uniforms.push(row),
            _ => {}
        }
    }
    v
}

/// The cases of one file, each with its group path (`a.b.name`).
fn parse_file(text: &str) -> Vec<Case> {
    let mut c = Cursor { s: text, p: 0 };
    let mut groups: Vec<String> = Vec::new();
    let mut cases = Vec::new();
    while let Some(tok) = c.next() {
        match tok.as_str() {
            "group" => {
                groups.push(c.next().expect("a group name"));
                c.next(); // the description
            }
            "end" => {
                groups.pop();
            }
            "import" | "bitness" => {
                c.next();
            }
            "case" => {
                let mut case = Case {
                    name: c.next().expect("a case name"),
                    glsl300: false,
                    expect: Expect::Pass,
                    values: Values::default(),
                    both: None,
                    vertex: Vec::new(),
                    fragment: Vec::new(),
                };
                if !groups.is_empty() {
                    case.name = format!("{}.{}", groups.join("."), case.name);
                }
                loop {
                    let t = c.next().expect("a case ends with `end`");
                    match t.as_str() {
                        "end" => break,
                        "desc" | "output_color" => {
                            c.next();
                        }
                        "expect" => {
                            case.expect = match c.next().expect("an expectation").as_str() {
                                "pass" | "build_successful" | "validation_fail" => Expect::Pass,
                                "compile_fail" => Expect::CompileFail,
                                "link_fail" => Expect::LinkFail,
                                "compile_or_link_fail" => Expect::CompileOrLinkFail,
                                other => panic!("{}: unknown expectation {other}", case.name),
                            }
                        }
                        "version" => {
                            let n = c.next().expect("a version number");
                            let save = c.p;
                            if c.next().as_deref() != Some("es") {
                                c.p = save;
                            }
                            case.glsl300 = n == "300";
                        }
                        "require" => {
                            c.next();
                        }
                        "values" => case.values = parse_values(&c.block()),
                        "both" => case.both = Some(c.next().expect("a source")),
                        "vertex" => case.vertex.push(c.next().expect("a source")),
                        "fragment" => case.fragment.push(c.next().expect("a source")),
                        other => panic!("{}: unexpected token {other}", case.name),
                    }
                }
                cases.push(case);
            }
            other => panic!("unexpected top-level token {other}"),
        }
    }
    cases
}

// ---- dEQP's source specialisation -------------------------------------------

fn float_of(ty: &str) -> String {
    match ty {
        "bool" | "int" | "uint" => "float".into(),
        t if t.starts_with("bvec") || t.starts_with("ivec") || t.starts_with("uvec") => format!("vec{}", &t[4..]),
        t => t.into(),
    }
}

fn is_float(ty: &str) -> bool {
    float_of(ty) == ty
}

fn is_int_like(ty: &str) -> bool {
    ty == "int" || ty.starts_with("ivec")
}

/// glu::sl::genCompareFunctions.
fn compare_functions(values: &Values, use_float_types: bool) -> String {
    let mut seen: Vec<&str> = Vec::new();
    for (ty, _) in &values.outputs {
        if !seen.contains(&ty.as_str()) {
            seen.push(ty);
        }
    }
    let mut s = String::new();
    for ty in seen {
        if is_float(ty) {
            if ty.starts_with("mat") {
                // the per-matrix columns of dEQP's table, folded into one loop-free form
                let cols: usize = ty[3..4].parse().unwrap();
                let rows: usize = if ty.len() > 4 { ty[5..6].parse().unwrap() } else { cols };
                let mut diff = "abs(a[0]-b[0])".to_string();
                for i in 1..cols {
                    diff = format!("max({diff}, abs(a[{i}]-b[{i}]))");
                }
                let _ = writeln!(
                    s,
                    "bool isOk ({ty} a, {ty} b, float eps) {{ vec{rows} diff = {diff}; return all(lessThanEqual(diff, vec{rows}(eps))); }}"
                );
            } else if ty == "float" {
                s.push_str("bool isOk (float a, float b, float eps) { return (abs(a-b) <= (eps*abs(b) + eps)); }\n");
            } else {
                let _ = writeln!(
                    s,
                    "bool isOk ({ty} a, {ty} b, float eps) {{ return all(lessThanEqual(abs(a-b), (eps*abs(b) + eps))); }}"
                );
            }
        } else if use_float_types {
            let f = float_of(ty);
            let body = match ty {
                "bool" => "return ((a > 0.5) == b);".to_string(),
                "int" => "float atemp = a+0.5; return (float(b) <= atemp && atemp <= float(b+1));".into(),
                "uint" => "float atemp = a+0.5; return (float(b) <= atemp && atemp <= float(b+1u));".into(),
                t if t.starts_with('b') => format!("return (greaterThan(a, {f}(0.5)) == b);"),
                t => format!("return ({t}(floor(a + 0.5)) == b);"),
            };
            let _ = writeln!(s, "bool isOk ({f} a, {ty} b) {{ {body} }}");
        } else {
            let _ = writeln!(s, "bool isOk ({ty} a, {ty} b) {{ return (a == b); }}");
        }
    }
    s
}

/// glu::sl::genCompareOp.
fn compare_op(dst: &str, values: &Values, prefix: &str) -> String {
    let mut s = String::new();
    for (i, (ty, name)) in values.outputs.iter().enumerate() {
        s.push_str(if i == 0 { "bool RES = " } else { "RES = RES && " });
        if is_float(ty) {
            let _ = writeln!(s, "isOk({name}, ref_{name}, 0.05);");
        } else {
            let _ = writeln!(s, "isOk({prefix}{name}, ref_{name});");
        }
    }
    if values.outputs.is_empty() {
        let _ = writeln!(s, "{dst} = vec4(1.0);");
    } else {
        let _ = writeln!(s, "{dst} = vec4(RES, RES, RES, 1.0);");
    }
    s
}

/// `${NAME}` replaced by its parameter; an unknown name is a harness error.
fn specialize(src: &str, params: &[(&str, String)]) -> Result<String, String> {
    let mut out = String::new();
    let mut rest = src;
    while let Some(i) = rest.find("${") {
        out.push_str(&rest[..i]);
        let Some(j) = rest[i..].find('}') else { return Err("unterminated ${".into()) };
        let key = &rest[i + 2..i + j];
        /* `${NAME:single-line}`: the value on one line (glu::StringTemplate) */
        let (key, single_line) = key.strip_suffix(":single-line").map_or((key, false), |k| (k, true));
        match params.iter().find(|(k, _)| *k == key) {
            Some((_, v)) if single_line => out.push_str(&v.replace('\n', " ")),
            Some((_, v)) => out.push_str(v),
            None => return Err(format!("no value for ${{{key}}}")),
        }
        rest = &rest[i + j + 1..];
    }
    out.push_str(rest);
    Ok(out)
}

struct Gen {
    inout: bool,
    version: &'static str,
}

impl Gen {
    fn vtx_in(&self) -> &'static str {
        if self.inout {
            "in"
        } else {
            "attribute"
        }
    }
    fn vtx_out(&self) -> &'static str {
        if self.inout {
            "out"
        } else {
            "varying"
        }
    }
    fn frag_in(&self) -> &'static str {
        if self.inout {
            "in"
        } else {
            "varying"
        }
    }
    fn frag_color(&self) -> &'static str {
        if self.inout {
            "dEQP_FragColor"
        } else {
            "gl_FragColor"
        }
    }

    /// genVertexShader: the vertex stage of a program testing the fragment shader.
    fn vertex_stage(&self, v: &Values) -> String {
        let mut s = format!("{}\nprecision highp float;\nprecision highp int;\n\n", self.version);
        let _ = writeln!(s, "{} highp vec4 dEQP_Position;", self.vtx_in());
        for (ty, name) in &v.inputs {
            let f = float_of(ty);
            let _ = writeln!(s, "{} {f} a_{name};", self.vtx_in());
            let pre = if is_float(ty) { "" } else { "v_" };
            let _ = writeln!(s, "{} {f} {pre}{name};", self.vtx_out());
        }
        s.push_str("\nvoid main()\n{\n    gl_Position = dEQP_Position;\n");
        for (ty, name) in &v.inputs {
            let pre = if is_float(ty) { "" } else { "v_" };
            let _ = writeln!(s, "    {pre}{name} = a_{name};");
        }
        s.push_str("}\n");
        s
    }

    /// genFragmentShader: the fragment stage of a program testing the vertex shader.
    fn fragment_stage(&self, v: &Values) -> String {
        let mut s = format!("{}\nprecision highp float;\nprecision highp int;\n\n", self.version);
        if self.inout {
            s.push_str("layout(location = 0) out mediump vec4 dEQP_FragColor;\n\n");
        }
        s.push_str(&compare_functions(v, true));
        s.push('\n');
        for (ty, name) in &v.outputs {
            let f = float_of(ty);
            let pre = if is_float(ty) { "" } else { "v_" };
            let _ = writeln!(s, "{} {f} {pre}{name};", self.frag_in());
            let _ = writeln!(s, "uniform {ty} ref_{name};");
        }
        let _ = write!(s, "\nvoid main()\n{{\n    {}}}\n", compare_op(self.frag_color(), v, "v_"));
        s
    }

    /// specializeVertexShader (a `both` case's vertex test).
    fn specialize_vertex(&self, src: &str, v: &Values) -> Result<String, String> {
        let mut decl = format!("{} highp vec4 dEQP_Position;\n", self.vtx_in());
        let mut setup = String::new();
        let mut output = "gl_Position = dEQP_Position;\n".to_string();
        for (ty, name) in &v.inputs {
            let f = float_of(ty);
            if is_float(ty) {
                let _ = writeln!(decl, "{} {f} {name};", self.vtx_in());
            } else {
                let _ = writeln!(decl, "{} {f} a_{name};", self.vtx_in());
                let _ = writeln!(setup, "{ty} {name} = {ty}(a_{name});");
            }
        }
        for (ty, name) in &v.outputs {
            let f = float_of(ty);
            if is_float(ty) {
                let _ = writeln!(decl, "{} {f} {name};", self.vtx_out());
            } else {
                let _ = writeln!(decl, "{} {f} v_{name};", self.vtx_out());
                let _ = writeln!(decl, "{ty} {name};");
                let _ = writeln!(output, "v_{name} = {f}({name});");
            }
        }
        specialize(
            src,
            &[
                ("DECLARATIONS", decl),
                ("SETUP", setup),
                ("OUTPUT", output),
                ("POSITION_FRAG_COLOR", "gl_Position".into()),
            ],
        )
    }

    /// specializeFragmentShader (a `both` case's fragment test).
    fn specialize_fragment(&self, src: &str, v: &Values) -> Result<String, String> {
        let mut decl = compare_functions(v, false);
        let output = compare_op(self.frag_color(), v, "");
        let mut setup = String::new();
        if self.inout {
            decl.push_str("layout(location = 0) out mediump vec4 dEQP_FragColor;\n");
        }
        for (ty, name) in &v.inputs {
            let f = float_of(ty);
            if is_float(ty) {
                let _ = writeln!(decl, "{} {f} {name};", self.frag_in());
            } else {
                let _ = writeln!(decl, "{} {f} v_{name};", self.frag_in());
                let off = if is_int_like(ty) { " * 1.0025" } else { "" };
                let _ = writeln!(setup, "{ty} {name} = {ty}(v_{name}{off});");
            }
        }
        for (ty, name) in &v.outputs {
            let _ = writeln!(decl, "uniform {ty} ref_{name};");
            let _ = writeln!(decl, "{ty} {name};");
        }
        specialize(
            src,
            &[
                ("DECLARATIONS", decl),
                ("SETUP", setup),
                ("OUTPUT", output),
                ("POSITION_FRAG_COLOR", self.frag_color().into()),
            ],
        )
    }

    fn uniform_decls(v: &Values) -> String {
        let mut s = String::new();
        for (ty, name) in &v.uniforms {
            if !name.contains('.') {
                let _ = writeln!(s, "uniform {ty} {name};");
            }
        }
        s
    }

    /// generateVertexSpecialization (a case with its own vertex and fragment shader).
    fn complete_vertex(&self, src: &str, v: &Values) -> Result<String, String> {
        let mut decl = format!("{} highp vec4 dEQP_Position;\n", self.vtx_in());
        let mut setup = String::new();
        for (ty, name) in &v.inputs {
            if is_float(ty) {
                let _ = writeln!(decl, "{} {ty} {name};", self.vtx_in());
            } else {
                let _ = writeln!(decl, "{} {} a_{name};", self.vtx_in(), float_of(ty));
                let _ = writeln!(setup, "{ty} {name} = {ty}(a_{name});");
            }
        }
        decl.push_str(&Self::uniform_decls(v));
        specialize(
            src,
            &[
                ("VERTEX_DECLARATIONS", decl),
                ("VERTEX_SETUP", setup),
                ("VERTEX_OUTPUT", "gl_Position = dEQP_Position;\n".into()),
            ],
        )
    }

    /// generateFragmentSpecialization.
    fn complete_fragment(&self, src: &str, v: &Values) -> Result<String, String> {
        let mut decl = compare_functions(v, false);
        let output = compare_op(self.frag_color(), v, "");
        if self.inout {
            decl.push_str("layout(location = 0) out mediump vec4 dEQP_FragColor;\n");
        }
        for (ty, name) in &v.outputs {
            let _ = writeln!(decl, "uniform {ty} ref_{name};");
            let _ = writeln!(decl, "{ty} {name};");
        }
        decl.push_str(&Self::uniform_decls(v));
        specialize(
            src,
            &[("FRAGMENT_DECLARATIONS", decl), ("FRAGMENT_OUTPUT", output), ("FRAG_COLOR", self.frag_color().into())],
        )
    }
}

/// A program's vertex and fragment source, or why this harness cannot form it.
type Sources = Result<(String, String), String>;

/// One program of a case: the dEQP test case's name suffix, and its sources.
fn programs(case: &Case) -> Vec<(String, Sources)> {
    let g = Gen { inout: case.glsl300, version: if case.glsl300 { "#version 300 es" } else { "#version 100" } };
    let v = &case.values;
    if let Some(both) = &case.both {
        return vec![
            ("_vertex".into(), g.specialize_vertex(both, v).map(|vs| (vs, g.fragment_stage(v)))),
            ("_fragment".into(), g.specialize_fragment(both, v).map(|fs| (g.vertex_stage(v), fs))),
        ];
    }
    let one = if case.vertex.len() == 1 && case.fragment.len() == 1 {
        g.complete_vertex(&case.vertex[0], v)
            .and_then(|vs| g.complete_fragment(&case.fragment[0], v).map(|fs| (vs, fs)))
    } else {
        Err(format!(
            "{} vertex and {} fragment sources (several units per stage are not linked here)",
            case.vertex.len(),
            case.fragment.len()
        ))
    };
    vec![(String::new(), one)]
}

// ---- running and scoring ----------------------------------------------------

#[derive(Debug, PartialEq, Clone, Copy)]
enum Verdict {
    /// expected to build, built
    PassCompiled,
    /// expected to build, refused
    PassRefused,
    /// expected to fail, refused as expected
    FailRefused,
    /// expected to fail, built (or failed at the wrong step)
    FailAccepted,
    /// the harness cannot form the program
    Skipped,
}

fn judge(expect: Expect, o: &Outcome) -> Verdict {
    match (expect, o) {
        (Expect::Pass, Outcome::Linked) => Verdict::PassCompiled,
        (Expect::Pass, _) => Verdict::PassRefused,
        (Expect::CompileFail, Outcome::CompileFail(_)) => Verdict::FailRefused,
        (Expect::LinkFail, Outcome::LinkFail(_)) => Verdict::FailRefused,
        (Expect::CompileOrLinkFail, Outcome::CompileFail(_) | Outcome::LinkFail(_)) => Verdict::FailRefused,
        _ => Verdict::FailAccepted,
    }
}

struct Unit {
    name: String,
    expect: Expect,
    verdict: Verdict,
    detail: String,
}

fn run_case(case: &Case) -> Vec<Unit> {
    programs(case)
        .into_iter()
        .map(|(suffix, srcs)| {
            let name = format!("{}{suffix}", case.name);
            let (verdict, detail) = match srcs {
                Err(why) => (Verdict::Skipped, why),
                Ok((vs, fs)) => {
                    let outcome = match std::panic::catch_unwind(|| build(&vs, &fs)) {
                        Ok(o) => o,
                        Err(_) => Outcome::CompileFail("PANIC in the compiler".into()),
                    };
                    let detail = match &outcome {
                        Outcome::Linked => "built".to_string(),
                        Outcome::CompileFail(m) => format!("compile: {m}"),
                        Outcome::LinkFail(m) => format!("link: {m}"),
                    };
                    (judge(case.expect, &outcome), detail)
                }
            };
            Unit { name, expect: case.expect, verdict, detail }
        })
        .collect()
}

// ---- part (a): inline cases --------------------------------------------------

/// Cases from VK-GL-CTS data/gles3/shaders (Apache-2.0, Copyright (c) 2014
/// The Android Open Source Project, The Khronos Group Inc.): declarations.test
/// `in_in_vertex_main` and `switch.test` `condition_float` (both compile_fail),
/// and a values-driven `both` case in the shape of conversions.test
/// `int_to_float` (pass).
const INLINE_CASES: &str = r#"
group inline "Inline cases"
    case in_in_vertex_main
        version 300 es
        expect compile_fail
        vertex ""
            #version 300 es
            ${VERTEX_DECLARATIONS}
            void main()
            {
                in mediump float val;
                gl_Position = vec4(1.0);
            }
        ""
        fragment ""
            #version 300 es
            ${FRAGMENT_DECLARATIONS}
            void main()
            {
                ${FRAG_COLOR} = vec4(1.0);
            }
        ""
    end

    case condition_float
        version 300 es
        expect compile_fail
        both ""
            #version 300 es
            precision mediump float;
            ${DECLARATIONS}

            void main ()
            {
                float c = 1.0;
                switch (c)
                {
                    case 1:
                        ${POSITION_FRAG_COLOR} = vec4(1.0);
                }

                ${OUTPUT}
            }
        ""
    end

    case int_to_float
        version 300 es
        values
        {
            input int in0 = [ 0 | 1 | 2 | 5 | 8 | 11 | -12 | -66 | -192 | 255 ];
            output float out0 = [ 0.0 | 1.0 | 2.0 | 5.0 | 8.0 | 11.0 | -12.0 | -66.0 | -192.0 | 255.0 ];
        }

        both ""
            #version 300 es
            precision mediump float;
            precision mediump int;

            ${DECLARATIONS}

            void main()
            {
                ${SETUP}
                out0 = float(in0);
                ${OUTPUT}
            }
        ""
    end
end
"#;

#[test]
fn inline_cases() {
    let _g = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    let cases = parse_file(INLINE_CASES);
    assert_eq!(cases.len(), 3);
    assert_eq!(cases[0].name, "inline.in_in_vertex_main");
    assert_eq!(cases[2].values.inputs, [("int".to_string(), "in0".to_string())]);
    let mut units = Vec::new();
    for c in &cases {
        units.extend(run_case(c));
    }
    // two programs from each `both` case, one from the complete case
    assert_eq!(units.len(), 5);
    for u in &units {
        assert!(
            matches!(u.verdict, Verdict::PassCompiled | Verdict::FailRefused),
            "{} (expect {:?}): {:?}: {}",
            u.name,
            u.expect,
            u.verdict,
            u.detail
        );
    }
}

// ---- part (b): the corpus ----------------------------------------------------

#[derive(Default)]
struct Counts {
    pass_compiled: usize,
    pass_refused: usize,
    fail_refused: usize,
    fail_accepted: usize,
    skipped: usize,
}

#[test]
fn corpus() {
    let Some(dir) = std::env::var_os("DEQP_GLES3_TEST_DIR") else {
        println!("es3_shaderc_corpus: DEQP_GLES3_TEST_DIR is not set; the dEQP corpus part is skipped");
        return;
    };
    let _g = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    let mut files: Vec<_> = std::fs::read_dir(Path::new(&dir))
        .expect("DEQP_GLES3_TEST_DIR is readable")
        .map(|e| e.expect("a directory entry").path())
        .filter(|p| p.extension().and_then(|x| x.to_str()) == Some("test"))
        .collect();
    files.sort();
    assert!(!files.is_empty(), "no .test files in {}", Path::new(&dir).display());

    // Parse first, so a malformed file panics with its message.
    let parsed: Vec<(String, Vec<Case>)> = files
        .iter()
        .map(|p| {
            let text = std::fs::read_to_string(p).expect("a UTF-8 .test file");
            (p.file_stem().unwrap().to_string_lossy().into_owned(), parse_file(&text))
        })
        .collect();

    // A compile that panics must not print a backtrace per case.
    let hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));

    let mut table: BTreeMap<String, Counts> = BTreeMap::new();
    let mut report = String::new();
    for (file, cases) in &parsed {
        let counts = table.entry(file.clone()).or_default();
        for case in cases {
            for u in run_case(case) {
                match u.verdict {
                    Verdict::PassCompiled => counts.pass_compiled += 1,
                    Verdict::PassRefused => counts.pass_refused += 1,
                    Verdict::FailRefused => counts.fail_refused += 1,
                    Verdict::FailAccepted => counts.fail_accepted += 1,
                    Verdict::Skipped => counts.skipped += 1,
                }
                if u.verdict != Verdict::PassCompiled {
                    let first = u.detail.lines().next().unwrap_or("");
                    let _ = writeln!(report, "{file}\t{}\t{:?}\t{:?}\t{first}", u.name, u.expect, u.verdict);
                }
            }
        }
    }
    std::panic::set_hook(hook);

    println!("{:<28}{:>8}{:>8}{:>10}{:>10}{:>8}", "file", "pass ok", "pass no", "fail ok", "fail acc", "skipped");
    let mut sum = Counts::default();
    for (f, c) in &table {
        println!(
            "{f:<28}{:>8}{:>8}{:>10}{:>10}{:>8}",
            c.pass_compiled, c.pass_refused, c.fail_refused, c.fail_accepted, c.skipped
        );
        sum.pass_compiled += c.pass_compiled;
        sum.pass_refused += c.pass_refused;
        sum.fail_refused += c.fail_refused;
        sum.fail_accepted += c.fail_accepted;
        sum.skipped += c.skipped;
    }
    println!(
        "{:<28}{:>8}{:>8}{:>10}{:>10}{:>8}",
        "total", sum.pass_compiled, sum.pass_refused, sum.fail_refused, sum.fail_accepted, sum.skipped
    );
    if let Some(path) = std::env::var_os("DEQP_GLES3_REPORT") {
        std::fs::write(path, &report).expect("the report file is writable");
    }
    let total = sum.pass_compiled + sum.pass_refused + sum.fail_refused + sum.fail_accepted + sum.skipped;
    assert!(total > 1000, "only {total} programs came out of {} files", files.len());
}
