// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Texture descriptors and sampler state for a Vivante draw.
//!
//! On HALTI5 a texture is a 256-byte descriptor in memory (addresses, format,
//! size, mip chain) that NTE_DESCRIPTOR_ADDR points at, plus a handful of
//! per-slot sampler registers (wrap, filters, LOD range). The descriptor cache
//! has to be invalidated for a slot whose descriptor address changed, which is
//! every draw here: descriptors are built into the frame arena, so each draw's
//! are at a fresh address.
//!
//! The two stages share one 32-entry slot array on this part: the fragment
//! stage takes the low slots and VS_SAMPLER_BASE tells the vertex stage where
//! its own start.

use crate::backend::{
    gles_texfmt_bytes, GlesTextureBind, GLES_TEXFMT_A8, GLES_TEXFMT_ETC2_RGB, GLES_TEXFMT_ETC2_RGBA,
    GLES_TEXFMT_ETC2_RGB_PT, GLES_TEXFMT_L8, GLES_TEXFMT_LA8, GLES_TEXFMT_R16F, GLES_TEXFMT_R32F, GLES_TEXFMT_RGB565,
    GLES_TEXFMT_RGBA16F, GLES_TEXTYPE_2D_ARRAY, GLES_TEXTYPE_3D, GLES_TEXTYPE_CUBE, GLES_WRAP_CLAMP_TO_EDGE,
    GLES_WRAP_MIRRORED_REPEAT,
};
use crate::shaderc::{ShcShader, SHC_UNIFORM_NO_BLOCK};
use crate::util::{Global, ENOMEM};

use super::cmd::GcCmd;
use super::regs::*;
use super::state::*;

/// log2 of a texture dimension in 8.8 fixed point, the form the descriptor's
/// LOG_SIZE_EXT field takes.
fn log2_fixp88(v: u32) -> u32 {
    if v == 0 {
        return 0;
    }
    let mut whole = 0u32;
    while (1u32 << (whole + 1)) <= v {
        whole += 1;
    }
    /* the fraction between 2^whole and 2^(whole+1), linear is close enough
     * for the LOD clamp this feeds */
    let frac = ((v - (1 << whole)) << 8) / (1 << whole);
    (whole << 8) | (frac & 0xff)
}

struct GcTexfmt {
    /// TEXDESC_CONFIG0 format, 0 when the extended set is used.
    format: u32,
    /// TEXDESC_CONFIG1 format, 0 otherwise.
    format_ext: u32,
    /// Block format: the addressing mode stays tiled.
    compressed: bool,
    /// The format carries no alpha: the swizzle reads 1.
    no_alpha: bool,
}

/// Bytes in one 4x4 block of a compressed format.
fn block_bytes(format: u8) -> u32 {
    if format == GLES_TEXFMT_ETC2_RGBA {
        16
    } else {
        8
    }
}

/// Is `format` a block format (addressed TILED, sized in blocks)?
pub(super) fn is_block_format(format: u8) -> bool {
    format_of(format).compressed
}

fn texture_format(t: &GlesTextureBind) -> GcTexfmt {
    format_of(t.format)
}

fn format_of(format: u8) -> GcTexfmt {
    let mut f = GcTexfmt { format: 0, format_ext: 0, compressed: false, no_alpha: false };

    match format {
        GLES_TEXFMT_RGB565 => f.format = GC_TEX_FORMAT_R5G6B5,
        /* the GL core stores these narrow only under gc_debug_narrow_la */
        GLES_TEXFMT_L8 => f.format = GC_TEX_FORMAT_L8,
        GLES_TEXFMT_A8 => f.format = GC_TEX_FORMAT_A8,
        GLES_TEXFMT_LA8 => f.format = GC_TEX_FORMAT_A8L8,
        GLES_TEXFMT_ETC2_RGB => {
            f.format = GC_TEX_FORMAT_ETC1; /* ETC2 RGB decodes ETC1 as a subset */
            f.compressed = true;
            f.no_alpha = true;
        }
        GLES_TEXFMT_ETC2_RGBA => {
            f.format_ext = GC_TEX_FORMAT_EXT_RGBA8_ETC2_EAC;
            f.compressed = true;
        }
        GLES_TEXFMT_ETC2_RGB_PT => {
            f.format_ext = GC_TEX_FORMAT_EXT_RGB8_PUNCHTHROUGH_ALPHA1_ETC2;
            f.compressed = true;
        }
        GLES_TEXFMT_RGBA16F => f.format_ext = GC_TEX_FORMAT_EXT_A16B16G16R16F,
        GLES_TEXFMT_R16F => f.format_ext = GC_TEX_FORMAT_EXT_R16F,
        GLES_TEXFMT_R32F => f.format_ext = GC_TEX_FORMAT_EXT_R32F,
        /* RGBA8: R,G,B,A in memory as the GL core wrote it, unless the pixel
         * engine produced it */
        _ => f.format = GC_TEX_FORMAT_A8B8G8R8,
    }
    f
}

/// Build one descriptor in the frame arena. Returns its GPU address, or 0.
fn build_descriptor(f: &mut GcFrame, t: &GlesTextureBind) -> u64 {
    let mut fmt = texture_format(t);
    /* the channel order is the storage's, even when a tiled copy is sampled */
    /* B,G,R,A texels (an XRGB8888 image, or a texture a draw rendered into:
     * the GL core converted it then and says so in `bgra`) take the format
     * that puts them back in R,G,B,A for the shader */
    let pe_order = fmt.format == GC_TEX_FORMAT_A8B8G8R8 && t.bgra != 0;
    /* a mipmapped layered texture is sampled from its tiled copy (tiled_copy.rs) */
    let copy = super::tiled_copy::lookup(t, super::backend::gc_kick_id());
    let levels = t.mip_levels.max(1) as usize;
    /* A cube map, a 2D array and a 3D texture are one stack of layers here and
     * the texture unit walks it: the type says which, SLICE is the step from
     * one layer to the next, and 3D_CONFIG's depth is how many a volume has.
     * Mesa's descriptor for a 6 x 8x8 cube reads TYPE 5, SIZE 8x8 and
     * SLICE = stride * height, which is this.
     *
     * SLICE is one value, level 0's, while a level's layer step halves with
     * it; the levels themselves are placed below, all layers each. Where the
     * hardware takes a level's layer step from is not measured: with one
     * LINEAR_STRIDE for the chain, stride * its own height is what the GL
     * core stores (dEQP texturecube on the board is the check). */
    let volume = t.type_ == GLES_TEXTYPE_3D || t.type_ == GLES_TEXTYPE_2D_ARRAY;
    let hw_type = if t.type_ == GLES_TEXTYPE_CUBE {
        GC_TEX_TYPE_CUBE_MAP
    } else if volume {
        GC_TEX_TYPE_3D
    } else {
        GC_TEX_TYPE_2D
    };
    /* A 2D array is a volume to this part -- Mesa's translate_texture_target
     * returns TEXTURE_TYPE_3D for both -- and CONFIG1's TEXTURE_ARRAY bit is
     * what tells the two apart (an array's layers do not filter into each
     * other). A cube's six faces are not a depth: 3D_CONFIG stays 1. */
    let depth = if volume { t.layers.max(1) as u32 } else { 1 };

    /* 64, not the descriptor's own 256 -- and the reason is "no measured
     * reason to change it", which is weaker than what this comment used to
     * claim. The claim it used to make was wrong, so it is written out here
     * rather than quietly deleted.
     *
     * It said that aligning to 256 costs twenty gl_test cases (32 of 38 at 64,
     * 12 of 38 at 256) by exhausting the frame arena. Both halves are false:
     *
     *   - The mechanism cannot happen. The allocation is GC_TEXDESC_BYTES
     *     either way; only the padding before it changes, by at most 192
     *     bytes. Exhausting the 1 MiB arena that way needs some 2,000 texture
     *     descriptors in a single frame, and gl_test records tens. The
     *     arithmetic was never done -- the explanation just sounded right.
     *   - The measurement does not reproduce. 20 runs of each build,
     *     alternating, offline shader table, nothing else holding the device:
     *     align 64 gave 30 of 38 in 20 runs out of 20, align 256 gave 30 in 16
     *     and 31 in 4, and the arena-full path below did not fire once in
     *     either column. The original 12 of 38 was almost certainly a second
     *     client running against a pool the previous one had left dirty (the
     *     collapse fixed in be1bd17), which put every reading taken that
     *     afternoon into the same trap -- a confound, not noise.
     *
     * So 64 stays because nothing measured asks for 256, not because 256 is
     * dangerous. Raising it is a hardware-documentation question (etnaviv is
     * said to align HALTI5 descriptors to their own size; unverified here),
     * and the regression set says it would cost nothing. */
    let Some((cpu, gpu)) = f.arena_alloc(GC_TEXDESC_BYTES, 64) else {
        return 0;
    };
    // SAFETY: the arena handed out GC_TEXDESC_BYTES at `cpu`, aligned to that
    // size, so it holds this many dwords and nothing else refers to them.
    let desc: &mut [u32] = unsafe { core::slice::from_raw_parts_mut(cpu as *mut u32, GC_TEXDESC_BYTES / 4) };
    desc.fill(0);

    if pe_order {
        fmt.format = GC_TEX_FORMAT_A8R8G8B8;
    }

    let tiled = fmt.compressed || copy.is_some();
    desc[GC_TEXDESC_CONFIG0 as usize / 4] = texdesc_type(hw_type)
        | texdesc_format(fmt.format)
        | texdesc_addressing_mode(if tiled { GC_TEX_ADDRESSING_TILED } else { GC_TEX_ADDRESSING_LINEAR });
    /* An RGB format has no alpha to read, and GL says such a texture samples
     * alpha as 1, so the descriptor's own swizzle says so -- Mesa writes
     * SWIZZLE_A = ONE for the same ETC1 texture (CONFIG1 0x00521000 against
     * 0x00321000 before this). Without it an ETC1 block comes back
     * transparent, whatever its colour. The same for every texture whose GL
     * base format has no alpha (`GlesTextureBind::no_alpha`: GL_RGB,
     * GL_LUMINANCE, RGB565, ...): its storage is RGBA8 or 565, and the alpha
     * byte of an RGBA8 one nobody wrote -- a texture rendered into but never
     * cleared -- is not 1. Mesa's descriptor for dEQP texture.vertex.2d's
     * GL_RGB texture reads SWIZZLE_A = ONE too (golden mode 26: CONFIG1
     * 0x08521000). */
    /* A depth texture under GL_OES_depth_texture samples as luminance,
     * (d, d, d, 1) (`GlesTextureBind::depth_luminance`): green and blue read
     * the red channel of its R32F texels (alpha is already 1: `no_alpha`, or
     * the one-channel format's own). */
    let lum = t.depth_luminance != 0;
    desc[GC_TEXDESC_CONFIG1 as usize / 4] = texdesc_format_ext(fmt.format_ext)
        | texdesc_swizzle_r(GC_SWIZZLE_R)
        | texdesc_swizzle_g(if lum { GC_SWIZZLE_R } else { GC_SWIZZLE_G })
        | texdesc_swizzle_b(if lum { GC_SWIZZLE_R } else { GC_SWIZZLE_B })
        | texdesc_swizzle_a(if fmt.no_alpha || t.no_alpha != 0 { GC_SWIZZLE_1 } else { GC_SWIZZLE_A })
        | if t.type_ == GLES_TEXTYPE_2D_ARRAY { GC_TEXDESC_TEXTURE_ARRAY } else { 0 }
        | texdesc_halign(0);
    desc[GC_TEXDESC_CONFIG2 as usize / 4] = GC_TEXDESC_CONFIG2_BASE;
    /* A block format's rows are rows of blocks, not of texels: the GL core
     * lays ceil(w/4) of them side by side
     * (`YakoglBackend::compressed_blocks_in_rows`), so the pitch is that many
     * block bytes. Taking it from stride_texels instead made it twice too
     * wide, and the sampler read the next row of blocks halfway through this
     * one. */
    desc[GC_TEXDESC_LINEAR_STRIDE as usize / 4] = if fmt.compressed {
        t.width.div_ceil(4) * block_bytes(t.format)
    } else if copy.is_some() {
        /* the tiled copy's level-0 row of texels, 4-aligned (Mesa's
         * levels[0].stride for a tiled layout) */
        t.width.next_multiple_of(4) * gles_texfmt_bytes(t.format)
    } else {
        t.stride_texels * gles_texfmt_bytes(t.format)
    };
    desc[GC_TEXDESC_SIZE as usize / 4] = texdesc_size_width(t.width) | texdesc_size_height(t.height);
    desc[GC_TEXDESC_LOG_SIZE_EXT as usize / 4] =
        texdesc_log_width(log2_fixp88(t.width)) | texdesc_log_height(log2_fixp88(t.height));
    desc[GC_TEXDESC_BASELOD as usize / 4] = texdesc_baselod_base(0) | texdesc_baselod_max(levels as u32 - 1);
    desc[GC_TEXDESC_VOLUME as usize / 4] = log2_fixp88(depth);
    /* The step from one layer to the next is the whole image, and a block
     * format's rows are block rows: a 16x8 ETC1 texture is four blocks by two,
     * not by eight. Mesa writes SLICE 0x40 for that texture where height gave
     * 0x100, four times too far. */
    desc[GC_TEXDESC_SLICE as usize / 4] = match copy {
        /* the copy's level-0 layer, tile-padded */
        Some((_, _, slice)) => slice,
        None => {
            desc[GC_TEXDESC_LINEAR_STRIDE as usize / 4] * if fmt.compressed { t.height.div_ceil(4) } else { t.height }
        }
    };
    desc[GC_TEXDESC_3D_CONFIG as usize / 4] = texdesc_3d_depth(depth);

    /* The mip chain, level 0 first.
     *
     * Two things about the layout, both of which the GL core keeps for this
     * backend (`YakoglBackend::mip_level_align`). There is one LINEAR_STRIDE
     * for all sixteen levels, so every level's rows are that far apart and a
     * level is stride * its own height -- working the size out from the
     * level's own width instead put level 1 a row short and sent the sampler
     * into level 2. And a level is fetched from a 64-byte boundary, so each
     * one starts on a multiple of that; at +0x60, level 2 of a 4x4 chain was
     * read as the level 1 that sits at +0x40.
     *
     * And a level holds every layer: a cube's level i is its six faces at
     * (w >> i) x (h >> i), then level i + 1 (gl/texture.rs chain_layout;
     * Mesa's setup_miptree, etnaviv_resource.c, sizes a level as
     * array_size * layer_stride the same way). Stepping one face per level
     * put a mipmapped cube's level 1 inside level 0's second face: dEQP's
     * fragment texturecube (nine levels) failed on the i.MX8MP while the
     * vertex one (one level) passed (2026-09-24).
     */
    if let Some((copy_gpu, off, _)) = copy {
        for (lod, o) in off.iter().enumerate().take(levels.min(16)) {
            desc[texdesc_lod_addr(lod as u32) as usize / 4] = (copy_gpu + *o as u64) as u32;
        }
        return gpu;
    }
    let layers = t.layers.max(1) as u32;
    let (mut level_w, mut level_h) = (t.width, t.height);
    let mut level_gpu = t.base_gpu;
    for lod in 0..levels.min(16) {
        level_gpu = (level_gpu + GC_TEXDESC_LOD_ALIGN as u64 - 1) & !(GC_TEXDESC_LOD_ALIGN as u64 - 1);
        desc[texdesc_lod_addr(lod as u32) as usize / 4] = level_gpu as u32;
        let bytes = if fmt.compressed {
            if lod == 0 {
                t.bytes
            } else {
                level_w.div_ceil(4) * level_h.div_ceil(4) * block_bytes(t.format) * layers
            }
        } else {
            t.stride_texels * level_h * gles_texfmt_bytes(t.format) * layers
        };
        level_gpu += bytes as u64;
        level_w = if level_w > 1 { level_w / 2 } else { 1 };
        level_h = if level_h > 1 { level_h / 2 } else { 1 };
    }
    gpu
}

/// Sampler state and the descriptor address for one hardware slot. Takes the
/// whole frame rather than its command writer: the descriptor is built in the
/// frame's arena first, and the two borrows are one after the other.
/// Does `t`'s sampler take INT_FILTER, under `gc_debug_int_filter`? Mesa's
/// rule (etnaviv_format.c texture_use_int_filter, the descriptor path): a
/// UNORM, non-sRGB format that is not ASTC, R10G10B10A2 or ETC2 R11/RG11,
/// on anything but a 3D texture, with no anisotropy. Of ours that is every
/// 8-bit and 5/6-bit format and the ETC ones; the float ones are not UNORM.
fn int_filter_wanted(t: &GlesTextureBind) -> bool {
    super::backend::int_filter_on()
        && t.type_ != GLES_TEXTYPE_3D
        && !matches!(t.format, GLES_TEXFMT_RGBA16F | GLES_TEXFMT_R16F | GLES_TEXFMT_R32F)
}

fn emit_slot(f: &mut GcFrame, slot: u32, t: &GlesTextureBind) -> i32 {
    /* GL ES 2.0 section 3.7.10: a texture that is not complete samples
     * (0,0,0,1) everywhere, whatever its texels hold. The GL core decides
     * which those are; here the binding becomes the backend's own 1x1 opaque
     * black texel instead. */
    let black;
    let t = if t.incomplete != 0 {
        black = GlesTextureBind {
            unit: t.unit,
            format: crate::backend::GLES_TEXFMT_RGBA8,
            base_gpu: super::backend::black_texel_gpu(),
            width: 1,
            height: 1,
            stride_texels: 1,
            bytes: 4,
            mip_levels: 1,
            wrap_s: GLES_WRAP_CLAMP_TO_EDGE,
            wrap_t: GLES_WRAP_CLAMP_TO_EDGE,
            ..GlesTextureBind::ZERO
        };
        &black
    } else {
        t
    };
    let desc_gpu = build_descriptor(f, t);
    let c = &mut f.w;
    /* S and T wrap independently in GL, and this used to send the S mode on
     * both axes: a texture with S clamped and T repeating, which is how
     * gl_test's conic gradient and FemtoVG's stroke dash sample their ramps,
     * came out clamped in T as well.
     *
     * A dimension one texel wide has one texel, so every wrap mode maps every
     * coordinate on that axis to it and clamp is exactly equivalent. That is
     * true of any hardware, but it is not idle here: gl_test case 24 samples
     * a 1x1 texture and never sets a wrap mode, so it takes GL's default of
     * repeat, and it is the one textured case on this part that came back
     * black while every passing one asked for clamp. */
    /* GL's three wrap modes; the GC7000's TEX_WRAP happens to number them the
     * same way. A one-texel axis always clamps: repeating or mirroring it
     * would sample outside the image. */
    let gc_wrap = |w: u8, one_texel: bool| -> u32 {
        if one_texel {
            return GC_TEX_WRAP_CLAMP_TO_EDGE;
        }
        match w {
            GLES_WRAP_CLAMP_TO_EDGE => GC_TEX_WRAP_CLAMP_TO_EDGE,
            GLES_WRAP_MIRRORED_REPEAT => GC_TEX_WRAP_MIRROR_REPEAT,
            _ => GC_TEX_WRAP_REPEAT,
        }
    };
    let uwrap = gc_wrap(t.wrap_s, t.width <= 1);
    let vwrap = gc_wrap(t.wrap_t, t.height <= 1);
    let mip = match t.mip_filter {
        2 => GC_TEX_FILTER_LINEAR,
        1 => GC_TEX_FILTER_NEAREST,
        _ => GC_TEX_FILTER_NONE,
    };

    if desc_gpu == 0 {
        /* The arena is full: `build_descriptor` allocates there and returns 0
         * for nothing else. Emitting the draw with the slot unprogrammed is
         * what this did, and it renders black with no GL error. No run has
         * ever been seen to reach this path -- it is here so that if one ever
         * does, it says so instead of quietly drawing the wrong picture. */
        crate::glog!(
            "vivante: no room in the frame arena for unit {slot}'s {GC_TEXDESC_BYTES}-byte texture descriptor"
        );
        return -ENOMEM;
    }
    c.state(nte_descriptor_tx_ctrl(slot), 0); /* no tile status */
    c.state(
        nte_descriptor_samp_ctrl0(slot),
        samp_uwrap(uwrap)
            | samp_vwrap(vwrap)
            | samp_wwrap(uwrap)
            | samp_min(if t.min_linear != 0 { GC_TEX_FILTER_LINEAR } else { GC_TEX_FILTER_NEAREST })
            | samp_mip(mip)
            | samp_mag(if t.linear_filter != 0 { GC_TEX_FILTER_LINEAR } else { GC_TEX_FILTER_NEAREST })
            | GC_SAMP_CTRL0_UNK21
            | if int_filter_wanted(t) { GC_SAMP_INT_FILTER } else { 0 },
    );
    c.state(nte_descriptor_samp_ctrl1(slot), GC_SAMP_CTRL1_UNK1);
    /*
     * The sampler's own LOD range, which is GL_TEXTURE_MIN_LOD and
     * GL_TEXTURE_MAX_LOD -- and ES 2.0 has no way to set either, so it is
     * always 0 and the 1000 GL starts at, saturated to the field's 0xfff.
     *
     * This used to hold the chain length instead, (levels - 1) << 8, on the
     * reasoning that the LOD should not run past the last level. The last
     * level is already named by the descriptor's BASELOD, so that was a
     * second clamp on the same thing -- and it sat exactly on the LOD a
     * minified draw of the last level asks for, which is why level 1 could be
     * fetched and level 2 could not. Mesa writes the sampler's range here and
     * nothing else.
     */
    c.state(nte_descriptor_samp_lod_minmax(slot), samp_lod_max(GC_SAMP_LOD_FP58_MAX) | samp_lod_min(0));
    c.state(nte_descriptor_samp_lod_bias(slot), 0);
    c.state(nte_descriptor_samp_anisotropy(slot), 0);
    write_desc_addr(c, slot, desc_gpu as u32);
    0
}

/// A draw's descriptor writes under [`super::backend::gc_debug_desc_seq`]:
/// how many NTE_DESCRIPTOR_ADDR writes the draw being emitted has made, the
/// slots whose INVALIDATE waits for [`emit_desc_seq`], and how many draws got
/// each part of the sequence since the process started, indexed bit 1, 2, 4,
/// 8 (the switch's counters), and whether the first draw that wrote a
/// descriptor has been logged.
struct DescSeqState {
    draw_addr_writes: u32,
    inval_pending: u32,
    seq_emits: [u32; 4],
    logged: bool,
}

static DESC_SEQ: Global<DescSeqState> =
    Global::new(DescSeqState { draw_addr_writes: 0, inval_pending: 0, seq_emits: [0; 4], logged: false });

/// How many draws have got each part of the
/// [`super::backend::gc_debug_desc_seq`] sequence since the process started,
/// indexed bit 1, 2, 4, 8.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn desc_seq_emits() -> [u32; 4] {
    // SAFETY: single-threaded by contract; read only.
    unsafe { DESC_SEQ.get() }.seq_emits
}

/// Point `slot` at the descriptor at `addr` and invalidate the slot's cached
/// copy. Under [`super::backend::GC_DESC_SEQ_MESA_ORDER`] (the default) the
/// INVALIDATE waits for [`emit_desc_seq`], which emits every slot's after all
/// of the draw's address writes, as Mesa does (etnaviv_texture_desc.c
/// 605-610); otherwise it follows the address at once.
fn write_desc_addr(c: &mut GcCmd, slot: u32, addr: u32) {
    c.state(nte_descriptor_addr(slot), addr);
    // SAFETY: single-threaded by contract; no caller holds DESC_SEQ.
    let st = unsafe { DESC_SEQ.get() };
    st.draw_addr_writes = st.draw_addr_writes.saturating_add(1);
    let bit = 1u32.checked_shl(slot).unwrap_or(0);
    if super::backend::desc_seq() & super::backend::GC_DESC_SEQ_MESA_ORDER != 0 && bit != 0 {
        st.inval_pending |= bit;
    } else {
        c.state(GC_NTE_DESCRIPTOR_INVALIDATE, GC_NTE_INVALIDATE_UNK29 | nte_invalidate_idx(slot));
    }
}

/// The end of a draw's descriptor writes under
/// [`super::backend::gc_debug_desc_seq`], in this order: the deferred
/// INVALIDATEs in slot order (bit 1), then NTE_DESCRIPTOR_FLUSH +
/// GL_FLUSH_CACHE DESCRIPTOR (bit 4), then GL_FLUSH_CACHE TEXTURE, TEXTUREVS
/// and an RA-PE stall (bit 2), then an FE-PE stall (bit 8), so the stall,
/// when asked for, waits for everything before it. Nothing when the draw
/// wrote no descriptor address. Each part counts itself in `seq_emits`.
fn emit_desc_seq(c: &mut GcCmd) {
    use super::backend::{GC_DESC_SEQ_DESC_FLUSH, GC_DESC_SEQ_FE_PE_STALL, GC_DESC_SEQ_TEX_FLUSH};
    // SAFETY: single-threaded by contract; no caller holds DESC_SEQ.
    let st = unsafe { DESC_SEQ.get() };
    let seq = super::backend::desc_seq();
    let pending = core::mem::take(&mut st.inval_pending);
    let wrote = core::mem::take(&mut st.draw_addr_writes);
    if pending != 0 {
        for slot in 0..u32::BITS {
            if pending & (1 << slot) != 0 {
                c.state(GC_NTE_DESCRIPTOR_INVALIDATE, GC_NTE_INVALIDATE_UNK29 | nte_invalidate_idx(slot));
            }
        }
        st.seq_emits[0] = st.seq_emits[0].saturating_add(1);
    }
    if wrote == 0 {
        return;
    }
    if seq & GC_DESC_SEQ_DESC_FLUSH != 0 {
        c.state(GC_NTE_DESCRIPTOR_FLUSH, 0);
        c.state(GC_GL_FLUSH_CACHE, GC_FLUSH_DESCRIPTOR);
        st.seq_emits[2] = st.seq_emits[2].saturating_add(1);
    }
    if seq & GC_DESC_SEQ_TEX_FLUSH != 0 {
        c.state(GC_GL_FLUSH_CACHE, GC_FLUSH_TEXTURE);
        c.state(GC_GL_FLUSH_CACHE, GC_FLUSH_TEXTUREVS);
        c.stall(GC_SYNC_RA, GC_SYNC_PE);
        st.seq_emits[1] = st.seq_emits[1].saturating_add(1);
    }
    if seq & GC_DESC_SEQ_FE_PE_STALL != 0 {
        c.stall(GC_SYNC_FE, GC_SYNC_PE);
        st.seq_emits[3] = st.seq_emits[3].saturating_add(1);
    }
    /* Once per process: the switch's counters after the first draw that
     * wrote a descriptor, so a run's log shows the sequence was exercised
     * (a counter at 0 there means the bit did nothing). */
    if !st.logged {
        st.logged = true;
        let [mesa_order, tex_flush, desc_flush, fe_pe_stall] = st.seq_emits;
        crate::glog!(
            "yakogl/vivante: desc-seq 0x{:x} first textured draw: {} addresses, emits mesa-order {} tex-flush {} desc-flush {} fe-pe-stall {}",
            seq,
            wrote,
            mesa_order,
            tex_flush,
            desc_flush,
            fe_pe_stall
        );
    }
}

/// `-ENOMEM` when a descriptor did not fit; the draw is then refused rather
/// than emitted without its texture.
pub fn emit_textures(f: &mut GcFrame, d: &GcDraw) -> i32 {
    {
        // SAFETY: single-threaded by contract; nothing else holds DESC_SEQ.
        let st = unsafe { DESC_SEQ.get() };
        st.draw_addr_writes = 0;
        st.inval_pending = 0;
    }
    let rc = emit_textures_inner(f, d);
    if rc == 0 {
        emit_desc_seq(&mut f.w);
    }
    rc
}

fn emit_textures_inner(f: &mut GcFrame, d: &GcDraw) -> i32 {
    /* The vertex stage's slots start after every fragment slot: the ones the
     * draw binds and the ones the shader declares with nothing bound, which
     * get the black texel below -- counting only the bound ones put vertex
     * sampler 0 in the slot a declared-but-unbound fragment sampler takes. */
    let black_unbound = super::backend::black_unbound_on();
    let fs_slots = if black_unbound {
        d.textures
            .iter()
            .map(|t| t.unit as u32 + 1)
            .chain(declared_samplers(d.fs).map(|b| b as u32 + 1))
            .max()
            .unwrap_or(0)
            .max(d.textures.len() as u32)
    } else {
        d.textures.len() as u32
    };
    let vs_base = fs_slots;

    /* the fragment stage owns slots 0.., the vertex stage the ones after */
    f.w.state(GC_VS_SAMPLER_BASE, vs_base);
    for t in d.textures.iter().take(GC_MAX_SAMPLERS) {
        if emit_slot(f, t.unit as u32, t) != 0 {
            return -ENOMEM;
        }
    }
    for t in d.vs_textures.iter().take(GC_MAX_SAMPLERS) {
        if emit_slot(f, vs_base + t.unit as u32, t) != 0 {
            return -ENOMEM;
        }
    }
    /* A sampler the shader declares with nothing usable bound gets no binding
     * from the GL core, which leaves it to the backend
     * (`sampler_without_a_usable_texture_is_left_to_the_backend`): GL ES 2.0
     * 3.7.10 has it sample (0,0,0,1), the PowerVR's descriptor block does
     * that for every declared sampler, and here the slot gets the black
     * texel. Before, nothing was written and the slot kept the previous
     * draw's descriptor: gl_test 61's "nothing bound" sampled the texture
     * bound before it (0xff0000ff where black was wanted, i.MX8MP). */
    if !black_unbound {
        return 0;
    }
    let black = GlesTextureBind { incomplete: 1, ..GlesTextureBind::ZERO };
    for (sh, bound, base) in [(d.fs, d.textures, 0u32), (d.vs, d.vs_textures, vs_base)] {
        for binding in declared_samplers(sh) {
            if bound.iter().any(|t| t.unit == binding) || binding as usize >= GC_MAX_SAMPLERS {
                continue;
            }
            if emit_slot(f, base + binding as u32, &GlesTextureBind { unit: binding, ..black }) != 0 {
                return -ENOMEM;
            }
        }
    }
    0
}

/// The sampler bindings a shader declares: one per sampler uniform, and one
/// per element of an array of samplers (element k reads binding `binding + k`,
/// as the GL core binds them).
fn declared_samplers(sh: &ShcShader) -> impl Iterator<Item = u8> + '_ {
    sh.uniform_table()
        .iter()
        .filter(|u| u.dword_off == SHC_UNIFORM_NO_BLOCK)
        .flat_map(|u| (0..u.array_len.max(1)).map(move |k| u.binding.saturating_add(k)))
}
