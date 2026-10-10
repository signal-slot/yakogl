// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The texels of mip levels above 0 of 3D and 2D-array textures (plan item
//! tex3d-finish): each level is given its own colours through the GL entry
//! points -- glTexImage3D, glTexStorage3D + glTexSubImage3D,
//! glCopyTexSubImage3D and glGenerateMipmap -- and every texel of every
//! level is read back, from the storage where the GL core says it is
//! (`texture::level_texel_offset`) and, where a level can be a framebuffer
//! attachment, through glFramebufferTextureLayer + glReadPixels, whose
//! arithmetic is framebuffer.rs's own.
//!
//! Two stand-in back ends, switched per test: a PowerVR-shaped one whose
//! shader addresses a 3D texture's slices in a grid (`native_layered_textures`
//! false) and a V3D/GC7000-shaped one whose texture unit walks stacked
//! slices and layers itself (true). On the first a 3D texture's draw is
//! still told of level 0 alone (`levels` 1, the levels' texels held after
//! it); a non-power-of-two 2D array there keeps no texels above level 0
//! (the PowerVR's tall stacked NPOT layers, plan item pvr-2darray), which
//! `a_powervr_npot_array_records_its_levels_only` holds.
//!
//! The level-validity rules (every level of a NPOT array or storage
//! texture is accepted) are tested next to the code, src/gl/es3_level_tests.rs.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr --test es3_tex_levels
//! ```

#![cfg(feature = "host-test")]

use core::ffi::{c_char, c_void};
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{GlesDraw, GlesHeap, YakoglBackend};
use yakogl::gl::consts::*;
use yakogl::gl::context;
use yakogl::gl::framebuffer::*;
use yakogl::gl::state::*;
use yakogl::gl::texture::*;
use yakogl::gl::types::*;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};

/* ---- a context on the host: the smallest OSAL and back ends that make one -- */

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
    lock: None,
    unlock: None,
    thread_id: None,
};

/// A back end of the given texture layout: `native_layered_textures` and
/// `mip_level_align` are what the GL core lays a chain out by.
const fn backend(native_layered_textures: bool, mip_level_align: u32) -> YakoglBackend {
    YakoglBackend {
        name: c"es3-tex-levels".as_ptr() as *const c_char,
        native_attrib_formats: 0,
        rt_rgba8888_is_bgra: false,
        mip_level_align,
        compressed_blocks_in_rows: false,
        native_layered_textures,
        front_facing_shader_variant: false,
        etc1_decoded_at_upload: false,
        init: Some(b_init),
        shutdown: None,
        draw: Some(b_draw),
        flush: Some(b_ok),
        finish: Some(b_ok),
        finish_previous: None,
        kick_id: None,
        kick_pending: None,
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
    }
}

/// The PowerVR's layout: a 3D texture's slices in a grid, layers stacked
/// as one tall image, a packed chain.
static GRID: YakoglBackend = backend(false, 0);
/// The V3D's: stacked slices and layers the texture unit walks, a packed
/// chain at each level's own pitch.
static LAYERED: YakoglBackend = backend(true, 0);
/// The GC7000's: stacked, every level at level 0's pitch on a 64-byte
/// boundary.
static LAYERED_ALIGNED: YakoglBackend = backend(true, 64);

/// One context for the whole file; the tests take turns on it.
static LOCK: Mutex<()> = Mutex::new(());
static ONCE: Once = Once::new();

fn context_with(b: &'static YakoglBackend) -> MutexGuard<'static, ()> {
    let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    ONCE.call_once(|| {
        // SAFETY: both tables are statics; the lock above serialises the tests.
        unsafe {
            osal_set_ops(&OSAL);
            yakogl::backend::g_backend = &GRID;
            assert!(!context::gl_context_create(8, 8).is_null());
        }
    });
    // SAFETY: a static table; the lock serialises the tests.
    unsafe { yakogl::backend::g_backend = b };
    // SAFETY: plain GL calls on the one context.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glPixelStorei(GL_UNPACK_ALIGNMENT, 1);
        glPixelStorei(GL_PACK_ALIGNMENT, 1);
        while glGetError() != GL_NO_ERROR {}
    }
    g
}

static BACKENDS: [(&str, &YakoglBackend); 3] =
    [("grid", &GRID), ("layered", &LAYERED), ("layered, aligned", &LAYERED_ALIGNED)];

fn err() -> GLenum {
    // SAFETY: plain GL call.
    unsafe { glGetError() }
}

fn bind_new(target: GLenum) -> GLuint {
    let mut name: GLuint = 0;
    // SAFETY: room for one name; plain GL calls.
    unsafe {
        glGenTextures(1, &mut name);
        glBindTexture(target, name);
    }
    name
}

/// The texel a test gives (x, y) of slice/layer z of level l: every level,
/// slice, row and column its own bytes.
fn colour(l: u32, x: u32, y: u32, z: u32) -> [u8; 4] {
    [(0x10 * l + z) as u8, x as u8, y as u8, (0xa0 + l) as u8]
}

/// `colour` over a w x h x d box at (x0, y0, z0) of level l, in the order
/// glTex*Image3D reads it.
fn image(l: u32, (x0, y0, z0): (u32, u32, u32), (w, h, d): (u32, u32, u32)) -> Vec<u8> {
    let mut v = Vec::new();
    for z in 0..d {
        for y in 0..h {
            for x in 0..w {
                v.extend(colour(l, x0 + x, y0 + y, z0 + z));
            }
        }
    }
    v
}

/// GL's size of level l (ES 3.0 3.8.14); `depth_halves`: a 3D texture's.
fn level_dims((w, h, d): (u32, u32, u32), l: u32, depth_halves: bool) -> (u32, u32, u32) {
    let s = |v: u32| (v >> l).max(1);
    (s(w), s(h), if depth_halves { s(d) } else { d })
}

/// GL's level count of a chain (log2 of the largest side + 1).
fn level_count(sides: u32) -> u32 {
    sides.max(1).ilog2() + 1
}

/// The four bytes the storage holds for texel (x, y, z) of level l of
/// texture `name`, or None where the GL core keeps no texels.
fn stored(name: GLuint, l: u32, x: u32, y: u32, z: u32) -> Option<[u8; 4]> {
    let c = context::current().expect("a current context");
    let t = c.texture_ref(name).expect("a texture");
    let off = level_texel_offset(t, l, x, y, z)?;
    assert!(off + 4 <= t.mem.size, "texel ({x}, {y}, {z}) of level {l} past the storage");
    // SAFETY: four bytes inside the texture's CPU mapping (checked above).
    Some(unsafe { *((t.mem.cpu as *const u8).add(off) as *const [u8; 4]) })
}

/// Every texel of level l (w x h x d) of `name` reads `colour`.
fn assert_level(what: &str, name: GLuint, l: u32, (w, h, d): (u32, u32, u32)) {
    for z in 0..d {
        for y in 0..h {
            for x in 0..w {
                assert_eq!(
                    stored(name, l, x, y, z),
                    Some(colour(l, x, y, z)),
                    "{what}: level {l} texel ({x}, {y}, {z})"
                );
            }
        }
    }
}

/// Level l, layer/slice z of `name` attached to a framebuffer and read back
/// with glReadPixels: what framebuffer.rs's own layout arithmetic finds.
fn read_layer(name: GLuint, l: u32, z: u32, (w, h): (u32, u32)) -> Vec<u8> {
    let mut fb: GLuint = 0;
    let mut px = vec![0u8; (w * h * 4) as usize];
    // SAFETY: `px` holds w x h RGBA8 pixels at a pack alignment of 1.
    unsafe {
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        glFramebufferTextureLayer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, name, l as GLint, z as GLint);
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE, "level {l} layer {z}");
        glReadPixels(0, 0, w as GLsizei, h as GLsizei, GL_RGBA, GL_UNSIGNED_BYTE, px.as_mut_ptr() as *mut c_void);
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glDeleteFramebuffers(1, &fb);
    }
    assert_eq!(err(), GL_NO_ERROR, "reading level {l} layer {z}");
    px
}

/// glTexImage3D of every level of a `size` texture of `target`, each from
/// its own colours.
fn specify_every_level(target: GLenum, size: (u32, u32, u32)) -> u32 {
    let is_3d = target == GL_TEXTURE_3D;
    let n = level_count(if is_3d { size.0.max(size.1).max(size.2) } else { size.0.max(size.1) });
    for l in 0..n {
        let (w, h, d) = level_dims(size, l, is_3d);
        let px = image(l, (0, 0, 0), (w, h, d));
        // SAFETY: `px` holds the level's w x h x d RGBA8 texels.
        unsafe {
            glTexImage3D(
                target,
                l as GLint,
                GL_RGBA8 as GLint,
                w as GLsizei,
                h as GLsizei,
                d as GLsizei,
                0,
                GL_RGBA,
                GL_UNSIGNED_BYTE,
                px.as_ptr() as *const c_void,
            )
        };
        assert_eq!(err(), GL_NO_ERROR, "glTexImage3D level {l} {w}x{h}x{d}");
    }
    n
}

/* ------------------------------------------------------------- 3D textures -- */

/// glTexImage3D at every level of a power-of-two and a non-power-of-two 3D
/// texture: each level reads back its own colours, on every layout. The
/// draw samples a stacked chain's levels; a grid's draw level 0 alone.
#[test]
fn every_level_of_a_3d_texture_keeps_its_texels() {
    for (what, b) in BACKENDS {
        let _g = context_with(b);
        for size in [(8u32, 8u32, 4u32), (7, 5, 3), (4, 2, 4)] {
            let name = bind_new(GL_TEXTURE_3D);
            let n = specify_every_level(GL_TEXTURE_3D, size);
            for l in 0..n {
                assert_level(&format!("{what} {size:?}"), name, l, level_dims(size, l, true));
            }
            let t = context::current().unwrap().texture_ref(name).unwrap();
            let want = if t.grid_cols != 0 { 1 } else { n as u8 };
            assert_eq!((t.levels, t.level_defined, t.level_bad), (want, (1 << n) - 1, 0), "{what} {size:?}");
        }
    }
}

/// glTexStorage3D, then a sub-image into the middle of every level: the
/// box lands where it was put and the rest of the level is untouched.
#[test]
fn sub_images_reach_every_level_of_a_3d_storage_texture() {
    for (what, b) in BACKENDS {
        let _g = context_with(b);
        for size in [(16u32, 8u32, 8u32), (9, 6, 5)] {
            let name = bind_new(GL_TEXTURE_3D);
            let n = level_count(size.0.max(size.1).max(size.2));
            // SAFETY: plain GL call.
            unsafe {
                glTexStorage3D(
                    GL_TEXTURE_3D,
                    n as GLsizei,
                    GL_RGBA8,
                    size.0 as GLsizei,
                    size.1 as GLsizei,
                    size.2 as GLsizei,
                )
            };
            assert_eq!(err(), GL_NO_ERROR, "{what} storage {size:?}");
            for l in 0..n {
                let (w, h, d) = level_dims(size, l, true);
                /* the whole level first, then a box from (1, 1, 1) where it fits */
                let px = image(l, (0, 0, 0), (w, h, d));
                // SAFETY: `px` holds the level's texels.
                unsafe {
                    glTexSubImage3D(
                        GL_TEXTURE_3D,
                        l as GLint,
                        0,
                        0,
                        0,
                        w as GLsizei,
                        h as GLsizei,
                        d as GLsizei,
                        GL_RGBA,
                        GL_UNSIGNED_BYTE,
                        px.as_ptr() as *const c_void,
                    )
                };
                assert_eq!(err(), GL_NO_ERROR, "{what} {size:?} level {l}");
                assert_level(&format!("{what} {size:?}"), name, l, (w, h, d));
                if w < 3 || h < 3 || d < 3 {
                    continue;
                }
                let boxed = vec![0xeeu8; ((w - 2) * (h - 2) * (d - 2) * 4) as usize];
                // SAFETY: `boxed` holds (w-2) x (h-2) x (d-2) texels.
                unsafe {
                    glTexSubImage3D(
                        GL_TEXTURE_3D,
                        l as GLint,
                        1,
                        1,
                        1,
                        (w - 2) as GLsizei,
                        (h - 2) as GLsizei,
                        (d - 2) as GLsizei,
                        GL_RGBA,
                        GL_UNSIGNED_BYTE,
                        boxed.as_ptr() as *const c_void,
                    )
                };
                assert_eq!(err(), GL_NO_ERROR);
                for z in 0..d {
                    for y in 0..h {
                        for x in 0..w {
                            let inside = (1..w - 1).contains(&x) && (1..h - 1).contains(&y) && (1..d - 1).contains(&z);
                            let want = if inside { [0xee; 4] } else { colour(l, x, y, z) };
                            assert_eq!(
                                stored(name, l, x, y, z),
                                Some(want),
                                "{what} {size:?} level {l} ({x}, {y}, {z})"
                            );
                        }
                    }
                }
            }
            /* a level's depth is its own: one slice past it is refused */
            let (w, h, d) = level_dims(size, 1, true);
            let px = vec![0u8; (w * h * 4) as usize];
            // SAFETY: `px` holds one slice of level 1.
            unsafe {
                glTexSubImage3D(
                    GL_TEXTURE_3D,
                    1,
                    0,
                    0,
                    d as GLint,
                    w as GLsizei,
                    h as GLsizei,
                    1,
                    GL_RGBA,
                    GL_UNSIGNED_BYTE,
                    px.as_ptr() as *const c_void,
                )
            };
            assert_eq!(err(), GL_INVALID_VALUE, "{what} {size:?}: slice {d} of level 1");
        }
    }
}

/// Where the slices are stacked, a level above 0 is a framebuffer
/// attachment: glReadPixels of each slice of each level finds what
/// glTexImage3D put there. (Stacked at each level's own pitch, the V3D's
/// layout: a level at level 0's wider pitch, the GC7000's, is no render
/// target, framebuffer.rs `attachment_storage`.)
#[test]
fn stacked_3d_levels_read_back_through_a_framebuffer() {
    let _g = context_with(&LAYERED);
    let size = (8u32, 4u32, 4u32);
    let name = bind_new(GL_TEXTURE_3D);
    let n = specify_every_level(GL_TEXTURE_3D, size);
    for l in 0..n {
        let (w, h, d) = level_dims(size, l, true);
        for z in 0..d {
            assert_eq!(read_layer(name, l, z, (w, h)), image(l, (0, 0, z), (w, h, 1)), "level {l} slice {z}");
        }
    }
}

/// A stacked chain is a 2D array's and stops where the sides reach 1x1;
/// GL's 3D chain goes on while the depth does. Those levels are accepted
/// and recorded, their texels dropped (no back end has them).
#[test]
fn a_3d_level_past_a_stacked_chain_is_recorded_only() {
    let _g = context_with(&LAYERED);
    let name = bind_new(GL_TEXTURE_3D);
    let n = specify_every_level(GL_TEXTURE_3D, (2, 2, 8));
    assert_eq!(n, 4, "2x2x8: 2x2x8, 1x1x4, 1x1x2, 1x1x1");
    assert_level("held", name, 0, (2, 2, 8));
    assert_level("held", name, 1, (1, 1, 4));
    assert_eq!((stored(name, 2, 0, 0, 0), stored(name, 3, 0, 0, 0)), (None, None));
    let t = context::current().unwrap().texture_ref(name).unwrap();
    assert_eq!((t.levels, t.level_defined, t.level_bad), (2, 0b1111, 0));
    /* a sub-image of such a level is checked, then dropped */
    let two = [0u8; 8];
    // SAFETY: `two` holds two 1x1 slices.
    unsafe {
        glTexSubImage3D(GL_TEXTURE_3D, 2, 0, 0, 0, 1, 1, 2, GL_RGBA, GL_UNSIGNED_BYTE, two.as_ptr() as *const c_void)
    };
    assert_eq!(err(), GL_NO_ERROR);
    // SAFETY: as above.
    unsafe {
        glTexSubImage3D(GL_TEXTURE_3D, 2, 0, 0, 1, 1, 1, 2, GL_RGBA, GL_UNSIGNED_BYTE, two.as_ptr() as *const c_void)
    };
    assert_eq!(err(), GL_INVALID_VALUE, "level 2 is 1x1x2");
}

/// A level of the wrong size or format is recorded as such (the texture is
/// incomplete under a mipmap filter) and leaves the level's texels alone.
#[test]
fn a_3d_level_of_another_size_or_format_is_not_written() {
    for (what, b) in BACKENDS {
        let _g = context_with(b);
        let name = bind_new(GL_TEXTURE_3D);
        specify_every_level(GL_TEXTURE_3D, (4, 4, 4));
        let px = [0x55u8; 2 * 2 * 2 * 4];
        // SAFETY: `px` holds 2x2x2 texels of 4 bytes at most.
        unsafe {
            glTexImage3D(
                GL_TEXTURE_3D,
                1,
                GL_RGBA8 as GLint,
                2,
                2,
                1,
                0,
                GL_RGBA,
                GL_UNSIGNED_BYTE,
                px.as_ptr() as *const c_void,
            );
            assert_eq!(err(), GL_NO_ERROR, "{what}: level 1 is 2x2x2, 2x2x1 is accepted");
        }
        let t = context::current().unwrap().texture_ref(name).unwrap();
        assert_eq!(t.level_bad & 2, 2, "{what}: recorded as another size");
        assert_level(what, name, 1, (2, 2, 2));
        // SAFETY: as above.
        unsafe {
            glTexImage3D(
                GL_TEXTURE_3D,
                1,
                GL_RGB8 as GLint,
                2,
                2,
                2,
                0,
                GL_RGB,
                GL_UNSIGNED_BYTE,
                px.as_ptr() as *const c_void,
            );
            assert_eq!(err(), GL_NO_ERROR);
        }
        let t = context::current().unwrap().texture_ref(name).unwrap();
        assert_eq!(t.level_bad & 2, 2, "{what}: recorded as another format");
        assert_level(what, name, 1, (2, 2, 2));
        /* the right image again: held and written */
        let n = specify_every_level(GL_TEXTURE_3D, (4, 4, 4));
        let t = context::current().unwrap().texture_ref(name).unwrap();
        assert_eq!((t.level_defined, t.level_bad), ((1 << n) - 1, 0), "{what}");
    }
}

/// glGenerateMipmap of a 3D texture: every level is the 2x2x2 box average
/// of the one above it (a side of 1 is not halved), the depth halving too.
#[test]
fn generate_mipmap_filters_a_3d_texture_in_depth_too() {
    for (what, b) in BACKENDS {
        let _g = context_with(b);
        for (w, h) in [(4u32, 4u32), (6, 6)] {
            let name = bind_new(GL_TEXTURE_3D);
            /* slice z is 40 * z in every texel and channel: level 1's slices
             * are (0 + 40) / 2 = 20 and (80 + 120) / 2 = 100, level 2's one
             * slice (20 + 100) / 2 = 60 */
            let d = 4u32;
            let mut px = Vec::new();
            for z in 0..d {
                px.extend(vec![(40 * z) as u8; (w * h * 4) as usize]);
            }
            // SAFETY: `px` holds w x h x d RGBA8 texels.
            unsafe {
                glTexImage3D(
                    GL_TEXTURE_3D,
                    0,
                    GL_RGBA8 as GLint,
                    w as GLsizei,
                    h as GLsizei,
                    d as GLsizei,
                    0,
                    GL_RGBA,
                    GL_UNSIGNED_BYTE,
                    px.as_ptr() as *const c_void,
                );
                glGenerateMipmap(GL_TEXTURE_3D);
            }
            assert_eq!(err(), GL_NO_ERROR, "{what} {w}x{h}x{d}");
            let n = level_count(w.max(d));
            let mut want = [0u8, 40, 80, 120].to_vec();
            for l in 0..n {
                let (lw, lh, ld) = level_dims((w, h, d), l, true);
                for z in 0..ld {
                    for y in 0..lh {
                        for x in 0..lw {
                            let v = want[z as usize];
                            assert_eq!(
                                stored(name, l, x, y, z),
                                Some([v; 4]),
                                "{what} {w}x{h}: level {l} ({x}, {y}, {z})"
                            );
                        }
                    }
                }
                want = want
                    .chunks(2)
                    .map(|p| ((p.iter().map(|&v| u32::from(v)).sum::<u32>() + 1) / p.len() as u32) as u8)
                    .collect();
            }
            let t = context::current().unwrap().texture_ref(name).unwrap();
            let sampled = if t.grid_cols != 0 { 1 } else { n as u8 };
            assert_eq!(
                (t.levels, t.levels_written, t.level_defined),
                (sampled, n as u8, (1 << n) - 1),
                "{what} {w}x{h}"
            );
        }
    }
}

/// glCopyTexSubImage3D into a slice of a level above 0 takes the read
/// framebuffer's pixels there; the slices around it keep theirs.
#[test]
fn copy_tex_sub_image_3d_writes_a_slice_of_a_level_above_0() {
    for (what, b) in BACKENDS {
        let _g = context_with(b);
        /* the source: a 4x4 2D texture of known pixels, as the read framebuffer */
        let src = bind_new(GL_TEXTURE_2D);
        let pixels: Vec<u8> = (0..4 * 4).flat_map(|i| [0xc0 | i as u8, i as u8, 0x33, 0xff]).collect();
        let mut fb: GLuint = 0;
        // SAFETY: `pixels` holds 4x4 RGBA8 texels; plain GL calls.
        unsafe {
            glTexImage2D(
                GL_TEXTURE_2D,
                0,
                GL_RGBA as GLint,
                4,
                4,
                0,
                GL_RGBA,
                GL_UNSIGNED_BYTE,
                pixels.as_ptr() as *const c_void,
            );
            glGenFramebuffers(1, &mut fb);
            glBindFramebuffer(GL_FRAMEBUFFER, fb);
            glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, src, 0);
            assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE);
        }
        let name = bind_new(GL_TEXTURE_3D);
        specify_every_level(GL_TEXTURE_3D, (8, 8, 4));
        /* level 1 is 4x4x2: framebuffer (1, 2) 2x2 into (1, 0) of slice 1 */
        // SAFETY: plain GL call.
        unsafe { glCopyTexSubImage3D(GL_TEXTURE_3D, 1, 1, 0, 1, 1, 2, 2, 2) };
        assert_eq!(err(), GL_NO_ERROR, "{what}");
        for z in 0..2 {
            for y in 0..4 {
                for x in 0..4 {
                    let copied = z == 1 && (1..3).contains(&x) && y < 2;
                    let want = if copied {
                        let i = ((y + 2) * 4 + (x - 1 + 1)) as usize * 4;
                        [pixels[i], pixels[i + 1], pixels[i + 2], pixels[i + 3]]
                    } else {
                        colour(1, x, y, z)
                    };
                    assert_eq!(stored(name, 1, x, y, z), Some(want), "{what}: level 1 ({x}, {y}, {z})");
                }
            }
        }
        // SAFETY: plain GL calls.
        unsafe {
            glCopyTexSubImage3D(GL_TEXTURE_3D, 1, 0, 0, 2, 0, 0, 1, 1);
            assert_eq!(err(), GL_INVALID_VALUE, "{what}: level 1 has two slices");
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
            glDeleteFramebuffers(1, &fb);
        }
    }
}

/* ------------------------------------------------------------- 2D arrays -- */

/// glTexImage3D at every level of a 2D array: a power-of-two one on every
/// layout, a non-power-of-two one where the layers are stacked for a
/// texture unit that walks them. Every layer of every level reads back.
#[test]
fn every_level_of_a_2d_array_keeps_its_texels() {
    for (what, b) in BACKENDS {
        let _g = context_with(b);
        let npot_held = !core::ptr::eq(b, &GRID);
        for size in [(8u32, 4u32, 3u32), (7, 5, 3)] {
            if !npot_held && size.0 == 7 {
                continue; /* a_powervr_npot_array_records_its_levels_only */
            }
            let name = bind_new(GL_TEXTURE_2D_ARRAY);
            let n = specify_every_level(GL_TEXTURE_2D_ARRAY, size);
            for l in 0..n {
                assert_level(&format!("{what} {size:?}"), name, l, level_dims(size, l, false));
            }
            let t = context::current().unwrap().texture_ref(name).unwrap();
            assert_eq!((t.levels, t.level_defined, t.level_bad), (n as u8, (1 << n) - 1, 0), "{what} {size:?}");
            if !core::ptr::eq(b, &LAYERED) {
                continue; /* a level at another pitch than its own is no render target */
            }
            for l in 0..n {
                let (w, h, d) = level_dims(size, l, false);
                for z in 0..d {
                    assert_eq!(
                        read_layer(name, l, z, (w, h)),
                        image(l, (0, 0, z), (w, h, 1)),
                        "{what} {size:?} level {l} layer {z}"
                    );
                }
            }
        }
    }
}

/// The PowerVR's non-power-of-two 2D array is one tall stacked image whose
/// levels its TPU cannot find (plan item pvr-2darray changes that storage):
/// its levels above 0 are accepted and recorded, their texels kept nowhere,
/// and a draw samples level 0. When pvr-2darray lands this test is to say
/// the levels are held.
#[test]
fn a_powervr_npot_array_records_its_levels_only() {
    let _g = context_with(&GRID);
    let name = bind_new(GL_TEXTURE_2D_ARRAY);
    let n = specify_every_level(GL_TEXTURE_2D_ARRAY, (7, 5, 3));
    assert_level("base", name, 0, (7, 5, 3));
    for l in 1..n {
        assert_eq!(stored(name, l, 0, 0, 0), None, "level {l}");
    }
    let t = context::current().unwrap().texture_ref(name).unwrap();
    assert_eq!((t.levels, t.chain_levels, t.level_defined, t.level_bad), (1, 1, (1 << n) - 1, 0));
}
