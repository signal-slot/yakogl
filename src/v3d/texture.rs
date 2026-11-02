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
//! in the backend's tests, as the job builder is.

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
/// order), the layer count of a 2D array and the level 0 depth of a 3D
/// texture (`GlesTextureBind::layers`, which the GL core sets to either; the
/// GL core stacks that many slots at every level, a 3D level `l` holding its
/// `max(depth >> l, 1)` slices in the first ones), 1 for a 2D texture.
pub fn layers(b: &GlesTextureBind) -> u32 {
    match b.type_ {
        GLES_TEXTYPE_CUBE => 6,
        GLES_TEXTYPE_2D_ARRAY | GLES_TEXTYPE_3D => u32::from(b.layers).max(1),
        _ => 1,
    }
}

/// The slices level `level` of a 3D texture `depth` deep has in the tiled
/// image: Mesa's level_depth (v3d_setup_slices, v3d_resource.c:609,
/// :636-639), `depth` at level 0 and `u_minify(pot_depth, level)` below it,
/// pot_depth being `2 * v3d_get_dimension_mpad(depth, 1, 1)` (:573-581), so
/// `2 * next_pot(max(depth >> 1, 1))`. From level 1 that can be more than
/// the GL core's `max(depth >> level, 1)` (a depth of 6 has 4 slices at
/// level 1, GL's 3): padding nothing writes or reads. The same rule as
/// `v3d_tex_level_depth` (driver/src/v3d/v3d_tex.c), which
/// tools/v3d/tex_check.c pins.
pub fn level_depth(depth: u32, level: u32) -> u32 {
    if level == 0 {
        return depth;
    }
    let pot = 2 * (depth >> 1).max(1).next_power_of_two();
    pot.checked_shr(level).unwrap_or(0).max(1)
}

/// Mesa's utile size in texels for `cpp` bytes a texel, width and height
/// (`v3d_utile_width`, `v3d_utile_height`, broadcom/common/v3d_tiling.c: a
/// 64-byte utile); (0, 0) for a size it has no utile for.
const fn utile(cpp: u32) -> (u32, u32) {
    match cpp {
        1 => (8, 8),
        2 => (8, 4),
        4 => (4, 4),
        8 => (4, 2),
        16 => (2, 2),
        _ => (0, 0),
    }
}

/// The layout of a 3D texture `depth` deep from `flat`, the layout
/// `v3d_tex_layout` gives the 2D texture of its width, height, levels and
/// texel size: Mesa's v3d_setup_slices with a depth (v3d_resource.c:583-767),
/// which lays out every slice of a level as the 2D level (the tiling, stride,
/// padded height, ub_pad and so `size` of a slice are the 2D level's: they
/// come from the level's width and height alone, :649-698) and changes only
/// where the levels are. Smallest level first, each `size *
/// level_depth(depth, level)` bytes (`slice_total_size`, :720-722); level
/// 1's total rounded up to a 4 KiB page when it is wider than four UIF
/// blocks and taller than `PAGE_CACHE_MINUS_1_5_UB_ROWS` (26) UIF block
/// rows (:729-733); level 0 then moved to a 4 KiB boundary with every level
/// after it (:749-755). The same arithmetic as `v3d_tex_layout_3d`
/// (driver/src/v3d/v3d_tex.c), which tools/v3d/tex_check.c checks against
/// Mesa; this one runs on the C's 2D geometry. `None` for a depth of 0 or
/// 16384 or more (TEXTURE_SHADER_STATE's 14-bit Image Depth), a texel size
/// the TMU has no utile for, or a size past 32 bits.
pub fn layout_3d(flat: &V3dTexLayout, depth: u32) -> Option<V3dTexLayout> {
    let (utile_w, utile_h) = utile(flat.cpp);
    let levels = flat.levels as usize;
    if depth == 0 || depth >= 1 << 14 || utile_w == 0 || levels == 0 || levels > V3D_TEX_MAX_LEVELS {
        return None;
    }
    let (uif_block_w, uif_block_h) = (2 * utile_w, 2 * utile_h);
    let mut l = *flat;
    let mut offset = 0u32;
    for i in (0..levels).rev() {
        let s = &mut l.slices[i];
        s.offset = offset;
        let mut total = s.size.checked_mul(level_depth(depth, i as u32))?;
        /* the level's padded width in texels and height in rows, the
         * level_width and level_height Mesa tests here */
        if i == 1 && s.stride / flat.cpp > 4 * uif_block_w && s.padded_height > 26 * uif_block_h {
            total = total.checked_next_multiple_of(4096)?;
        }
        offset = offset.checked_add(total)?;
    }
    let pad = l.slices[0].offset.checked_next_multiple_of(4096)? - l.slices[0].offset;
    l.size = offset.checked_add(pad)?;
    for s in &mut l.slices[..levels] {
        s.offset += pad;
    }
    Some(l)
}

/// Where layer `layer` of level `level` of `b`'s tiled image (laid out as
/// [`layout`]) starts, in bytes from the start of the image: a face of a
/// cube map or a layer of a 2D array at `slices[level].offset + layer *
/// v3d_tex_layer_stride` (Mesa's v3d_layer_offset, v3d_resource.c:771-779,
/// with cube_map_stride, the 64-byte-aligned end of one layer's mip tree,
/// :761-763), or a slice of a 3D texture at `slices[level].offset + layer *
/// slices[level].size` (v3d_layer_offset's 3D arm, :777-778: a level's
/// slices one after the other). `None` for a level the layout lacks, a layer
/// past [`layers`] or a slice past [`level_depth`], or an offset past 32
/// bits. To tile the layer, hand `v3d_tex_store_level`/`_box` the image
/// moved by this less `slices[level].offset` (they add the level's offset
/// themselves), as [`tile`] does.
pub fn image_offset(b: &GlesTextureBind, layout: &V3dTexLayout, level: u32, layer: u32) -> Option<u32> {
    if level >= layout.levels {
        return None;
    }
    let s = layout.slices[level as usize];
    let n = layers(b);
    let (count, step) = if b.type_ == GLES_TEXTYPE_3D {
        (level_depth(n, level), s.size)
    } else if n > 1 {
        // SAFETY: a live layout.
        (n, unsafe { v3d_tex_layer_stride(layout) })
    } else {
        (1, 0)
    };
    if layer >= count {
        return None;
    }
    s.offset.checked_add(layer.checked_mul(step)?)
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
    match b.type_ {
        GLES_TEXTYPE_2D if b.layers <= 1 => {}
        GLES_TEXTYPE_CUBE if b.layers == 6 => {
            if b.width != b.height {
                return Some("a cube map whose faces are not square");
            }
        }
        GLES_TEXTYPE_CUBE => return Some("a cube map binding without its six faces"),
        _ => return Some("a 2D array or 3D texture is not sampled yet"),
    }
    if V3D_SAMPLED_TEXFMTS & gles_texfmt_bit(b.format) == 0 {
        return Some(
            "a texel format the V3D does not sample (ETC2, or the RGB10A2 class the GL core keeps in RGBA8 here)",
        );
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
    if b.stride_texels != tex_pitch(b.width) {
        return Some("a texture whose level 0 pitch is not the GL core's pair-padded width");
    }
    None
}

/// Bytes of one texel of `b` in the GL core's storage and in the tiled
/// image: 4 for RGBA8, 1 for L8 and A8, 2 for LA8, R16F and RGB565, 8 for
/// the 16-bit-channel classes and 16 for the 32-bit ones
/// (`gles_texfmt_bytes`; the V3D tiles each format at its own size, as
/// Mesa's resources are: `v3d_tex_layout` takes cpp 1, 2, 4, 8 and 16).
pub fn texel_bytes(b: &GlesTextureBind) -> u32 {
    gles_texfmt_bytes(b.format)
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

/// The texture shader state of a binding whose image (`layout`) starts at
/// `image_gpu` (`v3d_setup_texture_shader_state` and
/// `create_texture_shader_state_bo`): the base pointer at level 0's slice,
/// levels 0 to the last sampled, the type and swizzle of [`tmu_format`].
///
/// A layered image's image depth is [`layers`]: a cube's 6, a 2D array's
/// layer count (`last_layer - first_layer + 1`, v3dx_state.c:938) and a 3D
/// texture's level 0 depth (`prsc->depth0`, :935-936; the TMU minifies it
/// itself). Its array stride is Mesa's `cube_map_stride / 64` (:961): a 2D
/// array's the tree stride `v3d_tex_layer_stride` (v3d_resource.c:761-763,
/// what `v3d_tex_state_from_layout` sets for a cube), a 3D texture's one
/// slice of level 0, `slices[0].size` (:765-766, as
/// `v3d_tex_state_from_layout_3d` sets it). The texture type is the
/// format's whatever the dimensionality: the record has no dimension field;
/// the look-up's TMU configuration tells the TMU 2D, array or 3D.
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
         * the RGBA8 row's */
        srgb: b.format == GLES_TEXFMT_SRGB8_A8,
        ..V3dTexState::default()
    };
    // SAFETY: both are live values of the mirrored types.
    unsafe { v3d_tex_state_from_layout(&mut s, layout) };
    /* a cube is `depth` layers `array_stride_64` apart (which
     * v3d_tex_state_from_layout set): Mesa's image_depth = the layer count,
     * v3dx_state.c:936-938 */
    s.depth = layers(b);
    /* a 2D array keeps the tree stride v3d_tex_state_from_layout set, a cube's;
     * a 3D texture's is one slice of level 0 (v3d_tex_state_from_layout_3d) */
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
/// layer's mip tree, which is also the layout of every layer of a cube map
/// or a 2D array (Mesa gives them the 2D arithmetic, v3d_resource.c:761-779;
/// [`image_bytes`] and [`image_offset`] add the layers), or for a 3D texture
/// [`layout_3d`] of it, [`layers`] deep.
pub fn layout(b: &GlesTextureBind) -> Option<V3dTexLayout> {
    let mut l = V3dTexLayout::default();
    // SAFETY: `l` is a live value of the mirrored type.
    if !unsafe { v3d_tex_layout(b.width, b.height, levels_sampled(b), texel_bytes(b), &mut l) } {
        return None;
    }
    if b.type_ == GLES_TEXTYPE_3D {
        return layout_3d(&l, layers(b));
    }
    Some(l)
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
/// or a cube's six of them or a 2D array's layers `v3d_tex_layer_stride`
/// apart (`v3d_tex_layers_size`, Mesa's `rsc->size += cube_map_stride *
/// (array_size - 1)`, v3d_resource.c:764), or a 3D texture's whole
/// [`layout_3d`], every slice of every level in its `size`; `None` past 32
/// bits.
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
/// start of the level after the last one sampled.
pub fn gl_bytes(b: &GlesTextureBind) -> u32 {
    gl_level_layered(b.width, b.height, layers(b), texel_bytes(b), levels_sampled(b)).0
}

/// Tile the sampled levels of every layer of `b` into `image`
/// (`image_bytes` bytes) from the GL core's storage at `src` (`gl_bytes`
/// bytes): layer f's level L from its stacked rows in the GL core's level
/// (`gl_level_layered`) to where [`image_offset`] puts it (Mesa's
/// v3d_layer_offset, v3d_resource.c:771-779): a face's or an array layer's
/// own mip tree `f * v3d_tex_layer_stride` into the image, or a 3D slice
/// after the level's slices before it. A 3D level L has the GL core's
/// `max(depth >> L, 1)` slices only; the tiled image's padding slices past
/// them stay as they are.
///
/// # Safety
/// `image` is writable for `image_bytes(b, layout)` bytes and `src` readable
/// for `gl_bytes(b)` bytes.
pub unsafe fn tile(b: &GlesTextureBind, layout: &V3dTexLayout, image: *mut u8, src: *const u8) {
    let n = layers(b);
    let bpp = texel_bytes(b);
    for level in 0..layout.levels {
        let (off, pitch, rows) = gl_level_layered(b.width, b.height, n, bpp, level);
        let slices = if b.type_ == GLES_TEXTYPE_3D { n.checked_shr(level).unwrap_or(0).max(1) } else { n };
        for f in 0..slices {
            let Some(at) = image_offset(b, layout, level, f) else { continue };
            let from = off + f * rows * pitch * bpp;
            // SAFETY: the caller's contract; layer f of the level starts
            // `at` into the image, and v3d_tex_store_level reads the level's
            // rows at `pitch * bpp` and writes the level at its offset into
            // the image it is handed, here the image moved so that the
            // offset lands on `at`.
            unsafe {
                v3d_tex_store_level(
                    image.add((at - layout.slices[level as usize].offset) as usize),
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

    /// A cube is sampled only whole and square; 2D arrays and 3D are still
    /// refused by name, and its storage spans every face of the sampled
    /// levels (to the start of the next level).
    #[test]
    fn a_cube_is_sampled_whole_and_square() {
        assert_eq!(refusal(&cube(8, 4)), None);
        assert_eq!(layers(&cube(8, 1)), 6);
        let wide = GlesTextureBind { width: 16, stride_texels: 16, ..cube(8, 1) };
        assert_eq!(refusal(&wide), Some("a cube map whose faces are not square"));
        let one = GlesTextureBind { layers: 1, ..cube(8, 1) };
        assert_eq!(refusal(&one), Some("a cube map binding without its six faces"));
        let arr = GlesTextureBind { type_: GLES_TEXTYPE_2D_ARRAY, ..cube(8, 1) };
        assert_eq!(refusal(&arr), Some("a 2D array or 3D texture is not sampled yet"));
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
            (|b| b.format = GLES_TEXFMT_ETC2_RGB, "ETC2"),
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

    /* ---- 2D arrays and 3D textures (docs/notes/v3d-tex-layout-es3.md) ----
     *
     * This module links the backend tests' fake v3d_tex.c (a packed layout,
     * backend_tests.rs), so Mesa's numbers are pinned here on layouts built
     * by hand with Mesa's v3d_setup_slices geometry for RGBA8 (utile 4x4
     * texels, UIF block 8x8): `layout_3d`, `image_offset`, `image_bytes` and
     * `state` take the layout as given. tools/v3d/tex_check.c pins the same
     * cases on the real C against Mesa. */

    /// `enum v3d_tex_tiling`'s UB-linear values (Mesa's v3d_tiling_mode).
    const UB1: u32 = 2;
    const UB2: u32 = 3;
    const LT: u32 = V3D_TEX_TILING_LINEARTILE;
    const UIF: u32 = V3D_TEX_TILING_UIF_NO_XOR;

    /// An RGBA8 layout `w` x `h` from per-level (offset, stride, padded
    /// height, tiling), level 0 first; each slice's size is stride x height.
    fn hand_layout(w: u32, h: u32, size: u32, slices: &[(u32, u32, u32, u32)]) -> V3dTexLayout {
        let mut l =
            V3dTexLayout { width: w, height: h, levels: slices.len() as u32, cpp: 4, size, ..Default::default() };
        for (i, &(offset, stride, padded_height, tiling)) in slices.iter().enumerate() {
            l.slices[i] =
                V3dTexSlice { offset, stride, padded_height, size: stride * padded_height, ub_pad: 0, tiling };
        }
        l
    }

    fn array(w: u32, h: u32, layers: u16, levels: u32) -> GlesTextureBind {
        GlesTextureBind {
            type_: GLES_TEXTYPE_2D_ARRAY,
            layers,
            mip_levels: levels as u8,
            mip_filter: (levels > 1) as u8,
            ..bind(w, h)
        }
    }

    fn tex3d(w: u32, h: u32, depth: u16, levels: u32) -> GlesTextureBind {
        GlesTextureBind { type_: GLES_TEXTYPE_3D, ..array(w, h, depth, levels) }
    }

    /// 64x32 RGBA8, 7 levels, one layer as Mesa's v3d_setup_slices lays it
    /// out (smallest first): 6, 5, 4 (1x1, 2x1, 4x2) LT, one 4x4 utile, 16 x
    /// 4 = 64 bytes each at 0, 64, 128; 3 (8x4: 4 rows <= utile_h) LT, 32 x 4
    /// = 128 at 192; 2 (16x8: <= 2 UIF blocks wide) UB 2 columns, 64 x 8 =
    /// 512 at 320; 1 (32x16) UIF, 128 x 16 = 2048 at 832 (2 UB rows, under
    /// the 32-row page cache: no ub_pad; 2 % 32 != 0: no XOR); 0 (64x32) UIF
    /// 256 x 32 = 8192 at 2880 (4 UB rows: no pad, no XOR); 11072 in all,
    /// level 0 moved to 4096 (+1216, :749-755): 12288.
    fn mesa_64x32x7() -> V3dTexLayout {
        hand_layout(
            64,
            32,
            12288,
            &[
                (4096, 256, 32, UIF),
                (2048, 128, 16, UIF),
                (1536, 64, 8, UB2),
                (1408, 32, 4, LT),
                (1344, 16, 4, LT),
                (1280, 16, 4, LT),
                (1216, 16, 4, LT),
            ],
        )
    }

    /// Mesa's level_depth (v3d_resource.c:609, :636-639): level 0 the depth,
    /// level i u_minify(2 * next_pot(u_minify(depth, 1)), i).
    #[test]
    fn a_3d_level_has_mesas_power_of_two_padded_depth() {
        let depths = |d: u32, n: u32| (0..n).map(|l| level_depth(d, l)).collect::<std::vec::Vec<_>>();
        // 8: pot_depth 2 * next_pot(4) = 8: 8, 4, 2, 1, 1
        assert_eq!(depths(8, 5), [8, 4, 2, 1, 1]);
        // 6: pot_depth 2 * next_pot(3) = 8: level 1 has 4 slices, GL's 3
        assert_eq!(depths(6, 4), [6, 4, 2, 1]);
        // 5: 2 * next_pot(2) = 4: 2 at level 1; 9: 2 * next_pot(4) = 8: 4
        assert_eq!(depths(5, 3), [5, 2, 1]);
        assert_eq!(depths(9, 4), [9, 4, 2, 1]);
        // 1: 2 * next_pot(u_minify(1, 1) = 1) = 2: one slice at every level
        assert_eq!(depths(1, 15), [1; 15]);
        assert_eq!(level_depth(16, 40), 1, "a shift past the width is 1, not a panic");
    }

    /// A 2D array and a 3D texture: their layers ([`layers`], the GL core's
    /// `GlesTextureBind::layers`), the GL core's storage of every slot, and
    /// the refusal by name, which stays until the backend uploads them
    /// (v3d-qpu-array-3d).
    #[test]
    fn a_2d_array_and_a_3d_texture_are_laid_out_but_still_refused() {
        let (arr, t3) = (array(64, 32, 4, 7), tex3d(32, 32, 8, 6));
        assert_eq!((layers(&arr), layers(&t3)), (4, 8));
        assert_eq!(layers(&GlesTextureBind { layers: 0, ..arr }), 1, "no layer count: one");
        for b in [arr, t3] {
            assert_eq!(refusal(&b), Some("a 2D array or 3D texture is not sampled yet"));
            assert_eq!(refusal_for(&b, true), Some("a 2D array or 3D texture is not sampled yet"));
        }
        /* the GL core stacks `layers` slots at every level of both */
        assert_eq!(gl_bytes(&arr), gl_level_layered(64, 32, 4, 4, 7).0);
        assert_eq!(gl_bytes(&t3), gl_level_layered(32, 32, 8, 4, 6).0);
        /* an array's layout is one layer's tree (the C's 2D layout), a 3D
         * texture's that tree rearranged by layout_3d */
        assert_eq!(
            layout(&array(64, 32, 4, 7)),
            layout(&GlesTextureBind { mip_levels: 7, mip_filter: 1, ..bind(64, 32) })
        );
        let mut flat3 = V3dTexLayout::default();
        // SAFETY: a live out-parameter.
        assert!(unsafe { v3d_tex_layout(32, 32, 6, 4, &mut flat3) });
        assert_eq!(layout(&t3), layout_3d(&flat3, 8));
    }

    /// A 2D array of 4 64x32 RGBA8 layers with its full chain: the cube's
    /// path (v3d_resource.c:761-764), four trees `cube_map_stride` apart.
    #[test]
    fn a_2d_array_is_four_mip_trees_a_tree_stride_apart() {
        let b = array(64, 32, 4, 7);
        let l = mesa_64x32x7();
        // the stride: align(slices[0].offset + slices[0].size, 64) =
        // align(4096 + 8192, 64) = 12288 (v3d_resource.c:761-763)
        // SAFETY: a live layout.
        assert_eq!(unsafe { v3d_tex_layer_stride(&l) }, 12288);
        // rsc->size += cube_map_stride * (array_size - 1) (:764): 12288 + 3 x
        // 12288
        assert_eq!(image_bytes(&b, &l), Some(49152));
        // v3d_layer_offset's non-3D arm (:778-779): slices[level].offset +
        // layer x stride; level 2 of layer 3: 1536 + 3 x 12288
        assert_eq!(image_offset(&b, &l, 2, 3), Some(38400));
        assert_eq!(image_offset(&b, &l, 0, 0), Some(4096), "level 0 of layer 0: slices[0].offset");
        assert_eq!(image_offset(&b, &l, 6, 3), Some(1216 + 3 * 12288));
        assert_eq!(image_offset(&b, &l, 0, 4), None, "layer 4 of 4");
        assert_eq!(image_offset(&b, &l, 7, 0), None, "level 7 of 7");
        /* the record (v3d_setup_texture_shader_state, v3dx_state.c:921-961):
         * image depth = last_layer - first_layer + 1 = 4 (:938), array
         * stride = cube_map_stride / 64 = 12288 / 64 = 192 (:961), the base
         * at layer 0's level 0 (:957-960), levels 0..6. The stride is
         * v3d_tex_state_from_layout's, the cube's (tex_check pins 192 on the
         * real C for this layout); this module's fake writes size / 64,
         * which for this layout is the same 192. */
        let s = state(&b, &l, 0x0040_0000);
        assert_eq!((s.width, s.height, s.depth, s.array_stride_64), (64, 32, 4, 192));
        assert_eq!((s.base, s.base_level, s.max_level), (0x0040_0000 + 4096, 0, 6));
        assert_eq!((s.type_, s.swizzle), tmu_format(&b), "the format's type: no dimension in the record");
    }

    /// A 32x32x8 RGBA8 3D texture with its full chain (6 levels).
    #[test]
    fn a_3d_texture_lays_out_its_slices_level_by_level() {
        /* the 2D geometry of each level (v3d_setup_slices, as a 32x32 2D
         * texture): 5, 4, 3 (1x1, 2x2, 4x4) LT 16 x 4 = 64; 2 (8x8: one UIF
         * block wide) UB 1 column, 32 x 8 = 256; 1 (16x16) UB 2 columns, 64 x
         * 16 = 1024; 0 (32x32) UIF, 128 x 32 = 4096 (4 UB rows: no pad, no
         * XOR). The 2D offsets are not used. */
        let flat = hand_layout(
            32,
            32,
            0,
            &[(0, 128, 32, UIF), (0, 64, 16, UB2), (0, 32, 8, UB1), (0, 16, 4, LT), (0, 16, 4, LT), (0, 16, 4, LT)],
        );
        let l = layout_3d(&flat, 8).unwrap();
        /* level_depth 8, 4, 2, 1, 1, 1 (pot_depth 2 * next_pot(4) = 8);
         * slice_total_size = size x level_depth (:720-722), smallest first:
         * 5 at 0 (64), 4 at 64 (64), 3 at 128 (64), 2 at 192 (2 x 256 = 512),
         * 1 at 704 (4 x 1024 = 4096; 16 texels wide is not past four UIF
         * blocks, 32: no page alignment, :729-733), 0 at 4800 (8 x 4096 =
         * 32768), 37568 in all; level 0 to 4 KiB, 8192 (+3392, :749-755) */
        let offsets: std::vec::Vec<u32> = l.slices[..6].iter().map(|s| s.offset).collect();
        assert_eq!(offsets, [8192, 4096, 3584, 3520, 3456, 3392]);
        assert_eq!(l.size, 37568 + 3392);
        for i in 0..6 {
            let (s, f) = (l.slices[i], flat.slices[i]);
            assert_eq!((s.stride, s.padded_height, s.size, s.tiling), (f.stride, f.padded_height, f.size, f.tiling));
        }
        let b = tex3d(32, 32, 8, 6);
        assert_eq!(image_bytes(&b, &l), Some(40960), "every slice is inside the layout's size");
        /* v3d_layer_offset's 3D arm (:777-778): slices[level].offset + slice
         * x slices[level].size: slice 7 of level 0 at 8192 + 7 x 4096 = 36864
         * (it ends at the size); slice 3 of level 1 at 4096 + 3 x 1024 = 7168
         * (it ends at level 0); slice 1 of level 2 at 3584 + 256 = 3840 */
        assert_eq!(image_offset(&b, &l, 0, 7), Some(36864));
        assert_eq!(image_offset(&b, &l, 1, 3), Some(7168));
        assert_eq!(image_offset(&b, &l, 2, 1), Some(3840));
        assert_eq!(image_offset(&b, &l, 5, 0), Some(3392));
        assert_eq!(image_offset(&b, &l, 1, 4), None, "level 1 has 4 slices");
        assert_eq!(image_offset(&b, &l, 3, 1), None, "level 3 has 1 slice");
        /* the record: image depth = depth0 = 8 (v3dx_state.c:935-936), array
         * stride = cube_map_stride / 64 with cube_map_stride = slices[0].size
         * for 3D (v3d_resource.c:765-766): 4096 / 64 = 64 */
        let s = state(&b, &l, 0x0040_0000);
        assert_eq!((s.width, s.height, s.depth, s.array_stride_64), (32, 32, 8, 64));
        assert_eq!((s.base, s.max_level), (0x0040_0000 + 8192, 5));
        assert_eq!((s.type_, s.swizzle), tmu_format(&b));
    }

    /// A non-power-of-two 3D texture, 13x7x6 RGBA8 with 3 levels: Mesa's
    /// padded depths are more than GL's.
    #[test]
    fn a_non_power_of_two_3d_texture_pads_its_depth_as_mesa() {
        /* the 2D geometry (tex_check's 13x7 x3): pot 2 * next_pot(6) = 16
         * wide, 2 * next_pot(3) = 8 high; 2: 16>>2 x 8>>2 = 4x2 LT, 16 x 4 =
         * 64; 1: 6x3 (u_minify of 13x7) LT, 8 x 4 texels: 32 x 4 = 128; 0:
         * 13x7 UB 2 columns, 16 x 8 texels: 64 x 8 = 512 */
        let flat = hand_layout(13, 7, 0, &[(0, 64, 8, UB2), (0, 32, 4, LT), (0, 16, 4, LT)]);
        let l = layout_3d(&flat, 6).unwrap();
        /* level_depth 6, 4, 2 (pot_depth 2 * next_pot(3) = 8), where GL has
         * 6, 3, 1: 2 at 0 (2 x 64 = 128), 1 at 128 (4 x 128 = 512), 0 at 640
         * (6 x 512 = 3072), 3712; level 0 to 4096 (+3456) */
        let offsets: std::vec::Vec<u32> = l.slices[..3].iter().map(|s| s.offset).collect();
        assert_eq!(offsets, [4096, 3584, 3456]);
        assert_eq!(l.size, 3712 + 3456);
        let b = tex3d(13, 7, 6, 3);
        assert_eq!(image_offset(&b, &l, 1, 3), Some(3584 + 3 * 128), "level 1's padding slice is allocated");
        assert_eq!(image_offset(&b, &l, 2, 1), Some(3456 + 64));
        /* the record: depth 6, array stride 512 / 64 = 8 */
        let s = state(&b, &l, 0x0040_0000);
        assert_eq!((s.width, s.height, s.depth, s.array_stride_64, s.max_level), (13, 7, 6, 8, 2));
    }

    /// Level 1's page alignment (v3d_resource.c:729-733) is taken on the
    /// level's slices together, after slice_total_size = size x level_depth.
    #[test]
    fn a_3d_level_1_is_page_aligned_with_all_its_slices() {
        /* 192x624 RGBA8, 2 levels: 1 (96x312) UIF 384 x 312 = 119808 (39 UB
         * rows, 39 % 32 = 7: no pad, no XOR); 0 (192x624) UIF 768 x 624 =
         * 479232 (78 rows, 14: no pad, no XOR) */
        let flat = hand_layout(192, 624, 0, &[(0, 768, 624, UIF), (0, 384, 312, UIF)]);
        /* depth 5: level 1 has 2 slices (2 * next_pot(2) = 4, u_minify 1), 2
         * x 119808 = 239616; 96 texels > 32 (four UIF blocks) and 312 rows >
         * 208 (26 UB rows): aligned to 4096, 241664; level 0's five slices
         * at 241664 (already 4 KiB aligned), 5 x 479232 = 2396160 */
        let l = layout_3d(&flat, 5).unwrap();
        assert_eq!((l.slices[1].offset, l.slices[0].offset, l.size), (0, 241664, 241664 + 2396160));
        /* depth 1, one slice a level: 119808 aligned to 122880, as the 2D
         * layout aligns it */
        let l = layout_3d(&flat, 1).unwrap();
        assert_eq!((l.slices[1].offset, l.slices[0].offset, l.size), (0, 122880, 122880 + 479232));
    }

    /// layout_3d's refusals: a depth outside the 14-bit Image Depth, a
    /// texel size with no utile, a size past 32 bits.
    #[test]
    fn layout_3d_refuses_what_the_tmu_cannot_address() {
        let flat = hand_layout(4, 4, 0, &[(0, 16, 4, LT)]);
        assert!(layout_3d(&flat, 1).is_some());
        assert_eq!(layout_3d(&flat, 0), None);
        assert_eq!(layout_3d(&flat, 1 << 14), None);
        assert_eq!(layout_3d(&V3dTexLayout { cpp: 3, ..flat }, 2), None);
        /* 4096x4096 RGBA8: 64 MiB a slice, 4096 slices */
        let big = hand_layout(4096, 4096, 0, &[(0, 16384, 4096, UIF)]);
        assert_eq!(layout_3d(&big, 4096), None);
        assert!(layout_3d(&big, 63).is_some());
    }

    /// `tile` puts the GL core's slices of a 3D level where `image_offset`
    /// says, `max(depth >> level, 1)` of them, and leaves Mesa's padding
    /// slices alone (here on the fake layout, whose "tiling" is a row copy).
    #[test]
    fn tile_places_each_3d_slice_and_skips_the_padding() {
        let b = tex3d(4, 4, 6, 3);
        let l = layout(&b).unwrap();
        let src: std::vec::Vec<u8> = (0..gl_bytes(&b)).map(|i| (i % 251) as u8 + 1).collect();
        let mut image = std::vec![0u8; image_bytes(&b, &l).unwrap() as usize];
        // SAFETY: both buffers have the sizes tile's contract asks for.
        unsafe { tile(&b, &l, image.as_mut_ptr(), src.as_ptr()) };
        for level in 0..3 {
            let (off, pitch, rows) = gl_level_layered(4, 4, 6, 4, level);
            let bytes = (pitch * rows * 4) as usize;
            for k in 0..level_depth(6, level) {
                let at = image_offset(&b, &l, level, k).unwrap() as usize;
                let got = &image[at..at + bytes];
                if k < (6 >> level).max(1) {
                    let from = (off + k * rows * pitch * 4) as usize;
                    assert_eq!(got, &src[from..from + bytes], "level {level} slice {k}");
                } else {
                    assert!(got.iter().all(|&v| v == 0), "level {level} padding slice {k} untouched");
                }
            }
        }
    }
}
