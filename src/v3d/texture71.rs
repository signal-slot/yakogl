// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The V3D 7.1 GL texture to texture-record composition (P10 of
//! docs/notes/pre-study-rpi5.md; docs/notes/v3d71-gl-table.md section 3).
//!
//! From a GL-level texture description ([`GlTex71`]: a 2D texture or a cube
//! map, its internal format and type, size, allocated levels, base and max
//! level, swizzle and sRGB decode, the buffer's GPU address) this gives every
//! value `driver/src/v3d/v3d_tex71.h`'s `struct v3d_tex71_state` takes, as a
//! plain struct ([`Tex71`]), or a named refusal ([`Refusal`]):
//!
//! * the record fields [`super::texstate71::texture_state`] owns (size,
//!   levels, transfer function, R/B swap, reverse, array stride);
//! * the ones it leaves to the caller: the TMU type ([`super::format71`]),
//!   the swizzle (the format's composed with the GL swizzle), the level 0
//!   UIF fields and `extended` from the layout, and the cube face stride.
//!
//! Mesa 26.2.1 is the reference; gallium's files are not vendored, their
//! line numbers are those `v3d_tex.h`, `v3d_tex71.h` and
//! docs/notes/v3d71-cube-seamless.md cite (the tag `mesa-26.2.1` of
//! `third_party/mesa-26.2.1/UPSTREAM`):
//!
//! * the layout is `v3d_setup_slices` (v3d_resource.c:582-768) for a
//!   single-sampled, never-shared, uncompressed resource, as
//!   `driver/src/v3d/v3d_tex.c`'s `layout()` transcribes it: levels from
//!   the smallest up, LINEARTILE / UBLINEAR 1 or 2 columns / UIF chosen by
//!   the level's size, UIF heights padded by `v3d_get_ub_pad`, level 1's
//!   slice page-aligned when it could be UIF XOR, the whole tree shifted so
//!   level 0 starts on 4 KiB; the cube face stride is `cube_map_stride`,
//!   level 0's end 64-byte aligned (:757-764);
//! * the record is `v3d_setup_texture_shader_state` (v3dx_state.c:903-988):
//!   width and height of level 0, depth = the layer count (6 for a cube),
//!   base = the buffer + level 0 of layer 0 (`v3d_layer_offset`, :956-960),
//!   `array_stride_64 = cube_map_stride / 64` for a 2D texture too (:961),
//!   level 0's UIF bits and UB_PAD, `extended` with a strictly-UIF level 0
//!   (:976-987, as `v3d_tex71_state_from_layout`);
//! * per revision, `v3dx_state.c:1023-1029`: at 7.1 the sRGB of the view's
//!   format is the 3-bit transfer function (`TRANSFER_FUNC_SRGB`), not 4.2's
//!   sRGB bit; the GL path never sets R/B swap or reverse (the format's
//!   swizzle carries the swap, `format71`), and the swap bit's layout needs
//!   7.1.5 or later (texstate71 refuses older 7.1 revisions); the swizzle is
//!   `util_format_compose_swizzles(format swizzle, view swizzle)` then
//!   `v3d_translate_pipe_swizzle` (v3dx_state.c:1031-1034), whatever the
//!   return size.
//!
//! The sampler half (sampler71) is not merged yet: this module has no
//! sampler record; the table wiring (mip71) adds it.
//!
//! Pure, no `crate::` import: its siblings are reached through `super::`, so
//! a test includes it with `#[path]` next to `texstate71.rs` and
//! `format71.rs` (tests/v3d71_gl_tex.rs). Not in `src/v3d/mod.rs` yet.

#![allow(dead_code)]

use std::fmt;

use super::format71::{self, Refusal71, Row71, Transfer};
use super::texstate71::{self, TexDesc};

/// A GL texture target this module composes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    Tex2d,
    Cube,
}

/// GL's swizzle sources (GL_TEXTURE_SWIZZLE_*).
pub const GL_ZERO: u32 = 0;
pub const GL_ONE: u32 = 1;
pub const GL_RED: u32 = 0x1903;
pub const GL_GREEN: u32 = 0x1904;
pub const GL_BLUE: u32 = 0x1905;
pub const GL_ALPHA: u32 = 0x1906;
/// GL's identity swizzle.
pub const GL_SWIZZLE_IDENTITY: [u32; 4] = [GL_RED, GL_GREEN, GL_BLUE, GL_ALPHA];
/// GL_TEXTURE_MAX_LEVEL's default.
pub const GL_MAX_LEVEL_DEFAULT: u32 = 1000;

/// One GL texture as the GL core holds it at draw time.
#[derive(Clone, Copy, Debug)]
pub struct GlTex71 {
    pub target: Target,
    /// glTexImage2D's internalformat (sized, or unsized with `ty`)
    pub internal_format: u32,
    /// glTexImage2D's type (used for an unsized internal format)
    pub ty: u32,
    /// level 0's size (a cube map: one face's)
    pub width: u32,
    pub height: u32,
    /// levels the texture's storage has, from level 0
    pub levels: u32,
    /// GL_TEXTURE_BASE_LEVEL, GL_TEXTURE_MAX_LEVEL (default 0 and 1000)
    pub base_level: u32,
    pub max_level: u32,
    /// GL_TEXTURE_SWIZZLE_R/G/B/A: GL_RED .. GL_ALPHA, GL_ZERO, GL_ONE
    pub swizzle: [u32; 4],
    /// GL_TEXTURE_SRGB_DECODE_EXT is DECODE (the default); false is
    /// SKIP_DECODE, which views an sRGB format as its linear one
    pub srgb_decode: bool,
    /// GPU address of the texture's buffer (the layout's offset 0)
    pub bo_address: u32,
    /// hub version (71) and IPREV (10 for the Pi 5's 7.1.10)
    pub ver: u32,
    pub iprev: u32,
}

/// Mesa's enum v3d_tiling_mode values (v3d_tex.h `enum v3d_tex_tiling`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tiling {
    LinearTile = 1,
    UbLinear1Column = 2,
    UbLinear2Column = 3,
    UifNoXor = 4,
    UifXor = 5,
}

/// One level of one layer, as `struct v3d_tex_slice`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Slice {
    /// from the start of the buffer (layer 0)
    pub offset: u32,
    /// bytes of one row of the padded level
    pub stride: u32,
    /// rows of the padded level
    pub padded_height: u32,
    /// stride * padded_height
    pub size: u32,
    /// UIF block rows of padding (UIF only)
    pub ub_pad: u32,
    pub tiling: Tiling,
}

/// The layout of one layer's mip tree (`struct v3d_tex_layout`) and the
/// layers after it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Layout {
    pub cpp: u32,
    /// slices[i] is level i
    pub slices: Vec<Slice>,
    /// bytes from one layer (cube face) to the next: Mesa's cube_map_stride
    pub layer_stride: u32,
    pub layers: u32,
    /// the buffer's bytes, every layer included (Mesa's rsc->size)
    pub size: u32,
}

impl Layout {
    /// Offset of level `level` of layer `layer` (`v3d_layer_offset`).
    pub fn offset(&self, level: u32, layer: u32) -> Option<u32> {
        let s = self.slices.get(level as usize)?;
        if layer >= self.layers {
            return None;
        }
        u32::try_from(u64::from(s.offset) + u64::from(self.layer_stride) * u64::from(layer)).ok()
    }
}

/// Every value `struct v3d_tex71_state` takes (its field names; `tex_type`
/// is the C `type`), plus what the table needs beside the record.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tex71 {
    pub base: u32,
    pub width: u32,
    pub height: u32,
    pub depth: u32,
    pub base_level: u32,
    pub max_level: u32,
    /// V3D_TEXTURE_DATA_FORMAT_* (the record's "Texture type")
    pub tex_type: u32,
    /// R, G, B, A: V3D_SWIZZLE_* (format71::SWIZZLE_*)
    pub swizzle: [u8; 4],
    pub transfer_func: u32,
    pub r_b_swap: bool,
    pub reverse: bool,
    pub level_0_is_strictly_uif: bool,
    pub level_0_xor_enable: bool,
    pub extended: bool,
    pub level_0_ub_pad: u32,
    pub array_stride_64: u32,
    // ---- beside the record ----
    /// the format71 row (pipe format, Mesa line, return size)
    pub row: Row71,
    /// the look-up wants the TMU's 32-bit output (TMU Config Parameter 1)
    pub return_32: bool,
    /// bytes between cube faces (0 for a 2D texture)
    pub cube_face_stride: u32,
    pub layout: Layout,
}

/// Why a GL texture has no 7.1 record here. Every variant is a decision.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// format71 has no 7.1 answer for the format
    Format(Refusal71),
    /// a compressed format: Mesa lays it out in blocks, which this module
    /// does not (yet) do
    Compressed { pipe: &'static str },
    /// a depth/stencil format whose texture half is a separate buffer in
    /// Mesa (Z32_FLOAT_S8X24_UINT, X32_S8X24_UINT, S8X24_UINT)
    SeparateStencil { pipe: &'static str },
    /// a format whose texel size this module does not know
    TexelSize { pipe: &'static str },
    /// a GL_TEXTURE_SWIZZLE_* value that is no swizzle source
    SwizzleSource { value: u32 },
    /// zero width, height or levels
    ZeroSize,
    /// more levels than the full chain of the size has
    LevelsBeyondChain { levels: u32, chain: u32 },
    /// GL_TEXTURE_BASE_LEVEL past the last level the storage has
    BaseLevelMissing { base_level: u32, levels: u32 },
    /// GL_TEXTURE_BASE_LEVEL above GL_TEXTURE_MAX_LEVEL (incomplete)
    BaseAboveMax { base_level: u32, max_level: u32 },
    /// a width or height the layout does not take (16384 or more)
    LayoutSize,
    /// the layout passes 32 bits of bytes
    LayoutTooLarge,
    /// the buffer address plus the layout passes 32 bits
    AddressOverflow,
    /// texstate71 refused the record
    State(texstate71::Refusal),
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Refusal::Format(r) => write!(f, "texture 7.1: no 7.1 TMU format ({r:?})"),
            Refusal::Compressed { pipe } => {
                write!(f, "texture 7.1: compressed format {pipe} is not laid out by texture71")
            }
            Refusal::SeparateStencil { pipe } => {
                write!(f, "texture 7.1: {pipe} keeps its stencil in a separate buffer")
            }
            Refusal::TexelSize { pipe } => write!(f, "texture 7.1: no texel size for {pipe}"),
            Refusal::SwizzleSource { value } => write!(f, "texture 7.1: swizzle source {value:#x} is not one"),
            Refusal::ZeroSize => write!(f, "texture 7.1: zero width, height or level count"),
            Refusal::LevelsBeyondChain { levels, chain } => {
                write!(f, "texture 7.1: {levels} levels where the chain has {chain}")
            }
            Refusal::BaseLevelMissing { base_level, levels } => {
                write!(f, "texture 7.1: base level {base_level} past the {levels} levels of the storage")
            }
            Refusal::BaseAboveMax { base_level, max_level } => {
                write!(f, "texture 7.1: base level {base_level} above max level {max_level}")
            }
            Refusal::LayoutSize => write!(f, "texture 7.1: width or height past the layout's 14 bits"),
            Refusal::LayoutTooLarge => write!(f, "texture 7.1: the layout passes 32 bits of bytes"),
            Refusal::AddressOverflow => write!(f, "texture 7.1: buffer address plus layout passes 32 bits"),
            Refusal::State(r) => write!(f, "{}", r.message()),
        }
    }
}

// ---- the layout: v3d_setup_slices ---------------------------------------

/// v3d_resource.c / v3d_tex.c constants
const UIFCFG_PAGE_SIZE: u32 = 4096;
const UIFCFG_BANKS: u32 = 8;
const PAGE_CACHE_SIZE: u32 = UIFCFG_PAGE_SIZE * UIFCFG_BANKS;
const UBLOCK_SIZE: u32 = 64;
const UIFBLOCK_SIZE: u32 = 4 * UBLOCK_SIZE;
const UIFBLOCK_ROW_SIZE: u32 = 4 * UIFBLOCK_SIZE;
const PAGE_UB_ROWS: u32 = UIFCFG_PAGE_SIZE / UIFBLOCK_ROW_SIZE;
const PAGE_UB_ROWS_TIMES_1_5: u32 = (PAGE_UB_ROWS * 3) >> 1;
const PAGE_CACHE_UB_ROWS: u32 = PAGE_CACHE_SIZE / UIFBLOCK_ROW_SIZE;
const PAGE_CACHE_MINUS_1_5_UB_ROWS: u32 = PAGE_CACHE_UB_ROWS - PAGE_UB_ROWS_TIMES_1_5;

fn align(v: u32, a: u32) -> u32 {
    v.div_ceil(a) * a
}

fn minify(v: u32, level: u32) -> u32 {
    (v >> level).max(1)
}

/// v3d_tiling.c v3d_utile_width / v3d_utile_height
fn utile_w(cpp: u32) -> u32 {
    match cpp {
        1 | 2 => 8,
        4 | 8 => 4,
        _ => 2,
    }
}

fn utile_h(cpp: u32) -> u32 {
    match cpp {
        1 => 8,
        2 | 4 => 4,
        _ => 2,
    }
}

/// v3d_resource.c v3d_get_ub_pad
fn ub_pad(cpp: u32, height: u32) -> u32 {
    let height_ub = height / (utile_h(cpp) * 2);
    let in_pc = height_ub % PAGE_CACHE_UB_ROWS;
    if in_pc == 0 {
        return 0;
    }
    if in_pc < PAGE_UB_ROWS_TIMES_1_5 {
        if height_ub < PAGE_CACHE_UB_ROWS {
            return 0;
        }
        return PAGE_UB_ROWS_TIMES_1_5 - in_pc;
    }
    if in_pc > PAGE_CACHE_MINUS_1_5_UB_ROWS {
        return PAGE_CACHE_UB_ROWS - in_pc;
    }
    0
}

/// v3d_get_dimension_mpad with a block of 1, level 1: level 1's size
/// rounded up to a power of two
fn pot_level1(dim: u32) -> u32 {
    minify(dim, 1).next_power_of_two()
}

/// The mip tree of one layer, as v3d_setup_slices lays out a 2D texture or
/// a cube face (no MSAA, no shared stride, `uif_top` false), and the
/// layers after it (`layers` = 1 or 6).
pub fn layout(width: u32, height: u32, levels: u32, cpp: u32, layers: u32) -> Result<Layout, Refusal> {
    if width == 0 || height == 0 || levels == 0 || layers == 0 {
        return Err(Refusal::ZeroSize);
    }
    if width >= 1 << 14 || height >= 1 << 14 {
        return Err(Refusal::LayoutSize);
    }
    let (uw, uh) = (utile_w(cpp), utile_h(cpp));
    let (bw, bh) = (uw * 2, uh * 2);
    let pot_w = 2 * pot_level1(width);
    let pot_h = 2 * pot_level1(height);
    let mut slices =
        vec![
            Slice { offset: 0, stride: 0, padded_height: 0, size: 0, ub_pad: 0, tiling: Tiling::LinearTile };
            levels as usize
        ];
    let mut offset: u64 = 0;
    for i in (0..levels).rev() {
        let (mut lw, mut lh) =
            if i < 2 { (minify(width, i), minify(height, i)) } else { (minify(pot_w, i), minify(pot_h, i)) };
        let mut pad = 0;
        let tiling = if lw <= uw || lh <= uh {
            lw = align(lw, uw);
            lh = align(lh, uh);
            Tiling::LinearTile
        } else if lw <= bw {
            lw = align(lw, bw);
            lh = align(lh, bh);
            Tiling::UbLinear1Column
        } else if lw <= 2 * bw {
            lw = align(lw, 2 * bw);
            lh = align(lh, bh);
            Tiling::UbLinear2Column
        } else {
            lw = align(lw, 4 * bw);
            lh = align(lh, bh);
            pad = ub_pad(cpp, lh);
            lh += pad * bh;
            if (lh / bh).is_multiple_of(PAGE_CACHE_SIZE / UIFBLOCK_ROW_SIZE) {
                Tiling::UifXor
            } else {
                Tiling::UifNoXor
            }
        };
        let off = u32::try_from(offset).map_err(|_| Refusal::LayoutTooLarge)?;
        let stride = lw * cpp;
        let size = u64::from(lh) * u64::from(stride);
        let size32 = u32::try_from(size).map_err(|_| Refusal::LayoutTooLarge)?;
        slices[i as usize] = Slice { offset: off, stride, padded_height: lh, size: size32, ub_pad: pad, tiling };
        let mut total = size;
        if i == 1 && lw > 4 * bw && lh > PAGE_CACHE_MINUS_1_5_UB_ROWS * bh {
            total = total.div_ceil(u64::from(UIFCFG_PAGE_SIZE)) * u64::from(UIFCFG_PAGE_SIZE);
        }
        offset += total;
    }
    // level 0 on 4 KiB (v3d_setup_slices' page_align_offset)
    let s0 = slices[0].offset;
    let shift = align(s0, 4096) - s0;
    offset += u64::from(shift);
    for s in slices.iter_mut() {
        s.offset += shift;
    }
    let tree = u32::try_from(offset).map_err(|_| Refusal::LayoutTooLarge)?;
    let end0 = u64::from(slices[0].offset) + u64::from(slices[0].size);
    let layer_stride = u32::try_from(end0.div_ceil(64) * 64).map_err(|_| Refusal::LayoutTooLarge)?;
    let size = u64::from(tree) + u64::from(layer_stride) * u64::from(layers - 1);
    let size = u32::try_from(size).map_err(|_| Refusal::LayoutTooLarge)?;
    Ok(Layout { cpp, slices, layer_stride, layers, size })
}

// ---- the format ----------------------------------------------------------

/// Bytes per texel of an uncompressed Mesa row (util_format_get_blocksize),
/// or the refusal for a format this module does not lay out.
pub fn cpp_of(pipe: &'static str) -> Result<u32, Refusal> {
    if pipe.starts_with("ETC2_") || pipe.starts_with("DXT") || pipe.starts_with("ASTC_") {
        return Err(Refusal::Compressed { pipe });
    }
    if matches!(pipe, "Z32_FLOAT_S8X24_UINT" | "X32_S8X24_UINT" | "S8X24_UINT") {
        return Err(Refusal::SeparateStencil { pipe });
    }
    // the bits of each channel, from the name: R8G8B8A8 -> 32
    let name = pipe.split('_').next().unwrap_or("");
    let mut bits = 0u32;
    let mut num = String::new();
    for c in name.chars().chain(std::iter::once('.')) {
        if c.is_ascii_digit() {
            num.push(c);
        } else {
            if let Ok(n) = num.parse::<u32>() {
                bits += n;
            }
            num.clear();
        }
    }
    // R9G9B9E5 = 32, X8Z24 = 32, R11G11B10 = 32, S8 Z24 = 32 (S8_UINT_Z24):
    // the depth-stencil rows name the stencil after an underscore
    if pipe == "S8_UINT_Z24_UNORM" {
        bits = 32;
    }
    match bits {
        8 | 16 | 32 | 64 | 128 => Ok(bits / 8),
        _ => Err(Refusal::TexelSize { pipe }),
    }
}

/// The record's swizzle: format71's (the format's own) composed with GL's.
pub fn compose_swizzle(fmt: [u8; 4], gl: [u32; 4]) -> Result<[u8; 4], Refusal> {
    let mut out = [0u8; 4];
    for (o, g) in out.iter_mut().zip(gl) {
        *o = match g {
            GL_RED => fmt[0],
            GL_GREEN => fmt[1],
            GL_BLUE => fmt[2],
            GL_ALPHA => fmt[3],
            GL_ZERO => format71::SWIZZLE_ZERO,
            GL_ONE => format71::SWIZZLE_ONE,
            value => return Err(Refusal::SwizzleSource { value }),
        };
    }
    Ok(out)
}

// ---- the composition -----------------------------------------------------

/// Levels of the full chain of a width x height image.
pub fn full_chain(width: u32, height: u32) -> u32 {
    32 - width.max(height).max(1).leading_zeros()
}

/// The 7.1 texture record values for one GL texture.
pub fn compose(t: &GlTex71) -> Result<Tex71, Refusal> {
    texstate71::check_revision(t.ver, t.iprev).map_err(Refusal::State)?;
    let row = format71::tmu_format71(t.internal_format, t.ty).map_err(Refusal::Format)?;
    let cpp = cpp_of(row.pipe)?;
    let swizzle = compose_swizzle(row.swizzle, t.swizzle)?;
    if t.width == 0 || t.height == 0 || t.levels == 0 {
        return Err(Refusal::ZeroSize);
    }
    let chain = full_chain(t.width, t.height);
    if t.levels > chain {
        return Err(Refusal::LevelsBeyondChain { levels: t.levels, chain });
    }
    if t.base_level >= t.levels {
        return Err(Refusal::BaseLevelMissing { base_level: t.base_level, levels: t.levels });
    }
    if t.base_level > t.max_level {
        return Err(Refusal::BaseAboveMax { base_level: t.base_level, max_level: t.max_level });
    }
    // the view's last level: GL_TEXTURE_MAX_LEVEL clamped to the storage
    let last = t.max_level.min(t.levels - 1);
    let (cube, layers) = match t.target {
        Target::Tex2d => (false, 1),
        Target::Cube => (true, 6),
    };
    // texstate71's size limit, before the layout sees the size
    for dim in [t.width, t.height] {
        if dim > texstate71::MAX_DIMENSION {
            return Err(Refusal::State(texstate71::Refusal::DimensionTooLarge { dim }));
        }
    }
    let l = layout(t.width, t.height, t.levels, cpp, layers)?;
    let s0 = l.slices[0];
    let base = t.bo_address.checked_add(s0.offset).ok_or(Refusal::AddressOverflow)?;
    t.bo_address.checked_add(l.size).ok_or(Refusal::AddressOverflow)?;
    let srgb = row.transfer == Transfer::Srgb && t.srgb_decode;
    let st = texstate71::texture_state(&TexDesc {
        width: t.width,
        height: t.height,
        depth: layers,
        levels: last - t.base_level + 1,
        base_level: t.base_level,
        cube,
        srgb,
        transfer_func: None,
        bytes_per_texel: cpp,
        layer_stride: u64::from(l.layer_stride),
        base,
        ver: t.ver,
        iprev: t.iprev,
    })
    .map_err(Refusal::State)?;
    let uif = matches!(s0.tiling, Tiling::UifNoXor | Tiling::UifXor);
    Ok(Tex71 {
        base: st.base,
        width: st.width,
        height: st.height,
        depth: st.depth,
        base_level: st.base_level,
        max_level: st.max_level,
        tex_type: row.tex_type,
        swizzle,
        transfer_func: st.transfer_func,
        r_b_swap: st.r_b_swap || row.r_b_swap,
        reverse: st.reverse || row.reverse,
        level_0_is_strictly_uif: uif,
        level_0_xor_enable: s0.tiling == Tiling::UifXor,
        extended: uif,
        level_0_ub_pad: if uif { s0.ub_pad } else { 0 },
        array_stride_64: st.array_stride_64,
        row,
        return_32: row.return_32,
        cube_face_stride: if cube { l.layer_stride } else { 0 },
        layout: l,
    })
}
