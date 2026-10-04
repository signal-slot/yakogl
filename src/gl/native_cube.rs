// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The PowerVR's native cube map: a twiddled copy of a cube texture the TPU
//! samples as TEXTYPE_CUBE, behind `pvr_native_cube` (default OFF).
//!
//! YakoGL stores every texture LINEAR and level-major -- level i holds the
//! six faces at that level's size, stacked -- for every GPU: the GC7000
//! samples that storage natively, the render-to-face, upload and
//! generateMipmap paths write it, and the PowerVR's emulated cube
//! (`pvr_cubeUV`, `Sel::cube_uv`) samples it as one tall 2D image. The Rogue
//! TPU's own cube exists only for TWIDDLED memory: Mesa's pvr texture state
//! has no CUBE case for a linear layout (`pvr_arch_tex_state.c`), and its
//! image is face-major, each face a full chain down to 1x1 whatever the
//! level count ("the hw calculates layer strides as if a full mip chain up
//! until 1x1x1 were present", `pvr_image.c` `pvr_image_plane_setup_mip_levels`),
//! extents rounded up to a power of two when there is more than one level,
//! and faces 4 KiB apart (the layer size aligned to the page size). So the
//! storage stays as it is and the PowerVR samples a DERIVED copy, made here
//! and made again when the texels change.
//!
//! When is it stale: the texels were written on the CPU (`GlTexture::
//! content_gen`, stamped at every CPU write), or a kick rendered into the
//! texture (`framebuffer::texture_target_use`). Both are compared where the
//! draw builds its sampler state (`gl::draw`), not only at bind: a cube bound
//! once, rendered into through a framebuffer and sampled by the next draw
//! without a rebind is copied again. A copy after a render waits for that
//! kick first.
//!
//! The board experiment this grew from (am62p-board's probe, 0 of 3072
//! texels wrong on silicon) called `yakogl_probe_native_cube`; that name
//! still turns the same switch.
//!
//! Step A of three: this makes the copy and the descriptor. The shaders
//! still sample the stacked image until the online compiler (step B) and the
//! table (step C) sample a real cube, so the switch stays off outside a
//! program whose shaders sample a `samplerCube` natively.

use core::ffi::c_int;

use super::context::{heap_data, GlTexture};
use super::texture::{compressed_face_base, twiddle_index};
use crate::backend::gles_texfmt_bytes;
use crate::osal::{self, OsalGpuMem, OSAL_MEM_DATA};
use crate::util::Global;

/// A face's start alignment in the twiddled layout (bytes): Mesa aligns a
/// layered image's layer size to the page size (`pvr_image.c`,
/// `image->alignment`; its comment ties it to
/// RGX_CR_TPU_TAG_CEM_4K_FACE_PACKING_EN). The probe packed faces 340 bytes
/// apart once: face 0 read right, faces 1..5 read zeros.
pub const FACE_ALIGN: u32 = 4096;

/// How many cube copies are kept. A draw binds at most a few cubes, and a
/// copy that was pushed out is only made again.
const ENTRIES: usize = 16;

/// Is the native cube on? The switch lives with the shader back end
/// (`shaderc::pvr_native_cube`), whose programs it changes.
pub fn enabled() -> bool {
    crate::shaderc::native_cube_enabled()
}

/// The probe program's name for the same switch.
#[no_mangle]
pub extern "C" fn yakogl_probe_native_cube(on: c_int) {
    crate::shaderc::pvr_set_native_cube(on);
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

/// Texels of one face of physical side `p` in the twiddled layout: the full
/// chain down to 1x1.
pub fn face_texels(p: u32) -> u32 {
    let (mut n, mut sum) = (p.max(1), 0u32);

    loop {
        sum += n * n;
        if n == 1 {
            return sum;
        }
        n /= 2;
    }
}

/// Texel offset of level `l` inside one face of physical side `p`.
pub fn level_texel_off(p: u32, l: u32) -> u32 {
    (0..l).map(|k| (p >> k).max(1).pow(2)).sum()
}

/// The physical side of a face of side `f` with `levels` levels: rounded up
/// to a power of two when there is more than one level (Mesa's
/// `pvr_image_plane_init_physical_extent`; twiddled memory is always a power
/// of two).
pub fn physical_side(f: u32) -> u32 {
    f.max(1).next_power_of_two()
}

/// One face's bytes in the twiddled layout, 4 KiB aligned: its whole
/// chain, or with `single` its level 0 alone -- the TPU's layer stride for
/// a one-level image (Mesa pvr_image_plane_setup_mip_levels adds the rest
/// of the chain only when mip_levels > 1).
pub fn face_bytes(f: u32, bpp: u32, single: bool) -> u32 {
    let p = physical_side(f);
    let texels = if single { p * p } else { face_texels(p) };
    (texels * bpp).next_multiple_of(FACE_ALIGN)
}

/// Can this texture be copied? A square uncompressed cube with a CPU view.
fn copyable(t: &GlTexture) -> bool {
    t.layers == 6 && t.width == t.height && t.width != 0 && !t.mem.cpu.is_null()
}

/// A compressed (4x4-block) cube's face in the TPU's walk: level l is a
/// max(1, g0 >> l)-square block grid, g0 = the power-of-two block grid of
/// level 0 (Mesa pvr_image_plane_setup_mip_levels over the physical extent,
/// in blocks) -- the grid the GL core already lays each (level, face) out in
/// (texture.rs level_block_grid). Returns (the face's bytes, 4 KiB aligned,
/// and the byte offset of level `l` inside it) for `levels` levels.
fn block_face(side: u32, bb: u32, levels: u32) -> (u32, [u32; 16]) {
    let g0 = side.div_ceil(4).max(1).next_power_of_two();
    let mut off = [0u32; 16];
    let mut at = 0u32;
    for (l, o) in off.iter_mut().enumerate().take(levels as usize) {
        *o = at;
        let g = (g0 >> l).max(1);
        at += g * g * bb;
    }
    (at.next_multiple_of(FACE_ALIGN), off)
}

/// The twiddled copy of cube `t`, as (gpu address, bytes): the kept one when
/// neither its texels' stamp nor the last kick that rendered into them
/// moved, else made again. `target_kick` is `framebuffer::texture_target_use`
/// for `t`; the caller has waited for it. None when the texture cannot be
/// described (compressed, not square, no CPU view) or there is no memory for
/// the copy; the caller then keeps the stacked image.
///
/// `single`: the draw samples one level (no mipmap filter, or a one-level
/// chain), so the copy is level 0 alone in one-level faces, which the
/// backend describes as a one-level image; else the whole chain. A
/// full-chain copy under a one-level sampler lets the TPU's min/mag choice
/// (made on the clamped LOD, which a one-level image's maxlod of 1.0 keeps
/// honest) walk into levels the texture never had: AM62P native arm at
/// 3b634e98, filtering.cube linear_nearest/nearest_linear (32) and
/// size.cube 256/512 non-mipmap (6, all white).
pub fn copy(t: &GlTexture, target_kick: u32, single: bool) -> Option<(u64, u32)> {
    if !copyable(t) {
        return None;
    }
    let f = t.width;
    let bpp = gles_texfmt_bytes(t.texfmt);
    let p = physical_side(f);
    /* the TPU's chain, to 1x1 of the face's power-of-two side, and the
     * levels the texture's storage holds (fewer for a NPOT face) */
    let chain = if single { 1 } else { 32 - p.leading_zeros() };
    let (block_layer, block_off) = block_face(f, t.block_bytes as u32, chain);
    let layer = if t.block_bytes != 0 { block_layer } else { face_bytes(f, bpp, single) };
    let bytes = layer.checked_mul(6)?;
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
    // SAFETY: both views cover what is read and written: the source is the
    // texture's own storage, which `level_off` / `level_pitch` describe
    // (level l holds six n x n faces, face k's rows from k * n), the
    // destination was allocated to at least `bytes`.
    unsafe {
        osal::cache_invd(t.mem.cpu, t.mem.size);
        let src = t.mem.cpu as *const u8;
        let dst = e.mem.cpu as *mut u8;
        core::ptr::write_bytes(dst, 0, bytes as usize);
        /* a block format: each (level, face) is already the twiddled block
         * grid the TPU walks; the copy only puts them face-major */
        let have = (t.chain_levels.max(1) as u32).min(chain);
        for face in (0..6u32).filter(|_| t.block_bytes != 0) {
            for l in 0..have {
                let (s, gw, gh) = compressed_face_base(t, l, face);
                let n = gw as usize * gh as usize * t.block_bytes as usize;
                let d = (face * layer + block_off[l as usize]) as usize;
                core::ptr::copy_nonoverlapping(src.add(s), dst.add(d), n);
            }
        }
        for face in (0..6u32).filter(|_| t.block_bytes == 0) {
            for l in 0..(if single { 1 } else { t.chain_levels.max(1) as u32 }) {
                let n = (f >> l).max(1);
                let np = (p >> l).max(1);
                let pitch = t.level_pitch(n);
                let lvl = face * layer + level_texel_off(p, l) * bpp;
                for y in 0..n {
                    for x in 0..n {
                        let s = t.level_off[l as usize] + ((face * n + y) * pitch + x) * bpp;
                        let d = lvl + twiddle_index(x, y, np, np) * bpp;
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gl::consts::*;
    use crate::gl::texture::{glBindTexture, glGenTextures, glTexStorage2D, glTexSubImage2D};
    use crate::gl::types::*;
    use crate::testutil::TestGl;

    /// A cube of side `f` and `levels` levels, texel = (face*16 + level, x*8,
    /// y*8, 255), RGBA8.
    fn cube(gl: &TestGl, f: u32, levels: u32) -> GLuint {
        let mut name: GLuint = 0;
        // SAFETY: plain GL calls.
        unsafe {
            glGenTextures(1, &mut name);
            glBindTexture(GL_TEXTURE_CUBE_MAP, name);
            glTexStorage2D(GL_TEXTURE_CUBE_MAP, levels as GLsizei, GL_RGBA8, f as GLsizei, f as GLsizei);
        }
        for face in 0..6u32 {
            for l in 0..levels {
                let n = (f >> l).max(1);
                let mut px = Vec::new();
                for y in 0..n {
                    for x in 0..n {
                        px.extend_from_slice(&[(face * 16 + l) as u8, (x * 8) as u8, (y * 8) as u8, 255]);
                    }
                }
                // SAFETY: n x n RGBA8 texels.
                unsafe {
                    glTexSubImage2D(
                        GL_TEXTURE_CUBE_MAP_POSITIVE_X + face,
                        l as GLint,
                        0,
                        0,
                        n as GLsizei,
                        n as GLsizei,
                        GL_RGBA,
                        GL_UNSIGNED_BYTE,
                        px.as_ptr() as *const core::ffi::c_void,
                    )
                };
            }
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

    #[test]
    fn the_copy_is_face_major_full_chains_in_twiddled_order() {
        let gl = TestGl::new(4, 4);
        let name = cube(&gl, 2, 2);
        let t = &gl.ctx().objs_ref().textures[name as usize];
        let (gpu, bytes) = copy(t, 0, false).expect("an RGBA8 cube has a copy");
        assert_eq!(bytes, 6 * FACE_ALIGN, "six faces of a 2x2 + 1x1 chain, each on a 4 KiB boundary");
        let s = bytes_at(gpu, bytes as usize);
        for f in 0..6usize {
            let face = f * FACE_ALIGN as usize;
            /* level 0 in twiddled order: y is the low bit, x the next */
            for (i, (x, y)) in [(0u8, 0u8), (0, 1), (1, 0), (1, 1)].iter().enumerate() {
                assert_eq!(
                    &s[face + i * 4..face + i * 4 + 4],
                    &[f as u8 * 16, x * 8, y * 8, 255],
                    "face {f} texel {i}"
                );
            }
            assert_eq!(&s[face + 16..face + 20], &[f as u8 * 16 + 1, 0, 0, 255], "face {f} level 1");
        }
    }

    /// A draw sampling one level gets a one-level copy: level 0 alone, each
    /// face level 0's size on a 4 KiB boundary (64x64 RGBA8: 16 KiB, where
    /// the chain takes 24 KiB), and nothing past level 0 in it. The cache
    /// keeps the two kinds apart even when their sizes agree (2x2: 4 KiB
    /// either way), so a mipmapped draw never gets the one-level copy.
    #[test]
    fn a_one_level_draw_gets_a_one_level_copy() {
        let gl = TestGl::new(4, 4);
        assert_eq!(face_bytes(64, 4, true), 64 * 64 * 4);
        assert_eq!(face_bytes(64, 4, false), 24 * 1024);
        let name = cube(&gl, 2, 2);
        let t = &gl.ctx().objs_ref().textures[name as usize];
        let n0 = copies();
        let (gpu, bytes) = copy(t, 0, true).expect("a one-level copy");
        assert_eq!(bytes, 6 * FACE_ALIGN);
        let s = bytes_at(gpu, bytes as usize);
        for f in 0..6usize {
            let face = f * FACE_ALIGN as usize;
            assert_eq!(&s[face..face + 4], &[f as u8 * 16, 0, 0, 255], "face {f} level 0");
            assert_eq!(&s[face + 16..face + 20], &[0, 0, 0, 0], "face {f}: no level 1 in a one-level copy");
        }
        let (gpu2, _) = copy(t, 0, false).expect("the chain's copy");
        assert_eq!(copies() - n0, 2, "the same size, another kind: copied again");
        let s = bytes_at(gpu2, bytes as usize);
        assert_eq!(&s[16..20], &[1, 0, 0, 255], "the chain's copy holds level 1");
    }

    /// An ETC1 cube (the AM62P's filtering.cube.*_etc1 and
    /// format.etc1_cube_pot under step C's native default): each (level,
    /// face) block grid goes face-major, level l at the TPU's offset -- 8x8
    /// faces: grids of 2x2, 1, 1, 1 blocks of 8 bytes, so levels at 0, 32,
    /// 40, 48 -- and every face on a 4 KiB boundary.
    #[test]
    fn an_etc1_cube_is_copied_as_block_grids_face_major() {
        use crate::gl::texture::glCompressedTexImage2D;
        let gl = TestGl::new(4, 4);
        let mut name: GLuint = 0;
        // SAFETY: plain GL calls; each upload is its level's blocks.
        unsafe {
            glGenTextures(1, &mut name);
            glBindTexture(GL_TEXTURE_CUBE_MAP, name);
            for face in 0..6u32 {
                for l in 0..4u32 {
                    let side = 8u32 >> l;
                    let n = (side.div_ceil(4) * side.div_ceil(4)) as usize;
                    let blocks: Vec<u8> = (0..n * 8).map(|i| (face * 16 + l * 4) as u8 + (i / 8) as u8).collect();
                    glCompressedTexImage2D(
                        GL_TEXTURE_CUBE_MAP_POSITIVE_X + face,
                        l as GLint,
                        GL_ETC1_RGB8_OES,
                        side as GLsizei,
                        side as GLsizei,
                        0,
                        (n * 8) as GLsizei,
                        blocks.as_ptr() as *const core::ffi::c_void,
                    );
                }
            }
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
        let t = &gl.ctx().objs_ref().textures[name as usize];
        let (gpu, bytes) = copy(t, 0, false).expect("an ETC1 cube has a copy");
        assert_eq!(bytes, 6 * FACE_ALIGN);
        let s = bytes_at(gpu, bytes as usize);
        for face in 0..6usize {
            for (l, off) in [(0usize, 0usize), (1, 32), (2, 40), (3, 48)] {
                let at = face * FACE_ALIGN as usize + off;
                assert_eq!(s[at], (face * 16 + l * 4) as u8, "face {face} level {l}: its first block");
            }
            /* level 0's grid is 2x2: its four blocks, in the storage's
             * (already twiddled) order, each tagged by its index */
            let at = face * FACE_ALIGN as usize;
            let tags: Vec<u8> = (0..4).map(|b| s[at + b * 8]).collect();
            let mut sorted = tags.clone();
            sorted.sort();
            assert_eq!(
                sorted,
                (0..4).map(|b| (face * 16) as u8 + b).collect::<Vec<u8>>(),
                "face {face} level 0: {tags:?}"
            );
        }
    }

    #[test]
    fn a_face_is_its_full_chain_to_one_by_one() {
        assert_eq!(face_texels(8), 64 + 16 + 4 + 1);
        assert_eq!(face_texels(1), 1);
        assert_eq!(level_texel_off(8, 3), 64 + 16 + 4);
        /* an NPOT face lives in the power-of-two physical face */
        assert_eq!(physical_side(5), 8);
        assert_eq!(face_bytes(5, 4, false), ((64 + 16 + 4 + 1) * 4u32).next_multiple_of(FACE_ALIGN));
    }

    #[test]
    fn an_npot_face_is_placed_in_its_power_of_two_face() {
        /* 3x3, one level: texel (x, y) at twiddle_index(x, y, 4, 4) */
        let gl = TestGl::new(4, 4);
        let name = cube(&gl, 3, 1);
        let t = &gl.ctx().objs_ref().textures[name as usize];
        let (gpu, bytes) = copy(t, 0, false).expect("a copy");
        let s = bytes_at(gpu, bytes as usize);
        for (x, y) in [(0u32, 0u32), (2, 0), (0, 2), (2, 2), (1, 1)] {
            let d = (FACE_ALIGN + twiddle_index(x, y, 4, 4) * 4) as usize;
            assert_eq!(&s[d..d + 4], &[16, (x * 8) as u8, (y * 8) as u8, 255], "face 1 ({x}, {y})");
        }
    }

    #[test]
    fn a_copy_is_kept_until_its_texels_change_or_a_kick_renders_into_it() {
        let gl = TestGl::new(4, 4);
        let name = cube(&gl, 2, 1);
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
        // SAFETY: one RGBA8 texel.
        unsafe {
            glTexSubImage2D(
                GL_TEXTURE_CUBE_MAP_NEGATIVE_Z,
                0,
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
        let d = (5 * FACE_ALIGN + twiddle_index(1, 1, 2, 2) * 4) as usize;
        assert_eq!(&s[d..d + 4], &px, "and it holds the new texel");
        released(t.mem.gpu);
        let n1 = copies();
        copy(t, 7, false).expect("a copy");
        assert_eq!(copies() - n1, 1, "a released storage's copy is gone");
    }
}
