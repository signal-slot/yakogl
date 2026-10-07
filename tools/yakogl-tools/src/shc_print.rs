// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The shader ABI as Rust source: the leaf printers of every generated table.
//!
//! Two emitters write tables of [`ShcShader`] rows -- `emit.rs` (the PowerVR
//! table from Mesa's pco_shc, tests/reference/blobs.rs) and `yakogl-precompile`
//! (the table our own compiler builds, tools/precompile, which compiles this
//! file in with `#[path]`). Both follow the idiom `emit.rs` describes: a row
//! is a real library struct, printed by destructuring it with a FULL
//! pattern, so a field added to the library fails the build of every emitter
//! until it prints the field. The structs below are shared; each emitter
//! prints `ShcShader` itself, because what a field means in a row is that
//! emitter's to say (a pco row never has `layout_at_link`; one of ours can).

use core::ffi::c_char;
use yakogl::shaderc::{ShcAttrib, ShcUniform, ShcVarying};
use yakogl::util::name_bytes;

/// `text` as the body of a Rust string literal. Everything outside printable
/// ASCII is escaped, so control and bidi code points in a shader's comments
/// cannot break (or disguise) the generated source.
pub fn rs_str(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            ' '..='~' => out.push(ch),
            _ => out.push_str(&format!("\\u{{{:x}}}", ch as u32)),
        }
    }
    out
}

/// A NUL-padded name field as the body of a string literal.
pub fn name_str(field: &[c_char]) -> String {
    rs_str(&String::from_utf8_lossy(name_bytes(field)))
}

pub fn print_varying(v: &ShcVarying) -> String {
    let ShcVarying { location, components, interp, vtxout_off, name } = *v;
    format!(
        "ShcVarying {{ location: {location}, components: {components}, interp: {interp}, vtxout_off: {vtxout_off}, name: cname(\"{}\") }}",
        name_str(&name)
    )
}

pub fn print_attrib(a: &ShcAttrib) -> String {
    let ShcAttrib { location, components, vi_reg, format, name, gl_type } = *a;
    format!(
        "ShcAttrib {{ location: {location}, components: {components}, vi_reg: {vi_reg}, format: {format}, name: cname(\"{}\"), gl_type: 0x{gl_type:04x} }}",
        name_str(&name)
    )
}

pub fn print_uniform(u: &ShcUniform) -> String {
    let ShcUniform { name, dword_off, dwords, gl_type, array_len, binding, inner } = *u;
    format!(
        "ShcUniform {{ name: cname(\"{}\"), dword_off: {dword_off}, dwords: {dwords}, gl_type: 0x{gl_type:04x}, array_len: {array_len}, binding: {binding}, inner: {inner} }}",
        name_str(&name)
    )
}

pub fn is_zero(name: &[c_char]) -> bool {
    name.iter().all(|&c| c == 0)
}

pub fn varying_is_zero(v: &ShcVarying) -> bool {
    let ShcVarying { location, components, interp, vtxout_off, name } = *v;
    location == 0 && components == 0 && interp == 0 && vtxout_off == 0 && is_zero(&name)
}

pub fn attrib_is_zero(a: &ShcAttrib) -> bool {
    let ShcAttrib { location, components, vi_reg, format, name, gl_type } = *a;
    location == 0 && components == 0 && vi_reg == 0 && format == 0 && gl_type == 0 && is_zero(&name)
}

/// An array field of the shader ABI: its name in the generated source, the
/// Rust type of its entries, and how one entry is printed and recognised as
/// empty. Every generated table prints these fields, and it prints them in
/// one of two forms -- the fixed array a [`ShcShader`] holds ([`print_array`]),
/// or a static a table's row or reflection points at (`print_list` in
/// tools/precompile) -- so both forms are described by one value per field.
/// The type belongs in the description because the fixed form pads with
/// `<ty>::ZERO`: the entry a pad fills with is named by the type, so no
/// emitter can pair a field with another type's zero.
pub struct Field<T> {
    pub field: &'static str,
    pub ty: &'static str,
    pub is_zero: fn(&T) -> bool,
    pub print: fn(&T) -> String,
}

/// A [`Field`] that is also written as a fixed array: the library's pad
/// helper, and the array's length as the row writes it in `[ZERO; N]` (a
/// constant's name, or "" for the literal length).
pub struct ArrayField<T> {
    pub kind: Field<T>,
    pub pad: &'static str,
    pub len: &'static str,
}

pub const VARYINGS: Field<ShcVarying> =
    Field { field: "varyings", ty: "ShcVarying", is_zero: varying_is_zero, print: print_varying };

pub const ATTRIBS: Field<ShcAttrib> =
    Field { field: "attribs", ty: "ShcAttrib", is_zero: attrib_is_zero, print: print_attrib };

pub const ARRAY_VARYINGS: ArrayField<ShcVarying> = ArrayField { kind: VARYINGS, pad: "pad_varyings", len: "" };

pub const ARRAY_ATTRIBS: ArrayField<ShcAttrib> = ArrayField { kind: ATTRIBS, pad: "pad_attribs", len: "" };

/// `field: pad([...]),` (or `field: [<ty>::ZERO; N],` when nothing is used),
/// its lines indented by `ind`.
pub fn print_array<T>(
    out: &mut Vec<String>,
    ind: &str,
    a: &ArrayField<T>,
    items: &[T],
    used: usize,
) -> Result<(), String> {
    let ArrayField { kind: Field { field, ty, is_zero, print }, pad, len } = a;
    if items[used.min(items.len())..].iter().any(|x| !is_zero(x)) {
        return Err(format!("{field}: non-zero entries past the count"));
    }
    if used == 0 {
        let n = if len.is_empty() { items.len().to_string() } else { len.to_string() };
        out.push(format!("{ind}{field}: [{ty}::ZERO; {n}],"));
        return Ok(());
    }
    out.push(format!("{ind}{field}: {pad}(["));
    for x in &items[..used] {
        out.push(format!("{ind}    {},", print(x)));
    }
    out.push(format!("{ind}]),"));
    Ok(())
}

/// `static NAME: [u32; N] = [...];`, eight-ish words a line (96 columns).
pub fn emit_u32_array(out: &mut Vec<String>, name: &str, words: &[u32]) {
    out.push(format!("static {name}: [u32; {}] = [", words.len()));
    let mut line = String::from("    ");
    for dw in words {
        let tok = format!("0x{dw:08x},");
        if line.len() + tok.len() + 1 > 96 {
            out.push(line.trim_end().to_string());
            line = "    ".into();
        }
        line.push_str(&tok);
        line.push(' ');
    }
    if !line.trim().is_empty() {
        out.push(line.trim_end().to_string());
    }
    out.push("];".into());
}

/// A `//` comment line whose text may hold newlines or odd code points.
pub fn rs_comment(line: &str) -> String {
    let mut s = String::new();
    for (i, part) in line.split('\n').enumerate() {
        if i > 0 {
            s.push_str("\n// ");
        }
        for ch in part.chars() {
            if ch == '\r' || (ch.is_control() && ch != '\t') || is_bidi(ch) {
                s.push_str(&format!("\\u{{{:x}}}", ch as u32));
            } else {
                s.push(ch);
            }
        }
    }
    s
}

/// Code points rustc's `text_direction_codepoint_in_comment` lint rejects.
fn is_bidi(c: char) -> bool {
    matches!(c, '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}' | '\u{200E}' | '\u{200F}' | '\u{061C}')
}
