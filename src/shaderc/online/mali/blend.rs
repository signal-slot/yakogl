// SPDX-License-Identifier: Apache-2.0 AND MIT
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
// SPDX-FileCopyrightText: 2018, 2022 Alyssa Rosenzweig
// SPDX-FileCopyrightText: 2019-2026 Collabora, Ltd.
// SPDX-FileCopyrightText: 2025 Arm Ltd.
// Derived from Mesa 26.2.1 src/panfrost/compiler/bifrost/bifrost_compile.c
// (bi_emit_alu, bi_alu_src_index, bi_make_vec_to, bi_emit_ld_tile,
// bi_emit_intrinsic; Collabora 2020, 2026, Alyssa Rosenzweig 2022, Arm
// 2025), bi_lower_swizzle.c (Collabora 2020) and
// src/gallium/drivers/panfrost/pan_blend_cso.c (Alyssa Rosenzweig 2018,
// Collabora 2019-2025) (MIT).
// The upstream part stays under its licence (LICENSES/MIT.txt); our changes
// are Apache-2.0. See NOTICE.
//! The blend shader: the program the fragment shader's `BRANCHZI` calls
//! when the draw's blend descriptor is in Shader mode (Mesa's
//! `panfrost_get_blend`, pan_context.c:163-229): a blend the fixed-function
//! unit cannot run (a factor pair it has no operands for, min or max,
//! SRC_ALPHA_SATURATE) or one reading constant channels that differ, which
//! the one constant of a fixed-function descriptor cannot hold.
//!
//! Mesa builds it as NIR and runs the bifrost back end's NIR passes on it
//! (`GENX(pan_blend_get_shader_locked)`, pan_blend_cso.c:59-132); that NIR
//! and those passes are [`super::blend_nir`]. This module selects the NIR
//! Mesa's passes leave into the back end's instructions as
//! bifrost_compile.c's `bi_emit_*` do (in NIR order, each preloaded
//! register read through one MOV at the start), then runs this back end's
//! passes with Mesa's blend-shader rules (`inputs->is_blend`): no pushed
//! words and no promoted immediates, r0-r15 only and no precoloured BLEND
//! (ra.rs `allocate_blend`), no wait before the BLEND or the LD_TILE
//! (flow.rs with `fragment` false). The constants are the sysval UBO's first
//! four words, which every fragment shader pushes as its first four FAU
//! words (`bi_pick_blend_constants`): the blend shader runs in the fragment
//! shader's thread and reads them there as `u0`, `u1`.
//!
//! The bytes are Mesa's (tools/mali/testdata/blend_deqp.words, every blend
//! shader of dEQP-GLES2 fragment_ops.blend; blend_*.expect.json and
//! blend_case*.words, the shim driver's; tests.rs).

use super::bir::{clamp, Dest, Idx, Ins, Op, Prog, Src, Swz};
pub use super::blend_nir::{Base, Channel, Factor, Func, Key};
use super::blend_nir::{Intr, Kind, Nir, Op as N};
use super::select::{alu, split};
use super::{flow, opt, pack, ra, sched, MaliError};
use crate::shaderc::online::arena::Arena;

/// `BI_PRELOAD_SAMPLE_ID`: r61, the sample id in bits 16-23
/// (bifrost_compile.c:887-896), five bits of it read.
const SAMPLE_ID_REG: u8 = 61;
const SAMPLE_ID_SHIFT: u32 = 16;
const SAMPLE_ID_MASK: u32 = 0x1f;

/// `-0.0`, the addend of a product alone (an `fmul` as an `FMA`) and of
/// `bi_f32_to_f16_to`'s `FADD`.
const NEG_ZERO: Src = Src { neg: true, ..Src::of(Idx::Const(0)) };

/// The blend shader's words (padded as every program is) and its work
/// register count.
pub struct BlendShader<'s> {
    pub code: &'s [u64],
    pub work_reg_count: u8,
}

/// A NIR value as the back end holds it.
#[derive(Clone, Copy)]
enum V {
    None,
    /// One register: a 32-bit scalar, or a 16-bit scalar or pair.
    Word(u16),
    /// A 16-bit register read through a swizzle: an `FCLAMP` whose source
    /// was swizzled, the swizzle moved out (`bi_lower_swizzle`,
    /// bi_lower_swizzle.c:117-130) to a `SWZ.v2i16` that copy propagation
    /// folds into the readers.
    Swz(u16, Swz),
    /// A 16-bit vec4 in two registers, and the vector of both when one
    /// was made (the LD_TILE's).
    Pair([u16; 2], Option<u16>),
    /// A 16-bit vec4 as the COLLECT `bi_make_vec_to` makes: read whole only
    /// (the colour the BLEND stores).
    Collected(u16),
    /// The blend constants, FAU words 0-3.
    Ubo,
    /// An fp32 colour, r0-r3 (four registers).
    Quad([u16; 4]),
    /// `u2u8` of a 32-bit value.
    U8(u16),
}

struct Sel<'n, 'a> {
    nir: &'n Nir<'a>,
    vals: [V; super::blend_nir::MAX],
    /// The preloaded registers in the order of their first read, and the
    /// value of each (`bi_preload`).
    preload: [(u8, u16); 8],
    np: usize,
}

impl Sel<'_, '_> {
    fn preload(&mut self, p: &mut Prog<'_>, reg: u8) -> Result<u16, MaliError> {
        if let Some(&(_, v)) = self.preload[..self.np].iter().find(|x| x.0 == reg) {
            return Ok(v);
        }
        let v = p.temp()?;
        self.preload[self.np] = (reg, v);
        self.np += 1;
        Ok(v)
    }

    /// A 32-bit source of an intrinsic (`bi_src_index`).
    fn word(&self, d: u16) -> Result<Src, MaliError> {
        let i = &self.nir.ins[d as usize];
        match (i.kind, self.vals[d as usize]) {
            (Kind::Const, _) => Ok(Src::of(Idx::Const(i.k[0] as u32))),
            (_, V::Word(v)) => Ok(Src::ssa(v)),
            _ => Err(MaliError::BlendVariantNotLowered),
        }
    }

    /// `bi_alu_src_index` (bifrost_compile.c:2261-2340): the word holding
    /// the components read, and a 16-bit source's halves.
    fn src(&self, user: u16, k: usize) -> Result<Src, MaliError> {
        let u = &self.nir.ins[user as usize];
        let s = u.srcs[k];
        let d = &self.nir.ins[s.def as usize];
        let comps = u.comps as usize;
        let bits = d.bits as usize;
        let offset = s.swz[0] as usize * bits / 32;
        if (1..comps).any(|c| s.swz[c] as usize * bits / 32 != offset) {
            return Err(MaliError::BlendVariantNotLowered);
        }
        let swz = if bits == 16 {
            let c0 = s.swz[0] & 1;
            let c1 = if comps > 1 { s.swz[1] & 1 } else { c0 };
            match (c0, c1) {
                (0, 0) => Swz::H00,
                (0, _) => Swz::H01,
                (_, 0) => Swz::H10,
                _ => Swz::H11,
            }
        } else if bits == 32 {
            Swz::H01
        } else {
            return Err(MaliError::BlendVariantNotLowered);
        };
        let src = match (d.kind, self.vals[s.def as usize]) {
            (Kind::Const, _) => {
                /* bi_emit_load_const: the word's components packed */
                let v = if bits == 16 {
                    let w = offset * 2;
                    let hi = if w + 1 < d.comps as usize { d.k[w + 1] } else { 0 };
                    (d.k[w] & 0xffff) as u32 | ((hi & 0xffff) as u32) << 16
                } else {
                    d.k[offset] as u32
                };
                Src::of(Idx::Const(v))
            }
            (_, V::Word(v)) if offset == 0 => Src::ssa(v),
            (_, V::Swz(v, inner)) if offset == 0 => return Ok(Src::ssa(v).swz(compose(inner, swz))),
            (_, V::Pair(w, _)) => Src::ssa(w[offset]),
            _ => return Err(MaliError::BlendVariantNotLowered),
        };
        Ok(src.swz(swz))
    }

    /// `bi_half(bi_extract(v, c >> 1), c & 1)` for `bi_make_vec_to`.
    fn half(&self, d: u16, c: u8) -> Result<Src, MaliError> {
        let i = &self.nir.ins[d as usize];
        let lane = if c & 1 == 1 { Swz::H11 } else { Swz::H00 };
        let w = (c >> 1) as usize;
        Ok(match (i.kind, self.vals[d as usize]) {
            (Kind::Const, _) => Src::of(Idx::Const((i.k[c as usize] & 0xffff) as u32)).swz(lane),
            (_, V::Word(v)) if w == 0 => Src::ssa(v).swz(lane),
            (_, V::Swz(v, inner)) if w == 0 => Src::ssa(v).swz(compose(inner, lane)),
            (_, V::Pair(ws, _)) => Src::ssa(ws[w]).swz(lane),
            _ => return Err(MaliError::BlendVariantNotLowered),
        })
    }
}

/// The lanes of a 16-bit swizzle.
fn lanes(s: Swz) -> (u8, u8) {
    match s {
        Swz::H00 => (0, 0),
        Swz::H10 => (1, 0),
        Swz::H11 => (1, 1),
        _ => (0, 1),
    }
}

/// `outer` read through `inner` (`bi_compose_swizzle_16`).
fn compose(inner: Swz, outer: Swz) -> Swz {
    let i = lanes(inner);
    let o = lanes(outer);
    let pick = |l: u8| if l == 0 { i.0 } else { i.1 };
    match (pick(o.0), pick(o.1)) {
        (0, 0) => Swz::H00,
        (0, _) => Swz::H01,
        (_, 0) => Swz::H10,
        _ => Swz::H11,
    }
}

/// Build and compile the blend shader of `key` for a target whose memory
/// format (the internal descriptor's conversion word) is `conversion`,
/// into `out`. A shader this compiler does not build as Mesa does is
/// refused (`BlendVariantNotLowered`), never approximated.
pub fn shader<'s>(
    out: &'s mut [u64],
    scratch: &Arena<'_>,
    key: Key,
    conversion: u32,
) -> Result<BlendShader<'s>, MaliError> {
    if conversion != 0xed000 || !matches!(key.samples, 1 | 4) || key.mask & 0xf == 0 {
        return Err(MaliError::BlendVariantNotLowered);
    }
    let mut nir = Nir::new(scratch)?;
    super::blend_nir::build(&mut nir, key)?;
    nir.optimize()?;
    let ins = scratch.alloc_filled(super::bir::MAX_INSTRS, Ins::NOP).map_err(|_| MaliError::OutOfScratch)?;
    let mut p = Prog::new(ins);
    let mut sel = Sel { nir: &nir, vals: [V::None; super::blend_nir::MAX], preload: [(0, 0); 8], np: 0 };
    for k in 0..nir.len {
        let i = nir.at(k);
        let v = select(&mut sel, &mut p, i)?;
        sel.vals[i as usize] = v;
    }
    /* the preloads' MOVs, each put first when made (`bi_preload`) */
    for &(reg, v) in &sel.preload[..sel.np] {
        p.insert(0, Ins::new(Op::MovI32, &[Dest::ssa(v)], &[Src::of(Idx::Reg(reg))]))?;
    }
    finish(p, out, scratch)
}

/// One NIR instruction's back-end instructions (`bi_emit_instr`).
fn select(s: &mut Sel<'_, '_>, p: &mut Prog<'_>, i: u16) -> Result<V, MaliError> {
    let ins = s.nir.ins[i as usize];
    let w = |v: u16| Ok(V::Word(v));
    match ins.kind {
        Kind::Const | Kind::Undef => Ok(V::None),
        Kind::Intr(Intr::BlendInput) if ins.bits == 32 => {
            let mut r = [0u16; 4];
            for (k, v) in r.iter_mut().enumerate() {
                *v = s.preload(p, k as u8)?;
            }
            Ok(V::Quad(r))
        }
        Kind::Intr(Intr::BlendInput) => {
            let r0 = s.preload(p, 0)?;
            let r1 = s.preload(p, 1)?;
            Ok(V::Pair([r0, r1], None))
        }
        Kind::Intr(Intr::BlendInputDual) => Err(MaliError::BlendVariantNotLowered),
        Kind::Intr(Intr::Coverage) => w(s.preload(p, ra::CUMULATIVE_COVERAGE)?),
        Kind::Intr(Intr::SampleId) => {
            /* bi_load_sample_id_to: (r61 >> 16) & 0x1f */
            let r61 = s.preload(p, SAMPLE_ID_REG)?;
            let c = |v: u32| Src::of(Idx::Const(v));
            w(alu(p, Op::RshiftAndI32, &[Src::ssa(r61), c(SAMPLE_ID_MASK), c(SAMPLE_ID_SHIFT)])?)
        }
        Kind::Intr(Intr::LoadTile) => {
            let srcs = [s.word(ins.srcs[0].def)?, s.word(ins.srcs[1].def)?, s.word(ins.srcs[2].def)?];
            let t = alu(p, Op::LdTile { vecsize: 4, f16: true }, &srcs)?;
            let d = split(p, t, 2)?;
            Ok(V::Pair([d[0], d[1]], Some(t)))
        }
        Kind::Intr(Intr::LoadUbo) => Ok(V::Ubo),
        Kind::Intr(Intr::Blend) => {
            let cd = &s.nir.ins[ins.srcs[2].def as usize];
            let col = match (cd.kind, s.vals[ins.srcs[2].def as usize]) {
                (_, V::Pair(_, Some(c)) | V::Collected(c)) => c,
                (Kind::Const, _) if cd.bits == 16 && cd.comps == 4 => {
                    /* bi_emit_load_const: a MOV of each word's
                     * immediate, then COLLECT */
                    /* (copy propagation takes the immediates into the
                     * COLLECT) */
                    let word = |w: usize| {
                        Src::of(Idx::Const((cd.k[2 * w] & 0xffff | (cd.k[2 * w + 1] & 0xffff) << 16) as u32))
                    };
                    alu(p, Op::Collect, &[word(0), word(1)])?
                }
                _ => return Err(MaliError::BlendVariantNotLowered),
            };
            let desc = s.nir.ins[ins.srcs[1].def as usize].k[0];
            alu(
                p,
                Op::Blend { sr_count: 2, f32: false, u32: false },
                &[
                    Src::ssa(col),
                    s.word(ins.srcs[0].def)?,
                    Src::of(Idx::Const(desc as u32)),
                    Src::of(Idx::Const((desc >> 32) as u32)),
                    Src::NULL,
                ],
            )?;
            Ok(V::None)
        }
        Kind::Intr(Intr::BlendReturn) => {
            /* blend_return_pan: back through r48 when it is not zero */
            let link = s.preload(p, ra::BLEND_LINK)?;
            p.push(Ins::new(Op::BranchzI { eq: false }, &[], &[Src::ssa(link), Src::ssa(link)]))?;
            Ok(V::None)
        }
        Kind::Alu(op) => select_alu(s, p, i, op),
    }
}

fn select_alu(s: &mut Sel<'_, '_>, p: &mut Prog<'_>, i: u16, op: N) -> Result<V, MaliError> {
    let ins = s.nir.ins[i as usize];
    let refused = Err(MaliError::BlendVariantNotLowered);
    let src = |k: usize| s.src(i, k);
    let v2 = |p: &mut Prog<'_>, o: Op, srcs: &[Src]| alu(p, o, srcs).map(V::Word);
    if op == N::Pack32_4x8Split {
        /* the tile word of the thread's sample: (sample, rt 0, 0, 255),
         * a v4i8 `bi_make_vec_to` as `bi_lower_mkvec_swz` leaves it on
         * Valhall (bi_lower_mkvec_swz.c:189-203): the high half `MKVEC.v2i8
         * 0, 255, #0` first, then the low bytes with it */
        let k = |j: usize| {
            let d = &s.nir.ins[ins.srcs[j].def as usize];
            (d.kind == Kind::Const).then_some(d.k[0])
        };
        let V::U8(id) = s.vals[ins.srcs[0].def as usize] else { return refused };
        if (k(1), k(2), k(3)) != (Some(0), Some(0), Some(255)) {
            return refused;
        }
        let c = |v: u32| Src::of(Idx::Const(v));
        let hi = alu(p, Op::MkvecV2i8, &[c(0), c(255), c(0)])?;
        return v2(p, Op::MkvecV2i8, &[Src::ssa(id).swz(Swz::B0), c(0), Src::ssa(hi)]);
    }
    if ins.bits == 32 {
        return match op {
            /* iand: LSHIFT_AND by 0 */
            N::Iand => v2(p, Op::LshiftAndI32, &[src(0)?, src(1)?, Src::of(Idx::Const(0))]),
            _ => refused,
        };
    }
    if ins.bits == 8 {
        return match op {
            N::U2u8 => match s.vals[ins.srcs[0].def as usize] {
                V::Word(v) => Ok(V::U8(v)),
                _ => refused,
            },
            _ => refused,
        };
    }
    match (op, ins.comps) {
        (N::F2f16, n) => {
            /* the blend constants' words (bi_extract of the pushed UBO
             * load): V2F32_TO_V2F16 of a pair, or bi_f32_to_f16_to's
             * FADD.f32 to the low half of one */
            let s0 = ins.srcs[0];
            let u = |c: usize| match s.vals[s0.def as usize] {
                V::Ubo => Ok(Src::of(Idx::Uniform(s0.swz[c] as u16))),
                /* an fp32 colour's registers */
                V::Quad(r) => Ok(Src::ssa(r[s0.swz[c] as usize & 3])),
                _ => Err(MaliError::BlendVariantNotLowered),
            };
            if n == 2 {
                v2(p, Op::V2f32ToV2f16, &[u(0)?, u(1)?])
            } else {
                let t = p.temp()?;
                let d = Dest { idx: Idx::Ssa(t, 0), swz: Swz::H00 };
                p.push(Ins::new(Op::FaddF32, &[d], &[u(0)?, NEG_ZERO]))?;
                Ok(V::Word(t))
            }
        }
        (N::Fsat, _) => {
            /* a swizzled source moves out of the FCLAMP (bi_lower_swizzle.c:
             * 117-130) */
            let a = src(0)?;
            let t = p.temp()?;
            let mut x = Ins::new(Op::FclampV2f16, &[Dest::ssa(t)], &[a.swz(Swz::H01)]);
            x.clamp = clamp::ZERO_ONE;
            p.push(x)?;
            Ok(if a.swz == Swz::H01 { V::Word(t) } else { V::Swz(t, a.swz) })
        }
        (N::Fneg, _) => {
            let a = src(0)?;
            v2(p, Op::FabsnegV2f16, &[Src { neg: !a.neg, ..a }])
        }
        (N::Fadd, _) => v2(p, Op::FaddV2f16, &[src(0)?, src(1)?]),
        (N::Fmul, _) => v2(p, Op::FmaV2f16, &[src(0)?, src(1)?, NEG_ZERO]),
        (N::Ffma, _) => v2(p, Op::FmaV2f16, &[src(0)?, src(1)?, src(2)?]),
        (N::Fmin, _) => v2(p, Op::FminV2f16, &[src(0)?, src(1)?]),
        (N::Fmax, _) => v2(p, Op::FmaxV2f16, &[src(0)?, src(1)?]),
        (N::Vec4, 4) => {
            /* bi_make_vec_to: a word of each pair of halves (a pair that
             * is one value's two lanes moved whole), then COLLECT */
            let mut words = [Src::NULL; 2];
            for (wk, word) in words.iter_mut().enumerate() {
                let (a, b) = (ins.srcs[2 * wk], ins.srcs[2 * wk + 1]);
                let ha = s.half(a.def, a.swz[0])?;
                let hb = s.half(b.def, b.swz[0])?;
                *word = if let (Idx::Const(x), Idx::Const(y)) = (ha.idx, hb.idx) {
                    /* two constant halves fold (bi_opt_constant_fold), the
                     * immediate copied into the COLLECT */
                    let half = |v: u32, s: Swz| if s == Swz::H11 { v >> 16 } else { v & 0xffff };
                    Src::of(Idx::Const(half(x, ha.swz) | half(y, hb.swz) << 16))
                } else if ha.idx == hb.idx && ha.swz == Swz::H00 && hb.swz == Swz::H11 {
                    Src::ssa(alu(p, Op::MovI32, &[ha.swz(Swz::H01)])?)
                } else {
                    Src::ssa(alu(p, Op::MkvecV2i16, &[ha, hb])?)
                };
            }
            Ok(V::Collected(alu(p, Op::Collect, &words)?))
        }
        _ => refused,
    }
}

/// The passes after selection in `bi_compile_variant_nir`'s order, as for a
/// fragment shader, with the blend shader's differences: no push words, no
/// promoted immediate (`inputs.fau.promote_immediates` unset,
/// pan_blend_cso.c:135-139), no helper analysis
/// (bi_helper_invocations.c:178), the blend register allocation and flow.
fn finish<'s>(mut p: Prog<'_>, out: &'s mut [u64], scratch: &Arena<'_>) -> Result<BlendShader<'s>, MaliError> {
    opt::mod_prop_forward(&mut p);
    opt::mod_prop_backward(&mut p);
    opt::dce(&mut p);
    opt::lower_opt_instructions(&mut p);
    opt::lower_mkvec_swz(&mut p)?;
    opt::lower_isel(&mut p)?;
    opt::fuse_add_imm(&mut p);
    let mut fau = super::fau::Layout::EMPTY;
    super::fau::lower_constants(&mut p, &mut fau, false)?;
    if fau.count != 0 {
        return Err(MaliError::Defect(super::Defect::FauOverflow));
    }
    sched::pressure_schedule(&mut p, scratch)?;
    let work_reg_count = ra::allocate_blend(&mut p, scratch)?;
    opt::control_flow(&mut p);
    flow::assign_slots(&mut p);
    flow::insert_flow_control_nops(&mut p, false)?;
    flow::merge_flow(&mut p, false);
    flow::mark_last(&mut p);
    let n = pack::pack(&p, out)?;
    Ok(BlendShader { code: &out[..n], work_reg_count })
}
