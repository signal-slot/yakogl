// SPDX-License-Identifier: Apache-2.0 AND MIT
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
// SPDX-FileCopyrightText: 2012-2017 Etnaviv Project
// SPDX-FileCopyrightText: Wladimir J. van der Laan <laanwj@gmail.com>
// SPDX-FileCopyrightText: Christian Gmeiner <christian.gmeiner@gmail.com>
// SPDX-FileCopyrightText: Lucas Stach <l.stach@pengutronix.de>
// SPDX-FileCopyrightText: Russell King <rmk@arm.linux.org.uk>
//! The GC7000UL (i.MX8MP, `src/vivante`) words of a 4x multisample
//! framebuffer object, as plain integers (docs/notes/es3-msaa-renderbuffers.md
//! (b)2, (c3), (e)4, (f3), (f5)). Not wired: `gc-msaa-fbo` calls them from
//! `src/vivante`.
//!
//! Three sources, each cited at the constant or function it supports:
//!
//! - today's window-only 4x path in `src/vivante/backend.rs` (`ms_resolvable`
//!   :965-967, `ms_scale` :970-976, `ms_pitch` :996-998, `ensure_msaa`
//!   :1046-1061, `ms_rs_format` :1260-1266, `emit_resolve` :1270-1286, the
//!   PE_LOGIC_OP write :2034-2042, the depth size :3382-3388) and
//!   `src/vivante/shader.rs` :364-367, :388-392;
//! - the measured arm 19 of docs/notes/gc-msaa-addresses.md ("The board A/B,
//!   run", 2026-10-15/16, `aed962e2`: case 16 passes only with bits 1 and 4
//!   together) and the register table of its section 2, read as
//!   docs/notes/gc-msaa-record.md describes;
//! - Mesa 26.2.1's etnaviv at tag `mesa-26.2.1`: `src/gallium/drivers/etnaviv/`
//!   (`etnaviv_translate.h`, `etnaviv_state.c`, `etnaviv_shader.c`,
//!   `etnaviv_emit.c`, `etnaviv_rs.c`, `etnaviv_resource.c`,
//!   `etnaviv_screen.c`, `etnaviv_internal.h`, `etnaviv_format.h`) and
//!   `src/etnaviv/hw/state_3d.xml.h` (the rnndb header the driver is built
//!   from). etnaviv is not in `third_party/mesa-26.2.1` (its UPSTREAM lists
//!   what was vendored); the copy read is the same tarball in the deps cache
//!   (`mesa-pan/mesa-26.2.1`), as gc-msaa-addresses.md section 2 did. Line
//!   numbers are those files'. MIT; read and transcribed.
//!
//! What `gc-msaa-fbo` must do with these ((c3), (e)4, (f5)):
//!
//! 1. **A scratch pool keyed by `(target_id, target_gen)`** through
//!    [`crate::msaa_store::MsaaStore`] (at least two slots, (e)2), one
//!    supertiled scratch surface of [`scratch_bytes`] per live 4x renderbuffer
//!    in place of the one `g().msaa` allocation every target shares today
//!    ((c3) "Sample storage"); the draw renders into the slot's scratch at
//!    [`MS_SCALE`] with the arm-19 state ([`pe_logic_op_4x`],
//!    [`ps_input_count_4x`], [`ps_temps_4x`]), the depth surface at
//!    [`depth_extent_4x`].
//! 2. **An RS resolve at every job end** into the renderbuffer's storage
//!    (`rb.mem`, contract (b)7), [`rs_config_resolve`] of
//!    [`rs_resolve_format_of_surface`] with [`rs_source_stride_resolve`] and
//!    [`rs_window_resolve`], so the resolved image is current whenever the GL
//!    core reads it; the slot is then `mark_stored`.
//! 3. **Refusing (`-ENOTSUP`) a draw it cannot make 4x**, per open point 4:
//!    a target whose slot has no samples yet (first draw, or evicted) and no
//!    whole clear (the RS cannot upsample `rb.mem` into the scratch, and a
//!    draw that copies it is not built), a size [`rs_resolvable`] refuses, and
//!    a format [`rs_resolve_format`] returns `None` for. Refusing is the
//!    choice this module documents; drawing single-sampled (today's rule for
//!    the window, `backend.rs:3354-3360`) is a coverage loss the item may
//!    take instead, with a host test of the stream either way.

use crate::backend::{
    GLES_SURF_RGB565, GLES_SURF_RGBA8888, GLES_SURF_SRGB8_A8, GLES_SURF_XRGB8888, GLES_TEXFMT_RGB565, GLES_TEXFMT_RGBA8,
};
use crate::gl::consts::*;
use crate::gl::types::GLenum;
use crate::msaa_store::SampleSize;

/// Samples of a 4x framebuffer object.
pub const SAMPLES_4X: u32 = 4;

/// The factor a 4x surface is scaled by on each axis: 4 samples are a 2x2
/// grid of pixels to the PE and the RS (`translate_samples_to_xyscale`,
/// etnaviv_translate.h:511-514; `ms_scale`, src/vivante/backend.rs:970-976).
/// 2x is `(2, 1)` in Mesa and does not work on a SMALL_MSAA core
/// (etnaviv_screen.c:463-465), which this is (gc-msaa-addresses.md section 2:
/// `REG_SmallMSAA` 1); the GL core never asks for it ((b)1).
pub const MS_SCALE: u32 = 2;

/// A supertile is 64x64 samples: a SUPER_TILED surface is padded to 64 on
/// both axes (etnaviv_resource.c:389-392; `ms_pitch` and `ensure_msaa`,
/// src/vivante/backend.rs:996-998, :1048).
pub const SUPERTILE: u32 = 64;

const fn align_up(v: u32, a: u32) -> u32 {
    (v + a - 1) & !(a - 1)
}

/// The row pitch in bytes of the 4x scratch of a target `w` pixels wide at
/// `bytes_per_pixel`: `2w` samples padded to a supertile, exactly `ms_pitch`
/// (src/vivante/backend.rs:996-998; Mesa pads `width * msaa_xscale` to
/// `paddingX` 64, etnaviv_resource.c:327, :390). Measured: `PE_COLOR_STRIDE`
/// `0x200` for a 64x64 RGBA8 target (gc-msaa-addresses.md section 1).
pub const fn scratch_pitch(w: u32, bytes_per_pixel: u32) -> u32 {
    align_up(w * MS_SCALE, SUPERTILE) * bytes_per_pixel
}

/// The rows of the 4x scratch of a target `h` pixels high: `2h` samples
/// padded to a supertile, as `ensure_msaa` sizes it (src/vivante/backend.rs:1048;
/// Mesa: `height * msaa_yscale` to `paddingY` 64, etnaviv_resource.c:328,
/// :391).
pub const fn scratch_rows(h: u32) -> u32 {
    align_up(h * MS_SCALE, SUPERTILE)
}

/// Bytes of the 4x scratch of a `w` x `h` target at `bytes_per_pixel`:
/// `scratch_pitch * scratch_rows`, the allocation `ensure_msaa` makes
/// (src/vivante/backend.rs:1046-1061, `bytes` at :1048). This is what the
/// slot's `alloc` must ask for: more than [`sample_size`]'s packed bytes, by
/// the supertile padding.
pub const fn scratch_bytes(w: u32, h: u32, bytes_per_pixel: u32) -> u32 {
    scratch_pitch(w, bytes_per_pixel) * scratch_rows(h)
}

/// The per-sample size of a `w` x `h` attachment of `bytes_per_pixel`, the
/// form [`crate::msaa_store::MsaaStore::get_or_alloc`] takes. Packed: the
/// scratch itself is [`scratch_bytes`] (the store's doc: a padding back end
/// asks its `alloc` for more itself).
pub const fn sample_size(w: u32, h: u32, bytes_per_pixel: u32) -> SampleSize {
    SampleSize { w, h, bytes_per_sample: bytes_per_pixel, samples: SAMPLES_4X }
}

/// Can the RS resolve a 4x target of this size? Every RS operation takes a
/// width that is a multiple of 16 (`ETNA_RS_WIDTH_MASK`,
/// etnaviv_internal.h:54-55; etnaviv aborts otherwise, etnaviv_rs.c:91) and a
/// tiled one a height that is a multiple of 4 (`ETNA_RS_HEIGHT_MASK`,
/// :56-57), both in **source samples** (etnaviv_rs.c:807-812): at 2x2 that is
/// a target width in multiples of 16 (the window of `2w` samples is then a
/// multiple of 32) and a target height in multiples of 4 (`2h` a multiple of
/// 8). `ms_resolvable`, src/vivante/backend.rs:965-967, note (c3) "Tile and
/// memory". dEQP's `fbo.msaa` and `multisample.*` targets (128, 256 or a
/// power of two a side) all pass.
pub const fn rs_resolvable(w: u32, h: u32) -> bool {
    w & 15 == 0 && h & 3 == 0
}

/// The depth/stencil surface a 4x draw tests and writes: the target's extent
/// times [`MS_SCALE`] on both axes (`ds_w * ms`, `ds_h * ms`,
/// src/vivante/backend.rs:3382-3388, into `ds_cache::select_in` keyed with
/// `scale` 2, src/vivante/ds_cache.rs:226-241), where `ds_w`/`ds_h` are
/// already the larger of the storage GL gave and the target. Note (c3)
/// "Depth" (`ds_w * 2`); whether the PE lays it out row-linear (measured
/// under SINGLE_BUFFER 1, 2026-09-30) or supertiled as Mesa's (E) is the
/// item's decision, not this function's.
pub const fn depth_extent_4x(ds_w: u32, ds_h: u32) -> (u32, u32) {
    (ds_w * MS_SCALE, ds_h * MS_SCALE)
}

/* ---- RS resolve format ------------------------------------------------- */

/// `RS_FORMAT_*` (state_3d.xml.h:76-87), the format field of `RS_CONFIG` on
/// both sides. The colour values equal `PE_FORMAT_*` (:88-94), which is why
/// `GC_RS_FORMAT_*` and `GC_PE_FORMAT_*` agree in src/vivante/regs.rs
/// (:1142-1145, :586-588).
pub const RS_FORMAT_A4R4G4B4: u32 = 0x01; // state_3d.xml.h:77
pub const RS_FORMAT_A1R5G5B5: u32 = 0x03; // state_3d.xml.h:79
pub const RS_FORMAT_R5G6B5: u32 = 0x04; // state_3d.xml.h:80
pub const RS_FORMAT_X8R8G8B8: u32 = 0x05; // state_3d.xml.h:81
pub const RS_FORMAT_A8R8G8B8: u32 = 0x06; // state_3d.xml.h:82
pub const RS_FORMAT_S8: u32 = 0x10; // state_3d.xml.h:84
pub const RS_FORMAT_D32: u32 = 0x17; // state_3d.xml.h:86
pub const RS_FORMAT_D16: u32 = 0x18; // state_3d.xml.h:87

/// The RS format of a 4x resolve of a renderbuffer of `internalformat`, or
/// `None` where the RS cannot resolve it: Mesa's `translate_rs_format`
/// (etnaviv_translate.h:279-310) for the pipe format a surface of that name
/// is drawn in. A downsampling RS takes only this table: the same-size
/// fallback `etna_compatible_rs_format` is allowed only when not downsampling
/// (etnaviv_rs.c:740-747), the source and destination format must be equal
/// (:728-738), and Mesa offers a format at 4x only when this table has it
/// (`gpu_supports_render_format`, etnaviv_screen.c:471-486).
///
/// - RGBA8, RGB8, R8, RG8: `A8R8G8B8` (the 8-bit group is stored RGBA8888,
///   src/gl/framebuffer.rs:776-789, which the PE writes as `A8R8G8B8` in
///   B,G,R,A order, src/vivante/backend.rs:333-350 — Mesa's `B8G8R8A8_UNORM`,
///   etnaviv_translate.h:306-309). The one 4x case, note (c3) "Formats";
///   measured as `RS_CONFIG` `0x6e6` in arm 19.
/// - SRGB8_ALPHA8: `A8R8G8B8`, Mesa's `B8G8R8A8_SRGB` row
///   (etnaviv_translate.h:307). The RS then averages the **encoded** bytes
///   ((f3): Mesa's shader resolve averages sampled values); whether that is
///   inside dEQP's tolerance is **board only**, and (f5) asks `gc-msaa-fbo`
///   to pin the choice in its host test before handing sRGB to the RS.
/// - RGB10_A2: `None`. No RS row (`R10G10B10A2` is absent from
///   etnaviv_translate.h:279-310); an `A8R8G8B8` RS of the packed word would
///   average its four 8-bit fields, wrong ((f3)). Stored RGBA8 or the packed
///   word (src/gl/framebuffer.rs:784-786), it is refused or drawn
///   single-sampled, the item's choice.
/// - RGB565: `R5G6B5` (etnaviv_translate.h:300-301; today's `ms_rs_format`,
///   src/vivante/backend.rs:1262, pinned by
///   `a_4x_rgb565_target_resolves_as_r5g6b5`).
/// - RGBA4: `A4R4G4B4` (`B4G4R4A4_UNORM`, etnaviv_translate.h:294-295).
/// - RGB5_A1: `A1R5G5B5` (`B5G5R5A1_UNORM`, etnaviv_translate.h:298-299).
///   Today's `ms_rs_format` gives every 2-byte surface but RGB565 `A4R4G4B4`
///   (src/vivante/backend.rs:1263), the mismatch note (c3) "State, measured"
///   names: an `A4R4G4B4` RS averages four 4-bit fields over a 5/5/5/1 word.
///   The two 16-bit formats are stored RGBA8888 today (src/gl/framebuffer.rs:
///   776-789), so with that storage the resolve is the surface's
///   ([`rs_resolve_format_of_surface`]); this row is the RS format of a true
///   16-bit RGB5_A1 surface (`PE_FORMAT_A1R5G5B5`, state_3d.xml.h:91).
/// - R16F, RGBA16F (and RG16F, RGB16F, stored RGBA16F): `None`. No RS row
///   has a float format, so Mesa offers none of them at 4x
///   (etnaviv_screen.c:479-486). They stay the single-sample float rows
///   `rt_format_drawable` allows (src/backend.rs:612-631, note (c3)
///   "Formats"): a 4x request draws 1x or is refused, open point 4.
/// - The 32F formats and R11F_G11F_B10F: `None`, as above and (b)2.
/// - Depth and stencil: `D32`. This part keeps one D24S8 word a pixel for
///   every depth or stencil renderbuffer (src/vivante/ds_cache.rs:10-15), and
///   Mesa resolves `S8_UINT_Z24_UNORM` / `X8Z24_UNORM` as `RS_FORMAT_D32`
///   (etnaviv_translate.h:287-289), depth and stencil together ((f3), (f5));
///   the emulated 32F depth formats are stored D24S8 too
///   (`translate_emulated_format_z32f`, etnaviv_format.h:105-114, applied at
///   etnaviv_translate.h:281). `RS_FORMAT_D16` (Z16, :285-286) and `S8`
///   (:290-291) are not reached: the part has no separate 16-bit depth or
///   stencil plane. Which sample the RS keeps is **board only**.
/// - Integer formats and every other name: `None` ((b)2: ES 3.0 has no
///   multisample integer format).
pub const fn rs_resolve_format(internalformat: GLenum) -> Option<u32> {
    match internalformat {
        GL_RGBA8 | GL_RGB8 | GL_R8 | GL_RG8 => Some(RS_FORMAT_A8R8G8B8),
        GL_SRGB8_ALPHA8 => Some(RS_FORMAT_A8R8G8B8),
        GL_RGB565 => Some(RS_FORMAT_R5G6B5),
        GL_RGBA4 => Some(RS_FORMAT_A4R4G4B4),
        GL_RGB5_A1 => Some(RS_FORMAT_A1R5G5B5),
        GL_DEPTH_COMPONENT16
        | GL_DEPTH_COMPONENT24
        | GL_DEPTH_COMPONENT32F
        | GL_DEPTH24_STENCIL8
        | GL_DEPTH32F_STENCIL8
        | GL_STENCIL_INDEX8 => Some(RS_FORMAT_D32),
        _ => None,
    }
}

/// The RS format of a 4x resolve of a colour surface as the GL core stores it
/// (`GLES_SURF_*`, src/backend.rs:374-450), the surface the PE draws the
/// scratch in and `rb.mem` holds: what `emit_resolve` must put on both sides
/// of `RS_CONFIG` (etnaviv_rs.c:728-738). The replacement of `ms_rs_format`
/// (src/vivante/backend.rs:1260-1266), keyed by surface alone: RGBA8888 and
/// the sRGB bytes `A8R8G8B8`, XRGB8888 `X8R8G8B8` (etnaviv_translate.h:
/// 302-305), RGB565 `R5G6B5`; `None` for the 10/10/10/2 word, every float
/// and integer surface (as [`rs_resolve_format`]). A 2-byte surface is never
/// `A4R4G4B4` by its size alone, which is the row this corrects.
pub const fn rs_resolve_format_of_surface(surf: u8) -> Option<u32> {
    match surf {
        GLES_SURF_RGBA8888 | GLES_SURF_SRGB8_A8 => Some(RS_FORMAT_A8R8G8B8),
        GLES_SURF_XRGB8888 => Some(RS_FORMAT_X8R8G8B8),
        GLES_SURF_RGB565 => Some(RS_FORMAT_R5G6B5),
        _ => None,
    }
}

/* ---- RS resolve words ---------------------------------------------------- */

/// `RS_CONFIG` field positions and bits (state_3d.xml.h:1351-1359, :1354-1356;
/// `rs_source_format`, `rs_dest_format`, `GC_RS_*`, src/vivante/regs.rs:
/// 1073-1085). The source format is bits 4:0 (shift 0, :1352), the
/// destination format bits 12:8.
const RS_CONFIG_DEST_FORMAT_SHIFT: u32 = 8; // state_3d.xml.h:1358
const RS_CONFIG_FORMAT_MASK: u32 = 0x1f; // state_3d.xml.h:1351, :1357
pub const RS_CONFIG_DOWNSAMPLE_X: u32 = 0x0000_0020; // state_3d.xml.h:1354
pub const RS_CONFIG_DOWNSAMPLE_Y: u32 = 0x0000_0040; // state_3d.xml.h:1355
pub const RS_CONFIG_SOURCE_TILED: u32 = 0x0000_0080; // state_3d.xml.h:1356

/// `RS_SOURCE_STRIDE` bits of a supertiled source on this
/// CACHE128B256BPERLINE core, with no tile status (TS mode 0)
/// (etnaviv_rs.c:104-110; state_3d.xml.h:1373, :1375; src/vivante/regs.rs:
/// 1091-1093).
pub const RS_SOURCE_STRIDE_SUPER_TILED_NEW: u32 = 0x0800_0000; // state_3d.xml.h:1373
pub const RS_SOURCE_STRIDE_TILING: u32 = 0x8000_0000; // state_3d.xml.h:1375
/// A tiled source's stride is given times four (`source_stride_shift`,
/// etnaviv_rs.c:104; `src_stride` of `emit_resolve`, src/vivante/backend.rs:
/// 1283; the record: `stride << 2`, gc-msaa-addresses.md section 2).
const RS_SOURCE_STRIDE_TILED_SHIFT: u32 = 2;

/// The `RS_CONFIG` word of the 2x2 resolve of a supertiled scratch into the
/// linear storage, both sides `fmt`: `SOURCE_FORMAT | DEST_FORMAT |
/// SOURCE_TILED | DOWNSAMPLE_X | DOWNSAMPLE_Y` (etnaviv_rs.c:95-102, the
/// downsample bits set when the source has more samples, :697-707;
/// `emit_resolve`, src/vivante/backend.rs:1276-1279). Measured `0x000006e6`
/// for `A8R8G8B8` (gc-msaa-addresses.md section 2, "same? yes").
pub const fn rs_config_resolve(fmt: u32) -> u32 {
    (fmt & RS_CONFIG_FORMAT_MASK)
        | ((fmt & RS_CONFIG_FORMAT_MASK) << RS_CONFIG_DEST_FORMAT_SHIFT)
        | RS_CONFIG_SOURCE_TILED
        | RS_CONFIG_DOWNSAMPLE_X
        | RS_CONFIG_DOWNSAMPLE_Y
}

/// The `RS_SOURCE_STRIDE` word of the resolve: the scratch's
/// [`scratch_pitch`] times four with `TILING` and `SUPER_TILED_NEW`, TS mode
/// 0 (etnaviv_rs.c:104-110; src/vivante/backend.rs:1283). Measured
/// `0x88000800` for the 512-byte pitch (gc-msaa-addresses.md section 2).
pub const fn rs_source_stride_resolve(scratch_pitch: u32) -> u32 {
    (scratch_pitch << RS_SOURCE_STRIDE_TILED_SHIFT) | RS_SOURCE_STRIDE_TILING | RS_SOURCE_STRIDE_SUPER_TILED_NEW
}

/// The `RS_WINDOW_SIZE` word of the resolve of a `w` x `h` target: the
/// window is in **source samples**, `2w` x `2h` (etnaviv_rs.c:807-810;
/// `emit_resolve`, src/vivante/backend.rs:1275, :1213; width bits 15:0, height
/// 31:16, state_3d.xml.h:1388-1393). Measured `0x00800080` for 64x64
/// (gc-msaa-addresses.md section 2).
pub const fn rs_window_resolve(w: u32, h: u32) -> u32 {
    ((w * MS_SCALE) & 0xffff) | (((h * MS_SCALE) & 0xffff) << 16)
}

/* ---- the arm-19 state ---------------------------------------------------- */

/// A 4x draw gives the fragment stage one more input and one more temporary
/// than the program's: "MSAA adds another input" / "another temp",
/// `PS_INPUT_COUNT_MSAA` and `PS_TEMP_REGISTER_CONTROL_MSAA`
/// (etnaviv_shader.c:233-241), chosen whenever `msaa_mode`
/// (etnaviv_emit.c:466-473). Today's `ms_extra`, src/vivante/shader.rs:
/// 364-367 and :390 (bit 1 of `gc_debug_msaa`); (A) of gc-msaa-addresses.md
/// section 3, in the passing arm 19.
pub const PS_4X_EXTRA_INPUTS: u32 = 1;
pub const PS_4X_EXTRA_TEMPS: u32 = 1;

/// `PS_INPUT_COUNT.COUNT` is bits 4:0 (state_3d.xml.h:635-637;
/// `ps_input_count_count`, src/vivante/regs.rs:357-359).
const PS_INPUT_COUNT_COUNT_MASK: u32 = 0x1f;
/// `PS_TEMP_REGISTER_CONTROL.NUM_TEMPS` is bits 5:0 (state_3d.xml.h:644-646;
/// `temp_register_num`, src/vivante/regs.rs:224-226).
const PS_TEMP_REGISTER_CONTROL_NUM_TEMPS_MASK: u32 = 0x3f;

/// The `COUNT` field of `PS_INPUT_COUNT` for a 4x draw of a program with
/// `varyings` inputs: the varyings, the position and the extra one,
/// `num_varyings + 2` (etnaviv_shader.c:236-238), against `+ 1` single-sampled
/// (:228-230). The caller ORs its `UNK8` field in as today
/// (src/vivante/shader.rs:388-391). Masked as the hardware masks it; the
/// caller's temp-limit check (`check_temp_limits`, src/vivante/shader.rs:368)
/// refuses a count that would wrap.
pub const fn ps_input_count_4x(varyings: u32) -> u32 {
    (varyings + 1 + PS_4X_EXTRA_INPUTS) & PS_INPUT_COUNT_COUNT_MASK
}

/// The `NUM_TEMPS` field of `PS_TEMP_REGISTER_CONTROL` for a 4x draw of a
/// program with `temps` temporaries and `varyings` inputs:
/// `MAX2(num_temps + 1, num_varyings + 2)` (etnaviv_shader.c:240-241;
/// `fs_temps`, src/vivante/shader.rs:367). Masked as [`ps_input_count_4x`].
pub const fn ps_temps_4x(temps: u32, varyings: u32) -> u32 {
    let a = temps + PS_4X_EXTRA_TEMPS;
    let b = varyings + 1 + PS_4X_EXTRA_INPUTS;
    (if a > b { a } else { b }) & PS_TEMP_REGISTER_CONTROL_NUM_TEMPS_MASK
}

/// `PE_LOGIC_OP.UNK24` is bits 26:24 (state_3d.xml.h:1144-1146; bit 27 is its
/// write mask, :1147, left clear as today: `pe_logic_op_unk24`,
/// src/vivante/regs.rs:553-557).
const PE_LOGIC_OP_UNK24_SHIFT: u32 = 24;
const PE_LOGIC_OP_UNK24_MASK: u32 = 0x0700_0000; // state_3d.xml.h:1144
/// `PE_LOGIC_OP.SINGLE_BUFFER` is bits 9:8 (state_3d.xml.h:1135-1137;
/// `pe_logic_op_single_buffer`, src/vivante/regs.rs:550-552).
const PE_LOGIC_OP_SINGLE_BUFFER_SHIFT: u32 = 8;
const PE_LOGIC_OP_SINGLE_BUFFER_MASK: u32 = 0x0000_0300; // state_3d.xml.h:1135

/// `UNK24 = 5` for a 4x target on a SMALL_MSAA core (etnaviv_state.c:524-525;
/// the feature: gc-msaa-addresses.md section 2, `REG_SmallMSAA` 1). Unnamed
/// by Mesa. Today behind bit 1 (`msaa_extras`, src/vivante/backend.rs:
/// 716-718, :2034); (C) of gc-msaa-addresses.md, in the passing arm 19.
pub const PE_LOGIC_OP_UNK24_4X: u32 = 5 << PE_LOGIC_OP_UNK24_SHIFT;

/// `SINGLE_BUFFER` of a 4x draw into the supertiled scratch: Mesa's value for
/// a target that is not linear on a SINGLE_BUFFER core, 3 at 16 bits a pixel
/// or less, else 2 (etnaviv_state.c:534-539, `target_16bpp` from the colour
/// block size :266-267 or a 16-bit depth :423-424; the feature:
/// `REG_PEEnhancements3` 1, `etna_hwdb.c:84`). Today's `single_buffer`,
/// src/vivante/backend.rs:813-825, behind bit 4. **2 is measured** (arm 19,
/// 32-bit target: (B) of gc-msaa-addresses.md); 3 is Mesa's rule for a
/// 16-bit one, not run on a board. The single-sampled value is 1, the linear
/// surface's (:2031-2033), which [`PE_LOGIC_OP_4X_MASK`] clears first.
pub const fn pe_logic_op_single_buffer_4x(bytes_per_pixel: u32) -> u32 {
    (if bytes_per_pixel <= 2 { 3 } else { 2 }) << PE_LOGIC_OP_SINGLE_BUFFER_SHIFT
}

/// The bits a 4x draw ORs into `PE_LOGIC_OP` over `BASE | op | sRGB`
/// (src/vivante/backend.rs:2035-2042): [`PE_LOGIC_OP_UNK24_4X`] and
/// [`pe_logic_op_single_buffer_4x`], after clearing [`PE_LOGIC_OP_4X_MASK`].
/// Arm 19 is bit 1 (UNK24, with the PS extras) and bit 4 (SINGLE_BUFFER 2)
/// together; neither alone writes the whole scratch (gc-msaa-addresses.md
/// "The board A/B, run": arm 3 half the chunks, arm 17 none, arm 19 passes).
pub const fn pe_logic_op_4x(bytes_per_pixel: u32) -> u32 {
    PE_LOGIC_OP_UNK24_4X | pe_logic_op_single_buffer_4x(bytes_per_pixel)
}

/// The `PE_LOGIC_OP` fields [`pe_logic_op_4x`] sets: clear them from the
/// single-sampled word before ORing (the single-sampled word has
/// `SINGLE_BUFFER` 1 and `UNK24` 0).
pub const PE_LOGIC_OP_4X_MASK: u32 = PE_LOGIC_OP_UNK24_MASK | PE_LOGIC_OP_SINGLE_BUFFER_MASK;

/* ---- the sample reload of a flushed 4x FBO ------------------------------- */
//
// A 4x FBO drawn into again after a flush: the job before it ended with the
// RS resolve into `rb.mem` (contract (b)7), and the slot of
// `crate::msaa_store` either still holds that job's samples or does not.
//
// - The slot is there and `needs_load()` is true: the scratch is used as it
//   is, no words at all. The PE reads and writes the supertiled scratch in
//   place (no tile buffer to load, unlike the Mali and the PowerVR), so
//   "loading" the kept samples is binding the same scratch again.
// - No samples (a new slot after an eviction or the first draw, or `None`
//   from `get_or_alloc`): `rb.mem` must be broadcast into all four samples.
//   The RS cannot do it. `RS_CONFIG` has `DOWNSAMPLE_X` / `DOWNSAMPLE_Y` and no
//   upsample field (state_3d.xml.h:1350-1362), and etnaviv's RS blit refuses
//   a destination with more samples than its source ("RS does not support
//   upscaling", etnaviv_rs.c:697-702). Its CPU tile copy takes single-sampled
//   resources only (`etna_manual_blit`, etnaviv_rs.c:594-595). etnaviv itself
//   never reloads: its 4x renderbuffer is the 4x resource and stays one. Its
//   only single-to-multi route is the blitter's draw: `etna_blit` falls from
//   `ctx->blit` (the RS, etnaviv_rs.c:1010) and the copy-region path to
//   `util_blitter_blit` (etnaviv_clear_blit.c:185-218). For a single-sampled
//   source that draws a textured quad with the plain texel-fetch program
//   (u_blitter.c:1002, the `src_nr_samples > 1` paths skipped), sample mask
//   `~0` and min samples 1 (u_blitter.c:1878-1890). One fragment per pixel
//   writes the same colour into every covered sample: the broadcast. The
//   functions below give that draw in this crate's terms. The quad,
//   program and sampler are `src/vivante/mipmap.rs`'s `record_level` (:134-211)
//   with a nearest filter, drawn at 4x.

/// The RS has no upsample: a 1x to 4x reload is a draw, never an
/// `RS_CONFIG` word (etnaviv_rs.c:697-702; state_3d.xml.h:1350-1362).
pub const RS_CAN_UPSAMPLE: bool = false;

/// What a 4x draw into an FBO starts from at the beginning of a job.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SampleLoad {
    /// The slot holds the last job's samples (`Slot::needs_load` true): draw
    /// into its scratch as it is, nothing emitted before the draw.
    Keep,
    /// The job starts with a clear of the whole attachment: the clear writes
    /// every sample, so nothing is loaded (`Slot::mark_cleared`,
    /// src/msaa_store.rs:99-102).
    Cleared,
    /// No samples kept: draw `rb.mem` into all four samples first
    /// ([`reload_draw`]), into the slot's scratch when there is a slot. With no
    /// slot (`get_or_alloc` returned `None`), into a scratch kept for this
    /// job only and resolved at its end as usual.
    Broadcast,
}

/// The [`SampleLoad`] of a job, from the slot the store gave
/// (`Some(slot.needs_load())`, or `None` when `get_or_alloc` returned
/// `None`) and whether the job begins with a whole clear of the attachment.
/// The store's contract: a new slot does not need a load
/// (src/msaa_store.rs:138), and a job starts "from its clear, or from the
/// resolved image in `rb.mem` in every sample" (:87-89). An evicted
/// attachment "starts from that image in every sample" (:23-26). The
/// samples-not-kept case is "drawn the same way as after an eviction"
/// (:28-30).
pub const fn sample_load(slot_needs_load: Option<bool>, whole_clear: bool) -> SampleLoad {
    if whole_clear {
        return SampleLoad::Cleared;
    }
    match slot_needs_load {
        Some(true) => SampleLoad::Keep,
        Some(false) | None => SampleLoad::Broadcast,
    }
}

/// `GL_MULTI_SAMPLE_CONFIG` (0x3818, state.xml.h:313-322): `MSAA_SAMPLES`
/// bits 1:0, `MSAA_ENABLES` bits 7:4.
const GL_MULTI_SAMPLE_CONFIG_MSAA_SAMPLES_4X: u32 = 0x0000_0002; // state.xml.h:318
const GL_MULTI_SAMPLE_CONFIG_MSAA_ENABLES_SHIFT: u32 = 4; // state.xml.h:321
const GL_MULTI_SAMPLE_CONFIG_MSAA_ENABLES_MASK: u32 = 0x0000_00f0; // state.xml.h:320

/// The `GL_MULTI_SAMPLE_CONFIG` word of the reload draw: 4x with every
/// sample enabled. This is the blitter's sample mask `~0` (u_blitter.c:1888)
/// through `etna_set_sample_mask` (etnaviv_state.c:74-79) and the emit, which
/// masks it to the four enables and ORs the framebuffer's 4X
/// (etnaviv_emit.c:358-363, etnaviv_state.c:505-506). It is also the word
/// every 4x draw already emits (`GC_MSAA_SAMPLES_4X | msaa_enables(0xf)`,
/// src/vivante/backend.rs:1108), so the reload adds no state of its own.
pub const GL_MULTI_SAMPLE_CONFIG_RELOAD: u32 = GL_MULTI_SAMPLE_CONFIG_MSAA_SAMPLES_4X
    | ((!0u32 << GL_MULTI_SAMPLE_CONFIG_MSAA_ENABLES_SHIFT) & GL_MULTI_SAMPLE_CONFIG_MSAA_ENABLES_MASK);

/// The reload draw's sampler and target, the fields `gc-msaa-fbo` puts into
/// the `GlesDraw` / `GlesTextureBind` it builds the way `record_level` does
/// (src/vivante/mipmap.rs:143-207).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ReloadDraw {
    /// `GlesTextureBind::format` that `rb.mem` is bound as.
    pub tex_format: u8,
    /// `GlesTextureBind::bgra`. 1 binds the PE's B,G,R,A bytes as they are
    /// (mipmap.rs:36-43, :196-197), so the draw writes back the bytes it read.
    pub bgra: u8,
    /// `GlesDraw::target_format` of the 4x draw into the scratch.
    pub target_format: u8,
    /// `GlesDraw::samples`.
    pub samples: u8,
}

/// The texture filter of the reload: nearest (`linear_filter` 0,
/// `min_linear` 0, `mip_filter` 0, one level, CLAMP_TO_EDGE). The program
/// `mipgen.frag` samples at `gl_FragCoord.xy * u_scale`
/// (tools/shaders/internal/es/mipgen.frag). With [`reload_scale`] that is the
/// centre of the pixel's own texel, so nearest returns that texel exactly.
/// mipmap.rs uses linear (:190-191) because it averages a 2x2 box.
pub const RELOAD_LINEAR_FILTER: u8 = 0;

/// The `u_scale` uniform of `mipgen.frag` for a reload of a `w` x `h`
/// target: one over the target's size, as `record_level` sets it for its
/// destination (src/vivante/mipmap.rs:140). Source and destination are the
/// same size here. `None` for an empty target.
pub fn reload_scale(w: u32, h: u32) -> Option<[f32; 2]> {
    if w == 0 || h == 0 {
        return None;
    }
    Some([1.0 / w as f32, 1.0 / h as f32])
}

/// The reload draw of a colour surface `surf` (`GLES_SURF_*`, as `rb.mem`
/// holds it), or `None` where there is no reload. Only the surfaces
/// [`rs_resolve_format_of_surface`] resolves reach 4x at all.
///
/// - RGBA8888: bound `GLES_TEXFMT_RGBA8` with `bgra` 1 and drawn as RGBA8888,
///   as mipmap.rs does (:147, :182, :197).
/// - SRGB8_A8: the same bytes, bound **and drawn** as RGBA8888. The bind
///   does not decode, and an SRGB8_A8 target would make the PE encode again
///   (`GC_RT_CONFIG_SRGB` and `pe_logic_op_srgb`, src/vivante/backend.rs:1995,
///   :2041). The scratch holds the same `A8R8G8B8` bytes either way, the RS
///   format of both (`rs_resolve_format_of_surface`), so the encoded values
///   go back unchanged.
/// - XRGB8888: bound RGBA8 `bgra` 1, drawn XRGB8888. The X byte is read as
///   alpha and not written (`X8R8G8B8`, etnaviv_translate.h:302-305).
/// - RGB565: bound `GLES_TEXFMT_RGB565`, which the texture unit reads as
///   `TEXTURE_FORMAT_R5G6B5` (src/vivante/texture.rs:271), the packing the PE
///   writes (`PE_FORMAT_R5G6B5`, the same value as `RS_FORMAT_R5G6B5`,
///   state_3d.xml.h:80, :88-94), `bgra` 0.
///   TODO(record): no board run of a 565 4x target draws `rb.mem` back. The
///   record that settles it is the gl_test 4x case redrawn after a flush
///   without a clear, at RGB565, on the i.MX8MP (`gc-msaa-fbo`'s board
///   check).
/// - Every other surface: `None`.
///
/// Depth and stencil are not here. TODO(record): this core's depth is the
/// `ds_cache` buffer at `ds_w * 2` (src/vivante/backend.rs:3382-3388), not a
/// scratch slot. Whether it survives the flush is `ds_persist` (note (c3)
/// "Depth"). Reloading it from the D24S8 resolve needs a depth-writing
/// draw, and for stencil Mesa's own fallback (etnaviv_clear_blit.c:191-207,
/// `etna_blit_stencil_fallback`). Neither has a record on this part. The
/// records that settle it: a host test of the 4x `ds_cache` slot across a
/// flush, then a board run of a 4x depth+stencil FBO redrawn after a flush.
pub const fn reload_draw(surf: u8) -> Option<ReloadDraw> {
    let (tex_format, bgra, target_format) = match surf {
        GLES_SURF_RGBA8888 | GLES_SURF_SRGB8_A8 => (GLES_TEXFMT_RGBA8, 1, GLES_SURF_RGBA8888),
        GLES_SURF_XRGB8888 => (GLES_TEXFMT_RGBA8, 1, GLES_SURF_XRGB8888),
        GLES_SURF_RGB565 => (GLES_TEXFMT_RGB565, 0, GLES_SURF_RGB565),
        _ => return None,
    };
    Some(ReloadDraw { tex_format, bgra, target_format, samples: SAMPLES_4X as u8 })
}

/// The scratch geometry of a reload is the 4x draw's own ([`scratch_pitch`],
/// [`scratch_rows`]): the draw writes the slot's scratch at [`MS_SCALE`] with
/// the arm-19 state ([`pe_logic_op_4x`], [`ps_input_count_4x`],
/// [`ps_temps_4x`] of `mipgen.frag`'s program, which reads no varyings), and
/// the viewport is the target's pixels `[0, 0, w, h]`, as every arm-19 draw
/// (bit 3, value 8, the scaled viewport, is off: src/vivante/backend.rs:859-860).
/// TODO(record): that `gl_FragCoord` is in pixels (not samples) for a 4x
/// draw on this core is Mesa's model. Mesa's viewport is in pixels
/// (etnaviv_state.c:585-598), and the extra MSAA PS input
/// (etnaviv_shader.c:233-241) is not named. It is not measured here. The
/// gl_test redraw case above settles it: a reload with `gl_FragCoord` in
/// samples would read the texel at twice the coordinate and show the
/// image's top-left quarter, scaled up.
pub const fn reload_viewport(w: u32, h: u32) -> [i32; 4] {
    [0, 0, w as i32, h as i32]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::*;

    /// The scratch geometry equals today's `ms_pitch` / `ensure_msaa`
    /// (src/vivante/backend.rs:996-998, :1048), computed by hand from that
    /// code: `align_up(w * 2, 64) * bpp` by `align_up(h * 2, 64)`.
    #[test]
    fn scratch_geometry_equals_ensure_msaa_by_hand() {
        // 64x64 RGBA8 (case 16): 128 samples a row, 512-byte rows, 128 rows,
        // 64 KiB: the record's PE_COLOR_STRIDE 0x200 and 65536-byte probe
        // (gc-msaa-addresses.md section 1).
        assert_eq!(scratch_pitch(64, 4), 0x200);
        assert_eq!(scratch_rows(64), 128);
        assert_eq!(scratch_bytes(64, 64, 4), 65536);
        // 100x50 RGB565: 200 samples padded to 256, times 2 bytes; 100 rows
        // padded to 128.
        assert_eq!(scratch_pitch(100, 2), 256 * 2);
        assert_eq!(scratch_rows(50), 128);
        assert_eq!(scratch_bytes(100, 50, 2), 512 * 128);
        // 128x128 RGBA8, dEQP's fbo.msaa size: 256 x 256 samples.
        assert_eq!(scratch_bytes(128, 128, 4), 256 * 4 * 256);
        // 1x1 RGBA8: one supertile.
        assert_eq!(scratch_bytes(1, 1, 4), 64 * 4 * 64);
        // 33x65 RGBA16F: 66 samples padded to 128, 130 rows padded to 192.
        assert_eq!(scratch_pitch(33, 8), 128 * 8);
        assert_eq!(scratch_rows(65), 192);
        assert_eq!(scratch_bytes(33, 65, 8), 1024 * 192);
    }

    /// The packed store size is never more than the padded scratch, so a slot
    /// allocated at `scratch_bytes` holds its `SampleSize`.
    #[test]
    fn store_size_fits_the_scratch() {
        for (w, h, bpp) in [(64, 64, 4), (100, 50, 2), (128, 128, 4), (1, 1, 4), (33, 65, 8), (2048, 2048, 4)] {
            let s = sample_size(w, h, bpp);
            assert_eq!(s, SampleSize { w, h, bytes_per_sample: bpp, samples: 4 });
            let packed = s.bytes().unwrap();
            assert!(packed <= scratch_bytes(w, h, bpp) as usize, "{w}x{h}x{bpp}: {packed}");
        }
        assert_eq!(sample_size(2048, 2048, 4).bytes(), Some(64 << 20), "the (b)3 cap, 64 MiB at RGBA8");
        assert_eq!(sample_size(0, 8, 4).bytes(), None);
    }

    /// `ms_resolvable`, src/vivante/backend.rs:965-967, and Mesa's masks in
    /// source samples (etnaviv_internal.h:55, :57; etnaviv_rs.c:807-812).
    #[test]
    fn resolvable_sizes() {
        for (w, h) in [(16, 4), (64, 64), (128, 128), (256, 256), (2048, 2048), (112, 36)] {
            assert!(rs_resolvable(w, h), "{w}x{h}");
            assert_eq!((w * MS_SCALE) % 16, 0);
            assert_eq!((h * MS_SCALE) % 4, 0);
        }
        for (w, h) in [(8, 4), (17, 4), (16, 2), (16, 3), (127, 127), (100, 50), (0, 0)] {
            assert_eq!(rs_resolvable(w, h), w & 15 == 0 && h & 3 == 0, "{w}x{h}");
        }
        assert!(!rs_resolvable(100, 50) && !rs_resolvable(127, 4) && !rs_resolvable(16, 6));
        assert!(rs_resolvable(0, 0), "an empty target is the caller's refusal, not the RS's");
    }

    /// The depth surface of a 4x draw is twice the target on both axes
    /// (src/vivante/backend.rs:3387-3388).
    #[test]
    fn depth_extent_doubles() {
        assert_eq!(depth_extent_4x(64, 64), (128, 128));
        assert_eq!(depth_extent_4x(100, 50), (200, 100));
        assert_eq!(depth_extent_4x(1, 1), (2, 2));
    }

    /// `RS_FORMAT_*` pin state_3d.xml.h:76-87, and equal the PE's colour
    /// values (state_3d.xml.h:88-94; src/vivante/regs.rs:586-588,
    /// :1142-1145).
    #[test]
    fn rs_format_values_pin_the_rnndb_header() {
        assert_eq!(RS_FORMAT_A4R4G4B4, 1);
        assert_eq!(RS_FORMAT_A1R5G5B5, 3);
        assert_eq!(RS_FORMAT_R5G6B5, 4);
        assert_eq!(RS_FORMAT_X8R8G8B8, 5);
        assert_eq!(RS_FORMAT_A8R8G8B8, 6);
        assert_eq!(RS_FORMAT_S8, 0x10);
        assert_eq!(RS_FORMAT_D32, 0x17);
        assert_eq!(RS_FORMAT_D16, 0x18);
    }

    /// Mesa's `translate_rs_format` rows (etnaviv_translate.h:279-310) per
    /// renderbuffer internalformat of note (b)2.
    #[test]
    fn resolve_format_per_internalformat() {
        for f in [GL_RGBA8, GL_RGB8, GL_R8, GL_RG8] {
            assert_eq!(rs_resolve_format(f), Some(RS_FORMAT_A8R8G8B8), "0x{f:x}: the RGBA8 group");
        }
        // B8G8R8A8_SRGB is in the table (:307); the averaging space is board only.
        assert_eq!(rs_resolve_format(GL_SRGB8_ALPHA8), Some(RS_FORMAT_A8R8G8B8));
        assert_eq!(rs_resolve_format(GL_RGB10_A2), None, "no R10G10B10A2 row; A8R8G8B8 would average the fields");
        assert_eq!(rs_resolve_format(GL_RGB565), Some(RS_FORMAT_R5G6B5), ":300-301");
        assert_eq!(rs_resolve_format(GL_RGBA4), Some(RS_FORMAT_A4R4G4B4), ":294-295");
        assert_eq!(rs_resolve_format(GL_RGB5_A1), Some(RS_FORMAT_A1R5G5B5), ":298-299");
        for f in [GL_R16F, GL_RG16F, GL_RGB16F, GL_RGBA16F, GL_R32F, GL_RG32F, GL_RGBA32F, GL_R11F_G11F_B10F] {
            assert_eq!(rs_resolve_format(f), None, "0x{f:x}: no float row; a single-sample float row");
        }
        for f in [
            GL_DEPTH_COMPONENT16,
            GL_DEPTH_COMPONENT24,
            GL_DEPTH_COMPONENT32F,
            GL_DEPTH24_STENCIL8,
            GL_DEPTH32F_STENCIL8,
            GL_STENCIL_INDEX8,
        ] {
            assert_eq!(rs_resolve_format(f), Some(RS_FORMAT_D32), "0x{f:x}: one D24S8 word, RS D32 (:287-289)");
        }
        for f in [GL_RGBA8I, GL_RGBA8UI, GL_R32UI, GL_RGB10_A2UI, GL_RGBA, GL_RGB, GL_RGBA16I] {
            assert_eq!(rs_resolve_format(f), None, "0x{f:x}: integer or unsized");
        }
    }

    /// Why RGB5_A1 is `A1R5G5B5` and not today's `A4R4G4B4`
    /// (`ms_rs_format`, src/vivante/backend.rs:1260-1266, `(_, 2) =>
    /// GC_RS_FORMAT_A4R4G4B4`): the RS reads and writes the one format on
    /// both sides (etnaviv_rs.c:728-738) and averages its fields, so a
    /// 5/5/5/1 word read as 4/4/4/4 averages bits of one channel into another
    /// (and the alpha bit into blue); Mesa names the 5551 row `A1R5G5B5`
    /// (etnaviv_translate.h:298-299) and the 4444 row `A4R4G4B4`
    /// (:294-295), two values (3 and 1, state_3d.xml.h:79, :77). RGB565 keeps
    /// today's `R5G6B5`. The today's-choice function is reproduced here from
    /// its source so the difference is pinned without `src/vivante`
    /// (compiled only with `gpu-vivante`).
    #[test]
    fn rgb5_a1_is_not_todays_a4r4g4b4() {
        // src/vivante/backend.rs:1260-1266, transcribed.
        fn todays_ms_rs_format(surf_format: u8, bpp: u32) -> u32 {
            match (surf_format, bpp) {
                (GLES_SURF_RGB565, _) => RS_FORMAT_R5G6B5,
                (_, 2) => RS_FORMAT_A4R4G4B4,
                _ => RS_FORMAT_A8R8G8B8,
            }
        }
        // A 16-bit surface that is not RGB565 is A4R4G4B4 today, whatever it
        // holds: for a 5/5/5/1 word that is the wrong field layout.
        let todays_16bit = todays_ms_rs_format(GLES_SURF_RGBA8888 + 100, 2);
        assert_eq!(todays_16bit, RS_FORMAT_A4R4G4B4);
        assert_ne!(rs_resolve_format(GL_RGB5_A1), Some(todays_16bit), "RGB5_A1 is A1R5G5B5, not A4R4G4B4");
        assert_eq!(rs_resolve_format(GL_RGB5_A1), Some(RS_FORMAT_A1R5G5B5));
        assert_eq!(rs_resolve_format(GL_RGBA4), Some(todays_16bit), "RGBA4 is the one 4/4/4/4 format");
        // The 5551 and 4444 layouts share no field boundary but the top: an
        // A4R4G4B4 average of two A1R5G5B5 words mixes red's low bit into
        // green's field and the alpha bit into blue's.
        let fields_5551 = [(15, 1), (10, 5), (5, 5), (0, 5)];
        let fields_4444 = [(12, 4), (8, 4), (4, 4), (0, 4)];
        let starts = |f: [(u32, u32); 4]| f.iter().map(|(s, _)| *s).collect::<Vec<_>>();
        assert_ne!(starts(fields_5551), starts(fields_4444));
        assert_eq!(fields_5551.iter().map(|(_, n)| n).sum::<u32>(), 16);
        assert_eq!(fields_4444.iter().map(|(_, n)| n).sum::<u32>(), 16);
        // RGB565 agrees with today.
        assert_eq!(rs_resolve_format(GL_RGB565), Some(todays_ms_rs_format(GLES_SURF_RGB565, 2)));
        // The RGBA8 group agrees with today's 4-byte row.
        assert_eq!(rs_resolve_format(GL_RGBA8), Some(todays_ms_rs_format(GLES_SURF_RGBA8888, 4)));
    }

    /// The surface-keyed table: what `rb.mem` holds today for each name
    /// (src/gl/framebuffer.rs:776-789), and where it differs from the
    /// internalformat table (RGBA4 and RGB5_A1 stored RGBA8888).
    #[test]
    fn resolve_format_per_stored_surface() {
        assert_eq!(rs_resolve_format_of_surface(GLES_SURF_RGBA8888), Some(RS_FORMAT_A8R8G8B8));
        assert_eq!(rs_resolve_format_of_surface(GLES_SURF_SRGB8_A8), Some(RS_FORMAT_A8R8G8B8));
        assert_eq!(rs_resolve_format_of_surface(GLES_SURF_XRGB8888), Some(RS_FORMAT_X8R8G8B8));
        assert_eq!(rs_resolve_format_of_surface(GLES_SURF_RGB565), Some(RS_FORMAT_R5G6B5));
        for s in [GLES_SURF_RGB10_A2, GLES_SURF_R16F, GLES_SURF_RGBA16F, GLES_SURF_R32F, GLES_SURF_RG32F] {
            assert_eq!(rs_resolve_format_of_surface(s), None, "surface {s}");
        }
        for s in GLES_SURF_INT_FIRST..=GLES_SURF_INT_LAST {
            assert_eq!(rs_resolve_format_of_surface(s), None, "integer surface {s}");
        }
        // The two tables agree where the stored surface is the format's own.
        assert_eq!(rs_resolve_format(GL_RGBA8), rs_resolve_format_of_surface(GLES_SURF_RGBA8888));
        assert_eq!(rs_resolve_format(GL_RGB565), rs_resolve_format_of_surface(GLES_SURF_RGB565));
        // Stored RGBA8888, RGBA4 and RGB5_A1 resolve as the surface, not as
        // their 16-bit rows: the item uses the surface table for RS_CONFIG.
        assert_ne!(rs_resolve_format(GL_RGBA4), rs_resolve_format_of_surface(GLES_SURF_RGBA8888));
        assert_ne!(rs_resolve_format(GL_RGB5_A1), rs_resolve_format_of_surface(GLES_SURF_RGBA8888));
        // A 16-bit float surface is never A4R4G4B4 by its size (today's row).
        assert_eq!(gles_surf_bytes_per_pixel(GLES_SURF_R16F), 2);
        assert_ne!(rs_resolve_format_of_surface(GLES_SURF_R16F), Some(RS_FORMAT_A4R4G4B4));
    }

    /// The resolve words equal the arm-19 record's (gc-msaa-addresses.md
    /// section 2: RS_CONFIG 0x6e6, RS_SOURCE_STRIDE 0x88000800, RS_WINDOW_SIZE
    /// 0x00800080 for the 64x64 RGBA8 target) and today's `emit_resolve`
    /// (src/vivante/backend.rs:1275-1285).
    #[test]
    fn resolve_words_pin_the_record() {
        assert_eq!(rs_config_resolve(RS_FORMAT_A8R8G8B8), 0x0000_06e6);
        assert_eq!(rs_source_stride_resolve(scratch_pitch(64, 4)), 0x8800_0800);
        assert_eq!(rs_window_resolve(64, 64), 0x0080_0080);
        // The fields: format on both sides, tiled source, both downsample bits.
        let c = rs_config_resolve(RS_FORMAT_R5G6B5);
        assert_eq!(c & 0x1f, RS_FORMAT_R5G6B5);
        assert_eq!((c >> 8) & 0x1f, RS_FORMAT_R5G6B5);
        assert_eq!(c & (RS_CONFIG_SOURCE_TILED | RS_CONFIG_DOWNSAMPLE_X | RS_CONFIG_DOWNSAMPLE_Y), 0xe0);
        assert_eq!(c, 0x0000_04e4);
        // 128x128: 256-sample window, 1024-byte pitch.
        assert_eq!(rs_window_resolve(128, 128), 0x0100_0100);
        assert_eq!(rs_source_stride_resolve(scratch_pitch(128, 4)), (1024 << 2) | 0x8800_0000);
    }

    /// The PS counts: `num_varyings + 2` and `MAX2(num_temps + 1,
    /// num_varyings + 2)` (etnaviv_shader.c:236-241), one more than the
    /// single-sampled `+ 1` / `MAX2(num_temps, num_varyings + 1)`
    /// (:228-231), as the host test of bit 1 pins ("one more fragment
    /// input", src/vivante/backend_tests.rs:4641).
    #[test]
    fn ps_counts_add_one() {
        assert_eq!(PS_4X_EXTRA_INPUTS, 1);
        assert_eq!(PS_4X_EXTRA_TEMPS, 1);
        for varyings in 0..8 {
            let single = varyings + 1;
            assert_eq!(ps_input_count_4x(varyings), single + 1);
            for temps in 0..12 {
                let single_temps = temps.max(varyings + 1);
                assert_eq!(ps_temps_4x(temps, varyings), (temps + 1).max(varyings + 2));
                assert!(ps_temps_4x(temps, varyings) > single_temps);
            }
        }
        // src/vivante/shader.rs:367 with ms_extra 1: a program of 3 temps and
        // 1 varying wants max(4, 3) = 4 temporaries; of 1 temp and 1 varying,
        // max(2, 3) = 3.
        assert_eq!(ps_temps_4x(3, 1), 4);
        assert_eq!(ps_temps_4x(1, 1), 3);
        // Field widths (state_3d.xml.h:635, :644).
        assert_eq!(ps_input_count_4x(0x1f), 1);
        assert_eq!(ps_temps_4x(0x3f, 0), 0);
    }

    /// `PE_LOGIC_OP`: UNK24 5 in bits 26:24 and SINGLE_BUFFER 2 in bits 9:8
    /// for a 32-bit target, the measured arm 19 (gc-msaa-addresses.md); 3
    /// for a 16-bit one is Mesa's (etnaviv_state.c:539), unmeasured. The bit
    /// positions are `pe_logic_op_unk24` / `pe_logic_op_single_buffer`
    /// (src/vivante/regs.rs:550-557), read back as the host tests do
    /// (`>> 24 & 7`, `>> 8 & 3`: src/vivante/backend_tests.rs:4627, :1431).
    #[test]
    fn pe_logic_op_words_pin_arm_19() {
        assert_eq!(PE_LOGIC_OP_UNK24_4X, 0x0500_0000);
        assert_eq!((PE_LOGIC_OP_UNK24_4X >> 24) & 0x7, 5);
        for bpp in [4, 8] {
            let w = pe_logic_op_4x(bpp);
            assert_eq!((w >> 8) & 0x3, 2, "{bpp} bytes: SINGLE_BUFFER 2, measured");
            assert_eq!((w >> 24) & 0x7, 5);
            assert_eq!(w, 0x0500_0200);
        }
        for bpp in [1, 2] {
            let w = pe_logic_op_4x(bpp);
            assert_eq!((w >> 8) & 0x3, 3, "{bpp} bytes: SINGLE_BUFFER 3, Mesa's, not run");
            assert_eq!(w, 0x0500_0300);
        }
        // Only the two fields are touched: the mask covers them and nothing
        // else, and the write-mask bit 27 stays clear.
        assert_eq!(PE_LOGIC_OP_4X_MASK, 0x0700_0300);
        for bpp in [1, 2, 4, 8] {
            assert_eq!(pe_logic_op_4x(bpp) & !PE_LOGIC_OP_4X_MASK, 0);
            assert_eq!(pe_logic_op_4x(bpp) & 0x0800_0000, 0);
        }
        // Over today's single-sampled word (BASE | COPY | SINGLE_BUFFER 1,
        // src/vivante/backend.rs:2035-2039): clear the mask, OR the bits.
        let base = 0x000e_4000 | 0xc;
        let single = base | (1 << 8);
        let four = (single & !PE_LOGIC_OP_4X_MASK) | pe_logic_op_4x(4);
        assert_eq!(four, base | 0x0500_0200);
        assert_eq!((four >> 8) & 3, 2);
    }

    /// The reload decision, by hand and as a real `MsaaStore` drives it:
    /// kept samples are used as they are, a whole clear loads nothing, and
    /// a new slot (first draw, eviction) or no slot at all broadcasts
    /// `rb.mem` (src/msaa_store.rs:23-30, :87-89, :138).
    #[test]
    fn sample_load_follows_the_store() {
        use crate::msaa_store::MsaaStore;
        assert_eq!(sample_load(Some(true), false), SampleLoad::Keep);
        assert_eq!(sample_load(Some(false), false), SampleLoad::Broadcast);
        assert_eq!(sample_load(None, false), SampleLoad::Broadcast);
        for s in [Some(true), Some(false), None] {
            assert_eq!(sample_load(s, true), SampleLoad::Cleared, "{s:?}: a whole clear writes every sample");
        }

        let size = sample_size(64, 64, 4);
        let bytes = scratch_bytes(64, 64, 4) as usize;
        let mut t: MsaaStore<usize> = MsaaStore::new();
        let mut freed = Vec::new();
        let state = |t: &mut MsaaStore<usize>, id: u64, freed: &mut Vec<usize>| {
            t.get_or_alloc(id, 1, size, |b| Some(b + id as usize), |h| freed.push(h)).map(|s| s.needs_load())
        };
        // First draw into id 5: a new slot, rb.mem broadcast.
        assert_eq!(sample_load(state(&mut t, 5, &mut freed), false), SampleLoad::Broadcast);
        // The job ends with the resolve and `mark_stored`: the next job keeps.
        t.get_or_alloc(5, 1, size, |_| None, |_| ()).unwrap().mark_stored();
        assert_eq!(sample_load(state(&mut t, 5, &mut freed), false), SampleLoad::Keep);
        // Two more attachments evict id 5 (MSAA_SLOTS 2): it comes back new.
        assert_eq!(sample_load(state(&mut t, 6, &mut freed), false), SampleLoad::Broadcast);
        assert_eq!(sample_load(state(&mut t, 7, &mut freed), false), SampleLoad::Broadcast);
        assert_eq!(freed, vec![bytes + 5], "id 5's scratch went to free");
        assert_eq!(sample_load(state(&mut t, 5, &mut freed), false), SampleLoad::Broadcast);
        // No memory: `None`, broadcast into a scratch kept for the job only.
        let none = t.get_or_alloc(9, 1, size, |_| None, |_| ()).map(|s| s.needs_load());
        assert_eq!(none, None);
        assert_eq!(sample_load(none, false), SampleLoad::Broadcast);
        // The window (id 0) is never a slot: its own rule, not this one.
        assert!(t.get_or_alloc(0, 1, size, Some, |_| ()).is_none());
    }

    /// No RS word upsamples: RS_CONFIG has only the two downsample bits
    /// (state_3d.xml.h:1354-1355), and the resolve word sets both.
    #[test]
    fn the_rs_cannot_upsample() {
        const { assert!(!RS_CAN_UPSAMPLE) };
        for fmt in [RS_FORMAT_A8R8G8B8, RS_FORMAT_X8R8G8B8, RS_FORMAT_R5G6B5] {
            let c = rs_config_resolve(fmt);
            assert_eq!(c & (RS_CONFIG_DOWNSAMPLE_X | RS_CONFIG_DOWNSAMPLE_Y), 0x60);
        }
    }

    /// GL_MULTI_SAMPLE_CONFIG of the reload: 4X (2) with the four enables
    /// (0xf0), the word src/vivante/backend.rs:1108 emits for every 4x draw
    /// (`GC_MSAA_SAMPLES_4X | msaa_enables(0xf)`, state.xml.h:318, :320-322).
    #[test]
    fn reload_multi_sample_config_is_the_4x_draws() {
        assert_eq!(GL_MULTI_SAMPLE_CONFIG_RELOAD, 0x0000_00f2);
        assert_eq!(GL_MULTI_SAMPLE_CONFIG_RELOAD & 0x3, 2, "MSAA_SAMPLES 4X");
        assert_eq!((GL_MULTI_SAMPLE_CONFIG_RELOAD >> 4) & 0xf, 0xf, "every sample enabled");
        assert_eq!(GL_MULTI_SAMPLE_CONFIG_RELOAD & 0x8, 0, "MSAA_SAMPLES_MASK (state.xml.h:319) clear");
    }

    /// The reload draw per stored surface: exactly the surfaces the RS
    /// resolves, each drawn in a surface whose RS format is the source's,
    /// so the next resolve reads the bytes the reload wrote; sRGB is drawn
    /// without encoding.
    #[test]
    fn reload_draw_per_surface() {
        assert_eq!(
            reload_draw(GLES_SURF_RGBA8888),
            Some(ReloadDraw { tex_format: GLES_TEXFMT_RGBA8, bgra: 1, target_format: GLES_SURF_RGBA8888, samples: 4 })
        );
        assert_eq!(
            reload_draw(GLES_SURF_SRGB8_A8),
            Some(ReloadDraw { tex_format: GLES_TEXFMT_RGBA8, bgra: 1, target_format: GLES_SURF_RGBA8888, samples: 4 })
        );
        assert!(!gles_surf_is_srgb(reload_draw(GLES_SURF_SRGB8_A8).unwrap().target_format), "no second encode");
        assert_eq!(
            reload_draw(GLES_SURF_XRGB8888),
            Some(ReloadDraw { tex_format: GLES_TEXFMT_RGBA8, bgra: 1, target_format: GLES_SURF_XRGB8888, samples: 4 })
        );
        assert_eq!(
            reload_draw(GLES_SURF_RGB565),
            Some(ReloadDraw { tex_format: GLES_TEXFMT_RGB565, bgra: 0, target_format: GLES_SURF_RGB565, samples: 4 })
        );
        for s in 0..=u8::MAX {
            let r = reload_draw(s);
            assert_eq!(r.is_some(), rs_resolve_format_of_surface(s).is_some(), "surface {s}");
            if let Some(r) = r {
                assert_eq!(
                    rs_resolve_format_of_surface(r.target_format),
                    rs_resolve_format_of_surface(s),
                    "surface {s}"
                );
                assert_eq!(gles_surf_bytes_per_pixel(r.target_format), gles_surf_bytes_per_pixel(s), "surface {s}");
                assert_eq!(u32::from(r.samples), SAMPLES_4X);
            }
        }
        assert_eq!(RELOAD_LINEAR_FILTER, 0, "nearest");
    }

    /// `mipgen.frag` samples at `gl_FragCoord.xy * u_scale`; with the reload
    /// scale every pixel centre falls inside its own texel, the one a
    /// nearest filter returns, in f32 as the shader computes it.
    #[test]
    fn reload_scale_hits_each_pixels_own_texel() {
        assert_eq!(reload_scale(0, 4), None);
        assert_eq!(reload_scale(4, 0), None);
        assert_eq!(reload_scale(64, 128), Some([1.0 / 64.0, 1.0 / 128.0]));
        for n in [1u32, 3, 16, 64, 100, 127, 128, 256, 1000, 2048] {
            let [s, _] = reload_scale(n, 1).unwrap();
            for x in 0..n {
                let u = (x as f32 + 0.5) * s;
                assert_eq!((u * n as f32).floor() as u32, x, "{n}: pixel {x}");
            }
        }
        assert_eq!(reload_viewport(128, 64), [0, 0, 128, 64]);
    }

    /// A broadcast reload then a resolve gives `rb.mem` back: four equal
    /// samples average to themselves, whether the RS box filter rounds or
    /// truncates (which one it does is not needed here). Per 8-bit channel
    /// and per 565 field.
    #[test]
    fn broadcast_then_resolve_is_the_identity() {
        let avg_trunc = |s: [u32; 4]| s.iter().sum::<u32>() / 4;
        let avg_round = |s: [u32; 4]| (s.iter().sum::<u32>() + 2) / 4;
        for v in 0..=255u32 {
            assert_eq!(avg_trunc([v; 4]), v);
            assert_eq!(avg_round([v; 4]), v);
        }
        for v in 0..=63u32 {
            assert_eq!(avg_trunc([v; 4]), v);
            assert_eq!(avg_round([v; 4]), v);
        }
    }
}
