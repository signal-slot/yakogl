// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! A 4x4-tiled shadow of a colour target with 64-bit pixels (RGBA16F), for
//! the pixel engine to render into.
//!
//! The pixel engine writes 64-bit pixels into a LINEAR surface at the wrong
//! places on this core: gl_test 44's probe (#6, cba8b199, record `got
//! 0x20000004 want 0x00c2073e`) drew one picture into RGBA16F targets 8, 4,
//! 16, 24 and 32 pixels wide and an RGBA8 control. Only the control came back
//! whole. In every row of every RGBA16F width, pixels 0 and 1 were right,
//! slot 2 held pixel 6's colour and slot 7 pixel 5's: the PE moves 16-byte
//! pixel pairs inside a row, at every row pitch from 32 to 256 bytes
//! (VIVANTE_NOTES.md, "RGBA16F targets", hypothesis 1).
//!
//! Mesa never renders a LINEAR target on this core (LINEAR_PE is used only
//! under the `linear_pe` debug flag, etnaviv_screen.c), so it never meets
//! this. Its render targets are tiled: `etna_resource_get_render_compatible`
//! (etnaviv_resource.c) gives a texture that is not render compatible a
//! tiled render copy, and the CPU untiles a tiled resource when it maps it
//! (this core has no BLT engine, and the RS copies no 64-bit format:
//! `etna_compatible_rs_format`). The same here, for 64-bit targets only:
//!
//! - A draw into such a target renders into this shadow: TILED (4x4 tiles,
//!   no SUPER_TILED bit), its width padded to 16 and its height to 4 as
//!   `etna_layout_multiple` pads a TILED render target on a core without the
//!   BLT engine, its row stride `padded width * 8` as Mesa programs
//!   PE_COLOR_STRIDE (`level->stride`), and PE_LOGIC_OP SINGLE_BUFFER(2),
//!   Mesa's value for a tiled 32/64-bit target on a SINGLE_BUFFER core
//!   (etnaviv_state.c). The shadow starts on a page, as every buffer Mesa
//!   hands this GPU does.
//! - Before the first draw of a kick into it, while the linear storage is
//!   the newer copy, the CPU tiles the linear storage into the shadow, so a
//!   draw that does not clear keeps what the target held.
//! - When the backend learns that the last kick rendering into it has
//!   completed (`retire`: from `gc_kick_pending`, `gc_finish`,
//!   `gc_finish_previous`), the CPU untiles the shadow into the linear
//!   storage. Every reader of the storage in the GL core asks one of those
//!   first (glReadPixels and glCopyTexImage through `finish`, a CPU write
//!   through `kick_pending` on the render's kick, the tiled sampling copy
//!   through `kick_pending`), so it reads the rendered pixels.
//! - The GL core drops the storage through `texture_released` before its
//!   address can be handed out again; the shadow goes with it, untouched.
//!
//! Not covered: a draw with more than one colour target (the 8-RT bank's
//! RT_CONFIG has no tiled bit of its own: SINGLE_BUFFER switches every
//! target), which keeps the linear path; and the depth surface of a draw
//! into a shadow, which SINGLE_BUFFER(2) switches too: its layout under (2)
//! is unmeasured, which matters only to `depth_resolve` and to a depth
//! buffer shared with draws into 32-bit targets (logged once when a draw
//! reaches it). ES 2.0 has no 64-bit colour format to render into.

use crate::backend::{gles_surf_bytes_per_pixel, GlesDraw};
use crate::osal::{self, OsalGpuMem, OSAL_MEM_DATA};
use crate::util::Global;

/// Bytes a pixel of a target this module takes.
const BPP: u32 = 8;
/// Where the shadow starts: a page.
const BASE_ALIGN: usize = 4096;
/// Shadows kept at once.
const ENTRIES: usize = 8;

/// The target a shadow stands for: a new value of any field is a new shadow.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Key {
    /// The linear storage's GPU address (`GlesDraw::target_gpu`).
    gpu: u64,
    w: u32,
    h: u32,
    /// The linear storage's row pitch in bytes.
    pitch: u32,
}

struct Entry {
    key: Key,
    /// The linear storage's CPU view.
    cpu: *mut u8,
    mem: OsalGpuMem,
    /// The shadow's GPU address and CPU view: the first page boundary in
    /// `mem`.
    gpu: u64,
    shadow: *mut u8,
    /// The last kick rendering into the shadow while the shadow is the
    /// newer copy; 0 while the linear storage is.
    kick: u32,
    /// For the LRU.
    stamp: u32,
}

struct Shadows {
    entries: [Option<Entry>; ENTRIES],
    clock: u32,
    /// Bytes untiled and tiled since init (`counts`).
    untiled: u64,
    tiled: u64,
}

static SHADOWS: Global<Shadows> =
    Global::new(Shadows { entries: [const { None }; ENTRIES], clock: 0, untiled: 0, tiled: 0 });

fn shadows() -> &'static mut Shadows {
    // SAFETY: single-threaded by contract; each entry point takes it once.
    unsafe { SHADOWS.get() }
}

const fn align_up(v: u32, a: u32) -> u32 {
    (v + a - 1) & !(a - 1)
}

/// The shadow's size in pixels: Mesa's TILED render-target padding on a core
/// without the BLT engine (`etna_layout_multiple`: paddingX 16, paddingY 4).
pub const fn padded(w: u32, h: u32) -> (u32, u32) {
    (align_up(w, 16), align_up(h, 4))
}

/// PE_COLOR_STRIDE for a shadow of a `w`-pixel-wide target: one row of
/// pixels of the padded width (Mesa's `level->stride`; only the RS takes a
/// tiled stride times 4).
pub const fn stride(w: u32) -> u32 {
    padded(w, 1).0 * BPP
}

/// Byte offset of pixel (x, y) in a tiled surface `tw` pixels wide (a
/// multiple of 4): 4x4 tiles row-major, pixels row-major inside a tile.
const fn tiled_offset(x: u32, y: u32, tw: u32) -> usize {
    let tile = (y / 4) * (tw / 4) + x / 4;
    ((tile * 16 + (y % 4) * 4 + x % 4) * BPP) as usize
}

/// Does this draw render into a shadow? One colour target (`single`: the
/// caller's count of hardware targets is 1), with 64-bit pixels, a CPU view
/// of its storage, and drawn with a positive stride straight into it (not
/// through the 4x scratch, not bottom-up).
pub fn wanted(gl: &GlesDraw, single: bool, straight: bool) -> bool {
    single
        && straight
        && gl.target_gpu != 0
        && !gl.target_cpu.is_null()
        && gles_surf_bytes_per_pixel(gl.target_format) == BPP
}

fn key_of(gl: &GlesDraw) -> Key {
    Key { gpu: gl.target_gpu, w: gl.target_w, h: gl.target_h, pitch: crate::backend::draw_pitch(gl) * BPP }
}

/// Make the shadow for `gl`'s target current, before the draw records
/// anything: allocate it, and tile the linear storage into it when the
/// storage is the newer copy. `finish` waits for every kick (and retires
/// them) when every slot holds a shadow a pending kick renders into.
/// Non-zero when there is no shadow; the draw is then refused rather than
/// rendered into the linear storage, which the PE writes wrong.
pub fn prepare(gl: &GlesDraw, finish: impl Fn() -> i32) -> i32 {
    let key = key_of(gl);
    let s = shadows();
    s.clock = s.clock.wrapping_add(1);
    let clock = s.clock;
    if let Some(e) = s.entries.iter_mut().flatten().find(|e| e.key == key) {
        e.stamp = clock;
        return if e.kick == 0 { tile_in(e) } else { 0 };
    }
    /* a shadow of the same storage at another size or pitch: the storage was
     * specified again; one that is still the newer copy goes back first */
    if s.entries.iter().flatten().any(|e| e.key.gpu == key.gpu && e.kick != 0) && finish() != 0 {
        return -1;
    }
    for e in s.entries.iter_mut() {
        if e.as_ref().is_some_and(|e| e.key.gpu == key.gpu) {
            if let Some(mut old) = e.take() {
                osal::gpu_free(&mut old.mem);
            }
        }
    }
    let slot = match free_slot(s) {
        Some(i) => i,
        None => {
            /* every shadow is the newer copy of its target: let the kicks
             * complete, which hands them all back (`retire`) */
            if finish() != 0 {
                return -1;
            }
            match free_slot(s) {
                Some(i) => i,
                None => return -1,
            }
        }
    };
    if let Some(mut old) = s.entries[slot].take() {
        osal::gpu_free(&mut old.mem);
    }
    let (pw, ph) = padded(key.w, key.h);
    let bytes = (pw * ph * BPP) as usize;
    let mut mem = OsalGpuMem::ZERO;
    if osal::gpu_alloc(super::backend::heap_data(), bytes + BASE_ALIGN - 1, OSAL_MEM_DATA, &mut mem) != 0 {
        crate::glog_once!("vivante: no GPU memory for a {bytes}-byte tiled colour target");
        return -1;
    }
    if mem.cpu.is_null() {
        osal::gpu_free(&mut mem);
        return -1;
    }
    let skip = (mem.gpu.wrapping_neg() & (BASE_ALIGN as u64 - 1)) as usize;
    let e = Entry {
        key,
        cpu: gl.target_cpu as *mut u8,
        gpu: mem.gpu + skip as u64,
        // SAFETY: `skip` is below BASE_ALIGN, inside the allocation.
        shadow: unsafe { (mem.cpu as *mut u8).add(skip) },
        mem,
        kick: 0,
        stamp: clock,
    };
    // SAFETY: `bytes` from the page boundary on, inside the allocation.
    unsafe { core::ptr::write_bytes(e.shadow, 0, bytes) };
    s.entries[slot] = Some(e);
    let Some(e) = s.entries[slot].as_mut() else { return -1 };
    tile_in(e)
}

/// An empty slot, else the least recently used one whose shadow is not the
/// newer copy of its target.
fn free_slot(s: &Shadows) -> Option<usize> {
    if let Some(i) = s.entries.iter().position(|e| e.is_none()) {
        return Some(i);
    }
    s.entries
        .iter()
        .enumerate()
        .filter_map(|(i, e)| e.as_ref().map(|e| (i, e)))
        .filter(|(_, e)| e.kick == 0)
        .min_by_key(|(_, e)| e.stamp)
        .map(|(i, _)| i)
}

/// The linear storage into the shadow (the storage is the newer copy, and
/// no kick writes either).
fn tile_in(e: &mut Entry) -> i32 {
    let Key { w, h, pitch, .. } = e.key;
    let tw = padded(w, h).0;
    let rows = (pitch as usize) * (h as usize - 1) + (w * BPP) as usize;
    // SAFETY: the storage holds `h` rows of `pitch` bytes (the GL core's
    // target), the shadow `padded(w, h)` pixels; both CPU views are live.
    unsafe {
        osal::cache_invd(e.cpu as *mut core::ffi::c_void, rows);
        for y in 0..h {
            for x in 0..w {
                let from = (y * pitch + x * BPP) as usize;
                core::ptr::copy_nonoverlapping(e.cpu.add(from), e.shadow.add(tiled_offset(x, y, tw)), BPP as usize);
            }
        }
        let (pw, ph) = padded(w, h);
        osal::cache_flush(e.shadow as *mut core::ffi::c_void, (pw * ph * BPP) as usize);
    }
    shadows().tiled += u64::from(w * h * BPP);
    0
}

/// The shadow `gl` renders into, marked as written by `kick` (the kick being
/// recorded): its GPU address. `None` when [`prepare`] made none.
pub fn bind(gl: &GlesDraw, kick: u32) -> Option<u64> {
    let key = key_of(gl);
    let e = shadows().entries.iter_mut().flatten().find(|e| e.key == key)?;
    e.kick = kick;
    Some(e.gpu)
}

/// Every shadow whose last rendering kick has completed (`pending` says it
/// has not) goes back into its linear storage, which is the newer copy from
/// then on.
pub fn retire(pending: impl Fn(u32) -> bool) {
    let s = shadows();
    let mut untiled = 0u64;
    for e in s.entries.iter_mut().flatten() {
        if e.kick == 0 || pending(e.kick) {
            continue;
        }
        let Key { w, h, pitch, .. } = e.key;
        let (pw, ph) = padded(w, h);
        let rows = (pitch as usize) * (h as usize - 1) + (w * BPP) as usize;
        // SAFETY: as in `tile_in`; the kick that wrote the shadow completed.
        unsafe {
            osal::cache_invd(e.shadow as *mut core::ffi::c_void, (pw * ph * BPP) as usize);
            for y in 0..h {
                for x in 0..w {
                    let to = (y * pitch + x * BPP) as usize;
                    core::ptr::copy_nonoverlapping(e.shadow.add(tiled_offset(x, y, pw)), e.cpu.add(to), BPP as usize);
                }
            }
            osal::cache_flush(e.cpu as *mut core::ffi::c_void, rows);
        }
        untiled += u64::from(w * h * BPP);
        e.kick = 0;
    }
    s.untiled += untiled;
}

/// The GL core drops the storage `[base, base + bytes)`
/// (`YakoglBackend::texture_released`): the shadows of targets in it go too,
/// without being written back.
pub fn released(base: u64, bytes: u64) {
    for e in shadows().entries.iter_mut() {
        if e.as_ref().is_some_and(|e| e.key.gpu >= base && e.key.gpu < base.wrapping_add(bytes)) {
            if let Some(mut old) = e.take() {
                osal::gpu_free(&mut old.mem);
            }
        }
    }
}

/// Bytes tiled into shadows and untiled out of them since init.
pub fn counts() -> (u64, u64) {
    let s = shadows();
    (s.tiled, s.untiled)
}

/// Free every shadow (backend shutdown).
pub fn release_all() {
    let s = shadows();
    for e in s.entries.iter_mut() {
        if let Some(mut old) = e.take() {
            osal::gpu_free(&mut old.mem);
        }
    }
    s.clock = 0;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_shadow_is_padded_as_mesa_pads_a_tiled_render_target() {
        assert_eq!(padded(8, 8), (16, 8));
        assert_eq!(padded(55, 63), (64, 64));
        assert_eq!(padded(16, 4), (16, 4));
        assert_eq!(stride(8), 128);
        assert_eq!(stride(32), 256);
    }

    #[test]
    fn a_pixel_lands_in_its_4x4_tile_row_major() {
        /* 16 wide: four tiles a tile row, 128 bytes a tile */
        assert_eq!(tiled_offset(0, 0, 16), 0);
        assert_eq!(tiled_offset(1, 0, 16), 8);
        assert_eq!(tiled_offset(0, 1, 16), 32);
        assert_eq!(tiled_offset(4, 0, 16), 128);
        assert_eq!(tiled_offset(0, 4, 16), 512);
        assert_eq!(tiled_offset(7, 5, 16), 512 + 128 + 32 + 24);
    }
}
