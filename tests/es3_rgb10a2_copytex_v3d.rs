// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! glCopyTexImage2D / glCopyTexSubImage2D from a GL_RGB10_A2 read buffer on
//! the V3D host back end, through the GL entry points only (plan #370,
//! copytex-api-draft).
//!
//! On the V3D 4.2 build GL_RGB10_A2 is its own render surface
//! (`GLES_SURF_RGB10_A2`): a pixel is the little-endian 10/10/10/2 word, red
//! in bits 0..9, alpha in 30..31. glReadPixels decodes it (each field becomes
//! (v * 255 + max / 2) / max, max 1023 or 3), and glCopyTex* reads through
//! glReadPixels RGBA/UNSIGNED_BYTE, so a copy into an RGBA8 texture already
//! holds the decoded bytes (those rows run). A copy into an RGB10_A2 texture
//! is lossy today (the words come back at 8-bit precision): item
//! copytex-rgb10a2 makes it exact, and those two rows are `#[ignore]`d with
//! that cause and carry the exact words they expect. The error rows run.
//!
//! The read buffer is an RGB10_A2 renderbuffer or an RGB10_A2 texture
//! attachment. Its words are put there by a texture upload (texture) or by a
//! class-to-class glBlitFramebuffer out of such a texture (renderbuffer).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_rgb10a2_copytex_v3d
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
    name: c"es3-rgb10a2-copytex".as_ptr() as *const c_char,
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
};

static LOCK: Mutex<()> = Mutex::new(());
static ONCE: Once = Once::new();

const W: GLsizei = 4;
const H: GLsizei = 2;

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

/// The RGBA8 bytes glReadPixels hands out for a word: (v * 255 + max / 2) / max.
fn decode8(w: u32) -> [u8; 4] {
    let c = |s: u32, m: u32| ((((w >> s) & m) * 255 + m / 2) / m) as u8;
    [c(0, 0x3ff), c(10, 0x3ff), c(20, 0x3ff), c(30, 3)]
}

/// A W x H image of words with the edge values of every field: 0, the maximum,
/// the midpoints of the 8-bit decode, and mixed values.
fn src_words() -> Vec<u32> {
    let px = [
        (1023, 512, 0, 3),
        (341, 1, 1022, 1),
        (100, 900, 700, 2),
        (0, 1023, 255, 0),
        (64, 128, 960, 1),
        (513, 300, 3, 3),
        (777, 20, 1000, 2),
        (900, 640, 33, 0),
    ];
    px.iter().map(|&(r, g, b, a)| a2(r, g, b, a)).collect()
}

/// A colour attachment of its own framebuffer: a texture, or a renderbuffer.
struct Att {
    fb: GLuint,
    tex: GLuint,
    rb: GLuint,
    w: GLsizei,
    h: GLsizei,
}

impl Drop for Att {
    fn drop(&mut self) {
        // SAFETY: names this test made (0 is skipped by GL).
        unsafe {
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
            glDeleteFramebuffers(1, &self.fb);
            glDeleteTextures(1, &self.tex);
            glDeleteRenderbuffers(1, &self.rb);
        }
    }
}

fn attach(internal: GLenum, w: GLsizei, h: GLsizei, ty: GLenum, px: *const c_void) -> Att {
    let (mut fb, mut tex) = (0, 0);
    // SAFETY: room for one name each; `px` is null or holds w x h texels of `ty`.
    unsafe {
        glGenTextures(1, &mut tex);
        glBindTexture(GL_TEXTURE_2D, tex);
        glTexImage2D(GL_TEXTURE_2D, 0, internal as GLint, w, h, 0, GL_RGBA, ty, px);
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex, 0);
        assert_eq!(err(), GL_NO_ERROR, "texture {internal:#x} {w}x{h}");
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE, "{internal:#x}");
    }
    Att { fb, tex, rb: 0, w, h }
}

fn tex10(w: GLsizei, h: GLsizei, words: &[u32]) -> Att {
    assert_eq!(words.len(), (w * h) as usize);
    attach(GL_RGB10_A2, w, h, GL_UNSIGNED_INT_2_10_10_10_REV, words.as_ptr() as *const c_void)
}

fn tex8(w: GLsizei, h: GLsizei, fill: [u8; 4]) -> Att {
    let px = vec![fill; (w * h) as usize];
    attach(GL_RGBA8, w, h, GL_UNSIGNED_BYTE, px.as_ptr() as *const c_void)
}

/// An RGB10_A2 renderbuffer holding `words`: made empty, filled by a
/// class-to-class blit out of a texture of the words (which keeps them).
fn rb10(w: GLsizei, h: GLsizei, words: &[u32]) -> Att {
    let src = tex10(w, h, words);
    let (mut fb, mut rb) = (0, 0);
    // SAFETY: room for one name each.
    unsafe {
        glGenRenderbuffers(1, &mut rb);
        glBindRenderbuffer(GL_RENDERBUFFER, rb);
        glRenderbufferStorage(GL_RENDERBUFFER, GL_RGB10_A2, w, h);
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_RENDERBUFFER, rb);
        assert_eq!(err(), GL_NO_ERROR, "renderbuffer {w}x{h}");
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE);
        glBindFramebuffer(GL_READ_FRAMEBUFFER, src.fb);
        glReadBuffer(GL_COLOR_ATTACHMENT0);
        glBindFramebuffer(GL_DRAW_FRAMEBUFFER, fb);
        let draw = [GL_COLOR_ATTACHMENT0];
        glDrawBuffers(1, draw.as_ptr());
        glBlitFramebuffer(0, 0, w, h, 0, 0, w, h, GL_COLOR_BUFFER_BIT, GL_NEAREST);
        assert_eq!(err(), GL_NO_ERROR, "filling the renderbuffer");
    }
    Att { fb, tex: 0, rb, w, h }
}

/// The two kinds of read buffer, each holding `words`.
fn read_buffers(words: &[u32]) -> [(&'static str, Att); 2] {
    [("renderbuffer", rb10(W, H, words)), ("texture", tex10(W, H, words))]
}

/// Make `a` the read framebuffer, attachment 0.
fn read_from(a: &Att) {
    // SAFETY: plain GL calls.
    unsafe {
        glBindFramebuffer(GL_READ_FRAMEBUFFER, a.fb);
        glReadBuffer(GL_COLOR_ATTACHMENT0);
    }
}

/// An RGBA8 attachment through glReadPixels.
fn read8(a: &Att) -> Vec<[u8; 4]> {
    let mut px = vec![0x77u8; (a.w * a.h * 4) as usize];
    // SAFETY: room for w x h RGBA bytes.
    unsafe {
        glBindFramebuffer(GL_READ_FRAMEBUFFER, a.fb);
        glReadBuffer(GL_COLOR_ATTACHMENT0);
        glReadPixels(0, 0, a.w, a.h, GL_RGBA, GL_UNSIGNED_BYTE, px.as_mut_ptr() as *mut c_void);
    }
    assert_eq!(err(), GL_NO_ERROR, "glReadPixels of an RGBA8 attachment");
    px.chunks(4).map(|c| [c[0], c[1], c[2], c[3]]).collect()
}

/// An RGB10_A2 attachment through glReadPixels RGBA/UNSIGNED_INT_2_10_10_10_REV.
fn read_words(a: &Att) -> Vec<u32> {
    let mut px = vec![0xdead_beefu32; (a.w * a.h) as usize];
    // SAFETY: room for w x h words.
    unsafe {
        glBindFramebuffer(GL_READ_FRAMEBUFFER, a.fb);
        glReadBuffer(GL_COLOR_ATTACHMENT0);
        glReadPixels(0, 0, a.w, a.h, GL_RGBA, GL_UNSIGNED_INT_2_10_10_10_REV, px.as_mut_ptr() as *mut c_void);
    }
    assert_eq!(err(), GL_NO_ERROR, "glReadPixels of an RGB10_A2 attachment as words");
    px
}

/// A fresh texture `t` bound to GL_TEXTURE_2D, framebuffer `fb` over it,
/// for a copy destination: made by `f` (which binds the texture).
fn dest(f: impl FnOnce()) -> Att {
    let (mut fb, mut tex) = (0, 0);
    // SAFETY: room for one name each.
    unsafe {
        glGenTextures(1, &mut tex);
        glBindTexture(GL_TEXTURE_2D, tex);
        f();
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex, 0);
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE);
    }
    Att { fb, tex, rb: 0, w: W, h: H }
}

/// The RGBA8 bytes a decoding copy leaves.
fn decoded(words: &[u32]) -> Vec<[u8; 4]> {
    words.iter().map(|&w| decode8(w)).collect()
}

/// The decode is the one glReadPixels does: pinned on a few of the words.
#[test]
fn decode_table() {
    assert_eq!(decode8(a2(1023, 512, 0, 3)), [255, 128, 0, 255]);
    assert_eq!(decode8(a2(341, 1, 1022, 1)), [85, 0, 255, 85]);
    assert_eq!(decode8(a2(100, 900, 700, 2)), [25, 224, 174, 170]);
    assert_eq!(decode8(a2(0, 1023, 255, 0)), [0, 255, 64, 0]);
}

/// glCopyTexImage2D of the read buffer into a GL_RGBA8 texture: the bytes are
/// the decoded fields.
#[test]
fn copy_tex_image_into_rgba8_decodes() {
    let _g = context();
    let w = src_words();
    for (kind, r) in read_buffers(&w) {
        read_from(&r);
        let d = dest(|| unsafe { glCopyTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA8, 0, 0, W, H, 0) });
        assert_eq!(err(), GL_NO_ERROR, "{kind}: glCopyTexImage2D");
        assert_eq!(read8(&d), decoded(&w), "{kind}: decoded bytes");
    }
}

/// glCopyTexSubImage2D of the read buffer into an existing RGBA8 texture.
#[test]
fn copy_tex_sub_image_into_rgba8_decodes() {
    let _g = context();
    let w = src_words();
    for (kind, r) in read_buffers(&w) {
        let d = tex8(W, H, [0x5a; 4]);
        read_from(&r);
        // SAFETY: plain GL calls.
        unsafe {
            glBindTexture(GL_TEXTURE_2D, d.tex);
            glCopyTexSubImage2D(GL_TEXTURE_2D, 0, 0, 0, 0, 0, W, H);
        }
        assert_eq!(err(), GL_NO_ERROR, "{kind}: glCopyTexSubImage2D");
        assert_eq!(read8(&d), decoded(&w), "{kind}: decoded bytes");
    }
}

/// glCopyTexSubImage2D into an RGB10_A2 texture keeps the words (a copy
/// inside the class), read back as GL_UNSIGNED_INT_2_10_10_10_REV; a
/// sub-rectangle lands where it is addressed.
#[test]
#[ignore = "copytex-rgb10a2: glCopyTex* copies through RGBA8, so an RGB10_A2 destination loses the low bits"]
fn copy_tex_sub_image_into_rgb10a2_keeps_words() {
    let _g = context();
    let w = src_words();
    for (kind, r) in read_buffers(&w) {
        let d = tex10(W, H, &vec![0; (W * H) as usize]);
        read_from(&r);
        // SAFETY: plain GL calls.
        unsafe {
            glBindTexture(GL_TEXTURE_2D, d.tex);
            glCopyTexSubImage2D(GL_TEXTURE_2D, 0, 0, 0, 0, 0, W, H);
        }
        assert_eq!(err(), GL_NO_ERROR, "{kind}: glCopyTexSubImage2D");
        assert_eq!(read_words(&d), w, "{kind}: the words");

        /* the 2 x 1 block at (1, 1) of the source into (2, 0) of a cleared copy */
        let d = tex10(W, H, &vec![0; (W * H) as usize]);
        read_from(&r);
        // SAFETY: plain GL calls.
        unsafe {
            glBindTexture(GL_TEXTURE_2D, d.tex);
            glCopyTexSubImage2D(GL_TEXTURE_2D, 0, 2, 0, 1, 1, 2, 1);
        }
        assert_eq!(err(), GL_NO_ERROR, "{kind}: glCopyTexSubImage2D block");
        let mut want = vec![0u32; (W * H) as usize];
        want[2] = w[5];
        want[3] = w[6];
        assert_eq!(read_words(&d), want, "{kind}: the block");
    }
}

/// glCopyTexImage2D into an RGB10_A2 internalformat is a copy inside the
/// class: the words survive.
#[test]
#[ignore = "copytex-rgb10a2: glCopyTex* copies through RGBA8, so an RGB10_A2 destination loses the low bits"]
fn copy_tex_image_into_rgb10a2_keeps_words() {
    let _g = context();
    let w = src_words();
    for (kind, r) in read_buffers(&w) {
        read_from(&r);
        let d = dest(|| unsafe { glCopyTexImage2D(GL_TEXTURE_2D, 0, GL_RGB10_A2, 0, 0, W, H, 0) });
        assert_eq!(err(), GL_NO_ERROR, "{kind}: glCopyTexImage2D");
        assert_eq!(read_words(&d), w, "{kind}: the words");
    }
}

/// ES 3.0 table 3.15: a fixed-point source cannot feed an integer
/// internalformat, for either entry point's destination.
#[test]
fn integer_destination_is_invalid_operation() {
    let _g = context();
    let w = src_words();
    for (kind, r) in read_buffers(&w) {
        read_from(&r);
        for internal in [GL_RGBA8UI, GL_RGBA8I, GL_RG32UI, GL_R8UI] {
            let mut tex = 0;
            // SAFETY: room for one name.
            unsafe {
                glGenTextures(1, &mut tex);
                glBindTexture(GL_TEXTURE_2D, tex);
                glCopyTexImage2D(GL_TEXTURE_2D, 0, internal, 0, 0, W, H, 0);
                assert_eq!(err(), GL_INVALID_OPERATION, "{kind}: glCopyTexImage2D into {internal:#x}");
                glDeleteTextures(1, &tex);
            }
        }
        /* an existing integer texture */
        let mut tex = 0;
        let px = vec![0u8; (W * H * 4) as usize];
        // SAFETY: room for one name; `px` holds W x H RGBA8UI texels.
        unsafe {
            glGenTextures(1, &mut tex);
            glBindTexture(GL_TEXTURE_2D, tex);
            glTexImage2D(
                GL_TEXTURE_2D,
                0,
                GL_RGBA8UI as GLint,
                W,
                H,
                0,
                GL_RGBA_INTEGER,
                GL_UNSIGNED_BYTE,
                px.as_ptr() as *const c_void,
            );
            assert_eq!(err(), GL_NO_ERROR);
            glCopyTexSubImage2D(GL_TEXTURE_2D, 0, 0, 0, 0, 0, W, H);
            assert_eq!(err(), GL_INVALID_OPERATION, "{kind}: glCopyTexSubImage2D into RGBA8UI");
            glDeleteTextures(1, &tex);
        }
    }
}

/// A rectangle outside the read buffer or a bad level is still an error, and
/// a failed copy leaves the destination untouched.
#[test]
fn bad_arguments_leave_the_destination() {
    let _g = context();
    let w = src_words();
    for (kind, r) in read_buffers(&w) {
        let d = tex8(W, H, [0x5a; 4]);
        read_from(&r);
        // SAFETY: plain GL calls.
        unsafe {
            glBindTexture(GL_TEXTURE_2D, d.tex);
            glCopyTexSubImage2D(GL_TEXTURE_2D, 0, 2, 0, 0, 0, W, H);
            assert_eq!(err(), GL_INVALID_VALUE, "{kind}: rectangle past the destination");
            glCopyTexSubImage2D(GL_TEXTURE_2D, 2, 0, 0, 0, 0, 1, 1);
            assert_eq!(err(), GL_INVALID_OPERATION, "{kind}: level without an image");
        }
        assert_eq!(read8(&d), vec![[0x5a; 4]; (W * H) as usize], "{kind}: untouched");
    }
}
