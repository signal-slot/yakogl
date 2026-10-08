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
use super::sel::{Bank, MachineOp, Mods, OpType, SelDst, SelGroup, SelOp, SelSrc, VReg};
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
        | SelOp::LoopEnd
        | SelOp::Pred(_) => None,
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
    /// Sums of a product made one `fmad` ([`fusion_pairs`]).
    pub fused: u32,
    /// Clamps to [0, 1] made one `fsat` ([`fusion_pairs`]).
    pub saturated: u32,
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
    prepare_with(sel, arena, &[])
}

/// [`prepare_counted`] with each sum of a product it alone reads made one
/// `fmad` ([`fusion_pairs`]): in the copy prepare makes anyway, so the
/// fusion costs no second copy of the program.
pub fn prepare_fused<'a>(
    sel: &Selected<'a>,
    arena: &'a Arena<'a>,
) -> Result<(Selected<'a>, PrepareStats), PrepareError> {
    let pairs = fusion_pairs(sel, arena)?;
    prepare_with(sel, arena, pairs)
}

/// [`prepare_counted`], with `pairs` applied as it copies: each pair's
/// product is left out and its sum replaced by the pair's `fmad`.
fn prepare_with<'a>(
    sel: &Selected<'a>,
    arena: &'a Arena<'a>,
    pairs: &[(u32, u32, SelGroup)],
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
    p.stats.fused = pairs.iter().filter(|q| q.2.op == SelOp::Alu(AluOp::Ffma)).count() as u32;
    p.stats.saturated = pairs.iter().filter(|q| q.2.op == SelOp::Machine(MachineOp::Fsat)).count() as u32;
    /* The products the pairs consume, sorted for the look-up. */
    let products = arena.alloc_filled(pairs.len(), 0u32)?;
    for (d, q) in products.iter_mut().zip(pairs) {
        *d = q.0;
    }
    products.sort_unstable();
    let mut next_pair = 0usize;

    for (idx, g0) in sel.groups.iter().enumerate() {
        if products.binary_search(&(idx as u32)).is_ok() {
            continue;
        }
        let gr = match pairs.get(next_pair) {
            Some(q) if q.1 as usize == idx => {
                next_pair += 1;
                q.2
            }
            _ => *g0,
        };
        let g = &gr;
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
<<<<<<< HEAD
        for src in &mut g.srcs[..n] {
=======
        for src in g.srcs.iter_mut().take(n) {
>>>>>>> 3985bee2a468281644eb0cf4988c8f06adfcb5e4
            if let Some(SelSrc::Imm(v)) = *src {
                *src = Some(p.imm(v)?);
            }
        }
        canonicalise(op, &mut g.srcs, &mut p.stats);

<<<<<<< HEAD
        for (i, src) in g.srcs[..n].iter_mut().enumerate() {
=======
        for (i, src) in g.srcs.iter_mut().enumerate().take(n) {
>>>>>>> 3985bee2a468281644eb0cf4988c8f06adfcb5e4
            let Some(s) = *src else { continue };
            if !bank_ok(op, i, &s) {
                p.stats.bank_move += 1;
                *src = Some(p.copy(s)?);
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
    /* How many of `kept` were before the open predicated `if` arm: a value
     * an arm computes exists only where the arm runs, so it is offered to
     * nothing past the arm's end (the other arm included); a value from
     * before the `if` is offered inside either arm. */
    let mut before_arm: Option<usize> = None;
    for g in sel.groups {
        if let SelOp::Pred(m) = g.op {
            match m {
                super::sel::PredArm::Then => before_arm = Some(kept.len()),
                super::sel::PredArm::Else | super::sel::PredArm::End => {
                    let n = before_arm.ok_or(PrepareError::Unsupported("an `if` arm marker out of order"))?;
                    while kept.len() > n {
                        kept.pop();
                    }
                    if m == super::sel::PredArm::End {
                        before_arm = None;
                    }
                }
            }
            out.push(arena, *g)?;
            continue;
        }
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

/// `sel` with each `fadd` of a product it alone reads made one `fmad`:
/// `fadd(fmul(a, b), c)` is `fmad(a, b, c)`. GLSL ES 1.00 lets an
/// implementation evaluate `a * b + c` either way (no `precise` in ES 1.00;
/// 4.6.1's `invariant` asks the same computation of every program, which a
/// pass that always fuses the same shape gives); Mesa fuses the same shape
/// (nir_opt_algebraic's `fadd(fmul(a, b), c) -> ffma`), and the offline
/// table's programs carry the fused form where ours carried the pair.
///
/// Fused only where the result is the same value the pair would compute
/// from the same inputs: the product is a virtual value defined once and
/// read by this one `fadd` alone (no output, discard or alpha-to-one reads
/// it), carries no `abs` or `floor` at the `fadd`, both are F32 operations,
/// no loop boundary or machine operation (a loop test among them) sits
/// between them, and
/// each of the product's own sources is a value nothing rewrites in between
/// (a virtual value defined once, or a shared, special or vertex-input
/// register). A `neg` on the product moves onto its first factor
/// (`-(a * b) == (-a) * b`, exactly). [`prepare_counted`] then legalises
/// the modifier sets as for any `fmad`. A clamp to [0, 1] becomes one
/// `fsat` the same way ([`fusion_pairs`]). Returns the program and how many
/// pairs were applied, of both kinds.
pub fn fuse_mul_add<'a>(sel: &Selected<'a>, arena: &'a Arena<'a>) -> Result<(Selected<'a>, u32), PrepareError> {
    let pairs = fusion_pairs(sel, arena)?;
    if pairs.is_empty() {
        return Ok((Selected { ..*sel }, 0));
    }
    let gone = |i: usize| pairs.iter().any(|p| p.0 as usize == i);
    let out = arena.alloc_filled(sel.groups.len() - pairs.len(), sel.groups[0])?;
    let (mut n, mut next) = (0usize, 0usize);
    for (i, g) in sel.groups.iter().enumerate() {
        if gone(i) {
            continue;
        }
        out[n] = match pairs.get(next) {
            Some(p) if p.1 as usize == i => {
                next += 1;
                p.2
            }
            _ => *g,
        };
        n += 1;
    }
    Ok((Selected { groups: out, ..*sel }, pairs.len() as u32))
}

/// The bounds of a clamp [`fusion_pairs`] makes one `fsat`: +0.0 and 1.0.
const SAT_LO: u32 = 0;
const SAT_HI: u32 = 0x3f80_0000;

/// The pairs [`fuse_mul_add`] fuses, as (inner group's index, outer group's
/// index, the group that replaces the outer), in the outer groups' program
/// order: what [`prepare_fused`] applies as it copies. A product and the
/// sum that alone reads it become one `fmad`; the two bounds of a clamp to
/// [0, 1] become one `fsat` (`MachineOp::Fsat`).
pub fn fusion_pairs<'a>(sel: &Selected<'a>, arena: &'a Arena<'a>) -> Result<&'a [(u32, u32, SelGroup)], PrepareError> {
    /* Per value, one byte: how often it is defined (bits 0..1) and read
     * (bits 2..3), each saturating at 3: "once" is all either question asks.
     * A byte, not two counts: a large program has tens of thousands of
     * values, and what this pass allocates comes out of the scratch the
     * rest of the compile has left. */
    let n = sel.vregs as usize;
    let seen = arena.alloc_filled(n, 0u8)?;
    let bump = |v: VReg, shift: u8, t: &mut [u8]| {
        if let Some(c) = t.get_mut(v.0 as usize) {
            if (*c >> shift) & 3 < 3 {
                *c += 1 << shift;
            }
        }
    };
    let (def, read) = (0u8, 2u8);
    for g in sel.groups {
        if let Some(SelDst::Virtual(v)) = g.dst {
            bump(v, def, seen);
            if matches!(g.op, SelOp::Sample { .. }) {
                for k in 1..4 {
                    bump(VReg(v.0 + k), def, seen);
                }
            }
        }
        for s in g.srcs.iter().flatten() {
            if let SelSrc::Virtual { v, .. } = s {
                bump(*v, read, seen);
            }
        }
    }
    let side = sel.outputs.iter().flat_map(|(_, c)| c.iter().flatten()).chain(sel.discard.iter());
    for s in side.chain(sel.alpha_to_one.iter().flat_map(|(x, f)| [x, f])) {
        if let SelSrc::Virtual { v, .. } = s {
            bump(*v, read, seen);
        }
    }
    for (_, comps) in sel.inputs {
        for v in comps.iter().flatten() {
            bump(*v, def, seen);
        }
    }
    /* The fixed temporaries a group writes. A fragment shader's iterated
     * inputs reach this pass in the temporaries the iteration pins them to
     * (chain.rs rewrites them before), which no group writes: those hold
     * one value all program long, as a uniform does, and a product of one
     * (fill_color's `ftcoord.x * 2.0 - 1.0`) fuses as pco's does. */
    let mut written = [0u64; 4];
    for g in sel.groups {
        if let Some(SelDst::Fixed { bank: Bank::Temp, reg }) = g.dst {
            /* a group that writes more than its register: every one past it */
            let width = if matches!(g.op, SelOp::Sample { .. }) { 4 } else { 1 };
            for r in reg..reg.saturating_add(width) {
                if let Some(w) = written.get_mut(usize::from(r / 64)) {
                    *w |= 1 << (r % 64);
                }
            }
        }
    }
    let unwritten = |r: u16| written.get(usize::from(r / 64)).is_some_and(|w| w & (1 << (r % 64)) == 0);
    let seen: &[u8] = seen;
    let once = |v: VReg, shift: u8| seen.get(v.0 as usize).is_some_and(|c| (c >> shift) & 3 == 1);
    let steady = |s: &SelSrc| match s {
        SelSrc::Virtual { v, .. } => once(*v, def),
        SelSrc::Fixed { bank: Bank::Temp, reg, .. } => unwritten(*reg),
        SelSrc::Fixed { bank, .. } => matches!(bank, Bank::Shared | Bank::Special | Bank::VtxIn),
        SelSrc::Imm(_) => true,
    };
    /* a predicated `if` arm's edges too: a pair across one would compute
     * in one place what the arm computes only where it runs */
    let barrier = |g: &SelGroup| matches!(g.op, SelOp::LoopBegin | SelOp::LoopEnd | SelOp::Machine(_) | SelOp::Pred(_));
    /* `fmax(x, 0.0)` or `fmin(x, 1.0)`, either operand order: the bound
     * (+0.0 or 1.0 exactly, as bits) and `x`. */
    let bounded = |g: &SelGroup| -> Option<(u32, SelSrc)> {
        let want = match g.op {
            SelOp::Alu(AluOp::Fmax) => SAT_LO,
            SelOp::Alu(AluOp::Fmin) => SAT_HI,
            _ => return None,
        };
        match (g.srcs[0]?, g.srcs[1]?) {
            (SelSrc::Imm(k), x) | (x, SelSrc::Imm(k)) if k == want && !matches!(x, SelSrc::Imm(_)) => Some((k, x)),
            _ => None,
        }
    };
    /* The candidate inner values, (value, where it is defined), sorted by
     * value for the look-up below: only those, not a slot per value. A
     * product for a sum's `fmad`; a bound of a clamp to [0, 1] for its
     * `fsat`. */
    let candidate = |g: &SelGroup| match (g.op, g.ty, g.dst) {
        (SelOp::Alu(AluOp::Fmul), OpType::F32, Some(SelDst::Virtual(v))) => {
            (once(v, def) && once(v, read) && g.srcs[..2].iter().flatten().all(steady)).then_some(v)
        }
        (SelOp::Alu(AluOp::Fmax | AluOp::Fmin), OpType::F32, Some(SelDst::Virtual(v))) => {
            (once(v, def) && once(v, read) && bounded(g).is_some_and(|(_, x)| steady(&x))).then_some(v)
        }
        _ => None,
    };
    let n_cand = sel.groups.iter().filter(|g| candidate(g).is_some()).count();
    if n_cand == 0 {
        return Ok(&[]);
    }
    let def_at = arena.alloc_filled(n_cand, (0u32, 0u32))?;
    let mut c = 0;
    for (i, g) in sel.groups.iter().enumerate() {
        if let Some(v) = candidate(g) {
            def_at[c] = (v.0, i as u32);
            c += 1;
        }
    }
    def_at.sort_unstable_by_key(|e| e.0);
    let def_at: &[(u32, u32)] = def_at;
    let product_at = |v: VReg| def_at.binary_search_by_key(&v.0, |e| e.0).ok().map(|k| def_at[k].1);
    /* The pairs to fuse, (product, sum) indices in program order of the
     * sum; decided before anything is copied, so a program with none is
     * handed back as it is, and one with some is copied once, at its final
     * size (the scratch a big program has left is what this spends). */
    let mut pairs: AVec<'a, (u32, u32, SelGroup)> = AVec::new();
    let mut last_barrier: Option<usize> = None;
    for (j, g) in sel.groups.iter().enumerate() {
        if barrier(g) {
            last_barrier = Some(j);
        }
        /* A clamp to [0, 1]: `fmin(fmax(x, 0.0), 1.0)` or `fmax(fmin(x,
         * 1.0), 0.0)`, the inner bound read by the outer alone and with no
         * modifier, is one `fsat` of x (nir_opt_algebraic forms `fsat`
         * from both). The inner group goes; the outer becomes the `fsat`.
         * An inner group that is itself the outer of an earlier pair stays
         * (each group takes part in one pair). */
        if let Some((k, inner)) = bounded(g) {
            if g.ty != OpType::F32 {
                continue;
            }
            let SelSrc::Virtual { v, mods } = inner else { continue };
            if mods != Mods::default() {
                continue;
            }
            let Some(i) = product_at(v) else { continue };
            if (i as usize) >= j || last_barrier.is_some_and(|b| b > i as usize) {
                continue;
            }
            let m = sel.groups[i as usize];
            let Some((ki, x)) = bounded(&m) else { continue };
            /* the other bound: max inside min, or min inside max */
            if ki == k || pairs.as_slice().iter().any(|p| p.1 == i) {
                continue;
            }
            let fsat = SelGroup {
                op: SelOp::Machine(MachineOp::Fsat),
                ty: OpType::F32,
                dst: g.dst,
                srcs: [Some(x), None, None],
            };
            pairs.push(arena, (i, j as u32, fsat))?;
            continue;
        }
        if !(g.op == SelOp::Alu(AluOp::Fadd) && g.ty == OpType::F32) {
            continue;
        }
        for k in 0..2 {
            let Some(SelSrc::Virtual { v, mods }) = g.srcs[k] else { continue };
            if mods.abs || mods.flr {
                continue;
            }
            let Some(i) = product_at(v) else { continue };
            if (i as usize) >= j || last_barrier.is_some_and(|b| b > i as usize) {
                continue;
            }
            let m = sel.groups[i as usize];
            /* the look-up holds the clamps' bounds too */
            if m.op != SelOp::Alu(AluOp::Fmul) {
                continue;
            }
            let (Some(a), Some(b)) = (m.srcs[0], m.srcs[1]) else { continue };
            /* -(a * b) is (-a) * b. An immediate carries no modifier
             * (`with_mods` leaves it as it is), so its sign bit is flipped
             * instead, which negates a float exactly; dropping the
             * negation there made `-(2.0 * q)` into `2.0 * q`. */
            let a = match a {
                SelSrc::Imm(v) if mods.neg => SelSrc::Imm(v ^ 0x8000_0000),
                SelSrc::Imm(v) => SelSrc::Imm(v),
                _ => {
                    let ma = mods_of(&Some(a));
                    with_mods(a, Mods { neg: ma.neg != mods.neg, ..ma })
                }
            };
            let fma = SelGroup {
                op: SelOp::Alu(AluOp::Ffma),
                ty: OpType::F32,
                dst: g.dst,
                srcs: [Some(a), Some(b), g.srcs[1 - k]],
            };
            pairs.push(arena, (i, j as u32, fma))?;
            break;
        }
    }
    Ok(pairs.into_slice())
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
            /* an arm's edge: this evaluator runs both arms */
            if matches!(gr.op, SelOp::Pred(_)) {
                continue;
            }
            let a: Vec<f32> = gr.srcs.iter().take(gr.op.arity()).map(|s| read(s.as_ref().unwrap(), &v)).collect();
            let r = match gr.op {
                SelOp::Copy => a[0],
                SelOp::Alu(AluOp::Fadd) => a[0] + a[1],
                SelOp::Alu(AluOp::Fmul) => a[0] * a[1],
                SelOp::Alu(AluOp::Ffma) => a[0] * a[1] + a[2],
                SelOp::Alu(AluOp::Frsq) => 1.0 / a[0].sqrt(),
                SelOp::Alu(AluOp::Frcp) => 1.0 / a[0],
                SelOp::Alu(AluOp::Fsqrt) => a[0].sqrt(),
                SelOp::Alu(AluOp::Fmin) => a[0].min(a[1]),
                SelOp::Alu(AluOp::Fmax) => a[0].max(a[1]),
                SelOp::Machine(MachineOp::Fsat) => a[0].clamp(0.0, 1.0),
                SelOp::Alu(o) => panic!("{o:?} is not in this evaluator"),
                o @ (SelOp::Sample { .. }
                | SelOp::WaitData
                | SelOp::TexWord { .. }
                | SelOp::SampleZ
                | SelOp::SampleGrad
                | SelOp::PlaneCoeff { .. }
                | SelOp::Machine(_)
                | SelOp::LoopBegin
                | SelOp::LoopEnd
                | SelOp::Pred(_)) => {
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

    /// `fuse_mul_add` on `groups` with output `out`: the program, how many
    /// pairs fused, and whether the result means what the input meant.
    fn fused(groups: &[SelGroup], out: [Option<SelSrc>; 4]) -> (Vec<SelGroup>, u32, bool) {
        let outputs = [(IoSlot::Position, out)];
        let sel = Selected { groups, vregs: 16, outputs: &outputs, inputs: &[], discard: None, alpha_to_one: None };
        let mut buf = std::vec![0u8; 64 << 10];
        let arena = Arena::new(&mut buf);
        let (f, n) = fuse_mul_add(&sel, &arena).expect("fuses");
        /* the evaluator runs straight-line programs only */
        let straight = groups.iter().all(|g| !matches!(g.op, SelOp::LoopBegin | SelOp::LoopEnd));
        let same = straight && eval(f.groups, f.outputs) == eval(groups, &outputs);
        (f.groups.to_vec(), n, same)
    }

    /// A product of a fixed temporary fuses with its sum when no group
    /// writes that temporary (a fragment shader's pinned iterated input:
    /// fill_color's `ftcoord.x * 2.0 - 1.0`, gl_test 191), and stays a
    /// product when one does: the fmad would read the temporary where the
    /// sum is, after the write. (No corpus program reads a written fixed
    /// temporary in a product -- sample results are moved to values first
    /// -- so this case is the only one that sees the second side.)
    #[test]
    fn a_product_of_a_fixed_temporary_fuses_only_when_nothing_writes_it() {
        let t = |r: u16| fixed(Bank::Temp, r, NONE);
        let k = Some(SelSrc::Imm(2.0f32.to_bits()));
        let mul = g(SelOp::Alu(AluOp::Fmul), 1, [Some(t(4)), k, None]);
        let add = g(SelOp::Alu(AluOp::Fadd), 2, [Some(virt(1, NONE)), Some(fixed(Bank::Shared, 0, NONE)), None]);
        let outputs = [(IoSlot::Position, [Some(virt(2, NONE)), None, None, None])];
        /* the pairs `prepare_fused` would apply (the evaluator `fused` uses
         * models no fixed temporary) */
        let pairs = |groups: &[SelGroup]| -> Vec<SelOp> {
            let sel = Selected { groups, vregs: 16, outputs: &outputs, inputs: &[], discard: None, alpha_to_one: None };
            let mut buf = std::vec![0u8; 64 << 10];
            let arena = Arena::new(&mut buf);
            fusion_pairs(&sel, &arena).expect("pairs").iter().map(|q| q.2.op).collect()
        };
        assert_eq!(pairs(&[mul, add]), [SelOp::Alu(AluOp::Ffma)], "t4 is written by nothing");
        /* t4 written between the product and the sum: fusing would read
         * the new t4 */
        let write = |op: SelOp, reg: u16| SelGroup {
            op,
            ty: OpType::F32,
            dst: Some(SelDst::Fixed { bank: Bank::Temp, reg }),
            srcs: [Some(fixed(Bank::Shared, 1, NONE)), None, None],
        };
        assert_eq!(pairs(&[mul, write(SelOp::Copy, 4), add]), [], "t4 is written");
        /* a sample writes four from its destination: t4 among t2..t5 */
        let sample = SelOp::Sample {
            binding: 0,
            proj: false,
            lod: super::super::sel::SmpLod::Implicit,
            fetch: false,
            offset: None,
            cube: false,
        };
        assert_eq!(pairs(&[mul, write(sample, 2), add]), [], "t4 is written by the sample");
        assert_eq!(pairs(&[mul, write(sample, 5), add]), [SelOp::Alu(AluOp::Ffma)], "t5..t8: t4 is not");
    }

    fn marker(m: super::super::sel::PredArm) -> SelGroup {
        SelGroup { op: SelOp::Pred(m), ty: OpType::U32, dst: None, srcs: [None; 3] }
    }

    /// A value a predicated `if` arm computes exists only where the arm
    /// runs, so `share_common` offers it to nothing past the arm's end --
    /// a recomputation after the `if` keeps its own group -- while a value
    /// from before the `if` is shared inside an arm.
    #[test]
    fn an_arm_value_is_not_shared_past_the_arm() {
        use super::super::sel::PredArm::{Else, End, Then};
        let mul = |d: u32| {
            g(
                SelOp::Alu(AluOp::Fmul),
                d,
                [Some(fixed(Bank::Shared, 0, NONE)), Some(fixed(Bank::Shared, 1, NONE)), None],
            )
        };
        let add = |d: u32, a: u32| {
            g(SelOp::Alu(AluOp::Fadd), d, [Some(virt(a, NONE)), Some(fixed(Bank::Shared, 2, NONE)), None])
        };
        let outputs = [(IoSlot::Position, [Some(virt(4, NONE)), Some(virt(5, NONE)), Some(virt(6, NONE)), None])];
        let run = |groups: &[SelGroup]| -> Vec<SelGroup> {
            let sel = Selected { groups, vregs: 16, outputs: &outputs, inputs: &[], discard: None, alpha_to_one: None };
            let mut buf = std::vec![0u8; 64 << 10];
            let arena = Arena::new(&mut buf);
            share_common(&sel, &arena).expect("shares").groups.to_vec()
        };
        /* computed in the then arm (1), again after the end (2): kept */
        let after = run(&[marker(Then), mul(1), add(4, 1), marker(Else), marker(End), mul(2), add(5, 2), add(6, 2)]);
        assert_eq!(after.iter().filter(|x| x.op == SelOp::Alu(AluOp::Fmul)).count(), 2, "{after:?}");
        /* in the then arm and again in the else arm: kept too */
        let other = run(&[marker(Then), mul(1), add(4, 1), marker(Else), mul(2), add(5, 2), marker(End), add(6, 2)]);
        assert_eq!(other.iter().filter(|x| x.op == SelOp::Alu(AluOp::Fmul)).count(), 2, "{other:?}");
        /* before the `if` (0) and in an arm (1): the arm reads 0's */
        let before = run(&[mul(0), add(6, 0), marker(Then), mul(1), add(4, 1), marker(Else), marker(End), add(5, 0)]);
        assert_eq!(before.iter().filter(|x| x.op == SelOp::Alu(AluOp::Fmul)).count(), 1, "{before:?}");
    }

    /// A product in an arm and a sum after the `if` are not one `fmad`:
    /// the pair would compute where the arm does not run.
    #[test]
    fn no_fmad_across_an_arm_edge() {
        use super::super::sel::PredArm::{Else, End, Then};
        let mul = g(
            SelOp::Alu(AluOp::Fmul),
            1,
            [Some(fixed(Bank::VtxIn, 0, NONE)), Some(fixed(Bank::Shared, 1, NONE)), None],
        );
        let add = g(SelOp::Alu(AluOp::Fadd), 2, [Some(virt(1, NONE)), Some(fixed(Bank::Shared, 2, NONE)), None]);
        let (_, n, _) =
            fused(&[marker(Then), mul, marker(Else), marker(End), add], [Some(virt(2, NONE)), None, None, None]);
        assert_eq!(n, 0, "across the end");
        let (_, n, _) =
            fused(&[marker(Then), mul, add, marker(Else), marker(End)], [Some(virt(2, NONE)), None, None, None]);
        assert_eq!(n, 1, "inside one arm it fuses");
    }

    /// `clamp(x, 0.0, 1.0)` in either nesting (`fmin(fmax(x, 0), 1)`,
    /// `fmax(fmin(x, 1), 0)`) and either operand order is one `fsat(x)`,
    /// computing the same value for an x below 0 (sh1, -2.25), inside
    /// (sh6, 0.3) and above 1 (sh0, 1.5): what pco makes of NIR's `fsat`,
    /// and what the table's fill_color.frag carries (`fadd sat=1`).
    #[test]
    fn a_clamp_to_0_1_is_one_fsat_in_either_nesting_and_order() {
        let (zero, one) = (SelSrc::Imm(0), SelSrc::Imm(1.0f32.to_bits()));
        let (max, min) = (SelOp::Alu(AluOp::Fmax), SelOp::Alu(AluOp::Fmin));
        for reg in [1u16, 6, 0] {
            let x = fixed(Bank::Shared, reg, NONE);
            let forms = [
                [g(max, 1, [Some(x), Some(zero), None]), g(min, 2, [Some(virt(1, NONE)), Some(one), None])],
                [g(max, 1, [Some(zero), Some(x), None]), g(min, 2, [Some(one), Some(virt(1, NONE)), None])],
                [g(min, 1, [Some(x), Some(one), None]), g(max, 2, [Some(virt(1, NONE)), Some(zero), None])],
                [g(min, 1, [Some(one), Some(x), None]), g(max, 2, [Some(zero), Some(virt(1, NONE)), None])],
            ];
            for (k, groups) in forms.iter().enumerate() {
                let (f, n, same) = fused(groups, [Some(virt(2, NONE)), None, None, None]);
                assert_eq!((n, f.len()), (1, 1), "sh{reg} form {k}: {f:?}");
                assert_eq!(f[0].op, SelOp::Machine(MachineOp::Fsat), "sh{reg} form {k}");
                assert_eq!(f[0].srcs, [Some(x), None, None], "sh{reg} form {k}");
                assert!(same, "sh{reg} form {k}: the fsat computes what the clamp did");
            }
        }
    }

    /// A clamp of a clamp is two: the second clamp's inner bound is the
    /// first's outer, already replaced, so it pairs with the next bound
    /// instead, and each group takes part in one pair.
    #[test]
    fn a_clamp_of_a_clamp_is_two_fsats() {
        let (zero, one) = (SelSrc::Imm(0), SelSrc::Imm(1.0f32.to_bits()));
        let (max, min) = (SelOp::Alu(AluOp::Fmax), SelOp::Alu(AluOp::Fmin));
        let groups = [
            g(max, 1, [Some(fixed(Bank::Shared, 0, NONE)), Some(zero), None]),
            g(min, 2, [Some(virt(1, NONE)), Some(one), None]),
            g(max, 3, [Some(virt(2, NONE)), Some(zero), None]),
            g(min, 4, [Some(virt(3, NONE)), Some(one), None]),
        ];
        let (f, n, same) = fused(&groups, [Some(virt(4, NONE)), None, None, None]);
        assert_eq!((n, f.len()), (2, 2), "{f:?}");
        assert_eq!(f[1].srcs[0], Some(virt(2, NONE)), "the second reads the first: {f:?}");
        assert!(same, "{f:?}");
    }

    /// Not a clamp to [0, 1], each for its own reason, and so left as it
    /// is: the inner bound read twice, read negated, the same bound twice,
    /// another upper bound (0.5), the bound -0.0, a loop boundary between,
    /// and x defined twice.
    #[test]
    fn what_is_not_a_clamp_to_0_1_is_left_as_it_is() {
        let (zero, one) = (SelSrc::Imm(0), SelSrc::Imm(1.0f32.to_bits()));
        let (max, min) = (SelOp::Alu(AluOp::Fmax), SelOp::Alu(AluOp::Fmin));
        let x = fixed(Bank::Shared, 6, NONE);
        let inner = g(max, 1, [Some(x), Some(zero), None]);
        let outer = |m: Mods, hi: SelSrc| g(min, 2, [Some(virt(1, m)), Some(hi), None]);
        let out2 = [Some(virt(2, NONE)), None, None, None];
        let (_, n, _) = fused(&[inner, outer(NONE, one)], [Some(virt(2, NONE)), Some(virt(1, NONE)), None, None]);
        assert_eq!(n, 0, "the inner bound read twice");
        let (_, n, _) = fused(&[inner, outer(NEG, one)], out2);
        assert_eq!(n, 0, "the inner bound read negated");
        let (_, n, _) = fused(&[inner, g(max, 2, [Some(virt(1, NONE)), Some(zero), None])], out2);
        assert_eq!(n, 0, "the same bound twice");
        let (_, n, _) = fused(&[inner, outer(NONE, SelSrc::Imm(0.5f32.to_bits()))], out2);
        assert_eq!(n, 0, "an upper bound of 0.5");
        let neg_zero = g(max, 1, [Some(x), Some(SelSrc::Imm(0x8000_0000)), None]);
        let (_, n, _) = fused(&[neg_zero, outer(NONE, one)], out2);
        assert_eq!(n, 0, "a lower bound of -0.0");
        let top = SelGroup { op: SelOp::LoopBegin, ty: OpType::F32, dst: None, srcs: [None; 3] };
        let (_, n, _) = fused(&[inner, top, outer(NONE, one)], out2);
        assert_eq!(n, 0, "a loop boundary between");
        let def = |v: u32| g(SelOp::Copy, v, [Some(fixed(Bank::Shared, 3, NONE)), None, None]);
        let inner5 = g(max, 1, [Some(virt(5, NONE)), Some(zero), None]);
        let (_, n, _) = fused(&[def(5), inner5, def(5), outer(NONE, one)], out2);
        assert_eq!(n, 0, "x defined twice");
    }

    /// `x = vtx0 * sh1; y = sh2 - x` is one `fmad(-vtx0, sh1, sh2)`: the
    /// product's negation moves onto its first factor, the `fmul` goes, and
    /// the program computes the same bits.
    #[test]
    fn a_sum_of_a_product_it_alone_reads_is_one_fmad() {
        let groups = [
            g(
                SelOp::Alu(AluOp::Fmul),
                1,
                [Some(fixed(Bank::VtxIn, 0, NONE)), Some(fixed(Bank::Shared, 1, NONE)), None],
            ),
            g(SelOp::Alu(AluOp::Fadd), 2, [Some(virt(1, NEG)), Some(fixed(Bank::Shared, 2, NONE)), None]),
        ];
        let (f, n, same) = fused(&groups, [Some(virt(2, NONE)), None, None, None]);
        assert_eq!((n, f.len()), (1, 1), "{f:?}");
        assert_eq!(f[0].op, SelOp::Alu(AluOp::Ffma));
        assert_eq!(
            f[0].srcs,
            [Some(fixed(Bank::VtxIn, 0, NEG)), Some(fixed(Bank::Shared, 1, NONE)), Some(fixed(Bank::Shared, 2, NONE))]
        );
        assert!(same, "the fused program computes what the pair did");
    }

    /// A negated product whose first factor is an immediate: the sign goes
    /// into the immediate's bits (an immediate carries no modifier).
    /// `y = sh2 - 2.0 * vtx0` is `fmad(-2.0, vtx0, sh2)`, the same bits. The
    /// shape that miscompiled shapestroke.vert's `c` (the negation dropped:
    /// usc_limits' execution check, 12 runs disagreed).
    #[test]
    fn a_negated_product_of_an_immediate_negates_the_immediate() {
        let two = 2.0f32.to_bits();
        let groups = [
            g(SelOp::Alu(AluOp::Fmul), 1, [Some(SelSrc::Imm(two)), Some(fixed(Bank::VtxIn, 0, NONE)), None]),
            g(SelOp::Alu(AluOp::Fadd), 2, [Some(virt(1, NEG)), Some(fixed(Bank::Shared, 2, NONE)), None]),
        ];
        let (f, n, same) = fused(&groups, [Some(virt(2, NONE)), None, None, None]);
        assert_eq!((n, f.len()), (1, 1), "{f:?}");
        assert_eq!(f[0].srcs[0], Some(SelSrc::Imm((-2.0f32).to_bits())), "{f:?}");
        assert!(same, "the fused program computes what the pair did");
    }

    /// Not fused, each for its own reason: the product read twice, read
    /// with `abs`, a loop boundary between the two, a factor defined twice.
    #[test]
    fn a_product_read_twice_or_through_abs_or_a_loop_boundary_is_not_fused() {
        let mul = g(
            SelOp::Alu(AluOp::Fmul),
            1,
            [Some(fixed(Bank::VtxIn, 0, NONE)), Some(fixed(Bank::Shared, 1, NONE)), None],
        );
        let add = |m: Mods| g(SelOp::Alu(AluOp::Fadd), 2, [Some(virt(1, m)), Some(fixed(Bank::Shared, 2, NONE)), None]);
        let out2 = [Some(virt(2, NONE)), None, None, None];
        /* read twice: by the add and by the output */
        let (_, n, _) = fused(&[mul, add(NONE)], [Some(virt(2, NONE)), Some(virt(1, NONE)), None, None]);
        assert_eq!(n, 0, "read twice");
        /* through abs */
        let abs = Mods { abs: true, ..NONE };
        let (_, n, _) = fused(&[mul, add(abs)], out2);
        assert_eq!(n, 0, "abs at the add");
        /* a loop boundary between them */
        let top = SelGroup { op: SelOp::LoopBegin, ty: OpType::F32, dst: None, srcs: [None; 3] };
        let (_, n, _) = fused(&[mul, top, add(NONE)], out2);
        assert_eq!(n, 0, "a loop boundary between");
        /* a factor defined twice (a loop-carried value) */
        let def = |v: u32| g(SelOp::Copy, v, [Some(fixed(Bank::Shared, 3, NONE)), None, None]);
        let mul_v = g(SelOp::Alu(AluOp::Fmul), 1, [Some(virt(5, NONE)), Some(fixed(Bank::Shared, 1, NONE)), None]);
        let (_, n, _) = fused(&[def(5), def(5), mul_v, add(NONE)], out2);
        assert_eq!(n, 0, "a factor defined twice");
        /* the control: the same program with the factor defined once fuses */
        let (_, n, same) = fused(&[def(5), mul_v, add(NONE)], out2);
        assert_eq!((n, same), (1, true), "defined once");
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
