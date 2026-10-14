// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Mip levels >= 1 of 3D and 2D-array textures keep their texels (plan item
//! tex3d-finish; dEQP-GLES3 texture.specification.*3d* and
//! texture.mipmap.3d.*). Until then a 3D texture held its base level only:
//! glTexImage3D, glTexSubImage3D and glCopyTexSubImage3D at a level above 0
//! were accepted and their texels dropped.
//!
//! Every test uploads distinct texels to every level (and slice) and reads
//! each one back, on stand-in back ends of the three storage layouts the GL
//! core has (gl/texture.rs `texture_alloc_storage_3d`):
//!
//!   - STACKED: a back end whose texture unit walks the layers itself
//!     (`native_layered_textures`), levels packed: a 3D level's slices are
//!     the first slots of an array-style level. Read back through the API
//!     (a layer of a level attached to a framebuffer, glReadPixels).
//!   - GC: the same, with the GC7000's one chain pitch and 64-byte level
//!     alignment (`mip_level_align`). Read where vivante/texture.rs's
//!     descriptor walk puts each level, computed here independently.
//!   - GRID: the PowerVR's, slices tiled in a grid that the back end is told
//!     is one level; the levels above it are held after it, each its own
//!     grid. Read from the storage at the offsets that layout promises,
//!     computed here independently; the back end's view (one level at offset
//!     0) is checked to be unchanged.
//!
//! No GPU runs: the back ends draw nothing, and the storage is host memory.
//!
//! Not in a `report-es2` build: it refuses GL_TEXTURE_3D and
//! GL_TEXTURE_2D_ARRAY (gl/texture.rs `es3_only_texture_target`, tested there).

#![cfg(all(feature = "host-test", not(feature = "report-es2")))]

use core::ffi::{c_char, c_void};
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{self, GlesDraw, GlesHeap, YakoglBackend};
use yakogl::gl::consts::*;
use yakogl::gl::context::{self, tex_pitch};
use yakogl::gl::framebuffer::{
    glBindFramebuffer, glCheckFramebufferStatus, glDeleteFramebuffers, glFramebufferTexture2D,
    glFramebufferTextureLayer, glGenFramebuffers, glReadPixels,
};
use yakogl::gl::names::{self, Kind};
use yakogl::gl::state::{glGetError, glPixelStorei};
use yakogl::gl::texture::{
    glBindTexture, glCopyTexSubImage3D, glDeleteTextures, glGenTextures, glGenerateMipmap, glTexImage2D, glTexImage3D,
    glTexStorage3D, glTexSubImage3D,
};
use yakogl::gl::types::*;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};

/* ---- a context on the host: the smallest OSAL and back ends that make one */

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

const fn backend_table(name: &'static core::ffi::CStr, native_layered: bool, mip_level_align: u32) -> YakoglBackend {
    YakoglBackend {
        name: name.as_ptr() as *const c_char,
        native_attrib_formats: 0,
        rt_rgba8888_is_bgra: false,
        mip_level_align,
        compressed_blocks_in_rows: false,
        native_layered_textures: native_layered,
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
        query_begin: None,
        query_end: None,
        query_result: None,
        generate_mipmap: None,
        return_mipmap_chain: None,
    }
}

/// The texture unit walks the layers itself; levels packed (the V3D's rule).
static STACKED: YakoglBackend = backend_table(c"es3-tex-levels-stacked", true, 0);
/// As STACKED, with the GC7000's one chain pitch and 64-byte level starts.
static GC: YakoglBackend = backend_table(c"es3-tex-levels-gc", true, 64);
/// The PowerVR's: no native layers, a 3D texture's slices in a grid.
static GRID: YakoglBackend = backend_table(c"es3-tex-levels-grid", false, 0);

/// One context for the whole file; the tests take turns on it, each with
/// the back end whose layout it reads.
static LOCK: Mutex<()> = Mutex::new(());
static ONCE: Once = Once::new();

fn context(b: &'static YakoglBackend) -> MutexGuard<'static, ()> {
    let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    // SAFETY: both tables are statics; the lock serialises the tests, and the
    // GL core reads the back-end table at every call.
    unsafe {
        ONCE.call_once(|| {
            osal_set_ops(&OSAL);
            backend::g_backend = &STACKED;
            assert!(!context::gl_context_create(8, 8).is_null());
        });
        backend::g_backend = b;
        glPixelStorei(GL_UNPACK_ALIGNMENT, 1);
        glPixelStorei(GL_PACK_ALIGNMENT, 1);
    }
    drain_errors();
    g
}

fn drain_errors() {
    // SAFETY: plain GL call.
    while unsafe { glGetError() } != GL_NO_ERROR {}
}

fn err() -> GLenum {
    // SAFETY: plain GL call.
    unsafe { glGetError() }
}

fn new_texture(target: GLenum) -> GLuint {
    let mut name = 0;
    // SAFETY: room for one name.
    unsafe {
        glGenTextures(1, &mut name);
        glBindTexture(target, name);
    }
    name
}

fn delete_texture(name: GLuint) {
    // SAFETY: a name this test made.
    unsafe { glDeleteTextures(1, &name) };
}

/* ---- the texels: distinct per level, slice and texel -------------------- */

/// Texel (x, y) of slice (or layer) z of level `level`: no two levels and no
/// two slices of a level share one, and the coordinates show in red/green.
fn texel(level: u32, x: u32, y: u32, z: u32) -> [u8; 4] {
    [(level * 40 + x) as u8, (z * 12 + y) as u8, 0x80 | level as u8, 0xff - z as u8]
}

/// `d` slices of `w` x `h` RGBA8 texels of [`texel`], tightly packed.
fn image(level: u32, w: u32, h: u32, d: u32) -> Vec<u8> {
    let mut v = Vec::with_capacity((w * h * d * 4) as usize);
    for z in 0..d {
        for y in 0..h {
            for x in 0..w {
                v.extend_from_slice(&texel(level, x, y, z));
            }
        }
    }
    v
}

/// The size of level `l` of a `w x h x d` texture: the sides halve, and the
/// depth too for a 3D texture (an array's layers do not).
fn level_size(target: GLenum, (w, h, d): (u32, u32, u32), l: u32) -> (u32, u32, u32) {
    let side = |s: u32| (s >> l).max(1);
    (side(w), side(h), if target == GL_TEXTURE_3D { side(d) } else { d })
}

/// The levels of a chain: log2 of its largest side (the depth of a 3D
/// texture counts) plus one.
fn chain_levels(target: GLenum, (w, h, d): (u32, u32, u32)) -> u32 {
    let big = if target == GL_TEXTURE_3D { w.max(h).max(d) } else { w.max(h) };
    big.ilog2() + 1
}

fn tex_image_3d(target: GLenum, level: u32, (w, h, d): (u32, u32, u32), px: &[u8]) {
    // SAFETY: `px` holds w x h x d RGBA8 texels.
    unsafe {
        glTexImage3D(
            target,
            level as GLint,
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
}

/// Every level of the chain, each through glTexImage3D with its own texels.
fn upload_every_level(target: GLenum, size: (u32, u32, u32)) {
    for l in 0..chain_levels(target, size) {
        let (lw, lh, ld) = level_size(target, size, l);
        tex_image_3d(target, l, (lw, lh, ld), &image(l, lw, lh, ld));
        assert_eq!(err(), GL_NO_ERROR, "glTexImage3D level {l} {lw}x{lh}x{ld}");
    }
}

/// Layer `layer` of level `level` of `tex`, through a framebuffer
/// (glFramebufferTextureLayer, glReadPixels): `w x h` RGBA8 texels.
fn read_layer(tex: GLuint, level: u32, layer: u32, w: u32, h: u32) -> Vec<u8> {
    let mut fb = 0;
    let mut px = vec![0xa5u8; (w * h * 4) as usize];
    // SAFETY: room for one name and for w x h RGBA8 pixels.
    unsafe {
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        glFramebufferTextureLayer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, tex, level as GLint, layer as GLint);
        assert_eq!(
            glCheckFramebufferStatus(GL_FRAMEBUFFER),
            GL_FRAMEBUFFER_COMPLETE,
            "layer {layer} of level {level} as a colour attachment"
        );
        glReadPixels(0, 0, w as GLsizei, h as GLsizei, GL_RGBA, GL_UNSIGNED_BYTE, px.as_mut_ptr() as *mut c_void);
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glDeleteFramebuffers(1, &fb);
    }
    assert_eq!(err(), GL_NO_ERROR, "read back layer {layer} of level {level}");
    px
}

/// Read every level and layer of `tex` back through the API and compare
/// with what [`upload_every_level`] put there.
fn assert_every_level_reads_back(tex: GLuint, target: GLenum, size: (u32, u32, u32)) {
    for l in 0..chain_levels(target, size) {
        let (lw, lh, ld) = level_size(target, size, l);
        for z in 0..ld {
            let got = read_layer(tex, l, z, lw, lh);
            let want: Vec<u8> = image(l, lw, lh, ld)[(z * lw * lh * 4) as usize..][..(lw * lh * 4) as usize].to_vec();
            assert_eq!(got, want, "level {l} ({lw}x{lh}x{ld}) slice {z}");
        }
    }
}

/// The GL-core texture object behind `name`.
fn texture_object(name: GLuint) -> &'static context::GlTexture {
    let c = context::current().expect("a current context");
    let slot = names::slot_of(c, Kind::Texture, name).expect("a texture name");
    c.texture_object(slot).expect("a texture object")
}

/// The RGBA8 texel `byte_off` bytes into the storage of `name`.
fn stored(name: GLuint, byte_off: usize) -> [u8; 4] {
    let t = texture_object(name);
    assert!(byte_off + 4 <= t.mem.size, "offset {byte_off} inside the {} bytes of storage", t.mem.size);
    // SAFETY: inside the texture's host allocation (checked above).
    unsafe { *((t.mem.cpu as *const u8).add(byte_off) as *const [u8; 4]) }
}

/* ---- STACKED: through the API ------------------------------------------- */

#[test]
fn every_level_of_a_power_of_two_3d_texture_reads_back() {
    let _g = context(&STACKED);
    let size = (8, 8, 4);
    let tex = new_texture(GL_TEXTURE_3D);
    upload_every_level(GL_TEXTURE_3D, size);
    assert_eq!(chain_levels(GL_TEXTURE_3D, size), 4);
    assert_every_level_reads_back(tex, GL_TEXTURE_3D, size);
    delete_texture(tex);
}

#[test]
fn every_level_of_a_npot_3d_texture_reads_back() {
    let _g = context(&STACKED);
    let size = (7, 5, 6);
    let tex = new_texture(GL_TEXTURE_3D);
    upload_every_level(GL_TEXTURE_3D, size);
    assert_eq!(chain_levels(GL_TEXTURE_3D, size), 3);
    assert_every_level_reads_back(tex, GL_TEXTURE_3D, size);
    delete_texture(tex);
}

/// A chain whose depth outlasts its sides: levels 3 and 4 of 4x4x16 are
/// 1x1x2 and 1x1x1, past the 2D chain of 4x4.
#[test]
fn a_3d_chain_deeper_than_its_sides_keeps_its_last_levels() {
    let _g = context(&STACKED);
    let size = (4, 4, 16);
    let tex = new_texture(GL_TEXTURE_3D);
    upload_every_level(GL_TEXTURE_3D, size);
    assert_eq!(chain_levels(GL_TEXTURE_3D, size), 5);
    assert_every_level_reads_back(tex, GL_TEXTURE_3D, size);
    delete_texture(tex);
}

#[test]
fn every_level_of_a_2d_array_reads_back() {
    let _g = context(&STACKED);
    for size in [(8, 4, 3), (6, 5, 3)] {
        let tex = new_texture(GL_TEXTURE_2D_ARRAY);
        upload_every_level(GL_TEXTURE_2D_ARRAY, size);
        assert_every_level_reads_back(tex, GL_TEXTURE_2D_ARRAY, size);
        delete_texture(tex);
    }
}

/// Immutable storage, then each level filled by glTexSubImage3D, for a
/// power-of-two and a non-power-of-two 3D texture.
#[test]
fn sub_images_fill_every_level_of_3d_storage() {
    let _g = context(&STACKED);
    for size in [(8, 8, 4), (7, 5, 6)] {
        let tex = new_texture(GL_TEXTURE_3D);
        let levels = chain_levels(GL_TEXTURE_3D, size);
        // SAFETY: plain GL call.
        unsafe { glTexStorage3D(GL_TEXTURE_3D, levels as GLsizei, GL_RGBA8, size.0 as _, size.1 as _, size.2 as _) };
        assert_eq!(err(), GL_NO_ERROR, "glTexStorage3D {size:?}");
        for l in 0..levels {
            let (lw, lh, ld) = level_size(GL_TEXTURE_3D, size, l);
            let px = image(l, lw, lh, ld);
            // SAFETY: `px` holds lw x lh x ld RGBA8 texels.
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
                    px.as_ptr() as *const c_void,
                )
            };
            assert_eq!(err(), GL_NO_ERROR, "glTexSubImage3D level {l} of {size:?}");
        }
        assert_every_level_reads_back(tex, GL_TEXTURE_3D, size);
        delete_texture(tex);
    }
}

/// A sub-image into the middle of level 1 lands there and nowhere else.
#[test]
fn a_sub_image_of_level_1_lands_in_its_slice() {
    let _g = context(&STACKED);
    let size = (8, 8, 4);
    let tex = new_texture(GL_TEXTURE_3D);
    upload_every_level(GL_TEXTURE_3D, size);
    /* level 1 is 4x4x2: a 2x2 box at (1, 2) of slice 1 */
    let patch = [0x11u8, 0x22, 0x33, 0x44].repeat(4);
    // SAFETY: `patch` holds 2x2x1 RGBA8 texels.
    unsafe {
        glTexSubImage3D(GL_TEXTURE_3D, 1, 1, 2, 1, 2, 2, 1, GL_RGBA, GL_UNSIGNED_BYTE, patch.as_ptr() as *const c_void)
    };
    assert_eq!(err(), GL_NO_ERROR);
    for z in 0..2 {
        let got = read_layer(tex, 1, z, 4, 4);
        for y in 0..4 {
            for x in 0..4 {
                let inside = z == 1 && (1..3).contains(&x) && (2..4).contains(&y);
                let want = if inside { [0x11, 0x22, 0x33, 0x44] } else { texel(1, x, y, z) };
                let i = ((y * 4 + x) * 4) as usize;
                assert_eq!(got[i..i + 4], want, "level 1 slice {z} texel ({x}, {y})");
            }
        }
    }
    /* the other levels are as uploaded */
    for l in [0u32, 2, 3] {
        let (lw, lh, _) = level_size(GL_TEXTURE_3D, size, l);
        assert_eq!(read_layer(tex, l, 0, lw, lh), image(l, lw, lh, 1), "level {l}");
    }
    delete_texture(tex);
}

/// Level 1 of 8x8x4 has two slices, level 2 one: a box past a level's own
/// depth is GL_INVALID_VALUE (ES 3.0 3.8.5), not level 0's depth.
#[test]
fn a_levels_depth_bounds_its_sub_images_and_copies() {
    let _g = context(&STACKED);
    let tex = new_texture(GL_TEXTURE_3D);
    upload_every_level(GL_TEXTURE_3D, (8, 8, 4));
    let px = [0u8; 4 * 4 * 4 * 3];
    // SAFETY: `px` holds 4x4x3 RGBA8 texels; plain GL calls.
    unsafe {
        glTexSubImage3D(GL_TEXTURE_3D, 1, 0, 0, 1, 4, 4, 2, GL_RGBA, GL_UNSIGNED_BYTE, px.as_ptr() as *const c_void);
        assert_eq!(glGetError(), GL_INVALID_VALUE, "slices 1..3 of a two-slice level");
        glTexSubImage3D(GL_TEXTURE_3D, 2, 0, 0, 0, 2, 2, 1, GL_RGBA, GL_UNSIGNED_BYTE, px.as_ptr() as *const c_void);
        assert_eq!(glGetError(), GL_NO_ERROR, "the one slice of level 2");
        glCopyTexSubImage3D(GL_TEXTURE_3D, 1, 0, 0, 2, 0, 0, 2, 2);
        assert_eq!(glGetError(), GL_INVALID_VALUE, "slice 2 of a two-slice level");
    }
    delete_texture(tex);
}

/// A 2D texture of known texels as the read framebuffer's colour buffer.
fn source_framebuffer(w: u32, h: u32) -> (GLuint, GLuint, Vec<u8>) {
    let px: Vec<u8> = (0..w * h).flat_map(|i| [i as u8 * 3 + 1, 0x5a, 0xff - i as u8, 0xc3]).collect();
    let src = new_texture(GL_TEXTURE_2D);
    let mut fb = 0;
    // SAFETY: `px` holds w x h RGBA8 texels; room for one name.
    unsafe {
        glTexImage2D(
            GL_TEXTURE_2D,
            0,
            GL_RGBA as GLint,
            w as GLsizei,
            h as GLsizei,
            0,
            GL_RGBA,
            GL_UNSIGNED_BYTE,
            px.as_ptr() as *const c_void,
        );
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, src, 0);
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE);
    }
    (src, fb, px)
}

fn release_source(src: GLuint, fb: GLuint) {
    // SAFETY: names the caller made.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glDeleteFramebuffers(1, &fb);
    }
    delete_texture(src);
}

#[test]
fn a_copy_into_level_1_of_a_3d_texture_reads_back() {
    let _g = context(&STACKED);
    let tex = new_texture(GL_TEXTURE_3D);
    upload_every_level(GL_TEXTURE_3D, (8, 8, 4));
    let (src, fb, px) = source_framebuffer(4, 4);
    // SAFETY: plain GL calls.
    unsafe {
        glBindTexture(GL_TEXTURE_3D, tex);
        glCopyTexSubImage3D(GL_TEXTURE_3D, 1, 0, 0, 1, 0, 0, 4, 4);
    }
    assert_eq!(err(), GL_NO_ERROR, "copy into slice 1 of level 1");
    release_source(src, fb);
    assert_eq!(read_layer(tex, 1, 1, 4, 4), px, "level 1 slice 1 holds the copy");
    assert_eq!(read_layer(tex, 1, 0, 4, 4), image(1, 4, 4, 1), "level 1 slice 0 is as uploaded");
    delete_texture(tex);
}

/// Level 1 uploaded before level 0 lays out the chain it implies, and the
/// level 0 of that chain keeps it (as glTexImage2D does).
#[test]
fn a_level_uploaded_before_level_0_is_kept() {
    let _g = context(&STACKED);
    let size = (8, 8, 4);
    let tex = new_texture(GL_TEXTURE_3D);
    for l in (0..chain_levels(GL_TEXTURE_3D, size)).rev() {
        let (lw, lh, ld) = level_size(GL_TEXTURE_3D, size, l);
        tex_image_3d(GL_TEXTURE_3D, l, (lw, lh, ld), &image(l, lw, lh, ld));
        assert_eq!(err(), GL_NO_ERROR, "level {l}");
    }
    assert_every_level_reads_back(tex, GL_TEXTURE_3D, size);
    delete_texture(tex);
}

/// A level of a size the chain does not have is no GL error (ES 3.0 3.8.3:
/// the texture is incomplete while it stands) and holds nothing; the levels
/// that fit keep theirs.
#[test]
fn a_level_of_the_wrong_size_is_recorded_and_held_nowhere() {
    let _g = context(&STACKED);
    let tex = new_texture(GL_TEXTURE_3D);
    upload_every_level(GL_TEXTURE_3D, (8, 8, 4));
    tex_image_3d(GL_TEXTURE_3D, 1, (4, 4, 3), &image(9, 4, 4, 3));
    assert_eq!(err(), GL_NO_ERROR);
    let t = texture_object(tex);
    assert_ne!(t.level_bad & 2, 0, "level 1 is recorded as not fitting the chain");
    assert_eq!(read_layer(tex, 1, 0, 4, 4), image(1, 4, 4, 1), "its storage keeps the level that fit");
    assert_eq!(read_layer(tex, 2, 0, 2, 2), image(2, 2, 2, 1));
    delete_texture(tex);
}

/// 4x4x4 whose slices alternate 0 and 200 (z even, odd), and whose rows
/// alternate 0 and 100 inside the odd slices: level 1 is the 2x2x2 box over
/// them, (0 + 0 + 200 + 200 [+ the row offsets]) / 8, the slices filtered into
/// each other. A per-slice 2D filter would give 0 and 200 instead.
fn mipmap_source() -> Vec<u8> {
    let mut v = Vec::new();
    for z in 0..4u32 {
        for y in 0..4u32 {
            for _x in 0..4 {
                let r = if z % 2 == 1 { 200u8 } else { 0 };
                let g = if z % 2 == 1 && y % 2 == 1 { 100u8 } else { 0 };
                v.extend_from_slice(&[r, g, 40, 255]);
            }
        }
    }
    v
}

/// The texel every level above 0 of [`mipmap_source`]'s chain holds.
const MIPMAPPED: [u8; 4] = [100, 25, 40, 255];

fn generate_from_mipmap_source() -> GLuint {
    let tex = new_texture(GL_TEXTURE_3D);
    tex_image_3d(GL_TEXTURE_3D, 0, (4, 4, 4), &mipmap_source());
    // SAFETY: plain GL call.
    unsafe { glGenerateMipmap(GL_TEXTURE_3D) };
    assert_eq!(err(), GL_NO_ERROR, "glGenerateMipmap(GL_TEXTURE_3D)");
    tex
}

#[test]
fn generate_mipmap_filters_a_3d_texture_across_its_slices() {
    let _g = context(&STACKED);
    let tex = generate_from_mipmap_source();
    assert_eq!(read_layer(tex, 1, 0, 2, 2), MIPMAPPED.repeat(4), "level 1 slice 0");
    assert_eq!(read_layer(tex, 1, 1, 2, 2), MIPMAPPED.repeat(4), "level 1 slice 1");
    assert_eq!(read_layer(tex, 2, 0, 1, 1), MIPMAPPED.to_vec(), "level 2");
    assert_eq!(read_layer(tex, 0, 1, 4, 4), mipmap_source()[64..128].to_vec(), "level 0 untouched");
    let t = texture_object(tex);
    assert_eq!((t.levels, t.levels_written), (3, 3), "the back end samples the whole chain");
    delete_texture(tex);
}

/// An SRGB8_ALPHA8 3D texture filters in linear space: slices of red 0 and
/// 255 average to linear 0.5, which is sRGB 188 (a byte mean would give 128).
/// Alpha is linear and averages as bytes.
#[test]
fn generate_mipmap_averages_an_srgb_3d_texture_in_linear_space() {
    let _g = context(&STACKED);
    let tex = new_texture(GL_TEXTURE_3D);
    let mut px = Vec::new();
    for z in 0..2u8 {
        px.extend_from_slice(&[255 * z, 0, 0, 100 * z].repeat(4));
    }
    // SAFETY: `px` holds 2x2x2 RGBA8 texels.
    unsafe {
        glTexImage3D(
            GL_TEXTURE_3D,
            0,
            GL_SRGB8_ALPHA8 as GLint,
            2,
            2,
            2,
            0,
            GL_RGBA,
            GL_UNSIGNED_BYTE,
            px.as_ptr() as *const c_void,
        );
        glGenerateMipmap(GL_TEXTURE_3D);
    }
    assert_eq!(err(), GL_NO_ERROR, "glGenerateMipmap of an sRGB 3D texture");
    let off = texture_object(tex).level_off[1] as usize;
    assert_eq!(stored(tex, off), [188, 0, 0, 50], "level 1, the 2x2x2 box");
    delete_texture(tex);
}

/* ---- GC: where the GC7000's descriptor finds each level ----------------- */

/// Byte offset of level `l` in a stacked chain as vivante/texture.rs's
/// `build_descriptor` walks it: every level on a 64-byte boundary, each one
/// `stride * its height * layers` bytes (one LINEAR_STRIDE for the chain,
/// the layer count of level 0 at every level), the layer step `stride *
/// height` of the level.
fn gc_level_offset(w: u32, h: u32, layers: u32, l: u32) -> usize {
    let stride = tex_pitch(w) as usize * 4;
    let (mut off, mut lh) = (0usize, h as usize);
    for _ in 0..l {
        off = off.next_multiple_of(64) + stride * lh * layers as usize;
        lh = (lh / 2).max(1);
    }
    off.next_multiple_of(64)
}

/// Check every texel of every level and slice of `tex` where the GC7000
/// walk finds it.
fn assert_where_the_gc7000_reads(tex: GLuint, target: GLenum, size: (u32, u32, u32)) {
    let (w, h, d) = size;
    for l in 0..chain_levels(target, size) {
        let (lw, lh, ld) = level_size(target, size, l);
        let base = gc_level_offset(w, h, d, l);
        let stride = tex_pitch(w) as usize * 4;
        for z in 0..ld {
            for y in 0..lh {
                for x in 0..lw {
                    let off = base + (z * lh + y) as usize * stride + x as usize * 4;
                    assert_eq!(stored(tex, off), texel(l, x, y, z), "level {l} slice {z} texel ({x}, {y})");
                }
            }
        }
    }
}

#[test]
fn gc7000_3d_levels_lie_where_its_descriptor_walks() {
    let _g = context(&GC);
    for size in [(8, 8, 4), (7, 5, 6), (4, 4, 16)] {
        let tex = new_texture(GL_TEXTURE_3D);
        upload_every_level(GL_TEXTURE_3D, size);
        let t = texture_object(tex);
        assert_eq!(u32::from(t.levels), chain_levels(GL_TEXTURE_3D, size), "{size:?}: every level is sampled");
        assert_where_the_gc7000_reads(tex, GL_TEXTURE_3D, size);
        delete_texture(tex);
    }
}

#[test]
fn gc7000_2d_array_levels_lie_where_its_descriptor_walks() {
    let _g = context(&GC);
    for size in [(8, 4, 3), (6, 5, 3)] {
        let tex = new_texture(GL_TEXTURE_2D_ARRAY);
        upload_every_level(GL_TEXTURE_2D_ARRAY, size);
        assert_where_the_gc7000_reads(tex, GL_TEXTURE_2D_ARRAY, size);
        delete_texture(tex);
    }
}

#[test]
fn gc7000_generate_mipmap_filters_a_3d_texture_across_its_slices() {
    let _g = context(&GC);
    let tex = generate_from_mipmap_source();
    for (l, ld, lw) in [(1u32, 2u32, 2u32), (2, 1, 1)] {
        let base = gc_level_offset(4, 4, 4, l);
        for z in 0..ld {
            for y in 0..lw {
                for x in 0..lw {
                    let off = base + ((z * lw + y) * tex_pitch(4) + x) as usize * 4;
                    assert_eq!(stored(tex, off), MIPMAPPED, "level {l} slice {z} texel ({x}, {y})");
                }
            }
        }
    }
    delete_texture(tex);
}

/* ---- GRID: the PowerVR's; the levels above 0 held after level 0 --------- */

/// ceil(sqrt(depth)): the columns a grid tiles `depth` slices in.
fn grid_cols(depth: u32) -> u32 {
    (1..).find(|c| c * c >= depth).unwrap()
}

/// Byte offset of level `l` of a `w x h x d` grid: level 0's grid of
/// cols x rows slices first, then each level's own grid (the same columns,
/// as many rows as its slices need), rows at the pair-padded grid width.
fn grid_level_offset(w: u32, h: u32, d: u32, l: u32) -> usize {
    let cols = grid_cols(d);
    (0..l)
        .map(|k| {
            let (lw, lh, ld) = level_size(GL_TEXTURE_3D, (w, h, d), k);
            tex_pitch(lw * cols) as usize * (lh * ld.div_ceil(cols)) as usize * 4
        })
        .sum()
}

fn assert_where_the_grid_holds(tex: GLuint, size: (u32, u32, u32)) {
    let (w, h, d) = size;
    let cols = grid_cols(d);
    for l in 0..chain_levels(GL_TEXTURE_3D, size) {
        let (lw, lh, ld) = level_size(GL_TEXTURE_3D, size, l);
        let base = grid_level_offset(w, h, d, l);
        let pitch = tex_pitch(lw * cols) as usize * 4;
        for z in 0..ld {
            for y in 0..lh {
                for x in 0..lw {
                    let (col, row) = ((z % cols) * lw + x, (z / cols) * lh + y);
                    let off = base + row as usize * pitch + col as usize * 4;
                    assert_eq!(stored(tex, off), texel(l, x, y, z), "level {l} slice {z} texel ({x}, {y})");
                }
            }
        }
    }
}

/// What the PowerVR's draw is handed is level 0's grid alone, at offset 0,
/// as before: one level, no chain for its TPU to walk.
fn assert_the_backend_sees_one_level(tex: GLuint, size: (u32, u32, u32)) {
    let t = texture_object(tex);
    assert_eq!(u32::from(t.grid_cols), grid_cols(size.2));
    assert_eq!((t.chain_levels, t.levels), (1, 1), "the back end is told of level 0 only");
    assert_eq!(t.level_off[0], 0);
    assert_eq!((t.width, t.height, u32::from(t.layers)), size);
}

#[test]
fn grid_3d_levels_are_held_after_level_0() {
    let _g = context(&GRID);
    for size in [(8, 8, 5), (7, 5, 6), (4, 4, 16)] {
        let tex = new_texture(GL_TEXTURE_3D);
        upload_every_level(GL_TEXTURE_3D, size);
        assert_the_backend_sees_one_level(tex, size);
        assert_where_the_grid_holds(tex, size);
        delete_texture(tex);
    }
}

#[test]
fn grid_3d_storage_takes_sub_images_and_copies_at_every_level() {
    let _g = context(&GRID);
    let size = (8, 8, 5);
    let tex = new_texture(GL_TEXTURE_3D);
    let levels = chain_levels(GL_TEXTURE_3D, size);
    // SAFETY: plain GL call.
    unsafe { glTexStorage3D(GL_TEXTURE_3D, levels as GLsizei, GL_RGBA8, 8, 8, 5) };
    assert_eq!(err(), GL_NO_ERROR);
    for l in 0..levels {
        let (lw, lh, ld) = level_size(GL_TEXTURE_3D, size, l);
        let px = image(l, lw, lh, ld);
        // SAFETY: `px` holds lw x lh x ld RGBA8 texels.
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
                px.as_ptr() as *const c_void,
            )
        };
        assert_eq!(err(), GL_NO_ERROR, "glTexSubImage3D level {l}");
    }
    assert_the_backend_sees_one_level(tex, size);
    assert_where_the_grid_holds(tex, size);

    /* a copy into slice 1 of level 1 (4x4x2): the grid keeps level 0's
     * three columns, so slice 1 is the second 4x4 square of level 1's row */
    let (src, fb, px) = source_framebuffer(4, 4);
    // SAFETY: plain GL calls.
    unsafe {
        glBindTexture(GL_TEXTURE_3D, tex);
        glCopyTexSubImage3D(GL_TEXTURE_3D, 1, 0, 0, 1, 0, 0, 4, 4);
    }
    assert_eq!(err(), GL_NO_ERROR, "copy into slice 1 of level 1");
    release_source(src, fb);
    let cols = grid_cols(5);
    let (base, pitch) = (grid_level_offset(8, 8, 5, 1), tex_pitch(4 * cols) as usize * 4);
    for y in 0..4u32 {
        for x in 0..4u32 {
            let off = base + y as usize * pitch + (4 + x) as usize * 4;
            let i = ((y * 4 + x) * 4) as usize;
            assert_eq!(stored(tex, off), px[i..i + 4], "level 1 slice 1 texel ({x}, {y})");
        }
    }
    delete_texture(tex);
}

#[test]
fn grid_generate_mipmap_fills_the_held_levels() {
    let _g = context(&GRID);
    let tex = generate_from_mipmap_source();
    /* 4 slices: two columns at every level */
    for (l, ld, lw) in [(1u32, 2u32, 2u32), (2, 1, 1)] {
        let base = grid_level_offset(4, 4, 4, l);
        let pitch = tex_pitch(lw * 2) as usize * 4;
        for z in 0..ld {
            for y in 0..lw {
                for x in 0..lw {
                    let off = base + y as usize * pitch + (z * lw + x) as usize * 4;
                    assert_eq!(stored(tex, off), MIPMAPPED, "level {l} slice {z} texel ({x}, {y})");
                }
            }
        }
    }
    assert_the_backend_sees_one_level(tex, (4, 4, 4));
    delete_texture(tex);
}

/// The PowerVR's power-of-two arrays keep their chain (stacked, the layers
/// of each level together): every level reads back through the API.
#[test]
fn grid_back_end_power_of_two_array_levels_read_back() {
    let _g = context(&GRID);
    let size = (8, 4, 3);
    let tex = new_texture(GL_TEXTURE_2D_ARRAY);
    upload_every_level(GL_TEXTURE_2D_ARRAY, size);
    assert_every_level_reads_back(tex, GL_TEXTURE_2D_ARRAY, size);
    delete_texture(tex);
}
