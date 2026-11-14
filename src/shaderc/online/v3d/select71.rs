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
}
