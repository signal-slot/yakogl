// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! A GL_RGB10_A2 colour attachment beside a GL_RGBA8 one in one framebuffer,
//! where the build has no class for the format (plan #370 item docs-draft2;
//! the class build's rows are tests/es3_rgb10a2_mrt_v3d.rs).
//!
//! On the V3D 4.2 build GL_RGB10_A2 is its own texture class and render
//! surface (`GLES_TEXFMT_RGB10A2` / `GLES_SURF_RGB10_A2`, the 10/10/10/2
//! word: src/gl/framebuffer.rs `rgb10a2_surface`, docs/notes/v3d-rgb10a2.md);
//! there an MRT framebuffer holding it beside an RGBA8 attachment is a mixed
//! draw. Everywhere else a GL_RGB10_A2 renderbuffer is RGBA8888 and a texture
//! of the name is in the RGBA8 class (es3_formats' fallback): the PowerVR and
//! GC7000 builds and every `report-es2` build. This file holds what the two
//! attachments answer there side by side (sibling of
//! tests/es3_rgb10a2_texclear_fallback.rs and
//! tests/es3_rgb10a2_layers_fallback.rs, same stand-in OSAL and back end, no
//! GPU), as textures and as renderbuffers, GL_RGB10_A2 on attachment 0 and on
//! attachment 1:
//!
//! - the framebuffer is GL_FRAMEBUFFER_COMPLETE with both points attached,
//!   and the back end is handed the GL_RGB10_A2 attachment as an RGBA8888
//!   target exactly like the GL_RGBA8 one — no `GLES_SURF_RGB10_A2` target
//!   ever appears in the draw's target list (`fb_target_n`,
//!   `resolve_targets`);
//! - glDrawBuffers({GL_COLOR_ATTACHMENT0, GL_COLOR_ATTACHMENT1}) is accepted;
//!   a glClear clears every draw buffer, and glClearBufferfv(GL_COLOR, n)
//!   clears that draw buffer alone and leaves the other attachment's bytes
//!   (ES 3.0 4.2.3, the pending clear's `clear_target_mask`): each attachment
//!   reads back its own clear's RGBA8 bytes through its own glReadBuffer, the
//!   GL_RGB10_A2 point holding the plain RGBA8 rounding of the clear colour
//!   (the bytes texclear_fallback pins there, not a 10/10/10/2 decode);
//! - GL_IMPLEMENTATION_COLOR_READ_FORMAT / _TYPE answer GL_RGBA /
//!   GL_UNSIGNED_BYTE for each read buffer, the GL_RGB10_A2 point among them
//!   (`implementation_color_read`: the 10/10/10/2 pair is only where
//!   `rgb10a2_surface` stores the class);
//! - glReadPixels GL_RGBA / GL_UNSIGNED_INT_2_10_10_10_REV at either point is
//!   GL_INVALID_OPERATION and writes nothing, as for the GL_RGBA8 one
//!   (`read_pixels`: an RGBA8 buffer does not take the packed read pair);
//! - glBlitFramebuffer NEAREST from each read buffer into a plain RGBA8
//!   framebuffer copies that buffer's bytes unchanged.
//!
//! An ES 2.0-reporting build cannot allocate GL_RGB10_A2 at all (ES 2.0 names
//! no such format): the renderbuffer storage is GL_INVALID_ENUM, the texture
//! with an ES 2.0 type GL_INVALID_VALUE and with GL_UNSIGNED_INT_2_10_10_10_REV
//! GL_INVALID_ENUM (as tests/es3_rgb10a2_fallback.rs holds). Two GL_RGBA8
//! attachments there still take glDrawBuffers, glClearBufferfv and
//! glReadBuffer the way the `es2` module pins them; glBlitFramebuffer is
//! refused outright (GL_INVALID_OPERATION — it is not an ES 2.0 command), and
//! so is the read/draw framebuffer split a blit between two framebuffers
//! needs: glBindFramebuffer(GL_READ_FRAMEBUFFER / GL_DRAW_FRAMEBUFFER) is
//! GL_INVALID_ENUM there (src/gl/framebuffer.rs).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online,test-knobs --test es3_rgb10a2_mrt_fallback
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online,test-knobs --test es3_rgb10a2_mrt_fallback
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table,report-es2 --test es3_rgb10a2_mrt_fallback
//! ```
//!
//! Compiled only where the class falls back: the V3D build without
//! `report-es2` stores the class and runs zero tests of this file (its answers
//! are tests/es3_rgb10a2_mrt_v3d.rs).

#![cfg(all(feature = "host-test", any(feature = "gpu-powervr", feature = "gpu-vivante", feature = "report-es2")))]
// the ES 2.0 report runs the refusal pins only; the helpers of the rest are idle there
#![cfg_attr(feature = "report-es2", allow(dead_code, unused_imports))]

use core::ffi::{c_char, c_void};
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{self, GlesDraw, GlesHeap, YakoglBackend};
use yakogl::gl::consts::*;
use yakogl::gl::es3::*;
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
    name: c"es3-rgb10a2-mrt-fallback".as_ptr() as *const c_char,
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
    // SAFETY: plain GL calls.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glFinish();
    }
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

/// An RGB10_A2 texture of `w` x `h`, uploaded through the pair the fallback
/// takes for it (GL_RGBA / GL_UNSIGNED_INT_2_10_10_10_REV).
fn rgb10a2_texture(w: GLsizei, h: GLsizei, words: Option<&[u32]>) -> GLuint {
    let p = words.map_or(core::ptr::null(), |w| w.as_ptr() as *const c_void);
    texture(GL_RGB10_A2, GL_RGBA, GL_UNSIGNED_INT_2_10_10_10_REV, w, h, p)
}

/// A GL_RGBA8 texture of `w` x `h`, allocated without pixels.
fn rgba8_texture(w: GLsizei, h: GLsizei) -> GLuint {
    texture(GL_RGBA8, GL_RGBA, GL_UNSIGNED_BYTE, w, h, core::ptr::null())
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

/// A framebuffer, bound to GL_FRAMEBUFFER, with renderbuffers `rb0` and `rb1`
/// on colour attachments 0 and 1.
fn rbo_mrt(rb0: GLuint, rb1: GLuint) -> GLuint {
    let mut fb = 0;
    // SAFETY: room for one name; plain GL calls on it.
    unsafe {
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_RENDERBUFFER, rb0);
        glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT1, GL_RENDERBUFFER, rb1);
    }
    fb
}

/// A framebuffer, bound to GL_FRAMEBUFFER, with level 0 of `tex0` and `tex1`
/// on colour attachments 0 and 1.
fn tex_mrt(tex0: GLuint, tex1: GLuint) -> GLuint {
    let mut fb = 0;
    // SAFETY: room for one name; plain GL calls on it.
    unsafe {
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex0, 0);
        glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT1, GL_TEXTURE_2D, tex1, 0);
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

/// What every pixel of `px` must be, `want` a single pixel.
fn assert_uniform(px: &[u8], want: &[u8; 4]) {
    for (i, p) in px.as_chunks::<4>().0.iter().enumerate() {
        assert_eq!(*p, *want, "pixel {i}");
    }
}

/// The clear of attachment 0 and the RGBA8 bytes it is stored as:
/// 0.2 * 255 = 51, 0.6 * 255 = 153, 1.5 clamps to 255, alpha 0.4 is 102. As a
/// 10/10/10/2 word decoded per channel the same clear would read 51, 153, 255,
/// 85 (alpha 0.4 is 1 of 3): the alpha tells the two apart.
const CLEAR0: [f32; 4] = [0.2, 0.6, 1.5, 0.4];
const CLEAR0_BYTES: [u8; 4] = [51, 153, 255, 102];

/// The clear of attachment 1 and its bytes: 0.25 * 255 = 63.75 -> 64,
/// 0.75 * 255 = 191.25 -> 191, 0.5 * 255 = 127.5 -> 128, alpha 0.4 again 102
/// (the 2-bit alpha of the class build would decode 85).
const CLEAR1: [f32; 4] = [0.25, 0.75, 0.5, 0.4];
const CLEAR1_BYTES: [u8; 4] = [64, 191, 128, 102];

/// glDrawBuffers({GL_COLOR_ATTACHMENT0, GL_COLOR_ATTACHMENT1}) on the bound
/// framebuffer: both draw buffers written.
fn draw_both() {
    // SAFETY: plain GL call on the bound framebuffer.
    unsafe { glDrawBuffers(2, [GL_COLOR_ATTACHMENT0, GL_COLOR_ATTACHMENT1].as_ptr()) };
    assert_eq!(err(), GL_NO_ERROR, "glDrawBuffers(ATT0, ATT1)");
}

/// glClear with CLEAR0: every draw buffer the glDrawBuffers mapping writes.
fn clear_all() {
    // SAFETY: plain GL calls.
    unsafe {
        glClearColor(CLEAR0[0], CLEAR0[1], CLEAR0[2], CLEAR0[3]);
        glClear(GL_COLOR_BUFFER_BIT);
    }
    assert_eq!(err(), GL_NO_ERROR, "glClear(GL_COLOR_BUFFER_BIT)");
}

/// glClearBufferfv(GL_COLOR, `n`) with `value`: that draw buffer alone.
fn clear_one(n: GLint, value: &[f32; 4]) {
    // SAFETY: four floats of room; plain GL call.
    unsafe { glClearBufferfv(GL_COLOR, n, value.as_ptr()) };
    assert_eq!(err(), GL_NO_ERROR, "glClearBufferfv(GL_COLOR, {n})");
}

/// glReadBuffer(`att`) on the bound framebuffer, then its w x h pixels as
/// GL_RGBA / GL_UNSIGNED_BYTE.
fn read_attachment(att: GLenum, w: GLsizei, h: GLsizei) -> Vec<u8> {
    // SAFETY: plain GL call on the bound framebuffer.
    unsafe { glReadBuffer(att) };
    assert_eq!(err(), GL_NO_ERROR, "glReadBuffer({att:#x})");
    let px = read(w, h, GL_UNSIGNED_BYTE);
    assert_eq!(err(), GL_NO_ERROR, "glReadPixels(GL_RGBA, GL_UNSIGNED_BYTE) of {att:#x}");
    px
}

/// GL_IMPLEMENTATION_COLOR_READ_FORMAT / _TYPE of `att` as the read buffer.
fn read_pair(att: GLenum) -> (GLenum, GLenum) {
    // SAFETY: plain GL call on the bound framebuffer.
    unsafe { glReadBuffer(att) };
    assert_eq!(err(), GL_NO_ERROR, "glReadBuffer({att:#x})");
    let pair = implementation_pair();
    assert_eq!(err(), GL_NO_ERROR, "the read pair of {att:#x}");
    pair
}

/// glReadPixels GL_RGBA / GL_UNSIGNED_INT_2_10_10_10_REV of `att` on the bound
/// framebuffer: the fallback answer, `err`, nothing written.
fn read_packed_refused(att: GLenum, w: GLsizei, h: GLsizei, want: GLenum) {
    // SAFETY: plain GL call on the bound framebuffer.
    unsafe { glReadBuffer(att) };
    assert_eq!(err(), GL_NO_ERROR, "glReadBuffer({att:#x})");
    let words = read(w, h, GL_UNSIGNED_INT_2_10_10_10_REV);
    assert_eq!(err(), want, "glReadPixels(GL_RGBA, GL_UNSIGNED_INT_2_10_10_10_REV) of {att:#x}");
    assert!(words.iter().all(|&b| b == 0x77), "nothing written at {att:#x}");
}

/// Blit framebuffer `src`'s read buffer `att` NEAREST into framebuffer `dst`
/// (a plain RGBA8 FBO) and read `dst`'s w x h pixels back.
fn blit_into(src: GLuint, dst: GLuint, att: GLenum, w: GLsizei, h: GLsizei) -> Vec<u8> {
    // SAFETY: plain GL calls on framebuffers this test made.
    unsafe {
        glBindFramebuffer(GL_READ_FRAMEBUFFER, src);
        glBindFramebuffer(GL_DRAW_FRAMEBUFFER, dst);
        glReadBuffer(att);
        assert_eq!(err(), GL_NO_ERROR, "glReadBuffer({att:#x}) before glBlitFramebuffer");
        glBlitFramebuffer(0, 0, w, h, 0, 0, w, h, GL_COLOR_BUFFER_BIT, GL_NEAREST);
        assert_eq!(err(), GL_NO_ERROR, "glBlitFramebuffer(NEAREST) from {att:#x}");
        glBindFramebuffer(GL_FRAMEBUFFER, dst);
    }
    let px = read(w, h, GL_UNSIGNED_BYTE);
    assert_eq!(err(), GL_NO_ERROR, "glReadPixels of the destination");
    px
}

/// The rows every fallback test wants of its framebuffer's two points, in
/// order: the read pair of each, and the 10/10/10/2 read refused at each.
fn pin_both_read_rows(w: GLsizei, h: GLsizei) {
    for att in [GL_COLOR_ATTACHMENT0, GL_COLOR_ATTACHMENT1] {
        assert_eq!(read_pair(att), (GL_RGBA, GL_UNSIGNED_BYTE), "the read pair of {att:#x}");
        read_packed_refused(att, w, h, GL_INVALID_OPERATION);
    }
}

/// The draws the stand-in back end was handed: every target an RGBA8888 one
/// (`GL_RGB10_A2` is no surface on this build), exactly `stores` of them a
/// clear was stored into.
fn pin_draws(stores: usize) {
    let draws = DRAWS.lock().unwrap().clone();
    assert_eq!(draws.iter().filter(|d| d.1).count(), stores, "the stored clears: {draws:?}");
    assert!(draws.iter().all(|d| d.0 == backend::GLES_SURF_RGBA8888), "only RGBA8888 targets: {draws:?}");
    assert!(draws.iter().all(|d| d.0 != backend::GLES_SURF_RGB10_A2), "no 10/10/10/2 surface: {draws:?}");
}

/* ---- the ES 2.0 report: GL_RGB10_A2 is no format ---------------------------- */

#[cfg(feature = "report-es2")]
mod es2 {
    use super::*;

    /// ES 2.0 4.4.3 names no GL_RGB10_A2 renderbuffer storage and 3.7.1 no
    /// such texture internal format: neither is allocated, so no
    /// attachment of the format reaches an MRT framebuffer here (as
    /// tests/es3_rgb10a2_fallback.rs and tests/es3_rgb10a2_texclear_fallback.rs
    /// hold).
    #[test]
    fn mrt_fallback_es2_rgb10a2_is_not_allocated() {
        let _g = context();
        let rb = renderbuffer(GL_RGB10_A2, 4, 2);
        assert_eq!(err(), GL_INVALID_ENUM, "glRenderbufferStorage(GL_RGB10_A2)");
        let tex = texture(GL_RGB10_A2, GL_RGBA, GL_UNSIGNED_BYTE, 4, 2, core::ptr::null());
        assert_eq!(err(), GL_INVALID_VALUE, "glTexImage2D(GL_RGB10_A2, GL_RGBA, GL_UNSIGNED_BYTE)");
        let tex2 = texture(GL_RGB10_A2, GL_RGBA, GL_UNSIGNED_INT_2_10_10_10_REV, 4, 2, core::ptr::null());
        assert_eq!(err(), GL_INVALID_ENUM, "glTexImage2D(GL_RGB10_A2, .., GL_UNSIGNED_INT_2_10_10_10_REV)");
        release(&[], &[rb], &[tex, tex2]);
    }

    /// Two GL_RGBA8 renderbuffers, the stand-in for the pair this build cannot
    /// make of GL_RGB10_A2: the multiple-draw-buffer entry points are not
    /// refused — glDrawBuffers, glClearBufferfv and glReadBuffer work as on an
    /// ES 3.0 build (src/gl/framebuffer.rs gates none of them on the report),
    /// and each point reads its own clear's bytes. The 10/10/10/2 read type is
    /// GL_INVALID_OPERATION at each, as tests/es3_rgb10a2_fallback.rs pins on
    /// the RGBA8 buffers an ES 2.0 context has.
    #[test]
    fn mrt_fallback_es2_draw_buffers_work_on_rgba8() {
        let _g = context();
        let rb0 = renderbuffer(GL_RGBA8, 4, 2);
        assert_eq!(err(), GL_NO_ERROR);
        let rb1 = renderbuffer(GL_RGBA8, 4, 2);
        assert_eq!(err(), GL_NO_ERROR);
        let fb = rbo_mrt(rb0, rb1);
        assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE);
        draw_both();
        clear_all();
        clear_one(1, &CLEAR1);
        assert_uniform(&read_attachment(GL_COLOR_ATTACHMENT0, 4, 2), &CLEAR0_BYTES);
        assert_uniform(&read_attachment(GL_COLOR_ATTACHMENT1, 4, 2), &CLEAR1_BYTES);
        pin_draws(3);
        pin_both_read_rows(4, 2);
        release(&[fb], &[rb0, rb1], &[]);
    }

    /// glBlitFramebuffer is not an ES 2.0 command: GL_INVALID_OPERATION from
    /// either read buffer, said once per call (src/gl/framebuffer.rs, the
    /// ES 3.0-only line the framebuffer unit test pins). And a read/draw
    /// framebuffer split — what a blit between two framebuffers needs — is
    /// refused before it: ES 2.0 4.4.1 names only GL_FRAMEBUFFER, and
    /// glBindFramebuffer(GL_READ_FRAMEBUFFER / GL_DRAW_FRAMEBUFFER) answers
    /// GL_INVALID_ENUM on this build (`is_public_fb_target`).
    #[test]
    fn mrt_fallback_es2_blit_and_split_targets_are_refused() {
        let _g = context();
        let rb0 = renderbuffer(GL_RGBA8, 4, 2);
        assert_eq!(err(), GL_NO_ERROR);
        let rb1 = renderbuffer(GL_RGBA8, 4, 2);
        assert_eq!(err(), GL_NO_ERROR);
        let src = rbo_mrt(rb0, rb1);
        assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE);
        draw_both();
        clear_all();
        assert_uniform(&read_attachment(GL_COLOR_ATTACHMENT0, 4, 2), &CLEAR0_BYTES);
        let dst_rb = renderbuffer(GL_RGBA8, 4, 2);
        assert_eq!(err(), GL_NO_ERROR);
        let dst = rb_framebuffer(dst_rb);
        // SAFETY: plain GL calls on framebuffers this test made.
        unsafe {
            glBindFramebuffer(GL_READ_FRAMEBUFFER, src);
        }
        assert_eq!(err(), GL_INVALID_ENUM, "glBindFramebuffer(GL_READ_FRAMEBUFFER)");
        // SAFETY: plain GL calls.
        unsafe {
            glBindFramebuffer(GL_DRAW_FRAMEBUFFER, dst);
        }
        assert_eq!(err(), GL_INVALID_ENUM, "glBindFramebuffer(GL_DRAW_FRAMEBUFFER)");
        // SAFETY: plain GL call.
        unsafe { glBindFramebuffer(GL_FRAMEBUFFER, src) };
        for att in [GL_COLOR_ATTACHMENT0, GL_COLOR_ATTACHMENT1] {
            // SAFETY: plain GL calls.
            unsafe {
                glReadBuffer(att);
                glBlitFramebuffer(0, 0, 4, 2, 0, 0, 4, 2, GL_COLOR_BUFFER_BIT, GL_NEAREST);
            }
            assert_eq!(err(), GL_INVALID_OPERATION, "glBlitFramebuffer(NEAREST) from {att:#x}");
            assert_eq!(err(), GL_NO_ERROR, "one error per glBlitFramebuffer call");
        }
        // SAFETY: plain GL call.
        unsafe { glBindFramebuffer(GL_FRAMEBUFFER, dst) };
        assert!(read(4, 2, GL_UNSIGNED_BYTE).iter().all(|&b| b == 0), "the destination was never written");
        release(&[src, dst], &[rb0, rb1, dst_rb], &[]);
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
    fn mrt_fallback_build_has_no_class() {
        assert!(!backend::rt_format_drawable(backend::GLES_SURF_RGB10_A2));
    }

    /// A GL_RGB10_A2 texture (attachment 0) beside a GL_RGBA8 one (attachment
    /// 1): complete, drawn, and indistinguishable from the RGBA8 one — its own
    /// clear per glClearBufferfv, the RGBA8 bytes of that clear through its
    /// own glReadBuffer, the RGBA8 read pair, no 10/10/10/2 read, and a blit
    /// out as plain RGBA8.
    #[test]
    fn mrt_fallback_textures_clear_and_read_per_buffer() {
        let _g = context();
        let tex0 = rgb10a2_texture(4, 2, None);
        assert_eq!(err(), GL_NO_ERROR, "glTexImage2D(GL_RGB10_A2)");
        let tex1 = rgba8_texture(4, 2);
        assert_eq!(err(), GL_NO_ERROR, "glTexImage2D(GL_RGBA8)");
        let fb = tex_mrt(tex0, tex1);
        assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE, "GL_RGB10_A2 beside RGBA8");
        draw_both();
        clear_one(0, &CLEAR0);
        clear_one(1, &CLEAR1);
        assert_uniform(&read_attachment(GL_COLOR_ATTACHMENT0, 4, 2), &CLEAR0_BYTES);
        assert_uniform(&read_attachment(GL_COLOR_ATTACHMENT1, 4, 2), &CLEAR1_BYTES);
        pin_draws(2);
        pin_both_read_rows(4, 2);
        let dst = rgba8_texture(4, 2);
        assert_eq!(err(), GL_NO_ERROR);
        let d_fb = tex_framebuffer(dst);
        assert_uniform(&blit_into(fb, d_fb, GL_COLOR_ATTACHMENT0, 4, 2), &CLEAR0_BYTES);
        assert_uniform(&blit_into(fb, d_fb, GL_COLOR_ATTACHMENT1, 4, 2), &CLEAR1_BYTES);
        release(&[fb, d_fb], &[], &[tex0, tex1, dst]);
    }

    /// The same as a renderbuffer pair: glRenderbufferStorage(GL_RGB10_A2) is
    /// RGBA8888 storage, attached beside a GL_RGBA8 renderbuffer, cleared per
    /// draw buffer, read per read buffer, blitted out into a plain RGBA8
    /// renderbuffer FBO.
    #[test]
    fn mrt_fallback_renderbuffers_clear_and_read_per_buffer() {
        let _g = context();
        let rb0 = renderbuffer(GL_RGB10_A2, 4, 2);
        assert_eq!(err(), GL_NO_ERROR, "glRenderbufferStorage(GL_RGB10_A2)");
        let rb1 = renderbuffer(GL_RGBA8, 4, 2);
        assert_eq!(err(), GL_NO_ERROR, "glRenderbufferStorage(GL_RGBA8)");
        let fb = rbo_mrt(rb0, rb1);
        assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE, "GL_RGB10_A2 beside RGBA8");
        draw_both();
        clear_one(0, &CLEAR0);
        clear_one(1, &CLEAR1);
        assert_uniform(&read_attachment(GL_COLOR_ATTACHMENT0, 4, 2), &CLEAR0_BYTES);
        assert_uniform(&read_attachment(GL_COLOR_ATTACHMENT1, 4, 2), &CLEAR1_BYTES);
        pin_draws(2);
        pin_both_read_rows(4, 2);
        let dst_rb = renderbuffer(GL_RGBA8, 4, 2);
        assert_eq!(err(), GL_NO_ERROR);
        let d_fb = rb_framebuffer(dst_rb);
        assert_uniform(&blit_into(fb, d_fb, GL_COLOR_ATTACHMENT0, 4, 2), &CLEAR0_BYTES);
        assert_uniform(&blit_into(fb, d_fb, GL_COLOR_ATTACHMENT1, 4, 2), &CLEAR1_BYTES);
        release(&[fb, d_fb], &[rb0, rb1, dst_rb], &[]);
    }

    /// The attachments swapped (GL_RGBA8 at attachment 0, GL_RGB10_A2 at 1):
    /// a glClear clears every draw buffer and both attachments hold its bytes
    /// alike; glClearBufferfv(GL_COLOR, 1) then changes attachment 1 alone —
    /// attachment 0 still holds the glClear's. The read rows are the same at
    /// each point, whichever attachment the format sits on.
    #[test]
    fn mrt_fallback_swapped_textures() {
        let _g = context();
        let tex0 = rgba8_texture(4, 2);
        assert_eq!(err(), GL_NO_ERROR);
        let tex1 = rgb10a2_texture(4, 2, None);
        assert_eq!(err(), GL_NO_ERROR);
        let fb = tex_mrt(tex0, tex1);
        assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE);
        draw_both();
        clear_all();
        clear_one(1, &CLEAR1);
        assert_uniform(&read_attachment(GL_COLOR_ATTACHMENT0, 4, 2), &CLEAR0_BYTES);
        assert_uniform(&read_attachment(GL_COLOR_ATTACHMENT1, 4, 2), &CLEAR1_BYTES);
        pin_draws(3);
        pin_both_read_rows(4, 2);
        let dst = rgba8_texture(4, 2);
        assert_eq!(err(), GL_NO_ERROR);
        let d_fb = tex_framebuffer(dst);
        assert_uniform(&blit_into(fb, d_fb, GL_COLOR_ATTACHMENT1, 4, 2), &CLEAR1_BYTES);
        assert_uniform(&blit_into(fb, d_fb, GL_COLOR_ATTACHMENT0, 4, 2), &CLEAR0_BYTES);
        release(&[fb, d_fb], &[], &[tex0, tex1, dst]);
    }

    /// Swapped as renderbuffers too: GL_RGBA8 at attachment 0, GL_RGB10_A2 at
    /// attachment 1, a glClear writing both and glClearBufferfv one, the read
    /// rows the same at both points, and the blit out as plain RGBA8.
    #[test]
    fn mrt_fallback_swapped_renderbuffers() {
        let _g = context();
        let rb0 = renderbuffer(GL_RGBA8, 4, 2);
        assert_eq!(err(), GL_NO_ERROR);
        let rb1 = renderbuffer(GL_RGB10_A2, 4, 2);
        assert_eq!(err(), GL_NO_ERROR);
        let fb = rbo_mrt(rb0, rb1);
        assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE);
        draw_both();
        clear_one(0, &CLEAR0);
        clear_one(1, &CLEAR1);
        assert_uniform(&read_attachment(GL_COLOR_ATTACHMENT0, 4, 2), &CLEAR0_BYTES);
        assert_uniform(&read_attachment(GL_COLOR_ATTACHMENT1, 4, 2), &CLEAR1_BYTES);
        pin_draws(2);
        pin_both_read_rows(4, 2);
        let dst_rb = renderbuffer(GL_RGBA8, 4, 2);
        assert_eq!(err(), GL_NO_ERROR);
        let d_fb = rb_framebuffer(dst_rb);
        assert_uniform(&blit_into(fb, d_fb, GL_COLOR_ATTACHMENT1, 4, 2), &CLEAR1_BYTES);
        assert_uniform(&blit_into(fb, d_fb, GL_COLOR_ATTACHMENT0, 4, 2), &CLEAR0_BYTES);
        release(&[fb, d_fb], &[rb0, rb1, dst_rb], &[]);
    }
}
