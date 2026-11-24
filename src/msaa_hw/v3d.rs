// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The V3D 4.2 (Pi 4, `src/v3d`, `driver/src/v3d`) values of a 4x
//! multisample framebuffer object: the tile size, the tile buffer's internal
//! type and bpp per render target, the per-RT sample buffer's layout and
//! bytes, and the load/store words of a resolved, a per-sample and a
//! sample-0 store (docs/notes/es3-msaa-renderbuffers.md (c2), (f1), (f5);
//! docs/notes/v3d-msaa-es3.md (3)). Not wired: `v3d-msaa-fbo` calls them
//! from `src/v3d`.
//!
//! Every rule is the one `driver/src/v3d/v3d_job.c` (after `v3d-msaa-job`),
//! `v3d_frame.c` and `v3d_tex.c` already apply, cited at each item; the enum
//! numbers are Mesa 26.2.1's `src/broadcom/cle/v3d_packet.xml`
//! (third_party/mesa-26.2.1, read for the values only). The tests compare
//! the results with the cases `tools/v3d/job_check.c` (part_msaa) and
//! `tools/v3d/tex_check.c` (the 4x layout) pin for the C code.
//!
//! What `v3d-msaa-fbo` does with these ((c2), (f1)):
//!
//! 1. Refuse a 4x job whose RT formats are not all [`rt_internal`] rows of a
//!    TLB-resolvable internal type ([`accepts_4x`]), as `v3d_job_begin`.
//! 2. Size each RT's sample buffer with [`ms_layout`] in the RT's own output
//!    format ([`output_cpp`]), not the literal 4 `src/v3d/backend.rs`
//!    `ms_buf` passes today, and key it in `msaa_store` with
//!    [`sample_size`].
//! 3. Per RT, store [`rt_stores`]: the kept samples (`ALL_SAMPLES`) first
//!    when the RT has a sample buffer, then the resolve (`4X`) into its
//!    raster buffer; load the kept samples with [`rt_sample_load`].
//! 4. Depth/stencil: the 4x buffer stored and loaded with [`zs_decimate`]
//!    (`ALL_SAMPLES`), plus the "keep sample 0" store [`zs_sample0_store`]
//!    into a 1x D24S8 buffer for `v3d_depth_resolve` to read (note (f1)
//!    item (3); Mesa's `v3d_tlb_blit` resolves Z/S with `SAMPLE_0`).

use crate::msaa_store::SampleSize;

/// Samples of a 4x framebuffer object.
pub const SAMPLES_4X: u32 = 4;

/// Render targets a V3D 4.2 job configures at most (`v3d_job_begin`
/// refuses `rts > 4`, `driver/src/v3d/v3d_job.c:332`; `Buffer to Store`
/// has render targets 0..3 below 7.1, v3d_packet.xml:513-516).
pub const MAX_RTS: u32 = 4;

/* ---- enums (v3d_packet.xml) ------------------------------------------- */

/// `<enum name="Decimate Mode">` (v3d_packet.xml:117-121).
pub const DECIMATE_SAMPLE_0: u32 = 0;
pub const DECIMATE_4X: u32 = 1;
pub const DECIMATE_ALL_SAMPLES: u32 = 3;

/// `<enum name="Memory Format">` (v3d_packet.xml:108-115).
pub const MEMORY_FORMAT_RASTER: u32 = 0;
pub const MEMORY_FORMAT_LINEARTILE: u32 = 1;
pub const MEMORY_FORMAT_UB_LINEAR_1: u32 = 2;
pub const MEMORY_FORMAT_UB_LINEAR_2: u32 = 3;
pub const MEMORY_FORMAT_UIF_NO_XOR: u32 = 4;
pub const MEMORY_FORMAT_UIF_XOR: u32 = 5;

/// `<enum name="Internal Type">` (v3d_packet.xml:123-133; also
/// `src/v3d/ffi.rs:306-314`).
pub const INTERNAL_TYPE_8I: u32 = 0;
pub const INTERNAL_TYPE_8UI: u32 = 1;
pub const INTERNAL_TYPE_8: u32 = 2;
pub const INTERNAL_TYPE_16I: u32 = 4;
pub const INTERNAL_TYPE_16UI: u32 = 5;
pub const INTERNAL_TYPE_16F: u32 = 6;
pub const INTERNAL_TYPE_32I: u32 = 8;
pub const INTERNAL_TYPE_32UI: u32 = 9;
pub const INTERNAL_TYPE_32F: u32 = 10;

/// `<enum name="Internal BPP">` (v3d_packet.xml:135-139).
pub const INTERNAL_BPP_32: u32 = 0;
pub const INTERNAL_BPP_64: u32 = 1;
pub const INTERNAL_BPP_128: u32 = 2;

/// `<enum name="Render Target Clamp">`, V3D 4.2 (v3d_packet.xml:147-152).
pub const CLAMP_NONE: u32 = 0;
pub const CLAMP_NORM: u32 = 1;
pub const CLAMP_INT: u32 = 3;

/// `Buffer to Store` / `Buffer to Load` (v3d_packet.xml:512-525, :543-552):
/// render target n is n.
pub const BUFFER_RT0: u32 = 0;
pub const BUFFER_NONE: u32 = 8;
pub const BUFFER_Z: u32 = 9;
pub const BUFFER_STENCIL: u32 = 10;
pub const BUFFER_ZSTENCIL: u32 = 11;

/// `<enum name="Output Image Format">` (v3d_packet.xml:222-270): the rows
/// `rt_internal` takes, and the depth/stencil ones.
pub mod fmt {
    pub const SRGB8_ALPHA8: u32 = 0;
    pub const RGB10_A2UI: u32 = 2;
    pub const RGB10_A2: u32 = 3;
    pub const BGR565: u32 = 7;
    pub const R11F_G11F_B10F: u32 = 8;
    pub const RGBA32F: u32 = 9;
    pub const RG32F: u32 = 10;
    pub const R32F: u32 = 11;
    pub const RGBA32I: u32 = 12;
    pub const RG32I: u32 = 13;
    pub const R32I: u32 = 14;
    pub const RGBA32UI: u32 = 15;
    pub const RG32UI: u32 = 16;
    pub const R32UI: u32 = 17;
    pub const RGBA16F: u32 = 18;
    pub const RG16F: u32 = 19;
    pub const R16F: u32 = 20;
    pub const RGBA16I: u32 = 21;
    pub const RG16I: u32 = 22;
    pub const R16I: u32 = 23;
    pub const RGBA16UI: u32 = 24;
    pub const RG16UI: u32 = 25;
    pub const R16UI: u32 = 26;
    pub const RGBA8: u32 = 27;
    pub const RGBA8I: u32 = 31;
    pub const RG8I: u32 = 32;
    pub const R8I: u32 = 33;
    pub const RGBA8UI: u32 = 34;
    pub const RG8UI: u32 = 35;
    pub const R8UI: u32 = 36;
    pub const D24S8: u32 = 43;
    pub const S8: u32 = 44;
}

/* ---- tile buffer ------------------------------------------------------ */

/// One render target's tile buffer configuration.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct RtInternal {
    /// `INTERNAL_TYPE_*`.
    pub ty: u32,
    /// `INTERNAL_BPP_*`.
    pub bpp: u32,
    /// `CLAMP_*`.
    pub clamp: u32,
}

/// The tile buffer's internal type, bpp and clamp of an output image format:
/// `rt_internal` of `driver/src/v3d/v3d_job.c:151-265` row for row (Mesa's
/// `v3dX(get_internal_type_bpp_for_output_format)`, v3dx_format_table.c:
/// 220-341, and `clamp_for_format_and_type`, v3dx_rcl.c:396-407: NORM for
/// sRGB, INT for an integer format). `None` for a format the table has no
/// row for (RG16F, the depth formats), which breaks a typed job.
pub const fn rt_internal(f: u32) -> Option<RtInternal> {
    const fn r(ty: u32, bpp: u32, clamp: u32) -> Option<RtInternal> {
        Some(RtInternal { ty, bpp, clamp })
    }
    match f {
        // the tile buffer holds linear halves (v3d_job.c:154-161)
        fmt::SRGB8_ALPHA8 => r(INTERNAL_TYPE_16F, INTERNAL_BPP_64, CLAMP_NORM),
        // 16F like sRGB, no clamp (v3d_job.c:162-170)
        fmt::RGB10_A2 => r(INTERNAL_TYPE_16F, INTERNAL_BPP_64, CLAMP_NONE),
        fmt::RGBA8 | fmt::BGR565 => r(INTERNAL_TYPE_8, INTERNAL_BPP_32, CLAMP_NONE),
        fmt::RGBA8I | fmt::RG8I | fmt::R8I => r(INTERNAL_TYPE_8I, INTERNAL_BPP_32, CLAMP_INT),
        fmt::RGBA8UI | fmt::RG8UI | fmt::R8UI => r(INTERNAL_TYPE_8UI, INTERNAL_BPP_32, CLAMP_INT),
        // R16F at 64 bpp too (v3d_job.c:189-197)
        fmt::RGBA16F | fmt::R16F | fmt::R11F_G11F_B10F => r(INTERNAL_TYPE_16F, INTERNAL_BPP_64, CLAMP_NONE),
        fmt::RGBA16I => r(INTERNAL_TYPE_16I, INTERNAL_BPP_64, CLAMP_INT),
        fmt::RG16I | fmt::R16I => r(INTERNAL_TYPE_16I, INTERNAL_BPP_32, CLAMP_INT),
        fmt::RGB10_A2UI | fmt::RGBA16UI => r(INTERNAL_TYPE_16UI, INTERNAL_BPP_64, CLAMP_INT),
        fmt::RG16UI | fmt::R16UI => r(INTERNAL_TYPE_16UI, INTERNAL_BPP_32, CLAMP_INT),
        fmt::RGBA32I => r(INTERNAL_TYPE_32I, INTERNAL_BPP_128, CLAMP_INT),
        fmt::RG32I => r(INTERNAL_TYPE_32I, INTERNAL_BPP_64, CLAMP_INT),
        fmt::R32I => r(INTERNAL_TYPE_32I, INTERNAL_BPP_32, CLAMP_INT),
        fmt::RGBA32UI => r(INTERNAL_TYPE_32UI, INTERNAL_BPP_128, CLAMP_INT),
        fmt::RG32UI => r(INTERNAL_TYPE_32UI, INTERNAL_BPP_64, CLAMP_INT),
        fmt::R32UI => r(INTERNAL_TYPE_32UI, INTERNAL_BPP_32, CLAMP_INT),
        fmt::RGBA32F => r(INTERNAL_TYPE_32F, INTERNAL_BPP_128, CLAMP_NONE),
        fmt::RG32F => r(INTERNAL_TYPE_32F, INTERNAL_BPP_64, CLAMP_NONE),
        fmt::R32F => r(INTERNAL_TYPE_32F, INTERNAL_BPP_32, CLAMP_NONE),
        _ => None,
    }
}

/// The TLB resolves (averages) an internal type: `8` or `16F` only (Mesa's
/// `v3d_format_supports_tlb_resolve_and_blend`, v3d_formats.c:148-163; the
/// 4x check of `v3d_job_begin`, `driver/src/v3d/v3d_job.c:350-361`).
pub const fn tlb_resolvable(ty: u32) -> bool {
    ty == INTERNAL_TYPE_8 || ty == INTERNAL_TYPE_16F
}

/// A typed 4x job over these RT formats builds: 1..=4 RTs, each an
/// [`rt_internal`] row of a [`tlb_resolvable`] type (`v3d_job_begin`,
/// `driver/src/v3d/v3d_job.c:332`, :350-361). R11F_G11F_B10F is a 16F row,
/// so the job builder takes it at 4x although ES 3.0 has no multisample
/// R11F_G11F_B10F without EXT_color_buffer_float.
pub fn accepts_4x(formats: &[u32]) -> bool {
    !formats.is_empty()
        && formats.len() <= MAX_RTS as usize
        && formats.iter().all(|&f| matches!(rt_internal(f), Some(i) if tlb_resolvable(i.ty)))
}

/// The largest internal bpp of the RTs, 32 (0) for none or an unknown
/// format: `job_max_bpp` (`driver/src/v3d/v3d_job.c:313-323`), which
/// TILE_BINNING_MODE_CFG and the rendering mode's common config carry.
pub fn max_bpp(formats: &[u32]) -> u32 {
    formats.iter().filter_map(|&f| rt_internal(f)).map(|i| i.bpp).fold(INTERNAL_BPP_32, u32::max)
}

/// The tile-size table of V3D 4.x, from 64x64 down
/// (`v3d_frame_tiling_gen`, `driver/src/v3d/v3d_frame.c:78-79`; Mesa's
/// v3d_choose_tile_size, v3d_util.c:122-200).
pub const TILE_SIZES: [(u32, u32); 7] = [(64, 64), (64, 32), (32, 32), (32, 16), (16, 16), (16, 8), (8, 8)];

/// The tile width and height of a V3D 4.2 job: index 0 (64x64), one step
/// down for 2 RTs and two for 3-4, two more at 4x, and one per bpp step of
/// the largest RT (`v3d_frame_tiling_gen`'s 4.x branch,
/// `driver/src/v3d/v3d_frame.c:87`: `(rts > 2 ? 2 : rts > 1 ? 1 : 0) +
/// (msaa ? 2 : 0) + (max_bpp > 2 ? 2 : max_bpp)`). The hardware takes the
/// size from the packet's RT count, bpp and 4x bit, so this is not a choice.
pub const fn tile_size(rts: u32, msaa: bool, max_bpp: u32) -> (u32, u32) {
    let idx = (if rts > 2 {
        2
    } else if rts > 1 {
        1
    } else {
        0
    }) + (if msaa { 2 } else { 0 })
        + (if max_bpp > 2 { 2 } else { max_bpp });
    TILE_SIZES[idx as usize]
}

/// Tiles across and down a `width` x `height` frame of those tiles
/// (`tiles_x`, `tiles_y` of `v3d_frame_tiling_gen`, v3d_frame.c:90-91).
pub const fn tiles(width: u32, height: u32, rts: u32, msaa: bool, max_bpp: u32) -> (u32, u32) {
    let (tw, th) = tile_size(rts, msaa, max_bpp);
    (width.div_ceil(tw), height.div_ceil(th))
}

/* ---- sample buffer ---------------------------------------------------- */

/// Bytes a texel of an output image format takes in memory, the `cpp` of
/// the RT's sample buffer: Mesa lays a multisample resource out in its own
/// format's block size (`rsc->cpp`, v3d_resource.c:797) and loads/stores it
/// in `v3d_get_rt_format` of it (v3dx_rcl.c:60-170; note (c2) "the
/// format's own cpp ... not the literal 4"). `None` for a format with no
/// [`rt_internal`] row.
pub const fn output_cpp(f: u32) -> Option<u32> {
    match f {
        fmt::BGR565 | fmt::R16F | fmt::R16I | fmt::R16UI => Some(2),
        fmt::R8I | fmt::R8UI => Some(1),
        fmt::RG8I | fmt::RG8UI => Some(2),
        fmt::RGBA16F | fmt::RGBA16I | fmt::RGBA16UI | fmt::RG32F | fmt::RG32I | fmt::RG32UI => Some(8),
        fmt::RGBA32F | fmt::RGBA32I | fmt::RGBA32UI => Some(16),
        _ => match rt_internal(f) {
            Some(_) => Some(4),
            None => None,
        },
    }
}

/// `v3d_utile_width` (Mesa `src/broadcom/common/v3d_tiling.c:38-52`).
pub const fn utile_width(cpp: u32) -> u32 {
    match cpp {
        1 | 2 => 8,
        16 => 2,
        _ => 4,
    }
}

/// `v3d_utile_height` (v3d_tiling.c:56-70).
pub const fn utile_height(cpp: u32) -> u32 {
    match cpp {
        1 => 8,
        8 | 16 => 2,
        _ => 4,
    }
}

/// UIF rows of a page and of the page cache (`v3d_tex.c:43-55`, Mesa's
/// v3d_screen.h and v3d_resource.c: 4096-byte pages, 8 banks, 1024-byte UIF
/// block rows).
const PAGE_UB_ROWS: u32 = 4096 / 1024;
const PAGE_UB_ROWS_TIMES_1_5: u32 = (PAGE_UB_ROWS * 3) >> 1;
const PAGE_CACHE_UB_ROWS: u32 = 4096 * 8 / 1024;
const PAGE_CACHE_MINUS_1_5_UB_ROWS: u32 = PAGE_CACHE_UB_ROWS - PAGE_UB_ROWS_TIMES_1_5;

/// `get_ub_pad` (`driver/src/v3d/v3d_tex.c:87-114`, Mesa's v3d_get_ub_pad).
const fn ub_pad(cpp: u32, height: u32) -> u32 {
    let height_ub = height / (utile_height(cpp) * 2);
    let off = height_ub % PAGE_CACHE_UB_ROWS;
    if off == 0 {
        0
    } else if off < PAGE_UB_ROWS_TIMES_1_5 {
        if height_ub < PAGE_CACHE_UB_ROWS {
            0
        } else {
            PAGE_UB_ROWS_TIMES_1_5 - off
        }
    } else if off > PAGE_CACHE_MINUS_1_5_UB_ROWS {
        PAGE_CACHE_UB_ROWS - off
    } else {
        0
    }
}

/// One RT's 4x sample buffer as the job builder loads and stores it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct MsLayout {
    /// Bytes per texel.
    pub cpp: u32,
    /// Bytes per row of the doubled image (the UIF-aligned width x cpp).
    pub stride: u32,
    /// Rows, padded to UIF blocks and the page-cache pad.
    pub padded_height: u32,
    /// `MEMORY_FORMAT_UIF_NO_XOR`: Mesa's choice may be UIF XOR, which the
    /// buffer drops (`zs_layout`, `src/v3d/backend.rs:1622-1630`; the
    /// `uif_address` comment of `driver/src/v3d/v3d_job.h:102-116`).
    pub memory_format: u32,
    /// Mesa's choice before the XOR is dropped (`UIF_XOR` or `UIF_NO_XOR`).
    pub mesa_tiling: u32,
    /// `Height in UB or Stride` of the load and store: the padded height in
    /// UIF blocks (padded_height / (2 x utile height)).
    pub height_in_ub: u32,
    /// Bytes of the buffer (`v3d_tex_layout.size`); the back end allocates
    /// 4096 more to align its start (`ms_buf`, `src/v3d/backend.rs:1858`).
    pub size: u32,
}

/// The sample buffer of a `width` x `height` RT of `cpp` bytes a texel at
/// 4x: one level of twice the width and height, UIF at the top
/// (`v3d_tex_layout_msaa`, `driver/src/v3d/v3d_tex.c:186-197`, Mesa's
/// v3d_resource.c:612-642), with `layout`'s level-0 UIF branch
/// (v3d_tex.c:260-276: width to a 4-UIF-block column, height to a UIF block
/// plus the pad). `None` where `v3d_tex_layout_msaa` refuses: a side of 0
/// or of 8192 and more, or a cpp other than 1, 2, 4, 8, 16.
pub const fn ms_layout(width: u32, height: u32, cpp: u32) -> Option<MsLayout> {
    if width == 0 || height == 0 || width >= 1 << 13 || height >= 1 << 13 {
        return None;
    }
    if !matches!(cpp, 1 | 2 | 4 | 8 | 16) {
        return None;
    }
    let (w, h) = (2 * width, 2 * height);
    let uif_block_w = utile_width(cpp) * 2;
    let uif_block_h = utile_height(cpp) * 2;
    let level_width = w.next_multiple_of(4 * uif_block_w);
    let mut level_height = h.next_multiple_of(uif_block_h);
    level_height += ub_pad(cpp, level_height) * uif_block_h;
    let mesa_tiling = if (level_height / uif_block_h).is_multiple_of(PAGE_CACHE_UB_ROWS) {
        MEMORY_FORMAT_UIF_XOR
    } else {
        MEMORY_FORMAT_UIF_NO_XOR
    };
    let stride = level_width * cpp;
    // one level at offset 0: the 4 KiB page alignment adds nothing
    // (v3d_tex.c:306-313)
    let size = match stride.checked_mul(level_height) {
        Some(s) => s,
        None => return None,
    };
    Some(MsLayout {
        cpp,
        stride,
        padded_height: level_height,
        memory_format: MEMORY_FORMAT_UIF_NO_XOR,
        mesa_tiling,
        height_in_ub: level_height / uif_block_h,
        size,
    })
}

/// The sample buffer of a `width` x `height` RT in output format `f`
/// ([`ms_layout`] at [`output_cpp`]).
pub const fn ms_layout_for(width: u32, height: u32, f: u32) -> Option<MsLayout> {
    match output_cpp(f) {
        Some(cpp) => ms_layout(width, height, cpp),
        None => None,
    }
}

/// The `msaa_store` key size of an RT's samples: `w` x `h`, the format's
/// cpp, 4 samples. Its `bytes()` is the packed count; [`ms_layout`]'s
/// `size` is what the back end allocates (UIF padding).
pub const fn sample_size(w: u32, h: u32, cpp: u32) -> SampleSize {
    SampleSize { w, h, bytes_per_sample: cpp, samples: SAMPLES_4X }
}

/* ---- load and store words --------------------------------------------- */

/// The fields of one STORE_TILE_BUFFER_GENERAL or LOAD_TILE_BUFFER_GENERAL
/// this module decides (v3d_packet.xml:494-552); the address comes from the
/// back end, and swap/alpha forcing stay as `v3d_job.c` sets them (never on
/// samples).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct TileBufferWords {
    /// `Buffer to Store` / `Buffer to Load` (`BUFFER_*`, RT n is n).
    pub buffer: u32,
    /// `MEMORY_FORMAT_*`.
    pub memory_format: u32,
    /// `Output Image Format` / `Input Image Format` (`fmt::*`).
    pub image_format: u32,
    /// `Height in UB or Stride`: UIF blocks for UIF, bytes a row for raster.
    pub height_in_ub_or_stride: u32,
    /// `DECIMATE_*`.
    pub decimate: u32,
}

/// RT `n`'s per-sample store: every sample into its sample buffer, in its
/// own format (`store_rt`'s first store, `driver/src/v3d/v3d_job.c:1119-1129`;
/// Mesa's store_general of a multisample surface, v3dx_rcl.c:157-158).
pub const fn rt_sample_store(n: u32, f: u32, ms: &MsLayout) -> TileBufferWords {
    TileBufferWords {
        buffer: BUFFER_RT0 + n,
        memory_format: ms.memory_format,
        image_format: f,
        height_in_ub_or_stride: ms.height_in_ub,
        decimate: DECIMATE_ALL_SAMPLES,
    }
}

/// RT `n`'s store into its raster buffer of `stride` bytes a row: the 4X
/// resolve at 4x, sample 0 single-sampled (`store_rt`'s second store,
/// `driver/src/v3d/v3d_job.c:1132-1141`; v3dx_rcl.c:159-167).
pub const fn rt_raster_store(n: u32, f: u32, stride: u32, msaa: bool) -> TileBufferWords {
    TileBufferWords {
        buffer: BUFFER_RT0 + n,
        memory_format: MEMORY_FORMAT_RASTER,
        image_format: f,
        height_in_ub_or_stride: stride,
        decimate: if msaa { DECIMATE_4X } else { DECIMATE_SAMPLE_0 },
    }
}

/// RT `n`'s stores in tile-list order: at 4x with a sample buffer the kept
/// samples first, then the raster store (`store_rt`, v3d_job.c:1109-1142).
/// Without a buffer (or single-sampled) the raster store alone, in `[0]`.
pub const fn rt_stores(n: u32, f: u32, stride: u32, msaa: bool, ms: Option<&MsLayout>) -> [Option<TileBufferWords>; 2] {
    match ms {
        Some(ms) if msaa => [Some(rt_sample_store(n, f, ms)), Some(rt_raster_store(n, f, stride, true))],
        _ => [Some(rt_raster_store(n, f, stride, msaa)), None],
    }
}

/// RT `n`'s load of the samples an earlier job kept (`emit_tile_list`'s 4x
/// load, `driver/src/v3d/v3d_job.c:1195-1205`; Mesa's load_general of a
/// multisample surface, v3dx_rcl.c:100-110). A 4x job that loads an RT
/// without a buffer is refused (`v3d_job_end`, v3d_job.c:1328-1338).
pub const fn rt_sample_load(n: u32, f: u32, ms: &MsLayout) -> TileBufferWords {
    TileBufferWords {
        buffer: BUFFER_RT0 + n,
        memory_format: ms.memory_format,
        image_format: f,
        height_in_ub_or_stride: ms.height_in_ub,
        decimate: DECIMATE_ALL_SAMPLES,
    }
}

/// The decimate of the Z/S loads and stores of a job's own depth/stencil
/// buffers: `ALL_SAMPLES` at 4x (they are 4x buffers), `SAMPLE_0` at 1x
/// (`zs_decimate`, `driver/src/v3d/v3d_job.c:1220-1221`).
pub const fn zs_decimate(msaa: bool) -> u32 {
    if msaa {
        DECIMATE_ALL_SAMPLES
    } else {
        DECIMATE_SAMPLE_0
    }
}

/// The "keep sample 0" store of a 4x job's depth/stencil into a 1x D24S8
/// buffer of `height_in_ub` UIF blocks (`memory_format` its tiling), beside
/// the 4x store, for `v3d_depth_resolve` (`src/v3d/backend.rs`) to read: Z
/// alone when stencil has its own buffer (`split`), Z+S together otherwise,
/// with decimate `SAMPLE_0` (note (f1) item (3); Mesa's `v3d_tlb_blit` stores
/// Z/S `SAMPLE_0`, v3d_blit.c:512-620, store at :613, v3dx_rcl.c:165-167).
/// `v3d_job.c` does not emit it yet: `v3d-msaa-fbo` adds it.
pub const fn zs_sample0_store(split: bool, memory_format: u32, height_in_ub: u32) -> TileBufferWords {
    TileBufferWords {
        buffer: if split { BUFFER_Z } else { BUFFER_ZSTENCIL },
        memory_format,
        image_format: fmt::D24S8,
        height_in_ub_or_stride: height_in_ub,
        decimate: DECIMATE_SAMPLE_0,
    }
}

/// The same for the split stencil buffer (`S8` when the stencil buffer is
/// S8, D24S8 otherwise; v3d_job.c:1292-1303, v3dx_rcl.c:288-304).
pub const fn s_sample0_store(s8: bool, memory_format: u32, height_in_ub: u32) -> TileBufferWords {
    TileBufferWords {
        buffer: BUFFER_STENCIL,
        memory_format,
        image_format: if s8 { fmt::S8 } else { fmt::D24S8 },
        height_in_ub_or_stride: height_in_ub,
        decimate: DECIMATE_SAMPLE_0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// tools/v3d/job_check.c's target (`W`, `H`, :172-173).
    const W: u32 = 200;
    const H: u32 = 100;

    #[test]
    fn tile_size_matches_job_check_part_msaa() {
        // 1 RT RGBA8 at 4x: idx 2, 32x32, 7 x 4 tiles (job_check.c:1968-1971)
        let one = [fmt::RGBA8];
        assert_eq!(tile_size(1, true, max_bpp(&one)), (32, 32));
        assert_eq!(tiles(W, H, 1, true, max_bpp(&one)), (7, 4));
        // RGBA8 + SRGB8_ALPHA8 at 4x: 64 bpp, idx 4, 16x16, 13 x 7
        // (job_check.c:2008-2031)
        let two = [fmt::RGBA8, fmt::SRGB8_ALPHA8];
        assert_eq!(max_bpp(&two), INTERNAL_BPP_64);
        assert_eq!(tile_size(2, true, max_bpp(&two)), (16, 16));
        assert_eq!(tiles(W, H, 2, true, max_bpp(&two)), (13, 7));
        // 1x: 64x64, 4 tiles across (job_check.c:2085-2092)
        assert_eq!(tiles(W, H, 1, false, INTERNAL_BPP_32).0, 4);
    }

    #[test]
    fn tile_size_matches_the_note() {
        // note (c2) "Tile and memory": a 64-bpp RT at 4x 32x16; 4 RTs at 4x
        // with a 128-bpp one 8x8
        assert_eq!(tile_size(1, true, INTERNAL_BPP_64), (32, 16));
        assert_eq!(tile_size(4, true, INTERNAL_BPP_128), (8, 8));
        assert_eq!(tile_size(3, true, INTERNAL_BPP_32), (16, 16));
        assert_eq!(tile_size(1, false, INTERNAL_BPP_32), (64, 64));
        assert_eq!(tile_size(2, false, INTERNAL_BPP_32), (64, 32));
    }

    #[test]
    fn tile_size_table_is_v3d_frame_tiling_gen_42() {
        // the index formula of v3d_frame.c:87 for every input it can see
        for rts in 1..=4u32 {
            for msaa in [false, true] {
                for bpp in 0..=2u32 {
                    let idx = (if rts > 2 { 2 } else { rts - 1 }) + 2 * msaa as u32 + bpp;
                    assert_eq!(tile_size(rts, msaa, bpp), TILE_SIZES[idx as usize]);
                }
            }
        }
        // rts 0 counts as 1 RT (job_rts), the same row
        assert_eq!(tile_size(0, true, 0), tile_size(1, true, 0));
    }

    #[test]
    fn internal_rows_and_4x_acceptance_match_v3d_job_begin() {
        assert_eq!(
            rt_internal(fmt::RGBA8),
            Some(RtInternal { ty: INTERNAL_TYPE_8, bpp: INTERNAL_BPP_32, clamp: CLAMP_NONE })
        );
        assert_eq!(
            rt_internal(fmt::SRGB8_ALPHA8),
            Some(RtInternal { ty: INTERNAL_TYPE_16F, bpp: INTERNAL_BPP_64, clamp: CLAMP_NORM })
        );
        assert_eq!(
            rt_internal(fmt::RGB10_A2).map(|i| (i.ty, i.bpp, i.clamp)),
            Some((INTERNAL_TYPE_16F, INTERNAL_BPP_64, CLAMP_NONE))
        );
        assert_eq!(rt_internal(fmt::R16F).map(|i| i.bpp), Some(INTERNAL_BPP_64));
        assert_eq!(rt_internal(fmt::RGBA32F).map(|i| i.bpp), Some(INTERNAL_BPP_128));
        assert_eq!(rt_internal(fmt::RG16F), None);
        assert_eq!(rt_internal(fmt::D24S8), None);
        // job_check.c:2077-2080: 4x RGBA8 + RGBA8UI refused at begin;
        // RGBA8 + SRGB8_ALPHA8 builds (:2008-2011)
        assert!(accepts_4x(&[fmt::RGBA8, fmt::SRGB8_ALPHA8]));
        assert!(!accepts_4x(&[fmt::RGBA8, fmt::RGBA8UI]));
        assert!(accepts_4x(&[fmt::RGBA16F, fmt::RGB10_A2, fmt::R16F, fmt::BGR565]));
        assert!(!accepts_4x(&[fmt::RGBA32F]));
        assert!(!accepts_4x(&[fmt::RG16F]));
        assert!(!accepts_4x(&[]));
        assert!(!accepts_4x(&[fmt::RGBA8; 5]));
        // every integer row has CLAMP_INT and is not resolvable
        for f in [fmt::RGBA8I, fmt::R8UI, fmt::RGBA16I, fmt::RG16UI, fmt::RGB10_A2UI, fmt::RGBA32UI, fmt::R32I] {
            let i = rt_internal(f).unwrap();
            assert_eq!(i.clamp, CLAMP_INT);
            assert!(!tlb_resolvable(i.ty));
        }
    }

    #[test]
    fn ms_layout_matches_tex_check() {
        // tools/v3d/tex_check.c:195-206: 8x8 at 4x is 16x16 RGBA8, UIF no
        // XOR, stride 128, padded height 16, 2048 bytes
        let l = ms_layout(8, 8, 4).unwrap();
        assert_eq!((l.stride, l.padded_height, l.size), (128, 16, 2048));
        assert_eq!(l.mesa_tiling, MEMORY_FORMAT_UIF_NO_XOR);
        assert_eq!(l.height_in_ub, 2);
        // tex_check.c:208: a doubled width past 14 bits is refused
        assert_eq!(ms_layout(8192, 4, 4), None);
        assert_eq!(ms_layout(0, 4, 4), None);
        assert_eq!(ms_layout(4, 4, 3), None);
    }

    #[test]
    fn ms_layout_sizes() {
        // job_check.c's 200x100 target: 400x200 RGBA8, width to 32 (416),
        // 25 UIF rows, no pad, no XOR
        let l = ms_layout(W, H, 4).unwrap();
        assert_eq!((l.stride, l.padded_height, l.height_in_ub), (416 * 4, 200, 25));
        assert_eq!(l.size, 416 * 4 * 200);
        // note (c2): 16 MiB for a 1024x1024 RGBA8 RT; 256 UIF rows are a
        // page-cache multiple, so Mesa would pick XOR, which the buffer drops
        let l = ms_layout(1024, 1024, 4).unwrap();
        assert_eq!(l.size, 16 << 20);
        assert_eq!(l.mesa_tiling, MEMORY_FORMAT_UIF_XOR);
        assert_eq!(l.memory_format, MEMORY_FORMAT_UIF_NO_XOR);
        // a 64-bpp RT (RGBA16F, cpp 8) at 1024x1024: 32 MiB, four of them
        // 128 MiB (note (c2)); its UIF block is 8x4 texels
        let l = ms_layout_for(1024, 1024, fmt::RGBA16F).unwrap();
        assert_eq!(l.size, 32 << 20);
        assert_eq!(l.height_in_ub, 2048 / 4);
        // the page-cache pad: 64x40 RGBA8 at 4x is 128x80, 10 UIF rows,
        // under the cache: no pad; 1024x60 is 2048x120, 15 rows: no pad;
        // 1024x264 is 2048x528, 66 rows, 66 % 32 = 2 < 6: pad to 6, 70 rows
        assert_eq!(ms_layout(64, 40, 4).unwrap().height_in_ub, 10);
        let l = ms_layout(1024, 264, 4).unwrap();
        assert_eq!(l.height_in_ub, 70);
        assert_eq!(l.padded_height, 560);
    }

    #[test]
    fn output_cpp_rows() {
        assert_eq!(output_cpp(fmt::RGBA8), Some(4));
        assert_eq!(output_cpp(fmt::SRGB8_ALPHA8), Some(4));
        assert_eq!(output_cpp(fmt::RGB10_A2), Some(4));
        assert_eq!(output_cpp(fmt::R11F_G11F_B10F), Some(4));
        assert_eq!(output_cpp(fmt::BGR565), Some(2));
        assert_eq!(output_cpp(fmt::R16F), Some(2));
        assert_eq!(output_cpp(fmt::RGBA16F), Some(8));
        assert_eq!(output_cpp(fmt::RGBA32F), Some(16));
        assert_eq!(output_cpp(fmt::R8UI), Some(1));
        assert_eq!(output_cpp(fmt::RG16F), None);
        assert_eq!(sample_size(W, H, 4).bytes(), Some((W * H * 16) as usize));
    }

    #[test]
    fn store_and_load_words_match_job_check() {
        // job_check.c:2037-2052: RGBA8 kept samples ALL_SAMPLES UIF no XOR,
        // then the resolve 4X raster at W * 4; RT 1 SRGB8_ALPHA8 likewise
        let ms = MsLayout { height_in_ub: 26, ..ms_layout(W, H, 4).unwrap() };
        let [kept, resolve] = rt_stores(0, fmt::RGBA8, W * 4, true, Some(&ms));
        assert_eq!(
            kept,
            Some(TileBufferWords {
                buffer: BUFFER_RT0,
                memory_format: MEMORY_FORMAT_UIF_NO_XOR,
                image_format: fmt::RGBA8,
                height_in_ub_or_stride: 26,
                decimate: DECIMATE_ALL_SAMPLES,
            })
        );
        assert_eq!(
            resolve,
            Some(TileBufferWords {
                buffer: BUFFER_RT0,
                memory_format: MEMORY_FORMAT_RASTER,
                image_format: fmt::RGBA8,
                height_in_ub_or_stride: W * 4,
                decimate: DECIMATE_4X,
            })
        );
        let [k1, r1] = rt_stores(1, fmt::SRGB8_ALPHA8, W * 4, true, Some(&ms));
        assert_eq!(
            k1.map(|w| (w.buffer, w.image_format, w.decimate)),
            Some((1, fmt::SRGB8_ALPHA8, DECIMATE_ALL_SAMPLES))
        );
        assert_eq!(r1.map(|w| (w.buffer, w.memory_format, w.decimate)), Some((1, MEMORY_FORMAT_RASTER, DECIMATE_4X)));
        let ld = rt_sample_load(1, fmt::SRGB8_ALPHA8, &ms);
        assert_eq!(
            (ld.buffer, ld.memory_format, ld.image_format, ld.height_in_ub_or_stride, ld.decimate),
            (1, MEMORY_FORMAT_UIF_NO_XOR, fmt::SRGB8_ALPHA8, 26, DECIMATE_ALL_SAMPLES)
        );
        // samples not kept: the resolve alone (job_check.c:1990-1999)
        let [only, none] = rt_stores(0, fmt::RGBA8, W * 4, true, None);
        assert_eq!(only.map(|w| w.decimate), Some(DECIMATE_4X));
        assert_eq!(none, None);
        // 1x: sample 0, no kept store (job_check.c:2085-2092)
        let [one, none] = rt_stores(0, fmt::RGBA8, W * 4, false, Some(&ms));
        assert_eq!(one.map(|w| w.decimate), Some(DECIMATE_SAMPLE_0));
        assert_eq!(none, None);
    }

    #[test]
    fn depth_stencil_words() {
        // job_check.c:1983-1986: Z/S of a 4x job ALL_SAMPLES; 1x SAMPLE_0
        assert_eq!(zs_decimate(true), DECIMATE_ALL_SAMPLES);
        assert_eq!(zs_decimate(false), DECIMATE_SAMPLE_0);
        // the keep-sample-0 store (note (f1) item (3))
        let z = zs_sample0_store(false, MEMORY_FORMAT_UIF_NO_XOR, 13);
        assert_eq!(
            (z.buffer, z.image_format, z.decimate, z.height_in_ub_or_stride),
            (BUFFER_ZSTENCIL, fmt::D24S8, DECIMATE_SAMPLE_0, 13)
        );
        assert_eq!(zs_sample0_store(true, MEMORY_FORMAT_UIF_NO_XOR, 13).buffer, BUFFER_Z);
        let s = s_sample0_store(true, MEMORY_FORMAT_UIF_NO_XOR, 13);
        assert_eq!((s.buffer, s.image_format, s.decimate), (BUFFER_STENCIL, fmt::S8, DECIMATE_SAMPLE_0));
        assert_eq!(s_sample0_store(false, 0, 0).image_format, fmt::D24S8);
    }
}
