// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GC7000UL (i.MX8MP): every texture-format constant of src/vivante/regs.rs
//! (`GC_TEX_FORMAT_*`) is the word Mesa's etnaviv assigns to the pipe
//! formats it stands for, read by pipe-format name out of Mesa 26.2.1
//! src/gallium/drivers/etnaviv/etnaviv_format.c (item gc-mesa-fmt-table).
//!
//! - The `formats[]` rows (`VT` / `_T` / `V_`) are parsed by pipe format,
//!   their symbolic `TEXTURE_FORMAT_*` words valued by name from the rnndb
//!   header (driver/src/vivante/hw/common_3d.xml.h), and the HALTI5
//!   override of translate_texture_format applied (the GC7000UL is HALTI5),
//!   so each pinned pipe format gives the regs.rs constant, in the right
//!   field (CONFIG0, or CONFIG1 FORMAT_EXT for the `| EXT_FORMAT` words).
//! - Every `F:<line>` / `etnaviv_format.c:<line>` citation in a constant's
//!   doc comment is a line listed here, and that line of Mesa's file holds
//!   the pipe format and word listed with it: a moved or wrong citation, or
//!   a new one nobody checked, fails.
//!
//! etnaviv_format.c is not vendored under third_party/ (only Mesa's v3d
//! driver is, third_party/mesa-26.2.1/UPSTREAM), so the lines read are kept
//! below byte for byte with their line numbers: `MESA_TABLE` is lines
//! 86-294 (the table and translate_texture_format), `MESA_REMAP` lines
//! 417-425 (remap_texture_format_rb_swap). The file of the mesa-26.2.1
//! tarball (commit da14d65e4499e66468094be52bff9ea0915a695e) has sha256
//! 4b30a18d0442b0c6fdad28bd6ec296e512220292b452efbfcf831ba89a48507e;
//! `excerpt_is_upstream` compares the copy against a Mesa tree:
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-vivante --test es3_gc_mesa_formats
//! YAKOGL_MESA_SRC=<mesa-26.2.1 tree> cargo test --no-default-features \
//!     --features host-test,gpu-vivante --test es3_gc_mesa_formats -- --ignored
//! ```

#![cfg(all(feature = "host-test", feature = "gpu-vivante", not(feature = "report-es2")))]

use std::collections::{BTreeMap, BTreeSet};

use yakogl::vivante::regs::*;

const COMMON_3D: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/driver/src/vivante/hw/common_3d.xml.h"));
const REGS_RS: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/vivante/regs.rs"));

/// Mesa 26.2.1 src/gallium/drivers/etnaviv/etnaviv_format.c, lines
/// `MESA_TABLE_FIRST`.. (Copyright (c) 2016 Etnaviv Project, MIT, the
/// licence of third_party/mesa-26.2.1/licenses/MIT).
const MESA_TABLE_FIRST: usize = 86;
const MESA_TABLE: &str = r#"static struct etna_format formats[PIPE_FORMAT_COUNT] = {
   /* 8-bit */
   VT(R8_UNORM,   UNSIGNED_BYTE, L8,                        R8),
   VT(R8_SNORM,   BYTE,          EXT_R8_SNORM | EXT_FORMAT, NONE),
   VT(R8_UINT,    BYTE_I,        EXT_R8I | EXT_FORMAT,      R8I),
   VT(R8_SINT,    BYTE_I,        EXT_R8I | EXT_FORMAT,      R8I),
   V_(R8_USCALED, UNSIGNED_BYTE, NONE),
   V_(R8_SSCALED, BYTE,          NONE),

   _T(A8_UNORM, A8, NONE),
   _T(L8_UNORM, L8, NONE),
   _T(I8_UNORM, I8, NONE),

   /* 16-bit */
   V_(R16_UNORM,   UNSIGNED_SHORT, NONE),
   V_(R16_SNORM,   SHORT,          NONE),
   VT(R16_UINT,    SHORT_I,        EXT_R16I | EXT_FORMAT, R16I),
   VT(R16_SINT,    SHORT_I,        EXT_R16I | EXT_FORMAT, R16I),
   V_(R16_USCALED, UNSIGNED_SHORT, NONE),
   V_(R16_SSCALED, SHORT,          NONE),
   VT(R16_FLOAT,   HALF_FLOAT,     EXT_R16F | EXT_FORMAT, R16F),

   _T(B4G4R4A4_UNORM, A4R4G4B4, A4R4G4B4),
   _T(B4G4R4X4_UNORM, X4R4G4B4, X4R4G4B4),

   _T(L8A8_UNORM, A8L8, NONE),

   _T(Z16_UNORM,      D16,      NONE),
   _T(B5G6R5_UNORM,   R5G6B5,   R5G6B5),
   _T(B5G5R5A1_UNORM, A1R5G5B5, A1R5G5B5),
   _T(B5G5R5X1_UNORM, X1R5G5B5, X1R5G5B5),

   VT(R8G8_UNORM,   UNSIGNED_BYTE,  EXT_G8R8 | EXT_FORMAT,       G8R8),
   VT(R8G8_SNORM,   BYTE,           EXT_G8R8_SNORM | EXT_FORMAT, NONE),
   VT(R8G8_UINT,    BYTE_I,         EXT_G8R8I | EXT_FORMAT,      G8R8I),
   VT(R8G8_SINT,    BYTE_I,         EXT_G8R8I | EXT_FORMAT,      G8R8I),
   V_(R8G8_USCALED, UNSIGNED_BYTE,  NONE),
   V_(R8G8_SSCALED, BYTE,           NONE),

   /* 24-bit */
   V_(R8G8B8_UNORM,   UNSIGNED_BYTE, NONE),
   V_(R8G8B8_SNORM,   BYTE,          NONE),
   V_(R8G8B8_UINT,    BYTE_I,        NONE),
   V_(R8G8B8_SINT,    BYTE_I,        NONE),
   V_(R8G8B8_USCALED, UNSIGNED_BYTE, NONE),
   V_(R8G8B8_SSCALED, BYTE,          NONE),

   /* 32-bit */
   V_(R32_UNORM,   UNSIGNED_INT, NONE),
   V_(R32_SNORM,   INT,          NONE),
   VT(R32_SINT,    FLOAT,        EXT_R32F | EXT_FORMAT, R32F),
   VT(R32_UINT,    FLOAT,        EXT_R32F | EXT_FORMAT, R32F),
   V_(R32_USCALED, UNSIGNED_INT, NONE),
   V_(R32_SSCALED, INT,          NONE),
   VT(R32_FLOAT,   FLOAT,        EXT_R32F | EXT_FORMAT, R32F),
   V_(R32_FIXED,   FIXED,        NONE),
   _T(Z32_FLOAT,   EXT_R32F | EXT_FORMAT, NONE),  /* emulated format */
   _T(Z32_FLOAT_S8X24_UINT,  EXT_R32F | EXT_FORMAT, NONE),  /* emulated format */

   V_(R16G16_UNORM,   UNSIGNED_SHORT, NONE),
   V_(R16G16_SNORM,   SHORT,          NONE),
   VT(R16G16_UINT,    SHORT_I,        EXT_G16R16I | EXT_FORMAT, G16R16I),
   VT(R16G16_SINT,    SHORT_I,        EXT_G16R16I | EXT_FORMAT, G16R16I),
   V_(R16G16_USCALED, UNSIGNED_SHORT, NONE),
   V_(R16G16_SSCALED, SHORT,          NONE),
   VT(R16G16_FLOAT,   HALF_FLOAT,     EXT_G16R16F | EXT_FORMAT, G16R16F),

   V_(A8B8G8R8_UNORM,   UNSIGNED_BYTE, NONE),

   VT(R8G8B8A8_UNORM,   UNSIGNED_BYTE, A8R8G8B8, A8B8G8R8),
   VT(R8G8B8A8_SNORM,   BYTE,          EXT_A8B8G8R8_SNORM | EXT_FORMAT, NONE),
   _T(R8G8B8X8_UNORM,   X8R8G8B8,      X8B8G8R8),
   _T(R8G8B8X8_SNORM,                  EXT_X8B8G8R8_SNORM | EXT_FORMAT, NONE),
   VT(R8G8B8A8_UINT,    BYTE_I,        EXT_A8B8G8R8I | EXT_FORMAT,      A8B8G8R8I),
   VT(R8G8B8A8_SINT,    BYTE_I,        EXT_A8B8G8R8I | EXT_FORMAT,      A8B8G8R8I),
   V_(R8G8B8A8_USCALED, UNSIGNED_BYTE, A8B8G8R8),
   V_(R8G8B8A8_SSCALED, BYTE,          A8B8G8R8),

   _T(B8G8R8A8_UNORM, A8R8G8B8, A8R8G8B8),
   _T(B8G8R8X8_UNORM, X8R8G8B8, X8R8G8B8),

   VT(R10G10B10A2_UNORM,   UNSIGNED_INT_2_10_10_10_REV, EXT_A2B10G10R10 | EXT_FORMAT, A2B10G10R10),
   _T(R10G10B10X2_UNORM,                                EXT_A2B10G10R10 | EXT_FORMAT, A2B10G10R10),
   V_(R10G10B10A2_SNORM,   INT_2_10_10_10_REV,          NONE),
   _T(R10G10B10A2_UINT,                                 EXT_A2B10G10R10UI | EXT_FORMAT, A2B10G10R10UI),
   V_(R10G10B10A2_USCALED, UNSIGNED_INT_2_10_10_10_REV, NONE),
   V_(R10G10B10A2_SSCALED, INT_2_10_10_10_REV,          NONE),

   V_(B10G10R10A2_UNORM,   UNSIGNED_INT_10_10_10_2,   NONE),
   V_(B10G10R10A2_SNORM,   INT_10_10_10_2,            NONE),
   V_(B10G10R10A2_USCALED, UNSIGNED_INT_10_10_10_2,   NONE),
   V_(B10G10R10A2_SSCALED, INT_10_10_10_2,            NONE),

   _T(X8Z24_UNORM,       D24X8, NONE),
   _T(S8_UINT_Z24_UNORM, D24X8, NONE),

   _T(S8_UINT,    EXT_R8I | EXT_FORMAT, NONE),
   _T(S8X24_UINT, EXT_D24S8 | EXT_FORMAT, NONE),
   _T(X32_S8X24_UINT, EXT_D24S8 | EXT_FORMAT, NONE),

   _T(R9G9B9E5_FLOAT,  E5B9G9R9,                    NONE),
   _T(R11G11B10_FLOAT, EXT_B10G11R11F | EXT_FORMAT, B10G11R11F),

   /* 48-bit */
   V_(R16G16B16_UNORM,   UNSIGNED_SHORT, NONE),
   V_(R16G16B16_SNORM,   SHORT,          NONE),
   V_(R16G16B16_UINT,    SHORT_I,        NONE),
   V_(R16G16B16_SINT,    SHORT_I,        NONE),
   V_(R16G16B16_USCALED, UNSIGNED_SHORT, NONE),
   V_(R16G16B16_SSCALED, SHORT,          NONE),
   V_(R16G16B16_FLOAT,   HALF_FLOAT,     NONE),

   /* 64-bit */
   V_(R16G16B16A16_UNORM,   UNSIGNED_SHORT, NONE),
   V_(R16G16B16A16_SNORM,   SHORT,          NONE),
   VT(R16G16B16A16_UINT,    SHORT_I,        EXT_A16B16G16R16I | EXT_FORMAT, A16B16G16R16I),
   VT(R16G16B16A16_SINT,    SHORT_I,        EXT_A16B16G16R16I | EXT_FORMAT, A16B16G16R16I),
   V_(R16G16B16A16_USCALED, UNSIGNED_SHORT, NONE),
   V_(R16G16B16A16_SSCALED, SHORT,          NONE),
   VT(R16G16B16A16_FLOAT,   HALF_FLOAT,     EXT_A16B16G16R16F | EXT_FORMAT, A16B16G16R16F),

   V_(R32G32_UNORM,   UNSIGNED_INT, NONE),
   V_(R32G32_SNORM,   INT,          NONE),
   VT(R32G32_UINT,    FLOAT,        EXT_G32R32F | EXT_FORMAT, G32R32F),
   VT(R32G32_SINT,    FLOAT,        EXT_G32R32F | EXT_FORMAT, G32R32F),
   V_(R32G32_USCALED, UNSIGNED_INT, NONE),
   V_(R32G32_SSCALED, INT,          NONE),
   VT(R32G32_FLOAT,   FLOAT,        EXT_G32R32F | EXT_FORMAT, G32R32F),
   V_(R32G32_FIXED,   FIXED,        NONE),

   /* 96-bit */
   V_(R32G32B32_UNORM,   UNSIGNED_INT, NONE),
   V_(R32G32B32_SNORM,   INT,          NONE),
   V_(R32G32B32_UINT,    FLOAT,        NONE),
   V_(R32G32B32_SINT,    FLOAT,        NONE),
   V_(R32G32B32_USCALED, UNSIGNED_INT, NONE),
   V_(R32G32B32_SSCALED, INT,          NONE),
   V_(R32G32B32_FLOAT,   FLOAT,        NONE),
   V_(R32G32B32_FIXED,   FIXED,        NONE),

   /* 128-bit */
   V_(R32G32B32A32_UNORM,   UNSIGNED_INT, NONE),
   V_(R32G32B32A32_SNORM,   INT,          NONE),
   VT(R32G32B32A32_UINT,    FLOAT,        EXT_G32R32I | EXT_FORMAT, G32R32F), /* emulated format */
   VT(R32G32B32A32_SINT,    FLOAT,        EXT_G32R32I | EXT_FORMAT, G32R32F), /* emulated format */
   V_(R32G32B32A32_USCALED, UNSIGNED_INT, NONE),
   V_(R32G32B32A32_SSCALED, INT,          NONE),
   VT(R32G32B32A32_FLOAT,   FLOAT,        EXT_G32R32F | EXT_FORMAT, G32R32F), /* emulated format */
   V_(R32G32B32A32_FIXED,   FIXED,        NONE),

   /* compressed */
   _T(ETC1_RGB8, ETC1, NONE),

   _T(DXT1_RGB,  DXT1,      NONE),
   _T(DXT1_RGBA, DXT1,      NONE),
   _T(DXT3_RGBA, DXT2_DXT3, NONE),
   _T(DXT5_RGBA, DXT4_DXT5, NONE),

   _T(ETC2_RGB8,       EXT_NONE | EXT_FORMAT,                          NONE), /* Extd. format NONE doubles as ETC2_RGB8 */
   _T(ETC2_RGB8A1,     EXT_RGB8_PUNCHTHROUGH_ALPHA1_ETC2 | EXT_FORMAT, NONE),
   _T(ETC2_RGBA8,      EXT_RGBA8_ETC2_EAC | EXT_FORMAT,                NONE),
   _T(ETC2_R11_UNORM,  EXT_R11_EAC | EXT_FORMAT,                       NONE),
   _T(ETC2_R11_SNORM,  EXT_SIGNED_R11_EAC | EXT_FORMAT,                NONE),
   _T(ETC2_RG11_UNORM, EXT_RG11_EAC | EXT_FORMAT,                      NONE),
   _T(ETC2_RG11_SNORM, EXT_SIGNED_RG11_EAC | EXT_FORMAT,               NONE),

   _T(ASTC_4x4,        ASTC_RGBA_4x4 | ASTC_FORMAT,                    NONE),
   _T(ASTC_5x4,        ASTC_RGBA_5x4 | ASTC_FORMAT,                    NONE),
   _T(ASTC_5x5,        ASTC_RGBA_5x5 | ASTC_FORMAT,                    NONE),
   _T(ASTC_6x5,        ASTC_RGBA_6x5 | ASTC_FORMAT,                    NONE),
   _T(ASTC_6x6,        ASTC_RGBA_6x6 | ASTC_FORMAT,                    NONE),
   _T(ASTC_8x5,        ASTC_RGBA_8x5 | ASTC_FORMAT,                    NONE),
   _T(ASTC_8x6,        ASTC_RGBA_8x6 | ASTC_FORMAT,                    NONE),
   _T(ASTC_8x8,        ASTC_RGBA_8x8 | ASTC_FORMAT,                    NONE),
   _T(ASTC_10x5,       ASTC_RGBA_10x5 | ASTC_FORMAT,                   NONE),
   _T(ASTC_10x6,       ASTC_RGBA_10x6 | ASTC_FORMAT,                   NONE),
   _T(ASTC_10x8,       ASTC_RGBA_10x8 | ASTC_FORMAT,                   NONE),
   _T(ASTC_10x10,      ASTC_RGBA_10x10 | ASTC_FORMAT,                  NONE),
   _T(ASTC_12x10,      ASTC_RGBA_12x10 | ASTC_FORMAT,                  NONE),
   _T(ASTC_12x12,      ASTC_RGBA_12x12 | ASTC_FORMAT,                  NONE),

   /* YUV */
   _T(YUYV, YUY2, YUY2),
   _T(UYVY, UYVY, NONE),

   /* multi-planar YUV */
   _T(NV12, YUY2, NONE),
};

uint32_t
translate_texture_format(enum pipe_format fmt, const struct etna_screen *screen)
{
   fmt = util_format_linear(fmt);
   fmt = translate_emulated_format_z32f(fmt);

   if (!formats[fmt].present)
      return ETNA_NO_MATCH;

   uint32_t format = formats[fmt].tex;

   if (screen->info->halti >= 5) {
      if (fmt == PIPE_FORMAT_R32_SINT || fmt == PIPE_FORMAT_R32_UINT)
         format = TEXTURE_FORMAT_EXT_R32I | EXT_FORMAT;
      else if (fmt == PIPE_FORMAT_R32G32_SINT || fmt == PIPE_FORMAT_R32G32_UINT)
         format = TEXTURE_FORMAT_EXT_G32R32I | EXT_FORMAT;
   }

   return format;
}"#;
/// The same file, lines `MESA_REMAP_FIRST`..
const MESA_REMAP_FIRST: usize = 417;
const MESA_REMAP: &str = r#"uint32_t
remap_texture_format_rb_swap(uint32_t format)
{
   switch (format) {
   case TEXTURE_FORMAT_A8R8G8B8: return TEXTURE_FORMAT_A8B8G8R8;
   case TEXTURE_FORMAT_X8R8G8B8: return TEXTURE_FORMAT_X8B8G8R8;
   default: return format;
   }
}"#;

/// etnaviv_format.h:35-36.
const EXT_FORMAT: u32 = 1 << 31;
const ASTC_FORMAT: u32 = 1 << 30;

/// The value of `#define <name>` in the vendored common_3d.xml.h.
fn header(name: &str) -> u32 {
    let v = COMMON_3D
        .lines()
        .find_map(|l| {
            let mut it = l.split_whitespace();
            (it.next() == Some("#define") && it.next() == Some(name)).then(|| it.next()).flatten()
        })
        .unwrap_or_else(|| panic!("{name} is in common_3d.xml.h"));
    u32::from_str_radix(v.trim_start_matches("0x"), 16).unwrap()
}

/// Line `n` of etnaviv_format.c, if the excerpt holds it.
fn mesa_line(n: usize) -> Option<&'static str> {
    let at = |first: usize, text: &'static str| n.checked_sub(first).and_then(|i| text.lines().nth(i));
    at(MESA_TABLE_FIRST, MESA_TABLE).or_else(|| at(MESA_REMAP_FIRST, MESA_REMAP))
}

fn numbered(first: usize, text: &'static str) -> impl Iterator<Item = (usize, &'static str)> {
    text.lines().enumerate().map(move |(i, l)| (first + i, l))
}

/// A Mesa `tex` word: an `| EXT_FORMAT` word sits in TEXDESC_CONFIG1
/// FORMAT_EXT (CONFIG0 format 0), the others in CONFIG0.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Word {
    Config0(u32),
    Ext(u32),
    Astc(u32),
}

/// `EXT_R8_SNORM | EXT_FORMAT` and the like.
fn word(expr: &str) -> Word {
    let mut v = 0;
    for term in expr.split('|').map(str::trim) {
        v |= match term {
            "EXT_FORMAT" => EXT_FORMAT,
            "ASTC_FORMAT" => ASTC_FORMAT,
            t => header(&format!("TEXTURE_FORMAT_{t}")),
        };
    }
    if v & EXT_FORMAT != 0 {
        Word::Ext(v & !EXT_FORMAT)
    } else if v & ASTC_FORMAT != 0 {
        Word::Astc(v & !ASTC_FORMAT)
    } else {
        Word::Config0(v)
    }
}

/// The word a regs.rs constant stands for: `GC_TEX_FORMAT_EXT_*` are
/// FORMAT_EXT values, the others CONFIG0 values.
fn regs_word(name: &str, value: u32) -> Word {
    if name.starts_with("GC_TEX_FORMAT_EXT_") {
        Word::Ext(value)
    } else {
        Word::Config0(value)
    }
}

/// The name a cite gives as a header term, `EXT_NONE` or `L8`.
fn cite_word(name: &str) -> Word {
    if name.starts_with("EXT_") {
        word(&format!("{name} | EXT_FORMAT"))
    } else {
        word(name)
    }
}

struct Row {
    pipe: &'static str,
    /// The `tex` expression, `None` for a vertex-only row.
    tex: Option<&'static str>,
}

/// The `formats[]` rows of the excerpt, by line.
fn rows() -> BTreeMap<usize, Row> {
    let mut out = BTreeMap::new();
    for (n, l) in numbered(MESA_TABLE_FIRST, MESA_TABLE) {
        let l = l.trim();
        let Some(kind) = ["VT(", "_T(", "V_("].into_iter().find(|k| l.starts_with(k)) else {
            continue;
        };
        let args = &l[3..l.rfind(')').unwrap()];
        let args: Vec<&str> = args.split(',').map(str::trim).collect();
        let tex = match kind {
            "VT(" => {
                assert_eq!(args.len(), 4, "etnaviv_format.c:{n} VT has 4 arguments");
                Some(args[2])
            }
            "_T(" => {
                assert_eq!(args.len(), 3, "etnaviv_format.c:{n} _T has 3 arguments");
                Some(args[1])
            }
            _ => {
                assert_eq!(args.len(), 3, "etnaviv_format.c:{n} V_ has 3 arguments");
                None
            }
        };
        out.insert(n, Row { pipe: args[0], tex });
    }
    out
}

/// The HALTI5 override of translate_texture_format: (condition line,
/// assignment line, pipe formats, word).
fn overrides() -> Vec<(usize, usize, Vec<&'static str>, Word)> {
    let lines: Vec<(usize, &str)> = numbered(MESA_TABLE_FIRST, MESA_TABLE).collect();
    let halti = lines.iter().position(|(_, l)| l.contains("if (screen->info->halti >= 5) {"));
    let halti = halti.expect("translate_texture_format has the HALTI5 override");
    assert_eq!(lines[halti].0, 286, "the HALTI5 override starts at etnaviv_format.c:286");
    let mut out = Vec::new();
    let mut i = halti + 1;
    while lines[i].1.trim() != "}" {
        let (cn, cond) = lines[i];
        let (an, assign) = lines[i + 1];
        assert!(cond.trim_start().starts_with("if (") || cond.trim_start().starts_with("else if ("), "{cn}: {cond}");
        let pipes: Vec<&str> = cond
            .split("PIPE_FORMAT_")
            .skip(1)
            .map(|s| s.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_')).next().unwrap())
            .collect();
        let rhs = assign.trim().strip_prefix("format = TEXTURE_FORMAT_").expect("format = TEXTURE_FORMAT_x");
        out.push((cn, an, pipes, word(rhs.trim_end_matches(';'))));
        i += 2;
    }
    out
}

/// translate_texture_format(pipe) on a HALTI5 screen, for a format whose
/// util_format_linear is itself (all the pinned ones).
fn mesa_texture_format(pipe: &str) -> Word {
    if let Some((_, _, _, w)) = overrides().into_iter().find(|(_, _, p, _)| p.contains(&pipe)) {
        return w;
    }
    let rows = rows();
    let row = rows.values().find(|r| r.pipe == pipe).unwrap_or_else(|| panic!("{pipe} has a formats[] row"));
    word(row.tex.unwrap_or_else(|| panic!("{pipe} is a texture format in formats[]")))
}

/// What a cited etnaviv_format.c line holds.
enum Cite {
    /// A formats[] row of this pipe format with this table word (before
    /// the HALTI5 override).
    Row(usize, &'static str, &'static str),
    /// A line (condition or assignment) of the HALTI5 override that gives
    /// these pipe formats this word.
    Override(usize, &'static [&'static str], &'static str),
    /// A remap_texture_format_rb_swap case: this word to that one.
    Remap(usize, &'static str, &'static str),
}

impl Cite {
    fn line(&self) -> usize {
        match *self {
            Cite::Row(n, ..) | Cite::Override(n, ..) | Cite::Remap(n, ..) => n,
        }
    }
}

struct Pin {
    name: &'static str,
    value: u32,
    /// The pipe formats whose HALTI5 texture word is this constant.
    pipes: &'static [&'static str],
    /// Every line the doc comment cites, with what it holds.
    cites: &'static [Cite],
    /// Pipe formats the doc comment names as not this word.
    unlike: &'static [&'static str],
}

macro_rules! pin {
    ($c:ident, [$($p:literal),*], [$($cite:expr),* $(,)?] $(, unlike [$($u:literal),*])?) => {
        Pin { name: stringify!($c), value: $c, pipes: &[$($p),*], cites: &[$($cite),*], unlike: &[$($($u),*)?] }
    };
}

use Cite::{Override as O, Remap as M, Row as R};

#[rustfmt::skip]
const PINS: &[Pin] = &[
    pin!(GC_TEX_FORMAT_A8, ["A8_UNORM"], [R(95, "A8_UNORM", "A8")]),
    pin!(GC_TEX_FORMAT_L8, ["R8_UNORM", "L8_UNORM"], [R(88, "R8_UNORM", "L8"), R(96, "L8_UNORM", "L8")]),
    pin!(GC_TEX_FORMAT_A8L8, ["L8A8_UNORM"], [R(111, "L8A8_UNORM", "A8L8")]),
    pin!(GC_TEX_FORMAT_A8R8G8B8, ["R8G8B8A8_UNORM"], [R(155, "R8G8B8A8_UNORM", "A8R8G8B8")]),
    pin!(GC_TEX_FORMAT_X8R8G8B8, ["B8G8R8X8_UNORM"], [R(165, "B8G8R8X8_UNORM", "X8R8G8B8")]),
    pin!(GC_TEX_FORMAT_A8B8G8R8, [], [M(421, "A8R8G8B8", "A8B8G8R8")]),
    pin!(GC_TEX_FORMAT_X8B8G8R8, [], [M(422, "X8R8G8B8", "X8B8G8R8")]),
    pin!(GC_TEX_FORMAT_R5G6B5, ["B5G6R5_UNORM"], [R(114, "B5G6R5_UNORM", "R5G6B5")]),
    pin!(GC_TEX_FORMAT_D24X8, ["X8Z24_UNORM", "S8_UINT_Z24_UNORM"],
         [R(179, "X8Z24_UNORM", "D24X8"), R(180, "S8_UINT_Z24_UNORM", "D24X8")]),
    pin!(GC_TEX_FORMAT_E5B9G9R9, ["R9G9B9E5_FLOAT"], [R(186, "R9G9B9E5_FLOAT", "E5B9G9R9")]),
    pin!(GC_TEX_FORMAT_ETC1, ["ETC1_RGB8"], [R(237, "ETC1_RGB8", "ETC1")], unlike ["ETC2_RGB8"]),
    pin!(GC_TEX_FORMAT_EXT_NONE, ["ETC2_RGB8"], [R(244, "ETC2_RGB8", "EXT_NONE")]),
    pin!(GC_TEX_FORMAT_EXT_RGB8_PUNCHTHROUGH_ALPHA1_ETC2, ["ETC2_RGB8A1"],
         [R(245, "ETC2_RGB8A1", "EXT_RGB8_PUNCHTHROUGH_ALPHA1_ETC2")]),
    pin!(GC_TEX_FORMAT_EXT_RGBA8_ETC2_EAC, ["ETC2_RGBA8"], [R(246, "ETC2_RGBA8", "EXT_RGBA8_ETC2_EAC")]),
    pin!(GC_TEX_FORMAT_EXT_R11_EAC, ["ETC2_R11_UNORM"], [
        R(247, "ETC2_R11_UNORM", "EXT_R11_EAC"),
        R(248, "ETC2_R11_SNORM", "EXT_SIGNED_R11_EAC"),
        R(249, "ETC2_RG11_UNORM", "EXT_RG11_EAC"),
        R(250, "ETC2_RG11_SNORM", "EXT_SIGNED_RG11_EAC"),
    ]),
    pin!(GC_TEX_FORMAT_EXT_RG11_EAC, ["ETC2_RG11_UNORM"], [R(249, "ETC2_RG11_UNORM", "EXT_RG11_EAC")]),
    pin!(GC_TEX_FORMAT_EXT_SIGNED_RG11_EAC, ["ETC2_RG11_SNORM"], [R(250, "ETC2_RG11_SNORM", "EXT_SIGNED_RG11_EAC")]),
    pin!(GC_TEX_FORMAT_EXT_SIGNED_R11_EAC, ["ETC2_R11_SNORM"], [R(248, "ETC2_R11_SNORM", "EXT_SIGNED_R11_EAC")]),
    pin!(GC_TEX_FORMAT_EXT_G8R8, ["R8G8_UNORM"], [R(118, "R8G8_UNORM", "EXT_G8R8")]),
    pin!(GC_TEX_FORMAT_EXT_R16F, ["R16_FLOAT"], [R(106, "R16_FLOAT", "EXT_R16F")]),
    pin!(GC_TEX_FORMAT_EXT_G16R16F, ["R16G16_FLOAT"], [R(151, "R16G16_FLOAT", "EXT_G16R16F")]),
    pin!(GC_TEX_FORMAT_EXT_A16B16G16R16F, ["R16G16B16A16_FLOAT"], [R(205, "R16G16B16A16_FLOAT", "EXT_A16B16G16R16F")]),
    pin!(GC_TEX_FORMAT_EXT_R32F, ["R32_FLOAT"], [R(140, "R32_FLOAT", "EXT_R32F")]),
    // R32G32_UINT/SINT carry it in the table only: on HALTI5 the override
    // makes them EXT_G32R32I (below).
    pin!(GC_TEX_FORMAT_EXT_G32R32F, ["R32G32_FLOAT", "R32G32B32A32_FLOAT"], [
        R(209, "R32G32_UINT", "EXT_G32R32F"),
        R(210, "R32G32_SINT", "EXT_G32R32F"),
        R(213, "R32G32_FLOAT", "EXT_G32R32F"),
        R(233, "R32G32B32A32_FLOAT", "EXT_G32R32F"),
    ]),
    pin!(GC_TEX_FORMAT_EXT_A2B10G10R10, ["R10G10B10X2_UNORM"], [R(168, "R10G10B10X2_UNORM", "EXT_A2B10G10R10")]),
    pin!(GC_TEX_FORMAT_EXT_R8_SNORM, ["R8_SNORM"], [
        R(89, "R8_SNORM", "EXT_R8_SNORM"),
        R(119, "R8G8_SNORM", "EXT_G8R8_SNORM"),
        R(156, "R8G8B8A8_SNORM", "EXT_A8B8G8R8_SNORM"),
        R(158, "R8G8B8X8_SNORM", "EXT_X8B8G8R8_SNORM"),
    ]),
    pin!(GC_TEX_FORMAT_EXT_G8R8_SNORM, ["R8G8_SNORM"], [R(119, "R8G8_SNORM", "EXT_G8R8_SNORM")]),
    pin!(GC_TEX_FORMAT_EXT_X8B8G8R8_SNORM, ["R8G8B8X8_SNORM"], [R(158, "R8G8B8X8_SNORM", "EXT_X8B8G8R8_SNORM")]),
    pin!(GC_TEX_FORMAT_EXT_A8B8G8R8_SNORM, ["R8G8B8A8_SNORM"], [R(156, "R8G8B8A8_SNORM", "EXT_A8B8G8R8_SNORM")]),
    pin!(GC_TEX_FORMAT_EXT_R8I, ["R8_UINT", "R8_SINT"], [
        R(90, "R8_UINT", "EXT_R8I"),
        R(91, "R8_SINT", "EXT_R8I"),
        R(120, "R8G8_UINT", "EXT_G8R8I"),
        R(121, "R8G8_SINT", "EXT_G8R8I"),
    ]),
    pin!(GC_TEX_FORMAT_EXT_G8R8I, ["R8G8_UINT", "R8G8_SINT"], [R(120, "R8G8_UINT", "EXT_G8R8I")]),
    pin!(GC_TEX_FORMAT_EXT_A8B8G8R8I, ["R8G8B8A8_UINT"], [R(159, "R8G8B8A8_UINT", "EXT_A8B8G8R8I")]),
    pin!(GC_TEX_FORMAT_EXT_R16I, ["R16_UINT", "R16_SINT"], [
        R(102, "R16_UINT", "EXT_R16I"),
        R(103, "R16_SINT", "EXT_R16I"),
        R(147, "R16G16_UINT", "EXT_G16R16I"),
        R(148, "R16G16_SINT", "EXT_G16R16I"),
    ]),
    pin!(GC_TEX_FORMAT_EXT_G16R16I, ["R16G16_UINT", "R16G16_SINT"], [R(147, "R16G16_UINT", "EXT_G16R16I")]),
    pin!(GC_TEX_FORMAT_EXT_A16B16G16R16I, ["R16G16B16A16_UINT"], [R(201, "R16G16B16A16_UINT", "EXT_A16B16G16R16I")]),
    pin!(GC_TEX_FORMAT_EXT_B10G11R11F, ["R11G11B10_FLOAT"], [R(187, "R11G11B10_FLOAT", "EXT_B10G11R11F")]),
    pin!(GC_TEX_FORMAT_EXT_A2B10G10R10UI, ["R10G10B10A2_UINT"], [R(170, "R10G10B10A2_UINT", "EXT_A2B10G10R10UI")]),
    // No row gives EXT_R8 (`no_row_samples_ext_r8`); the doc cites the
    // R8_UNORM row that takes CONFIG0 L8 instead.
    pin!(GC_TEX_FORMAT_EXT_R8, [], [R(88, "R8_UNORM", "L8")]),
    pin!(GC_TEX_FORMAT_EXT_R32I, ["R32_SINT", "R32_UINT"], [
        R(136, "R32_SINT", "EXT_R32F"),
        R(137, "R32_UINT", "EXT_R32F"),
        O(287, &["R32_SINT", "R32_UINT"], "EXT_R32I"),
        O(288, &["R32_SINT", "R32_UINT"], "EXT_R32I"),
    ]),
    pin!(GC_TEX_FORMAT_EXT_G32R32I, ["R32G32_SINT", "R32G32_UINT", "R32G32B32A32_UINT", "R32G32B32A32_SINT"], [
        R(209, "R32G32_UINT", "EXT_G32R32F"),
        R(210, "R32G32_SINT", "EXT_G32R32F"),
        R(229, "R32G32B32A32_UINT", "EXT_G32R32I"),
        R(230, "R32G32B32A32_SINT", "EXT_G32R32I"),
        O(289, &["R32G32_SINT", "R32G32_UINT"], "EXT_G32R32I"),
        O(290, &["R32G32_SINT", "R32G32_UINT"], "EXT_G32R32I"),
    ]),
];

/// The `GC_TEX_FORMAT_*` constants of regs.rs: (name, value, the `///`
/// lines right above it).
fn regs_consts() -> Vec<(&'static str, u32, Vec<&'static str>)> {
    let lines: Vec<&str> = REGS_RS.lines().collect();
    let mut out = Vec::new();
    for (i, l) in lines.iter().enumerate() {
        let Some(rest) = l.strip_prefix("pub const GC_TEX_FORMAT_") else {
            continue;
        };
        let (name, val) = rest.split_once(": u32 = ").unwrap();
        let val = u32::from_str_radix(val.trim_end_matches(';').trim_start_matches("0x"), 16).unwrap();
        let name = &l["pub const ".len().."pub const GC_TEX_FORMAT_".len() + name.len()];
        let mut doc: Vec<&str> = lines[..i].iter().rev().map_while(|l| l.strip_prefix("///")).collect();
        doc.reverse();
        out.push((name, val, doc));
    }
    out
}

/// The etnaviv_format.c lines a doc comment cites: `F:95`, `F:120-121`,
/// `etnaviv_format.c:247-250` (`X:` lines are the header's, checked by
/// regs.rs's own `mesa_header` test).
fn cited_lines(doc: &[&str]) -> BTreeSet<usize> {
    let text = doc.join(" ");
    let mut out = BTreeSet::new();
    for marker in ["F:", "etnaviv_format.c:"] {
        for (at, _) in text.match_indices(marker) {
            if marker == "F:" && text[..at].ends_with(|c: char| c.is_ascii_alphanumeric() || c == '_' || c == '.') {
                continue;
            }
            let rest = &text[at + marker.len()..];
            let num = |s: &str| {
                let d: String = s.chars().take_while(char::is_ascii_digit).collect();
                (!d.is_empty()).then(|| d.parse::<usize>().unwrap())
            };
            let Some(a) = num(rest) else { continue };
            let after = &rest[a.to_string().len()..];
            let b = after.strip_prefix('-').and_then(num).unwrap_or(a);
            out.extend(a..=b);
        }
    }
    out
}

fn pin_of(name: &str) -> &'static Pin {
    PINS.iter().find(|p| p.name == name).unwrap_or_else(|| panic!("{name} has a pin"))
}

/// Each pinned pipe format's HALTI5 texture word is the constant.
#[test]
fn pipe_formats_give_the_regs_words() {
    for p in PINS {
        let want = regs_word(p.name, p.value);
        for pipe in p.pipes {
            assert_eq!(mesa_texture_format(pipe), want, "{}: Mesa's PIPE_FORMAT_{pipe}", p.name);
        }
        for pipe in p.unlike {
            assert_ne!(mesa_texture_format(pipe), want, "{}: Mesa's PIPE_FORMAT_{pipe} is another word", p.name);
        }
    }
    // The examples of the item, spelled out.
    assert_eq!(mesa_texture_format("R32_SINT"), Word::Ext(GC_TEX_FORMAT_EXT_R32I));
    assert_eq!(mesa_texture_format("R32_UINT"), Word::Ext(GC_TEX_FORMAT_EXT_R32I));
    assert_eq!(mesa_texture_format("R32G32_SINT"), Word::Ext(GC_TEX_FORMAT_EXT_G32R32I));
    assert_eq!(mesa_texture_format("R32G32_UINT"), Word::Ext(GC_TEX_FORMAT_EXT_G32R32I));
    assert_eq!(mesa_texture_format("R8_SNORM"), Word::Ext(GC_TEX_FORMAT_EXT_R8_SNORM));
    assert_eq!(mesa_texture_format("ETC2_RGB8"), Word::Ext(GC_TEX_FORMAT_EXT_NONE));
    assert_ne!(mesa_texture_format("ETC2_RGB8"), Word::Config0(GC_TEX_FORMAT_ETC1));
}

/// Each cited line holds the pipe format and word listed with it.
#[test]
fn cited_lines_hold_their_formats() {
    let rows = rows();
    let overrides = overrides();
    for p in PINS {
        for c in p.cites {
            let n = c.line();
            let text = mesa_line(n).unwrap_or_else(|| panic!("{}: etnaviv_format.c:{n} is in the excerpt", p.name));
            match *c {
                Cite::Row(_, pipe, w) => {
                    let row =
                        rows.get(&n).unwrap_or_else(|| panic!("{}: etnaviv_format.c:{n} is a formats[] row", p.name));
                    assert_eq!(row.pipe, pipe, "{}: etnaviv_format.c:{n} is the {pipe} row: {text}", p.name);
                    let tex = row.tex.unwrap_or_else(|| panic!("{}: etnaviv_format.c:{n} has a texture word", p.name));
                    assert_eq!(word(tex), cite_word(w), "{}: etnaviv_format.c:{n} gives {pipe} {w}", p.name);
                }
                Cite::Override(_, pipes, w) => {
                    let o = overrides.iter().find(|(cn, an, ..)| *cn == n || *an == n);
                    let (_, _, opipes, ow) =
                        o.unwrap_or_else(|| panic!("{}: etnaviv_format.c:{n} is in the HALTI5 override", p.name));
                    assert_eq!(opipes.as_slice(), pipes, "{}: etnaviv_format.c:{n}: {text}", p.name);
                    assert_eq!(*ow, cite_word(w), "{}: etnaviv_format.c:{n}: {text}", p.name);
                }
                Cite::Remap(_, from, to) => {
                    let want = format!("case TEXTURE_FORMAT_{from}: return TEXTURE_FORMAT_{to};");
                    assert_eq!(text.trim(), want, "{}: etnaviv_format.c:{n}", p.name);
                    assert_eq!(cite_word(to), regs_word(p.name, p.value), "{}: TEXTURE_FORMAT_{to}", p.name);
                }
            }
        }
    }
}

/// The lines each constant's doc comment cites are the pin's lines, so a
/// new or moved citation cannot go unchecked; every constant with a
/// citation is pinned, every pin is a regs.rs constant of that value, and
/// every Mesa pipe format the doc names is pinned or cited.
#[test]
fn every_citation_is_pinned() {
    let consts = regs_consts();
    assert!(consts.len() >= 39, "only {} GC_TEX_FORMAT_* consts found", consts.len());
    let pipes: BTreeSet<&str> = rows().values().map(|r| r.pipe).collect();
    let mut pinned = 0;
    for (name, val, doc) in &consts {
        let cited = cited_lines(doc);
        if cited.is_empty() {
            assert!(!PINS.iter().any(|p| p.name == *name), "{name} is pinned but cites no etnaviv_format.c line");
            continue;
        }
        let p = pin_of(name);
        pinned += 1;
        assert_eq!(p.value, *val, "{name}: the regs.rs text and the constant agree");
        let listed: BTreeSet<usize> = p.cites.iter().map(Cite::line).collect();
        assert_eq!(cited, listed, "{name}: the etnaviv_format.c lines its doc comment cites");
        let named = doc.join(" ");
        for tok in named.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_')) {
            if !pipes.contains(tok) {
                continue;
            }
            let cites_it = p.cites.iter().any(|c| match *c {
                Cite::Row(_, pipe, _) => pipe == tok,
                Cite::Override(_, ps, _) => ps.contains(&tok),
                Cite::Remap(..) => false,
            });
            let named = p.pipes.contains(&tok) || p.unlike.contains(&tok) || cites_it;
            assert!(named, "{name}: its doc names PIPE_FORMAT_{tok}, which no pin or cite holds");
        }
    }
    assert_eq!(pinned, PINS.len(), "every pin is a regs.rs constant with a citation");
}

/// The constants without a citation are the ones Mesa's table has no row
/// for: only GC_TEX_FORMAT_EXT_R8 among the EXT words (its doc says so).
#[test]
fn no_row_samples_ext_r8() {
    for (_, row) in rows() {
        if let Some(tex) = row.tex {
            assert!(!tex.split('|').any(|t| t.trim() == "EXT_R8"), "PIPE_FORMAT_{} samples EXT_R8", row.pipe);
        }
    }
    let ext_r8 = pin_of("GC_TEX_FORMAT_EXT_R8");
    assert!(ext_r8.pipes.is_empty());
    for (name, _, doc) in regs_consts() {
        if name.starts_with("GC_TEX_FORMAT_EXT_") {
            assert!(!cited_lines(&doc).is_empty(), "{name} cites an etnaviv_format.c line");
        }
    }
}

/// The excerpt is the upstream file's text at those lines. Needs a Mesa
/// 26.2.1 tree (the tarball of third_party/mesa-26.2.1/UPSTREAM) in
/// YAKOGL_MESA_SRC.
#[test]
#[ignore = "needs YAKOGL_MESA_SRC, a mesa-26.2.1 source tree"]
fn excerpt_is_upstream() {
    let src = std::env::var("YAKOGL_MESA_SRC").expect("YAKOGL_MESA_SRC names a mesa-26.2.1 tree");
    let path = format!("{src}/src/gallium/drivers/etnaviv/etnaviv_format.c");
    let file = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
    let upstream: Vec<&str> = file.lines().collect();
    assert_eq!(upstream.len(), 451, "{path} is the 26.2.1 file");
    for (first, text) in [(MESA_TABLE_FIRST, MESA_TABLE), (MESA_REMAP_FIRST, MESA_REMAP)] {
        for (n, l) in numbered(first, text) {
            assert_eq!(l, upstream[n - 1], "etnaviv_format.c:{n}");
        }
    }
}
