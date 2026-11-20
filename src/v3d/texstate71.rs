// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! V3D 7.1 texture record field values (P10 of docs/notes/pre-study-rpi5.md,
//! docs/notes/v3d71-gl-table.md section 3).
//!
//! Pure functions over plain integers, no `crate::` import: a test includes
//! this file with `#[path]`. The output struct's field names match
//! `driver/src/v3d/v3d_tex71.h`'s `struct v3d_tex71_state`, which the C
//! packer `v3d_tex71_pack_state` takes. Bit positions are those of
//! `third_party/mesa-26.2.1/src/broadcom/cle/v3d_packet.xml`: 7.1 Texture
//! Shader State `:1633-1707` and 7.1 TMU Config Parameter 2 `:1575-1590`.
//! Gallium's `v3dx_state.c` is not vendored; its citations are those of
//! `v3d_tex71.h` (`v3dx_state.c:961-987`, `:1027-1029`).

#![allow(dead_code)]

/// Hub version this module answers (`V3D 7.1`).
pub const HUB_VER: u32 = 71;
/// Oldest IPREV with the R/B swap bit and the stride at bit 33
/// (`v3d_packet.xml:1686-1690`); the Pi 5's D0 is 7.1.10.
pub const MIN_IPREV: u32 = 5;
/// Largest texture width/height (`V3D_MAX_IMAGE_DIMENSION`, as limits71.rs).
pub const MAX_DIMENSION: u32 = 4096;
/// Largest array layer count / 3D depth (`GL_MAX_ARRAY_TEXTURE_LAYERS`).
pub const MAX_LAYERS: u32 = 2048;
/// Highest level number (the level fields are 4 bits).
pub const MAX_LEVEL: u32 = 15;
/// Array stride field: 24 bits of 64-byte units.
pub const ARRAY_STRIDE_BITS: u32 = 24;

pub const TRANSFER_NONE: u32 = 0;
pub const TRANSFER_SRGB: u32 = 1;

/// Every reason this module refuses a description. `message()` names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// Not a V3D 7.1 hub at all.
    NotV3d71 {
        ver: u32,
    },
    /// 7.1.2 has no R/B swap bit and a stride at bit 32: one layout only.
    RevisionBelow715 {
        iprev: u32,
    },
    ZeroSize,
    DimensionTooLarge {
        dim: u32,
    },
    LayersTooMany {
        layers: u32,
    },
    /// A cube map needs width == height.
    CubeNotSquare,
    /// A cube map's layer count is not a multiple of 6.
    CubeFaces {
        depth: u32,
    },
    LevelOutOfRange {
        base_level: u32,
        levels: u32,
    },
    ZeroBytesPerTexel,
    /// Layer stride is not a multiple of 64 bytes.
    StrideNotAligned {
        stride: u64,
    },
    /// Layer stride / 64 does not fit 24 bits.
    StrideTooLarge {
        stride: u64,
    },
    /// A multi-layer texture without a stride.
    StrideMissing,
    /// Base address not 64-byte aligned (its low 6 bits carry flags).
    BaseNotAligned {
        base: u32,
    },
    /// A transfer function other than none or sRGB (PQ, HLG, ... are not
    /// GL formats).
    TransferFunc {
        value: u32,
    },
    /// A TMU config field that does not fit.
    TmuField {
        name: &'static str,
        value: i32,
    },
}

impl Refusal {
    pub fn message(&self) -> &'static str {
        match self {
            Refusal::NotV3d71 { .. } => "texture state 7.1: hub version is not 7.1",
            Refusal::RevisionBelow715 { .. } => {
                "texture state 7.1: revision below 7.1.5 has no R/B swap bit and another array stride layout"
            }
            Refusal::ZeroSize => "texture state 7.1: zero width, height, depth or level count",
            Refusal::DimensionTooLarge { .. } => "texture state 7.1: width or height above 4096",
            Refusal::LayersTooMany { .. } => "texture state 7.1: more than 2048 layers or depth",
            Refusal::CubeNotSquare => "texture state 7.1: cube map faces are not square",
            Refusal::CubeFaces { .. } => "texture state 7.1: cube map layer count is not a multiple of 6",
            Refusal::LevelOutOfRange { .. } => "texture state 7.1: base level or level count out of range",
            Refusal::ZeroBytesPerTexel => "texture state 7.1: zero bytes per texel",
            Refusal::StrideNotAligned { .. } => "texture state 7.1: array stride is not a multiple of 64 bytes",
            Refusal::StrideTooLarge { .. } => "texture state 7.1: array stride does not fit 24 bits of 64-byte units",
            Refusal::StrideMissing => "texture state 7.1: a layered texture needs an array stride",
            Refusal::BaseNotAligned { .. } => "texture state 7.1: base address is not 64-byte aligned",
            Refusal::TransferFunc { .. } => "texture state 7.1: transfer function other than none or sRGB",
            Refusal::TmuField { .. } => "texture state 7.1: TMU config field out of range",
        }
    }
}

/// A plain description of one sampled texture view.
#[derive(Clone, Copy, Debug)]
pub struct TexDesc {
    pub width: u32,
    pub height: u32,
    /// Layer count of an array, depth of a 3D texture, 1 for a 2D texture;
    /// for a cube map 6 (or 6 * n for a cube array).
    pub depth: u32,
    /// Number of levels of the view, from `base_level`.
    pub levels: u32,
    pub base_level: u32,
    pub cube: bool,
    pub srgb: bool,
    /// `Some(v)` overrides the transfer function derived from `srgb`
    /// (any value but 0 and 1 is refused).
    pub transfer_func: Option<u32>,
    pub bytes_per_texel: u32,
    /// Byte distance between array layers / cube faces (the mip tree's
    /// stride; for a 3D texture, level 0's slice size). May be 0 for depth 1.
    pub layer_stride: u64,
    /// GPU address of level 0 of the first layer.
    pub base: u32,
    /// Hub version (71) and IPREV (10 for the Pi 5's 7.1.10).
    pub ver: u32,
    pub iprev: u32,
}

/// The field values `v3d_tex71_pack_state` takes (names as v3d_tex71.h).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TexState {
    pub base: u32,
    pub width: u32,
    pub height: u32,
    pub depth: u32,
    pub base_level: u32,
    pub max_level: u32,
    pub transfer_func: u32,
    /// Always false: gallium composes the format's swizzle instead.
    pub r_b_swap: bool,
    /// Always false: gallium never sets it.
    pub reverse: bool,
    pub array_stride_64: u32,
}

pub fn check_revision(ver: u32, iprev: u32) -> Result<(), Refusal> {
    if ver != HUB_VER {
        return Err(Refusal::NotV3d71 { ver });
    }
    if iprev < MIN_IPREV {
        return Err(Refusal::RevisionBelow715 { iprev });
    }
    Ok(())
}

pub fn texture_state(d: &TexDesc) -> Result<TexState, Refusal> {
    check_revision(d.ver, d.iprev)?;
    if d.width == 0 || d.height == 0 || d.depth == 0 || d.levels == 0 {
        return Err(Refusal::ZeroSize);
    }
    for dim in [d.width, d.height] {
        if dim > MAX_DIMENSION {
            return Err(Refusal::DimensionTooLarge { dim });
        }
    }
    if d.depth > MAX_LAYERS {
        return Err(Refusal::LayersTooMany { layers: d.depth });
    }
    if d.cube {
        if d.width != d.height {
            return Err(Refusal::CubeNotSquare);
        }
        if !d.depth.is_multiple_of(6) {
            return Err(Refusal::CubeFaces { depth: d.depth });
        }
    }
    if d.base_level.saturating_add(d.levels - 1) > MAX_LEVEL {
        return Err(Refusal::LevelOutOfRange { base_level: d.base_level, levels: d.levels });
    }
    if d.bytes_per_texel == 0 {
        return Err(Refusal::ZeroBytesPerTexel);
    }
    if d.base & 63 != 0 {
        return Err(Refusal::BaseNotAligned { base: d.base });
    }
    let transfer_func = match d.transfer_func {
        Some(v) => v,
        None if d.srgb => TRANSFER_SRGB,
        None => TRANSFER_NONE,
    };
    if transfer_func != TRANSFER_NONE && transfer_func != TRANSFER_SRGB {
        return Err(Refusal::TransferFunc { value: transfer_func });
    }
    if d.depth > 1 && d.layer_stride == 0 {
        return Err(Refusal::StrideMissing);
    }
    if !d.layer_stride.is_multiple_of(64) {
        return Err(Refusal::StrideNotAligned { stride: d.layer_stride });
    }
    let units = d.layer_stride / 64;
    if units >= 1u64 << ARRAY_STRIDE_BITS {
        return Err(Refusal::StrideTooLarge { stride: d.layer_stride });
    }
    Ok(TexState {
        base: d.base,
        width: d.width,
        height: d.height,
        depth: d.depth,
        base_level: d.base_level,
        max_level: d.base_level + d.levels - 1,
        transfer_func,
        r_b_swap: false,
        reverse: false,
        array_stride_64: units as u32,
    })
}

/// The 7.1 Texture Shader State's 192 bits as six words (word i = record
/// bits 32*i..32*i+31), for the fields this module owns. The texture type,
/// swizzle, UIF, extended and chroma fields are the caller's.
pub fn pack_words(s: &TexState) -> [u32; 6] {
    let mut r: u128 = 0;
    r |= u128::from(s.base); // bits 0-31 (low 6 bits are flag bits, zero here)
    r |= u128::from(s.reverse) << 5;
    r |= u128::from(s.transfer_func) << 2;
    r |= u128::from(s.r_b_swap) << 32;
    r |= u128::from(s.array_stride_64) << 33;
    r |= u128::from(s.width) << 58;
    r |= u128::from(s.height) << 72;
    r |= u128::from(s.depth) << 86;
    r |= u128::from(s.max_level) << 120;
    r |= u128::from(s.base_level) << 124;
    [r as u32, (r >> 32) as u32, (r >> 64) as u32, (r >> 96) as u32, 0, 0]
}

/// TMU Config Parameter 2 at 7.1 (`v3d_packet.xml:1575-1590`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TmuConfig2 {
    pub write_conversion: bool,
    pub dim_query: bool,
    pub lod_query: bool,
    /// TMU Op, 4 bits.
    pub op: u32,
    /// Signed 4-bit texel offsets.
    pub offset_r: i32,
    pub offset_t: i32,
    pub offset_s: i32,
    pub gather_mode: bool,
    /// 2 bits.
    pub gather_component: u32,
    pub coefficient_mode: bool,
    /// 2 bits.
    pub sample_number: u32,
    pub disable_autolod: bool,
    pub offset_format_8: bool,
}

fn off4(name: &'static str, v: i32) -> Result<u32, Refusal> {
    if !(-8..=7).contains(&v) {
        return Err(Refusal::TmuField { name, value: v });
    }
    Ok((v as u32) & 0xf)
}

/// Builds the TMU Config Parameter 2 word. Pad bits 27-31 stay zero.
pub fn tmu_config2_word(c: &TmuConfig2) -> Result<u32, Refusal> {
    if c.op > 0xf {
        return Err(Refusal::TmuField { name: "op", value: c.op as i32 });
    }
    if c.gather_component > 3 {
        return Err(Refusal::TmuField { name: "gather_component", value: c.gather_component as i32 });
    }
    if c.sample_number > 3 {
        return Err(Refusal::TmuField { name: "sample_number", value: c.sample_number as i32 });
    }
    Ok(u32::from(c.write_conversion) << 26
        | u32::from(c.dim_query) << 25
        | u32::from(c.lod_query) << 24
        | c.op << 20
        | off4("offset_r", c.offset_r)? << 16
        | off4("offset_t", c.offset_t)? << 12
        | off4("offset_s", c.offset_s)? << 8
        | u32::from(c.gather_mode) << 7
        | c.gather_component << 5
        | u32::from(c.coefficient_mode) << 4
        | c.sample_number << 2
        | u32::from(c.disable_autolod) << 1
        | u32::from(c.offset_format_8))
}
