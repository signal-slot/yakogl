// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The PowerVR's native 3D texture: a twiddled copy of a 3D texture that the
//! TPU samples as its own TEXTYPE_3D, behind `pvr_native_3d` (default OFF).
//! The design and the reason for it are docs/notes/pvr-3d-native.md.
//!
//! YakoGL stores a 3D texture as a grid of slices, `cols` across, and the
//! PowerVR's shader picks the two slices around the depth coordinate and
//! mixes them (`Sel::vol_uv`): level 0 only, and the wrap mode lost on every
//! axis because the coordinate is clamped into the slice before the TPU
//! sees it. The TPU has a 3D texture type of its own. Mesa's pvr describes
//! a `VK_IMAGE_TYPE_3D` image as `PVR_MEMLAYOUT_3DTWIDDLED`
//! (`pvr_image.c:58-59`) and the TPU then walks its mip chain, filters
//! between slices and wraps all three axes by the sampler's modes. So the
//! storage stays as it is, and the PowerVR samples a DERIVED copy made here
//! and made again when the texels change, the way `gl::native_array` does
//! for a 2D array.
//!
//! The copy's layout is Mesa's: each axis of level 0 rounded up to a power
//! of two (`pvr_image_plane_init_physical_extent`, `pvr_image.c:83-92`),
//! level l the previous level's extent halved on each axis (at least 1),
//! stored in `align(extent, 4)` per axis (`pvr_image_plane_setup_mip_levels`,
//! `pvr_image.c:146-171`: `extent_alignment` 4 for a 3D image), the levels
//! back to back, and a texel at `usclib_twiddle3d`'s index inside its level
//! (`pco/usclib/tex.cl`).
//!
//! When is it stale: the texels were written on the CPU (`GlTexture::
//! content_gen`) or a kick rendered into the texture (`framebuffer::
//! texture_target_use`). Both are compared where the draw builds its sampler
//! state (`gl::draw`), not only at bind.

use super::context::{heap_data, GlTexture};
use super::texture::{chain_levels_3d, depth_at, level_held, level_size, texel_offset_3d};
use crate::backend::gles_texfmt_bytes;
use crate::osal::{self, OsalGpuMem, OSAL_MEM_DATA};
use crate::util::Global;

/// How many volume copies are kept. A draw binds at most a few, and a copy
/// that was pushed out is only made again.
const ENTRIES: usize = 8;

/// The physical extent of a volume of `w x h x d`: each axis rounded up to
/// a power of two on its own (`pvr_image.c:83-92`).
pub fn physical_extent(w: u32, h: u32, d: u32) -> (u32, u32, u32) {
    (w.max(1).next_power_of_two(), h.max(1).next_power_of_two(), d.max(1).next_power_of_two())
}

/// Level `l`'s physical extent: level 0's halved `l` times, at least 1
/// (`u_minify`).
pub fn level_extent(phys: (u32, u32, u32), l: u32) -> (u32, u32, u32) {
    ((phys.0 >> l).max(1), (phys.1 >> l).max(1), (phys.2 >> l).max(1))
}

/// The levels of the TPU's chain over a physical extent: down to 1x1x1, the
/// largest axis deciding.
pub fn chain_levels(phys: (u32, u32, u32)) -> u32 {
    32 - phys.0.max(phys.1).max(phys.2).max(1).leading_zeros()
}

/// Texels a level of physical extent `e` takes: each axis padded to a
/// multiple of 4 (`pvr_image.c:159-163`).
pub fn level_texels(e: (u32, u32, u32)) -> u32 {
    e.0.next_multiple_of(4) * e.1.next_multiple_of(4) * e.2.next_multiple_of(4)
}

/// Texel offset of level `l` in the copy: the levels before it, back to back.
pub fn level_texel_off(phys: (u32, u32, u32), l: u32) -> u32 {
    (0..l).map(|k| level_texels(level_extent(phys, k))).sum()
}

/// Texel (x, y, z)'s index inside a level of extent `size`, in the order
/// the TPU stores a 3D image: Mesa's `usclib_twiddle3d`
/// (`pco/usclib/tex.cl:152-212`). The level is cut into 4x4x4 cubes; the
/// cubes follow each other in an interleave of their coordinates' bits, one
/// bit of y, then x, then z a round, an axis dropping out when its cubes are
/// used up; inside a cube the slices (z % 4) are 16 texels apart, and a
/// slice is twiddled in 2x2 quads.
pub fn twiddle3d(x: u32, y: u32, z: u32, size: (u32, u32, u32)) -> u32 {
    let cubes = |s: u32| s.max(4).next_power_of_two() / 4;
    let (mut width, mut height, mut depth) = (cubes(size.0), cubes(size.1), cubes(size.2));
    let (cx, cy, cz) = (x / 4, y / 4, z / 4);
    let (mut shift, mut cube, mut i) = (0u32, 0u32, 0u32);
    while width > 1 || height > 1 || depth > 1 {
        if height > 1 {
            cube |= ((cy >> i) & 1) << shift;
            shift += 1;
            height >>= 1;
        }
        if width > 1 {
            cube |= ((cx >> i) & 1) << shift;
            shift += 1;
            width >>= 1;
        }
        if depth > 1 {
            cube |= ((cz >> i) & 1) << shift;
            shift += 1;
            depth >>= 1;
        }
        i += 1;
    }
    let slice = (y & 1) | ((x & 1) << 1) | (((y >> 1) & 1) << 2) | (((x >> 1) & 1) << 3);
    cube * 64 + 16 * (z % 4) + slice
}

/// The levels of `t` a draw with its current filters may sample: 1 without a
/// mipmap filter, else the contiguous run of defined levels from level 0
/// that GL_TEXTURE_MAX_LEVEL allows. (The GL core keeps `levels` at 1 for a
/// 3D grid, so this reads the per-level record.)
pub fn sampled_levels(t: &GlTexture) -> u32 {
    use crate::gl::consts::*;
    let mips = matches!(
        t.min_filter,
        GL_NEAREST_MIPMAP_NEAREST | GL_LINEAR_MIPMAP_NEAREST | GL_NEAREST_MIPMAP_LINEAR | GL_LINEAR_MIPMAP_LINEAR
    );
    if !mips {
        return 1;
    }
    let cap = chain_levels_3d(t.width, t.height, u32::from(t.layers)).min(t.max_level as u32 + 1);
    let mut n = 0;
    while n < cap && t.level_defined & (1u16 << n) != 0 && t.level_bad & (1u16 << n) == 0 && level_held(t, n) {
        n += 1;
    }
    n.max(1)
}

/// One kept copy: whose texels (the storage's address and size), how fresh
/// (the texels' stamp and the last kick that rendered into them), and where.
#[derive(Clone, Copy)]
struct Entry {
    src_gpu: u64,
    src_size: usize,
    stamp: u32,
    target_kick: u32,
    mem: OsalGpuMem,
    bytes: u32,
    /// The levels copied ([`copy`]'s `levels`).
    levels: u32,
    used: u32,
}

const EMPTY: Entry =
    Entry { src_gpu: 0, src_size: 0, stamp: 0, target_kick: 0, mem: OsalGpuMem::ZERO, bytes: 0, levels: 0, used: 0 };

static CACHE: Global<[Entry; ENTRIES]> = Global::new([EMPTY; ENTRIES]);
static TICK: Global<u32> = Global::new(0);
/// Copies made since init (tests, and a board log that wants to know).
static COPIES: Global<u32> = Global::new(0);

/// Copies made since init.
pub fn copies() -> u32 {
    // SAFETY: single-threaded by contract.
    unsafe { *COPIES.get() }
}

/// Can this texture be copied? A 3D grid ([`texture_alloc_storage_3d`]'s
/// PowerVR arrangement) of some size, uncompressed, with a CPU view.
///
/// [`texture_alloc_storage_3d`]: super::texture
fn copyable(t: &GlTexture) -> bool {
    t.layers >= 1 && t.grid_cols != 0 && t.width != 0 && t.height != 0 && t.block_bytes == 0 && !t.mem.cpu.is_null()
}

/// The bytes of a copy of `levels` levels of a volume of physical extent
/// `phys`: the sampled levels alone, or with more than one the whole chain
/// to 1x1x1 (the TPU derives nothing from the tail, but a mip walk must not
/// leave the allocation).
pub fn copy_bytes(phys: (u32, u32, u32), bpp: u32, levels: u32) -> u32 {
    let n = if levels > 1 { chain_levels(phys) } else { 1 };
    level_texel_off(phys, n) * bpp
}

/// The twiddled copy of 3D texture `t` with its first `levels` levels, as
/// (gpu address, bytes): the kept one when neither its texels' stamp nor the
/// last kick that rendered into them moved, else made again. `target_kick`
/// is `framebuffer::texture_target_use` for `t`; the caller has waited for
/// it. None when the texture cannot be described (no CPU view, not a grid)
/// or there is no memory for the copy; the caller then keeps the grid.
pub fn copy(t: &GlTexture, target_kick: u32, levels: u32) -> Option<(u64, u32)> {
    if !copyable(t) {
        return None;
    }
    let (w, h, d) = (t.width, t.height, u32::from(t.layers));
    let bpp = gles_texfmt_bytes(t.texfmt);
    let levels = levels.max(1);
    let phys = physical_extent(w, h, d);
    let bytes = copy_bytes(phys, bpp, levels);
    // SAFETY: single-threaded by contract; no other reference is live.
    let (cache, tick) = unsafe { (CACHE.get(), TICK.get()) };
    *tick = tick.wrapping_add(1);

    let hit = cache.iter().position(|e| e.src_gpu == t.mem.gpu && e.src_size == t.mem.size && !e.mem.impl_.is_null());
    if let Some(i) = hit {
        let e = &mut cache[i];
        e.used = *tick;
        /* a stamp of 0 is storage an EGLImage shares: never trusted */
        if t.content_gen != 0
            && e.stamp == t.content_gen
            && e.target_kick == target_kick
            && e.bytes == bytes
            && e.levels == levels
        {
            return Some((e.mem.gpu, bytes));
        }
    }
    let i = hit.unwrap_or_else(|| {
        /* an empty slot, else the least recently used */
        cache
            .iter()
            .enumerate()
            .min_by_key(|(_, e)| if e.mem.impl_.is_null() { 0 } else { e.used.max(1) })
            .map_or(0, |(i, _)| i)
    });
    let e = &mut cache[i];
    if e.mem.size < bytes as usize || e.mem.impl_.is_null() {
        if !e.mem.impl_.is_null() {
            osal::gpu_free(&mut e.mem);
        }
        *e = EMPTY;
        if osal::gpu_alloc(heap_data(), bytes as usize, OSAL_MEM_DATA, &mut e.mem) != 0 || e.mem.cpu.is_null() {
            if !e.mem.impl_.is_null() {
                osal::gpu_free(&mut e.mem);
            }
            *e = EMPTY;
            return None;
        }
    }
    // SAFETY: both views cover what is read and written: the source is the
    // texture's own storage, which `texel_offset_3d` describes (every level
    // `level_held` and defined lies inside it), the destination was
    // allocated to at least `bytes`.
    unsafe {
        osal::cache_invd(t.mem.cpu, t.mem.size);
        let src = t.mem.cpu as *const u8;
        let dst = e.mem.cpu as *mut u8;
        core::ptr::write_bytes(dst, 0, bytes as usize);
        for l in 0..levels {
            if t.level_defined & (1u16 << l) == 0 || t.level_bad & (1u16 << l) != 0 || !level_held(t, l) {
                continue;
            }
            let (lw, lh) = level_size(t, l);
            let ld = depth_at(t, l);
            let ext = level_extent(phys, l);
            let base = level_texel_off(phys, l);
            for z in 0..ld {
                for y in 0..lh {
                    for x in 0..lw {
                        let s = texel_offset_3d(t, l, x, y, z);
                        let o = (base + twiddle3d(x, y, z, ext)) * bpp;
                        core::ptr::copy_nonoverlapping(src.add(s), dst.add(o as usize), bpp as usize);
                    }
                }
            }
        }
        osal::cache_flush(e.mem.cpu, bytes as usize);
    }
    e.src_gpu = t.mem.gpu;
    e.src_size = t.mem.size;
    e.stamp = t.content_gen;
    e.target_kick = target_kick;
    e.bytes = bytes;
    e.levels = levels;
    e.used = *tick;
    // SAFETY: single-threaded by contract.
    unsafe { *COPIES.get() = COPIES.get().wrapping_add(1) };
    Some((e.mem.gpu, bytes))
}

/// Texel storage at `gpu` is being released: drop its copy, so a later
/// texture handed the same address cannot hit it.
pub fn released(gpu: u64) {
    // SAFETY: single-threaded by contract; no other reference is live.
    let cache = unsafe { CACHE.get() };
    for e in cache.iter_mut().filter(|e| e.src_gpu == gpu && !e.mem.impl_.is_null()) {
        osal::gpu_free(&mut e.mem);
        *e = EMPTY;
    }
}

#[cfg(all(test, not(feature = "report-es2")))] /* an ES 3.0 target, entry point or type */
mod tests {
    use super::*;
    use crate::gl::consts::*;
    use crate::gl::texture::{glBindTexture, glGenTextures, glTexParameteri, glTexStorage3D, glTexSubImage3D};
    use crate::gl::types::*;
    use crate::testutil::TestGl;

    /// A `w x h x d` volume of `levels` levels, texel = (z*16 + level, x*8,
    /// y*8, 255), RGBA8, with the given min filter.
    fn volume(gl: &TestGl, w: u32, h: u32, d: u32, levels: u32, min: GLenum) -> GLuint {
        let mut name: GLuint = 0;
        // SAFETY: plain GL calls.
        unsafe {
            glGenTextures(1, &mut name);
            glBindTexture(GL_TEXTURE_3D, name);
            glTexStorage3D(GL_TEXTURE_3D, levels as GLsizei, GL_RGBA8, w as GLsizei, h as GLsizei, d as GLsizei);
            glTexParameteri(GL_TEXTURE_3D, GL_TEXTURE_MIN_FILTER, min as GLint);
        }
        for l in 0..levels {
            let (lw, lh, ld) = ((w >> l).max(1), (h >> l).max(1), (d >> l).max(1));
            let mut px = Vec::new();
            for z in 0..ld {
                for y in 0..lh {
                    for x in 0..lw {
                        px.extend_from_slice(&[(z * 16 + l) as u8, (x * 8) as u8, (y * 8) as u8, 255]);
                    }
                }
            }
            // SAFETY: lw x lh x ld RGBA8 texels.
            unsafe {
                glTexSubImage3D(
                    GL_TEXTURE_3D,
                    l as GLint,
                    0,
                    0,
                    0,
                    lw as GLsizei,
                    lh as GLsizei,
                    ld as GLsizei,
                    GL_RGBA,
                    GL_UNSIGNED_BYTE,
                    px.as_ptr() as *const core::ffi::c_void,
                )
            };
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
        name
    }

    fn bytes_at(gpu: u64, len: usize) -> &'static [u8] {
        // SAFETY: single-threaded; the copy lives in the cache until released.
        let cache = unsafe { CACHE.get() };
        let e = cache.iter().find(|e| e.mem.gpu == gpu).expect("a kept copy");
        // SAFETY: the copy was written through its CPU view, `len` <= its size.
        unsafe { core::slice::from_raw_parts(e.mem.cpu as *const u8, len) }
    }

    /// The texel the TPU would read at integer (x, y, z) of level l of a
    /// copy: the layout's own arithmetic, from the extents alone.
    fn tpu_texel(s: &[u8], phys: (u32, u32, u32), l: u32, x: u32, y: u32, z: u32) -> [u8; 4] {
        let at = ((level_texel_off(phys, l) + twiddle3d(x, y, z, level_extent(phys, l))) * 4) as usize;
        s[at..at + 4].try_into().unwrap()
    }

    /// The same 3D filter REPEAT address the TPU applies (GL 3.8.7 on each
    /// axis), over the copy: a texel coordinate outside the level wraps.
    fn repeat(i: i32, n: u32) -> u32 {
        i.rem_euclid(n as i32) as u32
    }

    /// Mesa's numbers (`pvr_image.c`): the physical extent is a power of two
    /// on each axis, every level's axes are padded to 4, the chain runs to
    /// 1x1x1.
    #[test]
    fn a_levels_extent_is_a_power_of_two_padded_to_four() {
        assert_eq!(physical_extent(3, 7, 5), (4, 8, 8));
        assert_eq!(physical_extent(64, 64, 64), (64, 64, 64));
        assert_eq!(level_extent((4, 8, 8), 1), (2, 4, 4));
        assert_eq!(level_extent((4, 8, 8), 3), (1, 1, 1));
        assert_eq!(chain_levels((4, 8, 8)), 4, "8 -> 4 -> 2 -> 1");
        assert_eq!(level_texels((4, 8, 8)), 4 * 8 * 8);
        assert_eq!(level_texels((2, 4, 4)), 4 * 4 * 4, "2 pads to 4 on x");
        assert_eq!(level_texels((1, 1, 1)), 64);
        assert_eq!(level_texel_off((4, 8, 8), 2), 256 + 64);
        assert_eq!(copy_bytes((4, 8, 8), 4, 1), 256 * 4, "one level: level 0 alone");
        assert_eq!(copy_bytes((4, 8, 8), 4, 3), (256 + 64 + 64 + 64) * 4, "the whole chain");
    }

    /// `usclib_twiddle3d` worked by hand: a 4x4x4 level is one cube (slice
    /// z at 16 z, 2x2 quads inside it); an 8x4x4 level is two cubes, x the
    /// cube bit; a 4x4x8 level is two cubes with z the cube bit; with all
    /// three axes 8 the cube bits go y, x, z from the low bit.
    #[test]
    fn the_3d_twiddle_follows_mesas_usclib() {
        let one = (4, 4, 4);
        assert_eq!(twiddle3d(0, 0, 0, one), 0);
        assert_eq!(twiddle3d(0, 1, 0, one), 1, "y is the low bit of the quad");
        assert_eq!(twiddle3d(1, 0, 0, one), 2);
        assert_eq!(twiddle3d(0, 2, 0, one), 4);
        assert_eq!(twiddle3d(2, 0, 0, one), 8);
        assert_eq!(twiddle3d(3, 3, 0, one), 15);
        assert_eq!(twiddle3d(0, 0, 3, one), 48, "slice 3");
        assert_eq!(twiddle3d(3, 3, 3, one), 63);
        assert_eq!(twiddle3d(4, 0, 0, (8, 4, 4)), 64, "the second cube along x");
        assert_eq!(twiddle3d(0, 0, 4, (4, 4, 8)), 64, "the second cube along z");
        assert_eq!(twiddle3d(0, 4, 0, (4, 8, 4)), 64, "the second cube along y");
        let cube = (8, 8, 8);
        assert_eq!(twiddle3d(0, 4, 0, cube), 64, "y first");
        assert_eq!(twiddle3d(4, 0, 0, cube), 128, "then x");
        assert_eq!(twiddle3d(0, 0, 4, cube), 256, "then z");
        /* a level smaller than a cube: 1x1x1 is texel 0 of its 64 */
        assert_eq!(twiddle3d(0, 0, 0, (1, 1, 1)), 0);
        /* a bijection onto 0..n over a level whose axes are all cubes' worth */
        for size in [(4u32, 4u32, 4u32), (8, 4, 4), (4, 8, 8), (16, 8, 4)] {
            let mut seen = vec![false; (size.0 * size.1 * size.2) as usize];
            for z in 0..size.2 {
                for y in 0..size.1 {
                    for x in 0..size.0 {
                        let i = twiddle3d(x, y, z, size) as usize;
                        assert!(!seen[i], "{size:?} ({x}, {y}, {z}) collides at {i}");
                        seen[i] = true;
                    }
                }
            }
        }
    }

    /// Every texel of every level lands at its level's offset and its
    /// twiddled index, for a non-cubic volume with a chain; the copy holds
    /// the whole chain to 1x1x1.
    #[test]
    fn the_copy_holds_every_level_in_the_tpus_order() {
        let gl = TestGl::new(4, 4);
        let (w, h, d, levels) = (8u32, 4u32, 4u32, 4u32);
        let name = volume(&gl, w, h, d, levels, GL_LINEAR_MIPMAP_LINEAR);
        let t = &gl.ctx().objs_ref().textures[name as usize];
        assert_eq!(sampled_levels(t), levels, "a mipmap filter over four defined levels");
        let (gpu, bytes) = copy(t, 0, levels).expect("an RGBA8 volume has a copy");
        let phys = physical_extent(w, h, d);
        assert_eq!(bytes, copy_bytes(phys, 4, levels));
        let s = bytes_at(gpu, bytes as usize);
        for l in 0..levels {
            let (lw, lh, ld) = ((w >> l).max(1), (h >> l).max(1), (d >> l).max(1));
            for z in 0..ld {
                for y in 0..lh {
                    for x in 0..lw {
                        assert_eq!(
                            tpu_texel(s, phys, l, x, y, z),
                            [(z * 16 + l) as u8, (x * 8) as u8, (y * 8) as u8, 255],
                            "level {l} ({x}, {y}, {z})"
                        );
                    }
                }
            }
        }
    }

    /// A NPOT volume lives in its power-of-two extent (3x7x5 in 4x8x8); a
    /// draw without a mipmap filter gets level 0 alone, and a texel the
    /// image does not have reads zero.
    #[test]
    fn an_npot_volume_is_placed_in_its_power_of_two_extent() {
        let gl = TestGl::new(4, 4);
        let name = volume(&gl, 3, 7, 5, 1, GL_LINEAR);
        let t = &gl.ctx().objs_ref().textures[name as usize];
        assert_eq!(sampled_levels(t), 1);
        let (gpu, bytes) = copy(t, 0, 1).expect("a one-level copy");
        assert_eq!(bytes, 4 * 8 * 8 * 4);
        let s = bytes_at(gpu, bytes as usize);
        let phys = physical_extent(3, 7, 5);
        for (x, y, z) in [(0u32, 0u32, 0u32), (2, 6, 4), (1, 3, 2)] {
            assert_eq!(tpu_texel(s, phys, 0, x, y, z), [(z * 16) as u8, (x * 8) as u8, (y * 8) as u8, 255]);
        }
        assert_eq!(tpu_texel(s, phys, 0, 3, 7, 7), [0, 0, 0, 0], "outside the 3x7x5 image");
    }

    /// REPEAT on all three axes, mip level 1 of a 4x4x4 volume: a texel
    /// coordinate outside [0, n) wraps into the level the TPU walks, and the
    /// wrapped texel is the one GL names (3.8.7: `i mod n`). This is what the
    /// grid could not do -- it clamped a coordinate into the slice and the
    /// slice index into the stack.
    #[test]
    fn repeat_wraps_inside_the_level_the_copy_holds() {
        let gl = TestGl::new(4, 4);
        let name = volume(&gl, 4, 4, 4, 3, GL_NEAREST_MIPMAP_NEAREST);
        let t = &gl.ctx().objs_ref().textures[name as usize];
        let (gpu, bytes) = copy(t, 0, 3).expect("a copy");
        let s = bytes_at(gpu, bytes as usize);
        let phys = physical_extent(4, 4, 4);
        for (l, n) in [(0u32, 4u32), (1, 2), (2, 1)] {
            for (i, j, k) in [(-1i32, 5i32, -3i32), (n as i32, -(n as i32) - 1, 2 * n as i32 + 1), (7, -7, 7)] {
                let (x, y, z) = (repeat(i, n), repeat(j, n), repeat(k, n));
                assert_eq!(
                    tpu_texel(s, phys, l, x, y, z),
                    [(z * 16 + l) as u8, (x * 8) as u8, (y * 8) as u8, 255],
                    "level {l}: ({i}, {j}, {k}) wraps to ({x}, {y}, {z})"
                );
            }
        }
    }

    #[test]
    fn a_copy_is_kept_until_its_texels_change_or_a_kick_renders_into_it() {
        let gl = TestGl::new(4, 4);
        let name = volume(&gl, 4, 4, 4, 1, GL_NEAREST);
        let t = &gl.ctx().objs_ref().textures[name as usize];
        let n0 = copies();
        let (a, _) = copy(t, 0, 1).expect("a copy");
        assert_eq!(copies() - n0, 1);
        assert_eq!(copy(t, 0, 1).map(|c| c.0), Some(a), "the same texels: the kept copy");
        assert_eq!(copies() - n0, 1, "not made again");
        copy(t, 7, 1).expect("a copy");
        assert_eq!(copies() - n0, 2, "made again after the render");
        /* the same volume with a mipmap filter's levels is another kind */
        copy(t, 7, 3).expect("a copy");
        assert_eq!(copies() - n0, 3, "made again for another level count");
        /* the texels were written on the CPU */
        let px = [9u8, 9, 9, 255];
        // SAFETY: one RGBA8 texel of slice 1.
        unsafe {
            glTexSubImage3D(
                GL_TEXTURE_3D,
                0,
                1,
                1,
                1,
                1,
                1,
                1,
                GL_RGBA,
                GL_UNSIGNED_BYTE,
                px.as_ptr() as *const core::ffi::c_void,
            )
        };
        let t = &gl.ctx().objs_ref().textures[name as usize];
        let (gpu, bytes) = copy(t, 7, 3).expect("a copy");
        assert_eq!(copies() - n0, 4, "made again after the write");
        let s = bytes_at(gpu, bytes as usize);
        assert_eq!(tpu_texel(s, physical_extent(4, 4, 4), 0, 1, 1, 1), px, "and it holds the new texel");
        released(t.mem.gpu);
        let n1 = copies();
        copy(t, 7, 3).expect("a copy");
        assert_eq!(copies() - n1, 1, "a released storage's copy is gone");
    }
}
