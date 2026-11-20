// SPDX-License-Identifier: Apache-2.0 AND MIT
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
// SPDX-FileCopyrightText: 2014-2017 Broadcom
// Derived from Mesa 26.2.1 src/broadcom/compiler/ (nir_to_vir.c,
// v3d_nir_lower_io.c, qpu_validate.c) and qpu/ (qpu_instr.h, qpu_pack.c)
// (MIT). The upstream part stays under its licence (LICENSES/MIT.txt); our
// changes are Apache-2.0. See NOTICE.
//! Selection at V3D 7.1: the 4.2 selection ([`super::select::Selected`]) to
//! 7.1 virtual operations ([`super::vop71::Op71`]), as the contract in
//! [`super::vop71`]'s header and `docs/notes/v3d71-backend.md` ("select71.rs")
//! say.
//!
//! Operation for operation, a [`VOp`] becomes the [`Op71`] with the same Mesa
//! enum value ([`add_op71`], [`mul_op71`]) and the same meaning, except in
//! the note's five places:
//!
//! 1. `Operand::R5`, `ldvary`'s C term, becomes [`Src71::LdvaryC`]: at 7.1 it
//!    lands in rf0 (`emit_fragment_varying`, nir_to_vir.c:1054-1057).
//! 2. `Operand::Payload(0)` (W), `(1)` (centroid W) and `(2)` (Z) become
//!    [`Src71::Payload`] 3, 1 and 2 (nir_to_vir.c:4851-4858).
//! 3. A vertex or coordinate shader's viewport-scaled x and y,
//!    `ftoiz(ffloor(t))` at 4.2, become one `ftoin(t)`
//!    (v3d_nir_lower_io.c:529-543, nir_to_vir.c:1452-1458). The pair is
//!    recognised by its data flow exactly as `select_vertex` builds it; a
//!    vertex or coordinate shader without both is
//!    [`Select71Error::ViewportXyMissing`].
//! 4. The closing `vpmwt` of a vertex or coordinate shader (GFXH-1684) is
//!    dropped: Mesa emits it at `ver == 42` only (`emit_vert_end`,
//!    nir_to_vir.c:2163-2170).
//! 5. An operation reading two small immediates reads the second from a new
//!    `ldunif` of its value: one small immediate per instruction at 7.1
//!    (qpu_validate.c:204-218).
//!
//! The SFU operations (select.rs [`is_sfu`]: an add operation with a register
//! destination, already so at 4.2) become [`Op71::Sfu`]. A 4.2 form with no
//! 7.1 one is refused by name ([`Select71Error::NotOn71`]), never lowered to
//! something else: the add operations missing from `v3d71_add_ops`, a magic
//! write address that is reserved or means something else at 7.1, a payload
//! register 4.2 does not have.
//!
//! What stays for the later stages: whether an [`Op71::Ldunif`] is `ldunif`
//! or `ldunifrf` (alloc71's choice of rf0, vop71.rs), every timing and
//! pairing rule (sched71), and the uniform stream's order (the operations'
//! order, which this lowering keeps; rule 5's load sits right before the
//! operation that reads it).

use super::isa::{small_imm_value, AddOp, MulOp};
use super::select::{
    is_sfu, Dst, FlagOp, Operand, Selected, VOp, QUNIFORM_VIEWPORT_X_SCALE, QUNIFORM_VIEWPORT_Y_SCALE,
};
use super::vop71::{AddOp71, Dst71, FlagOp71, Label, MulOp71, Op71, Selected71, Sfu71, Src71, Uniform, Unpack71, V};
use crate::shaderc::online::arena::{AVec, Arena, ArenaError};

/// Why 7.1 selection stopped.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Select71Error {
    /// A construct this lowering does not handle yet, by name.
    NotYet(&'static str),
    /// A 4.2 operation or operand with no 7.1 form, by name.
    NotOn71(&'static str),
    /// A vertex or coordinate shader without its viewport-scaled x and y
    /// pair to rewrite (rule 3; a defect: select.rs always emits both).
    ViewportXyMissing,
    OutOfScratch,
}

impl From<ArenaError> for Select71Error {
    fn from(_: ArenaError) -> Self {
        Select71Error::OutOfScratch
    }
}

/// The 7.1 add operation with the same `enum v3d_qpu_add_op` value, or the
/// 4.2 operation's Mesa name when `v3d71_add_ops` has no entry for it
/// (qpu_pack.c:586-770): the VPM outputs LDVPMV_OUT, LDVPMD_OUT, LDVPMG_OUT
/// and VPMSETUP. (VDWWT, the fifth 4.2-only one, is not an [`AddOp`].)
pub fn add_op71(op: AddOp) -> Result<AddOp71, &'static str> {
    match op {
        AddOp::LdvpmvOut => Err("LDVPMV_OUT"),
        AddOp::LdvpmdOut => Err("LDVPMD_OUT"),
        AddOp::LdvpmgOut => Err("LDVPMG_OUT"),
        AddOp::Vpmsetup => Err("VPMSETUP"),
        /* AddOp71 has every member of the enum, so the value is there. */
        _ => AddOp71::ALL.iter().copied().find(|o| *o as u32 == op as u32).ok_or("an add operation"),
    }
}

/// The 7.1 mul operation with the same `enum v3d_qpu_mul_op` value: every
/// 4.2 one has a `v3d71_mul_ops` entry (qpu_pack.c:772-812).
pub fn mul_op71(op: MulOp) -> MulOp71 {
    match op {
        MulOp::Add => MulOp71::Add,
        MulOp::Sub => MulOp71::Sub,
        MulOp::Umul24 => MulOp71::Umul24,
        MulOp::Vfmul => MulOp71::Vfmul,
        MulOp::Smul24 => MulOp71::Smul24,
        MulOp::Multop => MulOp71::Multop,
        MulOp::Fmov => MulOp71::Fmov,
        MulOp::Mov => MulOp71::Mov,
        MulOp::Nop => MulOp71::Nop,
        MulOp::Fmul => MulOp71::Fmul,
    }
}

/// The SFU operation an [`is_sfu`] add operation issues.
fn sfu71(op: AddOp) -> Option<Sfu71> {
    Some(match op {
        AddOp::Recip => Sfu71::Recip,
        AddOp::Rsqrt => Sfu71::Rsqrt,
        AddOp::Exp => Sfu71::Exp,
        AddOp::Log => Sfu71::Log,
        AddOp::Sin => Sfu71::Sin,
        AddOp::Rsqrt2 => Sfu71::Rsqrt2,
        _ => return None,
    })
}

/// The magic write addresses that mean the same at 4.2 and 7.1
/// (`enum v3d_qpu_waddr`, qpu_instr.h:93-136): NOP, TLB, TLBU, UNIFA,
/// TMUD, TMUA, TMUAU, SYNC*, and the TMU registers 32..46. The others are
/// reserved at 7.1 (r0..r4, TMUL, VPM, VPMU, the SFU 19..24, 47..63;
/// `v3d71_magic_waddr_is_reserved`, qpu_validate.c:104-115) or change
/// meaning (5: r5 at 4.2, QUAD at 7.1).
fn magic71(w: u8) -> bool {
    matches!(w, 6..=9 | 11..=13 | 16..=18 | 32..=46)
}

fn src71(o: Operand) -> Result<Src71, Select71Error> {
    Ok(match o {
        Operand::V(v) => Src71::V(v),
        Operand::Abs(v) => Src71::Unpacked(v, Unpack71::Abs),
        Operand::Lo(v) => Src71::Unpacked(v, Unpack71::L),
        Operand::Hi(v) => Src71::Unpacked(v, Unpack71::H),
        /* The table and its packing take no version (qpu_pack.c:239-294). */
        Operand::SmallImm(i) => Src71::SmallImm(i),
        /* rf0 W at 4.2, rf3 at 7.1; centroid W rf1 and Z rf2 on both
         * (nir_to_vir.c:4851-4858). */
        Operand::Payload(0) => Src71::Payload(3),
        Operand::Payload(1) => Src71::Payload(1),
        Operand::Payload(2) => Src71::Payload(2),
        Operand::Payload(_) => return Err(Select71Error::NotOn71("a payload register other than rf0..rf2")),
        /* ldvary's C: r5 at 4.2, rf0 at 7.1 (nir_to_vir.c:1054-1057). */
        Operand::R5 => Src71::LdvaryC,
        Operand::None => Src71::None,
    })
}

fn dst71(d: Dst) -> Result<Dst71, Select71Error> {
    Ok(match d {
        Dst::V(v) => Dst71::V(v),
        Dst::Tlb => Dst71::Tlb,
        Dst::Tlbu(u) => Dst71::Tlbu(u),
        Dst::Nop => Dst71::Nop,
        Dst::None => Dst71::None,
        Dst::Magic(w) if magic71(w) => Dst71::Magic(w),
        Dst::Magic(_) => return Err(Select71Error::NotOn71("a magic write address reserved or redefined at 7.1")),
    })
}

/// Rule 5: when both `a` and `b` are small immediates, `b` comes from a new
/// register loaded with its value right before the operation.
fn one_small_imm<'a>(
    a: Src71,
    b: Src71,
    ops: &mut AVec<'a, Op71>,
    next: &mut u32,
    ar: &Arena<'a>,
) -> Result<Src71, Select71Error> {
    match (a, b) {
        (Src71::SmallImm(_), Src71::SmallImm(i)) => {
            let value = small_imm_value(i).ok_or(Select71Error::NotOn71("a small-immediate index with no value"))?;
            let dst = V(*next);
            *next += 1;
            ops.push(ar, Op71::Ldunif { dst, u: Uniform::constant(value) })?;
            Ok(Src71::V(dst))
        }
        _ => Ok(b),
    }
}

/// Rule 3's match: the indices of each `ffloor` to drop, and for each
/// `ftoiz` the register it then reads directly (`ftoin` of the `ffloor`'s
/// operand). `u32::MAX`: not part of a pair.
struct Viewport<'a> {
    drop: &'a mut [bool],
    ftoin_of: &'a mut [u32],
}

fn viewport_pairs<'a>(ops: &[VOp], vregs: u32, ar: &Arena<'a>) -> Result<Viewport<'a>, Select71Error> {
    let n = vregs as usize;
    let def_at = ar.alloc_filled(n, u32::MAX)?;
    let uses = ar.alloc_filled(n, 0u32)?;
    for (i, op) in ops.iter().enumerate() {
        if let Some(V(v)) = op.def() {
            if (v as usize) < n && def_at[v as usize] == u32::MAX {
                def_at[v as usize] = i as u32;
            }
        }
        for o in op.uses() {
            if let Some(V(v)) = o.vreg() {
                if (v as usize) < n {
                    uses[v as usize] += 1;
                }
            }
        }
    }
    let def = |o: Operand| -> Option<&VOp> {
        let V(v) = o.vreg()?;
        if !matches!(o, Operand::V(_)) {
            return None;
        }
        ops.get(*def_at.get(v as usize)? as usize)
    };
    /* The FMUL chain select_vertex builds (select.rs:2564-2571):
     * t = pos * scale, t = t * (1/w), where scale is an ldunif of a viewport
     * scale; a shader's own code can never load those contents. */
    fn scale_of<'o>(def: &dyn Fn(Operand) -> Option<&'o VOp>, o: Operand, depth: u32) -> Option<u32> {
        let VOp::Mul { op: MulOp::Fmul, a, b, .. } = *def(o)? else {
            return None;
        };
        for x in [a, b] {
            match def(x) {
                Some(VOp::Ldunif { u, .. })
                    if u.contents == QUNIFORM_VIEWPORT_X_SCALE || u.contents == QUNIFORM_VIEWPORT_Y_SCALE =>
                {
                    return Some(u.contents)
                }
                _ if depth > 0 => {
                    if let Some(c) = scale_of(def, x, depth - 1) {
                        return Some(c);
                    }
                }
                _ => {}
            }
        }
        None
    }
    let drop = ar.alloc_filled(ops.len(), false)?;
    let ftoin_of = ar.alloc_filled(ops.len(), u32::MAX)?;
    let mut found = 0u32;
    for (i, op) in ops.iter().enumerate() {
        let VOp::Add { op: AddOp::Ftoiz, dst: Dst::V(t2), a: a @ Operand::V(t1), b: Operand::None } = *op else {
            continue;
        };
        let Some(&VOp::Add { op: AddOp::Ffloor, dst: Dst::V(_), a: t0 @ Operand::V(V(t0v)), b: Operand::None }) =
            def(a)
        else {
            continue;
        };
        let read_once = |V(v): V| uses.get(v as usize) == Some(&1);
        let stored = ops.iter().any(|o| matches!(o, VOp::Add { op: AddOp::Stvpmv, b: Operand::V(x), .. } if *x == t2));
        if !(read_once(t1) && read_once(t2) && stored) {
            continue;
        }
        let Some(contents) = scale_of(&def, t0, 2) else {
            continue;
        };
        drop[def_at[t1.0 as usize] as usize] = true;
        ftoin_of[i] = t0v;
        found |= 1 << (contents - QUNIFORM_VIEWPORT_X_SCALE);
    }
    if found != 0b11 {
        return Err(Select71Error::ViewportXyMissing);
    }
    Ok(Viewport { drop, ftoin_of })
}

/// Lower `sel`'s operations to 7.1 ones.
pub fn select71<'a>(sel: &Selected<'_>, a: &Arena<'a>) -> Result<Selected71<'a>, Select71Error> {
    let ops = sel.ops;
    let vp = if sel.fragment { None } else { Some(viewport_pairs(ops, sel.vregs, a)?) };
    /* Rule 4: the closing vpmwt. */
    let end = match ops.last() {
        Some(VOp::Add { op: AddOp::Vpmwt, .. }) if !sel.fragment => ops.len() - 1,
        _ => ops.len(),
    };
    let mut out: AVec<'a, Op71> = AVec::with_capacity(a, end)?;
    let mut next = sel.vregs;
    for (i, op) in ops[..end].iter().enumerate() {
        if let Some(vp) = &vp {
            if vp.drop[i] {
                continue;
            }
            if vp.ftoin_of[i] != u32::MAX {
                let dst = dst71(match *op {
                    VOp::Add { dst, .. } => dst,
                    _ => Dst::None,
                })?;
                out.push(a, Op71::Add { op: AddOp71::Ftoin, dst, a: Src71::V(V(vp.ftoin_of[i])), b: Src71::None })?;
                continue;
            }
        }
        let lowered = match *op {
            VOp::Add { op, dst, a: x, b: y } if is_sfu(op) => {
                let Dst::V(dst) = dst else {
                    return Err(Select71Error::NotOn71("an SFU operation without a register destination"));
                };
                if y != Operand::None {
                    return Err(Select71Error::NotOn71("an SFU operation with two operands"));
                }
                let op = sfu71(op).ok_or(Select71Error::NotYet("an SFU operation"))?;
                Op71::Sfu { op, dst, a: src71(x)? }
            }
            VOp::Add { op, dst, a: x, b: y } => {
                let op = add_op71(op).map_err(Select71Error::NotOn71)?;
                let (x, y) = (src71(x)?, src71(y)?);
                let y = one_small_imm(x, y, &mut out, &mut next, a)?;
                Op71::Add { op, dst: dst71(dst)?, a: x, b: y }
            }
            VOp::Mul { op, dst, a: x, b: y } => {
                let (x, y) = (src71(x)?, src71(y)?);
                let y = one_small_imm(x, y, &mut out, &mut next, a)?;
                Op71::Mul { op: mul_op71(op), dst: dst71(dst)?, a: x, b: y }
            }
            VOp::Ldunif { dst, u } => Op71::Ldunif { dst, u },
            VOp::Ldvary { dst } => Op71::Ldvary { dst },
            VOp::Ldtmu { dst } => Op71::Ldtmu { dst },
            VOp::Wrtmuc { u } => Op71::Wrtmuc { u },
            VOp::Thrsw => Op71::Thrsw,
            VOp::Flags { unit, a: x, b: y, pf, uf } => {
                let unit = match unit {
                    FlagOp::Add(op) if is_sfu(op) => {
                        return Err(Select71Error::NotOn71("a flag push from an SFU operation"))
                    }
                    FlagOp::Add(op) => FlagOp71::Add(add_op71(op).map_err(Select71Error::NotOn71)?),
                    FlagOp::Mul(op) => FlagOp71::Mul(mul_op71(op)),
                };
                let (x, y) = (src71(x)?, src71(y)?);
                let y = one_small_imm(x, y, &mut out, &mut next, a)?;
                Op71::Flags { unit, a: x, b: y, pf, uf }
            }
            VOp::CondMov { dst, src, cond } => Op71::CondMov { dst, src: src71(src)?, cond },
            VOp::Setmsf { src, cond } => Op71::Setmsf { src: src71(src)?, cond },
            VOp::Copy { dst, src } => Op71::Copy { dst, src: src71(src)? },
            VOp::LoopHead => Op71::LoopHead,
            VOp::BackEdge { cond, msfign } => Op71::BackEdge { cond, msfign },
            VOp::FwdBranch { cond, msfign, label } => Op71::FwdBranch { cond, msfign, label: Label(label) },
            VOp::Label(l) => Op71::Label(Label(l)),
        };
        out.push(a, lowered)?;
    }
    Ok(Selected71 { ops: out.into_slice(), fragment: sel.fragment, vregs: next })
}

// ---------------------------------------------------------------------------
// Interleaving: the operations in an order sched71 pairs.
// ---------------------------------------------------------------------------

/// How far ahead of the first operation not yet placed [`interleave71`]
/// looks for one to place (operations, not words): the bound on how far an
/// operation moves up, and so on how much longer its value lives.
pub const INTERLEAVE_WINDOW71: usize = 32;

/// What [`interleave71`] knows of one operation: the registers and pseudo
/// registers it reads and writes, the issue slots it takes, the latency of
/// its result.
#[derive(Clone, Copy)]
struct Shape {
    reads: [u32; 4],
    writes: [u32; 4],
    /// [`SLOT_ADD`] .. [`SLOT_EITHER`] bits.
    slots: u8,
    /// Words until a reader of its result (an `ldvary`'s: of its C term)
    /// may issue (sched71.rs header: an SFU result two, the C term two).
    latency: u8,
    /// A thread switch or control flow: it stays, and nothing crosses it.
    barrier: bool,
}

const NO_RES: u32 = u32::MAX;
const EMPTY: Shape = Shape { reads: [NO_RES; 4], writes: [NO_RES; 4], slots: 0, latency: 1, barrier: false };
const SLOT_ADD: u8 = 1;
const SLOT_MUL: u8 = 2;
/// The signal: `ldunif`, `ldvary`, `ldtmu`, `wrtmuc`.
const SLOT_SIG: u8 = 4;
/// The word's one uniform.
const SLOT_UNIF: u8 = 8;
/// The word's one small immediate (qpu_validate.c:204-218).
const SLOT_IMM: u8 = 16;
/// One TLB or TMU access.
const SLOT_PERIPH: u8 = 32;
/// The condition field: a flag push, a conditional write, or a load signal
/// that names its register (`ldunifrf`, `ldvary` and `ldtmu` at 7.1,
/// vir_to_qpu.c:328-350).
const SLOT_COND: u8 = 64;
/// The mul unit, or the add unit when the mul unit is taken (a MOV or
/// FMOV: sched71 converts it, `can_do_mul_as_add`).
const SLOT_EITHER: u8 = 128;

/// The add operations that move when they write a register: arithmetic,
/// logic, conversions, packs and moves of their operands only, with no
/// flag, lane, quad, VPM, TMU or rtop state (sched.rs `hoistable`'s list,
/// at 7.1).
fn pure_add71(op: AddOp71) -> bool {
    use AddOp71::*;
    matches!(
        op,
        Fadd | Faddnf
            | Vfpack
            | Add
            | Sub
            | Fsub
            | Min
            | Max
            | Umin
            | Umax
            | Shl
            | Shr
            | Asr
            | Ror
            | Fmin
            | Fmax
            | Vfmin
            | Vfmax
            | Vfadd
            | Vfsub
            | Vfmov
            | Vfabs
            | Vfneg
            | Vfnab
            | And
            | Or
            | Xor
            | Vadd
            | Vsub
            | Not
            | Neg
            | Fround
            | Ftoin
            | Ftrunc
            | Ftoiz
            | Ffloor
            | Ftouz
            | Fceil
            | Ftoc
            | Itof
            | Utof
            | Clz
            | Fmov
            | Mov
            | Vpack
            | V8pack
            | V10pack
            | V11fpack
    )
}

/// The mul operations that move (UMUL24 and MULTOP share the hidden rtop).
fn pure_mul71(op: MulOp71) -> bool {
    matches!(
        op,
        MulOp71::Add | MulOp71::Sub | MulOp71::Fmul | MulOp71::Fmov | MulOp71::Mov | MulOp71::Vfmul | MulOp71::Smul24
    )
}

/// A non-register destination's issue slots beyond its unit.
fn dst_slots(d: Dst71) -> u8 {
    match d {
        Dst71::Tlb | Dst71::Magic(_) => SLOT_PERIPH,
        Dst71::Tlbu(_) => SLOT_PERIPH | SLOT_UNIF,
        _ => 0,
    }
}

/// A destination that consumes a uniform, or a magic address that is no TMU
/// register (UNIFA, which moves the uniform stream, among them): in order
/// with the uniform loads.
fn dst_takes_uniform(d: Dst71) -> bool {
    match d {
        Dst71::Tlbu(_) => true,
        Dst71::Magic(w) => !matches!(w, 11..=13 | 32..=46),
        _ => false,
    }
}

/// The resources of [`shape`]: the virtual registers, then four pseudo
/// registers.
#[derive(Clone, Copy)]
struct Res {
    /// The flags (pushes, updates, conditional writes, flag reads).
    flags: u32,
    /// The uniform stream.
    unif: u32,
    /// rf0's implicit writes (`ldunif`, `ldvary`) against the `ldvary` C
    /// reads, which name the last `ldvary` before them.
    rf0: u32,
    /// Every operation with an effect beyond its register result: they keep
    /// their order among themselves.
    side: u32,
}

struct ShapeBuilder {
    s: Shape,
    r: usize,
    w: usize,
}

impl ShapeBuilder {
    fn read(&mut self, x: u32) {
        if !self.s.reads[..self.r].contains(&x) {
            self.s.reads[self.r] = x;
            self.r += 1;
        }
    }
    fn write(&mut self, x: u32) {
        if !self.s.writes[..self.w].contains(&x) {
            self.s.writes[self.w] = x;
            self.w += 1;
        }
    }
    /// An effect beyond the register result: in order with every other one,
    /// and with the flags (some of them read or write the flags).
    fn effect(&mut self, res: Res) {
        self.read(res.side);
        self.write(res.side);
        self.read(res.flags);
        self.write(res.flags);
    }
}

/// The shape of `op`. Reads: at most two operands, rf0 or the pseudo
/// registers; writes: a register and at most three pseudo registers.
fn shape(op: &Op71, res: Res) -> Shape {
    let mut b = ShapeBuilder { s: EMPTY, r: 0, w: 0 };
    let mut imm = 0u8;
    for src in op.uses() {
        match src {
            Src71::V(v) | Src71::Unpacked(v, _) => b.read(v.0),
            Src71::LdvaryC => b.read(res.rf0),
            Src71::SmallImm(_) => imm = SLOT_IMM,
            Src71::Payload(_) | Src71::None => {}
        }
    }
    /* an ALU operation: a pure one into a register moves freely; anything
     * else is an effect, and a non-register destination may take a
     * peripheral or a uniform */
    let alu = |b: &mut ShapeBuilder, pure: bool, dst: Dst71| match dst {
        Dst71::V(v) => {
            b.write(v.0);
            if !pure {
                b.effect(res);
            }
        }
        d => {
            b.effect(res);
            b.s.slots |= dst_slots(d);
            if dst_takes_uniform(d) {
                b.write(res.unif);
            }
        }
    };
    let either = |o: MulOp71| if matches!(o, MulOp71::Mov | MulOp71::Fmov) { SLOT_EITHER } else { SLOT_MUL };
    match *op {
        Op71::Thrsw | Op71::LoopHead | Op71::BackEdge { .. } | Op71::FwdBranch { .. } | Op71::Label(_) => {
            b.s.barrier = true;
        }
        Op71::Add { op: o, dst, .. } => {
            b.s.slots = SLOT_ADD | imm;
            alu(&mut b, pure_add71(o), dst);
        }
        Op71::Mul { op: o, dst, .. } => {
            b.s.slots = either(o) | imm;
            alu(&mut b, pure_mul71(o), dst);
        }
        Op71::Sfu { dst, .. } => {
            b.s.slots = SLOT_ADD | imm;
            b.s.latency = 2;
            b.write(dst.0);
        }
        Op71::Ldunif { dst, .. } => {
            b.s.slots = SLOT_SIG | SLOT_UNIF | SLOT_COND;
            b.write(dst.0);
            b.write(res.unif);
            b.write(res.rf0);
        }
        Op71::Ldvary { dst } => {
            b.s.slots = SLOT_SIG | SLOT_COND;
            b.s.latency = 2;
            b.write(dst.0);
            b.write(res.rf0);
            b.read(res.side);
            b.write(res.side);
        }
        Op71::Ldtmu { dst } => {
            b.s.slots = SLOT_SIG | SLOT_COND;
            b.write(dst.0);
            b.read(res.side);
            b.write(res.side);
        }
        Op71::Wrtmuc { .. } => {
            b.s.slots = SLOT_SIG | SLOT_UNIF;
            b.write(res.unif);
            b.read(res.side);
            b.write(res.side);
        }
        Op71::Flags { unit, .. } => {
            let pure = match unit {
                FlagOp71::Add(o) => {
                    b.s.slots = SLOT_ADD;
                    pure_add71(o) || o == AddOp71::Fcmp
                }
                FlagOp71::Mul(o) => {
                    b.s.slots = either(o);
                    pure_mul71(o)
                }
            };
            b.s.slots |= imm | SLOT_COND;
            b.read(res.flags);
            b.write(res.flags);
            if !pure {
                b.read(res.side);
                b.write(res.side);
            }
        }
        Op71::CondMov { dst, .. } => {
            /* a use of dst (Op71::uses) and a write of it */
            b.s.slots = SLOT_EITHER | SLOT_COND | imm;
            b.read(res.flags);
            b.write(dst.0);
        }
        Op71::Setmsf { .. } => {
            b.s.slots = SLOT_ADD | SLOT_COND | imm;
            b.effect(res);
        }
        Op71::Copy { dst, .. } => {
            /* loop-carried values: in order with the effects */
            b.s.slots = SLOT_EITHER | imm;
            b.write(dst.0);
            b.read(res.side);
            b.write(res.side);
        }
    }
    b.s
}

/// Every dependence of the region `start..end` of `shapes`, as
/// `emit(from, to, latency)`: read after write (the writer's latency),
/// write after write (1) and write after read (0: a read sees the old value
/// in the word of the write) on every register and pseudo register.
/// `last_w` and `head` have a slot per resource; `nodes` one per read.
fn region_edges(
    shapes: &[Shape],
    start: usize,
    end: usize,
    last_w: &mut [u32],
    head: &mut [u32],
    nodes: &mut [(u32, u32)],
    emit: &mut dyn FnMut(usize, usize, u32),
) {
    last_w.fill(u32::MAX);
    head.fill(u32::MAX);
    let mut used = 0usize;
    for (i, s) in shapes.iter().enumerate().take(end).skip(start) {
        for &r in s.reads.iter().filter(|&&r| r != NO_RES) {
            let p = last_w[r as usize];
            if p != u32::MAX {
                emit(p as usize, i, shapes[p as usize].latency as u32);
            }
        }
        for &w in s.writes.iter().filter(|&&w| w != NO_RES) {
            let p = last_w[w as usize];
            if p != u32::MAX {
                emit(p as usize, i, 1);
            }
            /* the reads since that write, this operation's own excepted */
            let mut k = head[w as usize];
            while k != u32::MAX {
                let (reader, next) = nodes[k as usize];
                if reader as usize != i {
                    emit(reader as usize, i, 0);
                }
                k = next;
            }
        }
        for &w in s.writes.iter().filter(|&&w| w != NO_RES) {
            last_w[w as usize] = i as u32;
            head[w as usize] = u32::MAX;
        }
        for &r in s.reads.iter().filter(|&&r| r != NO_RES) {
            /* a read and a write of the same pseudo register (an effect):
             * the next writer waits for this one through the write */
            if s.writes.contains(&r) {
                continue;
            }
            nodes[used] = (i as u32, head[r as usize]);
            head[r as usize] = used as u32;
            used += 1;
        }
    }
}

/// `sel`'s operations in an order sched71 pairs better. pack71 and
/// schedule71 keep the operations' order and only merge neighbours
/// (sched71.rs header); 4.2's sched.rs finds its pairs with `hoist` and
/// `ldvary_back`, which sched71 does not have. Between two barriers (a
/// thread switch or a control-flow operation, which stay in place), a list
/// scheduler fills one word at a time with the earliest operations whose
/// inputs are ready in it and whose issue slots are free (the add and mul
/// units, one signal, one uniform, one small immediate, one TLB or TMU
/// access, the condition field), looking [`INTERLEAVE_WINDOW71`] operations
/// past the first one not yet placed.
///
/// The new order means what the old one did: each virtual register's
/// reads and writes keep their order, and so do the flags', the uniform
/// stream's, rf0's (its implicit writes against the `ldvary` C reads), and
/// all operations with an effect beyond their register result (TMU, TLB,
/// VPM, `ldvary`, `ldtmu`, lane and flag reads, rtop, loop copies) among
/// themselves. chain71 keeps the result only when it compiles to fewer words
/// at no fewer threads.
pub fn interleave71<'a>(sel: &Selected71<'_>, a: &Arena<'a>) -> Result<Selected71<'a>, Select71Error> {
    let order = order71(sel.ops, sel.vregs, a)?;
    let out = a.alloc_filled(sel.ops.len(), Op71::Thrsw)?;
    for (o, &i) in out.iter_mut().zip(order.iter()) {
        *o = sel.ops[i as usize];
    }
    Ok(Selected71 { ops: out, fragment: sel.fragment, vregs: sel.vregs })
}

/// [`interleave71`]'s order: position k of the new order holds operation
/// `order[k]` of `ops`.
fn order71<'a>(ops: &[Op71], v: u32, a: &Arena<'a>) -> Result<&'a [u32], ArenaError> {
    let n = ops.len();
    let res = Res { flags: v, unif: v + 1, rf0: v + 2, side: v + 3 };
    let nres = v as usize + 4;
    let shapes = a.alloc_filled(n, EMPTY)?;
    for (s, op) in shapes.iter_mut().zip(ops) {
        *s = shape(op, res);
    }
    let out = a.alloc_filled(n, 0u32)?;
    let last_w = a.alloc_filled(nres, u32::MAX)?;
    let head = a.alloc_filled(nres, u32::MAX)?;
    let nodes = a.alloc_filled(4 * n, (0u32, 0u32))?;
    let first_edge = a.alloc_filled(n + 1, 0u32)?;
    let fill = a.alloc_filled(n, 0u32)?;
    let npred = a.alloc_filled(n, 0u32)?;
    let earliest = a.alloc_filled(n, 0u32)?;
    let done = a.alloc_filled(n, false)?;
    let mut placed = 0usize;
    let mut start = 0usize;
    while start < n {
        if shapes[start].barrier {
            out[placed] = start as u32;
            placed += 1;
            start += 1;
            continue;
        }
        let end = (start..n).find(|&i| shapes[i].barrier).unwrap_or(n);
        /* the successor lists, counted then filled */
        first_edge[start..=end].fill(0);
        region_edges(shapes, start, end, last_w, head, nodes, &mut |from, _, _| first_edge[from + 1] += 1);
        for i in start..end {
            first_edge[i + 1] += first_edge[i];
        }
        let succ = a.alloc_filled(first_edge[end] as usize - first_edge[start] as usize, (0u32, 0u32))?;
        let base = first_edge[start] as usize;
        fill[start..end].fill(0);
        npred[start..end].fill(0);
        earliest[start..end].fill(0);
        done[start..end].fill(false);
        region_edges(shapes, start, end, last_w, head, nodes, &mut |from, to, lat| {
            succ[first_edge[from] as usize - base + fill[from] as usize] = (to as u32, lat);
            fill[from] += 1;
            npred[to] += 1;
        });
        /* the words: each takes the earliest operations that fit */
        let mut first = start;
        let mut cycle = 0u32;
        while first < end {
            let mut used = 0u8;
            for j in first..(first + INTERLEAVE_WINDOW71).min(end) {
                if done[j] || npred[j] != 0 || earliest[j] > cycle {
                    continue;
                }
                let mut need = shapes[j].slots;
                if need & SLOT_EITHER != 0 {
                    need = (need & !SLOT_EITHER) | if used & SLOT_MUL == 0 { SLOT_MUL } else { SLOT_ADD };
                }
                if need & used != 0 {
                    continue;
                }
                used |= need;
                done[j] = true;
                out[placed] = j as u32;
                placed += 1;
                for &(to, lat) in &succ[first_edge[j] as usize - base..first_edge[j + 1] as usize - base] {
                    npred[to as usize] -= 1;
                    earliest[to as usize] = earliest[to as usize].max(cycle + lat);
                }
            }
            while first < end && done[first] {
                first += 1;
            }
            cycle += 1;
        }
        start = end;
    }
    debug_assert_eq!(placed, n);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shaderc::online::ir::IoKind;
    use crate::shaderc::online::sema::Stage;
    use crate::shaderc::online::v3d::prepare::{fragment_inputs, prepare_fragment_with, prepare_vertex_with, SOp};
    use crate::shaderc::online::v3d::select::{select_fragment, select_vertex, Pressure};
    use crate::shaderc::online::{lower, store};
    use std::vec::Vec;

    const PASS_VS: &str = "#version 100
attribute vec4 a_pos;
attribute vec4 a_color;
varying vec4 v_color;
void main() { gl_Position = a_pos; v_color = a_color; }
";
    const PASS_FS: &str = "#version 100
precision mediump float;
varying vec4 v_color;
void main() { gl_FragColor = v_color; }
";
    const UNIFORM_FS: &str = "#version 100
precision mediump float;
uniform vec4 u_color;
void main() { gl_FragColor = u_color; }
";
    const IF_FS: &str = "#version 100
precision mediump float;
varying vec4 v_color;
void main() {
    vec4 c = v_color;
    if (v_color.x > 0.5) { c = vec4(1.0, 0.0, 0.0, 1.0); }
    gl_FragColor = c;
}
";
    const TEX_FS: &str = "#version 100
precision mediump float;
uniform sampler2D u_tex;
varying vec4 v_color;
void main() { gl_FragColor = texture2D(u_tex, v_color.xy); }
";
    const SFU_FS: &str = "#version 100
precision mediump float;
varying vec4 v_color;
void main() { gl_FragColor = vec4(inversesqrt(v_color.x), exp2(v_color.y), 0.0, 1.0); }
";

    /// The 4.2 selection of `src` (no varying list, Pressure::None), as
    /// chain.rs's tests drive it, and its 7.1 lowering, given to `f`.
    fn with_fs<R>(src: &str, f: impl FnOnce(&Selected<'_>, Result<Selected71<'_>, Select71Error>) -> R) -> R {
        store::with_scratch(|a| {
            let s = a.alloc_str(src).unwrap();
            let m = lower::compile_to_ir(a, Stage::Fragment, s).map_err(|e| e.msg).unwrap();
            let p = prepare_fragment_with(&m.module, &[], 0, a).expect("prepares");
            let inputs = fragment_inputs(&p, a).expect("inputs");
            let sel = select_fragment(&p, inputs, 0, 0, Pressure::None, a).expect("selects");
            let low = select71(&sel, a);
            f(&sel, low)
        })
        .expect("scratch")
    }

    /// The vertex (or, `coord`, coordinate) shader of `vs` writing the
    /// varyings `fs` reads, selected and lowered.
    fn with_vs<R>(
        vs: &str,
        fs: &str,
        coord: bool,
        f: impl FnOnce(&Selected<'_>, Result<Selected71<'_>, Select71Error>) -> R,
    ) -> R {
        store::with_scratch(|a| {
            let vsrc = a.alloc_str(vs).unwrap();
            let fsrc = a.alloc_str(fs).unwrap();
            let v = lower::compile_to_ir(a, Stage::Vertex, vsrc).map_err(|e| e.msg).unwrap();
            let fr = lower::compile_to_ir(a, Stage::Fragment, fsrc).map_err(|e| e.msg).unwrap();
            let fsp = prepare_fragment_with(&fr.module, &[], 0, a).expect("prepares");
            let inputs = fragment_inputs(&fsp, a).expect("inputs");
            /* chain.rs compile_program_keyed: each input to the vertex
             * output of the same name. */
            let wanted: Vec<(u8, u8)> = if coord {
                Vec::new()
            } else {
                inputs
                    .iter()
                    .map(|&(loc, comp)| {
                        let name = fr.module.inputs.iter().find(|d| d.kind == IoKind::Varying && d.location == loc);
                        let out =
                            v.module.outputs.iter().find(|d| d.kind == IoKind::Varying && d.name == name.unwrap().name);
                        (out.unwrap().location, comp)
                    })
                    .collect()
            };
            let p = prepare_vertex_with(&v.module, &wanted, &[], 0, a).expect("prepares");
            let mut records: Vec<u8> = Vec::new();
            for op in p.ops {
                if let SOp::Attr { loc, .. } = *op {
                    if !records.contains(&loc) {
                        records.push(loc);
                    }
                }
            }
            records.sort_unstable();
            let sel = select_vertex(&p, &records, coord, 0, Pressure::None, a).expect("selects");
            let low = select71(&sel, a);
            f(&sel, low)
        })
        .expect("scratch")
    }

    /// No 4.2-only form is left: no magic address reserved or redefined at
    /// 7.1, no add operation without a 7.1 encoding, no two small immediates
    /// in one operation, no ffloor+ftoiz viewport pair, no `vpmwt`.
    fn no_42_forms(l: &Selected71<'_>) {
        for op in l.ops {
            if let Op71::Add { dst: Dst71::Magic(w), .. } | Op71::Mul { dst: Dst71::Magic(w), .. } = *op {
                assert!(magic71(w), "{op:?}");
            }
            if let Op71::Add { op: o, .. } | Op71::Flags { unit: FlagOp71::Add(o), .. } = *op {
                assert!(
                    !matches!(
                        o,
                        AddOp71::LdvpmvOut
                            | AddOp71::LdvpmdOut
                            | AddOp71::LdvpmgOut
                            | AddOp71::Vpmsetup
                            | AddOp71::Vpmwt
                    ),
                    "{op:?}"
                );
                assert!(!super::super::verify71::is_sfu_op(o), "an SFU op outside Op71::Sfu: {op:?}");
            }
            if let [Src71::SmallImm(_), Src71::SmallImm(_)] = op.uses() {
                panic!("two small immediates: {op:?}");
            }
            for s in op.uses() {
                if let Src71::Payload(p) = s {
                    assert!((1..=3).contains(&p), "{op:?}");
                }
            }
        }
        if !l.fragment {
            assert!(!l.ops.iter().any(|o| matches!(o, Op71::Add { op: AddOp71::Ffloor, .. })), "{:?}", l.ops);
            assert!(!l.ops.iter().any(|o| matches!(o, Op71::Add { op: AddOp71::Ftoiz, .. })), "{:?}", l.ops);
        }
    }

    fn count(l: &Selected71<'_>, f: impl Fn(&Op71) -> bool) -> usize {
        l.ops.iter().filter(|o| f(o)).count()
    }
    fn count42(s: &Selected<'_>, f: impl Fn(&VOp) -> bool) -> usize {
        s.ops.iter().filter(|o| f(o)).count()
    }
    fn reads(op: &Op71, s: Src71) -> bool {
        op.uses().contains(&s)
    }

    /// The fragment-shader facts every lowering keeps: the same number of
    /// operations (no rule but 5 changes a fragment shader's count), each
    /// r5 read an rf0 (LdvaryC) read, each payload W read rf3.
    fn fs_common(s: &Selected<'_>, l: &Selected71<'_>) {
        no_42_forms(l);
        assert!(l.fragment);
        let r5 = count42(s, |o| o.uses().contains(&Operand::R5));
        let w = count42(s, |o| o.uses().contains(&Operand::Payload(0)));
        assert_eq!(count(l, |o| reads(o, Src71::LdvaryC)), r5);
        assert_eq!(count(l, |o| reads(o, Src71::Payload(3))), w);
        assert_eq!(count(l, |o| matches!(o, Op71::Ldvary { .. })), count42(s, |o| matches!(o, VOp::Ldvary { .. })));
        assert_eq!(count(l, |o| matches!(o, Op71::Thrsw)), count42(s, |o| matches!(o, VOp::Thrsw)));
        let tlb = |o: &Op71| {
            matches!(
                o,
                Op71::Add { dst: Dst71::Tlb | Dst71::Tlbu(_), .. } | Op71::Mul { dst: Dst71::Tlb | Dst71::Tlbu(_), .. }
            )
        };
        let tlb42 = |o: &VOp| {
            matches!(o, VOp::Add { dst: Dst::Tlb | Dst::Tlbu(_), .. } | VOp::Mul { dst: Dst::Tlb | Dst::Tlbu(_), .. })
        };
        assert_eq!(count(l, tlb), count42(s, tlb42));
        let added = count(l, |o| matches!(o, Op71::Ldunif { .. })) - count42(s, |o| matches!(o, VOp::Ldunif { .. }));
        assert_eq!(l.ops.len(), s.ops.len() + added);
        assert_eq!(l.vregs, s.vregs + added as u32);
    }

    #[test]
    fn pass_through_fs() {
        with_fs(PASS_FS, |s, l| {
            let l = l.expect("lowers");
            fs_common(s, &l);
            /* four smooth varyings: ldvary * W + C */
            assert_eq!(count(&l, |o| matches!(o, Op71::Ldvary { .. })), 4);
            assert_eq!(count(&l, |o| reads(o, Src71::LdvaryC)), 4);
            assert_eq!(count(&l, |o| reads(o, Src71::Payload(3))), 4);
            /* the last-thread-switch pair, then two vfpack TLB writes */
            assert_eq!(count(&l, |o| matches!(o, Op71::Thrsw)), 2);
            assert_eq!(count(&l, |o| matches!(o, Op71::Add { op: AddOp71::Vfpack, dst: Dst71::Tlb, .. })), 2);
        });
    }

    #[test]
    fn uniform_colour_fs() {
        with_fs(UNIFORM_FS, |s, l| {
            let l = l.expect("lowers");
            fs_common(s, &l);
            assert_eq!(count(&l, |o| matches!(o, Op71::Ldunif { .. })), 4);
            assert_eq!(count(&l, |o| matches!(o, Op71::Ldvary { .. })), 0);
            assert_eq!(count(&l, |o| matches!(o, Op71::Add { op: AddOp71::Vfpack, dst: Dst71::Tlb, .. })), 2);
        });
    }

    #[test]
    fn if_fs() {
        with_fs(IF_FS, |s, l| {
            let l = l.expect("lowers");
            fs_common(s, &l);
            /* The comparison is a flag push and the arm conditional moves
             * (or a branch, carried through as one). */
            let flags = count(&l, |o| matches!(o, Op71::Flags { .. }));
            let cond = count(&l, |o| matches!(o, Op71::CondMov { .. }));
            let cf = count(&l, |o| o.is_control_flow());
            assert_eq!(flags, count42(s, |o| matches!(o, VOp::Flags { .. })));
            assert_eq!(cond, count42(s, |o| matches!(o, VOp::CondMov { .. })));
            assert!(flags >= 1, "{:?}", l.ops);
            assert!(cond + cf >= 1, "{:?}", l.ops);
        });
    }

    #[test]
    fn texture_fs() {
        with_fs(TEX_FS, |s, l| {
            let l = l.expect("lowers");
            fs_common(s, &l);
            /* T then the retiring S write, both TMU magic addresses */
            let tmu = count(&l, |o| {
                matches!(o, Op71::Mul { dst: Dst71::Magic(32..=46), .. } | Op71::Add { dst: Dst71::Magic(32..=46), .. })
            });
            assert_eq!(tmu, 2, "{:?}", l.ops);
            assert!(count(&l, |o| matches!(o, Op71::Wrtmuc { .. })) >= 1);
            assert_eq!(count(&l, |o| matches!(o, Op71::Ldtmu { .. })), count42(s, |o| matches!(o, VOp::Ldtmu { .. })));
            assert!(count(&l, |o| matches!(o, Op71::Ldtmu { .. })) >= 1);
        });
    }

    #[test]
    fn sfu_fs() {
        with_fs(SFU_FS, |s, l| {
            let l = l.expect("lowers");
            fs_common(s, &l);
            let sfu = |k: Sfu71| count(&l, |o| matches!(o, Op71::Sfu { op, .. } if *op == k));
            assert_eq!(sfu(Sfu71::Rsqrt), 1, "{:?}", l.ops);
            assert_eq!(sfu(Sfu71::Exp), 1, "{:?}", l.ops);
            assert_eq!(
                count(&l, |o| matches!(o, Op71::Sfu { .. })),
                count42(s, |o| matches!(o, VOp::Add { op, .. } if is_sfu(*op)))
            );
        });
    }

    /// The vertex-shader facts: rule 3 turned both pairs into `ftoin`s and
    /// rule 4 dropped the `vpmwt`, and nothing else changed in number.
    fn vs_common(s: &Selected<'_>, l: &Selected71<'_>) {
        no_42_forms(l);
        assert!(!l.fragment);
        assert!(matches!(s.ops.last(), Some(VOp::Add { op: AddOp::Vpmwt, .. })));
        assert_eq!(count(l, |o| matches!(o, Op71::Add { op: AddOp71::Ftoin, .. })), 2);
        assert_eq!(count42(s, |o| matches!(o, VOp::Add { op: AddOp::Ffloor, .. })), 2);
        /* two ffloor dropped, one vpmwt dropped; no small immediates in a VS */
        assert_eq!(l.ops.len(), s.ops.len() - 3);
        assert_eq!(l.vregs, s.vregs);
        let st = |o: &Op71| matches!(o, Op71::Add { op: AddOp71::Stvpmv, dst: Dst71::None, .. });
        assert_eq!(count(l, st), count42(s, |o| matches!(o, VOp::Add { op: AddOp::Stvpmv, .. })));
        /* each ftoin reads the FMUL the ffloor read, and is stored */
        for op in l.ops {
            if let Op71::Add { op: AddOp71::Ftoin, dst: Dst71::V(d), a: Src71::V(x), .. } = *op {
                assert!(l
                    .ops
                    .iter()
                    .any(|o| matches!(o, Op71::Mul { op: MulOp71::Fmul, dst: Dst71::V(y), .. } if *y == x)));
                assert!(l
                    .ops
                    .iter()
                    .any(|o| matches!(o, Op71::Add { op: AddOp71::Stvpmv, b: Src71::V(y), .. } if *y == d)));
            }
        }
    }

    #[test]
    fn pass_through_vs_and_cs() {
        with_vs(PASS_VS, PASS_FS, false, |s, l| {
            let l = l.expect("lowers");
            vs_common(s, &l);
            /* scaled x y, Zs, 1/Wc, four varyings */
            assert_eq!(count(&l, |o| matches!(o, Op71::Add { op: AddOp71::Stvpmv, .. })), 8);
            /* 1/Wc is the one SFU operation */
            assert_eq!(count(&l, |o| matches!(o, Op71::Sfu { op: Sfu71::Recip, .. })), 1);
        });
        with_vs(PASS_VS, PASS_FS, true, |s, l| {
            let l = l.expect("lowers");
            vs_common(s, &l);
            /* clip x y z w, scaled x y */
            assert_eq!(count(&l, |o| matches!(o, Op71::Add { op: AddOp71::Stvpmv, .. })), 6);
        });
    }

    #[test]
    fn empty_fragment_shader_lowers_to_nothing() {
        let mut buf = std::vec![0u8; 4096];
        let a = Arena::new(&mut buf);
        let sel = super::super::vop71::empty_selected(true);
        let l = select71(&sel, &a).expect("lowers");
        assert!(l.ops.is_empty() && l.fragment && l.vregs == 0);
    }

    #[test]
    fn a_vertex_shader_without_the_viewport_pair_is_refused() {
        let mut buf = std::vec![0u8; 4096];
        let a = Arena::new(&mut buf);
        let sel = super::super::vop71::empty_selected(false);
        assert_eq!(select71(&sel, &a).err(), Some(Select71Error::ViewportXyMissing));
    }

    /// A one-operation fragment shader, lowered.
    fn lower_one(ops: &[VOp], vregs: u32) -> Result<Vec<Op71>, Select71Error> {
        let mut buf = std::vec![0u8; 8192];
        let a = Arena::new(&mut buf);
        let mut sel = super::super::vop71::empty_selected(true);
        sel.ops = std::boxed::Box::leak(ops.to_vec().into_boxed_slice());
        sel.vregs = vregs;
        select71(&sel, &a).map(|l| l.ops.to_vec())
    }

    #[test]
    fn two_small_immediates_become_one_and_a_load() {
        let one = super::super::isa::small_imm_index(0x3f80_0000).unwrap();
        let ops =
            [VOp::Add { op: AddOp::Fadd, dst: Dst::V(V(0)), a: Operand::SmallImm(one), b: Operand::SmallImm(one) }];
        let l = lower_one(&ops, 1).unwrap();
        assert_eq!(
            l,
            [
                Op71::Ldunif { dst: V(1), u: Uniform::constant(0x3f80_0000) },
                Op71::Add { op: AddOp71::Fadd, dst: Dst71::V(V(0)), a: Src71::SmallImm(one), b: Src71::V(V(1)) },
            ]
        );
    }

    #[test]
    fn payload_r5_and_unpacks_take_their_71_form() {
        let ops = [
            VOp::Mul { op: MulOp::Fmul, dst: Dst::V(V(0)), a: Operand::Payload(0), b: Operand::R5 },
            VOp::Add { op: AddOp::Fadd, dst: Dst::V(V(1)), a: Operand::Payload(1), b: Operand::Payload(2) },
            VOp::Mul { op: MulOp::Fmov, dst: Dst::V(V(2)), a: Operand::Abs(V(0)), b: Operand::None },
            VOp::Add { op: AddOp::Fadd, dst: Dst::V(V(3)), a: Operand::Lo(V(2)), b: Operand::Hi(V(2)) },
            VOp::Add { op: AddOp::Recip, dst: Dst::V(V(4)), a: Operand::V(V(3)), b: Operand::None },
        ];
        let l = lower_one(&ops, 5).unwrap();
        assert_eq!(
            l,
            [
                Op71::Mul { op: MulOp71::Fmul, dst: Dst71::V(V(0)), a: Src71::Payload(3), b: Src71::LdvaryC },
                Op71::Add { op: AddOp71::Fadd, dst: Dst71::V(V(1)), a: Src71::Payload(1), b: Src71::Payload(2) },
                Op71::Mul {
                    op: MulOp71::Fmov,
                    dst: Dst71::V(V(2)),
                    a: Src71::Unpacked(V(0), Unpack71::Abs),
                    b: Src71::None
                },
                Op71::Add {
                    op: AddOp71::Fadd,
                    dst: Dst71::V(V(3)),
                    a: Src71::Unpacked(V(2), Unpack71::L),
                    b: Src71::Unpacked(V(2), Unpack71::H)
                },
                Op71::Sfu { op: Sfu71::Recip, dst: V(4), a: Src71::V(V(3)) },
            ]
        );
    }

    #[test]
    fn forms_without_a_71_meaning_are_refused_by_name() {
        let n = Operand::None;
        let refuse = |op: VOp| lower_one(&[op], 4).err();
        assert_eq!(
            refuse(VOp::Add { op: AddOp::LdvpmvOut, dst: Dst::V(V(0)), a: Operand::V(V(1)), b: n }),
            Some(Select71Error::NotOn71("LDVPMV_OUT"))
        );
        assert_eq!(
            refuse(VOp::Add { op: AddOp::Vpmsetup, dst: Dst::Nop, a: Operand::V(V(1)), b: n }),
            Some(Select71Error::NotOn71("VPMSETUP"))
        );
        /* the legacy SFU write, r0..r5 and the VPM address */
        for w in [0, 4, 5, 14, 19, 24, 55] {
            assert!(
                matches!(
                    refuse(VOp::Mul { op: MulOp::Mov, dst: Dst::Magic(w), a: Operand::V(V(1)), b: n }),
                    Some(Select71Error::NotOn71(_))
                ),
                "waddr {w}"
            );
        }
        assert!(matches!(
            refuse(VOp::Add { op: AddOp::Recip, dst: Dst::Magic(33), a: Operand::V(V(1)), b: n }),
            Some(Select71Error::NotOn71(_))
        ));
        assert!(matches!(
            refuse(VOp::Mul { op: MulOp::Fmov, dst: Dst::V(V(0)), a: Operand::Payload(3), b: n }),
            Some(Select71Error::NotOn71(_))
        ));
    }

    #[test]
    fn control_flow_and_flags_are_carried_through() {
        use super::super::isa::{BranchCond, Cond, Msfign, Pf, Uf};
        let ops = [
            VOp::Flags {
                unit: FlagOp::Add(AddOp::Fcmp),
                a: Operand::V(V(0)),
                b: Operand::V(V(1)),
                pf: Pf::PushN,
                uf: Uf::None,
            },
            VOp::FwdBranch { cond: BranchCond::AnyA, msfign: Msfign::None, label: 7 },
            VOp::CondMov { dst: V(0), src: Operand::R5, cond: Cond::IfA },
            VOp::Label(7),
            VOp::LoopHead,
            VOp::Copy { dst: V(1), src: Operand::V(V(0)) },
            VOp::BackEdge { cond: BranchCond::AnyA, msfign: Msfign::None },
            VOp::Setmsf { src: Operand::V(V(0)), cond: Cond::None },
        ];
        let l = lower_one(&ops, 2).unwrap();
        assert_eq!(
            l,
            [
                Op71::Flags {
                    unit: FlagOp71::Add(AddOp71::Fcmp),
                    a: Src71::V(V(0)),
                    b: Src71::V(V(1)),
                    pf: Pf::PushN,
                    uf: Uf::None
                },
                Op71::FwdBranch { cond: BranchCond::AnyA, msfign: Msfign::None, label: Label(7) },
                Op71::CondMov { dst: V(0), src: Src71::LdvaryC, cond: Cond::IfA },
                Op71::Label(Label(7)),
                Op71::LoopHead,
                Op71::Copy { dst: V(1), src: Src71::V(V(0)) },
                Op71::BackEdge { cond: BranchCond::AnyA, msfign: Msfign::None },
                Op71::Setmsf { src: Src71::V(V(0)), cond: Cond::None },
            ]
        );
    }

    #[test]
    fn op_maps_keep_mesas_values_and_names() {
        for op in AddOp::ALL {
            match add_op71(op) {
                Ok(o) => {
                    assert_eq!(o as u32, op as u32);
                    assert_eq!(std::format!("{o:?}"), std::format!("{op:?}"));
                }
                Err(_) => {
                    assert!(matches!(op, AddOp::LdvpmvOut | AddOp::LdvpmdOut | AddOp::LdvpmgOut | AddOp::Vpmsetup))
                }
            }
        }
        for op in MulOp::ALL {
            let o = mul_op71(op);
            assert_eq!(o as u32, op as u32);
            assert_eq!(std::format!("{o:?}"), std::format!("{op:?}"));
        }
    }

    // -----------------------------------------------------------------------
    // interleave71
    // -----------------------------------------------------------------------

    /// The virtual register an operation writes: its definition, or the one
    /// a CondMov or Copy writes.
    fn written(op: &Op71) -> Option<V> {
        match *op {
            Op71::CondMov { dst, .. } | Op71::Copy { dst, .. } => Some(dst),
            _ => op.def(),
        }
    }

    /// An operation whose place carries meaning beyond its registers and the
    /// uniform stream: these keep their order.
    fn ordered(op: &Op71) -> bool {
        match *op {
            Op71::Add { op: o, dst, .. } => !matches!(dst, Dst71::V(_)) || !pure_add71(o),
            Op71::Mul { op: o, dst, .. } => !matches!(dst, Dst71::V(_)) || !pure_mul71(o),
            Op71::Flags { unit: FlagOp71::Add(o), .. } => !(pure_add71(o) || o == AddOp71::Fcmp),
            Op71::Flags { unit: FlagOp71::Mul(o), .. } => !pure_mul71(o),
            Op71::Sfu { .. } | Op71::CondMov { .. } | Op71::Ldunif { .. } => false,
            _ => true,
        }
    }

    /// An operation that reads the uniform stream or may move it.
    fn takes_uniform(op: &Op71) -> bool {
        match *op {
            Op71::Add { dst, .. } | Op71::Mul { dst, .. } => dst_takes_uniform(dst),
            Op71::Ldunif { .. } | Op71::Wrtmuc { .. } => true,
            _ => false,
        }
    }

    /// For each operation of `ops` run in the order `order`, what it sees
    /// (original indices): the writer of each register it reads, the last
    /// implicit rf0 writer before a C read, the last flag push before a
    /// conditional operation.
    fn seen(ops: &[Op71], order: &[u32]) -> Vec<(u32, [u32; 2], u32, u32)> {
        let mut last: std::collections::BTreeMap<u32, u32> = Default::default();
        let (mut rf0, mut flags) = (u32::MAX, u32::MAX);
        let mut out = vec![(0, [0; 2], 0, 0); ops.len()];
        for &i in order {
            let op = &ops[i as usize];
            let reads = op.uses().map(|s| s.vreg().map_or(u32::MAX, |v| *last.get(&v.0).unwrap_or(&u32::MAX)));
            let c = if op.uses().contains(&Src71::LdvaryC) { rf0 } else { u32::MAX };
            let f = if matches!(op, Op71::CondMov { .. } | Op71::Setmsf { .. }) { flags } else { u32::MAX };
            out[i as usize] = (i, reads, c, f);
            if let Some(v) = written(op) {
                last.insert(v.0, i);
            }
            if op.writes_rf0_implicitly() {
                rf0 = i;
            }
            if matches!(op, Op71::Flags { .. }) {
                flags = i;
            }
        }
        out
    }

    /// `order` is a permutation of `ops` that means the same: every read sees
    /// the same writer, every C read the same rf0 writer, every conditional
    /// operation the same push; the ordered operations keep their order and
    /// the barriers their places.
    fn same_meaning(ops: &[Op71], order: &[u32]) {
        let n = ops.len();
        assert_eq!(order.len(), n);
        let mut hit = vec![false; n];
        for &i in order {
            assert!(!std::mem::replace(&mut hit[i as usize], true), "operation {i} twice");
        }
        let id: Vec<u32> = (0..n as u32).collect();
        assert_eq!(seen(ops, order), seen(ops, &id), "a read sees another write");
        let keep = |o: &[u32]| o.iter().copied().filter(|&i| ordered(&ops[i as usize])).collect::<Vec<_>>();
        assert_eq!(keep(order), keep(&id), "the ordered operations moved past each other");
        let pushes = |o: &[u32]| {
            o.iter().copied().filter(|&i| matches!(ops[i as usize], Op71::Flags { .. })).collect::<Vec<_>>()
        };
        assert_eq!(pushes(order), pushes(&id), "the flag pushes changed order");
        let unif = |o: &[u32]| o.iter().copied().filter(|&i| takes_uniform(&ops[i as usize])).collect::<Vec<_>>();
        assert_eq!(unif(order), unif(&id), "the uniform stream changed order");
        for (k, &i) in order.iter().enumerate() {
            if shape(&ops[i as usize], Res { flags: 0, unif: 0, rf0: 0, side: 0 }).barrier {
                assert_eq!(k, i as usize, "a barrier moved");
            }
        }
    }

    /// The fragment shader `fs`, or the vertex and coordinate shaders of
    /// `vs` with `fs`, lowered; each order interleave71 gives them means the
    /// same. Returns how many operations moved, summed.
    fn interleaved(vs: Option<&str>, fs: &str) -> usize {
        let mut moved = 0;
        let mut check = |l: &Selected71<'_>| {
            let mut buf = std::vec![0u8; 1 << 20];
            let a = Arena::new(&mut buf);
            let order = order71(l.ops, l.vregs, &a).expect("scratch");
            same_meaning(l.ops, order);
            moved += order.iter().enumerate().filter(|&(k, &i)| k != i as usize).count();
            let il = interleave71(l, &a).expect("scratch");
            assert_eq!(il.ops.len(), l.ops.len());
            assert_eq!((il.fragment, il.vregs), (l.fragment, l.vregs));
        };
        match vs {
            None => with_fs(fs, |_, l| check(&l.expect("lowers"))),
            Some(vs) => {
                with_vs(vs, fs, false, |_, l| check(&l.expect("lowers")));
                with_vs(vs, fs, true, |_, l| check(&l.expect("lowers")));
            }
        }
        moved
    }

    #[test]
    fn interleaving_keeps_what_every_operation_sees() {
        for fs in [PASS_FS, UNIFORM_FS, IF_FS, TEX_FS, SFU_FS] {
            interleaved(None, fs);
        }
        interleaved(Some(PASS_VS), PASS_FS);
    }

    const MATS_VS: &str = "#version 100
uniform mat4 u_m0, u_m1, u_m2;
attribute vec4 a_pos;
varying vec4 v_a, v_b;
void main() { v_a = u_m0 * a_pos; v_b = u_m1 * a_pos; gl_Position = u_m2 * a_pos; }
";
    const MATS_FS: &str = "#version 100
precision mediump float;
varying vec4 v_a, v_b;
void main() { gl_FragColor = v_a + v_b; }
";

    /// A vertex shader of three independent matrix products: its loads and
    /// products move up, and what each reads stays the same.
    #[test]
    fn interleaving_moves_a_vertex_shaders_loads_and_products() {
        assert!(interleaved(Some(MATS_VS), MATS_FS) > 20);
    }

    /// `ldunif ; fmul ; fadd` of the product, then an independent
    /// `ldunif ; fmul`: the second load goes up into the first product's
    /// word, and the second product into the sum's.
    #[test]
    fn a_load_goes_up_past_a_dependent_sum() {
        let u = Uniform::constant(0x3f80_0000);
        let m =
            |d: u32, x: u32| Op71::Mul { op: MulOp71::Fmul, dst: Dst71::V(V(d)), a: Src71::V(V(x)), b: Src71::V(V(x)) };
        let ops = [
            Op71::Ldunif { dst: V(0), u },
            m(1, 0),
            Op71::Add { op: AddOp71::Fadd, dst: Dst71::V(V(2)), a: Src71::V(V(1)), b: Src71::V(V(1)) },
            Op71::Ldunif { dst: V(3), u },
            m(4, 3),
            Op71::Add { op: AddOp71::Vfpack, dst: Dst71::Tlb, a: Src71::V(V(2)), b: Src71::V(V(4)) },
        ];
        let mut buf = std::vec![0u8; 1 << 16];
        let a = Arena::new(&mut buf);
        let order = order71(&ops, 5, &a).unwrap();
        same_meaning(&ops, order);
        assert_eq!(order, [0, 1, 3, 2, 4, 5]);
    }

    /// An `ldvary`'s C read stays before the next implicit rf0 write, and a
    /// load never passes a TLBU write's uniform.
    #[test]
    fn rf0_and_the_uniform_stream_keep_their_order() {
        let u = Uniform::constant(0);
        let ops = [
            Op71::Ldvary { dst: V(0) },
            Op71::Mul { op: MulOp71::Fmul, dst: Dst71::V(V(1)), a: Src71::V(V(0)), b: Src71::Payload(3) },
            Op71::Add { op: AddOp71::Fadd, dst: Dst71::V(V(2)), a: Src71::V(V(1)), b: Src71::LdvaryC },
            Op71::Ldunif { dst: V(3), u },
            Op71::Mul { op: MulOp71::Mov, dst: Dst71::Tlbu(u), a: Src71::V(V(2)), b: Src71::None },
            Op71::Ldunif { dst: V(4), u },
            Op71::Mul { op: MulOp71::Mov, dst: Dst71::Tlb, a: Src71::V(V(4)), b: Src71::None },
            Op71::Mul { op: MulOp71::Mov, dst: Dst71::Tlb, a: Src71::V(V(3)), b: Src71::None },
        ];
        let mut buf = std::vec![0u8; 1 << 16];
        let a = Arena::new(&mut buf);
        let order = order71(&ops, 5, &a).unwrap();
        same_meaning(&ops, order);
        let pos = |i: u32| order.iter().position(|&x| x == i).unwrap();
        assert!(pos(2) < pos(3), "the C read after the ldunif that overwrites rf0: {order:?}");
        assert!(pos(4) < pos(5), "a load before the TLBU write's uniform: {order:?}");
    }
}
