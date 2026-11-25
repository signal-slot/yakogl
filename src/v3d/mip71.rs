// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! V3D 7.1 `glGenerateMipmap`: a GL texture to the inputs of its TFU jobs
//! (P10 of docs/notes/pre-study-rpi5.md; docs/notes/v3d71-gl-table.md
//! section 3, "Mip chain").
//!
//! From the GL texture as the GL core holds it ([`GlTex71`]: format and
//! type, level 0's size, the levels its storage has, base and max level,
//! the buffer's GPU address) this gives one
//! [`V3dTfu71Args`](super::ffi71::V3dTfu71Args) per TFU job, the struct
//! `driver/src/v3d/v3d_tfu71.h`'s `v3d_tfu71_pack` takes, or a named
//! [`Refusal`] (the GL core then filters on the CPU).
//!
//! Mesa 26.2.2 is the reference: `v3d_generate_mipmap` (v3d_blit.c) calls
//! `v3dX(tfu)(pctx, prsc, prsc, base_level, base_level, last_level, layer,
//! layer, true)`, and the 7.1 branch of v3dx_tfu.c fills the words from it:
//!
//! * one job makes every level after the base: `num_mm` = last - base, the
//!   source and the destination are both the base level (IIA = IOA, the
//!   buffer plus `v3d_layer_offset(base_level, layer)`), IOS is the base
//!   level's size, the tiling and padded height are the base level's slice;
//! * the data type is the texture's own TMU type (`v3d_get_tex_format`),
//!   which is what [`texture71::compose`] gives the texture record, so the
//!   TFU writes the levels in the layout the sampler reads them from
//!   (both are `texture71::layout`, Mesa's `v3d_setup_slices`);
//! * the last level is GL_TEXTURE_MAX_LEVEL clamped to the storage, as the
//!   record's (`texture71::compose`).
//!
//! The TFU is not gated on `HUB_IDENT1` bit 17 (`WITH_TFU`): it reads 0 on
//! the Pi 5's 7.1.10 although the TFU runs the `IOC` job (`rpi5_tfu` PASS,
//! tools/probes/rpi_5/rpi5_tfu/README.md; docs/notes/v3d71-gl-table.md
//! section 6). Nothing here takes the identity.
//!
//! A cube map is six jobs, one per face (`tfuplan71`'s `layer_count`), at
//! the face's offset; every face is a whole mip tree at Mesa's
//! `cube_map_stride` (texture71's `layer_stride`), so the levels the TFU
//! writes below a face's base level are that face's. Mesa itself does not
//! send a cube map to the TFU (`v3d_generate_mipmap` declines
//! `first_layer != last_layer` and the state tracker blits instead): the
//! cube path is predicted, the 2D path is `gl_mipgen`'s.
//!
//! An empty plan is the answer for a texture with no level to make (the
//! base level is the last one: a 1x1 texture, or max level = base level);
//! GL generates nothing then.
//!
//! Pure, no `crate::` import: its siblings are reached through `super::`, so
//! a test includes it with `#[path]` next to `ffi71.rs`, `format71.rs`,
//! `texstate71.rs`, `texture71.rs` and `tfuplan71.rs` (tests/v3d71_mip.rs).
//! Not in `src/v3d/mod.rs` yet: the 7.1 table wiring is a later item.

#![allow(dead_code)]

use std::fmt;

use super::ffi71::{
    V3dTfu71Args, V3dTfu71Surface, V3D_TFU71_TILING_LINEARTILE, V3D_TFU71_TILING_UBLINEAR_1_COLUMN,
    V3D_TFU71_TILING_UBLINEAR_2_COLUMN, V3D_TFU71_TILING_UIF_NO_XOR, V3D_TFU71_TILING_UIF_XOR,
};
use super::texture71::{self, GlTex71, Target, Tiling};
use super::tfuplan71;

/// Why a GL texture's mip chain is not made by the TFU here. Every variant
/// is a decision; the GL core filters on the CPU for each.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// texture71 has no 7.1 texture for it (format, size, levels, revision).
    Texture(texture71::Refusal),
    /// The TFU cannot make the chain (tfuplan71: the type, the level count).
    Tfu(tfuplan71::Refusal),
    /// The buffer address plus a face's base level passes 32 bits.
    AddressOverflow,
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Refusal::Texture(r) => write!(f, "mip 7.1: {r}"),
            Refusal::Tfu(r) => write!(f, "mip 7.1: {}", r.message()),
            Refusal::AddressOverflow => write!(f, "mip 7.1: buffer address plus a face's base level passes 32 bits"),
        }
    }
}

/// One TFU job of the chain.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MipJob71 {
    /// The layer (cube face) the job makes the levels of; 0 for a 2D texture.
    pub layer: u32,
    /// `v3d_tfu71_pack`'s input.
    pub args: V3dTfu71Args,
}

/// The chain `glGenerateMipmap` makes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MipPlan71 {
    /// The level the jobs read (GL_TEXTURE_BASE_LEVEL).
    pub base_level: u32,
    /// The last level they write (GL_TEXTURE_MAX_LEVEL clamped to the
    /// storage); `base_level` when there is nothing to make.
    pub last_level: u32,
    /// One job per layer; empty when there is nothing to make.
    pub jobs: Vec<MipJob71>,
}

fn tfu_tiling(t: Tiling) -> (tfuplan71::Tiling, u32) {
    match t {
        Tiling::LinearTile => (tfuplan71::Tiling::LinearTile, V3D_TFU71_TILING_LINEARTILE),
        Tiling::UbLinear1Column => (tfuplan71::Tiling::UbLinear1Column, V3D_TFU71_TILING_UBLINEAR_1_COLUMN),
        Tiling::UbLinear2Column => (tfuplan71::Tiling::UbLinear2Column, V3D_TFU71_TILING_UBLINEAR_2_COLUMN),
        Tiling::UifNoXor => (tfuplan71::Tiling::UifNoXor, V3D_TFU71_TILING_UIF_NO_XOR),
        Tiling::UifXor => (tfuplan71::Tiling::UifXor, V3D_TFU71_TILING_UIF_XOR),
    }
}

/// The TFU jobs of `glGenerateMipmap` on `t`.
pub fn plan(t: &GlTex71) -> Result<MipPlan71, Refusal> {
    // the record's own answers: type, layout, base and max level checked
    let tex = texture71::compose(t).map_err(Refusal::Texture)?;
    let base = t.base_level;
    let last = t.max_level.min(t.levels - 1);
    if last == base {
        return Ok(MipPlan71 { base_level: base, last_level: last, jobs: Vec::new() });
    }
    let l = &tex.layout;
    let s = l.slices[base as usize];
    let (tiling, code) = tfu_tiling(s.tiling);
    let (width, height) = ((t.width >> base).max(1), (t.height >> base).max(1));
    let layers = match t.target {
        Target::Tex2d => 1,
        Target::Cube => 6,
    };
    let planned = tfuplan71::plan(&tfuplan71::Args {
        width,
        height,
        base_level: base,
        last_level: last,
        src_level: base,
        ttype: tex.tex_type,
        cpp: l.cpp,
        src_tiling: tiling,
        dst_tiling: tiling,
        layer: 0,
        layer_count: layers,
    })
    .map_err(Refusal::Tfu)?;
    // v3dx_tfu.c:114-127, 186-199: a UIF side's padded height (rows), a
    // raster side's stride (bytes; texture71 lays out no raster level), 0
    // for LINEARTILE and UB-linear
    let stride_or_padded_height = match s.tiling {
        Tiling::UifNoXor | Tiling::UifXor => s.padded_height,
        Tiling::LinearTile | Tiling::UbLinear1Column | Tiling::UbLinear2Column => 0,
    };
    let mut jobs = Vec::with_capacity(planned.len());
    for j in &planned {
        let off = l.offset(base, j.layer).ok_or(Refusal::AddressOverflow)?;
        let addr = t.bo_address.checked_add(off).ok_or(Refusal::AddressOverflow)?;
        let side = V3dTfu71Surface { addr, tiling: code, stride_or_padded_height };
        jobs.push(MipJob71 {
            layer: j.layer,
            args: V3dTfu71Args {
                ttype: tex.tex_type,
                cpp: l.cpp,
                width,
                height,
                num_mm: j.nummm,
                src: side,
                dst: side,
            },
        });
    }
    Ok(MipPlan71 { base_level: base, last_level: last, jobs })
}
