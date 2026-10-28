// SPDX-License-Identifier: Apache-2.0 AND MIT
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
// SPDX-FileCopyrightText: 2018-2022 Alyssa Rosenzweig, Collabora Ltd.
// Derived from Mesa 26.2.1 src/panfrost/compiler/bifrost/bi_opt_mod_props.c,
// bi_opt_dce.c, bi_opt_cse.c and valhall/va_optimize.c (MIT). The upstream
// part stays under its licence (LICENSES/MIT.txt); our changes are
// Apache-2.0. See NOTICE.
//! The BIR passes Mesa runs between selection and the Valhall lowering
//! (bifrost_compile.c `bi_compile_variant_nir`), in its order:
//!
//! * [`mod_prop_forward`]: an fneg or fabs is selected as the pseudo-op
//!   `FABSNEG` (`bi_emit_alu`, `bi_fabsneg_to`), folded into each user that
//!   takes the modifier;
//! * [`mod_prop_backward`]: an `FCLAMP` (an `fsat`) folds into its
//!   single-use producer's `.clamp`, and a compare whose one use is the
//!   `MUX` of a `b2f` takes the `.f1` result type;
//! * [`dce`] then removes the ones every user took;
//! * [`lower_opt_instructions`]: an `FABSNEG` left is `FADD x.mods, -0.0`,
//!   an `FCLAMP` left `FADD x, -0.0 .clamp`;
//! * [`lower_mkvec_swz`]: `MKVEC.v2i16 x, #0` is `U16_TO_U32 x`;
//! * [`lower_isel`] (`va_lower_isel`): an unswizzled `MUX` is a `CSEL`;
//! * [`fuse_add_imm`] (`va_optimize`): an FADD with a constant and a plain
//!   other source is `FADD_IMM`;
//! * [`fuse_cmp`] (`va_optimize_forward`): the OR of two compares is one
//!   compare with the other as its third source;
//! * [`cse`] and [`dce`] again after the constant lowering (fau.rs).
//!
//! Which source takes which modifier is the Bifrost opcode table's
//! (`bi_get_opcode_props`, generated from bifrost/ISA.xml, which Valhall
//! reads too), with `bi_takes_fabs`' and `bi_takes_fneg`'s exceptions.

use super::bir::{self, clamp, Idx, Ins, Op, Prog, ResultType, Src, Swz};
use super::MaliError;

/// `bi_get_opcode_props(op)->size` (bifrost_isa.py `typesize`: the last
/// two characters of the name, 32 when they are no number).
const fn size(op: Op) -> u8 {
    match op {
        Op::FaddV2f16
        | Op::FmaV2f16
        | Op::FmaxV2f16
        | Op::FminV2f16
        | Op::FabsnegV2f16
        | Op::FclampV2f16
        | Op::V2f32ToV2f16
        | Op::MkvecV2i16
        | Op::IaddV2u16
        | Op::MuxV2i16
        | Op::FroundV2f16 { .. }
        | Op::FcmpOrV2f16 { .. }
        | Op::IcmpOrV2u16 { .. } => 16,
        Op::ShaddxS64 => 64,
        _ => 32,
    }
}

/// The `abs<s>` and `neg<s>` modifiers of the op's Bifrost entry
/// (bifrost/ISA.xml; the pseudo-ops' in IR_pseudo.xml:116-135), one bit
/// per source.
const fn mods(op: Op) -> (u8, u8) {
    match op {
        Op::FaddF32
        | Op::FaddV2f16
        | Op::FmaxF32
        | Op::FminF32
        | Op::FmaxV2f16
        | Op::FminV2f16
        | Op::V2f32ToV2f16
        | Op::FcmpOrF32 { .. }
        | Op::FcmpOrV2f16 { .. } => (0b11, 0b11),
        Op::FmaF32 => (0b111, 0b111),
        Op::FmaV2f16 => (0, 0b111),
        /* "`.abs` and `.neg` modifiers allowed on Valhall DISCARD"
         * (bi_opt_mod_props.c:122-127): the fused compare's modifiers
         * travel with its sources */
        Op::DiscardF32 { .. } => (0b11, 0b11),
        /* the cube transform's coordinates (valhall/ISA.xml:3158-3216:
         * `absneg` on each coordinate, none on the face index) */
        Op::Cubeface1 | Op::Cubeface2 => (0b111, 0b111),
        Op::CubeSsel | Op::CubeTsel => (0b11, 0b11),
        Op::FrcpF32
        | Op::FrsqF32
        | Op::FroundF32 { .. }
        | Op::FroundV2f16 { .. }
        | Op::F32ToS32
        | Op::FabsnegF32
        | Op::FabsnegV2f16 => (0b1, 0b1),
        _ => (0, 0),
    }
}

/// `bi_opcode_props[op].clamp` (the ops here whose entry has a clamp
/// modifier): what `bi_takes_clamp` reads for them. `V2F32_TO_V2F16`'s
/// entry has one (bi_opcodes.c: a blend shader's `fsat(f2f16(c))`).
const fn takes_clamp(op: Op) -> bool {
    matches!(
        op,
        Op::FaddF32
            | Op::FmaF32
            | Op::FminF32
            | Op::FmaxF32
            | Op::FaddV2f16
            | Op::FmaV2f16
            | Op::FmaxV2f16
            | Op::FminV2f16
            | Op::V2f32ToV2f16
    )
}

/// `bi_takes_fabs` (bi_opt_mod_props.c:22-46) on Valhall
/// (`bi_would_impact_abs` is false past arch 8; no op here clamps).
const fn takes_fabs(op: Op, s: usize) -> bool {
    match op {
        /* TODO in Mesa: "Needs both match or lower" */
        Op::V2f32ToV2f16 => false,
        _ => mods(op).0 & (1 << s) != 0,
    }
}

/// `bi_takes_fneg` (bi_opt_mod_props.c:48-63) for the ops here.
const fn takes_fneg(op: Op, s: usize) -> bool {
    mods(op).1 & (1 << s) != 0
}

/// `bi_is_fabsneg`: the pseudo-op of the user's size.
const fn is_fabsneg(op: Op, size: u8) -> bool {
    matches!((op, size), (Op::FabsnegF32, 32) | (Op::FabsnegV2f16, 16))
}

/// The half each lane of a 16-bit swizzle reads: (low lane, high lane),
/// `true` for the high half (`bi_half`: H00 | x << 1 | y).
const fn halves(s: Swz) -> (bool, bool) {
    match s {
        Swz::H00 => (false, false),
        /* a byte lane exists only after the constants' lowering, past
         * every composition: read as the identity */
        Swz::H01 | Swz::B0 | Swz::B1 | Swz::B2 | Swz::B3 => (false, true),
        Swz::H10 => (true, false),
        Swz::H11 => (true, true),
    }
}

const fn swz_of(lo: bool, hi: bool) -> Swz {
    match (lo, hi) {
        (false, false) => Swz::H00,
        (false, true) => Swz::H01,
        (true, false) => Swz::H10,
        (true, true) => Swz::H11,
    }
}

/// `bi_compose_float_index`: `old` reads `repl`'s value through its own
/// modifiers and swizzle.
fn compose(old: Src, repl: Src) -> Src {
    let (al, ar) = halves(old.swz);
    let (bl, br) = halves(repl.swz);
    Src {
        /* abs(-x) is abs(x); otherwise the negations cancel */
        neg: old.neg ^ (repl.neg && !old.abs),
        abs: repl.abs | old.abs,
        swz: swz_of(if al { br } else { bl }, if ar { br } else { bl }),
        ..repl
    }
}

/// `bi_opt_mod_prop_forward` (bi_opt_mod_props.c:180-225): in program
/// order, a `DISCARD.b32` whose mask an `FCMP.f32` or `FCMP.v2f16` defines becomes
/// `DISCARD.f32` of the compare's two sources and condition
/// (`bi_fuse_discard_fcmp`, :111-134, "the only optimization DISCARD is
/// involved in", :190-198; the compare stays for `dce` to take when nothing
/// else reads it), and otherwise a source an `FABSNEG` of the user's size
/// defines reads the `FABSNEG`'s own source, with its modifiers composed,
/// when the user takes them.
pub fn mod_prop_forward(p: &mut Prog<'_>) {
    for k in 0..p.n {
        let user = p.ins[k];
        if user.op == Op::DiscardB32 {
            let Idx::Ssa(v, 0) = user.src[0].idx else { continue };
            let def = p.as_slice()[..k].iter().rev().find(|i| i.dests().iter().any(|d| d.idx == Idx::Ssa(v, 0)));
            /* every condition here is below `GTLT` (:118-119) */
            if let Some(&Ins { op: op @ (Op::FcmpOrF32 { cmpf, .. } | Op::FcmpOrV2f16 { cmpf, .. }), src, .. }) = def {
                let mut fused = Ins { op: Op::DiscardF32 { cmpf }, ..user };
                fused.src[..2].copy_from_slice(&src[..2]);
                /* a 16-bit compare's lane: the mask's swizzle composed into
                 * each source's (`bi_compose_swizzle_16`, :136-139) */
                if matches!(op, Op::FcmpOrV2f16 { .. }) {
                    let r = user.src[0].swz;
                    for x in &mut fused.src[..2] {
                        x.swz = compose(Src { swz: r, ..Src::NULL }, *x).swz;
                    }
                }
                fused.ns = 2;
                p.ins[k] = fused;
            }
            continue;
        }
        for s in 0..user.ns as usize {
            let Idx::Ssa(v, 0) = user.src[s].idx else { continue };
            /* the definition: SSA, so the one earlier instruction writing v */
            let def = p.as_slice()[..k].iter().rev().find(|i| i.dests().iter().any(|d| d.idx == Idx::Ssa(v, 0)));
            let Some(m) = def else { continue };
            if !is_fabsneg(m.op, size(user.op)) {
                continue;
            }
            let repl = m.src[0];
            if repl.abs && !takes_fabs(user.op, s) {
                continue;
            }
            if repl.neg && !takes_fneg(user.op, s) {
                continue;
            }
            p.ins[k].src[s] = compose(user.src[s], repl);
        }
    }
}

/// The one instruction reading SSA value `v` (written at position `k`),
/// or None when none or more than one does (`bi_record_use`'s
/// `multiple`). A loop header's phi reads a value written after it.
fn single_use(p: &Prog<'_>, k: usize, v: u16) -> Option<usize> {
    let mut found = None;
    for (j, i) in p.as_slice().iter().enumerate().filter(|&(j, _)| j != k) {
        if i.srcs().iter().any(|s| s.ssa_value() == Some(v)) {
            if found.is_some() {
                return None;
            }
            found = Some(j);
        }
    }
    found
}

/// A constant source of the value `v` (`bi_is_value_equiv` of a constant:
/// the value, whatever the swizzle).
fn is_const(s: &Src, v: u32) -> bool {
    s.idx == Idx::Const(v)
}

/// `bi_opt_mod_prop_backward` (bi_opt_mod_props.c:407-477) for the
/// pseudo-ops here, in reverse program order over each instruction with
/// one destination and a single use:
///
/// * `bi_optimizer_clamp` (:259-270): an `FCLAMP` of the instruction's
///   size whose producer takes a clamp composes its clamp into the
///   producer (`|=`: the clamps are bit fields) and takes its destination;
/// * `bi_optimizer_result_type` (:324-369): a `MUX.i32` between 0 and 1.0
///   on the instruction's result (`bi_is_fixed_mux`: a `b2f`'s `bcsel`)
///   folds into an `FCMP.f32`'s `.f1` result type. Mesa does the same for
///   a 16-bit compare and `MUX.v2i16` (with a swizzled mask composed into
///   the compare's sources); this port's compares are 32-bit only
///   (`FcmpOrF32`), so only the 32-bit pair is matched here.
///
/// The use is removed in both cases.
pub fn mod_prop_backward(p: &mut Prog<'_>) {
    let mut k = p.n;
    while k > 0 {
        k -= 1;
        let i = p.ins[k];
        if i.nd != 1 {
            continue;
        }
        let Idx::Ssa(v, 0) = i.dest[0].idx else { continue };
        let Some(u) = single_use(p, k, v) else { continue };
        let use_ = p.ins[u];
        let sz = size(i.op);
        /* `bi_is_fclamp(use, size)`: the FCLAMP of the producer's size */
        let fclamp = (use_.op == Op::FclampF32 && sz == 32) || (use_.op == Op::FclampV2f16 && sz == 16);
        let fused = if fclamp && takes_clamp(i.op) {
            p.ins[k].clamp |= use_.clamp;
            true
        } else if use_.op == Op::MuxI32
            && sz == 32
            && is_const(&use_.src[0], 0)
            && is_const(&use_.src[1], 0x3f80_0000)
            && matches!(i.op, Op::FcmpOrF32 { .. })
        {
            /* a mask read through a swizzle composes into the sources of
             * a 16-bit compare only, and there is none here: left as is */
            if use_.src[2].swz != Swz::H01 {
                continue;
            }
            if let Op::FcmpOrF32 { cmpf, .. } = p.ins[k].op {
                p.ins[k].op = Op::FcmpOrF32 { cmpf, result: ResultType::F1 };
            }
            true
        } else if use_.op == Op::MuxI32
            && sz == 32
            && is_const(&use_.src[0], 0)
            && is_const(&use_.src[1], 1)
            && use_.src[2].swz == Swz::H01
            && matches!(i.op, Op::FcmpOrF32 { .. } | Op::IcmpOrS32 { .. } | Op::IcmpOrU32 { .. })
        {
            /* a `b2i32`'s `bcsel` folds into the compare's `.i1` result
             * type (`bi_takes_int_result_type`; intbig_case79.expect.json's
             * `ICMP_OR.u32.ge.i1` of `nir_lower_idiv`'s second correction) */
            p.ins[k].op = match p.ins[k].op {
                Op::FcmpOrF32 { cmpf, .. } => Op::FcmpOrF32 { cmpf, result: ResultType::I1 },
                Op::IcmpOrS32 { cmpf, .. } => Op::IcmpOrS32 { cmpf, result: ResultType::I1 },
                Op::IcmpOrU32 { cmpf, .. } => Op::IcmpOrU32 { cmpf, result: ResultType::I1 },
                op => op,
            };
            true
        } else {
            false
        };
        if fused {
            p.ins[k].dest[0] = use_.dest[0];
            p.remove(u);
        }
    }
}

/// `bi_lower_opt_instructions` (bi_opt_mod_props.c:481-540) for the
/// pseudo-ops here: `FABSNEG` is `FADD x, -0.0` (`bi_negzero()`: the zero
/// with `.neg`), `FCLAMP` the same FADD with its clamp kept (:493-501), a
/// `DISCARD.b32` left unfused is `DISCARD.f32 mask, 0 .ne` (:503-516; the
/// zero read as a half when the mask is).
pub fn lower_opt_instructions(p: &mut Prog<'_>) {
    for i in p.ins[..p.n].iter_mut() {
        if i.op == Op::DiscardB32 {
            let zero = match i.src[0].swz {
                Swz::H01 => Src::of(Idx::Const(0)),
                _ => Src::of(Idx::Const(0)).swz(Swz::H00),
            };
            let op = Op::DiscardF32 { cmpf: bir::Cmpf::Ne };
            *i = Ins { op, flow: i.flow, block: i.block, ..Ins::new(op, &[], &[i.src[0], zero]) };
            continue;
        }
        let op = match i.op {
            Op::FabsnegF32 | Op::FclampF32 => Op::FaddF32,
            Op::FabsnegV2f16 | Op::FclampV2f16 => Op::FaddV2f16,
            _ => continue,
        };
        let neg_zero = Src { neg: true, ..Src::of(Idx::Const(0)) };
        *i = Ins { op, clamp: i.clamp, flow: i.flow, block: i.block, ..Ins::new(op, i.dests(), &[i.src[0], neg_zero]) };
    }
}

/// `bi_lower_mkvec_swz` (bi_lower_mkvec_swz.c:242-260) for the form here:
/// `MKVEC.v2i16 x, #0` is `U16_TO_U32 x` (the zero half is the
/// extension); a `MKVEC` of two halves of one word is that word swizzled,
/// `IADD.v2u16 x.hNN, 0` (`build_swz_v2i16_to` on Valhall, :92-102; the
/// two halves in order a `MOV.i32`). varyconst_185_kindcost_fmul.expect.json:
/// `IADD.v2u16 r1, r0.h00, 0x0` for the word `(x, x)`.
pub fn lower_mkvec_swz(p: &mut Prog<'_>) -> Result<(), MaliError> {
    for i in p.ins[..p.n].iter_mut() {
        if i.op != Op::MkvecV2i16 {
            continue;
        }
        if i.src[0].idx == i.src[1].idx && !matches!(i.src[0].idx, Idx::Const(_)) {
            let swz = match (i.src[0].swz, i.src[1].swz) {
                (Swz::H00, Swz::H00) => Swz::H00,
                (Swz::H11, Swz::H11) => Swz::H11,
                (Swz::H00, Swz::H11) => Swz::H01,
                (Swz::H11, Swz::H00) => Swz::H10,
                _ => return Err(MaliError::Defect(super::Defect::Operand)),
            };
            let x = i.src[0].swz(swz);
            *i = if swz == Swz::H01 {
                Ins { flow: i.flow, block: i.block, ..Ins::new(Op::MovI32, i.dests(), &[x]) }
            } else {
                Ins { flow: i.flow, block: i.block, ..Ins::new(Op::IaddV2u16, i.dests(), &[x, Src::of(Idx::Const(0))]) }
            };
            continue;
        }
        if is_const(&i.src[1], 0) {
            *i = Ins { flow: i.flow, block: i.block, ..Ins::new(Op::U16ToU32, i.dests(), &[i.src[0]]) };
        }
    }
    Ok(())
}

/// `va_lower_isel` (va_lower_isel.c:112-116) for the ops here: a `MUX`
/// whose three sources are unswizzled is a `CSEL` (`bi_csel_from_mux`:
/// `CSEL.u32.eq mask, 0, a, b`, the signed form since `.int_zero` compares
/// with zero). The 16-bit form's CSEL (`CSEL.v2u16`) has no reference.
pub fn lower_isel(p: &mut Prog<'_>) -> Result<(), MaliError> {
    for i in p.ins[..p.n].iter_mut() {
        if !matches!(i.op, Op::MuxI32 | Op::MuxV2i16) {
            continue;
        }
        if i.srcs().iter().any(|s| s.swz != Swz::H01) {
            continue;
        }
        if i.op == Op::MuxV2i16 {
            return Err(MaliError::Defect(super::Defect::Operand));
        }
        let srcs = [i.src[2], Src::of(Idx::Const(0)), i.src[0], i.src[1]];
        *i = Ins { flow: i.flow, block: i.block, ..Ins::new(Op::CselU32, i.dests(), &srcs) };
    }
    Ok(())
}

/// `va_fuse_add_imm` (va_optimize.c:67-110) for the FADDs: the first
/// constant source becomes the immediate (swizzled, its `.neg` flipping
/// the sign bits) when the other source has the H01 swizzle and no
/// modifier (`va_is_add_imm`, which also wants no clamp and no rounding on
/// the instruction), and the op does not convert through its destination's
/// swizzle (`va_op_dest_modifier_does_convert`: FADD.f32). An `IADD.s32`
/// with a constant is `IADD_IMM.i32` (`va_op_add_imm`; case108_uniidx's
/// `IADD_IMM.i32 r1, r1^, #0x10`, the array's byte offset).
pub fn fuse_add_imm(p: &mut Prog<'_>) {
    for i in p.ins[..p.n].iter_mut() {
        if i.op == Op::IaddS32 {
            let Some(s) = (0..2).find(|&s| matches!(i.src[s].idx, Idx::Const(_))) else { continue };
            let other = i.src[1 - s];
            if other.swz != Swz::H01 || other.abs || other.neg || i.src[s].neg {
                continue;
            }
            let Idx::Const(c) = i.src[s].idx else { continue };
            let imm = apply_swizzle(c, i.src[s].swz);
            *i = Ins {
                flow: i.flow,
                slot: i.slot,
                block: i.block,
                ..Ins::new(Op::IaddImmI32(imm), i.dests(), &[other])
            };
            continue;
        }
        if !matches!(i.op, Op::FaddF32 | Op::FaddV2f16) || i.clamp != clamp::NONE {
            continue;
        }
        if i.op == Op::FaddF32 && i.dest[0].swz != Swz::H01 {
            continue;
        }
        let Some(s) = (0..2).find(|&s| matches!(i.src[s].idx, Idx::Const(_))) else { continue };
        let other = i.src[1 - s];
        if other.swz != Swz::H01 || other.abs || other.neg {
            continue;
        }
        let Idx::Const(c) = i.src[s].idx else { continue };
        let mut imm = apply_swizzle(c, i.src[s].swz);
        if i.src[s].neg {
            imm ^= if i.op == Op::FaddF32 { 1 << 31 } else { 1 << 31 | 1 << 15 };
        }
        let op = if i.op == Op::FaddF32 { Op::FaddImmF32(imm) } else { Op::FaddImmV2f16(imm) };
        *i = Ins { flow: i.flow, slot: i.slot, block: i.block, ..Ins::new(op, i.dests(), &[other]) };
    }
}

/// The compare family of a 32-bit compare and its result type
/// (`va_cmp_opcode_to_cmp_type`, va_optimize.c:191-215): 0 float, 1
/// signed, 2 unsigned.
fn cmp_family(op: Op) -> Option<(u8, ResultType)> {
    match op {
        Op::FcmpOrF32 { result, .. } => Some((0, result)),
        Op::IcmpOrS32 { result, .. } => Some((1, result)),
        Op::IcmpOrU32 { result, .. } => Some((2, result)),
        _ => None,
    }
}

/// `va_fuse_cmp` (va_optimize.c:217-291, run to a fixed point by
/// `va_optimize_forward`, :349-385) for the 32-bit OR: `LSHIFT_OR.i32 x, y,
/// 0` of two compares of one family and result type is the compare `x`
/// (when it has this use alone and a zero third source) with `y` as its
/// third source, at the LSHIFT_OR's place; else `y` with `x`.
/// builtin_43.expect.json's `i == 0 || i == 3`: `ICMP_OR.u32.eq.m1 r2, r0,
/// 0x0, r1`. `LSHIFT_AND` would be the `_AND` compares, which no reference
/// draws.
pub fn fuse_cmp(p: &mut Prog<'_>) {
    let def = |p: &Prog<'_>, s: &Src| -> Option<usize> {
        let v = s.ssa_value()?;
        p.ins[..p.n].iter().position(|i| i.dests().iter().any(|d| d.idx == Idx::Ssa(v, 0)))
    };
    let uses = |p: &Prog<'_>, j: usize| -> usize {
        let Idx::Ssa(v, _) = p.ins[j].dest[0].idx else { return usize::MAX };
        p.ins[..p.n].iter().map(|i| i.srcs().iter().filter(|s| s.ssa_value() == Some(v)).count()).sum()
    };
    let mut progress = true;
    while progress {
        progress = false;
        for k in 0..p.n {
            let i = p.ins[k];
            if i.op != Op::LshiftOrI32 || i.ns != 3 || !is_const(&i.src[2], 0) {
                continue;
            }
            if i.src[..2].iter().any(|s| s.ssa_value().is_none() || s.swz != Swz::H01) {
                continue;
            }
            let (Some(a), Some(b)) = (def(p, &i.src[0]), def(p, &i.src[1])) else { continue };
            if a == b {
                continue;
            }
            let (Some(fa), Some(fb)) = (cmp_family(p.ins[a].op), cmp_family(p.ins[b].op)) else { continue };
            if fa != fb {
                continue;
            }
            let fusable = |p: &Prog<'_>, j: usize| uses(p, j) == 1 && is_const(&p.ins[j].src[2], 0);
            let (old, other) = if fusable(p, a) {
                (a, i.src[1])
            } else if fusable(p, b) {
                (b, i.src[0])
            } else {
                continue;
            };
            let o = p.ins[old];
            let srcs = [o.src[0], o.src[1], Src { swz: Swz::H01, ..other }];
            p.ins[k] = Ins { flow: i.flow, slot: i.slot, ..Ins::new(o.op, i.dests(), &srcs) };
            p.ins[k].block = i.block;
            p.remove(old);
            progress = true;
            break;
        }
    }
}

/// `bi_apply_swizzle` for the 16-bit swizzles: the halves the lanes read.
fn apply_swizzle(c: u32, s: Swz) -> u32 {
    let (lo, hi) = halves(s);
    let half = |h: bool| if h { c >> 16 } else { c & 0xffff };
    half(lo) | half(hi) << 16
}

/// `bi_opt_cse` (bi_opt_cse.c) after the constant lowering
/// (bifrost_compile.c:4525-4529): an instruction equal to an earlier one
/// of its block (op, sources with their modifiers, destination sizes;
/// the set is cleared per block, :152, "intra-block only") is removed and
/// its values read from the earlier one's. A staging source keeps its
/// value (`bi_is_staging_src`, :153-154): a later equal `COLLECT` a message
/// reads stays (unitype_166c's sixteen look-ups at `vec2(0.5)`, each moving
/// the coordinate into its own staging registers, `MOV.i32 r2, u6.w0`,
/// `MOV.i32 r3, u6.w0`).
pub fn cse(p: &mut Prog<'_>) {
    let mut k = 0;
    while k < p.n {
        let a = p.ins[k];
        let earlier = if can_cse(&a) {
            (0..k).find(|&j| {
                let b = &p.ins[j];
                b.op == a.op
                    && b.block == a.block
                    && b.nd == a.nd
                    && b.clamp == a.clamp
                    && b.srcs() == a.srcs()
                    && b.dests().iter().zip(a.dests()).all(|(x, y)| x.swz == y.swz)
            })
        } else {
            None
        };
        let Some(j) = earlier else {
            k += 1;
            continue;
        };
        let keep = p.ins[j];
        let mut staged = false;
        for d in 0..a.nd as usize {
            let (Idx::Ssa(from, 0), Idx::Ssa(to, 0)) = (a.dest[d].idx, keep.dest[d].idx) else { continue };
            /* every reader: a loop header's phi stands before the writer */
            for i in (0..p.n).filter(|&i| i != k) {
                for s in 0..p.ins[i].ns as usize {
                    if let Idx::Ssa(v, off) = p.ins[i].src[s].idx {
                        if v == from && p.ins[i].is_staging_src(s) {
                            staged = true;
                        } else if v == from {
                            p.ins[i].src[s].idx = Idx::Ssa(to, off);
                        }
                    }
                }
            }
        }
        if staged {
            k += 1;
            continue;
        }
        p.remove(k);
    }
}

/// `instr_can_cse` (bi_opt_cse.c:126-147) for the ops here: no `DISCARD`
/// (:117-124), no message other than a `lea_buf`, no branch.
fn can_cse(i: &Ins) -> bool {
    let p = bir::props(i.op);
    !matches!(i.op, Op::Nop | Op::Phi | Op::DiscardF32 { .. } | Op::DiscardB32)
        && !p.branch
        && (p.msg == bir::Msg::None || matches!(i.op, Op::LeaBufImm { .. }))
}

/// `is_simple_block` (bi_opt_control_flow.c:14-25): one predecessor, one
/// successor, and no instruction or a lone branch.
fn is_simple(p: &Prog<'_>, b: u8) -> bool {
    let blk = &p.blocks[b as usize];
    if blk.nsucc() != 1 || blk.npred != 1 {
        return false;
    }
    let (start, end) = (p.block_start(b), p.block_end(b));
    end == start || (end == start + 1 && bir::props(p.ins[start].op).branch)
}

/// `remove_simple_block` (:38-65): the predecessor's edge into `b` now
/// enters `b`'s successor (one edge where both did), `b` replaces itself
/// by its predecessor in the successor's predecessors (dropped where the
/// predecessor was one already), and `b` leaves the layout with its lone
/// branch, if any.
fn remove_simple(p: &mut Prog<'_>, b: u8) {
    let blk = p.blocks[b as usize];
    let (pred, succ) = (blk.pred[0], blk.succ[0].unwrap_or(b));
    let pb = &mut p.blocks[pred as usize];
    let idx = if pb.succ[0] == Some(b) { 0 } else { 1 };
    pb.succ[idx] = Some(succ);
    if pb.succ[idx ^ 1] == Some(succ) {
        pb.succ[1] = None;
    }
    let sb = &mut p.blocks[succ as usize];
    let old = *sb;
    sb.npred = 0;
    for &x in old.preds() {
        let x = if x == b { pred } else { x };
        if x == pred && sb.preds().contains(&pred) {
            continue;
        }
        sb.pred[sb.npred as usize] = x;
        sb.npred += 1;
    }
    let start = p.block_start(b);
    if p.block_end(b) > start {
        p.remove(start);
    }
    p.blocks[b as usize] = bir::Block { dead: true, ..bir::Block::NONE };
}

/// `try_remove_simple_block` (:97-146) for `block` between `prev` and
/// `next` in the layout: a simple block whose predecessor branches into it
/// (the branch retargeted to its successor), or whose predecessor is `prev`
/// and branches to `next` (the branch inverted and retargeted), or that
/// lies between its predecessor `prev` and successor `next` with no
/// branch, goes. A successor already seen would make the jump backward:
/// left alone (:114-120).
fn try_remove_simple(p: &mut Prog<'_>, prev: Option<u8>, block: Option<u8>, next: u8, seen: &mut u128) -> bool {
    let Some(b) = block else { return false };
    *seen |= 1 << b;
    if !is_simple(p, b) {
        return false;
    }
    let blk = p.blocks[b as usize];
    let (pred, succ) = (blk.pred[0], blk.succ[0].unwrap_or(b));
    let last = p.last_of(pred).filter(|&k| bir::props(p.ins[k].op).branch);
    let succ_seen = *seen & (1 << succ) != 0;
    match last.map(|k| p.ins[k].op) {
        Some(Op::Branchz { target, eq }) if !succ_seen && (target == b || (prev == Some(pred) && target == next)) => {
            let k = last.unwrap_or(0);
            /* `inverse_cmpf`: `.eq` becomes `.ne` (always possible here) */
            let eq = if target == b { eq } else { !eq };
            p.ins[k].op = Op::Branchz { target: succ, eq };
            remove_simple(p, b);
            true
        }
        Some(_) => false,
        None if prev == Some(pred) && succ == next => {
            remove_simple(p, b);
            true
        }
        None => false,
    }
}

/// `bi_opt_control_flow` (bi_opt_control_flow.c:148-191), after register
/// allocation: the simple blocks go until none is left
/// (`try_remove_simple_blocks`: each block tried when its successor in the
/// layout is seen, so the last is never tried, :149-166), then a branch
/// to the block laid out next goes (`remove_branch_to_next_block`,
/// :168-183). An `if` with an empty else side keeps its `BRANCHZ`, now
/// to the join, and its then side's jump to the join, which falls through,
/// is removed.
pub fn control_flow(p: &mut Prog<'_>) {
    loop {
        let mut changed = false;
        let mut seen = 0u128;
        let (mut prev, mut block) = (None, None);
        for next in p.live_blocks().collect::<arrayvec_blocks::Blocks>() {
            let removed = try_remove_simple(p, prev, block, next, &mut seen);
            changed |= removed;
            if !removed {
                prev = block;
            }
            block = Some(next);
        }
        if !changed {
            break;
        }
    }
    let mut block: Option<u8> = None;
    for next in p.live_blocks().collect::<arrayvec_blocks::Blocks>() {
        if let Some(b) = block {
            let blk = p.blocks[b as usize];
            if let Some(k) = p.last_of(b) {
                if blk.nsucc() == 1 && matches!(p.ins[k].op, Op::Branchz { target, .. } if target == next) {
                    p.remove(k);
                }
            }
        }
        block = Some(next);
    }
}

/// The block indices of a program, collected without the heap.
mod arrayvec_blocks {
    use super::bir::MAX_BLOCKS;

    pub struct Blocks {
        v: [u8; MAX_BLOCKS],
        n: usize,
    }

    impl FromIterator<u8> for Blocks {
        fn from_iter<I: IntoIterator<Item = u8>>(it: I) -> Blocks {
            let mut b = Blocks { v: [0; MAX_BLOCKS], n: 0 };
            for x in it {
                if b.n < MAX_BLOCKS {
                    b.v[b.n] = x;
                    b.n += 1;
                }
            }
            b
        }
    }

    impl IntoIterator for Blocks {
        type Item = u8;
        type IntoIter = core::iter::Take<core::array::IntoIter<u8, MAX_BLOCKS>>;
        fn into_iter(self) -> Self::IntoIter {
            self.v.into_iter().take(self.n)
        }
    }
}

/// `bi_opt_dce` (bi_opt_dce.c): an instruction without side effects whose
/// values nothing reads is removed. A reader may stand before the writer:
/// a loop header's phi of the body's value.
pub fn dce(p: &mut Prog<'_>) {
    let mut k = p.n;
    while k > 0 {
        k -= 1;
        let i = p.ins[k];
        if i.side_effects() || i.nd == 0 {
            continue;
        }
        let read = i.dests().iter().any(|d| match d.idx {
            Idx::Ssa(v, _) => p
                .as_slice()
                .iter()
                .enumerate()
                .any(|(j, u)| j != k && u.srcs().iter().any(|s| s.ssa_value() == Some(v))),
            _ => true,
        });
        if !read {
            p.remove(k);
        }
    }
}
