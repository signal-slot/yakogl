// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The V3D 7.1 TMU format answers per GL format (roadmap P10; the Pi 5's
//! V3D 7.1.10).
//!
//! Pure functions over the GL internal format and type as `u32`s; no
//! `crate::` import, so a test includes this file with `#[path]`
//! (tests/v3d71_fmt.rs). Nothing hooks it into `src/v3d/mod.rs` yet: the 7.1
//! texture state (tex71) does that and calls [`tmu_format71`].
//!
//! The answers are Mesa 26.2.1's, from
//! `third_party/mesa-26.2.1/src/gallium/drivers/v3d/v3dx_format_table.c`.
//! That file has no `V3D_VERSION` conditional, so it is one table for 4.2
//! and 7.1: [`MESA_ROWS`] transcribes every `FORMAT` row with its line
//! number, and tests/v3d71_fmt.rs compares the transcription with the file
//! itself. What differs at 7.1 is not the table but what the texture record
//! does with it (driver/src/v3d/v3d_tex71.h):
//!
//! * the sRGB bit is a 3-bit transfer function: [`Transfer::Srgb`] for a
//!   format whose pipe format is sRGB (`util_format_is_srgb`,
//!   v3dx_state.c:1027-1029 as v3d_tex71.h cites it), [`Transfer::None`] for
//!   the rest. PQ, HLG and the others are no GL format and have no variant;
//! * the R/B swap bit (7.1.5 and later) and the reverse bit exist, but
//!   gallium never sets them: it composes the format's swizzle (the ZYXW
//!   rows of the table) exactly as at 4.2 (v3d_tex71.h on `r_b_swap`).
//!   [`Row71::r_b_swap`] and [`Row71::reverse`] are therefore `false` for
//!   every row and the swizzle carries the swap. The Vulkan form (swap bit
//!   set, the view's own swizzle) is Mesa's broadcom/vulkan/v3dvx_image.c:
//!   137-157, a tree this repository does not vendor, so it is not
//!   transcribed;
//! * `return_32`: the TMU's 32-bit output (TMU Config Parameter 1) is the
//!   row's return size 32.
//!
//! A GL format with no row is refused by a [`Refusal71`] variant, never by a
//! default answer.

/// TEXTURE_SHADER_STATE's swizzle encoding (the same values as
/// driver/src/v3d/v3d_tex.h V3D_SWIZZLE_*).
pub const SWIZZLE_ZERO: u8 = 0;
pub const SWIZZLE_ONE: u8 = 1;
pub const SWIZZLE_RED: u8 = 2;
pub const SWIZZLE_GREEN: u8 = 3;
pub const SWIZZLE_BLUE: u8 = 4;
pub const SWIZZLE_ALPHA: u8 = 5;

/// The 7.1 record's transfer function (v3d_packet.xml:1697-1704): the two a
/// GL format asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Transfer {
    None,
    Srgb,
}

/// One transcribed row of Mesa's `format_table`: `FORMAT(pipe, rt, tex, swiz,
/// return_size, return_channels)` without the render-target column.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MesaRow {
    /// the pipe format, `PIPE_FORMAT_<pipe>`
    pub pipe: &'static str,
    /// the row's line in v3dx_format_table.c
    pub line: u32,
    /// TEXTURE_DATA_FORMAT_* (cle/v3d_packet.xml:328-)
    pub tex: u32,
    /// `SWIZ_<swiz>`: four of X Y Z W 0 1
    pub swiz: &'static str,
    pub return_size: u32,
    pub return_channels: u32,
}

const fn mr(
    pipe: &'static str,
    line: u32,
    tex: u32,
    swiz: &'static str,
    return_size: u32,
    return_channels: u32,
) -> MesaRow {
    MesaRow { pipe, line, tex, swiz, return_size, return_channels }
}

/// Every `FORMAT` row of v3dx_format_table.c, in the file's order.
pub const MESA_ROWS: [MesaRow; 126] = [
    mr("B8G8R8A8_UNORM", 61, 4, "ZYXW", 16, 0),
    mr("B8G8R8X8_UNORM", 62, 4, "ZYX1", 16, 0),
    mr("B8G8R8A8_SRGB", 63, 4, "ZYXW", 16, 0),
    mr("B8G8R8X8_SRGB", 64, 4, "ZYX1", 16, 0),
    mr("R8G8B8A8_UNORM", 65, 4, "XYZW", 16, 0),
    mr("R8G8B8X8_UNORM", 66, 4, "XYZ1", 16, 0),
    mr("R8G8B8A8_SRGB", 67, 4, "XYZW", 16, 0),
    mr("R8G8B8X8_SRGB", 68, 4, "XYZ1", 16, 0),
    mr("R8G8B8A8_SNORM", 69, 5, "XYZW", 16, 0),
    mr("R8G8B8X8_SNORM", 70, 5, "XYZ1", 16, 0),
    mr("R10G10B10A2_UNORM", 71, 9, "XYZW", 16, 0),
    mr("B10G10R10A2_UNORM", 72, 9, "ZYXW", 16, 0),
    mr("R10G10B10X2_UNORM", 73, 9, "XYZ1", 16, 0),
    mr("B10G10R10X2_UNORM", 74, 9, "ZYX1", 16, 0),
    mr("R10G10B10A2_UINT", 75, 114, "XYZW", 16, 0),
    mr("A4B4G4R4_UNORM", 77, 7, "XYZW", 16, 0),
    mr("A1B5G5R5_UNORM", 79, 8, "XYZW", 16, 0),
    mr("X1B5G5R5_UNORM", 80, 8, "XYZ1", 16, 0),
    mr("B5G6R5_UNORM", 81, 6, "XYZ1", 16, 0),
    mr("R8_UNORM", 83, 0, "X001", 16, 0),
    mr("R8_SNORM", 84, 1, "X001", 16, 0),
    mr("R8G8_UNORM", 85, 2, "XY01", 16, 0),
    mr("R8G8_SNORM", 86, 3, "XY01", 16, 0),
    mr("R16_UNORM", 88, 10, "X001", 32, 1),
    mr("R16_SNORM", 89, 11, "X001", 32, 1),
    mr("R16_FLOAT", 90, 16, "X001", 16, 0),
    mr("R32_FLOAT", 91, 29, "X001", 32, 1),
    mr("R16G16_UNORM", 93, 12, "XY01", 32, 2),
    mr("R16G16_SNORM", 94, 13, "XY01", 32, 2),
    mr("R16G16_FLOAT", 95, 17, "XY01", 16, 0),
    mr("R32G32_FLOAT", 96, 30, "XY01", 32, 2),
    mr("R16G16B16A16_UNORM", 98, 14, "XYZW", 32, 4),
    mr("R16G16B16A16_SNORM", 99, 15, "XYZW", 32, 4),
    mr("R16G16B16A16_FLOAT", 100, 18, "XYZW", 16, 0),
    mr("R32G32B32A32_FLOAT", 101, 31, "XYZW", 32, 4),
    mr("L16_UNORM", 104, 10, "XXX1", 32, 1),
    mr("L16_SNORM", 105, 11, "XXX1", 32, 1),
    mr("I16_UNORM", 106, 10, "XXXX", 32, 1),
    mr("I16_SNORM", 107, 11, "XXXX", 32, 1),
    mr("A16_UNORM", 108, 10, "000X", 32, 1),
    mr("A16_SNORM", 109, 11, "000X", 32, 1),
    mr("L16A16_UNORM", 110, 12, "XXXY", 32, 2),
    mr("L16A16_SNORM", 111, 13, "XXXY", 32, 2),
    mr("A8_UNORM", 113, 0, "000X", 16, 0),
    mr("L8_UNORM", 114, 0, "XXX1", 16, 0),
    mr("I8_UNORM", 115, 0, "XXXX", 16, 0),
    mr("L8A8_UNORM", 116, 2, "XXXY", 16, 0),
    mr("R8_SINT", 118, 96, "X001", 16, 0),
    mr("R8_UINT", 119, 97, "X001", 16, 0),
    mr("R8G8_SINT", 120, 98, "XY01", 16, 0),
    mr("R8G8_UINT", 121, 99, "XY01", 16, 0),
    mr("R8G8B8A8_SINT", 122, 100, "XYZW", 16, 0),
    mr("R8G8B8A8_UINT", 123, 101, "XYZW", 16, 0),
    mr("R16_SINT", 125, 102, "X001", 16, 0),
    mr("R16_UINT", 126, 103, "X001", 16, 0),
    mr("R16G16_SINT", 127, 104, "XY01", 16, 0),
    mr("R16G16_UINT", 128, 105, "XY01", 16, 0),
    mr("R16G16B16A16_SINT", 129, 106, "XYZW", 16, 0),
    mr("R16G16B16A16_UINT", 130, 107, "XYZW", 16, 0),
    mr("R32_SINT", 132, 108, "X001", 32, 1),
    mr("R32_UINT", 133, 109, "X001", 32, 1),
    mr("R32G32_SINT", 134, 110, "XY01", 32, 2),
    mr("R32G32_UINT", 135, 111, "XY01", 32, 2),
    mr("R32G32B32A32_SINT", 136, 112, "XYZW", 32, 4),
    mr("R32G32B32A32_UINT", 137, 113, "XYZW", 32, 4),
    mr("A8_SINT", 139, 96, "000X", 16, 0),
    mr("A8_UINT", 140, 97, "000X", 16, 0),
    mr("A16_SINT", 141, 102, "000X", 16, 0),
    mr("A16_UINT", 142, 103, "000X", 16, 0),
    mr("A32_SINT", 143, 108, "000X", 32, 1),
    mr("A32_UINT", 144, 109, "000X", 32, 1),
    mr("R11G11B10_FLOAT", 146, 19, "XYZ1", 16, 0),
    mr("R9G9B9E5_FLOAT", 147, 20, "XYZ1", 16, 0),
    mr("S8_UINT_Z24_UNORM", 149, 24, "XXXX", 32, 1),
    mr("X8Z24_UNORM", 150, 24, "XXXX", 32, 1),
    mr("S8X24_UINT", 151, 101, "XXXX", 16, 1),
    mr("Z32_FLOAT", 152, 23, "XXXX", 32, 1),
    mr("Z16_UNORM", 153, 21, "XXXX", 32, 1),
    mr("Z32_FLOAT_S8X24_UINT", 156, 23, "XXXX", 32, 1),
    mr("X32_S8X24_UINT", 157, 97, "XXXX", 16, 1),
    mr("ETC2_RGB8", 159, 32, "XYZ1", 16, 0),
    mr("ETC2_SRGB8", 160, 32, "XYZ1", 16, 0),
    mr("ETC2_RGB8A1", 161, 33, "XYZW", 16, 0),
    mr("ETC2_SRGB8A1", 162, 33, "XYZW", 16, 0),
    mr("ETC2_RGBA8", 163, 38, "XYZW", 16, 0),
    mr("ETC2_SRGBA8", 164, 38, "XYZW", 16, 0),
    mr("ETC2_R11_UNORM", 165, 34, "X001", 16, 0),
    mr("ETC2_R11_SNORM", 166, 35, "X001", 16, 0),
    mr("ETC2_RG11_UNORM", 167, 36, "XY01", 16, 0),
    mr("ETC2_RG11_SNORM", 168, 37, "XY01", 16, 0),
    mr("DXT1_RGB", 170, 48, "XYZ1", 16, 0),
    mr("DXT1_SRGB", 171, 48, "XYZ1", 16, 0),
    mr("DXT1_RGBA", 172, 48, "XYZW", 16, 0),
    mr("DXT1_SRGBA", 173, 48, "XYZW", 16, 0),
    mr("DXT3_RGBA", 174, 49, "XYZW", 16, 0),
    mr("DXT3_SRGBA", 175, 49, "XYZW", 16, 0),
    mr("DXT5_RGBA", 176, 50, "XYZW", 16, 0),
    mr("DXT5_SRGBA", 177, 50, "XYZW", 16, 0),
    mr("ASTC_4x4", 180, 64, "XYZW", 16, 1),
    mr("ASTC_4x4_SRGB", 181, 64, "XYZW", 16, 1),
    mr("ASTC_5x4", 182, 65, "XYZW", 16, 1),
    mr("ASTC_5x4_SRGB", 183, 65, "XYZW", 16, 1),
    mr("ASTC_5x5", 184, 66, "XYZW", 16, 1),
    mr("ASTC_5x5_SRGB", 185, 66, "XYZW", 16, 1),
    mr("ASTC_6x5", 186, 67, "XYZW", 16, 1),
    mr("ASTC_6x5_SRGB", 187, 67, "XYZW", 16, 1),
    mr("ASTC_6x6", 188, 68, "XYZW", 16, 1),
    mr("ASTC_6x6_SRGB", 189, 68, "XYZW", 16, 1),
    mr("ASTC_8x5", 190, 69, "XYZW", 16, 1),
    mr("ASTC_8x5_SRGB", 191, 69, "XYZW", 16, 1),
    mr("ASTC_8x6", 192, 70, "XYZW", 16, 1),
    mr("ASTC_8x6_SRGB", 193, 70, "XYZW", 16, 1),
    mr("ASTC_8x8", 194, 71, "XYZW", 16, 1),
    mr("ASTC_8x8_SRGB", 195, 71, "XYZW", 16, 1),
    mr("ASTC_10x5", 196, 72, "XYZW", 16, 1),
    mr("ASTC_10x5_SRGB", 197, 72, "XYZW", 16, 1),
    mr("ASTC_10x6", 198, 73, "XYZW", 16, 1),
    mr("ASTC_10x6_SRGB", 199, 73, "XYZW", 16, 1),
    mr("ASTC_10x8", 200, 74, "XYZW", 16, 1),
    mr("ASTC_10x8_SRGB", 201, 74, "XYZW", 16, 1),
    mr("ASTC_10x10", 202, 75, "XYZW", 16, 1),
    mr("ASTC_10x10_SRGB", 203, 75, "XYZW", 16, 1),
    mr("ASTC_12x10", 204, 76, "XYZW", 16, 1),
    mr("ASTC_12x10_SRGB", 205, 76, "XYZW", 16, 1),
    mr("ASTC_12x12", 206, 77, "XYZW", 16, 1),
    mr("ASTC_12x12_SRGB", 207, 77, "XYZW", 16, 1),
];

/// The answer for one GL format: what the TEXTURE_SHADER_STATE and the TMU
/// config need at 7.1.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Row71 {
    /// the Mesa row it is (`PIPE_FORMAT_<pipe>`) and its line in
    /// v3dx_format_table.c
    pub pipe: &'static str,
    pub mesa_line: u32,
    /// TEXTURE_DATA_FORMAT_* (the record's "Texture type")
    pub tex_type: u32,
    /// the swizzle that makes a program read R, G, B, A
    /// (SWIZZLE_*)
    pub swizzle: [u8; 4],
    /// the record's R/B swap bit: `false` always (module comment)
    pub r_b_swap: bool,
    /// the record's Reverse bit: `false` always (module comment)
    pub reverse: bool,
    pub transfer: Transfer,
    /// the look-up wants the TMU's 32-bit output (Mesa return size 32)
    pub return_32: bool,
}

/// Why a GL format has no 7.1 answer. Every variant is a decision, not a
/// fall-through.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal71 {
    /// The GL name is no texture format this table knows (an enum of
    /// another API object, or one no ES surface offers).
    UnknownFormat,
    /// An unsized format with a type that has no ES 2.0/3.0 meaning
    /// (ES 3.0.6 table 3.2), or one this table does not take.
    UnsizedTypeNotListed,
    /// Mesa's table has no three-channel row of this class (RGB16F, RGB32F
    /// and the RGB integer formats; no `R16G16B16`/`R32G32B32` pipe row
    /// exists in v3dx_format_table.c): the GL core must store them as the
    /// four-channel format and ask for that one.
    NoThreeChannelRow,
    /// STENCIL_INDEX8 as a texture (GL 4.3 stencil texturing, no ES
    /// surface): Mesa's S8 rows (v3dx_format_table.c:151, :157) are the
    /// depth-stencil split's stencil half and are not a texture format
    /// here.
    StencilTexturing,
}

// GL enums (the Khronos registry's values).
pub const GL_ALPHA: u32 = 0x1906;
pub const GL_RGB: u32 = 0x1907;
pub const GL_RGBA: u32 = 0x1908;
pub const GL_LUMINANCE: u32 = 0x1909;
pub const GL_LUMINANCE_ALPHA: u32 = 0x190A;
pub const GL_DEPTH_COMPONENT: u32 = 0x1902;
pub const GL_DEPTH_STENCIL: u32 = 0x84F9;
pub const GL_BGRA_EXT: u32 = 0x80E1;
pub const GL_UNSIGNED_BYTE: u32 = 0x1401;
pub const GL_UNSIGNED_SHORT: u32 = 0x1403;
pub const GL_UNSIGNED_INT: u32 = 0x1405;
pub const GL_FLOAT: u32 = 0x1406;
pub const GL_HALF_FLOAT: u32 = 0x140B;
pub const GL_HALF_FLOAT_OES: u32 = 0x8D61;
pub const GL_UNSIGNED_SHORT_4_4_4_4: u32 = 0x8033;
pub const GL_UNSIGNED_SHORT_5_5_5_1: u32 = 0x8034;
pub const GL_UNSIGNED_SHORT_5_6_5: u32 = 0x8363;
pub const GL_UNSIGNED_INT_24_8: u32 = 0x84FA;
pub const GL_FLOAT_32_UNSIGNED_INT_24_8_REV: u32 = 0x8DAD;

/// The Mesa row of a pipe format name.
pub fn mesa_row(pipe: &str) -> Option<&'static MesaRow> {
    MESA_ROWS.iter().find(|r| r.pipe == pipe)
}

fn swz_of(c: u8) -> u8 {
    match c {
        b'X' => SWIZZLE_RED,
        b'Y' => SWIZZLE_GREEN,
        b'Z' => SWIZZLE_BLUE,
        b'W' => SWIZZLE_ALPHA,
        b'0' => SWIZZLE_ZERO,
        _ => SWIZZLE_ONE,
    }
}

/// `SWIZ_<x><y><z><w>` as the record's four swizzle values (gallium's
/// `v3d_translate_pipe_swizzle`: X, Y, Z, W read R, G, B, A).
pub fn swizzle_of(swiz: &str) -> [u8; 4] {
    let b = swiz.as_bytes();
    [swz_of(b[0]), swz_of(b[1]), swz_of(b[2]), swz_of(b[3])]
}

/// Is the pipe format an sRGB one (`util_format_is_srgb`)? Mesa's names:
/// `*_SRGB`, `*_SRGBA` and `ETC2_SRGB*`.
pub fn pipe_is_srgb(pipe: &str) -> bool {
    pipe.contains("SRGB")
}

/// The 7.1 answer of a Mesa row.
pub fn row71(pipe: &'static str) -> Row71 {
    let m = mesa_row(pipe).expect("format71: a pipe format of v3dx_format_table.c");
    Row71 {
        pipe: m.pipe,
        mesa_line: m.line,
        tex_type: m.tex,
        swizzle: swizzle_of(m.swiz),
        r_b_swap: false,
        reverse: false,
        transfer: if pipe_is_srgb(m.pipe) { Transfer::Srgb } else { Transfer::None },
        return_32: m.return_size == 32,
    }
}

/// B8G8R8X8_UNORM (v3dx_format_table.c:62): the XRGB8888 EGLImage the GL
/// core attaches (alpha reads 1). It has no GL format name.
pub fn bgrx8_egl_image() -> Row71 {
    row71("B8G8R8X8_UNORM")
}

/// An unsized (format, type) pair as an ES 2.0 upload names it, or the
/// sized internal format it stands for (ES 3.0.6 table 3.2). `None` when the
/// pair is not listed.
fn unsized_pipe(format: u32, ty: u32) -> Option<&'static str> {
    Some(match (format, ty) {
        (GL_RGBA, GL_UNSIGNED_BYTE) => "R8G8B8A8_UNORM",
        (GL_RGBA, GL_UNSIGNED_SHORT_4_4_4_4) => "A4B4G4R4_UNORM",
        (GL_RGBA, GL_UNSIGNED_SHORT_5_5_5_1) => "A1B5G5R5_UNORM",
        (GL_RGBA, GL_FLOAT) => "R32G32B32A32_FLOAT",
        (GL_RGBA, GL_HALF_FLOAT | GL_HALF_FLOAT_OES) => "R16G16B16A16_FLOAT",
        (GL_RGB, GL_UNSIGNED_BYTE) => "R8G8B8X8_UNORM",
        (GL_RGB, GL_UNSIGNED_SHORT_5_6_5) => "B5G6R5_UNORM",
        (GL_LUMINANCE, GL_UNSIGNED_BYTE) => "L8_UNORM",
        (GL_ALPHA, GL_UNSIGNED_BYTE) => "A8_UNORM",
        (GL_LUMINANCE_ALPHA, GL_UNSIGNED_BYTE) => "L8A8_UNORM",
        (GL_BGRA_EXT, GL_UNSIGNED_BYTE) => "B8G8R8A8_UNORM",
        (GL_DEPTH_COMPONENT, GL_UNSIGNED_SHORT) => "Z16_UNORM",
        (GL_DEPTH_COMPONENT, GL_UNSIGNED_INT) => "X8Z24_UNORM",
        (GL_DEPTH_COMPONENT, GL_FLOAT) => "Z32_FLOAT",
        (GL_DEPTH_STENCIL, GL_UNSIGNED_INT_24_8) => "S8_UINT_Z24_UNORM",
        (GL_DEPTH_STENCIL, GL_FLOAT_32_UNSIGNED_INT_24_8_REV) => "Z32_FLOAT_S8X24_UINT",
        _ => return None,
    })
}

/// A sized or compressed internal format's pipe format.
fn sized_pipe(internal: u32) -> Option<&'static str> {
    Some(match internal {
        0x8229 => "R8_UNORM",             // GL_R8
        0x8F94 => "R8_SNORM",             // GL_R8_SNORM
        0x822B => "R8G8_UNORM",           // GL_RG8
        0x8F95 => "R8G8_SNORM",           // GL_RG8_SNORM
        0x8051 => "R8G8B8X8_UNORM",       // GL_RGB8
        0x8F96 => "R8G8B8X8_SNORM",       // GL_RGB8_SNORM
        0x8058 => "R8G8B8A8_UNORM",       // GL_RGBA8
        0x8F97 => "R8G8B8A8_SNORM",       // GL_RGBA8_SNORM
        0x8C41 => "R8G8B8X8_SRGB",        // GL_SRGB8
        0x8C43 => "R8G8B8A8_SRGB",        // GL_SRGB8_ALPHA8
        0x8056 => "A4B4G4R4_UNORM",       // GL_RGBA4
        0x8057 => "A1B5G5R5_UNORM",       // GL_RGB5_A1
        0x8D62 => "B5G6R5_UNORM",         // GL_RGB565
        0x8059 => "R10G10B10A2_UNORM",    // GL_RGB10_A2
        0x906F => "R10G10B10A2_UINT",     // GL_RGB10_A2UI
        0x822D => "R16_FLOAT",            // GL_R16F
        0x822F => "R16G16_FLOAT",         // GL_RG16F
        0x881A => "R16G16B16A16_FLOAT",   // GL_RGBA16F
        0x822E => "R32_FLOAT",            // GL_R32F
        0x8230 => "R32G32_FLOAT",         // GL_RG32F
        0x8814 => "R32G32B32A32_FLOAT",   // GL_RGBA32F
        0x8C3A => "R11G11B10_FLOAT",      // GL_R11F_G11F_B10F
        0x8C3D => "R9G9B9E5_FLOAT",       // GL_RGB9_E5
        0x8231 => "R8_SINT",              // GL_R8I
        0x8232 => "R8_UINT",              // GL_R8UI
        0x8233 => "R16_SINT",             // GL_R16I
        0x8234 => "R16_UINT",             // GL_R16UI
        0x8235 => "R32_SINT",             // GL_R32I
        0x8236 => "R32_UINT",             // GL_R32UI
        0x8237 => "R8G8_SINT",            // GL_RG8I
        0x8238 => "R8G8_UINT",            // GL_RG8UI
        0x8239 => "R16G16_SINT",          // GL_RG16I
        0x823A => "R16G16_UINT",          // GL_RG16UI
        0x823B => "R32G32_SINT",          // GL_RG32I
        0x823C => "R32G32_UINT",          // GL_RG32UI
        0x8D70 => "R32G32B32A32_UINT",    // GL_RGBA32UI
        0x8D76 => "R16G16B16A16_UINT",    // GL_RGBA16UI
        0x8D7C => "R8G8B8A8_UINT",        // GL_RGBA8UI
        0x8D82 => "R32G32B32A32_SINT",    // GL_RGBA32I
        0x8D88 => "R16G16B16A16_SINT",    // GL_RGBA16I
        0x8D8E => "R8G8B8A8_SINT",        // GL_RGBA8I
        0x81A5 => "Z16_UNORM",            // GL_DEPTH_COMPONENT16
        0x81A6 => "X8Z24_UNORM",          // GL_DEPTH_COMPONENT24
        0x8CAC => "Z32_FLOAT",            // GL_DEPTH_COMPONENT32F
        0x88F0 => "S8_UINT_Z24_UNORM",    // GL_DEPTH24_STENCIL8
        0x8CAD => "Z32_FLOAT_S8X24_UINT", // GL_DEPTH32F_STENCIL8
        /* ETC2 / EAC (and ETC1, whose blocks are ETC2's: texture.rs Etc2::Rgb8) */
        0x8D64 | 0x9274 => "ETC2_RGB8", // GL_ETC1_RGB8_OES, COMPRESSED_RGB8_ETC2
        0x9275 => "ETC2_SRGB8",
        0x9276 => "ETC2_RGB8A1",
        0x9277 => "ETC2_SRGB8A1",
        0x9278 => "ETC2_RGBA8",
        0x9279 => "ETC2_SRGBA8",
        0x9270 => "ETC2_R11_UNORM",
        0x9271 => "ETC2_R11_SNORM",
        0x9272 => "ETC2_RG11_UNORM",
        0x9273 => "ETC2_RG11_SNORM",
        /* S3TC (EXT_texture_compression_s3tc, EXT_texture_sRGB) */
        0x83F0 => "DXT1_RGB",
        0x83F1 => "DXT1_RGBA",
        0x83F2 => "DXT3_RGBA",
        0x83F3 => "DXT5_RGBA",
        0x8C4C => "DXT1_SRGB",
        0x8C4D => "DXT1_SRGBA",
        0x8C4E => "DXT3_SRGBA",
        0x8C4F => "DXT5_SRGBA",
        /* ASTC LDR (KHR_texture_compression_astc_ldr): the RGBA and the
         * SRGB8_ALPHA8 enums, in the block-size order of the table */
        0x93B0 => "ASTC_4x4",
        0x93B1 => "ASTC_5x4",
        0x93B2 => "ASTC_5x5",
        0x93B3 => "ASTC_6x5",
        0x93B4 => "ASTC_6x6",
        0x93B5 => "ASTC_8x5",
        0x93B6 => "ASTC_8x6",
        0x93B7 => "ASTC_8x8",
        0x93B8 => "ASTC_10x5",
        0x93B9 => "ASTC_10x6",
        0x93BA => "ASTC_10x8",
        0x93BB => "ASTC_10x10",
        0x93BC => "ASTC_12x10",
        0x93BD => "ASTC_12x12",
        0x93D0 => "ASTC_4x4_SRGB",
        0x93D1 => "ASTC_5x4_SRGB",
        0x93D2 => "ASTC_5x5_SRGB",
        0x93D3 => "ASTC_6x5_SRGB",
        0x93D4 => "ASTC_6x6_SRGB",
        0x93D5 => "ASTC_8x5_SRGB",
        0x93D6 => "ASTC_8x6_SRGB",
        0x93D7 => "ASTC_8x8_SRGB",
        0x93D8 => "ASTC_10x5_SRGB",
        0x93D9 => "ASTC_10x6_SRGB",
        0x93DA => "ASTC_10x8_SRGB",
        0x93DB => "ASTC_10x10_SRGB",
        0x93DC => "ASTC_12x10_SRGB",
        0x93DD => "ASTC_12x12_SRGB",
        _ => return None,
    })
}

/// The GL formats Mesa's table has no row for although ES offers them:
/// RGB16F, RGB32F and the six RGB integer formats (each a
/// [`Refusal71::NoThreeChannelRow`]), and STENCIL_INDEX8.
fn refusal_of(internal: u32) -> Refusal71 {
    match internal {
        0x881B | 0x8815 | 0x8D7D | 0x8D8F | 0x8D77 | 0x8D89 | 0x8D71 | 0x8D83 => Refusal71::NoThreeChannelRow,
        0x8D48 => Refusal71::StencilTexturing,
        _ => Refusal71::UnknownFormat,
    }
}

/// The 7.1 TMU answer for a GL texture's internal format and (for an unsized
/// one) its type. A sized or compressed `internal` ignores `ty`.
pub fn tmu_format71(internal: u32, ty: u32) -> Result<Row71, Refusal71> {
    match internal {
        GL_RGBA | GL_RGB | GL_LUMINANCE | GL_ALPHA | GL_LUMINANCE_ALPHA | GL_DEPTH_COMPONENT | GL_DEPTH_STENCIL
        | GL_BGRA_EXT => unsized_pipe(internal, ty).map(row71).ok_or(Refusal71::UnsizedTypeNotListed),
        _ => match sized_pipe(internal) {
            Some(p) => Ok(row71(p)),
            None => Err(refusal_of(internal)),
        },
    }
}
