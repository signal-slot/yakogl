// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! `glGenerateMipmap` on V3D 7.1 as TFU job inputs (P10 of
//! docs/notes/pre-study-rpi5.md): a GL texture in, the sequence of
//! `V3dTfu71Args` (the inputs of `v3d_tfu71_pack`, `driver/src/v3d/v3d_tfu71.h`)
//! out. The packing is the driver's; this module only decides what goes in.
//!
//! Built on the merged pieces and copying none of them: [`texture71::compose`]
//! gives the texture's format, texel size and layout (so the addresses and
//! padded heights are the ones the TMU record carries), [`tfuplan71::plan`] the
//! jobs (one per cube face) and their level count. Mesa's `v3d_generate_mipmap`
//! reads and writes the same base level (`DIMTW`: the base is the source, the
//! levels after it the destination), so source and destination are one surface.
//!
//! The TFU is not gated on `HUB_IDENT1` bit 17 (it reads 0 on the Pi 5 and the
//! TFU still runs, `tools/probes/rpi_5/rpi5_tfu/README.md`): nothing here
//! reads the identity.
//!
//! Levels generated: `base_level + 1 ..= min(max_level, levels - 1)`. A texture
//! with nothing after its base gives no jobs (GL's no-op). A base level other
//! than 0 is not refused: Mesa passes the texture's base level to the TFU as it
//! is, and the plan takes it.
//!
//! Refusals ([`Refusal::name`] is stable): 3D and array textures ([`Shape`]),
//! compressed formats, and everything `texture71` or `tfuplan71` refuse
//! (a type the TFU does not take for mipmaps, a raster destination, the level
//! count, ...).
//!
//! Pure: the siblings are reached through `super::`, so a test includes this
//! with `#[path]` next to `ffi71.rs`, `format71.rs`, `texstate71.rs`,
//! `texture71.rs` and `tfuplan71.rs` (tests/v3d71_mip.rs). Not in `mod.rs`.
//! `ffi71` has every field a job needs; no local copy of its structs is kept.

#![allow(dead_code)]

use super::ffi71::{V3dTfu71Args, V3dTfu71Surface};
use super::texture71::{self, GlTex71, Target, Tiling as TexTiling};
use super::tfuplan71::{self, Tiling};

/// What the GL texture is. 3D and array textures have no 7.1 texture
/// composition here and the plan does not walk their slices.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    Tex2d,
    Cube,
    Tex3d,
    Array2d,
}

/// One TFU job: the face (0 for a 2D texture) and the inputs of
/// `v3d_tfu71_pack`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MipJob {
    pub layer: u32,
    pub args: V3dTfu71Args,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// 3D textures (Mesa generates them per slice; not done here)
    Texture3d,
    /// array textures
    TextureArray,
    /// `shape` and `GlTex71::target` disagree
    ShapeMismatch,
    /// a compressed format: no block layout in texture71
    Compressed,
    /// texture71 refused the texture (anything but compressed)
    Texture(texture71::Refusal),
    /// the buffer address plus a face offset passes 32 bits
    AddressOverflow,
    /// tfuplan71 refused (type not supported for mipmaps, level count, ...)
    Plan(tfuplan71::Refusal),
}

impl Refusal {
    /// A stable name for logs and tests.
    pub fn name(&self) -> &'static str {
        match self {
            Refusal::Texture3d => "mip71: 3d texture",
            Refusal::TextureArray => "mip71: array texture",
            Refusal::ShapeMismatch => "mip71: shape does not match the texture target",
            Refusal::Compressed => "mip71: compressed format",
            Refusal::Texture(_) => "mip71: texture refused by texture71",
            Refusal::AddressOverflow => "mip71: address passes 32 bits",
            Refusal::Plan(r) => r.message(),
        }
    }
}

fn tiling_of(t: TexTiling) -> Tiling {
    match t {
        TexTiling::LinearTile => Tiling::LinearTile,
        TexTiling::UbLinear1Column => Tiling::UbLinear1Column,
        TexTiling::UbLinear2Column => Tiling::UbLinear2Column,
        TexTiling::UifNoXor => Tiling::UifNoXor,
        TexTiling::UifXor => Tiling::UifXor,
    }
}

fn is_uif(t: TexTiling) -> bool {
    matches!(t, TexTiling::UifNoXor | TexTiling::UifXor)
}

/// The jobs of `glGenerateMipmap` for `tex` (empty when there is no level
/// after the base). `tex.bo_address` is the buffer's GPU address.
pub fn generate(shape: Shape, tex: &GlTex71) -> Result<Vec<MipJob>, Refusal> {
    match (shape, tex.target) {
        (Shape::Tex3d, _) => return Err(Refusal::Texture3d),
        (Shape::Array2d, _) => return Err(Refusal::TextureArray),
        (Shape::Tex2d, Target::Tex2d) | (Shape::Cube, Target::Cube) => {}
        _ => return Err(Refusal::ShapeMismatch),
    }
    let t = texture71::compose(tex).map_err(|r| match r {
        texture71::Refusal::Compressed { .. } => Refusal::Compressed,
        o => Refusal::Texture(o),
    })?;
    let base = tex.base_level;
    let last = tex.max_level.min(tex.levels - 1);
    if last <= base {
        return Ok(Vec::new());
    }
    let slice = t.layout.slices[base as usize];
    let tiling = tiling_of(slice.tiling);
    let (w, h) = ((tex.width >> base).max(1), (tex.height >> base).max(1));
    let jobs = tfuplan71::plan(&tfuplan71::Args {
        width: w,
        height: h,
        base_level: base,
        last_level: last,
        src_level: base,
        ttype: t.tex_type,
        cpp: t.layout.cpp,
        src_tiling: tiling,
        dst_tiling: tiling,
        layer: 0,
        layer_count: t.layout.layers,
    })
    .map_err(Refusal::Plan)?;
    // UIF: the padded height in rows; the other tilings carry none
    let sp = if is_uif(slice.tiling) { slice.padded_height } else { 0 };
    jobs.iter()
        .map(|j| {
            let off = t.layout.offset(base, j.layer).ok_or(Refusal::AddressOverflow)?;
            let addr = tex.bo_address.checked_add(off).ok_or(Refusal::AddressOverflow)?;
            let surf = V3dTfu71Surface { addr, tiling: tiling as u32, stride_or_padded_height: sp };
            Ok(MipJob {
                layer: j.layer,
                args: V3dTfu71Args {
                    ttype: t.tex_type,
                    cpp: t.layout.cpp,
                    width: w,
                    height: h,
                    num_mm: j.nummm,
                    src: surf,
                    dst: surf,
                },
            })
        })
        .collect()
}
