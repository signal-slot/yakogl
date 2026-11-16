// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GL_RGB10_A2 where the build has no class for it, pinned through the GL
//! API (plan #370 item rgb10a2-fallback-pin).
//!
//! On the V3D 4.2 build GL_RGB10_A2 is its own render surface
//! (`GLES_SURF_RGB10_A2`, the 10/10/10/2 word: src/gl/framebuffer.rs
//! `rgb10a2_surface`, docs/notes/v3d-rgb10a2.md). Everywhere else a
//! GL_RGB10_A2 renderbuffer is RGBA8888 and a texture of the name is in the
//! RGBA8 class (es3_formats' fallback): the PowerVR and the GC7000 builds,
//! whose back ends do not draw the surface, and every `report-es2` build.
//! The 10/10/10/2 read-back of the class build (glReadPixels decoding the
//! word, GL_UNSIGNED_INT_2_10_10_10_REV, the implementation read pair) must
//! not reach those builds; this file holds what they answer today:
//!
//! - a cleared GL_RGB10_A2 renderbuffer and texture attachment read back
//!   with GL_RGBA / GL_UNSIGNED_BYTE as the RGBA8 bytes they hold;
//! - GL_IMPLEMENTATION_COLOR_READ_FORMAT / _TYPE answer GL_RGBA /
//!   GL_UNSIGNED_BYTE for them (framebuffer.rs `implementation_color_read`:
//!   a colour buffer that is neither integer is the RGBA8 pair);
//! - glReadPixels with GL_RGBA / GL_UNSIGNED_INT_2_10_10_10_REV is
//!   GL_INVALID_OPERATION and writes nothing (a known pair the RGBA8 buffer
//!   does not take, `read_pixels`);
//! - glBlitFramebuffer NEAREST from such a texture into an RGBA8 texture
//!   copies the bytes unchanged.
//!
//! An ES 2.0-reporting build cannot allocate GL_RGB10_A2 at all (ES 2.0
//! names no such format): renderbuffer storage is GL_INVALID_ENUM, the
//! texture GL_INVALID_VALUE (as tests/es2_report_guard.rs holds; with ES
//! 3.0's GL_UNSIGNED_INT_2_10_10_10_REV, not an ES 2.0 type,
//! GL_INVALID_ENUM), and the 10/10/10/2 read type is GL_INVALID_OPERATION on an RGBA8 framebuffer;
//! those errors are what it runs here (`rgb10a2_fallback_es2_*`).
//!
//! The context is made once on a stand-in OSAL and back end (as
//! tests/es3_srgb.rs): its `draw` stores a whole-target clear into an
//! RGBA8888 target as the RGBA8 bytes a back end would, at the core's
//! pair-padded pitch, and records each target's surface; no GPU runs.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online,test-knobs --test es3_rgb10a2_fallback
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online,test-knobs --test es3_rgb10a2_fallback
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table,report-es2 --test es3_rgb10a2_fallback
//! ```
//!
//! Compiled only where the class falls back: the V3D build without
//! `report-es2` stores the class and runs zero tests of this file (its
//! answers are the read-back item's, src/gl/blit.rs and
//! tests/es3_readback_v3d.rs).

#![cfg(all(feature = "host-test", any(feature = "gpu-powervr", feature = "gpu-vivante", feature = "report-es2")))]
// the ES 2.0 report runs the allocation pins only; the helpers of the rest are idle there
#![cfg_attr(feature = "report-es2", allow(dead_code, unused_imports))]

use core::ffi::{c_char, c_void};
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{self, GlesDraw, GlesHeap, YakoglBackend};
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
    name: c"es3-rgb10a2-fallback".as_ptr() as *const c_char,
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

/* ---- the ES 2.0 report: GL_RGB10_A2 is no format ---------------------------- */

#[cfg(feature = "report-es2")]
mod es2 {
    use super::*;

    /// ES 2.0 4.4.3 names no GL_RGB10_A2 renderbuffer storage and 3.7.1 no
    /// such texture internal format: neither is allocated, so nothing of the
    /// class build's read-back can be reached.
    #[test]
    fn rgb10a2_fallback_es2_is_not_allocated() {
        let _g = context();
        let rb = renderbuffer(GL_RGB10_A2, 4, 2);
        assert_eq!(err(), GL_INVALID_ENUM, "glRenderbufferStorage(GL_RGB10_A2)");
        let tex = texture(GL_RGB10_A2, GL_RGBA, GL_UNSIGNED_BYTE, 4, 2, core::ptr::null());
        assert_eq!(err(), GL_INVALID_VALUE, "glTexImage2D(GL_RGB10_A2, GL_RGBA, GL_UNSIGNED_BYTE)");
        let tex2 = texture(GL_RGB10_A2, GL_RGBA, GL_UNSIGNED_INT_2_10_10_10_REV, 4, 2, core::ptr::null());
        assert_eq!(err(), GL_INVALID_ENUM, "glTexImage2D(GL_RGB10_A2, .., GL_UNSIGNED_INT_2_10_10_10_REV)");
        release(&[], &[rb], &[tex, tex2]);
    }

    /// The 10/10/10/2 read type on the RGBA8 buffers an ES 2.0 context has:
    /// GL_INVALID_OPERATION, nothing written; the implementation pair stays
    /// GL_RGBA / GL_UNSIGNED_BYTE.
    #[test]
    fn rgb10a2_fallback_es2_read_type_is_refused() {
        let _g = context();
        let rb = renderbuffer(GL_RGBA8, 4, 2);
        assert_eq!(err(), GL_NO_ERROR);
        let fb = rb_framebuffer(rb);
        assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE);
        assert_eq!(implementation_pair(), (GL_RGBA, GL_UNSIGNED_BYTE));
        assert_eq!(err(), GL_NO_ERROR);
        let px = read(4, 2, GL_UNSIGNED_INT_2_10_10_10_REV);
        assert_eq!(err(), GL_INVALID_OPERATION, "glReadPixels(GL_RGBA, GL_UNSIGNED_INT_2_10_10_10_REV)");
        assert!(px.iter().all(|&b| b == 0x77), "nothing written");
        release(&[fb], &[rb], &[]);
    }
}

/* ---- ES 3.0 on a build without the class -------------------------------------- */

#[cfg(not(feature = "report-es2"))]
mod es3 {
    use super::*;

    /// The premise: this build's back end does not draw the 10/10/10/2
    /// surface, so GL_RGB10_A2 falls back to RGBA8 (framebuffer.rs
    /// `rgb10a2_surface`).
    #[test]
    fn rgb10a2_fallback_build_has_no_class() {
        assert!(!backend::rt_format_drawable(backend::GLES_SURF_RGB10_A2));
    }

    /// A cleared GL_RGB10_A2 renderbuffer reaches the back end as an
    /// RGBA8888 target and reads back the RGBA8 bytes it holds; the
    /// implementation pair is GL_RGBA / GL_UNSIGNED_BYTE; the 10/10/10/2
    /// read type is GL_INVALID_OPERATION and writes nothing.
    #[test]
    fn rgb10a2_fallback_renderbuffer_reads_as_rgba8() {
        let _g = context();
        let rb = renderbuffer(GL_RGB10_A2, 4, 2);
        assert_eq!(err(), GL_NO_ERROR, "glRenderbufferStorage(GL_RGB10_A2)");
        let fb = rb_framebuffer(rb);
        assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE);
        let px = clear_and_read(4, 2);
        let draws = DRAWS.lock().unwrap().clone();
        assert!(draws.contains(&(backend::GLES_SURF_RGBA8888, true)), "the clear went out as RGBA8888: {draws:?}");
        assert!(draws.iter().all(|d| d.0 != backend::GLES_SURF_RGB10_A2), "no 10/10/10/2 surface: {draws:?}");
        for (i, p) in px.as_chunks::<4>().0.iter().enumerate() {
            assert_eq!(*p, CLEAR_BYTES, "pixel {i}");
        }
        assert_eq!(implementation_pair(), (GL_RGBA, GL_UNSIGNED_BYTE));
        assert_eq!(err(), GL_NO_ERROR);
        let words = read(4, 2, GL_UNSIGNED_INT_2_10_10_10_REV);
        assert_eq!(err(), GL_INVALID_OPERATION, "glReadPixels(GL_RGBA, GL_UNSIGNED_INT_2_10_10_10_REV)");
        assert!(words.iter().all(|&b| b == 0x77), "nothing written");
        release(&[fb], &[rb], &[]);
    }

    /// The same for a GL_RGB10_A2 texture attachment (the RGBA8 class).
    #[test]
    fn rgb10a2_fallback_texture_reads_as_rgba8() {
        let _g = context();
        let tex = rgb10a2_texture(4, 2, None);
        assert_eq!(err(), GL_NO_ERROR, "glTexImage2D(GL_RGB10_A2)");
        let fb = tex_framebuffer(tex);
        assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE);
        let px = clear_and_read(4, 2);
        let draws = DRAWS.lock().unwrap().clone();
        assert!(draws.contains(&(backend::GLES_SURF_RGBA8888, true)), "the clear went out as RGBA8888: {draws:?}");
        assert!(draws.iter().all(|d| d.0 != backend::GLES_SURF_RGB10_A2), "no 10/10/10/2 surface: {draws:?}");
        for (i, p) in px.as_chunks::<4>().0.iter().enumerate() {
            assert_eq!(*p, CLEAR_BYTES, "pixel {i}");
        }
        assert_eq!(implementation_pair(), (GL_RGBA, GL_UNSIGNED_BYTE));
        assert_eq!(err(), GL_NO_ERROR);
        let words = read(4, 2, GL_UNSIGNED_INT_2_10_10_10_REV);
        assert_eq!(err(), GL_INVALID_OPERATION, "glReadPixels(GL_RGBA, GL_UNSIGNED_INT_2_10_10_10_REV)");
        assert!(words.iter().all(|&b| b == 0x77), "nothing written");
        release(&[fb], &[], &[tex]);
    }

    /// A 10/10/10/2 word (R in bits 0..9, G 10..19, B 20..29, A 30..31).
    const fn word(r: u32, g: u32, b: u32, a: u32) -> u32 {
        r | g << 10 | b << 20 | a << 30
    }

    /// glBlitFramebuffer NEAREST from a GL_RGB10_A2 texture into an RGBA8
    /// texture copies the RGBA8 bytes the source holds, unchanged. The
    /// source is uploaded as words whose channels are multiples of a third
    /// (341 of 1023, 1 of 3), so the RGBA8 store holds 0, 85, 170 or 255
    /// exactly; every pixel differs from its neighbours.
    #[test]
    fn rgb10a2_fallback_blit_nearest_copies_the_bytes() {
        let _g = context();
        let (w, h) = (4, 2);
        let level = |v: u32| v * 341;
        let byte = |v: u32| (v * 85) as u8;
        let chans: Vec<[u32; 4]> =
            (0..w * h).map(|i| [i % 4, (i / 4 + i) % 4, 3 - i % 4, (i + 1) % 4]).map(|c| c.map(|v| v as u32)).collect();
        let words: Vec<u32> = chans.iter().map(|c| word(level(c[0]), level(c[1]), level(c[2]), c[3])).collect();
        let want: Vec<u8> = chans.iter().flat_map(|c| c.map(byte)).collect();

        let src = rgb10a2_texture(w, h, Some(&words));
        assert_eq!(err(), GL_NO_ERROR, "glTexImage2D(GL_RGB10_A2) with words");
        let s_fb = tex_framebuffer(src);
        assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE);
        assert_eq!(read(w, h, GL_UNSIGNED_BYTE), want, "the source holds the RGBA8 bytes");
        assert_eq!(err(), GL_NO_ERROR);

        let dst = texture(GL_RGBA8, GL_RGBA, GL_UNSIGNED_BYTE, w, h, core::ptr::null());
        assert_eq!(err(), GL_NO_ERROR);
        let d_fb = tex_framebuffer(dst);
        assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE);
        // SAFETY: plain GL calls on framebuffers this test made.
        unsafe {
            glBindFramebuffer(GL_READ_FRAMEBUFFER, s_fb);
            glBindFramebuffer(GL_DRAW_FRAMEBUFFER, d_fb);
            glBlitFramebuffer(0, 0, w, h, 0, 0, w, h, GL_COLOR_BUFFER_BIT, GL_NEAREST);
        }
        assert_eq!(err(), GL_NO_ERROR, "glBlitFramebuffer(NEAREST)");
        // SAFETY: plain GL call.
        unsafe { glBindFramebuffer(GL_FRAMEBUFFER, d_fb) };
        assert_eq!(read(w, h, GL_UNSIGNED_BYTE), want, "the RGBA8 destination holds the same bytes");
        assert_eq!(err(), GL_NO_ERROR);
        release(&[s_fb, d_fb], &[], &[src, dst]);
    }
}
