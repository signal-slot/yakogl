// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The GLSL front end of `glsl2usc`: GLSL ES 1.00 / 3.00 -> the Vulkan GLSL
//! 450 the PCO pipeline compiles, plus the name reflection the GL core needs.
//!
//! A faithful port of the regular-expression rewriting of the former
//! `tools/glsl2usc.py` (upstream 7e76640): the output text is byte-identical
//! for every input (checked over the whole shader set). Where Python's `re`
//! and the regex crate differ the code says so:
//!   * `\s` is widened to Python's set (py::re);
//!   * `$` without `(?m)` also matches before a final newline in Python; every
//!     such pattern here is applied to text that cannot end in a newline or
//!     ends in `\s*$`, where the two agree;
//!   * `\w`/`\b`/`\d` differ for non-ASCII characters: code outside comments
//!     must be ASCII (GLSL's character set; glslang rejects anything else),
//!     which [`code_text`] enforces, so the rewriting only ever sees ASCII;
//!   * the one backreference pattern (the loop unroller) is matched by hand.

use crate::py::{self, esc, g, grp, pint, re, strip};
use crate::{Error, R};
use std::collections::{HashMap, HashSet};

macro_rules! exit {
    ($($t:tt)*) => { return Err(Error::Exit(format!($($t)*))) };
}
pub(crate) use exit;

/// GLSL type -> (dwords, std430 alignment in dwords, GL type token).
pub fn glsl_type(t: &str) -> Option<(i64, i64, i64)> {
    Some(match t {
        "float" => (1, 1, 0x1406),
        "vec2" => (2, 2, 0x8B50),
        "vec3" => (3, 4, 0x8B51),
        "vec4" => (4, 4, 0x8B52),
        "int" => (1, 1, 0x1404),
        "ivec2" => (2, 2, 0x8B53),
        "ivec3" => (3, 4, 0x8B54),
        "ivec4" => (4, 4, 0x8B55),
        "bool" => (1, 1, 0x8B56),
        "bvec2" => (2, 2, 0x8B57),
        "bvec3" => (3, 4, 0x8B58),
        "bvec4" => (4, 4, 0x8B59),
        "mat2" => (4, 2, 0x8B5A),
        "mat3" => (12, 4, 0x8B5B),
        "mat4" => (16, 4, 0x8B5C),
        "uint" => (1, 1, 0x1405),
        "uvec2" => (2, 2, 0x8DC6),
        "uvec3" => (3, 4, 0x8DC7),
        "uvec4" => (4, 4, 0x8DC8),
        _ => return None,
    })
}

/// GLSL ES 3.00 sampler types the compiler and the TPU state handle.
pub fn sampler_type(t: &str) -> Option<i64> {
    Some(match t {
        "sampler2D" => 0x8B5E,
        "samplerCube" => 0x8B60,
        "sampler3D" => 0x8B5F,
        "sampler2DShadow" => 0x8B62,
        "sampler2DArray" => 0x8DC1,
        "sampler2DArrayShadow" => 0x8DC4,
        "samplerCubeShadow" => 0x8DC5,
        _ => return None,
    })
}

/// Locations a GLSL ES attribute / varying of this type takes.
fn es100_loc(t: &str) -> Option<i64> {
    if let Some((columns, _)) = matrix_shape(t) {
        /* one location per column (GL ES 2.0 section 2.10.4) */
        return Some(columns);
    }
    Some(match t {
        "float" | "vec2" | "vec3" | "vec4" | "int" | "ivec2" | "ivec3" | "ivec4" | "bool" | "bvec2" | "bvec3"
        | "bvec4" | "uint" | "uvec2" | "uvec3" | "uvec4" => 1,
        _ => return None,
    })
}

/// The prefix of the per-column inputs a matrix attribute is split into
/// (`pvr_mat_<name>_col<k>`); [`reflect_attribs`] maps them back to the name
/// and the matrix type the application declared.
pub const MAT_ATTR_PREFIX: &str = "pvr_mat_";

/// A matrix type as (columns, the type of one column): `mat3` is three
/// `vec3`s, `mat2x4` is two `vec4`s. `matCxR` is C columns of R rows, which
/// is the opposite order from the one the name suggests.
///
/// [`es100_loc`] and [`column_type`] both come from here so that they cannot
/// disagree: a matrix attribute occupies one location per *column* and is
/// split into one input per column, and answering those two questions in two
/// places is how a `mat2x4` would quietly get `vec4` columns at two locations
/// from one and `vec2` from the other.
fn matrix_shape(typ: &str) -> Option<(i64, &'static str)> {
    let col = |r: i64| -> &'static str {
        match r {
            2 => "vec2",
            3 => "vec3",
            _ => "vec4",
        }
    };
    let d = |c: char| c.to_digit(10).map(|d| d as i64);
    match typ.strip_prefix("mat")?.as_bytes() {
        [n] => d(*n as char).filter(|n| (2..=4).contains(n)).map(|n| (n, col(n))),
        [c, b'x', r] => match (d(*c as char), d(*r as char)) {
            (Some(c), Some(r)) if (2..=4).contains(&c) && (2..=4).contains(&r) => Some((c, col(r))),
            _ => None,
        },
        _ => None,
    }
}

/// The type of one column of a matrix (`mat3` -> `vec3`, `mat4x2` -> `vec2`).
fn column_type(typ: &str) -> &'static str {
    matrix_shape(typ).expect("column_type is only reached for a type es100_loc called a matrix").1
}

/// A matrix vertex attribute that was split into per-column inputs, to be
/// reassembled at the top of `main()`.
struct MatAttrib {
    name: String,
    typ: String,
    columns: i64,
}

/// Samplers emulated through a stacked 2D texture (see EMU_HELPERS).
///
/// A `samplerCube` is no longer one (native cube step C, 2026-09-25): pco
/// samples it as the TPU's own cube (`smp` 3d) and the driver binds a
/// twiddled face-major copy (gl::native_cube), which the AM62P measured
/// against the stacked image over dEQP's 333 cube cases (am62p-board,
/// 9548e106: 323/10 with no case failing that the stacked image passes).
/// The cube helpers stay in EMU_HELPERS for the online compiler's stacked
/// path (`pvr_native_cube` off).
fn emu_type(t: &str) -> Option<&'static str> {
    match t {
        "sampler2DArray" => Some("array"),
        "sampler3D" => Some("vol"),
        _ => None,
    }
}

pub fn strip_comments(text: &str) -> String {
    let t = py::sub_lit(&re(r"(?s)/\*.*?\*/"), text, " ");
    py::sub_lit(&re(r"//[^\n]*"), &t, " ")
}

/// Comment-stripped source, which must be ASCII (see the module doc).
pub fn code_text(text: &str) -> R<String> {
    let t = strip_comments(text);
    if !t.is_ascii() {
        exit!("glsl2usc: non-ASCII characters outside comments are not supported (GLSL's character set is ASCII)");
    }
    Ok(t)
}

/// 100 for GLSL ES 1.00 (or version-less legacy text), 300/310 for GLSL ES
/// 3.x, None for anything the translator must leave alone.
pub fn es_version(text: &str) -> R<Option<i64>> {
    let t = code_text(text)?;
    if let Some(m) = re(r"(?m)^\s*#\s*version\s+(\d+)(\s+es)?").captures(&t) {
        if g(&m, 1) == "100" {
            return Ok(Some(100));
        }
        if grp(&m, 2).is_some() && (g(&m, 1) == "300" || g(&m, 1) == "310") {
            return Ok(Some(pint(g(&m, 1))?));
        }
        return Ok(None);
    }
    /* No #version at all is GLSL ES 1.00. The ES 1.00 specification says so
     * (section 3.3: a shader without a #version directive targets version
     * 100), and glslang agrees — its `-d` flag exists precisely to make a
     * version-less file desktop GLSL 110 instead of ES 100.
     *
     * The old test, "does the text contain attribute/varying/gl_FragColor",
     * is a guess that a version-less ES shader need not satisfy: a vertex
     * shader that writes gl_Position from constants contains none of those
     * words, was left untranslated, and was then handed to
     * `glslangValidator -V` as desktop GLSL, which rejects it. dEQP's
     * shaders.preprocessor.pragmas cases are exactly that shape. */
    let _ = t;
    Ok(Some(100))
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Kind {
    Pp,
    Block,
    Stmt,
}

/// Split comment-stripped source into (is_toplevel, kind, text) chunks at ';'
/// and '}' boundaries outside braces; preprocessor lines are their own chunks.
fn es100_statements(text: &str) -> Vec<(bool, Kind, String)> {
    let b = text.as_bytes();
    let mut out = Vec::new();
    let (mut i, mut bstart, mut depth) = (0usize, 0usize, 0i64);
    while i < b.len() {
        let c = b[i];
        if c == b'#' && strip(&text[bstart..i]).is_empty() {
            let j = match text[i..].find('\n') {
                Some(p) => i + p + 1,
                None => b.len(),
            };
            // A preprocessor directive must start a line: the preceding
            // statement chunk ends at its ';'/'}' without a newline, and the
            // whitespace in the buffer (the separating newline) is dropped.
            out.push((depth == 0, Kind::Pp, format!("\n{}", &text[i..j])));
            bstart = j;
            i = j;
            continue;
        }
        match c {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    out.push((true, Kind::Block, text[bstart..=i].to_string()));
                    bstart = i + 1;
                }
            }
            b';' if depth == 0 => {
                out.push((true, Kind::Stmt, text[bstart..=i].to_string()));
                bstart = i + 1;
            }
            _ => {}
        }
        i += 1;
    }
    if !strip(&text[bstart..]).is_empty() {
        out.push((depth == 0, Kind::Stmt, text[bstart..].to_string()));
    }
    out
}

/// Object-like `#define NAME <int>` macros (array sizes). Runs on text that
/// still has its comments, hence Python's `\w` spelled out.
pub fn scan_defines(text: &str) -> R<HashMap<String, i64>> {
    let pat = format!(r"(?m)^\s*#\s*define\s+({w}+)\s+\(?\s*(\d+)\s*\)?\s*$", w = py::PW);
    let mut out = HashMap::new();
    for m in re(&pat).captures_iter(text) {
        out.insert(g(&m, 1).to_string(), pint(g(&m, 2))?);
    }
    Ok(out)
}

/// An array size: an integer constant expression (GLSL ES 1.00 section 4.3.3).
///
/// A literal and a `#define`d name were enough while the translation ran
/// before the preprocessor. It runs after now, so a macro is already expanded
/// by the time this sees it — and what arrives is whatever the application
/// wrote around it: dEQP's light shader declares
/// `uniform Light u_lights[${NUM_DIR}+${NUM_OMNI}]`, which preprocesses to
/// `u_lights[2+1]`. So the size has to be evaluated, not looked up.
///
/// `+ - * / %`, parentheses and unary minus over integer literals, with a
/// `#define`d name still accepted for a caller that has not preprocessed.
fn resolve_size(tok: &str, defines: &HashMap<String, i64>) -> R<i64> {
    let mut p = ConstExpr { s: tok.as_bytes(), i: 0, defines };
    let v = p.sum()?;
    p.skip_ws();
    if p.i != p.s.len() {
        exit!("glsl2usc: array size '{tok}' is not an integer constant expression");
    }
    if v <= 0 {
        exit!("glsl2usc: array size '{tok}' evaluates to {v}, which is not a positive integer");
    }
    Ok(v)
}

/// The recursive-descent half of [`resolve_size`].
struct ConstExpr<'a> {
    s: &'a [u8],
    i: usize,
    defines: &'a HashMap<String, i64>,
}

impl ConstExpr<'_> {
    fn skip_ws(&mut self) {
        while self.i < self.s.len() && (self.s[self.i] as char).is_whitespace() {
            self.i += 1;
        }
    }

    fn eat(&mut self, c: u8) -> bool {
        self.skip_ws();
        if self.i < self.s.len() && self.s[self.i] == c {
            self.i += 1;
            return true;
        }
        false
    }

    /// `term (('+' | '-') term)*`
    fn sum(&mut self) -> R<i64> {
        let mut v = self.term()?;
        loop {
            if self.eat(b'+') {
                v = v.wrapping_add(self.term()?);
            } else if self.eat(b'-') {
                v = v.wrapping_sub(self.term()?);
            } else {
                return Ok(v);
            }
        }
    }

    /// `unary (('*' | '/' | '%') unary)*`
    fn term(&mut self) -> R<i64> {
        let mut v = self.unary()?;
        loop {
            if self.eat(b'*') {
                v = v.wrapping_mul(self.unary()?);
            } else if self.eat(b'/') {
                let d = self.unary()?;
                if d == 0 {
                    exit!("glsl2usc: division by zero in an array size");
                }
                v = v.wrapping_div(d);
            } else if self.eat(b'%') {
                let d = self.unary()?;
                if d == 0 {
                    exit!("glsl2usc: division by zero in an array size");
                }
                v = v.wrapping_rem(d);
            } else {
                return Ok(v);
            }
        }
    }

    fn unary(&mut self) -> R<i64> {
        if self.eat(b'-') {
            return Ok(-self.unary()?);
        }
        if self.eat(b'+') {
            return self.unary();
        }
        self.atom()
    }

    fn atom(&mut self) -> R<i64> {
        self.skip_ws();
        if self.eat(b'(') {
            let v = self.sum()?;
            if !self.eat(b')') {
                exit!("glsl2usc: unbalanced parentheses in an array size");
            }
            return Ok(v);
        }
        let start = self.i;
        while self.i < self.s.len() && (self.s[self.i] as char).is_ascii_digit() {
            self.i += 1;
        }
        if self.i > start {
            /* `10u` is an unsigned literal in ES 3.00 */
            if self.i < self.s.len() && matches!(self.s[self.i], b'u' | b'U') {
                self.i += 1;
                return pint(std::str::from_utf8(&self.s[start..self.i - 1]).unwrap_or(""));
            }
            return pint(std::str::from_utf8(&self.s[start..self.i]).unwrap_or(""));
        }
        while self.i < self.s.len() && ((self.s[self.i] as char).is_alphanumeric() || self.s[self.i] == b'_') {
            self.i += 1;
        }
        if self.i > start {
            let name = std::str::from_utf8(&self.s[start..self.i]).unwrap_or("");
            if let Some(v) = self.defines.get(name) {
                return Ok(*v);
            }
            exit!("glsl2usc: array size '{name}' is not an integer literal or a #define'd integer");
        }
        exit!("glsl2usc: an array size is missing where one is required")
    }
}

/// Assemble each split matrix attribute from its columns at the top of
/// `main()`.
///
/// It goes after a `gl_Position = ...;` that already sits at the top, because
/// pco_shc hands out vtxin registers in order of first use and the position
/// attribute has to be vtxin0 ([`hoist_gl_position`] exists for that reason
/// alone) — unless that statement is itself what reads the matrix, in which
/// case the assembly has to come first and the position is not the first
/// attribute read whatever we do.
fn assemble_matrix_attribs(text: &str, mats: &[MatAttrib]) -> String {
    if mats.is_empty() {
        return text.to_string();
    }
    let Some(mm) = re(r"\bvoid\s+main\s*\([^)]*\)\s*\{").find(text) else {
        return text.to_string();
    };
    let mut at = mm.end();
    let stmt: String = mats
        .iter()
        .map(|m| {
            let cols: Vec<String> = (0..m.columns).map(|c| format!("{MAT_ATTR_PREFIX}{}_col{c}", m.name)).collect();
            format!("\n  {} = {}({});", m.name, m.typ, cols.join(", "))
        })
        .collect();
    /* Step over a leading gl_Position assignment that does not read one of
     * the matrices. */
    if let Some(gm) = re(r"\A[ \t\n]*gl_Position\s*=\s*[^;{}]*;").find(&text[at..]) {
        let s = gm.as_str();
        if !mats.iter().any(|m| re(&format!(r"\b{}\b", esc(&m.name))).is_match(s)) {
            at += gm.end();
        }
    }
    format!("{}{}{}", &text[..at], stmt, &text[at..])
}

/// Put a function body in a block of its own when it re-declares one of the
/// function's own parameters.
///
/// GLSL ES 1.00 lets a local variable shadow a parameter -- the parameters and
/// the body's outermost declarations are different scopes -- and GLSL 4.50
/// does not: there they are one scope, and the second declaration is a
/// redefinition. So an ES shader that is legal becomes an error purely by
/// being rewritten (`dEQP-GLES2.functional.shaders.scoping.valid.
/// local_variable_hides_function_parameter`).
///
/// One more pair of braces restores the two scopes, and changes nothing else:
/// a function body is self-contained, so nesting it is semantically invisible.
/// It is applied only to a function that actually shadows a parameter, so no
/// other shader's text -- and no other row of the offline table -- moves.
fn rescope_shadowed_params(chunk: &str) -> String {
    let Some(m) = re(r"(?s)\A(\s*\w[\w\s]*\w\s*\(([^)]*)\)\s*\{)(.*)(\}\s*)\z").captures(chunk) else {
        return chunk.to_string();
    };
    let params = g(&m, 2);
    let body = g(&m, 3);
    /* `in lowp vec3 name`, `name[4]`, `void`: the last identifier of each
     * comma-separated parameter is its name. */
    let shadowed = params.split(',').any(|prm| {
        let Some(name) = re(r"(\w+)\s*(\[[^\]]*\])?\s*\z").captures(strip(prm)).map(|c| g(&c, 1).to_string()) else {
            return false;
        };
        if name == "void" {
            return false;
        }
        /* A declaration of that name in the body: a type token, the name,
         * then one of the characters that can follow a declarator. The word
         * before the name has to be a type and not a keyword that introduces
         * a statement -- `return x;` has exactly the shape of `float x;` and
         * is not one. */
        re(&format!(r"\b(\w+)\s+{}\s*[=;,\[]", esc(&name)))
            .captures_iter(body)
            .any(|c| !matches!(g(&c, 1), "return" | "else" | "do" | "case" | "default"))
    });
    if !shadowed {
        return chunk.to_string();
    }
    format!("{}{{{}}}{}", g(&m, 1), body, g(&m, 4))
}

/// Move the `gl_Position = ...;` statement to the front of main() when a
/// non-position attribute is used before it (pco_shc hands out vtxin
/// registers in order of first use; the position must be vtxin0). See the
/// Python original's docstring for the full rationale.
fn hoist_gl_position(text: &str, attrib_names: &[String]) -> String {
    let Some(mm) = re(r"\bvoid\s+main\s*\([^)]*\)\s*\{").find(text) else {
        return text.to_string();
    };
    let body_start = mm.end();
    let tail = &text[body_start..];
    let Some(gm) = re(r"[ \t]*gl_Position\s*=\s*[^;{}]*;[ \t]*\n?").find(tail) else {
        return text.to_string();
    };
    let stmt = gm.as_str();
    let gp_start = body_start + gm.start();
    let gp_end = body_start + gm.end();
    let before = &text[body_start..gp_start];
    if before.contains("gl_Position") {
        return text.to_string();
    }
    let rhs = re(r"gl_Position\s*=\s*([^;{}]*);").captures(stmt).map(|c| g(&c, 1).to_string()).unwrap_or_default();
    let used = |s: &str| -> HashSet<&str> {
        attrib_names.iter().filter(|n| re(&format!(r"\b{}\b", esc(n))).is_match(s)).map(|n| n.as_str()).collect()
    };
    let attrs_before = used(before);
    let attrs_rhs = used(&rhs);
    if attrs_before.difference(&attrs_rhs).next().is_none() {
        return text.to_string();
    }
    let assigned: HashSet<&str> = re(r"\b([A-Za-z_]\w*)\s*=").captures_iter(before).map(|c| g(&c, 1)).collect();
    let rhs_ids: HashSet<&str> = re(r"\b([A-Za-z_]\w*)\b").captures_iter(&rhs).map(|c| g(&c, 1)).collect();
    if assigned.intersection(&rhs_ids).next().is_some() {
        return text.to_string();
    }
    format!("{}{}{}{}", &text[..body_start], stmt, before, &text[gp_end..])
}

/// Helpers for cube maps and 2D texture arrays sampled through a stacked 2D
/// texture (faces / layers stacked vertically; the layer count arrives as the
/// hidden uniform pvr_layers_<sampler>).
///
/// The online PowerVR compiler transcribes these one operation for one
/// (`Sel::cube_uv`, `Sel::array_uv`), and the `pvr_layers_<sampler>` name and
/// position are a contract both compilers and the GL core share: see
/// docs/online-shaderc/usc-seam.md, "Cube maps, 2D arrays and 3D textures".
/// A change here changes the online path too.
pub const EMU_HELPERS: &str = "
vec2 pvr_arrUV(sampler2D s, float layers, vec3 c) {
\tfloat l = clamp(floor(c.z + 0.5), 0.0, layers - 1.0);
\tfloat h = float(textureSize(s, 0).y) / layers;
\tfloat v = clamp(c.y, 0.5 / h, 1.0 - 0.5 / h);
\treturn vec2(c.x, (v + l) / layers);
}
vec4 pvr_arr(sampler2D s, float layers, vec3 c) { return texture(s, pvr_arrUV(s, layers, c)); }
vec4 pvr_arrLod(sampler2D s, float layers, vec3 c, float lod) { return textureLod(s, pvr_arrUV(s, layers, c), lod); }
vec4 pvr_arrFetch(sampler2D s, float layers, ivec3 c, int lod) {
\tint h = textureSize(s, lod).y / int(layers);
\treturn texelFetch(s, ivec2(c.x, c.y + c.z * h), lod);
}
ivec3 pvr_arrSize(sampler2D s, float layers, int lod) {
\tivec2 sz = textureSize(s, lod);
\treturn ivec3(sz.x, sz.y / int(layers), int(layers));
}
vec2 pvr_cubeUV(sampler2D s, vec3 d) {
\tvec3 a = abs(d);
\tfloat face, ma;
\tvec2 uv;
\tif (a.x >= a.y && a.x >= a.z) {
\t\tma = a.x;
\t\tif (d.x > 0.0) { face = 0.0; uv = vec2(-d.z, -d.y); } else { face = 1.0; uv = vec2(d.z, -d.y); }
\t} else if (a.y >= a.z) {
\t\tma = a.y;
\t\tif (d.y > 0.0) { face = 2.0; uv = vec2(d.x, d.z); } else { face = 3.0; uv = vec2(d.x, -d.z); }
\t} else {
\t\tma = a.z;
\t\tif (d.z > 0.0) { face = 4.0; uv = vec2(d.x, -d.y); } else { face = 5.0; uv = vec2(-d.x, -d.y); }
\t}
\tuv = (uv / ma) * 0.5 + 0.5;
\tfloat h = float(textureSize(s, 0).y) / 6.0;
\tuv.y = clamp(uv.y, 0.5 / h, 1.0 - 0.5 / h);
\treturn vec2(uv.x, (uv.y + face) / 6.0);
}
vec2 pvr_cubeUVLod(sampler2D s, vec3 d, float lod) {
\tvec3 a = abs(d);
\tfloat face, ma;
\tvec2 uv;
\tif (a.x >= a.y && a.x >= a.z) {
\t\tma = a.x;
\t\tif (d.x > 0.0) { face = 0.0; uv = vec2(-d.z, -d.y); } else { face = 1.0; uv = vec2(d.z, -d.y); }
\t} else if (a.y >= a.z) {
\t\tma = a.y;
\t\tif (d.y > 0.0) { face = 2.0; uv = vec2(d.x, d.z); } else { face = 3.0; uv = vec2(d.x, -d.z); }
\t} else {
\t\tma = a.z;
\t\tif (d.z > 0.0) { face = 4.0; uv = vec2(d.x, -d.y); } else { face = 5.0; uv = vec2(-d.x, -d.y); }
\t}
\tuv = (uv / ma) * 0.5 + 0.5;
\tfloat h = float(textureSize(s, 0).y) / 6.0;
\th = max(h * exp2(-ceil(max(lod, 0.0))), 1.0);
\tuv.y = clamp(uv.y, 0.5 / h, 1.0 - 0.5 / h);
\treturn vec2(uv.x, (uv.y + face) / 6.0);
}
vec4 pvr_cube(sampler2D s, vec3 d) { return texture(s, pvr_cubeUV(s, d)); }
vec4 pvr_cubeLod(sampler2D s, vec3 d, float lod) { return textureLod(s, pvr_cubeUVLod(s, d, lod), lod); }
ivec2 pvr_cubeSize(sampler2D s, int lod) { ivec2 sz = textureSize(s, lod); return ivec2(sz.x, sz.y / 6); }
vec2 pvr_volUV(sampler2D s, float layers, vec2 uv, float k) {
	float cols = ceil(sqrt(layers - 0.5));
	float rows = ceil(layers / cols);
	vec2 sz = vec2(textureSize(s, 0)) / vec2(cols, rows);
	vec2 st = clamp(uv, 0.5 / sz, 1.0 - 0.5 / sz);
	return (vec2(mod(k, cols), floor(k / cols)) + st) / vec2(cols, rows);
}
vec4 pvr_vol(sampler2D s, float layers, vec3 c) {
	float z = clamp(c.z * layers - 0.5, 0.0, layers - 1.0);
	float z0 = floor(z);
	float z1 = min(z0 + 1.0, layers - 1.0);
	return mix(texture(s, pvr_volUV(s, layers, c.xy, z0)), texture(s, pvr_volUV(s, layers, c.xy, z1)), z - z0);
}
vec4 pvr_volLod(sampler2D s, float layers, vec3 c, float lod) {
	float z = clamp(c.z * layers - 0.5, 0.0, layers - 1.0);
	float z0 = floor(z);
	float z1 = min(z0 + 1.0, layers - 1.0);
	return mix(textureLod(s, pvr_volUV(s, layers, c.xy, z0), 0.0), textureLod(s, pvr_volUV(s, layers, c.xy, z1), 0.0), z - z0);
}
vec4 pvr_volFetch(sampler2D s, float layers, ivec3 c, int lod) {
	int cols = int(ceil(sqrt(layers - 0.5)));
	int rows = (int(layers) + cols - 1) / cols;
	ivec2 sz = textureSize(s, 0) / ivec2(cols, rows);
	return texelFetch(s, c.xy + ivec2(c.z % cols, c.z / cols) * sz, 0);
}
ivec3 pvr_volSize(sampler2D s, float layers, int lod) {
	int cols = int(ceil(sqrt(layers - 0.5)));
	int rows = (int(layers) + cols - 1) / cols;
	ivec2 sz = textureSize(s, 0) / ivec2(cols, rows);
	return ivec3(sz, int(layers));
}
";

/// Python list indexing (negative indices count from the end).
fn py_index(len: usize, k: i64) -> R<usize> {
    let i = if k < 0 { k + len as i64 } else { k };
    if i < 0 || i >= len as i64 {
        return Err(Error::Fatal("list index out of range".into()));
    }
    Ok(i as usize)
}

/// `list.insert(k, x)` position (clamped like Python's).
fn py_insert_pos(len: usize, k: i64) -> usize {
    let i = if k < 0 { (k + len as i64).max(0) } else { k };
    (i as usize).min(len)
}

struct Emu {
    cube: HashSet<String>,
    arr: HashSet<String>,
    /// sampler3D: an array (slice count) sampled with a z blend
    vol: HashSet<String>,
    /// user function -> argument indices of its array samplers (insertion order)
    arr_param_fns: Vec<(String, Vec<i64>)>,
}

impl Emu {
    fn fn_ks(&self, name: &str) -> Option<&Vec<i64>> {
        self.arr_param_fns.iter().find(|(n, _)| n == name).map(|(_, v)| v)
    }

    fn rewrite_calls(&self, text: &str) -> R<String> {
        let pat = if self.arr_param_fns.is_empty() {
            r"\b(texture|textureLod|texelFetch|textureSize)\s*\(".to_string()
        } else {
            let names: Vec<String> = self.arr_param_fns.iter().map(|(n, _)| esc(n)).collect();
            format!(r"\b(texture|textureLod|texelFetch|textureSize|{})\s*\(", names.join("|"))
        };
        let b = text.as_bytes();
        let mut out = String::new();
        let mut pos = 0usize;
        for cm in re(&pat).captures_iter(text) {
            let whole = cm.get(0).unwrap();
            let fname = g(&cm, 1);
            // split the argument list at the top level
            let mut i = whole.end();
            let mut depth = 1i64;
            let mut args: Vec<String> = Vec::new();
            let mut start = i;
            while i < b.len() && depth != 0 {
                match b[i] {
                    b'(' => depth += 1,
                    b')' => {
                        depth -= 1;
                        if depth == 0 {
                            args.push(text[start..i].to_string());
                            break;
                        }
                    }
                    b',' if depth == 1 => {
                        args.push(text[start..i].to_string());
                        start = i + 1;
                    }
                    _ => {}
                }
                i += 1;
            }
            if depth != 0 {
                break;
            }
            let end = i + 1;
            if whole.start() < pos {
                continue; // inside an argument list already rewritten
            }
            if re(r"\A\s*\{").is_match(&text[end..]) {
                continue; // a function definition, not a call
            }
            let mut args: Vec<String> = args.iter().map(|a| self.rewrite_calls(a)).collect::<R<_>>()?;
            let first = args.first().map(|a| strip(a).to_string()).unwrap_or_default();
            let mut repl = None;
            if let Some(ks) = self.fn_ks(fname) {
                for &k in ks {
                    if k < args.len() as i64 {
                        let nm = strip(&args[py_index(args.len(), k)?]).to_string();
                        let at = py_insert_pos(args.len(), k + 1);
                        args.insert(at, format!(" pvr_layers_{nm}"));
                    }
                }
                repl = Some(format!("{}({})", fname, args.join(",")));
            } else if self.cube.contains(&first) {
                let joined = args.join(",");
                repl = Some(match fname {
                    "texture" => format!("pvr_cube({joined})"),
                    "textureLod" => format!("pvr_cubeLod({joined})"),
                    "textureSize" => format!("pvr_cubeSize({joined})"),
                    _ => exit!("glsl2usc: {fname}() on a cube map is not supported"),
                });
            } else if self.vol.contains(&first) {
                let mut v = vec![args[0].clone(), format!(" pvr_layers_{first}")];
                v.extend(args[1..].iter().cloned());
                let rest = v.join(",");
                repl = Some(match fname {
                    "texture" => format!("pvr_vol({rest})"),
                    "textureLod" => format!("pvr_volLod({rest})"),
                    "texelFetch" => format!("pvr_volFetch({rest})"),
                    "textureSize" => format!("pvr_volSize({rest})"),
                    _ => exit!("glsl2usc: {fname}() on a 3D texture is not supported"),
                });
            } else if self.arr.contains(&first) {
                let layers = format!(" pvr_layers_{first}");
                let mut v = vec![args[0].clone(), layers];
                v.extend(args[1..].iter().cloned());
                let rest = v.join(",");
                repl = Some(match fname {
                    "texture" => format!("pvr_arr({rest})"),
                    "textureLod" => format!("pvr_arrLod({rest})"),
                    "texelFetch" => format!("pvr_arrFetch({rest})"),
                    "textureSize" => format!("pvr_arrSize({rest})"),
                    _ => exit!("glsl2usc: {fname}() on a texture array is not supported"),
                });
            }
            let repl = repl.unwrap_or_else(|| format!("{}({})", fname, args.join(",")));
            out.push_str(&text[pos..whole.start()]);
            out.push_str(&repl);
            pos = end;
        }
        out.push_str(&text[pos..]);
        Ok(out)
    }
}

/// Rewrite the builtin sampling calls on emulated samplers (globals and
/// function parameters) into the helpers, and the parameter declarations
/// into sampler2D. Returns the new body and whether anything is emulated.
fn emulate_samplers(
    body: &str,
    cube_names: &[String],
    array_names: &[String],
    vol_names: &[String],
) -> R<(String, bool)> {
    let mut emu = Emu {
        cube: cube_names.iter().cloned().collect(),
        arr: array_names.iter().cloned().collect(),
        vol: vol_names.iter().cloned().collect(),
        arr_param_fns: Vec::new(),
    };
    // function parameters of an emulated type: the parameter is a cube/array name too
    /* not samplerCube: a cube is the TPU's own now (`emu_type`) */
    let pat = r"\b(?:(highp|mediump|lowp)\s+)?(sampler2DArray|sampler3D)\s+(\w+)\b";
    let body = py::sub_fn(&re(pat), body, |m| {
        let (typ, name) = (g(m, 2), g(m, 3));
        if emu_type(typ) == Some("cube") {
            emu.cube.insert(name.to_string());
            return Ok(format!("sampler2D {name}"));
        }
        emu.arr.insert(name.to_string());
        if emu_type(typ) == Some("vol") {
            emu.vol.insert(name.to_string());
        }
        Ok(format!("sampler2D {name}, float pvr_layers_{name}"))
    })?;
    if emu.cube.is_empty() && emu.arr.is_empty() {
        return Ok((body, false));
    }
    // calls of user functions that take an array sampler must pass the layer
    // count too; find such functions from their definitions
    for fm in re(r"\b(\w+)\s+(\w+)\s*\(([^)]*)\)\s*\{").captures_iter(&body) {
        let params: Vec<&str> = g(&fm, 3).split(',').map(strip).filter(|p| !p.is_empty()).collect();
        let idx: Vec<i64> = params
            .iter()
            .enumerate()
            .filter(|(_, p)| re(r"\Afloat\s+pvr_layers_\w+\z").is_match(p))
            .map(|(i, _)| i as i64)
            .collect();
        if !idx.is_empty() {
            // the sampler2D parameter precedes its layers parameter
            let ks: Vec<i64> = idx.iter().enumerate().map(|(k, &i)| i - 1 - k as i64).collect();
            let name = g(&fm, 2).to_string();
            match emu.arr_param_fns.iter_mut().find(|(n, _)| *n == name) {
                Some(e) => e.1 = ks,
                None => emu.arr_param_fns.push((name, ks)),
            }
        }
    }
    Ok((emu.rewrite_calls(&body)?, true))
}

/// The name GL reflection reports for a push-constant member the translation
/// renamed (glGetUniformLocation).
#[derive(Clone, Debug, PartialEq)]
pub enum GlName {
    Name(String),
    /// A flattened array-of-structs member: the GL name and the elements per
    /// outer index.
    Inner(String, i64),
}

#[derive(Default, Debug)]
pub struct GlNames {
    /// flattened member -> GL name (insertion order)
    pub members: Vec<(String, GlName)>,
    /// emulated sampler -> the GL type it reports
    pub samplers: Vec<(String, i64)>,
}

impl GlNames {
    fn set_member(&mut self, k: String, v: GlName) {
        match self.members.iter_mut().find(|(n, _)| *n == k) {
            Some(e) => e.1 = v,
            None => self.members.push((k, v)),
        }
    }
    fn set_sampler(&mut self, k: String, v: i64) {
        match self.samplers.iter_mut().find(|(n, _)| *n == k) {
            Some(e) => e.1 = v,
            None => self.samplers.push((k, v)),
        }
    }
    fn member(&self, k: &str) -> Option<&GlName> {
        self.members.iter().find(|(n, _)| n == k).map(|(_, v)| v)
    }
    fn sampler(&self, k: &str) -> Option<i64> {
        self.samplers.iter().find(|(n, _)| n == k).map(|(_, v)| *v)
    }
}

/// Is `name` unused (not in main()'s text and declared only once)?
fn unused_sampler(name: &str, src: &str) -> bool {
    let r = re(&format!(r"\b{}\b", esc(name)));
    let after_main = match src.find("void main") {
        Some(i) => &src[i + "void main".len()..],
        None => "",
    };
    !r.is_match(after_main) && r.find_iter(src).count() <= 1
}

struct Member {
    typ: String,
    name: String,
    arr: String,
}

struct Translator<'a> {
    defines: &'a HashMap<String, i64>,
    structs: &'a HashMap<String, Vec<Member>>,
    uniforms: Vec<String>,
    gl_names: GlNames,
    renames: Vec<(String, String, String)>,
    renames_arr: Vec<(String, String, String, String, i64, bool)>,
}

impl Translator<'_> {
    /// Members are expanded depth first; a member of struct type contributes
    /// its own members as instance_member_sub.
    fn expand(&mut self, path_gl: &str, path_flat: &str, stype: &str, depth: usize) -> R<()> {
        if depth > 64 {
            return Err(Error::Fatal(format!("recursive struct type '{stype}'")));
        }
        let structs = self.structs;
        for m in &structs[stype] {
            let (mtype, mname, marr) = (m.typ.as_str(), m.name.as_str(), m.arr.as_str());
            if structs.contains_key(mtype) {
                if !marr.is_empty() {
                    // An array of structs: each element member becomes its own
                    // array path_member_sub[N*M], addressed as
                    // path.member[i].sub[j] -> flat[(i)*M+(j)].
                    let n = resolve_size(&py::sub_lit(&re(r"[\[\]\s]"), marr, ""), self.defines)?;
                    for s in &structs[mtype] {
                        let (stype2, sname, sarr) = (s.typ.as_str(), s.name.as_str(), s.arr.as_str());
                        if structs.contains_key(stype2) {
                            exit!("glsl2usc: struct inside an array of structs ('{path_gl}.{mname}[].{sname}') is not supported");
                        }
                        if glsl_type(stype2).is_none() {
                            exit!(
                                "glsl2usc: array-of-structs member '{path_gl}.{mname}[].{sname}' has type '{stype2}'"
                            );
                        }
                        let m_inner = if !sarr.is_empty() {
                            resolve_size(&py::sub_lit(&re(r"[\[\]\s]"), sarr, ""), self.defines)?
                        } else {
                            1
                        };
                        let flat = format!("{path_flat}_{mname}_{sname}");
                        self.uniforms.push(format!("\t{} {}[{}];", stype2, flat, n * m_inner));
                        self.gl_names
                            .set_member(flat.clone(), GlName::Inner(format!("{path_gl}.{mname}[0].{sname}"), m_inner));
                        self.renames_arr.push((
                            path_gl.to_string(),
                            mname.to_string(),
                            sname.to_string(),
                            flat,
                            m_inner,
                            !sarr.is_empty(),
                        ));
                    }
                    continue;
                }
                self.expand(&format!("{path_gl}.{mname}"), &format!("{path_flat}_{mname}"), mtype, depth + 1)?;
                continue;
            }
            if glsl_type(mtype).is_none() {
                exit!(
                    "glsl2usc: uniform struct member '{path_gl}.{mname}' has type '{mtype}' (samplers in structs are not supported)"
                );
            }
            let flat = format!("{path_flat}_{mname}");
            let marr = if marr.is_empty() {
                String::new()
            } else {
                format!("[{}]", resolve_size(&py::sub_lit(&re(r"[\[\]\s]"), marr, ""), self.defines)?)
            };
            self.uniforms.push(format!("\t{mtype} {flat}{marr};"));
            self.gl_names.set_member(flat.clone(), GlName::Name(format!("{path_gl}.{mname}")));
            self.renames.push((path_gl.to_string(), mname.to_string(), flat));
        }
        Ok(())
    }
}

/// GLSL ES 1.00 / 3.00 -> GLSL 450 (Vulkan). Returns the new text and the GL
/// names of renamed push-constant members.
pub fn translate(text: &str, stage: &str, es: i64) -> R<(String, GlNames)> {
    let src = code_text(text)?;
    let defines = scan_defines(text)?;
    let decl = re(concat!(
        r"(?s)\A\s*(invariant\s+)?(?:layout\s*\(\s*location\s*=\s*(\d+)\s*\)\s*)?",
        r"(?:(flat|smooth|centroid|noperspective)\s+)?(attribute|varying|uniform|in|out)\s+",
        r"(?:(?:highp|mediump|lowp)\s+)?(\w+)\s+(.*?)\s*;\s*\z"
    ));
    let mut out: Vec<Option<String>> = Vec::new();
    let (mut attrib_loc, mut varying_loc, mut binding, mut fs_out_loc) = (0i64, 0i64, 0i64, 0i64);
    let mut attrib_names: Vec<String> = Vec::new();
    let mut mat_attribs: Vec<MatAttrib> = Vec::new();
    let mut block_at: Option<usize> = None;
    let mut structs: HashMap<String, Vec<Member>> = HashMap::new();
    let (mut cube_names, mut array_names, mut vol_names): (Vec<String>, Vec<String>, Vec<String>) =
        (Vec::new(), Vec::new(), Vec::new());
    let uses_fragcolor = re(r"\bgl_FragColor\b").is_match(&src);
    let uses_fragdata = re(r"\bgl_FragData\b").is_match(&src);
    let chunks = es100_statements(&src);

    // Struct definitions are collected while walking; the uniform expansion
    // needs the ones seen so far, so the walk keeps its own state.
    let mut uniforms: Vec<String> = Vec::new();
    let mut gl_names = GlNames::default();
    let mut renames = Vec::new();
    let mut renames_arr = Vec::new();

    for (top, kind, chunk) in &chunks {
        let (top, kind) = (*top, *kind);
        if kind == Kind::Pp {
            if re(r"\A\s*#\s*(version|extension)\b").is_match(chunk) {
                continue; // replaced by the 450 header / core features
            }
            out.push(Some(chunk.clone()));
            continue;
        }
        if top && kind == Kind::Block {
            if let Some(sm) = re(r"(?s)\A\s*struct\s+(\w+)\s*\{(.*)\}\s*\z").captures(chunk) {
                // A struct type (Qt's qsb writes each uniform block as
                // `struct buf { ... }; uniform buf _16;`).
                let members = re(r"(?:(?:highp|mediump|lowp)\s+)?(\w+)\s+(\w+)\s*(\[[^\]]*\])?\s*;")
                    .captures_iter(g(&sm, 2))
                    .map(|mm| Member {
                        typ: g(&mm, 1).to_string(),
                        name: g(&mm, 2).to_string(),
                        arr: grp(&mm, 3).unwrap_or("").to_string(),
                    })
                    .collect();
                structs.insert(g(&sm, 1).to_string(), members);
                out.push(Some(chunk.clone())); // keep the type; harmless when unused
                continue;
            }
            out.push(Some(rescope_shadowed_params(chunk)));
            continue;
        }
        if top && kind == Kind::Stmt && re(r"\A\s*precision\s+\w+\s+sampler").is_match(chunk) {
            continue; // sampler precision statements: ES only
        }
        let m = if top && kind == Kind::Stmt { decl.captures(chunk) } else { None };
        let Some(m) = m else {
            out.push(Some(chunk.clone()));
            continue;
        };
        let invariant = grp(&m, 1);
        let explicit_loc = grp(&m, 2);
        let mut interp = grp(&m, 3).map(str::to_string);
        let mut storage = g(&m, 4);
        let typ = g(&m, 5);
        let declarators = g(&m, 6);
        if storage == "in" {
            storage = if stage == "vs" { "attribute" } else { "varying" };
        } else if storage == "out" {
            if stage == "vs" {
                storage = "varying";
            } else {
                // ES 3.00 fragment output: already the Vulkan form, keep the
                // location (qsb writes one) or hand out the next one.
                for d in declarators.split(',').map(strip) {
                    let loc = match explicit_loc {
                        Some(l) => pint(l)?,
                        None => fs_out_loc,
                    };
                    out.push(Some(format!("layout(location = {loc}) out {typ} {d};\n")));
                    fs_out_loc = loc + 1;
                }
                continue;
            }
        }
        for d in declarators.split(',').map(strip) {
            /* `[...]` holds an integer constant expression, not just a word:
             * `u_v[1 + 0]`, and after preprocessing `u_lights[2+1]`. */
            let Some(dm) = re(r"\A(\w+)\s*(\[\s*([^\]]*?)\s*\])?\z").captures(d) else {
                exit!("glsl2usc: cannot parse GLSL ES declarator '{d}'");
            };
            let name = g(&dm, 1);
            let count = match grp(&dm, 3) {
                Some(t) => resolve_size(t, &defines)?,
                None => 1,
            };
            /* The subscript is re-emitted as the number it evaluates to, not
             * as the text the application wrote: `vec4 u_v[1 + 0];` is legal
             * GLSL 450 and would compile, but `reflect_uniforms` reads the
             * size back out of this text and sees no integer, so the uniform
             * would vanish from the reflection and from the block size. */
            let array_owned = grp(&dm, 2).map(|_| format!("[{count}]")).unwrap_or_default();
            let array = array_owned.as_str();
            if storage == "attribute" {
                let Some(locs) = es100_loc(typ) else {
                    exit!("glsl2usc: attribute type '{typ}' is not supported");
                };
                if let Some(l) = explicit_loc {
                    attrib_loc = pint(l)?;
                }
                if locs > 1 {
                    /* A matrix attribute, split into one vector input per
                     * column. GL already says a matN attribute occupies N
                     * consecutive locations, one per column, so the locations
                     * are the same either way; what changes is that pco_shc
                     * never sees a matrix input, which it cannot lower
                     * (`pco_nir_pvfio.c:909: lower_pvi: Assertion 'attrib'
                     * failed`) and which `struct shc_attrib` -- one location,
                     * one to four components -- could not describe.
                     *
                     * The matrix itself becomes a file-scope variable
                     * assembled at the top of main(). At file scope it is a
                     * declaration only: `matN x = matN(a, b);` there would be
                     * an initialiser that is not a constant expression, which
                     * GLSL forbids, and a local in main() would be invisible
                     * to a function declared above it. */
                    let col = column_type(typ);
                    for c in 0..locs {
                        out.push(Some(format!(
                            "layout(location = {}) in {col} {}{name}_col{c};\n",
                            attrib_loc + c,
                            MAT_ATTR_PREFIX
                        )));
                    }
                    out.push(Some(format!("{typ} {name};\n")));
                    mat_attribs.push(MatAttrib { name: name.to_string(), typ: typ.to_string(), columns: locs });
                } else {
                    out.push(Some(format!("layout(location = {attrib_loc}) in {typ} {name};\n")));
                }
                attrib_loc += locs * count;
                attrib_names.push(name.to_string());
            } else if storage == "varying" {
                let Some(locs) = es100_loc(typ) else {
                    exit!("glsl2usc: varying type '{typ}' is not supported");
                };
                let direction = if stage == "vs" { "out" } else { "in" };
                let inv = if invariant.is_some() && stage == "vs" { "invariant " } else { "" };
                // Integer varyings must be flat (SPIR-V rejects a smooth int).
                if typ.starts_with(['i', 'u', 'b']) && interp.is_none() {
                    interp = Some("flat".into());
                }
                let qual = interp.as_ref().map(|i| format!("{i} ")).unwrap_or_default();
                out.push(Some(format!(
                    "{inv}layout(location = {varying_loc}) {qual}{direction} {typ} {name}{array};\n"
                )));
                varying_loc += locs * count;
            } else if let Some(kind) = emu_type(typ) {
                if unused_sampler(name, &src) {
                    continue; // unused, as below
                }
                out.push(Some(format!("layout(set = 0, binding = {binding}) uniform sampler2D {name};\n")));
                binding += 1;
                gl_names.set_sampler(name.to_string(), sampler_type(typ).unwrap());
                if kind == "cube" {
                    cube_names.push(name.to_string());
                } else {
                    array_names.push(name.to_string());
                    if kind == "vol" {
                        vol_names.push(name.to_string());
                    }
                    if block_at.is_none() {
                        block_at = Some(out.len());
                        out.push(None);
                    }
                    uniforms.push(format!("\tfloat pvr_layers_{name};"));
                }
            } else if sampler_type(typ).is_some() {
                // A sampler the body never reads is dropped (Qt Quick 3D
                // declares its shadow map slot in shaders that never sample it).
                if unused_sampler(name, &src) {
                    continue;
                }
                out.push(Some(format!("layout(set = 0, binding = {binding}) uniform {typ} {name};\n")));
                binding += 1;
            } else if structs.contains_key(typ) {
                // Struct-typed uniform instance: its members become members of
                // the push-constant block, flattened as instance_member; the GL
                // name stays `instance.member`.
                if storage != "uniform" {
                    exit!("glsl2usc: {storage} of struct type '{typ}' is not supported");
                }
                if block_at.is_none() {
                    block_at = Some(out.len());
                    out.push(None);
                }
                if !array.is_empty() {
                    /* `uniform Light u_lights[N];` -- one flattened array per
                     * member, `u_lights[i].position` -> `u_lights_position[i]`.
                     * The same shape the expander already gives a *member*
                     * that is an array of structs, one level up. */
                    for m in &structs[typ] {
                        let (mtype, mname, marr) = (m.typ.as_str(), m.name.as_str(), m.arr.as_str());
                        if structs.contains_key(mtype) {
                            exit!("glsl2usc: struct inside an array of structs ('{name}[].{mname}') is not supported");
                        }
                        if glsl_type(mtype).is_none() {
                            exit!("glsl2usc: array-of-structs member '{name}[].{mname}' has type '{mtype}'");
                        }
                        let m_inner = if marr.is_empty() {
                            1
                        } else {
                            resolve_size(&py::sub_lit(&re(r"[\[\]\s]"), marr, ""), &defines)?
                        };
                        let flat = format!("{name}_{mname}");
                        uniforms.push(format!("\t{} {}[{}];", mtype, flat, count * m_inner));
                        gl_names.set_member(flat.clone(), GlName::Inner(format!("{name}[0].{mname}"), m_inner));
                        renames_arr.push((
                            String::new(), /* no enclosing instance: the array IS the uniform */
                            name.to_string(),
                            mname.to_string(),
                            flat,
                            m_inner,
                            !marr.is_empty(),
                        ));
                    }
                    continue;
                }
                if block_at.is_none() {
                    block_at = Some(out.len());
                    out.push(None);
                }
                let mut t = Translator {
                    defines: &defines,
                    structs: &structs,
                    uniforms: std::mem::take(&mut uniforms),
                    gl_names: std::mem::take(&mut gl_names),
                    renames: std::mem::take(&mut renames),
                    renames_arr: std::mem::take(&mut renames_arr),
                };
                let r = t.expand(name, name, typ, 0);
                uniforms = t.uniforms;
                gl_names = t.gl_names;
                renames = t.renames;
                renames_arr = t.renames_arr;
                r?;
            } else {
                if glsl_type(typ).is_none() {
                    exit!("glsl2usc: uniform type '{typ}' is not supported (structs are not)");
                }
                if block_at.is_none() {
                    block_at = Some(out.len());
                    out.push(None);
                }
                uniforms.push(format!("\t{typ} {name}{array};"));
            }
        }
    }
    if let Some(at) = block_at {
        out[at] = Some(format!("layout(push_constant) uniform pvr_U {{\n{}\n}};\n", uniforms.join("\n")));
    }
    // SPIRV-Cross guards gl_BaseInstanceARB with GL_ARB_shader_draw_parameters,
    // which desktop GLSL 450 has; take its other branch like the ES 3.00 compile.
    let body_pp: String = out.into_iter().map(|c| c.unwrap_or_default()).collect();
    let mut body = py::sub_lit(&re(r"#\s*ifdef\s+GL_ARB_shader_draw_parameters\b"), &body_pp, "#if 0");
    let mut header = String::from("#version 450\n");
    if stage == "fs" {
        if uses_fragcolor {
            header.push_str("layout(location = 0) out vec4 pvr_FragColor;\n#define gl_FragColor pvr_FragColor\n");
        }
        if uses_fragdata {
            header.push_str("layout(location = 0) out vec4 pvr_FragData[1];\n#define gl_FragData pvr_FragData\n");
        }
    }
    header.push_str(
        "#define texture2D texture\n#define texture2DProj textureProj\n\
         #define texture2DLod textureLod\n#define texture2DProjLod textureProjLod\n",
    );
    if es >= 300 {
        header.push_str("#define gl_VertexID gl_VertexIndex\n#define gl_InstanceID gl_InstanceIndex\n");
    }
    let mut array_sizes: HashMap<String, i64> = HashMap::new();
    for u in &uniforms {
        if let Some(um) = re(r"\A\s*\w+\s+(\w+)\[(\d+)\];").captures(u) {
            array_sizes.insert(g(&um, 1).to_string(), pint(g(&um, 2))?);
        }
    }
    // arrays of structs first: `a.b[i].c[j]` -> flat[(i)*M+(j)], `a.b[i].c` -> flat[i]
    for (inst, member, sub, flat, m_inner, sub_is_arr) in &renames_arr {
        /* `inst` empty: the array of structs is the uniform itself
         * (`u_lights[i].position`), not a member of one
         * (`u_mat.lights[i].position`). */
        let prefix = if inst.is_empty() {
            String::new()
        } else {
            let head: Vec<String> = inst.split('.').map(esc).collect();
            format!(r"{}\s*\.\s*", head.join(r"\s*\.\s*"))
        };
        let pattern = format!(r"\b{}{}\s*\[([^\]]+)\]\s*\.\s*{}\b(\s*\[([^\]]+)\])?", prefix, esc(member), esc(sub));
        body = py::sub_fn(&re(&pattern), &body, |mm| {
            let outer = strip(g(mm, 1));
            match grp(mm, 3) {
                Some(j) if !sub_is_arr => Ok(format!("{flat}[{outer}][{}]", strip(j))),
                Some(j) => Ok(format!("{flat}[({outer})*{m_inner}+({})]", strip(j))),
                None => {
                    if *m_inner > 1 {
                        exit!("glsl2usc: whole inner array '{flat}' of an array-of-structs member referenced");
                    }
                    Ok(format!("{flat}[{outer}]"))
                }
            }
        })?;
    }
    // deepest paths first: `a.b.c` must be rewritten before any `a.b`
    let mut sorted: Vec<&(String, String, String)> = renames.iter().collect();
    sorted.sort_by_key(|r| -(r.0.matches('.').count() as i64));
    for (inst, member, flat) in sorted {
        let parts: Vec<String> = inst.split('.').chain(std::iter::once(member.as_str())).map(esc).collect();
        let pattern = format!(r"\b{}\b", parts.join(r"\s*\.\s*"));
        body = py::sub_lit(&re(&pattern), &body, flat);
    }
    // The light loops stay loops (pco_shc reads the common store through a
    // dynamic offset); GLSL2USC_UNROLL=1 restores the old unrolled text.
    if !array_sizes.is_empty() && py::env_set("GLSL2USC_UNROLL").is_some() {
        body = unroll_uniform_loops(&body, &array_sizes)?;
    }
    if !cube_names.is_empty()
        || !array_names.is_empty()
        || re(r"\b(samplerCube|sampler2DArray|sampler3D)\b").is_match(&body)
    {
        /* GLSL ES 1.00 spells the cube fetch `textureCube()`; GLSL 450 and
         * the emulation below both use `texture()` / `textureLod()`. The
         * `#define` header at the top cannot do this one, for two reasons:
         * the emulation rewrites the call text itself and so must already
         * see the 450 name, and in GLSL 450 `textureCube` is an opaque
         * *type* (the Vulkan separate-texture kind), so glslang reads an
         * un-rewritten call as a constructor and answers "'sampler/image' :
         * cannot construct this type" rather than "no such function".
         * The Lod form goes first: it is the longer name. */
        body = py::sub_lit(&re(r"\btextureCubeLod\s*\("), &body, "textureLod(");
        body = py::sub_lit(&re(r"\btextureCube\s*\("), &body, "texture(");
        let (b, used) = emulate_samplers(&body, &cube_names, &array_names, &vol_names)?;
        body = b;
        if used {
            // helpers go before the first function definition
            let at = re(r"(?m)^\w+\s+\w+\s*\([^)]*\)\s*\{").find(&body).map(|m| m.start()).unwrap_or(body.len());
            body = format!("{}{}{}", &body[..at], EMU_HELPERS, &body[at..]);
        }
    }
    if stage == "vs" {
        body = hoist_gl_position(&body, &attrib_names);
        body = assemble_matrix_attribs(&body, &mat_attribs);
    }
    Ok((header + &body, gl_names))
}

/// `for\s*\(\s*int\s+(\w+)\s*=\s*0\s*;\s*(?:float\s*\(\s*\1\s*\)|\1)\s*<\s*([^;]+?)\s*;\s*\1\s*\+\+\s*\)\s*\{`
/// searched from `pos` (the regex crate has no backreferences). The prefix up
/// to the first `\1` is deterministic (every `\s*` is followed by a
/// non-space token and `(\w+)` by `\s*=`), so each start position has at most
/// one candidate V and the rest is an ordinary pattern with V spliced in.
/// Returns (start, end, V, COND).
fn find_loop(body: &str, pos: usize) -> Option<(usize, usize, String, String)> {
    let prefix = re(r"\Afor\s*\(\s*int\s+(\w+)\s*=\s*0\s*;\s*");
    let mut from = pos;
    while let Some(off) = body[from..].find("for") {
        let start = from + off;
        if let Some(p) = prefix.captures(&body[start..]) {
            let var = g(&p, 1);
            let v = esc(var);
            let suffix = re(&format!(r"\A(?:float\s*\(\s*{v}\s*\)|{v})\s*<\s*([^;]+?)\s*;\s*{v}\s*\+\+\s*\)\s*\{{"));
            let pend = start + p.get(0).unwrap().end();
            if let Some(s) = suffix.captures(&body[pend..]) {
                let end = pend + s.get(0).unwrap().end();
                return Some((start, end, var.to_string(), g(&s, 1).to_string()));
            }
        }
        from = start + 1;
    }
    None
}

/// Unroll `for (int V = 0; [float(]V[)] < COND; V++) { ... }` loops whose
/// body indexes a push-constant array with V into N guarded copies.
fn unroll_uniform_loops(body: &str, array_sizes: &HashMap<String, i64>) -> R<String> {
    let mut body = body.to_string();
    let mut pos = 0usize;
    loop {
        let Some((mstart, mend, var, cond)) = find_loop(&body, pos) else {
            return Ok(body);
        };
        // the loop body: brace matching from the '{' the pattern ended on
        let b = body.as_bytes();
        let (mut depth, mut i) = (1i64, mend);
        while i < b.len() && depth != 0 {
            if b[i] == b'{' {
                depth += 1;
            } else if b[i] == b'}' {
                depth -= 1;
            }
            i += 1;
        }
        // body[m.end():i - 1] with Python's slice clamping
        let inner = if i >= 1 && i - 1 > mend { body[mend..i - 1].to_string() } else { String::new() };
        // arrays the body indexes with V: flat[V], flat[(V)*M+(j)]
        let v = esc(&var);
        let mut n: Option<i64> = None;
        for am in re(&format!(r"\b(\w+)\s*\[\s*(?:\(\s*{v}\s*\)\s*\*\s*(\d+)\s*\+|{v}\s*\])")).captures_iter(&inner) {
            let Some(&size) = array_sizes.get(g(&am, 1)) else {
                continue;
            };
            let per = match grp(&am, 2) {
                Some(d) => {
                    let d = pint(d)?;
                    if d == 0 {
                        return Err(Error::Fatal("integer division or modulo by zero".into()));
                    }
                    size.div_euclid(d)
                }
                None => size,
            };
            n = Some(n.map_or(per, |n| n.min(per)));
        }
        let Some(n) = n else {
            pos = mend;
            continue;
        };
        let is_float = body[mstart..mend].contains("float");
        let mut copies = String::new();
        for k in 0..n.max(0) {
            let lit = k.to_string();
            let guard = if is_float { format!("float({lit}) < {cond}") } else { format!("{lit} < {cond}") };
            let copy = py::sub_lit(&re(&format!(r"\b{v}\b")), &inner, &lit);
            copies.push_str(&format!("if ({guard}) {{{copy}}}\n"));
        }
        let repl = format!("/* unrolled {var} x{n} */ {{\n{copies}}}");
        body = format!("{}{}{}", &body[..mstart], repl, &body[i..]);
        pos = mstart + repl.len();
    }
}

/// {location: name} of the stage's user varyings from the (translated) text.
pub fn reflect_varyings(glsl: &str, stage: &str) -> R<HashMap<i64, String>> {
    let direction = if stage == "vs" { "out" } else { "in" };
    let text = code_text(glsl)?;
    let mut names = HashMap::new();
    let pat = format!(
        r"layout\s*\(\s*location\s*=\s*(\d+)\s*\)\s*(?:(?:flat|smooth|centroid|noperspective)\s+)?{direction}\s+(\w+)\s+(\w+)\s*(?:\[[^\]]*\])?\s*;"
    );
    for m in re(&pat).captures_iter(&text) {
        names.insert(pint(g(&m, 1))?, g(&m, 3).to_string());
    }
    Ok(names)
}

/// Locations of the stage's `flat` varyings.
pub fn reflect_flat_varyings(glsl: &str, stage: &str) -> R<HashSet<i64>> {
    let direction = if stage == "vs" { "out" } else { "in" };
    let text = code_text(glsl)?;
    let pat = format!(r"layout\s*\(\s*location\s*=\s*(\d+)\s*\)\s*flat\s+{direction}\s+");
    re(&pat).captures_iter(&text).map(|m| pint(g(&m, 1))).collect()
}

/// VS inputs: {location: (name, gl_type)}.
pub fn reflect_attribs(glsl: &str) -> R<HashMap<i64, (String, i64)>> {
    let text = code_text(glsl)?;
    let mut out = HashMap::new();
    /* A matrix attribute's columns (`pvr_mat_<name>_col<k>`) report the name
     * and the matrix type the application declared, at every one of the
     * locations the matrix occupies. GL says glGetAttribLocation of a matN
     * attribute is the first of its N consecutive locations and that the
     * application binds each column itself, so a lookup by name finding the
     * lowest of them is the answer GL asks for. */
    let split = re(&format!(r"\A{}(\w+)_col\d+\z", esc(MAT_ATTR_PREFIX)));
    for m in re(r"layout\s*\(\s*location\s*=\s*(\d+)\s*\)\s*in\s+(\w+)\s+(\w+)\s*;").captures_iter(&text) {
        let (loc, mut typ, mut name) = (pint(g(&m, 1))?, g(&m, 2), g(&m, 3));
        let base;
        if let Some(c) = split.captures(name) {
            base = g(&c, 1).to_string();
            let Some(d) = re(&format!(r"(?m)^(mat[234])\s+{}\s*;", esc(&base))).captures(&text) else {
                exit!("glsl2usc: split matrix attribute '{base}' has no matrix declaration");
            };
            name = &base;
            typ = d.get(1).map(|t| t.as_str()).unwrap_or(typ);
        }
        let Some((_, _, gl_type)) = glsl_type(typ) else {
            exit!("glsl2usc: unsupported attribute type '{typ}'");
        };
        out.insert(loc, (name.to_string(), gl_type));
    }
    Ok(out)
}

/// One entry of the name reflection (becomes a `ShcUniform`).
#[derive(Clone, Debug, PartialEq)]
pub struct Uniform {
    pub name: String,
    pub dword_off: i64,
    pub dwords: i64,
    pub gl_type: i64,
    pub array_len: i64,
    pub binding: i64,
    pub inner: i64,
}

/// `SHC_UNIFORM_NO_BLOCK` / `SHC_UNIFORM_NAME_MAX` of the library ABI.
pub const NO_BLOCK: i64 = yakogl::shaderc::SHC_UNIFORM_NO_BLOCK as i64;
const NAME_MAX: usize = yakogl::shaderc::SHC_UNIFORM_NAME_MAX;

/// The push-constant block members at their std430 dword offsets and the
/// sampler declarations with their binding. Returns (uniforms, block_dwords).
pub fn reflect_uniforms(glsl: &str, gl_names: &GlNames) -> R<(Vec<Uniform>, i64)> {
    let text = code_text(glsl)?;
    let defines = scan_defines(&text)?;
    let mut uniforms = Vec::new();
    let mut block_dw = 0;
    if let Some(m) = re(r"(?s)layout\s*\(\s*push_constant\s*\)\s*uniform\s+\w+\s*\{(.*?)\}\s*(\w*)\s*;").captures(&text)
    {
        if !g(&m, 2).is_empty() {
            exit!(
                "glsl2usc: push-constant block with an instance name ('{}') is not supported: members must be global",
                g(&m, 2)
            );
        }
        let mut off = 0i64;
        for mm in re(r"(\w+)\s+(\w+)\s*(?:\[\s*(\w+)\s*\])?\s*;").captures_iter(g(&m, 1)) {
            let (typ, name, arr) = (g(&mm, 1), g(&mm, 2), grp(&mm, 3));
            let Some((size, align, gl_type)) = glsl_type(typ) else {
                exit!("glsl2usc: unsupported uniform type '{typ}'");
            };
            let n = match arr {
                Some(a) => resolve_size(a, &defines)?,
                None => 1,
            };
            let stride = if arr.is_some() { (size + align - 1).div_euclid(align) * align } else { size };
            off = (off + align - 1).div_euclid(align) * align;
            let (gl, inner) = match gl_names.member(name) {
                Some(GlName::Name(s)) => (s.clone(), 1),
                Some(GlName::Inner(s, i)) => (s.clone(), *i),
                None => (name.to_string(), 1),
            };
            uniforms.push(Uniform {
                name: gl,
                dword_off: off,
                dwords: stride * n,
                gl_type,
                array_len: n,
                binding: 0,
                inner,
            });
            off += stride * n;
        }
        block_dw = off;
    }
    for mm in re(r"layout\s*\(([^)]*)\)\s*uniform\s+(sampler\w+)\s+(\w+)\s*;").captures_iter(&text) {
        let Some(st) = sampler_type(g(&mm, 2)) else {
            exit!("glsl2usc: sampler type '{}' is not supported", g(&mm, 2));
        };
        let binding = match re(r"binding\s*=\s*(\d+)").captures(g(&mm, 1)) {
            Some(b) => pint(g(&b, 1))?,
            None => 0,
        };
        uniforms.push(Uniform {
            name: g(&mm, 3).to_string(),
            dword_off: NO_BLOCK,
            dwords: 1,
            gl_type: gl_names.sampler(g(&mm, 3)).unwrap_or(st),
            array_len: 1,
            binding,
            inner: 1,
        });
    }
    for u in &uniforms {
        if u.name.chars().count() >= NAME_MAX {
            exit!("glsl2usc: uniform name '{}' is longer than SHC_UNIFORM_NAME_MAX-1 ({})", u.name, NAME_MAX - 1);
        }
        if u.array_len > 255 {
            exit!("glsl2usc: uniform array '{}' longer than 255", u.name);
        }
    }
    if uniforms.len() > 255 {
        exit!("glsl2usc: more than 255 uniforms (shc_shader.num_uniforms)");
    }
    Ok((uniforms, block_dw))
}

#[cfg(test)]
mod tests;
