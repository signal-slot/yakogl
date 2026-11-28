// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Renderbuffer-backed GL_COLOR_ATTACHMENT3 on the PowerVR host back end
//! under a four-target stand-in (`backend::HOST_DRAW_TARGETS` = 4; plan #368).
//!
//! Renderbuffers of 32-bit-per-pixel formats only (docs/notes/pvr-mrt4-budget.md:
//! four 32-bit output registers) on slot 3, alone and beside textures on slots
//! 0..2. Pinned: completeness, a clear of slot 3 read back through
//! glReadBuffer(GL_COLOR_ATTACHMENT3), the attachment queries at slot 3,
//! deleting the renderbuffer while attached detaches it, and the answer for a
//! size mismatch (ES 3.0 4.4.4: complete). The 4-sample renderbuffer on slot 3
//! is refused at storage and is pinned by es3_mrt4_msaa_pvr.rs. HOST-ONLY: the
//! mock back end fills the targets of a clear; what the GPU writes is not seen.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test es3_mrt4_rbo_pvr
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
    name: c"es3-mrt4-rbo-pvr".as_ptr() as *const c_char,
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

const A3: GLenum = GL_COLOR_ATTACHMENT0 + 3;

/// A renderbuffer of `f`, `w` x `h`.
fn renderbuffer(f: GLenum, w: GLsizei, h: GLsizei) -> GLuint {
    let mut rb = 0;
    // SAFETY: room for one name.
    unsafe {
        glGenRenderbuffers(1, &mut rb);
        glBindRenderbuffer(GL_RENDERBUFFER, rb);
        glRenderbufferStorage(GL_RENDERBUFFER, f, w, h);
    }
    assert_eq!(err(), GL_NO_ERROR, "glRenderbufferStorage {f:#x}");
    rb
}

fn texture(f: GLenum, w: GLsizei) -> GLuint {
    let mut tex = 0;
    // SAFETY: room for one name.
    unsafe {
        glGenTextures(1, &mut tex);
        glBindTexture(GL_TEXTURE_2D, tex);
        glTexStorage2D(GL_TEXTURE_2D, 1, f, w, w);
    }
    assert_eq!(err(), GL_NO_ERROR, "glTexStorage2D");
    tex
}

/// A bound framebuffer: RGBA8 textures (`w` square) on slots 0..2 when
/// `mixed`, a renderbuffer of `f` (`rw` square) on slot 3. Draw buffers are
/// 0..3 when `mixed`, else NONE,NONE,NONE,ATTACHMENT3.
fn fbo(mixed: bool, f: GLenum, w: GLsizei, rw: GLsizei) -> (GLuint, Vec<GLuint>, GLuint) {
    let mut fb = 0;
    let mut texs = Vec::new();
    // SAFETY: room for one name.
    unsafe {
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
    }
    if mixed {
        for i in 0..3 {
            let t = texture(GL_RGBA8, w);
            // SAFETY: a 2D level 0 texture.
            unsafe { glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0 + i, GL_TEXTURE_2D, t, 0) };
            texs.push(t);
        }
    }
    let rb = renderbuffer(f, rw, rw);
    // SAFETY: names made above.
    unsafe {
        glFramebufferRenderbuffer(GL_FRAMEBUFFER, A3, GL_RENDERBUFFER, rb);
        let bufs: [GLenum; 4] = if mixed {
            core::array::from_fn(|i| GL_COLOR_ATTACHMENT0 + i as GLenum)
        } else {
            [GL_NONE, GL_NONE, GL_NONE, A3]
        };
        glDrawBuffers(4, bufs.as_ptr());
    }
    assert_eq!(err(), GL_NO_ERROR, "framebuffer setup");
    (fb, texs, rb)
}

fn release(fb: GLuint, texs: &[GLuint], rb: GLuint) {
    // SAFETY: names made by the test (deleting 0 or a deleted name is ignored).
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glDeleteFramebuffers(1, &fb);
        glDeleteTextures(texs.len() as GLsizei, texs.as_ptr());
        glDeleteRenderbuffers(1, &rb);
    }
}

fn status() -> GLenum {
    // SAFETY: plain GL call.
    unsafe { glCheckFramebufferStatus(GL_FRAMEBUFFER) }
}

fn attach(pname: GLenum) -> GLint {
    let mut v = -1;
    // SAFETY: one GLint.
    unsafe { glGetFramebufferAttachmentParameteriv(GL_FRAMEBUFFER, A3, pname, &mut v) };
    v
}

fn read_rgba8(x: GLint, y: GLint) -> [u8; 4] {
    let mut px = [0xEEu8; 4];
    // SAFETY: room for one RGBA8 pixel.
    unsafe { glReadPixels(x, y, 1, 1, GL_RGBA, GL_UNSIGNED_BYTE, px.as_mut_ptr() as *mut c_void) };
    assert_eq!(err(), GL_NO_ERROR, "glReadPixels");
    px
}

fn read_ui(x: GLint, y: GLint) -> [GLuint; 4] {
    let mut px = [0xEEEEu32; 4];
    // SAFETY: room for one RGBA32UI pixel.
    unsafe { glReadPixels(x, y, 1, 1, GL_RGBA_INTEGER, GL_UNSIGNED_INT, px.as_mut_ptr() as *mut c_void) };
    assert_eq!(err(), GL_NO_ERROR, "glReadPixels integer");
    px
}

#[test]
fn rbo_alone_on_slot_3_is_complete_and_reads_back() {
    let _g = context();
    let (fb, texs, rb) = fbo(false, GL_RGBA8, W, W);
    assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE);
    // SAFETY: plain GL calls.
    unsafe {
        let c = [0.0f32, 0.0, 1.0, 1.0];
        glClearBufferfv(GL_COLOR, 3, c.as_ptr());
        glReadBuffer(A3);
    }
    assert_eq!(err(), GL_NO_ERROR, "clear and read buffer");
    assert_eq!(read_rgba8(1, 1), [0, 0, 255, 255]);
    release(fb, &texs, rb);
}

#[test]
fn rbo_on_slot_3_beside_textures_reads_back_its_own_clear() {
    let _g = context();
    let (fb, texs, rb) = fbo(true, GL_RGBA8, W, W);
    assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE);
    // SAFETY: plain GL calls.
    unsafe {
        glClearColor(1.0, 0.0, 0.0, 1.0);
        glClear(GL_COLOR_BUFFER_BIT);
        glReadBuffer(GL_COLOR_ATTACHMENT0);
    }
    assert_eq!(read_rgba8(2, 2), [255, 0, 0, 255], "slot 0's texture");
    // The mock fills every target of a clear, so slot 0 is not asserted after
    // this one; only slot 3's own value is.
    // SAFETY: plain GL calls.
    unsafe {
        let c = [0.0f32, 1.0, 0.0, 1.0];
        glClearBufferfv(GL_COLOR, 3, c.as_ptr());
    }
    assert_eq!(err(), GL_NO_ERROR, "clears");
    assert!(SEEN.lock().unwrap().iter().all(|&n| n == 4 || n == 1), "{:?}", SEEN.lock().unwrap());
    // SAFETY: plain GL call.
    unsafe { glReadBuffer(A3) };
    assert_eq!(read_rgba8(2, 2), [0, 255, 0, 255], "slot 3's renderbuffer");
    release(fb, &texs, rb);
}

#[test]
fn every_32_bit_format_on_slot_3_is_complete() {
    let _g = context();
    for f in [
        GL_RGBA8,
        GL_RGB8,
        GL_RG8,
        GL_R8,
        GL_RGB565,
        GL_RGBA4,
        GL_RGB5_A1,
        GL_RGB10_A2,
        GL_RGBA8UI,
        GL_RGBA8I,
        GL_R32UI,
        GL_RG16UI,
    ] {
        let (fb, texs, rb) = fbo(true, f, W, W);
        assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE, "format {f:#x}");
        release(fb, &texs, rb);
    }
}

#[test]
fn integer_rbo_on_slot_3_reads_back_its_clear() {
    let _g = context();
    let (fb, texs, rb) = fbo(true, GL_RGBA8UI, W, W);
    assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE);
    // SAFETY: plain GL calls.
    unsafe {
        let c = [1u32, 2, 3, 4];
        glClearBufferuiv(GL_COLOR, 3, c.as_ptr());
        glReadBuffer(A3);
    }
    assert_eq!(err(), GL_NO_ERROR, "clear and read buffer");
    assert_eq!(read_ui(0, 0), [1, 2, 3, 4]);
    release(fb, &texs, rb);
}

#[test]
fn attachment_queries_at_slot_3() {
    let _g = context();
    let (fb, texs, rb) = fbo(true, GL_RGBA8, W, W);
    assert_eq!(attach(GL_FRAMEBUFFER_ATTACHMENT_OBJECT_TYPE), GL_RENDERBUFFER as GLint);
    assert_eq!(attach(GL_FRAMEBUFFER_ATTACHMENT_OBJECT_NAME), rb as GLint);
    assert_eq!(attach(GL_FRAMEBUFFER_ATTACHMENT_RED_SIZE), 8);
    assert_eq!(attach(GL_FRAMEBUFFER_ATTACHMENT_ALPHA_SIZE), 8);
    assert_eq!(attach(GL_FRAMEBUFFER_ATTACHMENT_COMPONENT_TYPE), GL_UNSIGNED_NORMALIZED as GLint);
    assert_eq!(attach(GL_FRAMEBUFFER_ATTACHMENT_COLOR_ENCODING), GL_LINEAR as GLint);
    assert_eq!(err(), GL_NO_ERROR, "queries");
    release(fb, &texs, rb);
    let (fb, texs, rb) = fbo(true, GL_RGBA8UI, W, W);
    assert_eq!(attach(GL_FRAMEBUFFER_ATTACHMENT_COMPONENT_TYPE), GL_UNSIGNED_INT as GLint);
    release(fb, &texs, rb);
}

#[test]
fn deleting_the_attached_renderbuffer_detaches_slot_3() {
    let _g = context();
    let (fb, texs, rb) = fbo(true, GL_RGBA8, W, W);
    // SAFETY: a name made by the test.
    unsafe { glDeleteRenderbuffers(1, &rb) };
    assert_eq!(err(), GL_NO_ERROR);
    assert_eq!(attach(GL_FRAMEBUFFER_ATTACHMENT_OBJECT_TYPE), GL_NONE as GLint, "detached");
    assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE, "slots 0..2 still complete the framebuffer");
    release(fb, &texs, 0);
}

#[test]
fn deleting_the_only_attachment_makes_it_incomplete() {
    let _g = context();
    let (fb, texs, rb) = fbo(false, GL_RGBA8, W, W);
    // SAFETY: a name made by the test.
    unsafe { glDeleteRenderbuffers(1, &rb) };
    assert_eq!(status(), GL_FRAMEBUFFER_INCOMPLETE_MISSING_ATTACHMENT);
    release(fb, &texs, 0);
}

/// ES 3.0 4.4.4 does not require equal attachment sizes: a 2x2 renderbuffer
/// on slot 3 beside 4x4 textures is complete.
#[test]
fn size_mismatch_is_complete() {
    let _g = context();
    let (fb, texs, rb) = fbo(true, GL_RGBA8, W, 2);
    assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE);
    // SAFETY: plain GL calls.
    unsafe {
        let c = [1.0f32, 1.0, 0.0, 1.0];
        glClearBufferfv(GL_COLOR, 3, c.as_ptr());
        glReadBuffer(A3);
    }
    assert_eq!(err(), GL_NO_ERROR);
    assert_eq!(read_rgba8(1, 1), [255, 255, 0, 255]);
    release(fb, &texs, rb);
}
