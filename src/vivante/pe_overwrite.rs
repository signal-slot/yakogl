// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GC7000 (i.MX8MP): the PE colour COMPONENTS mask and OVERWRITE bit of a
//! draw, as a pure function (docs/notes/gc-slow-terrain-stream-diff.md,
//! rank 8).
//!
//! Self-contained on purpose: it uses `core` only, so a host test can include
//! it with `#[path]`. `emit_pe_state` (backend.rs) takes RT0's and the other
//! targets' OVERWRITE from it (tests/gc_pe_wire_stream.rs).
//!
//! What Mesa 26.2.1 etnaviv does (`src/gallium/drivers/etnaviv/`):
//!
//! - `etnaviv_blend.c:97`: `fo_allowed = !alpha_enable && !logicop_enable`;
//! - `etnaviv_blend.c:145-151`: `full_overwrite = fo_allowed &&
//!   util_format_colormask_full(desc, colormask)`; COMPONENTS is the pipe
//!   colour mask as given (`:166-169`);
//! - `util/format/u_format.h:751-786`: `util_format_colormask` counts only the
//!   channels whose swizzle is below 4, so the X byte of B8G8R8X8 is not a
//!   channel and a mask of 0x7 is "full" for it;
//! - `etnaviv_state.c:291-293`: OVERWRITE is offered to the blend state only
//!   for a single-sample target; `:316-318` drops it for TS compression on
//!   chips without v4 compression (no TS here);
//! - `etnaviv_emit.c:536-545`: the emitted word is the blend state's bits
//!   masked by the framebuffer's.
//!
//! What Mesa relies on for the alpha byte of such a target: nothing reads
//! it. `etnaviv_format.c:165` renders B8G8R8X8 as PE X8R8G8B8, and
//! `etnaviv_format.c:359-360` makes every sampler of a format without alpha
//! read alpha as `PIPE_SWIZZLE_1`, so a GL_RGB texture reads 1 whatever the
//! PE stored in the X byte. A scanout (X8R8G8B8 window) has the display
//! ignore the byte.
//!
//! Here only the window has that property. An FBO texture of GL_RGB is
//! stored as RGBA8 and sampled as RGBA8 with no swizzle, so its alpha byte
//! has to stay what the clear wrote (1): a draw must not write it, and
//! OVERWRITE (a whole-pixel write) is not allowed. That keeps today's answer
//! (COMPONENTS `mask & 7`, no OVERWRITE) for it; dEQP fbo.render.depth.
//! tex2d_rgb_* is the case. A RGBA target keeps today's answer too.

/// Where the draw lands.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PeTarget {
    /// The window: X8R8G8B8 scanout, alpha ignored by the display.
    Window,
    /// A texture or renderbuffer, stored as RGBA8 and sampled as such.
    Fbo,
}

/// The PE colour word's two fields.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct PeColor {
    /// `PE_COLOR_FORMAT.COMPONENTS`, 4 bits, R = bit 0 .. A = bit 3.
    pub components: u32,
    /// `PE_COLOR_FORMAT.OVERWRITE` (and the per-RT `PE_HALTI5_RT_COLORMASK`
    /// one).
    pub overwrite: bool,
}

/// `gl_mask`: the GL colour mask for this render target, bit 0 = R .. bit 3 =
/// A. `blend`: blending is on after the ONE/ZERO/ADD reduction. `alpha_bits`:
/// the target's alpha bits as the GL core reports them. `msaa`: the target is
/// multisampled. A draw, not a clear: a clear picks its own mask.
pub fn pe_color(gl_mask: u32, blend: bool, alpha_bits: u32, target: PeTarget, msaa: bool) -> PeColor {
    let mask = gl_mask & 0xf;
    if alpha_bits != 0 {
        // today: the mask as given; OVERWRITE for a full mask, no blend, no MSAA
        return PeColor { components: mask, overwrite: !blend && mask == 0xf && !msaa };
    }
    let rgb = mask & 0x7;
    match target {
        PeTarget::Window => {
            // the X byte is not a channel (u_format.h:751): a full RGB mask is
            // a full mask, and the PE may write the byte too
            let overwrite = rgb == 0x7 && !blend && !msaa;
            PeColor { components: if overwrite { 0xf } else { rgb }, overwrite }
        }
        // alpha stays as the clear wrote it: never written, never overwritten
        PeTarget::Fbo => PeColor { components: rgb, overwrite: false },
    }
}
