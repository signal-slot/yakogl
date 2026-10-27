// SPDX-License-Identifier: Apache-2.0 AND MIT
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
// SPDX-FileCopyrightText: 2025-2026 Collabora Ltd.
// Derived from Mesa 26.2.1 src/gallium/drivers/panfrost/pan_fb_preload.c
// (MIT). The upstream part stays under its licence (LICENSES/MIT.txt); our
// changes are Apache-2.0. See NOTICE.
//! The colour preload program: the fragment shader of a frame's pre-frame
//! draw, which draws the old image back into the tile buffer before the
//! frame's own draws (`pan_preload_get_shader`, pan_fb_preload.c:376-575,
//! for one single-sampled 2D float colour target).
//!
//! Mesa builds it as NIR, not from GLSL: the pixel's coordinate interpolated
//! from input 0 of the frame shader's attribute table (the draw's four
//! corners, in pixels), a `txl` of the target viewed as a texture (texture
//! table 3 and sampler table 2 at index 0, `PAN_BLIT_TABLE_*`; the sampler
//! does not normalise the coordinate) at level 0, its fp32 result stored to
//! colour output 0. It is compiled without promoted immediates and with no
//! push words, and without ATEST (the draw forces early depth/stencil):
//! BLEND takes the cumulative coverage in r60 as it is.
//!
//! The program goes through this back end's passes as any other fragment
//! shader does, and its words are the ones Mesa 26.2.1 made for it on the
//! shim and Mesa 25.0.7 on the board (`tests.rs`, `the_colour_preload_is_mesa_s`).

use super::bir::{Dest, Idx, Ins, LodMode, Op, Prog, Special, Src, TexDim};
use super::select::{alu, preloads, split};
use super::{flow, pack, ra, sched, MaliError};
use crate::shaderc::online::arena::Arena;

/// `pan_res_handle(PAN_BLIT_TABLE_TEXTURE, 0)` and
/// `(PAN_BLIT_TABLE_SAMPLER, 0)` packed as `va_tex_handle` packs a constant
/// pair (pan_nir_lower_tex.c:723-745): texture table 3, sampler table 2.
pub const PRELOAD_TEX_HANDLE: u32 = (3 << 27) | (2 << 11);

/// The Z/S preload's second look-up: texture index 1 of the same table
/// (`tex_hw_index(1)`), the same sampler.
pub const PRELOAD_TEX_HANDLE_1: u32 = PRELOAD_TEX_HANDLE | (1 << 16);

/// The attribute table and input the coordinate is read from
/// (`attr_hw_index(0)`: `PAN_BLIT_TABLE_ATTRIBUTE`, 0).
const COORD_TABLE: u8 = 0;

/// The colour preload program's words (padded as every program is), its
/// preloaded registers and work register count.
pub struct Preload<'s> {
    pub code: &'s [u64],
    pub preload: u64,
    pub work_reg_count: u8,
}

/// Build and compile the colour preload program into `out`.
pub fn colour<'s>(out: &'s mut [u64], scratch: &Arena<'_>) -> Result<Preload<'s>, MaliError> {
    let ins = scratch.alloc_filled(super::bir::MAX_INSTRS, Ins::NOP).map_err(|_| MaliError::OutOfScratch)?;
    let mut p = Prog::new(ins);
    /* the coordinate: two fp32 lanes interpolated at the centre */
    let ld = p.temp()?;
    let op = Op::LdVarImmF32 { table: COORD_TABLE, index: 0, vecsize: 2 };
    p.push(Ins::new(op, &[Dest::ssa(ld)], &[Src::of(Idx::Reg(61))]))?;
    let c = split(&mut p, ld, 2)?;
    let sr = alu(&mut p, Op::Collect, &[Src::ssa(c[0]), Src::ssa(c[1])])?;
    /* txl at level 0: an fp32 vec4 */
    let op = Op::TexSingle { sr_count: 2, mask: 0xf, f16: false, skip: false, lod: LodMode::Zero, dim: TexDim::D2 };
    let t = alu(&mut p, op, &[Src::ssa(sr), Src::of(Idx::Const(PRELOAD_TEX_HANDLE)), Src::of(Idx::Const(0))])?;
    let w = split(&mut p, t, 4)?;
    let col = alu(&mut p, Op::Collect, &[Src::ssa(w[0]), Src::ssa(w[1]), Src::ssa(w[2]), Src::ssa(w[3])])?;
    alu(
        &mut p,
        Op::Blend { sr_count: 4, f32: true },
        &[
            Src::ssa(col),
            Src::of(Idx::Reg(60)),
            Src::of(Idx::Special(Special::BlendDescriptor0, false)),
            Src::of(Idx::Special(Special::BlendDescriptor0, true)),
            Src::NULL,
        ],
    )?;
    finish(p, out, scratch)
}

/// The depth/stencil preload program (`pan_preload_get_shader` for a D24S8
/// whose depth (`z`), stencil (`s`) or both are preloaded): the coordinate
/// as for the colour; a `txl` of each preloaded plane's view, in that
/// order from texture index 0 (`pan_preload_emit_textures`: the depth view
/// float, its red channel the depth; the stencil view uint, its green
/// channel the stencil, pan_fb_preload.c:518-524), each a one-channel
/// look-up once NIR drops the unread channels; then the fragment outputs
/// as `pan_nir_lower_fs_outputs` lowers them (pan_nir_lower_fs_outputs.c:
/// 140-176): ATEST of the coverage with alpha 1.0, and ZS_EMIT of what is
/// preloaded, the other source a don't-care (0).
pub fn zs<'s>(out: &'s mut [u64], scratch: &Arena<'_>, z: bool, s: bool) -> Result<Preload<'s>, MaliError> {
    if !(z || s) {
        return Err(MaliError::Defect(super::Defect::PreloadPlanes));
    }
    let ins = scratch.alloc_filled(super::bir::MAX_INSTRS, Ins::NOP).map_err(|_| MaliError::OutOfScratch)?;
    let mut p = Prog::new(ins);
    let ld = p.temp()?;
    let op = Op::LdVarImmF32 { table: COORD_TABLE, index: 0, vecsize: 2 };
    p.push(Ins::new(op, &[Dest::ssa(ld)], &[Src::of(Idx::Reg(61))]))?;
    let c = split(&mut p, ld, 2)?;
    let sr = alu(&mut p, Op::Collect, &[Src::ssa(c[0]), Src::ssa(c[1])])?;
    let look = |p: &mut Prog<'_>, mask: u8, handle: u32| {
        let op = Op::TexSingle { sr_count: 2, mask, f16: false, skip: false, lod: LodMode::Zero, dim: TexDim::D2 };
        alu(p, op, &[Src::ssa(sr), Src::of(Idx::Const(handle)), Src::of(Idx::Const(0))])
    };
    let none = Src::of(Idx::Const(0));
    let depth = if z { Src::ssa(look(&mut p, 0x1, PRELOAD_TEX_HANDLE)?) } else { none };
    let stencil = if s {
        let handle = if z { PRELOAD_TEX_HANDLE_1 } else { PRELOAD_TEX_HANDLE };
        Src::ssa(look(&mut p, 0x2, handle)?)
    } else {
        none
    };
    let alpha = Src::of(Idx::Const(1.0f32.to_bits()));
    let cov =
        alu(&mut p, Op::Atest, &[Src::of(Idx::Reg(60)), alpha, Src::of(Idx::Special(Special::AtestDatum, false))])?;
    alu(&mut p, Op::ZsEmit { z, s }, &[depth, stencil, Src::ssa(cov)])?;
    finish(p, out, scratch)
}

/// The passes after selection, as for any fragment shader, without
/// promoted immediates or push words.
fn finish<'s>(mut p: Prog<'_>, out: &'s mut [u64], scratch: &Arena<'_>) -> Result<Preload<'s>, MaliError> {
    preloads(&mut p)?;
    /* no push words at all, and no immediate promoted */
    let mut fau = super::fau::Layout::EMPTY;
    super::fau::lower_constants(&mut p, &mut fau, false)?;
    if fau.count != 0 {
        return Err(MaliError::Defect(super::Defect::FauOverflow));
    }
    flow::analyze_helper_requirements(&mut p);
    sched::pressure_schedule(&mut p, scratch)?;
    let work_reg_count = ra::allocate(&mut p, scratch)?;
    flow::assign_slots(&mut p);
    flow::insert_flow_control_nops(&mut p, true)?;
    flow::merge_flow(&mut p, true);
    flow::mark_last(&mut p);
    let preload = flow::preload_mask(&p, true);
    pack::lower_blend(&mut p)?;
    let n = pack::pack(&p, out)?;
    Ok(Preload { code: &out[..n], preload, work_reg_count })
}
