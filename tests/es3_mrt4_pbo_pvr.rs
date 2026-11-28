// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! glReadPixels from GL_COLOR_ATTACHMENT3 into a GL_PIXEL_PACK_BUFFER on the
//! PowerVR host back end under a four-target stand-in
//! (`backend::HOST_DRAW_TARGETS` = 4; plan #368, item pbo4-pvr).
//!
//! Same harness as es3_mrt4_copytex_pvr.rs: the mock back end fills every
//! target of a clear with the clear colour, so the mapped buffer holds real
//! texels. Pinned: a read at a byte offset, a subrectangle read, GL_PACK_ROW_LENGTH
//! and GL_PACK_SKIP_PIXELS, an R32UI attachment 3 read as RGBA_INTEGER, and
//! switching the read buffer back to attachment 0. The attachments stay inside
//! PowerVR's four 32-bit output registers (docs/notes/pvr-mrt4-budget.md):
//! RGBA8 x4, or RGBA8 x3 + R32UI. HOST-ONLY: what the GPU would write is not seen.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test es3_mrt4_pbo_pvr
//! ```

#![cfg(all(feature = "host-test", feature = "gpu-powervr", feature = "shaderc-online", not(feature = "report-es2")))]

use core::ffi::{c_char, c_void};
use core::sync::atomic::Ordering;
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{self, GlesDraw, GlesHeap, YakoglBackend};
use yakogl::gl::buffer::*;
use yakogl::gl::consts::*;
use yakogl::gl::es3::*;
use yakogl::gl::framebuffer::*;
use yakogl::gl::state::*;
use yakogl::gl::texture::*;
use yakogl::gl::types::*;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};

/* ---- a context on the host: the smallest OSAL and back end that make one -- */

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

/// The number of targets of each draw (or clear) the back end was handed.
static SEEN: Mutex<Vec<usize>> = Mutex::new(Vec::new());

/// Records the targets of each draw; a clear fills the targets it names
/// with the clear colour in the target's layout (unorm bytes for a colour
/// surface, the raw 32-bit values for an integer one), rows at the
/// pair-padded pitch.
unsafe extern "C" fn b_draw(d: *const GlesDraw) -> i32 {
    let d = &*d;
    let n = d.num_targets.max(1) as usize;
    SEEN.lock().unwrap().push(n);
    if !d.do_clear {
        return 0;
    }
    for (k, t) in d.targets[..n].iter().enumerate() {
        // glClearBuffer*(GL_COLOR, k): only that target takes the clear.
        if t.gpu == 0 || (d.clear_target_mask != 0 && d.clear_target_mask & (1 << k) == 0) {
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
    name: c"es3-mrt4-pbo-pvr".as_ptr() as *const c_char,
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
    max_samples: 0,
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
            yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
            backend::HOST_DRAW_TARGETS.store(4, Ordering::Relaxed);
            assert!(!yakogl::gl::context::gl_context_create(8, 8).is_null());
        }
    });
    // SAFETY: plain GL calls.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glFinish();
    }
    SEEN.lock().unwrap().clear();
    // SAFETY: plain GL call.
    while unsafe { glGetError() } != GL_NO_ERROR {}
    g
}

fn err() -> GLenum {
    // SAFETY: plain GL call.
    unsafe { glGetError() }
}

const W: GLsizei = 4;

/// A `W` x `W` 2D texture of `f`.
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

/// A bound framebuffer with RGBA8 textures at attachments 0-2 and `f3` at
/// attachment 3, all four draw buffers on. Returns the framebuffer and its
/// textures.
fn fbo(f3: GLenum) -> (GLuint, [GLuint; 4]) {
    let texs = [texture(GL_RGBA8), texture(GL_RGBA8), texture(GL_RGBA8), texture(f3)];
    let mut fb = 0;
    // SAFETY: room for one name; the textures are 2D level 0.
    unsafe {
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        for (i, t) in texs.iter().enumerate() {
            glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0 + i as GLenum, GL_TEXTURE_2D, *t, 0);
        }
        let bufs: [GLenum; 4] = core::array::from_fn(|i| GL_COLOR_ATTACHMENT0 + i as GLenum);
        glDrawBuffers(4, bufs.as_ptr());
    }
    assert_eq!(err(), GL_NO_ERROR, "framebuffer setup");
    (fb, texs)
}

fn release(fb: GLuint, texs: [GLuint; 4]) {
    // SAFETY: names made by the test.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glDeleteFramebuffers(1, &fb);
        glDeleteTextures(4, texs.as_ptr());
    }
}

fn read_buffer() -> GLint {
    let mut v = -1;
    // SAFETY: one GLint.
    unsafe { glGetIntegerv(GL_READ_BUFFER, &mut v) };
    v
}

/// glReadBuffer(a) and its error.
fn set_read_buffer(a: GLenum) -> GLenum {
    // SAFETY: plain GL call.
    unsafe { glReadBuffer(a) };
    err()
}

/// Four distinct clear colours as RGBA8 bytes (v * 255 + 0.5 truncated).
const CLEAR: [[f32; 4]; 4] = [[1.0, 0.0, 0.0, 1.0], [0.0, 1.0, 0.0, 1.0], [0.0, 0.0, 1.0, 1.0], [0.2, 0.4, 0.6, 0.8]];
const BYTES: [[u8; 4]; 4] = [[255, 0, 0, 255], [0, 255, 0, 255], [0, 0, 255, 255], [51, 102, 153, 204]];
const SENTINEL: u8 = 0xEE;

/// A bound framebuffer with RGBA8 at 0-2 and `f3` at 3, all four draw
/// buffers on; attachments 0-2 and (RGBA8) 3 cleared to distinct colours,
/// an integer 3 to `u3`. The read buffer is left at attachment 3.
fn setup(f3: GLenum, u3: [GLuint; 4]) -> (GLuint, [GLuint; 4]) {
    let (fb, texs) = fbo(f3);
    // SAFETY: plain GL calls; the arrays hold four values.
    unsafe {
        for (i, c) in CLEAR.iter().enumerate() {
            if i == 3 && f3 != GL_RGBA8 {
                glClearBufferuiv(GL_COLOR, 3, u3.as_ptr());
            } else {
                glClearBufferfv(GL_COLOR, i as GLint, c.as_ptr());
            }
            glFinish();
        }
    }
    assert_eq!(err(), GL_NO_ERROR, "clears");
    assert_eq!(set_read_buffer(GL_COLOR_ATTACHMENT0 + 3), GL_NO_ERROR);
    (fb, texs)
}

/// A pack buffer of `size` bytes filled with SENTINEL, left bound.
fn pbo(size: usize) -> GLuint {
    let mut b = 0;
    let fill = vec![SENTINEL; size];
    // SAFETY: room for one name; `fill` holds `size` bytes.
    unsafe {
        glGenBuffers(1, &mut b);
        glBindBuffer(GL_PIXEL_PACK_BUFFER, b);
        glBufferData(GL_PIXEL_PACK_BUFFER, size as isize, fill.as_ptr() as *const c_void, GL_STREAM_READ);
    }
    assert_eq!(err(), GL_NO_ERROR, "pack buffer");
    b
}

/// The whole bound pack buffer, mapped and copied.
fn map_all(size: usize) -> Vec<u8> {
    // SAFETY: the buffer holds `size` bytes; it is unmapped after the copy.
    let v = unsafe {
        let p = glMapBufferRange(GL_PIXEL_PACK_BUFFER, 0, size as isize, GL_MAP_READ_BIT) as *const u8;
        assert!(!p.is_null(), "glMapBufferRange failed, error {:#x}", err());
        let v = core::slice::from_raw_parts(p, size).to_vec();
        assert_eq!(glUnmapBuffer(GL_PIXEL_PACK_BUFFER) as u32, GL_TRUE);
        v
    };
    assert_eq!(err(), GL_NO_ERROR, "map");
    v
}

fn drop_pbo(b: GLuint) {
    // SAFETY: a name made by the test.
    unsafe {
        glBindBuffer(GL_PIXEL_PACK_BUFFER, 0);
        glDeleteBuffers(1, &b);
    }
}

fn read_into_pbo(x: GLint, y: GLint, w: GLsizei, h: GLsizei, fmt: GLenum, ty: GLenum, offset: usize) {
    // SAFETY: with a pack buffer bound the pointer is a byte offset into it.
    unsafe { glReadPixels(x, y, w, h, fmt, ty, offset as *mut c_void) };
    assert_eq!(err(), GL_NO_ERROR, "glReadPixels into the pack buffer");
}

fn assert_px(buf: &[u8], at: usize, want: [u8; 4], what: &str) {
    assert_eq!(&buf[at..at + 4], &want, "{what} (byte {at})");
}

/// A 1x1 read at a byte offset lands there and only there.
#[test]
fn read_at_an_offset_holds_attachment_3() {
    let _g = context();
    let (fb, texs) = setup(GL_RGBA8, [0; 4]);
    let b = pbo(64);
    read_into_pbo(0, 0, 1, 1, GL_RGBA, GL_UNSIGNED_BYTE, 16);
    let m = map_all(64);
    assert_px(&m, 16, BYTES[3], "attachment 3's colour at the offset");
    assert!(m[..16].iter().chain(&m[20..]).all(|&b| b == SENTINEL), "bytes outside the pixel stay untouched");
    drop_pbo(b);
    release(fb, texs);
}

/// A whole-framebuffer read fills W*W pixels with attachment 3's colour.
#[test]
fn full_read_holds_attachment_3_everywhere() {
    let _g = context();
    let (fb, texs) = setup(GL_RGBA8, [0; 4]);
    let n = (W * W) as usize * 4;
    let b = pbo(n + 8);
    read_into_pbo(0, 0, W, W, GL_RGBA, GL_UNSIGNED_BYTE, 8);
    let m = map_all(n + 8);
    for i in 0..(W * W) as usize {
        assert_px(&m, 8 + i * 4, BYTES[3], "pixel");
    }
    assert!(m[..8].iter().all(|&b| b == SENTINEL));
    drop_pbo(b);
    release(fb, texs);
}

/// A 2x2 subrectangle at (1,1) writes exactly 4 pixels, tightly packed.
#[test]
fn subrectangle_read() {
    let _g = context();
    let (fb, texs) = setup(GL_RGBA8, [0; 4]);
    let b = pbo(48);
    read_into_pbo(1, 1, 2, 2, GL_RGBA, GL_UNSIGNED_BYTE, 4);
    let m = map_all(48);
    for i in 0..4 {
        assert_px(&m, 4 + i * 4, BYTES[3], "subrectangle pixel");
    }
    assert!(m[..4].iter().chain(&m[20..]).all(|&b| b == SENTINEL), "nothing outside the 2x2 block");
    drop_pbo(b);
    release(fb, texs);
}

/// GL_PACK_ROW_LENGTH 6 with GL_PACK_SKIP_PIXELS 1 and GL_PACK_SKIP_ROWS 1: a
/// 3x2 read starts at pixel (1,1) of a 6-pixel-wide destination.
#[test]
fn pack_row_length_and_skip_pixels() {
    let _g = context();
    let (fb, texs) = setup(GL_RGBA8, [0; 4]);
    let size = 6 * 4 * 4;
    let b = pbo(size);
    // SAFETY: plain GL calls.
    unsafe {
        glPixelStorei(GL_PACK_ROW_LENGTH, 6);
        glPixelStorei(GL_PACK_SKIP_PIXELS, 1);
        glPixelStorei(GL_PACK_SKIP_ROWS, 1);
    }
    read_into_pbo(0, 0, 3, 2, GL_RGBA, GL_UNSIGNED_BYTE, 0);
    // SAFETY: plain GL calls; restore the defaults before any assert.
    unsafe {
        glPixelStorei(GL_PACK_ROW_LENGTH, 0);
        glPixelStorei(GL_PACK_SKIP_PIXELS, 0);
        glPixelStorei(GL_PACK_SKIP_ROWS, 0);
    }
    let m = map_all(size);
    for row in 0..4usize {
        for col in 0..6usize {
            let inside = (1..=2).contains(&row) && (1..=3).contains(&col);
            for k in 0..4 {
                let got = m[(row * 6 + col) * 4 + k];
                let want = if inside { BYTES[3][k] } else { SENTINEL };
                assert_eq!(got, want, "row {row} col {col} byte {k}");
            }
        }
    }
    drop_pbo(b);
    release(fb, texs);
}

/// An R32UI attachment 3 (beside three RGBA8: four registers) read as
/// RGBA_INTEGER / UNSIGNED_INT into a pack buffer.
#[test]
fn r32ui_attachment_3_reads_as_rgba_integer() {
    let _g = context();
    let (fb, texs) = setup(GL_R32UI, [0xDEAD_BEEF, 0, 0, 1]);
    let b = pbo(64);
    read_into_pbo(0, 0, 2, 1, GL_RGBA_INTEGER, GL_UNSIGNED_INT, 0);
    let m = map_all(64);
    let u = |i: usize| u32::from_le_bytes(m[i * 4..i * 4 + 4].try_into().unwrap());
    for px in 0..2 {
        assert_eq!([u(px * 4), u(px * 4 + 1), u(px * 4 + 2), u(px * 4 + 3)], [0xDEAD_BEEF, 0, 0, 1], "pixel {px}");
    }
    assert!(m[32..].iter().all(|&b| b == SENTINEL));
    drop_pbo(b);
    release(fb, texs);
}

/// A float read of the integer attachment is GL_INVALID_OPERATION and leaves
/// the pack buffer alone.
#[test]
fn r32ui_attachment_3_float_read_is_invalid_operation() {
    let _g = context();
    let (fb, texs) = setup(GL_R32UI, [7, 0, 0, 1]);
    let b = pbo(16);
    // SAFETY: byte offset 0 into the bound pack buffer.
    unsafe { glReadPixels(0, 0, 1, 1, GL_RGBA, GL_UNSIGNED_BYTE, core::ptr::null_mut()) };
    assert_eq!(err(), GL_INVALID_OPERATION);
    assert!(map_all(16).iter().all(|&b| b == SENTINEL));
    drop_pbo(b);
    release(fb, texs);
}

/// Attachment 3, then 0, then 3 again, each into its own pack buffer offset.
#[test]
fn switching_the_read_buffer_back_to_attachment_0() {
    let _g = context();
    let (fb, texs) = setup(GL_RGBA8, [0; 4]);
    let b = pbo(16);
    read_into_pbo(0, 0, 1, 1, GL_RGBA, GL_UNSIGNED_BYTE, 0);
    assert_eq!(set_read_buffer(GL_COLOR_ATTACHMENT0), GL_NO_ERROR);
    assert_eq!(read_buffer(), GL_COLOR_ATTACHMENT0 as GLint);
    read_into_pbo(0, 0, 1, 1, GL_RGBA, GL_UNSIGNED_BYTE, 4);
    assert_eq!(set_read_buffer(GL_COLOR_ATTACHMENT0 + 3), GL_NO_ERROR);
    assert_eq!(read_buffer(), (GL_COLOR_ATTACHMENT0 + 3) as GLint);
    read_into_pbo(0, 0, 1, 1, GL_RGBA, GL_UNSIGNED_BYTE, 8);
    let m = map_all(16);
    assert_px(&m, 0, BYTES[3], "first read, attachment 3");
    assert_px(&m, 4, BYTES[0], "second read, attachment 0");
    assert_px(&m, 8, BYTES[3], "third read, attachment 3 again");
    assert!(m[12..].iter().all(|&b| b == SENTINEL));
    drop_pbo(b);
    release(fb, texs);
}

/// Attachments 1 and 2 reach the same pack buffer with their own colours, so
/// the read buffer selects the source and not just attachment 3 is special.
#[test]
fn attachments_1_and_2_read_their_own_colours() {
    let _g = context();
    let (fb, texs) = setup(GL_RGBA8, [0; 4]);
    let b = pbo(16);
    for (i, off) in [(1usize, 0usize), (2, 4)] {
        assert_eq!(set_read_buffer(GL_COLOR_ATTACHMENT0 + i as GLenum), GL_NO_ERROR);
        read_into_pbo(0, 0, 1, 1, GL_RGBA, GL_UNSIGNED_BYTE, off);
    }
    let m = map_all(16);
    assert_px(&m, 0, BYTES[1], "attachment 1");
    assert_px(&m, 4, BYTES[2], "attachment 2");
    drop_pbo(b);
    release(fb, texs);
}
