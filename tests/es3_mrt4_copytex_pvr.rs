// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! glReadBuffer(GL_COLOR_ATTACHMENT3) followed by glCopyTexImage2D,
//! glCopyTexSubImage2D and glCopyTexSubImage3D on the PowerVR host back end
//! under a four-target stand-in (`backend::HOST_DRAW_TARGETS` = 4; plan #368).
//!
//! Same harness as es3_readback4_pvr.rs: the mock back end fills every target
//! of a clear with the clear colour, so the copied texels are real and are
//! read back through a scratch framebuffer. Pinned: the copies take
//! attachment 3 (not attachment 0), an integer source copied into a float
//! level is GL_INVALID_OPERATION, and at HOST_DRAW_TARGETS 3 the read buffer
//! GL_COLOR_ATTACHMENT3 itself is GL_INVALID_OPERATION. HOST-ONLY: what the
//! GPU would write is not seen.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test es3_mrt4_copytex_pvr
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
    name: c"es3-mrt4-copytex-pvr".as_ptr() as *const c_char,
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

fn read_rgba8(x: GLint, y: GLint) -> [u8; 4] {
    let mut px = [0xEEu8; 4];
    // SAFETY: room for one RGBA8 pixel.
    unsafe { glReadPixels(x, y, 1, 1, GL_RGBA, GL_UNSIGNED_BYTE, px.as_mut_ptr() as *mut c_void) };
    assert_eq!(err(), GL_NO_ERROR, "glReadPixels");
    px
}

/// Attachments 0-2 cleared to red, attachment 3 to `c3` (RGBA8) or `u3`
/// (an integer format); the read buffer is attachment 3.
fn fill(f3: GLenum, c3: [f32; 4], u3: [GLuint; 4]) -> (GLuint, [GLuint; 4]) {
    let (fb, texs) = fbo(f3);
    // SAFETY: plain GL calls.
    unsafe {
        glClearColor(1.0, 0.0, 0.0, 1.0);
        glClear(GL_COLOR_BUFFER_BIT);
        if f3 == GL_RGBA8 {
            glClearBufferfv(GL_COLOR, 3, c3.as_ptr());
        } else {
            glClearBufferuiv(GL_COLOR, 3, u3.as_ptr());
        }
    }
    assert_eq!(err(), GL_NO_ERROR, "clears");
    assert_eq!(set_read_buffer(GL_COLOR_ATTACHMENT0 + 3), GL_NO_ERROR);
    (fb, texs)
}

/// Texel (1,1) of `tex` (layer `layer` for a 3D one) through a scratch
/// framebuffer bound as READ; the read framebuffer is restored afterwards.
fn read_back(tex: GLuint, target: GLenum, layer: GLint) -> [u8; 4] {
    let mut fb2 = 0;
    let mut prev = 0;
    // SAFETY: room for one name; plain GL calls.
    unsafe {
        glGetIntegerv(GL_READ_FRAMEBUFFER_BINDING, &mut prev);
        glGenFramebuffers(1, &mut fb2);
        glBindFramebuffer(GL_READ_FRAMEBUFFER, fb2);
        if target == GL_TEXTURE_2D {
            glFramebufferTexture2D(GL_READ_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex, 0);
        } else {
            glFramebufferTextureLayer(GL_READ_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, tex, 0, layer);
        }
    }
    assert_eq!(err(), GL_NO_ERROR, "scratch framebuffer");
    let px = read_rgba8(1, 1);
    // SAFETY: names made above.
    unsafe {
        glBindFramebuffer(GL_READ_FRAMEBUFFER, prev as GLuint);
        glDeleteFramebuffers(1, &fb2);
    }
    px
}

#[test]
fn copy_tex_image_2d_takes_attachment_3() {
    let _g = context();
    let (fb, texs) = fill(GL_RGBA8, [0.0, 0.0, 1.0, 1.0], [0; 4]);
    let mut dst = 0;
    // SAFETY: plain GL calls.
    unsafe {
        glGenTextures(1, &mut dst);
        glBindTexture(GL_TEXTURE_2D, dst);
        glCopyTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA8, 0, 0, W, W, 0);
    }
    assert_eq!(err(), GL_NO_ERROR, "glCopyTexImage2D from attachment 3");
    assert_eq!(read_back(dst, GL_TEXTURE_2D, 0), [0, 0, 255, 255], "attachment 3's texel, not 0's");
    // SAFETY: names made by the test.
    unsafe { glDeleteTextures(1, &dst) };
    release(fb, texs);
}

#[test]
fn copy_tex_sub_image_2d_takes_attachment_3() {
    let _g = context();
    let (fb, texs) = fill(GL_RGBA8, [0.0, 1.0, 0.0, 1.0], [0; 4]);
    let dst = texture(GL_RGBA8);
    // SAFETY: plain GL call; texture() left dst bound.
    unsafe { glCopyTexSubImage2D(GL_TEXTURE_2D, 0, 0, 0, 0, 0, W, W) };
    assert_eq!(err(), GL_NO_ERROR, "glCopyTexSubImage2D from attachment 3");
    assert_eq!(read_back(dst, GL_TEXTURE_2D, 0), [0, 255, 0, 255]);
    // SAFETY: names made by the test.
    unsafe { glDeleteTextures(1, &dst) };
    release(fb, texs);
}

#[test]
fn copy_tex_sub_image_3d_takes_attachment_3() {
    let _g = context();
    let (fb, texs) = fill(GL_RGBA8, [1.0, 1.0, 0.0, 1.0], [0; 4]);
    let mut dst = 0;
    // SAFETY: plain GL calls.
    unsafe {
        glGenTextures(1, &mut dst);
        glBindTexture(GL_TEXTURE_3D, dst);
        glTexStorage3D(GL_TEXTURE_3D, 1, GL_RGBA8, W, W, 2);
        glCopyTexSubImage3D(GL_TEXTURE_3D, 0, 0, 0, 1, 0, 0, W, W);
    }
    assert_eq!(err(), GL_NO_ERROR, "glCopyTexSubImage3D from attachment 3");
    assert_eq!(read_back(dst, GL_TEXTURE_3D, 1), [255, 255, 0, 255], "slice 1 holds attachment 3");
    // SAFETY: names made by the test.
    unsafe { glDeleteTextures(1, &dst) };
    release(fb, texs);
}

/// An integer attachment 3 into a float level: GL_INVALID_OPERATION (ES 3.0
/// 3.8.5) for glCopyTexImage2D, glCopyTexSubImage2D and glCopyTexSubImage3D.
#[test]
fn integer_source_into_a_float_level_is_invalid_operation() {
    let _g = context();
    let (fb, texs) = fill(GL_RGBA8UI, [0.0; 4], [1, 2, 3, 4]);
    let dst = texture(GL_RGBA8);
    // SAFETY: plain GL calls.
    unsafe { glCopyTexSubImage2D(GL_TEXTURE_2D, 0, 0, 0, 0, 0, W, W) };
    assert_eq!(err(), GL_INVALID_OPERATION, "glCopyTexSubImage2D, RGBA8UI into RGBA8");
    // SAFETY: plain GL calls.
    unsafe { glCopyTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA8, 0, 0, W, W, 0) };
    assert_eq!(err(), GL_INVALID_OPERATION, "glCopyTexImage2D, RGBA8UI into RGBA8");
    let mut d3 = 0;
    // SAFETY: plain GL calls.
    unsafe {
        glGenTextures(1, &mut d3);
        glBindTexture(GL_TEXTURE_3D, d3);
        glTexStorage3D(GL_TEXTURE_3D, 1, GL_RGBA8, W, W, 2);
        glCopyTexSubImage3D(GL_TEXTURE_3D, 0, 0, 0, 0, 0, 0, W, W);
    }
    assert_eq!(err(), GL_INVALID_OPERATION, "glCopyTexSubImage3D, RGBA8UI into RGBA8");
    // SAFETY: names made by the test.
    unsafe {
        glDeleteTextures(1, &dst);
        glDeleteTextures(1, &d3);
    }
    release(fb, texs);
}

/// An integer source into an integer level of the same signedness copies
/// without error (ES 3.0 3.8.5). DEFECT: es3_copy_compatible refuses every
/// Int/Uint destination class, so this answers GL_INVALID_OPERATION.
#[test]
#[ignore = "defect: src/gl/texture.rs:6482 es3_copy_compatible refuses Int/Uint destinations; ES 3.0 3.8.5 allows a matching integer copy"]
fn integer_source_into_an_integer_level_is_accepted() {
    let _g = context();
    let (fb, texs) = fill(GL_RGBA8UI, [0.0; 4], [1, 2, 3, 4]);
    let dst = texture(GL_RGBA8UI);
    // SAFETY: plain GL call.
    unsafe { glCopyTexSubImage2D(GL_TEXTURE_2D, 0, 0, 0, 0, 0, W, W) };
    assert_eq!(err(), GL_NO_ERROR, "glCopyTexSubImage2D, RGBA8UI into RGBA8UI");
    // SAFETY: names made by the test.
    unsafe { glDeleteTextures(1, &dst) };
    release(fb, texs);
}

/// At HOST_DRAW_TARGETS 3 there is no attachment 3: glReadBuffer refuses it
/// and the read buffer does not move.
#[test]
fn at_three_targets_attachment_3_is_not_a_read_buffer() {
    let _g = context();
    let (fb, texs) = fbo(GL_RGBA8);
    backend::HOST_DRAW_TARGETS.store(3, Ordering::Relaxed);
    let e = set_read_buffer(GL_COLOR_ATTACHMENT0 + 3);
    let rb = read_buffer();
    backend::HOST_DRAW_TARGETS.store(4, Ordering::Relaxed);
    release(fb, texs);
    assert_eq!(e, GL_INVALID_OPERATION, "glReadBuffer(GL_COLOR_ATTACHMENT3) at 3 targets");
    assert_ne!(rb, (GL_COLOR_ATTACHMENT0 + 3) as GLint, "the read buffer did not move");
}
