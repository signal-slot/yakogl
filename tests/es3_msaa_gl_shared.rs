// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! ES 3.0 one 4x renderbuffer attached to two framebuffers, on the host
//! (docs/notes/es3-msaa-renderbuffers.md (a), (b)5).
//!
//! The renderbuffer is COLOR_ATTACHMENT0 of FBO A and COLOR_ATTACHMENT1 of
//! FBO B: a draw through either lands in the same samples, and the back end is
//! handed the same `(target_id, target_gen)` for it. The stand-in back end is
//! that of tests/es3_msaa_gl.rs (`context_ms4`, helpers copied); its job-end
//! resolve fills each 4x target a clear touched with that clear's colour (a
//! clear names its target in `clear_target_mask`), and not before the job is
//! finished.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_msaa_gl_shared
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

/// The identity of one 4x colour target of a clear: (slot, `target_id`, `target_gen`).
type Ident = (usize, u32, u32);

/// The identities each clear of a 4x target was handed, in order.
static IDENTS: Mutex<Vec<Vec<Ident>>> = Mutex::new(Vec::new());

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
        let mut ids = Vec::new();
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
            ids.push((k, d.target_ids[k], d.target_gens[k]));
        }
        IDENTS.lock().unwrap_or_else(|e| e.into_inner()).push(ids);
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
    name: c"es3-msaa-gl-shared".as_ptr() as *const c_char,
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
    IDENTS.lock().unwrap_or_else(|e| e.into_inner()).clear();
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

fn status() -> GLenum {
    // SAFETY: plain GL call.
    unsafe { glCheckFramebufferStatus(GL_FRAMEBUFFER) }
}

/* ---- helpers of this file ---------------------------------------------- */

const RED: [f32; 4] = [1.0, 0.0, 0.0, 1.0];
const GREEN: [f32; 4] = [0.0, 1.0, 0.0, 1.0];
const BLUE: [f32; 4] = [0.0, 0.0, 1.0, 1.0];
const YELLOW: [f32; 4] = [1.0, 1.0, 0.0, 1.0];

fn bind(fb: GLuint) {
    // SAFETY: a framebuffer this test made.
    unsafe { glBindFramebuffer(GL_FRAMEBUFFER, fb) };
}

/// glClearBufferfv(GL_COLOR, `k`, c) on the bound framebuffer.
fn clear_buffer(k: GLint, c: [f32; 4]) {
    // SAFETY: four GLfloats in.
    unsafe { glClearBufferfv(GL_COLOR, k, c.as_ptr()) };
    assert_eq!(err(), GL_NO_ERROR);
}

/// `glDrawBuffers` on the bound framebuffer.
fn draw_buffers(b: &[GLenum]) {
    // SAFETY: `b` is a list of `b.len()` enums.
    unsafe { glDrawBuffers(b.len() as GLsizei, b.as_ptr()) };
    assert_eq!(err(), GL_NO_ERROR);
}

fn attach(fb: GLuint, att: GLenum, rb: GLuint) {
    // SAFETY: names this test made.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        glFramebufferRenderbuffer(GL_FRAMEBUFFER, att, GL_RENDERBUFFER, rb);
    }
    assert_eq!(err(), GL_NO_ERROR);
}

/// The 8x8 pixels of colour attachment 0 of single-sample `fb`.
fn pixels(fb: GLuint) -> Vec<[u8; 4]> {
    let mut px = vec![[0x11u8; 4]; 64];
    // SAFETY: room for 8x8 RGBA/UNSIGNED_BYTE pixels.
    unsafe {
        glBindFramebuffer(GL_READ_FRAMEBUFFER, fb);
        glReadBuffer(GL_COLOR_ATTACHMENT0);
        glReadPixels(0, 0, 8, 8, GL_RGBA, GL_UNSIGNED_BYTE, px.as_mut_ptr() as *mut c_void);
    }
    assert_eq!(err(), GL_NO_ERROR);
    px
}

/// Resolve attachment `rb` of `read` into the 1x framebuffer `dst`: the GL error.
fn resolve(read: GLuint, rb: GLenum, dst: GLuint) -> GLenum {
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

fn all(fb: GLuint, c: [f32; 4]) -> bool {
    pixels(fb).iter().all(|p| *p == bytes(c))
}

/// FBO A (shared on attachment 0), FBO B (own 4x on 0, shared on 1) and a 1x
/// destination. The shared renderbuffer is `a.rbs[0]`.
fn pair() -> (Fbo, Fbo, Fbo) {
    let a = framebuffer(4, &[GL_RGBA8]).unwrap();
    let b = framebuffer(4, &[GL_RGBA8]).unwrap();
    attach(b.fb, GL_COLOR_ATTACHMENT1, a.rbs[0]);
    assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE);
    bind(a.fb);
    assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE);
    let dst = framebuffer(0, &[GL_RGBA8]).unwrap();
    (a, b, dst)
}

/// The identity handed for slot `k` of the last clear that touched it.
fn last_ident(k: usize) -> (u32, u32) {
    // SAFETY: plain GL call; a clear is handed to the back end when the job is flushed.
    unsafe { glFinish() };
    let id = IDENTS.lock().unwrap_or_else(|e| e.into_inner());
    let i = id.iter().rev().flatten().find(|i| i.0 == k).expect("a clear touched the slot");
    (i.1, i.2)
}

/// (1) A clear through A shows when B's attachment 1 is resolved.
#[test]
fn draw_through_a_resolves_through_b() {
    let _g = context_ms4();
    let (a, b, dst) = pair();
    bind(a.fb);
    clear_buffer(0, RED);
    assert_eq!(resolve(b.fb, GL_COLOR_ATTACHMENT1, dst.fb), GL_NO_ERROR);
    assert!(all(dst.fb, RED), "what A drew");
}

/// (2) Draws through B to both attachments with glDrawBuffers; the shared
/// renderbuffer resolved through A holds B's attachment-1 colour, and B's own
/// attachment 0 its own.
#[test]
fn draw_through_b_resolves_through_a() {
    let _g = context_ms4();
    let (a, b, dst) = pair();
    bind(a.fb);
    clear_buffer(0, RED);
    bind(b.fb);
    draw_buffers(&[GL_COLOR_ATTACHMENT0, GL_COLOR_ATTACHMENT1]);
    clear_buffer(0, BLUE);
    clear_buffer(1, GREEN);
    assert_eq!(resolve(a.fb, GL_COLOR_ATTACHMENT0, dst.fb), GL_NO_ERROR);
    assert!(all(dst.fb, GREEN), "the shared renderbuffer holds B's attachment 1");
    assert_eq!(resolve(b.fb, GL_COLOR_ATTACHMENT0, dst.fb), GL_NO_ERROR);
    assert!(all(dst.fb, BLUE), "B's own renderbuffer is separate");
}

/// (3) Both framebuffers hand the back end one identity for the shared
/// renderbuffer (A's slot 0, B's slot 1) and another for B's own.
#[test]
fn both_draws_report_one_target_identity() {
    let _g = context_ms4();
    let (a, b, _dst) = pair();
    bind(a.fb);
    clear_buffer(0, RED);
    let via_a = last_ident(0);
    bind(b.fb);
    draw_buffers(&[GL_COLOR_ATTACHMENT0, GL_COLOR_ATTACHMENT1]);
    clear_buffer(1, GREEN);
    let via_b = last_ident(1);
    clear_buffer(0, BLUE);
    let own = last_ident(0);
    assert_ne!(via_a.0, 0, "a renderbuffer attachment has an id");
    assert_eq!(via_a, via_b, "(target_id, target_gen) of the shared renderbuffer");
    assert_ne!(own.0, via_a.0, "B's own renderbuffer is another object");
}

/// Detach A's attachment after a clear through A, and check B is complete and
/// still resolves what A drew and what B draws next. `settle`: finish the
/// jobs before the detach.
fn detach_from_a(settle: bool) {
    let (a, b, dst) = pair();
    bind(a.fb);
    clear_buffer(0, YELLOW);
    if settle {
        // SAFETY: plain GL call.
        unsafe { glFinish() };
    }
    attach(a.fb, GL_COLOR_ATTACHMENT0, 0);
    assert_eq!(status(), GL_FRAMEBUFFER_INCOMPLETE_MISSING_ATTACHMENT, "A has nothing now");
    bind(b.fb);
    assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE);
    assert_eq!(resolve(b.fb, GL_COLOR_ATTACHMENT1, dst.fb), GL_NO_ERROR);
    assert!(all(dst.fb, YELLOW), "A's draw survives the detach");
    bind(b.fb);
    draw_buffers(&[GL_COLOR_ATTACHMENT0, GL_COLOR_ATTACHMENT1]);
    let before = a_id_of(&b);
    clear_buffer(1, GREEN);
    assert_eq!(last_ident(1).0, before, "still the same renderbuffer");
    assert_eq!(resolve(b.fb, GL_COLOR_ATTACHMENT1, dst.fb), GL_NO_ERROR);
    assert!(all(dst.fb, GREEN), "B draws to it after the detach");
}

/// (4) Detaching from A (its jobs finished) leaves B complete: it resolves
/// what A drew and what B draws next. A without attachments is incomplete.
#[test]
fn detach_from_a_leaves_b() {
    let _g = context_ms4();
    detach_from_a(true);
}

/// (5) The same with A's clear still pending at the detach. A clear issued
/// while A was complete must still be done; today the blit that flushes it
/// reports GL_INVALID_FRAMEBUFFER_OPERATION because A is incomplete by then
/// (src/gl/draw.rs `emit_pending_clear` / `settle_pending_clear`, not quiet).
/// Issue draft: "glFramebufferRenderbuffer(.., 0) on a framebuffer with a
/// pending clear: the clear is lost with GL_INVALID_FRAMEBUFFER_OPERATION on
/// the next unrelated glBlitFramebuffer; ES 3.0 4.4.4: the clear was valid when
/// issued". Owner: src/gl/draw.rs (a plan item that holds it).
#[test]
#[ignore = "src/gl/draw.rs: a pending clear of a framebuffer made incomplete by a detach raises GL_INVALID_FRAMEBUFFER_OPERATION at the next blit"]
fn detach_from_a_with_clear_pending() {
    let _g = context_ms4();
    detach_from_a(false);
}

/// The id B hands for its attachment 1 (the shared renderbuffer) by a clear.
fn a_id_of(b: &Fbo) -> u32 {
    bind(b.fb);
    clear_buffer(1, GREEN);
    last_ident(1).0
}
