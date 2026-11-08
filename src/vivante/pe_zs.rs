// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! When the pixel engine may skip depth and stencil (`PE_DEPTH_CONFIG`
//! `DISABLE_ZS`), clears included, as a pure function
//! (docs/notes/gc-slow-terrain-stream-diff.md, 'The differences, ranked',
//! rank 9).
//!
//! Today `emit_pe_state` (backend.rs) treats every clear quad as depth tested
//! and leaves clears out of the `DISABLE_ZS` condition, so a clear into a
//! target with no depth or stencil attachment (terrain's noise, normal,
//! bloom_h, bloom_v and tilt_h passes) runs the PE depth stage against a
//! ds_cache scratch surface: `PE_DEPTH_CONFIG` 0x00040711 (D24S8, ALWAYS, no
//! write, no `DISABLE_ZS`). Mesa's draws there carry 0x01000700
//! (`DISABLE_ZS`). This file says what the PE needs; nothing calls it yet.
//! For every case with a depth or stencil attachment it gives exactly what
//! the inline code gives today, so it can replace that code byte for byte.
//! Core only, no crate imports: tests/gc_pe_zs.rs includes it by path.
//!
//! Mesa 26.2.1 (archive.mesa3d.org/mesa-26.2.1.tar.xz, the tarball whose
//! sha256 third_party/mesa-26.2.1/UPSTREAM records; the etnaviv driver
//! itself is not vendored), which this follows:
//! - src/gallium/drivers/etnaviv/etnaviv_zsa.c:48-49: `z_test_enabled =
//!   depth_enabled && depth_func != ALWAYS`, `z_write_enabled =
//!   depth_writemask`. A test with ALWAYS is no test.
//! - etnaviv_zsa.c:68-86: `stencil_enabled` only for an enabled stencil
//!   test whose function is not ALWAYS or whose operations are not all KEEP.
//! - etnaviv_state.c:1025-1034: a depth write or `stencil_enabled` needs the
//!   ZS stage (`early_zs` or `late_zs`); etnaviv_state.c:1036-1043: so does
//!   `z_test_enabled` (`early_z_test` or `late_z_test`).
//! - etnaviv_state.c:1045-1051, the word itself: `DEPTH_FUNC(z_test_enabled
//!   ? depth_func : ALWAYS) | COND(z_write_enabled, WRITE_ENABLE) | ... |
//!   COND(!late_zs && !late_z_test, DISABLE_ZS)`. With neither depth nor
//!   stencil used, `DISABLE_ZS` is set. (Mesa also sets it when the test and
//!   write moved to the RA, early Z; this backend runs depth late, so that
//!   case is not modelled.)
//! - No zsbuf: etnaviv_state.c:409 (`if (fb->zsbuf.texture != NULL)`) and
//!   :471-473, whose else gives `PE_DEPTH_CONFIG` `DEPTH_MODE_NONE`; and the
//!   state tracker never enables what the framebuffer lacks:
//!   src/mesa/state_tracker/st_atom_depth.c:86-91 (depth test and write
//!   only with `Visual.depthBits > 0`) and :100 (stencil only with
//!   `Visual.stencilBits > 0`). So a draw with no depth/stencil attachment
//!   has `DISABLE_ZS` whatever GL_DEPTH_TEST and GL_STENCIL_TEST say.
//! - Clears: etnaviv clears with the RS or BLT and only touches the zsbuf
//!   when there is one and depth/stencil is cleared (etnaviv_rs.c:538 and
//!   :576-578, etnaviv_blt.c:626); a colour clear never runs the PE depth stage.
//!   When it does clear by drawing (util_blitter), the depth state is
//!   src/gallium/auxiliary/util/u_blitter.c:246-269, bound at :1548-1558:
//!   a colour-only clear binds `dsa_keep_depth_stencil` (all zero: no
//!   depth, no stencil, so `DISABLE_ZS`), a depth clear
//!   `dsa_write_depth_keep_stencil` (depth enabled, write, ALWAYS), a stencil
//!   clear adds REPLACE stencil with ALWAYS.

/// What the GL asks of depth and stencil for one draw or clear quad.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ZsInput {
    /// The draw is a clear quad (`GcDraw::is_clear`).
    pub is_clear: bool,
    /// A clear that clears depth (`GcDraw::clear_depth`).
    pub clear_depth: bool,
    /// A clear that clears stencil (`GcDraw::clear_stencil`).
    pub clear_stencil: bool,
    /// GL_DEPTH_TEST is enabled.
    pub depth_test: bool,
    /// glDepthMask is true.
    pub depth_write: bool,
    /// GL_STENCIL_TEST is enabled.
    pub stencil_test: bool,
    /// The target has a depth attachment (or the default framebuffer has
    /// depth bits).
    pub has_depth_attachment: bool,
    /// The target has a stencil attachment (or stencil bits).
    pub has_stencil_attachment: bool,
}

/// The depth function the PE is given.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ZsFunc {
    /// ALWAYS (`GLES_CMP_ALWAYS`).
    Always,
    /// The GL's glDepthFunc.
    Gl,
}

/// What `PE_DEPTH_CONFIG` carries for depth and stencil.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ZsOutput {
    /// `DISABLE_ZS`: the PE depth/stencil stage is not needed at all.
    pub disable_zs: bool,
    /// The effective depth function.
    pub depth_func: ZsFunc,
    /// `WRITE_ENABLE`: depth is written.
    pub depth_write: bool,
}

/// The PE depth/stencil configuration for `z`.
pub fn pe_zs(z: ZsInput) -> ZsOutput {
    if !z.has_depth_attachment && !z.has_stencil_attachment {
        /* No zsbuf: nothing to test against or write (st_atom_depth.c:86,
         * :100; u_blitter.c:247 for a clear), so DISABLE_ZS
         * (etnaviv_state.c:1050-1051). A depth or stencil clear cannot
         * reach anything here either. */
        return ZsOutput { disable_zs: true, depth_func: ZsFunc::Always, depth_write: false };
    }
    /* With an attachment: today's backend.rs emit_pe_state, unchanged. A
     * clear counts as depth tested with ALWAYS and writes depth only when it
     * clears depth (u_blitter.c:250-252). A draw tests as GL asks and, GL
     * ES 2.0 4.1.5, writes only with the test on. */
    let depth_test = z.is_clear || z.depth_test;
    let depth_write = if z.is_clear { z.clear_depth } else { z.depth_test && z.depth_write };
    let depth_func = if z.is_clear || !z.depth_test { ZsFunc::Always } else { ZsFunc::Gl };
    let disable_zs = !depth_test && !depth_write && !z.stencil_test && !z.is_clear;
    ZsOutput { disable_zs, depth_func, depth_write }
}
