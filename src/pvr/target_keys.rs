// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The per-target key list of a draw's colour targets and the mixed key of
//! it (docs/notes/pvr-int-mixed-targets.md, sections 2.1, 3 and 5). Pure: it
//! reads the draw's formats and gives the key or the refusal line the back
//! end logs.

use crate::backend::{gles_surf_is_srgb, GlesDraw};
use crate::target_key::{gles_surf_target_key, shc_blend_key_mixed, shc_mixed_out_layout, SHC_MIXED_MAX_TARGETS};

/// A slot with no target (`gpu` 0) before the last present one.
pub const REFUSE_GL_NONE: &str =
    "yakogl/pvr: draw refused: a GL_NONE draw buffer between an integer colour attachment and another (not built)";
/// An SRGB8_ALPHA8 attachment in the list.
pub const REFUSE_SRGB: &str =
    "yakogl/pvr: draw refused: an SRGB8_ALPHA8 colour attachment beside an integer one (not built)";
/// More targets than a mixed key carries, or a layout past the pixel's four
/// output registers.
pub const REFUSE_FOUR_REGISTERS: &str =
    "yakogl/pvr: draw refused: integer and other colour attachments past the pixel's four output registers";
/// The list is not a mix (or a format has no per-target key): the line the
/// back end already logs.
pub const REFUSE_NOT_A_MIX: &str =
    "yakogl/pvr: draw refused: an integer colour attachment beside another colour attachment";

/// The dense per-target keys of draw buffers 0.. and how many are present.
/// `Err` is the refusal line: a GL_NONE gap, an sRGB target, or more than
/// `SHC_MIXED_MAX_TARGETS` targets / a layout past four registers.
pub fn draw_target_keys(d: &GlesDraw) -> Result<([u32; SHC_MIXED_MAX_TARGETS], usize), &'static str> {
    let n = (d.num_targets as usize).clamp(1, d.targets.len());
    // Target 0 is the frame's own (`target_*`), as `several_int_targets_drawn` reads it.
    let slot =
        |i: usize| if i == 0 { (d.target_gpu, d.target_format) } else { (d.targets[i].gpu, d.targets[i].format) };
    let Some(last) = (0..n).rev().find(|&i| slot(i).0 != 0) else {
        return Ok(([0; SHC_MIXED_MAX_TARGETS], 0));
    };
    if (0..last).any(|i| slot(i).0 == 0) {
        return Err(REFUSE_GL_NONE);
    }
    if (0..=last).any(|i| gles_surf_is_srgb(slot(i).1)) {
        return Err(REFUSE_SRGB);
    }
    if last + 1 > SHC_MIXED_MAX_TARGETS {
        return Err(REFUSE_FOUR_REGISTERS);
    }
    let mut keys = [0u32; SHC_MIXED_MAX_TARGETS];
    for (i, k) in keys.iter_mut().enumerate().take(last + 1) {
        let Some(t) = gles_surf_target_key(slot(i).1) else {
            return Err(REFUSE_NOT_A_MIX);
        };
        *k = t;
    }
    if shc_mixed_out_layout(&keys[..=last]).is_none() {
        return Err(REFUSE_FOUR_REGISTERS);
    }
    Ok((keys, last + 1))
}

/// The mixed key of the draw's target list; `Err(REFUSE_NOT_A_MIX)` when the
/// list is not a mix (one target, or all the same integer layout).
pub fn draw_mixed_key(d: &GlesDraw) -> Result<u32, &'static str> {
    let (keys, n) = draw_target_keys(d)?;
    shc_blend_key_mixed(&keys[..n]).ok_or(REFUSE_NOT_A_MIX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::*;
    use crate::target_key::*;

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

    #[test]
    fn rgba8_beside_r32ui() {
        let d = draw(&[GLES_SURF_RGBA8888, GLES_SURF_R32UI]);
        let (k, n) = draw_target_keys(&d).unwrap();
        assert_eq!((n, k[0], k[1]), (2, tk(GLES_SURF_RGBA8888), tk(GLES_SURF_R32UI)));
        assert_eq!(draw_mixed_key(&d), Ok(shc_blend_key_mixed(&k[..2]).unwrap()));
    }

    #[test]
    fn two_integer_layouts() {
        let d = draw(&[GLES_SURF_R32UI, GLES_SURF_R32I]);
        assert_eq!(draw_mixed_key(&d), Ok(shc_blend_key_mixed(&[tk(GLES_SURF_R32UI), tk(GLES_SURF_R32I)]).unwrap()));
    }

    #[test]
    fn three_target_mix() {
        let d = draw(&[GLES_SURF_RGBA8888, GLES_SURF_R32I, GLES_SURF_RGBA16F]);
        let (k, n) = draw_target_keys(&d).unwrap();
        assert_eq!((n, k), (3, [tk(GLES_SURF_RGBA8888), tk(GLES_SURF_R32I), tk(GLES_SURF_RGBA16F)]));
        assert!(shc_blend_key_is_mixed(draw_mixed_key(&d).unwrap()));
    }

    #[test]
    fn gl_none_gap() {
        let mut d = draw(&[GLES_SURF_RGBA8888, GLES_SURF_R32UI, GLES_SURF_R32UI]);
        d.targets[1].gpu = 0;
        assert_eq!(draw_target_keys(&d), Err(REFUSE_GL_NONE));
        // a trailing empty slot is not a gap
        let mut d = draw(&[GLES_SURF_RGBA8888, GLES_SURF_R32UI, GLES_SURF_R32UI]);
        d.targets[2].gpu = 0;
        assert_eq!(draw_target_keys(&d).unwrap().1, 2);
    }

    #[test]
    fn srgb_refused() {
        let d = draw(&[GLES_SURF_SRGB8_A8, GLES_SURF_R32UI]);
        assert_eq!(draw_target_keys(&d), Err(REFUSE_SRGB));
    }

    #[test]
    fn past_four_registers() {
        let d = draw(&[GLES_SURF_RGBA8888, GLES_SURF_RGBA32UI]);
        assert_eq!(draw_target_keys(&d), Err(REFUSE_FOUR_REGISTERS));
        let d = draw(&[GLES_SURF_RGBA16F, GLES_SURF_RGBA16F, GLES_SURF_RGBA16F]);
        assert_eq!(draw_target_keys(&d), Err(REFUSE_FOUR_REGISTERS));
    }

    #[test]
    fn not_a_mix() {
        assert_eq!(draw_mixed_key(&draw(&[GLES_SURF_R32UI])), Err(REFUSE_NOT_A_MIX));
        assert_eq!(draw_mixed_key(&draw(&[GLES_SURF_R32UI, GLES_SURF_R32UI])), Err(REFUSE_NOT_A_MIX));
        assert_eq!(draw_mixed_key(&draw(&[GLES_SURF_RGBA8888, GLES_SURF_RGBA16F])), Err(REFUSE_NOT_A_MIX));
        // a format with no per-target key (past RGB10_A2)
        assert_eq!(draw_mixed_key(&draw(&[GLES_SURF_RGB10_A2 + 1, GLES_SURF_R32UI])), Err(REFUSE_NOT_A_MIX));
    }
}
