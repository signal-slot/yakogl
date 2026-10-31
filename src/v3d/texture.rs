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
/// order), 1 for a 2D texture.
pub fn layers(b: &GlesTextureBind) -> u32 {
    if b.type_ == GLES_TEXTYPE_CUBE {
        6
    } else {
        1
    }
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
/// ([`texel_bytes`]), or `None` when `v3d_tex_layout` refuses the size.
pub fn layout(b: &GlesTextureBind) -> Option<V3dTexLayout> {
    let mut l = V3dTexLayout::default();
    // SAFETY: `l` is a live value of the mirrored type.
    unsafe { v3d_tex_layout(b.width, b.height, levels_sampled(b), texel_bytes(b), &mut l) }.then_some(l)
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
/// or a cube's six of them `v3d_tex_layer_stride` apart
/// (`v3d_tex_layers_size`); `None` past 32 bits.
pub fn image_bytes(b: &GlesTextureBind, layout: &V3dTexLayout) -> Option<u32> {
    let n = layers(b);
    if n == 1 {
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
/// (`gl_level_layered`) to its own mip tree `f * v3d_tex_layer_stride` into
/// the image (Mesa's v3d_layer_offset, v3d_resource.c:771-779).
///
/// # Safety
/// `image` is writable for `image_bytes(b, layout)` bytes and `src` readable
/// for `gl_bytes(b)` bytes.
pub unsafe fn tile(b: &GlesTextureBind, layout: &V3dTexLayout, image: *mut u8, src: *const u8) {
    let n = layers(b);
    let bpp = texel_bytes(b);
    // SAFETY: a live layout.
    let stride = if n > 1 { unsafe { v3d_tex_layer_stride(layout) } } else { 0 };
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
}
