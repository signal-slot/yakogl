// SPDX-License-Identifier: Apache-2.0 AND MIT
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
// SPDX-FileCopyrightText: 2019-2025 Collabora Ltd.
// SPDX-FileCopyrightText: 2025 Google LLC
// Derived from Mesa 26.2.1 src/panfrost/lib/pan_blend.c
// (GENX(pan_blend_create_shader), pan_blend_get_internal_desc),
// src/compiler/nir/nir_lower_blend.c (nir_color_blend),
// src/panfrost/compiler/pan_nir.h (pan_nir_tile_rt_sample) and
// src/gallium/drivers/panfrost/pan_blend_cso.c (MIT). The upstream part stays
// under its licence (LICENSES/MIT.txt); our changes are Apache-2.0. See NOTICE.
//! The blend shader: the program the fragment shader's `BRANCHZI` calls
//! when the draw's blend descriptor is in Shader mode (Mesa's
//! `panfrost_get_blend`, pan_context.c:163-229), for the equations a
//! reference draws: a GL blend that reads `CONSTANT_COLOR` (or its inverse)
//! whose channels differ, which the one constant of a fixed-function
//! descriptor cannot hold (`pan_blend_is_homogenous_constant`; gl_test case
//! 8's fifth frame, `CONSTANT_COLOR, ONE_MINUS_CONSTANT_COLOR` with
//! glBlendColor 0.25, 0.5, 0.75, 1.0, and case 211 the same at 4x), and two
//! equations the fixed-function unit cannot run (case 190's GL_MIN and
//! ONE_MINUS_DST_ALPHA columns; docs/notes/mali-blend-shaders.md).
//!
//! Mesa builds it as NIR (`pan_blend_create_shader`, pan_blend.c:797-946):
//! the source colour from r0-r1 (fp16, the fragment shader's `BLEND`
//! staging registers), the destination from the tile buffer (`load_tile`
//! with the pixel indices of the current pixel and sample, the coverage's
//! low five bits and the conversion word of the internal descriptor),
//! `nir_color_blend` (nir_lower_blend.c:910-990: the constants converted to
//! fp16, source and constants clamped to [0, 1] for a UNORM target, then
//! `src * f_src + dst * f_dst` or `fmin(src, dst)` per channel), the
//! `BLEND` of the result with the opaque internal descriptor as an
//! immediate, and the return through the link register r48
//! (`BRANCHZI.ne`). The constants are a load of the sysval UBO's first four
//! words (`pan_lower_blend_constants`, pan_blend_cso.c:98-114), which every
//! fragment shader pushes as its first four FAU words
//! (`bi_pick_blend_constants`): the blend shader runs in the fragment
//! shader's thread and reads them there as `u0`, `u1`. At 4x the thread
//! runs one sample (the draw's `Evaluate per-sample`), whose id r61 holds:
//! the tile word names it (pan_blend.c:880-883).
//!
//! The NIR is the vectorised one Mesa's passes leave (16-bit pairs, the
//! factor ONE and ZERO folded away, a product added to a term fused into
//! an `ffma`), selected here in its order, then run through this back end's
//! passes with Mesa's blend-shader rules (`inputs->is_blend`): no pushed
//! words and no promoted immediates, r0-r15 only and no precoloured BLEND
//! (ra.rs `allocate_blend`), no wait before the BLEND or the LD_TILE
//! (flow.rs with `fragment` false). The bytes are the shim driver's
//! (tools/mali/testdata/blend_*.expect.json and blend_case*.words;
//! tests.rs).

use super::bir::{clamp, Dest, Idx, Ins, Op, Prog, Src, Swz};
use super::select::{alu, split};
use super::{flow, opt, pack, ra, sched, MaliError};
use crate::shaderc::online::arena::Arena;

/// A blend factor of the equations built here (`pipe_blendfactor`): ZERO,
/// ONE, the constant colour (`CONSTANT_COLOR`) or its inverse, the source's
/// alpha or its inverse, the destination's alpha inverted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Factor {
    Zero,
    One,
    Const,
    InvConst,
    SrcAlpha,
    InvSrcAlpha,
    InvDstAlpha,
}

impl Factor {
    fn is_const(self) -> bool {
        matches!(self, Factor::Const | Factor::InvConst)
    }
}

/// `pipe_blend_func`, the functions built here: `src * f + dst * g`, or
/// `fmin(src, dst)` with no factor (`nir_blend_func`, nir_lower_blend.c:
/// 51-70; `nir_blend_factored` is false for MIN).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Func {
    Add,
    Min,
}

/// One channel group's equation (the colour's or the alpha's).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Channel {
    pub func: Func,
    pub src: Factor,
    pub dst: Factor,
}

impl Channel {
    /// `src * src_factor + dst * dst_factor` (GL_FUNC_ADD).
    pub const fn add(src: Factor, dst: Factor) -> Channel {
        Channel { func: Func::Add, src, dst }
    }

    /// GL_MIN. Its factors are not read; they are ONE, ONE (what st/mesa
    /// hands on, st_atom_blend.c), so that one equation is one key.
    pub const MIN: Channel = Channel { func: Func::Min, src: Factor::One, dst: Factor::One };
}

/// The equation of a blend shader built here, Mesa's cache key without the
/// constants (`pan_blend_get_shader_locked`, G/pan_blend_cso.c:59-98): the
/// colour's and the alpha's equations and the target's sample count, all
/// four channels written into an RGBA8 UNORM target, the fragment shader's
/// colour in fp16 (the caller's part: src/mali/blend.rs `shader_equation`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Equation {
    pub rgb: Channel,
    pub alpha: Channel,
    /// 1, or 4: the tile is read at the thread's sample
    /// (pan_blend.c:880-883).
    pub samples: u8,
}

impl Equation {
    /// glBlendFunc(src, dst) with GL_FUNC_ADD into a single-sampled target.
    pub const fn both(src: Factor, dst: Factor) -> Equation {
        Equation { rgb: Channel::add(src, dst), alpha: Channel::add(src, dst), samples: 1 }
    }

    /// The constant colour's equations (gl_test case 8): one equation on
    /// colour and alpha, ADD, the factors ZERO, ONE or the constant's, one
    /// of them the constant's; not both the same constant term (NIR
    /// factors `s * c + d * c` otherwise; no reference draws it).
    fn constant_shape(self) -> bool {
        let c = self.rgb;
        let plain = |f: Factor| matches!(f, Factor::Zero | Factor::One | Factor::Const | Factor::InvConst);
        self.rgb == self.alpha
            && c.func == Func::Add
            && plain(c.src)
            && plain(c.dst)
            && (c.src.is_const() || c.dst.is_const())
            && c.src != c.dst
    }

    /// The equations whose colour and alpha differ that a reference draws,
    /// single-sampled: gl_test case 190's columns 3 (GL_MIN colour, ADD
    /// SRC_ALPHA, ONE_MINUS_SRC_ALPHA alpha) and 4 (ADD ONE_MINUS_DST_ALPHA,
    /// ONE_MINUS_SRC_ALPHA colour, ADD ONE, ZERO alpha;
    /// tools/mali/testdata/blend_case190_col*.words).
    fn split_shape(self) -> bool {
        use Factor::*;
        const COL3: (Channel, Channel) = (Channel::MIN, Channel::add(SrcAlpha, InvSrcAlpha));
        const COL4: (Channel, Channel) = (Channel::add(InvDstAlpha, InvSrcAlpha), Channel::add(One, Zero));
        self.samples == 1 && ((self.rgb, self.alpha) == COL3 || (self.rgb, self.alpha) == COL4)
    }
}

/// `pan_nir_tile_rt_sample(rt 0, sample 0)` (pan_nir.h:19-26): sample 0,
/// target 0, and y = 255, "the current pixel".
const PIXEL_INDICES: u32 = 0xff00_0000;

/// `pan_nir_tile_default_coverage`'s mask (pan_nir.h:46-49), and the sample
/// id's (`bi_load_sample_id_to`: "a 5-bit mask instead of 8-bits").
const COVERAGE_MASK: u32 = 0x1f;

/// `BI_PRELOAD_SAMPLE_ID`: r61, the sample id in bits 16-23
/// (bifrost_compile.c:887-896).
const SAMPLE_ID_REG: u8 = 61;
const SAMPLE_ID_SHIFT: u32 = 16;

/// The opaque internal descriptor's low word (`pan_blend_get_internal_desc`,
/// pan_blend.c:975-1001, v10): mode Opaque (1), four components (bits 3-4,
/// minus one), target 0. Its high word is the target's memory format
/// (`pan_dithered_format_from_pipe_format(fmt, false)`), the caller's.
const INTERNAL_OPAQUE_RGBA: u32 = 1 | (4 - 1) << 3;

/// fp16 1.0 in both halves (`nir_imm_floatN_t(1.0, 16)` read `.xx`).
const F16_ONE_ONE: u32 = 0x3c00_3c00;

/// `-0.0`, the addend of a product alone (an `fmul` as an `FMA`).
const NEG_ZERO: Src = Src { neg: true, ..Src::of(Idx::Const(0)) };

/// The blend shader's words (padded as every program is) and its work
/// register count.
pub struct BlendShader<'s> {
    pub code: &'s [u64],
    pub work_reg_count: u8,
}

/// Whether [`shader`] builds `eq`: the constant colour's equations at 1x,
/// at 4x the one a reference draws (case 211's `CONSTANT_COLOR,
/// ONE_MINUS_CONSTANT_COLOR`), and case 190's two columns at 1x. Any other
/// is refused, never approximated.
pub fn builds(eq: Equation) -> bool {
    match eq.samples {
        1 => eq.constant_shape() || eq.split_shape(),
        4 => eq.constant_shape() && eq.rgb == Channel::add(Factor::Const, Factor::InvConst),
        _ => false,
    }
}

/// Build and compile the blend shader of `eq` for a target whose memory
/// format (the internal descriptor's conversion word) is `conversion`,
/// into `out`.
pub fn shader<'s>(
    out: &'s mut [u64],
    scratch: &Arena<'_>,
    eq: Equation,
    conversion: u32,
) -> Result<BlendShader<'s>, MaliError> {
    if !builds(eq) {
        return Err(MaliError::BlendVariantNotLowered);
    }
    let ins = scratch.alloc_filled(super::bir::MAX_INSTRS, Ins::NOP).map_err(|_| MaliError::OutOfScratch)?;
    let mut p = Prog::new(ins);
    /* NIR drops a source or a destination no term reads (a ZERO factor);
     * MIN and the alpha factors read both */
    let constant = eq.constant_shape();
    let (src_used, dst_used) =
        if constant { (eq.rgb.src != Factor::Zero, eq.rgb.dst != Factor::Zero) } else { (true, true) };
    /* the preloaded registers, read once each through a MOV at the start
     * (`bi_preload`), in the reverse order of their first read in the NIR:
     * the source colour (load_blend_input, r0 then r1), the sample id
     * (load_sample_id, r61), the coverage (load_cumulative_coverage, r60),
     * the link register (blend_return) */
    let mov = |p: &mut Prog<'_>, r: u8| alu(p, Op::MovI32, &[Src::of(Idx::Reg(r))]);
    let link = mov(&mut p, super::ra::BLEND_LINK)?;
    let coverage = mov(&mut p, super::ra::CUMULATIVE_COVERAGE)?;
    let sample = if eq.samples > 1 && dst_used { Some(mov(&mut p, SAMPLE_ID_REG)?) } else { None };
    let src = if src_used {
        let r1 = mov(&mut p, 1)?;
        let r0 = mov(&mut p, 0)?;
        Some([r0, r1])
    } else {
        None
    };
    /* the destination: LD_TILE of the current pixel and sample, its
     * coverage's low bits (an `iand`, `LSHIFT_AND` by 0), the conversion
     * word */
    let dst = if dst_used {
        let pixel = match sample {
            None => Src::of(Idx::Const(PIXEL_INDICES)),
            Some(r61) => Src::ssa(tile_word(&mut p, r61)?),
        };
        let cov = alu(
            &mut p,
            Op::LshiftAndI32,
            &[Src::ssa(coverage), Src::of(Idx::Const(COVERAGE_MASK)), Src::of(Idx::Const(0))],
        )?;
        let t = alu(
            &mut p,
            Op::LdTile { vecsize: 4, f16: true },
            &[pixel, Src::ssa(cov), Src::of(Idx::Const(conversion))],
        )?;
        let d = split(&mut p, t, 2)?;
        Some([d[0], d[1]])
    } else {
        None
    };
    let col = match (constant, src, dst) {
        (true, ..) => constant_body(&mut p, eq.rgb, src, dst)?,
        (false, Some(src), Some(dst)) => split_body(&mut p, eq, src, dst)?,
        _ => return Err(MaliError::BlendVariantNotLowered),
    };
    alu(
        &mut p,
        Op::Blend { sr_count: 2, f32: false },
        &[
            Src::ssa(col),
            Src::ssa(coverage),
            Src::of(Idx::Const(INTERNAL_OPAQUE_RGBA)),
            Src::of(Idx::Const(conversion)),
            Src::NULL,
        ],
    )?;
    /* `blend_return_pan`: back to the fragment shader, through r48 when it
     * is not zero */
    p.push(Ins::new(Op::BranchzI { eq: false }, &[], &[Src::ssa(link), Src::ssa(link)]))?;
    finish(p, out, scratch)
}

/// The tile word of the thread's sample: `pan_nir_tile_rt_sample(rt 0,
/// load_sample_id)`, the bytes `(sample, rt 0, 0, 255)`
/// (`nir_pack_32_4x8_split`), the sample id `(r61 >> 16) & 0x1f`
/// (`bi_load_sample_id_to`). The four bytes are what `bi_lower_mkvec_swz`
/// makes of a v4i8 on Valhall (bi_lower_mkvec_swz.c:189-203): the high half
/// `MKVEC.v2i8 0, 255, #0` first, then the low bytes with it.
fn tile_word(p: &mut Prog<'_>, r61: u16) -> Result<u16, MaliError> {
    let c = |v: u32| Src::of(Idx::Const(v));
    let id = alu(p, Op::RshiftAndI32, &[Src::ssa(r61), c(COVERAGE_MASK), c(SAMPLE_ID_SHIFT)])?;
    let hi = alu(p, Op::MkvecV2i8, &[c(0), c(255), c(0)])?;
    alu(p, Op::MkvecV2i8, &[Src::ssa(id).swz(Swz::B0), c(0), Src::ssa(hi)])
}

/// `fsat` (an `FCLAMP` to [0, 1]): of the source and of the constants for a
/// UNORM target (`nir_fsat_to_format`, nir_lower_blend.c:943-947).
fn sat(p: &mut Prog<'_>, v: u16) -> Result<u16, MaliError> {
    let t = p.temp()?;
    let mut i = Ins::new(Op::FclampV2f16, &[Dest::ssa(t)], &[Src::ssa(v)]);
    i.clamp = clamp::ZERO_ONE;
    p.push(i)?;
    Ok(t)
}

fn fma(p: &mut Prog<'_>, a: Src, b: Src, c: Src) -> Result<u16, MaliError> {
    alu(p, Op::FmaV2f16, &[a, b, c])
}

/// `1 - x`: `fadd(fneg(x), 1.0)` (`nir_blend_factor`).
fn one_minus(p: &mut Prog<'_>, x: Src) -> Result<u16, MaliError> {
    let n = alu(p, Op::FabsnegV2f16, &[Src { neg: true, ..x }])?;
    alu(p, Op::FaddV2f16, &[Src::ssa(n), Src::of(Idx::Const(F16_ONE_ONE))])
}

/// `src * f + dst * g` of one pair or lane (`nir_blend_factor`, then
/// `nir_blend_func`'s `fadd`): the source's product fused with the
/// destination's term (`dst` itself for ONE) as its addend, a ZERO or ONE
/// factor dropping its multiply; ONE, ZERO is the source as it is. `f` and
/// `g` are the factors' values (None for ZERO and ONE), `s` and `d` None
/// when not read; a lane (`d` swizzled) is a 16-bit scalar.
fn add_terms(
    p: &mut Prog<'_>,
    c: Channel,
    s: Option<Src>,
    d: Option<Src>,
    f: Option<Src>,
    g: Option<Src>,
) -> Result<Src, MaliError> {
    use Factor::{One, Zero};
    let missing = MaliError::Defect(super::Defect::Operand);
    let (s, d, f, g) = (|| s.ok_or(missing), || d.ok_or(missing), || f.ok_or(missing), || g.ok_or(missing));
    let v = match (c.src, c.dst) {
        (One, Zero) => return s(),
        (Zero | One, Zero | One) => return Err(MaliError::BlendVariantNotLowered),
        (_, One) => fma(p, s()?, f()?, d()?)?,
        (_, Zero) => fma(p, s()?, f()?, NEG_ZERO)?,
        (One, _) => fma(p, d()?, g()?, s()?)?,
        (Zero, _) => fma(p, d()?, g()?, NEG_ZERO)?,
        _ => {
            /* a lane's product is a scalar: read as its low half */
            let lane = d()?.swz != Swz::H01;
            let dt = Src::ssa(fma(p, d()?, g()?, NEG_ZERO)?);
            fma(p, s()?, f()?, if lane { dt.swz(Swz::H00) } else { dt })?
        }
    };
    Ok(Src::ssa(v))
}

/// The constant colour's equations ([`Equation::constant_shape`]): both
/// pairs (rg, ba) under the one equation `c`, the constants the pushed
/// fp32 words 0-3 as two fp16 pairs. Returns the vec4 of the two results.
fn constant_body(p: &mut Prog<'_>, c: Channel, src: Option<[u16; 2]>, dst: Option<[u16; 2]>) -> Result<u16, MaliError> {
    let mut c32 = [0u16; 2];
    for (k, x) in c32.iter_mut().enumerate() {
        let w = 2 * k as u16;
        *x = alu(p, Op::V2f32ToV2f16, &[Src::of(Idx::Uniform(w)), Src::of(Idx::Uniform(w + 1))])?;
    }
    /* `fsat` of the source, then of the constants */
    let s = match src {
        Some([r0, r1]) => Some([sat(p, r0)?, sat(p, r1)?]),
        None => None,
    };
    /* the constant itself clamped, when a factor reads it; when only its
     * inverse does, NIR's `1 - fsat(a)` rule (a used once) clamps the
     * inverse instead: `fsat(1.0 + -a)` (measured: blend_ic_one) */
    let const_used = c.src == Factor::Const || c.dst == Factor::Const;
    let k32 = if const_used { [sat(p, c32[0])?, sat(p, c32[1])?] } else { c32 };
    let mut result = [0u16; 2];
    for k in 0..2 {
        /* 1 - c, made once a pair */
        let inv = if c.src == Factor::InvConst || c.dst == Factor::InvConst {
            Some(if const_used {
                one_minus(p, Src::ssa(k32[k]))?
            } else {
                let n = alu(p, Op::FabsnegV2f16, &[Src { neg: true, ..Src::ssa(k32[k]) }])?;
                let a = alu(p, Op::FaddV2f16, &[Src::of(Idx::Const(F16_ONE_ONE)), Src::ssa(n)])?;
                sat(p, a)?
            })
        } else {
            None
        };
        let value = |f: Factor| match f {
            Factor::Const => Some(Src::ssa(k32[k])),
            Factor::InvConst => inv.map(Src::ssa),
            _ => None,
        };
        let (sk, dk) = (s.map(|s| Src::ssa(s[k])), dst.map(|d| Src::ssa(d[k])));
        let r = add_terms(p, c, sk, dk, value(c.src), value(c.dst))?;
        result[k] = r.ssa_value().ok_or(MaliError::Defect(super::Defect::Operand))?;
    }
    /* the vec4 of the two pairs (`bi_make_vec_to`: a word moved whole) */
    let w0 = alu(p, Op::MovI32, &[Src::ssa(result[0])])?;
    let w1 = alu(p, Op::MovI32, &[Src::ssa(result[1])])?;
    alu(p, Op::Collect, &[Src::ssa(w0), Src::ssa(w1)])
}

/// An equation whose colour and alpha differ ([`Equation::split_shape`]).
/// NIR's vectorisation leaves the colour's red and green one fp16 pair, and
/// blue and alpha one lane each of the second pair (`.h00`, `.h11`: a 16-bit
/// scalar op, replicated in both halves); a factor that is one value for
/// several channels (`1 - As`, `1 - Ad`) is made once; MIN is `fmin` of the
/// saturated source and the destination as it is. Returns the vec4: the
/// red-green word moved whole, then `MKVEC.v2i16` of blue and alpha
/// (tools/mali/testdata/blend_case190_col*.txt).
fn split_body(p: &mut Prog<'_>, eq: Equation, src: [u16; 2], dst: [u16; 2]) -> Result<u16, MaliError> {
    let s = [sat(p, src[0])?, sat(p, src[1])?];
    let lane = |v: u16, hi: bool| Src::ssa(v).swz(if hi { Swz::H11 } else { Swz::H00 });
    let reads = |f: Factor| [eq.rgb, eq.alpha].iter().any(|c| c.func == Func::Add && (c.src == f || c.dst == f));
    let inv_sa = if reads(Factor::InvSrcAlpha) { Some(one_minus(p, lane(s[1], true))?) } else { None };
    let inv_da = if reads(Factor::InvDstAlpha) { Some(one_minus(p, lane(dst[1], true))?) } else { None };
    let value = |f: Factor| match f {
        Factor::SrcAlpha => Some(lane(s[1], true)),
        Factor::InvSrcAlpha => inv_sa.map(|v| lane(v, false)),
        Factor::InvDstAlpha => inv_da.map(|v| lane(v, false)),
        _ => None,
    };
    let channel = |p: &mut Prog<'_>, c: Channel, s: Src, d: Src| -> Result<Src, MaliError> {
        match c.func {
            Func::Min => Ok(Src::ssa(alu(p, Op::FminV2f16, &[s, d])?)),
            Func::Add => add_terms(p, c, Some(s), Some(d), value(c.src), value(c.dst)),
        }
    };
    let rg = channel(p, eq.rgb, Src::ssa(s[0]), Src::ssa(dst[0]))?;
    let b = channel(p, eq.rgb, lane(s[1], false), lane(dst[1], false))?;
    let a = channel(p, eq.alpha, lane(s[1], true), lane(dst[1], true))?;
    /* a lane of `MKVEC.v2i16` is a half: a pair's result its low one */
    let half = |x: Src| if x.swz == Swz::H01 { x.swz(Swz::H00) } else { x };
    let w0 = alu(p, Op::MovI32, &[rg])?;
    let w1 = alu(p, Op::MkvecV2i16, &[half(b), half(a)])?;
    alu(p, Op::Collect, &[Src::ssa(w0), Src::ssa(w1)])
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
