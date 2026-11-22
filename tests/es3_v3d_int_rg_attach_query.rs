// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! V3D (Raspberry Pi 4): the attachment and read-format queries of an R or RG
//! integer texture that the V3D holds widened to RGBA (`WIDE_INT_RT`,
//! src/gl/framebuffer.rs). The queries must answer for the format, not the
//! store (ES 3.0 6.1.13, 4.3.1).
//!
//! For each of the twelve formats (R8I .. RG32UI), at GL_COLOR_ATTACHMENT0 of a
//! TEXTURE_2D, of layer 1 of a TEXTURE_2D_ARRAY and of level 1 of a mipmapped
//! TEXTURE_2D:
//!
//! - glGetFramebufferAttachmentParameteriv: RED/GREEN/BLUE/ALPHA_SIZE are the
//!   format's (0 for absent channels), COMPONENT_TYPE is GL_INT or
//!   GL_UNSIGNED_INT, COLOR_ENCODING is GL_LINEAR;
//! - glGetIntegerv GL_IMPLEMENTATION_COLOR_READ_FORMAT / _TYPE give a valid
//!   integer pair (RED_INTEGER / RG_INTEGER / RGBA_INTEGER with an integer
//!   type), and glReadPixels with that pair succeeds.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_v3d_int_rg_attach_query
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
    name: c"es3-v3d-int-rg-attach-query".as_ptr() as *const c_char,
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

fn status() -> GLenum {
    // SAFETY: plain GL call.
    unsafe { glCheckFramebufferStatus(GL_FRAMEBUFFER) }
}

struct Row {
    name: &'static str,
    ifmt: GLenum,
    channels: u8,
    signed: bool,
    bits: GLint,
}

const fn row(name: &'static str, ifmt: GLenum, channels: u8, signed: bool, bits: GLint) -> Row {
    Row { name, ifmt, channels, signed, bits }
}

const ROWS: [Row; 12] = [
    row("R8I", GL_R8I, 1, true, 8),
    row("R8UI", GL_R8UI, 1, false, 8),
    row("RG8I", GL_RG8I, 2, true, 8),
    row("RG8UI", GL_RG8UI, 2, false, 8),
    row("R16I", GL_R16I, 1, true, 16),
    row("R16UI", GL_R16UI, 1, false, 16),
    row("RG16I", GL_RG16I, 2, true, 16),
    row("RG16UI", GL_RG16UI, 2, false, 16),
    row("R32I", GL_R32I, 1, true, 32),
    row("R32UI", GL_R32UI, 1, false, 32),
    row("RG32I", GL_RG32I, 2, true, 32),
    row("RG32UI", GL_RG32UI, 2, false, 32),
];

#[derive(Clone, Copy, Debug)]
enum Kind {
    Texture2d,
    ArrayLayer,
    Level1,
}

struct Fbo {
    fb: GLuint,
    tex: GLuint,
}

impl Drop for Fbo {
    fn drop(&mut self) {
        // SAFETY: names this test made.
        unsafe {
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
            glDeleteFramebuffers(1, &self.fb);
            glDeleteTextures(1, &self.tex);
        }
    }
}

fn fbo(r: &Row, kind: Kind) -> Fbo {
    let (mut fb, mut tex) = (0, 0);
    // SAFETY: room for one name each; the layer and level named are there.
    unsafe {
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        glGenTextures(1, &mut tex);
        match kind {
            Kind::Texture2d | Kind::Level1 => {
                let (levels, level) = if matches!(kind, Kind::Level1) { (3, 1) } else { (1, 0) };
                glBindTexture(GL_TEXTURE_2D, tex);
                glTexStorage2D(GL_TEXTURE_2D, levels, r.ifmt, 8, 8);
                glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex, level);
            }
            Kind::ArrayLayer => {
                glBindTexture(GL_TEXTURE_2D_ARRAY, tex);
                glTexStorage3D(GL_TEXTURE_2D_ARRAY, 1, r.ifmt, 8, 8, 3);
                glFramebufferTextureLayer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, tex, 0, 1);
            }
        }
    }
    assert_eq!(err(), GL_NO_ERROR, "{} {kind:?}: storage and attach", r.name);
    Fbo { fb, tex }
}

fn attach(pname: GLenum) -> GLint {
    let mut v: GLint = -12345;
    // SAFETY: one GLint of room.
    unsafe { glGetFramebufferAttachmentParameteriv(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, pname, &mut v) };
    assert_eq!(err(), GL_NO_ERROR, "query {pname:#x}");
    v
}

fn check_attach(r: &Row, kind: Kind) {
    let _f = fbo(r, kind);
    assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE, "{} {kind:?}", r.name);
    let sizes = [
        (GL_FRAMEBUFFER_ATTACHMENT_RED_SIZE, r.bits),
        (GL_FRAMEBUFFER_ATTACHMENT_GREEN_SIZE, if r.channels == 2 { r.bits } else { 0 }),
        (GL_FRAMEBUFFER_ATTACHMENT_BLUE_SIZE, 0),
        (GL_FRAMEBUFFER_ATTACHMENT_ALPHA_SIZE, 0),
    ];
    for (p, want) in sizes {
        assert_eq!(attach(p), want, "{} {kind:?}: size {p:#x}", r.name);
    }
    let ty = if r.signed { GL_INT } else { GL_UNSIGNED_INT };
    assert_eq!(attach(GL_FRAMEBUFFER_ATTACHMENT_COMPONENT_TYPE) as GLenum, ty, "{} {kind:?}: component type", r.name);
    assert_eq!(
        attach(GL_FRAMEBUFFER_ATTACHMENT_COLOR_ENCODING) as GLenum,
        GL_LINEAR,
        "{} {kind:?}: colour encoding",
        r.name
    );
}

fn check_read_format(r: &Row, kind: Kind) {
    let _f = fbo(r, kind);
    assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE, "{} {kind:?}", r.name);
    let (mut fmt, mut ty): (GLint, GLint) = (0, 0);
    // SAFETY: one GLint each.
    unsafe {
        glGetIntegerv(GL_IMPLEMENTATION_COLOR_READ_FORMAT, &mut fmt);
        glGetIntegerv(GL_IMPLEMENTATION_COLOR_READ_TYPE, &mut ty);
    }
    assert_eq!(err(), GL_NO_ERROR);
    let (fmt, ty) = (fmt as GLenum, ty as GLenum);
    let fmt_ok = fmt == GL_RGBA_INTEGER
        || (r.channels == 1 && fmt == GL_RED_INTEGER)
        || (r.channels == 2 && fmt == GL_RG_INTEGER);
    assert!(fmt_ok, "{} {kind:?}: read format {fmt:#x}", r.name);
    let ty_ok = if r.signed { ty == GL_INT } else { ty == GL_UNSIGNED_INT };
    assert!(ty_ok, "{} {kind:?}: read type {ty:#x}", r.name);
    let mut buf = [0u32; 4];
    // SAFETY: 1x1 of at most four 32-bit values fits.
    unsafe { glReadPixels(0, 0, 1, 1, fmt, ty, buf.as_mut_ptr() as *mut c_void) };
    assert_eq!(err(), GL_NO_ERROR, "{} {kind:?}: glReadPixels {fmt:#x}/{ty:#x}", r.name);
}

fn each(f: fn(&Row, Kind)) {
    let _g = context();
    for r in &ROWS {
        for k in [Kind::Texture2d, Kind::ArrayLayer, Kind::Level1] {
            f(r, k);
        }
    }
}

#[test]
fn attachment_sizes_type_encoding_are_the_formats() {
    each(check_attach);
}

#[test]
fn read_format_and_type_are_a_valid_integer_pair_that_reads() {
    each(check_read_format);
}
