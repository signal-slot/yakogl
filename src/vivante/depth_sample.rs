// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The depth-sampling experiment (docs/notes/gc-depth-sample.md, case
//! `dtpu`; plan #328, #1204): can the texture unit read the row-linear D24S8
//! surface the PE wrote, without the CPU resolve (`ds_cache::resolve_depth`)?
//!
//! A test knob selects it, `gc_debug_set_depth_tpu(mode)`, and only gl_test
//! sets it. Mode 0, the default and the only mode without `test-knobs`, is
//! today's path: [`gc_depth_sampled`] answers no, the GL core resolves the
//! depth into the R32F texels on the CPU, and nothing here runs. With mode 1
//! or 2 [`gc_depth_sampled`] answers yes for a Live, single-sampled depth
//! plane that covers the texture, the GL core puts the id into the binding
//! (`GlesTextureBind::depth_id`), and `texture::emit_slot` samples the surface
//! through the binding [`view_binding`] makes:
//!
//! - mode 1, route 2 of the note (Mesa's): `TEXTURE_FORMAT_D24X8` (0x11),
//!   swizzle (R, R, R, 1), no INT_FILTER (Mesa F:320);
//! - mode 2, the raw view: A8R8G8B8 with the identity swizzle and INT_FILTER,
//!   so the texel is (D1, D0, S, D2) of the word `D2 D1 D0 S`.
//!
//! Both are LINEAR, one level, LOD 0 at the surface past the guard, and
//! `LINEAR_STRIDE` the surface's row stride; mode 2 is NEAREST. A binding whose
//! texture does not fit inside the live surface (or whose surface went away
//! between the answer and the draw) fails the draw with -ENOTSUP, by name: the
//! texels were left stale on the strength of the answer, so sampling them
//! would be wrong. This is not a product path: the release archive has no knob
//! (tools/check.sh, "no test knob in the artefact"), and the wiring for one is
//! docs/notes/gc-depth-sample-wire.md.

use crate::backend::{GlesTextureBind, GLES_TEXFMT_RGBA8, GLES_TEXTYPE_2D};
use crate::util::ENOTSUP;

/// `TEXTURE_FORMAT_D24X8` (Mesa `common_3d.xml.h`, X:60; etnaviv_format.c
/// F:179-180 for X8Z24_UNORM and S8_UINT_Z24_UNORM): a CONFIG0 format.
pub const GC_TEX_FORMAT_D24X8: u32 = 0x11;

/// A backend-private texel class: a binding that reads a depth surface in
/// place as D24X8 ([`view_binding`] mode 1). Never a `GLES_TEXFMT_*`.
pub const GC_TEXFMT_DEPTH_VIEW: u8 = 0xf5;

/// Knob modes: 0 the CPU resolve, 1 D24X8, 2 the raw A8R8G8B8 view.
pub const DEPTH_TPU_OFF: i32 = 0;
pub const DEPTH_TPU_D24X8: i32 = 1;
pub const DEPTH_TPU_RAW8888: i32 = 2;

/// The experiment's mode ([`mode`]): 0 unless gl_test set it.
#[cfg(feature = "test-knobs")]
#[no_mangle]
pub static mut gc_debug_depth_tpu: i32 = DEPTH_TPU_OFF;

/// Set [`gc_debug_depth_tpu`]; returns the previous value (gl_test's `dtpu`
/// runs each arm and puts it back).
#[cfg(feature = "test-knobs")]
#[no_mangle]
pub extern "C" fn gc_debug_set_depth_tpu(mode: i32) -> i32 {
    // SAFETY: single-threaded by contract; plain loads and stores of an int.
    unsafe { core::ptr::addr_of_mut!(gc_debug_depth_tpu).replace(mode) }
}

/// The experiment's mode: [`DEPTH_TPU_OFF`] in every build without
/// `test-knobs`, and in one that has not set the knob.
pub fn mode() -> i32 {
    #[cfg(feature = "test-knobs")]
    {
        // SAFETY: single-threaded by contract; a plain read of an int only
        // the setter writes.
        unsafe { core::ptr::addr_of!(gc_debug_depth_tpu).read() }
    }
    #[cfg(not(feature = "test-knobs"))]
    {
        DEPTH_TPU_OFF
    }
}

/// Where a depth plane lies: the GPU address of its first word (past the
/// guard), the surface's row stride in bytes and its rows (both 64-pixel
/// aligned, `ds_cache::Surface`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct DepthView {
    pub gpu: u64,
    pub stride: u32,
    pub rows: u32,
}

impl DepthView {
    /// Does a `w` x `h` texture sampled from it lie inside the surface?
    pub fn covers(&self, w: u32, h: u32) -> bool {
        w != 0 && h != 0 && u64::from(w) * 4 <= u64::from(self.stride) && h <= self.rows
    }
}

/// The live depth plane of GL object `id`; `kick` != 0 keeps the surface
/// until that kick completed.
fn view(id: u32, kick: u32) -> Option<DepthView> {
    let (gpu, stride, rows) = super::ds_cache::depth_view(id, super::backend::GC_DEPTH_GUARD_BYTES, kick)?;
    Some(DepthView { gpu, stride, rows })
}

/// `crate::backend::samples_rendered_depth`'s GC answer: no in mode 0 (every
/// product build); otherwise yes when `id`'s depth plane is Live here,
/// single-sampled, and covers `w` x `h`. No wait and no CPU access.
pub fn gc_depth_sampled(id: u32, w: u32, h: u32) -> bool {
    mode() != DEPTH_TPU_OFF && id != 0 && view(id, 0).is_some_and(|v| v.covers(w, h))
}

/// The binding `emit_slot` describes instead of `t` (whose texels are
/// stale) in `mode`, for the surface `v`. -ENOTSUP when there is no surface,
/// the texture does not fit inside it, or the mode is not 1 or 2.
pub fn view_binding(t: &GlesTextureBind, mode: i32, v: Option<DepthView>) -> Result<GlesTextureBind, i32> {
    let Some(v) = v.filter(|v| v.covers(t.width, t.height)) else {
        return Err(-ENOTSUP);
    };
    /* one 2D level at the surface: LINEAR (no `cpu`, so no tiled copy),
     * LINEAR_STRIDE = stride_texels * 4 = the surface's stride */
    let at = GlesTextureBind {
        type_: GLES_TEXTYPE_2D,
        layers: 1,
        base_gpu: v.gpu,
        stride_texels: v.stride / 4,
        bytes: v.stride * v.rows,
        mip_filter: 0,
        mip_levels: 1,
        bgra: 0,
        depth_id: 0,
        cpu: core::ptr::null(),
        swizzle: [0; 4],
        ..*t
    };
    match mode {
        /* (d, d, d, 1): R in G and B (`depth_luminance`), A 1 */
        DEPTH_TPU_D24X8 => Ok(GlesTextureBind { format: GC_TEXFMT_DEPTH_VIEW, depth_luminance: 1, no_alpha: 1, ..at }),
        /* the word's bytes as they are: NEAREST (a filter across bytes is
         * not a filter of the depth), identity swizzle */
        DEPTH_TPU_RAW8888 => Ok(GlesTextureBind {
            format: GLES_TEXFMT_RGBA8,
            bgra: 1,
            depth_luminance: 0,
            no_alpha: 0,
            linear_filter: 0,
            min_linear: 0,
            ..at
        }),
        _ => Err(-ENOTSUP),
    }
}

/// [`view_binding`] for a binding with `depth_id != 0` in the current mode,
/// keeping the surface for `kick`. A refusal is logged once.
pub fn binding(t: &GlesTextureBind, kick: u32) -> Result<GlesTextureBind, i32> {
    let mode = mode();
    let v = view(t.depth_id, kick);
    let b = view_binding(t, mode, v);
    match (&b, v) {
        (Ok(_), Some(v)) => crate::glog_once!(
            "vivante: dtpu mode {mode}: a {}x{} depth texture sampled from its depth surface (stride {})",
            t.width,
            t.height,
            v.stride
        ),
        _ => crate::glog_once!(
            "vivante: dtpu mode {mode}: a {}x{} depth texture does not fit a live depth surface; the draw is refused",
            t.width,
            t.height
        ),
    }
    b
}
