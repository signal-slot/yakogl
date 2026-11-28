// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! glInvalidateFramebuffer and glInvalidateSubFramebuffer (ES 3.0 4.5.1) of
//! GL_COLOR_ATTACHMENT3 over the PowerVR host back end, under the four-target
//! stand-in `backend::HOST_DRAW_TARGETS` = 4 (what `DRAW_TARGETS` = 4 would
//! answer; plan #368 RC14; `pvr_debug_set_draw_targets` needs the
//! `test-knobs` feature, so the stand-in is used). The PowerVR twin of
//! tests/es3_mrt4_invalidate_v3d.rs, on the rig of
//! tests/es3_mrt4_copytex_pvr.rs: the real GL core and a stand-in back end
//! that records every `GlesDraw` it is handed. What that record exposes for
//! RT 3 is `targets[3]`: its gpu address (0 = not stored, a `GL_NONE` draw
//! buffer) and its surface; `GlesDraw` has no load field, so a load is not
//! seen (the `#[ignore]`d pins say so). A clear fills every target it names,
//! so what is stored is read back by glReadPixels. HOST-ONLY.
//!
//! Pinned, for GL_COLOR_ATTACHMENT3 alone, then all four attachments, then
//! a sub-rectangle:
//! * no GL error, on the draw, read and combined targets;
//! * GL_COLOR_ATTACHMENT4 (m = GL_MAX_COLOR_ATTACHMENTS = 4) is
//!   GL_INVALID_OPERATION, as is GL_COLOR_ATTACHMENT3 at three targets, and
//!   a bad framebuffer target or attachment name is GL_INVALID_ENUM;
//! * the invalidation hands the back end nothing and the attachments keep
//!   their contents (it is a validated no-op, src/gl/framebuffer.rs
//!   `invalidate_framebuffer`);
//! * the draw after it carries the same four targets, RT 3 stored as without
//!   the call, and the untouched attachments keep their pixels until that
//!   draw writes them.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test es3_mrt4_invalidate_pvr
//! ```

#![cfg(all(feature = "host-test", feature = "gpu-powervr", feature = "shaderc-online", not(feature = "report-es2")))]

use core::ffi::{c_char, c_void};
use core::sync::atomic::Ordering;
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{self, GlesDraw, GlesHeap, YakoglBackend};
use yakogl::gl::consts::*;
use yakogl::gl::framebuffer::*;
use yakogl::gl::state::*;
use yakogl::gl::texture::*;
use yakogl::gl::types::*;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};

/* ---- the platform: the smallest host OSAL (as tests/es3_readback4_pvr.rs) - */

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

/// One draw the back end was handed. Per colour target, as one PBE emit of a
/// PowerVR render stores it: (gpu address, surface, w, h) — address 0 is a
/// `GL_NONE` draw buffer, configured but never stored.
#[derive(Clone, Debug, PartialEq, Eq)]
struct SeenDraw {
    num_targets: usize,
    targets: Vec<(u64, u8, u32, u32)>,
    do_clear: bool,
    /// glClearBuffer(GL_COLOR, n): bit n; 0 is glClear's "every target".
    clear_target_mask: u8,
}

static SEEN: Mutex<Vec<SeenDraw>> = Mutex::new(Vec::new());
/// The ids the GL core ever reported through `attachment_gone`
/// (src/ds_retention.rs); an invalidation must report nothing.
static GONE: Mutex<Vec<u32>> = Mutex::new(Vec::new());

/// Records every draw; a clear fills every target it names with the clear
/// colour in the target's layout (unorm bytes for a colour surface, the raw
/// 32-bit words for an integer one), rows at the pair-padded pitch — so the
/// store of target 3 is visible to glReadPixels.
unsafe extern "C" fn b_draw(d: *const GlesDraw) -> i32 {
    let d = &*d;
    let n = d.num_targets.max(1) as usize;
    SEEN.lock().unwrap().push(SeenDraw {
        num_targets: n,
        targets: d.targets[..n].iter().map(|t| (t.gpu, t.format, t.w, t.h)).collect(),
        do_clear: d.do_clear,
        clear_target_mask: d.clear_target_mask,
    });
    if !d.do_clear {
        return 0;
    }
    for t in &d.targets[..n] {
        if t.gpu == 0 {
            continue;
        }
        let bpp = backend::gles_surf_bytes_per_pixel(t.format) as usize;
        let mut px = vec![0u8; bpp];
        if let Some(l) = backend::gles_surf_int_layout(t.format) {
            let step = l.bits as usize / 8;
            for (ch, v) in d.clear_rgba.iter().enumerate().take(l.channels as usize) {
                px[ch * step..ch * step + step].copy_from_slice(&v.to_bits().to_le_bytes()[..step]);
            }
        } else {
            for (ch, v) in d.clear_rgba.iter().enumerate().take(bpp) {
                px[ch] = (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
            }
        }
        let pitch = ((t.w + 1) & !1) as usize;
        let base = t.gpu as *mut u8;
        for y in 0..t.h as usize {
            for x in 0..t.w as usize {
                core::ptr::copy_nonoverlapping(px.as_ptr(), base.add((y * pitch + x) * bpp), bpp);
            }
        }
    }
    0
}
unsafe extern "C" fn b_gone(id: u32) {
    GONE.lock().unwrap().push(id);
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
    name: c"es3-mrt4-invalidate-pvr".as_ptr() as *const c_char,
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
    attachment_gone: Some(b_gone),
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

/* ---- the context: one for the whole file, the tests take turns ------------- */

static LOCK: Mutex<()> = Mutex::new(());
static ONCE: Once = Once::new();

fn context() -> MutexGuard<'static, ()> {
    let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    backend::HOST_DRAW_TARGETS.store(4, Ordering::Relaxed);
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
    SEEN.lock().unwrap().clear();
    GONE.lock().unwrap().clear();
    // SAFETY: plain GL call.
    while err() != GL_NO_ERROR {}
    g
}

/// Three draw targets, what the build answered before `DRAW_TARGETS` was 4
/// on the PowerVR (and still answers under `report-es2`): the stand-in at 3.
fn three_targets() {
    backend::HOST_DRAW_TARGETS.store(3, Ordering::Relaxed);
}

fn err() -> GLenum {
    // SAFETY: plain GL call.
    unsafe { glGetError() }
}

fn get(pname: GLenum) -> GLint {
    let mut v = -1;
    // SAFETY: room for one value.
    unsafe { glGetIntegerv(pname, &mut v) };
    v
}

/* ---- the framebuffer: four W x W RGBA8 textures, each with its own pixels - */

const W: GLsizei = 4;

const ALL4: [GLenum; 4] = [GL_COLOR_ATTACHMENT0, GL_COLOR_ATTACHMENT1, GL_COLOR_ATTACHMENT2, GL_COLOR_ATTACHMENT3];

fn att(k: usize) -> GLenum {
    GL_COLOR_ATTACHMENT0 + k as GLenum
}

/// The bytes attachment `k` holds at pixel (x, y).
fn val(k: usize, x: usize, y: usize) -> [u8; 4] {
    [(k * 40 + x * 8 + 1) as u8, (y * 16 + k) as u8, (200 - k * 20) as u8, (x + y * 4 + k * 3 + 10) as u8]
}

struct Fbo {
    fb: GLuint,
    texs: Vec<GLuint>,
}

impl Drop for Fbo {
    fn drop(&mut self) {
        // SAFETY: names this test made.
        unsafe {
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
            glDeleteFramebuffers(1, &self.fb);
            glDeleteTextures(self.texs.len() as GLsizei, self.texs.as_ptr());
        }
    }
}

/// A bound framebuffer of `n` RGBA8 textures, attachment k filled with
/// `val(k, ..)`; with `n` = 4 all four draw buffers are on.
fn fbo(n: usize) -> Fbo {
    let (mut fb, mut texs) = (0, vec![0; n]);
    let w = W as usize;
    // SAFETY: room for the names; the pixel array holds W x W RGBA bytes.
    unsafe {
        glGenTextures(n as GLsizei, texs.as_mut_ptr());
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        for (k, &t) in texs.iter().enumerate() {
            let px: Vec<u8> = (0..w * w).flat_map(|i| val(k, i % w, i / w)).collect();
            glBindTexture(GL_TEXTURE_2D, t);
            let p = px.as_ptr() as *const c_void;
            glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA8 as GLint, W, W, 0, GL_RGBA, GL_UNSIGNED_BYTE, p);
            glFramebufferTexture2D(GL_FRAMEBUFFER, att(k), GL_TEXTURE_2D, t, 0);
        }
        if n == 4 {
            glDrawBuffers(4, ALL4.as_ptr());
        }
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE, "x{n}");
    }
    assert_eq!(err(), GL_NO_ERROR, "fbo");
    Fbo { fb, texs }
}

/// Attachment `k` of `f` as RGBA bytes.
fn read(f: &Fbo, k: usize) -> Vec<[u8; 4]> {
    let mut px = vec![0x77u8; (W * W * 4) as usize];
    // SAFETY: room for W x W RGBA pixels.
    unsafe {
        glBindFramebuffer(GL_READ_FRAMEBUFFER, f.fb);
        glReadBuffer(att(k));
        glReadPixels(0, 0, W, W, GL_RGBA, GL_UNSIGNED_BYTE, px.as_mut_ptr() as *mut c_void);
    }
    assert_eq!(err(), GL_NO_ERROR, "read {k}");
    px.chunks(4).map(|c| [c[0], c[1], c[2], c[3]]).collect()
}

fn expect(k: usize) -> Vec<[u8; 4]> {
    (0..(W * W) as usize).map(|i| val(k, i % W as usize, i / W as usize)).collect()
}

fn inv(target: GLenum, atts: &[GLenum]) -> GLenum {
    // SAFETY: the slice is live for the call.
    unsafe { glInvalidateFramebuffer(target, atts.len() as GLsizei, atts.as_ptr()) };
    err()
}

fn inv_sub(target: GLenum, atts: &[GLenum], x: i32, y: i32, w: i32, h: i32) -> GLenum {
    // SAFETY: the slice is live for the call.
    unsafe { glInvalidateSubFramebuffer(target, atts.len() as GLsizei, atts.as_ptr(), x, y, w, h) };
    err()
}

fn mark() -> usize {
    SEEN.lock().unwrap().len()
}

fn since(m: usize) -> Vec<SeenDraw> {
    SEEN.lock().unwrap()[m..].to_vec()
}

fn clear_and_finish() {
    // SAFETY: plain GL calls.
    unsafe {
        glClearColor(0.0, 0.0, 1.0, 1.0);
        glClear(GL_COLOR_BUFFER_BIT);
        glFinish();
    }
    assert_eq!(err(), GL_NO_ERROR, "clear");
}

/// The cases: (name, invalidation) for attachment 3 alone, all four, and a
/// sub-rectangle.
#[derive(Clone, Copy, Debug)]
enum Case {
    Alone,
    All,
    Sub,
}

const CASES: [Case; 3] = [Case::Alone, Case::All, Case::Sub];

fn invalidate(c: Case, target: GLenum) -> GLenum {
    match c {
        Case::Alone => inv(target, &[GL_COLOR_ATTACHMENT3]),
        Case::All => inv(target, &ALL4),
        Case::Sub => inv_sub(target, &[GL_COLOR_ATTACHMENT3], 1, 1, 2, 2),
    }
}

/* ---- the tests --------------------------------------------------------------- */

/// GL_COLOR_ATTACHMENT3 alone, all four and a sub-rectangle, with the
/// depth/stencil points beside, on every framebuffer target: no error.
#[test]
fn colour_attachment_3_is_accepted() {
    let _g = context();
    let _f = fbo(4);
    assert_eq!((get(GL_MAX_DRAW_BUFFERS), get(GL_MAX_COLOR_ATTACHMENTS)), (4, 4));
    for t in [GL_FRAMEBUFFER, GL_DRAW_FRAMEBUFFER, GL_READ_FRAMEBUFFER] {
        for c in CASES {
            assert_eq!(invalidate(c, t), GL_NO_ERROR, "{c:?} 0x{t:x}");
        }
        let mixed = [GL_COLOR_ATTACHMENT3, GL_DEPTH_ATTACHMENT, GL_STENCIL_ATTACHMENT, GL_DEPTH_STENCIL_ATTACHMENT];
        assert_eq!(inv(t, &mixed), GL_NO_ERROR, "0x{t:x} with depth/stencil");
        assert_eq!(inv_sub(t, &mixed, 2, 3, 5, 7), GL_NO_ERROR, "sub 0x{t:x} with depth/stencil");
        /* past the surface, empty and negative origins: legal */
        assert_eq!(inv_sub(t, &[GL_COLOR_ATTACHMENT3], -4, -4, 100, 100), GL_NO_ERROR);
        assert_eq!(inv_sub(t, &[GL_COLOR_ATTACHMENT3], 0, 0, 0, 0), GL_NO_ERROR);
    }
    /* a point with no image is still a point */
    let _f3 = fbo(3);
    assert_eq!(inv(GL_FRAMEBUFFER, &[GL_COLOR_ATTACHMENT3]), GL_NO_ERROR, "point 3 with no image");
}

/// Attachment 4 (= GL_MAX_COLOR_ATTACHMENTS) is GL_INVALID_OPERATION, alone
/// or after legal names; at three targets point 3 is; a bad target or name
/// is GL_INVALID_ENUM; a negative count or size is GL_INVALID_VALUE.
#[test]
fn errors() {
    let _g = context();
    let _f = fbo(4);
    for t in [GL_FRAMEBUFFER, GL_DRAW_FRAMEBUFFER, GL_READ_FRAMEBUFFER] {
        assert_eq!(inv(t, &[GL_COLOR_ATTACHMENT4]), GL_INVALID_OPERATION, "0x{t:x}");
        assert_eq!(inv_sub(t, &[GL_COLOR_ATTACHMENT4], 0, 0, 4, 4), GL_INVALID_OPERATION, "sub 0x{t:x}");
        assert_eq!(inv(t, &[GL_COLOR_ATTACHMENT0, GL_COLOR_ATTACHMENT3, GL_COLOR_ATTACHMENT4]), GL_INVALID_OPERATION);
        assert_eq!(inv(t, &[GL_COLOR_ATTACHMENT15]), GL_INVALID_OPERATION, "0x{t:x} attachment 15");
    }
    assert_eq!(inv(GL_TEXTURE_2D, &[GL_COLOR_ATTACHMENT3]), GL_INVALID_ENUM, "not a framebuffer target");
    assert_eq!(inv(GL_FRAMEBUFFER, &[GL_BACK]), GL_INVALID_ENUM, "GL_BACK on a framebuffer object");
    assert_eq!(inv_sub(GL_TEXTURE_2D, &[GL_COLOR_ATTACHMENT3], 0, 0, 4, 4), GL_INVALID_ENUM);
    // SAFETY: a live array; the count is the point.
    unsafe { glInvalidateFramebuffer(GL_FRAMEBUFFER, -1, ALL4.as_ptr()) };
    assert_eq!(err(), GL_INVALID_VALUE, "negative count");
    assert_eq!(inv_sub(GL_FRAMEBUFFER, &[GL_COLOR_ATTACHMENT3], 0, 0, -1, 4), GL_INVALID_VALUE, "negative width");
    three_targets();
    assert_eq!(get(GL_MAX_COLOR_ATTACHMENTS), 3);
    let r = (
        inv(GL_FRAMEBUFFER, &[GL_COLOR_ATTACHMENT3]),
        inv(GL_FRAMEBUFFER, &ALL4),
        inv_sub(GL_FRAMEBUFFER, &[GL_COLOR_ATTACHMENT3], 0, 0, 4, 4),
    );
    backend::HOST_DRAW_TARGETS.store(4, Ordering::Relaxed);
    assert_eq!(r, (GL_INVALID_OPERATION, GL_INVALID_OPERATION, GL_INVALID_OPERATION), "three targets: point 3");
}

/// The invalidation hands the back end no draw, reports nothing through
/// `attachment_gone`, and every attachment, 3 included, keeps its pixels (the
/// contents are undefined by the spec; today they are kept).
#[test]
fn the_attachments_keep_their_contents() {
    let _g = context();
    let f = fbo(4);
    let gone = GONE.lock().unwrap().len();
    for c in CASES {
        let m = mark();
        assert_eq!(invalidate(c, GL_FRAMEBUFFER), GL_NO_ERROR, "{c:?}");
        // SAFETY: plain GL call.
        unsafe { glFinish() };
        assert_eq!(since(m), Vec::new(), "{c:?}: the back end hears nothing");
        assert_eq!(GONE.lock().unwrap().len(), gone, "{c:?}: nothing gone");
        for k in 0..4 {
            assert_eq!(read(&f, k), expect(k), "{c:?}: attachment {k}");
        }
    }
}

/// An invalidation of attachment 3 alone leaves attachments 0..2 untouched
/// in particular; a sub-rectangle leaves the rest of 3 too.
#[test]
fn untouched_attachments_keep_their_pixels() {
    let _g = context();
    let f = fbo(4);
    assert_eq!(inv(GL_FRAMEBUFFER, &[GL_COLOR_ATTACHMENT3]), GL_NO_ERROR);
    for k in 0..3 {
        assert_eq!(read(&f, k), expect(k), "attachment {k} after invalidating 3");
    }
    assert_eq!(inv_sub(GL_FRAMEBUFFER, &[GL_COLOR_ATTACHMENT3], 1, 1, 2, 2), GL_NO_ERROR);
    assert_eq!(read(&f, 3), expect(3), "attachment 3 outside and inside the rectangle");
    /* the invalidation of 0..2 does not touch 3 */
    assert_eq!(inv(GL_FRAMEBUFFER, &ALL4[..3]), GL_NO_ERROR);
    assert_eq!(read(&f, 3), expect(3), "attachment 3 after invalidating 0..2");
}

/// The draw after the invalidation is the draw without it: four targets,
/// RT 3's address set (its PBE state and EOT emit are kept), and its clear
/// colour reaches memory; the back end never heard of the call.
#[test]
fn rt3_is_stored_as_without_the_call() {
    let _g = context();
    let f = fbo(4);
    let m = mark();
    clear_and_finish();
    let base = since(m);
    assert_eq!(base.len(), 1, "the clear went out as one draw");
    assert_eq!(base[0].num_targets, 4, "four draw buffers hand four targets: {:?}", base[0]);
    assert_ne!(base[0].targets[3].0, 0, "RT 3 is stored: {:?}", base[0]);
    for c in CASES {
        assert_eq!(invalidate(c, GL_FRAMEBUFFER), GL_NO_ERROR, "{c:?}");
        let m = mark();
        clear_and_finish();
        let after = since(m);
        assert_eq!(after.len(), 1, "{c:?}");
        /* the addresses move with the allocations only, not with the call */
        assert_eq!(after[0], base[0], "{c:?}: the same draw, RT 3 loaded and stored as before");
        for k in 0..4 {
            assert_eq!(read(&f, k), vec![[0, 0, 255, 255]; (W * W) as usize], "{c:?}: attachment {k} after the clear");
        }
        /* put the fill back for the next case */
        f_refill(&f);
    }
}

/// Refills the four textures with `val` (a texture upload between frames).
fn f_refill(f: &Fbo) {
    let w = W as usize;
    for (k, &t) in f.texs.iter().enumerate() {
        let px: Vec<u8> = (0..w * w).flat_map(|i| val(k, i % w, i / w)).collect();
        // SAFETY: a W x W RGBA8 texture and as many bytes.
        unsafe {
            glBindTexture(GL_TEXTURE_2D, t);
            glTexSubImage2D(GL_TEXTURE_2D, 0, 0, 0, W, W, GL_RGBA, GL_UNSIGNED_BYTE, px.as_ptr() as *const c_void);
        }
    }
    assert_eq!(err(), GL_NO_ERROR, "refill");
}

/// GL_NONE on draw buffer 3: RT 3 is not handed over, so never stored, with or without an invalidation of it.
#[test]
fn rt3_as_gl_none_is_not_stored() {
    let _g = context();
    let _f = fbo(4);
    let bufs = [GL_COLOR_ATTACHMENT0, GL_COLOR_ATTACHMENT1, GL_COLOR_ATTACHMENT2, GL_NONE];
    // SAFETY: a live array.
    unsafe { glDrawBuffers(4, bufs.as_ptr()) };
    assert_eq!(err(), GL_NO_ERROR);
    let m = mark();
    clear_and_finish();
    let base = since(m);
    assert_eq!(base.len(), 1);
    /* a trailing GL_NONE: the draw configures three targets, RT 3 is not in it */
    assert_eq!(base[0].num_targets, 3, "GL_NONE RT 3 is dropped: {:?}", base[0]);
    assert_eq!(inv(GL_FRAMEBUFFER, &[GL_COLOR_ATTACHMENT3]), GL_NO_ERROR);
    let m = mark();
    clear_and_finish();
    assert_eq!(since(m), base);
}

/* ES 3.0 4.5's answer, which the GL core cannot give today: the next render
 * pass may skip loading RT 3 but must still store it (Mesa's PowerVR driver
 * drops only the load), and an invalidation with no later draw may drop the
 * store. Both need the GL core to pass the invalidation on and a `GlesDraw`
 * field for it (plan #186, as tests/es3_mrt4_invalidate_v3d.rs names). */

#[test]
#[ignore = "defect: glInvalidateFramebuffer is a no-op in src/gl/framebuffer.rs and GlesDraw has no invalidate field, so the draw after an invalidation of GL_COLOR_ATTACHMENT3 still loads RT 3"]
fn a_draw_after_invalidating_rt3_skips_its_load_and_keeps_its_store() {
    let _g = context();
    let _f = fbo(4);
    let m = mark();
    clear_and_finish();
    let base = since(m);
    assert_eq!(inv(GL_FRAMEBUFFER, &[GL_COLOR_ATTACHMENT3]), GL_NO_ERROR);
    let m = mark();
    clear_and_finish();
    let after = since(m);
    assert_ne!(after[0].targets[3].0, 0, "RT 3 is still stored");
    assert_ne!(after[0], base[0], "the invalidation must reach the back end so RT 3 is not loaded");
}

#[test]
#[ignore = "defect: glInvalidateFramebuffer is a no-op in src/gl/framebuffer.rs and GlesDraw has no invalidate field, so an invalidation with no later draw reaches the back end as nothing"]
fn an_invalidation_with_no_later_draw_may_drop_the_store() {
    let _g = context();
    let _f = fbo(4);
    clear_and_finish();
    let m = mark();
    let gone = GONE.lock().unwrap().len();
    assert_eq!(inv(GL_FRAMEBUFFER, &[GL_COLOR_ATTACHMENT3]), GL_NO_ERROR);
    // SAFETY: plain GL call.
    unsafe { glFinish() };
    assert!(
        !since(m).is_empty() || GONE.lock().unwrap().len() != gone,
        "the invalidation must reach the back end so the frame's end can drop RT 3's store"
    );
}
