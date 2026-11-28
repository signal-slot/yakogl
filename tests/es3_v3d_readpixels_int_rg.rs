// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! V3D (Raspberry Pi 4): glReadPixels from the wide store of an R or RG
//! integer texture (gap G2 of docs/notes/v3d-format-matrix.md, plan #383).
//!
//! The twelve formats (R8I .. RG32UI) as textures are held in the
//! four-channel store of the same type and width and drawn as that wide
//! surface; read-back gives the format's channels and 0, 0, 1 for the absent
//! ones (ES 3.0 4.3.2). At attachment 0 of a TEXTURE_2D, at layer 1 of a
//! TEXTURE_2D_ARRAY and at level 1 of a mipmapped TEXTURE_2D, each format:
//!
//! - glClearBuffer{i,ui}v then glReadPixels(GL_RGBA_INTEGER, GL_INT /
//!   GL_UNSIGNED_INT) gives the cleared R (and G), absent channels 0, 0, 1;
//! - GL_IMPLEMENTATION_COLOR_READ_FORMAT / _TYPE answer GL_RGBA_INTEGER and
//!   GL_INT / GL_UNSIGNED_INT;
//! - a read with a wrong format or type is GL_INVALID_OPERATION (and leaves
//!   the buffer alone);
//! - a sub-rectangle at a non-zero offset of a store whose texels differ per
//!   pixel (glTexSubImage2D, so the wide row stride matters) reads back the
//!   right texels.
//!
//! What it cannot see: the GPU. The back end is a stand-in (see
//! tests/es3_v3d_fbo_int_rg.rs). Nothing here is verified on hardware.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_v3d_readpixels_int_rg
//! ```

#![cfg(all(feature = "host-test", feature = "shaderc-online", feature = "gpu-v3d", not(feature = "report-es2")))]
#![allow(dead_code)]

use core::ffi::{c_char, c_void};
use std::sync::atomic::Ordering;
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{self, GlesDraw, GlesHeap, YakoglBackend, *};
use yakogl::gl::consts::*;
use yakogl::gl::es3::*;
use yakogl::gl::framebuffer::*;
use yakogl::gl::program::*;
use yakogl::gl::state::*;
use yakogl::gl::texture::*;
use yakogl::gl::types::*;
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
    name: c"es3-v3d-readpixels-int-rg".as_ptr() as *const c_char,
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

/* ---- attachments, a program, a draw, a clear, a read-back ----------------- */

/// What carries the format at attachment 0.
#[derive(Clone, Copy, Debug)]
enum Kind {
    Renderbuffer,
    Texture2d,
    /// Layer 1 of a 3-layer TEXTURE_2D_ARRAY.
    ArrayLayer,
    /// Level 1 of a 3-level, 8x8 TEXTURE_2D.
    Level1,
}

/// A bound framebuffer with the format at attachment 0, and whatever else
/// was attached; dropped, it unbinds and deletes them.
struct Fbo {
    fb: GLuint,
    tex: Vec<GLuint>,
    rbs: Vec<GLuint>,
    /// Width of the attached level (the read-back's size).
    w: GLsizei,
}

impl Drop for Fbo {
    fn drop(&mut self) {
        // SAFETY: names this test made.
        unsafe {
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
            glDeleteFramebuffers(1, &self.fb);
            glDeleteTextures(self.tex.len() as GLsizei, self.tex.as_ptr());
            glDeleteRenderbuffers(self.rbs.len() as GLsizei, self.rbs.as_ptr());
            glUseProgram(0);
        }
    }
}

fn fbo(r: &Row, kind: Kind) -> Fbo {
    let (mut fb, mut name) = (0, 0);
    let mut f = Fbo { fb: 0, tex: vec![], rbs: vec![], w: 8 };
    // SAFETY: room for one name each; the layer and level named are there.
    unsafe {
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        f.fb = fb;
        match kind {
            Kind::Renderbuffer => {
                glGenRenderbuffers(1, &mut name);
                glBindRenderbuffer(GL_RENDERBUFFER, name);
                glRenderbufferStorage(GL_RENDERBUFFER, r.ifmt, 8, 8);
                glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_RENDERBUFFER, name);
                f.rbs.push(name);
            }
            Kind::Texture2d | Kind::Level1 => {
                let (levels, level) = if matches!(kind, Kind::Level1) { (3, 1) } else { (1, 0) };
                glGenTextures(1, &mut name);
                glBindTexture(GL_TEXTURE_2D, name);
                glTexStorage2D(GL_TEXTURE_2D, levels, r.ifmt, 8, 8);
                glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, name, level);
                f.tex.push(name);
                f.w = 8 >> level;
            }
            Kind::ArrayLayer => {
                glGenTextures(1, &mut name);
                glBindTexture(GL_TEXTURE_2D_ARRAY, name);
                glTexStorage3D(GL_TEXTURE_2D_ARRAY, 1, r.ifmt, 8, 8, 3);
                glFramebufferTextureLayer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, name, 0, 1);
                f.tex.push(name);
            }
        }
    }
    assert_eq!(err(), GL_NO_ERROR, "{} {kind:?}: storage and attach", r.name);
    f
}

/* ---- read-back ------------------------------------------------------------ */

const KINDS: [Kind; 3] = [Kind::Texture2d, Kind::ArrayLayer, Kind::Level1];

fn read_type(r: &Row) -> GLenum {
    if r.signed {
        GL_INT
    } else {
        GL_UNSIGNED_INT
    }
}

/// Clear attachment 0 with the four values (glClearBufferiv / uiv).
fn clear_with(r: &Row, v: [GLint; 4]) {
    // SAFETY: four values.
    unsafe {
        if r.signed {
            glClearBufferiv(GL_COLOR, 0, v.as_ptr());
        } else {
            glClearBufferuiv(GL_COLOR, 0, v.as_ptr() as *const GLuint);
        }
    }
}

/// A clear then a RGBA_INTEGER read: R (and G) as cleared, absent channels 0, 0, 1.
#[test]
fn clear_reads_back_with_absent_channels_0_0_1() {
    let _g = context();
    for r in ROWS {
        for kind in KINDS {
            let f = fbo(&r, kind);
            assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE, "{} {kind:?}", r.name);
            // a value that fits all widths, and one negative for the signed
            let v: [GLint; 4] = if r.signed { [-5, 6, 7, 8] } else { [5, 6, 7, 8] };
            SEEN.lock().unwrap().clear();
            clear_with(&r, v);
            let mut px = vec![0xdead_beefu32; (f.w * f.w * 4) as usize];
            // SAFETY: room for w x w pixels of four 32-bit values.
            unsafe {
                glReadPixels(0, 0, f.w, f.w, GL_RGBA_INTEGER, read_type(&r), px.as_mut_ptr() as *mut c_void);
            }
            assert_eq!(err(), GL_NO_ERROR, "{} {kind:?}: read", r.name);
            // the read flushed the clear, which reached the back end on the store's
            // wide surface (four channels of the format's type and width)
            let seen = SEEN.lock().unwrap().last().cloned();
            let s = seen.unwrap_or_else(|| panic!("{} {kind:?}: the back end saw no clear", r.name));
            assert_eq!(s.refused, None, "{} {kind:?}: the clear", r.name);
            let t = s.targets[0];
            assert_eq!((t.1, t.2), (f.w as u32, f.w as u32), "{} {kind:?}: the level's size", r.name);
            let l = backend::gles_surf_int_layout(t.3)
                .unwrap_or_else(|| panic!("{} {kind:?}: not an integer surface", r.name));
            let own = backend::gles_surf_int_layout(r.surf).unwrap();
            assert_eq!(
                (l.bits, l.signed, l.channels),
                (own.bits, own.signed, 4),
                "{} {kind:?}: the wide store",
                r.name
            );
            let want = [v[0] as u32, if r.channels == 2 { v[1] as u32 } else { 0 }, 0, 1];
            for (n, p) in px.chunks(4).enumerate() {
                assert_eq!(p, want, "{} {kind:?}: pixel {n}", r.name);
            }
        }
    }
}

/// The implementation's own read format and type for an integer attachment.
#[test]
fn implementation_read_format_and_type() {
    let _g = context();
    for r in ROWS {
        for kind in KINDS {
            let _f = fbo(&r, kind);
            let (mut fmt, mut ty) = (0, 0);
            // SAFETY: one value each.
            unsafe {
                glGetIntegerv(GL_IMPLEMENTATION_COLOR_READ_FORMAT, &mut fmt);
                glGetIntegerv(GL_IMPLEMENTATION_COLOR_READ_TYPE, &mut ty);
            }
            assert_eq!(err(), GL_NO_ERROR, "{} {kind:?}", r.name);
            assert_eq!(fmt as GLenum, GL_RGBA_INTEGER, "{} {kind:?}: read format", r.name);
            assert_eq!(ty as GLenum, read_type(&r), "{} {kind:?}: read type", r.name);
        }
    }
}

/// A wrong format or type is GL_INVALID_OPERATION and writes nothing.
#[test]
fn wrong_format_or_type_is_invalid_operation() {
    let _g = context();
    for r in ROWS {
        for kind in KINDS {
            let _f = fbo(&r, kind);
            clear_with(&r, [5, 6, 7, 8]);
            let wrong_int = if r.signed { GL_UNSIGNED_INT } else { GL_INT };
            let cases: [(GLenum, GLenum, &str); 5] = [
                (GL_RGBA, GL_UNSIGNED_BYTE, "RGBA/UNSIGNED_BYTE"),
                (GL_RGBA, GL_FLOAT, "RGBA/FLOAT"),
                (GL_RGBA_INTEGER, wrong_int, "RGBA_INTEGER with the other signedness"),
                (GL_RGBA_INTEGER, GL_FLOAT, "RGBA_INTEGER/FLOAT"),
                (GL_RGBA_INTEGER, GL_UNSIGNED_BYTE, "RGBA_INTEGER/UNSIGNED_BYTE"),
            ];
            for (fmt, ty, what) in cases {
                let mut px = [0xdead_beefu32; 4];
                // SAFETY: room for one pixel of four 32-bit values.
                unsafe { glReadPixels(0, 0, 1, 1, fmt, ty, px.as_mut_ptr() as *mut c_void) };
                assert_eq!(err(), GL_INVALID_OPERATION, "{} {kind:?}: {what}", r.name);
                assert_eq!(px, [0xdead_beefu32; 4], "{} {kind:?}: {what}: nothing written", r.name);
            }
        }
    }
}

/// Upload distinct texels, then read a w/2 x 2 rectangle at (w/4, w/2 + 1)
/// (4 x 2 at (2, 5) of 8 x 8; 2 x 1 at (1, 3) of level 1's 4 x 4): the wide
/// store's row stride and pixel size decide whether the right texels come back.
#[test]
fn sub_rectangle_at_an_offset_reads_the_right_texels() {
    let _g = context();
    for r in ROWS {
        // TEXTURE_2D, layer 1 of an array, level 1 (4x4)
        for kind in KINDS {
            let f = fbo(&r, kind);
            let w = f.w as usize;
            let level = if matches!(kind, Kind::Level1) { 1 } else { 0 };
            let tex = f.tex[0];
            let ch = r.channels as usize;
            // texel (x, y) = [10 + y * w + x, 20 + y * w + x]: fits R8 signed
            let val = |x: usize, y: usize, c: usize| if c == 0 { 10 } else { 20 } + (y * w + x) as u32;
            // the upload's type matches the format's width (ES 3.0 table 3.2)
            let bits = backend::gles_surf_int_layout(r.surf).unwrap().bits as usize;
            let upload_ty = match (bits, r.signed) {
                (8, true) => GL_BYTE,
                (8, false) => GL_UNSIGNED_BYTE,
                (16, true) => GL_SHORT,
                (16, false) => GL_UNSIGNED_SHORT,
                (_, true) => GL_INT,
                (_, false) => GL_UNSIGNED_INT,
            };
            let mut src = vec![0u8; w * w * ch * bits / 8];
            for y in 0..w {
                for x in 0..w {
                    for c in 0..ch {
                        let o = ((y * w + x) * ch + c) * bits / 8;
                        src[o..o + bits / 8].copy_from_slice(&val(x, y, c).to_le_bytes()[..bits / 8]);
                    }
                }
            }
            // SAFETY: src holds w x w texels of ch values of `bits` bits.
            unsafe {
                let fmt = if ch == 1 { GL_RED_INTEGER } else { GL_RG_INTEGER };
                glPixelStorei(GL_UNPACK_ALIGNMENT, 1);
                if matches!(kind, Kind::ArrayLayer) {
                    glBindTexture(GL_TEXTURE_2D_ARRAY, tex);
                    glTexSubImage3D(
                        GL_TEXTURE_2D_ARRAY,
                        0,
                        0,
                        0,
                        1,
                        w as GLsizei,
                        w as GLsizei,
                        1,
                        fmt,
                        upload_ty,
                        src.as_ptr() as *const c_void,
                    );
                } else {
                    glBindTexture(GL_TEXTURE_2D, tex);
                    glTexSubImage2D(
                        GL_TEXTURE_2D,
                        level,
                        0,
                        0,
                        w as GLsizei,
                        w as GLsizei,
                        fmt,
                        upload_ty,
                        src.as_ptr() as *const c_void,
                    );
                }
            }
            assert_eq!(err(), GL_NO_ERROR, "{} {kind:?}: upload", r.name);
            let (rx, ry, rw, rh) = (w / 4, w / 2 + 1, w / 2, 2usize.min(w - (w / 2 + 1)));
            let mut px = vec![0xdead_beefu32; rw * rh * 4];
            // SAFETY: room for rw x rh pixels of four 32-bit values.
            unsafe {
                glReadPixels(
                    rx as GLint,
                    ry as GLint,
                    rw as GLsizei,
                    rh as GLsizei,
                    GL_RGBA_INTEGER,
                    read_type(&r),
                    px.as_mut_ptr() as *mut c_void,
                );
            }
            assert_eq!(err(), GL_NO_ERROR, "{} {kind:?}: read", r.name);
            for j in 0..rh {
                for i in 0..rw {
                    let want = [val(rx + i, ry + j, 0), if ch == 2 { val(rx + i, ry + j, 1) } else { 0 }, 0, 1];
                    assert_eq!(
                        &px[(j * rw + i) * 4..][..4],
                        want,
                        "{} {kind:?}: texel ({}, {})",
                        r.name,
                        rx + i,
                        ry + j
                    );
                }
            }
        }
    }
}
