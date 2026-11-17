// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The PowerVR's native 3D texture (`pvr_native_3d`, `gl::native_vol`) under
//! GL_TEXTURE_BASE_LEVEL, GL_TEXTURE_MAX_LEVEL, GL_TEXTURE_MIN_LOD and
//! GL_TEXTURE_MAX_LOD (ES 3.0.6 3.8.10, 3.8.14; dEQP-GLES3 texture.mipmap.3d
//! base_level, max_level, min_lod, max_lod): the copy starts at the base
//! level and stops at the maximum level (`native_vol::level_range`), the
//! descriptor describes that volume (its level 0 is the base level: its
//! extent, the copy's level count) and clamps the sampler's LOD range by
//! the texture's min and max LOD; each (base, levels) keeps a copy of its
//! own, and a copy a recorded draw binds is not written again. With the
//! switch off nothing of this is reached: the program samples the grid and
//! the grid's and the 2D descriptors do not change. The host OSAL does not
//! render: the state is checked, not pixels.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test es3_pvr_3d_lod
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-powervr", not(feature = "report-es2")))]
use core::ffi::{c_char, c_void};
use core::sync::atomic::{AtomicU32, Ordering};
use yakogl::backend::{
    self, GlesDraw, GlesHeap, GlesTextureBind, YakoglBackend, GLES_TEXFMT_RGBA8, GLES_TEXTYPE_2D, GLES_TEXTYPE_3D,
    GLES_TEXTYPE_3D_TWIDDLED, GLES_WRAP_REPEAT,
};
use yakogl::gl::consts::*;
use yakogl::gl::context::{self, GlTexture};
use yakogl::gl::names::{self, Kind};
use yakogl::gl::native_vol::{
    self, copy_bytes, level_range, level_texel_off, physical_extent, sampled_levels, twiddle3d, twiddle_extent, view,
    VolView,
};
use yakogl::gl::state::{glGetError, glPixelStorei};
use yakogl::gl::texture::{
    glBindTexture, glDeleteTextures, glGenTextures, glTexImage3D, glTexParameterf, glTexParameteri, glTexStorage3D,
    glTexSubImage3D,
};
use yakogl::gl::types::*;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};
use yakogl::pvr::backend::texture_descriptor;
use yakogl::shaderc::online::powervr::chain::compile_fragment;
use yakogl::shaderc::online::sema::Stage;
use yakogl::shaderc::online::{lower, store};

/* ---- a GL context on the host: host memory, and a stand-in back end with
 * the PowerVR's texture layout (a 3D texture's slices in a grid) that draws
 * nothing and whose open kick a test sets ---- */

unsafe extern "C" fn m_gpu_alloc(_heap: u64, size: usize, _flags: u32, out: *mut OsalGpuMem) -> i32 {
    let mem = Box::leak(vec![0u8; size].into_boxed_slice());
    let p = mem.as_mut_ptr();
    *out = OsalGpuMem { cpu: p as *mut c_void, gpu: p as u64, phys: p as u64, size, impl_: p as *mut c_void };
    0
}
unsafe extern "C" fn m_gpu_free(_mem: *mut OsalGpuMem) -> i32 {
    0
}
unsafe extern "C" fn m_cache(_cpu: *mut c_void, _n: usize) {}
unsafe extern "C" fn b_init() -> i32 {
    0
}
unsafe extern "C" fn b_heap_base(_heap: GlesHeap) -> u64 {
    0x1000_0000
}
unsafe extern "C" fn b_ok() -> i32 {
    0
}
unsafe extern "C" fn b_draw(_d: *const GlesDraw) -> i32 {
    0
}

/// The kick the stand-in back end says it is recording (0: none, so no copy
/// is ever in flight).
static OPEN_KICK: AtomicU32 = AtomicU32::new(0);
unsafe extern "C" fn b_kick_id() -> u32 {
    OPEN_KICK.load(Ordering::Relaxed)
}
unsafe extern "C" fn b_kick_pending(_id: u32) -> bool {
    false
}

static OSAL: OsalOps = OsalOps {
    init: None,
    shutdown: None,
    gpu_alloc: Some(m_gpu_alloc),
    gpu_free: Some(m_gpu_free),
    mmu_map: None,
    map_phys: None,
    gpu_write: None,
    gpu_read: None,
    cache_flush: Some(m_cache),
    cache_invd: Some(m_cache),
    submit: None,
    wait: None,
    completed: None,
    reg_read: None,
    reg_write: None,
    query: None,
    log: None,
    wait_vblank: None,
    present: None,
    lock: None,
    unlock: None,
    thread_id: None,
    display_busy: None,
};

static GRID: YakoglBackend = YakoglBackend {
    name: c"es3-pvr-3d-lod-grid".as_ptr() as *const c_char,
    native_attrib_formats: 0,
    rt_rgba8888_is_bgra: false,
    mip_level_align: 0,
    compressed_blocks_in_rows: false,
    native_layered_textures: false,
    front_facing_shader_variant: false,
    etc1_decoded_at_upload: false,
    init: Some(b_init),
    shutdown: None,
    draw: Some(b_draw),
    flush: Some(b_ok),
    finish: Some(b_ok),
    finish_previous: None,
    kick_id: Some(b_kick_id),
    kick_pending: Some(b_kick_pending),
    heap_base: Some(b_heap_base),
    counters: None,
    texture_released: None,
    surface_swapped: None,
    texture_uploaded: None,
    max_instance_divisor: 1,
    wait_kick: None,
    samples_texel_copies: false,
    attachment_gone: None,
    depth_resolve: None,
    query_begin: None,
    query_end: None,
    query_result: None,
    generate_mipmap: None,
    return_mipmap_chain: None,
    linear_row_align: 0,
    uniforms_host_only: false,
    ds_write: None,
    stencil_resolve: None,
};

/// The one GL context of this file, current, with [`GRID`] as the back end.
fn gl_context() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    static ONCE: std::sync::Once = std::sync::Once::new();
    let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    // SAFETY: the tables are statics; the lock serialises the GL users.
    unsafe {
        ONCE.call_once(|| {
            osal_set_ops(&OSAL);
            backend::g_backend = &GRID;
            assert!(!context::gl_context_create(8, 8).is_null());
        });
        glPixelStorei(GL_UNPACK_ALIGNMENT, 1);
        while glGetError() != GL_NO_ERROR {}
    }
    OPEN_KICK.store(0, Ordering::Relaxed);
    g
}

/// Texel (x, y, z) of level `l`: its coordinates and level, so no two
/// texels of the volumes here are the same.
fn texel(l: u32, x: u32, y: u32, z: u32) -> [u8; 4] {
    [x as u8, y as u8, z as u8, 0x80 | l as u8]
}

fn level_dims(w: u32, h: u32, d: u32, l: u32) -> (u32, u32, u32) {
    ((w >> l).max(1), (h >> l).max(1), (d >> l).max(1))
}

fn level_pixels(w: u32, h: u32, d: u32, l: u32) -> Vec<u8> {
    let (lw, lh, ld) = level_dims(w, h, d, l);
    let mut px = Vec::new();
    for z in 0..ld {
        for y in 0..lh {
            for x in 0..lw {
                px.extend_from_slice(&texel(l, x, y, z));
            }
        }
    }
    px
}

/// A `w x h x d` RGBA8 volume with its first `levels` levels uploaded
/// (texels [`texel`]): glTexStorage3D with `storage`, else glTexImage3D per
/// level; the min filter `min`. Left bound to GL_TEXTURE_3D.
fn volume(w: u32, h: u32, d: u32, levels: u32, storage: bool, min: GLenum) -> GLuint {
    let mut name = 0;
    // SAFETY: room for one name; each level's pixels hold its texels.
    unsafe {
        glGenTextures(1, &mut name);
        glBindTexture(GL_TEXTURE_3D, name);
        if storage {
            glTexStorage3D(GL_TEXTURE_3D, levels as GLsizei, GL_RGBA8, w as GLsizei, h as GLsizei, d as GLsizei);
        }
        for l in 0..levels {
            let (lw, lh, ld) = level_dims(w, h, d, l);
            let px = level_pixels(w, h, d, l);
            if storage {
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
                    px.as_ptr() as *const c_void,
                );
            } else {
                glTexImage3D(
                    GL_TEXTURE_3D,
                    l as GLint,
                    GL_RGBA8 as GLint,
                    lw as GLsizei,
                    lh as GLsizei,
                    ld as GLsizei,
                    0,
                    GL_RGBA,
                    GL_UNSIGNED_BYTE,
                    px.as_ptr() as *const c_void,
                );
            }
        }
        glTexParameteri(GL_TEXTURE_3D, GL_TEXTURE_MIN_FILTER, min as GLint);
        assert_eq!(glGetError(), GL_NO_ERROR, "{w}x{h}x{d}, {levels} levels");
    }
    name
}

fn param_i(pname: GLenum, v: GLint) {
    // SAFETY: a texture is bound to GL_TEXTURE_3D.
    unsafe {
        glTexParameteri(GL_TEXTURE_3D, pname, v);
        assert_eq!(glGetError(), GL_NO_ERROR);
    }
}

fn param_f(pname: GLenum, v: f32) {
    // SAFETY: a texture is bound to GL_TEXTURE_3D.
    unsafe {
        glTexParameterf(GL_TEXTURE_3D, pname, v);
        assert_eq!(glGetError(), GL_NO_ERROR);
    }
}

fn delete(name: GLuint) {
    // SAFETY: a name this file made.
    unsafe { glDeleteTextures(1, &name) };
}

/// The GL core's object for `name`.
fn tex(name: GLuint) -> &'static GlTexture {
    let c = context::current().expect("a current context");
    let slot = names::slot_of(c, Kind::Texture, name).expect("a texture name");
    let t = c.texture_object(slot).expect("a texture object");
    assert_ne!(t.grid_cols, 0, "the PowerVR's grid of slices");
    // SAFETY: the context outlives every test (it is never destroyed), and
    // each test reads its texture before it changes it again.
    unsafe { &*(t as *const GlTexture) }
}

/// What the draw does for a native 3D binding of `t` (gl::draw, the
/// `sh.native_cube & 4` arm): the copy of [`sampled_levels`] levels, and the
/// bind with the copy's address and bytes, level 0's size, the layers, the
/// level count and the mip filter from the texture's min filter.
fn bind(t: &GlTexture) -> (GlesTextureBind, VolView) {
    let levels = sampled_levels(t);
    let (gpu, bytes) = native_vol::copy(t, 0, levels).expect("a copy");
    let v = view(gpu).expect("the copy's view");
    let lin = |f: GLenum| matches!(f, GL_LINEAR | GL_LINEAR_MIPMAP_NEAREST | GL_LINEAR_MIPMAP_LINEAR) as u8;
    let b = GlesTextureBind {
        format: GLES_TEXFMT_RGBA8,
        type_: GLES_TEXTYPE_3D_TWIDDLED,
        layers: t.layers,
        base_gpu: gpu,
        width: t.width,
        height: t.height,
        stride_texels: t.width,
        bytes,
        linear_filter: (t.mag_filter == GL_LINEAR) as u8,
        min_linear: lin(t.min_filter),
        mip_levels: levels as u8,
        mip_filter: match t.min_filter {
            _ if levels < 2 => 0,
            GL_NEAREST_MIPMAP_LINEAR | GL_LINEAR_MIPMAP_LINEAR => 2,
            _ => 1,
        },
        wrap_s: GLES_WRAP_REPEAT,
        wrap_t: GLES_WRAP_REPEAT,
        wrap_r: GLES_WRAP_REPEAT,
        ..GlesTextureBind::ZERO
    };
    (b, v)
}

/// The image words of a descriptor (texstate.xml IMAGE_WORD0 and the
/// twiddled IMAGE_WORD1): (width - 1, height - 1, depth - 1, num_mip_levels,
/// mipmaps_present).
fn image(w: &[u32; 20]) -> (u64, u64, u64, u64, u64) {
    let iw0 = w[0] as u64 | (w[1] as u64) << 32;
    let iw1 = w[2] as u64 | (w[3] as u64) << 32;
    ((iw0 >> 34) & 0x3fff, (iw0 >> 48) & 0x3fff, (iw1 >> 4) & 0x7ff, iw1 & 0xf, (iw1 >> 15) & 1)
}

/// The sampler word's LOD clamp and mip filter (texstate.xml SAMPLER_WORD0):
/// (minlod, maxlod, mipfilter), u4.6.
fn lod(w: &[u32; 20]) -> (u64, u64, u64) {
    let q = w[8] as u64 | (w[9] as u64) << 32;
    ((q >> 13) & 0x3ff, (q >> 23) & 0x3ff, (q >> 40) & 1)
}

fn desc(b: &GlesTextureBind) -> [u32; 20] {
    let w = texture_descriptor(b);
    w[..20].try_into().unwrap()
}

/// The copy at `gpu` holds levels `base ..` of a `w x h x d` volume as its
/// levels `0 ..`: texel (x, y, z) of GL level base + k at the copy's level k
/// offset plus the twiddle over the base extent's level k.
fn assert_copy_holds(gpu: u64, (w, h, d): (u32, u32, u32), base: u32, levels: u32) {
    let ext = level_dims(w, h, d, base);
    let phys = physical_extent(ext.0, ext.1, ext.2);
    let bytes = copy_bytes(phys, 4, levels);
    // SAFETY: the host OSAL's GPU address is its CPU pointer, `bytes` long.
    let out = unsafe { core::slice::from_raw_parts(gpu as *const u8, bytes as usize) };
    for k in 0..levels {
        let te = twiddle_extent(ext.0, ext.1, ext.2, k);
        assert_eq!(te, level_dims(w, h, d, base + k), "the copy's level {k} is GL level {}", base + k);
        for z in 0..te.2 {
            for y in 0..te.1 {
                for x in 0..te.0 {
                    let at = ((level_texel_off(phys, k) + twiddle3d(x, y, z, te)) * 4) as usize;
                    assert_eq!(
                        out[at..at + 4],
                        texel(base + k, x, y, z),
                        "base {base}: copy level {k} ({x}, {y}, {z}) is GL level {}'s",
                        base + k
                    );
                }
            }
        }
    }
}

/// GL_TEXTURE_BASE_LEVEL: the copy's level 0 is the base level, with its
/// extent, and holds the levels from it to the end of the chain; the
/// descriptor says the base level's size and depth, the copy's level count,
/// and the LOD range over those levels. Every base of a NPOT 20x12x9
/// volume (five levels), with a mipmap filter and without one; an immutable
/// texture's base past its last level is its last level (3.8.10).
#[test]
fn the_base_level_is_the_copys_level_0() {
    let _g = gl_context();
    let size = (20u32, 12u32, 9u32);
    let name = volume(size.0, size.1, size.2, 5, false, GL_LINEAR_MIPMAP_LINEAR);
    for base in 0..5u32 {
        param_i(GL_TEXTURE_BASE_LEVEL, base as GLint);
        let t = tex(name);
        let n = 5 - base;
        assert_eq!(level_range(t, sampled_levels(t)), (base, n), "base {base}");
        let (b, v) = bind(t);
        let ext = level_dims(size.0, size.1, size.2, base);
        assert_eq!(v, VolView { extent: ext, base, levels: n, min_lod: -1000.0, max_lod: 1000.0 });
        assert_eq!(b.bytes, copy_bytes(physical_extent(ext.0, ext.1, ext.2), 4, n), "base {base}: the copy's size");
        assert_copy_holds(b.base_gpu, size, base, n);
        let w = desc(&b);
        assert_eq!(
            image(&w),
            (u64::from(ext.0 - 1), u64::from(ext.1 - 1), u64::from(ext.2 - 1), u64::from(n), u64::from(n > 1)),
            "base {base}: the base level's extent and the copy's levels"
        );
        assert_eq!(lod(&w), (0, u64::from(n - 1) * 64, u64::from(n > 1)), "base {base}: LOD 0 to the last level");
    }
    /* no mipmap filter: the base level alone (3.8.11) */
    param_i(GL_TEXTURE_MIN_FILTER, GL_LINEAR as GLint);
    param_i(GL_TEXTURE_BASE_LEVEL, 2);
    let t = tex(name);
    assert_eq!(level_range(t, sampled_levels(t)), (2, 1));
    let (b, v) = bind(t);
    assert_eq!((v.extent, v.levels), ((5, 3, 2), 1));
    assert_copy_holds(b.base_gpu, size, 2, 1);
    let w = desc(&b);
    assert_eq!(image(&w), (4, 2, 1, 1, 0));
    assert_eq!(lod(&w), (0, 0, 0));
    delete(name);

    /* immutable: the base is clamped to the last of its three levels */
    let name = volume(16, 16, 16, 3, true, GL_NEAREST_MIPMAP_NEAREST);
    param_i(GL_TEXTURE_BASE_LEVEL, 7);
    let t = tex(name);
    assert_eq!(level_range(t, sampled_levels(t)), (2, 1));
    let (b, v) = bind(t);
    assert_eq!((v.extent, v.base, v.levels), ((4, 4, 4), 2, 1));
    assert_copy_holds(b.base_gpu, (16, 16, 16), 2, 1);
    delete(name);
}

/// GL_TEXTURE_MAX_LEVEL: the copy stops at it, counted from the base level
/// (q = min(p, level_max), 3.8.14), and the descriptor's level count and
/// maximum LOD with it. A maximum below the base leaves the base level
/// alone; an immutable texture's maximum is at least its base (3.8.10).
#[test]
fn the_max_level_ends_the_copy() {
    let _g = gl_context();
    let size = (16u32, 16u32, 16u32);
    let name = volume(size.0, size.1, size.2, 5, false, GL_NEAREST_MIPMAP_LINEAR);
    for (base, max, want) in [(0, 0, 1), (0, 2, 3), (0, 4, 5), (0, 9, 5), (1, 3, 3), (3, 15, 2), (2, 1, 1), (4, 4, 1)] {
        param_i(GL_TEXTURE_BASE_LEVEL, base);
        param_i(GL_TEXTURE_MAX_LEVEL, max);
        let t = tex(name);
        let (b, v) = bind(t);
        assert_eq!((v.base, v.levels), (base as u32, want), "base {base} max {max}");
        assert_copy_holds(b.base_gpu, size, base as u32, want);
        let w = desc(&b);
        let side = u64::from(16u32 >> base) - 1;
        assert_eq!(image(&w), (side, side, side, u64::from(want), u64::from(want > 1)), "base {base} max {max}");
        /* one level: the minification nudge (NEAREST_MIPMAP_LINEAR under
         * LINEAR magnification), LOD 1.0 at most, still level 0 */
        let top = if want > 1 { u64::from(want - 1) * 64 } else { 64 };
        assert_eq!(lod(&w).1, top, "base {base} max {max}: LOD to the maximum level");
    }
    delete(name);

    let name = volume(16, 16, 16, 5, true, GL_LINEAR_MIPMAP_LINEAR);
    param_i(GL_TEXTURE_BASE_LEVEL, 3);
    param_i(GL_TEXTURE_MAX_LEVEL, 1);
    let t = tex(name);
    assert_eq!(level_range(t, sampled_levels(t)), (3, 1), "immutable: the maximum is clamped up to the base");
    param_i(GL_TEXTURE_MAX_LEVEL, 3);
    param_i(GL_TEXTURE_BASE_LEVEL, 1);
    let t = tex(name);
    assert_eq!(level_range(t, sampled_levels(t)), (1, 3));
    delete(name);
}

/// GL_TEXTURE_MIN_LOD: the sampler's minimum LOD, u4.6 (64 per level),
/// counted from the base level as the copy's LOD is; 0 for a negative one
/// and the GL default (-1000); never above the maximum (the last level's
/// LOD), so a minimum past the chain samples the last level.
#[test]
fn the_min_lod_is_the_samplers_minimum() {
    let _g = gl_context();
    let name = volume(16, 16, 16, 5, false, GL_LINEAR_MIPMAP_LINEAR);
    for (min_lod, want) in [(-1000.0f32, 0u64), (-0.5, 0), (0.0, 0), (0.5, 32), (1.5, 96), (2.25, 144), (10.0, 256)] {
        param_f(GL_TEXTURE_MIN_LOD, min_lod);
        let t = tex(name);
        let (b, v) = bind(t);
        assert_eq!(v.min_lod, min_lod);
        assert_eq!(lod(&desc(&b)), (want, 4 * 64, 1), "min_lod {min_lod}");
    }
    /* from the base level: base 2 leaves three levels, LOD 0 to 2 */
    param_i(GL_TEXTURE_BASE_LEVEL, 2);
    for (min_lod, want) in [(1.0f32, 64u64), (5.0, 128)] {
        param_f(GL_TEXTURE_MIN_LOD, min_lod);
        let (b, _) = bind(tex(name));
        assert_eq!(lod(&desc(&b)), (want, 2 * 64, 1), "base 2, min_lod {min_lod}");
    }
    delete(name);
}

/// GL_TEXTURE_MAX_LOD: the sampler's maximum LOD cut down to it, u4.6; the
/// minimum follows it down. Without a mipmap filter the one-level copy's
/// minification nudge (maximum LOD 1.0, `tex_desc_sampler`) is cut too, so
/// a maximum LOD of 0 magnifies everywhere, as GL's clamped lambda does.
#[test]
fn the_max_lod_is_the_samplers_maximum() {
    let _g = gl_context();
    let name = volume(16, 16, 16, 5, false, GL_LINEAR_MIPMAP_NEAREST);
    for (max_lod, want) in [(1000.0f32, 256u64), (4.0, 256), (2.5, 160), (1.0, 64), (0.25, 16), (0.0, 0), (-3.0, 0)] {
        param_f(GL_TEXTURE_MAX_LOD, max_lod);
        let t = tex(name);
        let (b, v) = bind(t);
        assert_eq!(v.max_lod, max_lod);
        assert_eq!(lod(&desc(&b)), (0, want, 0), "max_lod {max_lod}");
    }
    param_f(GL_TEXTURE_MIN_LOD, 2.0);
    param_f(GL_TEXTURE_MAX_LOD, 1.0);
    let (b, _) = bind(tex(name));
    assert_eq!(lod(&desc(&b)), (64, 64, 0), "min_lod 2 over max_lod 1: both at 1");
    /* one level, NEAREST minification under LINEAR magnification */
    param_i(GL_TEXTURE_MIN_FILTER, GL_NEAREST as GLint);
    param_f(GL_TEXTURE_MIN_LOD, -1000.0);
    for (max_lod, want) in [(1000.0f32, 64u64), (0.5, 32), (0.0, 0)] {
        param_f(GL_TEXTURE_MAX_LOD, max_lod);
        let (b, _) = bind(tex(name));
        assert_eq!(lod(&desc(&b)), (0, want, 0), "one level, max_lod {max_lod}");
    }
    delete(name);
}

/// The LOD range is the texture's at every bind, whether the copy is made
/// or kept: changing min or max LOD makes no copy.
#[test]
fn the_lod_range_follows_the_texture_without_a_new_copy() {
    let _g = gl_context();
    let name = volume(8, 8, 8, 4, false, GL_LINEAR_MIPMAP_LINEAR);
    let (b0, _) = bind(tex(name));
    let n0 = native_vol::copies();
    param_f(GL_TEXTURE_MIN_LOD, 1.0);
    param_f(GL_TEXTURE_MAX_LOD, 2.0);
    let (b1, v) = bind(tex(name));
    assert_eq!(native_vol::copies(), n0, "the kept copy");
    assert_eq!(b1.base_gpu, b0.base_gpu);
    assert_eq!((v.min_lod, v.max_lod), (1.0, 2.0));
    assert_eq!(lod(&desc(&b1)), (64, 128, 1));
    delete(name);
}

/// A frame of dEQP's base_level and max_level grids binds one volume with a
/// base or maximum level per cell, one draw each: every (base, levels)
/// keeps a copy of its own, so binding the next cell's does not rewrite the
/// copy an earlier draw of the frame reads, and binding a cell's again
/// finds it.
#[test]
fn each_base_and_max_level_keeps_its_own_copy() {
    let _g = gl_context();
    let size = (16u32, 8u32, 16u32);
    let name = volume(size.0, size.1, size.2, 5, false, GL_LINEAR_MIPMAP_LINEAR);
    let n0 = native_vol::copies();
    let mut cells = Vec::new();
    for (base, max) in [(0, 15), (1, 15), (2, 15), (0, 1), (0, 2), (3, 15)] {
        param_i(GL_TEXTURE_BASE_LEVEL, base);
        param_i(GL_TEXTURE_MAX_LEVEL, max);
        let (b, v) = bind(tex(name));
        cells.push((base, max, b.base_gpu, v.levels));
    }
    assert_eq!(native_vol::copies() - n0, 6, "a copy per cell");
    for (i, (_, _, gpu, _)) in cells.iter().enumerate() {
        assert!(cells[..i].iter().all(|c| c.2 != *gpu), "cell {i} has an address of its own");
    }
    for &(base, max, gpu, levels) in &cells {
        param_i(GL_TEXTURE_BASE_LEVEL, base);
        param_i(GL_TEXTURE_MAX_LEVEL, max);
        let (b, _) = bind(tex(name));
        assert_eq!(b.base_gpu, gpu, "base {base} max {max}: the cell's copy again");
        assert_copy_holds(gpu, size, base as u32, levels);
    }
    assert_eq!(native_vol::copies() - n0, 6, "no copy made again");
    delete(name);
}

/// A copy bound in the kick being recorded is not written again when the
/// texels change: the new texels go to another copy, and the old one keeps
/// what the recorded draw reads. Once its kick has run, the copy is the one
/// written again.
#[test]
fn a_copy_a_recorded_draw_binds_is_not_written_again() {
    let _g = gl_context();
    let name = volume(4, 4, 4, 1, false, GL_NEAREST);
    OPEN_KICK.store(5, Ordering::Relaxed);
    let (a, _) = bind(tex(name));
    let px = [9u8, 9, 9, 255];
    // SAFETY: one RGBA8 texel of slice 1.
    unsafe {
        glTexSubImage3D(GL_TEXTURE_3D, 0, 1, 1, 1, 1, 1, 1, GL_RGBA, GL_UNSIGNED_BYTE, px.as_ptr() as *const c_void);
    }
    let (b, _) = bind(tex(name));
    assert_ne!(b.base_gpu, a.base_gpu, "kick 5 binds the first copy: the new texels go elsewhere");
    let at = (twiddle3d(1, 1, 1, (4, 4, 4)) * 4) as usize;
    // SAFETY: both copies are 4x4x4 RGBA8 host memory.
    let (old, new) = unsafe {
        (
            core::slice::from_raw_parts(a.base_gpu as *const u8, 256 * 4),
            core::slice::from_raw_parts(b.base_gpu as *const u8, 256 * 4),
        )
    };
    assert_eq!(old[at..at + 4], texel(0, 1, 1, 1), "the recorded draw's texels");
    assert_eq!(new[at..at + 4], px);
    /* kick 5 and the one after it have run: the copy is written in place */
    OPEN_KICK.store(9, Ordering::Relaxed);
    let px2 = [7u8, 7, 7, 255];
    // SAFETY: as above.
    unsafe {
        glTexSubImage3D(GL_TEXTURE_3D, 0, 1, 1, 1, 1, 1, 1, GL_RGBA, GL_UNSIGNED_BYTE, px2.as_ptr() as *const c_void);
    }
    let (c, _) = bind(tex(name));
    assert_eq!(c.base_gpu, b.base_gpu, "the copy no draw reads any more");
    // SAFETY: as above.
    let now = unsafe { core::slice::from_raw_parts(c.base_gpu as *const u8, 256 * 4) };
    assert_eq!(now[at..at + 4], px2);
    OPEN_KICK.store(0, Ordering::Relaxed);
    delete(name);
}

/// Native 3D off: the program samples the grid (the record does not ask for
/// the copy, so the draw never calls `native_vol`), and the descriptors the
/// off path builds -- the grid's stride image and a 2D image -- are the same
/// bytes whether or not a copy with a base level and a LOD range lives at
/// their address. A native bind at default parameters (base 0, LOD -1000 to
/// 1000) describes what it described before too: the same words with its
/// view as without one.
#[test]
fn off_nothing_changes() {
    let src = "#version 300 es\nprecision highp float;\nuniform highp sampler3D s;\nuniform vec3 c;\n\
               out vec4 o;\nvoid main() { o = texture(s, c) + textureLod(s, c, 1.0); }\n";
    let record = |on: bool| {
        yakogl::shaderc::pvr_set_native_3d(i32::from(on));
        let r = store::with_scratch(|a| {
            let _keep = store::begin_keep().expect("keep");
            let text = a.alloc_str(src).unwrap();
            let l = lower::compile_to_ir(a, Stage::Fragment, text).expect("front end");
            compile_fragment(&l.module, &l.reflection, a).map(|rec| rec.native_cube)
        })
        .unwrap();
        yakogl::shaderc::pvr_set_native_3d(yakogl::shaderc::PVR_NATIVE_3D_DEFAULT);
        r.expect("compiles")
    };
    assert_eq!(record(false) & 4, 0, "off: the grid, no copy asked for");
    assert_eq!(record(true) & 4, 4, "on: the copy");

    let _g = gl_context();
    let name = volume(8, 8, 8, 4, false, GL_LINEAR_MIPMAP_LINEAR);
    let (native, v) = bind(tex(name));
    let with_default = desc(&native);
    param_i(GL_TEXTURE_BASE_LEVEL, 1);
    param_f(GL_TEXTURE_MIN_LOD, 0.5);
    param_f(GL_TEXTURE_MAX_LOD, 1.5);
    let (lodded, _) = bind(tex(name));
    let grid = GlesTextureBind { type_: GLES_TEXTYPE_3D, height: 8 * 8, mip_levels: 1, mip_filter: 0, ..lodded };
    let flat = GlesTextureBind { type_: GLES_TEXTYPE_2D, ..grid };
    assert!(view(lodded.base_gpu).is_some_and(|v| v.base == 1 && v.min_lod == 0.5));
    let (grid_with, flat_with) = (desc(&grid), desc(&flat));
    let t = tex(name);
    native_vol::released(t.mem.gpu);
    assert_eq!(view(lodded.base_gpu), None, "released: no copy at the address");
    assert_eq!(view(native.base_gpu), None);
    assert_eq!(desc(&grid), grid_with, "the grid's descriptor");
    assert_eq!(desc(&flat), flat_with, "a 2D descriptor");
    assert_eq!(lod(&grid_with), (0, 0, 0), "the grid samples level 0, no LOD range");
    assert_eq!(desc(&native), with_default, "native, default parameters: as without a view");
    assert_eq!((v.base, v.levels), (0, 4));
    delete(name);
}
