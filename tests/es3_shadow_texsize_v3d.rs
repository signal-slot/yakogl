// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The V3D draw-time refusal of a shadow sampler's `textureSize` (plan #380,
//! image group S32c of tools/cts/pi4-gles3-tree.txt: S32c1
//! `texturesize.sampler2dshadow_fragment`, S32c2 `sampler2dshadow_vertex`,
//! S32c3 `samplercubeshadow_fragment`, S32c4 `samplercubeshadow_vertex`).
//! dEQP stopped at `sampler2dshadow_fragment` on the Pi 4 at b1b5b845 with
//!
//! ```text
//! a texture of 32-bit channels (R32F, RGBA32F, RGBA32UI, RGBA32I) read by a look-up compiled for the 16-bit TMU output
//! ```
//!
//! (`RET16_OF_32` in src/v3d/texture.rs, `Attempt::Unsupported`, -ENOTSUP).
//! The chain this file pins, in the two halves it can reach:
//!
//! * the compiler's program key. `LodKey::ret32` is set from the `Op::Tex`
//!   instructions of the shader (`v3d_sensitivity`, src/shaderc/online/
//!   backend.rs). A `textureSize` lowers to no `Op::Tex`, so a program that
//!   only asks a shadow sampler's size has `ret32` 0 for that binding: it is
//!   compiled for the 16-bit TMU output;
//! * the draw. The depth texture the case binds (DEPTH_COMPONENT32F, which
//!   the GL core hands over as `GLES_TEXFMT_R32F`) returns 32 bits
//!   (`return_32`), so `refusal_for(bind, false)` is `RET16_OF_32` and
//!   `tex_state_for` (src/v3d/backend.rs) refuses the draw, though no look-up
//!   reads the unit. The 32-bit key (`refusal_for(bind, true)`) accepts it.
//!
//! The fix would touch src/v3d/texture.rs (not refuse a unit no look-up
//! reads, or read the size with either output), src/v3d/backend.rs
//! (`tex_state_for`'s caller, the key it passes) and/or the compiler's
//! `LodKey` `ret32` choice for the binding (`v3d_sensitivity` in
//! src/shaderc/online/backend.rs, which sees only `Op::Tex`). It flips
//! `a_32_bit_depth_texture_of_a_size_only_shadow_sampler_is_refused`.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_shadow_texsize_v3d
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-v3d", not(feature = "report-es2")))]
use yakogl::backend::{GlesTextureBind, GLES_TEXFMT_R32F, GLES_TEXFMT_RGBA8, GLES_TEXTYPE_2D, GLES_TEXTYPE_CUBE};
use yakogl::shaderc::online::arena::Arena;
use yakogl::shaderc::online::ir::Op;
use yakogl::shaderc::online::{lower, sema};
use yakogl::v3d::texture;

/// RET16_OF_32, verbatim (src/v3d/texture.rs).
const RET16_OF_32: &str = "a texture of 32-bit channels (R32F, RGBA32F, RGBA32UI, RGBA32I) read by a look-up \
                           compiled for the 16-bit TMU output";

/// The number of `Op::Tex` instructions of a fragment shader reading
/// `sampler` `s` in `body`.
fn tex_ops(sampler: &str, body: &str) -> usize {
    let fs = format!(
        "#version 300 es\nprecision highp float;\nprecision highp {sampler};\nuniform {sampler} s;\n\
         in highp vec3 v_texCoord;\nlayout(location = 0) out highp vec4 o;\nvoid main() {{ {body} }}\n"
    );
    let mut mem = vec![0u8; 16 << 20];
    let a = Arena::new(&mut mem);
    let fs = a.alloc_str(&fs).unwrap();
    let f = lower::compile_to_ir(&a, sema::Stage::Fragment, fs).unwrap_or_else(|e| panic!("fragment: {}", e.msg));
    f.module.instrs.iter().filter(|op| matches!(op, Op::Tex { .. })).count()
}

/// The depth texture the dEQP case binds (DEPTH_COMPONENT32F, a cube's
/// faces): R32F in the GL core's storage.
fn depth32(type_: u8, layers: u16) -> GlesTextureBind {
    GlesTextureBind {
        format: GLES_TEXFMT_R32F,
        type_,
        layers,
        base_gpu: 0x1_0000_0000,
        width: 4,
        height: 4,
        stride_texels: 4,
        bytes: 4 * 4 * 4 * layers as u32,
        mip_levels: 1,
        ..GlesTextureBind::ZERO
    }
}

#[test]
fn the_refusal_text_is_pinned() {
    let why = texture::refusal_for(&depth32(GLES_TEXTYPE_2D, 1), false).expect("refused");
    assert_eq!(why, RET16_OF_32);
    assert!(texture::GAPS.contains(&why), "a backend gap: -ENOTSUP, not -EINVAL");
}

#[test]
fn a_shadow_texture_size_has_no_look_up_to_key_the_program_by() {
    let size = "o = vec4(vec2(textureSize(s, 0)), 0.0, 1.0);";
    assert_eq!(tex_ops("sampler2DShadow", size), 0, "textureSize of a 2D shadow sampler");
    assert_eq!(tex_ops("samplerCubeShadow", size), 0, "textureSize of a cube shadow sampler");
    assert_eq!(tex_ops("sampler2DShadow", "o = vec4(texture(s, v_texCoord));"), 1, "the control: a look-up is a Tex");
}

/// The assertion the fix flips: a program with no look-up of the binding is
/// compiled for the 16-bit output (`ret32` false), and the 32-bit depth
/// texture bound to it is refused.
#[test]
fn a_32_bit_depth_texture_of_a_size_only_shadow_sampler_is_refused() {
    for (what, b) in [("2D", depth32(GLES_TEXTYPE_2D, 1)), ("cube", depth32(GLES_TEXTYPE_CUBE, 6))] {
        assert_eq!(texture::refusal_for(&b, false), Some(RET16_OF_32), "{what}: flip this test when the fix lands");
        assert_eq!(texture::refusal_for(&b, true), None, "{what}: the 32-bit key accepts the depth texture");
        assert!(texture::return_32(&b), "{what}: an R32F depth returns 32 bits");
    }
}

#[test]
fn a_16_bit_format_is_not_refused_that_way() {
    let b = GlesTextureBind { format: GLES_TEXFMT_RGBA8, ..depth32(GLES_TEXTYPE_2D, 1) };
    assert_eq!(texture::refusal_for(&b, false), None);
}
