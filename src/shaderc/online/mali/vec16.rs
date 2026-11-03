// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! A mediump fragment shader whose colour is fp16 vector arithmetic of
//! uniforms and constants under one `if` Mesa flattens (gl_test case 142,
//! unibranch; docs/notes/mali-control-flow.md), as Mesa 26.2.1's NIR holds
//! it once bifrost's passes ran, then selected through `bi_emit_alu`'s rows
//! (emit.rs).
//!
//! The lane walk ([`super::nir_fs::walk`]) takes each IR value as scalar
//! lanes; Mesa does not. Panfrost sets `vectorize_vec2_16bit` and no
//! `lower_to_scalar` (bifrost_compile.h:127), so the GLSL program's vec4
//! operations stay vectors and bifrost's `nir_lower_alu_width` with
//! `bi_vectorize_filter` (width 2 at 16 bits, bifrost_nir.c:106-179) cuts
//! each into the operations of its aligned lane pairs, the chunks. Here,
//! in the order the passes act:
//!
//! 1. the values ([`program`]): `nir_lower_vars_to_ssa` and
//!    `nir_opt_copy_prop` (nir_opt_copy_propagate.c:59-104) make a local
//!    assigned in parts a vector of its lanes' sources (a constructor), a
//!    swizzled read of a constructor a new constructor
//!    (`merge_vec_and_mov`), and a local assigned whole the value itself;
//!    `fmin(fmax(a, 0.0), 1.0)` is `fsat(a)` (nir_opt_algebraic.py);
//! 2. the chunks ([`Prog16::chunk`]): a chunk's source is a constant pair,
//!    a uniform's word, a chunk of an operation, or a constructor's two
//!    lanes, `vec2` of them when they are of two values (a copy, `MKVEC`) and
//!    the value's own pair when copy propagation finds one. A source whose
//!    two lanes are of one value but two words is out of bounds
//!    (`alu_is_swizzled_in_bounds`, nir_lower_alu_width.c:71-86): the chunk
//!    is cut into its lanes, whose `vec2` its readers read;
//!    `nir_opt_cse` makes equal chunks one;
//! 3. the lanes cut alone pair again ([`Prog16::pair`]): `nir_opt_vectorize`
//!    (nir_opt_vectorize.c:650-710), an equal lane before them first being
//!    the same (`nir_opt_cse`), measured on case 142;
//! 4. `fadd(fadd(a, b), c)` of two products `a`, `b` and a `c` that is no
//!    product is `fadd(fadd(a, c), b)` (nir_opt_algebraic.py:2419,
//!    [`Prog16::reassociate`]); the late fusion (:3793-3816,
//!    [`Prog16::fuse`]); `nir_opt_shrink_vectors` of a chunk one lane of
//!    which is read ([`Prog16::shrink`]); `nir_opt_move` of the copies and
//!    the compare to their first reader (nir_opt_move.c:54-130);
//! 5. the selection ([`select`]): each chunk its row, then ATEST and BLEND.
//!
//! The shape is narrow, the one case 142 draws: one `if` at the top of
//! `main` on a compare of a uniform's component and a constant, its sides
//! sums, products, negations and clamps of fp16 uniforms, constants and
//! locals, the colour stored after it. Another program is not this route's
//! ([`program`] answers None) and goes through the lane walk.

use super::bir::{clamp, Cmpf, Idx, Op, Prog, Special, Src, Swz};
use super::emit::Row;
use super::nir::{self, Alu, Kind, NirNode, Val};
use super::nir_fs::{blend, row};
use super::select::{alu, f16_rtne, preloads, UboLayout};
use super::{Defect, MaliError};
use crate::shaderc::online::arena::Arena;
use crate::shaderc::online::ir::{AluOp, IoSlot, Module, Node, Op as IrOp, Src as IrSrc, ValueId};

/// The vector operations one program may hold here.
const MAX_OPS: usize = 96;
/// The chunk instructions one program may hold here.
const MAX_CIS: usize = 192;

/// One lane of a value: a uniform's dword, a constant's bits (fp32), lane
/// `l` of vector operation `i`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum At {
    None,
    Uni(u16),
    Const(u32),
    Op(u8, u8),
}

/// An IR value as NIR holds it after `nir_lower_vars_to_ssa` and copy
/// propagation: a uniform's load (its lanes the dwords from the first), a
/// constant, a constructor of lanes, or a vector operation.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum V {
    None,
    Uni(u16),
    Konst([u32; 4]),
    Ctor([At; 4]),
    Op(u8),
}

const ID: [u8; 4] = [0, 1, 2, 3];

/// An operation's source: a value through a swizzle (a constructor's or a
/// constant's is already permuted, `merge_vec_and_mov`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct VSrc {
    v: V,
    swz: [u8; 4],
}

impl VSrc {
    const NONE: VSrc = VSrc { v: V::None, swz: ID };
}

/// A vector operation of the program: its width, the lanes the colour
/// needs (`live`, bit per lane), whether a side of the `if` holds it.
#[derive(Clone, Copy, Debug)]
struct VOp {
    op: Alu,
    src: [VSrc; 3],
    w: u8,
    live: u8,
    side: bool,
}

impl VOp {
    const NONE: VOp = VOp { op: Alu::Fadd, src: [VSrc::NONE; 3], w: 0, live: 0, side: false };
}

/// A chunk instruction's source: two fp16 constants, a uniform's word read
/// at two of its dwords, or lanes `a` and `b` of instruction `i` (a lane
/// read alone is `a == b`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum S {
    None,
    Const(u16, u16),
    Uni(u16, u16),
    Ci(u16, u8, u8),
}

/// A chunk instruction: an ALU operation of one or two lanes, or a `vec2`
/// of two lanes (a constructor's word, the lanes of a chunk cut alone).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum CiOp {
    Alu(Alu),
    Mk,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Ci {
    op: CiOp,
    src: [S; 3],
    w: u8,
}

impl Ci {
    const NONE: Ci = Ci { op: CiOp::Mk, src: [S::None; 3], w: 0 };
}

/// What a vector operation's chunk became: one instruction of two lanes,
/// its two lanes cut alone, or one lane (a width's odd tail, a compare).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Ch {
    None,
    Vec(u16),
    Sc([u16; 2]),
    One(u16),
}

/// The program of this route.
pub struct Prog16<'s> {
    ops: &'s mut [VOp],
    nops: usize,
    chunks: &'s mut [[Ch; 2]],
    cis: &'s mut [Ci],
    nci: usize,
    /// The instructions in program order.
    order: &'s mut [u16],
    len: usize,
    /// The colour's four lanes, as values, then as sources.
    colour: [At; 4],
    out: [S; 4],
    /// The fp16 uniform block's layout ([`UboLayout::fs16`]).
    pub ubo: UboLayout,
}

/// The values of the IR walk ([`program`]).
struct Walk<'m, 'a, 's> {
    m: &'m Module<'a>,
    vals: &'s mut [V],
    ops: &'s mut [VOp],
    nops: usize,
    locals: [[At; 4]; nir::MAX_LOCALS],
    colour: Option<[At; 4]>,
    /// The `if`'s compare, once walked.
    cond: Option<u8>,
    /// Inside a side of the `if`; an operation stands there.
    side: bool,
    arith: bool,
}

fn lane(v: V, l: u8) -> At {
    let l = l & 3;
    match v {
        V::None => At::None,
        V::Uni(d) => At::Uni(d + u16::from(l)),
        V::Konst(c) => At::Const(c[l as usize]),
        V::Ctor(a) => a[l as usize],
        V::Op(i) => At::Op(i, l),
    }
}

impl Walk<'_, '_, '_> {
    /// A local's lanes as the value a load of `comps` of them is.
    fn load(&self, a: [At; 4], comps: u8) -> V {
        let n = comps as usize;
        if let At::Op(i, 0) = a[0] {
            if self.ops[i as usize].w == comps && (0..n).all(|k| a[k] == At::Op(i, k as u8)) {
                return V::Op(i);
            }
        }
        if let At::Uni(d) = a[0] {
            if (0..n).all(|k| a[k] == At::Uni(d + k as u16)) {
                return V::Uni(d);
            }
        }
        if (0..n).all(|k| matches!(a[k], At::Const(_))) {
            let mut c = [0u32; 4];
            for (k, x) in c.iter_mut().enumerate().take(n) {
                if let At::Const(b) = a[k] {
                    *x = b;
                }
            }
            return V::Konst(c);
        }
        V::Ctor(a)
    }

    fn src(&self, s: IrSrc) -> VSrc {
        let v = self.vals.get(s.val.0 as usize).copied().unwrap_or(V::None);
        match v {
            V::Ctor(_) | V::Konst(_) => {
                let mut a = [At::None; 4];
                for (k, x) in a.iter_mut().enumerate() {
                    *x = lane(v, s.swizzle[k]);
                }
                let v = match v {
                    V::Konst(_) => V::Konst(a.map(|x| if let At::Const(b) = x { b } else { 0 })),
                    _ => V::Ctor(a),
                };
                VSrc { v, swz: ID }
            }
            _ => VSrc { v, swz: s.swizzle },
        }
    }

    fn push(&mut self, op: Alu, src: [VSrc; 3], w: u8) -> Result<u8, MaliError> {
        if self.nops >= self.ops.len() {
            return Err(MaliError::FragmentTooLarge);
        }
        self.ops[self.nops] = VOp { op, src, w, live: 0, side: self.side };
        self.nops += 1;
        Ok((self.nops - 1) as u8)
    }

    /// One instruction; `false` for one this route does not take.
    fn instr(&mut self, id: ValueId) -> Result<bool, MaliError> {
        let Some(op) = self.m.def(id) else { return Ok(false) };
        let w = self.m.comps_of(id).min(4);
        let v = match *op {
            IrOp::ConstF32(b) => V::Konst(b),
            IrOp::LoadUniform { dword_off, .. } => V::Uni(dword_off),
            IrOp::LoadLocal { local, index: None, comps } => {
                let Some(a) = self.locals.get(local.0 as usize) else { return Ok(false) };
                self.load(*a, comps)
            }
            IrOp::StoreLocal { local, index: None, src, write_mask } => {
                let v = self.vals.get(src.val.0 as usize).copied().unwrap_or(V::None);
                let Some(a) = self.locals.get_mut(local.0 as usize) else { return Ok(false) };
                for (k, x) in a.iter_mut().enumerate() {
                    if write_mask & (1 << k) != 0 {
                        *x = lane(v, src.swizzle[k]);
                    }
                }
                return Ok(true);
            }
            IrOp::StoreOutput { slot: IoSlot::FragColor(0), src } if !self.side => {
                let v = self.vals.get(src.val.0 as usize).copied().unwrap_or(V::None);
                self.colour = Some([0, 1, 2, 3].map(|k| lane(v, src.swizzle[k])));
                return Ok(true);
            }
            IrOp::Alu(a, s) => {
                let (x, y) = (self.src(s[0]), self.src(s[1]));
                let konst = |v: VSrc, bits: u32| matches!(v.v, V::Konst(c) if c.iter().all(|&b| b == bits));
                let i = match a {
                    /* a constant operation the front end folds; arithmetic
                     * outside the sides has no reference here */
                    _ if [x, y].iter().take(super::nir_fs::ir_arity(a)).all(|s| matches!(s.v, V::Konst(_))) => {
                        return Ok(false)
                    }
                    AluOp::Fadd | AluOp::Fmul | AluOp::Fneg | AluOp::Fmax | AluOp::Fmin if !self.side => {
                        return Ok(false)
                    }
                    AluOp::Fadd => self.push(Alu::Fadd, [x, y, VSrc::NONE], w)?,
                    AluOp::Fmul => self.push(Alu::Fmul, [x, y, VSrc::NONE], w)?,
                    AluOp::Fneg => self.push(Alu::Fneg, [x, VSrc::NONE, VSrc::NONE], w)?,
                    /* `clamp(a, 0.0, 1.0)`, GLSL's `fmin(fmax(a, 0.0), 1.0)`:
                     * `fsat(a)` (nir_opt_algebraic.py) */
                    AluOp::Fmax if konst(y, 0) => self.push(Alu::Fmax, [x, y, VSrc::NONE], w)?,
                    AluOp::Fmin if konst(y, 0x3f80_0000) => {
                        let V::Op(m) = x.v else { return Ok(false) };
                        let mx = self.ops[m as usize];
                        if mx.op != Alu::Fmax || x.swz != ID {
                            return Ok(false);
                        }
                        self.push(Alu::Fclamp(clamp::ZERO_ONE), [mx.src[0], VSrc::NONE, VSrc::NONE], w)?
                    }
                    /* the `if`'s compare, GLSL's `<=` with its sources swapped */
                    AluOp::Flt if w == 1 && !self.side => self.push(Alu::Fcmp(Cmpf::Lt), [x, y, VSrc::NONE], 1)?,
                    AluOp::Fle if w == 1 && !self.side => self.push(Alu::Fcmp(Cmpf::Ge), [y, x, VSrc::NONE], 1)?,
                    _ => return Ok(false),
                };
                self.arith |= self.side;
                V::Op(i)
            }
            _ => return Ok(false),
        };
        let Some(slot) = self.vals.get_mut(id.0 as usize) else { return Ok(false) };
        *slot = v;
        Ok(true)
    }

    /// A run of nodes: instructions alone (`if`s at the top only).
    fn nodes(&mut self, nodes: &[Node], top: bool) -> Result<bool, MaliError> {
        for (k, n) in nodes.iter().enumerate() {
            let ok = match *n {
                Node::Instr(id) => self.instr(ValueId(id.0))?,
                Node::Return => top && k + 1 == nodes.len(),
                Node::If { cond, then_, else_ } if top && self.cond.is_none() => self.join(cond, then_, else_)?,
                _ => false,
            };
            if !ok {
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// The `if`: both sides, then at the join a select of each local lane
    /// the sides leave different (`nir_opt_peephole_select`'s `bcsel` of
    /// the phi, one vector a local); a lane one side leaves undefined is the
    /// other's (`nir_opt_undef`).
    fn join(
        &mut self,
        cond: IrSrc,
        then_: crate::shaderc::online::ir::Range,
        else_: crate::shaderc::online::ir::Range,
    ) -> Result<bool, MaliError> {
        let Some(V::Op(c)) = self.vals.get(cond.val.0 as usize).copied() else { return Ok(false) };
        if !matches!(self.ops[c as usize].op, Alu::Fcmp(_)) {
            return Ok(false);
        }
        self.cond = Some(c);
        let saved = self.locals;
        self.side = true;
        if !self.nodes(self.m.nodes_of(then_), false)? {
            return Ok(false);
        }
        let then_locals = core::mem::replace(&mut self.locals, saved);
        if !self.nodes(self.m.nodes_of(else_), false)? {
            return Ok(false);
        }
        self.side = false;
        for l in 0..self.locals.len() {
            let (t, e) = (then_locals[l], self.locals[l]);
            if t == e {
                continue;
            }
            if (0..4).any(|k| t[k] != e[k] && t[k] != At::None && e[k] != At::None) {
                let cv = VSrc { v: V::Op(c), swz: [0; 4] };
                let ts = VSrc { v: self.load(t, 4), swz: ID };
                let es = VSrc { v: self.load(e, 4), swz: ID };
                let sel = self.push(Alu::Bcsel, [cv, ts, es], 4)?;
                for k in 0..4 {
                    self.locals[l][k] = if t[k] == e[k] { t[k] } else { At::Op(sel, k as u8) };
                }
            } else {
                for k in 0..4 {
                    self.locals[l][k] = if t[k] == At::None { e[k] } else { t[k] };
                }
            }
        }
        Ok(true)
    }
}

/// The program of a shader this route takes, or None (the lane walk's).
pub fn program<'s>(
    m: &Module<'_>,
    scratch: &Arena<'s>,
    highp: &dyn Fn(&str) -> bool,
) -> Result<Option<Prog16<'s>>, MaliError> {
    let oom = |_| MaliError::OutOfScratch;
    if m.instrs.len() > nir::MAX_VALUES || m.locals.len() > nir::MAX_LOCALS {
        return Ok(None);
    }
    let mut w = Walk {
        m,
        vals: scratch.alloc_filled(m.instrs.len(), V::None).map_err(oom)?,
        ops: scratch.alloc_filled(MAX_OPS, VOp::NONE).map_err(oom)?,
        nops: 0,
        locals: [[At::None; 4]; nir::MAX_LOCALS],
        colour: None,
        cond: None,
        side: false,
        arith: false,
    };
    if !w.nodes(m.nodes_of(m.body), true)? || w.cond.is_none() || !w.arith {
        return Ok(None);
    }
    let Some(colour) = w.colour else { return Ok(None) };
    let mut p = Prog16 {
        ops: w.ops,
        nops: w.nops,
        chunks: scratch.alloc_filled(MAX_OPS, [Ch::None; 2]).map_err(oom)?,
        cis: scratch.alloc_filled(MAX_CIS, Ci::NONE).map_err(oom)?,
        nci: 0,
        order: scratch.alloc_filled(MAX_CIS, 0u16).map_err(oom)?,
        len: 0,
        colour,
        out: [S::None; 4],
        ubo: UboLayout::NONE,
    };
    for a in colour {
        if matches!(a, At::None) {
            return Ok(None);
        }
        p.mark(a);
    }
    /* the fp16 block: the uniforms the colour reads, as the lane walk's
     * nodes tell [`UboLayout::fs16`] */
    let mut g = nir::Nir::new(scratch)?;
    for i in 0..p.nops {
        let o = p.ops[i];
        for (k, s) in o.src.iter().enumerate().take(o.op.arity()) {
            for l in 0..o.w {
                if o.live & (1 << l) == 0 {
                    continue;
                }
                let l = if k == 0 && o.op == Alu::Bcsel { 0 } else { s.swz[l as usize] };
                if let At::Uni(d) = lane(s.v, l) {
                    let nd = NirNode {
                        kind: Kind::Uniform { dword: d, comps: 1 },
                        src: [Val::Undef; 3],
                        bits: 16,
                        width: 1,
                    };
                    if !g.nodes[..g.n].contains(&nd) {
                        g.push_node(nd)?;
                    }
                }
            }
        }
    }
    p.ubo = UboLayout::fs16(m, &g, highp)?;
    /* an `if` whose sides count past bifrost's limit keeps its branch */
    if p.side_count() > nir::PEEPHOLE_LIMIT as usize {
        return Err(MaliError::ControlFlow);
    }
    p.chunk()?;
    p.pair();
    p.dce();
    p.reassociate()?;
    p.dce();
    p.fuse()?;
    p.dce();
    p.shrink();
    p.dce();
    p.move_to_readers();
    Ok(Some(p))
}

fn swz(a: u8, b: u8) -> Swz {
    match (a, b) {
        (0, 0) => Swz::H00,
        (1, 1) => Swz::H11,
        (0, _) => Swz::H01,
        _ => Swz::H10,
    }
}

fn half(bits: u32) -> Result<u16, MaliError> {
    f16_rtne(bits).ok_or(MaliError::Fp16Inexact)
}

impl Prog16<'_> {
    /// Operation `i` is read by sums alone, or by negations read by sums
    /// alone (`is_only_used_by_fadd`, nir_search_helpers.h:593-613).
    fn op_only_fadd(&self, i: u8) -> bool {
        let reads = |s: &VSrc| match s.v {
            V::Op(j) => j == i,
            V::Ctor(a) => a.iter().any(|x| matches!(*x, At::Op(j, _) if j == i)),
            _ => false,
        };
        self.ops[..self.nops].iter().enumerate().filter(|(_, o)| o.live != 0).all(|(j, o)| {
            !o.src.iter().take(o.op.arity()).any(reads)
                || o.op == Alu::Fadd
                || (o.op == Alu::Fneg && self.op_only_fadd(j as u8))
        })
    }

    /// `nir_opt_peephole_select`'s count of the two sides at bifrost's
    /// call (nir_opt_peephole_select.c:243-311, limit 64): one a chunk of
    /// each operation, which the vectors are by then, but for the move-like
    /// negations, `fsat` and a product read by sums alone (fused later).
    /// Case 142 counts 16.
    fn side_count(&self) -> usize {
        (0..self.nops)
            .filter(|&i| {
                let o = self.ops[i];
                o.side
                    && o.live != 0
                    && !matches!(o.op, Alu::Fneg | Alu::Fclamp(_))
                    && !(o.op == Alu::Fmul && self.op_only_fadd(i as u8))
            })
            .map(|i| {
                let o = self.ops[i];
                (0..o.w.div_ceil(2)).filter(|k| o.live >> (2 * k) & 3 != 0).count()
            })
            .sum()
    }

    /// The colour's lane `a` and what it reads, needed ([`VOp::live`]).
    fn mark(&mut self, a: At) {
        let At::Op(i, l) = a else { return };
        let o = self.ops[i as usize];
        if o.live & (1 << l) != 0 {
            return;
        }
        self.ops[i as usize].live |= 1 << l;
        for (k, s) in o.src.iter().enumerate().take(o.op.arity()) {
            let sl = if k == 0 && o.op == Alu::Bcsel { 0 } else { s.swz[l as usize] };
            self.mark(lane(s.v, sl));
        }
    }

    /// An instruction appended to the program, or the equal one before it
    /// (`nir_opt_cse`) when `cse`.
    fn push(&mut self, ci: Ci, cse: bool) -> Result<u16, MaliError> {
        if cse {
            if let Some(&i) = self.order[..self.len].iter().find(|&&i| self.cis[i as usize] == ci) {
                return Ok(i);
            }
        }
        if self.nci >= self.cis.len() {
            return Err(MaliError::FragmentTooLarge);
        }
        self.cis[self.nci] = ci;
        self.order[self.len] = self.nci as u16;
        self.nci += 1;
        self.len += 1;
        Ok((self.nci - 1) as u16)
    }

    /// The `vec2` of lanes `a` and `b` (a copy, `MKVEC.v2i16`).
    fn mk(&mut self, a: S, b: S) -> Result<S, MaliError> {
        let i = self.push(Ci { op: CiOp::Mk, src: [a, b, S::None], w: 2 }, true)?;
        Ok(S::Ci(i, 0, 1))
    }

    /// Uniform dwords `a` and `b` are of one uniform (one `load_ubo`), and
    /// in one word of the fp16 block.
    fn word(&self, a: u16, b: u16) -> Option<bool> {
        let (wa, _, ba) = self.ubo.half(a)?;
        let (wb, _, bb) = self.ubo.half(b)?;
        (ba == bb).then_some(wa == wb)
    }

    /// Lane `a` as a source of one lane.
    fn atom(&self, a: At) -> Result<S, MaliError> {
        Ok(match a {
            At::None => return Err(MaliError::Defect(Defect::IrIndex)),
            At::Const(b) => {
                let h = half(b)?;
                S::Const(h, h)
            }
            At::Uni(d) => S::Uni(d, d),
            At::Op(i, l) => match self.chunks[i as usize][(l / 2) as usize] {
                Ch::Vec(c) => S::Ci(c, l % 2, l % 2),
                Ch::Sc(s) => S::Ci(s[(l % 2) as usize], 0, 0),
                Ch::One(c) => S::Ci(c, 0, 0),
                Ch::None => return Err(MaliError::Defect(Defect::IrIndex)),
            },
        })
    }

    /// Lanes `l1` and `l2` of operation `i` as one source, None out of
    /// bounds.
    fn op_pair(&mut self, i: u8, l1: u8, l2: u8) -> Result<Option<S>, MaliError> {
        if l1 / 2 != l2 / 2 {
            return Ok(None);
        }
        Ok(Some(match self.chunks[i as usize][(l1 / 2) as usize] {
            Ch::Vec(c) => S::Ci(c, l1 % 2, l2 % 2),
            Ch::One(c) => S::Ci(c, 0, 0),
            Ch::Sc(s) => {
                let S::Ci(m, ..) = self.mk(S::Ci(s[0], 0, 0), S::Ci(s[1], 0, 0))? else {
                    return Err(MaliError::Defect(Defect::IrIndex));
                };
                S::Ci(m, l1 % 2, l2 % 2)
            }
            Ch::None => return Err(MaliError::Defect(Defect::IrIndex)),
        }))
    }

    /// Lanes `i` and `j` of `s` as one source of a chunk: None when they
    /// are lanes of one value in two words (out of bounds).
    fn pair_src(&mut self, s: VSrc, i: u8, j: u8) -> Result<Option<S>, MaliError> {
        let (li, lj) = (s.swz[i as usize], s.swz[j as usize]);
        let (a, b) = (lane(s.v, li), lane(s.v, lj));
        Ok(match (a, b) {
            (At::Const(x), At::Const(y)) => Some(S::Const(half(x)?, half(y)?)),
            (At::Uni(x), At::Uni(y)) if self.word(x, y).is_some() => {
                self.word(x, y).filter(|&w| w).map(|_| S::Uni(x, y))
            }
            (At::Op(x, lx), At::Op(y, ly)) if x == y => self.op_pair(x, lx, ly)?,
            /* two values: the constructor's `vec2` (a copy); a uniform's or
             * an operation's lanes read through a swizzle are out of bounds
             * when the pair is no word of it (handled above) */
            _ if matches!(s.v, V::Ctor(_)) => Some(self.mk(self.atom(a)?, self.atom(b)?)?),
            _ => None,
        })
    }

    /// `nir_lower_alu_width` of each operation the colour needs, in
    /// program order: a chunk of two lanes whose every source is in bounds
    /// is one instruction (equal ones one, `nir_opt_cse`), any other two
    /// lanes alone; a vector's odd lane and a compare are one lane.
    fn chunk(&mut self) -> Result<(), MaliError> {
        for i in 0..self.nops {
            let o = self.ops[i];
            if o.live == 0 {
                continue;
            }
            /* an `fmax` the clamp did not take: no reference here */
            if o.op == Alu::Fmax {
                return Err(MaliError::Arithmetic);
            }
            let n = o.op.arity();
            for k in 0..o.w.div_ceil(2) {
                if o.live >> (2 * k) & 3 == 0 {
                    continue;
                }
                let lo = 2 * k;
                if o.w == 1 || lo + 1 >= o.w {
                    let c = self.lane_ci(&o, lo)?;
                    self.chunks[i][k as usize] = Ch::One(c);
                    continue;
                }
                let mut src = [S::None; 3];
                let mut ok = true;
                for (s, x) in src.iter_mut().enumerate().take(n) {
                    let (a, b) = if s == 0 && o.op == Alu::Bcsel { (0, 0) } else { (lo, lo + 1) };
                    let vs = if s == 0 && o.op == Alu::Bcsel { VSrc { swz: [0; 4], ..o.src[0] } } else { o.src[s] };
                    match self.pair_src(vs, a, b)? {
                        Some(v) => *x = v,
                        None => ok = false,
                    }
                }
                self.chunks[i][k as usize] = if ok {
                    Ch::Vec(self.push(Ci { op: CiOp::Alu(o.op), src, w: 2 }, true)?)
                } else {
                    Ch::Sc([self.lane_ci(&o, lo)?, self.lane_ci(&o, lo + 1)?])
                };
            }
        }
        for k in 0..4 {
            self.out[k] = self.atom(self.colour[k])?;
        }
        Ok(())
    }

    /// Lane `l` of operation `o` alone.
    fn lane_ci(&mut self, o: &VOp, l: u8) -> Result<u16, MaliError> {
        let mut src = [S::None; 3];
        for (s, x) in src.iter_mut().enumerate().take(o.op.arity()) {
            let sl = if s == 0 && o.op == Alu::Bcsel { 0 } else { o.src[s].swz[l as usize] };
            *x = self.atom(lane(o.src[s].v, sl))?;
        }
        self.push(Ci { op: CiOp::Alu(o.op), src, w: 1 }, false)
    }

    /// Every read of instruction `x` through `f`.
    fn rewrite(&mut self, x: u16, f: impl Fn(u8) -> (u16, u8)) {
        let map = |s: &mut S| {
            if let S::Ci(c, a, b) = *s {
                if c == x {
                    let (n, a2) = f(a);
                    let (_, b2) = f(b);
                    *s = S::Ci(n, a2, b2);
                }
            }
        };
        for k in 0..self.len {
            let i = self.order[k] as usize;
            for s in self.cis[i].src.iter_mut() {
                map(s);
            }
        }
        for s in self.out.iter_mut() {
            map(s);
        }
    }

    fn remove(&mut self, k: usize) {
        self.order.copy_within(k + 1..self.len, k);
        self.len -= 1;
    }

    /// What `nir_opt_vectorize` compares of a lane source: a constant, or
    /// the value and lane pair (a uniform's load and word, an
    /// instruction).
    fn key(&self, s: S) -> (u8, u16, u16) {
        match s {
            S::Const(..) => (1, 0, 0),
            S::Uni(d, _) => self.ubo.half(d).map_or((2, d, 0), |(w, _, b)| (2, b, w)),
            S::Ci(c, ..) => (3, c, 0),
            S::None => (0, 0, 0),
        }
    }

    /// The lanes cut alone, in program order (`nir_opt_vectorize`,
    /// nir_opt_vectorize.c:650-710, `bi_vectorize_filter`'s width 2): one
    /// equal to a lane before it still alone is that lane (`nir_opt_cse`);
    /// else it pairs with the earliest lane left unpaired whose operation is
    /// the same and whose every source is the same value's lane pair, or a
    /// constant where the other's is one (`instrs_equal`, :226-272), the
    /// two one instruction where the earlier stood (`instr_try_combine_alu`,
    /// :548-612), the later's readers reading its lane 1. Measured on case
    /// 142: `u.y * 0.25` pairs with `u.y * 0.125` of the then side, its
    /// `u.y * 0.5` with the else side's `u.y * 0.25`, and the else side's
    /// `u.y * 0.5` is the then side's (cflow_142_unibranch.expect.json:
    /// `FMA.v2f16 r4, u2.w0.h11, r3, ..` of (0.25, 0.125) and `FMA.v2f16 r5,
    /// u2.w0.h11, r4, ..` of (0.5, 0.25), the else side's `MKVEC.v2i16 r7,
    /// r4.h0, r5.h0` reading the second).
    fn pair(&mut self) {
        let mut pending = [0u16; MAX_CIS];
        let mut np = 0;
        let mut k = 0;
        while k < self.len {
            let x = self.order[k];
            let cx = self.cis[x as usize];
            let CiOp::Alu(op) = cx.op else {
                k += 1;
                continue;
            };
            if cx.w != 1 {
                k += 1;
                continue;
            }
            let n = op.arity();
            let commutes = matches!(op, Alu::Fadd | Alu::Fmul);
            let same = |c: &Ci| {
                c.op == cx.op
                    && c.w == 1
                    && (c.src[..n] == cx.src[..n] || (commutes && c.src[0] == cx.src[1] && c.src[1] == cx.src[0]))
            };
            if let Some(y) = self.order[..k].iter().copied().find(|&y| same(&self.cis[y as usize])) {
                self.rewrite(x, |_| (y, 0));
                self.remove(k);
                continue;
            }
            let class = |p: &Self, c: &Ci| c.op == cx.op && (0..n).all(|s| p.key(c.src[s]) == p.key(cx.src[s]));
            if let Some(j) = (0..np).find(|&j| class(self, &self.cis[pending[j] as usize])) {
                let y = pending[j];
                pending.copy_within(j + 1..np, j);
                np -= 1;
                let cy = self.cis[y as usize];
                let mut src = [S::None; 3];
                for s in 0..n {
                    src[s] = match (cy.src[s], cx.src[s]) {
                        (S::Const(a, _), S::Const(b, _)) => S::Const(a, b),
                        (S::Uni(a, _), S::Uni(b, _)) => S::Uni(a, b),
                        (S::Ci(c, a, _), S::Ci(_, b, _)) => S::Ci(c, a, b),
                        (s, _) => s,
                    };
                }
                self.cis[y as usize] = Ci { src, w: 2, ..cy };
                self.rewrite(x, |_| (y, 1));
                self.remove(k);
                continue;
            }
            pending[np] = x;
            np += 1;
            k += 1;
        }
    }

    /// The lanes of each instruction the colour reads (bit per lane),
    /// through the program backwards.
    fn needed(&self, need: &mut [u8; MAX_CIS]) {
        self.reads(need, false);
    }

    /// [`Prog16::needed`], or with `vec2_whole` each `vec2` the colour reads
    /// at all reading both its operands: what `nir_opt_shrink_vectors` sees,
    /// which leaves a `vec2` read at one lane to copy propagation
    /// (`create_smaller_vec`, nir_opt_shrink_vectors.c:191-200) and shrinks
    /// an operation to the components its readers read (:289-362).
    fn reads(&self, need: &mut [u8; MAX_CIS], vec2_whole: bool) {
        *need = [0; MAX_CIS];
        let use_ = |need: &mut [u8; MAX_CIS], s: S, lanes: u8| {
            if let S::Ci(c, a, b) = s {
                if lanes & 1 != 0 {
                    need[c as usize] |= 1 << a;
                }
                if lanes & 2 != 0 {
                    need[c as usize] |= 1 << b;
                }
            }
        };
        for s in self.out {
            use_(need, s, 1);
        }
        for k in (0..self.len).rev() {
            let i = self.order[k] as usize;
            let c = self.cis[i];
            let lanes = need[i];
            if lanes == 0 {
                continue;
            }
            match c.op {
                CiOp::Mk => {
                    if lanes & 1 != 0 || vec2_whole {
                        use_(need, c.src[0], 1);
                    }
                    if lanes & 2 != 0 || vec2_whole {
                        use_(need, c.src[1], 1);
                    }
                }
                CiOp::Alu(op) => {
                    /* lane k of each source for lane k of the result (a
                     * select's mask reads its compare's lane in both) */
                    let l = if c.w == 1 { 1 } else { lanes };
                    for s in 0..op.arity() {
                        use_(need, c.src[s], l);
                    }
                }
            }
        }
    }

    /// `nir_opt_dce`.
    fn dce(&mut self) {
        let mut need = [0u8; MAX_CIS];
        self.needed(&mut need);
        let mut j = 0;
        for k in 0..self.len {
            if need[self.order[k] as usize] != 0 {
                self.order[j] = self.order[k];
                j += 1;
            }
        }
        self.len = j;
    }

    /// The readers of instruction `x` in the program.
    fn readers(&self, x: u16) -> impl Iterator<Item = u16> + '_ {
        self.order[..self.len]
            .iter()
            .copied()
            .filter(move |&i| self.cis[i as usize].src.iter().any(|s| matches!(*s, S::Ci(c, ..) if c == x)))
    }

    fn alu_op(&self, s: S) -> Option<(u16, Ci)> {
        let S::Ci(c, ..) = s else { return None };
        let ci = self.cis[c as usize];
        matches!(ci.op, CiOp::Alu(_)).then_some((c, ci))
    }

    /// `is_fmul` (nir_search_helpers.h:403-416): a product, or the negation
    /// of one.
    fn is_fmul(&self, s: S) -> bool {
        match self.alu_op(s) {
            Some((_, Ci { op: CiOp::Alu(Alu::Fmul), .. })) => true,
            Some((_, Ci { op: CiOp::Alu(Alu::Fneg), src, .. })) => self.is_fmul(src[0]),
            _ => false,
        }
    }

    /// A read of instruction `c` whole: both its lanes in order, or its one.
    fn whole(&self, s: S, w: u8) -> Option<u16> {
        match s {
            S::Ci(c, 0, b) if self.cis[c as usize].w == w && (w == 1 || b == 1) => Some(c),
            _ => None,
        }
    }

    /// nir_opt_algebraic.py:2419, "Reassociate add chains for more
    /// MAD/FMA-friendly code": `fadd(fadd(a, b), c)`, the inner sum used
    /// once, `a` and `b` products (`is_fmul`) and `c` no product, is
    /// `fadd(fadd(a, c), b)`, the new sum before the old one's place
    /// (nir_search.c:687-714), each `fadd` matched both ways round. Case
    /// 142's `b * 0.5 - a * 0.25 + a.wzyx * 0.125` where the last product's
    /// pair was cut alone: `FMA.v2f16 r1, r1^, 0x38000000.h11, r3^` then
    /// `FMA.v2f16 r1, r2^.neg, 0x34000000.h11, r1^`.
    fn reassociate(&mut self) -> Result<(), MaliError> {
        let mut k = self.len;
        while k > 0 {
            k -= 1;
            let z = self.order[k];
            let cz = self.cis[z as usize];
            if cz.op != CiOp::Alu(Alu::Fadd) {
                continue;
            }
            let mut done = None;
            'outer: for comb in 0..2 {
                let (o, c) = (cz.src[comb], cz.src[1 - comb]);
                let Some(inner) = self.whole(o, cz.w) else { continue };
                let ci = self.cis[inner as usize];
                if ci.op != CiOp::Alu(Alu::Fadd) || self.readers(inner).count() != 1 || self.is_fmul(c) {
                    continue;
                }
                for ic in 0..2 {
                    let (a, b) = (ci.src[ic], ci.src[1 - ic]);
                    if self.is_fmul(a) && self.is_fmul(b) {
                        done = Some((a, b, c));
                        break 'outer;
                    }
                }
            }
            let Some((a, b, c)) = done else { continue };
            let x = self.insert(k, Ci { op: CiOp::Alu(Alu::Fadd), src: [a, c, S::None], w: cz.w })?;
            let full = S::Ci(x, 0, if cz.w == 2 { 1 } else { 0 });
            self.cis[z as usize] = Ci { src: [full, b, S::None], ..cz };
        }
        Ok(())
    }

    /// An instruction inserted before program position `k`.
    fn insert(&mut self, k: usize, ci: Ci) -> Result<u16, MaliError> {
        if self.nci >= self.cis.len() || self.len >= self.order.len() {
            return Err(MaliError::FragmentTooLarge);
        }
        self.cis[self.nci] = ci;
        self.order.copy_within(k..self.len, k + 1);
        self.order[k] = self.nci as u16;
        self.nci += 1;
        self.len += 1;
        Ok((self.nci - 1) as u16)
    }

    /// `is_only_used_by_fadd` (nir_search_helpers.h:593-613).
    fn only_fadd(&self, x: u16) -> bool {
        self.readers(x).all(|u| match self.cis[u as usize].op {
            CiOp::Alu(Alu::Fadd) => true,
            CiOp::Alu(Alu::Fneg) => self.only_fadd(u),
            _ => false,
        })
    }

    /// The late fusion (nir_opt_algebraic.py:3793-3816), the last
    /// instruction first: `fadd(fmul(a, b), c)` is `ffma(a, b, c)`, then
    /// `fadd(fneg(fmul(a, b)), c)` is `ffma(fneg(a), b, c)`, the products
    /// read by sums alone, each rule trying the sum's first source first.
    fn fuse(&mut self) -> Result<(), MaliError> {
        let mut k = self.len;
        while k > 0 {
            k -= 1;
            let z = self.order[k];
            let cz = self.cis[z as usize];
            if cz.op != CiOp::Alu(Alu::Fadd) {
                continue;
            }
            let product = |p: &Self, s: S| {
                let m = p.whole(s, cz.w)?;
                (p.cis[m as usize].op == CiOp::Alu(Alu::Fmul) && p.only_fadd(m)).then_some(m)
            };
            let mut fused = false;
            for comb in 0..2 {
                if let Some(m) = product(self, cz.src[comb]) {
                    let fm = self.cis[m as usize];
                    self.cis[z as usize] =
                        Ci { op: CiOp::Alu(Alu::Ffma), src: [fm.src[0], fm.src[1], cz.src[1 - comb]], w: cz.w };
                    fused = true;
                    break;
                }
            }
            if fused {
                continue;
            }
            for comb in 0..2 {
                let Some(g) = self.whole(cz.src[comb], cz.w) else { continue };
                let cg = self.cis[g as usize];
                if cg.op != CiOp::Alu(Alu::Fneg) || !self.only_fadd(g) {
                    continue;
                }
                let Some(m) = product(self, cg.src[0]) else { continue };
                let fm = self.cis[m as usize];
                /* the new fneg stands where the sum did */
                let n = self.insert(k, Ci { op: CiOp::Alu(Alu::Fneg), src: [fm.src[0], S::None, S::None], w: cz.w })?;
                let full = S::Ci(n, 0, if cz.w == 2 { 1 } else { 0 });
                self.cis[z as usize] =
                    Ci { op: CiOp::Alu(Alu::Ffma), src: [full, fm.src[1], cz.src[1 - comb]], w: cz.w };
                break;
            }
        }
        Ok(())
    }

    /// `nir_opt_shrink_vectors` (bifrost_nir.c:286): an instruction of two
    /// lanes one of which is read is one lane, its sources that lane's, a
    /// `vec2`'s lane its operand itself (copy propagation).
    fn shrink(&mut self) {
        let mut need = [0u8; MAX_CIS];
        self.reads(&mut need, true);
        for k in 0..self.len {
            let i = self.order[k] as usize;
            let c = self.cis[i];
            let (CiOp::Alu(_), 2) = (c.op, c.w) else { continue };
            let l = match need[i] {
                1 => 0,
                2 => 1,
                _ => continue,
            };
            let mut src = c.src;
            for s in src.iter_mut() {
                *s = match *s {
                    S::Const(a, b) => {
                        let h = if l == 0 { a } else { b };
                        S::Const(h, h)
                    }
                    S::Uni(a, b) => {
                        let d = if l == 0 { a } else { b };
                        S::Uni(d, d)
                    }
                    S::Ci(x, a, b) => {
                        let y = if l == 0 { a } else { b };
                        let cx = self.cis[x as usize];
                        if cx.op == CiOp::Mk {
                            cx.src[y as usize]
                        } else {
                            S::Ci(x, y, y)
                        }
                    }
                    S::None => S::None,
                };
            }
            self.cis[i] = Ci { src, w: 1, ..c };
            if l == 1 {
                self.rewrite(i as u16, |_| (i as u16, 0));
            }
        }
    }

    /// `nir_opt_move` with `nir_move_copies` and `nir_move_comparisons`
    /// (bifrost_nir.c:375-381, nir_opt_move.c:54-130) over the one block:
    /// walked backwards, each `vec2` and the compare move to just before
    /// their first reader; one already there keeps its own index, so an
    /// earlier one that shares the reader goes before it.
    fn move_to_readers(&mut self) {
        let mut index = [0u32; MAX_CIS];
        let mut next = 1u32;
        let mut pos = self.len;
        while pos > 0 {
            pos -= 1;
            let id = self.order[pos];
            index[id as usize] = next;
            next += 1;
            let movable = matches!(self.cis[id as usize].op, CiOp::Mk | CiOp::Alu(Alu::Fcmp(_)));
            if !movable {
                continue;
            }
            let first = (pos + 1..self.len)
                .find(|&q| self.cis[self.order[q] as usize].src.iter().any(|s| matches!(*s, S::Ci(c, ..) if c == id)));
            let Some(mut f) = first else { continue };
            let fi = index[self.order[f] as usize];
            while f > pos + 1 && index[self.order[f - 1] as usize] == fi {
                f -= 1;
            }
            if f == pos + 1 {
                continue;
            }
            self.order.copy_within(pos + 1..f, pos);
            self.order[f - 1] = id;
            index[id as usize] = fi;
        }
    }
}

/// [`program`]'s instructions into `p`, each its row, then ATEST on the
/// alpha and the colour's words to BLEND (as [`super::nir_fs::select`]
/// ends). The rows drawn are noted in `rows`.
pub fn select(p: &mut Prog<'_>, g: &Prog16<'_>, rows: &mut u128) -> Result<(), MaliError> {
    let defect = MaliError::Defect(Defect::IrIndex);
    let mut val = [None::<u16>; MAX_CIS];
    let ubo = g.ubo;
    let src = |val: &[Option<u16>; MAX_CIS], s: S, w: u8| -> Result<Src, MaliError> {
        Ok(match s {
            S::Const(a, b) if w == 2 => Src::of(Idx::Const(u32::from(a) | u32::from(b) << 16)),
            S::Const(a, _) => Src::of(Idx::Const(u32::from(a))).swz(Swz::H00),
            S::Uni(a, b) => {
                let (w, ha, base) = ubo.half(a).ok_or(MaliError::FsUniform)?;
                let (_, hb, _) = ubo.half(b).ok_or(MaliError::FsUniform)?;
                Src::of(Idx::Ubo { base, comp: (w - base) as u8 }).swz(swz(ha, hb))
            }
            S::Ci(c, a, b) => Src::ssa(val[c as usize].ok_or(defect)?).swz(swz(a, b)),
            S::None => Src::NULL,
        })
    };
    for &i in &g.order[..g.len] {
        let c = g.cis[i as usize];
        let r = match c.op {
            CiOp::Mk => {
                let s = [src(&val, c.src[0], 1)?, src(&val, c.src[1], 1)?];
                row(p, rows, Row { op: Alu::Vec2, bits: 16, width: 2 }, &s)?
            }
            CiOp::Alu(op) => {
                let mut s = [Src::NULL; 3];
                for (k, x) in s.iter_mut().enumerate().take(op.arity()) {
                    *x = src(&val, c.src[k], c.w)?;
                }
                /* a select's mask is its compare's low half in both lanes */
                if op == Alu::Bcsel {
                    s[0] = s[0].swz(Swz::H00);
                }
                row(p, rows, Row { op, bits: 16, width: c.w }, &s)?
            }
        };
        val[i as usize] = Some(r);
    }
    /* ATEST on the alpha: a constant's half, or the lane's */
    let at = src(&val, g.out[3], 1)?;
    let t = alu(p, Op::Atest, &[Src::of(Idx::Reg(60)), at, Src::of(Idx::Special(Special::AtestDatum, false))])?;
    let mut words = [Src::NULL; 2];
    for (k, w) in words.iter_mut().enumerate() {
        let (a, b) = (g.out[2 * k], g.out[2 * k + 1]);
        *w = match (a, b) {
            (S::Const(x, _), S::Const(y, _)) => Src::of(Idx::Const(u32::from(x) | u32::from(y) << 16)),
            /* both lanes of one instruction: its word */
            (S::Ci(c, 0, 0), S::Ci(d, 1, 1)) if c == d && g.cis[c as usize].w == 2 => {
                Src::ssa(alu(p, Op::MovI32, &[src(&val, S::Ci(c, 0, 1), 2)?])?)
            }
            _ => {
                let s = [src(&val, a, 1)?, src(&val, b, 1)?];
                Src::ssa(row(p, rows, Row { op: Alu::Vec2, bits: 16, width: 2 }, &s)?)
            }
        };
    }
    let col = alu(p, Op::Collect, &words)?;
    blend(p, 0, col, t, 2, false, false)?;
    preloads(p)
}
