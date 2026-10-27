// SPDX-License-Identifier: Apache-2.0 AND MIT
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
// SPDX-FileCopyrightText: 2023 Igalia S.L.
// SPDX-FileCopyrightText: 2012-2019 Etnaviv Project
// SPDX-FileCopyrightText: 2019 Zodiac Inflight Innovations
// Derived from Mesa 26.2.1 src/etnaviv/isa/etnaviv.xml and
// src/gallium/drivers/etnaviv/etnaviv_compiler_nir_emit.c (MIT).
// The upstream part stays under its licence (LICENSES/MIT.txt); our
// changes are Apache-2.0. See NOTICE.
//! Builtin lowering for the GC7000: which IR operations the machine has, and
//! the instruction sequence for the ones it does not.
//!
//! The code generator asks [`native`] first and calls [`lower`] only when the
//! answer is `None`. Both live here on purpose: a list of what the machine has
//! natively, kept anywhere other than beside the expansions, is two records of
//! one fact. The day an operation moves between them a shader silently gets a
//! worse or a wrong sequence, and nothing catches it, because both halves
//! still compile and the picture is merely different.
//!
//! What this file may claim is bounded by evidence. An entry is written only
//! when the operation's meaning is settled by the ISA table
//! (`docs/online-shaderc/isa/gc7000-isa.json`, transcribed from Mesa's
//! isaspec) or by the offline corpus. Everything else returns
//! [`LowerError::Unsupported`], which stops the compile loudly, rather than a
//! plausible sequence that would draw the wrong picture. An unimplemented
//! builtin must not fall through to something that looks like an answer.
//!
//! Two facts about this ISA shape most of the entries below.
//!
//! **Negate and absolute are source modifiers, not instructions.**
//! `#instruction-src` carries `SRC_NEG` at bit 8 (displayed `-`) and `SRC_ABS`
//! at bit 9 (displayed `|`), per source. So `Fneg` and `Fabs` cost no
//! instruction of their own when their consumer can take the modifier, and
//! cost exactly one `mov` when the IR asks for the value itself.
//!
//! **The operand type is not part of the opcode.** `TYPE` is three bits split
//! across the instruction -- `TYPE_BIT2` at bit 53 and `TYPE_BIT01` at bits
//! 95:94, combined as `{TYPE_BIT2} << 2 | {TYPE_BIT01}`. Several operations
//! exist only for some types, so [`native`] takes the type as well as the
//! opcode; asking with the opcode alone would be asking a different question.
//!

//! # What is left to write, and why it is not a list of GLSL builtins
//!
//! This file is below the IR, not beside it. `docs/online-shaderc/ir.md` and
//! `ir.rs` fix the boundary: "The front end lowers the whole ES 1.00 builtin
//! library to these. The back end implements these and only these." So
//! `normalize` never arrives here as `normalize`; it arrives as `Fdot3`,
//! `Frsq` and `Fmul`, all three of which the machine already has. Measuring
//! which GLSL builtins the shipped shaders call answers a question about the
//! FRONT end's work order, not this file's -- that measurement is real and
//! reproducible (`tools/gc7000-isa/builtin_usage.py`: 307 sources, 113 of them
//! calling no builtin at all, `normalize` completing 38 on its own), and it
//! belongs to whoever writes the expansions.
//!
//! This file's work list is therefore closed and small: the 52 `AluOp`s, minus
//! those the machine has natively, minus those already written. As of this
//! commit that leaves the comparisons, the bool conversions, the reductions,
//! `Ineg`, `Ftrunc` and `Idiv` -- and all but `Idiv` are written below.
//!
//! # Where each entry's authority comes from
//!
//! Two independent sources, and they agree wherever both speak:
//!
//! 1. **Our own corpus.** The 309 shipped Vivante blobs decode to 13,854
//!    instructions (`tools/gc7000-isa/decode.py --blobs`), and what they do is
//!    what this silicon has been fed successfully.
//! 2. **Mesa's etnaviv NIR table** (`etnaviv_compiler_nir_emit.c`, MIT), read
//!    and transcribed here, never vendored.
//!
//! The agreement is worth stating once because it is what makes the entries
//! below evidence rather than preference. Mesa maps `fmin` to `SELECT.GT`,
//! `fmax` to `SELECT.LT` and `b32csel` to `SELECT.NZ`; this file reached the
//! same three from the corpus before that table was read, by the operand
//! partition recorded on [`lower`]'s `Fmin` arm.
//!
//! **The four conditions.** Mesa emits `cmp` with `EQ`, `NE`, `GE` and `LT`
//! and has no `LE` or `GT` row at all; our corpus has `.lt` 192, `.ge` 122,
//! `.eq` 77, `.ne` 47 and zero of the other two. So `a <= b` is lowered as
//! `b >= a` -- an operand swap, not a different condition. Both sources say
//! the same thing, and a lowering that emitted `.le` would be the first
//! program on this machine to try it.

use super::super::ir::{AluOp, Es100Tex};
use super::alloc::{Cond, OpType, SelDst, SelInstr, SelSrc, Tex, VReg};
use super::build::{op_id, ImmError, OpId, Src};

/// Why a lowering could not be produced.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LowerError {
    /// No sequence is written for this operation. Deliberately not a
    /// fallback: an operation nobody has lowered must stop the compile, not
    /// silently become something close.
    Unsupported(AluOp),
    /// The operation was given the wrong number of sources. [`AluOp::arity`]
    /// says how many it reads; a mismatch would otherwise leave a slot unset
    /// and read whatever a previous instruction left there.
    Arity { op: AluOp, want: usize, got: usize },
    /// A value this lowering needs as an instruction immediate cannot be one.
    /// Every immediate below is 0, 1, -1 or 1.0f, so this is a broken build
    /// rather than a shader's doing -- but it is returned, not asserted,
    /// because a compiler that panics in a driver takes the process with it.
    Immediate(ImmError),
    /// The ISA table has no operation of this name. Same reasoning: the names
    /// below are fixed strings, so this means the table changed under us.
    NoSuchOp(&'static str),
    /// A sampler index beyond the five bits `TEX_ID` has. Refused here as well
    /// as in the allocator so that the error names the binding rather than the
    /// encoding.
    Sampler(u8),
    /// A texture look-up given a bias or level it does not take, or denied one
    /// it needs. The two forms are different instructions, so this is a wrong
    /// instruction rather than a missing operand.
    TexExtra(Es100Tex),
}

/// The single GC7000 operation that implements `op` at type `ty`, if there is
/// one.
///
/// `None` means "ask [`lower`]", not "impossible".
///
/// Notably absent, and why:
///
/// - `Fneg`, `Fabs` -- source modifiers, not operations; see [`lower`].
/// - `Fmin`, `Fmax`, `Bcsel` -- the machine has no `min`/`max`; all three are
///   ONE `select` carrying a `COND` field. One instruction, but not a bare
///   name, so they belong to [`lower`], which can set the field.
/// - the comparisons and the bool conversions -- also one instruction each,
///   and also not a bare name: a comparison is `cmp` plus a condition plus the
///   immediate that says what "true" is.
pub fn native(op: AluOp, ty: OpType) -> Option<&'static str> {
    use AluOp::*;
    let _ = ty; /* every mapping below holds for all types it is valid at; an
                 * entry that does not must branch on `ty` here. */
    Some(match op {
        /* float arithmetic */
        Fadd => "add",
        Fmul => "mul",
        Ffma => "mad",
        Fsign => "sign",

        /* float transcendental. All of these are scalar on this machine --
         * `OpId::reads_one_component` -- which is the code generator's
         * business, not this file's: the operation is still one instruction. */
        Frcp => "rcp",
        Frsq => "rsq",
        Fsqrt => "sqrt",
        Fexp2 => "exp",
        Flog2 => "log",
        Fsin => "sin",
        Fcos => "cos",

        /* float rounding. No `trunc`: see `lower`. */
        Ffloor => "floor",
        Fceil => "ceil",
        Ffract => "frc",

        /* derivatives; the fragment stage only, which the front end enforces */
        Fddx => "dsx",
        Fddy => "dsy",

        /* dot products. The IR keeps these as opcodes BECAUSE they are native
         * here; if that ever stops being true the IR's decision changes with
         * it, so the two are asserted together in the tests. */
        Fdot2 => "dp2",
        Fdot3 => "dp3",
        Fdot4 => "dp4",

        /* integer. `imullo0` is the low half of the product, which is what a
         * 32-bit GLSL `*` means; `imulhi0` would be the other half. */
        Iadd => "add",
        Imul => "imullo0",
        /* No `Imod => "imod"`: neither the reference compiler nor Mesa's
         * etnaviv emits `imod` (nir_lower_idiv lowers every integer
         * remainder), so what it computes on this core is not established.
         * [`imod`] lowers it through the exact unsigned remainder. */
        Iand => "and",
        Ior => "or",
        Ixor => "xor",
        Ishl => "lshift",
        /* arithmetic or logical by TYPE alone: `rshift.s32` / `rshift.u32`,
         * as Mesa's IOP(ishr, RSHIFT) and UOP(ushr, RSHIFT) */
        Ishr | Ushr => "rshift",
        Bnot | Inot => "not",

        /* conversions between the numeric types. The bool ones are not here:
         * a bool is not a machine type, so B2f/B2i/F2b/I2b are lowerings. */
        F2i => "f2i",
        I2f => "i2f",
        /* the unsigned conversions are the same two operations at U32:
         * Mesa's UOP(f2u32, F2I) and UOP(u2f32, I2F) */
        F2u => "f2i",
        U2f => "i2f",

        _ => return None,
    })
}

/// Emit the instruction sequence for an operation [`native`] does not answer.
///
/// `out` receives the instructions in order; `fresh` hands out a virtual
/// register when a sequence needs scratch, because a lowering cannot know what
/// is live.
pub fn lower(
    op: AluOp,
    ty: OpType,
    dst: SelDst,
    srcs: &[SelSrc],
    out: &mut dyn FnMut(SelInstr),
    fresh: &mut dyn FnMut() -> VReg,
) -> Result<(), LowerError> {
    use AluOp::*;

    let want = op.arity();
    if srcs.len() != want {
        return Err(LowerError::Arity { op, want, got: srcs.len() });
    }

    match op {
        /* Negate and absolute are SRC_NEG and SRC_ABS, per source. When the IR
         * asks for the value itself rather than folding it into a consumer,
         * that is one `mov` carrying the modifier -- not an arithmetic
         * instruction, and not a multiply by -1, which would cost the same and
         * round differently for an infinity or a NaN.
         *
         * `mov` reads SRC2 and has no SRC0 at all, which is why this goes
         * through `operand` and never through `src`. */
        Fneg | Fabs => {
            let mut s = srcs[0];
            apply_modifier(&mut s, op == Fneg, op == Fabs)?;
            out(inst("mov", ty)?.dst(dst).operand(0, s));
            Ok(())
        }

        /* min/max and the conditional select are ONE instruction: `select`
         * carries a COND field.
         *
         * The machine's rule is  dst = (s0 <cond> s1) ? s1 : s2,  so:
         *
         *     fmin(a, b) -> select.gt  dst, a, b, a     (a > b) ? b : a
         *     fmax(a, b) -> select.lt  dst, a, b, a     (a < b) ? b : a
         *
         * The min/max family repeats src0 into src2. Two independent sources:
         * Mesa does `inst.src[2] = src[0]` for fmin/fmax/imin/imax/umin/umax
         * (etnaviv_compiler_nir_emit.c), and our own corpus agrees -- of 606
         * selects in the shipped table, all 303 `.gt`/`.lt` have src2 == src0
         * and all 303 `.nz` do not. A perfect partition with no exceptions.
         *
         * NOT recognised in the other direction: the .gt/.lt choice is a
         * convention of the lowering, not something the hardware forces --
         * `select.lt dst, b, a, b` is an equally valid min. So nothing here
         * tries to read a select back as a min, and `native` must never claim
         * one. */
        Fmin | Fmax | Imin | Imax | Umin | Umax => {
            let cond = if matches!(op, Fmin | Imin | Umin) { Cond::Gt } else { Cond::Lt };
            out(inst_c("select", ty, cond)?.dst(dst).operand(0, srcs[0]).operand(1, srcs[1]).operand(2, srcs[0]));
            Ok(())
        }

        /* `.nz` tests s0 alone rather than comparing it against s1, so the two
         * arms sit in s1 and s2 and src2 is NOT a repeat of src0. Both arms are
         * evaluated, which is what the IR's `Bcsel` promises. */
        Bcsel => {
            out(inst_c("select", ty, Cond::Nz)?.dst(dst).operand(0, srcs[0]).operand(1, srcs[1]).operand(2, srcs[2]));
            Ok(())
        }

        /* The comparisons. `cmp` is three-source: the third is the value
         * written where the condition holds, zero elsewhere, so the compiler
         * chooses what "true" is per instruction. This file chooses all-ones,
         * as Mesa does (`inst.src[2] = etna_immediate_int(-1)`), because with
         * all-ones B2f and B2i are one `and` each and the reductions are `or`
         * and `and` -- see those arms.
         *
         * There is no `.le` and no `.gt`: `a <= b` is `b >= a`. That is an
         * operand SWAP, and it is why Fle and Ile hand `srcs[1]` to operand 0.
         *
         * The float and integer forms differ only in TYPE, which the caller
         * passes; this arm does not second-guess it. */
        Flt | Ilt | Ult => compare(Cond::Lt, ty, dst, srcs[0], srcs[1], out),
        Feq | Ieq => compare(Cond::Eq, ty, dst, srcs[0], srcs[1], out),
        Fne | Ine => compare(Cond::Ne, ty, dst, srcs[0], srcs[1], out),
        Fle | Ile | Ule => compare(Cond::Ge, ty, dst, srcs[1], srcs[0], out),

        /* A bool becomes a number by masking the constant with it, which is
         * exactly what all-ones was chosen for: `and` with the bit pattern of
         * 1.0f gives 1.0 or 0.0, `and` with 1 gives 1 or 0. Mesa's table says
         * the same in a comment it did not have to write -- UOP(b2f32, AND),
         * "AND with fui(1.0f)".
         *
         * The type is the CALLER'S, not one this file picks. It used to
         * hard-code U32 on the argument that a mask is bitwise rather than
         * arithmetic -- which is true, and was still wrong to decide here:
         * gles2-core measured the reference compiler and it uses U32 for this
         * `and` and S32 for the reductions below, and only the caller knows
         * which operation it is lowering. A type chosen in the wrong file is
         * the same class of defect as PMODE, and it was the last thing
         * between these sequences and byte-identity with the reference. */
        B2f => {
            let one = imm(Src::imm_f32(1.0))?;
            out(inst("and", ty)?.dst(dst).operand(0, srcs[0]).operand(1, one));
            Ok(())
        }
        B2i => {
            let one = imm(Src::imm_s20(1))?;
            out(inst("and", ty)?.dst(dst).operand(0, srcs[0]).operand(1, one));
            Ok(())
        }

        /* ...and a number becomes a bool by comparing it against zero, which
         * produces all-ones directly, so nothing has to normalise afterwards.
         * GLSL says bool(x) is x != 0, including for -0.0, which compares
         * equal to 0.0 and therefore gives false. That is the required
         * answer. */
        F2b => {
            let zero = imm(Src::imm_f32(0.0))?;
            compare(Cond::Ne, OpType::F32, dst, srcs[0], zero, out)
        }
        I2b => {
            let zero = imm(Src::imm_s20(0))?;
            compare(Cond::Ne, OpType::S32, dst, srcs[0], zero, out)
        }

        /* There is no integer negate. Mesa writes `ADD 0, -x` -- IOP(ineg,
         * ADD) with the comment "/* ADD 0, -x */" and src[0] set to the
         * immediate 0 -- which relies on SRC_NEG applying to an integer
         * source. That is the one assumption here that the ISA table does not
         * state and the corpus cannot confirm (it has no `ineg`, because
         * Mesa's own shaders fold the negate into a consumer). It is
         * transcribed rather than invented, and it is the first thing to
         * suspect if an integer subtraction comes out wrong on the board. */
        Ineg => {
            let zero = imm(Src::imm_s20(0))?;
            let mut x = srcs[0];
            apply_modifier(&mut x, true, false)?;
            out(inst("add", ty)?.dst(dst).operand(0, zero).operand(1, x));
            Ok(())
        }

        /* trunc(x) = sign(x) * floor(|x|).
         *
         * Exact for every finite value: floor of a magnitude is the magnitude
         * with its fraction removed, and multiplying by the sign puts it back
         * on the right side of zero. sign(0) is 0 and floor(|0|) is 0, so zero
         * survives -- though the sign of a negative zero does not, which GLSL
         * ES 1.00 does not require.
         *
         * The obvious alternative, f2i followed by i2f, is one instruction
         * shorter and wrong outside +-2^31; this one has no range limit. */
        Ftrunc => {
            let s = fresh();
            let m = fresh();
            let all = SelDst::Virtual { v: s, write_mask: 0xf };
            let mag = SelDst::Virtual { v: m, write_mask: 0xf };
            let mut x_abs = srcs[0];
            apply_modifier(&mut x_abs, false, true)?;
            out(inst("sign", ty)?.dst(all).operand(0, srcs[0]));
            out(inst("floor", ty)?.dst(mag).operand(0, x_abs));
            out(inst("mul", ty)?.dst(dst).operand(0, vreg(s)).operand(1, vreg(m)));
            Ok(())
        }

        /* The reductions, over all four lanes.
         *
         * With all-ones booleans `any` is `or` and `all` is `and`, and the
         * component count does not have to be passed in: repeating a lane
         * changes neither, because both are idempotent. So the caller
         * materialises a vec2 or vec3 with a swizzle that repeats one of its
         * real components -- which is what a generator writing into a vec4
         * register does anyway -- and these fold all four.
         *
         * Stated as a contract because getting it wrong is silent: a caller
         * that leaves an unwritten lane in the register reduces over whatever
         * was there before. The tests below pin the shape.
         *
         * MEASURED COST (gles2-core, against the reference compiler): for a
         * `bvec2` the reference emits ONE `or` where this emits three. The
         * answer is right -- repeated lanes are idempotent for `or` and `and`
         * -- and it costs two instructions per reduction. Closing that needs
         * the component count at this interface, which it does not have: the
         * width is not derivable from a source whose spare lanes are legally
         * a repeat. It is a cost, not a defect, and it is written down rather
         * than guessed at from the swizzle. */
        Bany | Ball => {
            let name = if op == Bany { "or" } else { "and" };
            let t = fresh();
            let acc = SelDst::Virtual { v: t, write_mask: 0xf };
            out(inst(name, ty)?.dst(acc).operand(0, lane(srcs[0], 0)).operand(1, lane(srcs[0], 1)));
            out(inst(name, ty)?.dst(acc).operand(0, vreg(t)).operand(1, lane(srcs[0], 2)));
            out(inst(name, ty)?.dst(dst).operand(0, vreg(t)).operand(1, lane(srcs[0], 3)));
            Ok(())
        }

        /* Idiv is refused on purpose, and this is the one gap worth a
         * paragraph.
         *
         * The instruction set has `idiv0`. Nothing establishes what it does:
         * it appears zero times in the 13,854 shipped instructions, Mesa never
         * emits it (NIR lowers integer division before etnaviv sees it), and
         * the table gives its encoding but not its meaning. A float-reciprocal
         * sequence would be the other route and is not exact for large
         * operands.
         *
         * Emitting either would be a plausible sequence that draws a wrong
         * picture in cases nobody would look at, which is the failure this
         * whole file is arranged to avoid. So it stops the compile until
         * somebody decodes `idiv0` on the board or writes an exact sequence
         * with the argument attached. */
        Idiv => {
            let lanes = match dst {
                SelDst::Virtual { write_mask, .. } | SelDst::Fixed { write_mask, .. } => write_mask,
                SelDst::Address { .. } => return Err(LowerError::Unsupported(op)),
            };
            for c in 0..4usize {
                if lanes & (1 << c) != 0 {
                    idiv_lane(lane_dst(dst, c), lane(srcs[0], c), lane(srcs[1], c), out, fresh)?;
                }
            }
            Ok(())
        }

        _ => Err(LowerError::Unsupported(op)),
    }
}

/// `dst = (a <cond> b) ? all-ones : 0`.
fn compare(
    cond: Cond,
    ty: OpType,
    dst: SelDst,
    a: SelSrc,
    b: SelSrc,
    out: &mut dyn FnMut(SelInstr),
) -> Result<(), LowerError> {
    let t = imm(Src::imm_s20(-1))?;
    out(inst_c("cmp", ty, cond)?.dst(dst).operand(0, a).operand(1, b).operand(2, t));
    Ok(())
}

/// A texture look-up: one `Op::Tex` becomes one GC7000 texture instruction,
/// except the projective form, which the machine does not have.
///
/// This is the machine half of the texture built-ins; the language half (which
/// GLSL name becomes which `Es100Tex`) is `online/builtin_call.rs`.
///
/// # Where each mapping comes from
///
/// Mesa's `etna_emit_tex` (etnaviv_compiler_nir_emit.c, MIT, read and
/// transcribed): `nir_texop_tex` -> TEXLD, `txb` -> TEXLDB, `txl` -> TEXLDL,
/// with the coordinate in src[0] and the bias or level in src[1]. Our corpus
/// agrees for the two it contains -- texld 284, texldl 78 -- and contains
/// **zero** texldb, texldd, tg4, texldgpcf and texldlpcf. So the BIAS form is
/// transcribed and not measured, and is named as such here the way `Ineg` is
/// in [`lower`]: it is the first thing to suspect if a biased look-up samples
/// the wrong level on the board.
///
/// # Two different kinds of unmeasured, and they are not the same claim
///
/// `Tex2DBias` -> `texldb` is an ENCODING transcribed from Mesa: the corpus
/// contains zero `texldb`, so nothing here has ever been fed to this silicon
/// and the instruction could be wrong. `Tex2DProj` below is a LOWERING nobody
/// could have measured: no shader in the table projects, so there is no corpus
/// instance to find, and the instructions it emits (`rcp`, `mul`, `texld`) are
/// each among the most exercised in the table. A reader who sees both as "not
/// in the corpus" would weigh them the same; they are not. The first is a risk
/// about this hardware, the second only about GLSL's arithmetic.
///
/// `Es100Tex::TexCube` is `texld` as well. Cube-ness is a property of the
/// sampler's descriptor, not of the instruction -- the driver programs it in
/// `src/vivante/texture.rs` -- so there is no separate opcode to choose, and a
/// cube look-up differs only in that its coordinate has three components.
///
/// # The slots are set explicitly
///
/// `texld` HAS a SRC2 field (with `SRC2_USE`), and `SRC2_USE` is 0 in all 284
/// texlds and all 78 texldls of the corpus. So the ordinal mapping is wrong
/// here in both directions: `slot_of_operand(1)` would put a bias in SRC2
/// rather than SRC1, and filling every slot the operation has would demand an
/// operand that must not be there. Everything below uses `src(slot, ..)`.
pub fn texture(
    kind: Es100Tex,
    sampler: u8,
    dst: SelDst,
    coord: SelSrc,
    extra: Option<SelSrc>,
    out: &mut dyn FnMut(SelInstr),
    fresh: &mut dyn FnMut() -> VReg,
) -> Result<(), LowerError> {
    /* 31 is the field's own limit (TEX_ID is five bits) and the allocator
     * refuses beyond it; refusing here as well means the caller's mistake is
     * named where the sampler came from rather than where it is encoded. */
    if sampler > 31 {
        return Err(LowerError::Sampler(sampler));
    }
    let t = Tex { id: sampler, swizzle: Tex::IDENTITY };

    /* The projective look-up divides the coordinate by its last component
     * before sampling. The machine has no projective instruction -- nothing
     * in the table takes a projector -- so it is a reciprocal, a multiply and
     * an ordinary look-up.
     *
     * `Frcp` reads one component on this machine, which is what makes the
     * divisor a scalar without a swizzle of its own here: the builder
     * composes `xxxx` for a scalar operation (`OpId::reads_one_component`).
     * The component divided BY is the coordinate's w -- IR v1 puts a
     * projective look-up's divisor in lane 3 whatever the GLSL width (it
     * was lane 2 until then) -- so it is selected explicitly rather than
     * left to that rule. */
    let coord = if kind == Es100Tex::Tex2DProj {
        let r = fresh();
        let m = fresh();
        let rd = SelDst::Virtual { v: r, write_mask: 0xf };
        let md = SelDst::Virtual { v: m, write_mask: 0xf };
        out(inst("rcp", OpType::F32)?.dst(rd).src(2, lane(coord, 3)));
        out(inst("mul", OpType::F32)?.dst(md).src(0, coord).src(1, vreg(r)));
        vreg(m)
    } else {
        coord
    };

    let (name, wants_extra) = match kind {
        Es100Tex::Tex2D | Es100Tex::TexCube | Es100Tex::Tex2DProj => ("texld", false),
        Es100Tex::Tex2DBias => ("texldb", true),
        Es100Tex::Tex2DLod => ("texldl", true),
    };
    if wants_extra != extra.is_some() {
        return Err(LowerError::TexExtra(kind));
    }

    let mut i = inst(name, OpType::F32)?.tex(t).dst(dst).src(0, coord);
    if let Some(x) = extra {
        i = i.src(1, x);
    }
    out(i);
    Ok(())
}

/// `texelFetch`: one `txf`, the integer coordinate in SRC0, the level as a
/// float in SRC1 and the immediate 0x1100 in SRC2.
///
/// Mesa's etnaviv (MIT), read and transcribed: etna_emit_tex maps
/// `nir_texop_txf` to `ISA_OPC_TXF` with `src[2] = etna_immediate_int(0x1100)`,
/// and etnaviv_nir_lower_texture.c `legalize_txf_lod` turns the level into a
/// float (`nir_i2f32`) before it. The reference compiler emitted exactly that
/// for gl_test's `es300/array.frag`: `txf t2.___w, tex0.xxxw, t1.xyzw,
/// 0.0:f20, 4352:s20`. What 0x1100 means is not established; it is copied.
/// The caller hands `lod` already a float.
pub fn texel_fetch(
    sampler: u8,
    dst: SelDst,
    coord: SelSrc,
    lod: SelSrc,
    out: &mut dyn FnMut(SelInstr),
) -> Result<(), LowerError> {
    if sampler > 31 {
        return Err(LowerError::Sampler(sampler));
    }
    let t = Tex { id: sampler, swizzle: Tex::IDENTITY };
    out(inst("txf", OpType::F32)?.tex(t).dst(dst).src(0, coord).src(1, lod).src(2, imm(Src::imm_s20(TXF_SRC2))?));
    Ok(())
}

/// The immediate in a `txf`'s SRC2 (Mesa's etnaviv, `etna_emit_tex`).
pub const TXF_SRC2: i32 = 0x1100;

/// `dst` narrowed to lane `c`.
fn lane_dst(dst: SelDst, c: usize) -> SelDst {
    match dst {
        SelDst::Virtual { v, write_mask } => SelDst::Virtual { v, write_mask: write_mask & (1 << c) },
        SelDst::Fixed { reg, write_mask } => SelDst::Fixed { reg, write_mask: write_mask & (1 << c) },
        other => other,
    }
}

/// One lane of an integer division, `a / b` truncated toward zero.
///
/// Neither the reference compiler nor Mesa's etnaviv emits the core's
/// `idiv0`: Mesa lowers integer division (nir_lower_idiv) and the reference
/// table contains no `idiv0` at all, so it is not trusted here. GLSL ES 1.00
/// integers need only about 16 bits (4.5.2), so this divides in float -- exact
/// while both magnitudes are below 2^24 -- and corrects the quotient with the
/// integer remainder:
///
/// ```text
///   A = |a|   B = |b|
///   q = f2i(i2f(A) * rcp(i2f(B)))     within one of A/B
///   r = A - q*B
///   q += (r >= B)                     the reciprocal rounded low
///   q -= (r < 0)                      ... or high
///   a/b = (a ^ b) < 0 ? -q : q
/// ```
///
/// `rcp` is a scalar operation, so this is emitted once per lane, each lane
/// reading its own lane of `a` and `b`. Division by zero is undefined in GLSL
/// and gives whatever the reciprocal of zero converts to.
fn idiv_lane(
    dst: SelDst,
    a: SelSrc,
    b: SelSrc,
    out: &mut dyn FnMut(SelInstr),
    fresh: &mut dyn FnMut() -> VReg,
) -> Result<(), LowerError> {
    let x = |v: VReg| SelSrc::Virtual { v, swizzle: [0; 4], neg: false, abs: false };
    let negx = |v: VReg| SelSrc::Virtual { v, swizzle: [0; 4], neg: true, abs: false };
    let one = |v: VReg| SelDst::Virtual { v, write_mask: 0x1 };
    let (va, vb, fa, fb, rb, pq, q0, t, r, ge, q1, lt, q2, sx, ng, nq) = (
        fresh(),
        fresh(),
        fresh(),
        fresh(),
        fresh(),
        fresh(),
        fresh(),
        fresh(),
        fresh(),
        fresh(),
        fresh(),
        fresh(),
        fresh(),
        fresh(),
        fresh(),
        fresh(),
    );
    let (s32, f32) = (OpType::S32, OpType::F32);
    out(inst("iabs", s32)?.dst(one(va)).operand(0, a));
    out(inst("iabs", s32)?.dst(one(vb)).operand(0, b));
    out(inst("i2f", s32)?.dst(one(fa)).operand(0, x(va)));
    out(inst("i2f", s32)?.dst(one(fb)).operand(0, x(vb)));
    out(inst("rcp", f32)?.dst(one(rb)).operand(0, x(fb)));
    out(inst("mul", f32)?.dst(one(pq)).operand(0, x(fa)).operand(1, x(rb)));
    out(inst("f2i", s32)?.dst(one(q0)).operand(0, x(pq)));
    out(inst("imullo0", s32)?.dst(one(t)).operand(0, x(q0)).operand(1, x(vb)));
    out(inst("add", s32)?.dst(one(r)).operand(0, x(va)).operand(1, negx(t)));
    compare(Cond::Ge, s32, one(ge), x(r), x(vb), out)?;
    out(inst("add", s32)?.dst(one(q1)).operand(0, x(q0)).operand(1, negx(ge)));
    compare(Cond::Lt, s32, one(lt), x(r), imm(Src::imm_s20(0))?, out)?;
    out(inst("add", s32)?.dst(one(q2)).operand(0, x(q1)).operand(1, x(lt)));
    out(inst("xor", s32)?.dst(one(sx)).operand(0, a).operand(1, b));
    compare(Cond::Lt, s32, one(ng), x(sx), imm(Src::imm_s20(0))?, out)?;
    out(inst("add", s32)?.dst(one(nq)).operand(0, imm(Src::imm_s20(0))?).operand(1, negx(q2)));
    out(inst_c("select", OpType::U32, Cond::Nz)?.dst(dst).operand(0, x(ng)).operand(1, x(nq)).operand(2, x(q2)));
    Ok(())
}

/// The bits of 4294966784.0f (0x4f7ffffe), the scale [`udiv`]'s reciprocal
/// estimate is taken at: NIR's `emit_udiv` constant, 2^32 less 512 so that
/// `f2u` of `rcp(1) * scale` does not overflow. Its low 12 mantissa bits are
/// set, so no f20 immediate holds it and the caller supplies it from the
/// constant file; a 20-bit truncation (0x4f7ff000) is 2.4e-4 low, the Newton
/// step squares that to about 250 at 2^32, and two refinements fix 2.
pub const UDIV_RCP_SCALE_BITS: u32 = 0x4f7f_fffe;

/// `a / b` (or, with `modulo`, `a % b`) on 32-bit unsigned integers, EXACTLY,
/// one lane at a time over the lanes `dst` writes.
///
/// Mesa `nir_lower_idiv.c`, `emit_udiv` (MIT, read and transcribed), which is
/// what etnaviv runs for every integer divide (etnaviv_compiler_nir.c,
/// nir_lower_idiv); usc-backend transcribed the same function for the USC
/// (ecd8c91):
///
/// ```text
///   r = f2u(rcp(u2f(b)) * 4294966784.0)     a reciprocal estimate
///   r = r + umul_high(r, r * -b)            one Newton step
///   q = umul_high(a, r), m = a - q * b      quotient and remainder
///   twice: if m >= b { q += 1; m -= b }     the two refinements
/// ```
///
/// `umul_high` is not an instruction here: etnaviv sets `lower_mul_high`
/// (etnaviv_compiler.c) and NIR expands it over 16-bit halves
/// (`nir_lower_alu.c`, Hacker's Delight figure 8-2) -- see [`umul_high`].
/// So the core's `imulhi0`, which nothing we have ever run emits, stays
/// unused.
///
/// `k` is [`UDIV_RCP_SCALE_BITS`] as a source (the constant file). The
/// sequence is exact only if the hardware `rcp` is close to 1/x: the host
/// differential proves it with an exact reciprocal, the board has to vouch
/// for this core's. A zero divisor is undefined in GLSL; this computes
/// whatever the sequence computes.
pub fn udiv(
    modulo: bool,
    dst: SelDst,
    srcs: &[SelSrc],
    k: SelSrc,
    out: &mut dyn FnMut(SelInstr),
    fresh: &mut dyn FnMut() -> VReg,
) -> Result<(), LowerError> {
    let op = if modulo { AluOp::Umod } else { AluOp::Udiv };
    if srcs.len() != 2 {
        return Err(LowerError::Arity { op, want: 2, got: srcs.len() });
    }
    let lanes = match dst {
        SelDst::Virtual { write_mask, .. } | SelDst::Fixed { write_mask, .. } => write_mask,
        SelDst::Address { .. } => return Err(LowerError::Unsupported(op)),
    };
    for c in 0..4usize {
        if lanes & (1 << c) != 0 {
            udiv_lane(modulo, lane_dst(dst, c), lane(srcs[0], c), lane(srcs[1], c), lane(k, c), out, fresh)?;
        }
    }
    Ok(())
}

/// `a % b` on 32-bit signed integers, one lane at a time over the lanes
/// `dst` writes: the exact unsigned remainder of the magnitudes ([`udiv`]),
/// negated where `a` is negative.
///
/// Mesa `nir_lower_idiv.c` (MIT, read and transcribed), which etnaviv runs
/// for every integer remainder: `emit_idiv` takes `irem` as
/// `r = umod(|a|, |b|)` and `a < 0 ? -r : r`. GLSL ES 3.00 section 5.9
/// leaves `%` undefined when either operand is negative, so the sign rule
/// is Mesa's choice, not the language's. The machine's own `imod` is not
/// used: no reference program contains one (`tests/encoding_vocabulary.rs`).
pub fn imod(
    dst: SelDst,
    srcs: &[SelSrc],
    k: SelSrc,
    out: &mut dyn FnMut(SelInstr),
    fresh: &mut dyn FnMut() -> VReg,
) -> Result<(), LowerError> {
    if srcs.len() != 2 {
        return Err(LowerError::Arity { op: AluOp::Imod, want: 2, got: srcs.len() });
    }
    let lanes = match dst {
        SelDst::Virtual { write_mask, .. } | SelDst::Fixed { write_mask, .. } => write_mask,
        SelDst::Address { .. } => return Err(LowerError::Unsupported(AluOp::Imod)),
    };
    let x = |v: VReg| SelSrc::Virtual { v, swizzle: [0; 4], neg: false, abs: false };
    let negx = |v: VReg| SelSrc::Virtual { v, swizzle: [0; 4], neg: true, abs: false };
    let one = |v: VReg| SelDst::Virtual { v, write_mask: 0x1 };
    let s32 = OpType::S32;
    for c in 0..4usize {
        if lanes & (1 << c) == 0 {
            continue;
        }
        let (a, b) = (lane(srcs[0], c), lane(srcs[1], c));
        let (va, vb, m, n, sa) = (fresh(), fresh(), fresh(), fresh(), fresh());
        out(inst("iabs", s32)?.dst(one(va)).operand(0, a));
        out(inst("iabs", s32)?.dst(one(vb)).operand(0, b));
        udiv_lane(true, one(m), x(va), x(vb), lane(k, c), out, fresh)?;
        out(inst("add", s32)?.dst(one(n)).operand(0, imm(Src::imm_s20(0))?).operand(1, negx(m)));
        compare(Cond::Lt, s32, one(sa), a, imm(Src::imm_s20(0))?, out)?;
        out(inst_c("select", OpType::U32, Cond::Nz)?
            .dst(lane_dst(dst, c))
            .operand(0, x(sa))
            .operand(1, x(n))
            .operand(2, x(m)));
    }
    Ok(())
}

/// One lane of [`udiv`]. Every scratch value is lane x, written with mask
/// 0x1 and read `.xxxx`, because `imullo0` computes lane x only.
fn udiv_lane(
    modulo: bool,
    dst: SelDst,
    a: SelSrc,
    b: SelSrc,
    k: SelSrc,
    out: &mut dyn FnMut(SelInstr),
    fresh: &mut dyn FnMut() -> VReg,
) -> Result<(), LowerError> {
    let x = |v: VReg| SelSrc::Virtual { v, swizzle: [0; 4], neg: false, abs: false };
    let negx = |v: VReg| SelSrc::Virtual { v, swizzle: [0; 4], neg: true, abs: false };
    let (s32, u32_, f32_) = (OpType::S32, OpType::U32, OpType::F32);
    let one = |f: &mut dyn FnMut() -> VReg| {
        let v = f();
        (v, SelDst::Virtual { v, write_mask: 0x1 })
    };
    /* b as a value of its own, so the negate below is a source modifier on a
     * register (Ineg's `ADD 0, -x`, which is the one transcribed assumption
     * Ineg already names), whatever form `b` arrived in. The copy is the
     * untyped `mov` every reference program copies integers with (`mov
     * t1.xy__, void, void, u1.xxxx` in es300/vol.frag's): no reference
     * program contains a `mov.u32` (tests/encoding_vocabulary.rs). */
    let (vb, db) = one(fresh);
    out(inst("mov", f32_)?.dst(db).operand(0, b));
    let (fb, dfb) = one(fresh);
    out(inst("i2f", u32_)?.dst(dfb).operand(0, x(vb)));
    let (rb, drb) = one(fresh);
    out(inst("rcp", f32_)?.dst(drb).operand(0, x(fb)));
    let (sr, dsr) = one(fresh);
    out(inst("mul", f32_)?.dst(dsr).operand(0, x(rb)).operand(1, k));
    let (r0, dr0) = one(fresh);
    out(inst("f2i", u32_)?.dst(dr0).operand(0, x(sr)));
    let (nb, dnb) = one(fresh);
    out(inst("add", s32)?.dst(dnb).operand(0, imm(Src::imm_s20(0))?).operand(1, negx(vb)));
    let (rnb, drnb) = one(fresh);
    out(inst("imullo0", s32)?.dst(drnb).operand(0, x(r0)).operand(1, x(nb)));
    let h = umul_high(x(r0), x(rnb), out, fresh)?;
    let (r, dr) = one(fresh);
    out(inst("add", s32)?.dst(dr).operand(0, x(r0)).operand(1, x(h)));
    let mut q = umul_high(a, x(r), out, fresh)?;
    let (qb, dqb) = one(fresh);
    out(inst("imullo0", s32)?.dst(dqb).operand(0, x(q)).operand(1, x(vb)));
    let (mut m, dm) = one(fresh);
    out(inst("add", s32)?.dst(dm).operand(0, a).operand(1, negx(qb)));
    for step in 0..2 {
        let last = step == 1;
        let (ge, dge) = one(fresh);
        compare(Cond::Ge, u32_, dge, x(m), x(vb), out)?;
        if !modulo {
            let (q1, dq1) = one(fresh);
            out(inst("add", s32)?.dst(dq1).operand(0, x(q)).operand(1, imm(Src::imm_s20(1))?));
            let (qn, dqn) = if last && !modulo { (q, dst) } else { one(fresh) };
            out(inst_c("select", u32_, Cond::Nz)?.dst(dqn).operand(0, x(ge)).operand(1, x(q1)).operand(2, x(q)));
            q = qn;
        }
        if !last || modulo {
            let (m1, dm1) = one(fresh);
            out(inst("add", s32)?.dst(dm1).operand(0, x(m)).operand(1, negx(vb)));
            let (mn, dmn) = if last { (m, dst) } else { one(fresh) };
            out(inst_c("select", u32_, Cond::Nz)?.dst(dmn).operand(0, x(ge)).operand(1, x(m1)).operand(2, x(m)));
            m = mn;
        }
    }
    Ok(())
}

/// The high 32 bits of the unsigned 64-bit product `a * b`, into a fresh
/// register's lane x.
///
/// NIR's expansion for a back end with `lower_mul_high` (`nir_lower_alu.c`,
/// MIT, read and transcribed; Hacker's Delight figure 8-2), unsigned:
///
/// ```text
///   al = a & 0xffff, bl = b & 0xffff, ah = a >> 16, bh = b >> 16
///   t  = ah*bl + ((al*bl) >> 16)
///   w1 = al*bh + (t & 0xffff)
///   hi = ah*bh + (w1 >> 16) + (t >> 16)
/// ```
///
/// Each product is of two 16-bit halves, so `imullo0` holds it whole.
fn umul_high(
    a: SelSrc,
    b: SelSrc,
    out: &mut dyn FnMut(SelInstr),
    fresh: &mut dyn FnMut() -> VReg,
) -> Result<VReg, LowerError> {
    let x = |v: VReg| SelSrc::Virtual { v, swizzle: [0; 4], neg: false, abs: false };
    let (s32, u32_) = (OpType::S32, OpType::U32);
    let mask = imm(Src::imm_u20(0xffff))?;
    let sixteen = imm(Src::imm_u20(16))?;
    let new = |i: SelInstr, f: &mut dyn FnMut() -> VReg, out: &mut dyn FnMut(SelInstr)| {
        let v = f();
        out(i.dst(SelDst::Virtual { v, write_mask: 0x1 }));
        v
    };
    let al = new(inst("and", s32)?.operand(0, a).operand(1, mask), fresh, out);
    let bl = new(inst("and", s32)?.operand(0, b).operand(1, mask), fresh, out);
    let ah = new(inst("rshift", u32_)?.operand(0, a).operand(1, sixteen), fresh, out);
    let bh = new(inst("rshift", u32_)?.operand(0, b).operand(1, sixteen), fresh, out);
    let lo = new(inst("imullo0", s32)?.operand(0, x(al)).operand(1, x(bl)), fresh, out);
    let m1 = new(inst("imullo0", s32)?.operand(0, x(al)).operand(1, x(bh)), fresh, out);
    let m2 = new(inst("imullo0", s32)?.operand(0, x(ah)).operand(1, x(bl)), fresh, out);
    let hh = new(inst("imullo0", s32)?.operand(0, x(ah)).operand(1, x(bh)), fresh, out);
    let lo16 = new(inst("rshift", u32_)?.operand(0, x(lo)).operand(1, sixteen), fresh, out);
    let t = new(inst("add", s32)?.operand(0, x(m2)).operand(1, x(lo16)), fresh, out);
    let tl = new(inst("and", s32)?.operand(0, x(t)).operand(1, mask), fresh, out);
    let th = new(inst("rshift", u32_)?.operand(0, x(t)).operand(1, sixteen), fresh, out);
    let w1 = new(inst("add", s32)?.operand(0, x(m1)).operand(1, x(tl)), fresh, out);
    let w1h = new(inst("rshift", u32_)?.operand(0, x(w1)).operand(1, sixteen), fresh, out);
    let s = new(inst("add", s32)?.operand(0, x(w1h)).operand(1, x(th)), fresh, out);
    Ok(new(inst("add", s32)?.operand(0, x(hh)).operand(1, x(s)), fresh, out))
}

/// Start an instruction, resolving the operation by its (unique) bitset name.
fn inst(name: &'static str, ty: OpType) -> Result<SelInstr, LowerError> {
    op_id(name).map(|id: OpId| SelInstr::new(id, ty)).ok_or(LowerError::NoSuchOp(name))
}

/// Start an instruction that carries a `COND` field.
///
/// Separate from [`inst`] so that a condition is never left unset on one of
/// the eight operations that has the field: `COND = 0` is `True`, a perfectly
/// valid encoding, so a forgotten condition would not fail -- it would make
/// `cmp.lt` into an unconditional write.
fn inst_c(name: &'static str, ty: OpType, cond: Cond) -> Result<SelInstr, LowerError> {
    let mut i = inst(name, ty)?;
    i.cond = Some(cond);
    Ok(i)
}

/// An instruction immediate as a source.
fn imm(v: Result<Src, ImmError>) -> Result<SelSrc, LowerError> {
    v.map(SelSrc::Physical).map_err(LowerError::Immediate)
}

/// Read a whole virtual register, unswizzled.
fn vreg(v: VReg) -> SelSrc {
    SelSrc::Virtual { v, swizzle: [0, 1, 2, 3], neg: false, abs: false }
}

/// Broadcast one component of a source across all four lanes.
fn lane(s: SelSrc, n: usize) -> SelSrc {
    let mut s = s;
    let pick = |sw: &[u8; 4]| [sw[n], sw[n], sw[n], sw[n]];
    match &mut s {
        SelSrc::Virtual { swizzle, .. } | SelSrc::Input { swizzle, .. } => *swizzle = pick(swizzle),
        /* An immediate is one value in every lane, and its swizzle bits are
         * payload: picking a lane of it would change the number. */
        SelSrc::Physical(p) if p.is_immediate() => {}
        SelSrc::Physical(p) => p.swizzle = pick(&p.swizzle),
    }
    s
}

/// Turn on `SRC_NEG` / `SRC_ABS` for a source.
///
/// The hardware takes the magnitude first and negates after -- the display
/// template prints `-|x|` -- so setting both means "negative magnitude". A
/// caller wanting `|-x|` should ask for `abs` alone, since the two are equal.
///
/// **Not on an immediate.** There the `neg` and `abs` fields are bits 17 and
/// 18 of the value's payload, so toggling them changed the NUMBER: `x - 1.0`
/// (`Fadd(x, Fneg(1.0))`) was encoded as `add x, 5.4e-20` -- 1.0's payload
/// 0x3f800 with bit 17 cleared -- and every literal subtracted anywhere came
/// out as a tiny positive number (present from the first online build to
/// 2026-09-23; the reference table's negative immediates carry their sign
/// in payload bit 19). An immediate is negated as a value instead: an f20
/// flips or clears its sign bit, an s20 is re-encoded negated, and anything
/// else is refused rather than guessed at.
fn apply_modifier(s: &mut SelSrc, want_neg: bool, want_abs: bool) -> Result<(), LowerError> {
    if let SelSrc::Physical(p) = s {
        if p.is_immediate() {
            *p = modify_immediate(*p, want_neg, want_abs)?;
            return Ok(());
        }
    }
    let (neg, abs) = match s {
        SelSrc::Virtual { neg, abs, .. } | SelSrc::Input { neg, abs, .. } => (neg, abs),
        SelSrc::Physical(p) => (&mut p.neg, &mut p.abs),
    };
    if want_abs {
        *abs = true;
    }
    /* Toggle rather than set: a negate applied to a source that already
     * carries one cancels. Setting a flag that is already set would silently
     * lose a sign. */
    if want_neg {
        *neg = !*neg;
    }
    Ok(())
}

/// Negate and/or take the magnitude of an immediate's VALUE (see
/// [`apply_modifier`] for why its modifier fields cannot be used). The
/// magnitude is taken first, as the hardware does, so both means `-|x|`.
fn modify_immediate(p: Src, want_neg: bool, want_abs: bool) -> Result<Src, LowerError> {
    let payload = p.imm_payload_bits().ok_or(LowerError::Immediate(ImmError::OutOfRange))?;
    match (p.amode >> 1) & 3 {
        /* f20: the top 20 bits of an f32, so the sign is payload bit 19. */
        0 => {
            let mut bits = payload << 12;
            if want_abs {
                bits &= 0x7fff_ffff;
            }
            if want_neg {
                bits ^= 0x8000_0000;
            }
            Src::imm_f32(f32::from_bits(bits)).map_err(LowerError::Immediate)
        }
        /* s20: a two's-complement integer, sign-extended from bit 19. */
        1 => {
            let mut v = ((payload as i32) << 12) >> 12;
            if want_abs {
                v = v.wrapping_abs();
            }
            if want_neg {
                v = v.wrapping_neg();
            }
            Src::imm_s20(v).map_err(LowerError::Immediate)
        }
        /* u20 and f16: nothing here makes one negative, and negating an
         * unsigned or half value has no encoding this can vouch for. */
        _ => Err(LowerError::Immediate(ImmError::OutOfRange)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const XYZW: [u8; 4] = [0, 1, 2, 3];
    /* A vec4 integer division is 17 instructions per lane. */
    const MAX_SEQ: usize = 80;

    /// Every operation in the IR's kernel, so that the tests below can say
    /// something about ALL of them rather than about the ones somebody
    /// remembered. Adding an `AluOp` and not listing it here fails
    /// `the_kernel_is_accounted_for` by count.
    const ALL: [AluOp; 64] = {
        use AluOp::*;
        [
            Fadd, Fmul, Ffma, Fmin, Fmax, Fneg, Fabs, Fsign, Frcp, Frsq, Fsqrt, Fexp2, Flog2, Fsin, Fcos, Ffloor,
            Fceil, Ftrunc, Ffract, Fddx, Fddy, Fdot2, Fdot3, Fdot4, Flt, Fle, Feq, Fne, Iadd, Imul, Idiv, Imod, Ineg,
            Ishl, Ishr, Iand, Ior, Ixor, Ilt, Ile, Ieq, Ine, B2f, B2i, F2i, I2f, F2b, I2b, Bnot, Bany, Ball, Bcsel,
            Udiv, Umod, Ushr, Ult, Ule, Imin, Imax, Umin, Umax, Inot, U2f, F2u,
        ]
    };

    /// The operations [`lower`] does not take because their sequence needs a
    /// constant-file source it cannot make: [`udiv`] takes it from the
    /// caller. Editing this list is the only way one can be added.
    const TAKES_A_CONSTANT: [AluOp; 3] = [AluOp::Udiv, AluOp::Umod, AluOp::Imod];

    fn vsrc(v: u32) -> SelSrc {
        SelSrc::Virtual { v: VReg(v), swizzle: XYZW, neg: false, abs: false }
    }

    struct Seq {
        got: [Option<SelInstr>; MAX_SEQ],
        n: usize,
    }

    impl Seq {
        fn at(&self, i: usize) -> SelInstr {
            self.got[i].expect("instruction was emitted")
        }

        fn name(&self, i: usize) -> &'static str {
            self.at(i).op.op().name
        }

        /// Operand `ordinal` of instruction `i`, looked up through the
        /// operation's slot map rather than read out of `srcs[ordinal]`.
        ///
        /// The two are not the same and the difference is not decorative:
        /// `and`, `or` and `add` have SRC0 and SRC2 and no SRC1, while `mul`
        /// has SRC0 and SRC1. An assertion that indexed `srcs` directly would
        /// read `None` for a correct instruction -- which is how these tests
        /// first failed.
        fn arg(&self, i: usize, ordinal: usize) -> SelSrc {
            let ins = self.at(i);
            let slot = ins.op.slot_of_operand(ordinal).expect("the operation has this operand");
            ins.srcs[slot].expect("the operand was set")
        }
    }

    /// Run a lowering and collect what it emitted. Fixed capacity, because a
    /// lowering that runs away should fail the test rather than allocate.
    fn run(op: AluOp, ty: OpType, srcs: &[SelSrc]) -> Result<Seq, LowerError> {
        let mut got: [Option<SelInstr>; MAX_SEQ] = [None; MAX_SEQ];
        let mut n = 0usize;
        let mut next = 100u32;
        lower(
            op,
            ty,
            SelDst::Virtual { v: VReg(0), write_mask: 0xf },
            srcs,
            &mut |i| {
                assert!(n < MAX_SEQ, "lowering emitted more than {MAX_SEQ} instructions");
                got[n] = Some(i);
                n += 1;
            },
            &mut || {
                next += 1;
                VReg(next)
            },
        )?;
        Ok(Seq { got, n })
    }

    fn run2(op: AluOp, ty: OpType) -> Seq {
        let n = op.arity();
        let srcs: [SelSrc; 3] = [vsrc(1), vsrc(2), vsrc(3)];
        run(op, ty, &srcs[..n]).expect("lowered")
    }

    /// Negating an immediate negates its VALUE. Its neg/abs fields are payload
    /// bits, so the old toggle made `-(1.0)` into 5.4e-20: every literal
    /// subtracted anywhere. Checked against the numbers, not the fields.
    #[test]
    fn negating_an_immediate_negates_the_number() {
        let value = |s: &SelSrc| -> f32 {
            let SelSrc::Physical(p) = s else { panic!("still an immediate") };
            assert!(p.is_immediate());
            f32::from_bits(p.imm_payload_bits().expect("payload") << 12)
        };
        for (x, neg, abs) in [(1.0f32, -1.0f32, 1.0f32), (0.5, -0.5, 0.5), (-2.0, 2.0, 2.0), (0.0, -0.0, 0.0)] {
            let imm = SelSrc::Physical(Src::imm_f32(x).expect("exact"));
            let n = run(AluOp::Fneg, OpType::F32, &[imm]).expect("lowered");
            let src = n.at(0).srcs[2].expect("mov reads SRC2");
            assert_eq!(value(&src).to_bits(), neg.to_bits(), "-({x})");
            let a = run(AluOp::Fabs, OpType::F32, &[imm]).expect("lowered");
            assert_eq!(value(&a.at(0).srcs[2].expect("mov reads SRC2")), abs, "|{x}|");
        }
        /* An integer immediate is re-encoded, not bit-flipped. */
        let five = SelSrc::Physical(Src::imm_s20(5).expect("fits"));
        let n = run(AluOp::Ineg, OpType::S32, &[five]).expect("lowered");
        let SelSrc::Physical(p) = n.at(0).srcs.iter().flatten().nth(1).copied().expect("add reads -x") else {
            panic!()
        };
        let v = ((p.imm_payload_bits().expect("payload") as i32) << 12) >> 12;
        assert_eq!(v, -5, "-(5)");
        /* The control: a register source still carries the modifier. */
        let r = run(AluOp::Fneg, OpType::F32, &[vsrc(1)]).expect("lowered");
        assert!(matches!(r.at(0).srcs[2], Some(SelSrc::Virtual { neg: true, .. })));
    }

    /// The point of the whole file: every operation the IR defines is either
    /// one instruction, or has a written sequence, or is a gap named here on
    /// purpose. Nothing may be silently missing, and the list of gaps cannot
    /// grow without this test being edited.
    #[test]
    fn the_kernel_is_accounted_for() {
        let mut native_n = 0;
        let mut lowered = 0;
        let mut with_constant = 0;
        for op in ALL {
            let ty = OpType::F32;
            if native(op, ty).is_some() {
                native_n += 1;
                continue;
            }
            let n = op.arity();
            let srcs: [SelSrc; 3] = [vsrc(1), vsrc(2), vsrc(3)];
            match run(op, ty, &srcs[..n]) {
                Ok(seq) => {
                    assert!(seq.n > 0, "{op:?} lowered to nothing");
                    lowered += 1;
                }
                Err(LowerError::Unsupported(_)) if TAKES_A_CONSTANT.contains(&op) => {
                    let k = SelSrc::Physical(Src::reg(super::super::build::RGROUP_UNIFORM_0, 0));
                    let mut n = 0usize;
                    let mut next = 100u32;
                    let dst = SelDst::Virtual { v: VReg(0), write_mask: 0xf };
                    let mut count = |_| n += 1;
                    let mut fresh = || {
                        next += 1;
                        VReg(next)
                    };
                    if op == AluOp::Imod {
                        imod(dst, &srcs[..2], k, &mut count, &mut fresh).expect("imod lowers it");
                    } else {
                        udiv(op == AluOp::Umod, dst, &srcs[..2], k, &mut count, &mut fresh).expect("udiv lowers it");
                    }
                    assert!(n > 0, "{op:?} lowered to nothing");
                    with_constant += 1;
                }
                Err(LowerError::Unsupported(_)) => panic!("{op:?} is an unannounced gap"),
                Err(e) => panic!("{op:?} failed for a reason that is not a gap: {e:?}"),
            }
        }
        /* Every operation is native or lowered; there is no gap left. */
        assert_eq!(with_constant, TAKES_A_CONSTANT.len());
        assert_eq!(native_n + lowered + with_constant, ALL.len());
    }

    /// An operation must not be in both halves. If it were, which one the code
    /// generator got would depend on the order it asked, and the day the two
    /// disagreed nothing would catch it.
    #[test]
    fn native_and_lowered_are_disjoint() {
        for op in ALL {
            if native(op, OpType::F32).is_none() {
                continue;
            }
            let n = op.arity();
            let srcs: [SelSrc; 3] = [vsrc(1), vsrc(2), vsrc(3)];
            assert!(
                matches!(run(op, OpType::F32, &srcs[..n]), Err(LowerError::Unsupported(_))),
                "{op:?} is native AND lowered"
            );
        }
    }

    #[test]
    fn the_common_arithmetic_is_native() {
        assert_eq!(native(AluOp::Fadd, OpType::F32), Some("add"));
        assert_eq!(native(AluOp::Fmul, OpType::F32), Some("mul"));
        assert_eq!(native(AluOp::Ffma, OpType::F32), Some("mad"));
    }

    /// The GLSL ES 3.00 integer operations are the ES 1.00 instructions at
    /// another TYPE, as Mesa's etnaviv table has them; the TYPE is the
    /// caller's (select.rs `op_type`), so this pins the instruction, the
    /// condition, the operand order and that the caller's type is carried.
    #[test]
    fn the_es3_integer_operations_are_the_es1_instructions() {
        use AluOp::*;
        for (op, name) in [(Ushr, "rshift"), (Inot, "not"), (U2f, "i2f"), (F2u, "f2i")] {
            assert_eq!(native(op, OpType::U32), Some(name), "{op:?}");
        }
        /* min/max: select.gt / select.lt with src2 == src0, the Fmin shape. */
        for (op, cond) in [(Imin, Cond::Gt), (Imax, Cond::Lt), (Umin, Cond::Gt), (Umax, Cond::Lt)] {
            let s = run2(op, OpType::U32);
            assert_eq!((s.n, s.name(0), s.at(0).cond), (1, "select", Some(cond)), "{op:?}");
            assert_eq!(s.at(0).ty, OpType::U32, "{op:?} keeps the caller's type");
            assert!(matches!(s.at(0).srcs[2], Some(SelSrc::Virtual { v: VReg(1), .. })), "{op:?}: src2 repeats src0");
        }
        /* Ult is cmp.lt a, b; Ule is cmp.ge b, a (no .le on this machine). */
        let lt = run2(Ult, OpType::U32);
        assert_eq!((lt.name(0), lt.at(0).cond, lt.at(0).ty), ("cmp", Some(Cond::Lt), OpType::U32));
        assert!(matches!(lt.at(0).srcs[0], Some(SelSrc::Virtual { v: VReg(1), .. })));
        let le = run2(Ule, OpType::U32);
        assert_eq!((le.name(0), le.at(0).cond, le.at(0).ty), ("cmp", Some(Cond::Ge), OpType::U32));
        assert!(matches!(le.at(0).srcs[0], Some(SelSrc::Virtual { v: VReg(2), .. })), "a <= b is b >= a");
    }

    /// The exact unsigned divide uses only instructions this core has been
    /// fed (`imullo0`, never the unmeasured `imulhi0` or `idiv0`), reads the
    /// caller's scale constant once per lane it writes, and costs what it
    /// costs, pinned so a change to the sequence is a visible decision.
    #[test]
    fn the_unsigned_divide_is_imullo0_and_the_scale_once_per_lane() {
        let k = SelSrc::Physical(Src::reg(super::super::build::RGROUP_UNIFORM_0, 7));
        /* 7 to the estimate, 16 per umul_high, 1 Newton add, 2 for q*b and m;
         * then 5 + 3 refinement for the quotient, 3 + 3 for the remainder */
        for (modulo, want) in [(false, 50usize), (true, 48)] {
            for mask in [0x1u8, 0xf] {
                let mut got: Vec<SelInstr> = Vec::new();
                let mut next = 100u32;
                udiv(
                    modulo,
                    SelDst::Virtual { v: VReg(0), write_mask: mask },
                    &[vsrc(1), vsrc(2)],
                    k,
                    &mut |i| got.push(i),
                    &mut || {
                        next += 1;
                        VReg(next)
                    },
                )
                .expect("lowers");
                let lanes = mask.count_ones() as usize;
                let names: Vec<&str> = got.iter().map(|i| i.op.op().name).collect();
                assert!(!names.iter().any(|n| *n == "imulhi0" || *n == "idiv0"), "{names:?}");
                let reads_k = got
                    .iter()
                    .filter(|i| i.srcs.iter().flatten().any(|s| matches!(s, SelSrc::Physical(p) if !p.is_immediate())))
                    .count();
                assert_eq!(reads_k, lanes, "the scale, once per lane");
                assert_eq!(got.len(), want * lanes, "modulo {modulo}: instructions per lane");
            }
        }
    }

    /// The dot products are native. The IR keeps Fdot2/3/4 as opcodes because
    /// of this, so if they ever stop being native the IR's decision changes
    /// with them; the two are asserted together to keep that link visible.
    #[test]
    fn the_dot_products_are_native() {
        assert_eq!(native(AluOp::Fdot2, OpType::F32), Some("dp2"));
        assert_eq!(native(AluOp::Fdot3, OpType::F32), Some("dp3"));
        assert_eq!(native(AluOp::Fdot4, OpType::F32), Some("dp4"));
    }

    /// A `mov` reads SRC2 and has no SRC0 at all. Filling slots by ordinal
    /// produces an instruction that reads the wrong register and still
    /// encodes, so this asserts the slot and not just the opcode.
    #[test]
    fn a_negate_is_a_mov_in_the_right_slot() {
        let s = run2(AluOp::Fneg, OpType::F32);
        assert_eq!(s.n, 1);
        assert_eq!(s.name(0), "mov");
        assert!(s.at(0).srcs[0].is_none(), "mov has no SRC0");
        assert!(s.at(0).srcs[2].is_some(), "mov reads SRC2");
        match s.at(0).srcs[2].unwrap() {
            SelSrc::Virtual { neg, abs, .. } => assert!(neg && !abs),
            other => panic!("{other:?}"),
        }
    }

    /// min and max are one `select`, and the family's signature is that src2
    /// repeats src0 -- the partition that holds for all 606 selects in the
    /// shipped table.
    #[test]
    fn min_and_max_repeat_src0_into_src2() {
        for (op, want) in [(AluOp::Fmin, Cond::Gt), (AluOp::Fmax, Cond::Lt)] {
            let s = run2(op, OpType::F32);
            assert_eq!(s.n, 1);
            assert_eq!(s.name(0), "select");
            assert_eq!(s.at(0).cond, Some(want));
            let (a, c) = (s.at(0).srcs[0].unwrap(), s.at(0).srcs[2].unwrap());
            match (a, c) {
                (SelSrc::Virtual { v: x, .. }, SelSrc::Virtual { v: y, .. }) => assert_eq!(x, y),
                other => panic!("{other:?}"),
            }
        }
    }

    /// `.nz` tests src0 alone, so src2 is the second arm and must NOT be a
    /// repeat of src0 -- the other half of the same partition.
    #[test]
    fn a_select_does_not_repeat_src0() {
        let s = run2(AluOp::Bcsel, OpType::F32);
        assert_eq!(s.at(0).cond, Some(Cond::Nz));
        match (s.at(0).srcs[0].unwrap(), s.at(0).srcs[2].unwrap()) {
            (SelSrc::Virtual { v: x, .. }, SelSrc::Virtual { v: y, .. }) => assert_ne!(x, y),
            other => panic!("{other:?}"),
        }
    }

    /// Mesa emits CMP with EQ, NE, GE and LT and has no LE or GT row; our
    /// corpus has zero `.le` and zero `.gt` in 13,854 instructions. A lowering
    /// that emitted either would be the first program on this machine to try
    /// it, so no lowering here may produce one.
    #[test]
    fn no_lowering_emits_le_or_gt_on_a_compare() {
        for op in ALL {
            if native(op, OpType::F32).is_some() {
                continue;
            }
            let n = op.arity();
            let srcs: [SelSrc; 3] = [vsrc(1), vsrc(2), vsrc(3)];
            let Ok(seq) = run(op, OpType::F32, &srcs[..n]) else { continue };
            for i in 0..seq.n {
                if seq.name(i) != "cmp" {
                    continue;
                }
                let c = seq.at(i).cond;
                assert!(c != Some(Cond::Le) && c != Some(Cond::Gt), "{op:?} emitted {c:?} on a cmp");
            }
        }
    }

    /// `a <= b` is `b >= a`: the condition changes AND the operands swap. If
    /// only one of the two happened the comparison would be inverted, which is
    /// the kind of wrong that still draws a picture.
    #[test]
    fn less_or_equal_swaps_its_operands() {
        let s = run(AluOp::Fle, OpType::F32, &[vsrc(1), vsrc(2)]).expect("lowered");
        assert_eq!(s.name(0), "cmp");
        assert_eq!(s.at(0).cond, Some(Cond::Ge));
        match (s.at(0).srcs[0].unwrap(), s.at(0).srcs[1].unwrap()) {
            (SelSrc::Virtual { v: a, .. }, SelSrc::Virtual { v: b, .. }) => {
                assert_eq!(a, VReg(2), "the right-hand side is compared first");
                assert_eq!(b, VReg(1));
            }
            other => panic!("{other:?}"),
        }
    }

    /// Every comparison writes all-ones on true, which is what B2f, B2i and
    /// the reductions are built on. The immediate is the -1 the encoding
    /// carries inside the instruction, not a constant-file read.
    #[test]
    fn a_comparison_writes_all_ones() {
        for op in [AluOp::Flt, AluOp::Feq, AluOp::Fne, AluOp::Fle, AluOp::Ilt, AluOp::Ieq, AluOp::Ine, AluOp::Ile] {
            let s = run(op, OpType::F32, &[vsrc(1), vsrc(2)]).expect("lowered");
            assert_eq!(s.name(0), "cmp", "{op:?}");
            let t = s.at(0).srcs[2].expect("cmp has a true value");
            match t {
                SelSrc::Physical(p) => {
                    assert!(p.is_immediate(), "{op:?}: the true value is an immediate");
                    assert_eq!(p.imm_payload_bits(), Some(0xf_ffff), "{op:?}: 20 bits of -1");
                }
                other => panic!("{op:?}: {other:?}"),
            }
        }
    }

    /// A bool becomes 1.0 by masking, which is the reason all-ones was chosen.
    ///
    /// The type is the CALLER'S and is carried through unchanged. gles2-core
    /// measured the reference compiler: U32 for this `and`, S32 for the
    /// reductions. Only the caller knows which operation it is lowering, so
    /// asserting a fixed type here would re-introduce the decision this arm
    /// stopped making.
    #[test]
    fn bool_to_float_is_one_and() {
        let s = run(AluOp::B2f, OpType::U32, &[vsrc(1)]).expect("lowered");
        assert_eq!(s.n, 1);
        assert_eq!(s.name(0), "and");
        assert_eq!(s.at(0).ty, OpType::U32, "the caller's type, carried through");
        match s.arg(0, 1) {
            SelSrc::Physical(p) => assert_eq!(p.imm_payload_bits(), Some(1.0f32.to_bits() >> 12)),
            other => panic!("{other:?}"),
        }
    }

    /// ...and a float becomes a bool by comparing against zero, which produces
    /// all-ones directly.
    #[test]
    fn float_to_bool_compares_against_zero() {
        let s = run(AluOp::F2b, OpType::F32, &[vsrc(1)]).expect("lowered");
        assert_eq!(s.name(0), "cmp");
        assert_eq!(s.at(0).cond, Some(Cond::Ne));
        match s.at(0).srcs[1].unwrap() {
            SelSrc::Physical(p) => assert_eq!(p.imm_payload_bits(), Some(0)),
            other => panic!("{other:?}"),
        }
    }

    /// The integer negate is `add 0, -x`, and the negate is a source modifier.
    #[test]
    fn integer_negate_subtracts_from_zero() {
        let s = run(AluOp::Ineg, OpType::S32, &[vsrc(1)]).expect("lowered");
        assert_eq!(s.n, 1);
        assert_eq!(s.name(0), "add");
        match (s.arg(0, 0), s.arg(0, 1)) {
            (SelSrc::Physical(z), SelSrc::Virtual { neg, .. }) => {
                assert_eq!(z.imm_payload_bits(), Some(0));
                assert!(neg, "the value is negated by SRC_NEG, not by a multiply");
            }
            other => panic!("{other:?}"),
        }
    }

    /// trunc is sign(x) * floor(|x|): the absolute is a source modifier on the
    /// floor, not an instruction of its own.
    #[test]
    fn trunc_is_sign_times_floor_of_the_magnitude() {
        let s = run(AluOp::Ftrunc, OpType::F32, &[vsrc(1)]).expect("lowered");
        assert_eq!(s.n, 3);
        assert_eq!([s.name(0), s.name(1), s.name(2)], ["sign", "floor", "mul"]);
        match s.at(1).srcs[0].or(s.at(1).srcs[2]).unwrap() {
            SelSrc::Virtual { abs, neg, .. } => assert!(abs && !neg),
            other => panic!("{other:?}"),
        }
    }

    /// any/all are or/and over the lanes, and each step reads ONE broadcast
    /// component -- so a caller that repeats a component to fill a vec2 gets
    /// the right answer, which is the contract on the arm.
    #[test]
    fn the_reductions_fold_four_broadcast_lanes() {
        for (op, want) in [(AluOp::Bany, "or"), (AluOp::Ball, "and")] {
            /* S32 is what the reference uses for a reduction's or/and, and
             * what gles2-core's op_type now passes; the point of the assert is
             * that whatever arrives is what comes out. */
            let s = run(op, OpType::S32, &[vsrc(1)]).expect("lowered");
            assert_eq!(s.n, 3, "{op:?}");
            for i in 0..3 {
                assert_eq!(s.name(i), want);
                assert_eq!(s.at(i).ty, OpType::S32, "the caller's type, carried through");
            }
            match s.arg(0, 0) {
                SelSrc::Virtual { swizzle, .. } => assert_eq!(swizzle, [0, 0, 0, 0]),
                other => panic!("{other:?}"),
            }
            match s.arg(0, 1) {
                SelSrc::Virtual { swizzle, .. } => assert_eq!(swizzle, [1, 1, 1, 1]),
                other => panic!("{other:?}"),
            }
        }
    }

    /// Integer division stops the compile. It must never become a plausible
    /// sequence: `idiv0` has no decoded meaning and zero instances in the
    /// corpus.
    #[test]
    fn integer_division_is_lowered_lane_by_lane_without_idiv0() {
        /* Once per written lane, each ending in the select that applies the
         * sign; never the core's idiv0, which neither the reference nor Mesa
         * emits. */
        let s = run(AluOp::Idiv, OpType::S32, &[vsrc(1), vsrc(2)]).expect("lowered");
        assert_eq!(s.n, 4 * 17, "17 instructions per lane of a vec4");
        assert!((0..s.n).all(|i| s.name(i) != "idiv0"), "the lowering must not use idiv0");
        for lane in 0..4 {
            let last = s.at(lane * 17 + 16);
            assert_eq!(s.name(lane * 17 + 16), "select");
            assert!(matches!(last.dst, Some(SelDst::Virtual { write_mask, .. }) if write_mask == 1 << lane));
        }
        /* The operands are read one lane at a time: rcp is scalar. */
        assert!(matches!(s.at(17).srcs[2], Some(SelSrc::Virtual { swizzle: [1, 1, 1, 1], .. })), "lane 1 reads lane 1");
    }

    fn run_tex(kind: Es100Tex, sampler: u8, extra: Option<SelSrc>) -> Result<Seq, LowerError> {
        let mut got: [Option<SelInstr>; MAX_SEQ] = [None; MAX_SEQ];
        let mut n = 0usize;
        let mut next = 100u32;
        texture(
            kind,
            sampler,
            SelDst::Virtual { v: VReg(0), write_mask: 0xf },
            vsrc(1),
            extra,
            &mut |i| {
                assert!(n < MAX_SEQ, "the lowering emitted more than {MAX_SEQ} instructions");
                got[n] = Some(i);
                n += 1;
            },
            &mut || {
                next += 1;
                VReg(next)
            },
        )?;
        Ok(Seq { got, n })
    }

    /// A plain look-up is ONE instruction, the coordinate is in SRC0, and
    /// SRC1/SRC2 are left unset.
    ///
    /// The slots are the point. `texld` HAS a SRC2 field, and `SRC2_USE` is 0
    /// in all 284 texlds and all 78 texldls of the corpus -- so filling every
    /// slot the operation has would demand an operand that must not be there,
    /// and mapping by ordinal would put a bias in SRC2 rather than SRC1.
    #[test]
    fn a_plain_lookup_puts_the_coordinate_in_src0_and_nothing_else() {
        for kind in [Es100Tex::Tex2D, Es100Tex::TexCube] {
            let s = run_tex(kind, 3, None).expect("lowers");
            assert_eq!(s.n, 1, "{kind:?}");
            assert_eq!(s.name(0), "texld");
            assert!(s.at(0).srcs[0].is_some(), "the coordinate is SRC0");
            assert!(s.at(0).srcs[1].is_none(), "no bias");
            assert!(s.at(0).srcs[2].is_none(), "SRC2 exists on texld and must stay unused");
        }
    }

    /// The sampler is stage-relative and rides in TEX_ID, not in a source.
    /// The identity destination swizzle is what 412 of the corpus's 427
    /// texture instructions carry.
    #[test]
    fn the_sampler_and_the_swizzle_ride_in_the_tex_fields() {
        let s = run_tex(Es100Tex::Tex2D, 7, None).expect("lowers");
        let t = s.at(0).tex.expect("a texture instruction carries them");
        assert_eq!(t.id, 7);
        assert_eq!(t.swizzle, Tex::IDENTITY);
    }

    /// The bias and the level are DIFFERENT INSTRUCTIONS, not an optional
    /// operand on one: texldb and texldl, with the extra value in SRC1.
    #[test]
    fn the_bias_and_the_level_are_their_own_instructions() {
        for (kind, name) in [(Es100Tex::Tex2DBias, "texldb"), (Es100Tex::Tex2DLod, "texldl")] {
            let s = run_tex(kind, 0, Some(vsrc(2))).expect("lowers");
            assert_eq!(s.n, 1);
            assert_eq!(s.name(0), name, "{kind:?}");
            assert!(s.at(0).srcs[1].is_some(), "the bias or level is SRC1, not SRC2");
            assert!(s.at(0).srcs[2].is_none());
        }
    }

    /// A look-up given an extra it does not take, or denied one it needs, is
    /// refused: the two are different instructions, so this is a WRONG
    /// instruction rather than a missing operand.
    #[test]
    fn a_mismatched_extra_is_refused() {
        assert_eq!(run_tex(Es100Tex::Tex2D, 0, Some(vsrc(2))).err(), Some(LowerError::TexExtra(Es100Tex::Tex2D)));
        assert_eq!(run_tex(Es100Tex::Tex2DLod, 0, None).err(), Some(LowerError::TexExtra(Es100Tex::Tex2DLod)));
    }

    /// The machine has no projective look-up, so it is a reciprocal of the
    /// coordinate's LAST lane, w (IR v1 puts the divisor there whatever the
    /// GLSL width; it was z until then), a multiply, and an ordinary texld.
    #[test]
    fn the_projective_lookup_divides_before_it_samples() {
        let s = run_tex(Es100Tex::Tex2DProj, 1, None).expect("lowers");
        assert_eq!(s.n, 3);
        assert_eq!([s.name(0), s.name(1), s.name(2)], ["rcp", "mul", "texld"]);
        /* the divisor is the coordinate's w, broadcast */
        match s.at(0).srcs[2].expect("rcp reads SRC2") {
            SelSrc::Virtual { swizzle, .. } => assert_eq!(swizzle, [3, 3, 3, 3]),
            other => panic!("{other:?}"),
        }
        assert!(s.at(2).tex.is_some(), "the look-up still carries its sampler");
    }

    /// A sampler beyond the five bits TEX_ID has is refused by name.
    #[test]
    fn a_sampler_past_the_field_is_refused() {
        assert_eq!(run_tex(Es100Tex::Tex2D, 32, None).err(), Some(LowerError::Sampler(32)));
        assert!(run_tex(Es100Tex::Tex2D, 31, None).is_ok(), "31 is the last one that fits");
    }

    /// The wrong number of sources is refused rather than read from a slot
    /// nobody wrote.
    #[test]
    fn a_wrong_arity_is_refused() {
        assert_eq!(
            run(AluOp::Fmin, OpType::F32, &[vsrc(1)]).err(),
            Some(LowerError::Arity { op: AluOp::Fmin, want: 2, got: 1 })
        );
    }
}
