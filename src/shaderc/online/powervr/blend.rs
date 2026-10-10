// SPDX-License-Identifier: Apache-2.0 AND MIT
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
// SPDX-FileCopyrightText: 2024 Imagination Technologies Ltd.
// SPDX-FileCopyrightText: 2019 Alyssa Rosenzweig
// SPDX-FileCopyrightText: 2019-2021 Collabora, Ltd.
// SPDX-FileCopyrightText: 2025 Google LLC
// Derived from Mesa 26.2.1 src/compiler/nir/nir_lower_blend.c and the pco
// compiler's blend variants (src/imagination/pco/) (MIT).
// The upstream part stays under its licence (LICENSES/MIT.txt); our
// changes are Apache-2.0. See NOTICE.
//! Blending, lowered into the fragment shader.
//!
//! The PowerVR blends in the USC: a draw with blending runs a variant of its
//! fragment shader that reads the colour already in the pixel, combines it
//! with the shader's own and writes the result (the record's `uses_fbfetch`
//! makes the driver run the pass that allows the read). pco builds these
//! variants from Mesa's `nir_lower_blend`; the forms here are its blend
//! variants' own (pco_shc build-new, `gl_FragColor = u` under
//! `src_alpha,inv_src_alpha`, 2026-09-23):
//!
//! ```text
//!   s' = fadd.sat(s, 0)                  the source clamped, as for UNORM
//!   p  = mbyp pixout0  (olchk)           the pixel's colour
//!   d  = unpck.u8888.scale p  (rpt 4)    as four floats
//!   per channel: s'*Fs (op) d*Fd         fmul/fadd/fmad, or min/max
//!   pck.u8888 -> pixout0                 as the plain shader ends
//! ```
//!
//! The key is [`crate::shaderc::shc_blend_key`]'s: the factors and equations
//! for colour and alpha, the channels `glColorMask` disabled, and the render
//! target class. A float class (RGBA16F in two pixel output dwords, R16F,
//! R32F) is read and written in its own format and its source is NOT
//! clamped, as pco's float variants do (pco_shc build-new,
//! `src_alpha,inv_src_alpha@f16x4`, `@f16`, `@f32`); a variant that does not
//! read the pixel (a plain replace) reads nothing, as pco's do not.
//!
//! An sRGB target ([`SHC_RT_SRGB`]) is read and written as RGBA16F is: the
//! accumulation holds linear halves (the frame load decodes, the PBE encodes
//! on store), so the blend is in linear space with no conversion of its own
//! -- `nir_lower_blend` has none either (docs/es3-srgb.md 4.2). The target is
//! unsigned normalised, so the source is clamped as for RGBA8, and so is the
//! blended colour (pco's `nir_fsat` before the F16 pack of an sRGB format,
//! pco_nir_pvfio.c:97-101): the next blend reads it back from the
//! accumulation, where a value past 1 would otherwise survive.

use super::assemble::{build_group, build_pixout_read_from, build_unpck, BuildError};
use super::decode::Group;
use super::sel::{Bank, Mods, OpType, SelDst, SelGroup, SelOp, SelSrc};
use crate::shaderc::online::ir::AluOp;
use crate::shaderc::*;

/// One decoded blend key.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Blend {
    /// Source and destination factor, and equation: `[rgb, alpha]`.
    pub src: [u32; 2],
    pub dst: [u32; 2],
    pub eq: [u32; 2],
    /// Bit k set: channel k is not written (it keeps the pixel's value).
    pub masked: u32,
    /// The render target class: SHC_RT_RGBA8, _F16X4, _F16, _F32 or _SRGB.
    pub class: u32,
}

/// Special registers holding the constants the arithmetic needs (pco's).
const SR_ZERO: u16 = 0;
const SR_ONE: u16 = 64;

impl Blend {
    /// The blend a key asks for; `Ok(None)` for no blend at all.
    pub fn from_key(key: u32) -> Result<Option<Blend>, &'static str> {
        if key == 0 {
            return Ok(None);
        }
        let f = |sh: u32| (key >> sh) & 0x1f;
        let b = Blend {
            src: [f(0), f(10)],
            dst: [f(5), f(15)],
            eq: [(key >> 20) & 7, (key >> 23) & 7],
            masked: shc_blend_key_masked(key),
            class: shc_blend_key_class(key),
        };
        for x in b.src.iter().chain(b.dst.iter()) {
            if !matches!(*x & !SHC_BF_INVERT, 1..=8) {
                return Err("a blend factor outside shc_blend_factor");
            }
        }
        if b.eq.iter().any(|e| *e > SHC_BLEND_MAX) {
            return Err("a blend equation outside shc_blend_func");
        }
        Ok(Some(b))
    }

    /// The channels the target has: all four, or red alone.
    pub fn channels(&self) -> u16 {
        if matches!(self.class, SHC_RT_F16 | SHC_RT_F32) {
            1
        } else {
            4
        }
    }

    /// Does the result depend on the colour already in the pixel? Not for
    /// a plain replace, which then reads nothing.
    pub fn reads_pixel(&self) -> bool {
        (0..self.channels()).any(|k| {
            let ch = usize::from(k == 3);
            let reads =
                |f: u32| matches!(f & !SHC_BF_INVERT, SHC_BF_DST_ALPHA | SHC_BF_DST_COLOR | SHC_BF_SRC_ALPHA_SATURATE);
            self.masked & (1 << k) != 0
                || matches!(self.eq[ch], SHC_BLEND_MIN | SHC_BLEND_MAX)
                || self.dst[ch] != SHC_BF_ZERO
                || reads(self.src[ch])
        })
    }

    /// Is the target unsigned normalised (RGBA8, sRGB), so that the source
    /// is clamped to [0, 1] before the blend (ES 3.0.6 4.1.7)?
    pub fn unorm(&self) -> bool {
        matches!(self.class, SHC_RT_RGBA8 | SHC_RT_SRGB)
    }

    /// Is the pixel two dwords of four halves (RGBA16F, and the sRGB
    /// target's linear accumulation)?
    pub fn wide(&self) -> bool {
        matches!(self.class, SHC_RT_F16X4 | SHC_RT_SRGB)
    }

    /// Is the blended colour clamped before it is packed? An sRGB target's
    /// halves are not clamped by a pack, unlike RGBA8's bytes; a plain
    /// replace writes the clamped source, which needs nothing more.
    pub fn clamps_result(&self) -> bool {
        let replace = self.src == [SHC_BF_ONE; 2] && self.dst == [SHC_BF_ZERO; 2] && self.eq == [SHC_BLEND_ADD; 2];
        self.class == SHC_RT_SRGB && !replace
    }

    /// Does any factor read the blend colour (`glBlendColor`)?
    pub fn reads_constants(&self) -> bool {
        self.src
            .iter()
            .chain(self.dst.iter())
            .any(|f| matches!(f & !SHC_BF_INVERT, SHC_BF_CONST_COLOR | SHC_BF_CONST_ALPHA))
    }
}

/// Where the tail's values live, in temporaries the allocator did not use.
#[derive(Clone, Copy, Debug)]
pub struct Regs {
    /// The shader's colour in, and the blended colour out: four
    /// consecutive temporaries the pack reads.
    pub colour: u16,
    /// Scratch from here: the clamped source (4), the pixel dword (1), the
    /// pixel as floats (4), the constants (4), and per channel up to 4.
    pub scratch: u16,
    /// The blend colour's first shared register, when a factor reads it.
    pub consts: Option<u16>,
    /// A shader that discards: the temporary holding the control word's
    /// alpha-to-one flag, applied to the blended alpha last (pco's
    /// `pco_nir_pfo` blends first).
    pub a2one: Option<u16>,
}

impl Regs {
    fn s(&self, k: u16) -> u16 {
        self.scratch + k
    }
    fn p(&self) -> u16 {
        self.scratch + 4
    }
    fn d(&self, k: u16) -> u16 {
        self.scratch + 5 + k
    }
    fn c(&self, k: u16) -> u16 {
        self.scratch + 9 + k
    }
    fn t(&self, k: u16) -> u16 {
        self.scratch + 13 + k
    }
    /// An RGBA16F pixel's second dword.
    fn p1(&self) -> u16 {
        self.scratch + 17
    }
    /// How many temporaries past `scratch` the tail uses.
    pub const SCRATCH: u16 = 18;
}

fn temp(r: u16) -> SelSrc {
    SelSrc::Fixed { bank: Bank::Temp, reg: r, mods: Mods::default() }
}
fn special(r: u16) -> SelSrc {
    SelSrc::Fixed { bank: Bank::Special, reg: r, mods: Mods::default() }
}
fn neg(s: SelSrc) -> SelSrc {
    match s {
        SelSrc::Fixed { bank, reg, mods } => SelSrc::Fixed { bank, reg, mods: Mods { neg: !mods.neg, ..mods } },
        other => other,
    }
}
fn alu(op: AluOp, dst: u16, a: SelSrc, b: Option<SelSrc>) -> SelGroup {
    SelGroup {
        op: SelOp::Alu(op),
        ty: OpType::F32,
        dst: Some(SelDst::Fixed { bank: Bank::Temp, reg: dst }),
        srcs: [Some(a), b, None],
    }
}
fn mov(dst: u16, a: SelSrc) -> SelGroup {
    SelGroup {
        op: SelOp::Copy,
        ty: OpType::F32,
        dst: Some(SelDst::Fixed { bank: Bank::Temp, reg: dst }),
        srcs: [Some(a), None, None],
    }
}

/// A step of the tail: an ordinary group, or one of the two the tail builds
/// itself.
#[derive(Clone, Copy)]
enum Step {
    Group(SelGroup),
    Sat(u16, u16),
    /// Pixel output register, destination.
    PixRead(u16, u16),
    /// Format, scale, repeat, destination, source.
    Unpack(&'static str, u32, u32, u16, u16),
}

/// The groups one channel's blend takes, into `out`; returns how many.
/// `fuse`: a product term and the other term as one `fmad` ([`fused`]);
/// the tail always fuses, and only the test that checks the fused form
/// against the separate terms builds a channel without it.
fn channel(b: &Blend, r: &Regs, k: u16, fuse: bool, out: &mut [Step; 8]) -> usize {
    let mut n = 0;
    let mut push = |s: Step| {
        out[n] = s;
        n += 1;
    };
    let dst = r.colour + k;
    if b.masked & (1 << k) != 0 {
        push(Step::Group(mov(dst, temp(r.d(k)))));
        return n;
    }
    let ch = usize::from(k == 3);
    let eq = b.eq[ch];
    let (s, d) = (temp(r.s(k)), temp(r.d(k)));
    if eq == SHC_BLEND_MIN || eq == SHC_BLEND_MAX {
        /* GL ignores the factors for MIN and MAX. */
        let op = if eq == SHC_BLEND_MIN { AluOp::Fmin } else { AluOp::Fmax };
        push(Step::Group(alu(op, dst, s, Some(d))));
        return n;
    }
    /* A factor's value, computed into temporary `into` when it is not a
     * register already; `None` for ZERO. A factor that is the same value
     * in every channel and that an earlier channel already computed into
     * `into` is read there (`computed_earlier`): pco's variants compute
     * `1 - As` once for the four channels. */
    let factor = |f: u32, into: u16, push: &mut dyn FnMut(Step)| -> Option<SelSrc> {
        let side = if into == r.t(0) { Side::Src } else { Side::Dst };
        if computed_earlier(b, k, side, f) {
            return Some(temp(into));
        }
        let base = match f & !SHC_BF_INVERT {
            SHC_BF_ONE => special(SR_ONE),
            SHC_BF_SRC_COLOR => temp(r.s(k)),
            SHC_BF_SRC_ALPHA => temp(r.s(3)),
            SHC_BF_DST_ALPHA => temp(r.d(3)),
            SHC_BF_DST_COLOR => temp(r.d(k)),
            SHC_BF_CONST_COLOR => temp(r.c(k)),
            SHC_BF_CONST_ALPHA => temp(r.c(3)),
            /* min(As, 1 - Ad) for colour, 1 for alpha */
            SHC_BF_SRC_ALPHA_SATURATE if k == 3 => special(SR_ONE),
            SHC_BF_SRC_ALPHA_SATURATE => {
                push(Step::Group(alu(AluOp::Fadd, into, neg(temp(r.d(3))), Some(special(SR_ONE)))));
                push(Step::Group(alu(AluOp::Fmin, into, temp(r.s(3)), Some(temp(into)))));
                temp(into)
            }
            _ => return None,
        };
        if f & SHC_BF_INVERT == 0 {
            return Some(base);
        }
        if f == SHC_BF_ZERO {
            return None;
        }
        push(Step::Group(alu(AluOp::Fadd, into, neg(base), Some(special(SR_ONE)))));
        Some(temp(into))
    };
    let mut steps: [Option<Step>; 8] = [None; 8];
    let mut m = 0;
    let mut collect = |st: Step| {
        steps[m] = Some(st);
        m += 1;
    };
    let fs = factor(b.src[ch], r.t(0), &mut collect);
    let fd = factor(b.dst[ch], r.t(1), &mut collect);
    for st in steps.iter().take(m).flatten() {
        push(*st);
    }
    /* A sum or difference of the two terms where one is a product is one
     * `fmad`, as pco's variants are (nir_lower_blend, then
     * nir_opt_algebraic's ffma): the other term first, the product in it.
     * The negations go where `fmad` carries one: on its first source (the
     * product's factor) or its third (the other term). */
    if fuse {
        if let Some(g) = fused(eq, s, fs, d, fd, r, dst, &mut push) {
            push(Step::Group(g));
            return n;
        }
    }
    /* The two terms: s * Fs and d * Fd, skipped for a ZERO factor and taken
     * as they are for ONE. */
    let term = |x: SelSrc, f: Option<SelSrc>, into: u16, push: &mut dyn FnMut(Step)| -> Option<SelSrc> {
        match f {
            None => None,
            Some(SelSrc::Fixed { bank: Bank::Special, reg: SR_ONE, .. }) => Some(x),
            Some(f) => {
                push(Step::Group(alu(AluOp::Fmul, into, x, Some(f))));
                Some(temp(into))
            }
        }
    };
    let mut steps: [Option<Step>; 8] = [None; 8];
    let mut m = 0;
    let mut collect = |st: Step| {
        steps[m] = Some(st);
        m += 1;
    };
    let ts = term(s, fs, r.t(2), &mut collect);
    let td = term(d, fd, r.t(3), &mut collect);
    for st in steps.iter().take(m).flatten() {
        push(*st);
    }
    let zero = special(SR_ZERO);
    let g = match (eq, ts, td) {
        (_, None, None) => mov(dst, zero),
        (SHC_BLEND_ADD, Some(a), Some(c)) => alu(AluOp::Fadd, dst, a, Some(c)),
        (SHC_BLEND_ADD, Some(a), None) | (SHC_BLEND_ADD, None, Some(a)) => mov(dst, a),
        /* The negated term goes FIRST: `fadd` takes a negate on its first
         * source only (the encoding refuses s1neg), and with it second no
         * subtract variant built at all -- dEQP fragment_ops.blend
         * *subtract* failed on the AM62P, online (ba0966d7). */
        (SHC_BLEND_SUBTRACT, Some(a), Some(c)) => alu(AluOp::Fadd, dst, neg(c), Some(a)),
        (SHC_BLEND_SUBTRACT, Some(a), None) => mov(dst, a),
        (SHC_BLEND_SUBTRACT, None, Some(c)) => alu(AluOp::Fadd, dst, neg(c), Some(zero)),
        (_, Some(a), Some(c)) => alu(AluOp::Fadd, dst, neg(a), Some(c)),
        (_, None, Some(c)) => mov(dst, c),
        (_, Some(a), None) => alu(AluOp::Fadd, dst, neg(a), Some(zero)),
    };
    push(Step::Group(g));
    n
}

/// Which factor of a channel: the source's (computed into `t(0)`) or the
/// destination's (`t(1)`).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Side {
    Src,
    Dst,
}

/// Whether factor `f` of channel `k` is already in its temporary: it is
/// computed (an inverted alpha, or SRC_ALPHA_SATURATE on a colour channel),
/// its value is the same in every channel that has it (it reads alpha
/// only), and an earlier channel computed the same factor on the same side.
/// Nothing between writes that temporary: a channel writes `t(0)` and
/// `t(1)` only with its factors, every colour channel's factor is the same,
/// and the alpha channel is the last.
fn computed_earlier(b: &Blend, k: u16, side: Side, f: u32) -> bool {
    let alpha_only = matches!(f & !SHC_BF_INVERT, SHC_BF_SRC_ALPHA | SHC_BF_DST_ALPHA | SHC_BF_CONST_ALPHA);
    let shared = (alpha_only && f & SHC_BF_INVERT != 0) || (f == SHC_BF_SRC_ALPHA_SATURATE && k < 3);
    if !shared {
        return false;
    }
    (0..k).any(|j| {
        let ch = usize::from(j == 3);
        let computes = b.masked & (1 << j) == 0 && b.eq[ch] != SHC_BLEND_MIN && b.eq[ch] != SHC_BLEND_MAX;
        let fj = if side == Side::Src { b.src[ch] } else { b.dst[ch] };
        computes && fj == f
    })
}

/// One channel's `s * Fs (op) d * Fd` as one `fmad` when one term is a
/// product and the other is there, for ADD, SUBTRACT and REVERSE_SUBTRACT;
/// the other term, when it is a product too, is computed into its
/// temporary first. `None` when the expression has no such form (a ZERO
/// factor on either side, or no product): the caller builds it as before.
fn fused(
    eq: u32,
    s: SelSrc,
    fs: Option<SelSrc>,
    d: SelSrc,
    fd: Option<SelSrc>,
    r: &Regs,
    dst: u16,
    push: &mut dyn FnMut(Step),
) -> Option<SelGroup> {
    let one = |f: &SelSrc| matches!(f, SelSrc::Fixed { bank: Bank::Special, reg: SR_ONE, .. });
    let (fs, fd) = (fs?, fd?);
    if one(&fs) && one(&fd) {
        return None;
    }
    /* The product the fmad takes: the destination term's when it is one. */
    let (px, pf, other, prod_is_d) = if !one(&fd) {
        let other = if one(&fs) {
            s
        } else {
            push(Step::Group(alu(AluOp::Fmul, r.t(2), s, Some(fs))));
            temp(r.t(2))
        };
        (d, fd, other, true)
    } else {
        (s, fs, d, false)
    };
    let neg_prod = match (eq, prod_is_d) {
        (SHC_BLEND_ADD, _) => false,
        /* s - d: the product negated when it is d's */
        (SHC_BLEND_SUBTRACT, true) => true,
        (SHC_BLEND_SUBTRACT, false) => false,
        /* d - s */
        (SHC_BLEND_REVERSE_SUBTRACT, true) => false,
        (SHC_BLEND_REVERSE_SUBTRACT, false) => true,
        _ => return None,
    };
    let neg_other = matches!((eq, prod_is_d), (SHC_BLEND_SUBTRACT, false) | (SHC_BLEND_REVERSE_SUBTRACT, true));
    let x = if neg_prod { neg(px) } else { px };
    let o = if neg_other { neg(other) } else { other };
    Some(SelGroup {
        op: SelOp::Alu(AluOp::Ffma),
        ty: OpType::F32,
        dst: Some(SelDst::Fixed { bank: Bank::Temp, reg: dst }),
        srcs: [Some(x), Some(pf), Some(o)],
    })
}

/// Step `i` of the blend tail, `None` past the end. Built one at a time: the
/// builder runs on Zephyr's stack.
pub fn step(b: &Blend, r: &Regs, i: usize) -> Result<Option<Group>, BuildError> {
    let mut i = i;
    let fixed = |st: Step| -> Result<Option<Group>, BuildError> {
        match st {
            Step::Group(g) => build_group(&g).map(Some),
            Step::Sat(dst, src) => super::shapes::build(
                &super::shapes::FSAT,
                &[Some(super::operands::Operand { bank: Bank::Temp.hw(), index: src as u32 }), None, None],
                super::operands::Operand { bank: Bank::Temp.hw(), index: dst as u32 },
            )
            .map(Some),
            Step::PixRead(index, dst) => build_pixout_read_from(index, dst).map(Some),
            Step::Unpack(format, scale, rpt, dst, src) => build_unpck(format, scale, rpt, src, dst).map(Some),
        }
    };
    /* The source: clamped for an RGBA8 or sRGB target, as it is for any
     * UNORM one; a float target takes it as it is. */
    if i < 4 {
        let (dst, src) = (r.s(i as u16), r.colour + i as u16);
        return fixed(if b.unorm() { Step::Sat(dst, src) } else { Step::Group(mov(dst, temp(src))) });
    }
    i -= 4;
    /* The pixel, in the target's format, the channels a format lacks read
     * as GL reads them (0, 0, 0, 1). */
    if b.reads_pixel() {
        let reads: &[Step] = match b.class {
            SHC_RT_F16X4 | SHC_RT_SRGB => &[
                Step::PixRead(0, r.p()),
                Step::PixRead(1, r.p1()),
                Step::Unpack("f16f16", 0, 2, r.d(0), r.p()),
                Step::Unpack("f16f16", 0, 2, r.d(2), r.p1()),
            ],
            SHC_RT_F16 => &[
                Step::PixRead(0, r.p()),
                Step::Unpack("f16f16", 0, 1, r.d(0), r.p()),
                Step::Group(mov(r.d(1), special(SR_ZERO))),
                Step::Group(mov(r.d(2), special(SR_ZERO))),
                Step::Group(mov(r.d(3), special(SR_ONE))),
            ],
            SHC_RT_F32 => &[
                Step::PixRead(0, r.d(0)),
                Step::Group(mov(r.d(1), special(SR_ZERO))),
                Step::Group(mov(r.d(2), special(SR_ZERO))),
                Step::Group(mov(r.d(3), special(SR_ONE))),
            ],
            _ => &[Step::PixRead(0, r.p()), Step::Unpack("u8888", 1, 4, r.d(0), r.p())],
        };
        if let Some(st) = reads.get(i) {
            return fixed(*st);
        }
        i -= reads.len();
    }
    if let Some(c) = r.consts {
        if i < 4 {
            return fixed(Step::Group(mov(
                r.c(i as u16),
                SelSrc::Fixed { bank: Bank::Shared, reg: c + i as u16, mods: Mods::default() },
            )));
        }
        i -= 4;
    }
    let mut buf = [Step::PixRead(0, 0); 8];
    for k in 0..b.channels() {
        let n = channel(b, r, k, true, &mut buf);
        if i < n {
            return fixed(buf[i]);
        }
        i -= n;
    }
    /* An sRGB target's blended colour, clamped for the F16 pack. */
    if b.clamps_result() {
        if i < 4 {
            let c = r.colour + i as u16;
            return fixed(Step::Sat(c, c));
        }
        i -= 4;
    }
    /* Alpha-to-one, on the blended colour. */
    if let (0, Some(flag)) = (i, r.a2one) {
        let a = r.colour + 3;
        return fixed(Step::Group(SelGroup {
            op: SelOp::Alu(AluOp::Bcsel),
            ty: OpType::U32,
            dst: Some(SelDst::Fixed { bank: Bank::Temp, reg: a }),
            srcs: [Some(temp(flag)), Some(special(SR_ONE)), Some(temp(a))],
        }));
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    /// Run the channels' groups up to channel `k`, in the tail's order, on
    /// temporaries holding `s` (source), `d` (pixel) and `c` (blend colour);
    /// channel `k`'s result. In order because a channel reads a factor an
    /// earlier channel computed (`computed_earlier`).
    fn run(b: &Blend, fuse: bool, s: [f32; 4], d: [f32; 4], c: [f32; 4], k: u16) -> f32 {
        let r = Regs { colour: 0, scratch: 4, consts: None, a2one: None };
        let mut t: BTreeMap<u16, f32> = BTreeMap::new();
        for i in 0..4u16 {
            t.insert(r.s(i), s[i as usize]);
            t.insert(r.d(i), d[i as usize]);
            t.insert(r.c(i), c[i as usize]);
        }
        let read = |x: &SelSrc, t: &BTreeMap<u16, f32>| -> f32 {
            let (v, m) = match *x {
                SelSrc::Fixed { bank: Bank::Temp, reg, mods } => (t[&reg], mods),
                SelSrc::Fixed { bank: Bank::Special, reg: SR_ONE, mods } => (1.0, mods),
                SelSrc::Fixed { bank: Bank::Special, reg: SR_ZERO, mods } => (0.0, mods),
                other => panic!("an operand this test does not model: {other:?}"),
            };
            let v = if m.abs { v.abs() } else { v };
            if m.neg {
                -v
            } else {
                v
            }
        };
        let mut out = [Step::Group(mov(0, special(SR_ZERO))); 8];
        let steps = (0..=k).flat_map(|j| {
            let n = channel(b, &r, j, fuse, &mut out);
            out[..n].to_vec()
        });
        for st in steps.collect::<Vec<_>>() {
            let Step::Group(g) = st else { panic!("a channel builds groups only") };
            let a: Vec<f32> = g.srcs.iter().flatten().map(|x| read(x, &t)).collect();
            let v = match g.op {
                SelOp::Copy => a[0],
                SelOp::Alu(AluOp::Fadd) => a[0] + a[1],
                SelOp::Alu(AluOp::Fmul) => a[0] * a[1],
                SelOp::Alu(AluOp::Ffma) => a[0] * a[1] + a[2],
                SelOp::Alu(AluOp::Fmin) => a[0].min(a[1]),
                SelOp::Alu(AluOp::Fmax) => a[0].max(a[1]),
                other => panic!("an operation this test does not model: {other:?}"),
            };
            let Some(SelDst::Fixed { reg, .. }) = g.dst else { panic!("a fixed destination") };
            t.insert(reg, v);
        }
        t[&k]
    }

    /// GL's blend of channel `k` (ES 2.0 4.1.6, table 4.1 and 4.2) in f32,
    /// rounded as the channel's separate groups round it: each product,
    /// then the sum; `1 - x` as one subtraction.
    fn reference(b: &Blend, s: [f32; 4], d: [f32; 4], c: [f32; 4], k: u16) -> f32 {
        let (k, ch) = (k as usize, usize::from(k == 3));
        let f = |f: u32| -> f32 {
            let base = match f & !SHC_BF_INVERT {
                SHC_BF_ONE => 1.0,
                SHC_BF_SRC_COLOR => s[k],
                SHC_BF_SRC_ALPHA => s[3],
                SHC_BF_DST_ALPHA => d[3],
                SHC_BF_DST_COLOR => d[k],
                SHC_BF_CONST_COLOR => c[k],
                SHC_BF_CONST_ALPHA => c[3],
                SHC_BF_SRC_ALPHA_SATURATE if k == 3 => 1.0,
                SHC_BF_SRC_ALPHA_SATURATE => s[3].min(1.0 - d[3]),
                other => panic!("a factor this test does not model: {other:#x}"),
            };
            match f {
                SHC_BF_ZERO => 0.0,
                _ if f & SHC_BF_INVERT != 0 => 1.0 - base,
                _ => base,
            }
        };
        let (ts, td) = (s[k] * f(b.src[ch]), d[k] * f(b.dst[ch]));
        match b.eq[ch] {
            SHC_BLEND_ADD => ts + td,
            SHC_BLEND_SUBTRACT => ts - td,
            SHC_BLEND_REVERSE_SUBTRACT => td - ts,
            SHC_BLEND_MIN => s[k].min(d[k]),
            SHC_BLEND_MAX => s[k].max(d[k]),
            other => panic!("an equation {other}"),
        }
    }

    /// Every equation and every pair of a set of factors (both inverted and
    /// plain, the constant and the alpha-saturate among them), on values
    /// where a misplaced negation shows: the fused channel computes what the
    /// pair of terms computes, bit for bit (the model rounds `fmad` as a
    /// product then a sum, so equal bits mean the same expression).
    #[test]
    fn a_fused_channel_computes_the_same_blend_as_the_terms() {
        let factors = [
            SHC_BF_ONE,
            SHC_BF_ZERO,
            SHC_BF_SRC_COLOR,
            SHC_BF_SRC_ALPHA,
            SHC_BF_DST_ALPHA,
            SHC_BF_DST_COLOR,
            SHC_BF_CONST_ALPHA,
            SHC_BF_SRC_ALPHA_SATURATE,
            SHC_BF_INVERT | SHC_BF_SRC_ALPHA,
            SHC_BF_INVERT | SHC_BF_DST_ALPHA,
            SHC_BF_INVERT | SHC_BF_CONST_ALPHA,
            SHC_BF_INVERT | SHC_BF_DST_COLOR,
            SHC_BF_INVERT | SHC_BF_CONST_COLOR,
        ];
        let eqs = [SHC_BLEND_ADD, SHC_BLEND_SUBTRACT, SHC_BLEND_REVERSE_SUBTRACT, SHC_BLEND_MIN, SHC_BLEND_MAX];
        let vals = [([0.75, 0.25, 0.5, 0.6], [0.125, 0.9, 0.3, 0.2]), ([0.1, 0.8, 0.0, 1.0], [0.6, 0.05, 1.0, 0.45])];
        let c = [0.2, 0.4, 0.7, 0.35];
        let (mut checked, mut fused_seen) = (0, 0);
        for &eq in &eqs {
            for &fs in &factors {
                for &fd in &factors {
                    let b = Blend { src: [fs, fs], dst: [fd, fd], eq: [eq, eq], masked: 0, class: SHC_RT_RGBA8 };
                    for (s, d) in vals {
                        for k in 0..4u16 {
                            let plain = run(&b, false, s, d, c, k);
                            let fusedv = run(&b, true, s, d, c, k);
                            let want = reference(&b, s, d, c, k);
                            assert_eq!(
                                plain.to_bits(),
                                want.to_bits(),
                                "eq {eq} fs {fs:#x} fd {fd:#x} ch {k}: {plain}, GL's formula {want}"
                            );
                            assert_eq!(
                                plain.to_bits(),
                                fusedv.to_bits(),
                                "eq {eq} fs {fs:#x} fd {fd:#x} ch {k}: {plain} vs {fusedv}"
                            );
                            checked += 1;
                        }
                    }
                    let r = Regs { colour: 0, scratch: 4, consts: None, a2one: None };
                    let mut out = [Step::Group(mov(0, special(SR_ZERO))); 8];
                    let n = channel(&b, &r, 0, true, &mut out);
                    if out.iter().take(n).any(|st| matches!(st, Step::Group(g) if g.op == SelOp::Alu(AluOp::Ffma))) {
                        fused_seen += 1;
                    }
                }
            }
        }
        /* A factor computed once and read by the later channels: the
         * colour and alpha factors differing, channels masked, and MIN/MAX
         * (which compute no factor) on the colour equation. */
        let (mut shared_checked, mut shared_seen) = (0, 0);
        let alpha_factors = [SHC_BF_ONE, SHC_BF_INVERT | SHC_BF_SRC_ALPHA, SHC_BF_INVERT | SHC_BF_DST_ALPHA];
        for &(eq_rgb, eq_a) in
            &[(SHC_BLEND_ADD, SHC_BLEND_ADD), (SHC_BLEND_MIN, SHC_BLEND_ADD), (SHC_BLEND_ADD, SHC_BLEND_MAX)]
        {
            for &fs in &factors {
                for &fd in &factors {
                    for (fsa, fda) in alpha_factors.iter().flat_map(|a| alpha_factors.iter().map(move |b| (*a, *b))) {
                        for masked in [0u32, 0b0001, 0b0101, 0b0111] {
                            let b = Blend {
                                src: [fs, fsa],
                                dst: [fd, fda],
                                eq: [eq_rgb, eq_a],
                                masked,
                                class: SHC_RT_RGBA8,
                            };
                            for (s, d) in vals {
                                for k in (0..4u16).filter(|k| masked & (1 << k) == 0) {
                                    let got = run(&b, true, s, d, c, k);
                                    let plain = run(&b, false, s, d, c, k);
                                    let want = reference(&b, s, d, c, k);
                                    assert_eq!(plain.to_bits(), want.to_bits(), "{b:?} ch {k}: {plain}, GL's {want}");
                                    assert_eq!(got.to_bits(), plain.to_bits(), "{b:?} ch {k}: fused {got}");
                                    shared_checked += 1;
                                    let side = |f: u32, x: Side| computed_earlier(&b, k, x, f);
                                    let ch = usize::from(k == 3);
                                    if side(b.src[ch], Side::Src) || side(b.dst[ch], Side::Dst) {
                                        shared_seen += 1;
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        /* the population: every unmasked channel compared, and the reuse
         * exercised (not a check that never saw a shared factor) */
        assert_eq!(shared_checked, 3 * factors.len() * factors.len() * 9 * vals.len() * (4 + 3 + 2 + 1));
        assert!(shared_seen > 1000, "only {shared_seen} channels read an earlier channel's factor");
        /* the population: the comparisons made, and the cases that fused */
        assert_eq!(checked, eqs.len() * factors.len() * factors.len() * vals.len() * 4);
        assert!(fused_seen > 100, "only {fused_seen} factor pairs fused: the check did not see the fmad");
    }
}
