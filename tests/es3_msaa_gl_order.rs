// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! ES 3.0 multisample renderbuffers: the order of draws and resolves on one 4x
//! framebuffer, on the host (docs/notes/es3-msaa-renderbuffers.md).
//!
//! A stand-in back end with `max_samples` 4 records, in order, every
//! `GlesDraw`, `flush` and `finish` the GL core hands it. Its `finish` is the
//! job-end resolve of note (b)7: it writes a colour that counts the resolves
//! (1, 2, 3, ...) into each 4x target drawn since the last `finish`, so the
//! pixels a blit copied say which resolve they came from. There is no
//! separate resolve hook: a resolve is the `finish` that writes a target.
//!
//! No other `es3_msaa_gl_*.rs` file records the hook order across several
//! draw/resolve rounds (`es3_msaa_gl.rs` has one draw and one resolve,
//! `es3_msaa_gl_draws.rs` counts draws); this file does not repeat them.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_msaa_gl_order
//! ```

#![cfg(all(feature = "host-test", feature = "shaderc-online", not(feature = "report-es2")))]

use core::ffi::{c_char, c_void};
use std::ffi::CString;
use std::sync::atomic::Ordering;
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{self, GlesDraw, GlesHeap, YakoglBackend};
use yakogl::gl::buffer::glBindBuffer;
use yakogl::gl::consts::*;
use yakogl::gl::draw::*;
use yakogl::gl::framebuffer::*;
use yakogl::gl::program::*;
use yakogl::gl::shader::*;
use yakogl::gl::state::*;
use yakogl::gl::types::*;
use yakogl::gl::vertex::{glEnableVertexAttribArray, glVertexAttribPointer};
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

/// One call the back end got.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Ev {
    /// An application draw: its sample count.
    Draw(u8),
    Flush,
    /// A job end that resolved this many 4x targets, the resolve number
    /// written (0 when none).
    Finish {
        targets: usize,
        resolve: u8,
    },
}

static EVENTS: Mutex<Vec<Ev>> = Mutex::new(Vec::new());
/// The 4x targets drawn since the last `finish`: (`target_gpu`, w, h).
static PENDING: Mutex<Vec<(u64, u32, u32)>> = Mutex::new(Vec::new());
/// Resolves done so far.
static RESOLVES: Mutex<u8> = Mutex::new(0);

unsafe extern "C" fn b_draw(d: *const GlesDraw) -> i32 {
    let d = &*d;
    if d.index_count != 0 {
        EVENTS.lock().unwrap().push(Ev::Draw(d.samples));
    }
    if d.samples > 1 {
        PENDING.lock().unwrap().push((d.target_gpu, d.target_w, d.target_h));
    }
    0
}

unsafe extern "C" fn b_flush() -> i32 {
    EVENTS.lock().unwrap().push(Ev::Flush);
    0
}

/// The job-end resolve: every 4x target drawn gets the next resolve number.
unsafe extern "C" fn b_finish() -> i32 {
    let pending = core::mem::take(&mut *PENDING.lock().unwrap());
    let mut n = RESOLVES.lock().unwrap();
    if !pending.is_empty() {
        *n += 1;
    }
    for (gpu, w, h) in &pending {
        let px = core::slice::from_raw_parts_mut(*gpu as *mut [u8; 4], (w * h) as usize);
        px.fill([*n, *n, *n, 0xff]);
    }
    let resolve = if pending.is_empty() { 0 } else { *n };
    EVENTS.lock().unwrap().push(Ev::Finish { targets: pending.len(), resolve });
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
    name: c"es3-msaa-gl-order".as_ptr() as *const c_char,
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
    flush: Some(b_flush),
    finish: Some(b_finish),
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
    max_samples: 4,
};

static LOCK: Mutex<()> = Mutex::new(());
static ONCE: Once = Once::new();

fn context() -> MutexGuard<'static, ()> {
    let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    backend::HOST_DRAW_TARGETS.store(4, Ordering::Relaxed);
    ONCE.call_once(|| {
        // SAFETY: statics; the lock serialises the tests.
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
        glFinish();
    }
    EVENTS.lock().unwrap().clear();
    PENDING.lock().unwrap().clear();
    *RESOLVES.lock().unwrap() = 0;
    // SAFETY: plain GL call.
    while unsafe { glGetError() } != GL_NO_ERROR {}
    g
}

fn err() -> GLenum {
    // SAFETY: plain GL call.
    unsafe { glGetError() }
}

struct Fbo {
    fb: GLuint,
    rb: GLuint,
}

impl Drop for Fbo {
    fn drop(&mut self) {
        // SAFETY: names this test made.
        unsafe {
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
            glDeleteFramebuffers(1, &self.fb);
            glDeleteRenderbuffers(1, &self.rb);
            glUseProgram(0);
        }
    }
}

/// An 8x8 RGBA8 framebuffer with a renderbuffer of `samples`, left bound.
fn framebuffer(samples: GLsizei) -> Fbo {
    let (mut fb, mut rb) = (0, 0);
    // SAFETY: plain GL calls.
    unsafe {
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        glGenRenderbuffers(1, &mut rb);
        glBindRenderbuffer(GL_RENDERBUFFER, rb);
        glRenderbufferStorageMultisample(GL_RENDERBUFFER, samples, GL_RGBA8, 8, 8);
        glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_RENDERBUFFER, rb);
    }
    assert_eq!(err(), GL_NO_ERROR);
    Fbo { fb, rb }
}

fn shader(ty: GLenum, text: &str) -> GLuint {
    let text = CString::new(text).unwrap();
    // SAFETY: plain GL calls; one NUL-terminated source.
    unsafe {
        let s = glCreateShader(ty);
        let p: [*const c_char; 1] = [text.as_ptr()];
        glShaderSource(s, 1, p.as_ptr(), core::ptr::null());
        glCompileShader(s);
        let mut ok: GLint = 0;
        glGetShaderiv(s, GL_COMPILE_STATUS, &mut ok);
        assert_eq!(ok, 1, "{text:?} did not compile");
        s
    }
}

fn program() {
    let v = shader(GL_VERTEX_SHADER, "#version 300 es\nin vec4 p;\nvoid main() { gl_Position = p; }\n");
    let f = shader(
        GL_FRAGMENT_SHADER,
        "#version 300 es\nprecision highp float;\nlayout(location = 0) out vec4 o;\nvoid main() { o = vec4(0.25, 0.5, 0.75, 1.0); }\n",
    );
    // SAFETY: plain GL calls.
    unsafe {
        let p = glCreateProgram();
        glAttachShader(p, v);
        glAttachShader(p, f);
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        assert_eq!(ok, 1);
        glUseProgram(p);
    }
}

/// One triangle over `fb`; its job is not finished.
fn draw_to(fb: GLuint) {
    // SAFETY: plain GL call.
    unsafe { glBindFramebuffer(GL_FRAMEBUFFER, fb) };
    draw();
}

/// One triangle over the bound framebuffer; its job is not finished.
fn draw() {
    static TRI: [f32; 12] = [-1.0, -1.0, 0.0, 1.0, 3.0, -1.0, 0.0, 1.0, -1.0, 3.0, 0.0, 1.0];
    // SAFETY: TRI is static and outlives the draw.
    unsafe {
        glBindBuffer(GL_ARRAY_BUFFER, 0);
        glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, TRI.as_ptr() as *const c_void);
        glEnableVertexAttribArray(0);
        glDrawArrays(GL_TRIANGLES, 0, 3);
    }
    assert_eq!(err(), GL_NO_ERROR);
}

/// Resolve the whole of `src` (4x) into `dst` (1x); the GL error.
fn resolve(src: GLuint, dst: GLuint) -> GLenum {
    // SAFETY: plain GL calls on names this test made.
    unsafe {
        glBindFramebuffer(GL_READ_FRAMEBUFFER, src);
        glBindFramebuffer(GL_DRAW_FRAMEBUFFER, dst);
        glReadBuffer(GL_COLOR_ATTACHMENT0);
        while glGetError() != GL_NO_ERROR {}
        glBlitFramebuffer(0, 0, 8, 8, 0, 0, 8, 8, GL_COLOR_BUFFER_BIT, GL_NEAREST);
    }
    err()
}

/// Pixels of colour attachment 0 of 1x `fb`, 8x8.
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

/// Every pixel of `fb` is resolve number `n`.
fn is_resolve(fb: GLuint, n: u8) -> bool {
    pixels(fb).iter().all(|p| *p == [n, n, n, 0xff])
}

/// The events since the last call, with the flushes and the empty job ends
/// (a readback's wait) left out: the draws and resolves are the order.
fn order() -> Vec<Ev> {
    core::mem::take(&mut *EVENTS.lock().unwrap())
        .into_iter()
        .filter(|e| !matches!(e, Ev::Flush | Ev::Finish { targets: 0, .. }))
        .collect()
}

/// Finish and drop the events (the setup of a test, not the test).
fn settle() {
    // SAFETY: plain GL call.
    unsafe { glFinish() };
    EVENTS.lock().unwrap().clear();
}

const D4: Ev = Ev::Draw(4);

#[test]
fn draw_resolve_draw_resolve_keeps_both_resolves() {
    let _g = context();
    let dst = framebuffer(0);
    let src = framebuffer(4);
    program();
    settle();
    draw_to(src.fb);
    assert_eq!(resolve(src.fb, dst.fb), GL_NO_ERROR);
    assert!(is_resolve(dst.fb, 1), "the first resolve is what the first blit copied");
    draw_to(src.fb);
    assert_eq!(resolve(src.fb, dst.fb), GL_NO_ERROR);
    assert!(is_resolve(dst.fb, 2), "the second resolve is not merged into the first");
    let got = order();
    assert_eq!(
        got,
        [D4, Ev::Finish { targets: 1, resolve: 1 }, D4, Ev::Finish { targets: 1, resolve: 2 },],
        "draw(A), resolve, draw(B), resolve: {got:?}"
    );
}

#[test]
fn two_resolves_of_one_fbo_into_two_targets() {
    let _g = context();
    let d1 = framebuffer(0);
    let d2 = framebuffer(0);
    let src = framebuffer(4);
    program();
    settle();
    draw_to(src.fb);
    assert_eq!(resolve(src.fb, d1.fb), GL_NO_ERROR);
    assert_eq!(resolve(src.fb, d2.fb), GL_NO_ERROR);
    assert!(is_resolve(d1.fb, 1));
    assert!(is_resolve(d2.fb, 1), "both targets get the one resolve of the one draw");
    // The second blit finds no job to wait for: one draw, one resolving finish.
    let got = order();
    assert_eq!(got.iter().filter(|e| matches!(e, Ev::Draw(_))).count(), 1, "{got:?}");
    assert_eq!(got.iter().filter(|e| matches!(e, Ev::Finish { resolve: 1, .. })).count(), 1, "{got:?}");
    assert_eq!(got[0], D4, "the draw comes first: {got:?}");
    // A draw between them makes the second target a newer resolve.
    draw_to(src.fb);
    assert_eq!(resolve(src.fb, d2.fb), GL_NO_ERROR);
    assert!(is_resolve(d1.fb, 1), "the first target keeps its resolve");
    assert!(is_resolve(d2.fb, 2), "the second takes the new one");
}

#[test]
fn resolve_with_no_draw_since_the_last_one() {
    let _g = context();
    let dst = framebuffer(0);
    let src = framebuffer(4);
    program();
    settle();
    draw_to(src.fb);
    assert_eq!(resolve(src.fb, dst.fb), GL_NO_ERROR);
    EVENTS.lock().unwrap().clear();
    assert_eq!(resolve(src.fb, dst.fb), GL_NO_ERROR);
    let got = order();
    // Pinned: the repeated blit draws nothing and resolves nothing new at the
    // back end (no 4x target is pending); any finish it makes writes none.
    assert!(!got.iter().any(|e| matches!(e, Ev::Draw(_))), "no draw reaches the back end: {got:?}");
    assert!(got.is_empty(), "no 4x target is resolved again: {got:?}");
    assert!(is_resolve(dst.fb, 1), "the destination still holds the one resolve");
}

#[test]
fn draw_into_the_destination_between_two_resolves() {
    let _g = context();
    let dst = framebuffer(0);
    let src = framebuffer(4);
    program();
    settle();
    draw_to(src.fb);
    assert_eq!(resolve(src.fb, dst.fb), GL_NO_ERROR);
    // A 1x draw into the destination, then a second 4x draw and resolve.
    draw_to(dst.fb);
    draw_to(src.fb);
    assert_eq!(resolve(src.fb, dst.fb), GL_NO_ERROR);
    assert!(is_resolve(dst.fb, 2), "the second resolve lands over the destination's own draw");
    let got = order();
    assert_eq!(
        got,
        [D4, Ev::Finish { targets: 1, resolve: 1 }, Ev::Draw(1), D4, Ev::Finish { targets: 1, resolve: 2 },],
        "{got:?}"
    );
}

#[test]
fn flush_and_finish_between_steps_keep_the_order() {
    let _g = context();
    let dst = framebuffer(0);
    let src = framebuffer(4);
    program();
    settle();
    // SAFETY: plain GL calls.
    unsafe {
        draw_to(src.fb);
        glFlush();
        assert_eq!(resolve(src.fb, dst.fb), GL_NO_ERROR);
        glFlush();
        draw_to(src.fb);
        glFinish();
        glFlush();
    }
    assert_eq!(resolve(src.fb, dst.fb), GL_NO_ERROR);
    // SAFETY: plain GL call.
    unsafe { glFinish() };
    assert!(is_resolve(dst.fb, 2));
    let got = order();
    let draws: Vec<_> = got.iter().filter(|e| matches!(e, Ev::Draw(_) | Ev::Finish { targets: 1.., .. })).collect();
    assert_eq!(
        draws,
        [&D4, &Ev::Finish { targets: 1, resolve: 1 }, &D4, &Ev::Finish { targets: 1, resolve: 2 }],
        "{got:?}"
    );
}
