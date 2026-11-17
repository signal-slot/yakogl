// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! glClear and glClearBufferfv into GL_RGB10_A2 renderbuffer and texture
//! attachments on the V3D host back end, through the GL entry points only
//! (plan #370, rgb10a2-clear-api), read back by glReadPixels
//! GL_RGBA/GL_UNSIGNED_BYTE (each field decoded to a byte, 8428875e).
//!
//! The GL core does not touch pixels for a clear: it hands the back end a
//! clear draw (`do_clear`, `clear_rgba`, the scissor box, `clear_color_mask`).
//! The back end here is a stand-in that applies such a draw to a
//! `GLES_SURF_RGB10_A2` target (see `b_draw` for the quantisation rule), so
//! the rows pin the GL core's half: the target is the RGB10_A2 surface, the
//! clear colour and its alpha rounding to 2 bits, the scissor box, the
//! colour mask, and the decode of the stored words. The read pair
//! (GL_IMPLEMENTATION_COLOR_READ_*) and the UNSIGNED_INT_2_10_10_10_REV
//! read-back are other items' and not asserted.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_rgb10a2_clear_v3d
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
/// The RGB10_A2 clear draws the stand-in back end applied.
static CLEARS: AtomicU32 = AtomicU32::new(0);

/// The stand-in GPU: a clear draw into a `GLES_SURF_RGB10_A2` target writes
/// the clear's 10/10/10/2 word inside the draw's scissor box, for the
/// channels of `clear_color_mask` only. That is the V3D back end's job (the
/// TLB clear or the clear quad), which does not run on the host; the test
/// judges what the GL core hands it and what glReadPixels makes of the words
/// that result.
///
/// Quantisation rule: red, green and blue are `clamp(v, 0, 1) * 1023`
/// rounded half up, alpha `clamp(v, 0, 1) * 3` rounded half up (the GL core
/// has already rounded alpha to the target's 2 bits, `quantise_unorm`).
/// glReadPixels then decodes a field to a byte as (v * 255 + max / 2) / max.
unsafe extern "C" fn b_draw(d: *const backend::GlesDraw) -> i32 {
    let d = &*d;
    if d.do_clear && d.target_format == backend::GLES_SURF_RGB10_A2 && d.target_gpu != 0 {
        CLEARS.fetch_add(1, Ordering::SeqCst);
        let q = |v: f32, n: f32| (v.clamp(0.0, 1.0) * n + 0.5) as u32;
        let c = d.clear_rgba;
        let new = [q(c[0], 1023.0), q(c[1], 1023.0), q(c[2], 1023.0), q(c[3], 3.0)];
        let pitch = backend::draw_pitch(d) as usize;
        let base = d.target_gpu as *mut u32;
        let [sx, sy, sw, sh] = if d.scissor { d.scissor_rect } else { [0, 0, d.target_w as i32, d.target_h as i32] };
        for y in sy.max(0)..(sy + sh).min(d.target_h as i32) {
            for x in sx.max(0)..(sx + sw).min(d.target_w as i32) {
                let p = base.add(y as usize * pitch + x as usize);
                let w = *p;
                let old = [w & 0x3ff, w >> 10 & 0x3ff, w >> 20 & 0x3ff, w >> 30];
                let f: Vec<u32> =
                    (0..4).map(|i| if d.clear_color_mask >> i & 1 != 0 { new[i] } else { old[i] }).collect();
                *p = f[0] | f[1] << 10 | f[2] << 20 | f[3] << 30;
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
    name: c"es3-rgb10a2-clear".as_ptr() as *const c_char,
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

/// A colour attachment: a texture at attachment 0 of its own framebuffer.
struct Att {
    fb: GLuint,
    tex: GLuint,
    w: GLsizei,
    h: GLsizei,
}

impl Drop for Att {
    fn drop(&mut self) {
        // SAFETY: names this test made.
        unsafe {
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
            glDeleteFramebuffers(1, &self.fb);
            glDeleteTextures(1, &self.tex);
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
    Att { fb, tex, w, h }
}

const N: GLsizei = 4;

/// An RGB10_A2 renderbuffer or texture attachment of its own framebuffer,
/// N x N.
struct Buf {
    att: Att,
    rb: GLuint,
}

impl Drop for Buf {
    fn drop(&mut self) {
        if self.rb != 0 {
            // SAFETY: a name this test made.
            unsafe {
                glBindFramebuffer(GL_FRAMEBUFFER, 0);
                glDeleteRenderbuffers(1, &self.rb);
            }
        }
    }
}

fn texture() -> Buf {
    let zero = [0u32; (N * N) as usize];
    let att = attach(GL_RGB10_A2, N, N, GL_UNSIGNED_INT_2_10_10_10_REV, zero.as_ptr() as *const c_void);
    Buf { att, rb: 0 }
}

fn renderbuffer() -> Buf {
    let (mut fb, mut rb) = (0, 0);
    // SAFETY: room for one name each.
    unsafe {
        glGenRenderbuffers(1, &mut rb);
        glBindRenderbuffer(GL_RENDERBUFFER, rb);
        glRenderbufferStorage(GL_RENDERBUFFER, GL_RGB10_A2, N, N);
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_RENDERBUFFER, rb);
        assert_eq!(err(), GL_NO_ERROR, "renderbuffer storage");
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE);
    }
    Buf { att: Att { fb, tex: 0, w: N, h: N }, rb }
}

fn scale(v: u32, max: u32) -> u8 {
    ((v * 255 + max / 2) / max) as u8
}

fn decoded(w: u32) -> [u8; 4] {
    [scale(w & 0x3ff, 1023), scale((w >> 10) & 0x3ff, 1023), scale((w >> 20) & 0x3ff, 1023), scale(w >> 30, 3)]
}

/// glReadPixels RGBA/UNSIGNED_BYTE of the whole buffer, row 0 first.
fn read_bytes(b: &Buf) -> Vec<u8> {
    let mut px = vec![0x77u8; (N * N) as usize * 4];
    // SAFETY: room for N x N RGBA8 pixels.
    unsafe {
        glBindFramebuffer(GL_READ_FRAMEBUFFER, b.att.fb);
        glReadBuffer(GL_COLOR_ATTACHMENT0);
        glReadPixels(0, 0, b.att.w, b.att.h, GL_RGBA, GL_UNSIGNED_BYTE, px.as_mut_ptr() as *mut c_void);
    }
    assert_eq!(err(), GL_NO_ERROR, "glReadPixels");
    px
}

/// Pixel i decodes from `words[i]`.
fn expect(b: &Buf, words: &[u32], what: &str) {
    assert_eq!(words.len(), (N * N) as usize);
    let want: Vec<u8> = words.iter().flat_map(|&w| decoded(w)).collect();
    assert_eq!(read_bytes(b), want, "{what}");
}

fn uniform(b: &Buf, word: u32, what: &str) {
    expect(b, &vec![word; (N * N) as usize], what);
}

fn bind(b: &Buf) {
    // SAFETY: plain GL calls.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, b.att.fb);
        let draw = [GL_COLOR_ATTACHMENT0];
        glDrawBuffers(1, draw.as_ptr());
    }
}

fn clear_color(c: [f32; 4]) {
    // SAFETY: plain GL calls.
    unsafe {
        glClearColor(c[0], c[1], c[2], c[3]);
        glClear(GL_COLOR_BUFFER_BIT);
    }
    assert_eq!(err(), GL_NO_ERROR, "glClear");
}

fn clear_bufferfv(c: [f32; 4]) {
    // SAFETY: four floats of room.
    unsafe { glClearBufferfv(GL_COLOR, 0, c.as_ptr()) };
    assert_eq!(err(), GL_NO_ERROR, "glClearBufferfv");
}

type Clear = fn([f32; 4]);

/// Clear values on the 10-bit and 2-bit boundaries and the words they must
/// store (R/G/B: v * 1023 half up; A: v * 3 half up):
///
/// - 0.5 * 1023 = 511.5 -> 512; alpha 0.5 * 3 = 1.5 -> 2
/// - 1/1023 -> 1; alpha 1/3 -> 1 and 2/3 -> 2
/// - 0.25 * 1023 = 255.75 -> 256, 0.75 * 1023 = 767.25 -> 767, 0.999 -> 1022
/// - out of range values clamp: -1 -> 0, 2 -> 1023 (alpha 2 -> 3)
fn clear_table() -> Vec<([f32; 4], u32)> {
    vec![
        ([0.0, 0.0, 0.0, 0.0], a2(0, 0, 0, 0)),
        ([1.0, 1.0, 1.0, 1.0], a2(1023, 1023, 1023, 3)),
        ([0.5, 0.5, 0.5, 0.5], a2(512, 512, 512, 2)),
        ([1.0 / 1023.0, 1.0 / 1023.0, 1.0 / 1023.0, 1.0 / 3.0], a2(1, 1, 1, 1)),
        ([2.0 / 1023.0, 0.25, 0.75, 2.0 / 3.0], a2(2, 256, 767, 2)),
        ([-1.0, 2.0, 0.999, 1.0 / 3.0], a2(0, 1023, 1022, 1)),
        ([1.0, 0.0, 1.0 / 1023.0, 2.0], a2(1023, 0, 1, 3)),
    ]
}

fn full_clears(b: Buf, clear: Clear, what: &str) {
    bind(&b);
    for (c, word) in clear_table() {
        let before = CLEARS.load(Ordering::SeqCst);
        clear(c);
        uniform(&b, word, &format!("{what} {c:?}"));
        assert!(CLEARS.load(Ordering::SeqCst) > before, "{what} {c:?}: the clear reached the RGB10_A2 surface");
    }
}

#[test]
fn renderbuffer_glclear() {
    let _g = context();
    full_clears(renderbuffer(), clear_color, "renderbuffer glClear");
}

#[test]
fn texture_glclear() {
    let _g = context();
    full_clears(texture(), clear_color, "texture glClear");
}

#[test]
fn renderbuffer_glclearbufferfv() {
    let _g = context();
    full_clears(renderbuffer(), clear_bufferfv, "renderbuffer glClearBufferfv");
}

#[test]
fn texture_glclearbufferfv() {
    let _g = context();
    full_clears(texture(), clear_bufferfv, "texture glClearBufferfv");
}

/// glClearBufferfv takes its value from the call, not from glClearColor.
#[test]
fn glclearbufferfv_ignores_the_clear_color() {
    let _g = context();
    let b = texture();
    bind(&b);
    // SAFETY: plain GL call.
    unsafe { glClearColor(1.0, 1.0, 1.0, 1.0) };
    clear_bufferfv([0.0, 1.0, 0.0, 1.0 / 3.0]);
    uniform(&b, a2(0, 1023, 0, 1), "glClearBufferfv value");
}

/// Only the scissor box (x 1..3, y 2) changes.
fn scissored(b: Buf, clear: Clear, what: &str) {
    bind(&b);
    let base = a2(512, 512, 512, 2);
    clear([0.5, 0.5, 0.5, 0.5]);
    uniform(&b, base, what);
    // SAFETY: plain GL calls.
    unsafe {
        glEnable(GL_SCISSOR_TEST);
        glScissor(1, 2, 2, 1);
    }
    clear([1.0, 0.0, 1.0 / 1023.0, 1.0 / 3.0]);
    // SAFETY: plain GL call.
    unsafe { glDisable(GL_SCISSOR_TEST) };
    let inside = a2(1023, 0, 1, 1);
    let words: Vec<u32> = (0..N * N)
        .map(|i| {
            let (x, y) = (i % N, i / N);
            if (1..3).contains(&x) && y == 2 {
                inside
            } else {
                base
            }
        })
        .collect();
    expect(&b, &words, &format!("{what}: scissored"));
}

#[test]
fn renderbuffer_scissored_glclear() {
    let _g = context();
    scissored(renderbuffer(), clear_color, "renderbuffer glClear");
}

#[test]
fn texture_scissored_glclear() {
    let _g = context();
    scissored(texture(), clear_color, "texture glClear");
}

#[test]
fn renderbuffer_scissored_glclearbufferfv() {
    let _g = context();
    scissored(renderbuffer(), clear_bufferfv, "renderbuffer glClearBufferfv");
}

#[test]
fn texture_scissored_glclearbufferfv() {
    let _g = context();
    scissored(texture(), clear_bufferfv, "texture glClearBufferfv");
}

/// glColorMask: a masked field keeps its stored 10 (or 2) bits, the others
/// take the clear. All 16 masks.
fn masked(b: Buf, clear: Clear, what: &str) {
    bind(&b);
    let old = (100, 900, 700, 2);
    let new = (1023, 1, 341, 1);
    for mask in 0..16u32 {
        clear([100.0 / 1023.0, 900.0 / 1023.0, 700.0 / 1023.0, 2.0 / 3.0]);
        uniform(&b, a2(old.0, old.1, old.2, old.3), what);
        let on = |i: u32| (mask >> i & 1) as GLboolean;
        // SAFETY: plain GL call.
        unsafe { glColorMask(on(0), on(1), on(2), on(3)) };
        clear([1.0, 1.0 / 1023.0, 341.0 / 1023.0, 1.0 / 3.0]);
        // SAFETY: plain GL call.
        unsafe { glColorMask(1, 1, 1, 1) };
        let pick = |i: u32, o: u32, n: u32| if mask >> i & 1 != 0 { n } else { o };
        let word = a2(pick(0, old.0, new.0), pick(1, old.1, new.1), pick(2, old.2, new.2), pick(3, old.3, new.3));
        uniform(&b, word, &format!("{what} colour mask {mask:#06b}"));
    }
}

#[test]
fn renderbuffer_colormask_glclear() {
    let _g = context();
    masked(renderbuffer(), clear_color, "renderbuffer glClear");
}

#[test]
fn texture_colormask_glclear() {
    let _g = context();
    masked(texture(), clear_color, "texture glClear");
}

#[test]
fn renderbuffer_colormask_glclearbufferfv() {
    let _g = context();
    masked(renderbuffer(), clear_bufferfv, "renderbuffer glClearBufferfv");
}

#[test]
fn texture_colormask_glclearbufferfv() {
    let _g = context();
    masked(texture(), clear_bufferfv, "texture glClearBufferfv");
}
