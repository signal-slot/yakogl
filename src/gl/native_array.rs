// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The PowerVR's native 2D array: a twiddled, layer-major copy of a 2D array
//! texture that the TPU samples one layer at a time by texture address
//! override (`smp` `tao`), behind `pvr_native_array` (default OFF).
//!
//! YakoGL stores a 2D array LINEAR and level-major -- level i holds the N
//! layers at that level's size, stacked -- and the PowerVR's stacked path
//! samples it as one tall 2D image (`pvr_arrUV`, `Sel::array_uv`), so a
//! LINEAR tap at a layer's edge blends the next layer and a mip level's
//! layer heights are the TPU's rounding, not GL's (dEQP
//! texture.filtering.2d_array, texture.shadow.2d_array). Mesa's pvr has no
//! hardware array path either (`pco_nir_tex.c` `hw_array_support = false`):
//! it describes the array as an ordinary twiddled 2D image whose IMAGE_WORD1
//! `depth` is the layer count less one, and reaches layer l by adding
//! `l * layer_size` to the image address in the shader. Each layer is then a
//! whole image of its own -- a full chain down to 1x1 whatever the level
//! count, extents rounded up to a power of two per axis, layers 4 KiB apart
//! (`pvr_image.c` `pvr_image_plane_setup_mip_levels`,
//! `pvr_image_plane_init_physical_extent`) -- and the TPU filters, wraps and
//! picks levels inside it. So the storage stays as it is and the PowerVR
//! samples a DERIVED copy, made here and made again when the texels change:
//! `gl::native_cube` generalised from six square faces to N layers of
//! `w x h` (docs/es3/pvr-2darray.md, section 6, choice C).
//!
//! When is it stale: the texels were written on the CPU (`GlTexture::
//! content_gen`, stamped at every CPU write), or a kick rendered into the
//! texture (`framebuffer::texture_target_use`). Both are compared where the
//! draw builds its sampler state (`gl::draw`), not only at bind.

use super::context::{heap_data, GlTexture};
use super::texture::{compressed_face_base, twiddle_index};
use crate::backend::gles_texfmt_bytes;
use crate::osal::{self, OsalGpuMem, OSAL_MEM_DATA};
use crate::util::Global;

/// A layer's start alignment in the twiddled layout (bytes): Mesa aligns a
/// layered image's layer size to the page size (`pvr_image.c:195-196`,
/// `image->alignment`), as `gl::native_cube::FACE_ALIGN` for a face.
pub const LAYER_ALIGN: u32 = 4096;

/// How many array copies are kept. A draw binds at most a few arrays, and a
/// copy that was pushed out is only made again.
const ENTRIES: usize = 16;

/// Is the native array on? The switch lives with the shader back end
/// (`shaderc::pvr_native_array`), whose programs it changes.
pub fn enabled() -> bool {
    crate::shaderc::native_array_enabled()
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
    /// Level 0 alone ([`copy`]'s `single`).
    single: bool,
    used: u32,
}

const EMPTY: Entry = Entry {
    src_gpu: 0,
    src_size: 0,
    stamp: 0,
    target_kick: 0,
    mem: OsalGpuMem::ZERO,
    bytes: 0,
    single: false,
    used: 0,
};

static CACHE: Global<[Entry; ENTRIES]> = Global::new([EMPTY; ENTRIES]);
static TICK: Global<u32> = Global::new(0);
/// Copies made since init (tests, and a board log that wants to know).
static COPIES: Global<u32> = Global::new(0);

/// Copies made since init.
pub fn copies() -> u32 {
    // SAFETY: single-threaded by contract.
    unsafe { *COPIES.get() }
}

/// The physical extent of a layer of `w x h`: each axis rounded up to a
/// power of two on its own (Mesa `pvr_image_plane_init_physical_extent`,
/// `pvr_image.c:83-92`; twiddled memory is always a power of two).
pub fn physical_extent(w: u32, h: u32) -> (u32, u32) {
    (w.max(1).next_power_of_two(), h.max(1).next_power_of_two())
}

/// The levels of the TPU's chain over a physical extent: down to 1x1, the
/// larger side deciding.
pub fn chain_levels(pw: u32, ph: u32) -> u32 {
    32 - pw.max(ph).max(1).leading_zeros()
}

/// Texels of one layer of physical extent `pw x ph` in the twiddled layout:
/// the full chain down to 1x1, each axis halved and clamped at 1 ("the hw
/// calculates layer strides as if a full mip chain ... were present",
/// `pvr_image.c:173-188`).
pub fn layer_texels(pw: u32, ph: u32) -> u32 {
    level_texel_off(pw, ph, chain_levels(pw, ph))
}

/// Texel offset of level `l` inside one layer of physical extent `pw x ph`.
pub fn level_texel_off(pw: u32, ph: u32, l: u32) -> u32 {
    (0..l).map(|k| (pw >> k).max(1) * (ph >> k).max(1)).sum()
}

/// One layer's bytes in the twiddled layout, 4 KiB aligned: its whole
/// chain, or with `single` its level 0 alone -- the TPU's layer stride for a
/// one-level image (Mesa adds the rest of the chain only when mip_levels >
/// 1), as `gl::native_cube::face_bytes`.
pub fn layer_bytes(w: u32, h: u32, bpp: u32, single: bool) -> u32 {
    let (pw, ph) = physical_extent(w, h);
    let texels = if single { pw * ph } else { layer_texels(pw, ph) };
    (texels * bpp).next_multiple_of(LAYER_ALIGN)
}

/// Can this texture be copied? A 2D array (layers stacked, not a 3D grid)
/// of some size with a CPU view.
fn copyable(t: &GlTexture) -> bool {
    t.layers >= 1 && t.grid_cols == 0 && t.width != 0 && t.height != 0 && !t.mem.cpu.is_null()
}

/// A compressed (4x4-block) array's layer in the TPU's walk: level l is a
/// max(1, gw0 >> l) x max(1, gh0 >> l) block grid, (gw0, gh0) the
/// power-of-two block grid of level 0 (Mesa `pvr_image_plane_setup_mip_levels`
/// over the physical extent, in blocks) -- the grid the GL core already lays
/// each (level, layer) out in (texture.rs `level_block_grid`). Returns (the
/// layer's bytes, 4 KiB aligned, and the byte offset of level `l` inside
/// it) for `levels` levels.
fn block_layer(w: u32, h: u32, bb: u32, levels: u32) -> (u32, [u32; 16]) {
    let g0 = |side: u32| side.div_ceil(4).max(1).next_power_of_two();
    let (gw0, gh0) = (g0(w), g0(h));
    let mut off = [0u32; 16];
    let mut at = 0u32;
    for (l, o) in off.iter_mut().enumerate().take(levels.min(16) as usize) {
        *o = at;
        at += (gw0 >> l).max(1) * (gh0 >> l).max(1) * bb;
    }
    (at.next_multiple_of(LAYER_ALIGN), off)
}

/// The row pitch of level `level` of `t`, whose width there is `lw`, in
/// texels: the layout every reader of the storage uses (texture.rs
/// `level_row_pitch`).
fn level_row_pitch(t: &GlTexture, level: u32, lw: u32) -> u32 {
    if t.pot_chain {
        t.level_pitch_at(level)
    } else {
        t.level_pitch(lw)
    }
}

/// The twiddled layer-major copy of 2D array `t`, as (gpu address, bytes):
/// the kept one when neither its texels' stamp nor the last kick that
/// rendered into them moved, else made again. `target_kick` is
/// `framebuffer::texture_target_use` for `t`; the caller has waited for it.
/// None when the texture cannot be described (no CPU view, a 3D grid) or
/// there is no memory for the copy; the caller then keeps the stacked image.
///
/// `single`: the draw samples one level (no mipmap filter, or a one-level
/// chain), so the copy is level 0 alone in one-level layers, which the
/// backend describes as a one-level image; else the whole chain (the rule
/// `gl::native_cube::copy` measured on the AM62P).
///
/// The bytes are `layers * layer_bytes`: the backend takes one layer's size
/// as `bytes / layers` (image meta dword 4, `PCO_IMAGE_META_LAYER_SIZE`).
pub fn copy(t: &GlTexture, target_kick: u32, single: bool) -> Option<(u64, u32)> {
    if !copyable(t) {
        return None;
    }
    let (w, h) = (t.width, t.height);
    let layers = u32::from(t.layers);
    let bpp = gles_texfmt_bytes(t.texfmt);
    let (pw, ph) = physical_extent(w, h);
    /* the TPU's chain, to 1x1 of the layer's power-of-two extent, and the
     * levels the texture's storage holds (fewer for a NPOT layer) */
    let chain = if single { 1 } else { chain_levels(pw, ph) };
    let (block_layer_bytes, block_off) = block_layer(w, h, u32::from(t.block_bytes), chain);
    let layer = if t.block_bytes != 0 { block_layer_bytes } else { layer_bytes(w, h, bpp, single) };
    let bytes = layer.checked_mul(layers)?;
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
            && e.single == single
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
    let have = (u32::from(t.chain_levels.max(1))).min(chain);
    // SAFETY: both views cover what is read and written: the source is the
    // texture's own storage, which `level_off` / `level_row_pitch` describe
    // (level l holds `layers` images of lw x lh, layer k's rows from
    // k * lh), the destination was allocated to at least `bytes`.
    unsafe {
        osal::cache_invd(t.mem.cpu, t.mem.size);
        let src = t.mem.cpu as *const u8;
        let dst = e.mem.cpu as *mut u8;
        core::ptr::write_bytes(dst, 0, bytes as usize);
        /* a block format: each (level, layer) is already the twiddled block
         * grid the TPU walks; the copy only puts them layer-major */
        for k in (0..layers).filter(|_| t.block_bytes != 0) {
            for l in 0..have {
                let (s, gw, gh) = compressed_face_base(t, l, k);
                let n = gw as usize * gh as usize * usize::from(t.block_bytes);
                let d = (k * layer + block_off[l as usize]) as usize;
                core::ptr::copy_nonoverlapping(src.add(s), dst.add(d), n);
            }
        }
        for k in (0..layers).filter(|_| t.block_bytes == 0) {
            for l in 0..have {
                let (lw, lh) = ((w >> l).max(1), (h >> l).max(1));
                let (pwl, phl) = ((pw >> l).max(1), (ph >> l).max(1));
                let pitch = level_row_pitch(t, l, lw);
                let lvl = k * layer + level_texel_off(pw, ph, l) * bpp;
                for y in 0..lh {
                    for x in 0..lw {
                        let s = t.level_off[l as usize] + ((k * lh + y) * pitch + x) * bpp;
                        let d = lvl + twiddle_index(x, y, pwl, phl) * bpp;
                        core::ptr::copy_nonoverlapping(src.add(s as usize), dst.add(d as usize), bpp as usize);
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
    e.single = single;
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
    use crate::gl::texture::{glBindTexture, glGenTextures, glTexStorage3D, glTexSubImage3D};
    use crate::gl::types::*;
    use crate::testutil::TestGl;

    /// A `w x h` array of `layers` layers and `levels` levels, texel =
    /// (layer*16 + level, x*8, y*8, 255), RGBA8.
    fn array(gl: &TestGl, w: u32, h: u32, layers: u32, levels: u32) -> GLuint {
        let mut name: GLuint = 0;
        // SAFETY: plain GL calls.
        unsafe {
            glGenTextures(1, &mut name);
            glBindTexture(GL_TEXTURE_2D_ARRAY, name);
            glTexStorage3D(
                GL_TEXTURE_2D_ARRAY,
                levels as GLsizei,
                GL_RGBA8,
                w as GLsizei,
                h as GLsizei,
                layers as GLsizei,
            );
        }
        for l in 0..levels {
            let (lw, lh) = ((w >> l).max(1), (h >> l).max(1));
            let mut px = Vec::new();
            for k in 0..layers {
                for y in 0..lh {
                    for x in 0..lw {
                        px.extend_from_slice(&[(k * 16 + l) as u8, (x * 8) as u8, (y * 8) as u8, 255]);
                    }
                }
            }
            // SAFETY: lw x lh x layers RGBA8 texels.
            unsafe {
                glTexSubImage3D(
                    GL_TEXTURE_2D_ARRAY,
                    l as GLint,
                    0,
                    0,
                    0,
                    lw as GLsizei,
                    lh as GLsizei,
                    layers as GLsizei,
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

    /// The layout numbers of docs/es3/pvr-2darray.md section 6: a layer is
    /// its power-of-two extent's chain to 1x1, each axis on its own, 4 KiB
    /// aligned; `single` is level 0 alone.
    #[test]
    fn a_layer_is_its_power_of_two_chain_to_one_by_one() {
        assert_eq!(physical_extent(5, 3), (8, 4));
        assert_eq!(chain_levels(8, 2), 4, "8x2, 4x1, 2x1, 1x1");
        assert_eq!(layer_texels(8, 2), 16 + 4 + 2 + 1);
        assert_eq!(level_texel_off(8, 2, 2), 16 + 4);
        assert_eq!(layer_texels(1, 1), 1);
        assert_eq!(layer_bytes(8, 2, 4, false), LAYER_ALIGN);
        assert_eq!(layer_bytes(5, 3, 4, false), LAYER_ALIGN, "8x4 + 4x2 + 2x1 + 1x1 texels of 4");
        assert_eq!(layer_bytes(64, 64, 4, true), 64 * 64 * 4, "one level, already aligned");
        assert_eq!(layer_bytes(64, 64, 4, false), 24 * 1024, "the whole chain");
        assert_eq!(layer_bytes(256, 64, 4, false), ((256 * 64 * 4 * 4) / 3 + 4095) & !4095);
    }

    /// Every texel of every (layer, level) lands at layer k's base, level
    /// l's texel offset and its twiddled index in the level's power-of-two
    /// extent, for a non-square array with a chain.
    #[test]
    fn the_copy_is_layer_major_full_chains_in_twiddled_order() {
        let gl = TestGl::new(4, 4);
        let (w, h, layers, levels) = (4u32, 2u32, 3u32, 3u32);
        let name = array(&gl, w, h, layers, levels);
        let t = &gl.ctx().objs_ref().textures[name as usize];
        let (gpu, bytes) = copy(t, 0, false).expect("an RGBA8 array has a copy");
        assert_eq!(bytes, layers * LAYER_ALIGN, "three layers of a 4x2 chain, each on a 4 KiB boundary");
        let s = bytes_at(gpu, bytes as usize);
        let (pw, ph) = physical_extent(w, h);
        for k in 0..layers {
            for l in 0..levels {
                let (lw, lh) = ((w >> l).max(1), (h >> l).max(1));
                let (pwl, phl) = ((pw >> l).max(1), (ph >> l).max(1));
                for y in 0..lh {
                    for x in 0..lw {
                        let d = (k * LAYER_ALIGN + (level_texel_off(pw, ph, l) + twiddle_index(x, y, pwl, phl)) * 4)
                            as usize;
                        assert_eq!(
                            &s[d..d + 4],
                            &[(k * 16 + l) as u8, (x * 8) as u8, (y * 8) as u8, 255],
                            "layer {k} level {l} ({x}, {y})"
                        );
                    }
                }
            }
        }
        /* level 0 of layer 1 in twiddled order over 4x2: y the low bit, x
         * the next, then x's remaining bit linear above the interleave */
        let base = LAYER_ALIGN as usize;
        for (i, (x, y)) in [(0u8, 0u8), (0, 1), (1, 0), (1, 1), (2, 0), (2, 1), (3, 0), (3, 1)].iter().enumerate() {
            assert_eq!(&s[base + i * 4..base + i * 4 + 4], &[16, x * 8, y * 8, 255], "layer 1 texel {i}");
        }
    }

    /// A NPOT array's layer lives in its power-of-two extent (5x3 in 8x4),
    /// and a draw sampling one level gets a one-level copy that holds
    /// nothing past level 0; the cache keeps the two kinds apart.
    #[test]
    fn an_npot_layer_is_placed_in_its_power_of_two_extent() {
        let gl = TestGl::new(4, 4);
        let name = array(&gl, 5, 3, 2, 1);
        let t = &gl.ctx().objs_ref().textures[name as usize];
        let n0 = copies();
        let (gpu, bytes) = copy(t, 0, true).expect("a one-level copy");
        assert_eq!(bytes, 2 * LAYER_ALIGN);
        let s = bytes_at(gpu, bytes as usize);
        for (x, y) in [(0u32, 0u32), (4, 0), (0, 2), (4, 2), (2, 1)] {
            let d = (LAYER_ALIGN + twiddle_index(x, y, 8, 4) * 4) as usize;
            assert_eq!(&s[d..d + 4], &[16, (x * 8) as u8, (y * 8) as u8, 255], "layer 1 ({x}, {y})");
        }
        /* a texel the 5x3 image does not have is zero in the 8x4 extent */
        let d = (LAYER_ALIGN + twiddle_index(7, 3, 8, 4) * 4) as usize;
        assert_eq!(&s[d..d + 4], &[0, 0, 0, 0]);
        let (gpu2, _) = copy(t, 0, false).expect("the chain's copy");
        assert_eq!(copies() - n0, 2, "the same size, another kind: copied again");
        assert_ne!(gpu, 0);
        let _ = gpu2;
    }

    #[test]
    fn a_copy_is_kept_until_its_texels_change_or_a_kick_renders_into_it() {
        let gl = TestGl::new(4, 4);
        let name = array(&gl, 2, 2, 2, 1);
        let t = &gl.ctx().objs_ref().textures[name as usize];
        let n0 = copies();
        let (a, _) = copy(t, 0, false).expect("a copy");
        assert_eq!(copies() - n0, 1);
        assert_eq!(copy(t, 0, false).map(|c| c.0), Some(a), "the same texels: the kept copy");
        assert_eq!(copies() - n0, 1, "not made again");
        /* a kick rendered into it */
        copy(t, 7, false).expect("a copy");
        assert_eq!(copies() - n0, 2, "made again after the render");
        /* the texels were written on the CPU */
        let px = [9u8, 9, 9, 255];
        // SAFETY: one RGBA8 texel of layer 1.
        unsafe {
            glTexSubImage3D(
                GL_TEXTURE_2D_ARRAY,
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
        let (gpu, bytes) = copy(t, 7, false).expect("a copy");
        assert_eq!(copies() - n0, 3, "made again after the write");
        let s = bytes_at(gpu, bytes as usize);
        let d = (LAYER_ALIGN + twiddle_index(1, 1, 2, 2) * 4) as usize;
        assert_eq!(&s[d..d + 4], &px, "and it holds the new texel");
        released(t.mem.gpu);
        let n1 = copies();
        copy(t, 7, false).expect("a copy");
        assert_eq!(copies() - n1, 1, "a released storage's copy is gone");
    }
}
