// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! ES 3.0 multiple render targets, the GL core's half, on the host: four
//! draw buffers (ES 3.0 Table 6.34 asks GL_MAX_DRAW_BUFFERS and
//! GL_MAX_COLOR_ATTACHMENTS of at least 4; docs/es3-cts-failures.md RC14).
//!
//! The count is the back end's (`backend::draw_targets`): every back end of
//! this repository still draws three, so the stand-in back end here claims
//! four through `backend::HOST_DRAW_TARGETS`, as a back end that can will
//! through `backend::DRAW_TARGETS`. What is held:
//!
//! - the queries: GL_MAX_DRAW_BUFFERS, GL_MAX_COLOR_ATTACHMENTS and
//!   GL_DRAW_BUFFER0..3 follow the back end, and fall back to the build's
//!   own count when the claim goes;
//! - glDrawBuffers and glReadBuffer state and errors, ES 3.0 4.2.1 / 4.3.1;
//! - GL_COLOR_ATTACHMENT3 is an attachment point and a complete framebuffer
//!   with four attachments hands all four to the back end, for a clear, a
//!   glClearBuffer and (with the in-process compiler) a draw;
//! - what does not write is dropped from the list the back end gets: a draw
//!   buffer 0 that is GL_NONE (or names an attachment with no image) moves
//!   the list to the first written attachment, with `targets[0].out_shift`
//!   saying how many it dropped (review R-09), and a framebuffer with no
//!   colour attachment 0 is complete on that list;
//! - the shader's `gl_MaxDrawBuffers` is the query's 4.
//!
//! An ES 2.0-reporting build (`report-es2`) must answer as before:
//! `es2_build_keeps_its_answers` is the only test it runs.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_mrt_gl
//! ```

#![cfg(feature = "host-test")]
// the ES 2.0 report runs one test of the file, and the draws need the in-process
// compiler; the helpers of the rest are idle there
#![cfg_attr(any(feature = "report-es2", not(feature = "shaderc-online")), allow(dead_code, unused_imports))]

use core::ffi::{c_char, c_void};
use std::sync::atomic::Ordering;
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{self, GlesDraw, GlesHeap, YakoglBackend, GLES_MAX_TARGETS};
use yakogl::gl::consts::*;
use yakogl::gl::es3::*;
use yakogl::gl::framebuffer::*;
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

/// What the back end was handed, one entry a draw (or clear).
#[derive(Clone, Debug)]
struct Seen {
    do_clear: bool,
    clear_target_mask: u8,
    /// An application draw (vertices), not a clear alone.
    draws: bool,
    /// `targets[..num_targets]`: (gpu, w, h, format).
    targets: Vec<(u64, u32, u32, u8)>,
    /// `targets[0].out_shift`: how many leading draw buffers the list
    /// dropped; target `k` takes fragment output `k + out_shift`.
    out_shift: u8,
}

static SEEN: Mutex<Vec<Seen>> = Mutex::new(Vec::new());

unsafe extern "C" fn b_draw(d: *const GlesDraw) -> i32 {
    let d = &*d;
    let n = (d.num_targets.max(1) as usize).min(GLES_MAX_TARGETS);
    SEEN.lock().unwrap().push(Seen {
        do_clear: d.do_clear,
        clear_target_mask: d.clear_target_mask,
        draws: d.index_count != 0,
        targets: d.targets[..n].iter().map(|t| (t.gpu, t.w, t.h, t.format)).collect(),
        out_shift: d.targets[0].out_shift,
    });
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
    name: c"es3-mrt".as_ptr() as *const c_char,
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

/// One context for the whole file; the tests take turns on it.
static LOCK: Mutex<()> = Mutex::new(());
static ONCE: Once = Once::new();

/// The context, with the stand-in back end claiming `targets` colour
/// targets a draw (0: the build's own count, `backend::DRAW_TARGETS`).
fn context_with(targets: u8) -> MutexGuard<'static, ()> {
    let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    backend::HOST_DRAW_TARGETS.store(targets, Ordering::Relaxed);
    ONCE.call_once(|| {
        // SAFETY: both tables are statics; the lock above serialises the tests.
        unsafe {
            osal_set_ops(&OSAL);
            backend::g_backend = &BACKEND;
            #[cfg(feature = "shaderc-online")]
            {
                yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
            }
            assert!(!yakogl::gl::context::gl_context_create(8, 8).is_null());
        }
    });
    /* what an earlier test left recorded is settled before this one counts */
    // SAFETY: plain GL calls.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glFinish();
    }
    SEEN.lock().unwrap().clear();
    drain_errors();
    g
}

/// The context with a four-target back end, ES 3.0's minimum.
fn context() -> MutexGuard<'static, ()> {
    context_with(4)
}

fn drain_errors() {
    // SAFETY: plain GL call.
    while unsafe { glGetError() } != GL_NO_ERROR {}
}

fn err() -> GLenum {
    // SAFETY: plain GL call.
    unsafe { glGetError() }
}

/// glGetIntegerv's answer, or its error.
fn geti(pname: GLenum) -> Result<GLint, GLenum> {
    let mut v: GLint = -777;
    // SAFETY: one GLint out.
    unsafe { glGetIntegerv(pname, &mut v) };
    match err() {
        GL_NO_ERROR => Ok(v),
        e => Err(e),
    }
}

/// glDrawBuffers with `bufs`, and the error it raised.
fn draw_buffers(bufs: &[GLenum]) -> GLenum {
    // SAFETY: `bufs.len()` readable enums.
    unsafe { glDrawBuffers(bufs.len() as GLsizei, bufs.as_ptr()) };
    err()
}

/// GL_DRAW_BUFFER0..3 of the bound draw framebuffer.
fn draw_buffer_state() -> [GLenum; 4] {
    core::array::from_fn(|i| geti(GL_DRAW_BUFFER0 + i as GLenum).expect("GL_DRAW_BUFFERi") as GLenum)
}

fn read_buffer(src: GLenum) -> GLenum {
    // SAFETY: plain GL call.
    unsafe { glReadBuffer(src) };
    err()
}

fn renderbuffer(w: GLsizei, h: GLsizei) -> GLuint {
    let mut rb = 0;
    // SAFETY: room for one name.
    unsafe {
        glGenRenderbuffers(1, &mut rb);
        glBindRenderbuffer(GL_RENDERBUFFER, rb);
        glRenderbufferStorage(GL_RENDERBUFFER, GL_RGBA8, w, h);
    }
    rb
}

/// A bound framebuffer with an 8x8 RGBA8 renderbuffer on each of colour
/// attachments 0..n.
fn framebuffer(n: usize) -> (GLuint, Vec<GLuint>) {
    let rbs: Vec<GLuint> = (0..n).map(|_| renderbuffer(8, 8)).collect();
    let mut fb = 0;
    // SAFETY: room for one name.
    unsafe {
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        for (i, rb) in rbs.iter().enumerate() {
            glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0 + i as GLenum, GL_RENDERBUFFER, *rb);
        }
    }
    (fb, rbs)
}

fn release(fb: GLuint, rbs: &[GLuint]) {
    // SAFETY: names this test made.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glDeleteFramebuffers(1, &fb);
        glDeleteRenderbuffers(rbs.len() as GLsizei, rbs.as_ptr());
    }
}

fn status() -> GLenum {
    // SAFETY: plain GL call.
    unsafe { glCheckFramebufferStatus(GL_FRAMEBUFFER) }
}

/// What the back end saw since the last call, after the GL handed it all.
fn seen() -> Vec<Seen> {
    // SAFETY: plain GL call.
    unsafe { glFinish() };
    core::mem::take(&mut *SEEN.lock().unwrap())
}

/// The GPU address of renderbuffer `rb`'s storage, as the draw names it:
/// the address of attachment `i` of the bound framebuffer, read from a clear.
fn target_addresses(n: usize) -> Vec<u64> {
    // SAFETY: plain GL calls.
    unsafe {
        glClearColor(0.0, 0.0, 0.0, 1.0);
        glClear(GL_COLOR_BUFFER_BIT);
    }
    let s = seen();
    let last = s.last().expect("the clear reached the back end");
    last.targets.iter().take(n).map(|t| t.0).collect()
}

/* ---------------------------------------------------------------- ES 2.0 -- */

/// The ES 2.0 report answers as before, whatever the back end can draw: no
/// GL_MAX_DRAW_BUFFERS or GL_DRAW_BUFFERi (GL_EXT_draw_buffers is not
/// claimed), and GL_MAX_COLOR_ATTACHMENTS(_NV) is the back end's point
/// count, 3 for the build's own.
#[cfg(feature = "report-es2")]
#[test]
fn es2_build_keeps_its_answers() {
    let _g = context_with(0);
    assert_eq!(geti(GL_MAX_DRAW_BUFFERS), Err(GL_INVALID_ENUM));
    for i in 0..4 {
        assert_eq!(geti(GL_DRAW_BUFFER0 + i), Err(GL_INVALID_ENUM), "GL_DRAW_BUFFER{i}");
    }
    assert_eq!(geti(GL_MAX_COLOR_ATTACHMENTS), Ok(backend::DRAW_TARGETS as GLint));
    assert_eq!(backend::DRAW_TARGETS, 3, "every back end draws three until its own half lands");
    backend::HOST_DRAW_TARGETS.store(4, Ordering::Relaxed);
    assert_eq!(geti(GL_MAX_DRAW_BUFFERS), Err(GL_INVALID_ENUM), "a four-target back end changes nothing");
    backend::HOST_DRAW_TARGETS.store(0, Ordering::Relaxed);
}

/* ------------------------------------------------------------- ES 3.0 -- */

#[cfg(not(feature = "report-es2"))]
mod es3 {
    use super::*;

    /// The limits are the back end's: 4 with a four-target back end, the
    /// build's own count (3 on every GPU today) without one.
    #[test]
    fn the_limits_follow_the_back_end() {
        let _g = context();
        assert_eq!(geti(GL_MAX_DRAW_BUFFERS), Ok(4));
        assert_eq!(geti(GL_MAX_COLOR_ATTACHMENTS), Ok(4));
        assert_eq!(geti(GL_DRAW_BUFFER0 + 3), Ok(GL_NONE as GLint), "the window's draw buffer 3");
        assert_eq!(geti(GL_DRAW_BUFFER0 + 4), Err(GL_INVALID_ENUM), "past GL_MAX_DRAW_BUFFERS");
        backend::HOST_DRAW_TARGETS.store(0, Ordering::Relaxed);
        let own = backend::DRAW_TARGETS as GLint;
        assert_eq!(own, 3);
        assert_eq!(geti(GL_MAX_DRAW_BUFFERS), Ok(own));
        assert_eq!(geti(GL_MAX_COLOR_ATTACHMENTS), Ok(own));
        assert_eq!(geti(GL_DRAW_BUFFER0 + own as GLenum), Err(GL_INVALID_ENUM));
        backend::HOST_DRAW_TARGETS.store(4, Ordering::Relaxed);
    }

    /// Attachment point 3 exists with four targets, a framebuffer with all
    /// four is complete, and point 4 is no point. A three-target back end
    /// has no point 3: nothing can be attached that it would not draw.
    #[test]
    fn colour_attachment_3_is_a_point() {
        let _g = context();
        let (fb, rbs) = framebuffer(4);
        assert_eq!(err(), GL_NO_ERROR, "GL_COLOR_ATTACHMENT0..3");
        assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE);
        let mut v: GLint = -1;
        // SAFETY: plain GL calls; one GLint out.
        unsafe {
            glGetFramebufferAttachmentParameteriv(
                GL_FRAMEBUFFER,
                GL_COLOR_ATTACHMENT3,
                GL_FRAMEBUFFER_ATTACHMENT_OBJECT_NAME,
                &mut v,
            );
            assert_eq!((err(), v), (GL_NO_ERROR, rbs[3] as GLint));
            glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0 + 4, GL_RENDERBUFFER, rbs[0]);
            assert_ne!(err(), GL_NO_ERROR, "GL_COLOR_ATTACHMENT4 is past GL_MAX_COLOR_ATTACHMENTS");
        }
        release(fb, &rbs);

        backend::HOST_DRAW_TARGETS.store(0, Ordering::Relaxed);
        let (fb, rbs) = framebuffer(0);
        let rb = renderbuffer(8, 8);
        // SAFETY: plain GL call.
        unsafe { glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT3, GL_RENDERBUFFER, rb) };
        assert_ne!(err(), GL_NO_ERROR, "a three-target back end has no point 3");
        release(fb, &[rb]);
        drop(rbs);
        backend::HOST_DRAW_TARGETS.store(4, Ordering::Relaxed);
    }

    /// glDrawBuffers on a framebuffer object, ES 3.0 4.2.1: the errors, in
    /// the order dEQP-GLES3 negative_api.buffer.draw_buffers checks them,
    /// and none of them changes the state.
    #[test]
    fn draw_buffers_errors_on_an_object() {
        let _g = context();
        let (fb, rbs) = framebuffer(4);
        let all = [GL_COLOR_ATTACHMENT0, GL_COLOR_ATTACHMENT1, GL_COLOR_ATTACHMENT2, GL_COLOR_ATTACHMENT3];
        assert_eq!(draw_buffer_state(), all, "the initial mapping: output i to attachment i");
        assert_eq!(draw_buffers(&[all[0], GL_NONE, all[2], GL_NONE]), GL_NO_ERROR);
        let kept = [all[0], GL_NONE, all[2], GL_NONE];
        assert_eq!(draw_buffer_state(), kept);

        // SAFETY: plain GL calls; at most `n` enums are read.
        unsafe {
            glDrawBuffers(-1, all.as_ptr());
            assert_eq!(err(), GL_INVALID_VALUE, "n < 0");
        }
        let five = [all[0], all[1], all[2], all[3], GL_NONE];
        assert_eq!(draw_buffers(&five), GL_INVALID_VALUE, "n past GL_MAX_DRAW_BUFFERS");
        let five_bad = [all[0], all[1], all[2], all[3], GL_DEPTH_ATTACHMENT];
        assert_eq!(draw_buffers(&five_bad), GL_INVALID_VALUE, "n is checked before the enums");
        assert_eq!(draw_buffers(&[all[0], GL_DEPTH_ATTACHMENT]), GL_INVALID_ENUM, "not a draw buffer");
        assert_eq!(draw_buffers(&[GL_BACK]), GL_INVALID_OPERATION, "GL_BACK on an object");
        assert_eq!(draw_buffers(&[all[1]]), GL_INVALID_OPERATION, "buffer 0 names attachment 1");
        assert_eq!(draw_buffers(&[all[0], all[1], all[3], all[2]]), GL_INVALID_OPERATION, "out of order");
        /* a COLOR_ATTACHMENTm is one of the values (Table 4.4); m at or past
         * GL_MAX_COLOR_ATTACHMENTS is GL_INVALID_OPERATION */
        assert_eq!(draw_buffers(&[all[0], GL_COLOR_ATTACHMENT0 + 4]), GL_INVALID_OPERATION, "attachment 4");
        assert_eq!(draw_buffer_state(), kept, "no error changed the state");
        release(fb, &rbs);
    }

    /// glDrawBuffers' state on a framebuffer object: all four, "beyond n is
    /// set to NONE", n = 0 sets every one to NONE (ES 3.0 4.2.1).
    #[test]
    fn draw_buffers_state_on_an_object() {
        let _g = context();
        let (fb, rbs) = framebuffer(4);
        let all = [GL_COLOR_ATTACHMENT0, GL_COLOR_ATTACHMENT1, GL_COLOR_ATTACHMENT2, GL_COLOR_ATTACHMENT3];
        assert_eq!(draw_buffers(&all), GL_NO_ERROR);
        assert_eq!(draw_buffer_state(), all);
        assert_eq!(draw_buffers(&[GL_NONE, all[1]]), GL_NO_ERROR);
        assert_eq!(draw_buffer_state(), [GL_NONE, all[1], GL_NONE, GL_NONE], "beyond n is NONE");
        assert_eq!(draw_buffers(&[]), GL_NO_ERROR);
        assert_eq!(draw_buffer_state(), [GL_NONE; 4], "n = 0: every one NONE");
        release(fb, &rbs);
    }

    /// The default framebuffer takes one buffer, GL_BACK or GL_NONE.
    #[test]
    fn draw_buffers_on_the_window() {
        let _g = context();
        assert_eq!(draw_buffer_state(), [GL_BACK, GL_NONE, GL_NONE, GL_NONE]);
        assert_eq!(draw_buffers(&[GL_BACK, GL_NONE]), GL_INVALID_OPERATION, "n = 2");
        assert_eq!(draw_buffers(&[GL_COLOR_ATTACHMENT0]), GL_INVALID_OPERATION, "an attachment");
        assert_eq!(draw_buffers(&[GL_NONE]), GL_NO_ERROR);
        assert_eq!(draw_buffer_state()[0], GL_NONE);
        assert_eq!(draw_buffers(&[GL_BACK]), GL_NO_ERROR);
        assert_eq!(draw_buffer_state()[0], GL_BACK);
    }

    /// glReadBuffer, ES 3.0 4.3.1: an object reads any attachment below
    /// GL_MAX_COLOR_ATTACHMENTS (3 included) or none; the window GL_BACK or
    /// none.
    #[test]
    fn read_buffer_takes_attachment_3() {
        let _g = context();
        let (fb, rbs) = framebuffer(4);
        assert_eq!(read_buffer(GL_COLOR_ATTACHMENT3), GL_NO_ERROR);
        assert_eq!(geti(GL_READ_BUFFER), Ok(GL_COLOR_ATTACHMENT3 as GLint));
        assert_eq!(read_buffer(GL_COLOR_ATTACHMENT0 + 4), GL_INVALID_OPERATION, "past the points");
        assert_eq!(read_buffer(GL_BACK), GL_INVALID_OPERATION, "GL_BACK on an object");
        assert_eq!(read_buffer(GL_DEPTH_ATTACHMENT), GL_INVALID_ENUM);
        assert_eq!(geti(GL_READ_BUFFER), Ok(GL_COLOR_ATTACHMENT3 as GLint), "no error changed it");
        assert_eq!(read_buffer(GL_NONE), GL_NO_ERROR);
        assert_eq!(geti(GL_READ_BUFFER), Ok(GL_NONE as GLint));
        release(fb, &rbs);
        assert_eq!(read_buffer(GL_COLOR_ATTACHMENT0), GL_INVALID_OPERATION, "the window");
        assert_eq!(read_buffer(GL_BACK), GL_NO_ERROR);
    }

    /// A clear of a framebuffer with four attachments reaches the back end
    /// with all four targets; glClearBuffer(GL_COLOR, 3) names target 3
    /// alone, and draw buffer 4 is GL_INVALID_VALUE.
    #[test]
    fn a_clear_hands_the_back_end_four_targets() {
        let _g = context();
        let (fb, rbs) = framebuffer(4);
        let gpus = target_addresses(4);
        assert_eq!(gpus.len(), 4, "four targets");
        assert!(gpus.iter().all(|&g| g != 0), "{gpus:x?}");
        let mut uniq = gpus.clone();
        uniq.sort_unstable();
        uniq.dedup();
        assert_eq!(uniq.len(), 4, "four distinct storages");

        let red = [1.0f32, 0.0, 0.0, 1.0];
        // SAFETY: four readable floats.
        unsafe { glClearBufferfv(GL_COLOR, 3, red.as_ptr()) };
        assert_eq!(err(), GL_NO_ERROR, "draw buffer 3");
        let s = seen();
        let c = s.last().expect("the clear reached the back end");
        assert!(c.do_clear);
        assert_eq!(c.clear_target_mask, 1 << 3, "target 3 alone");
        assert_eq!(c.targets.get(3).map(|t| t.0), Some(gpus[3]));
        // SAFETY: four readable floats.
        unsafe { glClearBufferfv(GL_COLOR, 4, red.as_ptr()) };
        assert_eq!(err(), GL_INVALID_VALUE, "draw buffer 4");
        release(fb, &rbs);
    }

    /// The framebuffer with no colour attachment 0 (the other form of review
    /// R-09): ES 3.0 4.4.4.2 asks nothing of attachment 0, so it is complete,
    /// and the frame's own target is the first attachment a draw buffer
    /// writes. The back end gets the dense list (`framebuffer::frame_attachment`,
    /// `draw::resolve_targets`): two targets, attachments 1 and 2, with
    /// `out_shift` 1; a `glClearBufferfv(GL_COLOR, n)` lands on target n - 1,
    /// and one naming the dropped draw buffer 0 touches nothing. A draw into
    /// such a framebuffer is the `draw` module's test.
    #[test]
    fn a_framebuffer_without_attachment_0_clears_through_attachment_1() {
        use yakogl::backend::GLES_SURF_RGBA8888;
        let _g = context();
        let rbs: Vec<GLuint> = (0..2).map(|_| renderbuffer(8, 8)).collect();
        let mut fb = 0;
        // SAFETY: room for one name, and the renderbuffers are this test's.
        unsafe {
            glGenFramebuffers(1, &mut fb);
            glBindFramebuffer(GL_FRAMEBUFFER, fb);
            for (i, rb) in rbs.iter().enumerate() {
                glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT1 + i as GLenum, GL_RENDERBUFFER, *rb);
            }
        }
        assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE, "ES 3.0 asks nothing of attachment 0");

        /* glClear touches the written draw buffers; buffer 0 names an
         * attachment with no image, and writes nothing (ES 3.0 4.2.1) */
        // SAFETY: plain GL calls.
        unsafe {
            glClearColor(0.2, 0.0, 0.0, 1.0);
            glClear(GL_COLOR_BUFFER_BIT);
        }
        let c = seen().pop().expect("the clear reached the back end");
        assert_eq!(c.out_shift, 1, "attachment 1 is the frame target, not attachment 0");
        assert_eq!(c.targets.len(), 2, "attachment 0 is not in the list at all");
        for t in &c.targets {
            assert!(t.0 != 0 && (t.1, t.2, t.3) == (8, 8, GLES_SURF_RGBA8888), "{t:?}");
        }
        let (a1, a2) = (c.targets[0].0, c.targets[1].0);

        let red = [1.0f32, 0.0, 0.0, 1.0];
        /* draw buffer 1 is target 0, and draw buffer 2 is target 1 */
        // SAFETY: four readable floats.
        unsafe { glClearBufferfv(GL_COLOR, 1, red.as_ptr()) };
        assert_eq!(err(), GL_NO_ERROR);
        let c = seen().pop().expect("the clear reached the back end");
        assert_eq!((c.out_shift, c.clear_target_mask), (1, 1), "draw buffer 1 is target 0");
        assert_eq!(c.targets[0].0, a1, "the named target is attachment 1");

        // SAFETY: four readable floats.
        unsafe { glClearBufferfv(GL_COLOR, 2, red.as_ptr()) };
        assert_eq!(err(), GL_NO_ERROR);
        let c = seen().pop().expect("the clear reached the back end");
        assert_eq!(c.clear_target_mask, 2, "draw buffer 2 is target 1");
        assert_eq!(c.targets[1].0, a2, "the named target is attachment 2");

        /* the dropped draw buffer 0: its bit moves out of the list, and the
         * clear names nothing (`clear_target_mask`'s empty value, bit 7) */
        // SAFETY: four readable floats.
        unsafe { glClearBufferfv(GL_COLOR, 0, red.as_ptr()) };
        assert_eq!(err(), GL_NO_ERROR);
        let c = seen().pop().expect("the clear reached the back end");
        assert_eq!(c.clear_target_mask, 1 << 7, "draw buffer 0 is not in the list");
        assert!(c.targets.iter().all(|t| t.0 != 0), "the list itself is unchanged");
        release(fb, &rbs);
    }

    #[cfg(feature = "shaderc-online")]
    mod draw {
        use super::*;
        use std::ffi::CString;
        use yakogl::backend::GLES_SURF_RGBA8888;
        use yakogl::gl::buffer::glBindBuffer;
        use yakogl::gl::draw::*;
        use yakogl::gl::program::*;
        use yakogl::gl::shader::*;
        use yakogl::gl::vertex::{glEnableVertexAttribArray, glVertexAttribPointer};

        const VS: &str = "#version 300 es\nin vec4 p;\nvoid main() { gl_Position = p; }\n";

        fn shader(ty: GLenum, text: &str) -> (GLuint, bool) {
            let text = CString::new(text).unwrap();
            // SAFETY: plain GL calls; one NUL-terminated source.
            unsafe {
                let s = glCreateShader(ty);
                let p: [*const c_char; 1] = [text.as_ptr()];
                glShaderSource(s, 1, p.as_ptr(), core::ptr::null());
                glCompileShader(s);
                let mut ok: GLint = 0;
                glGetShaderiv(s, GL_COMPILE_STATUS, &mut ok);
                (s, ok == 1)
            }
        }

        /// A linked, current program of `VS` and `fs`.
        fn program(fs: &str) {
            let (v, vok) = shader(GL_VERTEX_SHADER, VS);
            let (f, fok) = shader(GL_FRAGMENT_SHADER, fs);
            assert!(vok && fok, "{fs:?} did not compile");
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

        fn draw_triangle() {
            static TRI: [f32; 12] = [-1.0, -1.0, 0.0, 1.0, 3.0, -1.0, 0.0, 1.0, -1.0, 3.0, 0.0, 1.0];
            // SAFETY: TRI is static and outlives the draw.
            unsafe {
                glBindBuffer(GL_ARRAY_BUFFER, 0);
                glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, TRI.as_ptr() as *const c_void);
                glEnableVertexAttribArray(0);
                glDrawArrays(GL_TRIANGLES, 0, 3);
            }
        }

        /// GLSL ES 3.00's gl_MaxDrawBuffers is the query's value: an array
        /// of `gl_MaxDrawBuffers - 3` elements and one of `5 -
        /// gl_MaxDrawBuffers` both have a positive size only when it is 4.
        #[test]
        fn gl_max_draw_buffers_is_four() {
            let _g = context();
            let fs = "#version 300 es\nprecision mediump float;\nout vec4 o;\n\
                      void main() { float a[gl_MaxDrawBuffers - 3]; float b[5 - gl_MaxDrawBuffers];\n\
                      a[0] = 1.0; b[0] = 0.0; o = vec4(a[0], b[0], 0.0, 1.0); }\n";
            assert!(shader(GL_FRAGMENT_SHADER, fs).1, "gl_MaxDrawBuffers is not 4");
        }

        /// A draw into a framebuffer with four attachments hands the back end
        /// four targets in attachment order; glDrawBuffers(ATT0, NONE, ATT2,
        /// ATT3) keeps slot 1 with no target (gpu 0) and carries 2 and 3.
        #[test]
        fn a_draw_hands_the_back_end_four_targets() {
            let _g = context();
            let (fb, rbs) = framebuffer(4);
            let gpus = target_addresses(4);
            program(
                "#version 300 es\nprecision mediump float;\nlayout(location = 0) out vec4 o;\n\
                 void main() { o = vec4(1.0); }\n",
            );
            draw_triangle();
            assert_eq!(err(), GL_NO_ERROR);
            let s = seen();
            let d = s.iter().rev().find(|s| s.draws).expect("the draw reached the back end");
            assert_eq!(d.targets.len(), 4, "num_targets");
            assert_eq!(d.targets.iter().map(|t| t.0).collect::<Vec<_>>(), gpus);
            assert!(d.targets.iter().all(|t| (t.1, t.2, t.3) == (8, 8, GLES_SURF_RGBA8888)));

            assert_eq!(draw_buffers(&[GL_COLOR_ATTACHMENT0, GL_NONE, GL_COLOR_ATTACHMENT2, GL_COLOR_ATTACHMENT3]), 0);
            draw_triangle();
            let s = seen();
            let d = s.iter().rev().find(|s| s.draws).expect("the draw reached the back end");
            let got: Vec<u64> = d.targets.iter().map(|t| t.0).collect();
            assert_eq!(got, [gpus[0], 0, gpus[2], gpus[3]]);
            assert_eq!(d.out_shift, 0, "slot 0 is written: nothing moves");
            // SAFETY: plain GL call.
            unsafe { glUseProgram(0) };
            release(fb, &rbs);
        }

        /// glDrawBuffers({GL_NONE, GL_COLOR_ATTACHMENT1}) (review R-09): the
        /// list the back end gets is dense. It starts at the first draw
        /// buffer that writes, so one target — attachment 1 — with
        /// `out_shift` 1 (it takes fragment output 1), and the draw's own
        /// frame target is that attachment's storage, not attachment 0's.
        /// {GL_NONE, GL_NONE, GL_COLOR_ATTACHMENT2} drops two draw buffers
        /// and clears draw buffer 2 onto target 0. The back ends that read
        /// their output registers from output 0 refuse such a draw instead
        /// (src/pvr, src/v3d, src/mali); the GC7000 draws it
        /// (src/vivante/backend_tests.rs).
        #[test]
        fn a_dropped_draw_buffer_0_hands_the_back_end_one_dense_target() {
            let _g = context();
            let (fb, rbs) = framebuffer(4);
            let gpus = target_addresses(4);
            program(
                "#version 300 es\nprecision mediump float;\nlayout(location = 0) out vec4 o;\n\
                 void main() { o = vec4(1.0); }\n",
            );

            assert_eq!(draw_buffers(&[GL_NONE, GL_COLOR_ATTACHMENT1]), GL_NO_ERROR);
            draw_triangle();
            let d = seen().iter().rev().find(|s| s.draws).expect("the draw reached the back end").clone();
            assert_eq!((d.targets.len(), d.out_shift), (1, 1), "one target, from the first written slot");
            assert_eq!(d.targets[0].0, gpus[1], "attachment 1 is the frame target");
            assert_eq!((d.targets[0].1, d.targets[0].2, d.targets[0].3), (8, 8, GLES_SURF_RGBA8888));

            /* two dropped draw buffers */
            assert_eq!(draw_buffers(&[GL_NONE, GL_NONE, GL_COLOR_ATTACHMENT2]), GL_NO_ERROR);
            draw_triangle();
            let d = seen().iter().rev().find(|s| s.draws).expect("the draw reached the back end").clone();
            assert_eq!((d.targets.len(), d.out_shift, d.targets[0].0), (1, 2, gpus[2]));

            /* glClearBuffer(GL_COLOR, 2) names draw buffer 2: target 0 of
             * that list */
            let red = [1.0f32, 0.0, 0.0, 1.0];
            // SAFETY: four readable floats.
            unsafe { glClearBufferfv(GL_COLOR, 2, red.as_ptr()) };
            assert_eq!(err(), GL_NO_ERROR);
            let c = seen().pop().expect("the clear reached the back end");
            assert_eq!((c.out_shift, c.clear_target_mask, c.targets[0].0), (2, 1, gpus[2]));
            // SAFETY: plain GL call.
            unsafe { glUseProgram(0) };
            release(fb, &rbs);
        }
    }
}
