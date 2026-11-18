// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! A framebuffer with a GL_RGB10_A2 colour attachment beside an RGBA8 one on
//! the V3D host back end — multiple render targets — through the GL entry
//! points only (plan #370, mrt-api), in either attachment order.
//!
//! - the mixed framebuffer is complete with either format at either
//!   attachment, and glDrawBuffers names both draw buffers;
//! - glClearBufferfv clears one draw buffer and only that one (the other
//!   keeps what it held); the same clear value stores as 8-bit alpha on the
//!   RGBA8 attachment and as 2-bit alpha on the RGB10_A2 one, and
//!   glReadBuffer(GL_COLOR_ATTACHMENT1) reads the second attachment with the
//!   pairs the first one takes, GL_RGBA/GL_UNSIGNED_INT_2_10_10_10_REV
//!   included;
//! - glBlitFramebuffer NEAREST with the read buffer on the RGB10_A2 or the
//!   RGBA8 attachment, into an RGB10_A2 or an RGBA8 destination: same class
//!   keeps the pixels, RGBA8 <-> RGB10_A2 converts them;
//! - glClear clears both draw buffers, and skips the one named GL_NONE.
//!
//! The GL core does not touch pixels for a clear; `b_draw` is a stand-in
//! that applies the clear draw to the targets its mask names
//! (`GlesDraw::targets`, `clear_target_mask`), each in its own format (see
//! `b_draw`). The read pair (GL_IMPLEMENTATION_COLOR_READ_*) is another
//! item's and is not asserted here.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_rgb10a2_mrt_v3d
//! ```

#![cfg(all(feature = "host-test", feature = "shaderc-online", feature = "gpu-v3d", not(feature = "report-es2")))]

use core::ffi::{c_char, c_void};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{self, GlesHeap, YakoglBackend};
use yakogl::gl::consts::*;
use yakogl::gl::es3::*;
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
/// Colour targets the stand-in back end cleared (one per target of a clear
/// draw, so a clear reaching both draw buffers counts two).
static CLEARS: AtomicU32 = AtomicU32::new(0);
/// A clear target in a format this stand-in does not model (nothing was
/// written for it); the tests would fail on their read-backs anyway, this
/// names why.
static BAD_TARGET: AtomicU32 = AtomicU32::new(0);

/// The stand-in GPU: a clear draw writes the clear colour into every colour
/// target its mask names — `GlesDraw::clear_target_mask` bit k is
/// `GlesDraw::targets[k]`, 0 is every target — each in its own format: a
/// `GLES_SURF_RGB10_A2` target takes the 10/10/10/2 word (R/G/B
/// `clamp(v, 0, 1) * 1023` rounded half up, alpha over 3 — the rounding to
/// the format's real widths is the back end's, the GL core hands the clear
/// values through unchanged), a `GLES_SURF_RGBA8888` target four bytes
/// (`* 255` rounded half up), for the channels of `clear_color_mask` only
/// and inside the scissor box. That is the V3D back end's job (the TLB
/// clear), which does not run on the host; the test judges what the GL core
/// hands it per draw buffer and what glReadPixels makes of the stored
/// pixels.
unsafe extern "C" fn b_draw(d: *const backend::GlesDraw) -> i32 {
    let d = &*d;
    if !d.do_clear {
        return 0;
    }
    let q = |v: f32, steps: f32| (v.clamp(0.0, 1.0) * steps + 0.5) as u32;
    let c = d.clear_rgba;
    let mask = if d.clear_target_mask == 0 { 0xffu32 } else { d.clear_target_mask as u32 };
    let n = (d.num_targets as usize).clamp(1, backend::GLES_MAX_TARGETS);
    for k in 0..n {
        let t = &d.targets[k];
        if mask >> k & 1 == 0 || t.gpu == 0 {
            continue;
        }
        if t.format != backend::GLES_SURF_RGB10_A2 && t.format != backend::GLES_SURF_RGBA8888 {
            BAD_TARGET.fetch_add(1, Ordering::SeqCst);
            continue;
        }
        CLEARS.fetch_add(1, Ordering::SeqCst);
        let pitch = if k == 0 { backend::draw_pitch(d) } else { t.w.wrapping_add(1) & !1 } as usize;
        let [sx, sy, sw, sh] = if d.scissor { d.scissor_rect } else { [0, 0, t.w as i32, t.h as i32] };
        for y in sy.max(0)..(sy + sh).min(t.h as i32) {
            for x in sx.max(0)..(sx + sw).min(t.w as i32) {
                let off = y as usize * pitch + x as usize;
                if t.format == backend::GLES_SURF_RGB10_A2 {
                    let p = (t.gpu as *mut u32).add(off);
                    let w = *p;
                    let old = [w & 0x3ff, w >> 10 & 0x3ff, w >> 20 & 0x3ff, w >> 30];
                    let new = [q(c[0], 1023.0), q(c[1], 1023.0), q(c[2], 1023.0), q(c[3], 3.0)];
                    let f: Vec<u32> =
                        (0..4).map(|i| if d.clear_color_mask >> i & 1 != 0 { new[i] } else { old[i] }).collect();
                    *p = f[0] | f[1] << 10 | f[2] << 20 | f[3] << 30;
                } else {
                    let p = (t.gpu as *mut u8).add(off * 4);
                    let new = [q(c[0], 255.0) as u8, q(c[1], 255.0) as u8, q(c[2], 255.0) as u8, q(c[3], 255.0) as u8];
                    for (i, &v) in new.iter().enumerate() {
                        if d.clear_color_mask >> i & 1 != 0 {
                            *p.add(i) = v;
                        }
                    }
                }
            }
        }
    }
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
    name: c"es3-rgb10a2-mrt".as_ptr() as *const c_char,
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
        glDisable(GL_SCISSOR_TEST);
        glColorMask(1, 1, 1, 1);
        glClearColor(0.0, 0.0, 0.0, 0.0);
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

const N: GLsizei = 4;

#[derive(Clone, Copy, Debug, PartialEq)]
enum Fmt {
    Rgba8,
    Rgb10A2,
}

/// The image's slot in a `Mrt`'s `names` (and `uploads`).
fn slot(fmt: Fmt) -> usize {
    match fmt {
        Fmt::Rgba8 => 0,
        Fmt::Rgb10A2 => 1,
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Kind {
    Tex,
    Rb,
}

fn internal(fmt: Fmt) -> GLenum {
    match fmt {
        Fmt::Rgba8 => GL_RGBA8,
        Fmt::Rgb10A2 => GL_RGB10_A2,
    }
}

/// The upload and read type that pairs with the format.
fn upload_type(fmt: Fmt) -> GLenum {
    match fmt {
        Fmt::Rgba8 => GL_UNSIGNED_BYTE,
        Fmt::Rgb10A2 => GL_UNSIGNED_INT_2_10_10_10_REV,
    }
}

fn scale(v: u32, max: u32) -> u8 {
    ((v * 255 + max / 2) / max) as u8
}

/// The bytes glReadPixels RGBA/UNSIGNED_BYTE gives for a stored 10/10/10/2
/// word.
fn decoded_word(w: u32) -> [u8; 4] {
    [scale(w & 0x3ff, 1023), scale((w >> 10) & 0x3ff, 1023), scale((w >> 20) & 0x3ff, 1023), scale(w >> 30, 3)]
}

/// The bytes glReadPixels RGBA/UNSIGNED_BYTE gives for `c` stored in `fmt`:
/// the stand-in's quantisation (`b_draw`) then the GL core's decode of the
/// stored 10 (or 2) bits to a byte, (v * 255 + max / 2) / max.
fn stored(fmt: Fmt, c: [f32; 4]) -> [u8; 4] {
    let q = |v: f32, steps: f32| (v.clamp(0.0, 1.0) * steps + 0.5) as u32;
    match fmt {
        Fmt::Rgba8 => [q(c[0], 255.0) as u8, q(c[1], 255.0) as u8, q(c[2], 255.0) as u8, q(c[3], 255.0) as u8],
        Fmt::Rgb10A2 => decoded_word(a2(q(c[0], 1023.0), q(c[1], 1023.0), q(c[2], 1023.0), q(c[3], 3.0))),
    }
}

/// The word the stand-in back end stores for `c` in RGB10_A2.
fn word(c: [f32; 4]) -> u32 {
    let q = |v: f32, steps: f32| (v.clamp(0.0, 1.0) * steps + 0.5) as u32;
    a2(q(c[0], 1023.0), q(c[1], 1023.0), q(c[2], 1023.0), q(c[3], 3.0))
}

/// `c = (0.2, 0.4, 0.6, 0.4)`: the clear values under test, chosen so the
/// alpha differs between 8-bit rounding (102) and 2-bit rounding (1, which
/// glReadPixels decodes to 85), and so the colour bytes land the same way on
/// both formats (51, 102, 153).
const CLEAR: [f32; 4] = [0.2, 0.4, 0.6, 0.4];
/// A first clear that marks what each attachment held.
const SENT: [f32; 4] = [0.9, 0.1, 0.5, 1.0];

/// A framebuffer with one RGBA8 and one RGB10_A2 colour attachment, N x N;
/// `order[0]` names the format at GL_COLOR_ATTACHMENT0 (both draw buffers
/// are named with glDrawBuffers at once).
struct Mrt {
    kind: Kind,
    fb: GLuint,
    names: [GLuint; 2],
    at0: Fmt,
}

impl Mrt {
    fn new(order: [Fmt; 2], kind: Kind) -> Mrt {
        let (mut fb, mut names) = (0, [0; 2]);
        // SAFETY: room for the names; the uploads are null (storage only).
        unsafe {
            match kind {
                Kind::Tex => {
                    glGenTextures(2, names.as_mut_ptr());
                    for (i, &name) in names.iter().enumerate() {
                        glBindTexture(GL_TEXTURE_2D, name);
                        let fmt = if i == 0 { Fmt::Rgba8 } else { Fmt::Rgb10A2 };
                        glTexImage2D(
                            GL_TEXTURE_2D,
                            0,
                            internal(fmt) as GLint,
                            N,
                            N,
                            0,
                            GL_RGBA,
                            upload_type(fmt),
                            core::ptr::null(),
                        );
                        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST as GLint);
                    }
                }
                Kind::Rb => {
                    glGenRenderbuffers(2, names.as_mut_ptr());
                    for (i, &name) in names.iter().enumerate() {
                        glBindRenderbuffer(GL_RENDERBUFFER, name);
                        let fmt = if i == 0 { Fmt::Rgba8 } else { Fmt::Rgb10A2 };
                        glRenderbufferStorage(GL_RENDERBUFFER, internal(fmt), N, N);
                    }
                }
            }
            glGenFramebuffers(1, &mut fb);
            glBindFramebuffer(GL_FRAMEBUFFER, fb);
            for (i, &fmt) in order.iter().enumerate() {
                let point = GL_COLOR_ATTACHMENT0 + i as GLenum;
                match kind {
                    Kind::Tex => glFramebufferTexture2D(GL_FRAMEBUFFER, point, GL_TEXTURE_2D, names[slot(fmt)], 0),
                    Kind::Rb => glFramebufferRenderbuffer(GL_FRAMEBUFFER, point, GL_RENDERBUFFER, names[slot(fmt)]),
                }
            }
            assert_eq!(err(), GL_NO_ERROR, "attach {order:?} {kind:?}");
            let both = [GL_COLOR_ATTACHMENT0, GL_COLOR_ATTACHMENT1];
            glDrawBuffers(2, both.as_ptr());
            assert_eq!(err(), GL_NO_ERROR, "glDrawBuffers both attachments");
            assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE, "{order:?} {kind:?}");
        }
        Mrt { kind, fb, names, at0: order[0] }
    }

    /// Re-specify a texture attachment with N x N pixels of `upload_type`
    /// (`px` of upload_type bytes).
    fn upload(&self, fmt: Fmt, px: &[u8]) {
        assert_eq!(self.kind, Kind::Tex);
        assert_eq!(px.len(), (N * N) as usize * 4);
        // SAFETY: a name this Mrt made; `px` holds N x N texels of the type.
        unsafe {
            glBindTexture(GL_TEXTURE_2D, self.names[slot(fmt)]);
            glTexImage2D(
                GL_TEXTURE_2D,
                0,
                internal(fmt) as GLint,
                N,
                N,
                0,
                GL_RGBA,
                upload_type(fmt),
                px.as_ptr() as *const c_void,
            );
        }
        assert_eq!(err(), GL_NO_ERROR, "upload {fmt:?}");
    }

    /// The framebuffer's own draw-buffer names.
    fn draw_buffers(&self, bufs: &[GLenum]) {
        // SAFETY: plain GL calls.
        unsafe {
            glBindFramebuffer(GL_FRAMEBUFFER, self.fb);
            glDrawBuffers(bufs.len() as GLsizei, bufs.as_ptr());
        }
        assert_eq!(err(), GL_NO_ERROR, "glDrawBuffers {bufs:?}");
    }

    fn att(&self, fmt: Fmt) -> GLenum {
        GL_COLOR_ATTACHMENT0 + if fmt == self.at0 { 0 } else { 1 }
    }

    /// The framebuffer stays bound (read and draw), so a clear taken here
    /// reaches this framebuffer.
    fn bind(&self) {
        // SAFETY: a plain GL call.
        unsafe { glBindFramebuffer(GL_FRAMEBUFFER, self.fb) };
    }

    fn status(&self) -> GLenum {
        // SAFETY: a plain GL call.
        unsafe { glCheckFramebufferStatus(GL_FRAMEBUFFER) }
    }

    /// glReadPixels RGBA/`ty` of attachment `att(fmt)` (glReadBuffer names
    /// it; an RGB10_A2 attachment read with UNSIGNED_BYTE decodes each
    /// field, read with the REV pair returns the words).
    fn read(&self, fmt: Fmt, ty: GLenum) -> Vec<u8> {
        let mut px = vec![0x77u8; (N * N) as usize * 4];
        // SAFETY: room for N x N RGBA pixels of the type.
        unsafe {
            glBindFramebuffer(GL_READ_FRAMEBUFFER, self.fb);
            glReadBuffer(self.att(fmt));
            glReadPixels(0, 0, N, N, GL_RGBA, ty, px.as_mut_ptr() as *mut c_void);
        }
        assert_eq!(err(), GL_NO_ERROR, "glReadPixels {fmt:?} at attachment {}", self.att(fmt));
        px
    }

    /// Words read from the RGB10_A2 attachment.
    fn read_words(&self, fmt: Fmt) -> Vec<u32> {
        let px = self.read(fmt, GL_UNSIGNED_INT_2_10_10_10_REV);
        px.chunks(4).map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect()
    }

    fn expect_bytes(&self, fmt: Fmt, c: [f32; 4], what: &str) {
        let want = stored(fmt, c).to_vec().repeat((N * N) as usize);
        assert_eq!(self.read(fmt, GL_UNSIGNED_BYTE), want, "{what}: {fmt:?} at {}", self.att(fmt));
    }

    fn expect_word(&self, c: u32, what: &str) {
        assert_eq!(self.read_words(Fmt::Rgb10A2), vec![c; (N * N) as usize], "{what}");
    }
}

impl Drop for Mrt {
    fn drop(&mut self) {
        // SAFETY: names this test made.
        unsafe {
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
            glDeleteFramebuffers(1, &self.fb);
            match self.kind {
                Kind::Rb => glDeleteRenderbuffers(2, self.names.as_ptr()),
                Kind::Tex => glDeleteTextures(2, self.names.as_ptr()),
            }
        }
    }
}

/// glClearColor + glClear(GL_COLOR_BUFFER_BIT) on the bound framebuffer.
fn clear_all(c: [f32; 4]) {
    // SAFETY: plain GL calls.
    unsafe {
        glClearColor(c[0], c[1], c[2], c[3]);
        glClear(GL_COLOR_BUFFER_BIT);
    }
    assert_eq!(err(), GL_NO_ERROR, "glClear");
}

/// glClearBufferfv on draw buffer `i` of the bound framebuffer, with the
/// call's own value.
fn clear_one(m: &Mrt, i: GLint, c: [f32; 4]) {
    m.bind();
    // SAFETY: `c` holds four floats.
    unsafe { glClearBufferfv(GL_COLOR, i, c.as_ptr()) };
}

fn clears() -> u32 {
    assert_eq!(BAD_TARGET.load(Ordering::SeqCst), 0, "a clear reached a format the stand-in does not model");
    CLEARS.load(Ordering::SeqCst)
}

/* ---- (a) a mixed framebuffer, both orders, textures and renderbuffers ---- */

#[test]
fn mixed_attachments_are_complete_textures() {
    let _g = context();
    let m = Mrt::new([Fmt::Rgba8, Fmt::Rgb10A2], Kind::Tex);
    assert_eq!(m.status(), GL_FRAMEBUFFER_COMPLETE, "RGBA8 at attachment 0, RGB10_A2 at attachment 1");
    let m = Mrt::new([Fmt::Rgb10A2, Fmt::Rgba8], Kind::Tex);
    assert_eq!(m.status(), GL_FRAMEBUFFER_COMPLETE, "RGB10_A2 at attachment 0, RGBA8 at attachment 1");
}

#[test]
fn mixed_attachments_are_complete_renderbuffers() {
    let _g = context();
    let m = Mrt::new([Fmt::Rgba8, Fmt::Rgb10A2], Kind::Rb);
    assert_eq!(m.status(), GL_FRAMEBUFFER_COMPLETE, "renderbuffers, RGBA8 first");
    let m = Mrt::new([Fmt::Rgb10A2, Fmt::Rgba8], Kind::Rb);
    assert_eq!(m.status(), GL_FRAMEBUFFER_COMPLETE, "renderbuffers, RGB10_A2 first");
}

/* ---- (b) glClearBufferfv per draw buffer, and reading either attachment -- */

/// The clear of draw buffer 0 lands on that attachment only: the RGB10_A2
/// one keeps what the earlier glClear put in it.
#[test]
fn clear_bufferfv_clears_one_draw_buffer() {
    let _g = context();
    let m = Mrt::new([Fmt::Rgba8, Fmt::Rgb10A2], Kind::Tex);
    m.bind();
    clear_all(SENT);
    m.expect_bytes(Fmt::Rgba8, SENT, "the sentinel glClear reached both draw buffers");
    m.expect_bytes(Fmt::Rgb10A2, SENT, "the sentinel glClear reached both draw buffers");
    let before = clears();
    clear_one(&m, 0, CLEAR);
    m.expect_bytes(Fmt::Rgba8, CLEAR, "glClearBufferfv(GL_COLOR, 0, ..)");
    m.expect_bytes(Fmt::Rgb10A2, SENT, "draw buffer 1 kept its colour");
    assert_eq!(clears() - before, 1, "the clear went to one target");
}

/// The same with the formats swapped: draw buffer 1 now writes the RGBA8
/// attachment, and the read buffer moves to it (glReadBuffer of
/// GL_COLOR_ATTACHMENT1).
#[test]
fn clear_bufferfv_clears_one_draw_buffer_reversed() {
    let _g = context();
    let m = Mrt::new([Fmt::Rgb10A2, Fmt::Rgba8], Kind::Tex);
    m.bind();
    clear_all(SENT);
    m.expect_bytes(Fmt::Rgba8, SENT, "sentinel");
    m.expect_bytes(Fmt::Rgb10A2, SENT, "sentinel");
    let before = clears();
    clear_one(&m, 1, CLEAR);
    m.expect_bytes(Fmt::Rgba8, CLEAR, "glClearBufferfv(GL_COLOR, 1, ..) at attachment 1");
    m.expect_bytes(Fmt::Rgb10A2, SENT, "draw buffer 0 kept its colour");
    assert_eq!(clears() - before, 1, "the clear went to one target");
}

/// The same clear value in both draw buffers: alpha 0.4 stores as byte 102
/// in RGBA8 and as the 2-bit field 1 in RGB10_A2 (glReadPixels decodes that
/// to 85), while R/G/B land the same bytes (51, 102, 153) on both. The
/// GL_RGBA/GL_UNSIGNED_INT_2_10_10_10_REV read is accepted on the RGB10_A2
/// attachment read as attachment 1, and returns its word.
#[test]
fn clear_bufferfv_rounds_each_attachment_to_its_own_bits() {
    let _g = context();
    let m = Mrt::new([Fmt::Rgba8, Fmt::Rgb10A2], Kind::Tex);
    let before = clears();
    clear_one(&m, 0, CLEAR);
    clear_one(&m, 1, CLEAR);
    let (a8, a10) = (stored(Fmt::Rgba8, CLEAR)[3], stored(Fmt::Rgb10A2, CLEAR)[3]);
    assert_eq!((a8, a10), (102, 85), "alpha 0.4: 8-bit rounding vs 2-bit");
    m.expect_bytes(Fmt::Rgba8, CLEAR, "RGBA8 draw buffer 0");
    m.expect_bytes(Fmt::Rgb10A2, CLEAR, "RGB10_A2 draw buffer 1, read through glReadBuffer(ATT1)");
    m.expect_word(a2(205, 409, 614, 1), "the RGB10_A2 word, read as attachment 1");
    assert_eq!(clears() - before, 2, "both per-buffer clears reached the back end");
}

/// The same with the formats swapped; the word is read at attachment 0.
#[test]
fn clear_bufferfv_rounds_each_attachment_to_its_own_bits_reversed() {
    let _g = context();
    let m = Mrt::new([Fmt::Rgb10A2, Fmt::Rgba8], Kind::Tex);
    clear_one(&m, 0, CLEAR);
    clear_one(&m, 1, CLEAR);
    m.expect_bytes(Fmt::Rgba8, CLEAR, "RGBA8 draw buffer 1");
    m.expect_bytes(Fmt::Rgb10A2, CLEAR, "RGB10_A2 draw buffer 0");
    m.expect_word(a2(205, 409, 614, 1), "the RGB10_A2 word, read as attachment 0");
}

/// And with renderbuffer attachments.
#[test]
fn clear_bufferfv_rounds_each_renderbuffer_to_its_own_bits() {
    let _g = context();
    let m = Mrt::new([Fmt::Rgba8, Fmt::Rgb10A2], Kind::Rb);
    clear_one(&m, 0, CLEAR);
    clear_one(&m, 1, CLEAR);
    m.expect_bytes(Fmt::Rgba8, CLEAR, "RGBA8 renderbuffer");
    m.expect_bytes(Fmt::Rgb10A2, CLEAR, "RGB10_A2 renderbuffer, read through glReadBuffer(ATT1)");
    m.expect_word(a2(205, 409, 614, 1), "the RGB10_A2 renderbuffer word");
}

/// GL_MAX_DRAW_BUFFERS is 3 on this back end, so drawbuffer 3 is
/// GL_INVALID_VALUE; an in-range drawbuffer that names GL_NONE is no error
/// and clears nothing.
#[test]
fn clear_bufferfv_drawbuffer_bounds() {
    let _g = context();
    let m = Mrt::new([Fmt::Rgba8, Fmt::Rgb10A2], Kind::Tex);
    m.bind();
    clear_all(SENT);
    let c = [1.0, 1.0, 1.0, 1.0];
    // SAFETY: `c` holds four floats.
    unsafe { glClearBufferfv(GL_COLOR, 2, c.as_ptr()) };
    assert_eq!(err(), GL_NO_ERROR, "drawbuffer 2 names GL_NONE");
    m.expect_bytes(Fmt::Rgba8, SENT, "nothing was cleared");
    m.expect_bytes(Fmt::Rgb10A2, SENT, "nothing was cleared");
    // SAFETY: `c` holds four floats.
    unsafe { glClearBufferfv(GL_COLOR, 3, c.as_ptr()) };
    assert_eq!(err(), GL_INVALID_VALUE, "drawbuffer 3 >= GL_MAX_DRAW_BUFFERS");
}

/* ---- (c) glBlitFramebuffer with the read buffer on either attachment ----- */

/// The RGB10_A2 attachment's pixels: eight 10/10/10/2 words, each twice.
fn pattern10() -> Vec<u32> {
    let u = [
        (1023, 1023, 1023, 3),
        (0, 0, 0, 0),
        (1, 0, 0, 1),
        (0, 1, 0, 2),
        (0, 0, 1, 3),
        (512, 513, 2, 0),
        (341, 682, 1022, 1),
        (1022, 1, 0, 3),
    ];
    u.iter().cycle().take((N * N) as usize).map(|&(r, g, b, a)| a2(r, g, b, a)).collect()
}

fn upload10() -> Vec<u8> {
    pattern10().iter().flat_map(|w| w.to_le_bytes()).collect()
}

/// The RGBA8 attachment's pixels: eight bytes colours, each twice.
fn pattern8() -> Vec<[u8; 4]> {
    let u = [
        [0, 0, 0, 0],
        [255, 255, 255, 255],
        [255, 0, 0, 255],
        [0, 255, 0, 128],
        [0, 0, 255, 64],
        [102, 102, 102, 102],
        [1, 2, 3, 4],
        [128, 64, 32, 16],
    ];
    u.iter().cycle().take((N * N) as usize).copied().collect()
}

fn upload8() -> Vec<u8> {
    pattern8().iter().flatten().copied().collect()
}

/// A source framebuffer whose RGBA8 and RGB10_A2 attachments hold the two
/// patterns, both draw buffers named.
fn source(order: [Fmt; 2]) -> Mrt {
    let m = Mrt::new(order, Kind::Tex);
    m.upload(Fmt::Rgba8, &upload8());
    m.upload(Fmt::Rgb10A2, &upload10());
    m
}

/// A framebuffer with a single texture attachment, the blit destination.
struct Dst {
    fb: GLuint,
    tex: GLuint,
    fmt: Fmt,
}

impl Dst {
    fn new(fmt: Fmt) -> Dst {
        let (mut fb, mut tex) = (0, 0);
        // SAFETY: room for one name each; the upload is null (storage only).
        unsafe {
            glGenTextures(1, &mut tex);
            glBindTexture(GL_TEXTURE_2D, tex);
            glTexImage2D(
                GL_TEXTURE_2D,
                0,
                internal(fmt) as GLint,
                N,
                N,
                0,
                GL_RGBA,
                upload_type(fmt),
                core::ptr::null(),
            );
            glGenFramebuffers(1, &mut fb);
            glBindFramebuffer(GL_FRAMEBUFFER, fb);
            glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex, 0);
            assert_eq!(err(), GL_NO_ERROR, "destination {fmt:?}");
            assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE);
        }
        Dst { fb, tex, fmt }
    }

    fn read(&self, ty: GLenum) -> Vec<u8> {
        let mut px = vec![0x77u8; (N * N) as usize * 4];
        // SAFETY: room for N x N RGBA pixels of the type.
        unsafe {
            glBindFramebuffer(GL_READ_FRAMEBUFFER, self.fb);
            glReadBuffer(GL_COLOR_ATTACHMENT0);
            glReadPixels(0, 0, N, N, GL_RGBA, ty, px.as_mut_ptr() as *mut c_void);
        }
        assert_eq!(err(), GL_NO_ERROR, "read destination {:?}", self.fmt);
        px
    }

    fn read_words(&self) -> Vec<u32> {
        self.read(GL_UNSIGNED_INT_2_10_10_10_REV)
            .chunks(4)
            .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect()
    }
}

impl Drop for Dst {
    fn drop(&mut self) {
        // SAFETY: names this test made.
        unsafe {
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
            glDeleteFramebuffers(1, &self.fb);
            glDeleteTextures(1, &self.tex);
        }
    }
}

/// NEAREST blit of the whole N x N image of the source attachment `from`
/// (the read buffer names it) into the destination's attachment 0.
fn blit(src: &Mrt, from: Fmt, dst: &Dst) {
    // SAFETY: plain GL calls.
    unsafe {
        glBindFramebuffer(GL_READ_FRAMEBUFFER, src.fb);
        glReadBuffer(src.att(from));
        glBindFramebuffer(GL_DRAW_FRAMEBUFFER, dst.fb);
        let draw = [GL_COLOR_ATTACHMENT0];
        glDrawBuffers(1, draw.as_ptr());
        glBlitFramebuffer(0, 0, N, N, 0, 0, N, N, GL_COLOR_BUFFER_BIT, GL_NEAREST);
    }
    assert_eq!(err(), GL_NO_ERROR, "blit {from:?} at attachment {}", src.att(from));
}

/// A 10-bit (or 2-bit) field re-encoded to a byte: the CPU blit decodes the
/// source to [0, 1] floats (`decode_float`) and encodes each pixel again
/// (`encode_float`), so the byte is (v / max) * 255 rounded half up.
fn byte10(v: u32, max: u32) -> u8 {
    ((v as f32 / max as f32).clamp(0.0, 1.0) * 255.0 + 0.5) as u8
}

fn blit_to8(w: u32) -> [u8; 4] {
    [byte10(w & 0x3ff, 1023), byte10((w >> 10) & 0x3ff, 1023), byte10((w >> 20) & 0x3ff, 1023), byte10(w >> 30, 3)]
}

/// A byte re-encoded to a 10 (or 2) bit field: (v / 255) * max rounded half
/// up.
fn field8(b: u8, max: u32) -> u32 {
    ((b as f32 / 255.0).clamp(0.0, 1.0) * max as f32 + 0.5) as u32
}

fn blit_to10(p: [u8; 4]) -> u32 {
    a2(field8(p[0], 1023), field8(p[1], 1023), field8(p[2], 1023), field8(p[3], 3))
}

/// Both attachments are normalized fixed-point (class Float), so the blit is
/// accepted either way the read buffer names them. The RGB10_A2 attachment
/// (attachment 1) blitted into an RGB10_A2 destination: the words survive
/// (each field round-trips through the float decode and re-encode).
#[test]
fn blit_rgb10a2_to_rgb10a2() {
    let _g = context();
    let src = source([Fmt::Rgba8, Fmt::Rgb10A2]);
    let dst = Dst::new(Fmt::Rgb10A2);
    blit(&src, Fmt::Rgb10A2, &dst);
    let want = pattern10();
    assert_eq!(dst.read_words(), want, "the words cross the blit unchanged");
    let decoded: Vec<u8> = want.iter().flat_map(|&w| decoded_word(w)).collect();
    assert_eq!(dst.read(GL_UNSIGNED_BYTE), decoded, "the same words, read back RGBA/UNSIGNED_BYTE");
}

/// The RGB10_A2 attachment into an RGBA8 destination: every field decoded to
/// [0, 1] and rounded to a byte.
#[test]
fn blit_rgb10a2_to_rgba8() {
    let _g = context();
    let src = source([Fmt::Rgba8, Fmt::Rgb10A2]);
    let dst = Dst::new(Fmt::Rgba8);
    blit(&src, Fmt::Rgb10A2, &dst);
    let want: Vec<u8> = pattern10().iter().flat_map(|&w| blit_to8(w)).collect();
    assert_eq!(dst.read(GL_UNSIGNED_BYTE), want, "10/10/10/2 converted to bytes");
}

/// The RGBA8 attachment into an RGBA8 destination: the bytes survive.
#[test]
fn blit_rgba8_to_rgba8() {
    let _g = context();
    let src = source([Fmt::Rgba8, Fmt::Rgb10A2]);
    let dst = Dst::new(Fmt::Rgba8);
    blit(&src, Fmt::Rgba8, &dst);
    let want: Vec<u8> = pattern8().iter().flatten().copied().collect();
    assert_eq!(dst.read(GL_UNSIGNED_BYTE), want, "the bytes cross the blit unchanged");
}

/// The RGBA8 attachment into an RGB10_A2 destination: every byte decoded to
/// [0, 1] and rounded to a 10 (or 2) bit field, packed into the word.
#[test]
fn blit_rgba8_to_rgb10a2() {
    let _g = context();
    let src = source([Fmt::Rgba8, Fmt::Rgb10A2]);
    let dst = Dst::new(Fmt::Rgb10A2);
    blit(&src, Fmt::Rgba8, &dst);
    let want: Vec<u32> = pattern8().iter().map(|&p| blit_to10(p)).collect();
    assert_eq!(dst.read_words(), want, "bytes converted to 10/10/10/2 words");
    let back: Vec<u8> = want.iter().flat_map(|&w| decoded_word(w)).collect();
    assert_eq!(dst.read(GL_UNSIGNED_BYTE), back, "and read back RGBA/UNSIGNED_BYTE");
}

/// With the formats swapped in the source (RGB10_A2 at attachment 0), each
/// format is still read through its own attachment number.
#[test]
fn blit_reads_either_attachment_of_a_reversed_framebuffer() {
    let _g = context();
    let src = source([Fmt::Rgb10A2, Fmt::Rgba8]);
    let d8 = Dst::new(Fmt::Rgba8);
    blit(&src, Fmt::Rgba8, &d8); // attachment 1, where the RGBA8 image now is
    let want8: Vec<u8> = pattern8().iter().flatten().copied().collect();
    assert_eq!(d8.read(GL_UNSIGNED_BYTE), want8, "RGBA8 read as attachment 1");
    let d10 = Dst::new(Fmt::Rgb10A2);
    blit(&src, Fmt::Rgb10A2, &d10); // attachment 0
    assert_eq!(d10.read_words(), pattern10(), "RGB10_A2 read as attachment 0");
}

/* ---- (d) glClear with both draw buffers ----------------------------------- */

/// glClear clears both named draw buffers, each to its own format's bits:
/// the same alpha 0.4 becomes 102 on RGBA8 and 1 (85 read back) on
/// RGB10_A2.
#[test]
fn glclear_reaches_both_draw_buffers() {
    let _g = context();
    let m = Mrt::new([Fmt::Rgba8, Fmt::Rgb10A2], Kind::Tex);
    let before = clears();
    m.bind();
    clear_all(CLEAR);
    m.expect_bytes(Fmt::Rgba8, CLEAR, "draw buffer 0 (RGBA8)");
    m.expect_bytes(Fmt::Rgb10A2, CLEAR, "draw buffer 1 (RGB10_A2)");
    assert_eq!(stored(Fmt::Rgba8, CLEAR)[3], 102);
    assert_eq!(stored(Fmt::Rgb10A2, CLEAR)[3], 85);
    m.expect_word(word(CLEAR), "the RGB10_A2 draw buffer's word");
    assert_eq!(clears() - before, 2, "the one glClear cleared both targets");
}

/// The same with the attachments swapped (glClear is not an MRT-specific
/// path: it clears the draw buffers whichever format they hold).
#[test]
fn glclear_reaches_both_draw_buffers_reversed() {
    let _g = context();
    let m = Mrt::new([Fmt::Rgb10A2, Fmt::Rgba8], Kind::Tex);
    let before = clears();
    m.bind();
    clear_all(CLEAR);
    m.expect_bytes(Fmt::Rgba8, CLEAR, "draw buffer 1 (RGBA8)");
    m.expect_bytes(Fmt::Rgb10A2, CLEAR, "draw buffer 0 (RGB10_A2)");
    assert_eq!(clears() - before, 2, "the one glClear cleared both targets");
}

/// A draw buffer named GL_NONE takes nothing: the framebuffer's attachment 0
/// keeps what its upload put in it and only the RGB10_A2 attachment is
/// cleared.
#[test]
fn glclear_skips_a_none_draw_buffer() {
    let _g = context();
    let m = Mrt::new([Fmt::Rgba8, Fmt::Rgb10A2], Kind::Tex);
    m.draw_buffers(&[GL_NONE, GL_COLOR_ATTACHMENT1]);
    m.upload(Fmt::Rgba8, &upload8()); // what attachment 0 holds before
    let before = clears();
    m.bind();
    clear_all(CLEAR);
    m.expect_bytes(Fmt::Rgb10A2, CLEAR, "draw buffer 1 was cleared");
    let want: Vec<u8> = pattern8().iter().flatten().copied().collect();
    assert_eq!(m.read(Fmt::Rgba8, GL_UNSIGNED_BYTE), want, "draw buffer 0 (GL_NONE) kept its texels");
    assert_eq!(clears() - before, 1, "the clear went to one target");
}
