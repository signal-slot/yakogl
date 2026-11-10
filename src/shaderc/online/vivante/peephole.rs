// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Two rewrites of the selected program that only this machine's encoding
//! makes possible, run by [`super::alloc::eliminate_dead`]
//! (`docs/notes/gc-shadow-refract-shaders.md`, causes C1 and C2).
//!
//! **A copy read through the reader's own source fields** ([`fold_copies`]).
//! Negate and absolute are `SRC_NEG` and `SRC_ABS` of every source, and a
//! source has its own swizzle. So `mov t, -s` followed by `add d, a, t` is
//! `add d, a, -s`, and `mov t.xyz_, s.xyzx` followed by `mul d, u, t.xxxx` is
//! `mul d, u, s.xxxx`. Selection's `propagate_copies` declines both shapes (a
//! modifier, a partial mask); here they are folded where every lane the reader
//! consumes provably holds the copy's value. The copy itself, once nobody
//! reads it, is deleted by the dead-code pass that runs next.
//!
//! **One scalar instruction for lanes that read one value**
//! ([`merge_scalar_lanes`]). Selection emits a scalar operation once per lane
//! (`select.rs`, "emitted once per lane"), so `a / a.w` is four
//! `rcp t.x/y/z/w, s.wwww`. The machine computes the scalar from the first
//! swizzle lane and writes it to every lane of the mask (the same comment: a
//! `sin` of a vec4 emitted once was sin(u.x) in all four lanes), so adjacent
//! copies that read the same lane are one instruction with the union of the
//! masks, as Mesa's etnaviv writes them.
//!
//! Both work on straight-line code only and leave a program alone where a
//! branch could arrive in between.

use super::alloc::{consumed_lanes, SelDst, SelInstr, SelSrc, VReg};
use super::build::Src;

/// The operations a negate or absolute may be moved into: float arithmetic
/// that applies the modifiers to the value it reads, as `Fneg`/`Fabs` mean
/// them. Not `select` or `cmp` (they pass a source through as bits), not the
/// texture or memory operations.
fn takes_float_modifiers(s: &SelInstr) -> bool {
    s.ty == super::alloc::OpType::F32
        && matches!(
            s.op.op().name,
            "mov" | "add" | "mul" | "mad" | "dp2" | "dp3" | "dp4" | "rcp" | "rsq" | "sqrt" | "floor" | "ceil" | "frc"
        )
}

/// The scalar operations [`merge_scalar_lanes`] joins: one value from the
/// first swizzle lane, written to every lane of the mask. Not `sin`, `cos`
/// and `log`, which write a pair of lanes on this core
/// (`tests/common/mod.rs`), nor the two-source `div` and `imullo0`.
fn splats(s: &SelInstr) -> bool {
    matches!(s.op.op().name, "rcp" | "rsq" | "sqrt" | "exp")
}

fn writes_virtual(s: &SelInstr, v: VReg) -> u8 {
    match s.dst {
        Some(SelDst::Virtual { v: d, write_mask }) if d == v => write_mask,
        _ => 0,
    }
}

fn mask_of(s: &SelInstr) -> u8 {
    match s.dst {
        Some(SelDst::Virtual { write_mask, .. }) | Some(SelDst::Fixed { write_mask, .. }) => write_mask,
        Some(SelDst::Address { write_mask }) => write_mask,
        None => 0xf,
    }
}

/// Mark in `targets` (one bit an instruction, cleared first) every
/// instruction of `sel` a branch goes to.
pub fn mark_targets(sel: &[SelInstr], targets: &mut [u64]) {
    targets.fill(0);
    for s in sel {
        if let Some(t) = s.target.map(usize::from).filter(|&t| t < sel.len()) {
            targets[t / 64] |= 1 << (t % 64);
        }
    }
}

fn is_target(targets: &[u64], j: usize) -> bool {
    targets.get(j / 64).is_some_and(|w| w >> (j % 64) & 1 != 0)
}

/// How far past a copy [`fold_copies`] looks for its readers.
const FOLD_REACH: usize = 512;

/// The copy's source read through `outer` with the reader's own modifiers:
/// the swizzles composed, and the magnitude taken before the negate, as the
/// hardware applies them (`|x|` first, then `-`).
fn through(copy: SelSrc, outer: [u8; 4], neg_r: bool, abs_r: bool) -> SelSrc {
    let pick = |base: [u8; 4]| core::array::from_fn(|c| base[outer[c] as usize & 3]);
    let mods = |neg_m: bool, abs_m: bool| if abs_r { (neg_r, true) } else { (neg_r ^ neg_m, abs_m) };
    match copy {
        SelSrc::Virtual { v, swizzle, neg, abs } => {
            let (neg, abs) = mods(neg, abs);
            SelSrc::Virtual { v, swizzle: pick(swizzle), neg, abs }
        }
        SelSrc::Input { reg, swizzle, neg, abs } => {
            let (neg, abs) = mods(neg, abs);
            SelSrc::Input { reg, swizzle: pick(swizzle), neg, abs }
        }
        other => other,
    }
}

/// Does `s` write any lane of the register the copy source `src` reads?
fn writes_source(s: &SelInstr, src: SelSrc) -> bool {
    match (src, s.dst) {
        (SelSrc::Virtual { v, .. }, Some(SelDst::Virtual { v: d, .. })) => d == v,
        /* An input and a fixed destination are the same temporary file: the
         * fragment stage's gl_FragDepth is written to r0.z, the register
         * gl_FragCoord arrives in (select.rs FS_DEPTH_REG), and selection's
         * copy of r0 must not be read through past that write
         * (tests/es3_fragdepth_gc.rs). */
        (SelSrc::Input { reg, .. }, Some(SelDst::Fixed { reg: d, .. })) => d == reg,
        /* Any other output written over an input is placed by the allocator
         * around the input's reads. */
        _ => false,
    }
}

/// Fold each `mov` of a temporary or an input into the readers that follow it
/// in straight-line code (module documentation). Returns how many source
/// operands were rewritten.
///
/// A reader is rewritten only when every lane of the copy it consumes
/// ([`consumed_lanes`] over its whole write mask, so a superset) was written
/// by the copy and not since, the copy's source register has not been written
/// since, and no branch leaves or arrives in between. A copy that carries a
/// negate or an absolute goes only into [`takes_float_modifiers`] readers.
pub fn fold_copies(sel: &mut [SelInstr], targets: &[u64]) -> usize {
    let mut rewritten = 0usize;
    for i in 0..sel.len() {
        let m = sel[i];
        if !m.op.is_mov() || m.cond.is_some() || m.saturate || m.rtz || m.target.is_some() {
            continue;
        }
        let Some(SelDst::Virtual { v: t, write_mask }) = m.dst else { continue };
        let Some(src) = m.srcs[2] else { continue };
        let modified = match src {
            SelSrc::Virtual { v, neg, abs, .. } if v != t => neg || abs,
            SelSrc::Input { neg, abs, .. } => neg || abs,
            _ => continue,
        };
        if modified && m.ty != super::alloc::OpType::F32 {
            continue;
        }
        /* The lanes of `t` still holding the copy's value. */
        let mut held = write_mask;
        for k in i + 1..sel.len().min(i + 1 + FOLD_REACH) {
            if is_target(targets, k) {
                break;
            }
            let r = sel[k];
            let eff = mask_of(&r);
            if !modified || takes_float_modifiers(&r) {
                let mut out = r;
                let mut changed = false;
                for slot in 0..3 {
                    let Some(SelSrc::Virtual { v, swizzle, neg, abs }) = r.srcs[slot] else { continue };
                    if v != t {
                        continue;
                    }
                    let lanes = consumed_lanes(&r, swizzle, if eff == 0 { 0xf } else { eff });
                    if lanes & !held != 0 {
                        continue;
                    }
                    out.srcs[slot] = Some(through(src, swizzle, neg, abs));
                    changed = true;
                    rewritten += 1;
                }
                if changed {
                    sel[k] = out;
                }
            }
            /* What `r` writes ends the copy for those lanes, and a write of
             * the source ends it altogether; so does a branch. */
            held &= !writes_virtual(&r, t);
            if held == 0 || writes_source(&r, src) || r.target.is_some() {
                break;
            }
        }
    }
    rewritten
}

/// Two reads that give a scalar operation the same value: the same register
/// and modifiers, the same first swizzle lane (the only one it reads).
fn same_scalar_read(a: Option<SelSrc>, b: Option<SelSrc>) -> bool {
    match (a, b) {
        (None, None) => true,
        (
            Some(SelSrc::Virtual { v: va, swizzle: sa, neg: na, abs: aa }),
            Some(SelSrc::Virtual { v: vb, swizzle: sb, neg: nb, abs: ab }),
        ) => va == vb && sa[0] == sb[0] && na == nb && aa == ab,
        (
            Some(SelSrc::Input { reg: ra, swizzle: sa, neg: na, abs: aa }),
            Some(SelSrc::Input { reg: rb, swizzle: sb, neg: nb, abs: ab }),
        ) => ra == rb && sa[0] == sb[0] && na == nb && aa == ab,
        (Some(SelSrc::Physical(pa)), Some(SelSrc::Physical(pb))) => {
            if pa.is_immediate() || pb.is_immediate() {
                pa == pb
            } else {
                Src { swizzle: [0; 4], ..pa } == Src { swizzle: [0; 4], ..pb } && pa.swizzle[0] == pb.swizzle[0]
            }
        }
        _ => false,
    }
}

/// Join adjacent copies of one scalar operation that write disjoint lanes of
/// one temporary from the same value (module documentation). The later
/// instruction of each pair is given an empty write mask, which is how the
/// caller recognises it for deletion. Returns how many were emptied.
///
/// A copy is joined only when no branch arrives at it, and never when it
/// reads the register it writes (the earlier copy may have changed the lane).
pub fn merge_scalar_lanes(sel: &mut [SelInstr], targets: &[u64]) -> usize {
    let mut merged = 0usize;
    let mut lead = 0usize;
    for j in 1..sel.len() {
        let (a, b) = (sel[lead], sel[j]);
        let join = splats(&a)
            && a.op.0 == b.op.0
            && a.ty == b.ty
            && a.cond.is_none()
            && b.cond.is_none()
            && a.saturate == b.saturate
            && a.rtz == b.rtz
            && a.target.is_none()
            && b.target.is_none()
            && !is_target(targets, j)
            && match (a.dst, b.dst) {
                (Some(SelDst::Virtual { v: va, write_mask: ma }), Some(SelDst::Virtual { v: vb, write_mask: mb })) => {
                    va == vb
                        && ma & mb == 0
                        && !a
                            .srcs
                            .iter()
                            .chain(b.srcs.iter())
                            .flatten()
                            .any(|s| matches!(*s, SelSrc::Virtual { v, .. } if v == va))
                }
                _ => false,
            }
            && (0..3).all(|n| same_scalar_read(a.srcs[n], b.srcs[n]));
        if join {
            if let (Some(SelDst::Virtual { v, write_mask: ma }), Some(SelDst::Virtual { write_mask: mb, .. })) =
                (a.dst, b.dst)
            {
                sel[lead].dst = Some(SelDst::Virtual { v, write_mask: ma | mb });
                sel[j].dst = Some(SelDst::Virtual { v, write_mask: 0 });
                merged += 1;
            }
        } else {
            lead = j;
        }
    }
    merged
}

#[cfg(test)]
mod tests {
    use super::super::alloc::OpType;
    use super::super::build::op_id;
    use super::*;

    fn v(k: u32, swizzle: [u8; 4], neg: bool) -> SelSrc {
        SelSrc::Virtual { v: VReg(k), swizzle, neg, abs: false }
    }
    fn d(k: u32, write_mask: u8) -> SelDst {
        SelDst::Virtual { v: VReg(k), write_mask }
    }

    #[test]
    fn a_negated_copy_is_read_through_the_adders_modifier() {
        let (mov, add) = (op_id("mov").unwrap(), op_id("add").unwrap());
        let mut sel = [
            SelInstr::new(mov, OpType::F32).dst(d(1, 0xf)).operand(0, v(0, [3, 2, 1, 0], true)),
            SelInstr::new(add, OpType::F32)
                .dst(d(2, 0xf))
                .operand(0, v(5, [0, 1, 2, 3], false))
                .operand(1, v(1, [0, 0, 1, 1], false)),
        ];
        assert_eq!(fold_copies(&mut sel, &[0]), 1);
        match sel[1].srcs[2] {
            Some(SelSrc::Virtual { v, swizzle, neg, abs }) => {
                assert_eq!((v, swizzle, neg, abs), (VReg(0), [3, 3, 2, 2], true, false));
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_negated_copy_stays_for_a_select_and_after_its_source_changes() {
        let (mov, add, sel_op) = (op_id("mov").unwrap(), op_id("add").unwrap(), op_id("select").unwrap());
        let mut sel = [
            SelInstr::new(mov, OpType::F32).dst(d(1, 0xf)).operand(0, v(0, [0, 1, 2, 3], true)),
            SelInstr::new(sel_op, OpType::F32)
                .dst(d(3, 0xf))
                .operand(0, v(1, [0, 1, 2, 3], false))
                .operand(1, v(4, [0, 1, 2, 3], false))
                .operand(2, v(4, [0, 1, 2, 3], false)),
            SelInstr::new(add, OpType::F32)
                .dst(d(0, 0x1))
                .operand(0, v(4, [0; 4], false))
                .operand(1, v(4, [0; 4], false)),
            SelInstr::new(add, OpType::F32)
                .dst(d(2, 0xf))
                .operand(0, v(1, [0, 1, 2, 3], false))
                .operand(1, v(4, [0; 4], false)),
        ];
        assert_eq!(fold_copies(&mut sel, &[0]), 0, "a select passes bits; v0 was written before the add");
    }

    #[test]
    fn a_copy_of_an_input_stays_past_a_fixed_write_of_its_register() {
        let (mov, add) = (op_id("mov").unwrap(), op_id("add").unwrap());
        let r0 = SelSrc::Input { reg: 0, swizzle: [0, 1, 2, 3], neg: false, abs: false };
        let mut sel = [
            SelInstr::new(mov, OpType::F32).dst(d(1, 0xf)).operand(0, r0),
            SelInstr::new(mov, OpType::F32)
                .dst(SelDst::Fixed { reg: 0, write_mask: 0x4 })
                .operand(0, v(4, [0; 4], false)),
            SelInstr::new(add, OpType::F32)
                .dst(d(2, 0xf))
                .operand(0, v(1, [2, 2, 2, 2], false))
                .operand(1, v(4, [0; 4], false)),
        ];
        assert_eq!(fold_copies(&mut sel, &[0]), 0, "r0.z was written (gl_FragDepth) before the add reads the copy");
    }

    #[test]
    fn a_partial_copy_folds_only_for_the_lanes_it_wrote() {
        let (mov, mul) = (op_id("mov").unwrap(), op_id("mul").unwrap());
        let mut sel = [
            SelInstr::new(mov, OpType::F32).dst(d(1, 0x7)).operand(0, v(0, [0, 1, 2, 0], false)),
            SelInstr::new(mul, OpType::F32)
                .dst(d(2, 0xf))
                .operand(0, v(1, [1, 1, 1, 1], false))
                .operand(1, v(6, [0, 1, 2, 3], false)),
            SelInstr::new(mul, OpType::F32)
                .dst(d(3, 0xf))
                .operand(0, v(1, [3, 3, 3, 3], false))
                .operand(1, v(6, [0, 1, 2, 3], false)),
        ];
        assert_eq!(fold_copies(&mut sel, &[0]), 1, "the .wwww read is not the copy's");
        assert!(matches!(sel[1].srcs[0], Some(SelSrc::Virtual { v: VReg(0), swizzle: [1, 1, 1, 1], .. })));
        assert!(matches!(sel[2].srcs[0], Some(SelSrc::Virtual { v: VReg(1), .. })));
    }

    #[test]
    fn four_reciprocals_of_one_lane_are_one() {
        let rcp = op_id("rcp").unwrap();
        let mut sel: [SelInstr; 4] = core::array::from_fn(|l| {
            SelInstr::new(rcp, OpType::F32).dst(d(1, 1 << l)).operand(0, v(0, [3, 3, 3, 3], false))
        });
        assert_eq!(merge_scalar_lanes(&mut sel, &[0]), 3);
        assert!(matches!(sel[0].dst, Some(SelDst::Virtual { write_mask: 0xf, .. })));
        assert!(sel[1..].iter().all(|s| matches!(s.dst, Some(SelDst::Virtual { write_mask: 0, .. }))));
    }

    #[test]
    fn reciprocals_of_different_lanes_or_a_pair_writer_stay() {
        let (rcp, sin) = (op_id("rcp").unwrap(), op_id("sin").unwrap());
        let mut sel: [SelInstr; 2] = core::array::from_fn(|l| {
            SelInstr::new(rcp, OpType::F32).dst(d(1, 1 << l)).operand(0, v(0, [l as u8; 4], false))
        });
        assert_eq!(merge_scalar_lanes(&mut sel, &[0]), 0);
        let mut sel: [SelInstr; 2] =
            core::array::from_fn(|l| SelInstr::new(sin, OpType::F32).dst(d(1, 1 << l)).operand(0, v(0, [0; 4], false)));
        assert_eq!(merge_scalar_lanes(&mut sel, &[0]), 0);
        let mut sel: [SelInstr; 2] =
            core::array::from_fn(|l| SelInstr::new(rcp, OpType::F32).dst(d(0, 2 << l)).operand(0, v(0, [0; 4], false)));
        assert_eq!(merge_scalar_lanes(&mut sel, &[0]), 0, "reads the register it writes");
    }
}
