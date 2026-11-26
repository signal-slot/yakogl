// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The Raspberry Pi 5's V3D 7.1 table, `yakogl_backend_v3d71`: the build's
//! `g_backend` when `gpu-v3d71` is on (src/backend.rs, `v3d_table`), in
//! place of the 4.2 table `yakogl_backend_v3d` (roadmap stage 5, P9;
//! docs/notes/v3d71-table71.md, docs/notes/v3d71-gl-table.md section 4).
//!
//! This is the skeleton (plan item `feat71`): the table's constant answers
//! are the ones docs/notes/v3d71-table71.md section 1 decides, and its hooks
//! record nothing yet. Every draw and every clear the GL core hands over is
//! refused by name ([`REF_NO_RECORDING`], -ENOTSUP, which the GL core raises
//! as `GL_INVALID_OPERATION` with the reason in the log). With no job ever
//! recorded, `flush`, `finish`, `finish_previous` and `wait_kick` have nothing
//! to do and succeed, `kick_id` is 0 (the "nothing submitted" kick) and
//! `kick_pending` is false. Item `table71-draw` replaces the draw with the
//! mapping of the design note's section 2 (`glue71` into `fill71`, then the
//! C builder `driver/src/v3d/v3d_job71.c`).
//!
//! The hooks the design note leaves `None` in the first cut (texture,
//! depth/stencil resolve and write, queries, mipmaps) are `None` here: the
//! GL core then refuses by name what needs them, or does it on the CPU.

use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use crate::backend::*;
use crate::util::ENOTSUP;

/// The table's name (`GL_RENDERER`'s backend half), as section 1 of
/// docs/notes/v3d71-table71.md names it.
pub const NAME: &core::ffi::CStr = c"Broadcom V3D 7.1 (Raspberry Pi 5)";

/// The refusal of every draw and clear until `table71-draw` records them:
/// the log line is `yakogl/v3d71: draw refused: ` followed by this.
pub const REF_NO_RECORDING: &str = "V3D 7.1: the table records no draw or clear yet (table71-draw)";

/// The log prefix of a refusal, as the 4.2 table's `yakogl/v3d: draw
/// refused: ` (src/v3d/backend.rs `count_refusal`).
pub const REFUSAL_PREFIX: &str = "yakogl/v3d71: draw refused: ";

static INITED: AtomicBool = AtomicBool::new(false);
/// The draws refused since init (the 4.2 table's `refused`).
static REFUSED: AtomicU32 = AtomicU32::new(0);

/// The draws and clears refused since init.
pub fn refused_draws() -> u32 {
    REFUSED.load(Ordering::Relaxed)
}

/// Whether [`v3d71_init`] ran and no [`v3d71_shutdown`] since.
pub fn inited() -> bool {
    INITED.load(Ordering::Relaxed)
}

/// Init: nothing to allocate while the table records nothing. Item
/// `table71-draw` adds the VPM query, the hub identity check
/// (`backend71::init`, F1 of the design note), the job memory and the clear
/// program.
pub extern "C" fn v3d71_init() -> i32 {
    if !INITED.swap(true, Ordering::Relaxed) {
        REFUSED.store(0, Ordering::Relaxed);
    }
    0
}

/// Shutdown: frees what init allocated (nothing yet).
pub extern "C" fn v3d71_shutdown() {
    INITED.store(false, Ordering::Relaxed);
}

/// The draw: refused by name ([`REF_NO_RECORDING`]), logged once per log
/// epoch and counted every time.
///
/// # Safety
/// `draw` is null or a valid `GlesDraw` (the table's contract); it is not
/// read.
pub unsafe extern "C" fn v3d71_draw(_draw: *const GlesDraw) -> i32 {
    REFUSED.fetch_add(1, Ordering::Relaxed);
    crate::glog_once!("{}{}", REFUSAL_PREFIX, REF_NO_RECORDING);
    -ENOTSUP
}

/// Flush: no job is ever open.
pub extern "C" fn v3d71_flush() -> i32 {
    0
}

/// Finish: no job is ever in flight.
pub extern "C" fn v3d71_finish() -> i32 {
    0
}

/// Finish the previous frame: none was submitted.
pub extern "C" fn v3d71_finish_previous() -> i32 {
    0
}

/// The kick holding the last draw: 0, nothing was submitted.
pub extern "C" fn v3d71_kick_id() -> u32 {
    0
}

/// Whether `kick_id` is still running: no kick ever is.
pub extern "C" fn v3d71_kick_pending(_kick_id: u32) -> bool {
    false
}

/// Wait for `kick_id`: there is nothing to wait for.
pub extern "C" fn v3d71_wait_kick(_kick_id: u32) -> i32 {
    0
}

/// The heaps' bases: 0. The 7.1 OSAL places every allocation in its one
/// carve-out and ignores the base (`src/osal/zephyr_v3d71.c`, `z_gpu_alloc`);
/// a base of 0 also keeps the owner tally from sorting blocks into heaps
/// (`src/osal/mod.rs`, `OwnerTally::add`).
pub extern "C" fn v3d71_heap_base(_heap: GlesHeap) -> u64 {
    0
}

/// The jobs submitted and the CPU waits since init: none of either yet.
///
/// # Safety
/// Both pointers are null or writable.
pub unsafe extern "C" fn v3d71_counters(kicks: *mut u32, waits: *mut u32) {
    // SAFETY: the caller's contract.
    unsafe {
        if let Some(k) = kicks.as_mut() {
            *k = 0;
        }
        if let Some(w) = waits.as_mut() {
            *w = 0;
        }
    }
}

/// The V3D 7.1 table (`g_backend` on a `gpu-v3d71` build).
#[no_mangle]
pub static yakogl_backend_v3d71: YakoglBackend = YakoglBackend {
    name: NAME.as_ptr(),
    /* fill71 takes float records only (REF_FLOAT_ATTRS); the GL core
     * converts the rest, as for 4.2 */
    native_attrib_formats: gles_attribf_bit(GLES_ATTRIBF_FLOAT32),
    /* rt71 keeps r_b_swap false: the RGBA8 store writes R, G, B, A */
    rt_rgba8888_is_bgra: false,
    /* the texture answers stay the 4.2 table's (section 1): right for P10 */
    mip_level_align: 0,
    compressed_blocks_in_rows: true,
    native_layered_textures: true,
    front_facing_shader_variant: false,
    etc1_decoded_at_upload: true,
    init: Some(v3d71_init),
    shutdown: Some(v3d71_shutdown),
    draw: Some(v3d71_draw),
    flush: Some(v3d71_flush),
    finish: Some(v3d71_finish),
    finish_previous: Some(v3d71_finish_previous),
    kick_id: Some(v3d71_kick_id),
    kick_pending: Some(v3d71_kick_pending),
    heap_base: Some(v3d71_heap_base),
    counters: Some(v3d71_counters),
    texture_released: None,
    surface_swapped: None,
    texture_uploaded: None,
    /* backend71 refuses instancing (REF_INSTANCED): no divisor above 1 */
    max_instance_divisor: 1,
    wait_kick: Some(v3d71_wait_kick),
    /* nothing is sampled yet: false makes the GL core wait as for a GPU
     * reader, the safe answer */
    samples_texel_copies: false,
    attachment_gone: None,
    depth_resolve: None,
    query_begin: None,
    query_end: None,
    query_result: None,
    generate_mipmap: None,
    return_mipmap_chain: None,
    linear_row_align: 0,
    /* fill71 takes the uniform blocks as host words */
    uniforms_host_only: true,
    ds_write: None,
    stencil_resolve: None,
    max_samples: 0,
};
