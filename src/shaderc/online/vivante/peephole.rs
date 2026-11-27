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
    s.ty == super::alloc::OpType::F32 && float_modifier_op(s.op.op().name)
}

/// The operations, by name, of [`takes_float_modifiers`]: selection asks the
/// same question before it hands an operand with a modifier to an
/// instruction (select.rs `Sel::plain`).
pub(crate) fn float_modifier_op(name: &str) -> bool {
    matches!(
        name,
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
        /* Whether the copy is the only instruction anywhere that writes `t`
         * (asked once, at the first reader that needs it): then the lanes
         * outside its mask are written by nothing, and a reader that consumes
         * them reads an undefined value whichever register it reads. */
        let mut sole_writer: Option<bool> = None;
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
                    let mut swizzle = swizzle;
                    if lanes & !held != 0 {
                        /* A partial copy: the lanes it did not write may be
                         * read only when nothing ever writes them, and then
                         * from the copy's first lane, so the source is not
                         * read in a lane the copy itself did not read. */
                        if lanes & held == 0 || held != write_mask {
                            continue;
                        }
                        if *sole_writer.get_or_insert_with(|| {
                            sel.iter().enumerate().all(|(j, s)| j == i || writes_virtual(s, t) == 0)
                        }) {
                            let first = write_mask.trailing_zeros() as u8;
                            for c in swizzle.iter_mut() {
                                if write_mask & (1 << (*c & 3)) == 0 {
                                    *c = first;
                                }
                            }
                        } else {
                            continue;
                        }
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

/// The producers [`retarget_producers`] may move onto a copy's destination
/// unchanged: every lane of the result is the same value.
fn writes_one_value(s: &SelInstr) -> bool {
    matches!(s.op.op().name, "rcp" | "rsq" | "sqrt" | "exp" | "dp2" | "dp3" | "dp4")
}

/// The producers [`retarget_producers`] may move with their source swizzles
/// composed through the copy's: destination lane `c` is computed from lane
/// `c` of each source and nothing else.
fn lane_by_lane(s: &SelInstr) -> bool {
    matches!(s.op.op().name, "mov" | "add" | "mul" | "mad" | "select" | "cmp" | "floor" | "ceil" | "frc")
}

/// `src` read through the copy's swizzle `sw` on the lanes `mask`: lane `c`
/// takes what lane `sw[c]` took, and every lane outside the mask the first
/// lane of it, so no source lane is read that was not read before. An
/// immediate has no lanes to move.
fn compose(src: SelSrc, sw: [u8; 4], mask: u8) -> SelSrc {
    let pick = |base: [u8; 4]| {
        let first = base[sw[mask.trailing_zeros() as usize & 3] as usize & 3];
        core::array::from_fn(|c| if mask & (1 << c) != 0 { base[sw[c] as usize & 3] } else { first })
    };
    match src {
        SelSrc::Virtual { v, swizzle, neg, abs } => SelSrc::Virtual { v, swizzle: pick(swizzle), neg, abs },
        SelSrc::Input { reg, swizzle, neg, abs } => SelSrc::Input { reg, swizzle: pick(swizzle), neg, abs },
        SelSrc::Physical(p) if p.is_immediate() => SelSrc::Physical(p),
        SelSrc::Physical(p) => SelSrc::Physical(Src { swizzle: pick(p.swizzle), ..p }),
    }
}

/// Have the instruction that computes a value a copy is the only reader of
/// write the copy's destination itself, and empty the copy (the caller
/// deletes it as [`merge_scalar_lanes`]' emptied lanes). Returns how many.
///
/// `mov d.M, s.sw` qualifies when it is unguarded, unsaturated and plain;
/// `s` is a temporary written by exactly one instruction P and read by the
/// copy alone; P comes earlier in straight-line code (no branch leaves or
/// arrives between them); nothing between them reads `d` or writes a lane of
/// `M` of it; and every lane `sw` takes from `s` on `M` is one P writes. P
/// then writes `d.M`: unchanged when every lane of its result is one value
/// ([`writes_one_value`]), with its source swizzles composed through `sw`
/// when it works lane by lane ([`lane_by_lane`]). A texture fetch, a branch,
/// a guarded P and the lane-pair writers (`sin`, `cos`, `log`, `div`) stay.
///
/// In noise's `snoise` a dot product computed into a temporary and copied
/// into one lane of `norm` or `m` is this shape nine times a call
/// (`docs/notes/gc-terrain-shaders-rest.md`, rank 2).
pub fn retarget_producers(sel: &mut [SelInstr], targets: &[u64]) -> usize {
    let mut moved = 0usize;
    for j in 0..sel.len() {
        let m = sel[j];
        if !m.op.is_mov() || m.cond.is_some() || m.saturate || m.rtz || m.target.is_some() || is_target(targets, j) {
            continue;
        }
        let Some(SelDst::Virtual { v: d, write_mask: mask }) = m.dst else { continue };
        let Some(SelSrc::Virtual { v: s, swizzle: sw, neg: false, abs: false }) = m.srcs[2] else { continue };
        if mask == 0 || s == d {
            continue;
        }
        /* Back to the producer, through instructions that leave `d.M` alone. */
        let mut producer = None;
        for k in (j.saturating_sub(FOLD_REACH)..j).rev() {
            let x = &sel[k];
            if is_target(targets, k + 1) || x.target.is_some() {
                break;
            }
            if writes_virtual(x, s) != 0 {
                producer = Some(k);
                break;
            }
            if x.srcs.iter().flatten().any(|r| matches!(*r, SelSrc::Virtual { v, .. } if v == d))
                || writes_virtual(x, d) & mask != 0
            {
                break;
            }
        }
        let Some(k) = producer else { continue };
        let p = sel[k];
        let guarded = p.cond.is_some() && !matches!(p.op.op().name, "select" | "cmp");
        if guarded || p.tex.is_some() || !(writes_one_value(&p) || lane_by_lane(&p)) {
            continue;
        }
        let written = writes_virtual(&p, s);
        if (0..4).any(|c| mask & (1 << c) != 0 && written & (1 << (sw[c] & 3)) == 0) {
            continue;
        }
        let only = sel.iter().enumerate().all(|(i, x)| {
            let reads_s = x.srcs.iter().flatten().any(|r| matches!(*r, SelSrc::Virtual { v, .. } if v == s));
            (i == k || writes_virtual(x, s) == 0) && (i == j || !reads_s)
        });
        if !only {
            continue;
        }
        let mut q = p;
        q.dst = Some(SelDst::Virtual { v: d, write_mask: mask });
        if !writes_one_value(&p) {
            for slot in q.srcs.iter_mut() {
                if let Some(x) = *slot {
                    *slot = Some(compose(x, sw, mask));
                }
            }
        }
        sel[k] = q;
        sel[j].dst = Some(SelDst::Virtual { v: d, write_mask: 0 });
        moved += 1;
    }
    moved
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

    /// `vec2 c = vUv;` read by a whole-vec4 add: the copy is the only writer
    /// of `c`, so the add's lanes past `.xy` read nothing anyone wrote, and the
    /// add reads the source directly, those lanes from the copy's first lane.
    #[test]
    fn a_partial_copy_that_is_the_only_writer_is_read_through_for_every_lane() {
        let (mov, add) = (op_id("mov").unwrap(), op_id("add").unwrap());
        let uv = SelSrc::Input { reg: 3, swizzle: [0, 1, 2, 3], neg: false, abs: false };
        let reader = SelInstr::new(add, OpType::F32)
            .dst(d(2, 0xf))
            .operand(0, v(1, [0, 1, 2, 3], false))
            .operand(1, v(5, [0; 4], false));
        let mut sel = [SelInstr::new(mov, OpType::F32).dst(d(1, 0x3)).operand(0, uv), reader];
        assert_eq!(fold_copies(&mut sel, &[0]), 1);
        assert!(matches!(sel[1].srcs[0], Some(SelSrc::Input { reg: 3, swizzle: [0, 1, 0, 0], .. })), "{:?}", sel[1]);

        /* Refused: another instruction writes `c.z`, so lane z is a value. */
        let mut sel = [
            SelInstr::new(mov, OpType::F32).dst(d(1, 0x3)).operand(0, uv),
            SelInstr::new(mov, OpType::F32).dst(d(1, 0x4)).operand(0, v(6, [0; 4], false)),
            reader,
        ];
        assert_eq!(fold_copies(&mut sel, &[0]), 0, "neither copy is the only writer of c");
        assert!(matches!(sel[2].srcs[0], Some(SelSrc::Virtual { v: VReg(1), .. })), "{:?}", sel[2]);
    }

    /// `vec4(dot(a, a), dot(b, b), ...)`: the dot product computed into a
    /// temporary and copied into a lane writes the lane itself, and the copy
    /// is emptied.
    #[test]
    fn a_dot_product_copied_into_a_lane_writes_the_lane_itself() {
        let (mov, dp4) = (op_id("mov").unwrap(), op_id("dp4").unwrap());
        let dot = |dst: u32| {
            SelInstr::new(dp4, OpType::F32)
                .dst(d(dst, 0x1))
                .operand(0, v(0, [0, 1, 2, 3], false))
                .operand(1, v(0, [0, 1, 2, 3], false))
        };
        let copy = SelInstr::new(mov, OpType::F32).dst(d(2, 0x2)).operand(0, v(1, [0; 4], false));
        let mut sel = [dot(1), copy];
        assert_eq!(retarget_producers(&mut sel, &[0]), 1);
        assert!(matches!(sel[0].dst, Some(SelDst::Virtual { v: VReg(2), write_mask: 0x2 })), "{:?}", sel[0]);
        assert!(matches!(sel[1].dst, Some(SelDst::Virtual { write_mask: 0, .. })));
        assert!(
            sel[0]
                .srcs
                .iter()
                .flatten()
                .all(|s| matches!(s, SelSrc::Virtual { v: VReg(0), swizzle: [0, 1, 2, 3], .. })),
            "a dot product's sources do not move: {:?}",
            sel[0]
        );
    }

    /// A lane-by-lane producer has its sources composed through the copy:
    /// `add t.xy, p.xyzw, q.zwxy; mov o.__zw, t.xxxy` is `add o.__zw,
    /// p.xxxy, q.zzzw`, the lanes outside the mask read as its first lane.
    #[test]
    fn a_lane_by_lane_producer_reads_through_the_copys_swizzle() {
        let (mov, add) = (op_id("mov").unwrap(), op_id("add").unwrap());
        let mut sel = [
            SelInstr::new(add, OpType::F32)
                .dst(d(1, 0x3))
                .operand(0, v(5, [0, 1, 2, 3], false))
                .operand(1, v(6, [2, 3, 0, 1], false)),
            SelInstr::new(mov, OpType::F32).dst(d(2, 0xc)).operand(0, v(1, [0, 0, 0, 1], false)),
        ];
        assert_eq!(retarget_producers(&mut sel, &[0]), 1);
        assert!(matches!(sel[0].dst, Some(SelDst::Virtual { v: VReg(2), write_mask: 0xc })));
        assert!(
            matches!(sel[0].srcs[0], Some(SelSrc::Virtual { v: VReg(5), swizzle: [0, 0, 0, 1], .. })),
            "{:?}",
            sel[0]
        );
        assert!(
            matches!(sel[0].srcs[2], Some(SelSrc::Virtual { v: VReg(6), swizzle: [2, 2, 2, 3], .. })),
            "{:?}",
            sel[0]
        );
    }

    /// What [`retarget_producers`] leaves: a value read twice, written twice,
    /// a destination read in between, a branch arriving in between, a
    /// texture fetch, a lane-pair writer, a lane the producer did not write,
    /// a negated copy.
    #[test]
    fn a_producer_is_not_retargeted_where_the_copy_is_not_its_only_use() {
        let (mov, dp3, add, sin) =
            (op_id("mov").unwrap(), op_id("dp3").unwrap(), op_id("add").unwrap(), op_id("sin").unwrap());
        let dot = SelInstr::new(dp3, OpType::F32)
            .dst(d(1, 0x1))
            .operand(0, v(0, [0, 1, 2, 3], false))
            .operand(1, v(0, [0, 1, 2, 3], false));
        let copy = SelInstr::new(mov, OpType::F32).dst(d(2, 0x2)).operand(0, v(1, [0; 4], false));
        let reads = |s: u32| {
            SelInstr::new(add, OpType::F32)
                .dst(d(7, 0xf))
                .operand(0, v(s, [0; 4], false))
                .operand(1, v(s, [0; 4], false))
        };
        let cases: [(&str, Vec<SelInstr>, Vec<u64>); 6] = [
            ("read twice", vec![dot, copy, reads(1)], vec![0]),
            ("written twice", vec![dot, dot, copy], vec![0]),
            ("the destination read in between", vec![dot, reads(2), copy], vec![0]),
            ("a branch arrives in between", vec![dot, reads(9), copy], vec![1 << 1]),
            (
                "a lane-pair writer",
                vec![SelInstr::new(sin, OpType::F32).dst(d(1, 0x1)).operand(0, v(0, [0; 4], false)), copy],
                vec![0],
            ),
            (
                "a lane it did not write",
                vec![dot, SelInstr::new(mov, OpType::F32).dst(d(2, 0x2)).operand(0, v(1, [1; 4], false))],
                vec![0],
            ),
        ];
        for (what, mut sel, targets) in cases {
            assert_eq!(retarget_producers(&mut sel, &targets), 0, "{what}");
        }
        let mut sel = [dot, SelInstr::new(mov, OpType::F32).dst(d(2, 0x2)).operand(0, v(1, [0; 4], true))];
        assert_eq!(retarget_producers(&mut sel, &[0]), 0, "a negated copy");
        let texld = op_id("texld").unwrap();
        let mut sel = [
            SelInstr::new(texld, OpType::F32)
                .dst(d(1, 0xf))
                .operand(0, v(0, [0, 1, 2, 3], false))
                .tex(super::super::alloc::Tex { id: 0, swizzle: [0, 1, 2, 3] }),
            SelInstr::new(mov, OpType::F32).dst(d(2, 0x2)).operand(0, v(1, [0; 4], false)),
        ];
        assert_eq!(retarget_producers(&mut sel, &[0]), 0, "a texture fetch");
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
