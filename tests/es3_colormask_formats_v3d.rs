// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Partial glColorMask on formats that lack channels, on the V3D
//! (t-colormask-formats; docs/notes/v3d-fbo-render.md section 6, D4): what
//! `src/v3d/state.rs` makes of a mask for R, RG and RGB targets, float and
//! integer, for a clear (`colour_needs_quad`, `clear_quad_draw`) and for a
//! draw (`job_state`'s `colormask`, the word `COLOR_WRITE_MASKS` is built
//! from).
//!
//! The tests drive `state.rs`'s public helpers with the `GlesDraw` the GL core
//! builds: `target_format` and `target_alpha_bits` as
//! `framebuffer::fb_target_alpha_bits_n` and the format-to-surface mapping give
//! them, `color_mask` / `clear_color_mask` as glColorMask left them (the GL
//! core does not trim a mask to the format's channels). No job builder is
//! needed: `colormask` is a field of the `V3dJobState` the back end hands the
//! job builder unchanged.
//!
//! Two kinds of test. The plain ones pin today's behaviour: the V3D takes
//! only alpha out of the write mask (`st.colormask &= 0x7` when the target
//! has no alpha) and a clear with any channel off is a quad. The `#[ignore]`d
//! ones state what the format's own channels would give (ES 3.0 4.2.2: a
//! channel the format lacks has nothing to write; v3d's render target for
//! `R8`, `RG8`, `R16F`... stores only the channels it has, and the Mesa st
//! clear path decides on the channels of the buffer), and fail today. Mesa's
//! `v3dx_emit.c` and `st_cb_clear.c` are not in `third_party/mesa-26.2.1`
//! (only `v3dx_format_table.c` and the packer), so those expectations come
//! from the section-6 design of the note and the `st_cb_clear.c:440-471`
//! citation in `colour_needs_quad`, not from a line read here.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_colormask_formats_v3d
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-v3d", not(feature = "report-es2")))]

use yakogl::backend::*;
use yakogl::shaderc::ShcShader;
use yakogl::v3d::state::{self, ClearProg, QuadBuffers};

/// A format as the V3D sees it: (name, surface, alpha bits the GL core
/// reports, the channels the GL format has as a mask, bit 0 R .. 3 A).
type Fmt = (&'static str, u8, u8, u8);

const FORMATS: &[Fmt] = &[
    /* held in RGBA8 / RGBA16F, whose missing channels the PBE does not mask */
    ("R8", GLES_SURF_RGBA8888, 0, 0b0001),
    ("RG8", GLES_SURF_RGBA8888, 0, 0b0011),
    ("RGB8", GLES_SURF_RGBA8888, 0, 0b0111),
    ("RG16F", GLES_SURF_RGBA16F, 0, 0b0011),
    ("RGB16F", GLES_SURF_RGBA16F, 0, 0b0111),
    /* their own surfaces */
    ("RGB565", GLES_SURF_RGB565, 0, 0b0111),
    ("R16F", GLES_SURF_R16F, 0, 0b0001),
    ("R32F", GLES_SURF_R32F, 0, 0b0001),
    ("R32UI", GLES_SURF_R32UI, 0, 0b0001),
    ("R8I", GLES_SURF_R8I, 0, 0b0001),
    ("RG16I", GLES_SURF_RG16I, 0, 0b0011),
    ("RG32UI", GLES_SURF_RG32UI, 0, 0b0011),
    /* controls: every channel */
    ("RGBA8", GLES_SURF_RGBA8888, 8, 0b1111),
    ("RGBA16F", GLES_SURF_RGBA16F, 8, 0b1111),
    ("RGBA8UI", GLES_SURF_RGBA8UI, 8, 0b1111),
    /* RGB10_A2: the render surface is behind the GL core's gate, but the
     * V3D side takes it */
    ("RGB10_A2", GLES_SURF_RGB10_A2, 8, 0b1111),
];

fn draw(f: &Fmt) -> GlesDraw {
    GlesDraw {
        target_gpu: 0x0100_0000,
        target_w: 64,
        target_h: 64,
        target_format: f.1,
        target_alpha_bits: f.2,
        samples: 1,
        prim: GLES_PRIM_TRIANGLES,
        index_count: 3,
        line_width: 1.0,
        viewport: [0, 0, 64, 64],
        depth_range: [0.0, 1.0],
        flip_y: true,
        color_mask: 0xf,
        color_write: true,
        depth_func: GLES_CMP_LESS,
        ..GlesDraw::ZERO
    }
}

fn clear(f: &Fmt, mask: u8) -> GlesDraw {
    GlesDraw { do_clear: true, clear_color_mask: mask, index_count: 0, ..draw(f) }
}

/// What a draw writes today: only alpha is taken out, and only for a target
/// with no alpha bits.
fn today_draw(f: &Fmt, mask: u8) -> u32 {
    let m = u32::from(mask);
    if f.2 == 0 {
        m & 0x7
    } else {
        m
    }
}

#[test]
fn every_format_is_drawn_and_has_a_row() {
    for f in FORMATS {
        let d = draw(f);
        assert_eq!(state::refusal(&d), None, "{}", f.0);
        assert!(state::rt_format(f.1).is_some(), "{}", f.0);
    }
}

/// Draw, today: `colormask` is the GL mask with alpha dropped when the
/// target has no alpha, whatever other channels the format lacks. All 16
/// masks of every format.
#[test]
fn draw_colormask_today_takes_only_alpha_out() {
    for f in FORMATS {
        for mask in 0..16u8 {
            let mut d = draw(f);
            d.color_mask = mask;
            let st = state::job_state(&d).unwrap_or_else(|e| panic!("{}: {e}", f.0));
            assert_eq!(st.colormask, today_draw(f, mask), "{} GL mask {mask:#06b}", f.0);
        }
    }
}

/// glColorMask(false..) / colour writes off: 0 for every format.
#[test]
fn draw_with_writes_off_masks_everything() {
    for f in FORMATS {
        let mut d = draw(f);
        d.color_write = false;
        assert_eq!(state::job_state(&d).unwrap().colormask, 0, "{}", f.0);
    }
}

/// A mask of only channels the format lacks writes, today, wherever the
/// store holds the channel: R8 with glColorMask(0,1,0,0) has G on in the
/// job state (the RGBA8 store takes it; ES 3.0 reads G of an R8 buffer as 0,
/// the D4 rows of the Pi 4 `color.blend.r8`, `rg8`, `rg16f`).
#[test]
fn draw_missing_channel_mask_is_not_empty_today() {
    let by_name = |n: &str| FORMATS.iter().find(|f| f.0 == n).unwrap();
    for (n, mask, want) in [
        ("R8", 0b0010u8, 0b0010u32),
        ("R8", 0b0110, 0b0110),
        ("RG8", 0b0100, 0b0100),
        ("RG16F", 0b0100, 0b0100),
        ("R16F", 0b0010, 0b0010),
        ("R32UI", 0b0110, 0b0110),
        ("RG16I", 0b0100, 0b0100),
        /* alpha is the one channel dropped */
        ("R8", 0b1000, 0),
        ("RGB8", 0b1000, 0),
        ("R32UI", 0b1000, 0),
        /* a mask of present channels is kept */
        ("R8", 0b0001, 0b0001),
        ("RG8", 0b0011, 0b0011),
        ("RGB8", 0b0111, 0b0111),
        ("RGB8", 0b0101, 0b0101),
    ] {
        let mut d = draw(by_name(n));
        d.color_mask = mask;
        assert_eq!(state::job_state(&d).unwrap().colormask, want, "{n} {mask:#06b}");
    }
}

/// Draw, as the format's channels would have it: the job state's mask is the
/// GL mask limited to the channels the format has, so `R8` under
/// glColorMask(0,1,1,0) writes nothing.
#[test]
#[ignore = "the V3D takes only alpha out of the write mask (state.rs:670); a format's missing G and B stay writable (v3d-fbo-render.md D4; needs GlesDraw::target_channels from the GL core)"]
fn draw_colormask_limited_to_the_formats_channels() {
    for f in FORMATS {
        for mask in 0..16u8 {
            let mut d = draw(f);
            d.color_mask = mask;
            let st = state::job_state(&d).unwrap();
            assert_eq!(st.colormask, u32::from(mask & f.3), "{} GL mask {mask:#06b}", f.0);
        }
    }
}

/// Clear, today: a quad when the clear is marked `clear_quad` or any of the
/// four channels is off in glColorMask, the format's own channels not
/// considered. All 16 masks (0 never comes from the GL core; the test reads it as a mask with every channel off).
#[test]
fn clear_needs_a_quad_when_any_channel_is_masked_today() {
    for f in FORMATS {
        for mask in 0..16u8 {
            let d = clear(f, mask);
            let want = mask != 0xf; /* 0 too: the GL core never sends it */
            assert_eq!(state::colour_needs_quad(&d), want, "{} clear mask {mask:#06b}", f.0);
        }
        let mut d = clear(f, 0xf);
        d.clear_quad = true;
        assert!(state::colour_needs_quad(&d), "{}: a scissored clear", f.0);
        /* no do_clear: no clear */
        assert!(!state::colour_needs_quad(&GlesDraw { do_clear: false, clear_color_mask: 0b0101, ..draw(f) }));
    }
}

/// A clear quad, today: its job state is a draw's, so the mask reaching the
/// V3D is the clear's with alpha dropped for a target without alpha (the
/// target's alpha bits are copied), and the missing G and B stay writable.
#[test]
fn clear_quad_mask_reaches_the_job_state_like_a_draw_today() {
    /* the program is never run: the helpers only copy its handle */
    let fs = ShcShader::ZERO;
    let p = ClearProg { fs: &fs, code_gpu: 0x0200_0000, location: 0 };
    let verts = state::clear_quad_vertices(1.0);
    let colour = [0.25, 0.5, 0.75, 1.0];
    let b = QuadBuffers { colour: true, depth: false, stencil: false };
    for f in FORMATS {
        for mask in 1..16u8 {
            let d = clear(f, mask);
            let q = state::clear_quad_draw(&d, b, false, &p, &verts, &colour);
            assert!(q.color_write, "{}", f.0);
            assert_eq!(q.color_mask, mask, "{} {mask:#06b}", f.0);
            assert_eq!(q.target_format, f.1);
            let st = state::job_state(&q).unwrap();
            assert_eq!(st.colormask, today_draw(f, mask), "{} clear mask {mask:#06b}", f.0);
        }
        /* a quad that does not clear colour writes none */
        let q = state::clear_quad_draw(
            &clear(f, 0b0101),
            QuadBuffers { colour: false, depth: true, stencil: false },
            false,
            &p,
            &verts,
            &colour,
        );
        assert_eq!(state::job_state(&q).unwrap().colormask, 0, "{}", f.0);
    }
}

/// The TLB clear colour is the format's row, unmasked: for the formats held
/// wide the missing channels carry whatever the clear colour had (the note's
/// alternative design zeroes them; today they are kept).
#[test]
fn tlb_clear_words_keep_the_missing_channels_today() {
    let c = [0.25, 0.5, 0.75, 1.0];
    let w = state::clear_words(GLES_SURF_RGBA8888, c);
    assert_eq!(w[0], state::pack_rgba8(c), "R8/RG8/RGB8 share the RGBA8 word");
    let g = (w[0] >> 8) & 0xff;
    assert_ne!(g, 0, "G of an R8 target is not zeroed by the clear");
    let h = state::clear_words(GLES_SURF_RGBA16F, c);
    assert_ne!(h[0] >> 16, 0, "G of an RG16F-as-RGBA16F target is not zeroed");
}

/// Clear, as the format's channels would have it: a mask that covers every
/// channel the format has is a plain TLB clear (RGB8 under glColorMask(1,1,1,0),
/// R8 under (1,0,0,0)); one that misses a present channel is a quad.
#[test]
#[ignore = "colour_needs_quad tests the four GL channels, not the format's (state.rs:89-91); needs the format's channels from the GL core (v3d-fbo-render.md D4)"]
fn clear_needs_a_quad_only_for_a_present_channel_masked() {
    for f in FORMATS {
        for mask in 1..16u8 {
            let d = clear(f, mask);
            let want = mask & f.3 != f.3;
            assert_eq!(state::colour_needs_quad(&d), want, "{} clear mask {mask:#06b}", f.0);
        }
    }
}
