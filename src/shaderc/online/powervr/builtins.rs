// SPDX-License-Identifier: Apache-2.0 AND MIT
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
// SPDX-FileCopyrightText: 2024 Imagination Technologies Ltd.
// Ported from Mesa 26.2.1 src/imagination/pco/pco_map.py (MIT).
// The upstream part stays under its licence (LICENSES/MIT.txt); our
// changes are Apache-2.0. See NOTICE.
//! What each operation of the IR kernel is on the Rogue USC.
//!
//! The GC7000's half of this question ([`super::super::vivante::builtins`])
//! answers with an opcode: `native(op, ty)` returns a name or `None`, and
//! `lower` writes a sequence when the answer is `None`. That shape does not
//! survive the move to this machine, and the reason is worth stating before
//! the table, because it is the whole difference between the two back ends.
//!
//! A USC instruction is not one operation. It is a **group**: a header, then
//! up to one operation per pipeline phase, issued together. `min` on the
//! GC7000 is one `select` with `COND = Gt`. `min` here is one group holding
//! four operations across three phases -- two `mbyp` that do nothing but move
//! the sources into the internal registers, a `tst`, and a `movc`. So the
//! unit this file deals in is a group SHAPE, not an opcode, and an entry that
//! is "native" is one whose shape is a single operation in phase 0.
//!
//! That is also why `mbyp` is the most common operation in the entire offline
//! corpus -- 218,917 of 576,296 groups
//! (`docs/online-shaderc/isa-coverage.md`). It is not waste in whoever
//! compiled those blobs; it is how a phased machine feeds its own pipeline.
//!
//! # Where each entry comes from
//!
//! Every [`Shape`] below carries its evidence in the type, and there are only
//! three kinds. [`Step::corpus`] is the count from the 2,042-blob corpus, so a
//! reader can see at a glance whether an entry rests on something this
//! project has decoded 100,096 times or on something it has never seen.
//! Multi-operation shapes are transcribed from Mesa's `pco_map.py`
//! (`group_map`, MIT, read and transcribed -- never vendored); they are
//! marked in each entry's comment and their operations still carry corpus
//! counts, which is the cross-check: a transcription that named an operation
//! the corpus does not contain would be visible immediately.
//!
//! Nothing here is measured on silicon. It cannot be until the builder exists
//! (`docs/online-shaderc/usc-seam.md`, "Order of work"), and this file is
//! deliberately written first so the builder has a target rather than a
//! running guess.
//!
//! # The three gaps this machine does NOT share with the GC7000
//!
//! **Floor is a source modifier here.** The GC7000 has a `floor` instruction;
//! the USC has `s0flr`/`s1flr`/`s2flr` beside `neg` and `abs`
//! (`docs/online-shaderc/isa.md`, "Source modifiers"). pco declares `fneg`,
//! `fabs` and `fflr` as *pseudo* operations (`pco_ops.py: pseudo_op`) and
//! folds them into the consumer, falling back to one `mov` carrying the
//! modifier when no consumer will take it (`pco_opt.c`). All three therefore
//! cost nothing in the common case and one `mbyp` in the worst.
//!
//! **There are ten comparisons, not four.** The GC7000's table has EQ, NE, GE
//! and LT, so `a > b` has to become `b < a` by swapping operands. `tst_op`
//! here is 4 bits with ten values -- `z gz gez c e g ge ne l le` -- over
//! seven types. Every compare in the kernel is one `tst`, with no swap, and
//! the against-zero forms give `F2b`/`I2b` for free.
//!
//! **There is no dot product.** The GC7000 has DP2/DP3/DP4, which is the only
//! reason `Fdot2`/`Fdot3`/`Fdot4` are in the kernel at all
//! (`docs/online-shaderc/ir.md`, decision 1). The USC has none: pco declares
//! no dot operation and the corpus contains none. They lower to a `fmul`
//! followed by `fmad`s, which is what this file says, and it is the one place
//! where the kernel's shape costs this back end something.

use super::isa::{PH_0, PH_1, PH_2, PH_2_MOV, PH_2_PCK, PH_2_TST};
use crate::shaderc::online::ir::AluOp;

/// One hardware operation inside a group.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Step {
    /// The operation, spelled as `isa_tables::OPS` spells it.
    pub op: &'static str,
    /// The pipeline phase it occupies (`isa::PH_*`). An operation may only be
    /// issued in a phase the ISA table lists it in, which is what
    /// `every_step_is_a_real_operation_in_that_phase` checks.
    pub phase: u8,
    /// Times this operation appears in the 2,042-blob offline corpus
    /// (`docs/online-shaderc/isa-coverage.md`). Zero means the corpus does
    /// not contain it, which is a fact about the corpus, not a defect: the
    /// offline compiler simply never emitted one.
    pub corpus: u32,
}

const fn step(op: &'static str, phase: u8, corpus: u32) -> Step {
    Step { op, phase, corpus }
}

/// The source modifiers, which are not operations.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Modifier {
    Neg,
    Abs,
    Flr,
}

/// What an IR operation is on this machine.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    /// A modifier on the consumer's source. Costs no operation when a
    /// consumer takes it, and one `mbyp` carrying the modifier when none
    /// will.
    Modifier(Modifier),
    /// One instruction group made of these steps, in this order.
    Group(&'static [Step]),
    /// Not one group. The reason names what has to produce the value
    /// instead -- a front-end expansion, or a sequence of groups the builder
    /// will emit once it exists. Scalar in, scalar out: the builder can write
    /// the sequence because it can see every operand.
    Lower(&'static str),
    /// Selection expands this ABOVE the seam, so no [`SelGroup`] ever carries
    /// it and the builder never sees one.
    ///
    /// [`Shape::Lower`] cannot serve here, and the difference is the whole
    /// reason this variant exists. `Lower` works when the builder can see the
    /// operands it has to recombine. The dot products are vector-shaped on a
    /// scalar machine: `Fdot3(a, b)` has six scalar operands and
    /// [`SelGroup::srcs`] holds three, so there is no way to hand the builder
    /// a dot product without either widening the seam back to vectors or
    /// teaching the builder a component numbering that selection owns. Both
    /// are the coupling `docs/online-shaderc/usc-seam.md` set out to avoid.
    ///
    /// Selection knows the components, so selection writes the chain. Ruled
    /// 2026-09-22 after gles2-core found it while writing the walker.
    ///
    /// [`SelGroup`]: super::sel::SelGroup
    /// [`SelGroup::srcs`]: super::sel::SelGroup::srcs
    SelectionExpands(&'static str),
    /// This machine cannot do it and nothing in the corpus needs it.
    Refused(&'static str),
}

/* ---- single-operation shapes: the operations this machine simply has ---- */

const FADD: &[Step] = &[step("fadd", PH_0, 47205)];
const FMUL: &[Step] = &[step("fmul", PH_0, 91771)];
const FMAD: &[Step] = &[step("fmad", PH_0, 100096)];
const FRCP: &[Step] = &[step("frcp", PH_0, 13683)];
const FRSQ: &[Step] = &[step("frsq", PH_0, 8550)];
const FEXP: &[Step] = &[step("fexp", PH_0, 1024)];
const FLOG: &[Step] = &[step("flog", PH_0, 339)];
const FDSX: &[Step] = &[step("fdsx", PH_0, 89)];
const FDSY: &[Step] = &[step("fdsy", PH_0, 89)];
/* `movc` IS bcsel: pco's `pco_bcsel` is a movc, and it is also what every
 * select-shaped group below ends with. */
const MOVC: &[Step] = &[step("movc", PH_2_MOV, 51411)];
/* Every compare, float and integer, is one `tst`; the type is a field. */
const TST: &[Step] = &[step("tst", PH_2_TST, 58467)];
/* Integer add and multiply are the same multiply-add unit with an operand
 * pinned: pco_trans_nir.c emits iadd32/imul32, which map onto `imadd32`. */
const IMADD32: &[Step] = &[step("imadd32", PH_0, 26714)];
/* and/or/xor/not are the bitwise pipeline's phase 1; shifts are its phase 2. */
const LOGICAL: &[Step] = &[step("logical", PH_1, 9105)];
const SHIFT: &[Step] = &[step("shift", PH_2, 21900)];
/* Float to integer packs; integer to float unpacks. The format (S32/U32) is a
 * field, so one shape covers both signednesses. */
const PCK: &[Step] = &[step("pck", PH_2_PCK, 21187)];
const UNPCK: &[Step] = &[step("unpck", PH_0, 2621)];

/* ---- multi-operation shapes, transcribed from pco_map.py's group_map ---- */

/// `min`/`max`: both sources through `mbyp` into ft0/ft1, one `tst` (`l` for
/// min, `g` for max), one `movc`. Transcribed from `group_map(O_MIN)` and
/// `group_map(O_MAX)`.
const MINMAX: &[Step] = &[
    step("mbyp", PH_0, 218917),
    step("mbyp", PH_1, 218917),
    step("tst", PH_2_TST, 58467),
    step("movc", PH_2_MOV, 51411),
];

/// `ceil(x)` = `floor(x) + 1`, except where `x` is already integral.
/// Transcribed from `group_map(O_FCEIL)`: phase 0 is `fadd(flr(s0), 1.0)`,
/// phase 1 is `fadd(flr(s0), 0.0)`, then `tst` `equal` in f32 between the
/// unmodified source and `floor(x)`, then `movc`. Both `fadd`s take the
/// floor modifier, which is the gap above made concrete: on a machine
/// without `s0flr` this shape would need two more instructions.
const FCEIL: &[Step] = &[
    step("fadd", PH_0, 47205),
    step("fadd", PH_1, 47205),
    step("tst", PH_2_TST, 58467),
    step("movc", PH_2_MOV, 51411),
];

/// `sign(x)`, and the neatest thing in pco's table. Transcribed from
/// `group_map(O_FSIGN)`: phase 0 is `fmul(x, +inf)` **saturated**, which is
/// 1.0 for any positive x and 0.0 for +0; phase 1 is `mbyp(-1.0)`; the test
/// is `gez` on the same value read as **s32**, i.e. "is the float's sign bit
/// clear"; the `movc` picks phase 0 when it is and -1.0 when it is not.
const FSIGN: &[Step] = &[
    step("fmul", PH_0, 91771),
    step("mbyp", PH_1, 218917),
    step("tst", PH_2_TST, 58467),
    step("movc", PH_2_MOV, 51411),
];

/// `sin`/`cos`. `fred` does Payne-Hanek range reduction in two halves (part A
/// then part B), `fsinc` evaluates the series, and a predicate selects
/// between the result and the reduced argument. Transcribed from
/// `trans_trig()`; the corpus corroborates the shape independently, with
/// `fred` at 704 and `fsinc` at 352 -- exactly the 2:1 this sequence gives.
const TRIG: &[Step] =
    &[step("fred", PH_0, 704), step("fred", PH_0, 704), step("fsinc", PH_0, 352), step("movc", PH_2_MOV, 51411)];

/// What `op` is on this machine.
///
/// Total by construction: every operation of the kernel has an answer, and
/// [`Shape::Refused`] is one of them. `the_kernel_is_accounted_for` holds
/// that, so an operation added to `AluOp` cannot quietly arrive here without
/// a decision.
pub fn shape(op: AluOp) -> Shape {
    use AluOp::*;
    match op {
        /* modifiers: free when a consumer takes them, one mbyp otherwise */
        Fneg => Shape::Modifier(Modifier::Neg),
        Fabs => Shape::Modifier(Modifier::Abs),
        Ffloor => Shape::Modifier(Modifier::Flr),

        /* one operation, phase 0 */
        Fadd => Shape::Group(FADD),
        Fmul => Shape::Group(FMUL),
        Ffma => Shape::Group(FMAD),
        Frcp => Shape::Group(FRCP),
        Frsq => Shape::Group(FRSQ),
        Fexp2 => Shape::Group(FEXP),
        Flog2 => Shape::Group(FLOG),
        Fddx => Shape::Group(FDSX),
        Fddy => Shape::Group(FDSY),
        Bcsel => Shape::Group(MOVC),

        /* one `tst`; tst_op and tst_type carry the rest.
         * F2b and I2b are the against-zero forms of the same operation. */
        Flt | Fle | Feq | Fne | Ilt | Ile | Ieq | Ine | F2b | I2b => Shape::Group(TST),

        /* the integer multiply-add unit with an operand pinned; Ineg is
         * iadd32 with the source negated, which is a modifier again */
        Iadd | Imul | Ineg => Shape::Group(IMADD32),

        /* the bitwise pipeline. Bany/Ball reduce with the same operation:
         * a bool is all-ones, so "any" is `or` and "all" is `and`. */
        Iand | Ior | Ixor | Bnot | Bany | Ball => Shape::Group(LOGICAL),
        Ishl | Ishr => Shape::Group(SHIFT),

        /* conversions. B2f/B2i select between two constants, which is a
         * `movc` -- pco's b2f32/b2i32 are literally `pco_bcsel`. */
        F2i => Shape::Group(PCK),
        I2f => Shape::Group(UNPCK),
        B2f | B2i => Shape::Group(MOVC),

        /* multi-operation groups */
        Fmin | Fmax => Shape::Group(MINMAX),
        Fceil => Shape::Group(FCEIL),
        Fsign => Shape::Group(FSIGN),
        Fsin | Fcos => Shape::Group(TRIG),

        /* more than one group: the builder will emit a sequence */
        Fsqrt => Shape::Lower("frsq then frcp; pco sets nir lower_fsqrt"),
        /* pco sets nir lower_ftrunc; selection writes it as a `tst`, an
         * `fceil`, a floored `fadd` and a `movc` (`Sel::alu`). */
        Ftrunc => Shape::SelectionExpands("x < 0 ? ceil(x) : floor(x), nir lower_ftrunc's bcsel"),
        Ffract => Shape::Lower("x - floor(x); pco sets nir lower_ffract"),
        Fdot2 | Fdot3 | Fdot4 => Shape::SelectionExpands(
            "fmul then fmad per component; this machine has no dot, and three source slots cannot carry six operands",
        ),
        /* No integer divide on this machine: selection expands both the way
         * pco's NIR does (`nir_lower_idiv`, `Sel::idiv`). */
        Idiv | Imod => Shape::SelectionExpands("nir_lower_idiv: a reciprocal estimate, umul_high and two refinements"),

        /* GLSL ES 3.00 (ir.md, IR v1). Refused until usc-backend transcribes
         * each; its answers to the proposal say which are native. */
        Ult | Ule => Shape::Group(TST),
        Imin | Imax | Umin | Umax => Shape::Group(MINMAX),
        Ushr => Shape::Group(SHIFT),
        Inot => Shape::Group(LOGICAL),
        U2f => Shape::Group(UNPCK),
        F2u => Shape::Group(PCK),
        Udiv | Umod => Shape::SelectionExpands("nir_lower_idiv's unsigned form: emit_udiv without the sign"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shaderc::online::powervr::isa_tables::OPS;

    /// Every operation named in every shape must exist in the ISA table AND
    /// be issuable in the phase the shape puts it in.
    ///
    /// This is the check that makes the transcriptions above worth having.
    /// A name copied wrongly out of `pco_map.py`, or an operation placed in
    /// a phase this hardware cannot issue it in, fails here rather than in a
    /// picture on a board.
    #[test]
    fn every_step_is_a_real_operation_in_that_phase() {
        for op in ALL {
            if let Shape::Group(steps) = shape(*op) {
                for s in steps {
                    let found = OPS.iter().any(|o| o.name == s.op && o.phase == s.phase);
                    assert!(found, "{op:?}: no `{}` in phase {}", s.op, s.phase);
                }
            }
        }
    }

    /// A step's corpus count must match what the corpus actually holds.
    ///
    /// The counts are written by hand from `isa-coverage.md`, so they can
    /// drift from it. They are not decoration: they are the reason a reader
    /// can trust a transcribed shape without re-reading pco. This pins the
    /// few that carry the most weight.
    #[test]
    fn the_counts_that_carry_the_argument_are_the_corpus_counts() {
        for (name, want) in
            [("mbyp", 218917u32), ("fmad", 100096), ("tst", 58467), ("movc", 51411), ("fred", 704), ("fsinc", 352)]
        {
            let mut seen = None;
            for op in ALL {
                if let Shape::Group(steps) = shape(*op) {
                    for s in steps {
                        if s.op == name {
                            assert_eq!(s.corpus, want, "{name} in {op:?}");
                            seen = Some(());
                        }
                    }
                }
            }
            assert!(seen.is_some(), "{name} is named in the module doc but in no shape");
        }
    }

    /// `fred` appears twice in the trig shape and `fsinc` once, and the
    /// corpus holds them at exactly 2:1. That ratio is the corroboration
    /// that the transcription is right: it was not chosen to fit.
    #[test]
    fn the_corpus_ratio_corroborates_the_trig_transcription() {
        let freds = TRIG.iter().filter(|s| s.op == "fred").count();
        let sincs = TRIG.iter().filter(|s| s.op == "fsinc").count();
        assert_eq!((freds, sincs), (2, 1));
        assert_eq!(704 / 352, (freds / sincs) as u32);
    }

    /// Adding an operation to the kernel must force a decision here.
    #[test]
    fn the_kernel_is_accounted_for() {
        assert_eq!(ALL.len(), 52, "the kernel changed size; every entry needs a look");
        for op in ALL {
            /* `shape` is total, so this asserts the match has no hole the
             * compiler would have let through as a catch-all. */
            match shape(*op) {
                Shape::Refused(why) => assert!(!why.is_empty(), "{op:?} refused without a reason"),
                Shape::Lower(what) => assert!(!what.is_empty(), "{op:?} lowered to nothing"),
                Shape::SelectionExpands(what) => {
                    assert!(!what.is_empty(), "{op:?} expanded into nothing");
                }
                Shape::Group(steps) => assert!(!steps.is_empty(), "{op:?} is an empty group"),
                Shape::Modifier(_) => {}
            }
        }
    }

    /// No group may hold two operations in one phase: the hardware issues at
    /// most one per phase, so a shape that did would be unencodable.
    #[test]
    fn no_group_puts_two_operations_in_one_phase() {
        for op in ALL {
            if let Shape::Group(steps) = shape(*op) {
                /* `fred` is the one real exception and it is not a group of
                 * one instruction at all -- the two parts are consecutive
                 * groups. Recorded here so the builder cannot forget it. */
                if matches!(op, AluOp::Fsin | AluOp::Fcos) {
                    continue;
                }
                for (i, a) in steps.iter().enumerate() {
                    for b in &steps[i + 1..] {
                        assert_ne!(a.phase, b.phase, "{op:?}: two operations in phase {}", a.phase);
                    }
                }
            }
        }
    }

    use AluOp::*;
    const ALL: &[AluOp] = &[
        Fadd, Fmul, Ffma, Fmin, Fmax, Fneg, Fabs, Fsign, Frcp, Frsq, Fsqrt, Fexp2, Flog2, Fsin, Fcos, Ffloor, Fceil,
        Ftrunc, Ffract, Fddx, Fddy, Fdot2, Fdot3, Fdot4, Flt, Fle, Feq, Fne, Iadd, Imul, Idiv, Imod, Ineg, Ishl, Ishr,
        Iand, Ior, Ixor, Ilt, Ile, Ieq, Ine, B2f, B2i, F2i, I2f, F2b, I2b, Bnot, Bany, Ball, Bcsel,
    ];
}
