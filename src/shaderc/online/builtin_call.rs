// SPDX-License-Identifier: Apache-2.0 AND MIT
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
// SPDX-FileCopyrightText: 2015 Intel Corporation
// SPDX-FileCopyrightText: 2018 Red Hat Inc.
// Ported in part from Mesa 26.2.1 src/compiler/nir/nir_builtin_builder.c
// (nir_atan, nir_atan2) (MIT).
// The upstream part stays under its licence (LICENSES/MIT.txt); our
// changes are Apache-2.0. See NOTICE.
//! Lowering a call to the GLSL ES 1.00 built-in function library.
//!
//! This is a seam, deliberately. The built-in library is the largest single
//! body of work left in the front end and it is almost entirely independent
//! of the rest of it: given the arguments already lowered to IR operands, a
//! built-in is a rewrite from its name to a short run of [`AluOp`]s. Keeping
//! it behind [`Emit`] means it can be written and unit-tested against
//! hand-built operands, without a parser, a scope or an arena full of AST.
//!
//! # Not to be confused with `vivante/builtins.rs`
//!
//! There are two halves to "lowering a built-in" and they are in different
//! files because they answer different questions. This one is the *language*
//! half: a GLSL name and its arguments become [`AluOp`]s, and it knows
//! nothing about any GPU. `vivante/builtins.rs` is the *machine* half: an
//! `AluOp` becomes GC7000 instructions, and it knows nothing about GLSL.
//! `normalize` is expanded here, into `Fdot3`, `Frsq` and `Fmul`; how `Frsq`
//! reaches the hardware is decided there.
//!
//! # The order to implement them in
//!
//! Already measured, by `tools/gc7000-isa/builtin_usage.py`, and written up
//! at the head of `vivante/builtins.rs`: by how many additional sources
//! become *fully covered*, because a shader compiles only when every
//! built-in it calls is implemented. `normalize` is worth 38 sources, more
//! than the next five together; `clamp` is called 229 times and completes 2.
//! Frequency order is the obvious order and the wrong one.
//!
//! One caveat on reusing that table here: it was measured over all 307
//! sources in `SHC_SOURCE_TABLE`, so `texture`, `textureSize` and
//! `texelFetch` in it are ES 3.00 spellings that this compiler refuses at the
//! version line. For the 125 ES 1.00 sources the ES 2.0 spellings
//! (`texture2D`, `textureCube`) take their place.
//!
//! # What may and may not go in the opcode kernel
//!
//! `ir.rs` says it and it is worth repeating here, because this file is where
//! the temptation lives: the kernel is ~45 opcodes and everything else --
//! `pow`, `mod`, `clamp`, `mix`, `step`, `smoothstep`, `atan`, `asin`,
//! `acos`, the relational vector built-ins -- is *expanded* here.
//!
//! `matrixCompMult` is named in `ir.rs`'s list but is deliberately NOT one of
//! this file's, and the reason is worth a sentence because the obvious
//! reading sends a reader to write an arm that cannot exist. A `matN` has no
//! single-value form anywhere in the front end: `lower.rs`'s `matrix_value`
//! returns `[Src; 4]`, because neither GPU has a matrix register and a matrix
//! is N vectors wherever it lives. So the arguments arrive already split into
//! columns -- and `matrixCompMult`'s column j is `a[j] * b[j]` component-wise,
//! which is exactly an `Fmul` on two vectors, the operator `lower.rs` already
//! emits for `*`. There is nothing left for a built-in to do.
//!
//! Writing it here would also not fit: [`lower_call`] returns ONE `Src`, and
//! this built-in's result is N columns. Adding it would mean widening the
//! seam's return type for a single name that reduces to an operator. The dot products are the one exception, because the GC7000 has
//! `DP2`/`DP3`/`DP4` natively: expand `length`, `normalize`, `reflect` and
//! `refract` down to an `Fdot*` and **not past it**. `clamp` stays `Fmin` of
//! `Fmax` until `clamp0_max` is decoded.
//!
//! # What is implemented here today
//!
//! The built-ins that are one opcode and whose result has exactly the width
//! of their arguments: the table below. Everything else -- `dot`, `length`,
//! `normalize`, `mix`, `clamp`, `pow`, the texture look-ups -- is an
//! expansion still to be written, in the order at the head of
//! `vivante/builtins.rs`. A test asserts that an unwritten one is *not*
//! claimed, which is what stops a name being added to the table and silently
//! lowering a wrong width.
//!
//! Argument types are exact: [`Operand::ty`] carries what the type checker
//! worked out, so an expansion that needs to tell `vec3` from `mat3` can.

use super::ast::BaseType;
use super::ast::{SamplerDim, SamplerRet, SamplerType};
use super::ir::{AluOp, Es100Tex as TexKind, Op, ScalarType, Src, TexDim, TexLod, ValueId, SWIZZLE_IDENTITY};
use super::{Error, R};

/// Somewhere to put the instructions a built-in expands to.
///
/// The lowering pass implements this; a test implements it over a `Vec`. It
/// is the whole dependency this file has on the rest of the front end.
pub trait Emit<'a> {
    /// Append one instruction and give the operand that reads its result.
    fn emit(&mut self, op: Op) -> R<'a, Src>;

    /// A four-lane float constant, for the expansions that need one
    /// (`smoothstep`'s 3 and 2, `refract`'s 1).
    fn constant(&mut self, lanes: [f32; 4]) -> R<'a, Src>;

    /// The shader being lowered is a vertex shader: an implicit level of
    /// detail there is the base level (ES 3.00 section 8.8).
    fn vertex(&self) -> bool {
        false
    }

    /// The integer lanes `v` reads, when it reads an integer constant: a
    /// texel offset is one (section 8.8).
    fn int_lanes(&self, _v: Src) -> Option<[i32; 4]> {
        None
    }

    /// A value whose lanes 0..`comps` are `v`'s, read with the identity
    /// swizzle -- what `TexLod::Grad` names. The default takes `v` when it
    /// already is one and refuses otherwise; the lowering copies.
    fn identity(&mut self, v: Src, comps: u8) -> R<'a, ValueId> {
        if v.swizzle[..usize::from(comps)].iter().enumerate().all(|(i, &s)| usize::from(s) == i) {
            return Ok(v.val);
        }
        Err(Error::new(0, "texture", "a gradient that is not a whole value is not lowered here"))
    }
}

/// One already-lowered argument to a built-in call.
#[derive(Clone, Copy, Debug)]
pub enum Operand<'a> {
    Value {
        src: Src,
        /// Components the value has, 1..4.
        comps: u8,
        /// The GLSL type of the argument, as the type checker resolved it.
        ///
        /// `BaseType::Void` with an unknown `comps` is the checker having no
        /// opinion, which happens only for a construct it does not model; an
        /// expansion that needs the type must refuse rather than assume one.
        ty: BaseType<'a>,
    },
    /// A sampler argument, which is a binding rather than a value: it never
    /// reaches a register, it selects the texture unit in [`Op::Tex`]. Its
    /// type is what an ES 3.00 look-up's shape comes from.
    Sampler { binding: u8, ty: SamplerType },
}

impl<'a> Operand<'a> {
    /// The operand as a value, or the error for a sampler used as one.
    pub fn value(&self, name: &'a str) -> R<'a, (Src, u8)> {
        match *self {
            Operand::Value { src, comps, .. } => Ok((src, comps)),
            Operand::Sampler { .. } => Err(Error::new(0, name, "a sampler is not a value here")),
        }
    }
}

/// One built-in that is a single opcode applied component by component, with
/// the result as wide as the arguments.
struct Componentwise {
    name: &'static str,
    op: AluOp,
    args: usize,
}

/// The component-wise built-ins of section 8, in the specification's order.
///
/// Every one of these is a straight rename: the GLSL function and the opcode
/// mean the same thing on the same number of lanes, so no type beyond "how
/// many components" is needed to lower it, which is exactly why these are the
/// ones implemented first.
const COMPONENTWISE: &[Componentwise] = &[
    Componentwise { name: "sin", op: AluOp::Fsin, args: 1 },
    Componentwise { name: "cos", op: AluOp::Fcos, args: 1 },
    Componentwise { name: "exp2", op: AluOp::Fexp2, args: 1 },
    Componentwise { name: "log2", op: AluOp::Flog2, args: 1 },
    Componentwise { name: "sqrt", op: AluOp::Fsqrt, args: 1 },
    Componentwise { name: "inversesqrt", op: AluOp::Frsq, args: 1 },
    Componentwise { name: "abs", op: AluOp::Fabs, args: 1 },
    Componentwise { name: "sign", op: AluOp::Fsign, args: 1 },
    Componentwise { name: "floor", op: AluOp::Ffloor, args: 1 },
    Componentwise { name: "ceil", op: AluOp::Fceil, args: 1 },
    Componentwise { name: "fract", op: AluOp::Ffract, args: 1 },
    /* Fragment stage only; the type checker enforces where they may appear,
     * which is a question about the call's position rather than its meaning.
     * `GL_OES_standard_derivatives` is advertised, and measured before it was:
     * the shipped tables hold 39 call sites of the three on the PowerVR and 19
     * on the GC7000, all in sources with compiled blob rows. */
    Componentwise { name: "dFdx", op: AluOp::Fddx, args: 1 },
    Componentwise { name: "dFdy", op: AluOp::Fddy, args: 1 },
    Componentwise { name: "min", op: AluOp::Fmin, args: 2 },
    Componentwise { name: "max", op: AluOp::Fmax, args: 2 },
];

/// Lower a call to `name`.
///
/// `None` means the name is not a built-in function at all, so the caller
/// goes on to try a constructor and finally to refuse. `Some(Err)` is a
/// built-in used wrongly, or one not lowered yet -- and the refusal names the
/// function, so the histogram in `tests/online_lowering.rs` can count it.
pub fn lower_call<'a, E: Emit<'a>>(e: &mut E, name: &'a str, args: &[Operand<'a>], want: u8) -> Option<R<'a, Src>> {
    if let Some(b) = COMPONENTWISE.iter().find(|b| b.name == name) {
        return Some(componentwise(e, b, name, args, want));
    }
    if SEQUENCES.contains(&name) {
        return Some(sequence(e, name, args, want));
    }
    if TEXTURES.iter().any(|t| t.name == name) {
        return Some(texture(e, name, args));
    }
    if super::sema::ES300_TEXTURES.contains(&name) {
        return Some(texture_es3(e, name, args));
    }
    None
}

/// ES 3.00's added common functions (`sema::COMMON_ES3`), for an ES 3.00
/// source only -- the caller asks -- since ES 1.00 leaves the names free.
/// `None`: not one of them.
///
/// - `trunc` is `Ftrunc`; `round` is floor(x + 0.5), the specification
///   leaving the direction of a .5 to the implementation; `roundEven` is
///   that, one less where x is exactly halfway and the result odd.
/// - `isnan` and `isinf` test the BITS, with integer operations: a float
///   compare's behaviour with a NaN is the hardware's (x != x holds only
///   under IEEE rules neither back end promises), and an integer compare's
///   is not. NaN: exponent all ones and a non-zero fraction; infinity: all
///   ones and a zero fraction.
/// - The bit casts are no instruction: a lane holds bits, and only the
///   operations that read it say what kind they are (ir.md).
pub fn lower_es3<'a, E: Emit<'a>>(e: &mut E, name: &'a str, args: &[Operand<'a>]) -> Option<R<'a, Src>> {
    if !super::sema::COMMON_ES3.contains(&name) {
        return None;
    }
    Some((|| {
        let [a] = args else {
            return Err(Error::new(0, name, "this built-in takes a different number of arguments"));
        };
        let (x, _) = a.value(name)?;
        /* e^x and ln(x) from the base-2 pair, as `exp` and `log` do */
        let exp = |e: &mut E, v: Src| -> R<'a, Src> {
            let k = e.constant([core::f32::consts::LOG2_E; 4])?;
            let t = alu(e, AluOp::Fmul, [v, k, k])?;
            alu(e, AluOp::Fexp2, [t, t, t])
        };
        let ln = |e: &mut E, v: Src| -> R<'a, Src> {
            let l = alu(e, AluOp::Flog2, [v, v, v])?;
            let k = e.constant([core::f32::consts::LN_2; 4])?;
            alu(e, AluOp::Fmul, [l, k, k])
        };
        match name {
            /* Section 8.1 DEFINES each hyperbolic function by a formula, and
             * section 4.5.1 gives each the precision "inherited from" it --
             * which is what dEQP's builtin_precision checks against. So the
             * formulas are written out as they stand, not replaced by a
             * rearrangement that is better conditioned but checked against a
             * bound computed for something else. */
            "sinh" | "cosh" | "tanh" => {
                let ep = exp(e, x)?;
                let nx = alu(e, AluOp::Fneg, [x, x, x])?;
                let en = exp(e, nx)?;
                let half = e.constant([0.5; 4])?;
                let nen = alu(e, AluOp::Fneg, [en, en, en])?;
                let diff = alu(e, AluOp::Fadd, [ep, nen, nen])?;
                let sum = alu(e, AluOp::Fadd, [ep, en, en])?;
                match name {
                    /* (e^x - e^-x) / 2 */
                    "sinh" => alu(e, AluOp::Fmul, [diff, half, half]),
                    /* (e^x + e^-x) / 2 */
                    "cosh" => alu(e, AluOp::Fmul, [sum, half, half]),
                    /* sinh(x) / cosh(x): the halves cancel */
                    _ => {
                        let r = alu(e, AluOp::Frcp, [sum, sum, sum])?;
                        alu(e, AluOp::Fmul, [diff, r, r])
                    }
                }
            }
            /* ln(x + sqrt(x^2 + 1)) and ln(x + sqrt(x^2 - 1)) */
            "asinh" | "acosh" => {
                let one = e.constant([if name == "asinh" { 1.0 } else { -1.0 }; 4])?;
                let q = alu(e, AluOp::Ffma, [x, x, one])?;
                let s = alu(e, AluOp::Fsqrt, [q, q, q])?;
                let t = alu(e, AluOp::Fadd, [x, s, s])?;
                ln(e, t)
            }
            /* ln((1 + x) / (1 - x)) / 2 */
            "atanh" => {
                let one = e.constant([1.0; 4])?;
                let nx = alu(e, AluOp::Fneg, [x, x, x])?;
                let p = alu(e, AluOp::Fadd, [one, x, x])?;
                let m = alu(e, AluOp::Fadd, [one, nx, nx])?;
                let r = alu(e, AluOp::Frcp, [m, m, m])?;
                let q = alu(e, AluOp::Fmul, [p, r, r])?;
                let l = ln(e, q)?;
                let half = e.constant([0.5; 4])?;
                alu(e, AluOp::Fmul, [l, half, half])
            }
            "trunc" => alu(e, AluOp::Ftrunc, [x, x, x]),
            "round" | "roundEven" => {
                let half = e.constant([0.5; 4])?;
                let t = alu(e, AluOp::Fadd, [x, half, half])?;
                let r = alu(e, AluOp::Ffloor, [t, t, t])?;
                if name == "round" {
                    return Ok(r);
                }
                /* halfway: fract(x) == 0.5 (fract(-2.5) is 0.5 as well);
                 * odd: fract(r / 2) != 0 */
                let f = alu(e, AluOp::Ffract, [x, x, x])?;
                let tie = alu(e, AluOp::Feq, [f, half, half])?;
                let h = alu(e, AluOp::Fmul, [r, half, half])?;
                let hf = alu(e, AluOp::Ffract, [h, h, h])?;
                let zero = e.constant([0.0; 4])?;
                let odd = alu(e, AluOp::Fne, [hf, zero, zero])?;
                let both = alu(e, AluOp::Bcsel, [tie, odd, tie])?;
                let one = e.constant([1.0; 4])?;
                let n1 = alu(e, AluOp::Fneg, [one, one, one])?;
                let down = alu(e, AluOp::Fadd, [r, n1, n1])?;
                alu(e, AluOp::Bcsel, [both, down, r])
            }
            "isnan" | "isinf" => {
                let mask = e.emit(Op::ConstI32([0x7fff_ffff; 4]))?;
                let inf = e.emit(Op::ConstI32([0x7f80_0000; 4]))?;
                let m = alu(e, AluOp::Iand, [x, mask, mask])?;
                if name == "isnan" {
                    alu(e, AluOp::Ult, [inf, m, m])
                } else {
                    alu(e, AluOp::Ieq, [m, inf, inf])
                }
            }
            "packSnorm2x16" | "packUnorm2x16" | "packHalf2x16" => pack2x16(e, name, x),
            "unpackSnorm2x16" | "unpackUnorm2x16" | "unpackHalf2x16" => unpack2x16(e, name, x),
            /* floatBitsToInt / floatBitsToUint / intBitsToFloat /
             * uintBitsToFloat */
            _ => Ok(x),
        }
    })())
}

fn ci<'a, E: Emit<'a>>(e: &mut E, lanes: [i32; 4]) -> R<'a, Src> {
    e.emit(Op::ConstI32(lanes))
}

/// ES 3.00 section 8.4's pack functions: the two lanes of a vec2 as two
/// 16-bit fields of a uint, x in the low half. Worked on both lanes at once,
/// then combined.
///
/// - Snorm: round(clamp(c, -1, 1) * 32767) as a 16-bit two's complement;
///   Unorm: round(clamp(c, 0, 1) * 65535). "round" is to the nearest, a .5
///   up (floor(x + 0.5)), which the section's round() permits.
/// - Half: the float's bits, re-biased with integer operations -- NaN to
///   0x7e00, anything at or past 65520 to infinity, a normal half rounded
///   to the nearest even, and a value below the smallest normal half as a
///   denormal rounded to the nearest (a .5 up).
fn pack2x16<'a, E: Emit<'a>>(e: &mut E, name: &'a str, v: Src) -> R<'a, Src> {
    let h = match name {
        "packSnorm2x16" | "packUnorm2x16" => {
            let (lo, hi, k) = if name == "packSnorm2x16" { (-1.0, 1.0, 32767.0) } else { (0.0, 1.0, 65535.0) };
            let lo = e.constant([lo; 4])?;
            let hi = e.constant([hi; 4])?;
            let k = e.constant([k; 4])?;
            let half = e.constant([0.5; 4])?;
            let c = alu(e, AluOp::Fmax, [v, lo, lo])?;
            let c = alu(e, AluOp::Fmin, [c, hi, hi])?;
            let r = alu(e, AluOp::Ffma, [c, k, half])?;
            let r = alu(e, AluOp::Ffloor, [r, r, r])?;
            let i = alu(e, AluOp::F2i, [r, r, r])?;
            let mask = ci(e, [0xffff; 4])?;
            alu(e, AluOp::Iand, [i, mask, mask])?
        }
        _ => {
            /* the float's bits: v itself, read as integers */
            let k16 = ci(e, [16; 4])?;
            let k13 = ci(e, [13; 4])?;
            let s = alu(e, AluOp::Ushr, [v, k16, k16])?;
            let m8000 = ci(e, [0x8000; 4])?;
            let sign = alu(e, AluOp::Iand, [s, m8000, m8000])?;
            let m7f = ci(e, [0x7fff_ffff; 4])?;
            let a = alu(e, AluOp::Iand, [v, m7f, m7f])?;
            /* normal: (a - (112 << 23) + 0xfff + ((a >> 13) & 1)) >> 13 */
            let bias = ci(e, [-0x3800_0000; 4])?;
            let t = alu(e, AluOp::Iadd, [a, bias, bias])?;
            let q = alu(e, AluOp::Ushr, [a, k13, k13])?;
            let one = ci(e, [1; 4])?;
            let odd = alu(e, AluOp::Iand, [q, one, one])?;
            let kfff = ci(e, [0xfff; 4])?;
            let t = alu(e, AluOp::Iadd, [t, kfff, kfff])?;
            let t = alu(e, AluOp::Iadd, [t, odd, odd])?;
            let normal = alu(e, AluOp::Ushr, [t, k13, k13])?;
            /* denormal: |v| * 2^24, rounded */
            let av = alu(e, AluOp::Fabs, [v, v, v])?;
            let k24 = e.constant([16_777_216.0; 4])?;
            let half = e.constant([0.5; 4])?;
            let d = alu(e, AluOp::Ffma, [av, k24, half])?;
            let d = alu(e, AluOp::Ffloor, [d, d, d])?;
            let min_normal = ci(e, [0x3880_0000; 4])?;
            let is_small = alu(e, AluOp::Ult, [a, min_normal, min_normal])?;
            /* converted only where it is the answer: a large value, an
             * infinity or a NaN has no uint (and the conversion of one is
             * undefined) */
            let fzero = e.constant([0.0; 4])?;
            let d = alu(e, AluOp::Bcsel, [is_small, d, fzero])?;
            let denorm = alu(e, AluOp::F2u, [d, d, d])?;
            let h = alu(e, AluOp::Bcsel, [is_small, denorm, normal])?;
            let overflow = ci(e, [0x477f_f000; 4])?;
            let is_big = alu(e, AluOp::Ult, [overflow, a, a])?;
            let at_big = alu(e, AluOp::Ieq, [a, overflow, overflow])?;
            let big = alu(e, AluOp::Bcsel, [is_big, is_big, at_big])?;
            let inf = ci(e, [0x7c00; 4])?;
            let h = alu(e, AluOp::Bcsel, [big, inf, h])?;
            let finf = ci(e, [0x7f80_0000; 4])?;
            let nan = alu(e, AluOp::Ult, [finf, a, a])?;
            let qnan = ci(e, [0x7e00; 4])?;
            let h = alu(e, AluOp::Bcsel, [nan, qnan, h])?;
            alu(e, AluOp::Ior, [h, sign, sign])?
        }
    };
    let k16 = ci(e, [16; 4])?;
    let y = alu(e, AluOp::Ishl, [lane_of_src(h, 1), k16, k16])?;
    alu(e, AluOp::Ior, [lane_of_src(h, 0), y, y])
}

/// ES 3.00 section 8.4's unpack functions: a uint's two 16-bit fields as
/// the two lanes of a vec2, x from the low half.
///
/// - Snorm: clamp(f / 32767, -1, 1) of the sign-extended field; Unorm:
///   f / 65535.
/// - Half: re-biased into a float's bits -- a normal or an infinity / NaN
///   by shifting the exponent and fraction into place, a denormal through
///   its value, fraction * 2^-24.
fn unpack2x16<'a, E: Emit<'a>>(e: &mut E, name: &'a str, u: Src) -> R<'a, Src> {
    let w = Src { val: u.val, swizzle: [u.swizzle[0]; 4] };
    let shifts = ci(e, [0, 16, 0, 16])?;
    let mask = ci(e, [0xffff; 4])?;
    let f = alu(e, AluOp::Ushr, [w, shifts, shifts])?;
    let h = alu(e, AluOp::Iand, [f, mask, mask])?;
    match name {
        "unpackSnorm2x16" => {
            let k16 = ci(e, [16; 4])?;
            let up = alu(e, AluOp::Ishl, [h, k16, k16])?;
            let s = alu(e, AluOp::Ishr, [up, k16, k16])?;
            let x = alu(e, AluOp::I2f, [s, s, s])?;
            let k = e.constant([1.0 / 32767.0; 4])?;
            let x = alu(e, AluOp::Fmul, [x, k, k])?;
            let lo = e.constant([-1.0; 4])?;
            let hi = e.constant([1.0; 4])?;
            let x = alu(e, AluOp::Fmax, [x, lo, lo])?;
            alu(e, AluOp::Fmin, [x, hi, hi])
        }
        "unpackUnorm2x16" => {
            let x = alu(e, AluOp::U2f, [h, h, h])?;
            let k = e.constant([1.0 / 65535.0; 4])?;
            alu(e, AluOp::Fmul, [x, k, k])
        }
        _ => {
            let m8000 = ci(e, [0x8000; 4])?;
            let k16 = ci(e, [16; 4])?;
            let k13 = ci(e, [13; 4])?;
            let k10 = ci(e, [10; 4])?;
            let s = alu(e, AluOp::Iand, [h, m8000, m8000])?;
            let sign = alu(e, AluOp::Ishl, [s, k16, k16])?;
            let m7fff = ci(e, [0x7fff; 4])?;
            let em = alu(e, AluOp::Iand, [h, m7fff, m7fff])?;
            let ex = alu(e, AluOp::Ushr, [em, k10, k10])?;
            /* normal: ((e << 10 | m) + (112 << 10)) << 13 */
            let nb = ci(e, [0x1c000; 4])?;
            let n = alu(e, AluOp::Iadd, [em, nb, nb])?;
            let normal = alu(e, AluOp::Ishl, [n, k13, k13])?;
            /* infinity / NaN: exponent 31 -> 255 */
            let ib = ci(e, [0x38000; 4])?;
            let i = alu(e, AluOp::Iadd, [em, ib, ib])?;
            let special = alu(e, AluOp::Ishl, [i, k13, k13])?;
            /* denormal or zero: the fraction's value */
            let m3ff = ci(e, [0x3ff; 4])?;
            let m = alu(e, AluOp::Iand, [h, m3ff, m3ff])?;
            let mf = alu(e, AluOp::U2f, [m, m, m])?;
            let k = e.constant([1.0 / 16_777_216.0; 4])?;
            let denorm = alu(e, AluOp::Fmul, [mf, k, k])?;
            let zero = ci(e, [0; 4])?;
            let k31 = ci(e, [31; 4])?;
            let is_zero_e = alu(e, AluOp::Ieq, [ex, zero, zero])?;
            let is_top = alu(e, AluOp::Ieq, [ex, k31, k31])?;
            let r = alu(e, AluOp::Bcsel, [is_top, special, normal])?;
            let r = alu(e, AluOp::Bcsel, [is_zero_e, denorm, r])?;
            alu(e, AluOp::Ior, [r, sign, sign])
        }
    }
}

/// Lane `k` of `v`, in every lane.
fn lane_of_src(v: Src, k: usize) -> Src {
    Src { val: v.val, swizzle: [v.swizzle[k & 3]; 4] }
}

/// One texture built-in: its GLSL name, the IR shape it becomes, and whether
/// it takes a third argument (a bias or a level of detail).
struct Texture {
    name: &'static str,
    kind: TexKind,
    /// Components the coordinate has. `texture2DProj` takes a vec3 or a vec4
    /// and divides by the last, so it is checked separately.
    coord: u8,
    extra: bool,
}

/// The texture look-ups of GLSL ES 1.00, section 8.7.
///
/// The `Lod` forms are vertex-stage only and the bias forms fragment-stage
/// only; that is the type checker's rule to enforce, not this file's, because
/// it is about where a call appears rather than what it means.
///
/// `texture2DProjLod` is the projective divide and then `Tex2DLod`, as the
/// biased projective form is the divide and then `Tex2DBias`.
/// `textureCubeLod` and the biased `textureCube` have no ES 1.00 `TexKind`;
/// they are IR v1's `Op::Tex { dim: Cube, lod: Lod | Bias }`, the shape the
/// ES 3.00 `textureLod(samplerCube, ..)` and `texture(samplerCube, .., bias)`
/// already take, which both back ends agreed with the rest of IR v1.
const TEXTURES: &[Texture] = &[
    Texture { name: "texture2D", kind: TexKind::Tex2D, coord: 2, extra: false },
    Texture { name: "texture2DProj", kind: TexKind::Tex2DProj, coord: 3, extra: false },
    Texture { name: "texture2DLod", kind: TexKind::Tex2DLod, coord: 2, extra: true },
    Texture { name: "texture2DProjLod", kind: TexKind::Tex2DProj, coord: 3, extra: true },
    Texture { name: "textureCube", kind: TexKind::TexCube, coord: 3, extra: false },
    Texture { name: "textureCubeLod", kind: TexKind::TexCube, coord: 3, extra: true },
];

/// Lower a texture look-up to one `Op::Tex`.
///
/// The sampler is a BINDING, not a value: it never reaches a register, and
/// `Operand::Sampler` is how it arrives here. A texture call whose first
/// argument lowered to a value would mean the front end had turned a sampler
/// into a number, so that is refused rather than papered over.
fn texture<'a, E: Emit<'a>>(e: &mut E, name: &'a str, args: &[Operand<'a>]) -> R<'a, Src> {
    let t = TEXTURES.iter().find(|t| t.name == name).expect("the caller matched this name");
    let want_args = 2 + usize::from(t.extra);
    /* Section 8.7 gives texture2D, texture2DProj and textureCube each a
     * form with a trailing `float bias` (fragment stage only, which is the
     * checker's rule). The checker has the signatures and refuses a wrong
     * count first, so a count that reaches here is legal and a refusal is
     * a gap: it said "this built-in takes a different number of arguments"
     * until 2026-09-23, a verdict on six legal dEQP random shaders. */
    let bias_form = !t.extra && args.len() == 3;
    if args.len() != want_args && !bias_form {
        return Err(Error::new(0, name, "this form of the look-up is not lowered yet"));
    }
    /* Outside the fragment stage there are no derivatives. Section 8.7: "If
     * it is mip-mapped and running on the vertex shader, then the base
     * texture is used." IR v1 never hands a vertex module an `Implicit`
     * (ir.rs TexLod), so texture2D, texture2DProj and textureCube lower as
     * their Lod forms at level 0.0, as texture_es3 does for ES 3.00's
     * `texture`. The checker refuses the bias forms there first. */
    if e.vertex() && !t.extra && args.len() == 2 {
        let with_level = match t.kind {
            TexKind::Tex2D => "texture2DLod",
            TexKind::Tex2DProj => "texture2DProjLod",
            TexKind::TexCube => "textureCubeLod",
            _ => return Err(Error::new(0, name, "a vertex look-up with no level of this form is not lowered yet")),
        };
        let zero = e.constant([0.0; 4])?;
        let level = Operand::Value { src: zero, comps: 1, ty: BaseType::Float };
        return texture(e, with_level, &[args[0], args[1], level]);
    }
    let Operand::Sampler { binding, .. } = args[0] else {
        return Err(Error::new(0, name, "the first argument of a texture look-up is a sampler"));
    };
    let (coord, comps) = args[1].value(name)?;
    /* A cube look-up with a level or a bias: IR v1's cube `Op::Tex`. The
     * level or bias is one lane, splatted as `TexLod` wants it. */
    if t.kind == TexKind::TexCube && args.len() == 3 {
        if comps != 3 {
            return Err(Error::new(0, name, "this look-up's coordinate has the wrong number of components"));
        }
        let (x, n) = args[2].value(name)?;
        if n != 1 {
            return Err(Error::new(0, name, "a bias or level of detail is one component"));
        }
        let x = Src { val: x.val, swizzle: [x.swizzle[0]; 4] };
        let lod = if t.extra { TexLod::Lod(x) } else { TexLod::Bias(x) };
        return e.emit(Op::Tex {
            dim: TexDim::Cube,
            shadow: false,
            proj: false,
            ret: ScalarType::F32,
            sampler: binding,
            coord,
            lod,
            offset: None,
        });
    }

    /* texture2D with three arguments is the bias form, which is a different
     * IR shape rather than an optional extra on the same one: the hardware
     * has a separate instruction for it (`texldb`). */
    let kind = if t.name == "texture2D" && args.len() == 3 { TexKind::Tex2DBias } else { t.kind };
    let (coord, comps, kind) = if t.kind == TexKind::Tex2DProj && args.len() == 3 {
        /* The Lod form takes the level where the bias form takes a bias. */
        let then = if t.extra { TexKind::Tex2DLod } else { TexKind::Tex2DBias };
        /* texture2DProj with a bias: "(coord.s, coord.t) is divided by the
         * last component of coord" (section 8.7), then it is the biased 2D
         * look-up. The division is what `Tex2DProj` would do anyway; the IR
         * has no projective shape WITH a bias, so it is written out here. */
        if comps != 3 && comps != 4 {
            return Err(Error::new(0, name, "this look-up's coordinate has the wrong number of components"));
        }
        let last = coord.swizzle[usize::from(comps) - 1];
        let r = alu(e, AluOp::Frcp, [Src { val: coord.val, swizzle: [last; 4] }; 3])?;
        let st = alu(e, AluOp::Fmul, [coord, r, r])?;
        (st, 2, then)
    } else {
        (coord, comps, kind)
    };

    /* The projective form takes a vec3 OR a vec4 and divides by the last
     * component, so its width is the one case with two right answers. */
    let ok = match kind {
        TexKind::Tex2DProj => comps == 3 || comps == 4,
        /* The biased or levelled 2D look-up, whether written as texture2D
         * or reached from a projective form's projected (s, t). */
        TexKind::Tex2DBias | TexKind::Tex2DLod => comps == 2,
        _ => comps == t.coord,
    };
    if !ok {
        return Err(Error::new(0, name, "this look-up's coordinate has the wrong number of components"));
    }

    /* A projective look-up divides by lane w (IR v1, ir.md "Textures"):
     * a vec4 coordinate has its divisor there already, and a vec3 one has
     * it in z, which is repeated into w. Until IR v1 the divisor was lane z
     * and a vec4 arrived as (x, y, w, w). */
    let coord = if kind == TexKind::Tex2DProj && comps == 3 {
        let s = coord.swizzle;
        Src { val: coord.val, swizzle: [s[0], s[1], s[2], s[2]] }
    } else {
        coord
    };

    let extra = if args.len() == 3 {
        let (x, n) = args[2].value(name)?;
        if n != 1 {
            return Err(Error::new(0, name, "a bias or level of detail is one component"));
        }
        Some(x)
    } else {
        None
    };
    e.emit(Op::es100(kind, binding, coord, extra))
}

/// One ES 3.00 look-up (section 8.8) as IR v1's `Op::Tex` or `Op::TexSize`.
///
/// The checker has matched the call against the section's signatures, so the
/// arguments are in the order the name says: sampler, coordinate, then the
/// level (`Lod`) or the integer level (`texelFetch`) or the two gradients
/// (`Grad`), then the offset (`Offset`), then an optional bias.
fn texture_es3<'a, E: Emit<'a>>(e: &mut E, name: &'a str, args: &[Operand<'a>]) -> R<'a, Src> {
    let Some(&Operand::Sampler { binding, ty }) = args.first() else {
        return Err(Error::new(0, name, "the first argument of a texture look-up is a sampler"));
    };
    let dim = match ty.dim {
        SamplerDim::D2 => TexDim::D2,
        SamplerDim::D3 => TexDim::D3,
        SamplerDim::Cube => TexDim::Cube,
        SamplerDim::D2Array => TexDim::D2Array,
    };
    let lane0 = |(v, _): (Src, u8)| Src { val: v.val, swizzle: [v.swizzle[0]; 4] };
    let arg = |k: usize| -> R<'a, (Src, u8)> {
        match args.get(k) {
            Some(a) => a.value(name),
            None => Err(Error::new(0, name, "this look-up is missing an argument")),
        }
    };
    if name == "textureSize" {
        let lod = lane0(arg(1)?);
        return e.emit(Op::TexSize { sampler: binding, dim, lod });
    }
    let proj = name.starts_with("textureProj");
    let fetch = name.starts_with("texelFetch");
    let (coord, cw) = arg(1)?;
    let mut k = 2;
    let mut lod = if fetch {
        k += 1;
        TexLod::Fetch(lane0(arg(2)?))
    } else if name.contains("Lod") {
        k += 1;
        TexLod::Lod(lane0(arg(2)?))
    } else if name.contains("Grad") {
        let (dx, w) = arg(2)?;
        let (dy, _) = arg(3)?;
        k += 2;
        TexLod::Grad { ddx: e.identity(dx, w)?, ddy: e.identity(dy, w)? }
    } else {
        TexLod::Implicit
    };
    let offset = if name.ends_with("Offset") {
        let (o, ow) = arg(k)?;
        k += 1;
        let Some(lanes) = e.int_lanes(o) else {
            return Err(Error::new(0, name, "a texel offset that is not a literal is not lowered yet"));
        };
        let mut off = [0i8; 3];
        for (i, slot) in off.iter_mut().enumerate().take(usize::from(ow)) {
            /* MIN/MAX_PROGRAM_TEXEL_OFFSET are -8 and 7 (ES 3.0 table 6.28). */
            *slot = i8::try_from(lanes[i])
                .ok()
                .filter(|v| (-8..=7).contains(v))
                .ok_or_else(|| Error::new(0, name, "a texel offset outside -8..7 (MIN/MAX_PROGRAM_TEXEL_OFFSET)"))?;
        }
        Some(off)
    } else {
        None
    };
    if args.len() > k {
        lod = TexLod::Bias(lane0(arg(k)?));
    }
    /* Outside the fragment stage there are no derivatives: the implicit
     * level is the base one (section 8.8), and IR v1 never hands a vertex
     * module an `Implicit`. The checker refuses a bias there first. */
    if e.vertex() {
        match lod {
            TexLod::Implicit => lod = TexLod::Lod(e.constant([0.0; 4])?),
            TexLod::Bias(_) => {
                return Err(Error::new(0, name, "a texture look-up with a bias is only allowed in a fragment shader"))
            }
            _ => {}
        }
    }
    /* A projective 2D look-up's divisor is lane w in the IR: a vec3's z is
     * repeated there. */
    let coord = if proj && cw == 3 {
        let s = coord.swizzle;
        Src { val: coord.val, swizzle: [s[0], s[1], s[2], s[2]] }
    } else {
        coord
    };
    let ret = match (ty.shadow, ty.ret) {
        (true, _) | (_, SamplerRet::Float) => ScalarType::F32,
        (_, SamplerRet::Int) => ScalarType::I32,
        (_, SamplerRet::Uint) => ScalarType::U32,
    };
    e.emit(Op::Tex { dim, shadow: ty.shadow, proj, ret, sampler: binding, coord, lod, offset })
}

/// The integer kind of a built-in's first argument, or `None` for float.
fn integer_kind(args: &[Operand<'_>]) -> Option<SamplerRet> {
    match args.first() {
        Some(Operand::Value { ty: BaseType::Int | BaseType::IVec(_), .. }) => Some(SamplerRet::Int),
        Some(Operand::Value { ty: BaseType::Uint | BaseType::UVec(_), .. }) => Some(SamplerRet::Uint),
        _ => None,
    }
}

/// `abs`, `sign`, `min`, `max` and `clamp` on integers (ES 3.00 section
/// 8.3): the integer min and max of IR v1, and `abs`/`sign` built from them.
fn integer_common<'a, E: Emit<'a>>(e: &mut E, name: &'a str, k: SamplerRet, args: &[Operand<'a>]) -> R<'a, Src> {
    let arg = |i: usize| -> R<'a, Src> {
        match args.get(i) {
            Some(a) => a.value(name).map(|(s, _)| s),
            None => Err(Error::new(0, name, "this built-in takes a different number of arguments")),
        }
    };
    let signed = k == SamplerRet::Int;
    let (min, max) = if signed { (AluOp::Imin, AluOp::Imax) } else { (AluOp::Umin, AluOp::Umax) };
    match name {
        "min" => {
            let (a, b) = (arg(0)?, arg(1)?);
            alu(e, min, [a, b, b])
        }
        "max" => {
            let (a, b) = (arg(0)?, arg(1)?);
            alu(e, max, [a, b, b])
        }
        "clamp" => {
            let (x, lo, hi) = (arg(0)?, arg(1)?, arg(2)?);
            let m = alu(e, max, [x, lo, lo])?;
            alu(e, min, [m, hi, hi])
        }
        /* |x| = max(x, -x); -INT_MIN wraps to itself, which is as
         * undefined as the specification leaves it. */
        "abs" if signed => {
            let x = arg(0)?;
            let n = alu(e, AluOp::Ineg, [x, x, x])?;
            alu(e, AluOp::Imax, [x, n, n])
        }
        /* sign(x) = clamp(x, -1, 1). */
        "sign" if signed => {
            let x = arg(0)?;
            let lo = e.emit(Op::ConstI32([-1; 4]))?;
            let hi = e.emit(Op::ConstI32([1; 4]))?;
            let m = alu(e, AluOp::Imax, [x, lo, lo])?;
            alu(e, AluOp::Imin, [m, hi, hi])
        }
        _ => Err(Error::new(0, name, "this built-in has no integer form")),
    }
}

/// The built-ins that are more than one opcode.
///
/// Listed separately from their implementation so that "is this a built-in?"
/// and "how is it lowered?" cannot drift apart: a name here with no arm below
/// reaches the `unreachable` refusal rather than being mistaken for a
/// user-defined function, which is a different diagnostic for the author.
const SEQUENCES: &[&str] = &[
    "normalize",
    "length",
    "distance",
    "dot",
    "cross",
    "reflect",
    "clamp",
    "mix",
    "pow",
    "step",
    "smoothstep",
    "mod",
    "exp",
    "log",
    "radians",
    "degrees",
    "atan",
    "asin",
    "acos",
    "tan",
    "fwidth",
    "faceforward",
    "refract",
    "lessThan",
    "lessThanEqual",
    "greaterThan",
    "greaterThanEqual",
    "equal",
    "notEqual",
    "any",
    "all",
    "not",
];

/// The built-ins that expand to a short run of opcodes.
///
/// Written in the coverage order at the head of this file: `normalize` first,
/// because it alone completes 38 of the table's 307 sources.
fn sequence<'a, E: Emit<'a>>(e: &mut E, name: &'a str, args: &[Operand<'a>], want: u8) -> R<'a, Src> {
    let arg = |i: usize| -> R<'a, (Src, u8)> {
        match args.get(i) {
            Some(a) => a.value(name),
            None => Err(Error::new(0, name, "this built-in takes a different number of arguments")),
        }
    };
    match name {
        /* normalize(v) = v * inversesqrt(dot(v, v)).
         *
         * The dot stays a dot: the IR keeps Fdot2/3/4 as opcodes BECAUSE the
         * GC7000 has DP2/DP3/DP4, so expanding it into a multiply chain here
         * would be work the back end has to undo. */
        "normalize" => {
            let (v, n) = arg(0)?;
            let d = dot_of(e, name, v, v, n)?;
            let r = alu(e, AluOp::Frsq, [d, d, d])?;
            alu(e, AluOp::Fmul, [v, splat(r), splat(r)])
        }

        /* length(v) = sqrt(dot(v, v)); distance(a, b) = length(a - b). Both
         * give a scalar, so they need the ARGUMENT's width rather than the
         * caller's -- which is exactly what `Operand::comps` carries and why
         * these can be written at all. */
        "length" => {
            let (v, n) = arg(0)?;
            let d = dot_of(e, name, v, v, n)?;
            alu(e, AluOp::Fsqrt, [d, d, d])
        }
        "distance" => {
            let (a, n) = arg(0)?;
            let (b, m) = arg(1)?;
            if n != m {
                return Err(Error::new(0, name, "distance takes two values of the same width"));
            }
            let nb = alu(e, AluOp::Fneg, [b, b, b])?;
            let d = alu(e, AluOp::Fadd, [a, nb, nb])?;
            let q = dot_of(e, name, d, d, n)?;
            alu(e, AluOp::Fsqrt, [q, q, q])
        }
        "dot" => {
            let (a, n) = arg(0)?;
            let (b, m) = arg(1)?;
            if n != m {
                return Err(Error::new(0, name, "dot takes two values of the same width"));
            }
            dot_of(e, name, a, b, n)
        }

        /* cross(a, b) = a.yzx*b.zxy - a.zxy*b.yzx. vec3 only: GLSL has no
         * cross of any other width, so another one is a shader error and not
         * a case to handle. */
        "cross" => {
            let (a, n) = arg(0)?;
            let (b, m) = arg(1)?;
            if n != 3 || m != 3 {
                return Err(Error::new(0, name, "cross takes and gives a vec3"));
            }
            let m1 = alu(e, AluOp::Fmul, [sw(a, [2, 0, 1, 0]), sw(b, [1, 2, 0, 0]), sw(b, [1, 2, 0, 0])])?;
            let nm = alu(e, AluOp::Fneg, [m1, m1, m1])?;
            alu(e, AluOp::Ffma, [sw(a, [1, 2, 0, 0]), sw(b, [2, 0, 1, 0]), nm])
        }

        /* reflect(I, N) = I - 2*dot(N, I)*N, the specification's definition,
         * with N normalised as it also requires. */
        "reflect" => {
            let (i, n) = arg(0)?;
            let (nv, m) = arg(1)?;
            if n != m {
                return Err(Error::new(0, name, "reflect takes two values of the same width"));
            }
            let d = dot_of(e, name, nv, i, n)?;
            let k2 = e.constant([-2.0; 4])?;
            let k = alu(e, AluOp::Fmul, [d, k2, k2])?;
            alu(e, AluOp::Ffma, [splat(k), nv, i])
        }

        /* clamp(x, lo, hi) = min(max(x, lo), hi). The specification leaves the
         * result undefined when lo > hi, so the order needs no defending. */
        "clamp" if integer_kind(args).is_some() => {
            let k = integer_kind(args).unwrap_or(SamplerRet::Int);
            integer_common(e, name, k, args)
        }
        "clamp" => {
            let (x, _) = arg(0)?;
            let (lo, _) = arg(1)?;
            let (hi, _) = arg(2)?;
            let m = alu(e, AluOp::Fmax, [x, lo, lo])?;
            alu(e, AluOp::Fmin, [m, hi, hi])
        }

        /* mix(a, b, t) = a + t*(b - a): two instructions rather than the three
         * of a*(1-t) + b*t, and the same value. */
        /* ES 3.00 `mix(x, y, bvec a)`: y where a is true, x where it is not
         * -- a select, not an interpolation (section 8.3). */
        "mix" if matches!(args.get(2), Some(Operand::Value { ty: BaseType::Bool | BaseType::BVec(_), .. })) => {
            let (a, _) = arg(0)?;
            let (b, _) = arg(1)?;
            let (t, _) = arg(2)?;
            alu(e, AluOp::Bcsel, [t, b, a])
        }
        "mix" => {
            let (a, _) = arg(0)?;
            let (b, _) = arg(1)?;
            let (t, tn) = arg(2)?;
            let t = if tn == 1 && want > 1 { splat(t) } else { t };
            let na = alu(e, AluOp::Fneg, [a, a, a])?;
            let d = alu(e, AluOp::Fadd, [b, na, na])?;
            alu(e, AluOp::Ffma, [t, d, a])
        }

        /* pow(x, y) = exp2(y * log2(x)). GLSL leaves x < 0 undefined, which is
         * what makes this the definition rather than an approximation. */
        "pow" => {
            let (x, _) = arg(0)?;
            let (y, _) = arg(1)?;
            let l = alu(e, AluOp::Flog2, [x, x, x])?;
            let m = alu(e, AluOp::Fmul, [y, l, l])?;
            alu(e, AluOp::Fexp2, [m, m, m])
        }

        /* step(edge, x) is 0 below the edge and 1 at or above it. Flt gives a
         * bool and B2f makes it 0.0/1.0; the GC7000 back end turns B2f into
         * one `and`, so the whole thing is three instructions and no branch. */
        "step" => {
            let (edge, en) = arg(0)?;
            let (x, _) = arg(1)?;
            let edge = if en == 1 && want > 1 { splat(edge) } else { edge };
            let lt = alu(e, AluOp::Flt, [x, edge, edge])?;
            let ge = alu(e, AluOp::Bnot, [lt, lt, lt])?;
            alu(e, AluOp::B2f, [ge, ge, ge])
        }

        /* smoothstep: clamp((x-e0)/(e1-e0), 0, 1) then t*t*(3-2t), written out
         * from the specification. */
        "smoothstep" => {
            let (e0, n0) = arg(0)?;
            let (e1, n1) = arg(1)?;
            let (x, _) = arg(2)?;
            let e0 = if n0 == 1 && want > 1 { splat(e0) } else { e0 };
            let e1 = if n1 == 1 && want > 1 { splat(e1) } else { e1 };
            let ne0 = alu(e, AluOp::Fneg, [e0, e0, e0])?;
            let num = alu(e, AluOp::Fadd, [x, ne0, ne0])?;
            let den = alu(e, AluOp::Fadd, [e1, ne0, ne0])?;
            let inv = alu(e, AluOp::Frcp, [den, den, den])?;
            let t0 = alu(e, AluOp::Fmul, [num, inv, inv])?;
            let zero = e.constant([0.0; 4])?;
            let one = e.constant([1.0; 4])?;
            let lo = alu(e, AluOp::Fmax, [t0, zero, zero])?;
            let t = alu(e, AluOp::Fmin, [lo, one, one])?;
            let m2 = e.constant([-2.0; 4])?;
            let k3 = e.constant([3.0; 4])?;
            let inner = alu(e, AluOp::Ffma, [m2, t, k3])?;
            let tt = alu(e, AluOp::Fmul, [t, t, t])?;
            alu(e, AluOp::Fmul, [tt, inner, inner])
        }

        /* GLSL defines mod(x, y) as x - y * floor(x / y), in those words. Not
         * the machine's `imod`, which is a different operation on a different
         * type. */
        "mod" => {
            let (x, _) = arg(0)?;
            let (y, yn) = arg(1)?;
            let y = if yn == 1 && want > 1 { splat(y) } else { y };
            let r = alu(e, AluOp::Frcp, [y, y, y])?;
            let q = alu(e, AluOp::Fmul, [x, r, r])?;
            let f = alu(e, AluOp::Ffloor, [q, q, q])?;
            let nf = alu(e, AluOp::Fneg, [f, f, f])?;
            alu(e, AluOp::Ffma, [y, nf, x])
        }

        /* The machine has the base-2 pair only, so the natural exponential and
         * logarithm are one multiply away from it. */
        "exp" | "log" => {
            let (x, _) = arg(0)?;
            if name == "exp" {
                let k = e.constant([core::f32::consts::LOG2_E; 4])?;
                let t = alu(e, AluOp::Fmul, [x, k, k])?;
                alu(e, AluOp::Fexp2, [t, t, t])
            } else {
                let l = alu(e, AluOp::Flog2, [x, x, x])?;
                let k = e.constant([core::f32::consts::LN_2; 4])?;
                alu(e, AluOp::Fmul, [l, k, k])
            }
        }

        /* One multiply by a constant each; the constant is the whole content
         * of the built-in, which is why they are written out here rather than
         * sitting in a table of names. */
        "radians" | "degrees" => {
            let (x, _) = arg(0)?;
            let k = if name == "radians" { core::f32::consts::PI / 180.0 } else { 180.0 / core::f32::consts::PI };
            let k = e.constant([k; 4])?;
            alu(e, AluOp::Fmul, [x, k, k])
        }

        /* atan, in both forms. The last built-in the ES 1.00 half of the
         * shipped table needs: with it, 142 of 142 ES 1.00 sources call
         * nothing this file does not expand.
         *
         * MEASURED, and it decided which form to write first: of the 61 atan
         * calls in those sources, ZERO are the one-argument form and 61 are
         * the two-argument one. atan(y, x) is the real requirement and
         * atan(x) is the piece it is built from, which is the opposite of
         * what writing them in the specification's order would have given.
         *
         * Both are transcribed from Mesa's nir_atan / nir_atan2
         * (src/compiler/nir/nir_builtin_builder.c, MIT), read and rewritten
         * in our opcodes, never vendored. That is deliberate: the polynomial
         * and, much more, the quadrant and infinity handling are the parts
         * everybody gets wrong, and this is the version dEQP already passes
         * on other drivers. What is NOT transcribed is their bitwise
         * copysign, which we do not have the opcodes for -- see below. */
        "atan" => match args.len() {
            1 => {
                let (x, _) = arg(0)?;
                atan(e, x)
            }
            2 => {
                let (y, n) = arg(0)?;
                let (x, m) = arg(1)?;
                if n != m {
                    return Err(Error::new(0, name, "atan takes two values of the same width"));
                }
                atan2(e, y, x)
            }
            _ => Err(Error::new(0, name, "this built-in takes a different number of arguments")),
        },

        /* asin and acos share one polynomial, transcribed from Mesa's
         * build_asin (nir_builtin_builder.c:185, MIT). asin takes the
         * piecewise form -- a Taylor series below |x| = 0.21502245 -- because
         * the polynomial alone loses relative accuracy near zero, which is
         * where asin is most often used and where a relative error is most
         * visible. acos does not need it: pi/2 minus a small number is not
         * small, so the absolute error is what matters and the polynomial's
         * is fine. */
        "asin" => {
            let (x, _) = arg(0)?;
            asin(e, x, true)
        }
        "acos" => {
            let (x, _) = arg(0)?;
            let a = asin(e, x, false)?;
            let half_pi = e.constant([core::f32::consts::FRAC_PI_2; 4])?;
            let na = alu(e, AluOp::Fneg, [a, a, a])?;
            alu(e, AluOp::Fadd, [half_pi, na, na])
        }

        /* tan(x) = sin(x) / cos(x). The machine has both and a reciprocal, and
         * GLSL leaves tan undefined where cos is zero, so nothing here has to
         * defend the pole. */
        "tan" => {
            let (x, _) = arg(0)?;
            let s = alu(e, AluOp::Fsin, [x, x, x])?;
            let c = alu(e, AluOp::Fcos, [x, x, x])?;
            let r = alu(e, AluOp::Frcp, [c, c, c])?;
            alu(e, AluOp::Fmul, [s, r, r])
        }

        /* fwidth(p) = |dFdx(p)| + |dFdy(p)|, the specification's own
         * definition. Both absolutes are source modifiers on the GC7000, so
         * this is three instructions and not five. */
        "fwidth" => {
            let (x, _) = arg(0)?;
            let dx = alu(e, AluOp::Fddx, [x, x, x])?;
            let dy = alu(e, AluOp::Fddy, [x, x, x])?;
            let ax = alu(e, AluOp::Fabs, [dx, dx, dx])?;
            let ay = alu(e, AluOp::Fabs, [dy, dy, dy])?;
            alu(e, AluOp::Fadd, [ax, ay, ay])
        }

        /* faceforward(N, I, Nref) = dot(Nref, I) < 0 ? N : -N. */
        "faceforward" => {
            let (n_vec, w) = arg(0)?;
            let (i, wi) = arg(1)?;
            let (nref, wr) = arg(2)?;
            if w != wi || w != wr {
                return Err(Error::new(0, name, "faceforward takes three values of the same width"));
            }
            let d = dot_of(e, name, nref, i, w)?;
            let zero = e.constant([0.0; 4])?;
            let lt = alu(e, AluOp::Flt, [d, zero, zero])?;
            let neg = alu(e, AluOp::Fneg, [n_vec, n_vec, n_vec])?;
            /* the condition is one lane: every lane of the select reads it */
            alu(e, AluOp::Bcsel, [splat(lt), n_vec, neg])
        }

        /* refract(I, N, eta), the specification's own formula:
         *
         *   k = 1 - eta^2 * (1 - dot(N, I)^2)
         *   k < 0  ->  0          (total internal reflection)
         *   else   ->  eta*I - (eta*dot(N,I) + sqrt(k)) * N
         *
         * The k < 0 arm must be a select and not a branch: both arms are
         * evaluated, and sqrt of a negative k gives a NaN that the select then
         * discards. That is what the specification describes and what every
         * implementation does; a branch here would diverge a whole warp on a
         * per-pixel condition. */
        "refract" => {
            let (i, w) = arg(0)?;
            let (n_vec, wn) = arg(1)?;
            let (eta, we) = arg(2)?;
            if w != wn {
                return Err(Error::new(0, name, "refract takes two vectors of the same width"));
            }
            if we != 1 {
                return Err(Error::new(0, name, "refract's eta is one component"));
            }
            let eta = splat(eta);
            let d = dot_of(e, name, n_vec, i, w)?;
            let d = splat(d);
            let one = e.constant([1.0; 4])?;
            let nd2 = alu(e, AluOp::Fmul, [d, d, d])?;
            let nnd2 = alu(e, AluOp::Fneg, [nd2, nd2, nd2])?;
            let om = alu(e, AluOp::Fadd, [one, nnd2, nnd2])?;
            let e2 = alu(e, AluOp::Fmul, [eta, eta, eta])?;
            let t = alu(e, AluOp::Fmul, [e2, om, om])?;
            let nt = alu(e, AluOp::Fneg, [t, t, t])?;
            let k = alu(e, AluOp::Fadd, [one, nt, nt])?;

            let sk = alu(e, AluOp::Fsqrt, [k, k, k])?;
            let ed = alu(e, AluOp::Fmul, [eta, d, d])?;
            let sum = alu(e, AluOp::Fadd, [ed, sk, sk])?;
            let nsum = alu(e, AluOp::Fneg, [sum, sum, sum])?;
            let ei = alu(e, AluOp::Fmul, [eta, i, i])?;
            let val = alu(e, AluOp::Ffma, [nsum, n_vec, ei])?;

            let zero = e.constant([0.0; 4])?;
            let neg_k = alu(e, AluOp::Flt, [k, zero, zero])?;
            alu(e, AluOp::Bcsel, [neg_k, zero, val])
        }

        /* The relational vector built-ins (section 8.6). Each is one compare
         * per component, and the machine's compare gives all-ones, which is
         * this compiler's bool -- so there is nothing after the compare.
         *
         * There is no `.gt` and no `.le` on this hardware and none in Mesa's
         * table, so greaterThan and lessThanEqual are the operand swap, the
         * same rule the scalar compares follow. */
        "lessThan" | "lessThanEqual" | "greaterThan" | "greaterThanEqual" | "equal" | "notEqual" => {
            let (a, w) = arg(0)?;
            let (b, wb) = arg(1)?;
            if w != wb {
                return Err(Error::new(0, name, "a relational built-in takes two values of the same width"));
            }
            let int = matches!(
                args[0],
                Operand::Value {
                    ty: BaseType::Int
                        | BaseType::IVec(_)
                        | BaseType::Uint
                        | BaseType::UVec(_)
                        | BaseType::Bool
                        | BaseType::BVec(_),
                    ..
                }
            );
            /* ES 3.00's `uvec` orders as unsigned; equality is the same bits
             * either way. */
            let unsigned = matches!(args[0], Operand::Value { ty: BaseType::Uint | BaseType::UVec(_), .. });
            /* `swap` is the operand order, NOT a different condition: the
             * hardware has four conditions and the other two are reached by
             * exchanging the sides. */
            let (op, swap) = match (name, int) {
                ("lessThan", true) if unsigned => (AluOp::Ult, false),
                ("greaterThan", true) if unsigned => (AluOp::Ult, true),
                ("lessThanEqual", true) if unsigned => (AluOp::Ule, false),
                ("greaterThanEqual", true) if unsigned => (AluOp::Ule, true),
                ("lessThan", false) => (AluOp::Flt, false),
                ("lessThan", true) => (AluOp::Ilt, false),
                ("greaterThan", false) => (AluOp::Flt, true),
                ("greaterThan", true) => (AluOp::Ilt, true),
                ("lessThanEqual", false) => (AluOp::Fle, false),
                ("lessThanEqual", true) => (AluOp::Ile, false),
                ("greaterThanEqual", false) => (AluOp::Fle, true),
                ("greaterThanEqual", true) => (AluOp::Ile, true),
                ("equal", false) => (AluOp::Feq, false),
                ("equal", true) => (AluOp::Ieq, false),
                ("notEqual", false) => (AluOp::Fne, false),
                _ => (AluOp::Ine, false),
            };
            let (l, r) = if swap { (b, a) } else { (a, b) };
            alu(e, op, [l, r, r])
        }

        /* any/all reduce a bvec to one bool, `not` inverts component-wise.
         * The back end turns the reductions into `or` and `and` chains, which
         * is why the IR keeps them as opcodes rather than expanding here: the
         * width is the back end's to fold, not the front end's to unroll. */
        /* Ball and Bany fold ALL FOUR lanes (the contract in
         * vivante/builtins.rs), so a bvec2 or bvec3 argument has its last
         * real component repeated into the spare lanes -- idempotent for
         * `and` and `or`, as `compare_vector` does for `==`. Passed as it
         * came until 2026-09-23: `all(b)` on a bvec2 folded two lanes
         * nothing wrote. */
        "any" | "all" => {
            let (v, n) = arg(0)?;
            let mut swizzle = v.swizzle;
            let last = swizzle[usize::from(n.clamp(1, 4)) - 1];
            for lane in swizzle.iter_mut().skip(usize::from(n.clamp(1, 4))) {
                *lane = last;
            }
            let v = Src { val: v.val, swizzle };
            let op = if name == "any" { AluOp::Bany } else { AluOp::Ball };
            alu(e, op, [v, v, v])
        }
        "not" => {
            let (v, _) = arg(0)?;
            alu(e, AluOp::Bnot, [v, v, v])
        }

        /* A name in SEQUENCES with no arm here. Refused rather than passed on
         * as a user-defined function, because the two are different mistakes
         * for the shader's author to read about. */
        _ => Err(Error::new(0, name, "this built-in is named as a sequence but has no lowering")),
    }
}

/// The minimax polynomial `nir_atan` evaluates, highest power first.
///
/// Horner over x^2:
///   x*0.9999793128310355 - x^3*0.3326756418091246 + x^5*0.1938924977115610
///   - x^7*0.1173503194786851 + x^9*0.0536813784310406 - x^11*0.0121323213173444
const ATAN_COEFFS: [f32; 6] = [-0.012_132_321_5, 0.053_681_377, -0.117_350_32, 0.193_892_5, -0.332_675_64, 0.999_979_3];

/// `atan(x)`, one argument.
///
/// Range reduction to |u| <= 1 (using 1/x above that, which the polynomial
/// then evaluates and the bias corrects by pi/2), the polynomial, then the
/// sign of the input put back.
///
/// The sign step is ours rather than Mesa's: they use a bitwise copysign,
/// which needs opcodes this IR does not have. `|t| * sign(x)` agrees with it
/// everywhere except at x = -0.0, where copysign gives -0.0 and this gives
/// 0.0. GLSL ES 1.00 does not distinguish the two (it has no signed-zero
/// requirement at all), so the difference is unobservable to a conforming
/// shader -- but it IS a difference, and it is written down here rather than
/// left for someone to find in a precision test.
fn atan<'a, E: Emit<'a>>(e: &mut E, x: Src) -> R<'a, Src> {
    let one = e.constant([1.0; 4])?;
    let abs_x = alu(e, AluOp::Fabs, [x, x, x])?;
    let le1 = alu(e, AluOp::Fle, [abs_x, one, one])?;
    let rcp = alu(e, AluOp::Frcp, [x, x, x])?;
    let u = alu(e, AluOp::Bcsel, [le1, x, rcp])?;

    let x2 = alu(e, AluOp::Fmul, [u, u, u])?;
    let mut res = e.constant([ATAN_COEFFS[0]; 4])?;
    for c in &ATAN_COEFFS[1..] {
        let k = e.constant([*c; 4])?;
        res = alu(e, AluOp::Ffma, [res, x2, k])?;
    }

    /* The reduction's fix-up: nothing below 1, minus pi/2 above it, which
     * the multiply by |u| below turns into the pi/2 - atan(1/x) identity. */
    let zero = e.constant([0.0; 4])?;
    let nhalf_pi = e.constant([-core::f32::consts::FRAC_PI_2; 4])?;
    let bias = alu(e, AluOp::Bcsel, [le1, zero, nhalf_pi])?;
    let au = alu(e, AluOp::Fabs, [u, u, u])?;
    let t = alu(e, AluOp::Ffma, [au, res, bias])?;

    let sign = alu(e, AluOp::Fsign, [x, x, x])?;
    let at = alu(e, AluOp::Fabs, [t, t, t])?;
    alu(e, AluOp::Fmul, [at, sign, sign])
}

/// `atan(y, x)`: the quadrant-correct arc tangent.
///
/// Transcribed from Mesa's `nir_atan2`, whose comments explain the two parts
/// that are not obvious and that a first attempt always gets wrong:
///
/// 1. **The left half-plane is rotated by pi/2** before the division, so that
///    the y = 0 discontinuity lands on the vertical discontinuity of
///    atan(s/t) rather than on a division by zero.
/// 2. **A huge denominator is scaled down** by 1/4 before the reciprocal, so
///    that `1/t` does not flush to zero -- which would lose precision, and
///    for an infinite numerator would return NaN where the answer is finite.
///
/// The `|x| == |y|` case is forced to a tangent of 1 so that the infinities
/// of atan2(+-inf, +-inf) come out as +-pi/4 and +-3pi/4, as IEEE 754-2008
/// requires. GLSL explicitly allows deviating at (0, 0), which this takes.
fn atan2<'a, E: Emit<'a>>(e: &mut E, y: Src, x: Src) -> R<'a, Src> {
    let zero = e.constant([0.0; 4])?;
    let one = e.constant([1.0; 4])?;

    let ax = alu(e, AluOp::Fabs, [x, x, x])?;
    let ay = alu(e, AluOp::Fabs, [y, y, y])?;

    /* flip = x <= 0, the left half-plane. */
    let flip = alu(e, AluOp::Fle, [x, zero, zero])?;
    let s = alu(e, AluOp::Bcsel, [flip, ax, y])?;
    let t = alu(e, AluOp::Bcsel, [flip, y, ax])?;

    /* scale = 1/4 where |t| is huge, 1 otherwise. 1e18 and 0.25 are Mesa's
     * choice: huge <= 1/fmin, and a negative power of two so the scaling
     * itself is exact. */
    let huge = e.constant([1e18; 4])?;
    let at = alu(e, AluOp::Fabs, [t, t, t])?;
    let big = alu(e, AluOp::Fle, [huge, at, at])?;
    let quarter = e.constant([0.25; 4])?;
    let scale = alu(e, AluOp::Bcsel, [big, quarter, one])?;

    let st = alu(e, AluOp::Fmul, [t, scale, scale])?;
    let rcp_st = alu(e, AluOp::Frcp, [st, st, st])?;
    let ss = alu(e, AluOp::Fmul, [s, scale, scale])?;
    let ass = alu(e, AluOp::Fabs, [ss, ss, ss])?;
    let arcp = alu(e, AluOp::Fabs, [rcp_st, rcp_st, rcp_st])?;
    let q = alu(e, AluOp::Fmul, [ass, arcp, arcp])?;

    let eq = alu(e, AluOp::Feq, [ax, ay, ay])?;
    let tan = alu(e, AluOp::Bcsel, [eq, one, q])?;

    let a = atan(e, tan)?;
    let fb = alu(e, AluOp::B2f, [flip, flip, flip])?;
    let half_pi = e.constant([core::f32::consts::FRAC_PI_2; 4])?;
    let arc = alu(e, AluOp::Ffma, [fb, half_pi, a])?;

    /* The sign, and not with Fsign: where x < 0 the answer must distinguish
     * the two sides of y = 0, and `min(y, 1/t)` is the test that does it --
     * Mesa's own, kept rather than simplified. */
    let m = alu(e, AluOp::Fmin, [y, rcp_st, rcp_st])?;
    let neg = alu(e, AluOp::Flt, [m, zero, zero])?;
    let narc = alu(e, AluOp::Fneg, [arc, arc, arc])?;
    alu(e, AluOp::Bcsel, [neg, narc, arc])
}

/// Mesa's `build_asin`, in our opcodes.
///
/// `piecewise` selects the Taylor series below |x| = 0.21502245, which asin
/// needs (relative error near zero) and acos does not (pi/2 minus a small
/// number is not small).
fn asin<'a, E: Emit<'a>>(e: &mut E, x: Src, piecewise: bool) -> R<'a, Src> {
    let abs_x = alu(e, AluOp::Fabs, [x, x, x])?;

    let c0 = e.constant([-0.018_729_3; 4])?;
    let c1 = e.constant([0.074_261; 4])?;
    let p = alu(e, AluOp::Ffma, [abs_x, c0, c1])?;
    let c2 = e.constant([-0.212_114_4; 4])?;
    let inner = alu(e, AluOp::Ffma, [abs_x, p, c2])?;
    let c3 = e.constant([1.570_728_8; 4])?;
    let tail = alu(e, AluOp::Ffma, [abs_x, inner, c3])?;

    /* pi/2 - sqrt(1 - |x|) * tail, then the sign of x put back. */
    let one = e.constant([1.0; 4])?;
    /* -|x|. Negation is an instruction here, not a modifier: the IR's `Src`
     * is a value and a swizzle, and SRC_NEG lives one layer down in the
     * selection interface. Writing this as a modifier compiled and computed
     * 1 + |x|. */
    let nax = alu(e, AluOp::Fneg, [abs_x, abs_x, abs_x])?;
    let om = alu(e, AluOp::Fadd, [one, nax, nax])?;
    let root = alu(e, AluOp::Fsqrt, [om, om, om])?;
    let prod = alu(e, AluOp::Fmul, [root, tail, tail])?;
    let nprod = alu(e, AluOp::Fneg, [prod, prod, prod])?;
    let half_pi = e.constant([core::f32::consts::FRAC_PI_2; 4])?;
    let body = alu(e, AluOp::Fadd, [half_pi, nprod, nprod])?;
    let sign = alu(e, AluOp::Fsign, [x, x, x])?;
    let result0 = alu(e, AluOp::Fmul, [body, sign, sign])?;

    if !piecewise {
        return Ok(result0);
    }

    /* x * (1 + x^2/6), the Taylor series, where the polynomial's relative
     * error is worst. */
    let x2 = alu(e, AluOp::Fmul, [x, x, x])?;
    let sixth = e.constant([1.0 / 6.0; 4])?;
    let t = alu(e, AluOp::Ffma, [x2, sixth, one])?;
    let result1 = alu(e, AluOp::Fmul, [x, t, t])?;

    let cut = e.constant([0.215_022_45; 4])?;
    let small = alu(e, AluOp::Flt, [abs_x, cut, cut])?;
    alu(e, AluOp::Bcsel, [small, result1, result0])
}

/// `Fdot2`, `Fdot3` or `Fdot4`, by the operands' width.
fn dot_of<'a, E: Emit<'a>>(e: &mut E, name: &'a str, a: Src, b: Src, comps: u8) -> R<'a, Src> {
    let op = match comps {
        /* A one-component dot is a multiply. Emitting Fdot2 for it would read
         * a lane the value does not have. */
        1 => AluOp::Fmul,
        2 => AluOp::Fdot2,
        3 => AluOp::Fdot3,
        4 => AluOp::Fdot4,
        _ => return Err(Error::new(0, name, "a dot product of more than four components")),
    };
    alu(e, op, [a, b, b])
}

/// Emit one ALU instruction.
fn alu<'a, E: Emit<'a>>(e: &mut E, op: AluOp, srcs: [Src; 3]) -> R<'a, Src> {
    e.emit(Op::Alu(op, srcs))
}

/// Broadcast a value's first component across all four lanes, for a scalar
/// that has to meet a vector.
fn splat(s: Src) -> Src {
    Src { val: s.val, swizzle: [s.swizzle[0]; 4] }
}

/// Re-swizzle a value: `pick` names which of ITS components each lane reads.
fn sw(s: Src, pick: [u8; 4]) -> Src {
    let mut out = [0u8; 4];
    for (o, p) in out.iter_mut().zip(pick) {
        *o = s.swizzle[p as usize & 3];
    }
    Src { val: s.val, swizzle: out }
}

fn componentwise<'a, E: Emit<'a>>(
    e: &mut E,
    b: &Componentwise,
    name: &'a str,
    args: &[Operand<'a>],
    want: u8,
) -> R<'a, Src> {
    /* ES 3.00's integer forms (section 8.3): `abs` and `sign` on int, `min`
     * and `max` on int and uint. */
    if let Some(k) = integer_kind(args) {
        return integer_common(e, name, k, args);
    }
    if args.len() != b.args {
        /* The type checker has the signatures and catches this first; the
         * check is here so that the lowering cannot be made to read past the
         * end of the slice by a future caller that does not. */
        return Err(Error::new(0, name, "this built-in takes a different number of arguments"));
    }
    let mut srcs = [Src { val: super::ir::ValueId(0), swizzle: SWIZZLE_IDENTITY }; 3];
    for (slot, a) in srcs.iter_mut().zip(args) {
        let (src, comps) = a.value(name)?;
        if comps != want && comps != 1 {
            /* `min(vec3, float)` is legal and `min(vec3, vec2)` is not; the
             * type checker decides which, and this is the guard that keeps a
             * wrong one from being lowered into a silently wrong width. */
            return Err(Error::new(0, name, "this built-in's argument width is not lowered yet"));
        }
        *slot = src;
    }
    e.emit(Op::Alu(b.op, srcs))
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(super) const S2D: SamplerType = SamplerType::new(SamplerDim::D2, SamplerRet::Float, false);

    /// The smallest possible [`Emit`]: instructions in declaration order,
    /// which is what `ValueId` means. A fixed array rather than a `Vec`
    /// because the crate is `no_std` and this file must stay buildable for
    /// the target, tests and all.
    struct Log {
        /* Sized for the longest expansion, which is the two-argument `atan`
         * at forty-nine instructions -- it contains the one-argument one. A
         * smaller log fails as "the log is full", which reads like a broken
         * test rather than the expansion it is. */
        ops: [Op; 64],
        n: usize,
    }

    impl Log {
        fn new() -> Log {
            Log { ops: [Op::Discard; 64], n: 0 }
        }
        fn ops(&self) -> &[Op] {
            &self.ops[..self.n]
        }
    }

    impl<'a> Emit<'a> for Log {
        fn emit(&mut self, op: Op) -> R<'a, Src> {
            assert!(self.n < self.ops.len(), "the test's instruction log is full");
            self.ops[self.n] = op;
            self.n += 1;
            Ok(Src::id(ValueId(self.n as u32 - 1)))
        }
        fn constant(&mut self, lanes: [f32; 4]) -> R<'a, Src> {
            self.emit(Op::ConstF32([lanes[0].to_bits(), lanes[1].to_bits(), lanes[2].to_bits(), lanes[3].to_bits()]))
        }
    }

    fn v(id: u32, comps: u8) -> Operand<'static> {
        Operand::Value { src: Src::id(ValueId(id)), comps, ty: BaseType::Vec(comps) }
    }

    #[test]
    fn a_componentwise_builtin_is_one_opcode() {
        let mut log = Log::new();
        let r = lower_call(&mut log, "max", &[v(0, 3), v(1, 3)], 3).expect("max is a built-in");
        r.expect("lowers");
        assert_eq!(log.ops().len(), 1);
        let Op::Alu(op, srcs) = log.ops()[0] else { panic!("expected one ALU instruction, got {:?}", log.ops()[0]) };
        assert_eq!(op, AluOp::Fmax);
        assert_eq!(srcs[0].val, ValueId(0));
        assert_eq!(srcs[1].val, ValueId(1));
    }

    /// A scalar second argument is legal (`min(vec3, float)`, section 8.3)
    /// and reaches the same opcode: the back end splats it.
    #[test]
    fn a_scalar_argument_against_a_vector_is_allowed() {
        let mut log = Log::new();
        lower_call(&mut log, "min", &[v(0, 3), v(1, 1)], 3)
            .expect("min is a built-in")
            .expect("a scalar second argument lowers");
        assert_eq!(log.ops().len(), 1);
    }

    #[test]
    fn a_name_that_is_not_a_builtin_is_not_claimed() {
        let mut log = Log::new();
        assert!(lower_call(&mut log, "vec3", &[v(0, 1)], 3).is_none(), "a constructor is the caller's job");
        assert!(lower_call(&mut log, "my_helper", &[], 1).is_none(), "and so is a user function");
    }

    fn samp(binding: u8) -> Operand<'static> {
        Operand::Sampler { binding, ty: S2D }
    }

    /// A texture look-up is ONE IR node, and the sampler in it is a binding
    /// rather than a value: it never reaches a register.
    #[test]
    fn a_texture_lookup_is_one_node_naming_its_binding() {
        let mut log = Log::new();
        lower_call(&mut log, "texture2D", &[samp(3), v(0, 2)], 4).expect("a built-in").expect("lowers");
        assert_eq!(log.ops().len(), 1);
        let op = log.ops()[0];
        let Op::Tex { sampler, .. } = op else { panic!("{op:?}") };
        let (kind, extra) = op.es100_tex().expect("an ES 1.00 look-up");
        assert_eq!(kind, TexKind::Tex2D);
        assert_eq!(sampler, 3);
        assert!(extra.is_none());
    }

    /// The bias form is a DIFFERENT shape, not an optional extra on the same
    /// one: the hardware has a separate instruction (`texldb`) for it.
    #[test]
    fn texture2d_with_a_bias_is_a_different_kind() {
        let mut log = Log::new();
        lower_call(&mut log, "texture2D", &[samp(0), v(0, 2), v(1, 1)], 4).expect("a built-in").expect("lowers");
        let (kind, extra) = log.ops()[0].es100_tex().expect("an ES 1.00 look-up");
        assert_eq!(kind, TexKind::Tex2DBias);
        assert!(extra.is_some());
    }

    /// The projective look-up is the one coordinate with two right widths: it
    /// takes a vec3 or a vec4 and divides by the last component.
    #[test]
    fn the_projective_lookup_takes_three_or_four_components() {
        for comps in [3u8, 4] {
            let mut log = Log::new();
            lower_call(&mut log, "texture2DProj", &[samp(0), v(0, comps)], 4)
                .expect("a built-in")
                .unwrap_or_else(|e| panic!("vec{comps}: {}", e.msg));
        }
        let mut log = Log::new();
        let e = lower_call(&mut log, "texture2DProj", &[samp(0), v(0, 2)], 4)
            .expect("a built-in")
            .expect_err("a vec2 projector");
        assert!(e.msg.contains("components"), "{}", e.msg);
    }

    /// A sampler that arrived as a value means the front end turned a sampler
    /// into a number somewhere upstream. Refused rather than papered over,
    /// because sampling unit 0 by accident is a picture and not a failure.
    #[test]
    fn a_texture_lookup_without_a_sampler_is_refused() {
        let mut log = Log::new();
        let e =
            lower_call(&mut log, "texture2D", &[v(0, 1), v(1, 2)], 4).expect("a built-in").expect_err("not a sampler");
        assert!(e.msg.contains("sampler"), "{}", e.msg);
    }

    /// `textureCubeLod(s, v3, lod)` is IR v1's cube look-up with an explicit
    /// level (dEQP-GLES2 shaders.random.texture.vertex.*, 54 cases), the
    /// shape `textureLod(samplerCube, ..)` takes in ES 3.00.
    ///
    /// Revert check: drop the `textureCubeLod` row of `TEXTURES` and the
    /// call is not claimed at all.
    #[test]
    fn the_cube_lod_lookup_is_ir_v1s_cube_tex_with_a_level() {
        let mut log = Log::new();
        lower_call(&mut log, "textureCubeLod", &[samp(0), v(0, 3), v(1, 1)], 4).expect("claimed").expect("lowers");
        let ops = log.ops();
        assert!(
            matches!(
                ops[..],
                [Op::Tex {
                    dim: TexDim::Cube,
                    shadow: false,
                    proj: false,
                    ret: ScalarType::F32,
                    lod: TexLod::Lod(l),
                    offset: None,
                    ..
                }] if l.swizzle == [0; 4]
            ),
            "{ops:?}"
        );
    }

    /// `texture2DProjLod(s, v3, lod)`: (s, t) divided by the last component,
    /// then the levelled 2D look-up (section 8.7), as the biased projective
    /// form is the divide and then the biased one.
    #[test]
    fn the_projective_lod_lookup_divides_then_levels() {
        let mut log = Log::new();
        lower_call(&mut log, "texture2DProjLod", &[samp(0), v(0, 3), v(1, 1)], 4).expect("claimed").expect("lowers");
        let ops = log.ops();
        assert!(matches!(ops[0], Op::Alu(AluOp::Frcp, [src, _, _]) if src.swizzle == [2; 4]), "{ops:?}");
        assert!(matches!(ops[1], Op::Alu(AluOp::Fmul, _)), "{ops:?}");
        assert!(matches!(ops[2].es100_tex(), Some((TexKind::Tex2DLod, Some(_)))), "{ops:?}");
    }

    /// The opcodes a sequence produces, in order.
    fn opcodes(log: &Log) -> std::vec::Vec<AluOp> {
        log.ops().iter().filter_map(|o| if let Op::Alu(op, _) = o { Some(*op) } else { None }).collect()
    }

    fn run(name: &'static str, args: &[Operand<'static>], want: u8) -> Log {
        let mut log = Log::new();
        lower_call(&mut log, name, args, want).expect("a built-in").expect("lowers");
        log
    }

    /// The most valuable expansion in the table: 86 of the 307 sources call
    /// `normalize` and 38 of them call nothing else.
    ///
    /// The dot product must survive as a dot product. The IR keeps Fdot2/3/4
    /// BECAUSE the GC7000 has DP2/DP3/DP4, so a multiply chain here would be
    /// work the back end has to undo -- and the width comes from the
    /// ARGUMENT, which is why `Operand` carries it.
    #[test]
    fn normalize_is_a_dot_a_reciprocal_square_root_and_a_multiply() {
        for (comps, want_dot) in [(2u8, AluOp::Fdot2), (3, AluOp::Fdot3), (4, AluOp::Fdot4)] {
            let log = run("normalize", &[v(0, comps)], comps);
            assert_eq!(opcodes(&log), [want_dot, AluOp::Frsq, AluOp::Fmul], "vec{comps}");
        }
    }

    /// The reducing built-ins: a scalar out of a vector. These are the ones
    /// that could not be written before the type checker, because the
    /// caller's `want` is 1 and says nothing about the argument's width.
    #[test]
    fn the_reducing_builtins_use_the_arguments_width_not_the_results() {
        let log = run("length", &[v(0, 3)], 1);
        assert_eq!(opcodes(&log), [AluOp::Fdot3, AluOp::Fsqrt]);

        let log = run("dot", &[v(0, 2), v(1, 2)], 1);
        assert_eq!(opcodes(&log), [AluOp::Fdot2]);

        let log = run("distance", &[v(0, 4), v(1, 4)], 1);
        assert_eq!(opcodes(&log), [AluOp::Fneg, AluOp::Fadd, AluOp::Fdot4, AluOp::Fsqrt]);
    }

    /// Two values of different widths is a shader error, not a width to pick.
    #[test]
    fn a_dot_of_mismatched_widths_is_refused() {
        let mut log = Log::new();
        let e = lower_call(&mut log, "dot", &[v(0, 3), v(1, 2)], 1)
            .expect("dot is a built-in")
            .expect_err("mismatched widths");
        assert!(e.msg.contains("same width"), "{}", e.msg);
    }

    /// mix is the two-instruction form a + t*(b-a), not the three-instruction
    /// a*(1-t) + b*t, and a scalar `t` against a vector is broadcast.
    #[test]
    fn mix_is_a_negate_an_add_and_one_fma() {
        let log = run("mix", &[v(0, 4), v(1, 4), v(2, 4)], 4);
        assert_eq!(opcodes(&log), [AluOp::Fneg, AluOp::Fadd, AluOp::Ffma]);

        let log = run("mix", &[v(0, 4), v(1, 4), v(2, 1)], 4);
        let Op::Alu(AluOp::Ffma, srcs) = log.ops()[2] else { panic!("the fma") };
        assert!(srcs[0].swizzle.iter().all(|c| *c == srcs[0].swizzle[0]), "a scalar t is broadcast");
    }

    #[test]
    fn clamp_is_a_max_then_a_min_and_pow_is_the_base_two_pair() {
        assert_eq!(opcodes(&run("clamp", &[v(0, 4), v(1, 4), v(2, 4)], 4)), [AluOp::Fmax, AluOp::Fmin]);
        assert_eq!(opcodes(&run("pow", &[v(0, 4), v(1, 4)], 4)), [AluOp::Flog2, AluOp::Fmul, AluOp::Fexp2]);
    }

    /// cross is three components exactly; GLSL has no other width.
    #[test]
    fn cross_is_a_vec3_only() {
        assert_eq!(opcodes(&run("cross", &[v(0, 3), v(1, 3)], 3)), [AluOp::Fmul, AluOp::Fneg, AluOp::Ffma]);
        let mut log = Log::new();
        let e = lower_call(&mut log, "cross", &[v(0, 4), v(1, 4)], 4)
            .expect("cross is a built-in")
            .expect_err("a vec4 cross");
        assert!(e.msg.contains("vec3"), "{}", e.msg);
    }

    /// Every name this file claims must have an arm: a name in SEQUENCES with
    /// no lowering would be refused as "named as a sequence", which is a bug
    /// report about this file rather than about the shader.
    #[test]
    fn every_claimed_sequence_has_a_lowering() {
        for name in SEQUENCES {
            let mut log = Log::new();
            let args = [v(0, 3), v(1, 3), v(2, 3)];
            let r = lower_call(&mut log, name, &args, 3).expect("claimed");
            if let Err(e) = r {
                assert!(!e.msg.contains("named as a sequence"), "{name} is claimed but not lowered");
            }
        }
    }

    /// Evaluate what an expansion emitted, as scalars, so that a numerical
    /// built-in can be checked against a number rather than against a shape.
    ///
    /// Booleans are 1.0 and 0.0 here. That is the evaluator's convention, not
    /// the machine's -- the GC7000 back end uses all-ones -- and it is sound
    /// because every consumer of a bool in these expansions is `Bcsel`, which
    /// this evaluates as "not zero", or `B2f`, which both conventions define
    /// as 1.0.
    fn eval(log: &Log, inputs: &[(u32, f32)]) -> f32 {
        let mut v = std::collections::BTreeMap::new();
        for (id, x) in inputs {
            v.insert(*id, *x);
        }
        let mut last = 0.0f32;
        for (i, op) in log.ops().iter().enumerate() {
            let r = match *op {
                Op::ConstF32(bits) => f32::from_bits(bits[0]),
                Op::Alu(o, srcs) => {
                    let a = |n: usize| *v.get(&srcs[n].val.0).unwrap_or_else(|| panic!("value {:?}", srcs[n].val));
                    let b = |x: bool| if x { 1.0f32 } else { 0.0 };
                    match o {
                        AluOp::Fabs => a(0).abs(),
                        AluOp::Fneg => -a(0),
                        AluOp::Fsign => {
                            if a(0) > 0.0 {
                                1.0
                            } else if a(0) < 0.0 {
                                -1.0
                            } else {
                                0.0
                            }
                        }
                        AluOp::Frcp => 1.0 / a(0),
                        AluOp::Frsq => 1.0 / a(0).sqrt(),
                        AluOp::Fsqrt => a(0).sqrt(),
                        AluOp::Fsin => a(0).sin(),
                        AluOp::Fcos => a(0).cos(),
                        AluOp::Fexp2 => a(0).exp2(),
                        AluOp::Flog2 => a(0).log2(),
                        AluOp::Ffloor => a(0).floor(),
                        AluOp::Fceil => a(0).ceil(),
                        AluOp::Ffract => a(0) - a(0).floor(),
                        AluOp::Fmul => a(0) * a(1),
                        AluOp::Fadd => a(0) + a(1),
                        AluOp::Ffma => a(0) * a(1) + a(2),
                        AluOp::Fmin => a(0).min(a(1)),
                        AluOp::Fmax => a(0).max(a(1)),
                        AluOp::Fle => b(a(0) <= a(1)),
                        AluOp::Flt => b(a(0) < a(1)),
                        AluOp::Feq => b(a(0) == a(1)),
                        AluOp::Bcsel => {
                            if a(0) != 0.0 {
                                a(1)
                            } else {
                                a(2)
                            }
                        }
                        AluOp::B2f => a(0),
                        other => panic!("the evaluator has no case for {other:?}"),
                    }
                }
                ref other => panic!("the evaluator has no case for {other:?}"),
            };
            v.insert(i as u32, r);
            last = r;
        }
        last
    }

    /// atan is the last built-in the ES 1.00 half of the shipped table needs,
    /// and this checks the NUMBER rather than the shape: the expansion is
    /// evaluated over a grid and compared against the host's atan2.
    ///
    /// The grid deliberately includes both signs of both arguments and the
    /// axes, because the quadrant handling is the part of atan2 that a first
    /// attempt gets wrong while still looking plausible in one quadrant.
    #[test]
    fn atan2_agrees_with_the_host_over_every_quadrant() {
        let vals = [-8.0f32, -1.5, -1.0, -0.3, 0.0, 0.3, 1.0, 1.5, 8.0];
        let mut worst = 0.0f32;
        let mut worst_at = (0.0f32, 0.0f32);
        for &y in &vals {
            for &x in &vals {
                if y == 0.0 && x == 0.0 {
                    /* GLSL leaves atan2(0, 0) undefined, and says so. */
                    continue;
                }
                let mut log = Log::new();
                lower_call(&mut log, "atan", &[v(100, 1), v(101, 1)], 1).expect("atan is a built-in").expect("lowers");
                let got = eval(&log, &[(100, y), (101, x)]);
                let want = y.atan2(x);
                let err = (got - want).abs();
                if err > worst {
                    worst = err;
                    worst_at = (y, x);
                }
            }
        }
        assert!(worst < 1e-5, "atan2 is off by {worst} at y={}, x={}", worst_at.0, worst_at.1);
    }

    /// The one-argument form, over a range that crosses the |x| = 1 point
    /// where the range reduction switches to 1/x.
    #[test]
    fn atan_agrees_with_the_host_across_the_range_reduction() {
        let mut worst = 0.0f32;
        for i in -60..=60 {
            let x = i as f32 * 0.25;
            let mut log = Log::new();
            lower_call(&mut log, "atan", &[v(100, 1)], 1).expect("a built-in").expect("lowers");
            let got = eval(&log, &[(100, x)]);
            worst = worst.max((got - x.atan()).abs());
        }
        assert!(worst < 1e-5, "atan is off by {worst}");
    }

    /// Both forms are claimed, and the two-argument one is what the table
    /// actually calls: of the 61 atan calls in the 142 ES 1.00 sources, all
    /// 61 are atan(y, x).
    #[test]
    fn atan_takes_one_or_two_arguments() {
        let mut log = Log::new();
        assert!(lower_call(&mut log, "atan", &[v(0, 1)], 1).expect("a built-in").is_ok());
        let mut log = Log::new();
        assert!(lower_call(&mut log, "atan", &[v(0, 1), v(1, 1)], 1).expect("a built-in").is_ok());
        let mut log = Log::new();
        let e =
            lower_call(&mut log, "atan", &[v(0, 3), v(1, 2)], 3).expect("a built-in").expect_err("mismatched widths");
        assert!(e.msg.contains("same width"), "{}", e.msg);
    }

    /// asin and acos against the host, across the piecewise boundary at
    /// |x| = 0.21502245 and out to the endpoints where the derivative is
    /// infinite.
    ///
    /// This is the test that caught a real defect: the first version wrote
    /// `-|x|` as a source modifier, which does not exist at the IR level, so
    /// it computed sqrt(1 + |x|) instead of sqrt(1 - |x|). It compiled, and
    /// every shape assertion would have passed.
    #[test]
    fn asin_and_acos_agree_with_the_host() {
        let mut worst = (0.0f32, 0.0f32, "");
        for i in -100..=100 {
            let x = i as f32 / 100.0;
            for (name, want) in [("asin", x.asin()), ("acos", x.acos())] {
                let log = run(name, &[v(100, 1)], 1);
                let got = eval(&log, &[(100, x)]);
                let err = (got - want).abs();
                if err > worst.0 {
                    worst = (err, x, name);
                }
            }
        }
        /* The polynomial's own accuracy, not ours to improve: Mesa ships this
         * one and dEQP accepts it. The bound is what it measures at, with
         * room, so a REGRESSION fails and the approximation itself does not. */
        assert!(worst.0 < 1e-3, "{} is off by {} at x = {}", worst.2, worst.0, worst.1);
    }

    /// tan is sin over cos, checked away from the poles, where GLSL leaves it
    /// undefined and nothing is owed.
    #[test]
    fn tan_agrees_with_the_host_away_from_its_poles() {
        let mut worst = 0.0f32;
        for i in -60..=60 {
            let x = i as f32 / 50.0;
            if (x.cos()).abs() < 0.2 {
                continue;
            }
            let got = eval(&run("tan", &[v(100, 1)], 1), &[(100, x)]);
            worst = worst.max((got - x.tan()).abs());
        }
        assert!(worst < 1e-4, "tan is off by {worst}");
    }

    /// faceforward flips N against the incident direction, and the test uses
    /// scalars so the evaluator can check the VALUE rather than the shape.
    #[test]
    fn faceforward_flips_against_the_incident_direction() {
        for (nref, want) in [(-1.0f32, 1.0f32), (1.0, -1.0)] {
            let log = run("faceforward", &[v(100, 1), v(101, 1), v(102, 1)], 1);
            let got = eval(&log, &[(100, 1.0), (101, 1.0), (102, nref)]);
            assert_eq!(got, want, "dot(Nref, I) = {nref} keeps N only when it is negative");
        }
    }

    /// refract returns zero under total internal reflection, and the
    /// specification's value otherwise. Both arms are evaluated and selected
    /// between -- the NaN from sqrt of a negative k must be discarded by the
    /// select, not avoided by a branch.
    #[test]
    fn refract_returns_zero_under_total_internal_reflection() {
        /* I = 1, N = 1 (so dot = 1), eta = 2: k = 1 - 4*(1-1) = 1, no TIR. */
        let got = eval(&run("refract", &[v(100, 1), v(101, 1), v(102, 1)], 1), &[(100, 1.0), (101, 1.0), (102, 2.0)]);
        assert_eq!(got, 2.0 * 1.0 - (2.0 * 1.0 + 1.0) * 1.0, "eta*I - (eta*dot + sqrt k)*N");

        /* dot = 0 and eta = 2: k = 1 - 4 = -3 < 0, so zero. */
        let got = eval(&run("refract", &[v(100, 1), v(101, 1), v(102, 1)], 1), &[(100, 1.0), (101, 0.0), (102, 2.0)]);
        assert_eq!(got, 0.0, "total internal reflection gives zero, not a NaN");
    }

    /// The relational group: greaterThan and lessThanEqual are the operand
    /// SWAP, because this hardware has no .gt and no .le. A lowering that
    /// changed the condition instead would invert the comparison.
    #[test]
    fn the_relational_builtins_swap_rather_than_invert() {
        for (name, cond, swapped) in [
            ("lessThan", AluOp::Flt, false),
            ("greaterThan", AluOp::Flt, true),
            ("lessThanEqual", AluOp::Fle, false),
            ("greaterThanEqual", AluOp::Fle, true),
            ("equal", AluOp::Feq, false),
            ("notEqual", AluOp::Fne, false),
        ] {
            let log = run(name, &[v(0, 3), v(1, 3)], 3);
            assert_eq!(opcodes(&log), [cond], "{name}");
            let Op::Alu(_, srcs) = log.ops()[0] else { panic!() };
            let first = if swapped { ValueId(1) } else { ValueId(0) };
            assert_eq!(srcs[0].val, first, "{name} puts the right side first only when it swaps");
        }
    }

    /// `texture2DProj` divides by the coordinate's LAST component, and IR v1
    /// puts the divisor in lane 3 whatever the coordinate's width: a vec4
    /// has it there already, a vec3's z is repeated into it. (Until IR v1
    /// the divisor was lane 2 and a vec4 arrived as (x, y, w, w); pinned
    /// here so neither form can drift back to dividing by the wrong lane.)
    #[test]
    fn a_projective_look_up_divides_by_the_last_component() {
        let tex = |n: u8| {
            let mut log = Log::new();
            let r = lower_call(&mut log, "texture2DProj", &[Operand::Sampler { binding: 0, ty: S2D }, v(0, n)], 4)
                .expect("a built-in");
            r.expect("lowers");
            let op = log.ops().last().copied().expect("expected a look-up");
            let Op::Tex { coord, .. } = op else { panic!("expected a look-up") };
            assert_eq!(op.es100_tex().map(|t| t.0), Some(TexKind::Tex2DProj));
            coord.swizzle
        };
        assert_eq!(tex(3), [0, 1, 2, 2], "vec3: z is the divisor, in lane 3");
        assert_eq!(tex(4), [0, 1, 2, 3], "vec4: w is the divisor, in lane 3");
    }

    /// A reduction folds all four lanes, so a narrower vector's spare
    /// lanes must repeat its last real one: a `bvec2` reduced over lanes
    /// the value does not have reads what the register held there.
    #[test]
    fn a_reduction_over_a_narrow_vector_repeats_its_last_lane() {
        for (name, op) in [("any", AluOp::Bany), ("all", AluOp::Ball)] {
            for (n, want) in [(2u8, [0u8, 1, 1, 1]), (3, [0, 1, 2, 2]), (4, [0, 1, 2, 3])] {
                let log = run(name, &[v(0, n)], 1);
                let Op::Alu(got, srcs) = log.ops()[0] else { panic!("expected an ALU instruction") };
                assert_eq!(got, op);
                assert_eq!(srcs[0].swizzle, want, "{name} of a {n}-wide vector");
            }
        }
    }

    /// any/all/not stay as IR opcodes: the back end folds the reduction over
    /// the lanes with `or`/`and`, and expanding it here would be the width
    /// unrolled in the wrong file.
    #[test]
    fn the_boolean_reductions_stay_opcodes() {
        assert_eq!(opcodes(&run("any", &[v(0, 3)], 1)), [AluOp::Bany]);
        assert_eq!(opcodes(&run("all", &[v(0, 3)], 1)), [AluOp::Ball]);
        assert_eq!(opcodes(&run("not", &[v(0, 3)], 3)), [AluOp::Bnot]);
    }

    /// fwidth is |dFdx| + |dFdy|, and dFdx/dFdy are one opcode each.
    #[test]
    fn the_derivative_builtins() {
        assert_eq!(opcodes(&run("dFdx", &[v(0, 4)], 4)), [AluOp::Fddx]);
        assert_eq!(opcodes(&run("dFdy", &[v(0, 4)], 4)), [AluOp::Fddy]);
        assert_eq!(
            opcodes(&run("fwidth", &[v(0, 4)], 4)),
            [AluOp::Fddx, AluOp::Fddy, AluOp::Fabs, AluOp::Fabs, AluOp::Fadd]
        );
    }

    #[test]
    fn a_sampler_used_as_a_value_says_so() {
        let mut log = Log::new();
        let e = lower_call(&mut log, "abs", &[Operand::Sampler { binding: 0, ty: S2D }], 1)
            .expect("abs is a built-in")
            .expect_err("a sampler is not a number");
        assert_eq!(e.msg, "a sampler is not a value here");
    }
}

#[cfg(test)]
mod bias_form_tests {
    use super::*;
    use crate::shaderc::online::ir::ValueId;

    struct Log(std::vec::Vec<Op>);
    impl<'a> Emit<'a> for Log {
        fn emit(&mut self, op: Op) -> R<'a, Src> {
            self.0.push(op);
            Ok(Src::id(ValueId(self.0.len() as u32 - 1)))
        }
        fn constant(&mut self, lanes: [f32; 4]) -> R<'a, Src> {
            self.emit(Op::ConstF32(lanes.map(f32::to_bits)))
        }
    }

    /// `texture2DProj(s, v3, bias)` is the biased 2D look-up of (s, t)
    /// divided by the last component (section 8.7); `textureCube(s, v, bias)`
    /// is a gap worded as one. Both were refused as "this built-in takes a
    /// different number of arguments", a verdict on legal shaders (dEQP
    /// shaders.random.all_features.fragment.10, .16, .17, .42, .2, .52).
    #[test]
    fn the_bias_forms() {
        let s = Operand::Sampler { binding: 0, ty: super::tests::S2D };
        let v3 =
            Operand::Value { src: Src::id(ValueId(100)), comps: 3, ty: crate::shaderc::online::ast::BaseType::Vec(3) };
        let b =
            Operand::Value { src: Src::id(ValueId(101)), comps: 1, ty: crate::shaderc::online::ast::BaseType::Float };
        let mut log = Log(std::vec::Vec::new());
        texture(&mut log, "texture2DProj", &[s, v3, b]).expect("a legal overload");
        assert!(
            matches!(log.0[0], Op::Alu(AluOp::Frcp, [src, _, _]) if src.swizzle == [2; 4]),
            "1 / coord.p: {:?}",
            log.0
        );
        assert!(matches!(log.0[1], Op::Alu(AluOp::Fmul, _)), "(s, t) scaled: {:?}", log.0);
        assert!(
            matches!(log.0[2].es100_tex(), Some((TexKind::Tex2DBias, Some(_)))),
            "then the biased look-up: {:?}",
            log.0
        );
        /* textureCube with a bias (dEQP-GLES2 random.all_features.fragment.*,
         * 44 cases): IR v1's cube look-up with a bias. */
        let mut log = Log(std::vec::Vec::new());
        texture(&mut log, "textureCube", &[s, v3, b]).expect("lowers");
        assert!(
            matches!(log.0[..], [Op::Tex { dim: TexDim::Cube, lod: TexLod::Bias(_), proj: false, .. }]),
            "{:?}",
            log.0
        );
    }
}
