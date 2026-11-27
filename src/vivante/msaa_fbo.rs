// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The samples of 4x framebuffer objects across jobs: one supertiled scratch
//! surface per live 4x colour target, kept in a [`MsaaStore`]
//! (docs/notes/es3-msaa-renderbuffers.md (b)5-7, (c3); src/msaa_hw/gc.rs).
//!
//! The window's 4x path is not this: it keeps the one `g().msaa` scratch of
//! `backend.rs` (`ensure_msaa`), behind the `gc_debug_msaa` knob as before.
//! This store serves the draws [`super::backend`] calls FBO draws: a draw
//! with `samples > 1` that is not mirrored (`flip_y` false: a framebuffer
//! object, never the window), in a `msaa-fbo` build ([`ON`]).
//!
//! **Keying.** A slot is keyed on the draw's colour target address,
//! `GlesDraw::target_gpu`, at generation 0, as the merged PowerVR back end
//! keys its sample store (`src/pvr/backend.rs` `MsStore`): `GlesDraw` has no
//! colour attachment identity yet (`target_id` / `target_gen`, note (b)5, are
//! another item's). The store's size check (`SampleSize`: extent and bytes a
//! pixel) runs on every lookup, and the stored surface format is checked
//! here: a slot whose target was specified again with another extent or
//! format is freed and allocated afresh. A same-size respecification keeps
//! the address only after the GL core freed the old storage, and that free
//! reaches [`released`] first (`YakoglBackend::texture_released`, called
//! before the address can be handed out again, gl/context.rs
//! `release_storage_with`), which drops the slot: no stale sample survives.
//!
//! **Validity.** A slot is loadable (`Slot::needs_load`) once a job that drew
//! into it was submitted with its RS resolve ([`stored`]). Anything that
//! writes the target without its samples makes it not loadable
//! ([`touched`]): a single-sampled draw (a 4x FBO drawn 1x because
//! [`unsupported`] says so, or a 1x draw into the same address) and a CPU free of
//! the storage ([`released`]). The next job into a not-loadable slot starts
//! from the resolved image in every sample (`msaa_hw::gc::SampleLoad::
//! Broadcast`, the reload draw `backend.rs` builds), unless it begins with a
//! whole clear.
//!
//! **Memory.** Each slot's scratch is `msaa_hw::gc::scratch_bytes(w, h,
//! bpp)`, the padded supertiled surface, not the packed size the store asks
//! for (tests/msaa_hw_gc_api.rs, the handoff). Freed scratches go to
//! `osal::gpu_free`, which holds them until the kicks that may read them
//! have completed.

use crate::backend::{gles_surf_bytes_per_pixel, GlesDraw};
use crate::msaa_hw::gc as hw;
use crate::msaa_store::{MsaaStore, MSAA_SLOTS};
use crate::osal::{self, OsalGpuMem, OSAL_MEM_DATA};
use crate::util::Global;

/// 4x framebuffer objects are drawn with their samples kept: the `msaa-fbo`
/// feature (note (a): off until the back end's host test passes and the
/// board has run it). Without it no FBO is ever 4x here (`max_samples` 0).
pub const ON: bool = cfg!(feature = "msaa-fbo");

/// One slot's scratch: the allocation and the surface format it was made for.
pub struct Scratch {
    pub mem: OsalGpuMem,
    pub format: u8,
}

static STORE: Global<MsaaStore<Scratch>> = Global::new(MsaaStore::new());

fn store() -> &'static mut MsaaStore<Scratch> {
    // SAFETY: single-threaded by contract; every entry point below takes the
    // store once and lets go of it before it returns.
    unsafe { STORE.get() }
}

fn free(mut s: Scratch) {
    osal::gpu_free(&mut s.mem);
}

/// Why a 4x FBO draw cannot keep its samples here, or `None` when it can:
/// one colour target (the 8-RT bank is drawn in place, never through the
/// scratch, `emit_pe_state`), a surface the RS resolves
/// ([`hw::rs_resolve_format_of_surface`]: RGBA8888, the sRGB bytes, RGB565)
/// and the reload draw has a form for ([`hw::reload_draw`]).
/// Such a draw is drawn single-sampled (open point 4 of note (e)): the
/// resolved colour is right, its edges are not antialiased.
///
/// Any size is kept (gc-msaa-pad). A target whose size is not the RS's
/// alignment is resolved over the padded rectangle into a temporary
/// (`backend.rs` `emit_resolve`, [`hw::rs_resolve_extent`]), and the CPU
/// then copies its own `w` x `h` into the storage once the job is done. That
/// copy needs the storage's CPU view (`GlesDraw::target_cpu`), which every
/// GL core renderbuffer has; one without it is drawn single-sampled.
pub fn unsupported(gl: &GlesDraw, hw_targets: usize) -> Option<&'static str> {
    if gl.target_gpu == 0 {
        Some("no colour target")
    } else if hw_targets != 1 {
        Some("more than one colour target")
    } else if needs_pad(gl) && gl.target_cpu.is_null() {
        Some("a padded resolve with no CPU view of the storage")
    } else if hw::rs_resolve_format_of_surface(gl.target_format).is_none()
        || hw::reload_draw(gl.target_format).is_none()
    {
        Some("a colour format the resolve engine does not average")
    } else {
        None
    }
}

/// Does the resolve of `gl`'s target go through the padded temporary? True
/// when the target's size is not the RS's alignment ([`hw::rs_resolvable`]):
/// the RS window cannot be the target itself.
pub fn needs_pad(gl: &GlesDraw) -> bool {
    !hw::rs_resolvable(gl.target_w, gl.target_h)
}

/// The scratch of the 4x target of `gl`, made now when there is none, and
/// whether it holds the samples of the jobs before (`needs_load`). Keyed on
/// `target_gpu` at generation 0 (module doc); a slot of another surface
/// format is freed first, one of another extent or pixel size by the store
/// itself. `None` when there is no memory for it: the caller draws into a
/// scratch of this job only (`SampleLoad::Broadcast` with no slot).
pub(super) fn slot(gl: &GlesDraw, heap: u64) -> Option<(u64, bool)> {
    let st = store();
    let key = gl.target_gpu;
    if st.get(key).is_some_and(|s| s.handle.format != gl.target_format) {
        st.gone(key, free);
    }
    let (w, h, format) = (gl.target_w, gl.target_h, gl.target_format);
    let bpp = gles_surf_bytes_per_pixel(format);
    let size = hw::sample_size(w, h, bpp);
    let s = st.get_or_alloc(
        key,
        0,
        size,
        /* the padded supertiled surface, not the packed bytes asked for */
        |_packed| {
            let mut mem = OsalGpuMem::ZERO;
            (osal::gpu_alloc(heap, hw::scratch_bytes(w, h, bpp) as usize, OSAL_MEM_DATA, &mut mem) == 0)
                .then_some(Scratch { mem, format })
        },
        free,
    )?;
    Some((s.handle.mem.gpu, s.needs_load()))
}

/// The job that drew into the 4x target at `target_gpu` was submitted with
/// its RS resolve: its scratch holds every sample of everything drawn so far.
pub(super) fn stored(target_gpu: u64) {
    if let Some(s) = store().get(target_gpu) {
        s.mark_stored();
    }
}

/// A draw wrote the surface at `target_gpu` without its samples (drawn
/// single-sampled): the kept samples no longer match the resolved image.
pub(super) fn touched(target_gpu: u64) {
    if let Some(s) = store().get(target_gpu) {
        s.mark_cleared();
    }
}

/// The GL core drops the storage `[base, base + bytes)`
/// (`YakoglBackend::texture_released`): the slot of a target in it is freed,
/// so storage made again at the same address starts with no samples.
pub(super) fn released(base: u64, bytes: u64) {
    let end = base.saturating_add(bytes);
    let mut gone = [0u64; MSAA_SLOTS];
    let mut n = 0;
    for s in store().iter() {
        if (base..end).contains(&s.id) && n < gone.len() {
            gone[n] = s.id;
            n += 1;
        }
    }
    for id in &gone[..n] {
        store().gone(*id, free);
    }
}

/// Free every slot (`gc_shutdown`).
pub(super) fn release_all() {
    store().clear(free);
}

/// What the store keeps for the 4x target at `target_gpu`: its scratch's GPU
/// address and whether a job would draw on from its samples. A read-only
/// probe for the host tests (tests/es3_msaa_gc.rs).
pub fn kept(target_gpu: u64) -> Option<(u64, bool)> {
    store().get(target_gpu).map(|s| (s.handle.mem.gpu, s.needs_load()))
}

/// Slots in use (the host tests).
pub fn slots() -> usize {
    store().len()
}
