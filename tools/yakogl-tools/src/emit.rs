// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The offline shader table writer (`tests/reference/blobs.rs`).
//!
//! The ABI has one source of truth, the library's `#[repr(C)]` structs: every
//! entry is first built as a real [`ShcShader`] (with [`ShcVarying`],
//! [`ShcAttrib`], [`ShcUniform`]) using full struct literals, then printed by
//! destructuring those values with full patterns. A field added to a library
//! struct therefore fails this tool's build until the emitter handles it.
//!
//! # Row order
//!
//! Two of the three entry arrays are emitted sorted, one is not, and the
//! difference is load-bearing: for attributes and uniforms the row order IS
//! the order GL enumerates them, so it is observable through the API and a
//! reordering is a behaviour change, not a cosmetic diff.
//!
//! | array | emitted order | who depends on it |
//! |---|---|---|
//! | `attribs` | sorted by `location` | `glGetActiveAttrib` walks the array by position (`gl::program`), so enumeration order is ascending location **by construction** |
//! | `varyings` | sorted by `location` | nothing: `shaderc::link` pairs the stages by NAME. Sorted only so a rebuild diffs cleanly |
//! | `uniforms` | front-end order, NOT sorted | `glGetActiveUniform`: `build_locations` walks the VS rows then the FS rows in table order, appending each name it has not seen, and `active_uniform_nth` walks that table |
//!
//! The uniform row order is therefore the front end's declaration order --
//! the order `glsl.rs` pushed the members into the `pvr_U` push-constant
//! block, after struct flattening -- carried all the way out to the
//! application. ES 2.0 does not dictate an enumeration order, so any
//! deterministic one conforms; what would be a defect is an order that moves
//! between two builds of the same shader, or one that disagrees with the
//! location table built from the same rows.
//!
//! For the #47 name-pool flip this is the one invariant to hold on to: the
//! POOL may be ordered however deduplicates best (sorted by name is fine, and
//! is what makes a rebuild diff cleanly), because an entry reaches its name
//! through an offset. The ROWS must stay where they are. Sorting rows by name
//! while interning would silently renumber every `glGetActiveUniform` and
//! `glGetActiveAttrib` index in the table.

use crate::glsl::Uniform;
use crate::glsl2usc::{Attrib, Shader, Varying};
use crate::py;
use crate::shc_print::{emit_u32_array, print_array, print_uniform, rs_comment, rs_str, ARRAY_ATTRIBS, ARRAY_VARYINGS};
use crate::{Error, R};
use core::ffi::c_char;
use yakogl::shaderc::{ShcAttrib, ShcShader, ShcUniform, ShcVarying, SHC_STAGE_FRAGMENT, SHC_STAGE_VERTEX};
use yakogl::util::cname;

/// A value that must fit its ABI field.
fn fit<T: TryFrom<i64>>(v: i64, what: &str, shader: &str) -> R<T> {
    T::try_from(v).map_err(|_| Error::Fatal(format!("{shader}: {what} = {v} does not fit its shc_shader field")))
}

/// A NUL-padded name field (the library's own `cname`, truncating to N-1).
fn name_field<const N: usize>(s: &str) -> [c_char; N] {
    cname::<N>(s)
}

fn varying(v: &Varying, sh: &str) -> R<ShcVarying> {
    Ok(ShcVarying {
        location: fit(v.location, "varying location", sh)?,
        components: fit(v.components, "varying components", sh)?,
        interp: fit(v.interp, "varying interp", sh)?,
        vtxout_off: fit(v.vtxout_off, "varying vtxout_off", sh)?,
        name: name_field(&v.name),
    })
}

fn attrib(a: &Attrib, sh: &str) -> R<ShcAttrib> {
    Ok(ShcAttrib {
        location: fit(a.location, "attrib location", sh)?,
        components: fit(a.components, "attrib components", sh)?,
        vi_reg: fit(a.vi_reg, "attrib vi_reg", sh)?,
        format: fit(a.format, "attrib format", sh)?,
        name: name_field(&a.name),
        gl_type: fit(a.gl_type, "attrib gl_type", sh)?,
    })
}

fn uniform(u: &Uniform, sh: &str) -> R<ShcUniform> {
    Ok(ShcUniform {
        name: name_field(&u.name),
        dword_off: fit(u.dword_off, "uniform dword_off", sh)?,
        dwords: fit(u.dwords, "uniform dwords", sh)?,
        gl_type: fit(u.gl_type, "uniform gl_type", sh)?,
        array_len: fit(u.array_len, "uniform array_len", sh)?,
        binding: fit(u.binding, "uniform binding", sh)?,
        inner: fit(u.inner, "uniform inner", sh)?,
    })
}

/// One table row as ABI values. The arrays the pointers refer to are owned
/// by the returned vectors (kept alive with the row).
pub struct Row {
    pub stage: u32,
    pub shader: ShcShader,
    pub uniforms: Vec<ShcUniform>,
    usc: Vec<u32>,
}

/// Build the `ShcShader` the C emitter's designated initialisers described:
/// fields a stage does not set stay zero, as in C.
pub fn build(sh: &Shader) -> R<Row> {
    let n = sh.name.as_str();
    let fs = match sh.stage.as_str() {
        "fs" => true,
        "vs" => false,
        other => return Err(Error::Fatal(format!("{n}: unknown stage {}", py::repr(other)))),
    };
    let mut vs_sorted: Vec<&Varying> = sh.varyings.iter().collect();
    vs_sorted.sort_by_key(|v| v.location);
    let mut varyings = [ShcVarying::ZERO; 16];
    if vs_sorted.len() > varyings.len() {
        return Err(Error::Fatal(format!("{n}: {} varyings, shc_shader holds 16", vs_sorted.len())));
    }
    for (d, v) in varyings.iter_mut().zip(vs_sorted) {
        *d = varying(v, n)?;
    }
    let mut attribs = [ShcAttrib::ZERO; 16];
    if !fs {
        let mut sorted: Vec<&Attrib> = sh.attribs.iter().collect();
        sorted.sort_by_key(|a| a.location);
        if sorted.len() > attribs.len() {
            return Err(Error::Fatal(format!("{n}: {} attributes, shc_shader holds 16", sorted.len())));
        }
        for (d, a) in attribs.iter_mut().zip(sorted) {
            *d = attrib(a, n)?;
        }
    }
    let uniforms: Vec<ShcUniform> = sh.uniforms.iter().map(|u| uniform(u, n)).collect::<R<_>>()?;
    let usc = sh.usc.clone();
    let reg1 = |r: Option<i64>| -> R<u8> { fit(r.map_or(0, |r| r + 1), "vs id register", n) };
    let zero_unless = |cond: bool, v: i64| if cond { v } else { 0 };
    let shader = ShcShader {
        usc: usc.as_ptr(),
        usc_dw: fit(usc.len() as i64, "usc_dw", n)?,
        temps: fit(sh.temps, "temps", n)?,
        varyings,
        num_varyings: fit(sh.varyings.len() as i64, "num_varyings", n)?,
        uses_w: fit(zero_unless(fs, sh.uses_w), "uses_w", n)?,
        uses_discard: fit(zero_unless(fs, sh.uses_discard), "uses_discard", n)?,
        /* GC7000 only (tools/vivante/gen_shader_table.py compiles the second
         * binary); on the PowerVR the front-face sense is draw state, written
         * into the shared register fs_front_face_off1 names. */
        reads_front_facing: 0,
        meta_off: fit(zero_unless(fs, sh.meta_off), "meta_off", n)?,
        fs_front_face_off1: fit(zero_unless(fs, sh.front_face_off.map_or(0, |o| o + 1)), "fs_front_face_off1", n)?,
        num_outputs: fit(zero_unless(fs, sh.num_outputs), "num_outputs", n)?,
        num_tile_buffers: fit(zero_unless(fs, sh.num_tile_buffers), "num_tile_buffers", n)?,
        tile_buffer_sh: fit(zero_unless(fs, sh.tile_buffer_sh), "tile_buffer_sh", n)?,
        attribs,
        num_attribs: fit(zero_unless(!fs, sh.attribs.len() as i64), "num_attribs", n)?,
        vs_output_size: fit(zero_unless(!fs, sh.vs_output_size), "vs_output_size", n)?,
        vs_psize_off: fit(zero_unless(!fs, sh.vs_psize_off), "vs_psize_off", n)?,
        vs_vertex_id_reg1: if fs { 0 } else { reg1(sh.vs_vertex_id_reg)? },
        vs_instance_id_reg1: if fs { 0 } else { reg1(sh.vs_instance_id_reg)? },
        uniform_dwords: fit(sh.uniform_dwords, "uniform_dwords", n)?,
        /* The C emitter leaves the field out when it equals uniform_dwords
         * (a designated initialiser then zeroes it, which the backend reads
         * as "the whole block"); a Rust struct literal names every field, so
         * the same case is written as 0. */
        uniform_sh_dwords: fit(
            if sh.uniform_sh_dwords != 0 && sh.uniform_sh_dwords != sh.uniform_dwords {
                sh.uniform_sh_dwords
            } else {
                0
            },
            "uniform_sh_dwords",
            n,
        )?,
        uniforms: if uniforms.is_empty() { core::ptr::null() } else { uniforms.as_ptr() },
        num_uniforms: fit(uniforms.len() as i64, "num_uniforms", n)?,
        blend_key: if fs { sh.blend_key } else { 0 },
        uses_fbfetch: fit(zero_unless(fs, sh.uses_fbfetch), "uses_fbfetch", n)?,
        blend_consts_off: fit(zero_unless(fs, sh.blend_consts_off), "blend_consts_off", n)?,
        uses_pntc: fit(zero_unless(fs, sh.uses_pntc), "uses_pntc", n)?,
        uses_z: fit(zero_unless(fs, sh.uses_z), "uses_z", n)?,
        /* pco's programs read gl_FragCoord.y unchanged: no pair. */
        fragcoord_ytf_off1: 0,
        /* Since native cube step C glsl2usc hands pco a real samplerCube,
         * which it samples as the TPU's own cube: a program with a cube
         * sampler row gets the driver's twiddled copy (gl::native_cube). */
        native_cube: u8::from(uniforms.iter().any(|u| {
            u.dword_off == yakogl::shaderc::SHC_UNIFORM_NO_BLOCK
                && matches!(u.gl_type, 0x8B60 | 0x8DC5 | 0x8DCC | 0x8DD4) /* samplerCube, its shadow, int, uint */
        })),
        layout_at_link: 0,
        declared: core::ptr::null(),
        num_declared: 0,
        /* no source of tools/shaders/precompiled.list writes gl_FragDepth;
         * the depth feedback is the online compiler's (ShcShader::writes_depth) */
        writes_depth: 0,
        priv_: core::ptr::null(),
    };
    Ok(Row { stage: if fs { SHC_STAGE_FRAGMENT } else { SHC_STAGE_VERTEX }, shader, uniforms, usc })
}

impl Row {
    pub fn usc(&self) -> &[u32] {
        &self.usc
    }
}

fn print_shader(out: &mut Vec<String>, s: &ShcShader, ident: &str) -> R<()> {
    let ShcShader {
        usc,
        usc_dw,
        temps,
        varyings,
        num_varyings,
        uses_w,
        uses_discard,
        reads_front_facing,
        meta_off,
        fs_front_face_off1,
        num_outputs,
        num_tile_buffers,
        tile_buffer_sh,
        attribs,
        num_attribs,
        vs_output_size,
        vs_psize_off,
        vs_vertex_id_reg1,
        vs_instance_id_reg1,
        uniform_dwords,
        uniform_sh_dwords,
        uniforms,
        num_uniforms,
        blend_key,
        uses_fbfetch,
        blend_consts_off,
        uses_pntc,
        uses_z,
        fragcoord_ytf_off1,
        native_cube,
        layout_at_link: _,
        declared: _,
        num_declared: _,
        writes_depth,
        priv_,
    } = *s;
    if usc.is_null() || !priv_.is_null() || uniforms.is_null() != (num_uniforms == 0) {
        return Err(Error::Fatal(format!("{ident}: inconsistent pointers")));
    }
    out.push("        shader: ShcShader {".into());
    out.push(format!("            usc: {ident}_USC.as_ptr(),"));
    out.push(format!("            usc_dw: {usc_dw},"));
    out.push(format!("            temps: {temps},"));
    print_array(out, "            ", &ARRAY_VARYINGS, &varyings, num_varyings as usize).map_err(Error::Fatal)?;
    out.push(format!("            num_varyings: {num_varyings},"));
    out.push(format!("            uses_w: {uses_w},"));
    out.push(format!("            uses_discard: {uses_discard},"));
    out.push(format!("            reads_front_facing: {reads_front_facing},"));
    out.push(format!("            meta_off: 0x{meta_off:x},"));
    out.push(format!("            fs_front_face_off1: {fs_front_face_off1},"));
    out.push(format!("            num_outputs: {num_outputs},"));
    out.push(format!("            num_tile_buffers: {num_tile_buffers},"));
    out.push(format!("            tile_buffer_sh: {tile_buffer_sh},"));
    print_array(out, "            ", &ARRAY_ATTRIBS, &attribs, num_attribs as usize).map_err(Error::Fatal)?;
    out.push(format!("            num_attribs: {num_attribs},"));
    out.push(format!("            vs_output_size: {vs_output_size},"));
    out.push(format!("            vs_psize_off: 0x{vs_psize_off:x},"));
    out.push(format!("            vs_vertex_id_reg1: {vs_vertex_id_reg1},"));
    out.push(format!("            vs_instance_id_reg1: {vs_instance_id_reg1},"));
    out.push(format!("            uniform_dwords: {uniform_dwords},"));
    out.push(format!("            uniform_sh_dwords: {uniform_sh_dwords},"));
    if uniforms.is_null() {
        out.push("            uniforms: core::ptr::null(),".into());
    } else {
        out.push(format!("            uniforms: {ident}_UNIFORMS.as_ptr(),"));
    }
    out.push(format!("            num_uniforms: {num_uniforms},"));
    out.push(format!("            blend_key: 0x{blend_key:x},"));
    out.push(format!("            uses_fbfetch: {uses_fbfetch},"));
    out.push(format!("            blend_consts_off: 0x{blend_consts_off:x},"));
    out.push(format!("            uses_pntc: {uses_pntc},"));
    out.push(format!("            uses_z: {uses_z},"));
    out.push(format!("            fragcoord_ytf_off1: {fragcoord_ytf_off1},"));
    out.push(format!("            native_cube: {native_cube},"));
    out.push("            layout_at_link: 0,".into());
    out.push("            declared: core::ptr::null(),".into());
    out.push("            num_declared: 0,".into());
    out.push(format!("            writes_depth: {writes_depth},"));
    out.push("            priv_: core::ptr::null(),".into());
    out.push("        },".into());
    Ok(())
}

/// Identifiers: the basename, or parent_basename when two inputs share one.
fn c_ident(basename: &str) -> String {
    let mut ident = py::sub_lit(&py::re(r"[^0-9A-Za-z_]"), basename, "_");
    if ident.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        ident.insert(0, '_');
    }
    ident
}

pub fn emit(shaders: &[Shader], toolchain: &str) -> R<String> {
    let mut out: Vec<String> = vec![
        "// SPDX-License-Identifier: Apache-2.0".into(),
        "//".into(),
        "// blobs.rs - GENERATED by `yakogl-tools glsl2usc` (tools/glsl2usc.sh). DO NOT EDIT.".into(),
        "//".into(),
        "// PowerVR USC + wiring metadata for BXS-4-64 (BVNC 36.53.104.796),".into(),
        "// keyed by SHA-256 of the GLSL source. See src/shaderc/SHADERC_NOTES.md.".into(),
        "//".into(),
        "// The toolchain that built it. This table IS the reference: a rebuild by a".into(),
        "// different toolchain is a NEW reference and the generator refuses it unless".into(),
        "// GLSL2USC_NEW_REFERENCE=1 says so deliberately.".into(),
        format!("{}{}", crate::glsl2usc::TOOLCHAIN_MARK, toolchain),
        "//".into(),
        "// Shaders in this table:".into(),
    ];
    for sh in shaders {
        let bl = if sh.blend_spec.is_empty() { String::new() } else { format!("  blend={}", sh.blend_spec) };
        let dbg = if sh.pco_debug.is_empty() { String::new() } else { format!("  pco_debug={}", sh.pco_debug) };
        out.push(rs_comment(&format!(
            "//   {} {}  sha256={}{}{}",
            py::ljust(&sh.name, 20),
            sh.stage,
            sh.sha_hex(),
            bl,
            dbg
        )));
    }
    out.extend(
        [
            "#![allow(clippy::all)]",
            "#![cfg_attr(rustfmt, rustfmt_skip)]",
            "",
            "#[allow(unused_imports)]",
            "use super::{",
            "    pad_attribs, pad_varyings, ShcAttrib, ShcBlob, ShcShader, ShcSource, ShcUniform, ShcVarying,",
            "    SHC_STAGE_FRAGMENT, SHC_STAGE_VERTEX,",
            "};",
            "#[allow(unused_imports)]",
            "use crate::util::cname;",
            "",
        ]
        .map(String::from),
    );

    // identifiers of the base entries (a variant reuses its base's)
    let mut idents: std::collections::HashMap<&str, String> = std::collections::HashMap::new();
    let mut used = std::collections::HashSet::new();
    for sh in shaders {
        if sh.blend_key != 0 {
            continue;
        }
        let mut ident = c_ident(&sh.name);
        if used.contains(&ident) {
            ident = c_ident(&format!("{}_{}", py::basename(py::dirname(&sh.path)), sh.name));
        }
        used.insert(ident.clone());
        idents.insert(sh.path.as_str(), ident);
    }
    let ident_of = |sh: &Shader| -> R<String> {
        let base = idents.get(sh.path.as_str()).ok_or_else(|| {
            Error::Fatal(format!("KeyError: {} (a blend variant without its base shader)", py::repr(&sh.path)))
        })?;
        let v = if sh.blend_key != 0 { format!("{base}_blend{:x}", sh.blend_key) } else { base.clone() };
        Ok(v.to_uppercase())
    };

    let mut rows = Vec::with_capacity(shaders.len());
    for sh in shaders {
        let ident = ident_of(sh)?;
        let row = build(sh)?;
        if !sh.note.is_empty() {
            out.push(rs_comment(&format!("// {}: {}", sh.name, sh.note)));
        }
        if !sh.blend_spec.is_empty() {
            out.push(rs_comment(&format!(
                "// {} compiled for blending: {} (fb-fetch + blend in the FS)",
                sh.name, sh.blend_spec
            )));
        }
        if !sh.pco_debug.is_empty() {
            out.push(rs_comment(&format!("// {} built by pco_shc under PCO_DEBUG={}", sh.name, sh.pco_debug)));
        }
        emit_u32_array(&mut out, &format!("{ident}_USC"), row.usc());
        if !row.uniforms.is_empty() {
            out.push(format!("static {ident}_UNIFORMS: [ShcUniform; {}] = [", row.uniforms.len()));
            for u in &row.uniforms {
                out.push(format!("    {},", print_uniform(u)));
            }
            out.push("];".into());
        }
        out.push(String::new());
        rows.push((ident, row));
    }

    out.push(format!("pub static SHC_BLOB_TABLE: [ShcBlob; {}] = [", shaders.len()));
    for (sh, (ident, row)) in shaders.iter().zip(&rows) {
        out.push("    ShcBlob {".into());
        let sha: Vec<String> = sh.sha.iter().map(|b| format!("0x{b:02x}")).collect();
        out.push(format!("        sha256: [{}],", sha.join(", ")));
        /* NUL-terminated: shc_shader_source_name() hands the bytes to C as a
         * `const char *` (ShcBlob::source_name() drops the NUL here). */
        out.push(format!("        name: \"{}\\0\",", rs_str(&sh.name)));
        out.push(format!(
            "        stage: {},",
            if row.stage == SHC_STAGE_FRAGMENT { "SHC_STAGE_FRAGMENT" } else { "SHC_STAGE_VERTEX" }
        ));
        out.push(format!("        blend_key: 0x{:x},", sh.blend_key));
        print_shader(&mut out, &row.shader, ident)?;
        out.push("    },".into());
    }
    out.push("];".into());
    out.push(String::new());
    // The source text of every input, so an application on an offline-only
    // target can hand glShaderSource the exact bytes the table was built
    // from (gles_offline_shader_source()). Each text ends in a NUL.
    let mut seen = std::collections::HashSet::new();
    let mut sources = Vec::new();
    for sh in shaders {
        if sh.blend_key != 0 || !seen.insert(sh.name.as_str()) {
            continue;
        }
        sources.push(sh);
    }
    out.push(format!("pub static SHC_SOURCE_TABLE: [ShcSource; {}] = [", sources.len()));
    for sh in sources {
        out.push("    ShcSource {".into());
        out.push(format!("        name: \"{}\",", rs_str(&sh.name)));
        out.push("        text: concat!(".into());
        let lines: Vec<&str> = sh.source.split('\n').collect();
        // Python: split("\n")[:-1] if the text ends in a newline. A text
        // without a final newline still gets one (inherited behaviour).
        let lines = if sh.source.ends_with('\n') { &lines[..lines.len() - 1] } else { &lines[..] };
        for line in lines {
            out.push(format!("            \"{}\\n\",", rs_str(line)));
        }
        out.push("            \"\\0\",".into());
        out.push("        ),".into());
        out.push("    },".into());
    }
    out.push("];".into());
    out.push(String::new());
    Ok(out.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shc_print::name_str;

    fn u(name: &str, dword_off: i64) -> Uniform {
        Uniform { name: name.into(), dword_off, dwords: 4, gl_type: 0x8B52, array_len: 0, binding: 0, inner: 0 }
    }

    fn a(name: &str, location: i64) -> Attrib {
        Attrib { location, components: 4, vi_reg: location, format: 0, name: name.into(), gl_type: 0x8B52 }
    }

    /// The row order the generated table carries is observable GL state, so
    /// it is pinned here rather than left to the reader of `build`:
    /// attributes come out sorted by location because `glGetActiveAttrib`
    /// walks the array by position, and uniforms come out in the front end's
    /// order because `build_locations` and `active_uniform_nth` walk them in
    /// that order to number `glGetActiveUniform`. See the module comment.
    ///
    /// The #47 name pool may be sorted however it likes; if interning ever
    /// reorders these rows instead, this test is what says so.
    #[test]
    fn attribs_sort_by_location_and_uniforms_keep_the_front_ends_order() {
        let mut sh = Shader::new("vs");
        sh.name = "roworder".into();
        // Deliberately neither sorted by location nor by name.
        sh.attribs = vec![a("a_uv", 3), a("a_pos", 0), a("a_col", 1)];
        // Deliberately neither alphabetical nor ascending by offset.
        sh.uniforms = vec![u("u_mvp", 0), u("u_alpha", 64), u("u_colour", 16)];

        let row = build(&sh).expect("build");

        let attribs: Vec<String> =
            row.shader.attribs[..row.shader.num_attribs as usize].iter().map(|x| name_str(&x.name)).collect();
        assert_eq!(attribs, ["a_pos", "a_col", "a_uv"], "attributes are emitted sorted by location");

        let uniforms: Vec<String> = row.uniforms.iter().map(|x| name_str(&x.name)).collect();
        assert_eq!(
            uniforms,
            ["u_mvp", "u_alpha", "u_colour"],
            "uniform rows keep the order the front end produced -- sorting them would renumber glGetActiveUniform"
        );
    }

    #[test]
    fn escapes() {
        assert_eq!(rs_str("a\"b\\c\td\u{202e}é\x07"), "a\\\"b\\\\c\\td\\u{202e}\\u{e9}\\u{7}");
        assert_eq!(rs_comment("// x\ny\u{2066}"), "// x\n// y\\u{2066}");
        assert_eq!(c_ident("9fill.glyph.frag"), "_9fill_glyph_frag");
        assert_eq!(c_ident("é.frag"), "__frag");
    }
}
