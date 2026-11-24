// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GL texture and sampler state to the inputs of the merged 7.1 texture
//! modules (P10 of docs/notes/pre-study-rpi5.md; docs/notes/v3d71-gl-table.md
//! section 3), for the 7.1 GL table (`src/v3d/table71.rs`, plan #404).
//!
//! The GL core hands a back end one `GlesTextureBind` per sampled unit
//! (src/backend.rs): a storage class (`GLES_TEXFMT_*`, not the GL internal
//! format), a texture type (`GLES_TEXTYPE_*`), GL's swizzle as
//! `GlesSwizzle` codes in base-format terms, `no_alpha`, `bgra` and
//! `depth_luminance`, wrap modes as `GlesWrap` codes, the filters as
//! `linear_filter` / `min_linear` / `mip_filter` and the depth compare as
//! `compare_op`. The 4.2 table reads them in src/v3d/texture.rs
//! (`tmu_format`, `tmu_swizzle`, `sampler`, `size_words`). [`GlTexBind71`]
//! and [`GlSampler71`] carry the same values as plain fields (no `crate::`
//! import), plus what ES 3.0 and the extensions add that the bind does not
//! carry yet (max level, LOD range and bias, border colour, sRGB decode,
//! anisotropy, the seamless request). [`glue`] gives
//!
//! * [`texture71::GlTex71`], the input of `texture71::compose` (and of
//!   `texfill71::fill`): the storage class as the GL internal format and
//!   type `format71::tmu_format71` takes, and GL's swizzle with the class's
//!   own rule (`no_alpha`, `bgra`, `depth_luminance`) composed in;
//! * [`sampler71::Input`], the input of `sampler71::sampler71` (and of
//!   `texfill71::fill`): filters, wrap modes, compare, border;
//! * the texture-size words of the unit, and [`Glued71::unit`] for a
//!   [`texuni71::Unit`] once the table has placed the two records (TMU
//!   Config Parameter 0 and 1 point at them);
//!
//! and runs both modules once, so a refusal of theirs comes back here,
//! wrapped ([`Refusal::Texture`], [`Refusal::Sampler`]), and the composed
//! values ride along for the caller ([`Glued71::composed`],
//! [`Glued71::sampler_values`]).
//!
//! Rules (each one Mesa's, or the 4.2 table's where the GL core decides):
//!
//! * the class's base swizzle comes before GL's: `no_alpha` (and `bgra`, an
//!   XRGB8888 EGLImage, B8G8R8X8_UNORM at v3dx_format_table.c:62) reads
//!   alpha 1, `depth_luminance` reads (d, d, d, 1), as 4.2's `tmu_format`;
//!   GL's swizzle then picks from that (4.2's `tmu_swizzle`, Mesa's
//!   `util_format_compose_swizzles`), and texture71 composes the result with
//!   the format row's own swizzle;
//! * a cube map is sampled seamlessly (sampler71: 7.1 cannot turn that off)
//!   with wrap S, T and R all CLAMP_TO_EDGE, as Mesa's state tracker forces
//!   them and the gl_cube references show (wrap R 1 though GL left it at
//!   REPEAT; tests/v3d71_compose3.rs, `texfill71::sampler_input_for`);
//! * the depth compare is applied to a depth texture only (the GL core's
//!   R32F class, 4.2's `compare_op`), ES 3.0.6 3.8.15 bypasses it otherwise;
//! * the size words are the base level's width and height minified
//!   (`u_minify`), depth and array size 1 for a 2D texture and a cube
//!   (Mesa's `get_texture_size`, 4.2's `size_words`);
//! * no rule reads HUB_IDENT1 bit 17 (texstate71 checks the revision).
//!
//! What it does not cover is refused by name ([`Refusal`], the style of
//! docs/notes/v3d71-refusals.md): 2D arrays, 3D textures, the twiddled
//! probes, the GL core's incomplete texture (the table binds its own black
//! stand-in, GL ES 2.0 3.7.10), codes outside the GL core's enums. The
//! compressed classes are passed on to texture71, which refuses them
//! (`texture71::Refusal::Compressed`).
//!
//! Pure: the siblings are reached through `super::`, so a test includes this
//! with `#[path]` next to `format71.rs`, `texstate71.rs`, `texture71.rs`,
//! `sampler71.rs` and `texuni71.rs` (tests/v3d71_texglue.rs). Not in
//! `src/v3d/mod.rs` yet.
//!
//! How the 7.1 table calls it:
//! ```text
//! let g = texglue71::glue(&bind, &smp, ver, iprev)?;     // refusals are named
//! g.check_return_size(p1_data)?;                         // the program's P1 bit 0
//! // g.tex / g.sampler: texture71::compose, sampler71::sampler71 (or
//! // texfill71::fill) for the two records; place them, then
//! let unit = g.unit(tex_state_addr, sampler_addr);       // texuni71::word(.., &[unit])
//! ```

#![allow(dead_code)]

use std::fmt;

use super::format71;
use super::sampler71::{self, Border, Filter, Input, MipFilter, Sampler71, Wrap};
use super::texture71::{self, GlTex71, Target, Tex71};
use super::texuni71::Unit;

// ---- the GL core's codes (src/backend.rs; copied, this module is pure) ----

/// `GlesTexType` (src/backend.rs).
pub const GLES_TEXTYPE_2D: u8 = 0;
pub const GLES_TEXTYPE_CUBE: u8 = 1;
pub const GLES_TEXTYPE_2D_ARRAY: u8 = 2;
pub const GLES_TEXTYPE_3D: u8 = 3;
pub const GLES_TEXTYPE_CUBE_TWIDDLED_PROBE: u8 = 4;
pub const GLES_TEXTYPE_2D_ARRAY_TWIDDLED: u8 = 5;
pub const GLES_TEXTYPE_3D_TWIDDLED: u8 = 6;

/// `GLES_TEXFMT_*` (src/backend.rs), the storage classes.
pub const GLES_TEXFMT_RGBA8: u8 = 0;
pub const GLES_TEXFMT_ETC2_RGB: u8 = 1;
pub const GLES_TEXFMT_ETC2_RGBA: u8 = 2;
pub const GLES_TEXFMT_ETC2_RGB_PT: u8 = 3;
pub const GLES_TEXFMT_RGB565: u8 = 4;
pub const GLES_TEXFMT_RGBA16F: u8 = 5;
pub const GLES_TEXFMT_R16F: u8 = 6;
pub const GLES_TEXFMT_R32F: u8 = 7;
pub const GLES_TEXFMT_L8: u8 = 8;
pub const GLES_TEXFMT_A8: u8 = 9;
pub const GLES_TEXFMT_LA8: u8 = 10;
pub const GLES_TEXFMT_R11G11B10F: u8 = 11;
pub const GLES_TEXFMT_RGB9E5: u8 = 12;
pub const GLES_TEXFMT_RGB10A2: u8 = 13;
pub const GLES_TEXFMT_RGB10A2UI: u8 = 14;
pub const GLES_TEXFMT_RGBA8_SNORM: u8 = 15;
pub const GLES_TEXFMT_RGBA8UI: u8 = 16;
pub const GLES_TEXFMT_RGBA8I: u8 = 17;
pub const GLES_TEXFMT_RGBA16UI: u8 = 18;
pub const GLES_TEXFMT_RGBA16I: u8 = 19;
pub const GLES_TEXFMT_RGBA32UI: u8 = 20;
pub const GLES_TEXFMT_RGBA32I: u8 = 21;
pub const GLES_TEXFMT_RGBA32F: u8 = 22;
pub const GLES_TEXFMT_SRGB8_A8: u8 = 23;
pub const GLES_TEXFMT_ETC2_SRGB_RGB: u8 = 24;
pub const GLES_TEXFMT_ETC2_SRGB_RGBA: u8 = 25;
pub const GLES_TEXFMT_R32UI: u8 = 26;
pub const GLES_TEXFMT_R32I: u8 = 27;
pub const GLES_TEXFMT_RG32UI: u8 = 28;
pub const GLES_TEXFMT_RG32I: u8 = 29;
pub const GLES_TEXFMT_R8_SNORM: u8 = 30;
pub const GLES_TEXFMT_RG8_SNORM: u8 = 31;
pub const GLES_TEXFMT_R8UI: u8 = 32;
pub const GLES_TEXFMT_R8I: u8 = 33;
pub const GLES_TEXFMT_RG8UI: u8 = 34;
pub const GLES_TEXFMT_RG8I: u8 = 35;
pub const GLES_TEXFMT_R16UI: u8 = 36;
pub const GLES_TEXFMT_R16I: u8 = 37;
pub const GLES_TEXFMT_RG16UI: u8 = 38;
pub const GLES_TEXFMT_RG16I: u8 = 39;

/// `GlesSwizzle` (src/backend.rs): 0 is the component itself.
pub const GLES_SWIZZLE_SAME: u8 = 0;
pub const GLES_SWIZZLE_RED: u8 = 1;
pub const GLES_SWIZZLE_GREEN: u8 = 2;
pub const GLES_SWIZZLE_BLUE: u8 = 3;
pub const GLES_SWIZZLE_ALPHA: u8 = 4;
pub const GLES_SWIZZLE_ZERO: u8 = 5;
pub const GLES_SWIZZLE_ONE: u8 = 6;

/// `GlesWrap` (src/backend.rs), and two codes this module takes ahead of
/// the GL core (GL_CLAMP_TO_BORDER of ES 3.2 / EXT_texture_border_clamp,
/// GL_MIRROR_CLAMP_TO_EDGE of EXT_texture_mirror_clamp_to_edge), which have
/// no `GlesWrap` value yet.
pub const GLES_WRAP_REPEAT: u8 = 0;
pub const GLES_WRAP_MIRRORED_REPEAT: u8 = 1;
pub const GLES_WRAP_CLAMP_TO_EDGE: u8 = 2;
pub const WRAP_CLAMP_TO_BORDER: u8 = 3;
pub const WRAP_MIRROR_CLAMP_TO_EDGE: u8 = 4;

/// `GlesTextureBind::compare_op`'s bit 4: no compare (GL_NONE).
pub const COMPARE_OFF: u8 = 16;

/// GL internal formats and types the storage classes stand for (the
/// Khronos registry's values), where format71 has no name for them.
const GL_RGBA8: u32 = 0x8058;
const GL_RGB8: u32 = 0x8051;
const GL_SRGB8_ALPHA8: u32 = 0x8C43;
const GL_RGB565: u32 = 0x8D62;
const GL_RGBA16F: u32 = 0x881A;
const GL_R16F: u32 = 0x822D;
const GL_R32F: u32 = 0x822E;
const GL_R11F_G11F_B10F: u32 = 0x8C3A;
const GL_RGB9_E5: u32 = 0x8C3D;
const GL_RGB10_A2: u32 = 0x8059;
const GL_RGB10_A2UI: u32 = 0x906F;
const GL_RGBA8_SNORM: u32 = 0x8F97;
const GL_RGBA8UI: u32 = 0x8D7C;
const GL_RGBA8I: u32 = 0x8D8E;
const GL_RGBA16UI: u32 = 0x8D76;
const GL_RGBA16I: u32 = 0x8D88;
const GL_RGBA32UI: u32 = 0x8D70;
const GL_RGBA32I: u32 = 0x8D82;
const GL_RGBA32F: u32 = 0x8814;
const GL_R32UI: u32 = 0x8236;
const GL_R32I: u32 = 0x8235;
const GL_RG32UI: u32 = 0x823C;
const GL_RG32I: u32 = 0x823B;
const GL_R8_SNORM: u32 = 0x8F94;
const GL_RG8_SNORM: u32 = 0x8F95;
const GL_R8UI: u32 = 0x8232;
const GL_R8I: u32 = 0x8231;
const GL_RG8UI: u32 = 0x8238;
const GL_RG8I: u32 = 0x8237;
const GL_R16UI: u32 = 0x8234;
const GL_R16I: u32 = 0x8233;
const GL_RG16UI: u32 = 0x823A;
const GL_RG16I: u32 = 0x8239;
const GL_COMPRESSED_RGB8_ETC2: u32 = 0x9274;
const GL_COMPRESSED_SRGB8_ETC2: u32 = 0x9275;
const GL_COMPRESSED_RGB8_PUNCHTHROUGH_ALPHA1_ETC2: u32 = 0x9276;
const GL_COMPRESSED_RGBA8_ETC2_EAC: u32 = 0x9278;
const GL_COMPRESSED_SRGB8_ALPHA8_ETC2_EAC: u32 = 0x9279;

// ---- the inputs -------------------------------------------------------------

/// One sampled texture as the GL core describes it (`GlesTextureBind`'s
/// texture half, plus GL_TEXTURE_MAX_LEVEL and sRGB decode).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GlTexBind71 {
    /// the texture unit (the compiler's TMU config `data >> 24`)
    pub unit: u32,
    /// `GLES_TEXTYPE_*`
    pub tex_type: u8,
    /// `GLES_TEXFMT_*`
    pub texfmt: u8,
    /// images in the stack: 6 for a cube, 1 for a 2D texture
    pub layers: u32,
    /// level 0's size (a cube: one face's)
    pub width: u32,
    pub height: u32,
    /// levels the storage has, from level 0 (`mip_levels`)
    pub levels: u32,
    /// GL_TEXTURE_BASE_LEVEL, GL_TEXTURE_MAX_LEVEL (default 0, 1000)
    pub base_level: u32,
    pub max_level: u32,
    /// GL_TEXTURE_SWIZZLE_R/G/B/A as `GlesSwizzle` codes (all 0: identity)
    pub swizzle: [u8; 4],
    /// the base format has no alpha: alpha reads 1
    pub no_alpha: bool,
    /// an XRGB8888 EGLImage, B, G, R, X in memory (alpha reads 1)
    pub bgra: bool,
    /// a depth texture read as luminance (d, d, d, 1), ES 2.0's
    /// GL_OES_depth_texture
    pub depth_luminance: bool,
    /// GL_TEXTURE_SRGB_DECODE_EXT is DECODE (the default)
    pub srgb_decode: bool,
    /// the GL core's incomplete texture (samples read (0, 0, 0, 1))
    pub incomplete: bool,
    /// GPU address of the texture's buffer
    pub bo_address: u32,
}

impl GlTexBind71 {
    /// A complete 2D texture of `texfmt`, `w` x `h` with `levels` levels,
    /// GL's defaults for the rest, at `bo_address`.
    pub const fn tex2d(texfmt: u8, w: u32, h: u32, levels: u32, bo_address: u32) -> GlTexBind71 {
        GlTexBind71 {
            unit: 0,
            tex_type: GLES_TEXTYPE_2D,
            texfmt,
            layers: 1,
            width: w,
            height: h,
            levels,
            base_level: 0,
            max_level: texture71::GL_MAX_LEVEL_DEFAULT,
            swizzle: [GLES_SWIZZLE_SAME; 4],
            no_alpha: false,
            bgra: false,
            depth_luminance: false,
            srgb_decode: true,
            incomplete: false,
            bo_address,
        }
    }

    /// The same, a cube map of six `size` x `size` faces.
    pub const fn cube(texfmt: u8, size: u32, levels: u32, bo_address: u32) -> GlTexBind71 {
        let mut t = GlTexBind71::tex2d(texfmt, size, size, levels, bo_address);
        t.tex_type = GLES_TEXTYPE_CUBE;
        t.layers = 6;
        t
    }
}

/// The sampler half (`GlesTextureBind`'s filters, wraps and compare, the
/// sampler object's where one is bound), plus the ES 3.0 / extension state
/// the bind does not carry yet.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GlSampler71 {
    /// GL_TEXTURE_MAG_FILTER is LINEAR (`linear_filter`)
    pub mag_linear: bool,
    /// the minification filter's base is LINEAR (`min_linear`)
    pub min_linear: bool,
    /// 0 no mipmaps, 1 nearest level, 2 linear between levels
    pub mip_filter: u8,
    /// `GlesWrap` codes (and [`WRAP_CLAMP_TO_BORDER`],
    /// [`WRAP_MIRROR_CLAMP_TO_EDGE`])
    pub wrap_s: u8,
    pub wrap_t: u8,
    pub wrap_r: u8,
    /// GL_TEXTURE_MIN_LOD, _MAX_LOD, _LOD_BIAS (-1000, 1000, 0)
    pub min_lod: f32,
    pub max_lod: f32,
    pub lod_bias: f32,
    /// `GlesTextureBind::compare_op`: [`COMPARE_OFF`] or the function in
    /// bits 0..2 (bit 3, a fixed-point depth, is the shader's)
    pub compare_op: u8,
    /// GL_TEXTURE_MAX_ANISOTROPY_EXT as Gallium keeps it (0 off)
    pub max_anisotropy: u32,
    /// GL_TEXTURE_BORDER_COLOR (read only under CLAMP_TO_BORDER)
    pub border: Border,
    /// GL_TEXTURE_CUBE_MAP_SEAMLESS (ES 3.0: always)
    pub seamless: bool,
}

impl GlSampler71 {
    /// GL's defaults with the given filters: wrap REPEAT, LOD range
    /// (-1000, 1000), no bias, no compare, no anisotropy, border 0,
    /// seamless.
    pub const fn filters(mag_linear: bool, min_linear: bool, mip_filter: u8) -> GlSampler71 {
        GlSampler71 {
            mag_linear,
            min_linear,
            mip_filter,
            wrap_s: GLES_WRAP_REPEAT,
            wrap_t: GLES_WRAP_REPEAT,
            wrap_r: GLES_WRAP_REPEAT,
            min_lod: -1000.0,
            max_lod: 1000.0,
            lod_bias: 0.0,
            compare_op: COMPARE_OFF,
            max_anisotropy: 0,
            border: Border::Float([0.0; 4]),
            seamless: true,
        }
    }
}

// ---- the refusals -------------------------------------------------------------

/// Why a GL texture and sampler pair has no 7.1 inputs. Every variant is a
/// decision; the two last pass the module's own refusal on.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Refusal {
    /// a 2D array (`GLES_TEXTYPE_2D_ARRAY`): texture71 lays out 2D and
    /// cube only
    Array2d,
    /// a 3D texture (`GLES_TEXTYPE_3D`): texture71 has no 3D layout
    /// (`level_depth`'s slices)
    Texture3d,
    /// one of the twiddled probe types (`GLES_TEXTYPE_*_TWIDDLED*`), the
    /// PowerVR's
    TwiddledProbe { tex_type: u8 },
    /// a `GlesTexType` value the GL core does not define
    UnknownType { tex_type: u8 },
    /// a 2D binding with more than one layer
    Layers2d { layers: u32 },
    /// a cube binding without its six faces
    CubeLayers { layers: u32 },
    /// the GL core's incomplete texture: the table binds its own 1x1
    /// (0, 0, 0, 1) stand-in, these inputs do not describe it
    Incomplete,
    /// a `GLES_TEXFMT_*` value the GL core does not define
    UnknownTexfmt { texfmt: u8 },
    /// `bgra` on a class other than RGBA8 (the GL core sets it for an
    /// XRGB8888 EGLImage only)
    BgraNotRgba8 { texfmt: u8 },
    /// `depth_luminance` on a class other than the depth class (R32F)
    DepthLuminanceNotDepth { texfmt: u8 },
    /// a swizzle code outside `GlesSwizzle`
    SwizzleCode { value: u8 },
    /// a wrap code outside `GlesWrap` and the two codes taken ahead
    WrapCode { value: u8 },
    /// a mip filter code other than 0, 1, 2
    MipFilterCode { value: u8 },
    /// a compare function code with bits outside 0..3 and [`COMPARE_OFF`]
    CompareCode { value: u8 },
    /// CLAMP_TO_BORDER with a border colour of the other class (float on
    /// an integer format or integer on a float one)
    BorderClass { integer_format: bool },
    /// the program's TMU output size (TMU Config Parameter 1 bit 0) is not
    /// the format's return size (a 16-bit read of a 32-bit format or the
    /// reverse: not a measured behaviour, 4.2's `RET32_OF_16` /
    /// `RET16_OF_32`)
    ReturnSize { format_32: bool, program_32: bool },
    /// texture71 refused the texture
    Texture(texture71::Refusal),
    /// sampler71 refused the sampler
    Sampler(sampler71::Refusal),
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Refusal::Array2d => write!(f, "texture glue 7.1: a 2D array texture is not laid out by texture71"),
            Refusal::Texture3d => write!(f, "texture glue 7.1: a 3D texture is not laid out by texture71"),
            Refusal::TwiddledProbe { tex_type } => {
                write!(f, "texture glue 7.1: twiddled probe texture type {tex_type} is not a V3D binding")
            }
            Refusal::UnknownType { tex_type } => write!(f, "texture glue 7.1: unknown texture type {tex_type}"),
            Refusal::Layers2d { layers } => write!(f, "texture glue 7.1: a 2D texture binding with {layers} layers"),
            Refusal::CubeLayers { layers } => {
                write!(f, "texture glue 7.1: a cube map binding with {layers} layers, not its six faces")
            }
            Refusal::Incomplete => {
                write!(f, "texture glue 7.1: an incomplete texture, the table binds its black stand-in")
            }
            Refusal::UnknownTexfmt { texfmt } => write!(f, "texture glue 7.1: unknown texel format class {texfmt}"),
            Refusal::BgraNotRgba8 { texfmt } => {
                write!(f, "texture glue 7.1: bgra set on texel format class {texfmt}, not RGBA8")
            }
            Refusal::DepthLuminanceNotDepth { texfmt } => {
                write!(f, "texture glue 7.1: depth luminance set on texel format class {texfmt}, not the depth class")
            }
            Refusal::SwizzleCode { value } => write!(f, "texture glue 7.1: swizzle code {value} is not a GlesSwizzle"),
            Refusal::WrapCode { value } => write!(f, "texture glue 7.1: wrap code {value} is not a wrap mode"),
            Refusal::MipFilterCode { value } => write!(f, "texture glue 7.1: mip filter code {value} is not 0, 1 or 2"),
            Refusal::CompareCode { value } => write!(f, "texture glue 7.1: compare code {value:#x} is not one"),
            Refusal::BorderClass { integer_format } => {
                if *integer_format {
                    write!(f, "texture glue 7.1: a float border colour on an integer format")
                } else {
                    write!(f, "texture glue 7.1: an integer border colour on a float format")
                }
            }
            Refusal::ReturnSize { format_32, program_32 } => write!(
                f,
                "texture glue 7.1: the program reads {} bits where the format returns {}",
                if *program_32 { 32 } else { 16 },
                if *format_32 { 32 } else { 16 }
            ),
            Refusal::Texture(r) => write!(f, "{r}"),
            Refusal::Sampler(r) => write!(f, "{}", r.message()),
        }
    }
}

// ---- the mapping ------------------------------------------------------------------

/// The GL internal format and type `format71::tmu_format71` takes for a
/// storage class, as Mesa would hold the same memory (4.2's `tmu_format`
/// rows); `no_alpha` RGBA8 is GL_RGB8 (R8G8B8X8_UNORM, XYZ1), `bgra` is
/// GL_BGRA_EXT (B8G8R8A8_UNORM; alpha 1 comes from the base swizzle).
pub fn gl_format_of(texfmt: u8, no_alpha: bool, bgra: bool) -> Result<(u32, u32), Refusal> {
    use format71::{GL_ALPHA, GL_BGRA_EXT, GL_LUMINANCE, GL_LUMINANCE_ALPHA, GL_UNSIGNED_BYTE};
    if bgra && texfmt != GLES_TEXFMT_RGBA8 {
        return Err(Refusal::BgraNotRgba8 { texfmt });
    }
    let sized = |f: u32| Ok((f, GL_UNSIGNED_BYTE));
    match texfmt {
        GLES_TEXFMT_RGBA8 if bgra => Ok((GL_BGRA_EXT, GL_UNSIGNED_BYTE)),
        GLES_TEXFMT_RGBA8 if no_alpha => sized(GL_RGB8),
        GLES_TEXFMT_RGBA8 => sized(GL_RGBA8),
        GLES_TEXFMT_ETC2_RGB => sized(GL_COMPRESSED_RGB8_ETC2),
        GLES_TEXFMT_ETC2_RGBA => sized(GL_COMPRESSED_RGBA8_ETC2_EAC),
        GLES_TEXFMT_ETC2_RGB_PT => sized(GL_COMPRESSED_RGB8_PUNCHTHROUGH_ALPHA1_ETC2),
        GLES_TEXFMT_RGB565 => sized(GL_RGB565),
        GLES_TEXFMT_RGBA16F => sized(GL_RGBA16F),
        GLES_TEXFMT_R16F => sized(GL_R16F),
        GLES_TEXFMT_R32F => sized(GL_R32F),
        GLES_TEXFMT_L8 => Ok((GL_LUMINANCE, GL_UNSIGNED_BYTE)),
        GLES_TEXFMT_A8 => Ok((GL_ALPHA, GL_UNSIGNED_BYTE)),
        GLES_TEXFMT_LA8 => Ok((GL_LUMINANCE_ALPHA, GL_UNSIGNED_BYTE)),
        GLES_TEXFMT_R11G11B10F => sized(GL_R11F_G11F_B10F),
        GLES_TEXFMT_RGB9E5 => sized(GL_RGB9_E5),
        GLES_TEXFMT_RGB10A2 => sized(GL_RGB10_A2),
        GLES_TEXFMT_RGB10A2UI => sized(GL_RGB10_A2UI),
        GLES_TEXFMT_RGBA8_SNORM => sized(GL_RGBA8_SNORM),
        GLES_TEXFMT_RGBA8UI => sized(GL_RGBA8UI),
        GLES_TEXFMT_RGBA8I => sized(GL_RGBA8I),
        GLES_TEXFMT_RGBA16UI => sized(GL_RGBA16UI),
        GLES_TEXFMT_RGBA16I => sized(GL_RGBA16I),
        GLES_TEXFMT_RGBA32UI => sized(GL_RGBA32UI),
        GLES_TEXFMT_RGBA32I => sized(GL_RGBA32I),
        GLES_TEXFMT_RGBA32F => sized(GL_RGBA32F),
        GLES_TEXFMT_SRGB8_A8 => sized(GL_SRGB8_ALPHA8),
        GLES_TEXFMT_ETC2_SRGB_RGB => sized(GL_COMPRESSED_SRGB8_ETC2),
        GLES_TEXFMT_ETC2_SRGB_RGBA => sized(GL_COMPRESSED_SRGB8_ALPHA8_ETC2_EAC),
        GLES_TEXFMT_R32UI => sized(GL_R32UI),
        GLES_TEXFMT_R32I => sized(GL_R32I),
        GLES_TEXFMT_RG32UI => sized(GL_RG32UI),
        GLES_TEXFMT_RG32I => sized(GL_RG32I),
        GLES_TEXFMT_R8_SNORM => sized(GL_R8_SNORM),
        GLES_TEXFMT_RG8_SNORM => sized(GL_RG8_SNORM),
        GLES_TEXFMT_R8UI => sized(GL_R8UI),
        GLES_TEXFMT_R8I => sized(GL_R8I),
        GLES_TEXFMT_RG8UI => sized(GL_RG8UI),
        GLES_TEXFMT_RG8I => sized(GL_RG8I),
        GLES_TEXFMT_R16UI => sized(GL_R16UI),
        GLES_TEXFMT_R16I => sized(GL_R16I),
        GLES_TEXFMT_RG16UI => sized(GL_RG16UI),
        GLES_TEXFMT_RG16I => sized(GL_RG16I),
        _ => Err(Refusal::UnknownTexfmt { texfmt }),
    }
}

/// The GL-level swizzle texture71 composes with the format row: the class's
/// base rule (alpha 1 for `no_alpha` and `bgra`, (R, R, R, 1) for
/// `depth_luminance`), then GL's `GlesSwizzle` codes picking from it.
pub fn gl_swizzle(t: &GlTexBind71) -> Result<[u32; 4], Refusal> {
    use texture71::{GL_ONE, GL_RED, GL_ZERO};
    let mut base = texture71::GL_SWIZZLE_IDENTITY;
    if t.depth_luminance {
        if t.texfmt != GLES_TEXFMT_R32F {
            return Err(Refusal::DepthLuminanceNotDepth { texfmt: t.texfmt });
        }
        base = [GL_RED, GL_RED, GL_RED, GL_ONE];
    }
    if t.no_alpha || t.bgra {
        base[3] = GL_ONE;
    }
    let mut out = [0u32; 4];
    for (k, (o, s)) in out.iter_mut().zip(t.swizzle).enumerate() {
        *o = match s {
            GLES_SWIZZLE_SAME => base[k],
            GLES_SWIZZLE_RED..=GLES_SWIZZLE_ALPHA => base[(s - GLES_SWIZZLE_RED) as usize],
            GLES_SWIZZLE_ZERO => GL_ZERO,
            GLES_SWIZZLE_ONE => GL_ONE,
            value => return Err(Refusal::SwizzleCode { value }),
        };
    }
    Ok(out)
}

/// The texture71 target of a binding, or the refusal of its type.
pub fn target_of(t: &GlTexBind71) -> Result<Target, Refusal> {
    match t.tex_type {
        GLES_TEXTYPE_2D if t.layers <= 1 => Ok(Target::Tex2d),
        GLES_TEXTYPE_2D => Err(Refusal::Layers2d { layers: t.layers }),
        GLES_TEXTYPE_CUBE if t.layers == 6 => Ok(Target::Cube),
        GLES_TEXTYPE_CUBE => Err(Refusal::CubeLayers { layers: t.layers }),
        GLES_TEXTYPE_2D_ARRAY => Err(Refusal::Array2d),
        GLES_TEXTYPE_3D => Err(Refusal::Texture3d),
        GLES_TEXTYPE_CUBE_TWIDDLED_PROBE | GLES_TEXTYPE_2D_ARRAY_TWIDDLED | GLES_TEXTYPE_3D_TWIDDLED => {
            Err(Refusal::TwiddledProbe { tex_type: t.tex_type })
        }
        tex_type => Err(Refusal::UnknownType { tex_type }),
    }
}

/// texture71's input for a binding at hub version `ver`, IPREV `iprev`.
pub fn gl_tex(t: &GlTexBind71, ver: u32, iprev: u32) -> Result<GlTex71, Refusal> {
    let target = target_of(t)?;
    if t.incomplete {
        return Err(Refusal::Incomplete);
    }
    let (internal_format, ty) = gl_format_of(t.texfmt, t.no_alpha, t.bgra)?;
    Ok(GlTex71 {
        target,
        internal_format,
        ty,
        width: t.width,
        height: t.height,
        levels: t.levels,
        base_level: t.base_level,
        max_level: t.max_level,
        swizzle: gl_swizzle(t)?,
        srgb_decode: t.srgb_decode,
        bo_address: t.bo_address,
        ver,
        iprev,
    })
}

/// A `GlesWrap` code (or one taken ahead) as sampler71's mode.
pub fn wrap_of(value: u8) -> Result<Wrap, Refusal> {
    Ok(match value {
        GLES_WRAP_REPEAT => Wrap::Repeat,
        GLES_WRAP_MIRRORED_REPEAT => Wrap::MirroredRepeat,
        GLES_WRAP_CLAMP_TO_EDGE => Wrap::ClampToEdge,
        WRAP_CLAMP_TO_BORDER => Wrap::ClampToBorder,
        WRAP_MIRROR_CLAMP_TO_EDGE => Wrap::MirrorClampToEdge,
        value => return Err(Refusal::WrapCode { value }),
    })
}

fn filter(linear: bool) -> Filter {
    if linear {
        Filter::Linear
    } else {
        Filter::Nearest
    }
}

/// Is the storage class one of integers (an `isampler` / `usampler` reads
/// it raw; its border colour is integer)?
pub const fn texfmt_is_integer(texfmt: u8) -> bool {
    matches!(
        texfmt,
        GLES_TEXFMT_RGB10A2UI
            | GLES_TEXFMT_RGBA8UI
            | GLES_TEXFMT_RGBA8I
            | GLES_TEXFMT_RGBA16UI
            | GLES_TEXFMT_RGBA16I
            | GLES_TEXFMT_RGBA32UI
            | GLES_TEXFMT_RGBA32I
            | GLES_TEXFMT_R32UI
            | GLES_TEXFMT_R32I
            | GLES_TEXFMT_RG32UI
            | GLES_TEXFMT_RG32I
            | GLES_TEXFMT_R8UI
            | GLES_TEXFMT_R8I
            | GLES_TEXFMT_RG8UI
            | GLES_TEXFMT_RG8I
            | GLES_TEXFMT_R16UI
            | GLES_TEXFMT_R16I
            | GLES_TEXFMT_RG16UI
            | GLES_TEXFMT_RG16I
    )
}

/// sampler71's input for a sampler on a texture of `target` and storage
/// class `texfmt`.
pub fn sampler_input(s: &GlSampler71, target: Target, texfmt: u8) -> Result<Input, Refusal> {
    let mip_filter = match s.mip_filter {
        0 => MipFilter::None,
        1 => MipFilter::Nearest,
        2 => MipFilter::Linear,
        value => return Err(Refusal::MipFilterCode { value }),
    };
    let (mut wrap_s, mut wrap_t, mut wrap_r) = (wrap_of(s.wrap_s)?, wrap_of(s.wrap_t)?, wrap_of(s.wrap_r)?);
    // COMPARE_OFF, or a function in bits 0..2 with bit 3 (fixed point)
    if s.compare_op != COMPARE_OFF && s.compare_op > 0xf {
        return Err(Refusal::CompareCode { value: s.compare_op });
    }
    // the compare applies to the depth class only (ES 3.0.6 3.8.15)
    let compare_enabled = s.compare_op != COMPARE_OFF && texfmt == GLES_TEXFMT_R32F;
    let cube = target == Target::Cube;
    if cube && s.seamless {
        wrap_s = Wrap::ClampToEdge;
        wrap_t = Wrap::ClampToEdge;
        wrap_r = Wrap::ClampToEdge;
    }
    let integer_format = texfmt_is_integer(texfmt);
    let uses_border = [wrap_s, wrap_t, wrap_r].contains(&Wrap::ClampToBorder);
    if uses_border && matches!(s.border, Border::Int(_)) != integer_format {
        return Err(Refusal::BorderClass { integer_format });
    }
    Ok(Input {
        min_filter: filter(s.min_linear),
        mag_filter: filter(s.mag_linear),
        mip_filter,
        wrap_s,
        wrap_t,
        wrap_r,
        min_lod: s.min_lod,
        max_lod: s.max_lod,
        lod_bias: s.lod_bias,
        compare_enabled,
        compare_func: if compare_enabled { u32::from(s.compare_op & 7) } else { 0 },
        max_anisotropy: s.max_anisotropy,
        border: s.border,
        cube,
        seamless: s.seamless,
    })
}

/// The texture-size words of a binding (QUNIFORM_TEXTURE_WIDTH ..
/// _ARRAY_SIZE): the base level's width and height, 1 and 1 for depth and
/// array size (a 2D texture or a cube).
pub fn size_words(t: &GlTexBind71) -> (u32, u32, u32, u32) {
    let shift = t.base_level.min(31);
    ((t.width >> shift).max(1), (t.height >> shift).max(1), 1, 1)
}

/// Everything the 7.1 texture modules take for one sampled unit, and what
/// they made of it.
#[derive(Clone, Debug)]
pub struct Glued71 {
    pub unit: u32,
    /// texture71::compose's (and texfill71::fill's) texture input
    pub tex: GlTex71,
    /// sampler71::sampler71's (and texfill71::fill's) sampler input
    pub sampler: Input,
    /// texture71::compose(&tex)
    pub composed: Tex71,
    /// sampler71::sampler71(&sampler)
    pub sampler_values: Sampler71,
    /// (width, height, depth, array size) for the size words
    pub size: (u32, u32, u32, u32),
}

impl Glued71 {
    /// texuni71's unit, the texture state at `tex_state` and the sampler
    /// state at `sampler` (texuni71 checks the alignments).
    pub fn unit(&self, tex_state: u32, sampler: u32) -> Unit {
        let (width, height, depth, array_size) = self.size;
        Unit { unit: self.unit, tex_state, sampler, width, height, depth, array_size }
    }

    /// The program's TMU Config Parameter 1 `data` agrees with the format's
    /// return size (bit 0, "Output Type 32-bit").
    pub fn check_return_size(&self, p1_data: u32) -> Result<(), Refusal> {
        let program_32 = p1_data & 1 != 0;
        let format_32 = self.composed.return_32;
        if program_32 != format_32 {
            return Err(Refusal::ReturnSize { format_32, program_32 });
        }
        Ok(())
    }
}

/// The 7.1 inputs for a GL texture and its sampler, or a named refusal.
pub fn glue(t: &GlTexBind71, s: &GlSampler71, ver: u32, iprev: u32) -> Result<Glued71, Refusal> {
    let tex = gl_tex(t, ver, iprev)?;
    let sampler = sampler_input(s, tex.target, t.texfmt)?;
    let composed = texture71::compose(&tex).map_err(Refusal::Texture)?;
    let sampler_values = sampler71::sampler71(&sampler).map_err(Refusal::Sampler)?;
    Ok(Glued71 { unit: t.unit, tex, sampler, composed, sampler_values, size: size_words(t) })
}
