// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The draws of dEQP-GLES3 `fragment_out.random.*` with an integer colour
//! attachment beside an attachment of another layout that the PowerVR back
//! end still refuses, each with its framebuffer and the line it is refused
//! with (plan #325 item mrt-frameload; RC1 and RC14 of
//! docs/es3-cts-failures.md).
//!
//! The lists are the cases a host run of `fragment_out.` ends with
//! `glDrawElements: GL_INVALID_OPERATION` for (`GPU=powervr MODULE=gles3
//! tools/cts/host-deqp-run.sh dEQP-GLES3.functional.fragment_out.`, the CTS
//! at 93bca01, HOST-ONLY): fourteen at three draw buffers, and twenty
//! four-attachment ones that `backend::DRAW_TARGETS` 4 lets link (at three
//! they fail at the link: an output location past the draw buffers). The
//! run's fifteenth three-buffer case, random.65 (RGBA16F beside R8), has no
//! integer attachment and is not here. None of the thirty-four is a defect
//! of `pvr_backend_draw`'s own: each is refused for one of three reasons
//! that the back end cannot lift alone, and this file pins which, so a list
//! that changes its reason (or starts to be drawn) is seen:
//!
//! - **The four output registers** (`REFUSE_FOUR_REGISTERS`): a BXS-4-64
//!   pixel has four 32-bit output registers and the attachments' dwords do
//!   not fit them in draw-buffer order (`mrt_layout::register_layout`
//!   refuses the list with `OutputRegs`). The answer designed for such a
//!   framebuffer is GL_FRAMEBUFFER_UNSUPPORTED from the GL core
//!   (docs/notes/pvr-mrt4-budget.md, plan #368 item `pvr-budget-fb`), which
//!   dEQP reads as NotSupported. Where `mrt_layout::layout` spills the last
//!   attachment to a tile buffer with a captured end-of-tile program
//!   (`eot3_tb`) the mixed key still has no layout
//!   (`target_key::shc_mixed_out_layout`), so no variant exists to draw
//!   with.
//! - **SRGB8_ALPHA8 in a mix** (`REFUSE_SRGB`): the sRGB class has no slot
//!   code in the mixed key (plan #368 item `pvr-srgb-mix`,
//!   docs/notes/pvr-blend-multi.md section 3).
//! - **A fourth draw buffer in a mix** (`REFUSE_MIX_TARGETS`): the mixed key
//!   carries three targets (`target_key::SHC_MIXED_MAX_TARGETS`; the
//!   four-target key is docs/notes/pvr-blend-multi.md's, plan #368). The
//!   back end names this before it looks at the registers; fourteen of the
//!   sixteen lists are past the four registers as well, and two fit them.
//!
//! A mix inside the budget is drawn: the last test gives the lists of the
//! same group that are, with the program the key names.
//!
//! HOST-ONLY: the pure acceptance rule (`pvr::backend::int_targets_drawn`,
//! `pvr::target_keys`, `pvr::mrt_layout`); nothing is drawn.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test es3_pvr_int_beside
//! ```

#![cfg(all(feature = "host-test", feature = "gpu-powervr", feature = "shaderc-online"))]

use yakogl::backend::*;
use yakogl::pvr::backend::{int_targets_drawn, surf_out_dwords};
use yakogl::pvr::mrt_layout::{layout, register_layout, EotProgram, LayoutError};
use yakogl::pvr::target_keys::*;
use yakogl::shaderc::ShcShader;
use yakogl::target_key::shc_mixed_out_layout;

/* The surfaces the GL core keeps dEQP's renderbuffers as
 * (`renderbuffer_surf_format`, src/gl/framebuffer.rs): GL_R8, GL_RG8,
 * GL_RGB8, GL_RGBA4, GL_RGB5_A1 and GL_RGB565 are RGBA8888 surfaces (the
 * RGBA8 class, one dword of bytes in the pixel's output registers), GL_RG16F
 * an RGBA16F one; every other format here is its own. */
const R8: u8 = GLES_SURF_RGBA8888;
const RG8: u8 = GLES_SURF_RGBA8888;
const RGB8: u8 = GLES_SURF_RGBA8888;
const RGBA4: u8 = GLES_SURF_RGBA8888;
const RGB5_A1: u8 = GLES_SURF_RGBA8888;
const RGB565: u8 = GLES_SURF_RGBA8888;
const RG16F: u8 = GLES_SURF_RGBA16F;
const SRGB: u8 = GLES_SURF_SRGB8_A8;

/// A draw of a triangle into `fmts`, one attachment each, no clear, no
/// blend, every channel written.
fn draw(fmts: &[u8]) -> GlesDraw {
    let mut d = GlesDraw::ZERO;
    d.target_gpu = 0x1000;
    d.target_format = fmts[0];
    d.target_w = 128;
    d.target_h = 64;
    d.num_targets = fmts.len() as u8;
    d.index_count = 3;
    d.color_write = true;
    d.color_mask = 0xf;
    for (i, &f) in fmts.iter().enumerate() {
        d.targets[i] = GlesDrawTarget { gpu: 0x1000 * (i as u64 + 1), w: 128, h: 64, format: f, out_shift: 0 };
    }
    d
}

fn dwords(fmts: &[u8]) -> Vec<u8> {
    fmts.iter().map(|&f| surf_out_dwords(f) as u8).collect()
}

/// What `mrt_layout::layout` (the walk with tile buffers) makes of a list
/// the registers refuse.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Spill {
    /// The last attachment in tile buffer 0, emitted by `eot3_tb`.
    Eot3Tb,
    /// No end-of-tile program was captured for the spill.
    NoProgram,
}

/// `fragment_out.random.<n>`: the attachments and the spill of the list.
const PAST_FOUR_REGISTERS: &[(u32, &[u8], Spill)] = &[
    (9, &[GLES_SURF_R32I, GLES_SURF_R16I, GLES_SURF_RGBA32UI], Spill::Eot3Tb),
    (11, &[GLES_SURF_RG32I, GLES_SURF_RGBA32I], Spill::NoProgram),
    (18, &[GLES_SURF_R32UI, GLES_SURF_RGBA32I], Spill::NoProgram),
    (31, &[GLES_SURF_RG8UI, GLES_SURF_RGBA32UI, GLES_SURF_RG32I], Spill::NoProgram),
    (50, &[GLES_SURF_R16UI, GLES_SURF_RGBA16UI, RG8], Spill::Eot3Tb),
    (53, &[GLES_SURF_RG8I, GLES_SURF_RGBA32UI], Spill::NoProgram),
    (62, &[GLES_SURF_R16I, GLES_SURF_RG8I, GLES_SURF_RGBA32I], Spill::Eot3Tb),
    (85, &[GLES_SURF_RGBA8UI, GLES_SURF_RGBA16UI, GLES_SURF_RG8UI], Spill::Eot3Tb),
    (89, &[GLES_SURF_R32UI, GLES_SURF_RGBA32UI, GLES_SURF_RG16UI], Spill::NoProgram),
    (92, &[GLES_SURF_RGBA16I, GLES_SURF_RGBA32I], Spill::NoProgram),
    (97, &[GLES_SURF_RG8I, GLES_SURF_RG32I, GLES_SURF_RGBA8UI], Spill::Eot3Tb),
];

/// `fragment_out.random.<n>`: an SRGB8_ALPHA8 attachment in the mix.
const SRGB_IN_A_MIX: &[(u32, &[u8])] = &[
    (1, &[GLES_SURF_RGBA8I, SRGB, GLES_SURF_RGB10_A2]),
    (64, &[GLES_SURF_RG8I, SRGB]),
    (69, &[SRGB, GLES_SURF_RG16UI]),
    /* four attachments */
    (17, &[GLES_SURF_R16I, SRGB, GLES_SURF_RG32UI, GLES_SURF_RGBA8UI]),
    (67, &[SRGB, GLES_SURF_RG16UI, GLES_SURF_RGB10_A2UI, GLES_SURF_R16I]),
    (86, &[RGB5_A1, SRGB, GLES_SURF_RG32I, GLES_SURF_R16I]),
    (96, &[GLES_SURF_R8I, RGB565, GLES_SURF_RGB10_A2UI, SRGB]),
];

/// `fragment_out.random.<n>` with four attachments, an integer one among
/// them and no sRGB one, and whether the list fits the four registers.
const FOUR_IN_A_MIX: &[(u32, &[u8], bool)] = &[
    (0, &[GLES_SURF_R32UI, GLES_SURF_RGBA32I, GLES_SURF_RGBA8I, GLES_SURF_RGB10_A2], false),
    (21, &[GLES_SURF_RG8I, GLES_SURF_RG32I, GLES_SURF_RG16I, GLES_SURF_RG16I], false),
    (24, &[RGBA4, GLES_SURF_R8UI, GLES_SURF_RG16I, GLES_SURF_RG16UI], true),
    (29, &[GLES_SURF_R16UI, GLES_SURF_RGBA32UI, GLES_SURF_RG32UI, GLES_SURF_RGBA16UI], false),
    (33, &[RGB8, GLES_SURF_RG16UI, RGB5_A1, GLES_SURF_RGBA16UI], false),
    (41, &[GLES_SURF_RG8UI, GLES_SURF_RGBA8UI, GLES_SURF_RGBA16I, GLES_SURF_R32I], false),
    (55, &[RGBA4, GLES_SURF_RG16I, GLES_SURF_R8I, GLES_SURF_RGBA32I], false),
    (56, &[GLES_SURF_RGBA8UI, GLES_SURF_RG8UI, GLES_SURF_RG8UI, GLES_SURF_RG32I], false),
    (66, &[RGB8, GLES_SURF_RG32UI, GLES_SURF_R16UI, GLES_SURF_RG32I], false),
    (70, &[GLES_SURF_RGBA8UI, GLES_SURF_R16UI, RGB5_A1, GLES_SURF_RGB10_A2], false),
    (71, &[RG16F, GLES_SURF_RGBA16I, RG16F, GLES_SURF_R16F], false),
    (72, &[GLES_SURF_RG16I, GLES_SURF_RGBA16I, GLES_SURF_RG32I, GLES_SURF_R16I], false),
    (82, &[GLES_SURF_R32UI, R8, GLES_SURF_RGB10_A2, GLES_SURF_RGBA16F], false),
    (88, &[RGBA4, GLES_SURF_R8I, GLES_SURF_RGBA16I, GLES_SURF_RG32I], false),
    (90, &[GLES_SURF_R16I, GLES_SURF_RGBA32UI, GLES_SURF_R16I, GLES_SURF_RGBA16I], false),
    (94, &[GLES_SURF_RGBA8888, GLES_SURF_R32I, GLES_SURF_R8I, GLES_SURF_RGBA8888], true),
];

#[test]
fn lists_past_the_four_output_registers_are_refused_by_the_budget() {
    for &(case, fmts, spill) in PAST_FOUR_REGISTERS {
        let d = draw(fmts);
        let dw = dwords(fmts);
        assert_eq!(int_targets_drawn(&d, None), Err(REFUSE_FOUR_REGISTERS), "random.{case}");
        assert_eq!(draw_target_keys(&d), Err(REFUSE_FOUR_REGISTERS), "random.{case}");
        /* the budget rule itself: the registers refuse the list */
        assert!(
            matches!(register_layout(&dw), Err(LayoutError::OutputRegs { .. })),
            "random.{case}: {dw:?} gives {:?}",
            register_layout(&dw)
        );
        /* and what the tile buffers would make of it */
        let with_tb = match layout(&dw) {
            Ok(l) => {
                assert_eq!((l.program, l.tile_buffers), (EotProgram::Eot3Tb, 1), "random.{case}");
                Spill::Eot3Tb
            }
            Err(LayoutError::NoProgram { .. }) => Spill::NoProgram,
            Err(e) => panic!("random.{case}: {dw:?} gives {e:?}"),
        };
        assert_eq!(with_tb, spill, "random.{case}");
    }
}

#[test]
fn an_srgb_attachment_in_a_mix_is_refused_by_name() {
    for &(case, fmts) in SRGB_IN_A_MIX {
        let d = draw(fmts);
        assert_eq!(int_targets_drawn(&d, None), Err(REFUSE_SRGB), "random.{case}");
        assert_eq!(draw_mixed_key(&d), Err(REFUSE_SRGB), "random.{case}");
    }
}

#[test]
fn a_fourth_draw_buffer_in_a_mix_is_refused_by_the_key() {
    for &(case, fmts, fits) in FOUR_IN_A_MIX {
        let d = draw(fmts);
        assert_eq!(fmts.len(), 4, "random.{case}");
        assert_eq!(int_targets_drawn(&d, None), Err(REFUSE_MIX_TARGETS), "random.{case}");
        assert_eq!(draw_mixed_key(&d), Err(REFUSE_MIX_TARGETS), "random.{case}");
        /* what the budget says of the list, behind the key's answer */
        assert_eq!(register_layout(&dwords(fmts)).is_ok(), fits, "random.{case}: {:?}", dwords(fmts));
    }
    /* the two that fit are four one-dword attachments in registers 0-3 */
    let fitting = FOUR_IN_A_MIX.iter().filter(|r| r.2).map(|r| r.0).collect::<Vec<_>>();
    assert_eq!(fitting, [24, 94]);
}

/// A program of the list's mixed key spanning the registers its layout
/// takes: what the GL core hands the draw when the compiler built the
/// variant.
fn variant_of(d: &GlesDraw) -> ShcShader {
    let (keys, n) = draw_target_keys(d).expect("the list has per-target keys");
    let (_, span) = shc_mixed_out_layout(&keys[..n]).expect("the list fits the registers");
    let mut fs = ShcShader::ZERO;
    fs.blend_key = draw_mixed_key(d).expect("the list is a mix");
    fs.num_outputs = span;
    fs
}

#[test]
fn a_mix_inside_the_budget_is_drawn_with_its_variant() {
    /* fragment_out.random 2, 12, 16, 36 and 61, which the same run ends
     * without a GL error: two and three attachments within four registers
     */
    let inside: &[&[u8]] = &[
        &[RGB565, GLES_SURF_RGBA8UI],
        &[GLES_SURF_RG32I, GLES_SURF_R8I, RGB8],
        &[GLES_SURF_RGB10_A2UI, GLES_SURF_R16I, GLES_SURF_RGBA8I],
        &[GLES_SURF_R8UI, GLES_SURF_R16UI, GLES_SURF_R16F],
        &[GLES_SURF_R32I, GLES_SURF_RGBA16F],
    ];
    for fmts in inside {
        let d = draw(fmts);
        assert!(register_layout(&dwords(fmts)).is_ok(), "{fmts:?}");
        let fs = variant_of(&d);
        assert_eq!(int_targets_drawn(&d, Some(&fs)), Ok(()), "{fmts:?}");
        /* without the variant the draw is refused, never drawn with
         * another program's packing */
        assert_eq!(int_targets_drawn(&d, None), Err(REFUSE_NOT_A_MIX), "{fmts:?}");
        let mut other = fs;
        other.num_outputs += 1;
        assert_eq!(int_targets_drawn(&d, Some(&other)), Err(REFUSE_NOT_A_MIX), "{fmts:?}");
    }
}
