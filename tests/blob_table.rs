// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Dumps the offline shader table as JSON (the format of the C table dumper
//! used when the table was converted from the C shader_blobs.c), so the two can be
//! diffed: `YAKOGL_DUMP_BLOBS=out.json cargo test --no-default-features
//! --features host-test,gpu-powervr --test blob_table`.
//! Both tables have the same shape, but this one names the PowerVR table:
//! a `gpu-vivante` build compiles `vivante_blobs.rs` instead.
#![cfg(feature = "gpu-powervr")]

use std::fmt::Write;
use yakogl::shaderc::blobs::{SHC_BLOB_TABLE, SHC_SOURCE_TABLE};
use yakogl::util::name_bytes;

fn jstr(out: &mut String, s: &[u8]) {
    out.push('"');
    for &c in s {
        match c {
            b'"' | b'\\' => {
                out.push('\\');
                out.push(c as char)
            }
            c if c < 0x20 => write!(out, "\\u{:04x}", c).unwrap(),
            _ => out.push(c as char),
        }
    }
    out.push('"');
}

#[test]
fn dump_blob_table() {
    let mut o = String::new();
    o.push_str("{\"blobs\":[\n");
    for (i, b) in SHC_BLOB_TABLE.iter().enumerate() {
        let s = &b.shader;
        o.push_str(if i > 0 { ",\n" } else { "" });
        o.push_str("{\"sha\":\"");
        for x in b.sha256 {
            write!(o, "{:02x}", x).unwrap();
        }
        o.push_str("\",\"name\":");
        jstr(&mut o, b.source_name().as_bytes());
        write!(o, ",\"stage\":{},\"blend_key\":{},\"sh_blend_key\":{},\"temps\":{},\"uses_w\":{},\"uses_discard\":{},\"meta_off\":{},\"fs_front_face_off1\":{},\"num_outputs\":{},\"num_tile_buffers\":{},\"tile_buffer_sh\":{},\"vs_output_size\":{},\"vs_psize_off\":{},\"vs_vertex_id_reg1\":{},\"vs_instance_id_reg1\":{},\"uniform_dwords\":{},\"uniform_sh_dwords\":{},\"uses_fbfetch\":{},\"blend_consts_off\":{},\"num_varyings\":{},\"num_attribs\":{},\"num_uniforms\":{},\"priv_null\":{}",
            b.stage, b.blend_key, s.blend_key, s.temps, s.uses_w, s.uses_discard, s.meta_off, s.fs_front_face_off1, s.num_outputs, s.num_tile_buffers, s.tile_buffer_sh, s.vs_output_size, s.vs_psize_off, s.vs_vertex_id_reg1, s.vs_instance_id_reg1, s.uniform_dwords, s.uniform_sh_dwords, s.uses_fbfetch, s.blend_consts_off, s.num_varyings, s.num_attribs, s.num_uniforms, s.priv_.is_null() as i32).unwrap();
        o.push_str(",\"usc\":[");
        for (k, w) in s.usc_words().iter().enumerate() {
            write!(o, "{}{}", if k > 0 { "," } else { "" }, w).unwrap();
        }
        o.push_str("],\"varyings\":[");
        for (k, v) in s.varyings.iter().enumerate() {
            write!(
                o,
                "{}{{\"location\":{},\"components\":{},\"interp\":{},\"vtxout_off\":{},\"name\":",
                if k > 0 { "," } else { "" },
                v.location,
                v.components,
                v.interp,
                v.vtxout_off
            )
            .unwrap();
            jstr(&mut o, name_bytes(&v.name));
            o.push('}');
        }
        o.push_str("],\"attribs\":[");
        for (k, a) in s.attribs.iter().enumerate() {
            write!(
                o,
                "{}{{\"location\":{},\"components\":{},\"vi_reg\":{},\"format\":{},\"gl_type\":{},\"name\":",
                if k > 0 { "," } else { "" },
                a.location,
                a.components,
                a.vi_reg,
                a.format,
                a.gl_type
            )
            .unwrap();
            jstr(&mut o, name_bytes(&a.name));
            o.push('}');
        }
        o.push_str("],\"uniforms\":[");
        for (k, u) in s.uniform_table().iter().enumerate() {
            write!(o, "{}{{\"dword_off\":{},\"dwords\":{},\"gl_type\":{},\"array_len\":{},\"binding\":{},\"inner\":{},\"name\":", if k > 0 { "," } else { "" }, u.dword_off, u.dwords, u.gl_type, u.array_len, u.binding, u.inner).unwrap();
            jstr(&mut o, name_bytes(&u.name));
            o.push('}');
        }
        o.push_str("]}");
    }
    o.push_str("],\n\"sources\":[\n");
    for (i, s) in SHC_SOURCE_TABLE.iter().enumerate() {
        o.push_str(if i > 0 { ",\n" } else { "" });
        o.push_str("{\"name\":");
        jstr(&mut o, s.name.as_bytes());
        o.push_str(",\"text\":");
        assert!(s.text.ends_with('\0'), "{}: text must be NUL-terminated", s.name);
        jstr(&mut o, &s.text.as_bytes()[..s.text.len() - 1]);
        o.push('}');
    }
    o.push_str("]}\n");
    assert!(!SHC_BLOB_TABLE.is_empty());
    if let Ok(path) = std::env::var("YAKOGL_DUMP_BLOBS") {
        std::fs::write(path, o).unwrap();
    }
}
