// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GL_RGB10_A2 pack parameters, pixel pack buffers and multisampling where
//! the build has no class for it, pinned through the GL API (plan #370 item
//! packmsaa-fallback-pin).
//!
//! On the V3D 4.2 build GL_RGB10_A2 is its own render surface
//! (docs/notes/v3d-rgb10a2.md). On the PowerVR and GC7000UL builds and on
//! every `report-es2` build it is RGBA8 storage (see
//! tests/es3_rgb10a2_fallback.rs); the 10/10/10/2 work of the class build
//! must not reach them. This file pins, on those builds, today's answers:
//!
//! - glReadPixels(GL_RGBA, GL_UNSIGNED_BYTE) of a GL_RGB10_A2 texture
//!   attachment with GL_PACK_ALIGNMENT 1, 4 and 8, an odd width,
//!   GL_PACK_ROW_LENGTH / GL_PACK_SKIP_PIXELS / GL_PACK_SKIP_ROWS, into
//!   client memory and into a GL_PIXEL_PACK_BUFFER at a non-zero offset: the
//!   bytes are the RGBA8 storage, at the stride and start the pack state
//!   gives, and every byte outside the pixels (row padding, the skipped
//!   area, the bytes before the offset and after the last row) is untouched
//!   (`rgb10a2_packmsaa_fallback_pack_*`);
//! - glReadPixels(GL_RGBA, GL_UNSIGNED_INT_2_10_10_10_REV) of the same is
//!   GL_INVALID_OPERATION and writes nothing, to client memory and to a
//!   pack buffer (an RGBA8 buffer does not take the pair);
//! - glGetInternalformativ(GL_RENDERBUFFER, GL_RGB10_A2): one sample count,
//!   1 (the back ends keep no samples, `msaa::backend_max_samples` is 0);
//!   glRenderbufferStorageMultisample(GL_RGB10_A2) with 1 sample makes
//!   RGBA8 storage of one sample whose framebuffer has sample buffers (so
//!   glReadPixels of it is GL_INVALID_OPERATION), a resolve blit of it into
//!   a GL_RGB10_A2 renderbuffer copies the RGBA8 bytes (into an RGBA8
//!   renderbuffer it is GL_INVALID_OPERATION: the formats differ), and 2 or
//!   more samples are GL_INVALID_OPERATION
//!   (`rgb10a2_packmsaa_fallback_multisample_*`).
//!
//! An ES 2.0-reporting build has no pixel pack buffer, no pack row length or
//! skips, no multisampled storage and no GL_RGB10_A2 renderbuffer: only the
//! answers of the entry points reachable there are pinned
//! (`rgb10a2_packmsaa_fallback_es2_*`).
//!
//! The context is made once on a stand-in OSAL and back end (as
//! tests/es3_rgb10a2_fallback.rs): its `draw` stores a whole-target clear
//! into an RGBA8888 target as the RGBA8 bytes a back end would; no GPU runs.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online,test-knobs --test es3_rgb10a2_packmsaa_fallback
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online,test-knobs --test es3_rgb10a2_packmsaa_fallback
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table,report-es2 --test es3_rgb10a2_packmsaa_fallback
//! ```
//!
//! Compiled only where the class falls back: the V3D build without
//! `report-es2` stores the class and runs zero tests of this file (its pack
//! and multisample answers are tests/es3_rgb10a2_pack_v3d.rs and
//! tests/es3_rgb10a2_msaa_v3d.rs).

#![cfg(all(feature = "host-test", any(feature = "gpu-powervr", feature = "gpu-vivante", feature = "report-es2")))]
// the ES 2.0 report runs a few pins only; the helpers of the rest are idle there
#![cfg_attr(feature = "report-es2", allow(dead_code, unused_imports))]

use core::ffi::{c_char, c_void};
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{self, GlesDraw, GlesHeap, YakoglBackend};
use yakogl::gl::buffer::*;
use yakogl::gl::consts::*;
use yakogl::gl::framebuffer::*;
use yakogl::gl::state::*;
use yakogl::gl::texture::*;
use yakogl::gl::types::*;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};

/* ---- a context on the host: the smallest OSAL and backend that make one --- */

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

/// Targets the stand-in back end was handed: (surface, stored a clear).
static DRAWS: Mutex<Vec<(u8, bool)>> = Mutex::new(Vec::new());

/// A clear as a back end stores it into an RGBA8888 target: the colour
/// clamped to [0, 1] as one byte a channel, R, G, B, A, every pixel, rows at
/// the pair-padded pitch. Every other target is left alone.
unsafe extern "C" fn b_draw(d: *const GlesDraw) -> i32 {
    let d = &*d;
    for i in 0..d.num_targets as usize {
        let t = &d.targets[i];
        let hit = d.do_clear && (d.clear_target_mask == 0 || d.clear_target_mask & (1 << i) != 0) && t.gpu != 0;
        let rgba8 = t.format == backend::GLES_SURF_RGBA8888;
        DRAWS.lock().unwrap().push((t.format, hit && rgba8));
        if !hit || !rgba8 {
            continue;
        }
        let px = d.clear_rgba.map(unorm8);
        let pitch = ((t.w + 1) & !1) as usize;
        let base = t.gpu as *mut u8;
        for y in 0..t.h as usize {
            for x in 0..t.w as usize {
                core::ptr::copy_nonoverlapping(px.as_ptr(), base.add((y * pitch + x) * 4), 4);
            }
        }
    }
    0
}

/// A colour channel as the byte an RGBA8 target stores: clamped, rounded.
fn unorm8(v: f32) -> u8 {
    (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8
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
    name: c"es3-rgb10a2-packmsaa-fallback".as_ptr() as *const c_char,
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

/// One context for the whole file; the tests take turns on it.
static LOCK: Mutex<()> = Mutex::new(());
static ONCE: Once = Once::new();

fn context() -> MutexGuard<'static, ()> {
    let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    ONCE.call_once(|| {
        // SAFETY: both tables are statics; the lock above serialises the tests.
        unsafe {
            osal_set_ops(&OSAL);
            backend::g_backend = &BACKEND;
            assert!(!yakogl::gl::context::gl_context_create(8, 8).is_null());
        }
    });
    // SAFETY: plain GL call.
    unsafe { glFinish() };
    DRAWS.lock().unwrap().clear();
    // SAFETY: plain GL call.
    while unsafe { glGetError() } != GL_NO_ERROR {}
    g
}

fn err() -> GLenum {
    // SAFETY: plain GL call.
    unsafe { glGetError() }
}

fn renderbuffer(format: GLenum, w: GLsizei, h: GLsizei) -> GLuint {
    let mut rb = 0;
    // SAFETY: room for one name.
    unsafe {
        glGenRenderbuffers(1, &mut rb);
        glBindRenderbuffer(GL_RENDERBUFFER, rb);
        glRenderbufferStorage(GL_RENDERBUFFER, format, w, h);
    }
    rb
}

/// A texture of `w` x `h`, its level 0 from `pixels` (null: only allocate).
fn texture(
    internalformat: GLenum,
    format: GLenum,
    ty: GLenum,
    w: GLsizei,
    h: GLsizei,
    pixels: *const c_void,
) -> GLuint {
    let mut t = 0;
    // SAFETY: room for one name; `pixels` is null or holds w x h texels.
    unsafe {
        glGenTextures(1, &mut t);
        glBindTexture(GL_TEXTURE_2D, t);
        glTexImage2D(GL_TEXTURE_2D, 0, internalformat as GLint, w, h, 0, format, ty, pixels);
    }
    t
}

/// An RGB10_A2 texture of `w` x `h`, uploaded through ES 3.0's one pair
/// for it (table 3.2: GL_RGBA / GL_UNSIGNED_INT_2_10_10_10_REV).
fn rgb10a2_texture(w: GLsizei, h: GLsizei, words: Option<&[u32]>) -> GLuint {
    let p = words.map_or(core::ptr::null(), |w| w.as_ptr() as *const c_void);
    texture(GL_RGB10_A2, GL_RGBA, GL_UNSIGNED_INT_2_10_10_10_REV, w, h, p)
}

/// A framebuffer, bound to GL_FRAMEBUFFER, with renderbuffer `rb` on colour
/// attachment 0.
fn rb_framebuffer(rb: GLuint) -> GLuint {
    let mut fb = 0;
    // SAFETY: room for one name.
    unsafe {
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_RENDERBUFFER, rb);
    }
    fb
}

/// A framebuffer, bound to GL_FRAMEBUFFER, with level 0 of texture `tex` on
/// colour attachment 0.
fn tex_framebuffer(tex: GLuint) -> GLuint {
    let mut fb = 0;
    // SAFETY: room for one name.
    unsafe {
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex, 0);
    }
    fb
}

fn release(fbs: &[GLuint], rbs: &[GLuint], texs: &[GLuint]) {
    // SAFETY: names this test made.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glDeleteFramebuffers(fbs.len() as GLsizei, fbs.as_ptr());
        glDeleteRenderbuffers(rbs.len() as GLsizei, rbs.as_ptr());
        glDeleteTextures(texs.len() as GLsizei, texs.as_ptr());
    }
}

fn status() -> GLenum {
    // SAFETY: plain GL call.
    unsafe { glCheckFramebufferStatus(GL_FRAMEBUFFER) }
}

/// The bound read framebuffer's w x h pixels as GL_RGBA / `ty`, four bytes a
/// pixel, the buffer prefilled with 0x77 so that a read that writes nothing
/// shows.
fn read(w: GLsizei, h: GLsizei, ty: GLenum) -> Vec<u8> {
    let mut px = vec![0x77u8; (w * h * 4) as usize];
    // SAFETY: room for w x h pixels of four bytes.
    unsafe { glReadPixels(0, 0, w, h, GL_RGBA, ty, px.as_mut_ptr() as *mut c_void) };
    px
}

/// GL_IMPLEMENTATION_COLOR_READ_FORMAT and _TYPE of the bound read buffer.
#[allow(dead_code)]
fn implementation_pair() -> (GLenum, GLenum) {
    let (mut f, mut t) = (0, 0);
    // SAFETY: room for one value each.
    unsafe {
        glGetIntegerv(GL_IMPLEMENTATION_COLOR_READ_FORMAT, &mut f);
        glGetIntegerv(GL_IMPLEMENTATION_COLOR_READ_TYPE, &mut t);
    }
    (f as GLenum, t as GLenum)
}

/// The clear of the read-back tests and the RGBA8 bytes it is stored as:
/// 0.2 * 255 = 51, 0.6 * 255 = 153, 1.5 clamps to 255, alpha 0.4 is 102.
/// As a 10/10/10/2 word decoded per channel the same clear would read
/// 51, 153, 255, 85 (alpha 0.4 is 1 of 3): the alpha tells the two apart.
const CLEAR: [f32; 4] = [0.2, 0.6, 1.5, 0.4];
const CLEAR_BYTES: [u8; 4] = [51, 153, 255, 102];

/// Clear the bound framebuffer to [`CLEAR`] and read its `w` x `h` pixels
/// back as GL_RGBA / GL_UNSIGNED_BYTE.
#[allow(dead_code)]
fn clear_and_read(w: GLsizei, h: GLsizei) -> Vec<u8> {
    // SAFETY: plain GL calls.
    unsafe {
        glClearColor(CLEAR[0], CLEAR[1], CLEAR[2], CLEAR[3]);
        glClear(GL_COLOR_BUFFER_BIT);
    }
    assert_eq!(err(), GL_NO_ERROR, "the clear");
    let px = read(w, h, GL_UNSIGNED_BYTE);
    assert_eq!(err(), GL_NO_ERROR, "glReadPixels(GL_RGBA, GL_UNSIGNED_BYTE)");
    px
}

/// A 10/10/10/2 word (R in bits 0..9, G 10..19, B 20..29, A 30..31).
const fn word(r: u32, g: u32, b: u32, a: u32) -> u32 {
    r | g << 10 | b << 20 | a << 30
}

/// The bytes of pixel `(x, y)` of the source texture below, as the RGBA8
/// store holds them: channels that are multiples of a third (341 of 1023,
/// 1 of 3) come out as 0, 85, 170 or 255 exactly, and neighbours differ.
fn chans(x: u32, y: u32) -> [u8; 4] {
    let i = y * 5 + x;
    [i % 4, (y + i) % 4, 3 - i % 4, (i + 1) % 4].map(|v| (v * 85) as u8)
}

/// An RGB10_A2 texture of 5 x 3 (an odd width) holding [`chans`], made the
/// level 0 attachment of a framebuffer bound to GL_FRAMEBUFFER.
fn source() -> (GLuint, GLuint) {
    let words: Vec<u32> = (0..15)
        .map(|i| {
            let c = chans(i % 5, i / 5).map(|b| b as u32 / 85);
            word(c[0] * 341, c[1] * 341, c[2] * 341, c[3])
        })
        .collect();
    let tex = rgb10a2_texture(5, 3, Some(&words));
    assert_eq!(err(), GL_NO_ERROR, "glTexImage2D(GL_RGB10_A2) with words");
    let fb = tex_framebuffer(tex);
    assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE);
    (tex, fb)
}

/// Every pack parameter back to its default.
fn pack_defaults() {
    // SAFETY: plain GL calls.
    unsafe {
        glPixelStorei(GL_PACK_ALIGNMENT, 4);
        if !cfg!(feature = "report-es2") {
            glPixelStorei(GL_PACK_ROW_LENGTH, 0);
            glPixelStorei(GL_PACK_SKIP_PIXELS, 0);
            glPixelStorei(GL_PACK_SKIP_ROWS, 0);
        }
    }
}

/// One pack state: GL_PACK_ALIGNMENT, _ROW_LENGTH, _SKIP_PIXELS, _SKIP_ROWS.
type Pack = (GLint, GLint, GLint, GLint);

/// What a w x h read at (rx, ry) leaves in `len` bytes prefilled with 0x77
/// when the first byte of the pack area is `at`: the pixels of [`chans`] at
/// the row stride the pack state gives, from the skipped rows and pixels on,
/// every other byte as it was (ES 3.0 4.3.2).
fn expected(len: usize, at: usize, rx: u32, ry: u32, w: u32, h: u32, p: Pack) -> Vec<u8> {
    let mut v = vec![0x77u8; len];
    let row = if p.1 > 0 { p.1 as usize } else { w as usize } * 4;
    let stride = row.next_multiple_of(p.0 as usize);
    for j in 0..h as usize {
        for i in 0..w as usize {
            let o = at + (p.3 as usize + j) * stride + (p.2 as usize + i) * 4;
            v[o..o + 4].copy_from_slice(&chans(rx + i as u32, ry + j as u32));
        }
    }
    v
}

/// The pack states of the tests: alignment 1, 4 and 8, then row length,
/// skipped pixels and rows with each.
#[cfg(not(feature = "report-es2"))]
const PACKS: [Pack; 12] = [
    (1, 0, 0, 0),
    (4, 0, 0, 0),
    (8, 0, 0, 0),
    (1, 7, 0, 0),
    (4, 7, 0, 0),
    (8, 7, 0, 0),
    (1, 0, 2, 0),
    (8, 0, 0, 1),
    (8, 6, 1, 2),
    (4, 9, 3, 1),
    (2, 7, 1, 1),
    (1, 6, 2, 1),
];

/// The reads of the tests: (x, y, w, h) of the 5 x 3 source; rows of 12, 12
/// and 20 bytes (16, 16, 24 at alignment 8).
#[cfg(not(feature = "report-es2"))]
const READS: [(u32, u32, u32, u32); 3] = [(0, 0, 5, 3), (1, 1, 3, 2), (2, 0, 3, 3)];

#[cfg(not(feature = "report-es2"))]
fn set_pack(p: Pack) {
    // SAFETY: plain GL calls.
    unsafe {
        glPixelStorei(GL_PACK_ALIGNMENT, p.0);
        glPixelStorei(GL_PACK_ROW_LENGTH, p.1);
        glPixelStorei(GL_PACK_SKIP_PIXELS, p.2);
        glPixelStorei(GL_PACK_SKIP_ROWS, p.3);
    }
}

/// A buffer for one read: room for the biggest pack state plus the offset.
#[cfg(not(feature = "report-es2"))]
const BUF: usize = 4 * 12 * 8 + 64;

#[cfg(not(feature = "report-es2"))]
mod es3 {
    use super::*;

    /// glReadPixels(GL_RGBA, GL_UNSIGNED_BYTE) of a GL_RGB10_A2 texture into
    /// client memory with every pack state of [`PACKS`] and every read of
    /// [`READS`]: the RGBA8 bytes at the stride and start the state gives,
    /// the rest of the buffer untouched.
    #[test]
    fn rgb10a2_packmsaa_fallback_pack_client_memory() {
        let _g = context();
        let (tex, fb) = source();
        for p in PACKS {
            for (x, y, w, h) in READS {
                set_pack(p);
                let mut buf = vec![0x77u8; BUF];
                // SAFETY: BUF holds the largest read.
                unsafe {
                    glReadPixels(
                        x as GLint,
                        y as GLint,
                        w as GLsizei,
                        h as GLsizei,
                        GL_RGBA,
                        GL_UNSIGNED_BYTE,
                        buf.as_mut_ptr() as *mut c_void,
                    )
                };
                assert_eq!(err(), GL_NO_ERROR, "pack {p:?} read {:?}", (x, y, w, h));
                assert_eq!(buf, expected(BUF, 0, x, y, w, h, p), "pack {p:?} read {:?}", (x, y, w, h));
            }
        }
        pack_defaults();
        release(&[fb], &[], &[tex]);
    }

    /// The same into a GL_PIXEL_PACK_BUFFER at the non-zero offset 12: the
    /// bytes before the offset and after the rows are untouched.
    #[test]
    fn rgb10a2_packmsaa_fallback_pack_buffer_offset() {
        let _g = context();
        let (tex, fb) = source();
        let mut pbo = 0;
        // SAFETY: room for one name.
        unsafe { glGenBuffers(1, &mut pbo) };
        for p in PACKS {
            for (x, y, w, h) in READS {
                set_pack(p);
                let fill = vec![0x77u8; BUF];
                // SAFETY: `fill` holds BUF bytes.
                unsafe {
                    glBindBuffer(GL_PIXEL_PACK_BUFFER, pbo);
                    glBufferData(
                        GL_PIXEL_PACK_BUFFER,
                        BUF as GLsizeiptr,
                        fill.as_ptr() as *const c_void,
                        GL_STREAM_READ,
                    );
                }
                assert_eq!(err(), GL_NO_ERROR);
                // SAFETY: with a pack buffer bound the pointer is an offset into it.
                unsafe {
                    glReadPixels(
                        x as GLint,
                        y as GLint,
                        w as GLsizei,
                        h as GLsizei,
                        GL_RGBA,
                        GL_UNSIGNED_BYTE,
                        12 as *mut c_void,
                    )
                };
                assert_eq!(err(), GL_NO_ERROR, "pack {p:?} read {:?}", (x, y, w, h));
                assert_eq!(pack_buffer_bytes(), expected(BUF, 12, x, y, w, h, p), "pack {p:?} read {:?}", (x, y, w, h));
            }
        }
        // SAFETY: plain GL calls.
        unsafe {
            glBindBuffer(GL_PIXEL_PACK_BUFFER, 0);
            glDeleteBuffers(1, &pbo);
        }
        pack_defaults();
        release(&[fb], &[], &[tex]);
    }

    /// The bound pixel pack buffer's BUF bytes.
    fn pack_buffer_bytes() -> Vec<u8> {
        // SAFETY: the whole buffer is mapped for reading, copied, released.
        unsafe {
            let m = glMapBufferRange(GL_PIXEL_PACK_BUFFER, 0, BUF as GLsizeiptr, GL_MAP_READ_BIT) as *const u8;
            assert!(!m.is_null());
            let v = core::slice::from_raw_parts(m, BUF).to_vec();
            glUnmapBuffer(GL_PIXEL_PACK_BUFFER);
            v
        }
    }

    /// What the build answers today to GL_UNSIGNED_INT_2_10_10_10_REV for a
    /// GL_RGB10_A2 attachment (RGBA8 storage, a texture and a renderbuffer):
    /// GL_INVALID_OPERATION, nothing written, to client memory and to a pack
    /// buffer, with default and with non-default pack state.
    #[test]
    fn rgb10a2_packmsaa_fallback_word_type_is_refused() {
        let _g = context();
        let (tex, tfb) = source();
        let rb = renderbuffer(GL_RGB10_A2, 5, 3);
        assert_eq!(err(), GL_NO_ERROR);
        let rfb = rb_framebuffer(rb);
        assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE);
        let mut pbo = 0;
        let fill = vec![0x77u8; BUF];
        // SAFETY: room for one name; `fill` holds BUF bytes.
        unsafe {
            glGenBuffers(1, &mut pbo);
            glBindBuffer(GL_PIXEL_PACK_BUFFER, pbo);
            glBufferData(GL_PIXEL_PACK_BUFFER, BUF as GLsizeiptr, fill.as_ptr() as *const c_void, GL_STREAM_READ);
            glBindBuffer(GL_PIXEL_PACK_BUFFER, 0);
        }
        assert_eq!(err(), GL_NO_ERROR);
        for fb in [tfb, rfb] {
            // SAFETY: plain GL call.
            unsafe { glBindFramebuffer(GL_FRAMEBUFFER, fb) };
            for p in [(4, 0, 0, 0), (1, 7, 2, 1)] {
                set_pack(p);
                let mut buf = vec![0x77u8; BUF];
                // SAFETY: BUF holds the read.
                unsafe {
                    glReadPixels(0, 0, 5, 3, GL_RGBA, GL_UNSIGNED_INT_2_10_10_10_REV, buf.as_mut_ptr() as *mut c_void)
                };
                assert_eq!(err(), GL_INVALID_OPERATION, "client memory, fb {fb}, pack {p:?}");
                assert!(buf.iter().all(|&b| b == 0x77), "nothing written, fb {fb}");
                // SAFETY: an offset into the bound pack buffer.
                unsafe {
                    glBindBuffer(GL_PIXEL_PACK_BUFFER, pbo);
                    glReadPixels(0, 0, 5, 3, GL_RGBA, GL_UNSIGNED_INT_2_10_10_10_REV, 16 as *mut c_void);
                }
                assert_eq!(err(), GL_INVALID_OPERATION, "pack buffer, fb {fb}, pack {p:?}");
                assert!(pack_buffer_bytes().iter().all(|&b| b == 0x77), "buffer untouched, fb {fb}");
                // SAFETY: plain GL call.
                unsafe { glBindBuffer(GL_PIXEL_PACK_BUFFER, 0) };
            }
        }
        // SAFETY: plain GL call.
        unsafe { glDeleteBuffers(1, &pbo) };
        pack_defaults();
        release(&[tfb, rfb], &[rb], &[tex]);
    }

    fn samples_of(pname: GLenum) -> Vec<GLint> {
        let mut v = [-1; 4];
        // SAFETY: room for four values.
        unsafe { glGetInternalformativ(GL_RENDERBUFFER, GL_RGB10_A2, pname, 4, v.as_mut_ptr()) };
        v.to_vec()
    }

    /// GL_RGB10_A2 has one sample count on this build, 1, as GL_RGBA8 has;
    /// 2 and more samples of it are GL_INVALID_OPERATION, 0 is plain
    /// storage and 1 is stored as one sample.
    #[test]
    fn rgb10a2_packmsaa_fallback_multisample_counts() {
        let _g = context();
        assert_eq!(samples_of(GL_NUM_SAMPLE_COUNTS)[0], 1);
        assert_eq!(samples_of(GL_SAMPLES), [1, -1, -1, -1]);
        assert_eq!(err(), GL_NO_ERROR);
        let mut rb = 0;
        // SAFETY: room for one name.
        unsafe {
            glGenRenderbuffers(1, &mut rb);
            glBindRenderbuffer(GL_RENDERBUFFER, rb);
        }
        for n in [2, 4, 8] {
            // SAFETY: plain GL call.
            unsafe { glRenderbufferStorageMultisample(GL_RENDERBUFFER, n, GL_RGB10_A2, 4, 2) };
            assert_eq!(err(), GL_INVALID_OPERATION, "{n} samples");
        }
        for (n, want) in [(0, 0), (1, 1)] {
            // SAFETY: plain GL calls; room for one value.
            let got = unsafe {
                glRenderbufferStorageMultisample(GL_RENDERBUFFER, n, GL_RGB10_A2, 4, 2);
                let mut s = -1;
                glGetRenderbufferParameteriv(GL_RENDERBUFFER, GL_RENDERBUFFER_SAMPLES, &mut s);
                s
            };
            assert_eq!(err(), GL_NO_ERROR, "{n} samples");
            assert_eq!(got, want, "GL_RENDERBUFFER_SAMPLES after {n}");
        }
        release(&[], &[rb], &[]);
    }

    /// A one-sample GL_RGB10_A2 renderbuffer is RGBA8 storage: a clear
    /// reaches the back end as an RGBA8888 target, glReadPixels of its
    /// framebuffer (it has sample buffers) is GL_INVALID_OPERATION, and the
    /// resolve blit into a GL_RGB10_A2 renderbuffer copies the RGBA8 bytes
    /// (alpha 102, not a 10/10/10/2 alpha of 85) while one into an RGBA8
    /// renderbuffer is GL_INVALID_OPERATION (the formats differ).
    #[test]
    fn rgb10a2_packmsaa_fallback_multisample_resolve_is_rgba8() {
        let _g = context();
        let (w, h) = (4, 2);
        let mut ms = 0;
        // SAFETY: room for one name.
        unsafe {
            glGenRenderbuffers(1, &mut ms);
            glBindRenderbuffer(GL_RENDERBUFFER, ms);
            glRenderbufferStorageMultisample(GL_RENDERBUFFER, 1, GL_RGB10_A2, w, h);
        }
        assert_eq!(err(), GL_NO_ERROR, "glRenderbufferStorageMultisample(1, GL_RGB10_A2)");
        let ms_fb = rb_framebuffer(ms);
        assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE);
        // SAFETY: plain GL calls.
        unsafe {
            glClearColor(CLEAR[0], CLEAR[1], CLEAR[2], CLEAR[3]);
            glClear(GL_COLOR_BUFFER_BIT);
        }
        assert_eq!(err(), GL_NO_ERROR, "the clear");
        let px = read(w, h, GL_UNSIGNED_BYTE);
        assert_eq!(err(), GL_INVALID_OPERATION, "glReadPixels of a framebuffer with sample buffers");
        assert!(px.iter().all(|&b| b == 0x77), "nothing written");

        /* ES 3.0 4.3.3: a multisample resolve needs the same format on both
         * sides, and the buffer's format is the internal format asked for
         * (not the RGBA8 it is stored as): RGBA8 is refused, RGB10_A2 goes */
        for (format, ok) in [(GL_RGBA8, false), (GL_RGB10_A2, true)] {
            let dst = renderbuffer(format, w, h);
            assert_eq!(err(), GL_NO_ERROR);
            let d_fb = rb_framebuffer(dst);
            assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE);
            // SAFETY: plain GL calls on framebuffers this test made.
            unsafe {
                glBindFramebuffer(GL_READ_FRAMEBUFFER, ms_fb);
                glBindFramebuffer(GL_DRAW_FRAMEBUFFER, d_fb);
                glBlitFramebuffer(0, 0, w, h, 0, 0, w, h, GL_COLOR_BUFFER_BIT, GL_NEAREST);
            }
            let want = if ok { GL_NO_ERROR } else { GL_INVALID_OPERATION };
            assert_eq!(err(), want, "resolve blit into {format:#x}");
            // SAFETY: plain GL call.
            unsafe { glBindFramebuffer(GL_FRAMEBUFFER, d_fb) };
            let px = read(w, h, GL_UNSIGNED_BYTE);
            assert_eq!(err(), GL_NO_ERROR);
            if ok {
                let draws = DRAWS.lock().unwrap().clone();
                assert!(
                    draws.contains(&(backend::GLES_SURF_RGBA8888, true)),
                    "the clear went out as RGBA8888: {draws:?}"
                );
                assert!(draws.iter().all(|d| d.0 != backend::GLES_SURF_RGB10_A2), "no 10/10/10/2 surface: {draws:?}");
                for (i, p) in px.as_chunks::<4>().0.iter().enumerate() {
                    assert_eq!(*p, CLEAR_BYTES, "pixel {i} resolved into {format:#x}");
                }
            }
            release(&[d_fb], &[dst], &[]);
        }
        release(&[ms_fb], &[ms], &[]);
    }
}

/* ---- the ES 2.0 report ---------------------------------------------------------- */

#[cfg(feature = "report-es2")]
mod es2 {
    use super::*;

    /// GL_PACK_ALIGNMENT 1, 4 and 8 with an odd width on an RGBA8
    /// renderbuffer (all an ES 2.0 context has): rows at the aligned stride,
    /// the padding untouched.
    #[test]
    fn rgb10a2_packmsaa_fallback_es2_pack_alignment() {
        let _g = context();
        let rb = renderbuffer(GL_RGBA8, 5, 3);
        assert_eq!(err(), GL_NO_ERROR);
        let fb = rb_framebuffer(rb);
        assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE);
        // SAFETY: plain GL calls.
        unsafe {
            glClearColor(CLEAR[0], CLEAR[1], CLEAR[2], CLEAR[3]);
            glClear(GL_COLOR_BUFFER_BIT);
        }
        for a in [1, 4, 8] {
            // SAFETY: plain GL call.
            unsafe { glPixelStorei(GL_PACK_ALIGNMENT, a) };
            let mut buf = vec![0x77u8; 80];
            // SAFETY: room for three rows of at most 24 bytes.
            unsafe { glReadPixels(0, 0, 5, 3, GL_RGBA, GL_UNSIGNED_BYTE, buf.as_mut_ptr() as *mut c_void) };
            assert_eq!(err(), GL_NO_ERROR, "alignment {a}");
            let stride = 20usize.next_multiple_of(a as usize);
            let mut want = vec![0x77u8; 80];
            for j in 0..3 {
                for i in 0..5 {
                    want[j * stride + i * 4..][..4].copy_from_slice(&CLEAR_BYTES);
                }
            }
            assert_eq!(buf, want, "alignment {a}");
        }
        pack_defaults();
        release(&[fb], &[rb], &[]);
    }

    /// GL_RGB10_A2 is no ES 2.0 renderbuffer format, so no multisampled
    /// storage of it exists either: GL_INVALID_ENUM, as for the plain
    /// storage call (es3_rgb10a2_fallback).
    #[test]
    fn rgb10a2_packmsaa_fallback_es2_multisample_is_not_allocated() {
        let _g = context();
        let mut rb = 0;
        // SAFETY: room for one name.
        unsafe {
            glGenRenderbuffers(1, &mut rb);
            glBindRenderbuffer(GL_RENDERBUFFER, rb);
            glRenderbufferStorageMultisample(GL_RENDERBUFFER, 1, GL_RGB10_A2, 4, 2);
        }
        assert_eq!(err(), GL_INVALID_ENUM, "glRenderbufferStorageMultisample(GL_RGB10_A2)");
        release(&[], &[rb], &[]);
    }
}
