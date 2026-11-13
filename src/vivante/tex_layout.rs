// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GC7000 (i.MX8MP): the texture descriptor words of a 2D texture in the
//! linear, 4x4-tiled and 64x64 super-tiled layouts, as one tested pure
//! function (docs/notes/gc-tiled-wire.md steps W10 and W11;
//! gc-slow-terrain-stream-diff.md, rank 7). **Not wired**: nothing in the
//! backend calls it yet.
//!
//! Self-contained on purpose: `core` only, no crate imports, so a host test
//! can include it with `#[path]` (tests/gc_tex_layout.rs, as tests/gc_layout.rs
//! does for the address maps). The register values below are numeric copies;
//! the test file checks each one against `src/vivante/regs.rs`.
//!
//! # Sources
//!
//! Mesa 26.2.1 etnaviv, `src/gallium/drivers/etnaviv/`. The tree here does not
//! vendor etnaviv (`third_party/mesa-26.2.1` holds no `etnaviv_*`), so the
//! line numbers are the ones the repository's notes already cite, not
//! re-read here:
//!
//! - `etnaviv_resource.c:327-332` (`setup_miptree`): a level's padded width is
//!   `align(width, paddingX)`, its stride `padded_width * bpp`, its layer
//!   stride `stride * align(height, paddingY)`;
//!   `:351-352` levels start on `ETNA_PE_ALIGNMENT` (64) bytes;
//!   `:380-393` (`etna_layout_multiple`): paddings 16x4 (TILED when the
//!   resolve engine may touch it, `rs_align`) or 4x4, and 64x64 SUPER_TILED;
//!   `:382-388` and `:577-581` (`etna_resource_alloc`): the layout picks the
//!   resource's `halign`: TILED is HALIGN_SIXTEEN with `rs_align` and
//!   HALIGN_FOUR without, SUPER_TILED is HALIGN_SUPER_TILED, LINEAR keeps the
//!   zero FOUR.
//! - `etnaviv_texture_state.c:271-273` / `etnaviv_texture_desc.c:183-186`:
//!   CONFIG0's ADDRESSING_MODE is LINEAR for a linear resource and TILED for
//!   both tiled ones (super-tiling is told by HALIGN, not by addressing:
//!   golden CONFIG1 0x08321000 / 0x08521000 carry HALIGN 2,
//!   gc-slow-terrain-stream-diff.md:165); CONFIG1's HALIGN is the resource's.
//! - `etnaviv_texture.c:80-159` (`etnaviv_can_use_sampler_ts`, level 0 only,
//!   `:151-156`): the sampler reads the colour or depth TS itself, with the
//!   TS clear value, for a view of level 0 alone.
//! - `etnaviv_tiling.h`: the tile sizes (4x4, super-tile 64x64).
//!
//! # Two level placements
//!
//! The function lays a chain out two ways, because the repository has two:
//!
//! - **Linear** is `texture.rs`'s, byte for byte (`build_descriptor`,
//!   texture.rs:376-447): the GL core owns the pitch (`stride_texels`), one
//!   LINEAR_STRIDE is `stride_texels * bpp` for every level, a level is
//!   `stride * its own height` bytes, SLICE is `stride * height` with no
//!   padding, each level starts on 64 bytes, HALIGN is FOUR (0), and no row or
//!   height padding is added. It is *not* Mesa's linear (which pads 16x4 for
//!   the RS and gives each level its own stride): the GL core's storage is
//!   what the sampler reads, and a width or height padding would read
//!   outside it.
//! - **Tiled and super-tiled** are Mesa's `setup_miptree`: each level has its
//!   own padded width and height (`level_stride`), the descriptor's
//!   LINEAR_STRIDE and SLICE are level 0's, and each level starts on 64
//!   bytes. `stride_texels` of 0 takes Mesa's padded width; a non-zero one is
//!   level 0's existing pitch and must already satisfy the alignment.
//!
//! # TS
//!
//! `Ts::Sampler { fill }` says the sampler reads the surface through its tile
//! status; it needs a tiled layout and one level (`:151-156`). The result
//! carries the clear value for TS_SAMPLER_CLEAR_VALUE and `ts_enable` for
//! TX_CTRL. The TS_SAMPLER_CONFIG and TX_CTRL bit layouts are not in
//! `regs.rs` (and not measured beyond the golden words of
//! gc-slow-stream-diff.md:195-215), so they are left to the wiring.

/// CONFIG0 ADDRESSING_MODE: field shift (`regs.rs` `texdesc_addressing_mode`).
pub const ADDRESSING_SHIFT: u32 = 20;
/// CONFIG0 ADDRESSING_MODE: field mask before the shift (`texdesc_addressing_mode`).
pub const ADDRESSING_MASK: u32 = 0x3;
/// `GC_TEX_ADDRESSING_TILED` in `regs.rs`.
pub const ADDRESSING_TILED: u32 = 0;
/// `GC_TEX_ADDRESSING_LINEAR` in `regs.rs`.
pub const ADDRESSING_LINEAR: u32 = 3;
/// CONFIG1 HALIGN: field shift (`regs.rs` `texdesc_halign`).
pub const HALIGN_SHIFT: u32 = 26;
/// CONFIG1 HALIGN: field mask before the shift (`texdesc_halign`).
pub const HALIGN_MASK: u32 = 0x7;
/// `GC_TEXTURE_HALIGN_FOUR` in `regs.rs`.
pub const HALIGN_FOUR: u32 = 0;
/// `GC_TEXTURE_HALIGN_SIXTEEN` in `regs.rs`.
pub const HALIGN_SIXTEEN: u32 = 1;
/// TEXTURE_HALIGN_SUPER_TILED: **not in `regs.rs`**; 2, from the golden
/// CONFIG1 words 0x08321000 / 0x08521000 (gc-slow-terrain-stream-diff.md:165).
pub const HALIGN_SUPER_TILED: u32 = 2;
/// `GC_TEXDESC_LOD_ALIGN` in `regs.rs`: every level is fetched from here.
pub const LOD_ALIGN: u64 = 64;
/// Descriptor LOD address slots (`texdesc_lod_addr`: 16 of them).
pub const MAX_LEVELS: usize = 16;

/// Pixels along one side of a 4x4 tile (etnaviv_tiling.h).
pub const TILE: u32 = 4;
/// Pixels along one side of a super-tile (etnaviv_tiling.h).
pub const SUPER_TILE: u32 = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layout {
    Linear,
    Tiled,
    SuperTiled,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ts {
    None,
    /// The sampler reads through the TS; `fill` is its clear value.
    Sampler {
        fill: u32,
    },
}

/// What a layout needs of the surface it describes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Alignment {
    /// Row padding in texels (a tiled layout's pitch is a multiple of it).
    pub width: u32,
    /// Height padding in texels.
    pub height: u32,
    /// Byte alignment of the surface and of every level.
    pub base: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Input {
    pub width: u32,
    pub height: u32,
    pub levels: u32,
    pub bpp: u32,
    pub layout: Layout,
    pub ts: Ts,
    /// Mesa's `rs_align`: the resolve engine may touch the surface (TILED
    /// only: 16x4 padding and HALIGN SIXTEEN instead of 4x4 and FOUR).
    pub rs_align: bool,
    /// Level 0's pitch in texels. Linear: the GL core's, required. Tiled:
    /// 0 for Mesa's padded width, or an existing pitch.
    pub stride_texels: u32,
    /// GPU address the chain starts at; must be on [`Alignment::base`].
    pub base: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// A zero size, texel size or stride where one is needed.
    Empty,
    /// `levels` is 0 or above [`MAX_LEVELS`].
    Levels,
    /// A pitch below the row, or (tiled) not a multiple of the width padding.
    Stride,
    /// `base` is not on [`Alignment::base`].
    Base,
    /// The sampler TS needs a tiled layout.
    TsLinear,
    /// The sampler TS reads level 0 only.
    TsMipped,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Words {
    /// CONFIG0's ADDRESSING_MODE bits, shifted into place.
    pub config0_addressing: u32,
    /// CONFIG1's HALIGN bits, shifted into place.
    pub config1_halign: u32,
    /// The LINEAR_STRIDE word, bytes (level 0's).
    pub linear_stride: u32,
    /// The SLICE word, bytes (level 0's layer step).
    pub slice: u32,
    /// Each level's byte offset from `base`, the LOD address words minus it.
    pub level_offset: [u64; MAX_LEVELS],
    /// Each level's row pitch in bytes (Linear: all `linear_stride`).
    pub level_stride: [u32; MAX_LEVELS],
    /// Bytes from `base` to the end of the last level.
    pub size: u64,
    pub align: Alignment,
    /// TX_CTRL's TS_ENABLE wanted, and the TS_SAMPLER_CLEAR_VALUE.
    pub ts_enable: bool,
    pub ts_clear_value: u32,
}

const fn align_up(v: u32, to: u32) -> u32 {
    v.div_ceil(to) * to
}

const fn minify(v: u32, l: u32) -> u32 {
    let s = v >> l;
    if s == 0 {
        1
    } else {
        s
    }
}

/// What `layout` needs (`etna_layout_multiple`, `etnaviv_resource.c:380-393`).
/// Linear takes the GL core's pitch and pads nothing (see the module doc).
pub const fn alignment(layout: Layout, rs_align: bool) -> Alignment {
    match layout {
        Layout::Linear => Alignment { width: 1, height: 1, base: LOD_ALIGN },
        Layout::Tiled => Alignment { width: if rs_align { 16 } else { TILE }, height: TILE, base: LOD_ALIGN },
        Layout::SuperTiled => Alignment { width: SUPER_TILE, height: SUPER_TILE, base: LOD_ALIGN },
    }
}

/// The descriptor words and level placement for `i`.
pub const fn tex_words(i: &Input) -> Result<Words, Error> {
    if i.width == 0 || i.height == 0 || i.bpp == 0 {
        return Err(Error::Empty);
    }
    if i.levels == 0 || i.levels as usize > MAX_LEVELS {
        return Err(Error::Levels);
    }
    let a = alignment(i.layout, i.rs_align);
    if !i.base.is_multiple_of(a.base) {
        return Err(Error::Base);
    }
    let linear = matches!(i.layout, Layout::Linear);
    let pad_w = align_up(i.width, a.width);
    if linear {
        if i.stride_texels == 0 {
            return Err(Error::Empty);
        }
        if i.stride_texels < i.width {
            return Err(Error::Stride);
        }
    } else if i.stride_texels != 0 && (i.stride_texels < pad_w || !i.stride_texels.is_multiple_of(a.width)) {
        return Err(Error::Stride);
    }
    let (ts_enable, ts_clear_value) = match i.ts {
        Ts::None => (false, 0),
        Ts::Sampler { fill } => {
            if linear {
                return Err(Error::TsLinear);
            }
            if i.levels != 1 {
                return Err(Error::TsMipped);
            }
            (true, fill)
        }
    };
    let (addressing, halign) = match i.layout {
        Layout::Linear => (ADDRESSING_LINEAR, HALIGN_FOUR),
        Layout::Tiled => (ADDRESSING_TILED, if i.rs_align { HALIGN_SIXTEEN } else { HALIGN_FOUR }),
        Layout::SuperTiled => (ADDRESSING_TILED, HALIGN_SUPER_TILED),
    };
    let mut w = Words {
        config0_addressing: (addressing & ADDRESSING_MASK) << ADDRESSING_SHIFT,
        config1_halign: (halign & HALIGN_MASK) << HALIGN_SHIFT,
        linear_stride: 0,
        slice: 0,
        level_offset: [0; MAX_LEVELS],
        level_stride: [0; MAX_LEVELS],
        size: 0,
        align: a,
        ts_enable,
        ts_clear_value,
    };
    let mut at: u64 = 0;
    let mut l = 0;
    while l < i.levels {
        at = (at + LOD_ALIGN - 1) & !(LOD_ALIGN - 1);
        let lh = minify(i.height, l);
        let (stride, bytes) = if linear {
            let s = i.stride_texels * i.bpp;
            (s, s as u64 * lh as u64)
        } else {
            let pw =
                if l == 0 && i.stride_texels != 0 { i.stride_texels } else { align_up(minify(i.width, l), a.width) };
            let s = pw * i.bpp;
            (s, s as u64 * align_up(lh, a.height) as u64)
        };
        w.level_offset[l as usize] = at;
        w.level_stride[l as usize] = stride;
        if l == 0 {
            w.linear_stride = stride;
            w.slice = if linear { stride * i.height } else { stride * align_up(i.height, a.height) };
        }
        at += bytes;
        l += 1;
    }
    w.size = at;
    Ok(w)
}

/// Bytes a texel of the tiled chain takes: RGBA8.
pub const CHAIN_BPP: u32 = 4;

/// One level of a tiled chain.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ChainLevel {
    /// Bytes from the chain's base, on [`LOD_ALIGN`].
    pub offset: u64,
    /// Bytes of the level: its stride times its height padded to 4.
    pub size: u64,
    /// Row pitch in bytes: the width padded to 16, times 4.
    pub stride: u32,
}

/// A 2D RGBA8 chain laid out TILED with 16x4 padding (`rs_align`, HALIGN
/// SIXTEEN), each level on 64 bytes: `tiled_copy::tiled_layout`'s rule with
/// the resolve engine's padding (docs/notes/gc-tiled-wire.md, W11a).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Chain {
    pub level: [ChainLevel; MAX_LEVELS],
    pub levels: u32,
    /// Bytes from the base to the end of the last level.
    pub size: u64,
}

/// The level offsets and sizes of a tiled RGBA8 chain of `levels` levels of
/// `width` x `height`, each level `max(1, dim >> l)` and padded to 16x4.
pub const fn tiled_chain(width: u32, height: u32, levels: u32) -> Result<Chain, Error> {
    if width == 0 || height == 0 {
        return Err(Error::Empty);
    }
    if levels == 0 || levels as usize > MAX_LEVELS {
        return Err(Error::Levels);
    }
    let a = alignment(Layout::Tiled, true);
    let mut c = Chain { level: [ChainLevel { offset: 0, size: 0, stride: 0 }; MAX_LEVELS], levels, size: 0 };
    let mut at: u64 = 0;
    let mut l = 0;
    while l < levels {
        at = (at + LOD_ALIGN - 1) & !(LOD_ALIGN - 1);
        let stride = align_up(minify(width, l), a.width) * CHAIN_BPP;
        let size = stride as u64 * align_up(minify(height, l), a.height) as u64;
        c.level[l as usize] = ChainLevel { offset: at, size, stride };
        at += size;
        l += 1;
    }
    c.size = at;
    Ok(c)
}

/// The descriptor words sampling [`tiled_chain`]'s chain at `base`: TILED,
/// HALIGN SIXTEEN, level `l` at `base + level_offset[l]`. `tex_words`' own
/// path, so the levels agree with [`tiled_chain`].
pub const fn tiled_chain_words(width: u32, height: u32, levels: u32, base: u64) -> Result<Words, Error> {
    tex_words(&Input {
        width,
        height,
        levels,
        bpp: CHAIN_BPP,
        layout: Layout::Tiled,
        ts: Ts::None,
        rs_align: true,
        stride_texels: 0,
        base,
    })
}

#[cfg(test)]
#[path = "layout.rs"]
mod layout_ref;

#[cfg(test)]
mod chain_tests {
    use super::layout_ref as lr;
    use super::*;

    const SIZES: [(u32, u32); 6] = [(256, 256), (800, 600), (1, 1), (13, 7), (333, 101), (17, 1000)];

    fn full(w: u32, h: u32) -> u32 {
        w.max(h).ilog2() + 1
    }

    fn check(w: u32, h: u32, levels: u32) {
        let c = tiled_chain(w, h, levels).unwrap();
        let mut at = 0u64;
        for l in 0..levels {
            let (lw, lh) = ((w >> l).max(1), (h >> l).max(1));
            let size = lr::size(lr::Layout::Tiled, lw, lh, 4, true) as u64;
            let stride = lr::stride(lr::Layout::Tiled, lw, 4, true);
            at = at.div_ceil(64) * 64;
            let e = c.level[l as usize];
            assert_eq!((e.offset, e.size, e.stride), (at, size, stride), "{w}x{h} level {l}");
            // the last texel of the padded level ends the level
            let pw = lr::aligned_width(lr::Layout::Tiled, lw, true);
            let ph = lr::aligned_height(lr::Layout::Tiled, lh, true);
            let end = lr::offset(lr::Layout::Tiled, pw - 1, ph - 1, 4, stride) as u64 + 4;
            assert_eq!(end, size, "{w}x{h} level {l}: offset() ends at the size");
            at += size;
        }
        assert_eq!(c.size, at);
    }

    #[test]
    fn chain_matches_layout_rs() {
        for (w, h) in SIZES {
            for levels in [1, 3.min(full(w, h)), full(w, h)] {
                check(w, h, levels);
            }
        }
    }

    #[test]
    fn words_agree_with_chain() {
        for (w, h) in SIZES {
            let n = full(w, h);
            let (c, t) = (tiled_chain(w, h, n).unwrap(), tiled_chain_words(w, h, n, 0x1000).unwrap());
            for l in 0..n as usize {
                assert_eq!(t.level_offset[l], c.level[l].offset);
                assert_eq!(t.level_stride[l], c.level[l].stride);
            }
            assert_eq!(t.size, c.size);
        }
    }

    #[test]
    fn three_level_words() {
        // 800x600, 400x300, 200x150 -> pitches 800, 400, 208 texels
        let t = tiled_chain_words(800, 600, 3, 0x4000_0000).unwrap();
        assert_eq!(t.config0_addressing, ADDRESSING_TILED << ADDRESSING_SHIFT);
        assert_eq!(t.config1_halign, HALIGN_SIXTEEN << HALIGN_SHIFT);
        assert_eq!(t.align, Alignment { width: 16, height: 4, base: 64 });
        assert_eq!((t.linear_stride, t.slice), (3200, 3200 * 600));
        assert_eq!(&t.level_stride[..3], &[3200, 1600, 832]);
        assert_eq!(t.level_offset[0], 0);
        assert_eq!(t.level_offset[1], 3200 * 600);
        assert_eq!(t.level_offset[2], 3200 * 600 + 1600 * 300);
        assert_eq!(t.size, 3200 * 600 + 1600 * 300 + 832 * 152);
        assert!(!t.ts_enable);
        assert!(t.level_offset[..3].iter().all(|o| o % LOD_ALIGN == 0));
        assert_eq!(tiled_chain_words(64, 64, 1, 0x1001), Err(Error::Base));
    }

    #[test]
    fn small_levels_pad_to_16x4() {
        // 1x1 is still a 16x4 block: 256 bytes, and the next level starts there
        let c = tiled_chain(2, 2, 2).unwrap();
        assert_eq!((c.level[0].size, c.level[1].offset, c.level[1].size), (256, 256, 256));
        assert_eq!(c.size, 512);
        assert_eq!(tiled_chain(0, 4, 1), Err(Error::Empty));
        assert_eq!(tiled_chain(4, 4, 0), Err(Error::Levels));
        assert_eq!(tiled_chain(4, 4, 17), Err(Error::Levels));
    }
}
