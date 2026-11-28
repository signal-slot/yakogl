// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! V3D (Raspberry Pi 4): the twelve R and RG integer formats take no
//! multisampling (ES 3.0.6 6.1.15, 4.4.2; gap G2 of
//! docs/notes/v3d-format-matrix.md; plan #383).
//!
//! The back end here says `max_samples` 4 (the V3D build with `msaa-fbo`), so
//! the refusals are the GL core's own, not the capability's. Held, for each of
//! R8I .. RG32UI:
//!
//! - `glGetInternalformativ(GL_RENDERBUFFER, fmt, GL_NUM_SAMPLE_COUNTS)` is 0;
//! - `glRenderbufferStorageMultisample` with samples 1 and 4 gives
//!   GL_INVALID_OPERATION and leaves the renderbuffer as it was; with samples
//!   0 it works and the renderbuffer is complete as the only attachment;
//! - a samples-0 integer texture beside a 4-sample RGBA8 renderbuffer:
//!   GL_FRAMEBUFFER_INCOMPLETE_MULTISAMPLE;
//! - `glBlitFramebuffer` from a 4-sample RGBA8 FBO into an R8I texture:
//!   GL_INVALID_OPERATION (float read buffer, integer draw buffer).
//!
//! The back end is a stand-in that draws nothing. Nothing here is verified on
//! hardware.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_v3d_int_rg_msaa
//! ```

#![allow(dead_code, unused_imports)]
#![cfg(all(feature = "host-test", feature = "shaderc-online", feature = "gpu-v3d", not(feature = "report-es2")))]

use core::ffi::{c_char, c_void};
use std::ffi::CString;
use std::sync::atomic::Ordering;
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{self, GlesDraw, GlesHeap, YakoglBackend, *};
use yakogl::gl::buffer::glBindBuffer;
use yakogl::gl::consts::*;
use yakogl::gl::draw::*;
use yakogl::gl::es3::*;
use yakogl::gl::framebuffer::*;
use yakogl::gl::program::*;
use yakogl::gl::shader::*;
use yakogl::gl::state::*;
use yakogl::gl::texture::*;
use yakogl::gl::types::*;
use yakogl::gl::vertex::{glEnableVertexAttribArray, glVertexAttribPointer};
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};
use yakogl::v3d::state;

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

/// What the back end was handed by one call, and what the V3D made of it.
#[derive(Clone, Debug)]
struct Seen {
    /// An application draw (vertices), not a clear alone.
    draws: bool,
    /// `targets[..num_targets]`: (gpu, w, h, format).
    targets: Vec<(u64, u32, u32, u8)>,
    color_mask: u8,
    /// The job state's (colormask, rt_colormask_own, rt_colormask) of a
    /// draw the V3D takes.
    job: Option<(u32, bool, u32)>,
    /// `state::refusal` / `state::unsupported`, or the job builder's reason.
    refused: Option<&'static str>,
}

static SEEN: Mutex<Vec<Seen>> = Mutex::new(Vec::new());

/// Records each draw; a clear of an integer target stores the low bits of
/// the four raw values in the target's layout, little endian, rows tightly
/// packed (the stand-in's own layout: the GL core reads back what it was told).
unsafe extern "C" fn b_draw(d: *const GlesDraw) -> i32 {
    let d = &*d;
    let n = (d.num_targets.max(1) as usize).min(GLES_MAX_TARGETS);
    let refused = state::refusal(d).or_else(|| state::unsupported(d)).or_else(|| {
        if d.index_count != 0 {
            state::job_state(d).err()
        } else {
            None
        }
    });
    let job = (d.index_count != 0)
        .then(|| state::job_state(d).ok())
        .flatten()
        .map(|st| (st.colormask, st.rt_colormask_own, st.rt_colormask));
    SEEN.lock().unwrap().push(Seen {
        draws: d.index_count != 0,
        targets: d.targets[..n].iter().map(|t| (t.gpu, t.w, t.h, t.format)).collect(),
        color_mask: d.color_mask,
        job,
        refused,
    });
    let t = &d.targets[0];
    let Some(l) = backend::gles_surf_int_layout(t.format) else { return 0 };
    if !d.do_clear || t.gpu == 0 {
        return 0;
    }
    let bpp = backend::gles_surf_bytes_per_pixel(t.format) as usize;
    let v = d.clear_rgba.map(f32::to_bits);
    let step = l.bits as usize / 8;
    let mut px = vec![0u8; bpp];
    for ch in 0..l.channels as usize {
        px[ch * step..ch * step + step].copy_from_slice(&v[ch].to_le_bytes()[..step]);
    }
    let base = t.gpu as *mut u8;
    for y in 0..t.h as usize {
        for x in 0..t.w as usize {
            core::ptr::copy_nonoverlapping(px.as_ptr(), base.add((y * t.w as usize + x) * bpp), bpp);
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
    name: c"es3-v3d-int-rg-msaa".as_ptr() as *const c_char,
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
    SEEN.lock().unwrap().clear();
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

/* ---- the twelve formats -------------------------------------------------- */

/// One of the twelve R and RG integer formats.
#[derive(Clone, Copy)]
struct Row {
    name: &'static str,
    ifmt: GLenum,
    channels: u8,
    signed: bool,
    /// The format's own surface (the renderbuffer's).
    surf: u8,
}

const fn row(name: &'static str, ifmt: GLenum, channels: u8, signed: bool, surf: u8) -> Row {
    Row { name, ifmt, channels, signed, surf }
}

const ROWS: [Row; 12] = [
    row("R8I", GL_R8I, 1, true, GLES_SURF_R8I),
    row("R8UI", GL_R8UI, 1, false, GLES_SURF_R8UI),
    row("RG8I", GL_RG8I, 2, true, GLES_SURF_RG8I),
    row("RG8UI", GL_RG8UI, 2, false, GLES_SURF_RG8UI),
    row("R16I", GL_R16I, 1, true, GLES_SURF_R16I),
    row("R16UI", GL_R16UI, 1, false, GLES_SURF_R16UI),
    row("RG16I", GL_RG16I, 2, true, GLES_SURF_RG16I),
    row("RG16UI", GL_RG16UI, 2, false, GLES_SURF_RG16UI),
    row("R32I", GL_R32I, 1, true, GLES_SURF_R32I),
    row("R32UI", GL_R32UI, 1, false, GLES_SURF_R32UI),
    row("RG32I", GL_RG32I, 2, true, GLES_SURF_RG32I),
    row("RG32UI", GL_RG32UI, 2, false, GLES_SURF_RG32UI),
];

impl Row {
    fn ty(&self) -> &'static str {
        if self.signed {
            "ivec4"
        } else {
            "uvec4"
        }
    }
    /// The read-back of a clear of 5, 6, 7, 8: the format's channels, then
    /// 0, 0, 1 for the absent ones (ES 3.0 4.3.2).
    fn expected(&self) -> [u32; 4] {
        if self.channels == 1 {
            [5, 0, 0, 1]
        } else {
            [5, 6, 0, 1]
        }
    }
}

/// glGetInternalformativ(GL_RENDERBUFFER, fmt, pname) with room for one value.
fn iformat(fmt: GLenum, pname: GLenum) -> (GLint, GLenum) {
    let mut v: GLint = -777;
    // SAFETY: one GLint out.
    unsafe { glGetInternalformativ(GL_RENDERBUFFER, fmt, pname, 1, &mut v) };
    (v, err())
}

fn rb_param(rb: GLuint, pname: GLenum) -> GLint {
    let mut v: GLint = -777;
    // SAFETY: plain GL calls; one GLint out.
    unsafe {
        glBindRenderbuffer(GL_RENDERBUFFER, rb);
        glGetRenderbufferParameteriv(GL_RENDERBUFFER, pname, &mut v);
    }
    assert_eq!(err(), GL_NO_ERROR);
    v
}

fn new_rb(samples: GLsizei, fmt: GLenum) -> (GLuint, GLenum) {
    let mut rb = 0;
    // SAFETY: plain GL calls.
    unsafe {
        glGenRenderbuffers(1, &mut rb);
        glBindRenderbuffer(GL_RENDERBUFFER, rb);
        glRenderbufferStorageMultisample(GL_RENDERBUFFER, samples, fmt, 8, 8);
    }
    (rb, err())
}

fn int_tex(fmt: GLenum) -> GLuint {
    let mut t = 0;
    // SAFETY: plain GL calls.
    unsafe {
        glGenTextures(1, &mut t);
        glBindTexture(GL_TEXTURE_2D, t);
        glTexStorage2D(GL_TEXTURE_2D, 1, fmt, 8, 8);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST as GLint);
    }
    assert_eq!(err(), GL_NO_ERROR, "texture storage");
    t
}

#[test]
fn num_sample_counts_is_zero() {
    let _g = context();
    for r in ROWS {
        let (v, e) = iformat(r.ifmt, GL_NUM_SAMPLE_COUNTS);
        assert_eq!((v, e), (0, GL_NO_ERROR), "{} NUM_SAMPLE_COUNTS", r.name);
        // RGBA8 has counts: the 4x capability is on in this back end.
    }
    let (v, e) = iformat(GL_RGBA8, GL_NUM_SAMPLE_COUNTS);
    assert_eq!(e, GL_NO_ERROR);
    assert!(v >= 1, "RGBA8 control: {v}");
}

#[test]
fn multisample_storage_refused_and_leaves_renderbuffer() {
    let _g = context();
    for r in ROWS {
        for samples in [1, 4] {
            let (rb, e) = new_rb(0, r.ifmt);
            assert_eq!(e, GL_NO_ERROR, "{} storage 0", r.name);
            assert_eq!(rb_param(rb, GL_RENDERBUFFER_SAMPLES), 0);
            // SAFETY: plain GL call on the bound renderbuffer.
            unsafe { glRenderbufferStorageMultisample(GL_RENDERBUFFER, samples, r.ifmt, 4, 4) };
            assert_eq!(err(), GL_INVALID_OPERATION, "{} samples {samples}", r.name);
            assert_eq!(rb_param(rb, GL_RENDERBUFFER_SAMPLES), 0, "{} samples after {samples}", r.name);
            assert_eq!(rb_param(rb, GL_RENDERBUFFER_WIDTH), 8, "{} width after {samples}", r.name);
            assert_eq!(rb_param(rb, GL_RENDERBUFFER_HEIGHT), 8, "{} height after {samples}", r.name);
            assert_eq!(rb_param(rb, GL_RENDERBUFFER_INTERNAL_FORMAT) as GLenum, r.ifmt, "{} format", r.name);
            // SAFETY: a name this test made.
            unsafe { glDeleteRenderbuffers(1, &rb) };
        }
    }
}

#[test]
fn multisample_storage_on_fresh_renderbuffer_refused() {
    let _g = context();
    for r in ROWS {
        for samples in [1, 4] {
            let (rb, e) = new_rb(samples, r.ifmt);
            assert_eq!(e, GL_INVALID_OPERATION, "{} samples {samples}", r.name);
            assert_eq!(rb_param(rb, GL_RENDERBUFFER_WIDTH), 0, "{} untouched", r.name);
            // SAFETY: a name this test made.
            unsafe { glDeleteRenderbuffers(1, &rb) };
        }
    }
}

#[test]
fn samples_zero_is_complete() {
    let _g = context();
    for r in ROWS {
        let (rb, e) = new_rb(0, r.ifmt);
        assert_eq!(e, GL_NO_ERROR, "{}", r.name);
        let mut fb = 0;
        // SAFETY: plain GL calls.
        unsafe {
            glGenFramebuffers(1, &mut fb);
            glBindFramebuffer(GL_FRAMEBUFFER, fb);
            glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_RENDERBUFFER, rb);
        }
        assert_eq!(err(), GL_NO_ERROR);
        assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE, "{}", r.name);
        // SAFETY: names this test made.
        unsafe {
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
            glDeleteFramebuffers(1, &fb);
            glDeleteRenderbuffers(1, &rb);
        }
    }
}

/// A 4-sample RGBA8 renderbuffer; its storage must be accepted here.
fn rgba8_ms4() -> GLuint {
    let (rb, e) = new_rb(4, GL_RGBA8);
    assert_eq!(e, GL_NO_ERROR, "4x RGBA8 storage");
    rb
}

#[test]
fn int_texture_beside_ms4_renderbuffer_is_incomplete_multisample() {
    let _g = context();
    for r in ROWS {
        let rb = rgba8_ms4();
        let t = int_tex(r.ifmt);
        let mut fb = 0;
        // SAFETY: plain GL calls.
        unsafe {
            glGenFramebuffers(1, &mut fb);
            glBindFramebuffer(GL_FRAMEBUFFER, fb);
            glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, t, 0);
            glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT1, GL_RENDERBUFFER, rb);
        }
        assert_eq!(err(), GL_NO_ERROR);
        assert_eq!(status(), GL_FRAMEBUFFER_INCOMPLETE_MULTISAMPLE, "{} at 0, 4x RGBA8 at 1", r.name);
        // And with the roles swapped.
        // SAFETY: plain GL calls.
        unsafe {
            glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, 0, 0);
            glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_RENDERBUFFER, rb);
            glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT1, GL_TEXTURE_2D, t, 0);
        }
        assert_eq!(err(), GL_NO_ERROR);
        assert_eq!(status(), GL_FRAMEBUFFER_INCOMPLETE_MULTISAMPLE, "{} at 1, 4x RGBA8 at 0", r.name);
        // SAFETY: names this test made.
        unsafe {
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
            glDeleteFramebuffers(1, &fb);
            glDeleteRenderbuffers(1, &rb);
            glDeleteTextures(1, &t);
        }
    }
}

#[test]
fn blit_from_ms4_rgba8_into_int_texture_is_invalid_operation() {
    let _g = context();
    let rb = rgba8_ms4();
    let (mut rfb, mut dfb) = (0, 0);
    let t = int_tex(GL_R8I);
    // SAFETY: plain GL calls.
    unsafe {
        glGenFramebuffers(1, &mut rfb);
        glBindFramebuffer(GL_READ_FRAMEBUFFER, rfb);
        glFramebufferRenderbuffer(GL_READ_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_RENDERBUFFER, rb);
        glGenFramebuffers(1, &mut dfb);
        glBindFramebuffer(GL_DRAW_FRAMEBUFFER, dfb);
        glFramebufferTexture2D(GL_DRAW_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, t, 0);
    }
    assert_eq!(err(), GL_NO_ERROR);
    // SAFETY: plain GL calls.
    unsafe {
        assert_eq!(glCheckFramebufferStatus(GL_READ_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE);
        assert_eq!(glCheckFramebufferStatus(GL_DRAW_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE);
        glBlitFramebuffer(0, 0, 8, 8, 0, 0, 8, 8, GL_COLOR_BUFFER_BIT, GL_NEAREST);
    }
    assert_eq!(err(), GL_INVALID_OPERATION);
    // SAFETY: names this test made.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glDeleteFramebuffers(1, &rfb);
        glDeleteFramebuffers(1, &dfb);
        glDeleteRenderbuffers(1, &rb);
        glDeleteTextures(1, &t);
    }
}
