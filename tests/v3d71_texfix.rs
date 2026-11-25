// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The stage71 fixes of the V3D 7.1 texture path (src/v3d/format71.rs,
//! src/v3d/texture71.rs), each pinned against Mesa 26.2.1.
//!
//! * fix 1, depth: a depth texture's record swizzle is the row's (XXXX,
//!   read from the vendored v3dx_format_table.c) composed with the GLES depth
//!   mode's X X X 1 (Mesa's teximage.c:797-798, texobj.c:395, texobj.c:
//!   301-312), so the look-up is (d, d, d, 1); colour rows are unchanged;
//! * fix 2, packed depth-stencil: GL_DEPTH24_STENCIL8 is laid out as its
//!   storage row S8_UINT_Z24_UNORM and sampled as X8Z24_UNORM
//!   (v3dx_state.c:1083-1089), with the same texture type and swizzle, so
//!   the record's words do not move;
//! * fix 4: transfer function, R/B swap and reverse per revision
//!   (v3dx_state.c:1023-1029, v3d_packet.xml:1686-1704), checked against the
//!   words of Mesa's gl_texfmt records in tools/v3d/pi5/README.md (sRGB
//!   transfer 1 in word 0, R/B swap 0 at bit 32) and the revisions.
//!
//! No Mesa reference dump samples a depth texture yet (tools/v3d/pi5 has
//! gl_tex2d, gl_texfmt, gl_cube and gl_mipgen), so fixes 1 and 2 are pinned
//! against the vendored table and the rules cited, not against CLIF words.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test v3d71_texfix
//! ```
//! (the modules and this test use no crate feature; any set that builds the
//! tests runs it.)

#[path = "../src/v3d/format71.rs"]
#[allow(dead_code)]
mod format71;
#[path = "../src/v3d/texstate71.rs"]
mod texstate71;
#[path = "../src/v3d/texture71.rs"]
mod texture71;

use format71::*;
use texture71::{compose, GlTex71, Target, GL_ALPHA as GL_SWZ_ALPHA, GL_MAX_LEVEL_DEFAULT, GL_SWIZZLE_IDENTITY};

const ROOT: &str = env!("CARGO_MANIFEST_DIR");

fn read(rel: &str) -> String {
    std::fs::read_to_string(format!("{ROOT}/{rel}")).unwrap_or_else(|e| panic!("{rel}: {e}"))
}

const R: u8 = SWIZZLE_RED;
const ONE: u8 = SWIZZLE_ONE;

fn tex(internal_format: u32, ty: u32) -> GlTex71 {
    GlTex71 {
        target: Target::Tex2d,
        internal_format,
        ty,
        width: 64,
        height: 64,
        levels: 1,
        base_level: 0,
        max_level: GL_MAX_LEVEL_DEFAULT,
        swizzle: GL_SWIZZLE_IDENTITY,
        srgb_decode: true,
        bo_address: 0xf000,
        ver: 71,
        iprev: 10,
    }
}

/// The swizzle column of a pipe format's FORMAT row in the vendored table.
fn table_swiz(pipe: &str) -> String {
    let t = read("third_party/mesa-26.2.1/src/gallium/drivers/v3d/v3dx_format_table.c");
    let key = format!("FORMAT({pipe},");
    let line = t.lines().find(|l| l.trim_start().starts_with(&key)).unwrap_or_else(|| panic!("{pipe}"));
    line.split(',').nth(3).unwrap().trim().trim_start_matches("SWIZ_").to_string()
}

// GL depth formats (sized, and unsized with their type)
const DEPTH_GL: [(u32, u32, &str); 7] = [
    (0x81A5, 0, "Z16_UNORM"),         // GL_DEPTH_COMPONENT16
    (0x81A6, 0, "X8Z24_UNORM"),       // GL_DEPTH_COMPONENT24
    (0x8CAC, 0, "Z32_FLOAT"),         // GL_DEPTH_COMPONENT32F
    (0x88F0, 0, "S8_UINT_Z24_UNORM"), // GL_DEPTH24_STENCIL8
    (GL_DEPTH_COMPONENT, GL_UNSIGNED_SHORT, "Z16_UNORM"),
    (GL_DEPTH_COMPONENT, GL_UNSIGNED_INT, "X8Z24_UNORM"),
    (GL_DEPTH_STENCIL, GL_UNSIGNED_INT_24_8, "S8_UINT_Z24_UNORM"),
];

#[test]
fn fix1_depth_reads_luminance_not_alpha() {
    for (gl, ty, pipe) in DEPTH_GL {
        let storage = tmu_format71(gl, ty).unwrap();
        assert_eq!(storage.pipe, pipe);
        // Mesa's row: XXXX (v3dx_format_table.c:149-153)
        assert_eq!(table_swiz(pipe), "XXXX", "{pipe}");
        assert_eq!(storage.swizzle, [R, R, R, R]);
        // fmt[base[i]] with the GLES depth mode's base swizzle X X X 1
        assert_eq!(depth_view_swizzle(pipe), Some([R, R, R, ONE]));
        let t = compose(&tex(gl, ty)).unwrap();
        assert_eq!(t.swizzle, [R, R, R, ONE], "{pipe}: (d, d, d, 1)");
        // GL's swizzle composes on top: alpha from GL_ALPHA is the 1
        let a = compose(&GlTex71 { swizzle: [GL_SWZ_ALPHA; 4], ..tex(gl, ty) }).unwrap();
        assert_eq!(a.swizzle, [ONE; 4], "{pipe}");
    }
    // the 32F_S8 row is a depth row too (its texture is still refused by
    // texture71, a separate-stencil buffer)
    assert_eq!(depth_view_swizzle("Z32_FLOAT_S8X24_UINT"), Some([R, R, R, ONE]));
    // every colour row: the base-format swizzle changes nothing (the row
    // already has its base format's 0s and 1s)
    for m in MESA_ROWS.iter() {
        let row = row71(m.pipe);
        if pipe_is_depth(m.pipe) {
            continue;
        }
        assert_eq!(depth_view_swizzle(m.pipe), None, "{}", m.pipe);
        assert_eq!(view_swizzle71(&row), row.swizzle, "{}", m.pipe);
    }
    // the composition rule itself: out[i] = fmt[view[i]], 0 and 1 kept
    let zyxw = swizzle_of("ZYXW");
    assert_eq!(compose_swizzles(zyxw, [R, SWIZZLE_GREEN, SWIZZLE_ZERO, ONE]), [SWIZZLE_BLUE, SWIZZLE_GREEN, 0, ONE]);
}

#[test]
fn fix2_packed_depth_stencil_samples_as_x8z24() {
    let s8z24 = row71("S8_UINT_Z24_UNORM");
    let x8z24 = row71("X8Z24_UNORM");
    let sampled = sample_row71(s8z24);
    assert_eq!(sampled, x8z24);
    assert_eq!((sampled.pipe, sampled.mesa_line), ("X8Z24_UNORM", 150));
    // the record's words do not move: same type (DEPTH24_X8 = 24), swizzle
    // and return size as the storage row (v3dx_format_table.c:149-150)
    assert_eq!((s8z24.tex_type, s8z24.swizzle, s8z24.return_32), (x8z24.tex_type, x8z24.swizzle, x8z24.return_32));
    assert_eq!(x8z24.tex_type, 24);
    // every other row samples as stored
    for m in MESA_ROWS.iter().filter(|m| m.pipe != "S8_UINT_Z24_UNORM") {
        assert_eq!(sample_row71(row71(m.pipe)), row71(m.pipe), "{}", m.pipe);
    }
    // the texture: laid out as the 4-byte storage, sampled as X8Z24
    for (gl, ty) in [(0x88F0, 0), (GL_DEPTH_STENCIL, GL_UNSIGNED_INT_24_8)] {
        let t = compose(&tex(gl, ty)).unwrap();
        assert_eq!(t.row.pipe, "X8Z24_UNORM");
        assert_eq!(t.layout.cpp, 4);
        assert_eq!(t.tex_type, 24);
        assert!(t.return_32);
        // the same layout and record as a GL_DEPTH_COMPONENT24 texture
        let d24 = compose(&tex(0x81A6, 0)).unwrap();
        assert_eq!(t, d24);
    }
    // the other packed rows keep their storage size (Mesa's blocksize)
    for (pipe, cpp) in [("A4B4G4R4_UNORM", 2), ("A1B5G5R5_UNORM", 2), ("B5G6R5_UNORM", 2), ("Z16_UNORM", 2)] {
        assert_eq!(texture71::cpp_of(pipe), Ok(cpp), "{pipe}");
    }
}

/// The six words of a README record row (`| \`<shape>\` <job> | \`w0 .. w5\``).
fn readme_words(shape_job: &str) -> [u32; 6] {
    let md = read("tools/v3d/pi5/README.md");
    let key = format!("| `{shape_job}");
    let line = md.lines().find(|l| l.starts_with(&key)).unwrap_or_else(|| panic!("{shape_job}"));
    let cell = line.split('|').nth(2).unwrap().trim().trim_matches('`');
    let w: Vec<u32> =
        cell.split_whitespace().map(|h| u32::from_str_radix(h.trim_start_matches("0x"), 16).unwrap()).collect();
    w.try_into().unwrap()
}

fn packed(t: &texture71::Tex71) -> [u32; 6] {
    texstate71::pack_words(&texstate71::TexState {
        base: t.base,
        width: t.width,
        height: t.height,
        depth: t.depth,
        base_level: t.base_level,
        max_level: t.max_level,
        transfer_func: t.transfer_func,
        r_b_swap: t.r_b_swap,
        reverse: t.reverse,
        array_stride_64: t.array_stride_64,
    })
}

#[test]
fn fix4_transfer_and_rb_swap_per_revision() {
    // the bit positions of the 7.1.5+ record (v3d_packet.xml:1686-1704)
    let xml = read("third_party/mesa-26.2.1/src/broadcom/cle/v3d_packet.xml");
    let s = xml.find("<struct name=\"Texture Shader State\" min_ver=\"71\">").unwrap();
    let block = &xml[s..s + xml[s..].find("</struct>").unwrap()];
    assert!(block.contains("<field name=\"R/B swap\" size=\"1\" start=\"32\" type=\"bool\"/>"));
    assert!(block.contains("<field name=\"Reverse\" size=\"1\" start=\"5\" type=\"bool\"/>"));
    assert!(block.contains("<field name=\"Transfer func\" size=\"3\" start=\"2\" type=\"uint\">"));
    assert!(block.contains("<value name=\"Transfer Func sRGB\" value=\"1\"/>"));
    assert!(block.contains("V3D 7.1.2 doesn't have the RB swap bit"));

    // gl_texfmt job 1: GL_SRGB8_ALPHA8, transfer func 1 in word 0
    let srgb = 0x8C43;
    let row = tmu_format71(srgb, 0).unwrap();
    let bits = record_bits71(&row, true, 71, 10).unwrap();
    assert_eq!(bits, RecordBits71 { transfer_func: TRANSFER_FUNC_SRGB, r_b_swap: false, reverse: false });
    let mesa1 = readme_words("gl_texfmt` 1");
    let t1 = compose(&tex(srgb, 0)).unwrap();
    let w1 = packed(&t1);
    assert_eq!(w1[0], mesa1[0], "word 0: base | transfer func << 2");
    assert_eq!((mesa1[0] >> 2) & 7, TRANSFER_FUNC_SRGB);
    assert_eq!(w1[1], mesa1[1], "word 1: R/B swap (bit 32) and the array stride");
    assert_eq!(mesa1[1] & 1, 0);

    // gl_texfmt job 2: GL_RGB565, transfer 0, R/B swap 0 though the
    // 5-6-5 row is B-in-low-bits (the swizzle carries it, not the bit)
    let rgb565 = 0x8D62;
    let mesa2 = readme_words("gl_texfmt` 2");
    let t2 = compose(&GlTex71 { bo_address: 0x1b000, ..tex(rgb565, 0) }).unwrap();
    let w2 = packed(&t2);
    assert_eq!((w2[0], w2[1]), (mesa2[0], mesa2[1]));
    assert_eq!((mesa2[0] >> 2) & 7, TRANSFER_FUNC_NONE);

    // SKIP_DECODE: the view's format is linear (st_sampler_view.c:359-361)
    assert_eq!(record_bits71(&row, false, 71, 10).unwrap().transfer_func, TRANSFER_FUNC_NONE);
    // no row of the table sets R/B swap or reverse at any revision with the
    // bit; sRGB exactly for the sRGB pipe formats
    for m in MESA_ROWS.iter() {
        let r = row71(m.pipe);
        for iprev in [5, 10, 15] {
            let b = record_bits71(&r, true, 71, iprev).unwrap();
            assert!(!b.r_b_swap && !b.reverse, "{}", m.pipe);
            assert_eq!(b.transfer_func == TRANSFER_FUNC_SRGB, m.pipe.contains("SRGB"), "{}", m.pipe);
        }
    }
    // the revisions without this layout are named
    assert_eq!(record_bits71(&row, true, 71, 2), Err(Revision71::BelowIprev5 { iprev: 2 }));
    assert_eq!(record_bits71(&row, true, 42, 10), Err(Revision71::NotV3d71 { ver: 42 }));
    // and texture71 refuses them through texstate71's names
    assert_eq!(
        compose(&GlTex71 { iprev: 4, ..tex(srgb, 0) }).unwrap_err(),
        texture71::Refusal::State(texstate71::Refusal::RevisionBelow715 { iprev: 4 })
    );
}
