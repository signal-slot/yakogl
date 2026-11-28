// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! ES 3.0 resolve blits with several colour attachments, on the host
//! (docs/notes/es3-msaa-renderbuffers.md (a), (b)).
//!
//! ES 3.0 4.3.3: a blit reads the read framebuffer's GL_READ_BUFFER
//! attachment and writes every enabled draw buffer of the draw framebuffer.
//! The stand-in back end is that of tests/es3_msaa_gl.rs (`context_ms4`,
//! helpers copied); its job-end resolve fills each 4x target a clear touched
//! with that clear's colour (a clear names its target in `clear_target_mask`),
//! and not before the job is finished.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_msaa_gl_mrt
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
use yakogl::gl::texture::{glBindTexture, glDeleteTextures, glGenTextures, glTexImage2D};
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

const N: usize = backend::GLES_MAX_TARGETS;

/// `(target_ids, target_gens)` of every clear draw, in order.
static IDS: Mutex<Vec<([u32; N], [u32; N])>> = Mutex::new(Vec::new());

unsafe extern "C" fn b_draw(d: *const GlesDraw) -> i32 {
    let d = &*d;
    if d.index_count == 0 {
        let (ids, gens) = (d.target_ids, d.target_gens);
        assert_eq!((ids[0], gens[0]), (d.target_id, d.target_gen), "slot 0 mirrors the scalar pair");
        IDS.lock().unwrap().push((ids, gens));
    }
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
    name: c"es3-msaa-gl-mrt".as_ptr() as *const c_char,
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

fn status() -> GLenum {
    // SAFETY: plain GL call.
    unsafe { glCheckFramebufferStatus(GL_FRAMEBUFFER) }
}

/* ---- today, the capability off: passing pins -------------------------------- */

/* ---- helpers of this file ---------------------------------------------- */

const RED: [f32; 4] = [1.0, 0.0, 0.0, 1.0];
const GREEN: [f32; 4] = [0.0, 1.0, 0.0, 1.0];
const BLUE: [f32; 4] = [0.0, 0.0, 1.0, 1.0];
const YELLOW: [f32; 4] = [1.0, 1.0, 0.0, 1.0];

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

/// A 4x framebuffer with two RGBA8 attachments cleared to `a0` and `a1`.
fn source(a0: [f32; 4], a1: [f32; 4]) -> Fbo {
    let src = framebuffer(4, &[GL_RGBA8, GL_RGBA8]).unwrap();
    draw_buffers(&[GL_COLOR_ATTACHMENT0, GL_COLOR_ATTACHMENT1]);
    clear_buffer(0, a0);
    clear_buffer(1, a1);
    src
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

/// Blit the colour of `read` attachment `rb` to the draw buffers of `dst`
/// (set before): the GL error.
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

fn all(fb: GLuint, k: GLenum, c: [f32; 4]) -> bool {
    pixels_of(fb, k).iter().all(|p| *p == bytes(c))
}

/// A 1-sample two-attachment RGBA8 framebuffer with both draw buffers on.
fn dest() -> Fbo {
    let dst = framebuffer(0, &[GL_RGBA8, GL_RGBA8]).unwrap();
    draw_buffers(&[GL_COLOR_ATTACHMENT0, GL_COLOR_ATTACHMENT1]);
    dst
}

/// (1) GL_READ_BUFFER picks the source attachment: COLOR_ATTACHMENT1's colour.
#[test]
fn read_buffer_selects_attachment_1() {
    let _g = context_ms4();
    let src = source(RED, GREEN);
    let dst = framebuffer(0, &[GL_RGBA8]).unwrap();
    assert_eq!(blit_from(src.fb, GL_COLOR_ATTACHMENT1, dst.fb), GL_NO_ERROR);
    assert!(all(dst.fb, 0, GREEN), "attachment 1's colour, not attachment 0's");
    assert_eq!(blit_from(src.fb, GL_COLOR_ATTACHMENT0, dst.fb), GL_NO_ERROR);
    assert!(all(dst.fb, 0, RED), "and attachment 0's with the read buffer back");
}

/// (2) Every enabled draw buffer is written from the one read buffer.
#[test]
fn every_draw_buffer_is_written() {
    let _g = context_ms4();
    let src = source(RED, GREEN);
    let dst = dest();
    assert_eq!(blit_from(src.fb, GL_COLOR_ATTACHMENT1, dst.fb), GL_NO_ERROR);
    assert!(all(dst.fb, 0, GREEN), "draw buffer 0");
    assert!(all(dst.fb, 1, GREEN), "draw buffer 1");
}

/// (3) GL_NONE in the draw buffers writes nothing there; A0 keeps its contents.
#[test]
fn none_draw_buffer_is_not_written() {
    let _g = context_ms4();
    let first = source(RED, GREEN);
    let dst = dest();
    assert_eq!(blit_from(first.fb, GL_COLOR_ATTACHMENT0, dst.fb), GL_NO_ERROR);
    assert!(all(dst.fb, 0, RED) && all(dst.fb, 1, RED));
    let second = source(BLUE, YELLOW);
    // SAFETY: plain GL call.
    unsafe { glBindFramebuffer(GL_FRAMEBUFFER, dst.fb) };
    draw_buffers(&[GL_NONE, GL_COLOR_ATTACHMENT1]);
    assert_eq!(blit_from(second.fb, GL_COLOR_ATTACHMENT1, dst.fb), GL_NO_ERROR);
    assert!(all(dst.fb, 0, RED), "A0 keeps the earlier resolve");
    assert!(all(dst.fb, 1, YELLOW), "A1 is written");
}

/// (4) ES 3.0: with the read buffer GL_NONE nothing is copied and there is no
/// error. The GL core does that (src/gl/blit.rs drops the colour bit when
/// there is no read image).
#[test]
fn read_buffer_none_copies_nothing() {
    let _g = context_ms4();
    let first = source(RED, GREEN);
    let dst = dest();
    assert_eq!(blit_from(first.fb, GL_COLOR_ATTACHMENT0, dst.fb), GL_NO_ERROR);
    let second = source(BLUE, YELLOW);
    assert_eq!(blit_from(second.fb, GL_NONE, dst.fb), GL_NO_ERROR);
    assert!(all(dst.fb, 0, RED) && all(dst.fb, 1, RED), "nothing copied");
}

/// (5) A format mismatch on only one draw buffer (RGBA8 source, RGB565 on A1)
/// is GL_INVALID_OPERATION and copies nothing; with that buffer disabled the
/// blit is valid.
#[test]
fn format_mismatch_on_one_draw_buffer_is_invalid_operation() {
    let _g = context_ms4();
    let src = source(RED, GREEN);
    let dst = framebuffer(0, &[GL_RGBA8, GL_RGB565]).unwrap();
    draw_buffers(&[GL_COLOR_ATTACHMENT0, GL_COLOR_ATTACHMENT1]);
    assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE);
    assert_eq!(blit_from(src.fb, GL_COLOR_ATTACHMENT0, dst.fb), GL_INVALID_OPERATION);
    assert!(pixels_of(dst.fb, 0).iter().all(|p| *p == [0; 4]), "nothing is copied to A0 either");
    // SAFETY: plain GL call.
    unsafe { glBindFramebuffer(GL_FRAMEBUFFER, dst.fb) };
    draw_buffers(&[GL_COLOR_ATTACHMENT0, GL_NONE]);
    assert_eq!(blit_from(src.fb, GL_COLOR_ATTACHMENT0, dst.fb), GL_NO_ERROR);
    assert!(all(dst.fb, 0, RED));
}

/* ---- GlesDraw::target_ids / target_gens per MRT slot ---------------------- */

/// Clear every colour attachment of the bound FBO (`n` of them) and return the
/// ids and generations the last clear's draw carried.
fn ids_after_clears(n: GLint) -> ([u32; N], [u32; N]) {
    IDS.lock().unwrap().clear();
    for k in 0..n {
        clear_buffer(k, RED);
    }
    *IDS.lock().unwrap().last().expect("a clear was recorded")
}

/// Specify `rb` again with 4 samples.
fn restore(rb: GLuint) {
    // SAFETY: plain GL calls on a renderbuffer this test made.
    unsafe {
        glBindRenderbuffer(GL_RENDERBUFFER, rb);
        glRenderbufferStorageMultisample(GL_RENDERBUFFER, 4, GL_RGBA8, 8, 8);
    }
    assert_eq!(err(), GL_NO_ERROR);
}

fn both_draw_buffers() {
    draw_buffers(&[GL_COLOR_ATTACHMENT0, GL_COLOR_ATTACHMENT1, GL_COLOR_ATTACHMENT2]);
}

/// Each 4x colour slot has its own non-zero id and generation; they stay
/// across redraws.
#[test]
fn every_slot_has_a_distinct_stable_id() {
    let _g = context_ms4();
    let fbo = framebuffer(4, &[GL_RGBA8, GL_RGBA8, GL_RGBA8]).unwrap();
    both_draw_buffers();
    let (ids, gens) = ids_after_clears(3);
    for k in 0..3 {
        assert_ne!(ids[k], 0, "slot {k} id");
        assert_ne!(gens[k], 0, "slot {k} gen");
        for j in 0..k {
            assert_ne!(ids[k], ids[j], "slots {j} and {k} share an id");
        }
    }
    assert_eq!((ids[3], gens[3]), (0, 0), "slot past the attachments");
    assert_eq!(ids_after_clears(3), (ids, gens), "a redraw");
    drop(fbo);
}

/// Re-storing one attachment moves only its generation.
#[test]
fn restoring_one_slot_moves_only_its_gen() {
    let _g = context_ms4();
    let fbo = framebuffer(4, &[GL_RGBA8, GL_RGBA8, GL_RGBA8]).unwrap();
    both_draw_buffers();
    let (ids, mut gens) = ids_after_clears(3);
    for k in 0..3usize {
        restore(fbo.rbs[k]);
        // SAFETY: a framebuffer name this test made.
        unsafe { glBindFramebuffer(GL_FRAMEBUFFER, fbo.fb) };
        let (i2, g2) = ids_after_clears(3);
        assert_eq!(i2, ids, "re-storing slot {k}: ids stay");
        for j in 0..3 {
            if j == k {
                assert_ne!(g2[j], gens[j], "slot {k}'s gen moves");
            } else {
                assert_eq!(g2[j], gens[j], "slot {j}'s gen stays when slot {k} is re-stored");
            }
        }
        gens = g2;
    }
}

/// A texture attachment, a GL_NONE draw buffer and an unattached slot give 0.
#[test]
fn texture_and_empty_slots_are_zero() {
    let _g = context_ms4();
    let _fbo = framebuffer(0, &[GL_RGBA8]).unwrap();
    let mut tex = 0;
    // SAFETY: plain GL calls; one name out.
    unsafe {
        glGenTextures(1, &mut tex);
        glBindTexture(GL_TEXTURE_2D, tex);
        glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA8 as GLint, 8, 8, 0, GL_RGBA, GL_UNSIGNED_BYTE, core::ptr::null());
        glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT1, GL_TEXTURE_2D, tex, 0);
    }
    assert_eq!(err(), GL_NO_ERROR);
    let (e, rb3) = storage(0, GL_RGBA8);
    assert_eq!(e, GL_NO_ERROR);
    // SAFETY: a renderbuffer made above.
    unsafe { glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT3, GL_RENDERBUFFER, rb3) };
    // slot 2 has no attachment, slot 1 a texture, slot 3 a renderbuffer; one sample throughout, a texture cannot join a 4x FBO
    draw_buffers(&[GL_COLOR_ATTACHMENT0, GL_COLOR_ATTACHMENT1, GL_COLOR_ATTACHMENT2, GL_COLOR_ATTACHMENT3]);
    let (ids, gens) = ids_after_clears(4);
    assert_ne!(ids[0], 0);
    assert_ne!(ids[3], 0);
    assert_eq!((ids[1], gens[1]), (0, 0), "texture slot");
    assert_eq!((ids[2], gens[2]), (0, 0), "empty slot");
    // GL_NONE in the draw buffers: slot 3 off
    draw_buffers(&[GL_COLOR_ATTACHMENT0, GL_NONE, GL_NONE, GL_NONE]);
    IDS.lock().unwrap().clear();
    clear_buffer(0, RED);
    let (ids, gens) = *IDS.lock().unwrap().last().unwrap();
    assert_ne!(ids[0], 0);
    assert_eq!((ids[1..].to_vec(), gens[1..].to_vec()), (vec![0; N - 1], vec![0; N - 1]), "disabled draw buffers");
    // SAFETY: names made above.
    unsafe {
        glDeleteTextures(1, &tex);
        glDeleteRenderbuffers(1, &rb3);
    }
}
