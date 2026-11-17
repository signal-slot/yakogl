// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GL_RGB10_A2 (GL_RGBA/GL_UNSIGNED_INT_2_10_10_10_REV) 3D and 2D-array
//! texture uploads on the V3D host back end, through the GL entry points only
//! (plan #370, unpack3d-api): glTexImage3D with a distinct word per layer,
//! glTexSubImage3D of a sub-box, GL_UNPACK_IMAGE_HEIGHT larger than the
//! height with GL_UNPACK_SKIP_IMAGES, and the source in a
//! GL_PIXEL_UNPACK_BUFFER at an offset. Every layer is read back through a
//! fresh framebuffer with glFramebufferTextureLayer and glReadPixels
//! RGBA/UNSIGNED_BYTE (R/G/B over 1023, A over 3); a fresh FBO per layer
//! keeps clear of the attachment-switch defect that es3_rgb10a2_layers_v3d
//! keeps ignored (no clear is pending here). Each test resets the unpack
//! state and unbinds the unpack buffer on entry and on exit.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_rgb10a2_unpack3d_v3d
//! ```

#![cfg(all(feature = "host-test", feature = "shaderc-online", feature = "gpu-v3d", not(feature = "report-es2")))]

use core::ffi::{c_char, c_void};
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{self, GlesHeap, YakoglBackend};
use yakogl::gl::buffer::*;
use yakogl::gl::consts::*;
use yakogl::gl::framebuffer::*;
use yakogl::gl::state::*;
use yakogl::gl::texture::*;
use yakogl::gl::types::*;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};

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
unsafe extern "C" fn b_draw(_d: *const backend::GlesDraw) -> i32 {
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
    present: None,
    lock: None,
    unlock: None,
    thread_id: None,
    display_busy: None,
};

static BACKEND: YakoglBackend = YakoglBackend {
    name: c"es3-rgb10a2-unpack3d".as_ptr() as *const c_char,
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
    linear_row_align: 0,
    uniforms_host_only: false,
    ds_write: None,
    stencil_resolve: None,
};

static LOCK: Mutex<()> = Mutex::new(());
static ONCE: Once = Once::new();

fn context() -> MutexGuard<'static, ()> {
    let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    ONCE.call_once(|| {
        // SAFETY: both tables are statics; the lock above serialises the tests.
        unsafe {
            osal_set_ops(&OSAL);
            backend::g_backend = &BACKEND;
            yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
            assert!(!yakogl::gl::context::gl_context_create(8, 8).is_null());
        }
    });
    // SAFETY: plain GL calls.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glFinish();
    }
    while err() != GL_NO_ERROR {}
    g
}

fn err() -> GLenum {
    // SAFETY: plain GL call.
    unsafe { glGetError() }
}

fn a2(r: u32, g: u32, b: u32, a: u32) -> u32 {
    r | g << 10 | b << 20 | a << 30
}

fn scale(v: u32, max: u32) -> u8 {
    ((v * 255 + max / 2) / max) as u8
}

fn decoded(w: u32) -> [u8; 4] {
    [scale(w & 0x3ff, 1023), scale((w >> 10) & 0x3ff, 1023), scale((w >> 20) & 0x3ff, 1023), scale(w >> 30, 3)]
}

const W: usize = 4;
const H: usize = 4;
const D: usize = 3;
/// Word the padding between images of a source holds; never read.
const JUNK: u32 = 0xffff_ffff;

/// Distinct word of texel (x, y) of layer z.
fn word(x: usize, y: usize, z: usize) -> u32 {
    let (x, y, z) = (x as u32, y as u32, z as u32);
    a2(1 + x * 200 + z * 17, 3 + y * 150 + z * 61, 1000 - x * 31 - y * 73 - z * 190, (x + y + z) % 4)
}

fn layer_words(z: usize) -> Vec<u32> {
    (0..W * H).map(|i| word(i % W, i / W, z)).collect()
}

fn full_expect() -> Vec<Vec<u32>> {
    (0..D).map(layer_words).collect()
}

/// Word (x, y, z) of the 2x2x2 box the sub-image tests send.
fn box_word(x: usize, y: usize, z: usize) -> u32 {
    a2(900 + x as u32 * 20, 100 + y as u32 * 40, 50 + z as u32 * 300, (x + 2 * y + z) as u32 % 4 + 1)
}

/// The full image with the box at (1, 1, 1) replaced.
fn box_expect() -> Vec<Vec<u32>> {
    let mut e = full_expect();
    for z in 0..2 {
        for y in 0..2 {
            for x in 0..2 {
                e[1 + z][(1 + y) * W + 1 + x] = box_word(x, y, z);
            }
        }
    }
    e
}

/// The full image laid out with `ih` rows per image after `skip` skipped
/// images: junk everywhere the image does not write.
fn laid_out(ih: usize, skip: usize) -> Vec<u32> {
    let mut v = vec![JUNK; (skip + D) * ih * W];
    for z in 0..D {
        for y in 0..H {
            for x in 0..W {
                v[(skip + z) * ih * W + y * W + x] = word(x, y, z);
            }
        }
    }
    v
}

/// The box laid out with `ih` rows per image after `skip` skipped images.
fn box_laid_out(ih: usize, skip: usize) -> Vec<u32> {
    let mut v = vec![JUNK; (skip + 2) * ih * 2];
    for z in 0..2 {
        for y in 0..2 {
            for x in 0..2 {
                v[(skip + z) * ih * 2 + y * 2 + x] = box_word(x, y, z);
            }
        }
    }
    v
}

fn reset_unpack() {
    // SAFETY: plain GL calls.
    unsafe {
        glBindBuffer(GL_PIXEL_UNPACK_BUFFER, 0);
        glPixelStorei(GL_UNPACK_ALIGNMENT, 4);
        glPixelStorei(GL_UNPACK_ROW_LENGTH, 0);
        glPixelStorei(GL_UNPACK_IMAGE_HEIGHT, 0);
        glPixelStorei(GL_UNPACK_SKIP_PIXELS, 0);
        glPixelStorei(GL_UNPACK_SKIP_ROWS, 0);
        glPixelStorei(GL_UNPACK_SKIP_IMAGES, 0);
    }
    assert_eq!(err(), GL_NO_ERROR, "reset unpack state");
}

/// A test context with the unpack state reset now and again on drop.
struct Ctx(#[allow(dead_code)] MutexGuard<'static, ()>);

impl Drop for Ctx {
    fn drop(&mut self) {
        reset_unpack();
    }
}

fn setup() -> Ctx {
    let g = Ctx(context());
    reset_unpack();
    g
}

const TARGETS: [GLenum; 2] = [GL_TEXTURE_3D, GL_TEXTURE_2D_ARRAY];

fn upload_full(t: GLenum, data: *const c_void) {
    // SAFETY: the caller's pointer (or buffer offset) covers the unpack layout.
    unsafe {
        glTexImage3D(
            t,
            0,
            GL_RGB10_A2 as GLint,
            W as GLsizei,
            H as GLsizei,
            D as GLsizei,
            0,
            GL_RGBA,
            GL_UNSIGNED_INT_2_10_10_10_REV,
            data,
        )
    };
}

fn upload_box(t: GLenum, data: *const c_void) {
    // SAFETY: as above; a 2x2x2 box at (1, 1, 1).
    unsafe { glTexSubImage3D(t, 0, 1, 1, 1, 2, 2, 2, GL_RGBA, GL_UNSIGNED_INT_2_10_10_10_REV, data) };
}

/// A new texture of `t` bound to its target, defined with the full image
/// under the default unpack state (which it restores first).
fn textured(t: GLenum) -> GLuint {
    reset_unpack();
    let mut tex = 0;
    let px: Vec<u32> = (0..D).flat_map(layer_words).collect();
    // SAFETY: room for one name.
    unsafe {
        glGenTextures(1, &mut tex);
        glBindTexture(t, tex);
    }
    upload_full(t, px.as_ptr() as *const c_void);
    assert_eq!(err(), GL_NO_ERROR, "glTexImage3D (plain)");
    tex
}

/// A new texture of `t` bound to its target, storage defined with no data.
fn empty(t: GLenum) -> GLuint {
    let mut tex = 0;
    // SAFETY: room for one name.
    unsafe {
        glGenTextures(1, &mut tex);
        glBindTexture(t, tex);
    }
    upload_full(t, core::ptr::null());
    assert_eq!(err(), GL_NO_ERROR, "glTexImage3D (empty)");
    tex
}

/// Layer `z` through a fresh framebuffer: glReadPixels RGBA/UNSIGNED_BYTE.
fn read_layer(tex: GLuint, z: usize) -> Vec<u8> {
    let mut fb = 0;
    let mut px = vec![0x77u8; W * H * 4];
    // SAFETY: room for one name and W x H RGBA8 pixels.
    unsafe {
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        glFramebufferTextureLayer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, tex, 0, z as GLint);
        assert_eq!(err(), GL_NO_ERROR, "glFramebufferTextureLayer {z}");
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE, "layer {z}");
        glReadBuffer(GL_COLOR_ATTACHMENT0);
        glReadPixels(0, 0, W as GLsizei, H as GLsizei, GL_RGBA, GL_UNSIGNED_BYTE, px.as_mut_ptr() as *mut c_void);
        assert_eq!(err(), GL_NO_ERROR, "glReadPixels layer {z}");
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glDeleteFramebuffers(1, &fb);
    }
    px
}

fn check_all(t: GLenum, tex: GLuint, expect: &[Vec<u32>], what: &str) {
    for (z, w) in expect.iter().enumerate() {
        let bytes: Vec<u8> = w.iter().flat_map(|&p| decoded(p)).collect();
        assert_eq!(read_layer(tex, z), bytes, "{what}: target {t:#x} layer {z}");
    }
}

fn pbo(words: &[u32]) -> GLuint {
    let mut b = 0;
    // SAFETY: room for one name; the data pointer covers `words`.
    unsafe {
        glGenBuffers(1, &mut b);
        glBindBuffer(GL_PIXEL_UNPACK_BUFFER, b);
        glBufferData(
            GL_PIXEL_UNPACK_BUFFER,
            (words.len() * 4) as isize,
            words.as_ptr() as *const c_void,
            GL_STATIC_DRAW,
        );
    }
    assert_eq!(err(), GL_NO_ERROR, "glBufferData");
    b
}

fn cleanup(tex: GLuint, buf: GLuint) {
    // SAFETY: names made above (0 is ignored).
    unsafe {
        glBindBuffer(GL_PIXEL_UNPACK_BUFFER, 0);
        glDeleteBuffers(1, &buf);
        glDeleteTextures(1, &tex);
    }
}

fn set(pname: GLenum, v: GLint) {
    // SAFETY: plain GL call.
    unsafe { glPixelStorei(pname, v) };
}

#[test]
fn tex_image_3d_distinct_words_per_layer() {
    let _g = setup();
    for t in TARGETS {
        let tex = textured(t);
        check_all(t, tex, &full_expect(), "glTexImage3D");
        cleanup(tex, 0);
    }
}

#[test]
fn tex_sub_image_3d_replaces_a_box() {
    let _g = setup();
    for t in TARGETS {
        let tex = textured(t);
        let b = box_laid_out(2, 0);
        upload_box(t, b.as_ptr() as *const c_void);
        assert_eq!(err(), GL_NO_ERROR, "glTexSubImage3D");
        check_all(t, tex, &box_expect(), "glTexSubImage3D");
        cleanup(tex, 0);
    }
}

#[test]
fn image_height_larger_than_height() {
    let _g = setup();
    for t in TARGETS {
        let tex = empty(t);
        set(GL_UNPACK_IMAGE_HEIGHT, 6);
        let px = laid_out(6, 0);
        upload_full(t, px.as_ptr() as *const c_void);
        assert_eq!(err(), GL_NO_ERROR, "glTexImage3D image height");
        check_all(t, tex, &full_expect(), "IMAGE_HEIGHT 6 TexImage3D");
        // the sub-box over that image with image height 5
        let tex2 = textured(t);
        set(GL_UNPACK_IMAGE_HEIGHT, 5);
        let b = box_laid_out(5, 0);
        upload_box(t, b.as_ptr() as *const c_void);
        assert_eq!(err(), GL_NO_ERROR, "glTexSubImage3D image height");
        check_all(t, tex2, &box_expect(), "IMAGE_HEIGHT 5 TexSubImage3D");
        cleanup(tex, 0);
        cleanup(tex2, 0);
        set(GL_UNPACK_IMAGE_HEIGHT, 0);
    }
}

#[test]
fn skip_images() {
    let _g = setup();
    for t in TARGETS {
        let tex = empty(t);
        set(GL_UNPACK_SKIP_IMAGES, 2);
        let px = laid_out(H, 2);
        upload_full(t, px.as_ptr() as *const c_void);
        assert_eq!(err(), GL_NO_ERROR, "glTexImage3D skip images");
        check_all(t, tex, &full_expect(), "SKIP_IMAGES 2 TexImage3D");
        let tex2 = textured(t);
        set(GL_UNPACK_SKIP_IMAGES, 1);
        let b = box_laid_out(2, 1);
        upload_box(t, b.as_ptr() as *const c_void);
        assert_eq!(err(), GL_NO_ERROR, "glTexSubImage3D skip images");
        check_all(t, tex2, &box_expect(), "SKIP_IMAGES 1 TexSubImage3D");
        cleanup(tex, 0);
        cleanup(tex2, 0);
        set(GL_UNPACK_SKIP_IMAGES, 0);
    }
}

#[test]
fn image_height_with_skip_images() {
    let _g = setup();
    for t in TARGETS {
        let tex = empty(t);
        set(GL_UNPACK_IMAGE_HEIGHT, 7);
        set(GL_UNPACK_SKIP_IMAGES, 1);
        let px = laid_out(7, 1);
        upload_full(t, px.as_ptr() as *const c_void);
        assert_eq!(err(), GL_NO_ERROR, "glTexImage3D");
        check_all(t, tex, &full_expect(), "IMAGE_HEIGHT 7 + SKIP_IMAGES 1");
        cleanup(tex, 0);
        reset_unpack();
    }
}

#[test]
fn unpack_buffer_at_an_offset() {
    let _g = setup();
    for t in TARGETS {
        // TexImage3D: 8 junk words (32 bytes), then the images.
        let tex = empty(t);
        let mut px = vec![JUNK; 8];
        px.extend((0..D).flat_map(layer_words));
        let buf = pbo(&px);
        upload_full(t, 32usize as *const c_void);
        assert_eq!(err(), GL_NO_ERROR, "glTexImage3D from PBO");
        check_all(t, tex, &full_expect(), "PBO TexImage3D at offset 32");
        cleanup(tex, buf);

        // TexSubImage3D: offset 12, image height 4 and skip images 1.
        let tex = textured(t);
        let mut bx = vec![JUNK; 3];
        bx.extend(box_laid_out(4, 1));
        let buf = pbo(&bx);
        set(GL_UNPACK_IMAGE_HEIGHT, 4);
        set(GL_UNPACK_SKIP_IMAGES, 1);
        upload_box(t, 12usize as *const c_void);
        assert_eq!(err(), GL_NO_ERROR, "glTexSubImage3D from PBO");
        check_all(t, tex, &box_expect(), "PBO TexSubImage3D at offset 12");
        cleanup(tex, buf);
        reset_unpack();
    }
}
