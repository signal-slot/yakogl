// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The texture unit asked to sample the D24S8 depth surface in place: an
//! experiment, off for every GL use (docs/notes/gc-depth-sample.md, "The
//! board experiment that decides the verdict").
//!
//! Today a draw that samples a depth texture a kick rendered reads R32F
//! texels the CPU resolved from the surface (`ds_cache::resolve_depth`,
//! behind a `finish`; glmark2's shadow and refract, once a frame). Mesa's
//! etnaviv samples such a texture in place, as `TEXTURE_FORMAT_D24X8`, with
//! no copy; whether this part's TPU reads the back end's row-linear surface
//! that way is the question gl_test 222 (`dtpu`) puts to the board. The
//! case turns one switch, [`set_view`] (the test knob
//! `gc_debug_set_depth_tpu`, `test-knobs` builds only), and with it:
//!
//! - [`gc_depth_sampled`], the GC7000's arm of
//!   `backend::samples_rendered_depth`, answers yes for a depth object whose
//!   plane is Live in a surface at scale 1 that covers the texture; the GL
//!   core then leaves the texels stale and gives the draw's binding
//!   `GlesTextureBind::depth_id`;
//! - [`descriptor`] gives `texture::build_descriptor` the words of that
//!   binding's descriptor: the surface's first pixel word as LOD 0, its
//!   row stride, LINEAR addressing, one level, and the view's format and
//!   swizzle. [`View::D24X8`] is Mesa's route: depth format 0x11, the depth
//!   in X, (d, d, d, 1) under GL_OES_depth_texture or (d, 0, 0, 1) on ES
//!   3.0, `INT_FILTER` off (etnaviv_format.c texture_use_int_filter is
//!   false for a depth format). [`View::Raw8888`] is the case's second arm:
//!   A8R8G8B8 with the identity swizzle, so the word's four bytes arrive as
//!   (depth 23..16, depth 15..8, stencil, depth 31..24), `INT_FILTER` on.
//!
//! With the switch at its default ([`View::Off`], every build) the answer
//! is no, no binding carries a `depth_id`, and nothing here is reached. A
//! product build has neither the knob nor its storage (`tools/check.sh`,
//! "no test knob in the artefact"). Nothing here submits or waits: the
//! answer is a reading of the depth cache, and the sampling draw is ordered
//! after the kick that rendered the depth in the same ring.

use super::backend::{g, GC_DEPTH_GUARD_BYTES};
use super::ds_cache;
use super::regs::*;
use crate::backend::GlesTextureBind;
use crate::util::ENOTSUP;

/// How a binding with a `depth_id` is sampled.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum View {
    /// The CPU resolve, as every GL use has it: the answer is no.
    Off,
    /// The surface as Mesa's `TEXTURE_FORMAT_D24X8` (knob 1).
    D24X8,
    /// The surface's words as A8R8G8B8 texels, identity swizzle (knob 2).
    Raw8888,
}

#[cfg(any(feature = "test-knobs", feature = "host-test"))]
static MODE: core::sync::atomic::AtomicI32 = core::sync::atomic::AtomicI32::new(0);

/// Set the view: 0 [`View::Off`], 1 [`View::D24X8`], 2 [`View::Raw8888`]
/// (anything else: off). Returns the previous value. The host test's
/// switch; an image sets it through the knob below.
#[cfg(any(feature = "test-knobs", feature = "host-test"))]
pub fn set_view(mode: i32) -> i32 {
    MODE.swap(mode, core::sync::atomic::Ordering::Relaxed)
}

/// The view in force: [`View::Off`] in every build without the switch.
pub fn view() -> View {
    #[cfg(any(feature = "test-knobs", feature = "host-test"))]
    {
        match MODE.load(core::sync::atomic::Ordering::Relaxed) {
            1 => View::D24X8,
            2 => View::Raw8888,
            _ => View::Off,
        }
    }
    #[cfg(not(any(feature = "test-knobs", feature = "host-test")))]
    {
        View::Off
    }
}

/// The knob gl_test 222 turns: [`set_view`] for C, in the pattern of
/// `gc_debug_set_narrow_la` (returns the previous value). `test-knobs`
/// builds only.
#[cfg(feature = "test-knobs")]
#[no_mangle]
pub extern "C" fn gc_debug_set_depth_tpu(mode: i32) -> i32 {
    set_view(mode)
}

/// The knob's value, for a log line.
#[cfg(feature = "test-knobs")]
#[no_mangle]
pub extern "C" fn gc_debug_depth_tpu() -> i32 {
    MODE.load(core::sync::atomic::Ordering::Relaxed)
}

/// The surface the texture unit reads for depth object `id` (the
/// `GlesDraw::depth_id` form) of a `w` x `h` texture: the GPU address of
/// its first pixel word (past the guard, where `resolve_depth` reads) and
/// its row stride in bytes. `None` with the view off, before `gc_init`, and
/// when no surface holds the object's depth plane Live at scale 1 (a plane
/// that is a whole clear's value only has no words behind it; a
/// multisampled plane is not this layout) or the surface is smaller than
/// the texture. A reading of the cache: no submit, no wait.
pub fn surface(id: u32, w: u32, h: u32) -> Option<(u64, u32)> {
    if view() == View::Off || !g().inited || id == 0 || w == 0 || h == 0 {
        return None;
    }
    let (gpu, stride, rows) = ds_cache::live_depth_surface(id, GC_DEPTH_GUARD_BYTES)?;
    (w.checked_mul(4).is_some_and(|row| row <= stride) && h <= rows).then_some((gpu, stride))
}

/// `backend::samples_rendered_depth`'s GC7000 arm: may the draw sample the
/// depth of object `id`, `w` x `h`, from the surface itself? Yes only under
/// the switch, for a surface [`surface`] finds.
pub fn gc_depth_sampled(id: u32, w: u32, h: u32) -> bool {
    surface(id, w, h).is_some()
}

/// What `build_descriptor` writes for a binding that samples the surface.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct DepthView {
    /// TEXDESC_CONFIG0, CONFIG1 and CONFIG2.
    pub words: [u32; 3],
    /// LOD 0: the surface's first pixel word.
    pub lod0: u64,
    /// LINEAR_STRIDE: the surface's row stride in bytes.
    pub stride: u32,
    /// SAMP_CTRL0 INT_FILTER for the view's format.
    pub int_filter: bool,
}

/// The descriptor of binding `t` when it samples rendered depth
/// (`depth_id` set by the GL core on the strength of [`gc_depth_sampled`]):
/// `Ok(None)` for every other binding, whose texels are the texture.
/// `Err(-ENOTSUP)` when the surface it was promised is gone, or the switch
/// was turned off in between: the GL core left the texels stale on the
/// strength of the answer, so the draw must fail by name (`emit_slot` logs
/// it) rather than sample them.
pub fn descriptor(t: &GlesTextureBind) -> Result<Option<DepthView>, i32> {
    if t.depth_id == 0 {
        return Ok(None);
    }
    let Some((lod0, stride)) = surface(t.depth_id, t.width, t.height) else {
        return Err(-ENOTSUP);
    };
    /* GL_OES_depth_texture samples (d, d, d, 1); ES 3.0 "treated as RED",
     * (d, 0, 0, 1) (`GlesTextureBind::depth_luminance`). The D24X8 texel's
     * X is the depth; what the TPU puts in Y, Z and W is not known
     * (docs/notes/gc-depth-sample.md (a) 6), so neither is read. */
    let lum = t.depth_luminance != 0;
    let dd = if lum { GC_SWIZZLE_R } else { GC_SWIZZLE_0 };
    let (format, swizzle, int_filter) = match view() {
        View::Raw8888 => (GC_TEX_FORMAT_A8R8G8B8, [GC_SWIZZLE_R, GC_SWIZZLE_G, GC_SWIZZLE_B, GC_SWIZZLE_A], true),
        _ => (GC_TEX_FORMAT_D24X8, [GC_SWIZZLE_R, dd, dd, GC_SWIZZLE_1], false),
    };
    let config0 =
        texdesc_type(GC_TEX_TYPE_2D) | texdesc_format(format) | texdesc_addressing_mode(GC_TEX_ADDRESSING_LINEAR);
    let config1 = texdesc_format_ext(0)
        | texdesc_swizzle_r(swizzle[0])
        | texdesc_swizzle_g(swizzle[1])
        | texdesc_swizzle_b(swizzle[2])
        | texdesc_swizzle_a(swizzle[3])
        | texdesc_halign(0);
    Ok(Some(DepthView { words: [config0, config1, GC_TEXDESC_CONFIG2_BASE], lod0, stride, int_filter }))
}
