// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! V3D (Raspberry Pi 4): R and RG integer textures, cube faces and 3D slices
//! as colour attachments, with the store address and row pitch the draw uses
//! (gap G2 of docs/notes/v3d-format-matrix.md; dEQP fbo.color.texcube.* and
//! fbo.color.tex3d.*, plan #383). Beside tests/es3_v3d_fbo_int_rg_layers.rs
//! (level 0 of a face and one slice, with a clear read-back) this adds, for
//! each of the twelve formats (R8I .. RG32UI):
//!
//! - GL_TEXTURE_CUBE_MAP_POSITIVE_X and NEGATIVE_Z at level 0 and level 1;
//! - slices 0..3 of an 8x8x4 TEXTURE_3D (slice 2 and 3 are not at the start);
//!
//! each complete, drawn on the four-channel wide surface of the store (the
//! format's channel type and width, so the per-target bytes per pixel is the
//! wide one) with the format's channels as the colour mask, and with the
//! address and row pitch of the draw's target matching the store's layout:
//! the faces of a level lie one face apart, level 1's six faces follow level
//! 0's six at the level's own pair-padded pitch, and a slice is a sub-rectangle of
//! the 2 x 2 grid at the grid's pitch (16 texels, not the slice's 8).
//!
//! What it cannot see: the GPU. The back end is a stand-in that records each
//! draw; a shader output is not rasterized. The addresses are checked
//! relative to the texture's first face or slice, not against Mesa's PBE
//! store. Nothing here is verified on hardware.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_v3d_fbo_int_rg_cube3d
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
    /// Attachment 0's address, and its row pitch in pixels.
    gpu: u64,
    pitch: u32,
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
        gpu: d.target_gpu,
        pitch: backend::draw_pitch(d),
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
    name: c"es3-v3d-fbo-int-rg-cube3d".as_ptr() as *const c_char,
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
}

/* ---- attachments, a program, a draw, a clear, a read-back ----------------- */

/// What carries the format at attachment 0.
#[derive(Clone, Copy, Debug)]
enum Kind {
    /// One face of a 3-level 8x8 cube map, at `level`.
    CubeFace(GLenum, GLint),
}

/// A bound framebuffer with the format at attachment 0; dropped, it unbinds
/// and deletes it.
struct Fbo {
    fb: GLuint,
    tex: Vec<GLuint>,
    rbs: Vec<GLuint>,
    /// Width of the attached level (the target's size).
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
    // SAFETY: room for one name each; the face, level and slice named are there.
    unsafe {
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        f.fb = fb;
        glGenTextures(1, &mut name);
        match kind {
            Kind::CubeFace(face, level) => {
                glBindTexture(GL_TEXTURE_CUBE_MAP, name);
                glTexStorage2D(GL_TEXTURE_CUBE_MAP, 3, r.ifmt, 8, 8);
                glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, face, name, level);
                f.w = 8 >> level;
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

/// The channels (bit 0 R .. bit 3 A) of the format.
fn own_mask(r: &Row) -> u8 {
    (1u8 << r.channels) - 1
}

/// What one draw on `kind` was handed: (address, row pitch in pixels).
struct Store {
    gpu: u64,
    pitch: u32,
    /// Bytes per pixel of the target's surface.
    bpp: u64,
}

/// Everything a texture attachment of `kind` owes: complete; a draw reaches
/// the back end taken (not refused, `state::job_state` takes it) on the
/// four-channel surface of the store, of the format's channel type and
/// width, with the format's channels as the write mask. Returns where it
/// stores.
fn expect_texture(r: &Row, kind: Kind) -> Store {
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
    assert_eq!(t.0, s.gpu, "{} {kind:?}: targets[0] is the frame's target", r.name);
    assert_eq!((t.1, t.2), (f.w as u32, f.w as u32), "{} {kind:?}: the level's size", r.name);
    let l = backend::gles_surf_int_layout(t.3).unwrap_or_else(|| panic!("{} {kind:?}: not an integer surface", r.name));
    let want = backend::gles_surf_int_layout(r.surf).unwrap();
    assert_eq!((l.bits, l.signed), (want.bits, want.signed), "{} {kind:?}: the store's type", r.name);
    assert_eq!(l.channels, 4, "{} {kind:?}: the wide surface of the store, {}", r.name, gles_surf_name(t.3));
    let bpp = backend::gles_surf_bytes_per_pixel(t.3) as u64;
    assert_eq!(bpp, 4 * (l.bits as u64 / 8), "{} {kind:?}: bytes per pixel of the wide surface", r.name);
    assert_eq!(s.color_mask, own_mask(r), "{} {kind:?}: only the format's channels are written", r.name);
    assert!(s.pitch >= f.w as u32 && s.pitch % 2 == 0, "{} {kind:?}: pitch {} holds a row", r.name, s.pitch);
    Store { gpu: s.gpu, pitch: s.pitch, bpp }
}

/// +X and -Z at level 0 and 1: complete and drawn on the wide surface, at
/// the pair-padded pitch of the level (8, then 4).
#[test]
fn r_and_rg_integer_cube_faces_levels_store_layout() {
    let _g = context();
    for r in ROWS {
        let px0 = expect_texture(&r, Kind::CubeFace(GL_TEXTURE_CUBE_MAP_POSITIVE_X, 0));
        let nz0 = expect_texture(&r, Kind::CubeFace(GL_TEXTURE_CUBE_MAP_NEGATIVE_Z, 0));
        let px1 = expect_texture(&r, Kind::CubeFace(GL_TEXTURE_CUBE_MAP_POSITIVE_X, 1));
        let nz1 = expect_texture(&r, Kind::CubeFace(GL_TEXTURE_CUBE_MAP_NEGATIVE_Z, 1));
        assert_eq!((px0.pitch, nz0.pitch), (8, 8), "{}: level 0 pitch", r.name);
        assert_eq!((px1.pitch, nz1.pitch), (4, 4), "{}: level 1 pitch", r.name);
        assert_eq!(px0.bpp, nz0.bpp, "{}", r.name);
    }
}

/// The addresses inside one cube map: the chain is level-major, the six
/// faces of a level are one face (pitch x height x bytes per pixel) apart,
/// and level 1 starts past level 0's six faces.
#[test]
fn r_and_rg_integer_cube_face_addresses_inside_one_texture() {
    let _g = context();
    for r in ROWS {
        let (mut fb, mut tex) = (0, 0);
        // SAFETY: room for one name each.
        unsafe {
            glGenFramebuffers(1, &mut fb);
            glBindFramebuffer(GL_FRAMEBUFFER, fb);
            glGenTextures(1, &mut tex);
            glBindTexture(GL_TEXTURE_CUBE_MAP, tex);
            glTexStorage2D(GL_TEXTURE_CUBE_MAP, 3, r.ifmt, 8, 8);
        }
        program_one(&r);
        let at = |face: GLenum, level: GLint| -> Store {
            // SAFETY: the face and level are in the storage.
            unsafe { glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, face, tex, level) };
            assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE, "{} face 0x{face:x} level {level}", r.name);
            let (seen, e) = draw();
            assert_eq!(e, GL_NO_ERROR, "{} the draw", r.name);
            let s = seen.unwrap_or_else(|| panic!("{}: the back end saw no draw", r.name));
            assert_eq!(s.refused, None, "{} face 0x{face:x} level {level}", r.name);
            let bpp = backend::gles_surf_bytes_per_pixel(s.targets[0].3) as u64;
            Store { gpu: s.gpu, pitch: s.pitch, bpp }
        };
        let faces: Vec<GLenum> = (0..6).map(|i| GL_TEXTURE_CUBE_MAP_POSITIVE_X + i).collect();
        let l0: Vec<Store> = faces.iter().map(|&f| at(f, 0)).collect();
        let l1: Vec<Store> = faces.iter().map(|&f| at(f, 1)).collect();
        // The chain is level-major: level 0's six faces, then level 1's six.
        let bytes0 = 8 * l0[0].pitch as u64 * l0[0].bpp;
        let bytes1 = 4 * l1[0].pitch as u64 * l1[0].bpp;
        for (i, s) in l0.iter().enumerate() {
            assert_eq!(s.gpu, l0[0].gpu + i as u64 * bytes0, "{}: face {i} level 0 one face apart", r.name);
        }
        assert!(l1[0].gpu >= l0[0].gpu + 6 * bytes0, "{}: level 1 starts past level 0's six faces", r.name);
        for (i, s) in l1.iter().enumerate() {
            assert_eq!(s.gpu, l1[0].gpu + i as u64 * bytes1, "{}: face {i} level 1 one face apart", r.name);
            assert_eq!(s.pitch, 4, "{}: level 1 pitch", r.name);
        }
        // SAFETY: names made above.
        unsafe {
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
            glDeleteFramebuffers(1, &fb);
            glDeleteTextures(1, &tex);
            glUseProgram(0);
        }
    }
}

/// Slices 0..3 of an 8x8x4 TEXTURE_3D: each complete and drawn on the wide
/// surface; the slices are 8x8 sub-rectangles of a 2 x 2 grid at the grid's
/// 16-texel pitch: slice k at column k % 2, row k / 2.
#[test]
fn r_and_rg_integer_texture_3d_slices_store_layout() {
    let _g = context();
    for r in ROWS {
        let (mut fb, mut tex) = (0, 0);
        // SAFETY: room for one name each.
        unsafe {
            glGenFramebuffers(1, &mut fb);
            glBindFramebuffer(GL_FRAMEBUFFER, fb);
            glGenTextures(1, &mut tex);
            glBindTexture(GL_TEXTURE_3D, tex);
            glTexStorage3D(GL_TEXTURE_3D, 1, r.ifmt, 8, 8, 4);
        }
        program_one(&r);
        let mut stores = vec![];
        for n in 0..4 {
            // SAFETY: slice n of 4.
            unsafe { glFramebufferTextureLayer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, tex, 0, n) };
            assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE, "{} slice {n}", r.name);
            let (seen, e) = draw();
            assert_eq!(e, GL_NO_ERROR, "{} slice {n}: the draw", r.name);
            let s = seen.unwrap_or_else(|| panic!("{} slice {n}: the back end saw no draw", r.name));
            assert_eq!(s.refused, None, "{} slice {n}", r.name);
            let t = s.targets[0];
            assert_eq!((t.1, t.2), (8, 8), "{} slice {n}: the slice's size", r.name);
            let l = backend::gles_surf_int_layout(t.3).unwrap();
            let want = backend::gles_surf_int_layout(r.surf).unwrap();
            assert_eq!((l.bits, l.signed, l.channels), (want.bits, want.signed, 4), "{} slice {n}", r.name);
            assert_eq!(s.color_mask, own_mask(&r), "{} slice {n}: the format's channels", r.name);
            let bpp = backend::gles_surf_bytes_per_pixel(t.3) as u64;
            stores.push(Store { gpu: s.gpu, pitch: s.pitch, bpp });
        }
        for (n, s) in stores.iter().enumerate() {
            assert_eq!(s.pitch, 16, "{} slice {n}: the grid's pitch", r.name);
            let (col, row) = (n as u64 % 2, n as u64 / 2);
            let texels = row * 8 * 16 + col * 8;
            assert_eq!(
                s.gpu,
                stores[0].gpu + texels * s.bpp,
                "{} slice {n}: column {col} row {row} of the grid",
                r.name
            );
        }
        // SAFETY: names made above.
        unsafe {
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
            glDeleteFramebuffers(1, &fb);
            glDeleteTextures(1, &tex);
            glUseProgram(0);
        }
    }
}
