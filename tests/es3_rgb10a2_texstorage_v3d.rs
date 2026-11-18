// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GL_RGB10_A2 immutable textures (glTexStorage2D/3D) on the V3D host back
//! end, through the GL entry points only (plan #370, texstorage-api2).
//!
//! On the V3D 4.2 build GL_RGB10_A2 is its own texture class
//! (`GLES_TEXFMT_RGB10A2`): a texel is the little-endian 10/10/10/2 word. The
//! back end is the smallest stand-in that makes a context (nothing is drawn).
//!
//! Every row reads back through an FBO with glReadPixels RGBA/UNSIGNED_BYTE,
//! which decodes each field to a byte as `(v * 255 + max / 2) / max` (max 1023
//! for R/G/B, 3 for A): round half up, as es3_rgb10a2_texture_v3d expects.
//!
//! Covered: glTexStorage2D and the immutable queries, glTexSubImage2D into
//! levels 0 and 1, glTexImage2D / glTexStorage2D on an immutable texture, a
//! type invalid for the format, glTexStorage3D for GL_TEXTURE_2D_ARRAY and
//! GL_TEXTURE_3D read through glFramebufferTextureLayer, and completeness of
//! the framebuffer.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_rgb10a2_texstorage_v3d
//! ```

#![cfg(all(feature = "host-test", feature = "shaderc-online", feature = "gpu-v3d", not(feature = "report-es2")))]

use core::ffi::{c_char, c_void};
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{self, GlesHeap, YakoglBackend};
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
unsafe extern "C" fn b_ok_draw(_d: *const backend::GlesDraw) -> i32 {
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
    name: c"es3-rgb10a2-texstorage".as_ptr() as *const c_char,
    native_attrib_formats: 0,
    rt_rgba8888_is_bgra: false,
    mip_level_align: 0,
    compressed_blocks_in_rows: false,
    native_layered_textures: false,
    front_facing_shader_variant: false,
    etc1_decoded_at_upload: false,
    init: Some(b_init),
    shutdown: None,
    draw: Some(b_ok_draw),
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
    max_samples: 0,
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

/// The 10/10/10/2 word of (r, g, b, a), as GL_UNSIGNED_INT_2_10_10_10_REV
/// packs it.
fn a2(r: u32, g: u32, b: u32, a: u32) -> u32 {
    assert!(r < 1024 && g < 1024 && b < 1024 && a < 4);
    r | g << 10 | b << 20 | a << 30
}

fn fields(w: u32) -> [u32; 4] {
    [w & 0x3ff, (w >> 10) & 0x3ff, (w >> 20) & 0x3ff, w >> 30]
}

fn scale(v: u32, max: u32) -> u8 {
    ((v * 255 + max / 2) / max) as u8
}

fn decoded(w: u32) -> [u8; 4] {
    let f = fields(w);
    [scale(f[0], 1023), scale(f[1], 1023), scale(f[2], 1023), scale(f[3], 3)]
}

fn want_bytes(words: &[u32]) -> Vec<u8> {
    words.iter().flat_map(|&w| decoded(w)).collect()
}

/// `n` words that differ in every field.
fn words(n: u32, seed: u32) -> Vec<u32> {
    (0..n)
        .map(|i| {
            a2((i * 131 + seed) % 1024, (i * 17 + seed * 3 + 5) % 1024, 1023 - (i * 100 + seed) % 1024, (i + seed) % 4)
        })
        .collect()
}

/// A texture bound to its target and a framebuffer to attach it to.
struct Tex {
    tex: GLuint,
    fb: GLuint,
}

impl Drop for Tex {
    fn drop(&mut self) {
        // SAFETY: names this test made.
        unsafe {
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
            glDeleteFramebuffers(1, &self.fb);
            glDeleteTextures(1, &self.tex);
        }
    }
}

/// A fresh texture bound to `target`, its storage made by `store`.
fn make(target: GLenum, store: impl FnOnce()) -> Tex {
    let (mut tex, mut fb) = (0, 0);
    // SAFETY: room for one name each.
    unsafe {
        glGenTextures(1, &mut tex);
        glBindTexture(target, tex);
        store();
        glGenFramebuffers(1, &mut fb);
    }
    assert_eq!(err(), GL_NO_ERROR, "storage");
    Tex { tex, fb }
}

fn storage_2d(w: GLsizei, h: GLsizei, levels: GLsizei) -> Tex {
    // SAFETY: plain GL call.
    make(GL_TEXTURE_2D, || unsafe { glTexStorage2D(GL_TEXTURE_2D, levels, GL_RGB10_A2, w, h) })
}

fn upload(level: GLint, w: GLsizei, h: GLsizei, words: &[u32]) {
    assert_eq!(words.len(), (w * h) as usize);
    // SAFETY: `words` holds w x h texels.
    unsafe {
        glTexSubImage2D(
            GL_TEXTURE_2D,
            level,
            0,
            0,
            w,
            h,
            GL_RGBA,
            GL_UNSIGNED_INT_2_10_10_10_REV,
            words.as_ptr() as *const c_void,
        );
    }
    assert_eq!(err(), GL_NO_ERROR, "glTexSubImage2D level {level}");
}

fn upload_slice(target: GLenum, z: GLint, w: GLsizei, h: GLsizei, words: &[u32]) {
    assert_eq!(words.len(), (w * h) as usize);
    // SAFETY: `words` holds one w x h slice.
    unsafe {
        glTexSubImage3D(
            target,
            0,
            0,
            0,
            z,
            w,
            h,
            1,
            GL_RGBA,
            GL_UNSIGNED_INT_2_10_10_10_REV,
            words.as_ptr() as *const c_void,
        );
    }
    assert_eq!(err(), GL_NO_ERROR, "glTexSubImage3D z {z}");
}

fn read_fb(w: GLsizei, h: GLsizei, what: &str) -> Vec<u8> {
    let mut px = vec![0x77u8; (w * h) as usize * 4];
    // SAFETY: room for w x h RGBA8 pixels.
    unsafe {
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE, "{what}");
        glReadBuffer(GL_COLOR_ATTACHMENT0);
        glReadPixels(0, 0, w, h, GL_RGBA, GL_UNSIGNED_BYTE, px.as_mut_ptr() as *mut c_void);
    }
    assert_eq!(err(), GL_NO_ERROR, "glReadPixels {what}");
    px
}

/// glReadPixels RGBA/UNSIGNED_BYTE of `level` of the 2D texture through its FBO.
fn read_level(t: &Tex, level: GLint, w: GLsizei, h: GLsizei) -> Vec<u8> {
    // SAFETY: plain GL calls.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, t.fb);
        glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, t.tex, level);
    }
    assert_eq!(err(), GL_NO_ERROR, "glFramebufferTexture2D level {level}");
    read_fb(w, h, &format!("level {level}"))
}

fn read_layer(t: &Tex, layer: GLint, w: GLsizei, h: GLsizei) -> Vec<u8> {
    // SAFETY: plain GL calls.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, t.fb);
        glFramebufferTextureLayer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, t.tex, 0, layer);
    }
    assert_eq!(err(), GL_NO_ERROR, "glFramebufferTextureLayer {layer}");
    read_fb(w, h, &format!("layer {layer}"))
}

fn param(target: GLenum, pname: GLenum) -> GLint {
    let mut v = -1;
    // SAFETY: one GLint.
    unsafe { glGetTexParameteriv(target, pname, &mut v) };
    assert_eq!(err(), GL_NO_ERROR, "glGetTexParameteriv {pname:#x}");
    v
}

#[test]
fn storage_2d_succeeds_and_is_immutable() {
    let _g = context();
    let _t = storage_2d(4, 4, 3);
    assert_eq!(param(GL_TEXTURE_2D, GL_TEXTURE_IMMUTABLE_FORMAT), 1);
    assert_eq!(param(GL_TEXTURE_2D, GL_TEXTURE_IMMUTABLE_LEVELS), 3);
}

#[test]
fn sub_image_reads_back_at_levels_0_and_1() {
    let _g = context();
    let t = storage_2d(4, 4, 2);
    let l0 = words(16, 7);
    let l1 = words(4, 300);
    upload(0, 4, 4, &l0);
    upload(1, 2, 2, &l1);
    assert_eq!(read_level(&t, 0, 4, 4), want_bytes(&l0), "level 0");
    assert_eq!(read_level(&t, 1, 2, 2), want_bytes(&l1), "level 1");
    // 1 -> 0, 511 -> 127, 512 -> 128, 1022 -> 255; alpha 1 -> 85, 2 -> 170
    let edge = [a2(1, 511, 512, 1), a2(1022, 1023, 0, 2), a2(0, 0, 0, 0), a2(1023, 1023, 1023, 3)];
    upload(1, 2, 2, &edge);
    assert_eq!(&read_level(&t, 1, 2, 2)[..8], [0, 127, 128, 85, 255, 255, 0, 170]);
    assert_eq!(read_level(&t, 1, 2, 2), want_bytes(&edge), "level 1 edges");
    assert_eq!(read_level(&t, 0, 4, 4), want_bytes(&l0), "level 0 untouched");
}

#[test]
fn tex_image_and_second_storage_on_immutable_are_invalid_operation() {
    let _g = context();
    let _t = storage_2d(4, 4, 2);
    let w = words(16, 1);
    // SAFETY: `w` holds 4x4 texels.
    unsafe {
        glTexImage2D(
            GL_TEXTURE_2D,
            0,
            GL_RGB10_A2 as GLint,
            4,
            4,
            0,
            GL_RGBA,
            GL_UNSIGNED_INT_2_10_10_10_REV,
            w.as_ptr() as *const c_void,
        );
    }
    assert_eq!(err(), GL_INVALID_OPERATION, "glTexImage2D");
    // SAFETY: plain GL call.
    unsafe { glTexStorage2D(GL_TEXTURE_2D, 1, GL_RGB10_A2, 4, 4) };
    assert_eq!(err(), GL_INVALID_OPERATION, "second glTexStorage2D");
    assert_eq!(param(GL_TEXTURE_2D, GL_TEXTURE_IMMUTABLE_LEVELS), 2);
}

#[test]
fn sub_image_with_a_type_the_format_refuses() {
    let _g = context();
    let _t = storage_2d(4, 4, 1);
    let px = [0u8; 64];
    let mut got = Vec::new();
    for ty in [GL_UNSIGNED_BYTE, GL_UNSIGNED_SHORT_4_4_4_4, GL_FLOAT] {
        // SAFETY: 64 bytes cover 4x4 texels of up to 4 bytes.
        unsafe { glTexSubImage2D(GL_TEXTURE_2D, 0, 0, 0, 4, 4, GL_RGBA, ty, px.as_ptr() as *const c_void) };
        got.push(err());
    }
    assert_eq!(got, EXPECT_TYPES);
}

#[test]
fn storage_3d_array_layer_reads_back() {
    let _g = context();
    // SAFETY: plain GL call.
    let t = make(GL_TEXTURE_2D_ARRAY, || unsafe { glTexStorage3D(GL_TEXTURE_2D_ARRAY, 1, GL_RGB10_A2, 4, 2, 3) });
    assert_eq!(param(GL_TEXTURE_2D_ARRAY, GL_TEXTURE_IMMUTABLE_FORMAT), 1);
    assert_eq!(param(GL_TEXTURE_2D_ARRAY, GL_TEXTURE_IMMUTABLE_LEVELS), 1);
    let w = words(8, 11);
    upload_slice(GL_TEXTURE_2D_ARRAY, 1, 4, 2, &w);
    assert_eq!(read_layer(&t, 1, 4, 2), want_bytes(&w), "layer 1");
}

#[test]
fn storage_3d_texture_slice_reads_back() {
    let _g = context();
    // SAFETY: plain GL call.
    let t = make(GL_TEXTURE_3D, || unsafe { glTexStorage3D(GL_TEXTURE_3D, 1, GL_RGB10_A2, 4, 2, 2) });
    assert_eq!(param(GL_TEXTURE_3D, GL_TEXTURE_IMMUTABLE_FORMAT), 1);
    let w = words(8, 21);
    upload_slice(GL_TEXTURE_3D, 1, 4, 2, &w);
    assert_eq!(read_layer(&t, 1, 4, 2), want_bytes(&w), "slice 1");
}

#[test]
fn framebuffer_is_complete_with_an_immutable_level() {
    let _g = context();
    let t = storage_2d(4, 4, 3);
    for level in 0..3 {
        // SAFETY: plain GL calls.
        unsafe {
            glBindFramebuffer(GL_FRAMEBUFFER, t.fb);
            glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, t.tex, level);
            assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE, "level {level}");
        }
        assert_eq!(err(), GL_NO_ERROR);
    }
}

/// What the code answers now: the type must match the format the immutable level
/// unpacks from (ES 3.0 3.8.3: a mismatched pair is INVALID_OPERATION).
const EXPECT_TYPES: [GLenum; 3] = [GL_INVALID_OPERATION; 3];
