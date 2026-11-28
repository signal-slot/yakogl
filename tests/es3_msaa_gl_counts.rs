// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! ES 3.0 multisample renderbuffers, the GL core's half: every sample count
//! follows the back end's `YakoglBackend.max_samples`, not a hard-coded 4.
//! Three file-local stand-in back ends say 2, 4 and 8 (the helpers are copied
//! from tests/es3_msaa_gl_draws.rs); the same checks run on each: GL_MAX_SAMPLES,
//! glGetInternalformativ GL_NUM_SAMPLE_COUNTS / GL_SAMPLES, what
//! glRenderbufferStorageMultisample stores (`GL_RENDERBUFFER_SAMPLES`) and
//! refuses, GL_SAMPLES of the bound framebuffer and `GlesDraw::samples`.
//!
//! ES 3.0 requires GL_MAX_SAMPLES >= 4, so a back end with 2 is not
//! conformant; the 2 stand-in only shows what the core derives from the field.
//!
//! What the core does today (src/gl/msaa.rs): GL_MAX_SAMPLES is `max_samples`
//! (so 2 and 8 are reported as they are); a request above it is
//! GL_INVALID_OPERATION; but the one count a multisample renderbuffer is
//! stored with, and the one GL_SAMPLES lists, is 4 when `max_samples` >= 4
//! (`MS_SAMPLES`), and 1 below that. So 8 still keeps 4 samples and 2 keeps
//! one. Those are pinned as passing tests; the result the capability implies
//! (the largest supported count, `max_samples`, stored and listed) is in the
//! `#[ignore]`d tests, owner: the GL core (src/gl/msaa.rs `stored_samples`,
//! `internalformat_sample_counts`, `draw_samples` callers in src/gl). Rounding
//! of in-between requests at `max_samples` 4 is tests/es3_msaa_gl_rbsamples.rs.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_msaa_gl_counts
//! ```

#![allow(dead_code)]
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

/// `GlesDraw::samples` of every application draw the back end received.
static ALL: Mutex<Vec<u8>> = Mutex::new(Vec::new());

unsafe extern "C" fn b_draw(d: *const GlesDraw) -> i32 {
    let d = &*d;
    if d.index_count != 0 {
        ALL.lock().unwrap().push(d.samples);
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
    name: c"es3-msaa-gl-counts".as_ptr() as *const c_char,
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
    max_samples: 4,
};

/// The three stand-ins, by their `max_samples`.
static BACKEND_MS2: YakoglBackend = YakoglBackend { max_samples: 2, ..BACKEND };
static BACKEND_MS4: YakoglBackend = YakoglBackend { max_samples: 4, ..BACKEND };
static BACKEND_MS8: YakoglBackend = YakoglBackend { max_samples: 8, ..BACKEND };

static LOCK: Mutex<()> = Mutex::new(());
static ONCE: Once = Once::new();

/// The context with the stand-in whose `max_samples` is `max` (2, 4 or 8).
fn context(max: u32) -> MutexGuard<'static, ()> {
    let table: &'static YakoglBackend = match max {
        2 => &BACKEND_MS2,
        4 => &BACKEND_MS4,
        8 => &BACKEND_MS8,
        _ => panic!("no stand-in for {max}"),
    };
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
    // SAFETY: a static table; plain GL calls.
    unsafe {
        backend::g_backend = table;
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glFinish();
    }
    ALL.lock().unwrap().clear();
    // SAFETY: plain GL call.
    while unsafe { glGetError() } != GL_NO_ERROR {}
    g
}

fn err() -> GLenum {
    // SAFETY: plain GL call.
    unsafe { glGetError() }
}

fn geti(pname: GLenum) -> GLint {
    let mut v: GLint = -777;
    // SAFETY: one GLint out.
    unsafe { glGetIntegerv(pname, &mut v) };
    assert_eq!(err(), GL_NO_ERROR);
    v
}

/// The GL_SAMPLES list of `format` (as long as GL_NUM_SAMPLE_COUNTS says) and
/// the count.
fn counts(format: GLenum) -> (GLint, Vec<GLint>) {
    let mut n: GLint = -777;
    // SAFETY: one GLint out.
    unsafe { glGetInternalformativ(GL_RENDERBUFFER, format, GL_NUM_SAMPLE_COUNTS, 1, &mut n) };
    assert_eq!(err(), GL_NO_ERROR);
    let mut v = vec![-777; n.max(0) as usize + 1];
    // SAFETY: room for n + 1 GLints.
    unsafe { glGetInternalformativ(GL_RENDERBUFFER, format, GL_SAMPLES, v.len() as GLsizei, v.as_mut_ptr()) };
    assert_eq!(err(), GL_NO_ERROR);
    assert_eq!(v[n as usize], -777, "GL_SAMPLES wrote more than GL_NUM_SAMPLE_COUNTS");
    v.truncate(n as usize);
    (n, v)
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

/// glRenderbufferStorageMultisample on a fresh 8x8 RGBA8 renderbuffer, left
/// bound: the error and the name.
fn storage(samples: GLsizei) -> (GLenum, GLuint) {
    let mut rb = 0;
    // SAFETY: plain GL calls.
    unsafe {
        glGenRenderbuffers(1, &mut rb);
        glBindRenderbuffer(GL_RENDERBUFFER, rb);
        glRenderbufferStorageMultisample(GL_RENDERBUFFER, samples, GL_RGBA8, 8, 8);
    }
    (err(), rb)
}

fn rb_samples() -> GLint {
    let mut v: GLint = -777;
    // SAFETY: one GLint out.
    unsafe { glGetRenderbufferParameteriv(GL_RENDERBUFFER, GL_RENDERBUFFER_SAMPLES, &mut v) };
    assert_eq!(err(), GL_NO_ERROR);
    v
}

/// What a request stores: the error, or the GL_RENDERBUFFER_SAMPLES.
fn stored(samples: GLsizei) -> Result<GLint, GLenum> {
    let (e, rb) = storage(samples);
    let r = if e == GL_NO_ERROR { Ok(rb_samples()) } else { Err(e) };
    // SAFETY: a name this test made.
    unsafe { glDeleteRenderbuffers(1, &rb) };
    r
}

/// A bound complete 8x8 framebuffer with an RGBA8 renderbuffer of `samples`.
fn framebuffer(samples: GLsizei) -> Fbo {
    let mut fb = 0;
    // SAFETY: plain GL calls.
    unsafe {
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
    }
    let (e, rb) = storage(samples);
    assert_eq!(e, GL_NO_ERROR, "storage {samples}");
    // SAFETY: a renderbuffer this test made.
    unsafe { glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_RENDERBUFFER, rb) };
    assert_eq!(err(), GL_NO_ERROR);
    // SAFETY: plain GL call.
    assert_eq!(unsafe { glCheckFramebufferStatus(GL_FRAMEBUFFER) }, GL_FRAMEBUFFER_COMPLETE, "samples {samples}");
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

/// Use a one-output program and draw one triangle over the bound
/// framebuffer: the `GlesDraw::samples` the back end was handed.
fn draw_samples() -> u8 {
    static TRI: [f32; 12] = [-1.0, -1.0, 0.0, 1.0, 3.0, -1.0, 0.0, 1.0, -1.0, 3.0, 0.0, 1.0];
    let v = shader(GL_VERTEX_SHADER, "#version 300 es\nin vec4 p;\nvoid main() { gl_Position = p; }\n");
    let f = shader(
        GL_FRAGMENT_SHADER,
        "#version 300 es\nprecision highp float;\nlayout(location = 0) out vec4 o;\nvoid main() { o = vec4(0.25, 0.5, 0.75, 1.0); }\n",
    );
    ALL.lock().unwrap().clear();
    // SAFETY: plain GL calls; TRI is static and outlives the draw.
    unsafe {
        let p = glCreateProgram();
        glAttachShader(p, v);
        glAttachShader(p, f);
        glLinkProgram(p);
        glUseProgram(p);
        glBindBuffer(GL_ARRAY_BUFFER, 0);
        glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, TRI.as_ptr() as *const c_void);
        glEnableVertexAttribArray(0);
        glDrawArrays(GL_TRIANGLES, 0, 3);
        glFinish();
    }
    assert_eq!(err(), GL_NO_ERROR);
    *ALL.lock().unwrap().last().expect("the draw reached the back end")
}

/* ---- what the core does today ---------------------------------------------- */

/// GL_MAX_SAMPLES is the field, for 2 (not conformant: ES 3.0 wants >= 4), 4
/// and 8.
#[test]
fn max_samples_is_the_field() {
    for max in [2, 4, 8] {
        let _g = context(max);
        assert_eq!(geti(GL_MAX_SAMPLES), max as GLint, "max_samples {max}");
    }
}

/// A request above `max_samples` is GL_INVALID_OPERATION, one at it is not; a
/// negative one is GL_INVALID_VALUE; 0 is plain storage.
#[test]
fn requests_above_the_maximum_are_refused() {
    for max in [2, 4, 8] {
        let _g = context(max);
        let m = max as GLsizei;
        assert_eq!(stored(m + 1), Err(GL_INVALID_OPERATION), "max {max}: {}", m + 1);
        assert_eq!(stored(m + 100), Err(GL_INVALID_OPERATION), "max {max}: above");
        assert!(stored(m).is_ok(), "max {max}: at the maximum");
        assert_eq!(stored(-1), Err(GL_INVALID_VALUE), "max {max}: negative");
        assert_eq!(stored(0), Ok(0), "max {max}: 0 is plain storage");
    }
}

/// The counts listed today: descending, the 4 of `MS_SAMPLES` when the back
/// end keeps 4 or more (8 lists 4, not 8), the single 1 below that. The list
/// is as long as GL_NUM_SAMPLE_COUNTS says.
#[test]
fn listed_counts_today() {
    for (max, want) in [(2, vec![1]), (4, vec![4]), (8, vec![4])] {
        let _g = context(max);
        let (n, v) = counts(GL_RGBA8);
        assert_eq!(n as usize, want.len(), "max {max}");
        assert_eq!(v, want, "max {max}");
        assert!(v.windows(2).all(|w| w[0] > w[1]), "max {max}: descending");
        let (_, d) = counts(GL_DEPTH24_STENCIL8);
        assert_eq!(d, want, "max {max}: depth/stencil");
    }
}

/// The count stored today for every request from 1 to `max_samples`: 4 from
/// `max_samples` 4 on, 1 on the 2 stand-in.
#[test]
fn stored_counts_today() {
    for (max, want) in [(2, 1), (4, 4), (8, 4)] {
        let _g = context(max);
        for r in 1..=max as GLsizei {
            assert_eq!(stored(r), Ok(want), "max {max}: request {r}");
        }
    }
}

/// GL_SAMPLES / GL_SAMPLE_BUFFERS of a bound framebuffer, and the samples its
/// draw is handed, follow what was stored, today: the 2 stand-in keeps one
/// sample, 4 and 8 keep 4.
#[test]
fn framebuffer_samples_today() {
    for (max, want) in [(2, 1), (4, 4), (8, 4)] {
        let _g = context(max);
        let fbo = framebuffer(max as GLsizei);
        assert_eq!(geti(GL_SAMPLES), want, "max {max}: GL_SAMPLES");
        assert_eq!(geti(GL_SAMPLE_BUFFERS), 1, "max {max}: GL_SAMPLE_BUFFERS");
        assert_eq!(draw_samples() as GLint, want, "max {max}: GlesDraw.samples");
        drop(fbo);
        let fbo = framebuffer(0);
        assert_eq!(geti(GL_SAMPLES), 0, "max {max}: plain storage");
        assert_eq!(geti(GL_SAMPLE_BUFFERS), 0, "max {max}");
        assert_eq!(draw_samples(), 1, "max {max}: plain storage draws one sample");
        drop(fbo);
    }
}

/// A framebuffer's samples come from the storage, not from the back end:
/// switching the stand-in under a framebuffer made on another keeps what was
/// stored, and a request made on 8 and read on 4 is not above 4's maximum.
#[test]
fn counts_are_asked_at_the_call() {
    let _g = context(8);
    assert_eq!(geti(GL_MAX_SAMPLES), 8);
    drop(_g);
    let _g = context(2);
    assert_eq!(geti(GL_MAX_SAMPLES), 2);
    assert_eq!(stored(3), Err(GL_INVALID_OPERATION));
    drop(_g);
    let _g = context(8);
    assert!(stored(3).is_ok());
}

/* ---- the result the capability implies: ignored, owner the GL core -------- */

/// The largest supported count is `max_samples`: GL_SAMPLES lists it (alone,
/// as every stand-in keeps one count). Owner: src/gl/msaa.rs
/// (`internalformat_sample_counts` lists 4 for 8 and 1 for 2).
#[test]
#[ignore = "owner: GL core, src/gl/msaa.rs internalformat_sample_counts is fixed at MS_SAMPLES"]
fn listed_counts_contain_the_maximum() {
    for max in [2, 4, 8] {
        let _g = context(max);
        let (n, v) = counts(GL_RGBA8);
        assert!(n >= 1, "max {max}");
        assert_eq!(v[0], max as GLint, "max {max}: the list starts at max_samples");
        assert!(v.windows(2).all(|w| w[0] > w[1]), "max {max}: descending");
    }
}

/// A request at `max_samples` stores `max_samples`, and the framebuffer draws
/// with that many. Owner: src/gl/msaa.rs `stored_samples` (4 or 1 only).
#[test]
#[ignore = "owner: GL core, src/gl/msaa.rs stored_samples stores MS_SAMPLES or 1"]
fn a_request_at_the_maximum_stores_it() {
    for max in [2, 4, 8] {
        let _g = context(max);
        assert_eq!(stored(max as GLsizei), Ok(max as GLint), "max {max}");
        let _fbo = framebuffer(max as GLsizei);
        assert_eq!(geti(GL_SAMPLES), max as GLint, "max {max}: GL_SAMPLES");
        assert_eq!(draw_samples() as GLint, max as GLint, "max {max}: GlesDraw.samples");
    }
}

/// A request below the maximum rounds up to a supported count: with one count
/// per stand-in every request 1..=max stores `max_samples`. Owner: as above.
#[test]
#[ignore = "owner: GL core, src/gl/msaa.rs stored_samples stores MS_SAMPLES or 1"]
fn in_between_requests_round_up_to_the_maximum() {
    for max in [2, 4, 8] {
        let _g = context(max);
        for r in 1..=max as GLsizei {
            assert_eq!(stored(r), Ok(max as GLint), "max {max}: request {r}");
        }
    }
}
