// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! glReadBuffer(GL_COLOR_ATTACHMENT0 + i), i = 0..3, followed by glReadPixels
//! on the PowerVR host back end under a four-target stand-in
//! (`backend::HOST_DRAW_TARGETS` = 4; plan #368).
//!
//! Each of four attachments is cleared to its own value and read back through
//! its own read buffer: RGBA8, RGBA8UI with GL_RGBA_INTEGER, the
//! GL_IMPLEMENTATION_COLOR_READ_FORMAT / _TYPE query at slot 3, and a
//! GL_PIXEL_PACK_BUFFER read. Errors: GL_NONE as the read buffer makes
//! glReadPixels GL_INVALID_OPERATION; an attachment index past the count is
//! GL_INVALID_OPERATION (ES 3.0 4.3.1). Every attachment is 32 bits a pixel,
//! inside the four-register output budget (docs/notes/pvr-mrt4-budget.md).
//! Rule: a test that sets HOST_DRAW_TARGETS builds at 4 and restores 4 before
//! asserting. HOST-ONLY: what the GPU would write is not seen.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test es3_mrt4_readbuf_pvr
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

/// Records the targets of each draw; a clear fills every target it names
/// (only those `clear_target_mask` names, when it names any)
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
    for (ti, t) in d.targets[..n].iter().enumerate() {
        if t.gpu == 0 || (d.clear_target_mask != 0 && d.clear_target_mask & (1 << ti) == 0) {
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
    name: c"es3-mrt4-readbuf-pvr".as_ptr() as *const c_char,
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

/// A bound framebuffer with an `f` texture at each of attachments 0-3, all
/// four draw buffers on.
fn fbo(f: GLenum) -> (GLuint, [GLuint; 4]) {
    let texs = [texture(f), texture(f), texture(f), texture(f)];
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
    assert_eq!(err(), GL_NO_ERROR, "framebuffer setup {f:#x}");
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

/// Slot i of an RGBA8 framebuffer holds this colour.
fn color_of(i: usize) -> [f32; 4] {
    [[1.0, 0.0, 0.0, 1.0], [0.0, 1.0, 0.0, 1.0], [0.0, 0.0, 1.0, 1.0], [1.0, 1.0, 0.0, 0.5]][i]
}

/// Slot i of an RGBA8UI framebuffer holds this value.
fn uint_of(i: usize) -> [GLuint; 4] {
    let b = i as GLuint * 10;
    [b + 1, b + 2, b + 3, b + 4]
}

fn clear_each_rgba8() {
    for i in 0..4 {
        // SAFETY: four floats.
        unsafe { glClearBufferfv(GL_COLOR, i as GLint, color_of(i).as_ptr()) };
    }
    assert_eq!(err(), GL_NO_ERROR, "glClearBufferfv x4");
}

fn clear_each_rgba8ui() {
    for i in 0..4 {
        // SAFETY: four uints.
        unsafe { glClearBufferuiv(GL_COLOR, i as GLint, uint_of(i).as_ptr()) };
    }
    assert_eq!(err(), GL_NO_ERROR, "glClearBufferuiv x4");
}

fn bytes_of(c: [f32; 4]) -> [u8; 4] {
    c.map(|v| (v * 255.0 + 0.5) as u8)
}

fn read_rgba8(x: GLint, y: GLint) -> [u8; 4] {
    let mut px = [0xEEu8; 4];
    // SAFETY: room for one RGBA8 pixel.
    unsafe { glReadPixels(x, y, 1, 1, GL_RGBA, GL_UNSIGNED_BYTE, px.as_mut_ptr() as *mut c_void) };
    assert_eq!(err(), GL_NO_ERROR, "glReadPixels");
    px
}

fn read_uint(x: GLint, y: GLint) -> [GLuint; 4] {
    let mut px = [0xEEEE_EEEEu32; 4];
    // SAFETY: room for one RGBA_INTEGER pixel.
    unsafe { glReadPixels(x, y, 1, 1, GL_RGBA_INTEGER, GL_UNSIGNED_INT, px.as_mut_ptr() as *mut c_void) };
    assert_eq!(err(), GL_NO_ERROR, "glReadPixels(GL_RGBA_INTEGER)");
    px
}

#[test]
fn each_read_buffer_reads_its_own_rgba8_attachment() {
    let _g = context();
    let (fb, texs) = fbo(GL_RGBA8);
    clear_each_rgba8();
    for i in 0..4 {
        assert_eq!(set_read_buffer(GL_COLOR_ATTACHMENT0 + i as GLenum), GL_NO_ERROR, "glReadBuffer({i})");
        assert_eq!(read_buffer(), (GL_COLOR_ATTACHMENT0 + i as GLenum) as GLint);
        assert_eq!(read_rgba8(2, 3), bytes_of(color_of(i)), "attachment {i}");
    }
    release(fb, texs);
}

#[test]
fn each_read_buffer_reads_its_own_rgba8ui_attachment() {
    let _g = context();
    let (fb, texs) = fbo(GL_RGBA8UI);
    clear_each_rgba8ui();
    for i in 0..4 {
        assert_eq!(set_read_buffer(GL_COLOR_ATTACHMENT0 + i as GLenum), GL_NO_ERROR, "glReadBuffer({i})");
        assert_eq!(read_uint(0, 1), uint_of(i), "attachment {i}");
    }
    release(fb, texs);
}

/// The implementation-chosen read format follows the read buffer: slot 3
/// RGBA8 gives RGBA / UNSIGNED_BYTE, slot 3 RGBA8UI gives RGBA_INTEGER /
/// UNSIGNED_INT, and reading with the answered pair works.
#[test]
fn implementation_read_format_at_slot_3() {
    let _g = context();
    for (f, want_f, want_t) in [(GL_RGBA8, GL_RGBA, GL_UNSIGNED_BYTE), (GL_RGBA8UI, GL_RGBA_INTEGER, GL_UNSIGNED_INT)] {
        let (fb, texs) = fbo(f);
        assert_eq!(set_read_buffer(GL_COLOR_ATTACHMENT0 + 3), GL_NO_ERROR);
        let (mut rf, mut rt) = (0, 0);
        // SAFETY: one GLint each.
        unsafe {
            glGetIntegerv(GL_IMPLEMENTATION_COLOR_READ_FORMAT, &mut rf);
            glGetIntegerv(GL_IMPLEMENTATION_COLOR_READ_TYPE, &mut rt);
        }
        assert_eq!(err(), GL_NO_ERROR, "the read-format query");
        assert_eq!((rf as GLenum, rt as GLenum), (want_f, want_t), "format {f:#x} at slot 3");
        let mut px = [0u8; 16];
        // SAFETY: room for 16 bytes, one pixel of either pair.
        unsafe { glReadPixels(0, 0, 1, 1, rf as GLenum, rt as GLenum, px.as_mut_ptr() as *mut c_void) };
        assert_eq!(err(), GL_NO_ERROR, "glReadPixels with the answered pair, {f:#x}");
        release(fb, texs);
    }
}

/// A GL_PIXEL_PACK_BUFFER read of each slot: the bytes land in the buffer.
#[test]
fn pack_buffer_read_of_each_slot() {
    let _g = context();
    let (fb, texs) = fbo(GL_RGBA8);
    clear_each_rgba8();
    let mut pbo = 0;
    // SAFETY: room for one name; a null data pointer sizes the store.
    unsafe {
        glGenBuffers(1, &mut pbo);
        glBindBuffer(GL_PIXEL_PACK_BUFFER, pbo);
        glBufferData(GL_PIXEL_PACK_BUFFER, 16, core::ptr::null(), GL_STREAM_READ);
    }
    assert_eq!(err(), GL_NO_ERROR, "pack buffer");
    for i in 0..4 {
        assert_eq!(set_read_buffer(GL_COLOR_ATTACHMENT0 + i as GLenum), GL_NO_ERROR);
        // SAFETY: the pack buffer is bound; the pointer is an offset into it.
        unsafe { glReadPixels(1, 1, 1, 1, GL_RGBA, GL_UNSIGNED_BYTE, core::ptr::null_mut()) };
        assert_eq!(err(), GL_NO_ERROR, "glReadPixels into the pack buffer, slot {i}");
        // SAFETY: maps 4 of the 16 bytes read-only; unmapped below.
        let got = unsafe {
            let p = glMapBufferRange(GL_PIXEL_PACK_BUFFER, 0, 4, GL_MAP_READ_BIT) as *const u8;
            assert!(!p.is_null(), "glMapBufferRange, err {:#x}", err());
            let v = [*p, *p.add(1), *p.add(2), *p.add(3)];
            glUnmapBuffer(GL_PIXEL_PACK_BUFFER);
            v
        };
        assert_eq!(got, bytes_of(color_of(i)), "pack buffer, slot {i}");
    }
    // SAFETY: names made by the test.
    unsafe {
        glBindBuffer(GL_PIXEL_PACK_BUFFER, 0);
        glDeleteBuffers(1, &pbo);
    }
    release(fb, texs);
}

/// glReadBuffer(GL_NONE): glReadPixels is GL_INVALID_OPERATION and the pixels
/// are untouched; a following glReadBuffer(3) reads again.
#[test]
fn read_buffer_none_makes_read_pixels_invalid_operation() {
    let _g = context();
    let (fb, texs) = fbo(GL_RGBA8);
    clear_each_rgba8();
    assert_eq!(set_read_buffer(GL_NONE), GL_NO_ERROR);
    let mut px = [0xEEu8; 4];
    // SAFETY: room for one pixel.
    unsafe { glReadPixels(0, 0, 1, 1, GL_RGBA, GL_UNSIGNED_BYTE, px.as_mut_ptr() as *mut c_void) };
    assert_eq!(err(), GL_INVALID_OPERATION, "glReadPixels with read buffer NONE");
    assert_eq!(px, [0xEE; 4], "no pixel written");
    assert_eq!(set_read_buffer(GL_COLOR_ATTACHMENT0 + 3), GL_NO_ERROR);
    assert_eq!(read_rgba8(0, 0), bytes_of(color_of(3)));
    release(fb, texs);
}

/// An attachment index at or past the count: GL_INVALID_OPERATION, the read
/// buffer stays (ES 3.0 4.3.1).
#[test]
fn out_of_range_read_buffer_is_invalid_operation() {
    let _g = context();
    let (fb, texs) = fbo(GL_RGBA8);
    assert_eq!(set_read_buffer(GL_COLOR_ATTACHMENT0 + 3), GL_NO_ERROR);
    for a in [GL_COLOR_ATTACHMENT0 + 4, GL_COLOR_ATTACHMENT0 + 15] {
        assert_eq!(set_read_buffer(a), GL_INVALID_OPERATION, "glReadBuffer({a:#x})");
        assert_eq!(read_buffer(), (GL_COLOR_ATTACHMENT0 + 3) as GLint, "read buffer stays");
    }
    /* GL_BACK names the default framebuffer only */
    assert_eq!(set_read_buffer(GL_BACK), GL_INVALID_OPERATION, "glReadBuffer(GL_BACK) on an FBO");
    release(fb, texs);
}

/// At HOST_DRAW_TARGETS 3 slot 3 is out of range; at 4 it reads again.
#[test]
fn slot_3_follows_the_target_count() {
    let _g = context();
    let (fb, texs) = fbo(GL_RGBA8);
    clear_each_rgba8();
    backend::HOST_DRAW_TARGETS.store(3, Ordering::Relaxed);
    let e = set_read_buffer(GL_COLOR_ATTACHMENT0 + 3);
    backend::HOST_DRAW_TARGETS.store(4, Ordering::Relaxed);
    assert_eq!(e, GL_INVALID_OPERATION, "glReadBuffer(3) at three targets");
    assert_eq!(set_read_buffer(GL_COLOR_ATTACHMENT0 + 3), GL_NO_ERROR);
    assert_eq!(read_rgba8(0, 0), bytes_of(color_of(3)));
    release(fb, texs);
}
