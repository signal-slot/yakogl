// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Fixed-function blending as Mesa decides and packs it for one colour
//! target: whether an equation runs in the hardware's blend unit, its
//! operands, whether it is opaque or reads the destination, and its
//! constant. Each function is the Mesa 26.2.1 function it names
//! (src/panfrost/lib/pan_blend.c, unless said otherwise), for the
//! UNORM targets the frame builds (`is_float` false). An equation Mesa
//! runs in a blend shader runs in one when src/shaderc/online/mali/blend.rs
//! builds it ([`shader_equation`]: the constant colour's equations at 1x,
//! case 211's 4x constant colour, case 190's GL_MIN and
//! ONE_MINUS_DST_ALPHA columns), and is refused by the frame builder by
//! name otherwise.

use super::genxml::v10::{BlendEquation, BlendFunction, BlendOperandA, BlendOperandB, BlendOperandC};

/// A blend factor as gallium spells it (`pipe_blendfactor`): a base and an
/// invert bit, ZERO being ONE inverted (`util_blendfactor_without_invert`,
/// `util_blendfactor_is_inverted`). The GL core's `SHC_BF_*` codes are the
/// same pair.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Factor {
    pub base: Base,
    pub invert: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Base {
    One,
    SrcColor,
    SrcAlpha,
    DstAlpha,
    DstColor,
    SrcAlphaSaturate,
    ConstColor,
    ConstAlpha,
}

impl Factor {
    pub const ZERO: Factor = Factor { base: Base::One, invert: true };
    pub const ONE: Factor = Factor { base: Base::One, invert: false };
    pub const SRC_ALPHA: Factor = Factor { base: Base::SrcAlpha, invert: false };
    pub const INV_SRC_ALPHA: Factor = Factor { base: Base::SrcAlpha, invert: true };
}

/// `pipe_blend_func`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Func {
    Add,
    Subtract,
    ReverseSubtract,
    Min,
    Max,
}

/// One channel group's equation: `func(src * src_factor, dst * dst_factor)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Channel {
    pub func: Func,
    pub src: Factor,
    pub dst: Factor,
}

/// `pan_blend_equation` with blending on: the colour and alpha equations.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Blending {
    pub rgb: Channel,
    pub alpha: Channel,
}

/// `factor_is_supported` (:26-34).
fn factor_is_supported(f: Factor) -> bool {
    f.base != Base::SrcAlphaSaturate
}

/// `is_2srcdest` (:53-62): src * dst + dst * src, encoded as 0 + dst * 2src.
fn is_2srcdest(c: Channel, is_alpha: bool) -> bool {
    let plain = |base| Factor { base, invert: false };
    c.func == Func::Add
        && (c.src == plain(Base::DstColor) || (c.src == plain(Base::DstAlpha) && is_alpha))
        && (c.dst == plain(Base::SrcColor) || (c.dst == plain(Base::SrcAlpha) && is_alpha))
}

/// `can_fixed_function_equation` (:64-106) for a UNORM target on v10, which
/// has the 2src operand (`pan_blend_supports_2src`).
fn channel_fixed_function(c: Channel, is_alpha: bool) -> bool {
    if !matches!(c.func, Func::Add | Func::Subtract | Func::ReverseSubtract) {
        return false;
    }
    if is_2srcdest(c, is_alpha) {
        return true;
    }
    if !factor_is_supported(c.src) || !factor_is_supported(c.dst) {
        return false;
    }
    c.src.base == c.dst.base || c.src.base == Base::One || c.dst.base == Base::One
}

/// `pan_blend_can_fixed_function` (:284-298).
pub fn can_fixed_function(b: &Blending) -> bool {
    channel_fixed_function(b.rgb, false) && channel_fixed_function(b.alpha, true)
}

/// `blend_factor_constant_mask` (:108-120) and `pan_blend_constant_mask`
/// (:122-128): the constant's channels the factors read, RGB 0b0111, A
/// 0b1000.
pub fn constant_mask(b: &Blending) -> u32 {
    let m = |f: Factor| match f.base {
        Base::ConstColor => 0b0111,
        Base::ConstAlpha => 0b1000,
        _ => 0,
    };
    m(b.rgb.src) | m(b.rgb.dst) | m(b.alpha.src) | m(b.alpha.dst)
}

/// `pan_blend_get_constant` (pan_blend.h:84-87): the first channel read;
/// and whether every channel read holds it (`pan_blend_is_homogenous_constant`,
/// :244-258). Fixed function has one constant; Mesa runs any other in a
/// blend shader (`panfrost_get_blend`, pan_context.c:176-184).
pub fn constant(mask: u32, colour: &[f32; 4]) -> Option<f32> {
    if mask == 0 {
        return Some(0.0);
    }
    let c = colour[mask.trailing_zeros() as usize];
    (0..4).filter(|i| mask >> i & 1 == 1).all(|i| colour[i] == c).then_some(c)
}

/// The blend descriptor's constant: a UNORM of the target's channel size in
/// the top bits. The board's Mesa 25.0.7 truncates to the channel size
/// first, `(uint16_t)(cons * ((1 << chan_size) - 1)) << (16 - chan_size)`
/// (its gallium pan_cmdstream.c:290-301; 0x7f00 for 0.5 in an 8-bit
/// target, 0dce654a8:tools/mali/board-ref/2026-10-19-blend); 26.2.1 multiplies by the
/// shifted maximum instead (`pan_pack_blend_constant`, :261-280, 0x7f80).
/// This follows the board, whose pixels are the proof. `chan_size` is
/// channel 0's size, as both read it (`format_desc->channel[0].size`).
pub fn pack_constant(cons: f32, chan_size: u32) -> u32 {
    let unorm = (cons * ((1u32 << chan_size) - 1) as f32) as u16 as u32;
    (unorm << (16 - chan_size)) & 0xffff
}

/// `pan_blend_is_opaque` (:430-458) for an unmasked target: ONE, ZERO with
/// add or subtract on both channel groups.
pub fn is_opaque(b: &Blending) -> bool {
    let replace =
        |c: Channel| c.src == Factor::ONE && c.dst == Factor::ZERO && matches!(c.func, Func::Add | Func::Subtract);
    replace(b.rgb) && replace(b.alpha)
}

/// `is_dest_factor` (:559-567).
fn is_dest_factor(f: Factor, alpha: bool) -> bool {
    f.base == Base::DstAlpha || f.base == Base::DstColor || (f.base == Base::SrcAlphaSaturate && !alpha)
}

/// `pan_blend_reads_dest` (:573-600) with blending on and every channel
/// written (a masked target reads it whatever the equation).
pub fn reads_dest(b: &Blending) -> bool {
    if matches!(b.rgb.func, Func::Min | Func::Max) || matches!(b.alpha.func, Func::Min | Func::Max) {
        return true;
    }
    is_dest_factor(b.rgb.src, false)
        || is_dest_factor(b.alpha.src, true)
        || b.rgb.dst != Factor::ZERO
        || b.alpha.dst != Factor::ZERO
}

/// `to_c_factor` (:300-327).
fn c_factor(f: Factor) -> BlendOperandC {
    match f.base {
        /* the hardware's 0, inverted back by the caller */
        Base::One => BlendOperandC::ZERO,
        Base::SrcAlpha => BlendOperandC::SRC_ALPHA,
        Base::DstAlpha => BlendOperandC::DEST_ALPHA,
        Base::SrcColor => BlendOperandC::SRC,
        Base::DstColor => BlendOperandC::DEST,
        Base::ConstColor | Base::ConstAlpha => BlendOperandC::CONSTANT,
        /* refused before (`can_fixed_function`) */
        Base::SrcAlphaSaturate => BlendOperandC::ZERO,
    }
}

/// `to_mali_function` (:329-428): `A op (B * C)`, the factor in C, ZERO and
/// ONE as the hardware's 0 and its invert.
fn function(c: Channel, is_alpha: bool) -> BlendFunction {
    let inverted = |f: Factor| f.invert ^ (f.base == Base::One);
    let (src_inverted, dst_inverted) = (inverted(c.src), inverted(c.dst));
    let mut f = BlendFunction::default();
    if c.src == Factor::ZERO {
        f.a = BlendOperandA::ZERO;
        f.b = BlendOperandB::DEST;
        f.negate_b = c.func == Func::Subtract;
        f.invert_c = dst_inverted;
        f.c = c_factor(c.dst);
    } else if c.src == Factor::ONE {
        f.a = BlendOperandA::SRC;
        f.b = BlendOperandB::DEST;
        f.negate_b = c.func == Func::Subtract;
        f.negate_a = c.func == Func::ReverseSubtract;
        f.invert_c = dst_inverted;
        f.c = c_factor(c.dst);
    } else if c.dst == Factor::ZERO {
        f.a = BlendOperandA::ZERO;
        f.b = BlendOperandB::SRC;
        f.negate_b = c.func == Func::ReverseSubtract;
        f.invert_c = src_inverted;
        f.c = c_factor(c.src);
    } else if c.dst == Factor::ONE {
        f.a = BlendOperandA::DEST;
        f.b = BlendOperandB::SRC;
        f.negate_a = c.func == Func::Subtract;
        f.negate_b = c.func == Func::ReverseSubtract;
        f.invert_c = src_inverted;
        f.c = c_factor(c.src);
    } else if c.src == c.dst {
        f.a = BlendOperandA::ZERO;
        f.invert_c = src_inverted;
        f.c = c_factor(c.src);
        match c.func {
            Func::Add => f.b = BlendOperandB::SRC_PLUS_DEST,
            Func::ReverseSubtract => {
                f.negate_b = true;
                f.b = BlendOperandB::SRC_MINUS_DEST;
            }
            _ => f.b = BlendOperandB::SRC_MINUS_DEST,
        }
    } else if is_2srcdest(c, is_alpha) {
        f.a = BlendOperandA::ZERO;
        f.b = BlendOperandB::DEST;
        f.c = BlendOperandC::SRC_X_2;
    } else {
        /* the same base, one inverted (`can_fixed_function` holds) */
        f.a = BlendOperandA::DEST;
        f.invert_c = src_inverted;
        f.c = c_factor(c.src);
        match c.func {
            Func::Add => f.b = BlendOperandB::SRC_MINUS_DEST,
            Func::ReverseSubtract => {
                f.b = BlendOperandB::SRC_PLUS_DEST;
                f.negate_b = true;
            }
            _ => {
                f.b = BlendOperandB::SRC_PLUS_DEST;
                f.negate_a = true;
            }
        }
    }
    f
}

/// Whether Mesa draws `b` with a blend shader (`panfrost_get_blend`,
/// pan_context.c:170-200): the equation is not fixed function, or the
/// channels of the constant it reads differ (the descriptor holds one).
/// For a target the draw writes and blending that is not opaque, which the
/// caller has checked.
pub fn needs_shader(b: &Blending, colour: &[f32; 4]) -> bool {
    !(can_fixed_function(b) && constant(constant_mask(b), colour).is_some())
}

/// The blend shader of `b` written through `color_mask` into a target of
/// `samples` and `format` that src/shaderc/online/mali/blend.rs builds
/// (Mesa's cache key without the constants, G/pan_blend_cso.c:59-98): all
/// four channels into RGBA8, ADD or MIN on each channel group, the factors
/// ZERO, ONE, the constant colour, SRC_ALPHA and the inverses of the
/// constant colour, SRC_ALPHA and DST_ALPHA, and of those the equations
/// `blend::builds` takes (the constant colour's at 1x, case 211's at 4x,
/// case 190's columns 3 and 4). None for any other key, which stays
/// refused.
pub fn shader_equation(
    b: &Blending,
    color_mask: u32,
    samples: u32,
    format: super::frame::TargetFormat,
) -> Option<crate::shaderc::online::mali::blend::Equation> {
    use crate::shaderc::online::mali::blend::{builds, Channel as C, Equation, Factor as F};
    if color_mask != 0xf || format != super::frame::TargetFormat::Rgba8888 {
        return None;
    }
    let factor = |f: Factor| match (f.base, f.invert) {
        (Base::One, false) => Some(F::One),
        (Base::One, true) => Some(F::Zero),
        (Base::ConstColor, false) => Some(F::Const),
        (Base::ConstColor, true) => Some(F::InvConst),
        (Base::SrcAlpha, false) => Some(F::SrcAlpha),
        (Base::SrcAlpha, true) => Some(F::InvSrcAlpha),
        (Base::DstAlpha, true) => Some(F::InvDstAlpha),
        _ => None,
    };
    /* a MIN channel reads no factor (`nir_blend_factored`) */
    let channel = |c: Channel| match c.func {
        Func::Add => Some(C::add(factor(c.src)?, factor(c.dst)?)),
        Func::Min => Some(C::MIN),
        _ => None,
    };
    let eq = Equation { rgb: channel(b.rgb)?, alpha: channel(b.alpha)?, samples: u8::try_from(samples).ok()? };
    builds(eq).then_some(eq)
}

/// `pan_blend_to_fixed_function_equation` (:606-631) with blending on.
pub fn equation(b: &Blending, color_mask: u32) -> BlendEquation {
    BlendEquation { rgb: function(b.rgb, false), alpha: function(b.alpha, true), color_mask }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DST_COLOR: Factor = Factor { base: Base::DstColor, invert: false };
    const SRC_COLOR: Factor = Factor { base: Base::SrcColor, invert: false };

    fn both(func: Func, src: Factor, dst: Factor) -> Blending {
        let c = Channel { func, src, dst };
        Blending { rgb: c, alpha: c }
    }

    /// Mesa's rules: add/subtract only, factors that match up to an invert
    /// or one of them ZERO/ONE, the 2src form, no SRC_ALPHA_SATURATE.
    #[test]
    fn the_fixed_function_equations_are_mesa_s() {
        assert!(can_fixed_function(&both(Func::Add, Factor::SRC_ALPHA, Factor::INV_SRC_ALPHA)));
        assert!(can_fixed_function(&both(Func::Add, DST_COLOR, SRC_COLOR)), "2 * src * dst");
        assert!(!can_fixed_function(&both(Func::Min, Factor::ONE, Factor::ONE)), "min");
        assert!(!can_fixed_function(&both(Func::Add, Factor::SRC_ALPHA, DST_COLOR)), "two different factors");
        let sat = Factor { base: Base::SrcAlphaSaturate, invert: false };
        assert!(!can_fixed_function(&both(Func::Add, sat, Factor::ZERO)), "SRC_ALPHA_SATURATE");
    }

    /// One constant: the first channel read, refused when the channels read
    /// differ; the board's packing for an 8-bit target.
    #[test]
    fn the_constant_is_one_value_packed_as_the_board_packs_it() {
        let cc = Factor { base: Base::ConstColor, invert: false };
        let b = both(Func::Add, cc, Factor { invert: true, ..cc });
        assert_eq!(constant_mask(&b), 0b0111);
        assert_eq!(constant(0b0111, &[0.5, 0.5, 0.5, 0.25]), Some(0.5), "alpha is not read");
        assert_eq!(constant(0b0111, &[0.5, 0.25, 0.5, 0.5]), None);
        assert_eq!(pack_constant(0.5, 8), 0x7f00);
        assert_eq!(pack_constant(1.0, 8), 0xff00);
    }
}
