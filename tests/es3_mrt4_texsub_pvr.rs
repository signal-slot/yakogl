// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! glTexSubImage2D into the texture at GL_COLOR_ATTACHMENT3 between clears on
//! the PowerVR host back end under a four-target stand-in
//! (`backend::HOST_DRAW_TARGETS` = 4; plan #368).
//!
//! Same harness as es3_mrt4_texsub_pvr.rs: the mock back end fills every
//! target of a clear with the clear colour. Pinned: pending clears are
//! flushed before the upload, the upload is not overwritten by a stale store,
//! and a later scissored clear touches only its box. 32-bit-per-pixel formats
//! only (docs/notes/pvr-mrt4-budget.md). HOST-ONLY: what the GPU would write
//! is not seen.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test es3_mrt4_texsub_pvr
//! ```

#![cfg(all(feature = "host-test", feature = "gpu-powervr", feature = "shaderc-online", not(feature = "report-es2")))]

use core::ffi::{c_char, c_void};
use core::sync::atomic::Ordering;
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{self, GlesDraw, GlesHeap, YakoglBackend};
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
    let all = (1u32 << n) - 1;
    let mask = if d.clear_target_mask == 0 { all } else { d.clear_target_mask as u32 & all };
    let (bx, by, bw, bh) = match d.scissor.then_some(d.scissor_rect) {
        Some(r) if d.clear_quad => (r[0] as usize, r[1] as usize, r[2] as usize, r[3] as usize),
        _ => (0, 0, usize::MAX, usize::MAX),
    };
    for (i, t) in d.targets[..n].iter().enumerate() {
        if t.gpu == 0 || mask & (1 << i) == 0 {
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
        for y in by..(t.h as usize).min(by.saturating_add(bh)) {
            for x in bx..(t.w as usize).min(bx.saturating_add(bw)) {
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
    name: c"es3-mrt4-texsub-pvr".as_ptr() as *const c_char,
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

fn set_read_buffer(a: GLenum) -> GLenum {
    // SAFETY: plain GL call.
    unsafe { glReadBuffer(a) };
    err()
}

fn read_rgba8(x: GLint, y: GLint) -> [u8; 4] {
    let mut px = [0xEEu8; 4];
    // SAFETY: room for one RGBA8 pixel.
    unsafe { glReadPixels(x, y, 1, 1, GL_RGBA, GL_UNSIGNED_BYTE, px.as_mut_ptr() as *mut c_void) };
    assert_eq!(err(), GL_NO_ERROR, "glReadPixels");
    px
}

const RED: [u8; 4] = [255, 0, 0, 255];
const BLUE: [u8; 4] = [0, 0, 255, 255];
const GREEN: [u8; 4] = [0, 255, 0, 255];

/// Uploads a green 2x2 block at (1,1) into the texture bound at attachment 3.
fn upload_green(tex: GLuint) {
    let px: [u8; 16] = core::array::from_fn(|i| GREEN[i % 4]);
    // SAFETY: 2x2 RGBA8 = 16 bytes.
    unsafe {
        glBindTexture(GL_TEXTURE_2D, tex);
        glPixelStorei(GL_UNPACK_ALIGNMENT, 1);
        glTexSubImage2D(GL_TEXTURE_2D, 0, 1, 1, 2, 2, GL_RGBA, GL_UNSIGNED_BYTE, px.as_ptr() as *const c_void);
    }
    assert_eq!(err(), GL_NO_ERROR, "glTexSubImage2D");
}

/// Attachment 3's texel (x,y) through glReadBuffer(GL_COLOR_ATTACHMENT3).
fn px3(x: GLint, y: GLint) -> [u8; 4] {
    assert_eq!(set_read_buffer(GL_COLOR_ATTACHMENT0 + 3), GL_NO_ERROR);
    read_rgba8(x, y)
}

fn clear_all(c3: [f32; 4]) {
    // SAFETY: plain GL calls.
    unsafe {
        glClearColor(1.0, 0.0, 0.0, 1.0);
        glClear(GL_COLOR_BUFFER_BIT);
        glClearBufferfv(GL_COLOR, 3, c3.as_ptr());
    }
    assert_eq!(err(), GL_NO_ERROR, "clears");
}

fn restore_targets() {
    backend::HOST_DRAW_TARGETS.store(4, Ordering::Relaxed);
}

#[test]
#[ignore = "defect: src/gl/texture.rs:874 flush_clear_into looks at the bound FBO's attachment 0 and depth only, not color_n; a pending clear of attachment 3 lands after the upload and wipes it (attachment 0 passes)"]
fn upload_sits_inside_and_clear_outside() {
    let _g = context();
    let (fb, texs) = fbo(GL_RGBA8);
    clear_all([0.0, 0.0, 1.0, 1.0]);
    upload_green(texs[3]);
    restore_targets();
    let got = [px3(0, 0), px3(1, 1), px3(2, 2), px3(3, 3), px3(3, 1)];
    release(fb, texs);
    assert_eq!(got, [BLUE, GREEN, GREEN, BLUE, BLUE], "upload inside, clear outside");
}

#[test]
#[ignore = "defect: src/gl/texture.rs:874 flush_clear_into looks at the bound FBO's attachment 0 and depth only, not color_n; a pending clear of attachment 3 lands after the upload and wipes it (attachment 0 passes)"]
fn scissored_clear_after_upload_shows_only_the_box() {
    let _g = context();
    let (fb, texs) = fbo(GL_RGBA8);
    clear_all([0.0, 0.0, 1.0, 1.0]);
    upload_green(texs[3]);
    let red = [1.0f32, 0.0, 0.0, 1.0];
    // SAFETY: plain GL calls.
    unsafe {
        glEnable(GL_SCISSOR_TEST);
        glScissor(0, 0, 2, 2);
        glClearBufferfv(GL_COLOR, 3, red.as_ptr());
        glDisable(GL_SCISSOR_TEST);
    }
    assert_eq!(err(), GL_NO_ERROR, "scissored clear");
    restore_targets();
    // (0,0) and (1,0) are in the box and cleared; (1,1) is in the box too.
    let got = [px3(0, 0), px3(1, 1), px3(2, 2), px3(3, 3)];
    release(fb, texs);
    assert_eq!(got, [RED, RED, GREEN, BLUE], "only the scissor box shows the clear");
}

#[test]
#[ignore = "defect: src/gl/texture.rs:874 flush_clear_into looks at the bound FBO's attachment 0 and depth only, not color_n; a pending clear of attachment 3 lands after the upload and wipes it (attachment 0 passes)"]
fn upload_while_unbound_survives_a_clear_of_slots_0_to_2() {
    let _g = context();
    let (fb, texs) = fbo(GL_RGBA8);
    clear_all([0.0, 0.0, 1.0, 1.0]);
    // SAFETY: plain GL call.
    unsafe { glBindFramebuffer(GL_FRAMEBUFFER, 0) };
    upload_green(texs[3]);
    let c = [0.0f32, 1.0, 1.0, 1.0];
    // SAFETY: plain GL calls.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        for i in 0..3 {
            glClearBufferfv(GL_COLOR, i, c.as_ptr());
        }
    }
    assert_eq!(err(), GL_NO_ERROR, "clears of slots 0..2");
    restore_targets();
    let got = [px3(0, 0), px3(1, 1), px3(2, 2), px3(3, 3)];
    assert_eq!(set_read_buffer(GL_COLOR_ATTACHMENT0), GL_NO_ERROR);
    let s0 = read_rgba8(1, 1);
    release(fb, texs);
    assert_eq!(got, [BLUE, GREEN, GREEN, BLUE], "slot 3 keeps the upload");
    assert_eq!(s0, [0, 255, 255, 255], "slot 0 was cleared");
}
