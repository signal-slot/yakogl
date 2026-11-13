// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! 32-bit tiled shadow render targets, resolved by the resolve engine (RS) in
//! the same stream, and the early depth they allow
//! (docs/notes/gc-tiled-wire.md W4 and W5). Behind the test knob
//! `gc_tiled_rt` ([`gc_debug_set_tiled_rt`]), off by default until the
//! Armadillo-X2 has run it (session A): with it off no draw reaches this
//! module and every stream is the one before it.
//!
//! Every colour surface the back end renders to is LINEAR, and Mesa's early
//! depth rule (`early_z.rs`, `etna_update_zsa`) denies early depth to a
//! linear target outright. A draw that qualifies ([`target_fits`], and a
//! [`super::ds_cache::pe_private`] depth renderbuffer, `backend.rs`
//! `draw_one`) renders instead into a 4x4-TILED shadow of its target, under
//! PE_LOGIC_OP SINGLE_BUFFER(2), with its depth SUPER_TILED: Mesa's
//! combination for a TILED colour target beside a super-tiled depth (its
//! 128-bit targets stay TILED, `etnaviv_resource.c:592-594`, depth is always
//! super-tiled, `:572-576`). The draws of one target in one stream are a
//! *run*: the first draw of a run tiles the target's storage into the shadow
//! with an RS linear-to-tiled copy (left out when the draw opens with a
//! whole, unmasked colour clear), and the run ends with the RS
//! tiled-to-linear copy back into the storage, recorded where the stream
//! leaves the target (another target, the linear path, or the end of the
//! stream). Every reader of the storage after that, the GPU's (a later draw,
//! the mipmap chain, the tiled sampling copy) and the CPU's (glReadPixels and
//! the others through `finish`), finds the rendered pixels there: the copy is
//! in the kick that rendered the shadow, before anything after it.
//!
//! What this does not do yet: keep the shadow in step across runs (each run
//! tiles in again unless it opens with a clear), tile status and fast clear
//! (W7-W9), and sampling the shadow directly (W10). The RS copies are Mesa's
//! words for a TILED side (`etna_compile_rs_state`, `etnaviv_rs.c:76-135`):
//! the tiled side's stride times four, no TILING bit (that bit is for a
//! super-tiled side), and SOURCE_TILED or DEST_TILED in RS_CONFIG.

use super::layout::{self, Layout};
use super::regs::*;
use crate::backend::{gles_surf_bytes_per_pixel, GlesDraw};
use crate::osal::{self, OsalGpuMem, OSAL_MEM_DATA};
use crate::util::Global;

/// The test knob: non-zero renders qualifying draws into tiled shadows. Set
/// by [`gc_debug_set_tiled_rt`] (gl_test's and glmark2's
/// `CONFIG_*_GC_TILED_RT`); only with the `test-knobs` feature, and without
/// it the shadows are never used.
#[cfg(feature = "test-knobs")]
#[no_mangle]
pub static mut gc_debug_tiled_rt: i32 = 0;

/// Set [`gc_debug_tiled_rt`].
#[cfg(feature = "test-knobs")]
#[no_mangle]
pub extern "C" fn gc_debug_set_tiled_rt(on: i32) {
    // SAFETY: single-threaded by contract; a plain store of an int.
    unsafe { core::ptr::addr_of_mut!(gc_debug_tiled_rt).write(on) }
}

/// Is the knob on? Never in a build without `test-knobs`.
pub fn on() -> bool {
    #[cfg(feature = "test-knobs")]
    {
        // SAFETY: single-threaded by contract; a plain read of an int only
        // the setter writes.
        (unsafe { core::ptr::addr_of!(gc_debug_tiled_rt).read() }) != 0
    }
    #[cfg(not(feature = "test-knobs"))]
    {
        false
    }
}

/// Bytes a pixel of a shadow.
const BPP: u32 = 4;
/// Where a shadow starts: a page, as every buffer Mesa hands this GPU.
const BASE_ALIGN: usize = 4096;
/// Shadows kept at once.
const ENTRIES: usize = 8;

/// Does a draw into `gl`'s colour target qualify for a shadow, as far as the
/// target goes? The knob is on; one hardware target (`single`), drawn
/// `straight` (not through the 4x scratch, not bottom-up); 4 bytes a pixel in
/// a format of the PE's base set (`ext_format` false: no float or integer
/// format); and a size the RS copies whole: the width a multiple of 16 and
/// the height of 4, so the padded shadow is the target and the RS writes
/// nothing past the GL core's rows. The depth side is the caller's.
pub fn target_fits(gl: &GlesDraw, single: bool, straight: bool, ext_format: bool) -> bool {
    on() && single
        && straight
        && !ext_format
        && gl.target_gpu != 0
        && gl.target_gpu >> 32 == 0
        && gles_surf_bytes_per_pixel(gl.target_format) == BPP
        && gl.target_w != 0
        && gl.target_h != 0
        && gl.target_w.is_multiple_of(16)
        && gl.target_h.is_multiple_of(4)
}

/// PE_COLOR_STRIDE of a shadow of a `w`-pixel-wide target, and the RS's
/// stride of it before the times-four: one row of pixels of the padded width
/// (Mesa's `level->stride`).
pub const fn stride(w: u32) -> u32 {
    layout::stride(Layout::Tiled, w, BPP, true)
}

/// One run: the shadow a target's draws render into in the open stream, and
/// the storage it is copied back into when the run ends.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Run {
    /// The shadow's GPU address.
    pub shadow: u32,
    /// The linear storage's GPU address and row pitch in bytes.
    pub storage: u32,
    pub pitch: u32,
    pub w: u32,
    pub h: u32,
}

/// The target a shadow stands for.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Key {
    gpu: u64,
    w: u32,
    h: u32,
    pitch: u32,
}

struct Entry {
    key: Key,
    mem: OsalGpuMem,
    /// The shadow's GPU address: the first page boundary in `mem`.
    gpu: u64,
    stamp: u32,
}

struct Shadows {
    entries: [Option<Entry>; ENTRIES],
    clock: u32,
    counts: Counts,
}

/// What the shadows did since init ([`counts`]).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Counts {
    /// Runs opened, and of those the ones that tiled the storage in first.
    pub runs: u32,
    pub tile_ins: u32,
    /// Runs resolved back into their storage.
    pub resolves: u32,
}

static SHADOWS: Global<Shadows> = Global::new(Shadows {
    entries: [const { None }; ENTRIES],
    clock: 0,
    counts: Counts { runs: 0, tile_ins: 0, resolves: 0 },
});

fn shadows() -> &'static mut Shadows {
    // SAFETY: single-threaded by contract; each entry point takes it once.
    unsafe { SHADOWS.get() }
}

fn key_of(gl: &GlesDraw) -> Key {
    Key { gpu: gl.target_gpu, w: gl.target_w, h: gl.target_h, pitch: crate::backend::draw_pitch(gl) * BPP }
}

/// The run a draw into `gl`'s target opens or continues, its shadow made
/// (allocated on first use, page-aligned, `w * h * 4` bytes): `None` when
/// there is no memory for one, and the draw then keeps the linear path.
/// `keep` is the shadow of the run open now, which no slot is taken from.
pub fn prepare(gl: &GlesDraw, keep: Option<Run>) -> Option<Run> {
    let key = key_of(gl);
    let s = shadows();
    s.clock = s.clock.wrapping_add(1);
    let clock = s.clock;
    let run = |e: &Entry| Run { shadow: e.gpu as u32, storage: key.gpu as u32, pitch: key.pitch, w: key.w, h: key.h };
    if let Some(e) = s.entries.iter_mut().flatten().find(|e| e.key == key) {
        e.stamp = clock;
        return Some(run(e));
    }
    /* the storage specified again at another size or pitch: its old shadow goes */
    for e in s.entries.iter_mut() {
        if e.as_ref().is_some_and(|e| e.key.gpu == key.gpu && keep.is_none_or(|k| e.gpu as u32 != k.shadow)) {
            if let Some(mut old) = e.take() {
                osal::gpu_free(&mut old.mem);
            }
        }
    }
    let slot = match s.entries.iter().position(|e| e.is_none()) {
        Some(i) => i,
        None => s
            .entries
            .iter()
            .enumerate()
            .filter_map(|(i, e)| e.as_ref().map(|e| (i, e)))
            .filter(|(_, e)| keep.is_none_or(|k| e.gpu as u32 != k.shadow))
            .min_by_key(|(_, e)| e.stamp)
            .map(|(i, _)| i)?,
    };
    /* the OSAL holds a free until the kicks that may use it have completed
     * (OSAL_QUERY_GC_FRAME_OPEN), so a shadow an earlier kick renders into
     * can go */
    if let Some(mut old) = s.entries[slot].take() {
        osal::gpu_free(&mut old.mem);
    }
    let bytes = layout::size(Layout::Tiled, key.w, key.h, BPP, true);
    let mut mem = OsalGpuMem::ZERO;
    if osal::gpu_alloc(super::backend::heap_data(), bytes + BASE_ALIGN - 1, OSAL_MEM_DATA, &mut mem) != 0 {
        crate::glog_once!("vivante: no GPU memory for a {bytes}-byte tiled shadow target; drawing linear");
        return None;
    }
    let skip = mem.gpu.wrapping_neg() & (BASE_ALIGN as u64 - 1);
    let gpu = mem.gpu + skip;
    if gpu >> 32 != 0 {
        osal::gpu_free(&mut mem);
        return None;
    }
    let e = Entry { key, mem, gpu, stamp: clock };
    let r = run(&e);
    s.entries[slot] = Some(e);
    Some(r)
}

/// Does `gl` open with a clear that writes every pixel of every channel of
/// its colour target, so a run it opens needs no tile-in? A full colour clear
/// (`clear_components` 0xf), unscissored, of every target (`clear_target_mask`
/// 0).
pub fn opens_with_full_clear(gl: &GlesDraw) -> bool {
    let mask = u32::from(gl.clear_color_mask) & 0xf;
    gl.do_clear && (mask == 0 || mask == 0xf) && !gl.scissor && gl.clear_target_mask == 0
}

/// The RS format of a shadow's copies: a bit copy of 4-byte pixels.
const FORMAT: u32 = GC_RS_FORMAT_A8R8G8B8;

/// The RS copy that ends `r`: the shadow (TILED) into the storage (linear).
pub(super) fn resolve_blit(r: &Run) -> super::backend::RsBlit {
    super::backend::RsBlit {
        config: rs_source_format(FORMAT) | rs_dest_format(FORMAT) | GC_RS_SOURCE_TILED,
        src: r.shadow,
        src_stride: stride(r.w) << 2,
        dst: r.storage,
        dst_stride: r.pitch,
        w: r.w,
        h: r.h,
    }
}

/// The RS copy that opens `r` when the storage holds what the target shows:
/// the storage (linear) into the shadow (TILED).
pub(super) fn tile_in_blit(r: &Run) -> super::backend::RsBlit {
    super::backend::RsBlit {
        config: rs_source_format(FORMAT) | rs_dest_format(FORMAT) | GC_RS_DEST_TILED,
        src: r.storage,
        src_stride: r.pitch,
        dst: r.shadow,
        dst_stride: stride(r.w) << 2,
        w: r.w,
        h: r.h,
    }
}

/// A run was opened (`tiled`: with a tile-in first), or resolved.
pub fn opened(tiled: bool) {
    let c = &mut shadows().counts;
    c.runs = c.runs.wrapping_add(1);
    if tiled {
        c.tile_ins = c.tile_ins.wrapping_add(1);
    }
}

pub fn resolved() {
    let c = &mut shadows().counts;
    c.resolves = c.resolves.wrapping_add(1);
}

/// What the shadows did since init (tests and the harness).
pub fn counts() -> Counts {
    shadows().counts
}

/// The GL core drops the storage `[base, base + bytes)`
/// (`YakoglBackend::texture_released`): the shadows of targets in it go too.
pub fn released(base: u64, bytes: u64) {
    for e in shadows().entries.iter_mut() {
        if e.as_ref().is_some_and(|e| e.key.gpu >= base && e.key.gpu < base.wrapping_add(bytes)) {
            if let Some(mut old) = e.take() {
                osal::gpu_free(&mut old.mem);
            }
        }
    }
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
    s.counts = Counts::default();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_shadow_row_is_the_padded_width_of_4_byte_pixels() {
        assert_eq!(stride(800), 3200);
        assert_eq!(stride(256), 1024);
        assert_eq!(stride(16), 64);
    }

    #[test]
    fn the_resolve_and_the_tile_in_are_mesas_words_for_a_tiled_side() {
        let r = Run { shadow: 0x4000_1000, storage: 0x4010_0000, pitch: 1024, w: 256, h: 64 };
        let fmt = GC_RS_FORMAT_A8R8G8B8;
        let out = resolve_blit(&r);
        assert_eq!(out.config, rs_source_format(fmt) | rs_dest_format(fmt) | GC_RS_SOURCE_TILED);
        /* etna_compile_rs_state: a TILED side's stride times four, TILING only for a super-tiled one */
        assert_eq!((out.src, out.src_stride, out.dst, out.dst_stride), (r.shadow, 1024 << 2, r.storage, 1024));
        assert_eq!(out.src_stride & (GC_RS_SOURCE_STRIDE_TILING | GC_RS_SOURCE_STRIDE_SUPER_TILED_NEW), 0);
        let inn = tile_in_blit(&r);
        assert_eq!(inn.config, rs_source_format(fmt) | rs_dest_format(fmt) | GC_RS_DEST_TILED);
        assert_eq!((inn.src, inn.src_stride, inn.dst, inn.dst_stride), (r.storage, 1024, r.shadow, 1024 << 2));
        assert_eq!((out.w, out.h, inn.w, inn.h), (256, 64, 256, 64));
    }

    #[test]
    fn only_an_unscissored_clear_of_every_channel_skips_the_tile_in() {
        let mut gl = GlesDraw::ZERO;
        assert!(!opens_with_full_clear(&gl), "no clear");
        gl.do_clear = true;
        assert!(opens_with_full_clear(&gl), "glClear(GL_COLOR_BUFFER_BIT)");
        gl.clear_color_mask = 0xf;
        assert!(opens_with_full_clear(&gl));
        gl.clear_color_mask = 0x7;
        assert!(!opens_with_full_clear(&gl), "alpha masked");
        gl.clear_color_mask = 0xf;
        gl.scissor = true;
        assert!(!opens_with_full_clear(&gl), "scissored");
        gl.scissor = false;
        gl.clear_target_mask = 1;
        assert!(!opens_with_full_clear(&gl), "glClearBuffer");
    }
}
