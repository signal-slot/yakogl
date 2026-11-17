// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The V3D refusal of `textureSize` of a `sampler2DArrayShadow` (plan #380,
//! issue #1261, image S32b of tools/cts/pi4-gles3-tree.txt: the last two of
//! its 18 textureSize cases, `texturesize.sampler2darrayshadow_fragment` and
//! `_vertex`; the other 16 are tests/es3_tex_words_v3d.rs). Issue draft 2 of
//! tools/cts/log/2026-11-17-pi4-gles3-351fb70d-rerun/drafts.md names the
//! cause; the chain was derived from the code, not reproduced with a host
//! dEQP build. The refusal is, verbatim,
//!
//! ```text
//! a texture of 32-bit channels (R32F, RGBA32F, RGBA32UI, RGBA32I) read by a look-up compiled for the 16-bit TMU output
//! ```
//!
//! (`RET16_OF_32` in src/v3d/texture.rs, `Attempt::Unsupported`, -ENOTSUP).
//! One test per link, as tests/es3_shadow_texsize_v3d.rs (the 2D and cube
//! shadow cases, S32c) does:
//!
//! * the text;
//! * the compiler's key: a `textureSize` lowers to no `Op::Tex`, so the
//!   binding's `ret32` is 0 and the program is compiled for the 16-bit output;
//! * the draw: the DEPTH_COMPONENT32F array (`GLES_TEXFMT_R32F`) returns 32
//!   bits, so `refusal_for(bind, false)` is `RET16_OF_32`; the 32-bit key
//!   accepts it. This flips with the fix (fix-ret16-size);
//! * a 2D array's size also needs the layer count, uniform word 47: once the
//!   RET16_OF_32 fix lands the case meets the job builder's refusal pinned by
//!   tests/es3_tex_words_v3d.rs (`..._flip_when_words_46_47_are_written`), so
//!   both fixes are needed before S32b passes;
//! * a 16-bit control.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_shadow_array_texsize_v3d
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-v3d", not(feature = "report-es2")))]
use yakogl::backend::{GlesTextureBind, GLES_TEXFMT_R32F, GLES_TEXFMT_RGBA8, GLES_TEXTYPE_2D_ARRAY};
use yakogl::shaderc::online::arena::Arena;
use yakogl::shaderc::online::ir::Op;
use yakogl::shaderc::online::v3d::chain::{compile_program, Prim, Target};
use yakogl::shaderc::online::v3d::select::QUNIFORM_TEXTURE_ARRAY_SIZE;
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

/// The depth array the dEQP case binds (DEPTH_COMPONENT32F, `layers` layers):
/// R32F in the GL core's storage.
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

const VS: &str = "#version 300 es\nin highp vec4 a_position;\nvoid main() { gl_Position = a_position; }\n";

/// The fragment uniform stream of a program whose fragment shader asks the
/// size of a `sampler2DArrayShadow`.
fn size_stream() -> Vec<u32> {
    let fs = "#version 300 es\nprecision highp float;\nprecision highp sampler2DArrayShadow;\n\
              uniform sampler2DArrayShadow s;\nlayout(location = 0) out highp vec4 o;\n\
              void main() { o = vec4(vec3(textureSize(s, 0)), 1.0); }\n";
    let mut mem = vec![0u8; 16 << 20];
    let a = Arena::new(&mut mem);
    let vs = a.alloc_str(VS).unwrap();
    let fs = a.alloc_str(fs).unwrap();
    let v = lower::compile_to_ir(&a, sema::Stage::Vertex, vs).unwrap_or_else(|e| panic!("vertex: {}", e.msg));
    let f = lower::compile_to_ir(&a, sema::Stage::Fragment, fs).unwrap_or_else(|e| panic!("fragment: {}", e.msg));
    let p = compile_program(&v.module, &f.module, Prim::Triangles, Target { vpm_size: 65536 }, &a)
        .unwrap_or_else(|e| panic!("program: {e:?}"));
    p.fs.code.uniforms.iter().map(|u| u.contents).collect()
}

#[test]
fn the_refusal_text_is_pinned() {
    let why = texture::refusal_for(&depth32(GLES_TEXTYPE_2D_ARRAY, 4), false).expect("refused");
    assert_eq!(why, RET16_OF_32);
    assert!(texture::GAPS.contains(&why), "a backend gap: -ENOTSUP, not -EINVAL");
}

#[test]
fn an_array_shadow_texture_size_has_no_look_up_to_key_the_program_by() {
    let size = "o = vec4(vec3(textureSize(s, 0)), 1.0);";
    assert_eq!(tex_ops("sampler2DArrayShadow", size), 0, "textureSize of a 2D array shadow sampler");
    assert_eq!(
        tex_ops("sampler2DArrayShadow", "o = vec4(texture(s, vec4(v_texCoord, 0.5)));"),
        1,
        "the control: a look-up is a Tex"
    );
}

/// The assertion the fix flips: no look-up means `ret32` false, and the
/// 32-bit depth array bound to the unit is refused.
#[test]
fn a_32_bit_depth_array_of_a_size_only_shadow_sampler_is_refused_flip_when_ret16_size_is_fixed() {
    let b = depth32(GLES_TEXTYPE_2D_ARRAY, 4);
    assert_eq!(texture::refusal_for(&b, false), Some(RET16_OF_32), "flip this test when the fix lands");
    assert_eq!(texture::refusal_for(&b, true), None, "the 32-bit key accepts the depth array");
    assert!(texture::return_32(&b), "an R32F depth returns 32 bits");
}

/// The second obstacle behind the first: the size of a 2D array carries word
/// 47 (tests/es3_tex_words_v3d.rs pins that the job builder lacks it).
#[test]
fn the_array_size_program_also_carries_word_47() {
    let c = size_stream();
    assert!(c.contains(&QUNIFORM_TEXTURE_ARRAY_SIZE), "{c:?}");
}

#[test]
fn a_16_bit_format_is_not_refused_that_way() {
    let b = GlesTextureBind { format: GLES_TEXFMT_RGBA8, ..depth32(GLES_TEXTYPE_2D_ARRAY, 4) };
    assert_eq!(texture::refusal_for(&b, false), None);
}
