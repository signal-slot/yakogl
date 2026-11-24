// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The ES 3.0 level and LOD parameters of a sampled texture in the PowerVR's
//! texture state words: GL_TEXTURE_BASE_LEVEL, GL_TEXTURE_MAX_LEVEL,
//! GL_TEXTURE_MIN_LOD and GL_TEXTURE_MAX_LOD (ES 3.0.6 3.8.10, 3.8.13).
//!
//! The fields are Mesa's (texstate.xml, `third_party/mesa-26.2.1/src/
//! imagination/csbgen/rogue/texstate.xml`), filled as Mesa 26.2.1's
//! pvr_arch_tex_state.c, pvr_arch_sampler.c and pvr_arch_image.c fill them
//! for a Vulkan image view of base level `b` and a sampler of minLod and
//! maxLod:
//!
//! - TEXSTATE_IMAGE_WORD1 (the twiddled word, :292-313) has `baselevel` at
//!   [60:63]; Mesa packs the view's `base_mip_level` there and the view's
//!   level count, counted from it, in `num_mip_levels` [0:3]
//!   (pvr_arch_tex_state.c, `word1.baselevel = info->base_level`).
//! - TEXSTATE_STRIDE_IMAGE_WORD1 (the linear word, :315-332) has no base
//!   level: [60:63] is `num_mip_levels` there. A linear image's base level is
//!   stated the way pvr_arch_image.c's `pvr_adjust_non_compressed_view`
//!   states a level it cannot name: the address moved to the level
//!   (`info->offset += plane->mip_levels[base_level].offset`), the extent
//!   minified to it, baselevel 0; with the stride `u_minify(physical_width,
//!   base_level)` as its attachment view states it. The level's offset is
//!   pvr_image.c `pvr_image_plane_setup_mip_levels`': the levels follow each
//!   other from 0, level l `cpp * u_minify(physical_width, l)` bytes a row
//!   and `u_minify(physical_height, l)` rows, the physical extent the
//!   power-of-two one of a mipmapped image (`gl::texture::pot_chain_layout`
//!   lays the GL chain out the same way).
//! - TEXSTATE_SAMPLER_WORD0 (:342-361) has `minlod` [13:22] and `maxlod`
//!   [23:32], both CLAMP (u4.6, at most CLAMP_MAX 959): pvr_arch_sampler.c
//!   packs `util_unsigned_fixed(CLAMP(lod, 0, CLAMP_MAX / 64), 6)` of the
//!   sampler's minLod and maxLod (no 0.5 rounding bias: that is quirk 51025,
//!   which the BXS-4-64 does not have, common/device_info/bxs-4-64.h). The
//!   LOD the TPU clamps is relative to the base level, as ES 3.0's lambda is
//!   (3.8.10: the level is `level_base + lambda`).
//!
//! GL_TEXTURE_MAX_LEVEL caps the levels the sampler may walk (ES 3.0
//! 3.8.13: q = min(p, max_level)); the sampler's LOD range stops at the last
//! of them, `q - base` levels past the base.
//!
//! [`TexLod::DEFAULT`] is GL's default state (base 0, max level 1000, LOD
//! -1000..1000): with it every word is the word the backend packed before
//! these fields were encoded (tests/pvr_tex_lod_words.rs).
use crate::backend::{gles_texfmt_bytes, GlesTextureBind};

/// GL_TEXTURE_BASE_LEVEL, GL_TEXTURE_MAX_LEVEL, GL_TEXTURE_MIN_LOD and
/// GL_TEXTURE_MAX_LOD of one sampled texture (the sampler object's LODs
/// when one is bound to the unit), as the PowerVR's descriptor builder
/// takes them (`pvr::backend::texture_descriptor_lod`).
///
/// The levels are GL's numbering of the bind's storage: level 0 is the
/// image at `GlesTextureBind::base_gpu`, `mip_levels` the levels it holds
/// from there.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TexLod {
    /// GL_TEXTURE_BASE_LEVEL.
    pub base_level: u32,
    /// GL_TEXTURE_MAX_LEVEL.
    pub max_level: u32,
    /// GL_TEXTURE_MIN_LOD.
    pub min_lod: f32,
    /// GL_TEXTURE_MAX_LOD.
    pub max_lod: f32,
}

impl TexLod {
    /// GL's initial state (ES 3.0.6 table 6.10): base level 0, max level
    /// 1000, min LOD -1000, max LOD 1000.
    pub const DEFAULT: TexLod = TexLod { base_level: 0, max_level: 1000, min_lod: -1000.0, max_lod: 1000.0 };
}

impl Default for TexLod {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// The base level the words may name: GL_TEXTURE_BASE_LEVEL, but never past
/// the last level the bind holds (a base level past it is GL's incomplete
/// texture, which the GL core binds as its own black texture).
pub fn base_level(t: &GlesTextureBind, lod: &TexLod) -> u32 {
    lod.base_level.min(u32::from(t.mip_levels.max(1)) - 1)
}

/// The levels from the base level on that the sampler walks: one without a
/// mipmap filter, else the bind's levels up to GL_TEXTURE_MAX_LEVEL, less
/// the levels below the base. With the defaults this is the count the
/// backend always walked (`mip_levels` under a mipmap filter, else 1).
pub fn sampled_levels(t: &GlesTextureBind, lod: &TexLod) -> u32 {
    let last = u32::from(t.mip_levels).min(lod.max_level.saturating_add(1));
    let b = base_level(t, lod);
    if t.mip_filter != 0 && last > b + 1 {
        last - b
    } else {
        1
    }
}

/// A LOD in the sampler's u4.6 CLAMP (texstate.xml SAMPLER_WORD0 minlod,
/// maxlod; pvr_arch_sampler.c `util_unsigned_fixed(CLAMP(lod, 0.0,
/// lod_clamp_max), 6)`): 64 per level, at least 0 (a NaN too), at most
/// CLAMP_MAX (959).
pub fn lod_clamp(lod: f32) -> u32 {
    const CLAMP_MAX: u32 = 959;
    if lod > 0.0 {
        ((lod.min(15.0) * 64.0) as u32).min(CLAMP_MAX)
    } else {
        0
    }
}

/// The byte offset of level `level` in a linear mip chain whose level 0 is
/// `stride` texels a row (the physical width) and `height` rows, `bpp` bytes
/// a texel: Mesa's pvr_image_plane_setup_mip_levels over the power-of-two
/// physical extent of a mipmapped image (`u_minify` at every level).
pub fn stride_level_offset(stride: u32, height: u32, bpp: u32, level: u32) -> u64 {
    let ph = height.max(1).next_power_of_two();
    (0..level).map(|k| u64::from((stride >> k).max(1)) * u64::from((ph >> k).max(1)) * u64::from(bpp)).sum()
}

/// A linear (STRIDE) 2D bind restated from its base level, as a Vulkan view
/// whose level 0 is that level (pvr_adjust_non_compressed_view's offset and
/// minified extent; TEXSTATE_STRIDE_IMAGE_WORD1 has no `baselevel`): the
/// address moved to level `base_level(t, lod)`, its width, height and
/// stride, its levels counted from it. `t` itself at base level 0.
///
/// A chain whose base level's power-of-two height or width is not the
/// physical extent halved that many times (a non-power-of-two side just
/// above a power of two: 513 rows, whose level 1 has 256 rows but sits in
/// 512) would be walked by the TPU from the wrong rows past the base: such
/// a bind keeps its base level alone (`mip_levels` 1), the base level's
/// texels right and no level above it read.
pub fn rebase_stride_bind(t: &GlesTextureBind, lod: &TexLod) -> GlesTextureBind {
    let b = base_level(t, lod);
    if b == 0 {
        return *t;
    }
    let bpp = gles_texfmt_bytes(t.format);
    let mut v = *t;
    v.base_gpu = t.base_gpu + stride_level_offset(t.stride_texels, t.height, bpp, b);
    v.width = (t.width >> b).max(1);
    v.height = (t.height >> b).max(1);
    v.stride_texels = (t.stride_texels >> b).max(1);
    v.mip_levels = (u32::from(t.mip_levels) - b) as u8;
    let ph = t.height.max(1).next_power_of_two();
    let pw = t.width.max(1).next_power_of_two();
    if v.height.next_power_of_two() != (ph >> b).max(1) || v.width.next_power_of_two() != (pw >> b).max(1) {
        v.mip_levels = 1;
    }
    v.bytes = v.stride_texels.wrapping_mul(v.height).wrapping_mul(bpp);
    v
}

/// [`TexLod`] for the bind [`rebase_stride_bind`] made: its base level is 0
/// and its max level counted from the old base.
pub fn rebased_lod(t: &GlesTextureBind, lod: &TexLod) -> TexLod {
    let b = base_level(t, lod);
    TexLod { base_level: 0, max_level: lod.max_level.saturating_sub(b), ..*lod }
}

/// TEXSTATE_IMAGE_WORD1.baselevel (texstate.xml:293, bits [60:63] of the
/// 64-bit word: bits 28..31 of its second dword) set in the packed twiddled
/// word `w1` (two dwords). Not for the STRIDE word, where those bits are
/// `num_mip_levels`.
pub fn set_image_word1_baselevel(w1: &mut [u32], baselevel: u32) {
    w1[1] = (w1[1] & 0x0fff_ffff) | ((baselevel & 0xf) << 28);
}
