// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! ES 3.0 glReadBuffer on a 4x read framebuffer, on the host
//! (docs/notes/es3-msaa-renderbuffers.md (a), (b)).
//!
//!
//! ES 3.0 4.3.3: a blit reads the read framebuffer's GL_READ_BUFFER
//! attachment and writes every enabled draw buffer of the draw framebuffer.
//! The stand-in back end is that of tests/es3_msaa_gl.rs (`context_ms4`,
//! helpers copied); its job-end resolve fills each 4x target a clear touched
//! with that clear's colour (a clear names its target in `clear_target_mask`),
//! and not before the job is finished.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_msaa_gl_readbuf
//! ```

#![cfg(all(feature = "host-test", feature = "shaderc-online", not(feature = "report-es2")))]

use core::ffi::{c_char, c_void};
use std::sync::atomic::Ordering;
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{self, GlesDraw, GlesHeap, YakoglBackend};
use yakogl::gl::consts::*;
use yakogl::gl::es3::glClearBufferfv;
use yakogl::gl::framebuffer::*;
use yakogl::gl::program::*;
use yakogl::gl::state::*;
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

/// A 4x target a clear touched: (`gpu`, w, h, colour).
type Cleared = (u64, u32, u32, [u8; 4]);

/// The 4x targets a clear touched since the last `finish`.
static PENDING: Mutex<Vec<Cleared>> = Mutex::new(Vec::new());

fn bytes(c: [f32; 4]) -> [u8; 4] {
    c.map(|v| (v * 255.0 + 0.5) as u8)
}

unsafe extern "C" fn b_draw(d: *const GlesDraw) -> i32 {
    let d = &*d;
    if d.samples > 1 && d.index_count == 0 {
        let mask = if d.clear_target_mask != 0 { d.clear_target_mask } else { 1 };
        let mut p = PENDING.lock().unwrap();
        for k in 0..(d.num_targets.max(1) as usize).min(8) {
            if mask & (1 << k) == 0 {
                continue;
            }
            let t = if k == 0 && d.targets[0].gpu == 0 {
                (d.target_gpu, d.target_w, d.target_h)
            } else {
                (d.targets[k].gpu, d.targets[k].w, d.targets[k].h)
            };
            p.push((t.0, t.1, t.2, bytes(d.clear_rgba)));
        }
    }
    0
}

/// Note (b)7 in the stand-in: when the jobs are complete, the memory of every
/// 4x target cleared holds the colour of its last clear, and not before (a
/// copy that did not wait reads the zeros the storage was made with).
unsafe extern "C" fn b_finish_resolve() -> i32 {
    for (gpu, w, h, c) in core::mem::take(&mut *PENDING.lock().unwrap()) {
        /* the stand-in OSAL's gpu address is the CPU pointer; RGBA8 rows of w*4 */
        let px = core::slice::from_raw_parts_mut(gpu as *mut [u8; 4], (w * h) as usize);
        px.fill(c);
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
    name: c"es3-msaa-gl-readbuf".as_ptr() as *const c_char,
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

/// [`BACKEND`] with the multisample capability on: it keeps 4 samples.
static BACKEND_MS4: YakoglBackend = YakoglBackend { max_samples: 4, finish: Some(b_finish_resolve), ..BACKEND };

/// One context for the whole file; the tests take turns on it.
static LOCK: Mutex<()> = Mutex::new(());
static ONCE: Once = Once::new();

/// The context with `table` as the back end (a test that swapped in the 4x
/// one does not leak it into the next).
fn context_on(table: &'static YakoglBackend) -> MutexGuard<'static, ()> {
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
    // SAFETY: a static table; the lock above serialises the tests. Plain GL calls.
    unsafe {
        backend::g_backend = table;
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glFinish();
    }
    PENDING.lock().unwrap().clear();
    // SAFETY: plain GL call.
    while unsafe { glGetError() } != GL_NO_ERROR {}
    g
}

/// The context with a stand-in back end that says `max_samples` 4.
fn context_ms4() -> MutexGuard<'static, ()> {
    context_on(&BACKEND_MS4)
}

fn err() -> GLenum {
    // SAFETY: plain GL call.
    unsafe { glGetError() }
}

struct Fbo {
    fb: GLuint,
    rbs: Vec<GLuint>,
}

impl Drop for Fbo {
    fn drop(&mut self) {
        // SAFETY: names this test made.
        unsafe {
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
            glDeleteFramebuffers(1, &self.fb);
            glDeleteRenderbuffers(self.rbs.len() as GLsizei, self.rbs.as_ptr());
            glUseProgram(0);
        }
    }
}

fn storage(samples: GLsizei, format: GLenum) -> (GLenum, GLuint) {
    let mut rb = 0;
    // SAFETY: plain GL calls.
    unsafe {
        glGenRenderbuffers(1, &mut rb);
        glBindRenderbuffer(GL_RENDERBUFFER, rb);
        while glGetError() != GL_NO_ERROR {}
        glRenderbufferStorageMultisample(GL_RENDERBUFFER, samples, format, 8, 8);
    }
    (err(), rb)
}

/// A bound 8x8 framebuffer with a renderbuffer of `samples` on each of
/// `formats`' colour attachments. A storage error comes back as `Err(error)`.
fn framebuffer(samples: GLsizei, formats: &[GLenum]) -> Result<Fbo, GLenum> {
    let mut fb = 0;
    // SAFETY: room for one name; plain GL call.
    unsafe {
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
    }
    let mut fbo = Fbo { fb, rbs: Vec::new() };
    for (i, f) in formats.iter().enumerate() {
        let (e, rb) = storage(samples, *f);
        fbo.rbs.push(rb);
        if e != GL_NO_ERROR {
            return Err(e);
        }
        // SAFETY: a renderbuffer this test made.
        unsafe { glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0 + i as GLenum, GL_RENDERBUFFER, rb) };
    }
    assert_eq!(err(), GL_NO_ERROR);
    Ok(fbo)
}

const RED: [f32; 4] = [1.0, 0.0, 0.0, 1.0];
const GREEN: [f32; 4] = [0.0, 1.0, 0.0, 1.0];
const BLUE: [f32; 4] = [0.0, 0.0, 1.0, 1.0];
const COLOURS: [[f32; 4]; 3] = [RED, GREEN, BLUE];

fn draw_buffers(b: &[GLenum]) {
    // SAFETY: `b` is a list of `b.len()` enums.
    unsafe { glDrawBuffers(b.len() as GLsizei, b.as_ptr()) };
    assert_eq!(err(), GL_NO_ERROR);
}

/// A 4x framebuffer with three RGBA8 attachments, cleared to red, green, blue.
fn source() -> Fbo {
    let src = framebuffer(4, &[GL_RGBA8; 3]).unwrap();
    draw_buffers(&[GL_COLOR_ATTACHMENT0, GL_COLOR_ATTACHMENT1, GL_COLOR_ATTACHMENT2]);
    for (k, c) in COLOURS.iter().enumerate() {
        // SAFETY: four GLfloats in.
        unsafe { glClearBufferfv(GL_COLOR, k as GLint, c.as_ptr()) };
        assert_eq!(err(), GL_NO_ERROR);
    }
    src
}

/// A 1x framebuffer of `n` RGBA8 attachments with draw buffers 0..n on.
fn dest(n: usize) -> Fbo {
    let dst = framebuffer(0, &vec![GL_RGBA8; n]).unwrap();
    let bufs: Vec<GLenum> = (0..n as GLenum).map(|i| GL_COLOR_ATTACHMENT0 + i).collect();
    draw_buffers(&bufs);
    dst
}

/// The 8x8 pixels of colour attachment `k` of single-sample `fb`.
fn pixels_of(fb: GLuint, k: GLenum) -> Vec<[u8; 4]> {
    let mut px = vec![[0x11u8; 4]; 64];
    // SAFETY: room for 8x8 RGBA/UNSIGNED_BYTE pixels.
    unsafe {
        glBindFramebuffer(GL_READ_FRAMEBUFFER, fb);
        glReadBuffer(GL_COLOR_ATTACHMENT0 + k);
        glReadPixels(0, 0, 8, 8, GL_RGBA, GL_UNSIGNED_BYTE, px.as_mut_ptr() as *mut c_void);
    }
    assert_eq!(err(), GL_NO_ERROR);
    px
}

fn all(fb: GLuint, k: GLenum, c: [f32; 4]) -> bool {
    pixels_of(fb, k).iter().all(|p| *p == bytes(c))
}

/// glReadBuffer(`rb`) on `read`, then a whole-surface colour blit into `dst`.
fn blit_from(read: GLuint, rb: GLenum, dst: GLuint) -> GLenum {
    // SAFETY: plain GL calls on names this test made.
    unsafe {
        glBindFramebuffer(GL_READ_FRAMEBUFFER, read);
        glReadBuffer(rb);
        glBindFramebuffer(GL_DRAW_FRAMEBUFFER, dst);
        while glGetError() != GL_NO_ERROR {}
        glBlitFramebuffer(0, 0, 8, 8, 0, 0, 8, 8, GL_COLOR_BUFFER_BIT, GL_NEAREST);
    }
    err()
}

/// Each of the three attachments resolves its own colour when it is the read buffer.
#[test]
fn each_read_buffer_resolves_its_attachment() {
    let _g = context_ms4();
    let src = source();
    let dst = dest(1);
    for (k, c) in COLOURS.iter().enumerate() {
        assert_eq!(blit_from(src.fb, GL_COLOR_ATTACHMENT0 + k as GLenum, dst.fb), GL_NO_ERROR);
        assert!(all(dst.fb, 0, *c), "read buffer COLOR_ATTACHMENT{k}");
    }
    // in reverse order, so no result is a leftover of the previous blit
    for (k, c) in COLOURS.iter().enumerate().rev() {
        assert_eq!(blit_from(src.fb, GL_COLOR_ATTACHMENT0 + k as GLenum, dst.fb), GL_NO_ERROR);
        assert!(all(dst.fb, 0, *c), "read buffer COLOR_ATTACHMENT{k}, second pass");
    }
}

/// The read buffer is per read framebuffer and queryable.
#[test]
fn read_buffer_state_is_queryable() {
    let _g = context_ms4();
    let src = source();
    for k in 0..3 {
        let mut v: GLint = -1;
        // SAFETY: plain GL calls; one GLint out.
        unsafe {
            glBindFramebuffer(GL_READ_FRAMEBUFFER, src.fb);
            glReadBuffer(GL_COLOR_ATTACHMENT0 + k);
            glGetIntegerv(GL_READ_BUFFER, &mut v);
        }
        assert_eq!(err(), GL_NO_ERROR);
        assert_eq!(v as GLenum, GL_COLOR_ATTACHMENT0 + k);
    }
}

/// One blit writes every enabled draw buffer of the draw framebuffer.
#[test]
fn one_blit_writes_both_draw_buffers() {
    let _g = context_ms4();
    let src = source();
    let dst = dest(2);
    assert_eq!(blit_from(src.fb, GL_COLOR_ATTACHMENT2, dst.fb), GL_NO_ERROR);
    assert!(all(dst.fb, 0, BLUE), "draw buffer 0");
    assert!(all(dst.fb, 1, BLUE), "draw buffer 1");
    assert_eq!(blit_from(src.fb, GL_COLOR_ATTACHMENT1, dst.fb), GL_NO_ERROR);
    assert!(all(dst.fb, 0, GREEN) && all(dst.fb, 1, GREEN), "both again, from attachment 1");
}

/// ES 3.0 4.3.3: with the read buffer GL_NONE the colour bit has no source,
/// so a COLOR_BUFFER_BIT blit copies nothing and raises no error (the spec
/// names no error for it).
#[test]
fn read_buffer_none_copies_nothing() {
    let _g = context_ms4();
    let src = source();
    let dst = dest(2);
    assert_eq!(blit_from(src.fb, GL_COLOR_ATTACHMENT0, dst.fb), GL_NO_ERROR);
    assert!(all(dst.fb, 0, RED) && all(dst.fb, 1, RED));
    assert_eq!(blit_from(src.fb, GL_NONE, dst.fb), GL_NO_ERROR);
    assert!(all(dst.fb, 0, RED) && all(dst.fb, 1, RED), "nothing copied, no error");
}
