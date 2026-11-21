// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GL texture and sampler state to the `ffi71` texture structs (P10 of
//! docs/notes/pre-study-rpi5.md): [`texture71::compose`] and
//! [`sampler71::sampler71`] filled into `V3dTex71State` and `V3dTex71Sampler`,
//! the arguments of `v3d_tex71_pack_state` and `v3d_tex71_pack_sampler`
//! (`driver/src/v3d/v3d_tex71.h`).
//!
//! What the two modules leave to the caller is set here, from the composed
//! values and nowhere else: the TMU `type`, the swizzle (the format's composed
//! with GL's, already in `Tex71`), the level 0 UIF fields (strictly UIF, XOR
//! enable, UB pad), `extended`, the transfer function, R/B swap, reverse and the
//! array stride; and for a sampler the cube rule below.
//!
//! Cube maps: the sampler's `cube` is taken from the texture's target (a caller
//! cannot disagree with it), and a seamless cube sampler has all three wrap
//! modes CLAMP_TO_EDGE as Mesa's state tracker forces them (the gl_cube
//! references have wrap R CLAMP though GL left it at REPEAT; the TMU ignores
//! the modes for a cube look-up, `docs/notes/v3d71-cube-seamless.md`).
//!
//! Pure: the siblings are reached through `super::`, so a test includes this
//! with `#[path]` next to `format71.rs`, `texstate71.rs`, `texture71.rs`,
//! `sampler71.rs` and `ffi71.rs` (tests/v3d71_texfill.rs). Not in
//! `src/v3d/mod.rs` yet.
//!
//! How mip71 calls it:
//! ```text
//! let f = texfill71::fill(&gl_tex, &sampler_input)?;   // refusals are named
//! // f.state / f.sampler: pass to v3d_tex71_pack_state / _pack_sampler
//! // (ffi71) into 24-byte, 32-byte aligned records; f.tex has the layout
//! // (offset(level, layer) for uploads, cube_face_stride) and return_32 for
//! // TMU parameter 1.
//! ```

#![allow(dead_code)]

use std::fmt;

use super::ffi71::{V3dTex71Sampler, V3dTex71State};
use super::sampler71::{self, Input, Sampler71, Wrap};
use super::texture71::{self, GlTex71, Target, Tex71};

/// Why a texture and sampler pair has no 7.1 records: the module that said no.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    Texture(texture71::Refusal),
    Sampler(sampler71::Refusal),
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Refusal::Texture(r) => write!(f, "{r}"),
            Refusal::Sampler(r) => write!(f, "{}", r.message()),
        }
    }
}

/// Both records' values, and the composition they came from.
#[derive(Clone, Debug)]
pub struct Filled71 {
    pub state: V3dTex71State,
    pub sampler: V3dTex71Sampler,
    /// layout, cube face stride, return size, the format row
    pub tex: Tex71,
}

/// A composed texture as `struct v3d_tex71_state`.
pub fn state_of(t: &Tex71) -> V3dTex71State {
    V3dTex71State {
        base: t.base,
        width: t.width,
        height: t.height,
        depth: t.depth,
        base_level: t.base_level,
        max_level: t.max_level,
        type_: t.tex_type,
        swizzle: t.swizzle,
        transfer_func: t.transfer_func,
        r_b_swap: t.r_b_swap,
        reverse: t.reverse,
        level_0_is_strictly_uif: t.level_0_is_strictly_uif,
        level_0_xor_enable: t.level_0_xor_enable,
        extended: t.extended,
        level_0_ub_pad: t.level_0_ub_pad,
        array_stride_64: t.array_stride_64,
    }
}

/// Sampler values as `struct v3d_tex71_sampler`.
pub fn sampler_of(s: &Sampler71) -> V3dTex71Sampler {
    V3dTex71Sampler {
        wrap_s: s.wrap_s,
        wrap_t: s.wrap_t,
        wrap_r: s.wrap_r,
        min_nearest: s.min_nearest,
        mag_nearest: s.mag_nearest,
        mip_nearest: s.mip_nearest,
        mip_none: s.mip_none,
        min_lod: s.min_lod,
        max_lod: s.max_lod,
        lod_bias: s.lod_bias,
        compare_func: s.compare_func,
        max_anisotropy: s.max_anisotropy,
        border_color_mode: s.border_color_mode,
        border_color: s.border_color,
    }
}

/// The sampler input as the texture's target makes it (see the module docs).
pub fn sampler_input_for(target: Target, smp: &Input) -> Input {
    let mut i = *smp;
    i.cube = target == Target::Cube;
    if i.cube && i.seamless {
        i.wrap_s = Wrap::ClampToEdge;
        i.wrap_t = Wrap::ClampToEdge;
        i.wrap_r = Wrap::ClampToEdge;
    }
    i
}

/// The texture and sampler structs for one GL texture and its sampler state.
pub fn fill(gl: &GlTex71, smp: &Input) -> Result<Filled71, Refusal> {
    let tex = texture71::compose(gl).map_err(Refusal::Texture)?;
    let s = sampler71::sampler71(&sampler_input_for(gl.target, smp)).map_err(Refusal::Sampler)?;
    Ok(Filled71 { state: state_of(&tex), sampler: sampler_of(&s), tex })
}
