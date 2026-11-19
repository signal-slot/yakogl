// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The PowerVR's per-target key list of a draw (`pvr::target_keys`,
//! docs/notes/pvr-int-mixed-targets.md sections 2.1, 3 and 5), pinned from
//! outside the crate: the mixed key of the three plan mixes (RGBA8 beside
//! R32UI, the `common.modf` pair of a highp float target beside an integer
//! target of another layout, and a three-target mix), each `REFUSE_*` line
//! on a draw that triggers it, and that a draw of one class is not a mix.
//! Pure: no GPU, the draws are `GlesDraw` values built here.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test pvr_target_keys_refusals
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-powervr", not(feature = "report-es2")))]
use yakogl::backend::*;
use yakogl::pvr::target_keys::*;
use yakogl::target_key::*;

fn draw(fmts: &[u8]) -> GlesDraw {
    let mut d = GlesDraw::ZERO;
    d.target_gpu = 0x1000;
    d.target_format = fmts[0];
    d.num_targets = fmts.len() as u8;
    for (i, &f) in fmts.iter().enumerate() {
        d.targets[i] = GlesDrawTarget { gpu: 0x1000 * (i as u64 + 1), w: 64, h: 64, format: f, out_shift: 0 };
    }
    d
}

fn tk(f: u8) -> u32 {
    gles_surf_target_key(f).unwrap()
}

/// The mix's key must be the builder's over the per-format keys.
fn assert_mix(fmts: &[u8]) {
    let d = draw(fmts);
    let keys: Vec<u32> = fmts.iter().map(|&f| tk(f)).collect();
    let (k, n) = draw_target_keys(&d).unwrap();
    assert_eq!(&k[..n], &keys[..]);
    let key = shc_blend_key_mixed(&keys).expect("a mix");
    assert_eq!(draw_mixed_key(&d), Ok(key));
    assert_eq!(shc_blend_key_mixed_targets(key), Some((k, n)));
}

#[test]
fn rgba8_beside_r32ui_is_a_mix() {
    assert_mix(&[GLES_SURF_RGBA8888, GLES_SURF_R32UI]);
}

#[test]
fn modf_pair_is_a_mix() {
    // highp float + int outputs: a float target beside an integer of another layout
    assert_mix(&[GLES_SURF_R32F, GLES_SURF_RG32I]);
    assert_mix(&[GLES_SURF_RGBA16F, GLES_SURF_R32I]);
    assert_mix(&[GLES_SURF_R32I, GLES_SURF_R32F]);
}

#[test]
fn three_target_mix_is_a_mix() {
    assert_mix(&[GLES_SURF_RGBA8888, GLES_SURF_R32I, GLES_SURF_RGBA16F]);
}

#[test]
fn refuse_gl_none() {
    let mut d = draw(&[GLES_SURF_RGBA8888, GLES_SURF_R32UI, GLES_SURF_R32UI]);
    d.targets[1].gpu = 0;
    assert_eq!(draw_target_keys(&d), Err(REFUSE_GL_NONE));
    assert_eq!(draw_mixed_key(&d), Err(REFUSE_GL_NONE));
}

#[test]
fn refuse_srgb() {
    assert_eq!(draw_target_keys(&draw(&[GLES_SURF_SRGB8_A8, GLES_SURF_R32UI])), Err(REFUSE_SRGB));
    assert_eq!(draw_mixed_key(&draw(&[GLES_SURF_R32UI, GLES_SURF_SRGB8_A8])), Err(REFUSE_SRGB));
}

#[test]
fn refuse_four_registers() {
    // RGBA32UI takes four registers, the RGBA8 one more: two targets, the
    // second spilled, which no end-of-tile program emits
    assert_eq!(draw_target_keys(&draw(&[GLES_SURF_RGBA8888, GLES_SURF_RGBA32UI])), Err(REFUSE_FOUR_REGISTERS));
    // three targets, the last two spilled: none either
    assert_eq!(
        draw_mixed_key(&draw(&[GLES_SURF_R32UI, GLES_SURF_RGBA32UI, GLES_SURF_RG16UI])),
        Err(REFUSE_FOUR_REGISTERS)
    );
    // three two-register targets: the third spills to a tile buffer
    // (`eot3_tb`), so the list is kept, and with no integer target it is
    // no mix (plan #325 item mrt-frameload)
    assert_eq!(
        draw_mixed_key(&draw(&[GLES_SURF_RGBA16F, GLES_SURF_RGBA16F, GLES_SURF_RGBA16F])),
        Err(REFUSE_NOT_A_MIX)
    );
}

#[test]
fn refuse_not_a_mix() {
    // a format with no per-target key
    let d = draw(&[GLES_SURF_RGB10_A2 + 1, GLES_SURF_R32UI]);
    assert_eq!(draw_target_keys(&d), Err(REFUSE_NOT_A_MIX));
    assert_eq!(draw_mixed_key(&d), Err(REFUSE_NOT_A_MIX));
}

#[test]
fn single_class_is_not_a_mix() {
    for fmts in [
        &[GLES_SURF_R32UI][..],
        &[GLES_SURF_RGBA8888],
        &[GLES_SURF_R32UI, GLES_SURF_R32UI],
        &[GLES_SURF_R32UI, GLES_SURF_R32UI, GLES_SURF_R32UI],
        &[GLES_SURF_RGBA8888, GLES_SURF_RGBA16F],
        &[GLES_SURF_RGBA8888, GLES_SURF_RGBA8888, GLES_SURF_R32F],
    ] {
        let d = draw(fmts);
        assert!(draw_target_keys(&d).is_ok(), "{fmts:?}");
        assert_eq!(draw_mixed_key(&d), Err(REFUSE_NOT_A_MIX), "{fmts:?}");
    }
}
