// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! glBlitFramebuffer and glInvalidateFramebuffer (ES 3.0 4.3.3 and 4.5.1) on
//! GL_COLOR_ATTACHMENT3 over the PowerVR host build, under the four-target
//! stand-in `backend::HOST_DRAW_TARGETS` = 4 (what `DRAW_TARGETS` = 4 would
//! answer; plan #368 RC14, docs/notes/mrt4-v3d-pvr.md). The V3D halves are
//! tests/es3_mrt4_blit_v3d.rs and tests/es3_mrt4_invalidate_v3d.rs; the rig
//! is tests/es3_readback4_pvr.rs's: the real GL core (the blit is its CPU
//! copy over the attachments' storage, src/gl/blit.rs) and the smallest
//! stand-in back end that makes a context. The stand-in records every
//! `GlesDraw` it is handed — what the PowerVR's draw stores, one PBE emit
//! per colour target (gpu address 0: a `GL_NONE` draw buffer, never
//! stored) — and fills a clear's colour into every target it names, so
//! glReadPixels sees that store. Nothing here is a GPU result: what the
//! PowerVR's own PBE words do is the board's.
//!
//! Pinned as today:
//! * `glReadBuffer(GL_COLOR_ATTACHMENT3)` as the blit source reaches a
//!   single-target framebuffer; a blit into a framebuffer whose draw buffers
//!   enable only attachment 3 writes attachment 3 alone; with all four draw
//!   buffers on, every draw buffer takes the read buffer's pixels (ES 3.0:
//!   every draw buffer gets the blit) — RGBA8 textures and RGBA8UI
//!   renderbuffers;
//! * an integer read buffer 3 into a float draw buffer is GL_INVALID_OPERATION
//!   and writes nothing; an R32UI texture at attachment 3 makes the FBO
//!   complete on the PowerVR (its widened integer RT, gl/framebuffer.rs
//!   `WIDE_INT_RT`), and integer-to-integer copies its red channel, the
//!   others reading 0, 0, 1 (src/gl/blit.rs copies integer textures,
//!   tests/es3_blit_int_tex.rs);
//! * an RGBA8UI texture attachment is blitted as an integer renderbuffer is
//!   (until src/gl/blit.rs `attachment_image` had an integer texture case,
//!   a silent no-op with no error), as tests/es3_mrt4_blit_v3d.rs checks it
//!   for the V3D build;
//! * GL_COLOR_ATTACHMENT3 in glInvalidateFramebuffer / glInvalidateSubFramebuffer
//!   is accepted (no error) alone, with 0..2, with the depth/stencil points,
//!   whole and as a sub-rectangle, on the draw, read and combined targets,
//!   and as a point with no image; GL_COLOR_ATTACHMENT4 (m =
//!   GL_MAX_COLOR_ATTACHMENTS = 4) is GL_INVALID_OPERATION; with the stand-in
//!   off (three, the build as it is) GL_COLOR_ATTACHMENT3 is past the limit
//!   and GL_INVALID_OPERATION;
//! * the invalidation records nothing in the back end: it hands the stand-in
//!   no draw and adds nothing to `attachment_gone`, and the draw after it
//!   carries the same four targets, target 3 stored as without the call (its
//!   clear colour reaches memory, seen by glReadPixels).
//!
//! The `#[ignore]`d case is the discard Mesa's PowerVR driver applies
//! (`pvr_setup_pbe_state` marks an invalidated surface): it needs the GL core
//! to pass the invalidation on and a `GlesDraw` field for it (plan #186's
//! route, as tests/es3_mrt4_invalidate_v3d.rs names for the V3D).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test es3_mrt4_blit_pvr
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
    name: c"es3-mrt4-blit-pvr".as_ptr() as *const c_char,
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

/// A three-target back end (the stand-in at 3; the PowerVR build itself
/// draws four since plan #325 item mrt-frameload, `DRAW_TARGETS`).
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

/* ---- the framebuffers: W x W attachments, each with its own pixels --------- */

const W: GLsizei = 4;

#[derive(Clone, Copy, PartialEq, Debug)]
enum Fmt {
    Rgba8,
    Rgba8ui,
}

/// A framebuffer of `n` attachments: textures for RGBA8, renderbuffers for
/// RGBA8UI (the GL core's blit copies an integer renderbuffer and, since
/// tests/es3_blit_int_tex.rs, an integer texture).
struct Fbo {
    fb: GLuint,
    texs: Vec<GLuint>,
    rbs: Vec<GLuint>,
}

impl Drop for Fbo {
    fn drop(&mut self) {
        // SAFETY: names this test made.
        unsafe {
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
            glDeleteFramebuffers(1, &self.fb);
            glDeleteTextures(self.texs.len() as GLsizei, self.texs.as_ptr());
            glDeleteRenderbuffers(self.rbs.len() as GLsizei, self.rbs.as_ptr());
        }
    }
}

/// The bytes attachment `k` holds at pixel (x, y) in the four-attachment
/// framebuffers.
fn val(k: usize, x: usize, y: usize) -> [u8; 4] {
    [(k * 40 + x * 8 + 1) as u8, (y * 16 + k) as u8, (200 - k * 20) as u8, (x + y * 4 + k * 3 + 10) as u8]
}

/// `n` attachments, attachment k filled with `fill(k, x, y)`.
fn fbo(fmt: Fmt, n: usize, fill: impl Fn(usize, usize, usize) -> [u8; 4]) -> Fbo {
    let (mut fb, mut texs, mut rbs) = (0, vec![0; n], vec![0; n]);
    let w = W as usize;
    // SAFETY: room for the names; the pixel array holds W x W RGBA bytes.
    unsafe {
        glGenTextures(n as GLsizei, texs.as_mut_ptr());
        glGenRenderbuffers(n as GLsizei, rbs.as_mut_ptr());
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        for k in 0..n {
            let px: Vec<u8> = (0..w * w).flat_map(|i| fill(k, i % w, i / w)).collect();
            match fmt {
                Fmt::Rgba8 => {
                    glBindTexture(GL_TEXTURE_2D, texs[k]);
                    let p = px.as_ptr() as *const c_void;
                    glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA8 as GLint, W, W, 0, GL_RGBA, GL_UNSIGNED_BYTE, p);
                    glFramebufferTexture2D(GL_FRAMEBUFFER, att(k), GL_TEXTURE_2D, texs[k], 0);
                }
                Fmt::Rgba8ui => {
                    glBindRenderbuffer(GL_RENDERBUFFER, rbs[k]);
                    glRenderbufferStorage(GL_RENDERBUFFER, GL_RGBA8UI, W, W);
                    glFramebufferRenderbuffer(GL_FRAMEBUFFER, att(k), GL_RENDERBUFFER, rbs[k]);
                    // the host's "GPU memory" is CPU memory; rows at the pair-padded pitch (W here)
                    let c = yakogl::gl::context::current().expect("a context");
                    let mem = c.objs_ref().renderbuffers[rbs[k] as usize].mem.cpu as *mut u8;
                    core::ptr::copy_nonoverlapping(px.as_ptr(), mem, px.len());
                }
            }
        }
        assert_eq!(err(), GL_NO_ERROR, "{fmt:?} fbo");
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE, "{fmt:?} x{n}");
    }
    Fbo { fb, texs, rbs }
}

fn att(k: usize) -> GLenum {
    GL_COLOR_ATTACHMENT0 + k as GLenum
}

const ALL4: [GLenum; 4] = [GL_COLOR_ATTACHMENT0, GL_COLOR_ATTACHMENT1, GL_COLOR_ATTACHMENT2, GL_COLOR_ATTACHMENT3];

/// Attachment `k` of `f` as RGBA bytes (the integer formats read their words'
/// low bytes: the test data is bytes).
fn read(f: &Fbo, fmt: Fmt, k: usize) -> Vec<[u8; 4]> {
    // SAFETY: room for W x W RGBA pixels (bytes, or words for the integer format).
    unsafe {
        glBindFramebuffer(GL_READ_FRAMEBUFFER, f.fb);
        glReadBuffer(att(k));
        let out: Vec<[u8; 4]> = match fmt {
            Fmt::Rgba8 => {
                let mut px = vec![0x77u8; (W * W * 4) as usize];
                glReadPixels(0, 0, W, W, GL_RGBA, GL_UNSIGNED_BYTE, px.as_mut_ptr() as *mut c_void);
                assert_eq!(err(), GL_NO_ERROR, "read {fmt:?} {k}");
                px.chunks(4).map(|c| [c[0], c[1], c[2], c[3]]).collect()
            }
            Fmt::Rgba8ui => {
                let mut u = vec![0xdead_u32; (W * W * 4) as usize];
                glReadPixels(0, 0, W, W, GL_RGBA_INTEGER, GL_UNSIGNED_INT, u.as_mut_ptr() as *mut c_void);
                assert_eq!(err(), GL_NO_ERROR, "read {fmt:?} {k}");
                u.chunks(4).map(|c| [c[0] as u8, c[1] as u8, c[2] as u8, c[3] as u8]).collect()
            }
        };
        out
    }
}

fn expect(f: impl Fn(usize, usize) -> [u8; 4]) -> Vec<[u8; 4]> {
    (0..(W * W) as usize).map(|i| f(i % W as usize, i / W as usize)).collect()
}

fn blit(src: &Fbo, read_k: usize, dst: &Fbo, draw: &[GLenum]) {
    // SAFETY: plain GL calls; `draw` outlives glDrawBuffers.
    unsafe {
        glBindFramebuffer(GL_READ_FRAMEBUFFER, src.fb);
        glReadBuffer(att(read_k));
        glBindFramebuffer(GL_DRAW_FRAMEBUFFER, dst.fb);
        glDrawBuffers(draw.len() as GLsizei, draw.as_ptr());
        assert_eq!(err(), GL_NO_ERROR, "state");
        glBlitFramebuffer(0, 0, W, W, 0, 0, W, W, GL_COLOR_BUFFER_BIT, GL_NEAREST);
    }
}

/* ---- the blit of attachment 3 ------------------------------------------------ */

fn read_att3_into_single(fmt: Fmt) {
    let _g = context();
    let src = fbo(fmt, 4, val);
    let dst = fbo(fmt, 1, |_, _, _| [9, 9, 9, 9]);
    blit(&src, 3, &dst, &[att(0)]);
    assert_eq!(err(), GL_NO_ERROR, "blit");
    assert_eq!(read(&dst, fmt, 0), expect(|x, y| val(3, x, y)), "{fmt:?}: attachment 3 reaches the single target");
    for k in 0..4 {
        assert_eq!(read(&src, fmt, k), expect(|x, y| val(k, x, y)), "{fmt:?}: source {k} is untouched");
    }
}

fn only_att3_is_written(fmt: Fmt) {
    let _g = context();
    let src = fbo(fmt, 1, |_, x, y| val(7, x, y));
    let dst = fbo(fmt, 4, val);
    blit(&src, 0, &dst, &[GL_NONE, GL_NONE, GL_NONE, att(3)]);
    assert_eq!(err(), GL_NO_ERROR, "blit");
    assert_eq!(read(&dst, fmt, 3), expect(|x, y| val(7, x, y)), "{fmt:?}: attachment 3 takes the source");
    for k in 0..3 {
        assert_eq!(read(&dst, fmt, k), expect(|x, y| val(k, x, y)), "{fmt:?}: attachment {k} keeps its pixels");
    }
}

fn all_four_are_written(fmt: Fmt) {
    let _g = context();
    let src = fbo(fmt, 4, val);
    let dst = fbo(fmt, 4, |_, _, _| [3, 3, 3, 3]);
    blit(&src, 3, &dst, &ALL4);
    assert_eq!(err(), GL_NO_ERROR, "blit");
    for k in 0..4 {
        assert_eq!(read(&dst, fmt, k), expect(|x, y| val(3, x, y)), "{fmt:?}: attachment {k} takes attachment 3");
    }
}

#[test]
fn four_draw_buffers_are_offered() {
    let _g = context();
    assert_eq!((get(GL_MAX_DRAW_BUFFERS), get(GL_MAX_COLOR_ATTACHMENTS)), (4, 4), "the stand-in");
    let _src = fbo(Fmt::Rgba8, 4, val);
    // SAFETY: a live array of four names.
    unsafe { glDrawBuffers(4, ALL4.as_ptr()) };
    assert_eq!(err(), GL_NO_ERROR, "glDrawBuffers(4)");
}

#[test]
fn rgba8_read_attachment3_into_single() {
    read_att3_into_single(Fmt::Rgba8);
}

#[test]
fn rgba8_draw_only_attachment3() {
    only_att3_is_written(Fmt::Rgba8);
}

#[test]
fn rgba8_draw_all_four() {
    all_four_are_written(Fmt::Rgba8);
}

#[test]
fn rgba8ui_read_attachment3_into_single() {
    read_att3_into_single(Fmt::Rgba8ui);
}

#[test]
fn rgba8ui_draw_only_attachment3() {
    only_att3_is_written(Fmt::Rgba8ui);
}

#[test]
fn rgba8ui_draw_all_four() {
    all_four_are_written(Fmt::Rgba8ui);
}

/// An RGBA8UI *texture* at attachment 3 is blitted: the GL core copies an
/// integer texture image as it copies an integer renderbuffer
/// (src/gl/blit.rs `attachment_image`; until then the blit was a silent
/// no-op with no error), as tests/es3_mrt4_blit_v3d.rs checks for the V3D
/// build. Attachment 3's 10s arrive over the destination's 20s.
#[test]
fn rgba8ui_texture_attachment3_blit_copies() {
    let _g = context();
    let (mut fb, mut tex) = ([0; 2], [0; 2]);
    // SAFETY: plain GL calls; the pixel array holds W x W RGBA bytes.
    unsafe {
        glGenFramebuffers(2, fb.as_mut_ptr());
        glGenTextures(2, tex.as_mut_ptr());
        for i in 0..2 {
            let px = vec![(i as u8 + 1) * 10; (W * W * 4) as usize];
            let p = px.as_ptr() as *const c_void;
            glBindTexture(GL_TEXTURE_2D, tex[i]);
            glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA8UI as GLint, W, W, 0, GL_RGBA_INTEGER, GL_UNSIGNED_BYTE, p);
            glBindFramebuffer(GL_FRAMEBUFFER, fb[i]);
            glFramebufferTexture2D(GL_FRAMEBUFFER, att(if i == 0 { 3 } else { 0 }), GL_TEXTURE_2D, tex[i], 0);
        }
        glBindFramebuffer(GL_READ_FRAMEBUFFER, fb[0]);
        glReadBuffer(att(3));
        glBindFramebuffer(GL_DRAW_FRAMEBUFFER, fb[1]);
        glBlitFramebuffer(0, 0, W, W, 0, 0, W, W, GL_COLOR_BUFFER_BIT, GL_NEAREST);
        assert_eq!(err(), GL_NO_ERROR);
        let mut u = vec![0u32; (W * W * 4) as usize];
        glBindFramebuffer(GL_READ_FRAMEBUFFER, fb[1]);
        glReadBuffer(att(0));
        glReadPixels(0, 0, W, W, GL_RGBA_INTEGER, GL_UNSIGNED_INT, u.as_mut_ptr() as *mut c_void);
        assert_eq!(err(), GL_NO_ERROR);
        assert!(u.iter().all(|&v| v == 10), "attachment 3's 10s over the destination's 20s: {u:?}");
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glDeleteFramebuffers(2, fb.as_ptr());
        glDeleteTextures(2, tex.as_ptr());
    }
}

/// An integer read buffer 3 into a normalised draw buffer is refused, and
/// nothing is written.
#[test]
fn integer_attachment3_into_rgba8_is_refused() {
    let _g = context();
    let src = fbo(Fmt::Rgba8ui, 4, val);
    let dst = fbo(Fmt::Rgba8, 4, |_, _, _| [5, 5, 5, 5]);
    blit(&src, 3, &dst, &[GL_NONE, GL_NONE, GL_NONE, att(3)]);
    assert_eq!(err(), GL_INVALID_OPERATION);
    assert_eq!(read(&dst, Fmt::Rgba8, 3), vec![[5, 5, 5, 5]; (W * W) as usize]);
}

/// An R32UI texture at attachment 3 makes the framebuffer complete on the
/// PowerVR: the build draws an R integer texture as the four-channel widened
/// surface (gl/framebuffer.rs `WIDE_INT_RT`, `pbe_int_packmode`). A blit from
/// that read buffer keeps the format rules: into a float draw buffer it is
/// GL_INVALID_OPERATION, and integer-to-integer copies the red channel out
/// of the widened store, the channels R32UI lacks reading 0, 0, 1.
#[test]
fn r32ui_attachment3_is_drawable_and_its_blit_follows_the_integer_rules() {
    let _g = context();
    let texs = [texture(GL_RGBA8), texture(GL_RGBA8), texture(GL_RGBA8), texture(GL_R32UI)];
    let mut fb = 0;
    let red = vec![9u32; (W * W) as usize];
    // SAFETY: room for one name; the textures are 2D level 0; `red` holds
    // W x W R32UI texels.
    unsafe {
        glBindTexture(GL_TEXTURE_2D, texs[3]);
        let p = red.as_ptr() as *const c_void;
        glTexSubImage2D(GL_TEXTURE_2D, 0, 0, 0, W, W, GL_RED_INTEGER, GL_UNSIGNED_INT, p);
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        for (i, t) in texs.iter().enumerate() {
            glFramebufferTexture2D(GL_FRAMEBUFFER, att(i), GL_TEXTURE_2D, *t, 0);
        }
        assert_eq!(err(), GL_NO_ERROR, "attach an R32UI texture at attachment 3");
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE, "widened integer RT");
    }
    let dst = fbo(Fmt::Rgba8, 1, |_, _, _| [5, 5, 5, 5]);
    // SAFETY: plain GL calls.
    unsafe {
        glBindFramebuffer(GL_READ_FRAMEBUFFER, fb);
        glReadBuffer(att(3));
        glBindFramebuffer(GL_DRAW_FRAMEBUFFER, dst.fb);
        glBlitFramebuffer(0, 0, W, W, 0, 0, W, W, GL_COLOR_BUFFER_BIT, GL_NEAREST);
    }
    assert_eq!(err(), GL_INVALID_OPERATION, "integer read buffer 3 into a float draw buffer");
    assert_eq!(read(&dst, Fmt::Rgba8, 0), vec![[5, 5, 5, 5]; (W * W) as usize]);
    /* integer-to-integer passes the format rules and copies */
    let uint = fbo(Fmt::Rgba8ui, 1, |_, _, _| [7, 7, 7, 7]);
    // SAFETY: plain GL calls.
    unsafe {
        glBindFramebuffer(GL_READ_FRAMEBUFFER, fb);
        glReadBuffer(att(3));
        glBindFramebuffer(GL_DRAW_FRAMEBUFFER, uint.fb);
        glBlitFramebuffer(0, 0, W, W, 0, 0, W, W, GL_COLOR_BUFFER_BIT, GL_NEAREST);
    }
    assert_eq!(err(), GL_NO_ERROR, "R32UI read buffer 3 into an RGBA8UI draw buffer");
    assert_eq!(read(&uint, Fmt::Rgba8ui, 0), vec![[9, 0, 0, 1]; (W * W) as usize], "R of R32UI, then 0, 0, 1");
    // SAFETY: names this test made.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glDeleteFramebuffers(1, &fb);
        glDeleteTextures(4, texs.as_ptr());
    }
}

/// A `W x W` 2D texture of `f`, filled by storage alone (contents unread: the
/// blit rules see these through the format, not the pixels).
fn texture(f: GLenum) -> GLuint {
    let mut tex = 0;
    // SAFETY: room for one name.
    unsafe {
        glGenTextures(1, &mut tex);
        glBindTexture(GL_TEXTURE_2D, tex);
        glTexStorage2D(GL_TEXTURE_2D, 1, f, W, W);
    }
    assert_eq!(err(), GL_NO_ERROR, "glTexStorage2D {f:#x}");
    tex
}

/* ---- glInvalidateFramebuffer / glInvalidateSubFramebuffer ------------------ */

fn rb(format: GLenum) -> GLuint {
    let mut r = 0;
    // SAFETY: room for one name.
    unsafe {
        glGenRenderbuffers(1, &mut r);
        glBindRenderbuffer(GL_RENDERBUFFER, r);
        glRenderbufferStorage(GL_RENDERBUFFER, format, W, W);
    }
    assert_eq!(err(), GL_NO_ERROR, "storage 0x{format:x}");
    r
}

/// A complete framebuffer of `n` RGBA8 renderbuffers on attachment 0..
/// (deleted by its Drop; read it as `Fmt::Rgba8`).
fn fbo_rbs(n: u32) -> Fbo {
    let (mut f, rbs) = (0, (0..n).map(|_| rb(GL_RGBA8)).collect::<Vec<_>>());
    // SAFETY: room for one name; the renderbuffers are live.
    unsafe {
        glGenFramebuffers(1, &mut f);
        glBindFramebuffer(GL_FRAMEBUFFER, f);
        for (i, &r) in rbs.iter().enumerate() {
            glFramebufferRenderbuffer(GL_FRAMEBUFFER, att(i), GL_RENDERBUFFER, r);
        }
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE);
    }
    assert_eq!(err(), GL_NO_ERROR);
    Fbo { fb: f, texs: Vec::new(), rbs }
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

/// GL_COLOR_ATTACHMENT3 alone, with 0..2, with the depth/stencil points,
/// whole and as a sub-rectangle, on every framebuffer target: no error. A
/// point with no image is still a point.
#[test]
fn invalidate_colour_attachment_3_is_accepted() {
    let _g = context();
    let _f = fbo_rbs(4);
    draw_buffers(&ALL4);
    let lists: [&[GLenum]; 5] = [
        &[GL_COLOR_ATTACHMENT3],
        &ALL4,
        &[GL_COLOR_ATTACHMENT3, GL_COLOR_ATTACHMENT0],
        &[GL_COLOR_ATTACHMENT2, GL_COLOR_ATTACHMENT3],
        &[GL_COLOR_ATTACHMENT3, GL_DEPTH_ATTACHMENT, GL_STENCIL_ATTACHMENT, GL_DEPTH_STENCIL_ATTACHMENT],
    ];
    for t in [GL_FRAMEBUFFER, GL_DRAW_FRAMEBUFFER, GL_READ_FRAMEBUFFER] {
        for atts in lists {
            assert_eq!(inv(t, atts), GL_NO_ERROR, "0x{t:x} {atts:x?}");
            assert_eq!(inv_sub(t, atts, 0, 0, W, W), GL_NO_ERROR, "sub 0x{t:x} {atts:x?}");
            assert_eq!(inv_sub(t, atts, 2, 3, 5, 7), GL_NO_ERROR, "sub 0x{t:x} {atts:x?}");
        }
        /* a rectangle past the surface, and an empty one: legal */
        assert_eq!(inv_sub(t, &[GL_COLOR_ATTACHMENT3], -4, -4, 100, 100), GL_NO_ERROR);
        assert_eq!(inv_sub(t, &[GL_COLOR_ATTACHMENT3], 0, 0, 0, 0), GL_NO_ERROR);
    }
    let _f3 = fbo_rbs(3);
    assert_eq!(inv(GL_FRAMEBUFFER, &[GL_COLOR_ATTACHMENT3]), GL_NO_ERROR, "point 3 with no image");
}

/// m = GL_MAX_COLOR_ATTACHMENTS (4) is GL_INVALID_OPERATION (ES 3.0 4.5.1),
/// alone or after legal names; with a three-target stand-in point 3 is.
#[test]
fn invalidate_colour_attachment_4_is_invalid_operation() {
    let _g = context();
    let _f = fbo_rbs(4);
    draw_buffers(&ALL4);
    for t in [GL_FRAMEBUFFER, GL_DRAW_FRAMEBUFFER, GL_READ_FRAMEBUFFER] {
        assert_eq!(inv(t, &[GL_COLOR_ATTACHMENT4]), GL_INVALID_OPERATION, "0x{t:x}");
        assert_eq!(inv_sub(t, &[GL_COLOR_ATTACHMENT4], 0, 0, 4, 4), GL_INVALID_OPERATION, "sub 0x{t:x}");
        assert_eq!(inv(t, &[GL_COLOR_ATTACHMENT0, GL_COLOR_ATTACHMENT3, GL_COLOR_ATTACHMENT4]), GL_INVALID_OPERATION);
    }
    three_targets();
    assert_eq!(get(GL_MAX_COLOR_ATTACHMENTS), 3);
    let r =
        (inv(GL_FRAMEBUFFER, &[GL_COLOR_ATTACHMENT3]), inv_sub(GL_FRAMEBUFFER, &[GL_COLOR_ATTACHMENT3], 0, 0, 4, 4));
    backend::HOST_DRAW_TARGETS.store(4, Ordering::Relaxed);
    assert_eq!(r, (GL_INVALID_OPERATION, GL_INVALID_OPERATION), "three targets: point 3 is past the limit");
}

fn draw_buffers(bufs: &[GLenum]) {
    // SAFETY: a live array.
    unsafe { glDrawBuffers(bufs.len() as GLsizei, bufs.as_ptr()) };
    assert_eq!(err(), GL_NO_ERROR, "glDrawBuffers({bufs:x?})");
}

/// Draws the stand-in has been handed since the mark.
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

/// A four-draw-buffer framebuffer cleared twice, the second time after an
/// invalidation of GL_COLOR_ATTACHMENT3: the draws each target stores.
fn stores_with_and_without(atts: &[GLenum]) -> (SeenDraw, SeenDraw) {
    let m = mark();
    clear_and_finish();
    let base = since(m);
    assert_eq!(base.len(), 1, "the clear went out as one draw");
    assert_eq!(base[0].num_targets, 4, "four draw buffers hand four targets: {:?}", base[0]);
    let m = mark();
    /* storage specification already fires attachment_gone (gl/framebuffer.rs
     * respecified); the pin is that the invalidation adds nothing */
    let gone = GONE.lock().unwrap().len();
    assert_eq!(inv(GL_FRAMEBUFFER, atts), GL_NO_ERROR, "{atts:x?}");
    assert_eq!(inv_sub(GL_FRAMEBUFFER, atts, 0, 0, W, W), GL_NO_ERROR, "sub {atts:x?}");
    assert_eq!(since(m), Vec::new(), "the invalidation hands the back end nothing");
    assert_eq!(GONE.lock().unwrap().len(), gone, "and adds nothing to attachment_gone");
    clear_and_finish();
    let after = since(m);
    assert_eq!(after.len(), 1);
    (base[0].clone(), after[0].clone())
}

/// The invalidation records nothing in the back end, and the draw after it
/// stores target 3 exactly as without the call: target 3's address is in the
/// record (the PowerVR render's fourth PBE emit keeps its pair,
/// docs/notes/mrt4-v3d-pvr.md 2.1) and its clear colour reaches memory.
#[test]
fn invalidating_attachment3_changes_what_the_next_draw_stores_in_no_way() {
    let _g = context();
    let f = fbo_rbs(4);
    draw_buffers(&ALL4);
    let lists: [&[GLenum]; 4] = [
        &[GL_COLOR_ATTACHMENT3],
        &ALL4,
        &[GL_COLOR_ATTACHMENT0, GL_COLOR_ATTACHMENT3],
        &[GL_COLOR_ATTACHMENT0, GL_COLOR_ATTACHMENT1, GL_COLOR_ATTACHMENT2],
    ];
    for atts in lists {
        let (base, after) = stores_with_and_without(atts);
        assert!(base.targets[3].0 != 0, "target 3 is stored: {:?}", base);
        assert_eq!(after, base, "{atts:x?}");
    }
    /* and the store is real: the clear colour comes back from attachment 3 */
    assert_eq!(read(&f, Fmt::Rgba8, 3), vec![[0, 0, 255, 255]; (W * W) as usize], "attachment 3 after the clear");
}

/// The discard Mesa's PowerVR driver applies to an invalidated surface
/// (`pvr_setup_pbe_state` / `pvr_arch_pbe_pack_state`): the next render must
/// not store attachment 3. Ignored: the invalidation is a validated no-op in
/// src/gl/framebuffer.rs (`invalidate_framebuffer`, plan #186) and GlesDraw
/// has no field for it, so the fourth PBE emit stores it anyway (the named
/// limit tests/es3_mrt4_invalidate_v3d.rs gives for the V3D).
#[test]
#[ignore = "defect: glInvalidateFramebuffer reaches no back-end field, so the draw after an invalidation of GL_COLOR_ATTACHMENT3 still stores target 3"]
fn an_invalidated_attachment3_is_not_stored_by_the_next_draw() {
    let _g = context();
    let _f = fbo_rbs(4);
    draw_buffers(&ALL4);
    let (base, after) = stores_with_and_without(&[GL_COLOR_ATTACHMENT3]);
    assert_ne!(after.targets[3].0, 0, "and today it is stored ({base:?} -> {after:?})");
    assert_eq!(after.targets[3].0, 0, "a discarded target would reach the back end as 0 / an unstored target");
}
