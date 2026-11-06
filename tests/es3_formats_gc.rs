// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GC7000UL (i.MX8MP): the texture descriptor of every ES 3.0 storage class
//! and integer samplers (item formats-gc, docs/es3/gc-formats.md).
//!
//! - The table of docs/es3/gc-formats.md section 2, read out of the file:
//!   one row per `GLES_TEXFMT_*` class of src/backend.rs, each with a Mesa
//!   citation, and each row's TE format, swizzle, CONFIG2 sign bit,
//!   INT_FILTER and mask column equal to what the back end writes
//!   (`vivante::texture::{gc_texfmt, format_words}`) and to the rnndb
//!   header's values, read by name (driver/src/vivante/hw/common_3d.xml.h),
//!   so a wrong constant in src/vivante/regs.rs, or a stale doc row, fails.
//! - The binding-dependent bits of the same words (no alpha, depth as
//!   luminance, a PE-order texture, a 2D array).
//! - The online compiler: an `isampler*` / `usampler*` look-up is the float
//!   look-up of its shape, the integers left unconverted, as Mesa's etnaviv
//!   emits it; the shapes the float path refuses are still refused.
//!
//! What it cannot see: the GPU. The eight decoded classes are in
//! `GC_SAMPLED_TEXFMTS` (src/backend.rs; the doc's `in mask` column).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-vivante --test es3_formats_gc
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online --test es3_formats_gc
//! ```

#![cfg(all(feature = "host-test", feature = "gpu-vivante", not(feature = "report-es2")))]

use yakogl::backend::*;
use yakogl::vivante::regs::*;
use yakogl::vivante::texture::{format_words, gc_texfmt};

/* Compiled in, so the test binary (and the check's test cache, keyed on it)
 * changes with each of them. */
const DOC: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/docs/es3/gc-formats.md"));
const COMMON_3D: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/driver/src/vivante/hw/common_3d.xml.h"));
const BACKEND_RS: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/backend.rs"));

/// The value of `#define <name>` in the vendored common_3d.xml.h.
fn rnndb(name: &str) -> u32 {
    let line = COMMON_3D
        .lines()
        .find(|l| l.starts_with("#define") && l.split_whitespace().nth(1) == Some(name))
        .unwrap_or_else(|| panic!("{name} is in common_3d.xml.h"));
    let v = line.split_whitespace().nth(2).expect("a value");
    u32::from_str_radix(v.trim_start_matches("0x"), 16).expect("hex")
}

/// Every storage class by name.
const CLASSES: [(&str, u8); 30] = [
    ("GLES_TEXFMT_RGBA8", GLES_TEXFMT_RGBA8),
    ("GLES_TEXFMT_ETC2_RGB", GLES_TEXFMT_ETC2_RGB),
    ("GLES_TEXFMT_ETC2_RGBA", GLES_TEXFMT_ETC2_RGBA),
    ("GLES_TEXFMT_ETC2_RGB_PT", GLES_TEXFMT_ETC2_RGB_PT),
    ("GLES_TEXFMT_RGB565", GLES_TEXFMT_RGB565),
    ("GLES_TEXFMT_RGBA16F", GLES_TEXFMT_RGBA16F),
    ("GLES_TEXFMT_R16F", GLES_TEXFMT_R16F),
    ("GLES_TEXFMT_R32F", GLES_TEXFMT_R32F),
    ("GLES_TEXFMT_L8", GLES_TEXFMT_L8),
    ("GLES_TEXFMT_A8", GLES_TEXFMT_A8),
    ("GLES_TEXFMT_LA8", GLES_TEXFMT_LA8),
    ("GLES_TEXFMT_R11G11B10F", GLES_TEXFMT_R11G11B10F),
    ("GLES_TEXFMT_RGB9E5", GLES_TEXFMT_RGB9E5),
    ("GLES_TEXFMT_RGB10A2", GLES_TEXFMT_RGB10A2),
    ("GLES_TEXFMT_RGB10A2UI", GLES_TEXFMT_RGB10A2UI),
    ("GLES_TEXFMT_RGBA8_SNORM", GLES_TEXFMT_RGBA8_SNORM),
    ("GLES_TEXFMT_RGBA8UI", GLES_TEXFMT_RGBA8UI),
    ("GLES_TEXFMT_RGBA8I", GLES_TEXFMT_RGBA8I),
    ("GLES_TEXFMT_RGBA16UI", GLES_TEXFMT_RGBA16UI),
    ("GLES_TEXFMT_RGBA16I", GLES_TEXFMT_RGBA16I),
    ("GLES_TEXFMT_RGBA32UI", GLES_TEXFMT_RGBA32UI),
    ("GLES_TEXFMT_RGBA32I", GLES_TEXFMT_RGBA32I),
    ("GLES_TEXFMT_RGBA32F", GLES_TEXFMT_RGBA32F),
    ("GLES_TEXFMT_SRGB8_A8", GLES_TEXFMT_SRGB8_A8),
    ("GLES_TEXFMT_ETC2_SRGB_RGB", GLES_TEXFMT_ETC2_SRGB_RGB),
    ("GLES_TEXFMT_ETC2_SRGB_RGBA", GLES_TEXFMT_ETC2_SRGB_RGBA),
    ("GLES_TEXFMT_R32UI", GLES_TEXFMT_R32UI),
    ("GLES_TEXFMT_R32I", GLES_TEXFMT_R32I),
    ("GLES_TEXFMT_RG32UI", GLES_TEXFMT_RG32UI),
    ("GLES_TEXFMT_RG32I", GLES_TEXFMT_RG32I),
];

/// One row of the doc's table.
struct Row {
    class: String,
    te: String,
    swizzle: String,
    config2: String,
    int_filter: String,
    in_mask: String,
    mesa: String,
}

fn rows() -> Vec<Row> {
    DOC.lines()
        .filter(|l| l.starts_with("| `GLES_TEXFMT_"))
        .map(|l| {
            let c: Vec<&str> = l.trim().trim_matches('|').split('|').map(str::trim).collect();
            assert_eq!(c.len(), 11, "eleven columns: {l}");
            let unquote = |s: &str| s.trim_matches('`').to_string();
            Row {
                class: unquote(c[0]),
                te: unquote(c[1]),
                swizzle: c[2].to_string(),
                config2: unquote(c[3]),
                int_filter: c[4].to_string(),
                in_mask: c[6].to_string(),
                mesa: c[10].to_string(),
            }
        })
        .collect()
}

fn id_of(class: &str) -> u8 {
    CLASSES.iter().find(|(n, _)| *n == class).unwrap_or_else(|| panic!("{class} is a storage class")).1
}

/// The four swizzle fields of CONFIG1 as letters.
fn swizzle_letters(config1: u32) -> String {
    [8, 12, 16, 20]
        .iter()
        .map(|s| match (config1 >> s) & 7 {
            0 => 'R',
            1 => 'G',
            2 => 'B',
            3 => 'A',
            4 => '0',
            5 => '1',
            x => panic!("swizzle {x}"),
        })
        .collect()
}

fn bind(format: u8) -> GlesTextureBind {
    GlesTextureBind { format, width: 4, height: 4, stride_texels: 4, mip_levels: 1, layers: 1, ..GlesTextureBind::ZERO }
}

/// src/backend.rs names exactly the classes above, and the doc has one row
/// for each, with a Mesa citation.
#[test]
fn every_storage_class_has_one_row_with_a_mesa_citation() {
    let mut declared: Vec<&str> = BACKEND_RS
        .lines()
        .filter_map(|l| l.strip_prefix("pub const GLES_TEXFMT_"))
        .map(|rest| rest.split(':').next().unwrap())
        .collect();
    declared.sort_unstable();
    let mut known: Vec<&str> = CLASSES.iter().map(|(n, _)| n.trim_start_matches("GLES_TEXFMT_")).collect();
    known.sort_unstable();
    assert_eq!(declared, known, "src/backend.rs's GLES_TEXFMT_* classes are this test's");

    let rows = rows();
    for (name, _) in CLASSES {
        let n = rows.iter().filter(|r| r.class == name).count();
        assert_eq!(n, 1, "{name}: one row in docs/es3/gc-formats.md");
    }
    assert_eq!(rows.len(), CLASSES.len(), "no row for anything else");
    for r in &rows {
        /* a cited Mesa file and line: `F:155`, `D:187-189`, ... */
        let cited = r.mesa.split('`').skip(1).step_by(2).any(|c| {
            c.split_once(':').is_some_and(|(f, l)| !f.is_empty() && l.starts_with(|ch: char| ch.is_ascii_digit()))
        });
        assert!(cited, "{}: a Mesa citation in `{}`", r.class, r.mesa);
    }
}

/// Each row's TE format, swizzle, CONFIG2 and INT_FILTER are the descriptor
/// words the back end builds, by the rnndb header's values.
#[test]
fn every_row_is_the_descriptor_the_back_end_builds() {
    for r in rows() {
        let id = id_of(&r.class);
        let f = gc_texfmt(id);
        let name = &r.class;
        if r.te == "none" {
            assert!(!f.decoded, "{name}: no TE format, not decoded");
            continue;
        }
        assert!(f.decoded, "{name}: decoded");
        let v = rnndb(&r.te);
        if r.te.starts_with("TEXTURE_FORMAT_EXT_") {
            assert_eq!((f.format, f.format_ext), (0, v), "{name}: CONFIG1 FORMAT_EXT {}", r.te);
        } else {
            assert_eq!((f.format, f.format_ext), (v, 0), "{name}: CONFIG0 FORMAT {}", r.te);
        }
        let tiled = f.compressed;
        let [c0, c1, c2] = format_words(&bind(id), tiled);
        assert_eq!((c0 >> 13) & 0x1f, f.format, "{name}: CONFIG0 FORMAT field");
        assert_eq!(c1 & 0x3f, f.format_ext, "{name}: CONFIG1 FORMAT_EXT field");
        assert_eq!(c0 & 7, GC_TEX_TYPE_2D, "{name}: 2D");
        let addressing = (c0 >> 20) & 3;
        assert_eq!(addressing, if tiled { GC_TEX_ADDRESSING_TILED } else { GC_TEX_ADDRESSING_LINEAR }, "{name}");
        assert_eq!(swizzle_letters(c1), r.swizzle, "{name}: swizzle");
        let sign = if r.config2 == "-" { 0 } else { rnndb(&format!("TE_SAMPLER_CONFIG2_{}", r.config2)) };
        assert_eq!(c2, GC_TEXDESC_CONFIG2_BASE | sign, "{name}: CONFIG2 {}", r.config2);
        assert_eq!(f.config2, sign, "{name}");
        assert_eq!(f.int_filter, r.int_filter == "yes", "{name}: INT_FILTER {}", r.int_filter);
    }
}

/// The `in mask` column is GC_SAMPLED_TEXFMTS, and the mask holds no class
/// the descriptor cannot decode.
#[test]
fn the_mask_column_is_gc_sampled_texfmts() {
    for r in rows() {
        let id = id_of(&r.class);
        let in_mask = GC_SAMPLED_TEXFMTS & gles_texfmt_bit(id) != 0;
        assert_eq!(in_mask, r.in_mask == "yes", "{}: in GC_SAMPLED_TEXFMTS", r.class);
        if in_mask {
            assert!(gc_texfmt(id).decoded, "{}: sampled, so decoded", r.class);
        }
    }
}

/// The GL core's R32I/UI and RG32I/UI classes (26-29) are in no back end's
/// mask yet: the four formats fall to their RGBA32 class, which the GC7000UL
/// lacks too, so its build still refuses them at upload; the PowerVR and the
/// V3D store them in RGBA32UI/I, the Mali refuses them. The native class
/// holds the first 4 / 8 bytes of the RGBA32 texel of the same source.
#[test]
fn the_r32_and_rg32_integer_classes_are_in_no_mask_yet() {
    use yakogl::gl::consts::*;
    use yakogl::gl::es3_formats::{convert_row, find, storage_bytes};
    let rows = [
        (GL_R32UI, GL_RED_INTEGER, GL_UNSIGNED_INT, GLES_TEXFMT_R32UI, GLES_TEXFMT_RGBA32UI, 4),
        (GL_R32I, GL_RED_INTEGER, GL_INT, GLES_TEXFMT_R32I, GLES_TEXFMT_RGBA32I, 4),
        (GL_RG32UI, GL_RG_INTEGER, GL_UNSIGNED_INT, GLES_TEXFMT_RG32UI, GLES_TEXFMT_RGBA32UI, 8),
        (GL_RG32I, GL_RG_INTEGER, GL_INT, GLES_TEXFMT_RG32I, GLES_TEXFMT_RGBA32I, 8),
    ];
    for (name, base, type_, native, wide, bytes) in rows {
        let e = find(name).expect("an ES 3.0 format").1;
        assert_eq!(e.storage, [native, wide], "{}: the native class, then the RGBA32 fallback", e.name);
        assert_eq!(gles_texfmt_bytes(native), bytes, "{}", e.name);
        assert_eq!(storage_bytes(native), bytes as usize, "{}", e.name);
        for mask in [GC_SAMPLED_TEXFMTS, PVR_SAMPLED_TEXFMTS, V3D_SAMPLED_TEXFMTS] {
            assert_eq!(mask & gles_texfmt_bit(native), 0, "{}: no mask has class {native}", e.name);
        }
        assert_eq!(e.storage_in(GC_SAMPLED_TEXFMTS), None, "{}: GC7000UL, refused as before", e.name);
        assert_eq!(e.storage_in(GC_SAMPLED_TEXFMTS | gles_texfmt_bit(wide)), Some(wide), "{}", e.name);
        assert_eq!(e.storage_in(PVR_SAMPLED_TEXFMTS), Some(wide), "{}: PowerVR", e.name);
        assert_eq!(e.storage_in(V3D_SAMPLED_TEXFMTS), Some(wide), "{}: V3D", e.name);
        assert_eq!(e.storage_in(0), None, "{}: Mali (mask 0)", e.name);
        assert_eq!(e.storage_in(gles_texfmt_bit(native) | gles_texfmt_bit(wide)), Some(native), "{}", e.name);

        let src: Vec<u8> =
            [0x8000_0001u32, 0xffff_fffbu32][..e.comps as usize].iter().flat_map(|w| w.to_ne_bytes()).collect();
        let mut packed = vec![0xaau8; bytes as usize];
        let mut rgba = vec![0xaau8; 16];
        assert!(convert_row(e, native, &src, base, type_, 1, &mut packed), "{}", e.name);
        assert!(convert_row(e, wide, &src, base, type_, 1, &mut rgba), "{}", e.name);
        assert_eq!(packed[..], src[..], "{}: the source words", e.name);
        assert_eq!(packed[..], rgba[..bytes as usize], "{}: a prefix of the RGBA32 texel", e.name);
    }
}

/// The constants regs.rs adds for this item are the header's.
#[test]
fn the_new_register_constants_are_the_rnndb_values() {
    for (v, name) in [
        (GC_TEX_FORMAT_E5B9G9R9, "TEXTURE_FORMAT_E5B9G9R9"),
        (GC_TEX_FORMAT_EXT_A2B10G10R10, "TEXTURE_FORMAT_EXT_A2B10G10R10"),
        (GC_TEX_FORMAT_EXT_A8B8G8R8_SNORM, "TEXTURE_FORMAT_EXT_A8B8G8R8_SNORM"),
        (GC_TEX_FORMAT_EXT_A8B8G8R8I, "TEXTURE_FORMAT_EXT_A8B8G8R8I"),
        (GC_TEX_FORMAT_EXT_A16B16G16R16I, "TEXTURE_FORMAT_EXT_A16B16G16R16I"),
        (GC_TEX_FORMAT_EXT_B10G11R11F, "TEXTURE_FORMAT_EXT_B10G11R11F"),
        (GC_TEX_FORMAT_EXT_A2B10G10R10UI, "TEXTURE_FORMAT_EXT_A2B10G10R10UI"),
        (GC_TEXDESC_CONFIG2_SIGNED_INT8, "TE_SAMPLER_CONFIG2_SIGNED_INT8"),
        (GC_TEXDESC_CONFIG2_SIGNED_INT16, "TE_SAMPLER_CONFIG2_SIGNED_INT16"),
    ] {
        assert_eq!(v, rnndb(name), "{name}");
    }
}

/// What the binding adds to the same words, for an old and a new class
/// alike: no alpha reads 1, a depth texture's G and B read R, a PE-order
/// RGBA8 texture is A8R8G8B8 (and only RGBA8 is), a 2D array is a 3D type
/// with the array bit.
#[test]
fn the_binding_bits_apply_to_every_class() {
    for (name, id) in CLASSES {
        if !gc_texfmt(id).decoded {
            continue;
        }
        /* no gc_texfmt arm yet (r32-rg32-classes adds it with the mask bits):
         * they fall to the RGBA8 default, which no texture reaches while
         * GC_SAMPLED_TEXFMTS lacks them */
        if matches!(id, GLES_TEXFMT_R32UI | GLES_TEXFMT_R32I | GLES_TEXFMT_RG32UI | GLES_TEXFMT_RG32I) {
            assert_eq!(GC_SAMPLED_TEXFMTS & gles_texfmt_bit(id), 0, "{name}");
            continue;
        }
        let base = format_words(&bind(id), false);
        let rgb = format_words(&GlesTextureBind { no_alpha: 1, ..bind(id) }, false);
        assert_eq!(&swizzle_letters(rgb[1])[3..], "1", "{name}: no alpha reads 1");
        let lum = format_words(&GlesTextureBind { depth_luminance: 1, ..bind(id) }, false);
        assert_eq!(&swizzle_letters(lum[1])[1..3], "RR", "{name}: luminance");
        let arr = format_words(&GlesTextureBind { type_: GLES_TEXTYPE_2D_ARRAY, layers: 3, ..bind(id) }, false);
        assert_eq!(arr[0] & 7, GC_TEX_TYPE_3D, "{name}: an array is a volume");
        assert_ne!(arr[1] & GC_TEXDESC_TEXTURE_ARRAY, 0, "{name}: with the array bit");
        let bgra = format_words(&GlesTextureBind { bgra: 1, ..bind(id) }, false);
        if id == GLES_TEXFMT_RGBA8 {
            assert_eq!((bgra[0] >> 13) & 0x1f, GC_TEX_FORMAT_A8R8G8B8, "PE-order RGBA8");
        } else if id != GLES_TEXFMT_SRGB8_A8 {
            assert_eq!(bgra, base, "{name}: bgra is about RGBA8 bytes only");
        }
    }
}

/* ---- 128-bit classes: two G32R32 planes, kept inert ------------------------- */

/// The back-end half of gc-formats.md section 3
/// (docs/notes/gc-128bit-texture-sampling.md): each 128-bit class has a
/// G32R32 plane format, the header's TEXTURE_FORMAT_EXT_G32R32F for RGBA32F
/// and TEXTURE_FORMAT_EXT_G32R32I for both integer classes (Mesa
/// etnaviv_format.c:229-233), and the descriptor words of either plane are
/// those: no CONFIG0 format, the extended one in CONFIG1, TILED, the
/// binding's swizzle, no sign bit and no INT_FILTER. The class itself stays
/// undecoded and out of GC_SAMPLED_TEXFMTS (its doc row says `none`) until
/// the compiler joins the two look-ups.
#[test]
fn a_128_bit_class_is_a_pair_of_g32r32_plane_descriptors_behind_the_closed_mask() {
    use yakogl::vivante::texture::{is_wide128, wide128_plane_format};
    assert_eq!(GC_TEX_FORMAT_EXT_G32R32F, rnndb("TEXTURE_FORMAT_EXT_G32R32F"));
    assert_eq!(GC_TEX_FORMAT_EXT_G32R32I, rnndb("TEXTURE_FORMAT_EXT_G32R32I"));
    for (name, id) in CLASSES {
        let wide = matches!(id, GLES_TEXFMT_RGBA32F | GLES_TEXFMT_RGBA32UI | GLES_TEXFMT_RGBA32I);
        assert_eq!(is_wide128(id), wide, "{name}");
        assert_eq!(wide128_plane_format(id).is_some(), wide, "{name}: a plane format for 128-bit classes only");
        if !wide {
            continue;
        }
        assert!(!gc_texfmt(id).decoded, "{name}: the class itself is still not decoded");
        assert_eq!(GC_SAMPLED_TEXFMTS & gles_texfmt_bit(id), 0, "{name}: and not sampled");
        let plane = wide128_plane_format(id).unwrap();
        let want =
            rnndb(if id == GLES_TEXFMT_RGBA32F { "TEXTURE_FORMAT_EXT_G32R32F" } else { "TEXTURE_FORMAT_EXT_G32R32I" });
        assert_eq!((plane.format, plane.format_ext, plane.config2, plane.int_filter), (0, want, 0, false), "{name}");
        let [c0, c1, c2] = format_words(&bind(id), true);
        assert_eq!((c0 >> 13) & 0x1f, 0, "{name}: no CONFIG0 format");
        assert_eq!(c1 & 0x3f, want, "{name}: CONFIG1 FORMAT_EXT");
        assert_eq!((c0 >> 20) & 3, GC_TEX_ADDRESSING_TILED, "{name}: the planes are tiled");
        assert_eq!(swizzle_letters(c1), "RGBA", "{name}: the binding's swizzle, in both planes");
        assert_eq!(c2, GC_TEXDESC_CONFIG2_BASE, "{name}: no sign bit for 32-bit channels");
    }
}

/// The companion-slot policy: Mesa's unified packing on this core
/// (etnaviv_texture.c:301-322), the stage's k-th 128-bit binding in unit
/// order at `base + count + k`, after every slot the stage's own units take,
/// so no GL unit is ever displaced; nothing past the 32-slot array the two
/// stages share (the binding then samples the black texel, by name).
#[test]
fn a_companion_follows_its_stages_own_slots_and_never_leaves_the_array() {
    use yakogl::vivante::texture::{companion_slot, GC_HW_SAMPLER_SLOTS};
    assert_eq!(GC_HW_SAMPLER_SLOTS, 32);
    assert_eq!(companion_slot(0, 1, 0), Some(1), "one fragment unit: its companion is slot 1");
    assert_eq!(companion_slot(0, 3, 1), Some(4), "the second 128-bit unit of three");
    assert_eq!(companion_slot(5, 2, 0), Some(7), "a vertex stage at 5 with two units");
    assert_eq!(companion_slot(0, 16, 15), Some(31), "sixteen fragment units, all 128-bit, fill the array");
    assert_eq!(companion_slot(16, 16, 0), None, "past slot 31");
}

/// Plane 1 of a level starts half-way into it, after plane 0's layers of
/// `align4(w) * align4(h) * 8` bytes (Mesa etnaviv_resource.h:91-94).
#[test]
fn the_second_plane_starts_half_way_into_each_level() {
    use yakogl::vivante::tiled_copy::{copy_bpp, second_plane_offset, tiled_layout};
    let t = GlesTextureBind { width: 13, height: 6, layers: 4, mip_levels: 4, ..bind(GLES_TEXFMT_RGBA32UI) };
    let (off, total) = tiled_layout(13, 6, 4, 4, 16);
    let planes = [16 * 8 * 8 * 4, 8 * 4 * 8 * 4, 4 * 4 * 8 * 4, 4 * 4 * 8 * 4];
    for (l, p) in planes.iter().enumerate() {
        assert_eq!(second_plane_offset(&t, l as u32), *p, "level {l}");
        let end = if l < 3 { off[l + 1] } else { total };
        assert_eq!(end - off[l], 2 * p, "level {l}: the two planes");
    }
    assert_eq!(copy_bpp(&t), 8);
    assert_eq!(copy_bpp(&bind(GLES_TEXFMT_RGBA16F)), 8);
    assert_eq!(copy_bpp(&bind(GLES_TEXFMT_RGBA8)), 4);
}

/* ---- the compiler's half: integer look-ups ---------------------------------- */

#[cfg(feature = "shaderc-online")]
mod compiler {
    use std::sync::Mutex;
    use yakogl::shaderc::online::backend::{forget_compiled, SHC_ONLINE};
    use yakogl::shaderc::online::vivante::{decode, listing};
    use yakogl::shaderc::{ShcShader, SHC_STAGE_FRAGMENT, SHC_STAGE_VERTEX};

    /// Held across the compile AND the read of its code: `forget_compiled`
    /// resets the arena the code lives in (src/test_rules.rs, the third
    /// family), so the listing is taken before the guard drops.
    static COMPILING: Mutex<()> = Mutex::new(());

    /// The listing of `src`, one line an instruction, or the refusal's log.
    fn compile(stage: u32, src: &str) -> Result<Vec<String>, String> {
        let _serial = COMPILING.lock().unwrap_or_else(|e| e.into_inner());
        forget_compiled();
        let text = std::ffi::CString::new(src).unwrap();
        let mut sh = ShcShader::ZERO;
        let mut log = [0 as core::ffi::c_char; 512];
        let compile = SHC_ONLINE.compile.expect("the online back end compiles");
        // SAFETY: a NUL-terminated source, a real output, and a log of the size announced.
        let rc = unsafe { compile(stage, text.as_ptr(), &mut sh, log.as_mut_ptr(), log.len()) };
        let log: String = log.iter().take_while(|c| **c != 0).map(|c| *c as u8 as char).collect();
        if rc != 0 {
            return Err(log);
        }
        // SAFETY: the record's code, `usc_dw` dwords, alive until the next compile.
        let words = unsafe { core::slice::from_raw_parts(sh.usc, sh.usc_dw as usize) };
        Ok(words
            .chunks(4)
            .enumerate()
            .map(|(i, c)| {
                let ins = decode::decode_instr(&c.try_into().unwrap()).expect("decodes");
                let mut t = String::new();
                listing::instr_text(&mut t, i, &ins).unwrap();
                t.split_whitespace().skip(5).collect::<Vec<_>>().join(" ")
            })
            .collect())
    }

    fn ops(listing: &[String]) -> Vec<&str> {
        listing.iter().map(|l| l.split_whitespace().next().unwrap_or("")).collect()
    }

    fn fs(decl: &str, out: &str, body: &str) -> String {
        format!("#version 300 es\nprecision highp float;\n{decl}\nlayout(location = 0) out highp {out} o;\nvoid main() {{ {body} }}\n")
    }

    /// `usampler2D` into a `uvec4` output: the look-up and nothing else,
    /// the float `texld` of the same shape (no conversion either way).
    #[test]
    fn an_integer_lookup_is_the_float_texld_unconverted() {
        let float =
            compile(SHC_STAGE_FRAGMENT, &fs("uniform sampler2D s;\nin vec2 uv;", "vec4", "o = texture(s, uv);"))
                .expect("compiles");
        for (sampler, out) in [("usampler2D", "uvec4"), ("isampler2D", "ivec4")] {
            let src = fs(&format!("uniform highp {sampler} s;\nin vec2 uv;"), out, "o = texture(s, uv);");
            let got = compile(SHC_STAGE_FRAGMENT, &src).unwrap_or_else(|e| panic!("{sampler}: {e}"));
            assert_eq!(got, float, "{sampler}: the same instruction as sampler2D");
        }
    }

    /// The integers are what the shader computes with: integer arithmetic
    /// on the result, and one conversion where the shader asks for a float.
    #[test]
    fn the_result_is_read_as_integers() {
        let add = compile(
            SHC_STAGE_FRAGMENT,
            &fs("uniform highp isampler2D s;\nin vec2 uv;", "ivec4", "o = texture(s, uv) + 1;"),
        )
        .unwrap();
        assert_eq!(ops(&add), ["texld", "add.s32"], "{add:?}");
        let conv = compile(
            SHC_STAGE_FRAGMENT,
            &fs("uniform highp usampler2D s;\nin vec2 uv;", "vec4", "o = vec4(texture(s, uv));"),
        )
        .unwrap();
        assert_eq!(ops(&conv), ["texld", "i2f.u32"], "{conv:?}");
    }

    /// The other shapes: textureLod on a 3D texture (texldl), a 2D array
    /// and a cube (texld), texelFetch (txf with Mesa's 0x1100,
    /// etnaviv_compiler_nir_emit.c:234-236), textureProj, in both stages.
    #[test]
    fn every_float_shape_selects_for_integer_samplers() {
        let cases = [
            ("uniform highp usampler3D s;\nin vec3 c;", "uvec4", "o = textureLod(s, c, 1.0);", "texldl"),
            ("uniform highp isampler2DArray s;\nin vec3 c;", "ivec4", "o = texture(s, c);", "texld"),
            ("uniform highp usamplerCube s;\nin vec3 c;", "uvec4", "o = texture(s, c);", "texld"),
            ("uniform highp isampler2D s;\nin vec3 c;", "ivec4", "o = textureProj(s, c);", "texld"),
            ("uniform highp isampler2D s;", "ivec4", "o = texelFetch(s, ivec2(1, 2), 0);", "txf"),
            ("uniform highp usampler2D s;\nin vec2 c;", "uvec4", "o = texture(s, c, 1.0);", "texldb"),
        ];
        for (decl, out, body, op) in cases {
            let got = compile(SHC_STAGE_FRAGMENT, &fs(decl, out, body)).unwrap_or_else(|e| panic!("{body}: {e}"));
            assert!(ops(&got).contains(&op), "{body}: {op} in {got:?}");
            assert!(!ops(&got).iter().any(|o| o.starts_with("i2f") || o.starts_with("f2i")), "{body}: {got:?}");
            if op == "txf" {
                assert!(got.iter().any(|l| l.contains("4352")), "txf's 0x1100: {got:?}");
            }
        }
        let vs = "#version 300 es\nuniform highp usampler2D s;\nin vec4 p;\nflat out highp uvec4 v;\n\
                  void main() { v = textureLod(s, p.xy, 0.0); gl_Position = p; }\n";
        let got = compile(SHC_STAGE_VERTEX, vs).expect("a vertex-stage integer look-up compiles");
        assert!(ops(&got).contains(&"texldl"), "{got:?}");
    }

    /// What the float path does not select either is still refused by name.
    #[test]
    fn an_offset_lookup_is_still_refused_by_name() {
        let e = compile(
            SHC_STAGE_FRAGMENT,
            &fs("uniform highp isampler2D s;\nin vec2 uv;", "ivec4", "o = textureOffset(s, uv, ivec2(1, 0));"),
        )
        .expect_err("refused");
        assert!(e.contains("not selected yet"), "{e}");
    }
}
