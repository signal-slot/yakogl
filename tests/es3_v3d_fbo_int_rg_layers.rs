// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! V3D (Raspberry Pi 4): R and RG integer textures, cube faces and 3D slices
//! as colour attachments (gap G2 of docs/notes/v3d-format-matrix.md; dEQP
//! fbo.color.texcube.* and fbo.color.tex3d.*, plan #383). The companion of
//! tests/es3_v3d_fbo_int_rg.rs, which covers TEXTURE_2D, a 2D-array layer,
//! level 1 and MRT.
//!
//! For each of the twelve formats (R8I .. RG32UI), drawn as the wide store of
//! its channel type and width with the format's channels as the write mask:
//!
//! - glFramebufferTexture2D on GL_TEXTURE_CUBE_MAP_POSITIVE_X and
//!   GL_TEXTURE_CUBE_MAP_NEGATIVE_Z;
//! - glFramebufferTextureLayer on a GL_TEXTURE_3D, slice 2 of 4.
//!
//! Each is complete, an `ivec4` / `uvec4` draw reaches the back end on the wide
//! surface with the format's channel mask, and glClearBuffer then
//! glReadPixels(GL_RGBA_INTEGER) gives the format's channels and 0, 0, 1 for
//! the absent ones.
//!
//! What it cannot see: the GPU. The back end is a stand-in that records each
//! draw and stores a clear in the target's memory; a draw's shader output is
//! not rasterized. Nothing here is verified on hardware.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_v3d_fbo_int_rg_layers
//! ```

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
    SEEN.lock().unwrap().push(Seen {
        draws: d.index_count != 0,
        targets: d.targets[..n].iter().map(|t| (t.gpu, t.w, t.h, t.format)).collect(),
        color_mask: d.color_mask,
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
    // A 3D slice is a sub-rectangle of the grid, at the grid's row pitch.
    let pitch = backend::draw_pitch(d) as usize;
    let base = t.gpu as *mut u8;
    for y in 0..t.h as usize {
        for x in 0..t.w as usize {
            core::ptr::copy_nonoverlapping(px.as_ptr(), base.add((y * pitch + x) * bpp), bpp);
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
    name: c"es3-v3d-fbo-int-rg".as_ptr() as *const c_char,
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
    /// One face of an 8x8 cube map.
    CubeFace(GLenum),
    /// Slice 2 of an 8x8x4 TEXTURE_3D.
    Slice3d,
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
    // SAFETY: room for one name each; the face and slice named are there.
    unsafe {
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        f.fb = fb;
        glGenTextures(1, &mut name);
        match kind {
            Kind::CubeFace(face) => {
                glBindTexture(GL_TEXTURE_CUBE_MAP, name);
                glTexStorage2D(GL_TEXTURE_CUBE_MAP, 1, r.ifmt, 8, 8);
                glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, face, name, 0);
            }
            Kind::Slice3d => {
                glBindTexture(GL_TEXTURE_3D, name);
                glTexStorage3D(GL_TEXTURE_3D, 1, r.ifmt, 8, 8, 4);
                glFramebufferTextureLayer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, name, 0, 2);
            }
        }
        f.tex.push(name);
    }
    assert_eq!(err(), GL_NO_ERROR, "{} {kind:?}: storage and attach", r.name);
    f
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

/// Make `fs` (with the trivial vertex shader) current.
fn program(fs: &str) {
    let v = shader(GL_VERTEX_SHADER, "#version 300 es\nin vec4 p;\nvoid main() { gl_Position = p; }\n");
    let f = shader(GL_FRAGMENT_SHADER, fs);
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

/// One output of `ty` at location 0 writing a known value.
fn program_one(r: &Row) {
    let val = if r.signed { "ivec4(1, -2, 3, -4)" } else { "uvec4(1u, 2u, 3u, 4u)" };
    program(&format!(
        "#version 300 es\nprecision highp int;\nlayout(location = 0) out highp {} o0;\n\
         void main() {{ o0 = {val}; }}\n",
        r.ty()
    ));
}

/// Draw one triangle; what the back end was handed (None: no draw reached it)
/// and the GL error the call left.
fn draw() -> (Option<Seen>, GLenum) {
    static TRI: [f32; 12] = [-1.0, -1.0, 0.0, 1.0, 3.0, -1.0, 0.0, 1.0, -1.0, 3.0, 0.0, 1.0];
    SEEN.lock().unwrap().clear();
    // SAFETY: TRI is static and outlives the draw.
    unsafe {
        glBindBuffer(GL_ARRAY_BUFFER, 0);
        glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, TRI.as_ptr() as *const c_void);
        glEnableVertexAttribArray(0);
        glDrawArrays(GL_TRIANGLES, 0, 3);
        glFinish();
    }
    let e = err();
    let s = core::mem::take(&mut *SEEN.lock().unwrap());
    (s.into_iter().rev().find(|s| s.draws), e)
}

/// glClearBuffer{i,ui}v of attachment 0 with 5, 6, 7, 8, then glReadPixels of
/// the `w` x `w` attachment as GL_RGBA_INTEGER: each pixel must be
/// [`Row::expected`].
fn clear_and_read_back(r: &Row, w: GLsizei, what: &str) {
    let v: [GLint; 4] = [5, 6, 7, 8];
    let mut px = vec![0u32; (w * w * 4) as usize];
    // SAFETY: four values; room for w x w pixels of four 32-bit values.
    unsafe {
        if r.signed {
            glClearBufferiv(GL_COLOR, 0, v.as_ptr());
        } else {
            glClearBufferuiv(GL_COLOR, 0, v.as_ptr() as *const GLuint);
        }
        glReadPixels(
            0,
            0,
            w,
            w,
            GL_RGBA_INTEGER,
            if r.signed { GL_INT } else { GL_UNSIGNED_INT },
            px.as_mut_ptr() as *mut c_void,
        );
    }
    assert_eq!(err(), GL_NO_ERROR, "{} {what}: clear and read back", r.name);
    for (n, p) in px.chunks(4).enumerate() {
        assert_eq!(p, r.expected(), "{} {what}: pixel {n}", r.name);
    }
}

/// The channels (bit 0 R .. bit 3 A) of the format.
fn own_mask(r: &Row) -> u8 {
    (1u8 << r.channels) - 1
}

/// Everything a texture attachment of `kind` owes: complete; a draw reaches
/// the back end taken (not refused, `state::job_state` takes it) on the
/// four-channel surface of the store, of the format's channel type and
/// width, with the format's channels as the write mask; a clear reads back
/// with 0, 0, 1 in the absent channels.
fn expect_texture(r: &Row, kind: Kind) {
    let f = fbo(r, kind);
    assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE, "{} {kind:?}", r.name);
    program_one(r);
    let (seen, e) = draw();
    assert_eq!(e, GL_NO_ERROR, "{} {kind:?}: the draw", r.name);
    let s = seen.unwrap_or_else(|| panic!("{} {kind:?}: the back end saw no draw", r.name));
    assert_eq!(s.refused, None, "{} {kind:?}", r.name);
    assert_eq!(s.targets.len(), 1, "{} {kind:?}", r.name);
    let t = s.targets[0];
    assert_ne!(t.0, 0, "{} {kind:?}: the store", r.name);
    assert_eq!((t.1, t.2), (f.w as u32, f.w as u32), "{} {kind:?}: the level's size", r.name);
    let l = backend::gles_surf_int_layout(t.3).unwrap_or_else(|| panic!("{} {kind:?}: not an integer surface", r.name));
    let want = backend::gles_surf_int_layout(r.surf).unwrap();
    assert_eq!((l.bits, l.signed), (want.bits, want.signed), "{} {kind:?}: the store's type", r.name);
    assert_eq!(l.channels, 4, "{} {kind:?}: the wide surface of the store, {}", r.name, gles_surf_name(t.3));
    assert_eq!(s.color_mask, own_mask(r), "{} {kind:?}: only the format's channels are written", r.name);
    clear_and_read_back(r, f.w, &format!("{kind:?}"));
}

/// GL_TEXTURE_CUBE_MAP_POSITIVE_X (fbo.color.texcube.*).
#[test]
fn r_and_rg_integer_cube_positive_x_completes_and_draws() {
    let _g = context();
    for r in ROWS {
        expect_texture(&r, Kind::CubeFace(GL_TEXTURE_CUBE_MAP_POSITIVE_X));
    }
}

/// GL_TEXTURE_CUBE_MAP_NEGATIVE_Z, the last face.
#[test]
fn r_and_rg_integer_cube_negative_z_completes_and_draws() {
    let _g = context();
    for r in ROWS {
        expect_texture(&r, Kind::CubeFace(GL_TEXTURE_CUBE_MAP_NEGATIVE_Z));
    }
}

/// Slice 2 of a 4-slice TEXTURE_3D (fbo.color.tex3d.*).
#[test]
fn r_and_rg_integer_texture_3d_slice_completes_and_draws() {
    let _g = context();
    for r in ROWS {
        expect_texture(&r, Kind::Slice3d);
    }
}
