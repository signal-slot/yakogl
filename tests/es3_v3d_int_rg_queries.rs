// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! V3D (Raspberry Pi 4): the ES 3.0 queries of R and RG integer attachments
//! (plan #383, gap G2 of docs/notes/v3d-format-matrix.md). ES 3.0 has no
//! glGetTexLevelParameter; what applies is
//! glGetIntegerv(GL_IMPLEMENTATION_COLOR_READ_FORMAT / _TYPE) with the format
//! bound as the read framebuffer, and glGetInternalformativ on GL_RENDERBUFFER.
//! glGetFramebufferAttachmentParameteriv is held by
//! tests/es3_v3d_int_rg_attach_query.rs and not repeated here.
//!
//! Held, for each of the twelve formats (R8I .. RG32UI) bound as
//! GL_READ_FRAMEBUFFER at GL_COLOR_ATTACHMENT0, as a texture and as a
//! renderbuffer:
//!
//! - GL_IMPLEMENTATION_COLOR_READ_FORMAT is GL_RGBA_INTEGER or the format's own
//!   GL_RED_INTEGER / GL_RG_INTEGER, and _TYPE is GL_INT (signed) or
//!   GL_UNSIGNED_INT (unsigned); glReadPixels with that pair is accepted; and
//!   the pair is the same for the texture and the renderbuffer of the format.
//!   Values the tip gives (pinned by `recorded_values_are_pinned`): for all
//!   twelve, texture and renderbuffer alike, GL_RGBA_INTEGER (0x8D99) with
//!   GL_INT (0x1404, *I) or GL_UNSIGNED_INT (0x1405, *UI)
//!   (`framebuffer::implementation_color_read`); never RED_INTEGER/RG_INTEGER;
//! - glGetError stays GL_NO_ERROR through the queries;
//! - with glReadBuffer(GL_NONE) the query should give GL_INVALID_OPERATION and
//!   leave the output untouched. DEFECT on the tip: `implementation_color_read`
//!   (src/gl/framebuffer.rs) falls back to RGBA/UNSIGNED_BYTE for a read buffer
//!   with no attachment and raises no error; the test is `#[ignore]`d (issue
//!   text in the report);
//! - glGetInternalformativ(GL_RENDERBUFFER, fmt, GL_NUM_SAMPLE_COUNTS) and
//!   GL_SAMPLES: an integer format is not multisample renderable in ES 3.0, so
//!   NUM_SAMPLE_COUNTS is 0 (this back end has `max_samples` 0).
//!
//! What it cannot see: the GPU; nothing here is verified on hardware.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_v3d_int_rg_queries
//! ```

#![cfg(all(feature = "host-test", feature = "shaderc-online", feature = "gpu-v3d", not(feature = "report-es2")))]

use core::ffi::{c_char, c_void};
use std::sync::atomic::Ordering;
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{self, GlesDraw, GlesHeap, YakoglBackend};
use yakogl::gl::consts::*;
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
unsafe extern "C" fn b_draw(_d: *const GlesDraw) -> i32 {
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
    name: c"es3-v3d-int-rg-queries".as_ptr() as *const c_char,
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
    // SAFETY: plain GL calls.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glColorMask(1, 1, 1, 1);
        glDisable(GL_BLEND);
        glFinish();
    }
    // SAFETY: plain GL call.
    while unsafe { glGetError() } != GL_NO_ERROR {}
    g
}

fn err() -> GLenum {
    // SAFETY: plain GL call.
    unsafe { glGetError() }
}

/* ---- the twelve formats -------------------------------------------------- */

#[derive(Clone, Copy)]
struct Row {
    name: &'static str,
    ifmt: GLenum,
    channels: u8,
    signed: bool,
}

const fn row(name: &'static str, ifmt: GLenum, channels: u8, signed: bool) -> Row {
    Row { name, ifmt, channels, signed }
}

const ROWS: [Row; 12] = [
    row("R8I", GL_R8I, 1, true),
    row("R8UI", GL_R8UI, 1, false),
    row("RG8I", GL_RG8I, 2, true),
    row("RG8UI", GL_RG8UI, 2, false),
    row("R16I", GL_R16I, 1, true),
    row("R16UI", GL_R16UI, 1, false),
    row("RG16I", GL_RG16I, 2, true),
    row("RG16UI", GL_RG16UI, 2, false),
    row("R32I", GL_R32I, 1, true),
    row("R32UI", GL_R32UI, 1, false),
    row("RG32I", GL_RG32I, 2, true),
    row("RG32UI", GL_RG32UI, 2, false),
];

#[derive(Clone, Copy, Debug, PartialEq)]
enum Kind {
    Renderbuffer,
    Texture2d,
}

/// A framebuffer with the format at attachment 0 bound as the read
/// framebuffer; dropped, it unbinds and deletes.
struct Fbo {
    fb: GLuint,
    tex: GLuint,
    rb: GLuint,
}

impl Drop for Fbo {
    fn drop(&mut self) {
        // SAFETY: names this test made.
        unsafe {
            glBindFramebuffer(GL_READ_FRAMEBUFFER, 0);
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
            glDeleteFramebuffers(1, &self.fb);
            glDeleteTextures(1, &self.tex);
            glDeleteRenderbuffers(1, &self.rb);
        }
    }
}

fn read_fbo(r: &Row, kind: Kind) -> Fbo {
    let mut f = Fbo { fb: 0, tex: 0, rb: 0 };
    // SAFETY: room for one name each.
    unsafe {
        glGenFramebuffers(1, &mut f.fb);
        glBindFramebuffer(GL_READ_FRAMEBUFFER, f.fb);
        match kind {
            Kind::Renderbuffer => {
                glGenRenderbuffers(1, &mut f.rb);
                glBindRenderbuffer(GL_RENDERBUFFER, f.rb);
                glRenderbufferStorage(GL_RENDERBUFFER, r.ifmt, 8, 8);
                glFramebufferRenderbuffer(GL_READ_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_RENDERBUFFER, f.rb);
            }
            Kind::Texture2d => {
                glGenTextures(1, &mut f.tex);
                glBindTexture(GL_TEXTURE_2D, f.tex);
                glTexStorage2D(GL_TEXTURE_2D, 1, r.ifmt, 8, 8);
                glFramebufferTexture2D(GL_READ_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, f.tex, 0);
            }
        }
        glReadBuffer(GL_COLOR_ATTACHMENT0);
    }
    assert_eq!(err(), GL_NO_ERROR, "{} {kind:?}: storage and attach", r.name);
    // SAFETY: plain GL call.
    let st = unsafe { glCheckFramebufferStatus(GL_READ_FRAMEBUFFER) };
    assert_eq!(st, GL_FRAMEBUFFER_COMPLETE, "{} {kind:?}", r.name);
    f
}

fn read_pair() -> (GLenum, GLenum) {
    let (mut fmt, mut ty): (GLint, GLint) = (0, 0);
    // SAFETY: one GLint each.
    unsafe {
        glGetIntegerv(GL_IMPLEMENTATION_COLOR_READ_FORMAT, &mut fmt);
        glGetIntegerv(GL_IMPLEMENTATION_COLOR_READ_TYPE, &mut ty);
    }
    (fmt as GLenum, ty as GLenum)
}

#[test]
fn read_pair_is_valid_for_every_format_and_error_free() {
    let _g = context();
    for r in &ROWS {
        for k in [Kind::Texture2d, Kind::Renderbuffer] {
            let _f = read_fbo(r, k);
            let (fmt, ty) = read_pair();
            assert_eq!(err(), GL_NO_ERROR, "{} {k:?}: query", r.name);
            let fmt_ok = fmt == GL_RGBA_INTEGER
                || (r.channels == 1 && fmt == GL_RED_INTEGER)
                || (r.channels == 2 && fmt == GL_RG_INTEGER);
            assert!(fmt_ok, "{} {k:?}: read format {fmt:#x}", r.name);
            let ty_ok = if r.signed { ty == GL_INT } else { ty == GL_UNSIGNED_INT };
            assert!(ty_ok, "{} {k:?}: read type {ty:#x}", r.name);
            let mut buf = [0u32; 4];
            // SAFETY: 1x1 of at most four 32-bit values fits.
            unsafe { glReadPixels(0, 0, 1, 1, fmt, ty, buf.as_mut_ptr() as *mut c_void) };
            assert_eq!(err(), GL_NO_ERROR, "{} {k:?}: glReadPixels {fmt:#x}/{ty:#x}", r.name);
        }
    }
}

#[test]
fn texture_and_renderbuffer_give_the_same_pair() {
    let _g = context();
    for r in &ROWS {
        let t = {
            let _f = read_fbo(r, Kind::Texture2d);
            read_pair()
        };
        let b = {
            let _f = read_fbo(r, Kind::Renderbuffer);
            read_pair()
        };
        assert_eq!(err(), GL_NO_ERROR, "{}", r.name);
        assert_eq!(t, b, "{}: texture {t:x?} vs renderbuffer {b:x?}", r.name);
    }
}

#[test]
fn recorded_values_are_pinned() {
    let _g = context();
    for r in &ROWS {
        for k in [Kind::Texture2d, Kind::Renderbuffer] {
            let _f = read_fbo(r, k);
            let want_fmt = GL_RGBA_INTEGER;
            let want_ty = if r.signed { GL_INT } else { GL_UNSIGNED_INT };
            assert_eq!(read_pair(), (want_fmt, want_ty), "{} {k:?}", r.name);
        }
    }
}

#[test]
#[ignore = "implementation_color_read (src/gl/framebuffer.rs) ignores GL_NONE read buffer: no GL_INVALID_OPERATION, RGBA/UNSIGNED_BYTE returned"]
fn read_buffer_none_makes_the_query_invalid_operation() {
    let _g = context();
    for r in &ROWS {
        for k in [Kind::Texture2d, Kind::Renderbuffer] {
            let _f = read_fbo(r, k);
            let (mut fmt, mut ty): (GLint, GLint) = (-7, -7);
            // SAFETY: one GLint each.
            unsafe {
                glReadBuffer(GL_NONE);
                assert_eq!(glGetError(), GL_NO_ERROR);
                glGetIntegerv(GL_IMPLEMENTATION_COLOR_READ_FORMAT, &mut fmt);
            }
            assert_eq!(err(), GL_INVALID_OPERATION, "{} {k:?}: READ_FORMAT with GL_NONE", r.name);
            // SAFETY: as above.
            unsafe { glGetIntegerv(GL_IMPLEMENTATION_COLOR_READ_TYPE, &mut ty) };
            assert_eq!(err(), GL_INVALID_OPERATION, "{} {k:?}: READ_TYPE with GL_NONE", r.name);
            assert_eq!((fmt, ty), (-7, -7), "{} {k:?}: output untouched", r.name);
        }
    }
}

#[test]
fn renderbuffer_samples_of_an_integer_format_are_none() {
    let _g = context();
    for r in &ROWS {
        let (mut n, mut s): (GLint, GLint) = (-1, -1);
        // SAFETY: one GLint each; a buffer of one for GL_SAMPLES.
        unsafe {
            glGetInternalformativ(GL_RENDERBUFFER, r.ifmt, GL_NUM_SAMPLE_COUNTS, 1, &mut n);
            assert_eq!(glGetError(), GL_NO_ERROR, "{}: NUM_SAMPLE_COUNTS", r.name);
            glGetInternalformativ(GL_RENDERBUFFER, r.ifmt, GL_SAMPLES, 1, &mut s);
            assert_eq!(glGetError(), GL_NO_ERROR, "{}: SAMPLES", r.name);
        }
        assert_eq!(n, 0, "{}: NUM_SAMPLE_COUNTS", r.name);
        assert_eq!(s, -1, "{}: GL_SAMPLES writes nothing with no counts", r.name);
    }
}
