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
    /// The render target class: SHC_RT_RGBA8, _F16X4, _F16 or _F32.
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
            masked: (key >> 26) & 0xf,
            class: key >> SHC_BLEND_KEY_RT_SHIFT,
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
fn channel(b: &Blend, r: &Regs, k: u16, out: &mut [Step; 8]) -> usize {
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
     * register already; `None` for ZERO. */
    let factor = |f: u32, into: u16, push: &mut dyn FnMut(Step)| -> Option<SelSrc> {
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
    /* The source: clamped for an RGBA8 target, as it is for any UNORM
     * one; a float target takes it as it is. */
    if i < 4 {
        let (dst, src) = (r.s(i as u16), r.colour + i as u16);
        return fixed(if b.class == SHC_RT_RGBA8 { Step::Sat(dst, src) } else { Step::Group(mov(dst, temp(src))) });
    }
    i -= 4;
    /* The pixel, in the target's format, the channels a format lacks read
     * as GL reads them (0, 0, 0, 1). */
    if b.reads_pixel() {
        let reads: &[Step] = match b.class {
            SHC_RT_F16X4 => &[
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
        let n = channel(b, r, k, &mut buf);
        if i < n {
            return fixed(buf[i]);
        }
        i -= n;
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
