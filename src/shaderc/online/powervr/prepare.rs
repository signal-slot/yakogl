// SPDX-License-Identifier: Apache-2.0 AND MIT
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
// SPDX-FileCopyrightText: 2024 Imagination Technologies Ltd.
// Ported from Mesa 26.2.1 src/imagination/pco/pco_const_imms.c (MIT).
// The upstream part stays under its licence (LICENSES/MIT.txt); our
// changes are Apache-2.0. See NOTICE.
//! Making selection's groups ones the builder can emit, before allocation.
//!
//! Selection states operands as the program means them: a uniform, an
//! immediate, a negated second factor. The machine takes fewer shapes than
//! that, and every gap between the two is closed HERE, on virtual registers,
//! so the allocator places whatever this inserts like any other value:
//!
//! 1. **An immediate** becomes a constant register when the hardware has one
//!    holding that value (pco's `const_reg_defs`, transcribed below), and
//!    otherwise a `bbyp0bm_imm32` move into a fresh value. The USC has no
//!    source slot that carries a literal.
//! 2. **A register file an operation is never seen reading in that slot**
//!    gets an `mbyp` into a fresh value first. The measured case is the
//!    uniform file: across the 576,296 groups of the offline table no
//!    arithmetic operation ever reads `shared` -- 31,555 `mbyp`s do, and
//!    their results are what the arithmetic reads. Whether the hardware
//!    would accept a uniform in an `fmad` directly is a question the corpus
//!    cannot answer, and a program is not the place to ask it.
//! 3. **A modifier no form of the operation carries on that slot** is moved:
//!    first by an exact algebraic identity where one exists (a product's sign
//!    moves between its factors; a sum's operands commute), and otherwise into
//!    a copy that applies it.
//! 4. **An output component `uvsw.write` cannot take as it stands** -- a
//!    modifier, an immediate with no constant register, a uniform -- is moved
//!    into a fresh value the write reads.
//!
//! What the builder can emit, and what the corpus has seen, are separate
//! questions. This file answers the first; `tests/usc_limits.rs` checks every
//! group the chain builds against the second.

use super::builtins::{shape, Shape};
use super::instr::{instr_region, NoForm};
use super::isa::PH_0;
use super::sel::{Bank, Mods, OpType, SelDst, SelGroup, SelOp, SelSrc, VReg};
use super::select::Selected;
use crate::shaderc::online::arena::{AVec, Arena, ArenaError};
use crate::shaderc::online::ir::{AluOp, IoSlot};

/// Why a program could not be prepared.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PrepareError {
    OutOfScratch,
    /// A shape this pass has no move for, named.
    Unsupported(&'static str),
}

impl From<ArenaError> for PrepareError {
    fn from(_: ArenaError) -> Self {
        PrepareError::OutOfScratch
    }
}

/// The hardware constant registers: value -> index in the special file.
///
/// Transcribed from Mesa `src/imagination/pco/pco_const_imms.c`
/// (`const_reg_defs`, MIT, Copyright Imagination Technologies), which maps a
/// `movi32` of one of these values onto a read of `PCO_REG_CLASS_CONST`
/// register `idx`; `pco_map_reg_bank` puts that class in the special bank at
/// the same index. Every entry there has `flr` and `neg` clear. Sorted by
/// value, as the source is.
///
/// NOT measured: the corpus shows which special registers pco reads, not
/// what they hold. The values are Mesa's claim about the hardware.
pub const CONST_REGS: &[(u32, u16)] = &[
    (0x00000000, 0),
    (0x00000001, 1),
    (0x00000002, 2),
    (0x00000003, 3),
    (0x00000004, 4),
    (0x00000005, 5),
    (0x00000006, 6),
    (0x00000007, 7),
    (0x00000008, 8),
    (0x00000009, 9),
    (0x0000000a, 10),
    (0x0000000b, 11),
    (0x0000000c, 12),
    (0x0000000d, 13),
    (0x0000000e, 14),
    (0x0000000f, 15),
    (0x00000010, 16),
    (0x00000011, 17),
    (0x00000012, 18),
    (0x00000013, 19),
    (0x00000014, 20),
    (0x00000015, 21),
    (0x00000016, 22),
    (0x00000017, 23),
    (0x00000018, 24),
    (0x00000019, 25),
    (0x0000001a, 26),
    (0x0000001b, 27),
    (0x0000001c, 28),
    (0x0000001d, 29),
    (0x0000001e, 30),
    (0x0000001f, 31),
    (0x0000007f, 147),
    (0x37800000, 134),
    (0x38000000, 135),
    (0x38800000, 88),
    (0x39000000, 87),
    (0x39800000, 86),
    (0x3a000000, 85),
    (0x3a800000, 84),
    (0x3b000000, 83),
    (0x3b4d2e1c, 136),
    (0x3b800000, 82),
    (0x3c000000, 81),
    (0x3c800000, 80),
    (0x3d000000, 79),
    (0x3d25aee6, 156),
    (0x3d6147ae, 140),
    (0x3d800000, 78),
    (0x3d9e8391, 157),
    (0x3e000000, 77),
    (0x3e2aaaab, 153),
    (0x3e800000, 76),
    (0x3e9a209b, 145),
    (0x3ea2f983, 128),
    (0x3eaaaaab, 152),
    (0x3ebc5ab2, 90),
    (0x3ed55555, 138),
    (0x3f000000, 75),
    (0x3f22f983, 129),
    (0x3f317218, 146),
    (0x3f3504f3, 92),
    (0x3f490fdb, 93),
    (0x3f72a76f, 158),
    (0x3f800000, 64),
    (0x3f860a92, 151),
    (0x3f870a3d, 139),
    (0x3fa2f983, 130),
    (0x3fb504f3, 91),
    (0x3fb8aa3b, 155),
    (0x3fc90fdb, 94),
    (0x40000000, 65),
    (0x4019999a, 159),
    (0x402df854, 89),
    (0x40490fdb, 95),
    (0x40549a78, 154),
    (0x40800000, 66),
    (0x40c90fdb, 131),
    (0x41000000, 67),
    (0x41490fdb, 132),
    (0x414eb852, 137),
    (0x41800000, 68),
    (0x41c90fdb, 133),
    (0x42000000, 69),
    (0x42800000, 70),
    (0x43000000, 71),
    (0x43800000, 72),
    (0x44000000, 73),
    (0x44800000, 74),
    (0x4b000000, 149),
    (0x4b800000, 150),
    (0x7f7fffff, 148),
    (0x7f800000, 142),
    (0x7fff7fff, 144),
    (0x80000000, 141),
    (0xffffffff, 143),
];

/// The special register holding `v`, if the hardware has one.
pub fn const_reg(v: u32) -> Option<u16> {
    CONST_REGS.binary_search_by_key(&v, |(x, _)| *x).ok().map(|i| CONST_REGS[i].1)
}

/// The point size range pco clamps `gl_PointSize` to: Mesa
/// `src/imagination/common/pvr_limits.h`, PVR_POINT_SIZE_RANGE_MIN / _MAX.
pub const POINT_SIZE_MIN: f32 = 1.0;
pub const POINT_SIZE_MAX: f32 = 511.0;

/// `1.0f`, the multiplier that turns `fmul` into a floor: `flr(x) * 1.0` is
/// `floor(x)` for every input, `-0.0` and NaN included, which `x + 0.0` is not.
const FONE: u16 = 64;

/// The register files each operation reads in each source slot anywhere in
/// the offline table, measured over its 576,296 groups (usc-backend,
/// 2026-09-23; `every_attested_bank_occurs_in_the_corpus` re-measures every
/// entry). A file absent here for a slot is one pco never put there, and
/// this pass moves the value rather than find out on a board.
pub fn attested_banks(op: &str, slot: usize) -> &'static [Bank] {
    use Bank::*;
    const TSV: &[Bank] = &[Temp, Special, VtxIn];
    match (op, slot) {
        ("fadd" | "fmul", 0 | 1) => TSV,
        ("fmad", 0..=2) => TSV,
        ("frcp" | "frsq", 0) => &[Temp, VtxIn],
        ("fexp" | "flog", 0) => &[Temp],
        /* The derivatives: 89 groups each, every one reading a temporary. */
        ("fdsx" | "fdsy", 0) => &[Temp],
        /* idx0: an indexed read (`shared[idx0 + k]`), as pco's program for
         * a dynamically indexed uniform reads it. The coefficient file: a
         * flat varying's plane constant, as pco reads it
         * (`fragment::flat_group`), and a plane's slopes for exact
         * gradients (`SelOp::PlaneCoeff`). */
        ("mbyp", 0) => &[Temp, Special, VtxIn, Shared, Idx0, Coeff],
        _ => &[],
    }
}

/// A source-modifier set as a mask: bit `3 * slot + k`, `k` 0 negate, 1
/// abs, 2 floor.
pub fn mods_mask(m: &[Mods; 3]) -> u16 {
    let mut x = 0u16;
    for (slot, m) in m.iter().enumerate() {
        x |= (u16::from(m.neg) | u16::from(m.abs) << 1 | u16::from(m.flr) << 2) << (3 * slot);
    }
    x
}

const N0: u16 = 1;
const A0: u16 = 2;
const F0: u16 = 4;
const A1: u16 = 2 << 3;
const N2: u16 = 1 << 6;
const F2: u16 = 4 << 6;

/// The source-modifier sets each operation carries in the offline table's
/// single-operation groups (usc-backend, 2026-09-23;
/// `every_attested_modifier_set_occurs_in_the_corpus` re-measures every
/// entry). A set outside these is split across moves by `prepare`, as the
/// register files outside `attested_banks` are.
pub fn attested_mods(op: &str) -> &'static [u16] {
    match op {
        "fadd" => &[0, A0, A0 | A1, F0, N0, N0 | A1, A1],
        "fmul" => &[0, A0, A0 | A1, F0, A1],
        "fmad" => &[0, A0, N0, N0 | F2, N0 | N2, A1, F2, N2],
        "fexp" => &[0, N0],
        "flog" => &[0, A0],
        "frcp" => &[0],
        "fdsx" | "fdsy" => &[0],
        "frsq" => &[0, N0],
        "mbyp" => &[0, A0, N0],
        _ => &[],
    }
}

/// The one hardware operation a group becomes, when it is one this pass and
/// the builder handle.
fn single_op(g: &SelGroup) -> Option<&'static str> {
    match g.op {
        SelOp::Copy => Some("mbyp"),
        SelOp::Sample { .. }
        | SelOp::WaitData
        | SelOp::TexWord { .. }
        | SelOp::SampleZ
        | SelOp::SampleGrad
        | SelOp::PlaneCoeff { .. }
        | SelOp::Machine(_)
        | SelOp::LoopBegin
        | SelOp::LoopEnd => None,
        SelOp::Alu(a) => match shape(a) {
            Shape::Group(steps) if steps.len() == 1 && steps[0].phase == PH_0 => Some(steps[0].op),
            _ => None,
        },
    }
}

/// How often each rewrite fired: a rewrite that never fires over a corpus
/// is one the corpus says nothing about, and its unit test is the only
/// evidence for it.
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct PrepareStats {
    /// Immediates read from a constant register.
    pub imm_const: u32,
    /// Immediates moved in with `bbyp0bm_imm32`.
    pub imm_move: u32,
    /// Operands moved because the operation is never seen reading their file.
    pub bank_move: u32,
    /// Products whose sign moved from the second factor to the first.
    pub sign_moved: u32,
    /// Sums whose operands were swapped to put a negation on slot 0.
    pub swapped: u32,
    /// Operands moved because no form carries their modifiers.
    pub mod_move: u32,
    /// Output components moved before the write.
    pub output_move: u32,
    /// Plain moves inserted in front of a modifying move.
    pub plain_first: u32,
    /// Square roots lowered to `frcp(frsq(x))`.
    pub sqrt: u32,
    /// b2f/b2i/f2b/i2b rewritten as a select or a compare.
    pub bool_rewrites: u32,
    /// Template operands moved into a plain register.
    pub template_move: u32,
    /// Operands whose abs was moved out to reach a modifier set pco uses.
    pub mod_split: u32,
    /// gl_PointSize values clamped to the part's range.
    pub point_size: u32,
}

struct Prep<'a> {
    arena: &'a Arena<'a>,
    out: AVec<'a, SelGroup>,
    next: u32,
    stats: PrepareStats,
}

impl<'a> Prep<'a> {
    fn fresh(&mut self) -> VReg {
        let v = VReg(self.next);
        self.next += 1;
        v
    }

    fn emit(&mut self, g: SelGroup) -> Result<(), PrepareError> {
        self.out.push(self.arena, g)?;
        Ok(())
    }

    /// An immediate as an operand: its constant register, or a fresh value
    /// an immediate move writes.
    fn imm(&mut self, v: u32) -> Result<SelSrc, PrepareError> {
        if let Some(idx) = const_reg(v) {
            self.stats.imm_const += 1;
            return Ok(SelSrc::Fixed { bank: Bank::Special, reg: idx, mods: Mods::default() });
        }
        self.stats.imm_move += 1;
        let d = self.fresh();
        self.emit(SelGroup {
            op: SelOp::Copy,
            ty: OpType::F32,
            dst: Some(SelDst::Virtual(d)),
            srcs: [Some(SelSrc::Imm(v)), None, None],
        })?;
        Ok(SelSrc::Virtual { v: d, mods: Mods::default() })
    }

    /// `s` moved into a fresh value, with its modifiers applied by the move.
    /// The result carries no modifiers.
    fn copy(&mut self, s: SelSrc) -> Result<SelSrc, PrepareError> {
        let s = match s {
            SelSrc::Imm(v) => self.imm(v)?,
            other => other,
        };
        let mods = match s {
            SelSrc::Virtual { mods, .. } | SelSrc::Fixed { mods, .. } => mods,
            SelSrc::Imm(_) => Mods::default(),
        };
        /* Several modifiers are applied one move each, in the order a source
         * applies them -- floor, abs, negate -- because the moves carry one
         * each in the corpus (mbyp: abs 729, neg 508, both never). */
        let kinds = u8::from(mods.flr) + u8::from(mods.abs) + u8::from(mods.neg);
        if kinds > 1 {
            let mut v = strip(s);
            for m in [
                Mods { flr: true, ..Mods::default() },
                Mods { abs: true, ..Mods::default() },
                Mods { neg: true, ..Mods::default() },
            ] {
                if (m.flr && mods.flr) || (m.abs && mods.abs) || (m.neg && mods.neg) {
                    v = self.copy(with_mods(v, m))?;
                }
            }
            return Ok(v);
        }
        let mut bare = strip(s);
        /* A move that APPLIES a modifier reads a temporary in every
         * reference group (all 729 single `mbyp`s with `abs`, all `neg`s):
         * the section form they use has no mux, and one reading a wider file
         * would. So a modified value from another file is moved plainly
         * first. */
        if mods != Mods::default() && !matches!(bare, SelSrc::Virtual { .. }) {
            self.stats.plain_first += 1;
            bare = self.copy(bare)?;
        }
        let d = self.fresh();
        let g = if mods.flr {
            /* `mbyp` has no floor field; `fmul` carries `s0flr` (with
             * `s0neg`/`s0abs`) and its second slot reads the special file. */
            SelGroup {
                op: SelOp::Alu(AluOp::Fmul),
                ty: OpType::F32,
                dst: Some(SelDst::Virtual(d)),
                srcs: [
                    Some(with_mods(bare, mods)),
                    Some(SelSrc::Fixed { bank: Bank::Special, reg: FONE, mods: Mods::default() }),
                    None,
                ],
            }
        } else {
            SelGroup {
                op: SelOp::Copy,
                ty: OpType::F32,
                dst: Some(SelDst::Virtual(d)),
                srcs: [Some(with_mods(bare, mods)), None, None],
            }
        };
        /* The move's own operand must be one it can read. */
        let op = single_op(&g).ok_or(PrepareError::Unsupported("a move with no single-operation shape"))?;
        if !bank_ok(op, 0, &bare) {
            return Err(PrepareError::Unsupported("a value in a file no move is seen reading"));
        }
        let m = [mods_of(&g.srcs[0]), Mods::default(), Mods::default()];
        if instr_region(op, PH_0, &m, &[]).is_err() {
            return Err(PrepareError::Unsupported("a modifier combination no move carries"));
        }
        self.emit(g)?;
        Ok(SelSrc::Virtual { v: d, mods: Mods::default() })
    }
}

fn strip(s: SelSrc) -> SelSrc {
    with_mods(s, Mods::default())
}

fn with_mods(s: SelSrc, mods: Mods) -> SelSrc {
    match s {
        SelSrc::Virtual { v, .. } => SelSrc::Virtual { v, mods },
        SelSrc::Fixed { bank, reg, .. } => SelSrc::Fixed { bank, reg, mods },
        SelSrc::Imm(v) => SelSrc::Imm(v),
    }
}

fn mods_of(s: &Option<SelSrc>) -> Mods {
    match s {
        Some(SelSrc::Virtual { mods, .. }) | Some(SelSrc::Fixed { mods, .. }) => *mods,
        _ => Mods::default(),
    }
}

/// Is `s`'s file one `op` is seen reading in `slot`? A virtual value will be
/// a temporary.
fn bank_ok(op: &str, slot: usize, s: &SelSrc) -> bool {
    let bank = match s {
        SelSrc::Virtual { .. } => Bank::Temp,
        SelSrc::Fixed { bank, .. } => *bank,
        SelSrc::Imm(_) => return false,
    };
    attested_banks(op, slot).contains(&bank)
}

/// Move a sign between the factors of a product, or swap a sum's operands,
/// when that lets the operation's own form carry what the sources need.
/// Exact: `a * -b == -a * b` and `a + b == b + a` for every float, zeros and
/// NaNs included, and a modifier applies abs before neg on both sides.
fn canonicalise(op: &str, srcs: &mut [Option<SelSrc>; 3], stats: &mut PrepareStats) {
    let neg1 = mods_of(&srcs[1]).neg;
    match op {
        "fmul" | "fmad" if neg1 => {
            let (m0, m1) = (mods_of(&srcs[0]), mods_of(&srcs[1]));
            if let (Some(a), Some(b)) = (srcs[0], srcs[1]) {
                srcs[0] = Some(with_mods(a, Mods { neg: !m0.neg, ..m0 }));
                srcs[1] = Some(with_mods(b, Mods { neg: false, ..m1 }));
                stats.sign_moved += 1;
            }
        }
        "fadd" if neg1 => {
            /* Only when the operand moving into slot 1 needs nothing slot 1
             * lacks: `fadd`'s second slot carries `abs` alone. */
            let m0 = mods_of(&srcs[0]);
            if !m0.neg && !m0.flr {
                srcs.swap(0, 1);
                stats.swapped += 1;
            }
        }
        _ => {}
    }
}

/// Close every gap between selection's groups and the builder's; see the
/// module documentation. Returns a new [`Selected`] whose groups, outputs and
/// virtual register count include what was inserted.
pub fn prepare<'a>(sel: &Selected<'a>, arena: &'a Arena<'a>) -> Result<Selected<'a>, PrepareError> {
    prepare_counted(sel, arena).map(|(s, _)| s)
}

/// [`prepare`], and how often each rewrite fired.
pub fn prepare_counted<'a>(
    sel: &Selected<'a>,
    arena: &'a Arena<'a>,
) -> Result<(Selected<'a>, PrepareStats), PrepareError> {
    /* Above every value anything names: the groups, the outputs, the inputs
     * and selection's own count. Two passes that each took "the highest the
     * groups name" would collide on an output-only value. */
    let mut next = sel.vregs;
    let mut note = |v: VReg| next = next.max(v.0 + 1);
    for g in sel.groups {
        if let Some(SelDst::Virtual(v)) = g.dst {
            note(v);
        }
        for s in g.srcs.iter().flatten() {
            if let SelSrc::Virtual { v, .. } = s {
                note(*v);
            }
        }
    }
    if let Some(SelSrc::Virtual { v, .. }) = sel.discard {
        note(v);
    }
    for (_, comps) in sel.outputs {
        for s in comps.iter().flatten() {
            if let SelSrc::Virtual { v, .. } = s {
                note(*v);
            }
        }
    }
    for (_, comps) in sel.inputs {
        for v in comps.iter().flatten() {
            note(*v);
        }
    }
    /* Sized up front: an AVec grown by doubling leaves every smaller buffer
     * behind in the arena, and the moves this pass adds are a fraction of
     * the program (femtovg's filter_image: 1,500 groups in, 1,504 out). */
    let out = AVec::with_capacity(arena, sel.groups.len() + sel.groups.len() / 2)?;
    let mut p = Prep { arena, out, next, stats: PrepareStats::default() };

    for g in sel.groups {
        /* `sqrt(x)` is `frcp(frsq(x))`, which is what pco emits for it (NIR
         * `lower_fsqrt`; measured on a `sqrt(u)` shader with pco_shc
         * build-new, 2026-09-23). Both halves go round this loop again, so
         * their operands are prepared like any other. */
        if g.op == SelOp::Alu(AluOp::Fsqrt) {
            let (Some(x), Some(d)) = (g.srcs[0], g.dst) else {
                return Err(PrepareError::Unsupported("a sqrt with no operand or destination"));
            };
            let t = p.fresh();
            p.stats.sqrt += 1;
            for h in [
                SelGroup {
                    op: SelOp::Alu(AluOp::Frsq),
                    ty: g.ty,
                    dst: Some(SelDst::Virtual(t)),
                    srcs: [Some(x), None, None],
                },
                SelGroup {
                    op: SelOp::Alu(AluOp::Frcp),
                    ty: g.ty,
                    dst: Some(d),
                    srcs: [Some(SelSrc::Virtual { v: t, mods: Mods::default() }), None, None],
                },
            ] {
                prepare_one(&mut p, h)?;
            }
            continue;
        }
        prepare_one(&mut p, *g)?;
    }
    finish_outputs(&mut p, sel, arena)
}

/// Prepare one group into `p.out`.
fn prepare_one(p: &mut Prep<'_>, g: SelGroup) -> Result<(), PrepareError> {
    /* Booleans to numbers and numbers to booleans are the select and the
     * compare with constant operands, which is what NIR hands pco for them:
     * `b2f32(b)` is `bcsel(b, 1.0, 0.0)`, `f2b(x)` is `x != 0.0`. Our boolean
     * is all-ones, as the compare templates produce it. */
    if let SelOp::Alu(a) = g.op {
        let imm = |v: u32| Some(SelSrc::Imm(v));
        let (op, srcs) = match a {
            AluOp::B2f => (AluOp::Bcsel, [g.srcs[0], imm(1.0f32.to_bits()), imm(0)]),
            AluOp::B2i => (AluOp::Bcsel, [g.srcs[0], imm(1), imm(0)]),
            AluOp::F2b => (AluOp::Fne, [g.srcs[0], imm(0), None]),
            AluOp::I2b => (AluOp::Ine, [g.srcs[0], imm(0), None]),
            /* The boolean not: our booleans are all-ones or zero, so it is
             * `b == 0`, the same compare template. (pco XNORs with zero in
             * the bitwise pipeline, which this builder does not emit.) */
            AluOp::Bnot => (AluOp::Ieq, [g.srcs[0], imm(0), None]),
            _ => (a, g.srcs),
        };
        if op != a {
            p.stats.bool_rewrites += 1;
            return prepare_one(p, SelGroup { op: SelOp::Alu(op), srcs, ..g });
        }
    }
    {
        /* A template's operands are plain registers in the temporaries or
         * the constants: an immediate becomes its constant or a move, and
         * anything with a modifier or in another file is moved. */
        if super::shapes::template_for(g.op).is_some() {
            let mut g = g;
            for i in 0..g.op.arity() {
                let s = g.srcs[i].ok_or(PrepareError::Unsupported("a template operand that is missing"))?;
                let s = match s {
                    SelSrc::Imm(v) => p.imm(v)?,
                    other => other,
                };
                let plain = match s {
                    SelSrc::Virtual { mods, .. } => mods == Mods::default(),
                    SelSrc::Fixed { bank, mods, .. } => {
                        mods == Mods::default() && matches!(bank, Bank::Temp | Bank::Special)
                    }
                    SelSrc::Imm(_) => false,
                };
                g.srcs[i] = Some(if plain {
                    s
                } else {
                    p.stats.template_move += 1;
                    p.copy(s)?
                });
            }
            return p.emit(g);
        }
    }
    {
        let g = &g;
        /* Only what the builder transcribes: anything else is passed on for
         * the builder to refuse by name, rather than refused here with a
         * reason that hides which operation it was. */
        let Some(op) = single_op(g).filter(|o| super::assemble::TRANSCRIBED_SINGLE.contains(o)) else {
            /* Left for the builder to refuse by name. */
            p.emit(*g)?;
            return Ok(());
        };
        let mut g = *g;

        /* An immediate move needs nothing unless the value is a constant
         * register, which is cheaper than the move and what pco does. */
        if let (SelOp::Copy, Some(SelSrc::Imm(v))) = (g.op, g.srcs[0]) {
            if let Some(idx) = const_reg(v) {
                g.srcs[0] = Some(SelSrc::Fixed { bank: Bank::Special, reg: idx, mods: Mods::default() });
            }
            p.emit(g)?;
            return Ok(());
        }

        let n = g.op.arity();
        for i in 0..n {
            if let Some(SelSrc::Imm(v)) = g.srcs[i] {
                g.srcs[i] = Some(p.imm(v)?);
            }
        }
        canonicalise(op, &mut g.srcs, &mut p.stats);

        for i in 0..n {
            let Some(s) = g.srcs[i] else { continue };
            if !bank_ok(op, i, &s) {
                p.stats.bank_move += 1;
                g.srcs[i] = Some(p.copy(s)?);
            }
        }
        /* A selection copy that applies a modifier to a value outside the
         * temporaries: the same rule as `Prep::copy`, one plain move first. */
        if op == "mbyp" {
            if let Some(s) = g.srcs[0] {
                let m = mods_of(&g.srcs[0]);
                if m != Mods::default() && !matches!(s, SelSrc::Virtual { .. }) {
                    p.stats.plain_first += 1;
                    g.srcs[0] = Some(with_mods(p.copy(strip(s))?, m));
                }
            }
        }
        /* Whatever modifiers the form still cannot carry, one slot at a
         * time, as the form chooser names them. Bounded: each round clears
         * one slot's modifiers, and a slot without any never fails. */
        for _ in 0..3 {
            let m = [mods_of(&g.srcs[0]), mods_of(&g.srcs[1]), mods_of(&g.srcs[2])];
            match instr_region(op, PH_0, &m, &[]) {
                Ok(_) => break,
                Err(NoForm::Modifier { slot, .. }) => {
                    let s = g.srcs[slot].ok_or(PrepareError::Unsupported("a modifier on a slot with no operand"))?;
                    p.stats.mod_move += 1;
                    g.srcs[slot] = Some(p.copy(s)?);
                }
                Err(_) => return Err(PrepareError::Unsupported("no form of the operation carries its modifiers")),
            }
        }
        /* And to a set the corpus shows. First by moving an operand's
         * floor and abs out and leaving the negate (`-|a| + b` becomes
         * `-(t) + b` with `t = |a|`), then by moving the operand's modifiers
         * out whole, one slot at a time. The copy takes the LEADING part of
         * the order a set applies in (floor, abs, negate): moving the abs
         * alone and leaving the floor on the consumer turned |floor(x)|
         * into floor(|x|) (found running abs(floor(x)) against the IR). */
        let n = g.op.arity();
        let ms = |g: &SelGroup| [mods_of(&g.srcs[0]), mods_of(&g.srcs[1]), mods_of(&g.srcs[2])];
        let attested = |g: &SelGroup| attested_mods(op).contains(&mods_mask(&ms(g)));
        'fix: while !attested(&g) {
            for slot in 0..n {
                let m = ms(&g)[slot];
                if m.abs && m.neg {
                    let mut t = g;
                    let rest = Mods { neg: true, abs: false, flr: false };
                    t.srcs[slot] = t.srcs[slot].map(|x| with_mods(x, rest));
                    if attested(&t) {
                        let s =
                            g.srcs[slot].ok_or(PrepareError::Unsupported("a modifier on a slot with no operand"))?;
                        let moved = p.copy(with_mods(s, Mods { neg: false, ..m }))?;
                        p.stats.mod_split += 1;
                        g.srcs[slot] = Some(with_mods(moved, rest));
                        continue 'fix;
                    }
                }
            }
            let Some(slot) = (0..n).find(|&i| ms(&g)[i] != Mods::default()) else {
                return Err(PrepareError::Unsupported("an operation whose bare form the corpus does not show"));
            };
            let s = g.srcs[slot].ok_or(PrepareError::Unsupported("a modifier on a slot with no operand"))?;
            p.stats.mod_move += 1;
            g.srcs[slot] = Some(p.copy(s)?);
        }
        p.emit(g)?;
    }
    Ok(())
}

/// Close over the outputs and hand back the prepared program.
fn finish_outputs<'a>(
    p: &mut Prep<'a>,
    sel: &Selected<'a>,
    arena: &'a Arena<'a>,
) -> Result<(Selected<'a>, PrepareStats), PrepareError> {
    /* The outputs. */
    let mut outs: AVec<'a, (IoSlot, [Option<SelSrc>; 4])> = AVec::new();
    for (slot, comps) in sel.outputs {
        let mut c2 = *comps;
        /* gl_PointSize is clamped to the part's range, as NIR's
         * `nir_lower_point_size` does for pco with PVR_POINT_SIZE_RANGE_MIN
         * and _MAX (Mesa `src/imagination/common/pvr_limits.h`: 1.0 and
         * 511.0): max against the floor, then min against the ceiling. */
        if *slot == IoSlot::PointSize {
            if let Some(x) = c2[0] {
                let lo = p.fresh();
                let hi = p.fresh();
                prepare_one(
                    p,
                    SelGroup {
                        op: SelOp::Alu(AluOp::Fmax),
                        ty: OpType::F32,
                        dst: Some(SelDst::Virtual(lo)),
                        srcs: [Some(x), Some(SelSrc::Imm(POINT_SIZE_MIN.to_bits())), None],
                    },
                )?;
                prepare_one(
                    p,
                    SelGroup {
                        op: SelOp::Alu(AluOp::Fmin),
                        ty: OpType::F32,
                        dst: Some(SelDst::Virtual(hi)),
                        srcs: [
                            Some(SelSrc::Virtual { v: lo, mods: Mods::default() }),
                            Some(SelSrc::Imm(POINT_SIZE_MAX.to_bits())),
                            None,
                        ],
                    },
                )?;
                p.stats.point_size += 1;
                c2[0] = Some(SelSrc::Virtual { v: hi, mods: Mods::default() });
            }
        }
        for c in c2.iter_mut() {
            let Some(s) = *c else { continue };
            let ok = match s {
                SelSrc::Virtual { mods, .. } => mods == Mods::default(),
                SelSrc::Fixed { bank, mods, .. } => {
                    mods == Mods::default() && super::assemble::UVSW_WRITE_BANKS.contains(&bank)
                }
                SelSrc::Imm(_) => false,
            };
            if ok {
                continue;
            }
            p.stats.output_move += 1;
            *c = Some(match s {
                SelSrc::Imm(v) => match p.imm(v)? {
                    f @ SelSrc::Fixed { .. } => f,
                    other => other,
                },
                other => p.copy(other)?,
            });
        }
        outs.push(arena, (*slot, c2))?;
    }

    /* The discard flag is read by the builder's `tst`, which takes a plain
     * register like any template operand. */
    let discard = match sel.discard {
        None => None,
        Some(s @ SelSrc::Virtual { mods, .. }) if mods == Mods::default() => Some(s),
        Some(other) => Some(p.copy(other)?),
    };
    /* Alpha-to-one's two values, likewise plain registers the builder's
     * blend tail reads. */
    let plain = |p: &mut Prep<'a>, s: SelSrc| -> Result<SelSrc, PrepareError> {
        match s {
            SelSrc::Virtual { mods, .. } if mods == Mods::default() => Ok(s),
            other => p.copy(other),
        }
    };
    let alpha_to_one = match sel.alpha_to_one {
        None => None,
        Some((a, f)) => Some((plain(p, a)?, plain(p, f)?)),
    };
    let stats = p.stats;
    let out = core::mem::take(&mut p.out);
    Ok((
        Selected {
            groups: out.into_slice(),
            vregs: p.next,
            outputs: outs.into_slice(),
            inputs: sel.inputs,
            discard,
            alpha_to_one,
        },
        stats,
    ))
}

/// `sel` with every group that recomputes an earlier group's value removed
/// and its readers pointed at the earlier value: common subexpressions, over
/// the straight-line program selection produces.
///
/// It matters because loops are unrolled: every iteration of femtovg's
/// filter_image moved the same two uniforms into temporaries and took the
/// same `rcp(extent)` again (24 iterations, 102 reciprocals).
///
/// A group is shared only when its value cannot differ between the two
/// points: an ALU operation or a move into a virtual register written once,
/// every source a virtual register written at most once, a uniform, a
/// special register, a vertex input or an immediate. A fixed temporary is
/// never a shareable source (the sample blocks are rewritten per sample)
/// and neither is an `idx0` read (the index is rewritten).
pub fn share_common<'a>(sel: &Selected<'a>, arena: &'a Arena<'a>) -> Result<Selected<'a>, PrepareError> {
    let n = sel.vregs as usize;
    let defs = arena.alloc_filled(n, 0u8)?;
    let bump = |v: VReg, defs: &mut [u8]| {
        if let Some(d) = defs.get_mut(v.0 as usize) {
            *d = d.saturating_add(1);
        }
    };
    for g in sel.groups {
        if let Some(SelDst::Virtual(v)) = g.dst {
            bump(v, defs);
            /* A sample writes a block of four from its first. */
            if matches!(g.op, SelOp::Sample { .. }) {
                for k in 1..4 {
                    bump(VReg(v.0 + k), defs);
                }
            }
        }
    }
    let defs: &[u8] = defs;
    let once = |v: VReg| defs.get(v.0 as usize).is_some_and(|d| *d <= 1);
    let to = arena.alloc_filled(n, VReg(u32::MAX))?;
    for (i, t) in to.iter_mut().enumerate() {
        *t = VReg(i as u32);
    }
    let rename = |s: SelSrc, to: &[VReg]| match s {
        SelSrc::Virtual { v, mods } => SelSrc::Virtual { v: to.get(v.0 as usize).copied().unwrap_or(v), mods },
        other => other,
    };
    let pure_src = |s: &SelSrc| match s {
        SelSrc::Virtual { v, .. } => once(*v),
        SelSrc::Fixed { bank, .. } => matches!(bank, Bank::Shared | Bank::Special | Bank::VtxIn),
        SelSrc::Imm(_) => true,
    };
    let mut out: AVec<'a, SelGroup> = AVec::with_capacity(arena, sel.groups.len())?;
    /* Indices into `out` of the groups a later one may reuse. */
    let mut kept: AVec<'a, u32> = AVec::with_capacity(arena, sel.groups.len())?;
    for g in sel.groups {
        let mut g = *g;
        for s in g.srcs.iter_mut().flatten() {
            *s = rename(*s, to);
        }
        let shareable = matches!(g.op, SelOp::Alu(_) | SelOp::Copy)
            && matches!(g.dst, Some(SelDst::Virtual(v)) if once(v))
            && g.srcs.iter().flatten().all(pure_src);
        if shareable {
            let Some(SelDst::Virtual(v)) = g.dst else { unreachable!() };
            let same = kept
                .as_slice()
                .iter()
                .map(|i| &out.as_slice()[*i as usize])
                .find(|e| e.op == g.op && e.ty == g.ty && e.srcs == g.srcs);
            if let Some(Some(SelDst::Virtual(e))) = same.map(|e| e.dst) {
                to[v.0 as usize] = e;
                continue;
            }
            kept.push(arena, out.len() as u32)?;
        }
        out.push(arena, g)?;
    }
    let to: &[VReg] = to;
    let outputs = arena.alloc_copy(sel.outputs)?;
    for (_, c) in outputs.iter_mut() {
        for s in c.iter_mut().flatten() {
            *s = rename(*s, to);
        }
    }
    let discard = sel.discard.map(|s| rename(s, to));
    let alpha_to_one = sel.alpha_to_one.map(|(a, f)| (rename(a, to), rename(f, to)));
    Ok(Selected { groups: out.into_slice(), vregs: sel.vregs, outputs, inputs: sel.inputs, discard, alpha_to_one })
}

/// The first shared register a source can NOT name directly: 256.
///
/// The source forms carry an 11-bit register index (`3b11i`), but the
/// hardware does not honour bits 8..10 on a direct shared read. Measured on
/// the AM62P (records/am62p/shared-size, gl_test 150): a vertex `mbyp` of
/// sh256+k, index bits 8..10 set in the encoding, returns what sh<k> holds.
/// Mesa's pco never emits such an operand: `try_legalize_large_hwreg_offsets`
/// (src/imagination/pco/pco_legalize.c, MIT) rewrites every hardware-register
/// operand at 256 or above, of any register class, into a `movi32` of the
/// index into index register 0 and a read through it. [`index_high_shared`]
/// does the same.
pub const SHARED_DIRECT_LIMIT: u16 = 256;

/// `sel` with every read of a shared register at [`SHARED_DIRECT_LIMIT`] or
/// above made through index register 0, as pco does it: before the group,
/// `idx0 = m` (`bbyp0bm_imm32`), `m` the lowest such register the group
/// reads, and each of them read as `shared[idx0 + (n - m)]`. Registers
/// below the limit stay direct.
///
/// It runs after [`prepare_counted`] and [`share_common`], on the final
/// register numbers. `prepare` has moved every uniform into an `mbyp` by
/// then, so the groups that still read the shared file are those moves (an
/// `mbyp` takes an `idx0` source in slot 0, [`attested_banks`]) and a
/// sample's descriptor words (image and sampler state, `build_smp`).
/// `share_common` must not run on the result: it would merge two `idx0`
/// reads whose index differs.
///
/// idx0 is set right before each group, so a dynamically indexed read
/// (`LoadUniformIdx`, which sets idx0 too) between two of them is safe.
///
/// Anything else that reads past the limit is refused, not wrapped: another
/// operation, an output read straight from the shared file, or registers
/// 256 or more apart in one group (one index register cannot reach both;
/// pco asserts the same).
pub fn index_high_shared<'a>(sel: &Selected<'a>, arena: &'a Arena<'a>) -> Result<Selected<'a>, PrepareError> {
    let high = |s: &SelSrc| matches!(s, SelSrc::Fixed { bank: Bank::Shared, reg, .. } if *reg >= SHARED_DIRECT_LIMIT);
    let mut side = sel.outputs.iter().flat_map(|(_, c)| c.iter().flatten()).chain(sel.discard.iter());
    if side.any(high) || sel.alpha_to_one.is_some_and(|(x, f)| high(&x) || high(&f)) {
        return Err(PrepareError::Unsupported("an output read straight from a shared register past sh255"));
    }
    let n_high = sel.groups.iter().filter(|g| g.srcs.iter().flatten().any(high)).count();
    let same = |groups| Selected {
        groups,
        vregs: sel.vregs,
        outputs: sel.outputs,
        inputs: sel.inputs,
        discard: sel.discard,
        alpha_to_one: sel.alpha_to_one,
    };
    /* Nothing to rewrite (every shader below sh256): no copy in the arena. */
    if n_high == 0 {
        return Ok(same(sel.groups));
    }
    let mut out: AVec<'a, SelGroup> = AVec::with_capacity(arena, sel.groups.len() + n_high)?;
    for g in sel.groups {
        let mut g = *g;
        let mut lo = u16::MAX;
        let mut hi = 0u16;
        for (slot, s) in g.srcs.iter().enumerate() {
            let Some(SelSrc::Fixed { reg, .. }) = s.filter(high) else { continue };
            let slot_ok = match g.op {
                SelOp::Copy => slot == 0,
                SelOp::Sample { .. } => slot != 1,
                _ => false,
            };
            if !slot_ok {
                return Err(PrepareError::Unsupported(
                    "a shared register past sh255 read by a group other than a move or a sample",
                ));
            }
            lo = lo.min(reg);
            hi = hi.max(reg);
        }
        if lo != u16::MAX {
            if hi - lo >= SHARED_DIRECT_LIMIT {
                return Err(PrepareError::Unsupported(
                    "shared registers past sh255 too far apart for one index register",
                ));
            }
            out.push(
                arena,
                SelGroup {
                    op: SelOp::Copy,
                    ty: OpType::U32,
                    dst: Some(SelDst::Fixed { bank: Bank::Idx0, reg: super::sel::IDX_SELF }),
                    srcs: [Some(SelSrc::Imm(u32::from(lo))), None, None],
                },
            )?;
            for s in g.srcs.iter_mut() {
                if let Some(SelSrc::Fixed { reg, mods, .. }) = s.filter(high) {
                    *s = Some(SelSrc::Fixed { bank: Bank::Idx0, reg: super::sel::idx_shared(reg - lo), mods });
                }
            }
        }
        out.push(arena, g)?;
    }
    Ok(same(out.into_slice()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::vec::Vec;

    const SHARED: [f32; 8] = [1.5, -2.25, 3.75, -0.5, 7.0, -1.75, 0.3, -9.5];
    const VTXIN: [f32; 4] = [-2.6, 0.7, 5.5, -3.2];

    fn fixed(bank: Bank, reg: u16, mods: Mods) -> SelSrc {
        SelSrc::Fixed { bank, reg, mods }
    }
    fn virt(v: u32, mods: Mods) -> SelSrc {
        SelSrc::Virtual { v: VReg(v), mods }
    }
    const NEG: Mods = Mods { neg: true, abs: false, flr: false };
    const FLR: Mods = Mods { neg: false, abs: false, flr: true };
    const NONE: Mods = Mods { neg: false, abs: false, flr: false };

    fn g(op: SelOp, dst: u32, srcs: [Option<SelSrc>; 3]) -> SelGroup {
        SelGroup { op, ty: OpType::F32, dst: Some(SelDst::Virtual(VReg(dst))), srcs }
    }

    /// Evaluate a program at the selection level: the meaning both sides of
    /// a rewrite must share. Modifiers apply floor, then abs, then negate.
    fn eval(groups: &[SelGroup], outputs: &[(IoSlot, [Option<SelSrc>; 4])]) -> Vec<u32> {
        let mut v: BTreeMap<u32, f32> = BTreeMap::new();
        let read = |s: &SelSrc, v: &BTreeMap<u32, f32>| -> f32 {
            let (x, m) = match *s {
                SelSrc::Virtual { v: r, mods } => (v[&r.0], mods),
                SelSrc::Fixed { bank: Bank::Shared, reg, mods } => (SHARED[reg as usize], mods),
                SelSrc::Fixed { bank: Bank::VtxIn, reg, mods } => (VTXIN[reg as usize], mods),
                SelSrc::Fixed { bank: Bank::Special, reg, mods } => {
                    (f32::from_bits(CONST_REGS.iter().find(|(_, i)| *i == reg).expect("a known constant").0), mods)
                }
                SelSrc::Fixed { .. } => panic!("an unexpected fixed register"),
                SelSrc::Imm(b) => (f32::from_bits(b), NONE),
            };
            let mut x = x;
            if m.flr {
                x = x.floor();
            }
            if m.abs {
                x = x.abs();
            }
            if m.neg {
                x = -x;
            }
            x
        };
        for gr in groups {
            let a: Vec<f32> = gr.srcs.iter().take(gr.op.arity()).map(|s| read(s.as_ref().unwrap(), &v)).collect();
            let r = match gr.op {
                SelOp::Copy => a[0],
                SelOp::Alu(AluOp::Fadd) => a[0] + a[1],
                SelOp::Alu(AluOp::Fmul) => a[0] * a[1],
                SelOp::Alu(AluOp::Ffma) => a[0] * a[1] + a[2],
                SelOp::Alu(AluOp::Frsq) => 1.0 / a[0].sqrt(),
                SelOp::Alu(AluOp::Frcp) => 1.0 / a[0],
                SelOp::Alu(AluOp::Fsqrt) => a[0].sqrt(),
                SelOp::Alu(o) => panic!("{o:?} is not in this evaluator"),
                o @ (SelOp::Sample { .. }
                | SelOp::WaitData
                | SelOp::TexWord { .. }
                | SelOp::SampleZ
                | SelOp::SampleGrad
                | SelOp::PlaneCoeff { .. }
                | SelOp::Machine(_)
                | SelOp::LoopBegin
                | SelOp::LoopEnd) => {
                    panic!("{o:?} is not in this evaluator")
                }
            };
            let Some(SelDst::Virtual(d)) = gr.dst else { panic!("a fixed destination") };
            v.insert(d.0, r);
        }
        outputs
            .iter()
            .flat_map(|(_, c)| c.iter().flatten().map(|s| read(s, &v).to_bits()).collect::<Vec<_>>())
            .collect()
    }

    /// Prepare `groups`, check the rewrite named by `fired` happened, that
    /// every group left is one the builder takes as it stands, and that the
    /// program still computes the same bits.
    fn same_meaning(groups: &[SelGroup], out: [Option<SelSrc>; 4], fired: fn(&PrepareStats) -> u32) {
        let outputs = [(IoSlot::Position, out)];
        let sel = Selected { groups, vregs: 16, outputs: &outputs, inputs: &[], discard: None, alpha_to_one: None };
        let mut buf = std::vec![0u8; 64 << 10];
        let arena = Arena::new(&mut buf);
        let (p, st) = prepare_counted(&sel, &arena).expect("prepares");
        assert!(fired(&st) > 0, "the rewrite under test did not fire: {st:?}");
        for gr in p.groups {
            let op = single_op(gr).expect("a single operation");
            if let (SelOp::Copy, Some(SelSrc::Imm(_))) = (gr.op, gr.srcs[0]) {
                continue;
            }
            for i in 0..gr.op.arity() {
                assert!(bank_ok(op, i, gr.srcs[i].as_ref().unwrap()), "{gr:?}: slot {i} still unreadable");
            }
            let m = [mods_of(&gr.srcs[0]), mods_of(&gr.srcs[1]), mods_of(&gr.srcs[2])];
            assert!(instr_region(op, PH_0, &m, &[]).is_ok(), "{gr:?}: modifiers no form carries");
            assert!(attested_mods(op).contains(&mods_mask(&m)), "{gr:?}: a modifier set pco never uses");
        }
        assert_eq!(eval(groups, &outputs), eval(p.groups, p.outputs), "the rewrite changed what the program computes");
    }

    fn pos(v: u32) -> [Option<SelSrc>; 4] {
        [Some(virt(v, NONE)), None, None, None]
    }

    /// A recomputation is dropped and its readers take the first value; a
    /// value read from a fixed temporary, or written twice, is never shared.
    #[test]
    fn a_repeated_value_is_computed_once_and_a_temporary_read_is_not_shared() {
        let temp = |r: u16| fixed(Bank::Temp, r, NONE);
        let groups = [
            g(SelOp::Copy, 1, [Some(fixed(Bank::Shared, 5, NONE)), None, None]),
            g(SelOp::Alu(AluOp::Frcp), 2, [Some(virt(1, NONE)), None, None]),
            /* The same two again: both go, and 4's reader reads 2. */
            g(SelOp::Copy, 3, [Some(fixed(Bank::Shared, 5, NONE)), None, None]),
            g(SelOp::Alu(AluOp::Frcp), 4, [Some(virt(3, NONE)), None, None]),
            g(SelOp::Alu(AluOp::Fmul), 5, [Some(virt(2, NONE)), Some(virt(4, NEG)), None]),
            /* Reads of a fixed temporary: kept, whatever they look like. */
            g(SelOp::Copy, 6, [Some(temp(20)), None, None]),
            g(SelOp::Copy, 7, [Some(temp(20)), None, None]),
            /* Register 8 is written twice: neither write is shared. */
            g(SelOp::Alu(AluOp::Fadd), 8, [Some(virt(5, NONE)), Some(virt(5, NONE)), None]),
            g(SelOp::Alu(AluOp::Fadd), 8, [Some(virt(5, NONE)), Some(virt(5, NONE)), None]),
            g(SelOp::Alu(AluOp::Fadd), 9, [Some(virt(8, NONE)), Some(virt(6, NONE)), None]),
            g(SelOp::Alu(AluOp::Fadd), 10, [Some(virt(9, NONE)), Some(virt(7, NONE)), None]),
        ];
        let outputs = [(IoSlot::Position, pos(10))];
        let sel =
            Selected { groups: &groups, vregs: 16, outputs: &outputs, inputs: &[], discard: None, alpha_to_one: None };
        let mut buf = std::vec![0u8; 64 << 10];
        let arena = Arena::new(&mut buf);
        let p = share_common(&sel, &arena).expect("shares");
        assert_eq!(p.groups.len(), groups.len() - 2, "{:?}", p.groups);
        assert_eq!(p.groups[2].srcs[1], Some(virt(2, NEG)), "the reader of 4 reads 2, its modifier kept");
        assert_eq!(p.groups.iter().filter(|x| x.srcs[0] == Some(temp(20))).count(), 2);
        assert_eq!(p.groups.iter().filter(|x| x.dst == Some(SelDst::Virtual(VReg(8)))).count(), 2);
        assert_eq!(p.outputs, &outputs[..]);
    }

    /// A move of sh255 stays direct; a move of sh256 or sh279 becomes
    /// `idx0 = n` and a read of shared[idx0 + 0], its modifier kept; a
    /// sample's descriptor words likewise; any other group reading past sh255
    /// is refused, never wrapped.
    #[test]
    fn a_shared_read_past_sh255_goes_through_idx0() {
        use super::super::sel::{idx_shared, SmpLod, IDX_SELF};
        let idx_write = |n: u32| SelGroup {
            op: SelOp::Copy,
            ty: OpType::U32,
            dst: Some(SelDst::Fixed { bank: Bank::Idx0, reg: IDX_SELF }),
            srcs: [Some(SelSrc::Imm(n)), None, None],
        };
        let groups = [
            g(SelOp::Copy, 1, [Some(fixed(Bank::Shared, 255, NONE)), None, None]),
            g(SelOp::Copy, 2, [Some(fixed(Bank::Shared, 256, NONE)), None, None]),
            g(SelOp::Copy, 3, [Some(fixed(Bank::Shared, 279, NEG)), None, None]),
        ];
        let outputs = [(IoSlot::Position, pos(3))];
        let sel =
            Selected { groups: &groups, vregs: 8, outputs: &outputs, inputs: &[], discard: None, alpha_to_one: None };
        let mut buf = std::vec![0u8; 64 << 10];
        let arena = Arena::new(&mut buf);
        let p = index_high_shared(&sel, &arena).expect("rewrites");
        let via = |mods| Some(fixed(Bank::Idx0, idx_shared(0), mods));
        assert_eq!(
            p.groups,
            &[
                groups[0],
                idx_write(256),
                g(SelOp::Copy, 2, [via(NONE), None, None]),
                idx_write(279),
                g(SelOp::Copy, 3, [via(NEG), None, None]),
            ][..]
        );

        for bad in [
            g(SelOp::Alu(AluOp::Fmul), 4, [Some(virt(1, NONE)), Some(fixed(Bank::Shared, 256, NONE)), None]),
            g(SelOp::Alu(AluOp::Fmul), 4, [Some(fixed(Bank::Shared, 300, NONE)), Some(virt(1, NONE)), None]),
        ] {
            let groups = [bad];
            let sel =
                Selected { groups: &groups, vregs: 8, outputs: &[], inputs: &[], discard: None, alpha_to_one: None };
            assert!(matches!(index_high_shared(&sel, &arena), Err(PrepareError::Unsupported(_))), "{bad:?} refused");
        }
        let outputs = [(IoSlot::Position, [Some(fixed(Bank::Shared, 260, NONE)), None, None, None])];
        let sel = Selected { groups: &[], vregs: 8, outputs: &outputs, inputs: &[], discard: None, alpha_to_one: None };
        assert!(matches!(index_high_shared(&sel, &arena), Err(PrepareError::Unsupported(_))), "an output refused");

        /* A sample of descriptor 13 (20 dwords each): image sh260, sampler
         * sh268, one idx0 = 260 for both, as pco legalises it. A descriptor
         * below sh256 stays direct beside one past it. */
        let smp = |image: u16, sampler: u16| SelGroup {
            op: SelOp::Sample {
                binding: 13,
                proj: false,
                lod: SmpLod::Implicit,
                fetch: false,
                offset: None,
                cube: false,
            },
            ty: OpType::F32,
            dst: Some(SelDst::Fixed { bank: Bank::Temp, reg: 4 }),
            srcs: [
                Some(fixed(Bank::Shared, image, NONE)),
                Some(fixed(Bank::Temp, 0, NONE)),
                Some(fixed(Bank::Shared, sampler, NONE)),
            ],
        };
        let groups = [smp(260, 268), smp(240, 256)];
        let sel = Selected { groups: &groups, vregs: 8, outputs: &[], inputs: &[], discard: None, alpha_to_one: None };
        let p = index_high_shared(&sel, &arena).expect("rewrites");
        let mut want0 = groups[0];
        want0.srcs[0] = via(NONE);
        want0.srcs[2] = Some(fixed(Bank::Idx0, idx_shared(8), NONE));
        let mut want1 = groups[1];
        want1.srcs[2] = via(NONE);
        assert_eq!(p.groups, &[idx_write(260), want0, idx_write(256), want1][..]);

        /* 256 apart: one index register cannot reach both */
        let groups = [smp(256, 512)];
        let sel = Selected { groups: &groups, vregs: 8, outputs: &[], inputs: &[], discard: None, alpha_to_one: None };
        assert!(matches!(index_high_shared(&sel, &arena), Err(PrepareError::Unsupported(_))), "too far apart");
    }

    /* v0 = vtxin0, v1 = vtxin1: ordinary temporaries to operate on. */
    fn base() -> Vec<SelGroup> {
        std::vec![
            g(SelOp::Copy, 0, [Some(fixed(Bank::VtxIn, 0, NONE)), None, None]),
            g(SelOp::Copy, 1, [Some(fixed(Bank::VtxIn, 1, NONE)), None, None]),
        ]
    }

    #[test]
    fn a_products_sign_moves_to_the_first_factor() {
        let mut p = base();
        p.push(g(SelOp::Alu(AluOp::Fmul), 2, [Some(virt(0, NONE)), Some(virt(1, NEG)), None]));
        same_meaning(&p, pos(2), |s| s.sign_moved);
        let mut p = base();
        p.push(g(SelOp::Alu(AluOp::Ffma), 2, [Some(virt(0, NEG)), Some(virt(1, NEG)), Some(virt(0, NONE))]));
        same_meaning(&p, pos(2), |s| s.sign_moved);
    }

    #[test]
    fn a_sum_swaps_to_put_its_negation_first() {
        let mut p = base();
        p.push(g(SelOp::Alu(AluOp::Fadd), 2, [Some(virt(0, NONE)), Some(virt(1, NEG)), None]));
        same_meaning(&p, pos(2), |s| s.swapped);
    }

    #[test]
    fn a_modifier_no_form_carries_is_applied_by_a_move() {
        /* Both operands negated: no swap helps, so the second is moved. */
        let mut p = base();
        p.push(g(SelOp::Alu(AluOp::Fadd), 2, [Some(virt(0, NEG)), Some(virt(1, NEG)), None]));
        same_meaning(&p, pos(2), |s| s.mod_move);
        /* A floor on the second factor: `fmul` has no `s1flr`, so it becomes
         * `fmul(flr(x), 1.0)`. */
        let mut p = base();
        p.push(g(SelOp::Alu(AluOp::Fmul), 2, [Some(virt(0, NONE)), Some(virt(1, FLR)), None]));
        same_meaning(&p, pos(2), |s| s.mod_move);
    }

    /// `-|a| + b`: no `fadd` in the corpus negates and takes the absolute
    /// value of one operand, so the abs moves into an `mbyp` and the sum
    /// keeps the negate, a set pco uses 26,942 times.
    #[test]
    fn a_modifier_set_pco_never_uses_is_split() {
        let na = Mods { neg: true, abs: true, flr: false };
        let mut p = base();
        p.push(g(SelOp::Alu(AluOp::Fadd), 2, [Some(virt(0, na)), Some(virt(1, NONE)), None]));
        same_meaning(&p, pos(2), |s| s.mod_split);
    }

    #[test]
    fn a_uniform_is_moved_before_arithmetic_reads_it() {
        let mut p = base();
        p.push(g(SelOp::Alu(AluOp::Fmul), 2, [Some(fixed(Bank::Shared, 3, NONE)), Some(virt(1, NONE)), None]));
        same_meaning(&p, pos(2), |s| s.bank_move);
        /* ...and a negated one gets a plain move before the negating one. */
        let mut p = base();
        p.push(g(SelOp::Copy, 2, [Some(fixed(Bank::Shared, 5, NEG)), None, None]));
        same_meaning(&p, pos(2), |s| s.plain_first);
    }

    #[test]
    fn immediates_become_constants_or_moves() {
        let mut p = base();
        p.push(g(SelOp::Alu(AluOp::Fmul), 2, [Some(virt(0, NONE)), Some(SelSrc::Imm(1.0f32.to_bits())), None]));
        same_meaning(&p, pos(2), |s| s.imm_const);
        let mut p = base();
        p.push(g(SelOp::Alu(AluOp::Fadd), 2, [Some(virt(0, NONE)), Some(SelSrc::Imm(1.2f32.to_bits())), None]));
        same_meaning(&p, pos(2), |s| s.imm_move);
    }

    #[test]
    fn an_output_the_write_cannot_take_is_moved() {
        let p = base();
        same_meaning(
            &p,
            [
                Some(virt(0, NONE)),
                Some(SelSrc::Imm(1.2f32.to_bits())),
                Some(fixed(Bank::Shared, 2, NONE)),
                Some(virt(1, NEG)),
            ],
            |s| s.output_move,
        );
    }

    /// `sqrt` becomes `frcp(frsq(x))`, both halves prepared (a uniform
    /// operand moved first), and the value within rounding of `sqrt`: the
    /// two roundings of the pair are not the one of `sqrt`, which is why
    /// this compares by units in the last place rather than bits.
    #[test]
    fn a_square_root_is_a_reciprocal_of_a_reciprocal_square_root() {
        let groups = [g(SelOp::Alu(AluOp::Fsqrt), 2, [Some(fixed(Bank::Shared, 4, NONE)), None, None])];
        let outputs = [(IoSlot::Position, pos(2))];
        let sel =
            Selected { groups: &groups, vregs: 16, outputs: &outputs, inputs: &[], discard: None, alpha_to_one: None };
        let mut buf = std::vec![0u8; 64 << 10];
        let arena = Arena::new(&mut buf);
        let (p, st) = prepare_counted(&sel, &arena).expect("prepares");
        assert_eq!(st.sqrt, 1);
        let ops: Vec<SelOp> = p.groups.iter().map(|g| g.op).collect();
        assert_eq!(ops, [SelOp::Copy, SelOp::Alu(AluOp::Frsq), SelOp::Alu(AluOp::Frcp)]);
        let (want, got) = (eval(&groups, &outputs)[0], eval(p.groups, p.outputs)[0]);
        assert!(want.abs_diff(got) <= 2, "sqrt {} vs frcp(frsq) {}", f32::from_bits(want), f32::from_bits(got));
    }

    #[test]
    fn the_constant_table_is_sorted_and_unique() {
        assert!(CONST_REGS.windows(2).all(|w| w[0].0 < w[1].0), "binary_search needs the table sorted");
        assert_eq!(const_reg(0x3f800000), Some(64), "1.0");
        assert_eq!(const_reg(0), Some(0));
        assert_eq!(const_reg(0x3f800001), None);
        assert_eq!(CONST_REGS.len(), 96, "the transcription lost or gained a row of pco_const_imms.c");
    }
}
