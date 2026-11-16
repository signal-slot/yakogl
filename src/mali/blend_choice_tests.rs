// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Whether a GLES2 blend state runs in the blend unit or in a blend shader:
//! `blend.rs`'s choice against a direct transcription of Mesa 26.2.1's rule
//! (MESA_PAN_TREE, read-only) over every GLES2 state: all fifteen source and
//! destination factors (SRC_ALPHA_SATURATE included), ADD, SUBTRACT,
//! REVERSE_SUBTRACT, MIN and MAX, each given separately for colour and alpha,
//! and constant colours that are homogeneous or not. RGBA8 and RGB565 take
//! the same path (both are blendable UNORM formats,
//! `blendable_formats[fmt].internal`; the rule reads no other property of the
//! format) and RGBA16F takes the float one.
//!
//! Feature set: host-test,gpu-mali,shaderc-online,no-shader-table
//! (`cargo test --lib --no-default-features --features ... mali::blend_choice_tests`).

use super::blend::{
    can_fixed_function, can_fixed_function_float, constant, constant_mask, is_opaque, needs_shader, Base, Blending,
    Channel, Factor, Func,
};

/// `enum pipe_blendfactor`, as a test-local mirror, so Mesa's rule is
/// written against Mesa's names.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Pf {
    One,
    SrcColor,
    SrcAlpha,
    DstAlpha,
    DstColor,
    SrcAlphaSaturate,
    ConstColor,
    ConstAlpha,
    Zero,
    InvSrcColor,
    InvSrcAlpha,
    InvDstAlpha,
    InvDstColor,
    InvConstColor,
    InvConstAlpha,
}

/// `enum pipe_blend_func`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Pb {
    Add,
    Subtract,
    ReverseSubtract,
    Min,
    Max,
}

/// `util_blendfactor_without_invert` (util/u_blend.h).
fn without_invert(f: Pf) -> Pf {
    match f {
        Pf::Zero => Pf::One,
        Pf::InvSrcColor => Pf::SrcColor,
        Pf::InvSrcAlpha => Pf::SrcAlpha,
        Pf::InvDstAlpha => Pf::DstAlpha,
        Pf::InvDstColor => Pf::DstColor,
        Pf::InvConstColor => Pf::ConstColor,
        Pf::InvConstAlpha => Pf::ConstAlpha,
        f => f,
    }
}

/// The GLES2 factors (`glBlendFunc*`) as a gallium factor and as ours.
fn factors() -> Vec<(Pf, Factor)> {
    let f = |base, invert| Factor { base, invert };
    vec![
        (Pf::Zero, Factor::ZERO),
        (Pf::One, Factor::ONE),
        (Pf::SrcColor, f(Base::SrcColor, false)),
        (Pf::InvSrcColor, f(Base::SrcColor, true)),
        (Pf::DstColor, f(Base::DstColor, false)),
        (Pf::InvDstColor, f(Base::DstColor, true)),
        (Pf::SrcAlpha, f(Base::SrcAlpha, false)),
        (Pf::InvSrcAlpha, f(Base::SrcAlpha, true)),
        (Pf::DstAlpha, f(Base::DstAlpha, false)),
        (Pf::InvDstAlpha, f(Base::DstAlpha, true)),
        (Pf::ConstColor, f(Base::ConstColor, false)),
        (Pf::InvConstColor, f(Base::ConstColor, true)),
        (Pf::ConstAlpha, f(Base::ConstAlpha, false)),
        (Pf::InvConstAlpha, f(Base::ConstAlpha, true)),
        (Pf::SrcAlphaSaturate, f(Base::SrcAlphaSaturate, false)),
    ]
}

fn funcs() -> [(Pb, Func); 5] {
    [
        (Pb::Add, Func::Add),
        (Pb::Subtract, Func::Subtract),
        (Pb::ReverseSubtract, Func::ReverseSubtract),
        (Pb::Min, Func::Min),
        (Pb::Max, Func::Max),
    ]
}

type MesaChannel = (Pb, Pf, Pf);

/// `factor_is_supported` (src/panfrost/lib/pan_blend.c:26-34); SRC1_COLOR
/// and SRC1_ALPHA are no GLES2 factors.
fn m_factor_is_supported(f: Pf) -> bool {
    without_invert(f) != Pf::SrcAlphaSaturate
}

/// `factor_is_supported_for_float` (pan_blend.c:38-47).
fn m_factor_is_supported_for_float(f: Pf) -> bool {
    matches!(f, Pf::Zero | Pf::One | Pf::SrcAlpha | Pf::InvSrcAlpha)
}

/// `is_2srcdest` (pan_blend.c:53-62).
fn m_is_2srcdest((func, src, dst): MesaChannel, is_alpha: bool) -> bool {
    func == Pb::Add
        && (src == Pf::DstColor || (src == Pf::DstAlpha && is_alpha))
        && (dst == Pf::SrcColor || (dst == Pf::SrcAlpha && is_alpha))
}

/// `can_fixed_function_equation` (pan_blend.c:64-106) on arch 10, which
/// supports 2src (`pan_blend_supports_2src`, pan_blend.h:101-105).
fn m_can_fixed_function_equation(c: MesaChannel, is_alpha: bool, is_float: bool) -> bool {
    let (func, src, dst) = c;
    if !matches!(func, Pb::Add | Pb::Subtract | Pb::ReverseSubtract) {
        return false;
    }
    if is_float {
        if func == Pb::Add && src == Pf::Zero && (dst == Pf::InvSrcColor || dst == Pf::DstAlpha) {
            return true;
        }
        return m_factor_is_supported_for_float(src) && m_factor_is_supported_for_float(dst);
    }
    if m_is_2srcdest(c, is_alpha) {
        return true;
    }
    if !m_factor_is_supported(src) || !m_factor_is_supported(dst) {
        return false;
    }
    let (s, d) = (without_invert(src), without_invert(dst));
    /* `arch < 6 && src == DST_ALPHA` is false on arch 10 */
    s == d || s == Pf::One || d == Pf::One
}

/// `blend_factor_constant_mask` (pan_blend.c:108-120).
fn m_constant_mask(f: Pf) -> u32 {
    match without_invert(f) {
        Pf::ConstColor => 0b0111,
        Pf::ConstAlpha => 0b1000,
        _ => 0,
    }
}

fn m_mask(rgb: MesaChannel, alpha: MesaChannel) -> u32 {
    m_constant_mask(rgb.1) | m_constant_mask(rgb.2) | m_constant_mask(alpha.1) | m_constant_mask(alpha.2)
}

/// `pan_blend_is_homogenous_constant` (pan_blend.c:244-258), with
/// `pan_blend_get_constant` (pan_blend.h:84-87): the first channel of the
/// mask (0 for an empty one).
fn m_is_homogenous_constant(mask: u32, constants: &[f32; 4]) -> bool {
    let constant = if mask == 0 { 0.0 } else { constants[mask.trailing_zeros() as usize] };
    (0..4).filter(|i| mask >> i & 1 == 1).all(|i| constants[i] == constant)
}

/// `pan_blend_is_opaque` (pan_blend.c:430-458), blending on, no channel
/// masked.
fn m_is_opaque(rgb: MesaChannel, alpha: MesaChannel) -> bool {
    let replace =
        |(func, src, dst): MesaChannel| src == Pf::One && dst == Pf::Zero && (func == Pb::Add || func == Pb::Subtract);
    replace(rgb) && replace(alpha)
}

/// `panfrost_get_blend` (gallium/drivers/panfrost/pan_context.c:156-200) for
/// a blendable target the draw writes: does it come back with a blend
/// shader? `info.fixed_function` is `pan_blend_can_fixed_function` and the
/// constant check (pan_cmdstream.c:4398-4406; arch 10 supports a constant on
/// every target), `info.opaque` is `pan_blend_is_opaque` (:4392).
fn m_needs_shader(rgb: MesaChannel, alpha: MesaChannel, is_float: bool, constants: &[f32; 4]) -> bool {
    let fixed_function =
        m_can_fixed_function_equation(rgb, false, is_float) && m_can_fixed_function_equation(alpha, true, is_float);
    if fixed_function && m_is_homogenous_constant(m_mask(rgb, alpha), constants) {
        return false;
    }
    !m_is_opaque(rgb, alpha)
}

/// Every channel group's equation: 5 funcs x 15 x 15 factors.
fn channels() -> Vec<(MesaChannel, Channel)> {
    let mut v = Vec::new();
    for (pb, func) in funcs() {
        for &(ps, src) in &factors() {
            for &(pd, dst) in &factors() {
                v.push(((pb, ps, pd), Channel { func, src, dst }));
            }
        }
    }
    v
}

/// The constants: all alike, RGB alike and alpha apart, and three that
/// differ inside the RGB or the pair of channels a factor reads.
const CONSTANTS: [[f32; 4]; 5] = [
    [0.5, 0.5, 0.5, 0.5],
    [0.25, 0.25, 0.25, 0.75],
    [0.25, 0.5, 0.25, 0.25],
    [0.25, 0.25, 0.5, 0.25],
    [0.0, 1.0, 0.0, 1.0],
];

#[test]
fn the_fixed_function_choice_is_mesa_s_over_every_gles2_state() {
    let channels = channels();
    assert_eq!(channels.len(), 5 * 15 * 15);
    let (mut shader, mut fixed) = (0usize, 0usize);
    for (mrgb, rgb) in &channels {
        for (malpha, alpha) in &channels {
            let b = Blending { rgb: *rgb, alpha: *alpha };
            let ff = |is_float| {
                m_can_fixed_function_equation(*mrgb, false, is_float)
                    && m_can_fixed_function_equation(*malpha, true, is_float)
            };
            assert_eq!(can_fixed_function(&b), ff(false), "can_fixed_function {mrgb:?} {malpha:?}");
            assert_eq!(can_fixed_function_float(&b), ff(true), "can_fixed_function_float {mrgb:?} {malpha:?}");
            assert_eq!(is_opaque(&b), m_is_opaque(*mrgb, *malpha), "is_opaque {mrgb:?} {malpha:?}");
            let mask = m_mask(*mrgb, *malpha);
            assert_eq!(constant_mask(&b), mask, "constant_mask {mrgb:?} {malpha:?}");
            for k in &CONSTANTS {
                /* the frame builder keeps an opaque blend off the shader path
                 * (`panfrost_get_blend`: `info.opaque`) before `needs_shader` */
                let ours = !is_opaque(&b) && needs_shader(&b, k);
                let want = m_needs_shader(*mrgb, *malpha, false, k);
                assert_eq!(ours, want, "UNORM {mrgb:?} {malpha:?} {k:?}");
                /* RGBA16F: `fixed_function_float` instead of `fixed_function` */
                let ours_f = !is_opaque(&b) && !(can_fixed_function_float(&b) && constant(mask, k).is_some());
                assert_eq!(ours_f, m_needs_shader(*mrgb, *malpha, true, k), "float {mrgb:?} {malpha:?} {k:?}");
                if want {
                    shader += 1;
                } else {
                    fixed += 1;
                }
            }
        }
    }
    /* both outcomes occur: the comparison is not empty */
    assert!(shader > 0 && fixed > 0, "{shader} shader, {fixed} fixed");
}
