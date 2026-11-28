// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! V3D (Raspberry Pi 4): an attached R or RG integer texture that is
//! respecified, deleted or re-attached (gap G2 of docs/notes/v3d-format-matrix.md,
//! plan #383). The twelve formats (R8I .. RG32UI) are drawn as their store's
//! wide surface with the format's channels as the write mask
//! (`WIDE_INT_RT`, `fb_target_channel_mask_n`, src/gl/framebuffer.rs).
//!
//! Held, each for the twelve formats where it applies:
//!
//! 1. glTexImage2D of the attached texture as a non-renderable format (RGB8I)
//!    makes the framebuffer incomplete; as RGBA8 it is complete and the draw's
//!    surface and mask follow (RGBA8, all four channels);
//! 2. respecified R8I -> RG32UI (and the reverse, and each format to every
//!    other) on the same name: the target surface and the channel mask follow
//!    the new store;
//! 3. the attached texture deleted: the attachment is GL_NONE and the
//!    framebuffer is GL_FRAMEBUFFER_INCOMPLETE_MISSING_ATTACHMENT;
//! 4. detached and re-attached at another level or layer: the drawn level's
//!    size and the layer's address follow.
//!
//! What it cannot see: the GPU. The back end is a stand-in that records each
//! draw; a drawn shader value is not read back. Nothing here is verified on
//! hardware.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_v3d_int_rg_respecify
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
    name: c"es3-v3d-int-rg-respecify".as_ptr() as *const c_char,
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

/// A bound framebuffer with the format at attachment 0, and whatever else
/// was attached; dropped, it unbinds and deletes them.
struct Fbo {
    fb: GLuint,
    tex: Vec<GLuint>,
    rbs: Vec<GLuint>,
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
        if ok != 1 {
            let mut log = vec![0u8; 1024];
            let mut n: GLsizei = 0;
            glGetShaderInfoLog(s, 1024, &mut n, log.as_mut_ptr() as *mut c_char);
            panic!("{text:?} did not compile: {}", String::from_utf8_lossy(&log[..n as usize]));
        }
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
        // the shaders and the program made before are not needed: the tests
        // make hundreds of programs, and a context holds a bounded number
        glDeleteShader(v);
        glDeleteShader(f);
        let old = PROG.swap(p, Ordering::Relaxed);
        if old != 0 {
            glDeleteProgram(old);
        }
    }
}

static PROG: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

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

/* ---- helpers for a texture respecified while attached --------------------- */

fn own_mask(r: &Row) -> u8 {
    (1u8 << r.channels) - 1
}

/// glTexImage2D of the bound TEXTURE_2D, level `level`, `w` x `w`, no data.
fn tex_image(ifmt: GLenum, format: GLenum, ty: GLenum, level: GLint, w: GLsizei) {
    // SAFETY: a null pointer is "no data".
    unsafe { glTexImage2D(GL_TEXTURE_2D, level, ifmt as GLint, w, w, 0, format, ty, core::ptr::null()) };
}

/// glTexImage2D of the row's own format.
fn tex_image_row(r: &Row, level: GLint, w: GLsizei) {
    let format = if r.channels == 1 { GL_RED_INTEGER } else { GL_RG_INTEGER };
    let bits = match r.surf {
        GLES_SURF_R8I | GLES_SURF_R8UI | GLES_SURF_RG8I | GLES_SURF_RG8UI => 8,
        GLES_SURF_R16I | GLES_SURF_R16UI | GLES_SURF_RG16I | GLES_SURF_RG16UI => 16,
        _ => 32,
    };
    let ty = match (bits, r.signed) {
        (8, true) => GL_BYTE,
        (8, false) => GL_UNSIGNED_BYTE,
        (16, true) => GL_SHORT,
        (16, false) => GL_UNSIGNED_SHORT,
        (_, true) => GL_INT,
        (_, false) => GL_UNSIGNED_INT,
    };
    tex_image(r.ifmt, format, ty, level, w);
}

/// A mutable 8x8 TEXTURE_2D of the row's format, attached at attachment 0 of a
/// new bound framebuffer (the texture is left bound to TEXTURE_2D).
fn attached_mutable(r: &Row) -> Fbo {
    let (mut fb, mut name) = (0, 0);
    // SAFETY: room for one name each.
    unsafe {
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        glGenTextures(1, &mut name);
        glBindTexture(GL_TEXTURE_2D, name);
    }
    tex_image_row(r, 0, 8);
    // SAFETY: the texture was made above.
    unsafe { glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, name, 0) };
    assert_eq!(err(), GL_NO_ERROR, "{}: mutable texture and attach", r.name);
    Fbo { fb, tex: vec![name], rbs: vec![] }
}

fn program_rgba8() {
    program(
        "#version 300 es\nprecision highp float;\nlayout(location = 0) out highp vec4 o0;\n\
         void main() { o0 = vec4(0.25, 0.5, 0.75, 1.0); }\n",
    );
}

/// The draw into the texture of row `r` (the wide store's surface, the
/// format's channels as the mask) at the attachment's `w` x `w` size; returns
/// the target's store address.
fn expect_wide_draw(r: &Row, w: GLsizei, what: &str) -> u64 {
    assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE, "{} {what}", r.name);
    program_one(r);
    let (seen, e) = draw();
    assert_eq!(e, GL_NO_ERROR, "{} {what}: the draw", r.name);
    let s = seen.unwrap_or_else(|| panic!("{} {what}: the back end saw no draw", r.name));
    assert_eq!(s.refused, None, "{} {what}", r.name);
    assert_eq!(s.targets.len(), 1, "{} {what}", r.name);
    let t = s.targets[0];
    assert_ne!(t.0, 0, "{} {what}: the store", r.name);
    assert_eq!((t.1, t.2), (w as u32, w as u32), "{} {what}: the level's size", r.name);
    let l = backend::gles_surf_int_layout(t.3).unwrap_or_else(|| panic!("{} {what}: not an integer surface", r.name));
    let want = backend::gles_surf_int_layout(r.surf).unwrap();
    assert_eq!(
        (l.bits, l.signed, l.channels),
        (want.bits, want.signed, 4),
        "{} {what}: the wide store, {}",
        r.name,
        gles_surf_name(t.3)
    );
    assert_eq!(s.color_mask, own_mask(r), "{} {what}: the format's channels", r.name);
    assert!(s.job.is_some(), "{} {what}: the V3D's job state takes the draw", r.name);
    t.0
}

fn attachment_type() -> GLint {
    let mut v: GLint = -1;
    // SAFETY: one value.
    unsafe {
        glGetFramebufferAttachmentParameteriv(
            GL_FRAMEBUFFER,
            GL_COLOR_ATTACHMENT0,
            GL_FRAMEBUFFER_ATTACHMENT_OBJECT_TYPE,
            &mut v,
        )
    };
    v
}

/* ---- (1) respecified as a non-renderable format and as RGBA8 --------------- */

/// RGB8I is not colour-renderable: the framebuffer is incomplete the moment
/// the attached texture is respecified, and complete again with RGBA8 (drawn
/// as RGBA8, all four channels) and with the row's own format (wide store and
/// the format's channels).
#[test]
fn respecify_as_non_renderable_then_rgba8_follows_the_new_format() {
    let _g = context();
    for r in ROWS {
        let _f = attached_mutable(&r);
        expect_wide_draw(&r, 8, "as created");

        tex_image(GL_RGB8I, GL_RGB_INTEGER, GL_BYTE, 0, 8);
        assert_eq!(err(), GL_NO_ERROR, "{}: RGB8I", r.name);
        assert_eq!(status(), GL_FRAMEBUFFER_INCOMPLETE_ATTACHMENT, "{}: respecified as RGB8I", r.name);

        tex_image(GL_RGBA8, GL_RGBA, GL_UNSIGNED_BYTE, 0, 8);
        assert_eq!(err(), GL_NO_ERROR, "{}: RGBA8", r.name);
        assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE, "{}: respecified as RGBA8", r.name);
        program_rgba8();
        let (seen, e) = draw();
        assert_eq!(e, GL_NO_ERROR, "{}: the RGBA8 draw", r.name);
        let s = seen.unwrap_or_else(|| panic!("{}: the back end saw no RGBA8 draw", r.name));
        assert_eq!(s.refused, None, "{}: RGBA8", r.name);
        assert_eq!(s.targets.len(), 1, "{}: RGBA8", r.name);
        assert_eq!(s.targets[0].3, GLES_SURF_RGBA8888, "{}: RGBA8 surface {}", r.name, gles_surf_name(s.targets[0].3));
        assert_eq!(s.color_mask, 0xf, "{}: RGBA8 writes all four channels", r.name);

        tex_image_row(&r, 0, 8);
        assert_eq!(err(), GL_NO_ERROR, "{}: back to the format", r.name);
        expect_wide_draw(&r, 8, "respecified back");
    }
}

/* ---- (2) respecified as another R/RG integer format ------------------------ */

/// Each of the twelve respecified as each of the twelve while attached: the
/// target is the new store's wide surface and the mask is the new format's
/// channels.
#[test]
fn respecify_as_another_integer_format_follows_the_new_store() {
    let _g = context();
    for a in ROWS {
        let _f = attached_mutable(&a);
        expect_wide_draw(&a, 8, "as created");
        for b in ROWS {
            tex_image_row(&b, 0, 8);
            assert_eq!(err(), GL_NO_ERROR, "{} -> {}", a.name, b.name);
            expect_wide_draw(&b, 8, &format!("respecified from {}", a.name));
        }
    }
}

/* ---- (3) deleted while attached ------------------------------------------- */

/// glDeleteTextures of the attached texture detaches it: GL_NONE, and the
/// framebuffer is incomplete (missing attachment); a draw is refused with
/// GL_INVALID_FRAMEBUFFER_OPERATION and reaches no back end.
#[test]
fn deleting_the_attached_texture_leaves_the_framebuffer_missing_an_attachment() {
    let _g = context();
    for r in ROWS {
        let f = attached_mutable(&r);
        expect_wide_draw(&r, 8, "before the delete");
        assert_eq!(attachment_type(), GL_TEXTURE as GLint, "{}: attached", r.name);
        // SAFETY: a name this test made.
        unsafe { glDeleteTextures(1, f.tex.as_ptr()) };
        assert_eq!(err(), GL_NO_ERROR, "{}: delete", r.name);
        assert_eq!(attachment_type(), GL_NONE as GLint, "{}: the attachment after the delete", r.name);
        assert_eq!(status(), GL_FRAMEBUFFER_INCOMPLETE_MISSING_ATTACHMENT, "{}", r.name);
        let (seen, e) = draw();
        assert!(seen.is_none(), "{}: a draw reached the back end", r.name);
        assert_eq!(e, GL_INVALID_FRAMEBUFFER_OPERATION, "{}", r.name);
    }
}

/* ---- (4) detached and re-attached at another level or layer ---------------- */

/// A 3-level TEXTURE_2D: attached at level 0, detached (missing), re-attached
/// at levels 2, then 1: the size and the store address follow the level.
#[test]
fn reattaching_at_another_level_follows_the_level() {
    let _g = context();
    for r in ROWS {
        let (mut fb, mut name) = (0, 0);
        // SAFETY: room for one name each.
        unsafe {
            glGenFramebuffers(1, &mut fb);
            glBindFramebuffer(GL_FRAMEBUFFER, fb);
            glGenTextures(1, &mut name);
            glBindTexture(GL_TEXTURE_2D, name);
            glTexStorage2D(GL_TEXTURE_2D, 3, r.ifmt, 8, 8);
        }
        let _f = Fbo { fb, tex: vec![name], rbs: vec![] };
        let attach = |level: GLint, tex: GLuint| {
            // SAFETY: the texture has these levels.
            unsafe { glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex, level) };
        };
        attach(0, name);
        let a0 = expect_wide_draw(&r, 8, "level 0");
        attach(0, 0);
        assert_eq!(status(), GL_FRAMEBUFFER_INCOMPLETE_MISSING_ATTACHMENT, "{}: detached", r.name);
        attach(2, name);
        let a2 = expect_wide_draw(&r, 2, "level 2");
        attach(1, name);
        let a1 = expect_wide_draw(&r, 4, "level 1");
        assert!(a0 != a1 && a1 != a2 && a0 != a2, "{}: levels share a store address", r.name);
        assert_eq!(err(), GL_NO_ERROR, "{}", r.name);
    }
}

/// A 3-layer TEXTURE_2D_ARRAY: attached at layer 0, detached, re-attached at
/// layers 2 and 1: the store address follows the layer, the size does not.
#[test]
fn reattaching_at_another_layer_follows_the_layer() {
    let _g = context();
    for r in ROWS {
        let (mut fb, mut name) = (0, 0);
        // SAFETY: room for one name each.
        unsafe {
            glGenFramebuffers(1, &mut fb);
            glBindFramebuffer(GL_FRAMEBUFFER, fb);
            glGenTextures(1, &mut name);
            glBindTexture(GL_TEXTURE_2D_ARRAY, name);
            glTexStorage3D(GL_TEXTURE_2D_ARRAY, 1, r.ifmt, 8, 8, 3);
        }
        let _f = Fbo { fb, tex: vec![name], rbs: vec![] };
        let attach = |layer: GLint, tex: GLuint| {
            // SAFETY: the array has these layers.
            unsafe { glFramebufferTextureLayer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, tex, 0, layer) };
        };
        attach(0, name);
        let l0 = expect_wide_draw(&r, 8, "layer 0");
        attach(0, 0);
        assert_eq!(status(), GL_FRAMEBUFFER_INCOMPLETE_MISSING_ATTACHMENT, "{}: detached", r.name);
        attach(2, name);
        let l2 = expect_wide_draw(&r, 8, "layer 2");
        attach(1, name);
        let l1 = expect_wide_draw(&r, 8, "layer 1");
        assert!(l0 != l1 && l1 != l2 && l0 != l2, "{}: layers share a store address", r.name);
        assert_eq!(err(), GL_NO_ERROR, "{}", r.name);
    }
}
