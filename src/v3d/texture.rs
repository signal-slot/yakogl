// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The V3D's textures, from the GL core's `GlesTextureBind` to what the TMU
//! reads (docs/online-shaderc/v3d-backend.md, "M6 textures", T2): where the
//! GL core keeps each level, the tiled image the TMU samples instead (laid
//! out and tiled by `driver/src/v3d/v3d_tex.c`, Mesa's rules), the texture
//! shader state and the sampler state.
//!
//! The TMU does not sample a raster 2D image (Mesa's v3d_tiling.h: raster is
//! "Not valid as texture inputs"), and the GL core keeps every texture
//! linear, so the backend tiles a copy once per job (`backend.rs`). This file
//! is the part of that with no memory of its own. Its pure rules have host
//! tests here; the layout, the tiling and the record packing are C
//! (`v3d_tex.c`), checked against Mesa by `tools/v3d/tex_check.c` and faked
//! in the backend's tests, as the job builder is. The layered images (a
//! cube's or a 2D array's trees, a 3D texture's stack of slices:
//! docs/notes/v3d-tex-layout-es3.md) are placed here, over the C's one-slice
//! levels, by Mesa's rules; tests/es3_formats_v3d.rs pins them against the
//! real C and Mesa's numbers. The ETC2/EAC formats are laid out in 4x4
//! blocks ([`layout_compressed`], the C's v3d_tex_layout_compressed
//! transcribed and pinned on its numbers here), and a depth binding's
//! sampler record carries GL's compare function ([`compare_function`],
//! `v3d_tex_pack_sampler_compare`).

use super::ffi::*;
use super::state::Refusal;
use crate::backend::*;

/// A row pitch of whole pixel pairs (the GL core's `tex_pitch`).
const fn tex_pitch(w: u32) -> u32 {
    w.wrapping_add(1) & !1
}

/// Level `level` of a one-layer RGBA8 2D texture `width` x `height` in the
/// GL core's storage: (byte offset from `base_gpu`, row pitch in texels,
/// rows). [`gl_level_layered`] with one layer of 4-byte texels.
pub fn gl_level(width: u32, height: u32, level: u32) -> (u32, u32, u32) {
    gl_level_layered(width, height, 1, 4, level)
}

/// Level `level` of a texture of `layers` stacked layers (a cube map's six
/// faces) `width` x `height` of `bpp`-byte texels in the GL core's storage:
/// (byte offset of the level from `base_gpu`, row pitch in texels, rows of
/// ONE layer). Layer `f` of the level starts `f * rows * pitch * bpp` bytes
/// after the level. The pitch in texels is the same for every texel size:
/// the GL core's `chain_layout` multiplies the same texel pitch by the
/// format's bytes (`gles_texfmt_bytes`; L8 and A8 are 1, LA8 2).
///
/// The GL core's `texture.rs` `chain_layout` with `mip_level_align` 0, which
/// the V3D keeps (a common chain pitch would give an FBO attached to a level
/// above 0 the wrong row pitch, since the V3D writes a target at its own
/// width's pitch): the levels packed one after another, each at its own
/// width's pair-padded pitch, except that a 1-wide level above a wider base
/// is one texel a row (`one_wide_level_pitch`). A level holds its layers
/// stacked, `max(h >> level, 1)` rows each, packed: the V3D walks the layers
/// itself (native_layered_textures), so the GL core does not round the
/// stacked height up to a power of two for it (the PowerVR's chain rule,
/// `chain_layout`'s `pot_stack`). A non-power-of-two texture has more than
/// level 0 once glGenerateMipmap or a level above 0 asked for its chain
/// (GL_OES_texture_npot; `texture.rs` `grow_npot_chain`), level `k` being
/// `max(width >> k, 1)` wide as for any other size. Pinned against the GL
/// core's own `level_off`, and the faces' places, by backend tests (with
/// this backend's table installed).
pub fn gl_level_layered(width: u32, height: u32, layers: u32, bpp: u32, level: u32) -> (u32, u32, u32) {
    let pitch = |lw: u32| if lw == 1 && width > 1 { 1 } else { tex_pitch(lw) };
    let mut off = 0u32;
    for k in 0..level {
        let (lw, lh) = ((width >> k).max(1), (height >> k).max(1));
        off += pitch(lw) * lh * layers * bpp;
    }
    let (lw, lh) = ((width >> level).max(1), (height >> level).max(1));
    (off, if level == 0 { tex_pitch(width) } else { pitch(lw) }, lh)
}

/// The levels the draw samples: the whole chain the application filled when
/// the min filter uses mipmaps, level 0 otherwise.
pub fn levels_sampled(b: &GlesTextureBind) -> u32 {
    if b.mip_filter != 0 {
        (b.mip_levels as u32).max(1)
    } else {
        1
    }
}

/// The layers of the sampled image: 6 for a cube map (GL's faces +X, -X,
/// +Y, -Y, +Z, -Z, the order the GL core stacks them in and the V3D's layer
/// order), the binding's `layers` for a 2D array (its layer count) and for a
/// 3D texture (the depth of level 0; `GlesTextureBind::layers`,
/// src/backend.rs), 1 for a 2D texture. It is also the record's Image Depth
/// ([`state`]: v3dx_state.c:935-939) and the slots the GL core stacks at
/// every level ([`gl_level_layered`]).
pub fn layers(b: &GlesTextureBind) -> u32 {
    match b.type_ {
        GLES_TEXTYPE_CUBE => 6,
        GLES_TEXTYPE_2D_ARRAY | GLES_TEXTYPE_3D => u32::from(b.layers).max(1),
        _ => 1,
    }
}

/// The slices Mesa allocates for level `level` of a 3D texture `depth` deep
/// (v3d_setup_slices' `level_depth`, v3d_resource.c:609-611 and :636-639;
/// `v3d_tex_level_depth` in driver/src/v3d/v3d_tex.c is the same rule, which
/// tools/v3d/tex_check.c pins): the depth itself at level 0 (`u_minify(depth,
/// 0)`); from level 1, `pot_depth = 2 * v3d_get_dimension_mpad(depth, 1, 1) =
/// 2 * next_pow2(max(depth >> 1, 1))` minified `level` times, at least 1. So
/// a depth of 6 has 4 slices at level 1 (next_pow2(3)), then 2, 1; a depth of
/// 5 has 2 (next_pow2(2) = 2, doubled 4, halved 2), then 1; 9 has 4, 2, 1.
/// GL fills the first `max(depth >> level, 1)` (`gl::texture::depth_at`);
/// the rest is padding the tiled copy owns.
pub fn level_depth(depth: u32, level: u32) -> u32 {
    if level == 0 {
        return depth;
    }
    let pot_depth = 2 * (depth >> 1).max(1).next_power_of_two();
    (pot_depth >> level.min(31)).max(1)
}

/// The ES 3.0 compressed formats, the rows of driver/src/v3d/v3d_tex.c's
/// table (`V3D_TEX_FORMAT_ETC2_*`, in its order), which tools/v3d/tex_check.c
/// compares with Mesa's built V3D 4.2 table and util_format.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Etc2 {
    /// COMPRESSED_RGB8_ETC2 (and ETC1, whose blocks are ETC2's)
    Rgb8,
    /// COMPRESSED_SRGB8_ETC2
    Srgb8,
    /// COMPRESSED_RGB8_PUNCHTHROUGH_ALPHA1_ETC2
    Rgb8A1,
    /// COMPRESSED_SRGB8_PUNCHTHROUGH_ALPHA1_ETC2
    Srgb8A1,
    /// COMPRESSED_RGBA8_ETC2_EAC
    Rgba8,
    /// COMPRESSED_SRGB8_ALPHA8_ETC2_EAC
    Srgba8,
    /// COMPRESSED_R11_EAC
    R11,
    /// COMPRESSED_SIGNED_R11_EAC
    R11Snorm,
    /// COMPRESSED_RG11_EAC
    Rg11,
    /// COMPRESSED_SIGNED_RG11_EAC
    Rg11Snorm,
}

/// Every [`Etc2`] row, in the C table's order.
pub const ETC2_FORMATS: [Etc2; 10] = [
    Etc2::Rgb8,
    Etc2::Srgb8,
    Etc2::Rgb8A1,
    Etc2::Srgb8A1,
    Etc2::Rgba8,
    Etc2::Srgba8,
    Etc2::R11,
    Etc2::R11Snorm,
    Etc2::Rg11,
    Etc2::Rg11Snorm,
];

/// The block side of every ETC2/EAC format, in texels.
pub const ETC2_BLOCK: u32 = 4;

/// One ETC2/EAC row: the TMU's texture data type, the swizzle, the bytes of
/// one 4x4 block (Mesa's rsc->cpp for the format, v3d_resource.c:797: the
/// tiling's texel) and the state's sRGB bit (util_format_is_srgb,
/// v3dx_state.c:1023-1025; the type is the linear row's). Mesa's rows
/// (v3dx_format_table.c, return size 16 each):
///   ETC2_RGB8 / ETC2_SRGB8        RGB8_ETC2                 XYZ1   8
///   ETC2_RGB8A1 / ETC2_SRGB8A1    RGB8_PUNCHTHROUGH_ALPHA1  XYZW   8
///   ETC2_RGBA8 / ETC2_SRGBA8      RGBA8_ETC2_EAC            XYZW  16
///   ETC2_R11_UNORM / _SNORM       R11_EAC / SIGNED_R11_EAC  X001   8
///   ETC2_RG11_UNORM / _SNORM      RG11_EAC / SIGNED_RG11_EAC XY01 16
pub const fn etc2_row(e: Etc2) -> (u32, [u8; 4], u32, bool) {
    const X001: [u8; 4] = [V3D_SWIZZLE_RED, V3D_SWIZZLE_ZERO, V3D_SWIZZLE_ZERO, V3D_SWIZZLE_ONE];
    const XY01: [u8; 4] = [V3D_SWIZZLE_RED, V3D_SWIZZLE_GREEN, V3D_SWIZZLE_ZERO, V3D_SWIZZLE_ONE];
    match e {
        Etc2::Rgb8 => (V3D_TEXTURE_DATA_FORMAT_RGB8_ETC2, XYZ1, 8, false),
        Etc2::Srgb8 => (V3D_TEXTURE_DATA_FORMAT_RGB8_ETC2, XYZ1, 8, true),
        Etc2::Rgb8A1 => (V3D_TEXTURE_DATA_FORMAT_RGB8_PUNCHTHROUGH_ALPHA1, XYZW, 8, false),
        Etc2::Srgb8A1 => (V3D_TEXTURE_DATA_FORMAT_RGB8_PUNCHTHROUGH_ALPHA1, XYZW, 8, true),
        Etc2::Rgba8 => (V3D_TEXTURE_DATA_FORMAT_RGBA8_ETC2_EAC, XYZW, 16, false),
        Etc2::Srgba8 => (V3D_TEXTURE_DATA_FORMAT_RGBA8_ETC2_EAC, XYZW, 16, true),
        Etc2::R11 => (V3D_TEXTURE_DATA_FORMAT_R11_EAC, X001, 8, false),
        Etc2::R11Snorm => (V3D_TEXTURE_DATA_FORMAT_SIGNED_R11_EAC, X001, 8, false),
        Etc2::Rg11 => (V3D_TEXTURE_DATA_FORMAT_RG11_EAC, XY01, 16, false),
        Etc2::Rg11Snorm => (V3D_TEXTURE_DATA_FORMAT_SIGNED_RG11_EAC, XY01, 16, false),
    }
}

/// The [`Etc2`] row of a GL core texel format, `None` for an uncompressed
/// one. The GL core has a class for five of the ten rows today
/// (`GLES_TEXFMT_ETC2_*`, src/backend.rs; gl::texture::compressed_format_info);
/// punch-through sRGB and the four EAC formats have none yet.
pub const fn etc2_of(format: u8) -> Option<Etc2> {
    match format {
        GLES_TEXFMT_ETC2_RGB => Some(Etc2::Rgb8),
        GLES_TEXFMT_ETC2_SRGB_RGB => Some(Etc2::Srgb8),
        GLES_TEXFMT_ETC2_RGB_PT => Some(Etc2::Rgb8A1),
        GLES_TEXFMT_ETC2_RGBA => Some(Etc2::Rgba8),
        GLES_TEXFMT_ETC2_SRGB_RGBA => Some(Etc2::Srgba8),
        _ => None,
    }
}

/// The bytes of one 4x4 block of `b`'s format, `None` for an uncompressed
/// format ([`etc2_row`]).
pub const fn block_bytes(b: &GlesTextureBind) -> Option<u32> {
    match etc2_of(b.format) {
        Some(e) => Some(etc2_row(e).2),
        None => None,
    }
}

/// Level `level` of a compressed texture of `layers` stacked layers in the
/// GL core's storage when the backend reads its blocks in raster rows
/// (`YakoglBackend::compressed_blocks_in_rows`, `linear_row_align` and
/// `mip_level_align` 0: gl::texture::compressed_chain_layout and
/// level_block_grid): (byte offset of the level, row pitch in BLOCKS, rows of
/// blocks of ONE layer). Level `l` is `ceil(max(w >> l, 1) / 4)` x
/// `ceil(max(h >> l, 1) / 4)` blocks of `block_bytes`, packed; its layers
/// one after the other, layer `f` `f * rows * pitch * block_bytes` after the
/// level, as [`gl_level_layered`]'s.
pub fn gl_level_blocks(width: u32, height: u32, layers: u32, block_bytes: u32, level: u32) -> (u32, u32, u32) {
    let grid = |l: u32| ((width >> l).max(1).div_ceil(ETC2_BLOCK), (height >> l).max(1).div_ceil(ETC2_BLOCK));
    let mut off = 0u32;
    for k in 0..level {
        let (gw, gh) = grid(k);
        off += gw * gh * layers * block_bytes;
    }
    let (gw, gh) = grid(level);
    (off, gw, gh)
}

/* Mesa's tiling constants (v3d_resource.c, v3d_tiling.c; driver/src/v3d/
 * v3d_tex.c holds the same): a UIF block row is 4 UIF blocks of 256 bytes,
 * a page 4 KiB, the page cache 8 banks of a page. */
const UIF_PAGE: u32 = 4096;
const PAGE_UB_ROWS: u32 = UIF_PAGE / 1024;
const PAGE_UB_ROWS_TIMES_1_5: u32 = (PAGE_UB_ROWS * 3) >> 1;
const PAGE_CACHE_UB_ROWS: u32 = UIF_PAGE * 8 / 1024;
const PAGE_CACHE_MINUS_1_5_UB_ROWS: u32 = PAGE_CACHE_UB_ROWS - PAGE_UB_ROWS_TIMES_1_5;

/// Mesa's v3d_get_ub_pad (v3d_resource.c; `get_ub_pad` in v3d_tex.c): the
/// UIF block rows of padding a UIF level of `height` rows of a `uif_block_h`
/// block takes.
const fn ub_pad(uif_block_h: u32, height: u32) -> u32 {
    let height_ub = height / uif_block_h;
    let in_pc = height_ub % PAGE_CACHE_UB_ROWS;
    if in_pc == 0 {
        0
    } else if in_pc < PAGE_UB_ROWS_TIMES_1_5 {
        if height_ub < PAGE_CACHE_UB_ROWS {
            0
        } else {
            PAGE_UB_ROWS_TIMES_1_5 - in_pc
        }
    } else if in_pc > PAGE_CACHE_MINUS_1_5_UB_ROWS {
        PAGE_CACHE_UB_ROWS - in_pc
    } else {
        0
    }
}

/// The layout of a compressed `width` x `height` image (in texels) of
/// `levels` levels of `block_w` x `block_h` blocks of `block_bytes` bytes:
/// `v3d_tex_layout_compressed` (driver/src/v3d/v3d_tex.c, Mesa's
/// v3d_setup_slices for a compressed resource, v3d_resource.c:582-768, with
/// rsc->cpp the block's bytes, :797) transcribed, because the 2D layout
/// (`v3d_tex_layout`) cannot give it: the power-of-two padding of the levels
/// from 2 is taken in whole blocks (v3d_get_dimension_mpad with the block,
/// :573-581: 36 texels wide has 4 blocks at level 2 where 36 >> 2 = 9 texels
/// is 3), a level is counted in blocks (:646-647), and from there a block is
/// one texel of `block_bytes` bytes for the tiling choice, the alignment and
/// the sizes. `width` and `height` stay in texels, `cpp` is `block_bytes`,
/// as the C's. tools/v3d/tex_check.c pins the C's layouts (the cases the
/// tests below repeat) by hand from v3d_setup_slices and tiles them with
/// Mesa's own v3d_store_tiled_image. `None` where the C returns false: a
/// block side of 0 or past 16, a block other than 8 or 16 bytes, a size of 0
/// or 16384 and more, a level count of 0 or past 15, or past 32 bits.
pub fn layout_compressed(
    width: u32,
    height: u32,
    levels: u32,
    block_w: u32,
    block_h: u32,
    block_bytes: u32,
) -> Option<V3dTexLayout> {
    if block_w == 0 || block_w > 16 || block_h == 0 || block_h > 16 || !matches!(block_bytes, 8 | 16) {
        return None;
    }
    if width == 0 || height == 0 || width >= 1 << 14 || height >= 1 << 14 {
        return None;
    }
    if levels == 0 || levels as usize > V3D_TEX_MAX_LEVELS {
        return None;
    }
    let cpp = block_bytes;
    /* v3d_utile_width / _height of cpp 8 and 16: a 64-byte utile */
    let (utile_w, utile_h) = if cpp == 8 { (4, 2) } else { (2, 2) };
    let (ub_w, ub_h) = (2 * utile_w, 2 * utile_h);
    let minify = |v: u32, l: u32| (v >> l).max(1);
    /* 2 * v3d_get_dimension_mpad(dimension, 1, block) (:596-608) */
    let pot = |d: u32, block: u32| 2 * block * minify(d, 1).div_ceil(block).next_power_of_two();
    let (pot_w, pot_h) = (pot(width, block_w), pot(height, block_h));
    let mut l = V3dTexLayout { width, height, levels, cpp, ..V3dTexLayout::default() };
    let mut offset = 0u64;
    for i in (0..levels).rev() {
        let (lw, lh) = if i < 2 { (minify(width, i), minify(height, i)) } else { (minify(pot_w, i), minify(pot_h, i)) };
        let (mut lw, mut lh) = (lw.div_ceil(block_w), lh.div_ceil(block_h));
        let s = &mut l.slices[i as usize];
        if lw <= utile_w || lh <= utile_h {
            s.tiling = V3D_TEX_TILING_LINEARTILE;
            (lw, lh) = (lw.next_multiple_of(utile_w), lh.next_multiple_of(utile_h));
        } else if lw <= ub_w {
            s.tiling = V3D_TEX_TILING_UBLINEAR_1_COLUMN;
            (lw, lh) = (lw.next_multiple_of(ub_w), lh.next_multiple_of(ub_h));
        } else if lw <= 2 * ub_w {
            s.tiling = V3D_TEX_TILING_UBLINEAR_2_COLUMN;
            (lw, lh) = (lw.next_multiple_of(2 * ub_w), lh.next_multiple_of(ub_h));
        } else {
            (lw, lh) = (lw.next_multiple_of(4 * ub_w), lh.next_multiple_of(ub_h));
            s.ub_pad = ub_pad(ub_h, lh);
            lh += s.ub_pad * ub_h;
            s.tiling =
                if (lh / ub_h) % PAGE_CACHE_UB_ROWS == 0 { V3D_TEX_TILING_UIF_XOR } else { V3D_TEX_TILING_UIF_NO_XOR };
        }
        s.offset = u32::try_from(offset).ok()?;
        s.stride = lw * cpp;
        s.padded_height = lh;
        s.size = lh * s.stride;
        let mut total = u64::from(s.size);
        /* level 1's base is page aligned if it could be UIF XOR (:729-734) */
        if i == 1 && lw > 4 * ub_w && lh > PAGE_CACHE_MINUS_1_5_UB_ROWS * ub_h {
            total = total.next_multiple_of(u64::from(UIF_PAGE));
        }
        offset += total;
    }
    /* level 0 to a page, every level with it (:749-755) */
    let pad = l.slices[0].offset.next_multiple_of(UIF_PAGE) - l.slices[0].offset;
    l.size = u32::try_from(offset + u64::from(pad)).ok()?;
    for s in &mut l.slices[..levels as usize] {
        s.offset += pad;
    }
    Some(l)
}

/// What this cut samples, refused by name otherwise.
pub fn refusal(b: &GlesTextureBind) -> Option<Refusal> {
    refusal_for(b, false)
}

/// The refusal of sampling `b` with a look-up whose TMU output is 32 bits
/// (`ret32`: its program was compiled with `LodKey::ret32` for the binding,
/// TMU Config Parameter 1 bit 0 in its P1 word) or 16 bits. The two must
/// agree with the format's return size ([`return_32`]): an R32F texture read
/// at 16 bits, or a 16-bit format read at 32, is not a measured behaviour.
pub fn refusal_for(b: &GlesTextureBind, ret32: bool) -> Option<Refusal> {
    refusal_in(b, ret32, V3D_SAMPLED_TEXFMTS)
}

/// [`refusal_for`] with the formats sampled (`sampled`, a mask of
/// [`gles_texfmt_bit`]) given: [`V3D_SAMPLED_TEXFMTS`] for the backend, a
/// wider set where a test asks what the rest of the rules say of a format
/// the set does not hold yet (the ETC2 classes).
fn refusal_in(b: &GlesTextureBind, ret32: bool, sampled: u64) -> Option<Refusal> {
    match b.type_ {
        GLES_TEXTYPE_2D if b.layers <= 1 => {}
        GLES_TEXTYPE_CUBE if b.layers == 6 => {
            if b.width != b.height {
                return Some("a cube map whose faces are not square");
            }
        }
        GLES_TEXTYPE_CUBE => return Some("a cube map binding without its six faces"),
        /* the layout, the image and the record ([`layout`], [`image_offset`],
         * [`state`], [`tile`]); the compiler writes the layer to TMUI, a 3D
         * look-up's r to TMUR (src/shaderc/online/v3d/prepare.rs) */
        GLES_TEXTYPE_2D_ARRAY | GLES_TEXTYPE_3D if b.layers >= 1 => {}
        _ => return Some("a texture binding of a type or layer count the V3D does not sample"),
    }
    if sampled & gles_texfmt_bit(b.format) == 0 {
        return Some("a texel format the V3D does not sample");
    }
    if return_32(b) != ret32 {
        return Some(if ret32 {
            "a look-up compiled for the 32-bit TMU output reads a texture of a 16-bit format"
        } else {
            "a texture of 32-bit channels (R32F, RGBA32F, RGBA32UI, RGBA32I) read by a look-up compiled for the \
             16-bit TMU output"
        });
    }
    if b.width == 0 || b.height == 0 || b.width >= 1 << 14 || b.height >= 1 << 14 {
        return Some("a texture size the TMU cannot address (0, or 16384 or more)");
    }
    /* A non-power-of-two chain is sampled (GL_OES_texture_npot): the GL
     * core lays it out with this backend's table as `gl_level_layered` says
     * and the tiled copy as Mesa's v3d_setup_slices does (levels 0 and 1 at
     * their own size, the rest of the power-of-two-rounded level 1,
     * driver/src/v3d/v3d_tex.c `layout`). */
    if block_bytes(b).is_some() {
        /* ES 3.0 has no compressed 3D texture (3.8.6: ETC2/EAC take
         * TEXTURE_2D, TEXTURE_2D_ARRAY and the cube faces); the GL core's
         * blocks lie in rows of their own, which `stride_texels` (a texel
         * pitch) does not describe ([`gl_level_blocks`]) */
        if b.type_ == GLES_TEXTYPE_3D {
            return Some("a 3D texture of a compressed format");
        }
        return None;
    }
    if b.stride_texels != tex_pitch(b.width) {
        return Some("a texture whose level 0 pitch is not the GL core's pair-padded width");
    }
    None
}

/// Bytes of one texel of `b` in the GL core's storage and in the tiled
/// image: 4 for RGBA8, 1 for L8 and A8, 2 for LA8, R16F and RGB565, 8 for
/// the 16-bit-channel classes and 16 for the 32-bit ones
/// (`gles_texfmt_bytes`; the V3D tiles each format at its own size, as
/// Mesa's resources are: `v3d_tex_layout` takes cpp 1, 2, 4, 8 and 16). For
/// an ETC2/EAC format, the bytes of one 4x4 block ([`block_bytes`]: 8 or
/// 16), the "texel" its tiled image is laid out and tiled in, as Mesa's
/// rsc->cpp is (v3d_resource.c:797); the TFU's byte-copy types (1, 2 and 4
/// bytes) then never take it.
pub fn texel_bytes(b: &GlesTextureBind) -> u32 {
    block_bytes(b).unwrap_or(gles_texfmt_bytes(b.format))
}

/// The TMU's texture data type for `b` and the swizzle that makes a program
/// read R, G, B, A (TEXTURE_SHADER_STATE's encoding). Mesa's V3D 4.2 rows
/// (gallium/drivers/v3d/v3dx_format_table.c; these four return size 16):
///   R8G8B8A8_UNORM  RGBA8 XYZW
///   L8_UNORM        R8    XXX1  (L, L, L, 1)
///   A8_UNORM        R8    000X  (0, 0, 0, A)
///   L8A8_UNORM      RG8   XXXY  (L, L, L, A)
/// the table ES 2.0 3.7.2 (table 3.12) gives for GL_LUMINANCE, GL_ALPHA
/// and GL_LUMINANCE_ALPHA. driver/src/v3d/v3d_tex.c's format table holds the
/// same rows (tools/v3d/tex_check.c compares them with Mesa's own table),
/// and a backend test compares these with those.
///
/// `bgra` is set by the GL core for one thing only: an EGLImage of an
/// XRGB8888 native buffer (src/gl/texture.rs texture_attach_external), B,G,R,X
/// in memory. Its X byte is undefined, so alpha reads 1, as Mesa samples
/// B8G8R8X8_UNORM (v3dx_format_table.c:62, SWIZ_ZYX1); reading alpha from X
/// made blending and any shader that used the alpha depend on whatever the
/// buffer's producer left there (v3d-tex, 2026-09-24).
pub fn tmu_format(b: &GlesTextureBind) -> (u32, [u8; 4]) {
    const R: u8 = V3D_SWIZZLE_RED;
    const Z: u8 = V3D_SWIZZLE_ZERO;
    /* the ETC2/EAC rows ([`etc2_row`]) */
    if let Some(e) = etc2_of(b.format) {
        let (type_, swizzle, _, _) = etc2_row(e);
        return (type_, swizzle);
    }
    match b.format {
        /* A depth texture as the GL core keeps it (texture.rs: DEPTH_COMPONENT
         * is an R32F look-up). Mesa's row: R32_FLOAT -> R32F, SWIZ_X001,
         * return size 32 (v3dx_format_table.c:91; driver/src/v3d/v3d_tex.c
         * holds it, tools/v3d/tex_check.c compares). On the ES 2.0 surface
         * GL_OES_depth_texture samples it as luminance (d, d, d, 1):
         * `depth_luminance`, the swizzle XXX1. Mesa's depth formats
         * (DEPTH_COMP16, DEPTH24_X8, DEPTH_COMP32F, v3dx_format_table.c:
         * 149-156) are return size 32 too, so none is cheaper than R32F, which
         * needs no conversion of the core's texels. */
        GLES_TEXFMT_R32F if b.depth_luminance != 0 => (V3D_TEXTURE_DATA_FORMAT_R32F, [R, R, R, V3D_SWIZZLE_ONE]),
        GLES_TEXFMT_R32F => (V3D_TEXTURE_DATA_FORMAT_R32F, [R, Z, Z, V3D_SWIZZLE_ONE]),
        GLES_TEXFMT_L8 => (V3D_TEXTURE_DATA_FORMAT_R8, [R, R, R, V3D_SWIZZLE_ONE]),
        GLES_TEXFMT_A8 => (V3D_TEXTURE_DATA_FORMAT_R8, [V3D_SWIZZLE_ZERO, V3D_SWIZZLE_ZERO, V3D_SWIZZLE_ZERO, R]),
        GLES_TEXFMT_LA8 => (V3D_TEXTURE_DATA_FORMAT_RG8, [R, R, R, V3D_SWIZZLE_GREEN]),
        /* The ES 3.0 storage classes (backend.rs: four components each, the
         * GL core storing (0, 0, 0, 1) in those a narrower format lacks), and
         * an RGB565 EGLImage: Mesa's V3D 4.2 row of the pipe format of the
         * same memory layout (v3dx_format_table.c, Mesa 26.2.1):
         *   R16_FLOAT           R16F            X001  16  (:90)
         *   R16G16B16A16_FLOAT  RGBA16F         XYZW  16  (:100)
         *   R32G32B32A32_FLOAT  RGBA32F         XYZW  32  (:101)
         *   B5G6R5_UNORM        RGB565          XYZ1  16  (:81)
         *   R11G11B10_FLOAT     R11F_G11F_B10F  XYZ1  16  (:146)
         *   R9G9B9E5_FLOAT      RGB9_E5         XYZ1  16  (:147)
         *   R10G10B10A2_UNORM   RGB10_A2        XYZW  16  (:71)
         *   R10G10B10A2_UINT    RGB10_A2UI      XYZW  16  (:75)
         *   R8G8B8A8_SNORM      RGBA8_SNORM     XYZW  16  (:69)
         *   R8G8B8A8_UINT/SINT  RGBA8UI/I       XYZW  16  (:123, :122)
         *   R16G16B16A16_UINT/SINT  RGBA16UI/I  XYZW  16  (:130, :129)
         *   R32G32B32A32_UINT/SINT  RGBA32UI/I  XYZW  32  (:137, :136)
         * The type numbers are v3d_packet.xml's "Texture Data Formats". The
         * TMU applies the swizzle whatever the return size (Mesa programs
         * the record's swizzle for every format, v3dx_state.c:1031-1034),
         * and an integer type's ONE is the integer 1. */
        GLES_TEXFMT_R16F => (V3D_TEXTURE_DATA_FORMAT_R16F, [R, Z, Z, V3D_SWIZZLE_ONE]),
        GLES_TEXFMT_RGB565 => (V3D_TEXTURE_DATA_FORMAT_RGB565, XYZ1),
        GLES_TEXFMT_R11G11B10F => (V3D_TEXTURE_DATA_FORMAT_R11F_G11F_B10F, XYZ1),
        GLES_TEXFMT_RGB9E5 => (V3D_TEXTURE_DATA_FORMAT_RGB9_E5, XYZ1),
        GLES_TEXFMT_RGBA16F => (V3D_TEXTURE_DATA_FORMAT_RGBA16F, XYZW),
        GLES_TEXFMT_RGBA32F => (V3D_TEXTURE_DATA_FORMAT_RGBA32F, XYZW),
        GLES_TEXFMT_RGB10A2 => (V3D_TEXTURE_DATA_FORMAT_RGB10_A2, XYZW),
        GLES_TEXFMT_RGB10A2UI => (V3D_TEXTURE_DATA_FORMAT_RGB10_A2UI, XYZW),
        GLES_TEXFMT_RGBA8_SNORM => (V3D_TEXTURE_DATA_FORMAT_RGBA8_SNORM, XYZW),
        GLES_TEXFMT_RGBA8UI => (V3D_TEXTURE_DATA_FORMAT_RGBA8UI, XYZW),
        GLES_TEXFMT_RGBA8I => (V3D_TEXTURE_DATA_FORMAT_RGBA8I, XYZW),
        GLES_TEXFMT_RGBA16UI => (V3D_TEXTURE_DATA_FORMAT_RGBA16UI, XYZW),
        GLES_TEXFMT_RGBA16I => (V3D_TEXTURE_DATA_FORMAT_RGBA16I, XYZW),
        GLES_TEXFMT_RGBA32UI => (V3D_TEXTURE_DATA_FORMAT_RGBA32UI, XYZW),
        GLES_TEXFMT_RGBA32I => (V3D_TEXTURE_DATA_FORMAT_RGBA32I, XYZW),
        _ if b.bgra != 0 => (V3D_TEXTURE_DATA_FORMAT_RGBA8, [V3D_SWIZZLE_BLUE, V3D_SWIZZLE_GREEN, R, V3D_SWIZZLE_ONE]),
        _ => (V3D_TEXTURE_DATA_FORMAT_RGBA8, XYZW),
    }
}

const XYZW: [u8; 4] = [V3D_SWIZZLE_RED, V3D_SWIZZLE_GREEN, V3D_SWIZZLE_BLUE, V3D_SWIZZLE_ALPHA];
const XYZ1: [u8; 4] = [V3D_SWIZZLE_RED, V3D_SWIZZLE_GREEN, V3D_SWIZZLE_BLUE, V3D_SWIZZLE_ONE];

/// Does a look-up of `b` need the TMU's 32-bit output (TMU Config
/// Parameter 1, "Output Type 32-bit", v3d_packet.xml)? Mesa's return size
/// for the format (v3dx_format_table.c): 32 for the classes of 32-bit
/// channels, R32F (:91), RGBA32F (:101), RGBA32UI and RGBA32I (:136-137);
/// 16 for every other format sampled here. A look-up with a 32-bit output
/// returns one word per channel, a 16-bit one two channels a word (f16, or
/// 16-bit integers for an integer type: Mesa's compiler unpacks those with
/// nir_lower_tex's `nir_lower_tex_packing_16`, broadcom/compiler/vir.c:
/// 632-642, nir_lower_tex.c:1136-1160).
pub fn return_32(b: &GlesTextureBind) -> bool {
    gles_texfmt_return_32(b.format)
}

/// Is the texel format one of integers, read raw by an `isampler` /
/// `usampler` (ES 3.0 3.8.13: complete only with NEAREST filters, which the
/// GL core enforces)?
pub const fn format_is_integer(format: u8) -> bool {
    matches!(
        format,
        GLES_TEXFMT_RGB10A2UI
            | GLES_TEXFMT_RGBA8UI
            | GLES_TEXFMT_RGBA8I
            | GLES_TEXFMT_RGBA16UI
            | GLES_TEXFMT_RGBA16I
            | GLES_TEXFMT_RGBA32UI
            | GLES_TEXFMT_RGBA32I
    )
}

/// GL's wrap mode to the TMU's (`translate_wrap`, v3dx_state.c: REPEAT,
/// CLAMP_TO_EDGE -> CLAMP, MIRRORED_REPEAT -> MIRROR).
pub fn wrap(w: GlesWrap) -> u32 {
    match w {
        GLES_WRAP_MIRRORED_REPEAT => V3D_WRAP_MODE_MIRROR,
        GLES_WRAP_CLAMP_TO_EDGE => V3D_WRAP_MODE_CLAMP,
        _ => V3D_WRAP_MODE_REPEAT,
    }
}

/// The record's swizzle: the format's own ([`tmu_format`]) composed with the
/// binding's GL_TEXTURE_SWIZZLE_* (`GlesTextureBind::swizzle`). Mesa does the
/// same for a view: `util_format_compose_swizzles` of the format's swizzle
/// and the view's, then `v3d_translate_pipe_swizzle` of each component
/// (v3dx_state.c:1031-1034). A GL component reading a channel takes what the
/// format swizzle puts there (the GL side is in base-format terms, so an L8's
/// G is the format's second component, L); ZERO and ONE stay constants. All
/// zero is the identity, and then this is [`tmu_format`]'s own swizzle.
pub fn tmu_swizzle(b: &GlesTextureBind) -> [u8; 4] {
    let (_, fmt) = tmu_format(b);
    let mut out = fmt;
    for (k, o) in out.iter_mut().enumerate() {
        *o = match swizzle_source(k, b.swizzle[k]) {
            c @ 0..=3 => fmt[c],
            4 => V3D_SWIZZLE_ZERO,
            _ => V3D_SWIZZLE_ONE,
        };
    }
    out
}

/// The sampler state of a binding, as st/mesa's sampler state reaches
/// `v3d_upload_sampler_state_variant`: GL ES 2.0 has no LOD range and no
/// bias, so the LOD range is GL's default (-1000, 1000), clamped there; no
/// border colour (BORDER_0000, the variant Mesa picks for a zero border).
pub fn sampler(b: &GlesTextureBind) -> V3dSampler {
    V3dSampler {
        wrap_s: wrap(b.wrap_s),
        wrap_t: wrap(b.wrap_t),
        wrap_r: wrap(b.wrap_r),
        mag_nearest: b.linear_filter == 0,
        min_nearest: b.min_linear == 0,
        mip_nearest: b.mip_filter != 2,
        mip_none: b.mip_filter == 0,
        min_lod: -1000.0,
        max_lod: 1000.0,
        lod_bias: 0.0,
        border_color_mode: V3D_BORDER_COLOR_0000,
    }
}

/// `GlesTextureBind::compare_op`'s bit 4: no compare (GL_NONE).
pub const COMPARE_OFF: u32 = 16;

/// The first bit of SAMPLER_STATE's "Depth Compare Function" (start 4, size
/// 3, v3d_packet.xml:1728), in the record's first 32-bit word.
pub const SAMPLER_COMPARE_SHIFT: u32 = 4;

/// The `compare_op` of `b` as `v3d_tex_pack_sampler_compare` takes it:
/// the binding's own for a depth binding, [`COMPARE_OFF`] for any other. A
/// depth texture is R32F in the GL core (DEPTH_COMPONENT is an R32F
/// look-up, [`tmu_format`]), and sampled either from those texels or from
/// its D24S8 buffer ([`depth_buffer_state`], the same R32F binding with
/// `depth_id`); ES 3.0.6 3.8.15 bypasses the compare for any other base
/// format, and no other class is a depth class here. The GL core sets the
/// compare from the texture's (or the sampler object's) mode and function
/// whatever the format (gl::draw::compare_op), so the format is asked here.
/// A depth binding is read at the TMU's 32-bit output ([`return_32`]): its
/// look-up writes TMU Config Parameter 1's payload 0x5 (output type 32-bit,
/// bit 0, and the per-pixel mask, bit 2, as Mesa's), which the compiler
/// writes for every ret32 binding (select.rs).
pub fn compare_op(b: &GlesTextureBind) -> u32 {
    if b.format == GLES_TEXFMT_R32F {
        u32::from(b.compare_op)
    } else {
        COMPARE_OFF
    }
}

/// The "Depth Compare Function" field `b`'s sampler record carries, as
/// `v3d_tex_pack_sampler_compare` sets it (docs/notes/v3d-shadow.md (a)):
/// with the compare off ([`COMPARE_OFF`], or not a depth binding,
/// [`compare_op`]) NEVER (0), Mesa's compare-off word (v3dx_state.c:598-600,
/// tree); otherwise bits 0-2, GL's order NEVER, LESS, EQUAL, LEQUAL,
/// GREATER, NOTEQUAL, GEQUAL, ALWAYS, which is the field's enum
/// (v3d_packet.xml:3-12). Bit 3 (a fixed-point depth, whose reference GL
/// clamps to [0, 1]) is not part of the record. V3D 4.2 has no enable bit:
/// the three bits are the whole state.
pub fn compare_function(b: &GlesTextureBind) -> u32 {
    let op = compare_op(b);
    if op & COMPARE_OFF != 0 {
        0
    } else {
        op & 7
    }
}

/// The bits [`compare_function`] adds to the first word of the sampler
/// record over the compare-off record of the same [`sampler`] (the rest of
/// the 24 bytes is the same): tools/v3d/tex_check.c checks on Mesa's own
/// SAMPLER_STATE packer that this is all a function changes.
pub fn sampler_compare_bits(b: &GlesTextureBind) -> u32 {
    compare_function(b) << SAMPLER_COMPARE_SHIFT
}

/// Pack `b`'s sampler record, its depth compare included ([`sampler`],
/// [`compare_op`]), with `v3d_tex_pack_sampler_compare`; false, nothing
/// written, for a field that does not fit. Its key, for a cache of packed
/// records, is ([`sampler`], [`compare_function`]): two bindings that differ
/// only in the compare need two records.
///
/// # Safety
/// `out` is writable for `V3D_SAMPLER_STATE_BYTES` bytes.
pub unsafe fn pack_sampler(b: &GlesTextureBind, out: *mut u8) -> bool {
    let s = sampler(b);
    // SAFETY: the caller's contract; `s` is a live sampler.
    unsafe { v3d_tex_pack_sampler_compare(out, &s, compare_op(b)) }
}

/// The texture shader state of a binding whose image (`layout`) starts at
/// `image_gpu` (`v3d_setup_texture_shader_state` and
/// `create_texture_shader_state_bo`): the base pointer at level 0's slice
/// (layer 0's, v3dx_state.c:957-960), levels 0 to the last sampled, the type
/// and swizzle of [`tmu_format`], Image Depth = [`layers`] (:935-939: the
/// level 0 depth of a 3D texture, else the layer count), and the Array
/// Stride Mesa programs as `cube_map_stride / 64` (:961): for a 3D texture
/// level 0's slice size (v3d_resource.c:765-766, "the stride between slices
/// of miplevel 0"; `v3d_tex_state_from_layout_3d`), for every other
/// texture the tree stride `v3d_tex_state_from_layout` sets (:761-763).
pub fn state(b: &GlesTextureBind, layout: &V3dTexLayout, image_gpu: u32) -> V3dTexState {
    let (type_, _) = tmu_format(b);
    let swizzle = tmu_swizzle(b);
    let mut s = V3dTexState {
        base: image_gpu + layout.slices[0].offset,
        width: b.width,
        height: b.height,
        depth: 1,
        base_level: 0,
        max_level: layout.levels - 1,
        type_,
        swizzle,
        /* TEXTURE_SHADER_STATE.sRGB (v3d_packet.xml:1627, V3D 4.2): the TMU
         * decodes red, green and blue before it filters; Mesa sets it from
         * util_format_is_srgb (v3dx_state.c:1023-1028), the texels being
         * the RGBA8 row's, or an sRGB ETC2 row's ([`etc2_row`]) */
        srgb: b.format == GLES_TEXFMT_SRGB8_A8 || etc2_of(b.format).is_some_and(|e| etc2_row(e).3),
        ..V3dTexState::default()
    };
    // SAFETY: both are live values of the mirrored types.
    unsafe { v3d_tex_state_from_layout(&mut s, layout) };
    /* a cube or an array is `depth` layers `array_stride_64` apart (which
     * v3d_tex_state_from_layout set): Mesa's image_depth = the layer count,
     * v3dx_state.c:937-938; a 3D texture's depth is level 0's (:935-936) and
     * its slices are one slice size apart (v3d_resource.c:765-766, whole
     * 64-byte utiles, so the division is exact) */
    s.depth = layers(b);
    if b.type_ == GLES_TEXTYPE_3D {
        s.array_stride_64 = layout.slices[0].size / 64;
    }
    s
}

/// The texture shader state of a depth texture binding read from the D24S8
/// depth buffer its jobs stored (`backend::ZsBuf`, `GlesTextureBind::depth_id`),
/// laid out as [`rt_layout`] lays out one level (the depth buffer's own
/// layout, `backend::zs_buf`): [`state`] with the TMU's DEPTH24_X8 type,
/// Mesa's row for S8_UINT_Z24_UNORM (v3dx_format_table.c:149, return size
/// 32 as R32F's), which reads bits 31..8 of each word as the depth, the
/// same bits the CPU resolve read (`backend::v3d_depth_resolve`). The
/// swizzle stays the GL core's depth swizzle ([`tmu_format`] for the R32F
/// binding: (d, d, d, 1) or (d, 0, 0, 1)); Mesa's row has XXXX.
pub fn depth_buffer_state(b: &GlesTextureBind, layout: &V3dTexLayout, buffer_gpu: u32) -> V3dTexState {
    V3dTexState { type_: V3D_TEXTURE_DATA_FORMAT_DEPTH24_X8, ..state(b, layout, buffer_gpu) }
}

/// The layout of the tiled image for a binding, at its texel size
/// ([`texel_bytes`]), or `None` when `v3d_tex_layout` refuses the size: one
/// mip tree of one layer (a 2D texture; a cube's or an array's layers are
/// each this tree, `v3d_tex_layer_stride` apart, [`image_offset`]), or for
/// a 3D texture the whole stack of its slices ([`stack_3d`]; a depth of 1
/// is one slice a level, the tree as laid out). An ETC2/EAC binding's tree
/// is [`layout_compressed`]'s, in 4x4 blocks (a cube's or an array's layers
/// each that tree, as above).
pub fn layout(b: &GlesTextureBind) -> Option<V3dTexLayout> {
    if let Some(bb) = block_bytes(b) {
        return layout_compressed(b.width, b.height, levels_sampled(b), ETC2_BLOCK, ETC2_BLOCK, bb);
    }
    let mut l = V3dTexLayout::default();
    // SAFETY: `l` is a live value of the mirrored type.
    if !unsafe { v3d_tex_layout(b.width, b.height, levels_sampled(b), texel_bytes(b), &mut l) } {
        return None;
    }
    if b.type_ == GLES_TEXTYPE_3D && layers(b) > 1 {
        stack_3d(&mut l, layers(b))?;
    }
    Some(l)
}

/// Mesa's v3d_setup_slices for a PIPE_TEXTURE_3D resource (v3d_resource.c:
/// 582-768; `v3d_tex_layout_3d` in driver/src/v3d/v3d_tex.c is the C of it,
/// checked against Mesa's numbers by tools/v3d/tex_check.c, and
/// tests/es3_formats_v3d.rs checks that this and the C agree), over the
/// levels `v3d_tex_layout` laid out for one slice: a level's slices are
/// tiled by the level's width and height alone, so each level's `stride`,
/// `padded_height`, `size` (ONE slice) and `tiling` are the 2D layout's
/// (:646-698), and only the offsets and the whole size change. From the
/// smallest level up, a level's total is `size * level_depth` (:722; the
/// slices one after the other, [`image_offset`]); level 1's total is
/// rounded to a 4 KiB page when level 1 could be UIF XOR (:729-734: the
/// TOTAL, not each slice); then level 0 is moved to a page and every level
/// with it (:749-755); `size` is Mesa's `rsc->size` (:739, :752), to which
/// a 3D texture adds no tree stride (:761-767).
///
/// Whether the page rule hit level 1 is read off the 2D layout rather than
/// re-derived from the tiling constants: there `offset[0] - offset[1]` is
/// level 1's total, `align(size[1], 4096)` under the rule and `size[1]`
/// otherwise, so the two differ exactly when the rule applied AND `size[1]`
/// is not a page multiple; when it is, `size[1] * depth` is one too and the
/// rounding changes nothing. With a depth of 1 every step reproduces the
/// 2D layout, so [`layout`] does not run it then. `None` when the stack
/// passes 32 bits.
fn stack_3d(l: &mut V3dTexLayout, depth: u32) -> Option<()> {
    let n = l.levels as usize;
    let page_aligned_1 = n >= 2 && l.slices[0].offset - l.slices[1].offset != l.slices[1].size;
    let mut offset = 0u64;
    for i in (0..n).rev() {
        let mut total = u64::from(l.slices[i].size) * u64::from(level_depth(depth, i as u32));
        if i == 1 && page_aligned_1 {
            total = total.next_multiple_of(4096);
        }
        l.slices[i].offset = u32::try_from(offset).ok()?;
        offset += total;
    }
    let pad = u64::from(l.slices[0].offset).next_multiple_of(4096) - u64::from(l.slices[0].offset);
    l.size = u32::try_from(offset + pad).ok()?;
    for s in &mut l.slices[..n] {
        s.offset += pad as u32;
    }
    Some(())
}

/// The byte offset, from the start of the tiled image, of level `level` of
/// layer (or slice) `layer` of `b` laid out as `layout`: Mesa's
/// v3d_layer_offset (v3d_resource.c:770-780). A 3D texture's slice is
/// `layer` slice sizes after the level (:776-777; `v3d_tex_slice_offset`);
/// a cube's or an array's layer is `layer` whole mip trees
/// (`v3d_tex_layer_stride`, the cube_map_stride of :761-763) after the
/// level (:778-779; `v3d_tex_layer_offset`). The backend tiles layer `f`'s
/// level `L` by handing `v3d_tex_store_level` the image at this offset less
/// the level's own `slices[L].offset`, which the store adds back ([`tile`]).
/// `None` for a level the layout lacks, a layer the image lacks (a 3D
/// texture has [`level_depth`] slices at a level, of which GL fills the
/// first `max(depth >> level, 1)`; the others are padding and still
/// addressable), or an offset past 32 bits.
pub fn image_offset(b: &GlesTextureBind, layout: &V3dTexLayout, level: u32, layer: u32) -> Option<u32> {
    if level >= layout.levels {
        return None;
    }
    let s = &layout.slices[level as usize];
    let (count, stride) = if b.type_ == GLES_TEXTYPE_3D {
        (level_depth(layers(b), level), s.size)
    } else {
        // SAFETY: a live layout.
        (layers(b), unsafe { v3d_tex_layer_stride(layout) })
    };
    if layer >= count {
        return None;
    }
    u32::try_from(u64::from(s.offset) + u64::from(stride) * u64::from(layer)).ok()
}

/// The layout of a render image (`backend::RtImage`): `levels` levels of a
/// `w` x `h` RGBA8 texture as Mesa's v3d_setup_slices lays out a tiled
/// resource (`v3d_tex_layout`), with level 0's UIF XOR dropped as the
/// depth/stencil and 4x buffers drop it (`backend::zs_layout`). With one
/// level it is also the layout of a D24S8 depth/stencil buffer
/// (`backend::zs_buf`: `v3d_tex_layout(w, h, 1, 4)`, the XOR dropped), so a
/// depth texture sampled from its buffer ([`depth_buffer_state`]) and the
/// CPU resolve of it read the buffer through this one layout. The TLB
/// stores level 0, the TMU reads it and the TFU reads it to make the levels
/// below, all three told one layout (NO_XOR is that layout without the
/// page-cache swizzle), and the CPU never tiles into it. The levels below 0
/// keep Mesa's tilings: the TFU writes them and the TMU reads them by the
/// hardware's own rule, which Mesa's layout transcribes, and the CPU reads
/// them back through that layout alone (`v3d_tex_load_box`). A target
/// narrower than 32 texels or shorter than 8 is UB-linear or LT, which the
/// TLB stores as well (v3d_packet.xml "Memory Format"). `None` when the
/// layout refuses the size (0, or 16384 or more) or the level count.
pub fn rt_layout(w: u32, h: u32, levels: u32) -> Option<V3dTexLayout> {
    let mut l = V3dTexLayout::default();
    // SAFETY: `l` is a live value of the mirrored type.
    if !unsafe { v3d_tex_layout(w, h, levels, 4, &mut l) } {
        return None;
    }
    if l.slices[0].tiling == V3D_TEX_TILING_UIF_XOR {
        l.slices[0].tiling = V3D_TEX_TILING_UIF_NO_XOR;
    }
    Some(l)
}

/// The bytes of the tiled image for `b` laid out as `layout`: one mip tree,
/// a cube's six or an array's `layers` of them `v3d_tex_layer_stride` apart
/// (`v3d_tex_layers_size`, Mesa's rsc->size with the trees added,
/// v3d_resource.c:764), or a 3D texture's whole stack, which its layout's
/// `size` already is (:739, :765-767: no stride is added for 3D); `None`
/// past 32 bits.
pub fn image_bytes(b: &GlesTextureBind, layout: &V3dTexLayout) -> Option<u32> {
    let n = layers(b);
    if n == 1 || b.type_ == GLES_TEXTYPE_3D {
        return Some(layout.size);
    }
    let mut out = 0u32;
    // SAFETY: a live layout and out-parameter.
    unsafe { v3d_tex_layers_size(layout, n, &mut out) }.then_some(out)
}

/// The GL core's storage bytes the sampled levels span, every layer: the
/// start of the level after the last one sampled ([`gl_level_blocks`]'s for
/// an ETC2/EAC binding).
pub fn gl_bytes(b: &GlesTextureBind) -> u32 {
    if let Some(bb) = block_bytes(b) {
        return gl_level_blocks(b.width, b.height, layers(b), bb, levels_sampled(b)).0;
    }
    gl_level_layered(b.width, b.height, layers(b), texel_bytes(b), levels_sampled(b)).0
}

/// Tile the sampled levels of every layer of `b` into `image`
/// (`image_bytes` bytes) from the GL core's storage at `src` (`gl_bytes`
/// bytes): layer f's level L from its stacked rows in the GL core's level
/// (`gl_level_layered`) to its own mip tree `f * v3d_tex_layer_stride` into
/// the image (Mesa's v3d_layer_offset, v3d_resource.c:771-779). A 3D
/// texture's level L has `max(depth >> L, 1)` slices in the GL core's first
/// slots of the level (`gl::texture::depth_at`; the GL core stacks `depth`
/// slots at every level), each tiled `k * slices[L].size` after the level
/// (:776-777) as Mesa's own upload does, one z at a time with the same 2D
/// box (v3d_resource.c:136-150, :398-412); the padding slices Mesa
/// allocates above that count ([`level_depth`]) are left as they are. Every
/// slice or layer lands at [`image_offset`].
///
/// # Safety
/// `image` is writable for `image_bytes(b, layout)` bytes and `src` readable
/// for `gl_bytes(b)` bytes.
pub unsafe fn tile(b: &GlesTextureBind, layout: &V3dTexLayout, image: *mut u8, src: *const u8) {
    let n = layers(b);
    let bpp = texel_bytes(b);
    if b.type_ == GLES_TEXTYPE_3D {
        for level in 0..layout.levels {
            let (off, pitch, rows) = gl_level_layered(b.width, b.height, n, bpp, level);
            let size = layout.slices[level as usize].size;
            for k in 0..(n >> level).max(1) {
                // SAFETY: the caller's contract; slice k of the level starts
                // k * size after the level, which v3d_tex_store_level finds
                // at slices[level].offset from the pointer it is given, and
                // the GL core's slice k is k slices of `rows` rows after the
                // level's first.
                unsafe {
                    v3d_tex_store_level(
                        image.add((k * size) as usize),
                        layout,
                        level,
                        src.add((off + k * rows * pitch * bpp) as usize),
                        pitch * bpp,
                    )
                };
            }
        }
        return;
    }
    // SAFETY: a live layout.
    let stride = if n > 1 { unsafe { v3d_tex_layer_stride(layout) } } else { 0 };
    if let Some(bb) = block_bytes(b) {
        /* An ETC2/EAC level: its grid of blocks from the GL core's rows of
         * blocks (`gl_level_blocks`), each block one texel of `bb` bytes as
         * `layout_compressed` lays it out. The box is the grid at (0, 0),
         * which v3d_tex_store_box takes on the texel grid (the level's
         * texel size is at least its block count) and inside the slice (the
         * layout allocated the grid and its padding): the same move as the
         * C's v3d_tex_store_level_blocks, whose grid is this one. The
         * padding blocks Mesa allocates past the grid stay as they are. */
        for f in 0..n {
            for level in 0..layout.levels {
                let (off, gw, gh) = gl_level_blocks(b.width, b.height, n, bb, level);
                // SAFETY: the caller's contract; layer f's grid is f grids
                // after the level in the GL core's storage and its tree f *
                // stride into the image; the box is checked by the C.
                unsafe {
                    v3d_tex_store_box(
                        image.add((f * stride) as usize),
                        layout,
                        level,
                        0,
                        0,
                        gw,
                        gh,
                        src.add((off + f * gw * gh * bb) as usize),
                        gw * bb,
                    )
                };
            }
        }
        return;
    }
    for f in 0..n {
        for level in 0..layout.levels {
            let (off, pitch, rows) = gl_level_layered(b.width, b.height, n, bpp, level);
            let from = off + f * rows * pitch * bpp;
            // SAFETY: the caller's contract; layer f's tree starts f * stride
            // into the image, and v3d_tex_store_level reads the level's rows
            // at `pitch * bpp` and writes inside that tree.
            unsafe {
                v3d_tex_store_level(
                    image.add((f * stride) as usize),
                    layout,
                    level,
                    src.add(from as usize),
                    pitch * bpp,
                )
            };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cube(size: u32, levels: u32) -> GlesTextureBind {
        GlesTextureBind {
            width: size,
            height: size,
            stride_texels: tex_pitch(size),
            type_: GLES_TEXTYPE_CUBE,
            layers: 6,
            mip_levels: levels as u8,
            mip_filter: (levels > 1) as u8,
            format: GLES_TEXFMT_RGBA8,
            ..GlesTextureBind::ZERO
        }
    }

    /// A cube is sampled only whole and square, and its storage spans every
    /// face of the sampled levels (to the start of the next level).
    #[test]
    fn a_cube_is_sampled_whole_and_square() {
        assert_eq!(refusal(&cube(8, 4)), None);
        assert_eq!(layers(&cube(8, 1)), 6);
        let wide = GlesTextureBind { width: 16, stride_texels: 16, ..cube(8, 1) };
        assert_eq!(refusal(&wide), Some("a cube map whose faces are not square"));
        let one = GlesTextureBind { layers: 1, ..cube(8, 1) };
        assert_eq!(refusal(&one), Some("a cube map binding without its six faces"));
        let arr = GlesTextureBind { type_: GLES_TEXTYPE_2D_ARRAY, ..cube(8, 1) };
        assert_eq!(refusal(&arr), None, "six layers of a 2D array");
        assert_eq!(gl_bytes(&cube(8, 4)), gl_level_layered(8, 8, 6, 4, 4).0);
        assert_eq!(gl_bytes(&cube(12, 1)), 12 * 12 * 6 * 4, "one level, not a chain: packed");
    }

    /// An XRGB8888 EGLImage (`bgra`): B,G,R from the buffer and alpha ONE,
    /// never the undefined X byte (Mesa's B8G8R8X8_UNORM, SWIZ_ZYX1); a
    /// plain RGBA8 texture reads its own four channels.
    #[test]
    fn an_xrgb_image_samples_alpha_one_and_rgba8_its_own() {
        let layout = layout(&bind(4, 4)).unwrap();
        let rgba = state(&bind(4, 4), &layout, 0x1000);
        assert_eq!(rgba.swizzle, [V3D_SWIZZLE_RED, V3D_SWIZZLE_GREEN, V3D_SWIZZLE_BLUE, V3D_SWIZZLE_ALPHA]);
        let xrgb = state(&GlesTextureBind { bgra: 1, ..bind(4, 4) }, &layout, 0x1000);
        assert_eq!(xrgb.swizzle, [V3D_SWIZZLE_BLUE, V3D_SWIZZLE_GREEN, V3D_SWIZZLE_RED, V3D_SWIZZLE_ONE]);
    }

    /// The GL swizzle is composed with the format's own, as Mesa's
    /// util_format_compose_swizzles then v3d_translate_pipe_swizzle:
    /// identity changes nothing, (B,G,R,A) reverses an RGBA8, ZERO/ONE are
    /// constants, and a format with its own swizzle (XRGB, L8, A8, depth)
    /// is read through.
    #[test]
    fn the_gl_swizzle_composes_with_the_formats() {
        use crate::backend::{GLES_SWIZZLE_ALPHA as A, GLES_SWIZZLE_BLUE as B, GLES_SWIZZLE_GREEN as G};
        use crate::backend::{GLES_SWIZZLE_ONE as ONE, GLES_SWIZZLE_RED as R, GLES_SWIZZLE_ZERO as ZERO};
        let layout = layout(&bind(4, 4)).unwrap();
        let sw = |b: &GlesTextureBind| state(b, &layout, 0x1000).swizzle;
        let (vr, vg, vb, va) = (V3D_SWIZZLE_RED, V3D_SWIZZLE_GREEN, V3D_SWIZZLE_BLUE, V3D_SWIZZLE_ALPHA);
        let (v0, v1) = (V3D_SWIZZLE_ZERO, V3D_SWIZZLE_ONE);
        let rgba = bind(4, 4);
        assert_eq!(sw(&rgba), tmu_format(&rgba).1);
        assert_eq!(sw(&GlesTextureBind { swizzle: [R, G, B, A], ..rgba }), [vr, vg, vb, va]);
        assert_eq!(sw(&GlesTextureBind { swizzle: [B, G, R, A], ..rgba }), [vb, vg, vr, va]);
        assert_eq!(sw(&GlesTextureBind { swizzle: [ZERO, ONE, R, ZERO], ..rgba }), [v0, v1, vr, v0]);
        assert_eq!(
            sw(&GlesTextureBind { swizzle: [GLES_SWIZZLE_SAME, ONE, GLES_SWIZZLE_SAME, GLES_SWIZZLE_SAME], ..rgba }),
            [vr, v1, vb, va]
        );
        // XRGB (B,G,R,1 in the format): GL R reads the format's component 0
        let xrgb = GlesTextureBind { bgra: 1, ..rgba };
        assert_eq!(sw(&GlesTextureBind { swizzle: [B, G, R, A], ..xrgb }), [vr, vg, vb, v1]);
        assert_eq!(sw(&GlesTextureBind { swizzle: [A, ZERO, ONE, R], ..xrgb }), [v1, v0, v1, vb]);
        // L8 (L,L,L,1): GL A reads the format's ONE, GL R of (A,..) too
        let l8 = GlesTextureBind { format: GLES_TEXFMT_L8, ..rgba };
        assert_eq!(sw(&GlesTextureBind { swizzle: [A, R, ZERO, B], ..l8 }), [v1, vr, v0, vr]);
        // A8 (0,0,0,A): GL A reads the format's fourth component
        let a8 = GlesTextureBind { format: GLES_TEXFMT_A8, ..rgba };
        assert_eq!(sw(&GlesTextureBind { swizzle: [A, A, A, R], ..a8 }), [vr, vr, vr, v0]);
        // depth (X001 kept for R32F)
        let d = GlesTextureBind { format: GLES_TEXFMT_R32F, ..rgba };
        assert_eq!(sw(&d), [vr, v0, v0, v1]);
        assert_eq!(sw(&GlesTextureBind { swizzle: [A, R, G, ONE], ..d }), [v1, vr, v0, v1]);
    }

    /// wrap R goes through the same map as S and T; REPEAT is the default.
    #[test]
    fn wrap_r_maps_like_s_and_t() {
        let b = bind(4, 4);
        assert_eq!(sampler(&b).wrap_r, V3D_WRAP_MODE_REPEAT);
        let c = sampler(&GlesTextureBind { wrap_r: GLES_WRAP_CLAMP_TO_EDGE, ..b });
        assert_eq!((c.wrap_r, c.wrap_s, c.wrap_t), (V3D_WRAP_MODE_CLAMP, V3D_WRAP_MODE_REPEAT, V3D_WRAP_MODE_REPEAT));
        let m = sampler(&GlesTextureBind { wrap_r: GLES_WRAP_MIRRORED_REPEAT, ..b });
        assert_eq!(m.wrap_r, V3D_WRAP_MODE_MIRROR);
    }

    /// Each sampled format is Mesa's V3D 4.2 row (v3dx_format_table.c):
    /// its TEXTURE_DATA_FORMAT (v3d_packet.xml: R8 0, RG8 2, RGBA8 4), the
    /// swizzle v3d_translate_pipe_swizzle makes of SWIZ_XYZW, XXX1, 000X and
    /// XXXY (X..W -> 2..5, 0 -> 0, 1 -> 1), and its bytes a texel. These are
    /// also the rows of driver/src/v3d/v3d_tex.c's table, which
    /// tools/v3d/tex_check.c compares with Mesa's own.
    #[test]
    fn each_format_samples_as_mesas_row() {
        let rows: [(u8, u32, [u8; 4], u32); 4] = [
            (GLES_TEXFMT_RGBA8, 4, [2, 3, 4, 5], 4),
            (GLES_TEXFMT_L8, 0, [2, 2, 2, 1], 1),
            (GLES_TEXFMT_A8, 0, [0, 0, 0, 2], 1),
            (GLES_TEXFMT_LA8, 2, [2, 2, 2, 3], 2),
        ];
        for (format, type_, swizzle, bytes) in rows {
            let b = GlesTextureBind { format, ..bind(4, 4) };
            assert_eq!(refusal(&b), None, "format {format}");
            assert_eq!(tmu_format(&b), (type_, swizzle), "format {format}");
            assert_eq!(texel_bytes(&b), bytes, "format {format}");
            assert_eq!(layout(&b).unwrap().cpp, bytes, "format {format}: laid out at its own size");
        }
    }

    /// The sRGB class is the RGBA8 row (Mesa's R8G8B8A8_SRGB: TMU format
    /// RGBA8, SWIZ_XYZW, v3dx_format_table.c:67) with TEXTURE_SHADER_STATE's
    /// sRGB bit (v3d_packet.xml:1627, set from util_format_is_srgb,
    /// v3dx_state.c:1023-1028); no other class sets the bit.
    #[test]
    fn the_srgb_class_is_the_rgba8_row_with_the_tmu_decode() {
        let b = GlesTextureBind { format: GLES_TEXFMT_SRGB8_A8, ..bind(4, 4) };
        let rgba = GlesTextureBind { format: GLES_TEXFMT_RGBA8, ..bind(4, 4) };
        assert_eq!(refusal(&b), None);
        assert_eq!(tmu_format(&b), tmu_format(&rgba));
        assert_eq!((texel_bytes(&b), return_32(&b)), (4, false));
        let layout = layout(&b).unwrap();
        let s = state(&b, &layout, 0x1000);
        let r = state(&rgba, &layout, 0x1000);
        assert!(s.srgb && !r.srgb);
        assert_eq!(V3dTexState { srgb: false, ..s }, r, "nothing else differs");
        for format in [GLES_TEXFMT_L8, GLES_TEXFMT_A8, GLES_TEXFMT_LA8, GLES_TEXFMT_R32F] {
            assert!(!state(&GlesTextureBind { format, ..bind(4, 4) }, &layout, 0x1000).srgb, "format {format}");
        }
    }

    /// A depth texture's R32F storage samples as Mesa's R32_FLOAT row
    /// (v3d_packet.xml: TEXTURE_DATA_FORMAT_R32F 29; SWIZ_X001 -> 2, 0, 0,
    /// 1; 4 bytes a texel) on the ES 3.0 surface, and as GL_OES_depth_texture's
    /// luminance (XXX1) where the core asks for it; it needs the 32-bit TMU
    /// output, and no other format here does: sampled only by a look-up
    /// compiled for it (`refusal_for`), and the other way round.
    #[test]
    fn a_depth_textures_r32f_storage_is_mesas_r32_float_row() {
        let es3 = GlesTextureBind { format: GLES_TEXFMT_R32F, ..bind(4, 4) };
        let es2 = GlesTextureBind { depth_luminance: 1, ..es3 };
        assert_eq!(tmu_format(&es3), (29, [2, 0, 0, 1]));
        assert_eq!(tmu_format(&es2), (29, [2, 2, 2, 1]));
        assert_eq!(texel_bytes(&es3), 4);
        assert_eq!(layout(&es3).unwrap().cpp, 4, "laid out at 4 bytes a texel");
        assert!(return_32(&es3) && return_32(&es2));
        for format in [GLES_TEXFMT_RGBA8, GLES_TEXFMT_L8, GLES_TEXFMT_A8, GLES_TEXFMT_LA8] {
            assert!(!return_32(&GlesTextureBind { format, ..bind(4, 4) }), "format {format}");
        }
        assert!(refusal(&es3).is_some(), "refused at the 16-bit output");
        assert_eq!(refusal_for(&es3, true), None, "sampled at the 32-bit output");
        let rgba = GlesTextureBind { format: GLES_TEXFMT_RGBA8, ..bind(4, 4) };
        assert!(refusal_for(&rgba, true).is_some(), "a 16-bit format at the 32-bit output is refused");
    }

    /// A depth texture read from its D24S8 buffer (`depth_buffer_state`,
    /// `GlesTextureBind::depth_id`): the R32F record with Mesa's
    /// S8_UINT_Z24_UNORM type (v3d_packet.xml: TEXTURE_DATA_FORMAT_DEPTH24_X8
    /// 24, "X low, D high") in place of R32F, the GL depth swizzle kept, the
    /// base at the buffer (one level, whose slice 0 starts at 0), one level.
    /// The buffer's layout is `rt_layout` with one level.
    #[test]
    fn a_depth_texture_read_from_its_buffer_is_the_r32f_record_with_the_depth24_x8_type() {
        let es3 = GlesTextureBind { format: GLES_TEXFMT_R32F, depth_id: 0x4000_0021, ..bind(64, 48) };
        let es2 = GlesTextureBind { depth_luminance: 1, ..es3 };
        let layout = rt_layout(64, 48, 1).unwrap();
        assert_eq!((layout.levels, layout.cpp, layout.slices[0].offset), (1, 4, 0));
        let s3 = depth_buffer_state(&es3, &layout, 0x8000);
        let s2 = depth_buffer_state(&es2, &layout, 0x8000);
        assert_eq!((s3.type_, s3.swizzle), (V3D_TEXTURE_DATA_FORMAT_DEPTH24_X8, [2, 0, 0, 1]));
        assert_eq!((s2.type_, s2.swizzle), (24, [2, 2, 2, 1]));
        assert_eq!((s3.base, s3.max_level, s3.width, s3.height, s3.depth), (0x8000, 0, 64, 48, 1));
        let r32f = state(&es3, &layout, 0x8000);
        assert_eq!(V3dTexState { type_: r32f.type_, ..s3 }, r32f, "nothing but the type differs from the R32F record");
    }

    /// A 2D array of `layers` layers, `levels` levels, RGBA8.
    fn array(w: u32, h: u32, layers: u32, levels: u32) -> GlesTextureBind {
        GlesTextureBind {
            type_: GLES_TEXTYPE_2D_ARRAY,
            layers: layers as u16,
            mip_levels: levels as u8,
            mip_filter: (levels > 1) as u8,
            ..bind(w, h)
        }
    }

    /// A 3D texture `depth` deep (`layers` carries the level 0 depth,
    /// src/backend.rs), `levels` levels, RGBA8.
    fn vol(w: u32, h: u32, depth: u32, levels: u32) -> GlesTextureBind {
        GlesTextureBind { type_: GLES_TEXTYPE_3D, ..array(w, h, depth, levels) }
    }

    /// Both are sampled whatever their layer count (one layer or slice
    /// included), with the format checks of a 2D texture; a binding with no
    /// layer, or a 2D one with more than one, is refused by name.
    #[test]
    fn an_array_and_a_volume_are_sampled() {
        for b in [array(64, 32, 4, 7), array(8, 8, 1, 1), vol(32, 32, 8, 6), vol(4, 4, 1, 1)] {
            assert_eq!(refusal(&b), None);
            assert_eq!(
                refusal_for(&b, true),
                Some("a look-up compiled for the 32-bit TMU output reads a texture of a 16-bit format")
            );
            assert!(layout(&b).is_some(), "laid out");
        }
        let what = Some("a texture binding of a type or layer count the V3D does not sample");
        assert_eq!(refusal(&array(8, 8, 0, 1)), what);
        assert_eq!(refusal(&vol(8, 8, 0, 1)), what);
        assert_eq!(refusal(&GlesTextureBind { layers: 2, ..bind(8, 8) }), what);
    }

    /// `layers` is the GL core's count for an array and the level 0 depth for
    /// a 3D texture, and the GL core stacks that many slots at every level
    /// (`gl_level_layered`): 4x4x16 RGBA8 over its 5 levels (`chain_levels_3d`,
    /// log2(16) + 1) is 4x4 (16 x 64 B), 2x2 (16 x 16 B), then 1x1 at one
    /// texel a row, 16 x 4 B each, three times: 1024 + 256 + 64 + 64 + 64.
    #[test]
    fn layers_are_the_bindings_count_and_the_gl_cores_slots() {
        assert_eq!(layers(&array(64, 32, 4, 7)), 4);
        assert_eq!(layers(&vol(32, 32, 8, 6)), 8);
        assert_eq!(layers(&vol(32, 32, 0, 1)), 1, "a depth of 0 is read as 1");
        assert_eq!((layers(&cube(8, 1)), layers(&bind(8, 8))), (6, 1));
        assert_eq!(gl_bytes(&array(64, 32, 4, 7)), gl_level_layered(64, 32, 4, 4, 7).0);
        assert_eq!(gl_bytes(&vol(4, 4, 16, 5)), 1024 + 256 + 64 + 64 + 64);
        assert_eq!(gl_level_layered(4, 4, 16, 4, 3), (1344, 1, 1), "a 1x1 level of 16 slots, one texel a row");
    }

    /// Mesa's level_depth (v3d_setup_slices, v3d_resource.c:609-611, :636-639):
    /// the depth at level 0, then `u_minify(2 * next_pow2(u_minify(depth, 1)),
    /// level)`, at least 1.
    #[test]
    fn level_depth_is_mesas_power_of_two_rule_from_level_1() {
        /* 8: pot_depth 2 * next_pow2(4) = 8 -> 8, 4, 2, 1, 1 */
        assert_eq!((0..5).map(|l| level_depth(8, l)).collect::<std::vec::Vec<_>>(), [8, 4, 2, 1, 1]);
        /* 6: 2 * next_pow2(3) = 8 -> 6, 4, 2, 1 (GL has 6, 3, 1, 1) */
        assert_eq!((0..4).map(|l| level_depth(6, l)).collect::<std::vec::Vec<_>>(), [6, 4, 2, 1]);
        /* 5: 2 * next_pow2(2) = 4 -> 5, 2, 1 */
        assert_eq!((0..3).map(|l| level_depth(5, l)).collect::<std::vec::Vec<_>>(), [5, 2, 1]);
        /* 9: 2 * next_pow2(4) = 8 -> 9, 4, 2, 1 */
        assert_eq!((0..4).map(|l| level_depth(9, l)).collect::<std::vec::Vec<_>>(), [9, 4, 2, 1]);
        /* 16: 2 * next_pow2(8) = 16 -> 16, 8, 4, 2, 1, 1 */
        assert_eq!((0..6).map(|l| level_depth(16, l)).collect::<std::vec::Vec<_>>(), [16, 8, 4, 2, 1, 1]);
        /* 1: u_minify(1, 1) = 1, 2 * next_pow2(1) = 2 -> 1 at every level */
        assert_eq!((0..4).map(|l| level_depth(1, l)).collect::<std::vec::Vec<_>>(), [1, 1, 1, 1]);
    }

    /// A 2D array is the cube path with its own layer count (v3d_resource.c:
    /// 757-764, :778-779): the layout is one layer's tree (the 2D layout of
    /// 64x32 x7), a layer is `v3d_tex_layer_stride` = align(end of level 0,
    /// 64) further, the image is `size + 3 * stride` (:764), and the record
    /// has Image Depth 4 (v3dx_state.c:937-938) and Array Stride stride / 64
    /// (:961). The slice numbers themselves are the fake layout's
    /// (backend_tests.rs: levels packed, level 0 on a 64-byte boundary) and
    /// are not pinned here; Mesa's are pinned on the real C by
    /// tests/es3_formats_v3d.rs and tools/v3d/tex_check.c.
    #[test]
    fn a_2d_array_is_layers_of_whole_mip_trees() {
        let b = array(64, 32, 4, 7);
        let l = layout(&b).unwrap();
        let flat = GlesTextureBind { mip_levels: 7, mip_filter: 1, ..bind(64, 32) };
        assert_eq!(l, layout(&flat).unwrap(), "one layer's tree is the 2D layout");
        let stride = (l.slices[0].offset + l.slices[0].size).next_multiple_of(64);
        assert_eq!(image_bytes(&b, &l), Some(l.size + 3 * stride));
        for level in 0..7 {
            for f in 0..4 {
                assert_eq!(image_offset(&b, &l, level, f), Some(l.slices[level as usize].offset + f * stride));
            }
            assert_eq!(image_offset(&b, &l, level, 4), None, "layer 4 of 4");
        }
        assert_eq!(image_offset(&b, &l, 7, 0), None, "level 7 of 7");
        let s = state(&b, &l, 0x1000);
        assert_eq!((s.base, s.width, s.height, s.depth), (0x1000 + l.slices[0].offset, 64, 32, 4));
        assert_eq!((s.base_level, s.max_level, s.array_stride_64), (0, 6, stride / 64));
        /* a one-layer array is a 2D texture's image and record, depth 1 */
        let one = array(13, 7, 1, 1);
        let lo = layout(&one).unwrap();
        assert_eq!((image_bytes(&one, &lo), state(&one, &lo, 0x1000).depth), (Some(lo.size), 1));
    }

    /// A 3D texture stacks each level's slices (v3d_resource.c:722, :736:
    /// `offset += size * level_depth`, smallest level first), moves level 0
    /// to a page (:749-755), adds no tree stride (:765-767), and its record
    /// has Image Depth 8 (v3dx_state.c:935-936) and Array Stride level 0's
    /// slice size / 64 (v3d_resource.c:766, v3dx_state.c:961), which is not
    /// the array's tree stride. Per level the slice is the 2D layout's. On
    /// the fake layout 32x32x8 x6 is, smallest first, 1x1 (4 B x1), 2x2 (16
    /// B x1), 4x4 (64 B x1), 8x8 (256 B x2 = 512), 16x16 (1024 B x4 = 4096),
    /// 32x32 (4096 B x8 = 32768): offsets 0, 4, 20, 84, 596, 4692, 37460 in
    /// all; level 0 to 8192 shifts every offset by 3500: 40960 bytes. (The
    /// real C differs below level 2, where a slice is a whole 64-byte utile:
    /// tests/es3_formats_v3d.rs.) Slice k of level 0 is at 8192 + 4096 k
    /// (:776-777), level 1's at 4096 + 1024 k, k < 4 (`level_depth`).
    #[test]
    fn a_3d_texture_stacks_each_levels_slices() {
        let b = vol(32, 32, 8, 6);
        let l = layout(&b).unwrap();
        let flat = GlesTextureBind { mip_levels: 6, mip_filter: 1, ..bind(32, 32) };
        let f = layout(&flat).unwrap();
        for i in 0..6 {
            let (s, t) = (l.slices[i], f.slices[i]);
            assert_eq!(
                (s.stride, s.padded_height, s.size, s.tiling, s.ub_pad),
                (t.stride, t.padded_height, t.size, t.tiling, t.ub_pad),
                "level {i}: one slice is the 2D level"
            );
        }
        let offsets: [u32; 6] = core::array::from_fn(|i| l.slices[i].offset);
        assert_eq!(offsets, [8192, 4096, 3584, 3520, 3504, 3500]);
        assert_eq!((l.size, image_bytes(&b, &l)), (40960, Some(40960)));
        /* the rule itself, level by level from the smallest */
        let mut at = 3500;
        for i in (0..6).rev() {
            assert_eq!(l.slices[i].offset, at, "level {i}");
            at += l.slices[i].size * level_depth(8, i as u32);
        }
        assert_eq!(at, l.size);
        assert_eq!(image_offset(&b, &l, 0, 7), Some(8192 + 7 * 4096));
        assert_eq!(image_offset(&b, &l, 1, 3), Some(4096 + 3 * 1024));
        assert_eq!(image_offset(&b, &l, 2, 1), Some(3584 + 256));
        assert_eq!(image_offset(&b, &l, 2, 2), None, "slice 2 of level 2's 2");
        assert_eq!(image_offset(&b, &l, 1, 4), None, "slice 4 of level 1's 4");
        assert_eq!(image_offset(&b, &l, 6, 0), None, "level 6 of 6");
        let s = state(&b, &l, 0x1000);
        assert_eq!((s.base, s.width, s.height, s.depth, s.max_level), (0x1000 + 8192, 32, 32, 8, 5));
        assert_eq!(s.array_stride_64, 4096 / 64, "level 0's slice size, not the tree stride");
        assert_ne!(s.array_stride_64, (8192 + 4096) / 64);
        /* a depth of 1 is the 2D layout */
        let one = vol(32, 32, 1, 6);
        assert_eq!(layout(&one).unwrap(), f);
        assert_eq!(state(&one, &f, 0x1000).depth, 1);
    }

    /// The non-power-of-two 13x7x6 x4: 6 slices at level 0, then
    /// `level_depth` 4, 2, 1 where GL has 3, 1, 1 (v3d_resource.c:609-611:
    /// 2 * next_pow2(3) = 8). On the fake layout (a level is `(13 >> l) x
    /// (7 >> l)` texels, at least 1, packed): 1x1 (4 B x1), 3x1 (12 B x2 =
    /// 24), 6x3 (72 B x4 = 288), 13x7 (364 B x6 = 2184): offsets 0, 4, 28,
    /// 316, 2500 in all; level 0 to 4096: +3780, 6280 bytes. GL fills slices
    /// 0..3 of level 1 and slice 3 is padding, still addressable.
    #[test]
    fn a_non_power_of_two_3d_texture_pads_its_lower_levels_depth() {
        let b = vol(13, 7, 6, 4);
        let l = layout(&b).unwrap();
        let offsets: [u32; 4] = core::array::from_fn(|i| l.slices[i].offset);
        assert_eq!(offsets, [4096, 3808, 3784, 3780]);
        assert_eq!(l.size, 6280);
        assert_eq!([level_depth(6, 0), level_depth(6, 1), level_depth(6, 2), level_depth(6, 3)], [6, 4, 2, 1]);
        assert_eq!(image_offset(&b, &l, 0, 5), Some(4096 + 5 * 364));
        assert_eq!(image_offset(&b, &l, 1, 3), Some(3808 + 3 * 72), "the padding slice");
        assert_eq!(image_offset(&b, &l, 1, 4), None);
        assert_eq!(image_offset(&b, &l, 2, 1), Some(3784 + 12));
        assert_eq!(image_offset(&b, &l, 2, 2), None);
        assert_eq!(state(&b, &l, 0x1000).depth, 6);
    }

    /// `tile` puts slice k of level L of a 3D texture at `image_offset(L, k)`
    /// from the GL core's slot k of the level, and leaves the padding slices
    /// alone: 4x4x6 x3 RGBA8 on the fake layout (a level stored as its packed
    /// rows): level 2 (1x1, 4 B x `level_depth` 2) at 0, level 1 (2x2, 16 B
    /// x4) at 8, level 0 (64 B x6) at 72, then level 0 to 4096 (+4024): 4480
    /// bytes. The GL core's storage: level 0 at 0 (6 slots of 64 B), level 1
    /// at 384 (6 slots of 16 B), level 2 at 480 (6 slots of 4 B, one texel a
    /// row); it fills 6, 3 and 1 of them (`depth_at`).
    #[test]
    fn tile_puts_each_slice_of_a_3d_texture_at_its_offset() {
        let b = vol(4, 4, 6, 3);
        let l = layout(&b).unwrap();
        assert_eq!((l.slices[0].offset, l.slices[1].offset, l.slices[2].offset, l.size), (4096, 4032, 4024, 4480));
        let n = gl_bytes(&b) as usize;
        assert_eq!(n, 384 + 96 + 24);
        let src: std::vec::Vec<u8> = (0..n).map(|i| (i * 7 + 3) as u8).collect();
        let mut image = std::vec![0xa5u8; image_bytes(&b, &l).unwrap() as usize];
        // SAFETY: the image is image_bytes long and the source gl_bytes.
        unsafe { tile(&b, &l, image.as_mut_ptr(), src.as_ptr()) };
        for (level, gl_at, bytes, slices) in [(0u32, 0usize, 64usize, 6u32), (1, 384, 16, 3), (2, 480, 4, 1)] {
            for k in 0..slices {
                let at = image_offset(&b, &l, level, k).unwrap() as usize;
                let from = gl_at + k as usize * bytes;
                assert_eq!(&image[at..at + bytes], &src[from..from + bytes], "level {level} slice {k}");
            }
        }
        /* the padding slices (level 1's slice 3, level 2's slice 1) and the
         * bytes below level 2 keep theirs */
        let untouched = |from: usize, to: usize| image[from..to].iter().all(|&x| x == 0xa5);
        assert!(untouched(4032 + 3 * 16, 4096), "level 1 slice 3");
        assert!(untouched(4028, 4032), "level 2 slice 1");
        assert!(untouched(0, 4024));
    }

    /// `tile` puts layer f of a 2D array `f * stride` into the image, every
    /// level: 4x4 x2 with 3 layers on the fake layout (level 1 (2x2, 16 B)
    /// at 48, level 0 (64 B) at 64, 128 a tree, stride 128; the GL core's
    /// level 0 holds 3 slots of 64 B at 0, level 1 3 slots of 16 B at 192).
    #[test]
    fn tile_puts_each_layer_of_an_array_a_stride_apart() {
        let b = array(4, 4, 3, 2);
        let l = layout(&b).unwrap();
        assert_eq!((l.slices[0].offset, l.slices[1].offset, l.size), (64, 48, 128));
        assert_eq!(image_bytes(&b, &l), Some(128 + 2 * 128));
        let n = gl_bytes(&b) as usize;
        assert_eq!(n, 192 + 48);
        let src: std::vec::Vec<u8> = (0..n).map(|i| (i * 5 + 1) as u8).collect();
        let mut image = std::vec![0u8; image_bytes(&b, &l).unwrap() as usize];
        // SAFETY: the image is image_bytes long and the source gl_bytes.
        unsafe { tile(&b, &l, image.as_mut_ptr(), src.as_ptr()) };
        for f in 0..3u32 {
            let at = image_offset(&b, &l, 0, f).unwrap() as usize;
            assert_eq!(at, 64 + f as usize * 128);
            assert_eq!(&image[at..at + 64], &src[f as usize * 64..f as usize * 64 + 64], "layer {f} level 0");
            let at = image_offset(&b, &l, 1, f).unwrap() as usize;
            assert_eq!(at, 48 + f as usize * 128);
            assert_eq!(
                &image[at..at + 16],
                &src[192 + f as usize * 16..192 + f as usize * 16 + 16],
                "layer {f} level 1"
            );
        }
    }

    /// dEQP texture.size.cube.512x512_l8_mipmap: the GL core's L8 chain is a
    /// quarter of the RGBA8 one, and so is what the backend reads of it.
    #[test]
    fn an_l8_chain_is_a_quarter_of_the_rgba8_one() {
        let rgba = GlesTextureBind { format: GLES_TEXFMT_RGBA8, ..cube(512, 10) };
        let l8 = GlesTextureBind { format: GLES_TEXFMT_L8, ..cube(512, 10) };
        assert_eq!(gl_bytes(&rgba), 4 * gl_bytes(&l8));
        assert_eq!(gl_bytes(&l8), gl_level_layered(512, 512, 6, 1, 10).0);
    }

    fn bind(w: u32, h: u32) -> GlesTextureBind {
        GlesTextureBind {
            width: w,
            height: h,
            stride_texels: tex_pitch(w),
            layers: 1,
            mip_levels: 1,
            format: GLES_TEXFMT_RGBA8,
            ..GlesTextureBind::ZERO
        }
    }

    /// The GL core's packed chain, by hand: 8x4 is 8x4 (128 B), 4x2 (32 B),
    /// 2x1 (8 B), then 1x1 at one texel a row (4 B).
    #[test]
    fn the_gl_cores_levels_are_packed_at_their_own_pitch() {
        assert_eq!(gl_level(8, 4, 0), (0, 8, 4));
        assert_eq!(gl_level(8, 4, 1), (128, 4, 2));
        assert_eq!(gl_level(8, 4, 2), (160, 2, 1));
        assert_eq!(gl_level(8, 4, 3), (168, 1, 1), "a 1-wide level above a wider base: 1 texel a row");
        /* an odd width's level 0 is pair-padded */
        assert_eq!(gl_level(13, 7, 0), (0, 14, 7));
        /* a 1x1 texture's only level keeps the pair pitch */
        assert_eq!(gl_level(1, 1, 0), (0, 2, 1));
    }

    #[test]
    fn a_binding_samples_its_chain_only_with_a_mip_filter() {
        let mut b = bind(64, 64);
        b.mip_levels = 7;
        assert_eq!(levels_sampled(&b), 1, "no mip filter: level 0");
        assert_eq!(gl_bytes(&b), 64 * 64 * 4);
        b.mip_filter = 1;
        assert_eq!(levels_sampled(&b), 7);
        assert_eq!(gl_bytes(&b), gl_level(64, 64, 6).0 + 4, "through the 1x1 level, one texel a row");
    }

    /// One distinct reason per refusal.
    #[test]
    fn what_this_cut_does_not_sample_is_refused_by_name() {
        assert_eq!(refusal(&bind(64, 64)), None);
        type Case = (fn(&mut GlesTextureBind), &'static str);
        let cases: [Case; 4] = [
            (|b| b.type_ = GLES_TEXTYPE_CUBE, "cube"),
            (|b| b.format = GLES_TEXFMT_R32UI, "does not sample"),
            (|b| b.width = 16384, "16384"),
            (|b| b.stride_texels = 70, "pitch"),
        ];
        let mut seen = std::vec::Vec::new();
        for (make, word) in cases {
            let mut b = bind(64, 64);
            make(&mut b);
            let r = refusal(&b).expect("refused");
            assert!(r.contains(word), "{r}");
            assert!(!seen.contains(&r), "{r} twice");
            seen.push(r);
        }
    }

    /// A binding of an ETC2 class (`GLES_TEXFMT_ETC2_*`), `levels` levels.
    fn etc2(format: u8, w: u32, h: u32, levels: u32) -> GlesTextureBind {
        GlesTextureBind { format, mip_levels: levels as u8, mip_filter: (levels > 1) as u8, ..bind(w, h) }
    }

    /// The GL core's ETC2 classes, with their rows.
    const ETC2_CLASSES: [(u8, Etc2); 5] = [
        (GLES_TEXFMT_ETC2_RGB, Etc2::Rgb8),
        (GLES_TEXFMT_ETC2_SRGB_RGB, Etc2::Srgb8),
        (GLES_TEXFMT_ETC2_RGB_PT, Etc2::Rgb8A1),
        (GLES_TEXFMT_ETC2_RGBA, Etc2::Rgba8),
        (GLES_TEXFMT_ETC2_SRGB_RGBA, Etc2::Srgba8),
    ];

    /// Every ETC2/EAC row is driver/src/v3d/v3d_tex.c's (`formats[]`,
    /// V3D_TEX_FORMAT_ETC2_*), which tools/v3d/tex_check.c compares with
    /// Mesa's built V3D 4.2 table (type, swizzle, return size 16, cpp) and
    /// util_format (4x4 block, sRGB): v3d_packet.xml's types RGB8_ETC2 32,
    /// RGB8_PUNCHTHROUGH_ALPHA1 33, R11_EAC 34, SIGNED_R11_EAC 35, RG11_EAC
    /// 36, SIGNED_RG11_EAC 37, RGBA8_ETC2_EAC 38; swizzles XYZ1, XYZW, X001,
    /// XY01 (X..W -> 2..5); 8 or 16 bytes a block; the sRGB rows the linear
    /// type with the state's bit.
    #[test]
    fn each_etc2_row_is_the_c_tables() {
        let want: [(Etc2, u32, [u8; 4], u32, bool); 10] = [
            (Etc2::Rgb8, 32, [2, 3, 4, 1], 8, false),
            (Etc2::Srgb8, 32, [2, 3, 4, 1], 8, true),
            (Etc2::Rgb8A1, 33, [2, 3, 4, 5], 8, false),
            (Etc2::Srgb8A1, 33, [2, 3, 4, 5], 8, true),
            (Etc2::Rgba8, 38, [2, 3, 4, 5], 16, false),
            (Etc2::Srgba8, 38, [2, 3, 4, 5], 16, true),
            (Etc2::R11, 34, [2, 0, 0, 1], 8, false),
            (Etc2::R11Snorm, 35, [2, 0, 0, 1], 8, false),
            (Etc2::Rg11, 36, [2, 3, 0, 1], 16, false),
            (Etc2::Rg11Snorm, 37, [2, 3, 0, 1], 16, false),
        ];
        for (i, (e, type_, swizzle, bytes, srgb)) in want.into_iter().enumerate() {
            assert_eq!(ETC2_FORMATS[i], e, "the C table's order");
            assert_eq!(etc2_row(e), (type_, swizzle, bytes, srgb), "{e:?}");
        }
        assert_eq!(ETC2_BLOCK, 4);
    }

    /// The GL core's five ETC2 classes sample as their rows: the record's
    /// type and swizzle, the sRGB bit for the two sRGB ones only, a block
    /// for a texel ([`texel_bytes`]: never a TFU byte-copy type), the 16-bit
    /// TMU output (return size 16), and every rule but the sampled set
    /// passes. Until src/backend.rs's V3D_SAMPLED_TEXFMTS lists them, the
    /// set refuses them, with the one message every format outside it gets.
    #[test]
    fn the_gl_cores_etc2_classes_sample_as_their_rows() {
        let wider = V3D_SAMPLED_TEXFMTS | ETC2_CLASSES.iter().fold(0, |m, &(f, _)| m | gles_texfmt_bit(f));
        for (format, e) in ETC2_CLASSES {
            let b = etc2(format, 36, 36, 6);
            let (type_, swizzle, bytes, srgb) = etc2_row(e);
            assert_eq!(etc2_of(format), Some(e));
            assert_eq!(tmu_format(&b), (type_, swizzle), "format {format}");
            assert_eq!((texel_bytes(&b), block_bytes(&b)), (bytes, Some(bytes)), "format {format}");
            assert!(!return_32(&b) && !format_is_integer(format), "format {format}");
            assert_eq!(refusal_in(&b, false, wider), None, "format {format}");
            assert_eq!(
                refusal_in(&b, true, wider),
                Some("a look-up compiled for the 32-bit TMU output reads a texture of a 16-bit format")
            );
            let set = V3D_SAMPLED_TEXFMTS & gles_texfmt_bit(format) != 0;
            assert_eq!(refusal(&b), (!set).then_some("a texel format the V3D does not sample"), "format {format}");
            let l = layout(&b).unwrap();
            assert_eq!(l, layout_compressed(36, 36, 6, 4, 4, bytes).unwrap(), "format {format}: the block layout");
            let s = state(&b, &l, 0x10_0000);
            assert_eq!((s.type_, s.swizzle, s.srgb), (type_, swizzle, srgb), "format {format}");
            assert_eq!((s.base, s.width, s.height, s.max_level), (0x10_0000 + 4096, 36, 36, 5), "format {format}");
        }
        /* the texel pitch is not the blocks' (`gl_level_blocks`): not asked */
        let odd = GlesTextureBind { stride_texels: 3, ..etc2(GLES_TEXFMT_ETC2_RGB, 36, 36, 1) };
        assert_eq!(refusal_in(&odd, false, wider), None);
        /* ES 3.0 has no compressed 3D texture; an array or a cube is laid out */
        let vol = GlesTextureBind { type_: GLES_TEXTYPE_3D, layers: 4, ..etc2(GLES_TEXFMT_ETC2_RGB, 8, 8, 1) };
        assert_eq!(refusal_in(&vol, false, wider), Some("a 3D texture of a compressed format"));
        let arr = GlesTextureBind { type_: GLES_TEXTYPE_2D_ARRAY, ..vol };
        assert_eq!(refusal_in(&arr, false, wider), None);
        let cube = GlesTextureBind { type_: GLES_TEXTYPE_CUBE, layers: 6, ..vol };
        assert_eq!(refusal_in(&cube, false, wider), None);
        /* uncompressed formats have no block */
        for format in [GLES_TEXFMT_RGBA8, GLES_TEXFMT_RGBA32F, GLES_TEXFMT_R32F, GLES_TEXFMT_SRGB8_A8] {
            assert_eq!((etc2_of(format), block_bytes(&GlesTextureBind { format, ..bind(4, 4) })), (None, None));
        }
    }

    /// `layout_compressed` is the C reference's v3d_tex_layout_compressed:
    /// the nine layouts tools/v3d/tex_check.c derives by hand from Mesa's
    /// v3d_setup_slices (part 12, `clayout_case`) and checks on the C, then
    /// tiles with Mesa's own v3d_store_tiled_image. (size, then per level
    /// offset, stride, padded height, ub_pad, tiling.)
    #[test]
    fn the_etc2_layouts_are_the_c_references() {
        const LT: u32 = V3D_TEX_TILING_LINEARTILE;
        const UB1: u32 = V3D_TEX_TILING_UBLINEAR_1_COLUMN;
        const UB2: u32 = V3D_TEX_TILING_UBLINEAR_2_COLUMN;
        const UIF: u32 = V3D_TEX_TILING_UIF_NO_XOR;
        const UIFX: u32 = V3D_TEX_TILING_UIF_XOR;
        type Case = (u32, u32, u32, u32, u32, &'static [(u32, u32, u32, u32, u32)]);
        let cases: [Case; 9] = [
            (
                17,
                9,
                5,
                8,
                4352,
                &[
                    (4096, 64, 4, 0, UB1),
                    (4032, 32, 2, 0, LT),
                    (3968, 32, 2, 0, LT),
                    (3904, 32, 2, 0, LT),
                    (3840, 32, 2, 0, LT),
                ],
            ),
            (
                17,
                9,
                5,
                16,
                4608,
                &[
                    (4096, 128, 4, 0, UB2),
                    (4032, 32, 2, 0, LT),
                    (3968, 32, 2, 0, LT),
                    (3904, 32, 2, 0, LT),
                    (3840, 32, 2, 0, LT),
                ],
            ),
            (
                36,
                36,
                6,
                8,
                5632,
                &[
                    (4096, 128, 12, 0, UB2),
                    (3584, 64, 8, 0, UB1),
                    (3456, 32, 4, 0, LT),
                    (3392, 32, 2, 0, LT),
                    (3328, 32, 2, 0, LT),
                    (3264, 32, 2, 0, LT),
                ],
            ),
            (
                36,
                36,
                6,
                16,
                7168,
                &[
                    (4096, 256, 12, 0, UIF),
                    (3072, 128, 8, 0, UB2),
                    (2816, 64, 4, 0, UB1),
                    (2752, 32, 2, 0, LT),
                    (2688, 32, 2, 0, LT),
                    (2624, 32, 2, 0, LT),
                ],
            ),
            (1, 1, 1, 8, 64, &[(0, 32, 2, 0, LT)]),
            (1, 1, 1, 16, 64, &[(0, 32, 2, 0, LT)]),
            (
                256,
                256,
                9,
                16,
                90112,
                &[
                    (24576, 1024, 64, 0, UIF),
                    (8192, 512, 32, 0, UIF),
                    (4096, 256, 16, 0, UIF),
                    (3072, 128, 8, 0, UB2),
                    (2816, 64, 4, 0, UB1),
                    (2752, 32, 2, 0, LT),
                    (2688, 32, 2, 0, LT),
                    (2624, 32, 2, 0, LT),
                    (2560, 32, 2, 0, LT),
                ],
            ),
            (1024, 1056, 1, 16, 4096 * 280, &[(0, 4096, 280, 4, UIF)]),
            (64, 960, 1, 16, 65536, &[(0, 256, 256, 4, UIFX)]),
        ];
        for (w, h, levels, cpp, size, want) in cases {
            let l = layout_compressed(w, h, levels, 4, 4, cpp).unwrap();
            assert_eq!(
                (l.width, l.height, l.levels, l.cpp, l.size),
                (w, h, levels, cpp, size),
                "{w}x{h} x{levels} cpp {cpp}"
            );
            for (i, &(offset, stride, padded_height, ub_pad, tiling)) in want.iter().enumerate() {
                let s = l.slices[i];
                assert_eq!(
                    s,
                    V3dTexSlice { offset, stride, padded_height, size: stride * padded_height, ub_pad, tiling },
                    "{w}x{h} x{levels} cpp {cpp} level {i}"
                );
            }
        }
        /* the block-aware padding (tex_check's control): 36x36 level 2 is 9
         * texels, 3 blocks, in Mesa's 4 (64 >> 2 = 16 texels); level 1 is 5
         * blocks wide (18 texels), where level 0's 9 blocks minified are 4 */
        let l = layout_compressed(36, 36, 6, 4, 4, 8).unwrap();
        assert_eq!((l.slices[2].stride / 8, l.slices[2].padded_height), (4, 4));
        assert_eq!(gl_level_blocks(36, 36, 1, 8, 2).1, 3, "GL's blocks of level 2");
        assert_eq!(gl_level_blocks(36, 36, 1, 8, 1).1, 5, "GL's blocks of level 1");
    }

    /// The UIF padding and the XOR in blocks: cpp 16 (UIF block 4x4 blocks),
    /// 1024x1024 x2: level 1 is 128x128 blocks, UIF, 32 UB rows (32 % 32 =
    /// 0: no pad, XOR), stride 2048, 256 KiB; level 0 256x256 blocks, 64 UB
    /// rows, XOR, at 262144. 1024x1040 x2: level 1 is 130 blocks high, 132
    /// rows, 33 UB rows (33 % 32 = 1 < 6 and 33 >= 32: pad 5, 152 rows, no
    /// XOR); level 0 260 blocks, 65 UB rows, pad 5, 280 rows.
    ///
    /// Level 1's TOTAL rounded to a page where it could be UIF XOR
    /// (v3d_resource.c:729-734: wider than 4 UIF blocks, higher than 26 UB
    /// rows), seen where it is no page multiple and the levels below are not
    /// either: 256x1248 x11, cpp 16. Level 1 is 32x156 blocks (39 UB rows,
    /// 39 % 32 = 7: no pad, no XOR), stride 512, 79872 bytes (19.5 pages).
    /// Below it, level 2 (pot 256 x 2048: 16x128 blocks, UIF XOR, 32768),
    /// level 3 (8x64, UB2, 8192), level 4 (4x32, UB1, 2048), levels 5..10
    /// (LT: 512, 256, 128, 64, 64, 64): 44096. Level 1 at 44096, its total
    /// rounded to 81920, level 0 at 126016; to a page: +960. Without the
    /// rule level 0 would be at 123968 and the pad 3008. Level 0: 64x312
    /// blocks, UIF, 78 UB rows (78 % 32 = 14: no pad), 1024 x 312 bytes.
    #[test]
    fn an_etc2_level_1_that_could_be_xor_starts_a_page() {
        const XOR: u32 = V3D_TEX_TILING_UIF_XOR;
        const NO_XOR: u32 = V3D_TEX_TILING_UIF_NO_XOR;
        let l = layout_compressed(1024, 1024, 2, 4, 4, 16).unwrap();
        assert_eq!(
            l.slices[1],
            V3dTexSlice { offset: 0, stride: 2048, padded_height: 128, size: 262144, ub_pad: 0, tiling: XOR }
        );
        assert_eq!((l.slices[0].offset, l.slices[0].padded_height, l.slices[0].tiling), (262144, 256, XOR));
        assert_eq!(l.size, 262144 + 4096 * 256);
        let l = layout_compressed(1024, 1040, 2, 4, 4, 16).unwrap();
        assert_eq!((l.slices[1].padded_height, l.slices[1].ub_pad, l.slices[1].tiling), (152, 5, NO_XOR));
        assert_eq!((l.slices[0].offset, l.slices[0].padded_height, l.slices[0].ub_pad), (311296, 280, 5));
        let l = layout_compressed(256, 1248, 11, 4, 4, 16).unwrap();
        let (s0, s1) = (l.slices[0], l.slices[1]);
        assert_eq!((s1.stride, s1.padded_height, s1.size, s1.ub_pad, s1.tiling), (512, 156, 79872, 0, NO_XOR));
        assert_eq!((s1.offset, s0.offset), (44096 + 960, 126016 + 960));
        assert_eq!(s0.offset - s1.offset, 81920, "level 1's total, rounded to a page");
        assert_eq!((s0.stride, s0.padded_height, s0.tiling), (1024, 312, NO_XOR));
        assert_eq!(l.size, s0.offset + 1024 * 312);
        assert_eq!((l.slices[2].size, l.slices[2].tiling, l.slices[10].offset), (32768, XOR, 960));
    }

    /// The refusals of v3d_tex_layout_compressed (tex_check part 12): a
    /// block side of 0 or past 16, a block of other than 8 or 16 bytes, a
    /// size of 0 or 16384, a level count of 0 or past 15.
    #[test]
    fn layout_compressed_refuses_what_the_c_refuses() {
        assert!(layout_compressed(36, 36, 1, 0, 4, 8).is_none());
        assert!(layout_compressed(36, 36, 1, 4, 17, 8).is_none());
        assert!(layout_compressed(36, 36, 1, 4, 4, 4).is_none());
        assert!(layout_compressed(16384, 4, 1, 4, 4, 8).is_none());
        assert!(layout_compressed(4, 0, 1, 4, 4, 8).is_none());
        assert!(layout_compressed(4, 4, 0, 4, 4, 8).is_none());
        assert!(layout_compressed(4, 4, 16, 4, 4, 8).is_none());
        assert!(layout_compressed(16383, 16383, 14, 4, 4, 16).is_some());
    }

    /// The GL core's raster rows of blocks (`gl_level_blocks`): 36x36 x3 of
    /// 8-byte blocks is 9x9 (648 B), 5x5 (200 B), 3x3 (72 B); a cube's six
    /// faces are stacked in each level. `tile` puts block (bx, by) of layer f
    /// at the level's slice in layer f's tree, at the fake store's packed
    /// rows (backend_tests.rs: a box's rows at the slice's stride).
    #[test]
    fn tile_puts_each_etc2_block_grid_into_its_layers_tree() {
        assert_eq!(gl_level_blocks(36, 36, 1, 8, 0), (0, 9, 9));
        assert_eq!(gl_level_blocks(36, 36, 1, 8, 1), (648, 5, 5));
        assert_eq!(gl_level_blocks(36, 36, 1, 8, 2), (848, 3, 3));
        assert_eq!(gl_level_blocks(36, 36, 6, 16, 1), (6 * 81 * 16, 5, 5));
        assert_eq!(gl_bytes(&etc2(GLES_TEXFMT_ETC2_RGB, 36, 36, 3)), 920);
        assert_eq!(gl_bytes(&etc2(GLES_TEXFMT_ETC2_RGB, 36, 36, 1)), 648, "no mip filter: level 0");
        /* a 1x1 level is one block */
        assert_eq!(gl_level_blocks(1, 1, 1, 16, 0), (0, 1, 1));
        let b = GlesTextureBind { type_: GLES_TEXTYPE_CUBE, layers: 6, ..etc2(GLES_TEXFMT_ETC2_RGBA, 36, 36, 3) };
        let l = layout(&b).unwrap();
        let stride = (l.slices[0].offset + l.slices[0].size).next_multiple_of(64);
        assert_eq!(image_bytes(&b, &l), Some(l.size + 5 * stride));
        let n = gl_bytes(&b) as usize;
        assert_eq!(n, 6 * (81 + 25 + 9) * 16);
        let src: std::vec::Vec<u8> = (0..n).map(|i| (i * 13 + 5) as u8).collect();
        let mut image = std::vec![0u8; image_bytes(&b, &l).unwrap() as usize];
        // SAFETY: the image is image_bytes long and the source gl_bytes.
        unsafe { tile(&b, &l, image.as_mut_ptr(), src.as_ptr()) };
        for level in 0..3u32 {
            let (off, gw, gh) = gl_level_blocks(36, 36, 6, 16, level);
            let s = l.slices[level as usize];
            for f in 0..6u32 {
                for (bx, by) in [(0, 0), (gw - 1, 0), (0, gh - 1), (gw - 1, gh - 1)] {
                    let from = (off + ((f * gh + by) * gw + bx) * 16) as usize;
                    let at = (image_offset(&b, &l, level, f).unwrap() + by * s.stride + bx * 16) as usize;
                    assert_eq!(&image[at..at + 16], &src[from..from + 16], "level {level} face {f} block ({bx}, {by})");
                }
            }
        }
    }

    /// A depth binding (R32F, from its texels or its D24S8 buffer) carries
    /// GL's compare function in the record's "Depth Compare Function" (bits
    /// 4-6 of word 0): GlesTextureBind::compare_op bits 0-2, GL's order,
    /// which is the field's (v3d_packet.xml:3-12); bit 3 (a fixed-point
    /// depth) does not reach it; bit 4 is compare off, NEVER, Mesa's
    /// compare-off word. tools/v3d/tex_check.c checks on Mesa's packer that
    /// Mesa's record of each function is the compare-off one with the
    /// function at bit 4 of word 0 (0x00100007 | f << 4 for a nearest,
    /// unmipmapped sampler, 0x00100004 | f << 4 for a linear one), and that
    /// v3d_tex_pack_sampler_compare packs Mesa's bytes for each `compare_op`
    /// given here. The record is read at the TMU's 32-bit output (P1 payload
    /// 0x5, select.rs).
    #[test]
    fn a_depth_binding_carries_its_compare_function() {
        let depth = GlesTextureBind { format: GLES_TEXFMT_R32F, ..bind(64, 64) };
        let buffer = GlesTextureBind { depth_id: 0x4000_0001, ..depth };
        for f in 0..8u8 {
            for (op, what) in [(f, "float depth"), (f | 8, "fixed-point depth")] {
                for b in [GlesTextureBind { compare_op: op, ..depth }, GlesTextureBind { compare_op: op, ..buffer }] {
                    assert_eq!(compare_op(&b), u32::from(op), "{what} {f}");
                    assert_eq!(compare_function(&b), u32::from(f), "{what} {f}");
                    assert_eq!(sampler_compare_bits(&b), u32::from(f) << 4, "{what} {f}");
                    assert!(return_32(&b));
                    assert_eq!(4 | u32::from(return_32(&b)), 0x5, "P1's payload for the binding");
                }
            }
            let off = GlesTextureBind { compare_op: f | 16, ..depth };
            assert_eq!(
                (compare_op(&off), compare_function(&off), sampler_compare_bits(&off)),
                (u32::from(f) | 16, 0, 0)
            );
        }
        /* the GL core's compare-off value, and its fixed-depth variant */
        for op in [16u8, 24] {
            assert_eq!(compare_function(&GlesTextureBind { compare_op: op, ..depth }), 0);
        }
        /* ES 3.0.6 3.8.15: no compare for a texture of another base format */
        for format in [GLES_TEXFMT_RGBA8, GLES_TEXFMT_R16F, GLES_TEXFMT_RGBA32F, GLES_TEXFMT_ETC2_RGB] {
            let b = GlesTextureBind { format, compare_op: 1, ..bind(64, 64) };
            assert_eq!((compare_op(&b), compare_function(&b), sampler_compare_bits(&b)), (COMPARE_OFF, 0, 0));
        }
        /* the compare is not in the V3dSampler (the C struct is unchanged):
         * a cache keyed by it alone would give two compares one record */
        let less = GlesTextureBind { compare_op: 1, ..depth };
        let greater = GlesTextureBind { compare_op: 4, ..depth };
        assert_eq!(sampler(&less), sampler(&greater));
        assert_ne!(compare_function(&less), compare_function(&greater));
    }

    /// The GL filters and wrap modes to the TMU's, as st/mesa and
    /// v3d_upload_sampler_state_variant take them.
    #[test]
    fn the_sampler_is_the_bindings_filters_and_wraps() {
        let mut b = bind(64, 64);
        (b.linear_filter, b.min_linear, b.mip_filter) = (1, 0, 2);
        (b.wrap_s, b.wrap_t) = (GLES_WRAP_MIRRORED_REPEAT, GLES_WRAP_CLAMP_TO_EDGE);
        let s = sampler(&b);
        assert!(!s.mag_nearest && s.min_nearest && !s.mip_nearest && !s.mip_none);
        assert_eq!((s.wrap_s, s.wrap_t), (V3D_WRAP_MODE_MIRROR, V3D_WRAP_MODE_CLAMP));
        b.mip_filter = 1;
        assert!(sampler(&b).mip_nearest && !sampler(&b).mip_none);
        b.mip_filter = 0;
        assert!(sampler(&b).mip_none, "no mip filter clamps the LOD to level 0");
        assert_eq!(sampler(&bind(4, 4)).wrap_s, V3D_WRAP_MODE_REPEAT, "GL's default");
    }
}
