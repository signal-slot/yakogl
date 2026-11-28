// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! `src/vivante/tex_layout.rs`'s texture words against etnaviv's, for the
//! textures glmark2's terrain, shadow and refract sample:
//!
//! * terrain: the height and normal maps, 256x256 RGBA8, 1 level
//!   (docs/notes/gc-terrain-tiled-copy.md:16-17, gc-slow-terrain.md:84-85);
//! * shadow: the 1600x1200 depth texture, resolved to 4 bytes a texel
//!   (gc-slow-shadow-refract.md:158), 1 level;
//! * refract: the 1600x1200 RGBA8 colour target with its whole chain of 11
//!   levels (gc-slow-shadow-refract.md:143-144), and `nasa1.png`, 512x512
//!   RGBA8 (glmark2 `data/textures/nasa1.png`, `scene-refract.cpp:79`).
//!
//! **Where the reference is.** `third_party/` does not hold etnaviv
//! (`third_party/mesa-26.2.1` has no `etnaviv_*`, `git grep -il etnaviv --
//! third_party` finds nothing), so the rules below cite Mesa 26.2.1's
//! `src/gallium/drivers/etnaviv/` (the tree of the farm's dependency cache,
//! `mesa-pan/mesa-26.2.1`), file:line as read there. They are written out in
//! this test and not shared with `tex_layout.rs`.
//!
//! * `etnaviv_resource.c:327` padded width = `align(width, paddingX)`; `:328`
//!   padded height; `:329` stride = padded width * bytes a texel; `:330` level
//!   offset = the running size; `:331` layer stride = stride * padded height;
//!   `:338` the running size grows by `align(size, ETNA_PE_ALIGNMENT)`
//!   (`etnaviv_internal.h:59`, 64); `:340-341` the next level is
//!   `u_minify(.., 1)`.
//! * `etnaviv_resource.c:384-387` TILED: paddingX 16 (4 without `rs_align`),
//!   paddingY 4, HALIGN SIXTEEN (FOUR); `:389-392` SUPER_TILED: 64x64, HALIGN
//!   SUPER_TILED. `rs_align` is true on this core: the GC7000UL has no BLT and
//!   TEXTURE_HALIGN (`:361-363`).
//! * `etnaviv_texture_desc.c:180-182` ADDRESSING_MODE is set only for a LINEAR
//!   resource (a tiled one leaves it 0); `:186` CONFIG1 HALIGN is the
//!   resource's; `:190` LINEAR_STRIDE is level 0's stride; `:192` SLICE is
//!   level 0's layer stride; `:204-205` the LOD address is the base plus the
//!   level's offset.
//!
//! LINEAR is not compared: `tex_layout` deliberately lays a linear chain out
//! as `texture.rs` does, not as Mesa (tex_layout.rs module doc).
//!
//! Run: `cargo test --no-default-features --features host-test,gpu-vivante
//! --test gc_tex_layout_etnaviv`.

#![cfg(all(feature = "host-test", feature = "gpu-vivante"))]

use yakogl::vivante::tex_layout::*;

const BPP: u32 = 4;

/// (name, width, height, levels)
const TEXTURES: [(&str, u32, u32, u32); 6] = [
    ("terrain height map", 256, 256, 1),
    ("terrain normal map", 256, 256, 1),
    ("shadow depth texture", 1600, 1200, 1),
    ("refract colour chain", 1600, 1200, 11),
    ("refract nasa1", 512, 512, 1),
    ("refract nasa1 mipped", 512, 512, 10),
];

fn align(v: u32, a: u32) -> u32 {
    v.div_ceil(a) * a
}

fn minify(v: u32) -> u32 {
    (v >> 1).max(1)
}

struct Mesa {
    halign: u32,
    stride0: u32,
    layer_stride0: u32,
    /// (offset, stride) per level
    levels: Vec<(u64, u32)>,
}

/// `etna_layout_multiple` (:384-392) and `setup_miptree` (:327-342).
fn mesa(w: u32, h: u32, n: u32, layout: Layout, rs_align: bool) -> Mesa {
    let (px, py, halign) = match layout {
        Layout::Tiled => (if rs_align { 16 } else { 4 }, 4, if rs_align { 1 } else { 0 }),
        Layout::SuperTiled => (64, 64, 2),
        Layout::Linear => unreachable!("not compared"),
    };
    let (mut lw, mut lh, mut size) = (w, h, 0u64);
    let mut levels = vec![];
    let (mut stride0, mut ls0) = (0, 0);
    for l in 0..n {
        let stride = align(lw, px) * BPP; // :327, :329
        let layer_stride = stride * align(lh, py); // :328, :331
        if l == 0 {
            (stride0, ls0) = (stride, layer_stride);
        }
        levels.push((size, stride)); // :330
        size += (layer_stride as u64).next_multiple_of(64); // :338
        lw = minify(lw); // :340
        lh = minify(lh); // :341
    }
    Mesa { halign, stride0, layer_stride0: ls0, levels }
}

fn input(w: u32, h: u32, n: u32, layout: Layout, rs_align: bool) -> Input {
    Input { width: w, height: h, levels: n, bpp: BPP, layout, ts: Ts::None, rs_align, stride_texels: 0, base: 0 }
}

#[test]
fn words_match_etnaviv_for_glmark2_textures() {
    for (name, w, h, n) in TEXTURES {
        for (layout, rs) in [(Layout::Tiled, true), (Layout::Tiled, false), (Layout::SuperTiled, true)] {
            let m = mesa(w, h, n, layout, rs);
            let t = tex_words(&input(w, h, n, layout, rs)).unwrap();
            let ctx = format!("{name} {layout:?} rs_align {rs}");
            // desc.c:180-182: tiled leaves ADDRESSING_MODE 0.
            assert_eq!(t.config0_addressing, 0, "{ctx}: addressing");
            // desc.c:186
            assert_eq!(t.config1_halign, m.halign << HALIGN_SHIFT, "{ctx}: halign");
            assert_eq!(t.linear_stride, m.stride0, "{ctx}: LINEAR_STRIDE (desc.c:190)");
            assert_eq!(t.slice, m.layer_stride0, "{ctx}: SLICE (desc.c:192)");
            for (l, (off, stride)) in m.levels.iter().enumerate() {
                assert_eq!(t.level_offset[l], *off, "{ctx}: level {l} offset (desc.c:205)");
                assert_eq!(t.level_stride[l], *stride, "{ctx}: level {l} stride");
            }
            assert_eq!(t.align.base, 64, "{ctx}: ETNA_PE_ALIGNMENT");
        }
    }
}

/// Hand-worked values from the same rules, so a slip in the transcription
/// above shows too (resource.c:327-341, 384-392).
#[test]
fn worked_values() {
    // 256x256 RGBA8 TILED rs_align: no padding; stride 1024, layer 262144.
    let t = tex_words(&input(256, 256, 1, Layout::Tiled, true)).unwrap();
    assert_eq!((t.linear_stride, t.slice, t.size), (1024, 262_144, 262_144));
    assert_eq!(t.config1_halign, 1 << HALIGN_SHIFT);
    // 1600x1200 TILED: 1600 % 16 == 0, 1200 % 4 == 0; stride 6400.
    let t = tex_words(&input(1600, 1200, 1, Layout::Tiled, true)).unwrap();
    assert_eq!((t.linear_stride, t.slice), (6400, 7_680_000));
    // 1600x1200 SUPER_TILED: 1600 is a multiple of 64, 1200 pads to 1216.
    let t = tex_words(&input(1600, 1200, 1, Layout::SuperTiled, true)).unwrap();
    assert_eq!((t.linear_stride, t.slice), (6400, 7_782_400));
    assert_eq!(t.config1_halign, 2 << HALIGN_SHIFT);
    // The refract chain, TILED: level 1 is 800x600 at 7,680,000 (already on 64).
    let t = tex_words(&input(1600, 1200, 11, Layout::Tiled, true)).unwrap();
    assert_eq!(t.level_offset[1], 7_680_000);
    assert_eq!(t.level_offset[2], 7_680_000 + 800 * 4 * 600);
    // Level 10 is 1x1, padded to 16x4: 64 bytes.
    assert_eq!(t.level_stride[10], 64);
}
