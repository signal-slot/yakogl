// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! V3D (Raspberry Pi 4): switching between an R/RG integer FBO, an RGBA8 FBO
//! and the default framebuffer between draws (gap G2/G3 of
//! docs/notes/v3d-format-matrix.md, plan #383).
//!
//! An R/RG integer texture is drawn as its four-channel wide store with the
//! format's channels as the write mask (`WIDE_INT_RT`, `per_target_masks`
//! in src/gl/draw.rs). That state belongs to the draw's target, so a draw
//! after a switch must carry its own FBO's surface format, colour mask and
//! bytes-per-pixel class, and nothing of the previous draw.
//!
//! Held (every case drives the GL core and sees what the stand-in back end is
//! handed; nothing is drawn or verified on hardware):
//!
//! 1. integer FBO -> RGBA8 FBO -> default framebuffer -> integer FBO again
//!    (all twelve formats): each draw's target format, `color_mask`, job
//!    colour mask and bpp class are its own;
//! 2. one FBO whose attachment 0 is swapped between an R/RG integer texture
//!    and an RGBA8 texture between draws (R8I, RG16UI, RG32UI);
//! 3. a clear on one FBO and a draw on the other, interleaved: the integer
//!    clear stays narrowed (G/B 0, A 1) and the RGBA8 clear keeps its values;
//! 4. glColorMask set while the integer FBO is bound is the application's on
//!    the RGBA8 FBO and the default framebuffer, ANDed with the format's
//!    channels on the integer one.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_v3d_int_rg_fbo_switch
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
    /// A clear (alone or fused), with its four raw values as bit patterns.
    do_clear: bool,
    clear_bits: [u32; 4],
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
        do_clear: d.do_clear,
        clear_bits: d.clear_rgba.map(f32::to_bits),
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
    name: c"es3-v3d-int-rg-fbo-switch".as_ptr() as *const c_char,
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
#[allow(dead_code)]
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

/* ---- the RGBA8 side, the default framebuffer, clears --------------------- */

/// A float output at location 0.
fn program_float() {
    program(
        "#version 300 es\nprecision highp float;\nlayout(location = 0) out highp vec4 o0;\n\
         void main() { o0 = vec4(0.25, 0.5, 0.75, 1.0); }\n",
    );
}

/// An RGBA8 renderbuffer or texture FBO; `tex` picks which.
fn rgba8_fbo(tex: bool) -> Fbo {
    let (mut fb, mut name) = (0, 0);
    let mut f = Fbo { fb: 0, tex: vec![], rbs: vec![], w: 8 };
    // SAFETY: room for one name each.
    unsafe {
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        f.fb = fb;
        if tex {
            glGenTextures(1, &mut name);
            glBindTexture(GL_TEXTURE_2D, name);
            glTexStorage2D(GL_TEXTURE_2D, 1, GL_RGBA8, 8, 8);
            glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, name, 0);
            f.tex.push(name);
        } else {
            glGenRenderbuffers(1, &mut name);
            glBindRenderbuffer(GL_RENDERBUFFER, name);
            glRenderbufferStorage(GL_RENDERBUFFER, GL_RGBA8, 8, 8);
            glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_RENDERBUFFER, name);
            f.rbs.push(name);
        }
    }
    assert_eq!(err(), GL_NO_ERROR, "RGBA8 FBO");
    f
}

fn bind(fb: GLuint) {
    // SAFETY: plain GL call.
    unsafe { glBindFramebuffer(GL_FRAMEBUFFER, fb) };
}

fn is_int_surface(format: u8) -> bool {
    backend::gles_surf_int_layout(format).is_some()
}

/// The channels (bit 0 R .. bit 3 A) of the format.
fn own_mask(r: &Row) -> u8 {
    (1u8 << r.channels) - 1
}

/// Draw into the integer FBO bound now: the wide store of `r`, its own mask,
/// and the bpp class of the four-channel store.
fn expect_int_draw(r: &Row, what: &str, app_mask: u8) -> Seen {
    program_one(r);
    let (seen, e) = draw();
    assert_eq!(e, GL_NO_ERROR, "{} {what}: the draw", r.name);
    let s = seen.unwrap_or_else(|| panic!("{} {what}: the back end saw no draw", r.name));
    assert_eq!(s.refused, None, "{} {what}", r.name);
    assert_eq!(s.targets.len(), 1, "{} {what}", r.name);
    let l = backend::gles_surf_int_layout(s.targets[0].3)
        .unwrap_or_else(|| panic!("{} {what}: not an integer surface", r.name));
    let want = backend::gles_surf_int_layout(r.surf).unwrap();
    assert_eq!((l.bits, l.signed, l.channels), (want.bits, want.signed, 4), "{} {what}: the wide store", r.name);
    assert_eq!(
        backend::gles_surf_bytes_per_pixel(s.targets[0].3),
        u32::from(4 * want.bits / 8),
        "{} {what}: bpp class of the wide store",
        r.name
    );
    assert_eq!(s.color_mask, own_mask(r) & app_mask, "{} {what}: the format's channels AND the application's", r.name);
    assert_eq!(s.job.map(|j| j.0), Some(u32::from(own_mask(r) & app_mask)), "{} {what}: job colour mask", r.name);
    s
}

/// Draw into the RGBA8 target bound now (an FBO or the default framebuffer):
/// not an integer surface, 4 bytes a pixel, the application's mask whole.
fn expect_rgba8_draw(r: &Row, what: &str, app_mask: u8) -> Seen {
    program_float();
    let (seen, e) = draw();
    assert_eq!(e, GL_NO_ERROR, "{} {what}: the draw", r.name);
    let s = seen.unwrap_or_else(|| panic!("{} {what}: the back end saw no draw", r.name));
    assert_eq!(s.refused, None, "{} {what}", r.name);
    assert_eq!(s.targets.len(), 1, "{} {what}", r.name);
    let t = s.targets[0];
    assert!(!is_int_surface(t.3), "{} {what}: an integer surface {} leaked", r.name, gles_surf_name(t.3));
    assert_eq!(backend::gles_surf_bytes_per_pixel(t.3), 4, "{} {what}: bpp class", r.name);
    assert_eq!(s.color_mask, app_mask, "{} {what}: the application's mask, not the format's", r.name);
    assert_eq!(s.job.map(|j| j.0), Some(u32::from(app_mask)), "{} {what}: job colour mask", r.name);
    s
}

/// The default-framebuffer draw and the RGBA8 FBO's must agree on the format
/// class; `rgba8_fmt` is the FBO's surface.
const FULL: u8 = 0xf;

/// 1. int FBO -> RGBA8 FBO -> default framebuffer -> int FBO, all twelve.
#[test]
fn integer_fbo_then_rgba8_fbo_then_default_framebuffer_then_integer_again() {
    let _g = context();
    for r in ROWS {
        let fi = fbo(&r, Kind::Texture2d);
        let fr = rgba8_fbo(false);
        let first = expect_int_draw_on(&r, fi.fb, "first");
        bind(fr.fb);
        let rgba = expect_rgba8_draw(&r, "after the integer FBO", FULL);
        bind(0);
        let def = expect_rgba8_draw(&r, "default framebuffer", FULL);
        let again = expect_int_draw_on(&r, fi.fb, "again");
        assert_eq!(first.targets[0].3, again.targets[0].3, "{}: the same surface after the round trip", r.name);
        assert_ne!(first.targets[0].3, rgba.targets[0].3, "{}", r.name);
        assert_ne!(first.targets[0].3, def.targets[0].3, "{}", r.name);
        assert_ne!(first.targets[0].0, rgba.targets[0].0, "{}: each FBO its own store", r.name);
        assert_ne!(rgba.targets[0].0, def.targets[0].0, "{}: the default framebuffer its own", r.name);
    }
}

fn expect_int_draw_on(r: &Row, fb: GLuint, what: &str) -> Seen {
    bind(fb);
    expect_int_draw(r, what, FULL)
}

/// 2. One FBO, attachment 0 swapped between an integer texture and an RGBA8
/// texture between draws.
#[test]
fn one_fbo_swapping_an_integer_and_an_rgba8_attachment_follows_the_attachment() {
    let _g = context();
    for r in ROWS.iter().filter(|r| matches!(r.name, "R8I" | "RG16UI" | "RG32UI")) {
        let (mut fb, mut tex) = (0, [0u32; 2]);
        // SAFETY: room for the names; both stores are 8x8, one level.
        unsafe {
            glGenFramebuffers(1, &mut fb);
            glBindFramebuffer(GL_FRAMEBUFFER, fb);
            glGenTextures(2, tex.as_mut_ptr());
            glBindTexture(GL_TEXTURE_2D, tex[0]);
            glTexStorage2D(GL_TEXTURE_2D, 1, r.ifmt, 8, 8);
            glBindTexture(GL_TEXTURE_2D, tex[1]);
            glTexStorage2D(GL_TEXTURE_2D, 1, GL_RGBA8, 8, 8);
        }
        assert_eq!(err(), GL_NO_ERROR, "{}: storage", r.name);
        let attach = |t: GLuint| {
            // SAFETY: names made above.
            unsafe { glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, t, 0) };
        };
        let mut formats = vec![];
        for round in 0..3 {
            attach(tex[0]);
            assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE, "{} round {round}: integer", r.name);
            let a = expect_int_draw(r, &format!("integer, round {round}"), FULL);
            attach(tex[1]);
            assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE, "{} round {round}: RGBA8", r.name);
            let b = expect_rgba8_draw(r, &format!("RGBA8, round {round}"), FULL);
            assert_ne!(a.targets[0].0, b.targets[0].0, "{} round {round}: the attached store", r.name);
            formats.push((a.targets[0].3, b.targets[0].3));
        }
        assert!(formats.windows(2).all(|w| w[0] == w[1]), "{}: formats drift across swaps {formats:?}", r.name);
        // SAFETY: names made above.
        unsafe {
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
            glDeleteFramebuffers(1, &fb);
            glDeleteTextures(2, tex.as_ptr());
            glUseProgram(0);
        }
    }
}

/// The clears the back end saw since the last take.
fn clears() -> Vec<Seen> {
    // SAFETY: plain GL call.
    unsafe { glFinish() };
    core::mem::take(&mut *SEEN.lock().unwrap()).into_iter().filter(|s| s.do_clear).collect()
}

/// 3. Clear on one FBO, draw on the other, interleaved.
#[test]
fn clears_and_draws_interleaved_between_integer_and_rgba8_fbos_keep_their_values() {
    let _g = context();
    let iv: [GLint; 4] = [5, 6, 7, 8];
    let fv: [GLfloat; 4] = [0.25, 0.5, 0.75, 1.0];
    for r in ROWS.iter().filter(|r| matches!(r.name, "R8I" | "RG16UI" | "RG32UI")) {
        let fi = fbo(r, Kind::Texture2d);
        let fr = rgba8_fbo(false);
        for round in 0..2 {
            let what = format!("round {round}");
            bind(fi.fb);
            SEEN.lock().unwrap().clear();
            // SAFETY: four values each.
            unsafe {
                if r.signed {
                    glClearBufferiv(GL_COLOR, 0, iv.as_ptr());
                } else {
                    glClearBufferuiv(GL_COLOR, 0, iv.as_ptr() as *const GLuint);
                }
            }
            let c = clears();
            assert_eq!(c.len(), 1, "{} {what}: one integer clear", r.name);
            let want: [u32; 4] = if r.channels == 1 { [5, 0, 0, 1] } else { [5, 6, 0, 1] };
            assert_eq!(c[0].clear_bits, want, "{} {what}: own channels kept, G/B 0, A 1", r.name);

            bind(fr.fb);
            expect_rgba8_draw(r, &format!("{what}: RGBA8 draw between"), FULL);
            SEEN.lock().unwrap().clear();
            // SAFETY: four values.
            unsafe { glClearBufferfv(GL_COLOR, 0, fv.as_ptr()) };
            let c = clears();
            assert_eq!(c.len(), 1, "{} {what}: one RGBA8 clear", r.name);
            assert_eq!(c[0].clear_bits, fv.map(f32::to_bits), "{} {what}: RGBA8 clear is not narrowed", r.name);
            assert!(!is_int_surface(c[0].targets[0].3), "{} {what}: RGBA8 clear surface", r.name);

            bind(fi.fb);
            expect_int_draw(r, &format!("{what}: integer draw after the RGBA8 clear"), FULL);
        }
    }
}

/// 4. glColorMask set while the integer FBO is bound.
#[test]
fn a_colour_mask_set_with_the_integer_fbo_bound_applies_per_framebuffer() {
    let _g = context();
    for r in ROWS {
        let fi = fbo(&r, Kind::Texture2d);
        let fr = rgba8_fbo(true);
        for app in [0b1011u8, 0b0101, 0b1111] {
            // SAFETY: plain GL call.
            unsafe {
                glColorMask(app & 1, app >> 1 & 1, app >> 2 & 1, app >> 3 & 1);
            }
            let what = format!("mask {app:#06b}");
            bind(fi.fb);
            expect_int_draw(&r, &what, app);
            bind(fr.fb);
            expect_rgba8_draw(&r, &format!("{what}: RGBA8 FBO"), app);
            bind(0);
            expect_rgba8_draw(&r, &format!("{what}: default framebuffer"), app);
            bind(fi.fb);
            expect_int_draw(&r, &format!("{what}: integer again"), app);
        }
        // SAFETY: plain GL call.
        unsafe { glColorMask(1, 1, 1, 1) };
    }
}
