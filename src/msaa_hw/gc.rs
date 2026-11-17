// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The GC7000UL (i.MX8MP) numbers of a 4x multisample framebuffer object:
//! the scratch surface, the resolve engine's (RS) size rule and resolve
//! format, and the state words a 4x draw needs on this core (docs/notes/
//! es3-msaa-renderbuffers.md (c3), (f3), (f5)). Not wired: `gc-msaa-fbo`
//! calls them from `src/vivante`.
//!
//! What `gc-msaa-fbo` must do with these (note (c3), (f5), (e) open point 4):
//!
//! - **A scratch per attachment.** Today one `g().msaa` scratch serves every
//!   target and nothing reloads it (`ensure_msaa`, src/vivante/backend.rs:
//!   1046-1060). For an FBO the back end keeps a pool of scratch surfaces
//!   keyed by `(target_id, target_gen)` through [`crate::msaa_store`], each
//!   [`scratch`] in size, so a job that does not clear continues in its
//!   target's samples.
//! - **An RS resolve at every job end** (note (b)7): the scratch averaged
//!   into `rb.mem` with DOWNSAMPLE_X/Y in [`rs_resolve_format`], so the
//!   resolved image is current whatever the GL core reads next. A format
//!   with no RS resolve ([`rs_resolve_format`] `None`) or a size the RS
//!   cannot resolve ([`rs_resolvable`] false) is not drawn at 4x.
//! - **No draw on garbage.** A target with no scratch (first draw, or
//!   evicted) whose job does not open with a whole clear has no samples to
//!   start from, and the RS cannot upsample `rb.mem` into the scratch. Until
//!   a draw that fills the scratch from `rb.mem` is built, such a draw is
//!   refused (`-ENOTSUP`), open point 4.
//! - **Arm 19's state on every 4x draw** ([`ps_input_count_4x`],
//!   [`ps_temps_4x`], [`pe_logic_op_4x`]): the only measured arm under which
//!   the PE writes the scratch (docs/notes/gc-msaa-addresses.md, "The board
//!   A/B, run", 2026-10-15/16, `aed962e2`), today behind `gc_debug_msaa`
//!   bits 1 and 4.
//!
//! Mesa 26.2.1 etnaviv is cited from its source tree
//! (`src/gallium/drivers/etnaviv`, `src/etnaviv/hw`; etnaviv is not vendored
//! in third_party/mesa-26.2.1, the deps cache holds the release).

use crate::gl::consts::{
    GL_R16F, GL_R8, GL_RG16F, GL_RG8, GL_RGB10_A2, GL_RGB16F, GL_RGB565, GL_RGB5_A1, GL_RGB8, GL_RGBA16F, GL_RGBA4,
    GL_RGBA8, GL_SRGB8_ALPHA8,
};

/// The scratch surface is twice the target on each axis: 2x2 samples a pixel
/// (`ms_scale`, src/vivante/backend.rs:970-976; Mesa's `msaa_xscale` /
/// `msaa_yscale` of 2 for 4 samples, etnaviv_resource.c:327-328).
pub const MS_SCALE_4X: u32 = 2;

/// Rows and columns of the scratch are padded to a 64-pixel supertile
/// (`ms_pitch` and `ensure_msaa`, src/vivante/backend.rs:996-998, 1048).
pub const SUPERTILE: u32 = 64;

const fn align_up(v: u32, a: u32) -> Option<u32> {
    match v.checked_add(a - 1) {
        Some(s) => Some(s / a * a),
        None => None,
    }
}

/// The scratch surface of one 4x target.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Scratch {
    /// Row pitch in bytes (`PE_COLOR_STRIDE`; the RS source stride before
    /// its `<< 2` and tiling bits, src/vivante/backend.rs:1283).
    pub pitch: u32,
    /// Bytes of the allocation.
    pub bytes: u32,
}

/// The scratch of a `w` x `h` 4x target of `bpp` bytes a pixel:
/// pitch `align_up(2w, 64) * bpp` (`ms_pitch`, src/vivante/backend.rs:
/// 996-998) and size `pitch * align_up(2h, 64)` (`ensure_msaa`,
/// src/vivante/backend.rs:1048). The height is padded to a supertile as the
/// width is, as `ensure_msaa` allocates (note (c3) writes `* 2h`; the code
/// is the measured one, `gc-msaa-addresses.md`: `PE_COLOR_STRIDE` 0x200 for
/// 64x64). `None` on overflow.
pub const fn scratch(w: u32, h: u32, bpp: u32) -> Option<Scratch> {
    let Some(w2) = w.checked_mul(MS_SCALE_4X) else { return None };
    let Some(h2) = h.checked_mul(MS_SCALE_4X) else { return None };
    let Some(cols) = align_up(w2, SUPERTILE) else { return None };
    let Some(rows) = align_up(h2, SUPERTILE) else { return None };
    let Some(pitch) = cols.checked_mul(bpp) else { return None };
    let Some(bytes) = pitch.checked_mul(rows) else { return None };
    Some(Scratch { pitch, bytes })
}

/// Whether the RS can resolve a 4x target of this size: a width in multiples
/// of 16 and, in source samples, a height in multiples of 8, i.e. a target
/// height in multiples of 4 (`ms_resolvable`, src/vivante/backend.rs:
/// 965-967; etnaviv aborts on an RS width that is not a multiple of 16,
/// etnaviv_rs.c:87-92, `ETNA_RS_WIDTH_MASK` / `ETNA_RS_HEIGHT_MASK`
/// etnaviv_internal.h:55-57, times the sample scale, etnaviv_rs.c:811-812). Otherwise
/// today's back end draws single-sampled (src/vivante/backend.rs:3354-3360).
pub const fn rs_resolvable(w: u32, h: u32) -> bool {
    w & 15 == 0 && h & 3 == 0
}

/// The depth/stencil surface of a 4x draw, `ds_w` x `ds_h` the larger of the
/// GL storage and the target: twice each axis, as `select_in` is given
/// `ds_w * ms` and `ds_h * ms` (src/vivante/backend.rs:3383-3388). `None`
/// on overflow.
pub const fn depth_extent_4x(ds_w: u32, ds_h: u32) -> Option<(u32, u32)> {
    match (ds_w.checked_mul(MS_SCALE_4X), ds_h.checked_mul(MS_SCALE_4X)) {
        (Some(w), Some(h)) => Some((w, h)),
        _ => None,
    }
}

/// `RS_FORMAT` values (Mesa 26.2.1 src/etnaviv/hw/state_3d.xml.h:76-82; the
/// same values as `GC_RS_FORMAT_*`, src/vivante/regs.rs:1142-1145).
pub const RS_FORMAT_A4R4G4B4: u32 = 0x1; // state_3d.xml.h:77
pub const RS_FORMAT_A1R5G5B5: u32 = 0x3; // state_3d.xml.h:79
pub const RS_FORMAT_R5G6B5: u32 = 0x4; // state_3d.xml.h:80
pub const RS_FORMAT_A8R8G8B8: u32 = 0x6; // state_3d.xml.h:82

/// The RS format of the 4x resolve of a renderbuffer of `internalformat`
/// whose surface is `stored_bpp` bytes a pixel, or `None` when the GC has no
/// RS resolve for it (the back end then does not draw it at 4x). The RS
/// downsamples only in a format of `translate_rs_format`
/// (etnaviv_translate.h:278-313); the same-size fallback
/// `etna_compatible_rs_format` is refused when downsampling
/// (etnaviv_rs.c:740-747), and the RS takes one format for both sides
/// (etnaviv_rs.c:733-738).
///
/// - The RGBA8 group (note (b)2): every format of eight bits a channel or
///   fewer is stored RGBA8888 (`renderbuffer_surf_format`,
///   src/gl/framebuffer.rs:745-758), drawn `PE_FORMAT_A8R8G8B8`
///   (`pe_color_format`, src/vivante/backend.rs:349) and resolved
///   `A8R8G8B8` (`PIPE_FORMAT_B8G8R8A8_UNORM`, etnaviv_translate.h:306-309).
///   This is the one 4x case today.
/// - A 16-bit surface of RGB565 / RGBA4 / RGB5_A1 resolves in its own row:
///   `R5G6B5` (B5G6R5_UNORM, :300-301), `A4R4G4B4` (B4G4R4A4_UNORM,
///   :294-295), `A1R5G5B5` (B5G5R5A1_UNORM, :298-299). Today's
///   `ms_rs_format` (src/vivante/backend.rs:1260-1266) gives RGB5_A1
///   `A4R4G4B4`, which would average 4-bit fields of a 5-5-5-1 word (note
///   (c3)).
/// - SRGB8_ALPHA8 and RGB10_A2: `None`. Mesa has no RS resolve for
///   R8G8B8A8_SRGB or R10G10B10A2 and resolves them with a shader (note
///   (f3)); (f5) forbids handing them to the `A8R8G8B8` RS without a host test
///   in `gc-msaa-fbo` that pins the choice.
/// - R16F, RG16F, RGB16F, RGBA16F: `None`. They follow the single-sample
///   float rows (note (c3)): the RS has no float format (etnaviv_translate.h:
///   278-313), so the back end draws them single-sampled as today. Today's
///   `ms_rs_format` would give a 2-byte R16F `A4R4G4B4`.
pub const fn rs_resolve_format(internalformat: u32, stored_bpp: u32) -> Option<u32> {
    match (internalformat, stored_bpp) {
        (GL_RGBA8 | GL_RGB8 | GL_RG8 | GL_R8 | GL_RGB565 | GL_RGBA4 | GL_RGB5_A1, 4) => Some(RS_FORMAT_A8R8G8B8),
        (GL_RGB565, 2) => Some(RS_FORMAT_R5G6B5),
        (GL_RGBA4, 2) => Some(RS_FORMAT_A4R4G4B4),
        (GL_RGB5_A1, 2) => Some(RS_FORMAT_A1R5G5B5),
        (GL_SRGB8_ALPHA8 | GL_RGB10_A2, _) => None,
        (GL_R16F | GL_RG16F | GL_RGB16F | GL_RGBA16F, _) => None,
        _ => None,
    }
}

/// `PS_INPUT_COUNT_MSAA` / `PS_TEMP_REGISTER_CONTROL_MSAA`: a 4x draw has one
/// more fragment input and one more temporary than a single-sampled one
/// ("MSAA adds another input" / "another temp", etnaviv_shader.c:235-241).
pub const PS_EXTRA_INPUTS_4X: u32 = 1;
pub const PS_EXTRA_TEMPS_4X: u32 = 1;

/// The `COUNT` field of `PS_INPUT_COUNT` (bits 4:0, state_3d.xml.h:635-637)
/// for a 4x draw with `varyings` varyings: varyings + position + the 4x
/// input, `varyings + 2` (etnaviv_shader.c:236-238; yakogl's arm 19,
/// src/vivante/shader.rs:366, 388-391). The bits to OR with `UNK8`.
pub const fn ps_input_count_4x(varyings: u32) -> u32 {
    (varyings + 1 + PS_EXTRA_INPUTS_4X) & 0x1f
}

/// `PS_TEMP_REGISTER_CONTROL` `NUM_TEMPS` (bits 5:0, state_3d.xml.h:644-646)
/// for a 4x draw: `MAX2(temps + 1, varyings + 2)` (etnaviv_shader.c:240-241;
/// src/vivante/shader.rs:367).
pub const fn ps_temps_4x(temps: u32, varyings: u32) -> u32 {
    let a = temps + PS_EXTRA_TEMPS_4X;
    let b = varyings + 1 + PS_EXTRA_TEMPS_4X;
    (if a > b { a } else { b }) & 0x3f
}

/// `PE_LOGIC_OP` `UNK24` (bits 26:24, state_3d.xml.h:1144-1146) = 5 for a 4x
/// target on a SMALL_MSAA core (etnaviv_state.c:524-525; the GC7000UL has
/// `REG_SmallMSAA` 1, gc-msaa-addresses.md section 2). Its write mask, bit
/// 27, stays clear (`pe_logic_op_unk24`, src/vivante/regs.rs:553-557).
pub const PE_LOGIC_OP_UNK24_4X: u32 = 5 << 24;

/// `PE_LOGIC_OP` `SINGLE_BUFFER` (bits 9:8, state_3d.xml.h:1135-1137) for the
/// supertiled 4x scratch on this SINGLE_BUFFER core: 2, or 3 for a 16-bit
/// target (etnaviv_state.c:536-539; `single_buffer`,
/// src/vivante/backend.rs:812-825). Arm 19 measured 2 on RGBA8.
pub const fn pe_logic_op_single_buffer_4x(bpp: u32) -> u32 {
    (if bpp <= 2 { 3 } else { 2 }) << 8
}

/// The `PE_LOGIC_OP` bits a 4x draw ORs in: `UNK24` 5 and `SINGLE_BUFFER` 2
/// (3 at 16 bits), in place of the linear `SINGLE_BUFFER` 1. Arm 19's whole
/// word on RGBA8 is `0x050e420c` (src/vivante/backend_tests.rs:4871).
pub const fn pe_logic_op_4x(bpp: u32) -> u32 {
    PE_LOGIC_OP_UNK24_4X | pe_logic_op_single_buffer_4x(bpp)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scratch_matches_today_s_ms_pitch_and_ensure_msaa() {
        // By hand from ms_pitch = align_up(2w, 64) * bpp and ensure_msaa's
        // bytes = ms_pitch * align_up(2h, 64) (src/vivante/backend.rs:996-998,
        // 1048).
        // gl_test case 16, 64x64 RGBA8: 128 * 4 = 512 (PE_COLOR_STRIDE 0x200,
        // gc-msaa-addresses.md) and 512 * 128 = 64 KiB (gc-msaa-record.md).
        assert_eq!(scratch(64, 64, 4), Some(Scratch { pitch: 512, bytes: 65536 }));
        // 800 wide: 1600 is a multiple of 64 (backend_tests.rs:1296); 2*480=960.
        assert_eq!(scratch(800, 480, 4), Some(Scratch { pitch: 6400, bytes: 6400 * 960 }));
        // 100x30 at 2 bytes: 200 -> 256, * 2 = 512; 60 -> 64.
        assert_eq!(scratch(100, 30, 2), Some(Scratch { pitch: 512, bytes: 512 * 64 }));
        // 16x4: 32 -> 64, * 4 = 256; 8 -> 64.
        assert_eq!(scratch(16, 4, 4), Some(Scratch { pitch: 256, bytes: 16384 }));
        // RGBA16F, 128x128: 256 * 8 = 2048; 2048 * 256.
        assert_eq!(scratch(128, 128, 8), Some(Scratch { pitch: 2048, bytes: 524288 }));
        assert_eq!(scratch(u32::MAX, 4, 4), None);
        assert_eq!(scratch(1 << 20, 1 << 20, 4), None);
    }

    #[test]
    fn rs_size_rule_and_depth() {
        assert!(rs_resolvable(64, 64));
        assert!(rs_resolvable(16, 4));
        assert!(!rs_resolvable(8, 8), "width not a multiple of 16");
        assert!(!rs_resolvable(16, 6), "height not a multiple of 4");
        assert!(!rs_resolvable(127, 128));
        assert_eq!(depth_extent_4x(64, 32), Some((128, 64)));
        assert_eq!(depth_extent_4x(u32::MAX, 1), None);
    }

    #[test]
    fn rs_format_values_pin_mesa() {
        // state_3d.xml.h:77, :79, :80, :82.
        assert_eq!([RS_FORMAT_A4R4G4B4, RS_FORMAT_A1R5G5B5, RS_FORMAT_R5G6B5, RS_FORMAT_A8R8G8B8], [1, 3, 4, 6]);
    }

    #[test]
    fn rs_resolve_format_per_internalformat() {
        for f in [GL_RGBA8, GL_RGB8, GL_RG8, GL_R8, GL_RGB565, GL_RGBA4, GL_RGB5_A1] {
            assert_eq!(rs_resolve_format(f, 4), Some(RS_FORMAT_A8R8G8B8), "{f:#x} stored RGBA8888");
        }
        assert_eq!(rs_resolve_format(GL_RGB565, 2), Some(RS_FORMAT_R5G6B5));
        assert_eq!(rs_resolve_format(GL_RGBA4, 2), Some(RS_FORMAT_A4R4G4B4));
        // RGB5_A1 is a 5-5-5-1 word: Mesa resolves B5G5R5A1_UNORM as
        // A1R5G5B5 (etnaviv_translate.h:298-299). Today's ms_rs_format gives
        // every 2-byte target but RGB565 A4R4G4B4, which averages it as four
        // 4-bit fields: red's low bit lands in green, and so on (note (c3)).
        assert_eq!(rs_resolve_format(GL_RGB5_A1, 2), Some(RS_FORMAT_A1R5G5B5));
        assert_ne!(rs_resolve_format(GL_RGB5_A1, 2), Some(RS_FORMAT_A4R4G4B4));
        // A layout the format is not stored in.
        assert_eq!(rs_resolve_format(GL_RGBA8, 2), None);
        // Note (f3), (f5): no RS resolve in Mesa; gc-msaa-fbo decides.
        assert_eq!(rs_resolve_format(GL_SRGB8_ALPHA8, 4), None);
        assert_eq!(rs_resolve_format(GL_RGB10_A2, 4), None);
        // No float RS format: single-sampled, as the float rows are today.
        assert_eq!(rs_resolve_format(GL_R16F, 2), None);
        assert_eq!(rs_resolve_format(GL_RGBA16F, 8), None);
        assert_eq!(rs_resolve_format(GL_RG16F, 8), None);
        assert_eq!(rs_resolve_format(GL_RGB16F, 8), None);
    }

    #[test]
    fn arm_19_ps_counts() {
        // One varying: single-sampled 2 inputs and MAX2(temps, 2) temps
        // (src/vivante/backend_tests.rs:1784-1787); 4x one more of each.
        assert_eq!(ps_input_count_4x(1), 3);
        assert_eq!(ps_temps_4x(0, 1), 3);
        assert_eq!(ps_temps_4x(5, 1), 6);
        assert_eq!(ps_input_count_4x(0), 2);
        assert_eq!(ps_input_count_4x(31) & !0x1f, 0, "COUNT is five bits");
    }

    #[test]
    fn arm_19_pe_logic_op() {
        // GC_PE_LOGIC_OP_BASE | pe_logic_op_op(GC_LOGIC_OP_COPY)
        // (src/vivante/regs.rs:566-567), the 4x word measured on the board.
        let base = 0x000e_4000 | 0xc;
        assert_eq!(base | pe_logic_op_4x(4), 0x050e_420c, "backend_tests.rs:4871");
        assert_eq!(base | pe_logic_op_single_buffer_4x(2), 0x000e_430c, "backend_tests.rs:4879");
        assert_eq!(PE_LOGIC_OP_UNK24_4X & 0x0800_0000, 0, "UNK24's mask bit stays clear");
        assert_eq!(pe_logic_op_4x(8), (5 << 24) | (2 << 8));
    }
}
