// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! A derived 4x4-tiled copy of a mipmapped layered texture (a cube, a 2D
//! array, a 3D texture) for the texture unit to sample.
//!
//! The texture unit steps from one face (layer) to the next by the level's
//! 4x4-TILE-padded size, `align4(w) * align4(h) * bpp`, whatever the
//! addressing mode: gl_test 113 on the i.MX8MP (2163f271 / 89cbc3dd) read
//! every face right at levels 0 and 4 of a 16x16 cube and faces 1-5 wrong at
//! levels 1-3, exactly what that step predicts over our linear storage
//! (VIVANTE_NOTES.md, "A mipmapped cube cannot be linear here"). At a level
//! narrower than its chain pitch that step is shorter than one face of linear
//! rows, so no linear layout holds such a texture. The GL core keeps every
//! texture linear; this backend keeps a tiled copy per texture and samples
//! that instead -- Mesa's own sampler layout is tiled.
//!
//! The copy is rebuilt from the linear storage whenever the texture's
//! `content_gen` (a CPU write) or `target_kick` (a render into it) differs
//! from the copy's, before the draw starts recording: a render into the
//! texture still in flight is finished first, and the CPU reads the storage
//! after invalidating its cache. A texture with one level, a compressed one
//! and a plain 2D one are sampled as they are -- except a 2D texture the
//! vertex stage samples, under `gc_debug_vs_tiled` (see `wanted_vs`).

use crate::backend::{
    gles_texfmt_bytes, GlesDraw, GlesTextureBind, GLES_TEXTYPE_2D, GLES_TEXTYPE_2D_ARRAY, GLES_TEXTYPE_3D,
    GLES_TEXTYPE_CUBE,
};
use crate::osal::{self, OsalGpuMem, OSAL_MEM_DATA};
use crate::util::Global;

/// Where a level of the copy starts: a 64-byte boundary, as the linear
/// chain's levels do (`GC_TEXDESC_LOD_ALIGN`).
const LEVEL_ALIGN: usize = 64;
/// Copies kept at once; the least recently used one whose last sampling kick
/// has completed is the one replaced.
const ENTRIES: usize = 16;
/// Bytes the copies may hold together. The i.MX8MP's GPU memory is a 64 MiB
/// carveout that also holds every texture, the frame arenas and the depth
/// surfaces; sixteen 512x512 mipmapped cubes would take twice this. A copy
/// is made room for by freeing the least recently used copies no pending
/// kick samples; a single copy larger than this evicts every other one and
/// is still made (dEQP texture.size.cube.512x512_rgba4444_mipmap ended the
/// run on the board at 24f64b64 with copies nothing freed).
const BUDGET: usize = 16 << 20;

/// What a copy was made from: a new value of any field is a new copy.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Key {
    base_gpu: u64,
    bytes: u32,
    content_gen: u32,
    target_kick: u32,
    width: u32,
    height: u32,
    layers: u16,
    levels: u8,
    format: u8,
}

struct Entry {
    key: Key,
    mem: OsalGpuMem,
    /// The kick that last sampled the copy: not reused until it completed.
    used_by: u32,
    /// For the LRU.
    stamp: u32,
}

struct Cache {
    entries: [Option<Entry>; ENTRIES],
    clock: u32,
}

static CACHE: Global<Cache> = Global::new(Cache { entries: [const { None }; ENTRIES], clock: 0 });

fn cache() -> &'static mut Cache {
    // SAFETY: single-threaded by contract; each entry point takes it once.
    unsafe { CACHE.get() }
}

/// Does this texture need the copy? Layered, more than one level or a layer
/// the texture unit would step differently, uncompressed, complete, and
/// with a CPU view of its storage.
///
/// A block format is out, by name: `gles_texfmt_bytes` answers 4 for it, so
/// the copy would move 8-byte blocks as texels. It needs no copy either --
/// the GL core lays its chain out as block grids (`compressed_chain_layout`),
/// a face of which is exactly the texture unit's padded step, and the
/// descriptor addresses it TILED as it is.
///
/// One level is enough when its linear layer is not the tile-padded one:
/// the texture unit steps layers by `align4(w) * align4(h)` texels, and the
/// GL core stores a layer as `stride_texels * h`. For a power-of-two face the
/// two agree and the storage is sampled as it is; for a 63x63 cube face
/// (64 x 63 stored, 64 x 64 stepped) every face after the first is read
/// shifted -- dEQP filtering.cube's sixteen *_npot cases on the i.MX8MP, the
/// only filtering.cube cases the copy had not moved (652000b4), since the GL
/// core gives an NPOT texture one level.
pub fn wanted(t: &GlesTextureBind) -> bool {
    let layered = t.type_ == GLES_TEXTYPE_CUBE || t.type_ == GLES_TEXTYPE_2D_ARRAY || t.type_ == GLES_TEXTYPE_3D;
    let padded = t.stride_texels * t.height != align4(t.width) * align4(t.height);
    layered
        && (t.mip_levels > 1 || padded)
        && t.incomplete == 0
        && !t.cpu.is_null()
        && !super::texture::is_block_format(t.format)
}

/// A plain 2D texture the vertex stage samples, under `gc_debug_vs_tiled`
/// (backend.rs): tiled like a layered one, because Mesa never lets this
/// core's vertex stage read a linear texture (golden mode 26: every level is
/// resolved into a TILED texture first) and every dEQP texture.vertex.2d
/// case fails on the i.MX8MP over our linear storage while the cube cases,
/// sampled from this copy, pass. Off until the board has measured it.
fn wanted_vs(t: &GlesTextureBind) -> bool {
    wanted(t)
        || (super::backend::vs_tiled_on()
            && t.type_ == GLES_TEXTYPE_2D
            && t.incomplete == 0
            && !t.cpu.is_null()
            && !super::texture::is_block_format(t.format))
}

fn key_of(t: &GlesTextureBind) -> Key {
    Key {
        base_gpu: t.base_gpu,
        bytes: t.bytes,
        content_gen: t.content_gen,
        target_kick: t.target_kick,
        width: t.width,
        height: t.height,
        layers: t.layers.max(1),
        levels: t.mip_levels,
        format: t.format,
    }
}

const fn align4(v: u32) -> u32 {
    (v + 3) & !3
}

/// Level `l`'s size, one layer.
const fn level_dims(w: u32, h: u32, l: u32) -> (u32, u32) {
    let lw = w >> l;
    let lh = h >> l;
    (if lw == 0 { 1 } else { lw }, if lh == 0 { 1 } else { lh })
}

/// The tiled copy's layout: level `l` starts at `off[l]`, each of its layers
/// `align4(w_l) * align4(h_l) * bpp` bytes back to back. Returns the offsets
/// and the total.
pub fn tiled_layout(w: u32, h: u32, layers: u32, levels: u32, bpp: u32) -> ([usize; 16], usize) {
    let mut off = [0usize; 16];
    let mut at = 0usize;
    for (l, o) in off.iter_mut().enumerate().take(levels.min(16) as usize) {
        at = (at + LEVEL_ALIGN - 1) & !(LEVEL_ALIGN - 1);
        *o = at;
        let (lw, lh) = level_dims(w, h, l as u32);
        at += (align4(lw) * align4(lh) * bpp) as usize * layers as usize;
    }
    (off, at)
}

/// The linear storage's layout, as the GL core keeps it for this backend and
/// as `texture.rs` build_descriptor walks it: level `l` at a 64-byte boundary,
/// its layers `stride_texels * h_l * bpp` bytes apart, rows `stride_texels *
/// bpp` apart.
pub fn linear_layout(t: &GlesTextureBind, bpp: u32) -> [usize; 16] {
    let layers = t.layers.max(1) as usize;
    let mut off = [0usize; 16];
    let mut at = 0usize;
    for (l, o) in off.iter_mut().enumerate().take(t.mip_levels.min(16) as usize) {
        at = (at + LEVEL_ALIGN - 1) & !(LEVEL_ALIGN - 1);
        *o = at;
        let (_, lh) = level_dims(t.width, t.height, l as u32);
        at += (t.stride_texels * lh * bpp) as usize * layers;
    }
    off
}

/// Byte offset of texel (x, y) inside one tiled layer `tw` texels wide
/// (a multiple of 4): 4x4 tiles row-major, texels row-major inside a tile.
const fn tiled_offset(x: u32, y: u32, tw: u32, bpp: u32) -> usize {
    let tile = (y / 4) * (tw / 4) + x / 4;
    ((tile * 16 + (y % 4) * 4 + x % 4) * bpp) as usize
}

/// Fill `dst` (the tiled layout) from `src` (the linear one).
///
/// # Safety
/// `src` holds the linear storage of `t` and `dst` the tiled layout's bytes.
unsafe fn tile(t: &GlesTextureBind, src: *const u8, dst: *mut u8, bpp: u32) {
    let layers = t.layers.max(1) as u32;
    let (toff, _) = tiled_layout(t.width, t.height, layers, t.mip_levels as u32, bpp);
    let loff = linear_layout(t, bpp);
    let pitch = t.stride_texels * bpp;
    for l in 0..t.mip_levels.min(16) as u32 {
        let (lw, lh) = level_dims(t.width, t.height, l);
        let tw = align4(lw);
        let layer_tiled = (tw * align4(lh) * bpp) as usize;
        let layer_linear = (pitch * lh) as usize;
        for k in 0..layers {
            let s = loff[l as usize] + k as usize * layer_linear;
            let d = toff[l as usize] + k as usize * layer_tiled;
            for y in 0..lh {
                for x in 0..lw {
                    let from = s + (y * pitch + x * bpp) as usize;
                    let to = d + tiled_offset(x, y, tw, bpp);
                    // SAFETY: both inside their layouts (the caller's contract).
                    unsafe { core::ptr::copy_nonoverlapping(src.add(from), dst.add(to), bpp as usize) };
                }
            }
        }
    }
}

/// Make every texture of `d` that wants a copy have a current one. Called
/// before the draw records anything: it may finish the GPU (a render into
/// the texture still in flight). Returns non-zero when a copy could not be
/// made -- the draw is then refused rather than sampling the linear storage,
/// which reads the wrong faces.
pub fn prepare(d: &GlesDraw, kick_pending: impl Fn(u32) -> bool, finish: impl Fn() -> i32) -> i32 {
    let fs = &d.textures[..(d.num_textures as usize).min(d.textures.len())];
    let vs = &d.vs_textures[..(d.num_vs_textures as usize).min(d.vs_textures.len())];
    for t in fs.iter().filter(|t| wanted(t)).chain(vs.iter().filter(|t| wanted_vs(t))) {
        if refresh(t, &kick_pending, &finish) != 0 {
            return -1;
        }
    }
    0
}

fn refresh(t: &GlesTextureBind, kick_pending: &impl Fn(u32) -> bool, finish: &impl Fn() -> i32) -> i32 {
    let key = key_of(t);
    let c = cache();
    c.clock = c.clock.wrapping_add(1);
    let clock = c.clock;
    if let Some(e) = c.entries.iter_mut().flatten().find(|e| e.key == key) {
        e.stamp = clock;
        return 0;
    }
    /* the texels the GPU wrote must be in memory before the CPU reads them */
    if t.target_kick != 0 && kick_pending(t.target_kick) && finish() != 0 {
        return -1;
    }
    /* the older copies of this storage are stale now: free the ones no kick
     * still reads, so a texture updated every frame holds one slot */
    for e in c.entries.iter_mut() {
        if e.as_ref().is_some_and(|e| e.key.base_gpu == key.base_gpu && !busy(e, kick_pending)) {
            if let Some(mut old) = e.take() {
                osal::gpu_free(&mut old.mem);
            }
        }
    }
    let bpp = gles_texfmt_bytes(t.format);
    let layers = t.layers.max(1) as u32;
    let (_, total) = tiled_layout(t.width, t.height, layers, t.mip_levels as u32, bpp);
    /* room under the budget: least recently used first, never one a
     * pending kick samples */
    loop {
        let held: usize = c.entries.iter().flatten().map(|e| e.mem.size).sum();
        if held + total <= BUDGET {
            break;
        }
        let Some(i) = lru_idle(c, kick_pending) else { break };
        if let Some(mut old) = c.entries[i].take() {
            osal::gpu_free(&mut old.mem);
        }
    }
    let Some(slot) = victim(c, kick_pending) else {
        crate::glog_once!("vivante: no free tiled-copy slot (every one in flight)");
        return -1;
    };
    if let Some(mut old) = c.entries[slot].take() {
        osal::gpu_free(&mut old.mem);
    }
    let mut mem = OsalGpuMem::ZERO;
    if total == 0 || osal::gpu_alloc(super::backend::heap_data(), total, OSAL_MEM_DATA, &mut mem) != 0 {
        crate::glog_once!("vivante: no GPU memory for a {total}-byte tiled copy");
        return -1;
    }
    if mem.cpu.is_null() {
        osal::gpu_free(&mut mem);
        return -1;
    }
    let src_bytes = t.bytes as usize;
    // SAFETY: `t.cpu` is the storage's CPU view for `t.bytes` bytes (the GL
    // core's contract); `mem` was just allocated with `total` bytes.
    unsafe {
        osal::cache_invd(t.cpu as *mut core::ffi::c_void, src_bytes);
        core::ptr::write_bytes(mem.cpu as *mut u8, 0, total);
        tile(t, t.cpu as *const u8, mem.cpu as *mut u8, bpp);
        osal::cache_flush(mem.cpu, total);
    }
    c.entries[slot] = Some(Entry { key, mem, used_by: 0, stamp: clock });
    0
}

/// The slot to (re)use: an empty one, else the least recently used one whose
/// last sampling kick has completed.
fn victim(c: &Cache, kick_pending: &impl Fn(u32) -> bool) -> Option<usize> {
    if let Some(i) = c.entries.iter().position(|e| e.is_none()) {
        return Some(i);
    }
    c.entries
        .iter()
        .enumerate()
        .filter_map(|(i, e)| e.as_ref().map(|e| (i, e)))
        .filter(|(_, e)| !busy(e, kick_pending))
        .min_by_key(|(_, e)| e.stamp)
        .map(|(i, _)| i)
}

/// The least recently used copy no pending kick samples, if any.
fn lru_idle(c: &Cache, kick_pending: &impl Fn(u32) -> bool) -> Option<usize> {
    c.entries
        .iter()
        .enumerate()
        .filter_map(|(i, e)| e.as_ref().map(|e| (i, e)))
        .filter(|(_, e)| !busy(e, kick_pending))
        .min_by_key(|(_, e)| e.stamp)
        .map(|(i, _)| i)
}

/// Does a kick still to complete sample this copy?
fn busy(e: &Entry, kick_pending: &impl Fn(u32) -> bool) -> bool {
    e.used_by != 0 && kick_pending(e.used_by)
}

/// The copy of `t` (after [`prepare`]): its GPU address, its levels' offsets
/// and its level-0 layer size, and it is marked as used by `kick`, the kick
/// being recorded (`gc_kick_id`). `None` when `t` wants no copy. A 2D
/// texture has one only when [`prepare`] made it for the vertex stage; the
/// fragment stage then samples the same copy, which is as current as the
/// storage (the key carries `content_gen` and `target_kick`).
pub fn lookup(t: &GlesTextureBind, kick: u32) -> Option<(u64, [usize; 16], u32)> {
    if !wanted_vs(t) {
        return None;
    }
    let key = key_of(t);
    let e = cache().entries.iter_mut().flatten().find(|e| e.key == key)?;
    e.used_by = kick;
    let bpp = gles_texfmt_bytes(t.format);
    let (off, _) = tiled_layout(t.width, t.height, t.layers.max(1) as u32, t.mip_levels as u32, bpp);
    let slice = align4(t.width) * align4(t.height) * bpp;
    Some((e.mem.gpu, off, slice))
}

/// Free every copy (backend shutdown).
pub fn release_all() {
    for e in cache().entries.iter_mut() {
        if let Some(mut old) = e.take() {
            osal::gpu_free(&mut old.mem);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_texel_lands_in_its_4x4_tile_row_major() {
        /* an 8-wide layer is two tiles across: (5, 6) is in tile (1, 1), the
         * fourth tile, at row 2 column 1 inside it */
        assert_eq!(tiled_offset(0, 0, 8, 4), 0);
        assert_eq!(tiled_offset(3, 0, 8, 4), 3 * 4);
        assert_eq!(tiled_offset(0, 1, 8, 4), 4 * 4, "the next row of a tile follows its first");
        assert_eq!(tiled_offset(4, 0, 8, 4), 16 * 4, "the next tile follows a whole tile");
        assert_eq!(tiled_offset(0, 4, 8, 4), 2 * 16 * 4, "the second row of tiles");
        assert_eq!(tiled_offset(5, 6, 8, 4), (3 * 16 + 2 * 4 + 1) * 4);
    }

    #[test]
    fn a_16_cubes_levels_are_six_tile_padded_faces_each() {
        /* worked by hand: level l is six faces of align4(16 >> l)^2 texels,
         * 4 bytes each; 2x2 and 1x1 pad to a whole 4x4 tile */
        let (off, total) = tiled_layout(16, 16, 6, 5, 4);
        let face = [16 * 16 * 4, 8 * 8 * 4, 4 * 4 * 4, 4 * 4 * 4, 4 * 4 * 4];
        let mut at = 0;
        for (l, f) in face.iter().enumerate() {
            assert_eq!(off[l], at, "level {l}");
            at += 6 * f;
        }
        assert_eq!(total, at);
        assert_eq!(&off[..5], &[0, 6144, 7680, 8064, 8448]);
        assert!(off[..5].iter().all(|o| o % LEVEL_ALIGN == 0), "every level on a 64-byte boundary");
    }
}
