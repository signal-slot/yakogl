// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! A reader for dEQP's shader library `.test` files (VK-GL-CTS
//! `data/gles2/shaders` and `data/gles3/shaders`), shared by the tests that
//! measure a shader compiler over them. Backend-independent: text in, GLSL
//! sources out, no compiler call.
//!
//! Include it with
//! `#[path = "deqp_shader_library/mod.rs"] mod deqp_shader_library;`.
//!
//! The case format and the `${...}` expansion follow dEQP's `glsShaderLibrary`
//! (framework/opengl/gluShaderLibrary.cpp and
//! modules/glshared/glsShaderLibraryCase.cpp). `version 100` cases get
//! `attribute`/`varying` in place of `in`/`out`, `gl_FragColor` in place of a
//! declared output, and a `mediump` default fragment precision in the
//! generated fragment stage (highp is optional in a 1.00 fragment shader);
//! `version 300 es` cases get `in`/`out` and `dEQP_FragColor`.
#![allow(dead_code)]
use std::fmt::Write as _;

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Expect {
    Pass,
    CompileFail,
    LinkFail,
    CompileOrLinkFail,
}

#[derive(Default, Clone)]
pub struct Values {
    pub inputs: Vec<(String, String)>,
    pub outputs: Vec<(String, String)>,
    pub uniforms: Vec<(String, String)>,
}

pub struct Case {
    pub name: String,
    pub glsl300: bool,
    pub expect: Expect,
    pub values: Values,
    pub both: Option<String>,
    pub vertex: Vec<String>,
    pub fragment: Vec<String>,
}

/// gluShaderLibrary's `removeExtraIndentation`: the indentation of the source's
/// first line (a tab counts 4) is taken off every line, after the escapes are
/// resolved. It matters for a line continuation: `#def\` / `    ine` is the
/// directive `#define` only once the second line's indentation is gone
/// (dEQP `preprocessor.line_continuation`).
pub fn remove_extra_indentation(src: &str) -> String {
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
pub struct Cursor<'a> {
    pub s: &'a str,
    pub p: usize,
}

impl<'a> Cursor<'a> {
    pub fn skip(&mut self) {
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
    pub fn next(&mut self) -> Option<String> {
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
    pub fn block(&mut self) -> String {
        self.skip();
        assert_eq!(self.s.as_bytes()[self.p], b'{', "a values block opens with {{");
        self.p += 1;
        let start = self.p;
        let end = self.s[start..].find('}').expect("a values block closes with }") + start;
        self.p = end + 1;
        self.s[start..end].to_string()
    }
}

pub fn parse_values(text: &str) -> Values {
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
pub fn parse_file(text: &str) -> Vec<Case> {
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

pub fn float_of(ty: &str) -> String {
    match ty {
        "bool" | "int" | "uint" => "float".into(),
        t if t.starts_with("bvec") || t.starts_with("ivec") || t.starts_with("uvec") => format!("vec{}", &t[4..]),
        t => t.into(),
    }
}

pub fn is_float(ty: &str) -> bool {
    float_of(ty) == ty
}

pub fn is_int_like(ty: &str) -> bool {
    ty == "int" || ty.starts_with("ivec")
}

/// glu::sl::genCompareFunctions.
pub fn compare_functions(values: &Values, use_float_types: bool) -> String {
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
pub fn compare_op(dst: &str, values: &Values, prefix: &str) -> String {
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
pub fn specialize(src: &str, params: &[(&str, String)]) -> Result<String, String> {
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

pub struct Gen {
    pub inout: bool,
    pub version: &'static str,
}

impl Gen {
    /// The generator for a case's `version` (`#version 300 es` or `#version 100`).
    pub fn for_case(case: &Case) -> Gen {
        Gen { inout: case.glsl300, version: if case.glsl300 { "#version 300 es" } else { "#version 100" } }
    }

    /// The default precision lines of a generated fragment stage: highp in
    /// 3.00; mediump in 1.00, where highp is optional in a fragment shader.
    fn fragment_precision(&self) -> &'static str {
        if self.inout {
            "precision highp float;\nprecision highp int;\n"
        } else {
            "precision mediump float;\nprecision mediump int;\n"
        }
    }

    pub fn vtx_in(&self) -> &'static str {
        if self.inout {
            "in"
        } else {
            "attribute"
        }
    }
    pub fn vtx_out(&self) -> &'static str {
        if self.inout {
            "out"
        } else {
            "varying"
        }
    }
    pub fn frag_in(&self) -> &'static str {
        if self.inout {
            "in"
        } else {
            "varying"
        }
    }
    pub fn frag_color(&self) -> &'static str {
        if self.inout {
            "dEQP_FragColor"
        } else {
            "gl_FragColor"
        }
    }

    /// genVertexShader: the vertex stage of a program testing the fragment shader.
    pub fn vertex_stage(&self, v: &Values) -> String {
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
    pub fn fragment_stage(&self, v: &Values) -> String {
        let mut s = format!("{}\n{}\n", self.version, self.fragment_precision());
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
    pub fn specialize_vertex(&self, src: &str, v: &Values) -> Result<String, String> {
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
    pub fn specialize_fragment(&self, src: &str, v: &Values) -> Result<String, String> {
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

    pub fn uniform_decls(v: &Values) -> String {
        let mut s = String::new();
        for (ty, name) in &v.uniforms {
            if !name.contains('.') {
                let _ = writeln!(s, "uniform {ty} {name};");
            }
        }
        s
    }

    /// generateVertexSpecialization (a case with its own vertex and fragment shader).
    pub fn complete_vertex(&self, src: &str, v: &Values) -> Result<String, String> {
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
    pub fn complete_fragment(&self, src: &str, v: &Values) -> Result<String, String> {
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
pub type Sources = Result<(String, String), String>;

/// One program of a case: the dEQP test case's name suffix, and its sources.
pub fn programs(case: &Case) -> Vec<(String, Sources)> {
    let g = Gen::for_case(case);
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Small hand-written snippets in the `.test` format, version 100.
    const ES100: &str = r#"
group g "g"
    case both_case
        version 100
        values { input float in0 = 1.0; output float out0 = 1.0; }
        both ""
            precision mediump float;
            ${DECLARATIONS}
            void main()
            {
                ${SETUP}
                out0 = in0;
                ${OUTPUT}
            }
        ""
    end
    case values_case
        version 100
        values { input vec2 a = vec2(1.0, 2.0); output vec2 r = vec2(1.0, 2.0); uniform float u = 1.0; }
        vertex ""
            ${VERTEX_DECLARATIONS}
            varying mediump vec2 v;
            void main()
            {
                ${VERTEX_SETUP}
                v = a;
                ${VERTEX_OUTPUT}
            }
        ""
        fragment ""
            precision mediump float;
            ${FRAGMENT_DECLARATIONS}
            varying mediump vec2 v;
            void main()
            {
                r = v * u;
                ${FRAGMENT_OUTPUT}
            }
        ""
    end
    case bad
        expect compile_fail
        version 100
        values {}
        both ""
            ${DECLARATIONS}
            void main() { ${OUTPUT} int x = 1.0; }
        ""
    end
end
"#;

    fn one(name: &str) -> Case {
        parse_file(ES100).into_iter().find(|c| c.name == format!("g.{name}")).expect("the case")
    }

    fn sources(case: &Case) -> Vec<(String, String, String)> {
        programs(case)
            .into_iter()
            .map(|(n, s)| {
                let (vs, fs) = s.expect("sources form");
                (n, vs, fs)
            })
            .collect()
    }

    fn no_es3_words(s: &str) {
        assert!(!s.contains("dEQP_FragColor"), "{s}");
        assert!(!s.contains("layout("), "{s}");
        assert!(!s.lines().any(|l| l.starts_with("in ") || l.starts_with("out ")), "{s}");
    }

    #[test]
    fn es100_both_case() {
        let c = one("both_case");
        assert!(!c.glsl300);
        let p = sources(&c);
        assert_eq!(p.iter().map(|x| x.0.as_str()).collect::<Vec<_>>(), ["_vertex", "_fragment"]);
        // vertex test: the helper fragment stage compares into gl_FragColor
        // under mediump defaults.
        let (_, vs, fs) = &p[0];
        no_es3_words(vs);
        no_es3_words(fs);
        assert!(vs.contains("attribute float in0;"), "{vs}");
        assert!(vs.contains("varying float out0;"), "{vs}");
        assert!(fs.starts_with("#version 100\n"), "{fs}");
        assert!(fs.contains("precision mediump float;"), "{fs}");
        assert!(!fs.contains("highp float"), "{fs}");
        assert!(fs.contains("varying float out0;"), "{fs}");
        assert!(fs.contains("gl_FragColor ="), "{fs}");
        // fragment test: the helper vertex stage forwards the inputs.
        let (_, vs, fs) = &p[1];
        no_es3_words(vs);
        no_es3_words(fs);
        assert!(vs.contains("attribute highp vec4 dEQP_Position;"), "{vs}");
        assert!(vs.contains("attribute float a_in0;") || vs.contains("attribute float in0;"), "{vs}");
        assert!(fs.contains("varying float in0;"), "{fs}");
        assert!(fs.contains("uniform float ref_out0;"), "{fs}");
        assert!(fs.contains("gl_FragColor ="), "{fs}");
    }

    #[test]
    fn es100_values_case() {
        let c = one("values_case");
        assert_eq!((c.vertex.len(), c.fragment.len()), (1, 1));
        let p = sources(&c);
        assert_eq!(p.len(), 1);
        let (_, vs, fs) = &p[0];
        no_es3_words(vs);
        no_es3_words(fs);
        assert!(vs.contains("attribute highp vec4 dEQP_Position;"), "{vs}");
        assert!(vs.contains("attribute vec2 a;"), "{vs}");
        assert!(vs.contains("uniform float u;"), "{vs}");
        assert!(vs.contains("gl_Position = dEQP_Position;"), "{vs}");
        assert!(fs.contains("uniform float u;"), "{fs}");
        assert!(fs.contains("uniform vec2 ref_r;"), "{fs}");
        assert!(fs.contains("gl_FragColor ="), "{fs}");
    }

    #[test]
    fn es100_compile_fail_case() {
        let c = one("bad");
        assert_eq!(c.expect, Expect::CompileFail);
        let p = sources(&c);
        let (_, vs, fs) = &p[1];
        no_es3_words(vs);
        no_es3_words(fs);
        assert!(fs.contains("int x = 1.0;"), "{fs}");
    }

    #[test]
    fn es300_keeps_inout() {
        let c = Case {
            name: "t".into(),
            glsl300: true,
            expect: Expect::Pass,
            values: parse_values("input float a = 1.0; output float b = 1.0;"),
            both: Some("${DECLARATIONS}\nvoid main() { ${SETUP} ${OUTPUT} }".into()),
            vertex: vec![],
            fragment: vec![],
        };
        let p = sources(&c);
        assert!(p[0].1.starts_with("in highp vec4 dEQP_Position;"));
        assert!(p[0].2.starts_with("#version 300 es\n") && p[1].1.starts_with("#version 300 es\n"));
        assert!(p[0].2.contains("precision highp float;") && p[0].2.contains("dEQP_FragColor"));
    }
}
