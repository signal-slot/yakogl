// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! V3D (Raspberry Pi 4): clears of R and RG integer colour attachments (gap G2
//! of docs/notes/v3d-format-matrix.md, plan #383; the clear half of
//! tests/es3_v3d_fbo_int_rg.rs).
//!
//! An R or RG integer texture is held in and drawn as the four-channel store
//! of the same type and width. The TLB clear words (`state::clear_words`) and
//! the clear quad carry all four values of `GlesDraw::clear_rgba` into the
//! store, so the GL core narrows them (`draw.rs` `int_clear_channels`,
//! `NARROW_INT_CLEAR`, V3D only): the channels the format lacks get 0 (G, B)
//! and 1 (A), never the application's B and A.
//!
//! Renderbuffers: an R/RG integer renderbuffer's clear reaches the back end
//! on the format's own surface with the raw values, target mask and quad
//! decision the GL core sets (a scissor or a partial colour mask make it a
//! quad); a clear of an R8I renderbuffer at attachment 1 beside a float
//! attachment 0 addresses draw buffer 1 only.
//!
//! Textures: for each of the twelve formats, glClearBuffer{i,ui}v {5, 6, 7,
//! 8} at attachment 0, at layer 1 of a 2D array and at level 1 leaves 0 (G,
//! B) and 1 (A) in the store's absent channels; a glClear with a partial
//! colour mask or a scissor (the quad path) changes only the format's
//! channels; an R8I texture at attachment 1 beside a float attachment 0
//! keeps RT 0's clear value and is narrowed, or the clear is refused by a
//! named reason (one clear value for every target: the GL core drops it with
//! GL_INVALID_OPERATION and a "clear refused" log line).
//!
//! What the stand-in sees, and what it cannot. It sees each clear as the
//! `GlesDraw` the back end is handed: `clear_rgba`, `clear_color_mask`,
//! `clear_quad`, `clear_target_mask`, the target formats, and the clear words
//! `state::clear_words(format, clear_rgba)` the TLB clear writes (word 0
//! packs the four bytes of an 8I store, words 0-1 two halves each for 16I, the
//! four words of a 32I store). It does NOT see the store's hidden channels:
//! the stand-in writes a clear in the layout of target 0 and the GL core
//! reads back 0, 0, 1 for an absent channel whatever the store holds, so a
//! read-back cannot tell a narrowed clear from a wide one (it is checked only
//! where it holds). It also cannot see the quad's colour uniform once the
//! back end builds it (`clear_quad_draw`, called from src/v3d/backend.rs): for
//! the quad path the cases accept a clear mask that leaves the absent
//! channels out (nothing is written there) or narrowed values. The GPU is not
//! involved; nothing here is verified on hardware.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_v3d_int_rg_clear
//! ```

#![cfg(all(feature = "host-test", feature = "shaderc-online", feature = "gpu-v3d", not(feature = "report-es2")))]

use core::ffi::{c_char, c_void};
use std::sync::atomic::Ordering;
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{self, GlesDraw, GlesHeap, YakoglBackend, *};
use yakogl::gl::consts::*;
use yakogl::gl::es3::*;
use yakogl::gl::framebuffer::*;
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

/// One clear as the back end was handed it, and what the V3D made of it.
#[derive(Clone, Debug)]
struct Seen {
    /// `targets[..num_targets]`: (gpu, w, h, format).
    targets: Vec<(u64, u32, u32, u8)>,
    do_clear: bool,
    /// The four raw values, as bit patterns (integer targets).
    clear_bits: [u32; 4],
    clear_color_mask: u8,
    clear_quad: bool,
    clear_target_mask: u8,
    /// `state::colour_needs_quad`: the quad path rather than the TLB clear.
    needs_quad: bool,
    /// `state::refusal` / `state::unsupported` (a clear has no job state).
    refused: Option<&'static str>,
}

static SEEN: Mutex<Vec<Seen>> = Mutex::new(Vec::new());

/// Records each clear; one of an integer target also stores the low bits of
/// the four raw values in target 0's layout, little endian, rows tightly
/// packed (the stand-in's own layout: the GL core reads back what it was told).
unsafe extern "C" fn b_draw(d: *const GlesDraw) -> i32 {
    let d = &*d;
    let n = (d.num_targets.max(1) as usize).min(GLES_MAX_TARGETS);
    SEEN.lock().unwrap().push(Seen {
        targets: d.targets[..n].iter().map(|t| (t.gpu, t.w, t.h, t.format)).collect(),
        do_clear: d.do_clear,
        clear_bits: d.clear_rgba.map(f32::to_bits),
        clear_color_mask: d.clear_color_mask,
        clear_quad: d.clear_quad,
        clear_target_mask: d.clear_target_mask,
        needs_quad: state::colour_needs_quad(d),
        refused: state::refusal(d).or_else(|| state::unsupported(d)),
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
    name: c"es3-v3d-int-rg-clear".as_ptr() as *const c_char,
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
        glDisable(GL_SCISSOR_TEST);
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
    /// The read-back of a clear of 5, 6, 7, 8: the format's channels, then
    /// 0, 0, 1 for the absent ones (ES 3.0 4.3.2).
    fn expected(&self) -> [u32; 4] {
        if self.channels == 1 {
            [5, 0, 0, 1]
        } else {
            [5, 6, 0, 1]
        }
    }
    /// Bit k: the format has channel k (R, G).
    fn channel_mask(&self) -> u8 {
        if self.channels == 1 {
            0b0001
        } else {
            0b0011
        }
    }
}

/// `raw` with the channels the format lacks as the store must hold them:
/// 0 in G (R formats) and B, 1 in A. Bit patterns, as `clear_rgba`'s.
fn narrowed_bits(r: &Row, raw: [u32; 4]) -> [u32; 4] {
    let mut v = raw;
    for (ch, w) in v.iter_mut().enumerate() {
        if ch >= r.channels as usize {
            *w = u32::from(ch == 3);
        }
    }
    v
}

/// The clear words of `format` for raw clear values `bits`
/// (`state::clear_words`).
fn words(format: u8, bits: [u32; 4]) -> [u32; 4] {
    state::clear_words(format, bits.map(f32::from_bits))
}

/// Does this clear keep the application's B and A out of the store's absent
/// channels? Either the clear words of the target's own format are those of
/// the narrowed values (0, 0, 1 in the absent channels), or the clear's mask
/// leaves the absent channels alone (quad path only).
fn hidden_channels_clean(r: &Row, s: &Seen, t_format: u8) -> bool {
    let narrowed = words(t_format, s.clear_bits) == words(t_format, narrowed_bits(r, s.clear_bits));
    let masked_out = s.needs_quad && s.clear_color_mask & 0xf & !r.channel_mask() == 0 && s.clear_color_mask != 0;
    narrowed || masked_out
}

/* ---- attachments, a clear, a read-back ----------------------------------- */

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
        }
    }
}

/// Attach `ifmt` of `kind` at colour attachment `at` of the bound `f`.
fn attach(f: &mut Fbo, ifmt: GLenum, kind: Kind, at: GLenum) {
    let mut name = 0;
    // SAFETY: room for one name each; the layer and level named are there.
    unsafe {
        match kind {
            Kind::Renderbuffer => {
                glGenRenderbuffers(1, &mut name);
                glBindRenderbuffer(GL_RENDERBUFFER, name);
                glRenderbufferStorage(GL_RENDERBUFFER, ifmt, 8, 8);
                glFramebufferRenderbuffer(GL_FRAMEBUFFER, at, GL_RENDERBUFFER, name);
                f.rbs.push(name);
            }
            Kind::Texture2d | Kind::Level1 => {
                let (levels, level) = if matches!(kind, Kind::Level1) { (3, 1) } else { (1, 0) };
                glGenTextures(1, &mut name);
                glBindTexture(GL_TEXTURE_2D, name);
                glTexStorage2D(GL_TEXTURE_2D, levels, ifmt, 8, 8);
                glFramebufferTexture2D(GL_FRAMEBUFFER, at, GL_TEXTURE_2D, name, level);
                f.tex.push(name);
                if at == GL_COLOR_ATTACHMENT0 {
                    f.w = 8 >> level;
                }
            }
            Kind::ArrayLayer => {
                glGenTextures(1, &mut name);
                glBindTexture(GL_TEXTURE_2D_ARRAY, name);
                glTexStorage3D(GL_TEXTURE_2D_ARRAY, 1, ifmt, 8, 8, 3);
                glFramebufferTextureLayer(GL_FRAMEBUFFER, at, name, 0, 1);
                f.tex.push(name);
            }
        }
    }
    assert_eq!(err(), GL_NO_ERROR, "{ifmt:#x} {kind:?}: storage and attach");
}

fn fbo(r: &Row, kind: Kind) -> Fbo {
    let mut fb = 0;
    // SAFETY: room for one name.
    unsafe {
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
    }
    let mut f = Fbo { fb, tex: vec![], rbs: vec![], w: 8 };
    attach(&mut f, r.ifmt, kind, GL_COLOR_ATTACHMENT0);
    f
}

/// An RGBA8 renderbuffer at attachment 0 and `r`'s format of `kind` at
/// attachment 1; both are draw buffers.
fn fbo_float_then(r: &Row, kind: Kind) -> Fbo {
    let mut fb = 0;
    // SAFETY: room for one name.
    unsafe {
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
    }
    let mut f = Fbo { fb, tex: vec![], rbs: vec![], w: 8 };
    attach(&mut f, GL_RGBA8, Kind::Renderbuffer, GL_COLOR_ATTACHMENT0);
    attach(&mut f, r.ifmt, kind, GL_COLOR_ATTACHMENT0 + 1);
    let bufs = [GL_COLOR_ATTACHMENT0, GL_COLOR_ATTACHMENT0 + 1];
    // SAFETY: two draw buffers listed.
    unsafe { glDrawBuffers(2, bufs.as_ptr()) };
    f
}

/// The clears the back end saw since the last call (the last one is the one
/// the test made; a clear is sent at glFinish at the latest).
fn clears() -> Vec<Seen> {
    // SAFETY: plain GL call.
    unsafe { glFinish() };
    core::mem::take(&mut *SEEN.lock().unwrap()).into_iter().filter(|s| s.do_clear).collect()
}

/// glClearBuffer{i,ui}v of draw buffer `buf` with 5, 6, 7, 8.
fn clear_buffer(r: &Row, buf: GLint) {
    let v: [GLint; 4] = [5, 6, 7, 8];
    // SAFETY: four values.
    unsafe {
        if r.signed {
            glClearBufferiv(GL_COLOR, buf, v.as_ptr());
        } else {
            glClearBufferuiv(GL_COLOR, buf, v.as_ptr() as *const GLuint);
        }
    }
}

/// glReadPixels of the `w` x `w` attachment as GL_RGBA_INTEGER: each pixel must
/// be [`Row::expected`].
fn read_back(r: &Row, w: GLsizei, what: &str) {
    let mut px = vec![0u32; (w * w * 4) as usize];
    // SAFETY: room for w x w pixels of four 32-bit values.
    unsafe {
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
    assert_eq!(err(), GL_NO_ERROR, "{} {what}: read back", r.name);
    for (n, p) in px.chunks(4).enumerate() {
        assert_eq!(p, r.expected(), "{} {what}: pixel {n}", r.name);
    }
}

/* ---- the renderbuffers: held today ---------------------------------------- */

/// A glClearBuffer of an R/RG renderbuffer at attachment 0 reaches the back
/// end once, on the format's own surface, with the application's raw values,
/// draw buffer 0 and the TLB path (no quad); it reads back 0, 0, 1 in the
/// absent channels.
#[test]
fn an_r_or_rg_renderbuffer_clear_reaches_the_back_end_on_its_own_surface() {
    let _g = context();
    for r in ROWS {
        let f = fbo(&r, Kind::Renderbuffer);
        assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE, "{}", r.name);
        SEEN.lock().unwrap().clear();
        clear_buffer(&r, 0);
        assert_eq!(err(), GL_NO_ERROR, "{}", r.name);
        let c = clears();
        assert_eq!(c.len(), 1, "{}: one clear", r.name);
        let s = &c[0];
        assert_eq!(s.refused, None, "{}", r.name);
        assert_eq!(s.targets.len(), 1, "{}", r.name);
        assert_eq!(s.targets[0].3, r.surf, "{}: surface {}", r.name, gles_surf_name(s.targets[0].3));
        assert_eq!(s.clear_bits, [5, 6, 7, 8], "{}: the raw values, as given", r.name);
        assert_eq!(s.clear_target_mask & 1, 1, "{}: draw buffer 0", r.name);
        assert!(!s.needs_quad && !s.clear_quad, "{}: a whole-target clear is the TLB's", r.name);
        read_back(&r, f.w, "renderbuffer");
    }
}

/// glClear with a scissor, or with a colour mask that leaves channels out, is
/// the quad path (`state::colour_needs_quad`) on an R/RG renderbuffer, and the
/// clear's own mask is carried as the application set it.
#[test]
fn an_r_or_rg_renderbuffer_glclear_with_a_scissor_or_a_mask_is_the_quad_path() {
    let _g = context();
    for r in ROWS {
        let _f = fbo(&r, Kind::Renderbuffer);
        // SAFETY: plain GL calls.
        unsafe {
            glClearColor(0.25, 0.5, 0.75, 1.0);
            glColorMask(1, 0, 0, 0);
            glClear(GL_COLOR_BUFFER_BIT);
        }
        let c = clears();
        assert_eq!(c.len(), 1, "{}: one clear", r.name);
        assert!(c[0].needs_quad, "{}: a partial mask is a quad", r.name);
        assert_eq!(c[0].clear_color_mask, 0b0001, "{}: the application's mask", r.name);
        assert_eq!(c[0].refused, None, "{}", r.name);

        // SAFETY: plain GL calls.
        unsafe {
            glColorMask(1, 1, 1, 1);
            glEnable(GL_SCISSOR_TEST);
            glScissor(1, 1, 3, 3);
            glClear(GL_COLOR_BUFFER_BIT);
            glDisable(GL_SCISSOR_TEST);
        }
        let c = clears();
        assert_eq!(c.len(), 1, "{}: one clear", r.name);
        assert!(c[0].needs_quad && c[0].clear_quad, "{}: a scissored clear is a quad", r.name);
        assert_eq!(c[0].refused, None, "{}", r.name);
    }
}

/// An R8I renderbuffer at attachment 1 beside an RGBA8 one at attachment 0:
/// glClearBufferiv of draw buffer 1 addresses RT 1 only, on the R8I surface,
/// and leaves RT 0 out of the clear.
#[test]
fn clearing_an_r8i_renderbuffer_at_attachment_1_beside_a_float_target_addresses_it_only() {
    let _g = context();
    let r = ROWS[0];
    let _f = fbo_float_then(&r, Kind::Renderbuffer);
    assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE);
    SEEN.lock().unwrap().clear();
    clear_buffer(&r, 1);
    assert_eq!(err(), GL_NO_ERROR);
    let c = clears();
    assert_eq!(c.len(), 1, "one clear");
    let s = &c[0];
    assert_eq!(s.refused, None);
    assert_eq!(s.targets.len(), 2, "two targets");
    assert_eq!(s.targets[1].3, GLES_SURF_R8I, "RT 1 is the R8I surface");
    assert_eq!(s.clear_target_mask, 0b10, "draw buffer 1 only: RT 0 is not cleared");
    assert_eq!(s.clear_bits, [5, 6, 7, 8]);
}

/* ---- the textures: owed to the wide store and the narrowing --------------- */

/// glClearBuffer{i,ui}v {5, 6, 7, 8} at attachment 0 of `kind`: the texture is
/// complete, the clear is taken and reads back 0, 0, 1 in the absent
/// channels, and the clear words of the store's surface hold 0 (G/B) and 1
/// (A) there.
fn expect_narrowed_clear(r: &Row, kind: Kind) {
    let f = fbo(r, kind);
    assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE, "{} {kind:?}", r.name);
    SEEN.lock().unwrap().clear();
    clear_buffer(r, 0);
    assert_eq!(err(), GL_NO_ERROR, "{} {kind:?}: the clear", r.name);
    let c = clears();
    assert_eq!(c.len(), 1, "{} {kind:?}: one clear", r.name);
    let s = &c[0];
    assert_eq!(s.refused, None, "{} {kind:?}", r.name);
    assert_eq!(s.clear_target_mask & 1, 1, "{} {kind:?}: draw buffer 0", r.name);
    let t = s.targets[0];
    assert_eq!((t.1, t.2), (f.w as u32, f.w as u32), "{} {kind:?}: the level's size", r.name);
    let l = backend::gles_surf_int_layout(t.3).unwrap_or_else(|| panic!("{} {kind:?}: not an integer surface", r.name));
    assert!(l.channels >= r.channels, "{} {kind:?}: the store holds the format's channels", r.name);
    // the format's channels keep the application's values ...
    assert_eq!(&s.clear_bits[..r.channels as usize], &[5, 6][..r.channels as usize], "{} {kind:?}", r.name);
    // ... and the absent ones are 0 (G/B) and 1 (A) in the clear words
    assert!(
        hidden_channels_clean(r, s, t.3),
        "{} {kind:?}: clear words {:x?} carry the application's B/A (want {:x?}) in the hidden channels of {}",
        r.name,
        words(t.3, s.clear_bits),
        words(t.3, narrowed_bits(r, s.clear_bits)),
        gles_surf_name(t.3)
    );
    read_back(r, f.w, &format!("{kind:?}"));
}

/// A TEXTURE_2D of each of the twelve at GL_COLOR_ATTACHMENT0.
#[test]
fn texture_2d_clear_at_attachment_0_leaves_0_0_1_in_the_absent_channels() {
    let _g = context();
    for r in ROWS {
        expect_narrowed_clear(&r, Kind::Texture2d);
    }
}

/// Layer 1 of a TEXTURE_2D_ARRAY.
#[test]
fn texture_2d_array_layer_clear_leaves_0_0_1_in_the_absent_channels() {
    let _g = context();
    for r in ROWS {
        expect_narrowed_clear(&r, Kind::ArrayLayer);
    }
}

/// Level 1 of a mipmapped TEXTURE_2D.
#[test]
fn texture_level_1_clear_leaves_0_0_1_in_the_absent_channels() {
    let _g = context();
    for r in ROWS {
        expect_narrowed_clear(&r, Kind::Level1);
    }
}

/// glClear (the float clear colour, which is the application's for an integer
/// buffer undefined, but its B and A must still not reach the absent
/// channels) with only R written, and then with a scissor and every channel:
/// the quad path. Only the format's channels may change: the quad's mask
/// excludes the absent ones, or the values are narrowed.
#[test]
fn texture_glclear_quad_path_changes_only_the_formats_channels() {
    let _g = context();
    for r in ROWS {
        let f = fbo(&r, Kind::Texture2d);
        assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE, "{}", r.name);
        // SAFETY: plain GL calls.
        unsafe {
            glClearColor(0.25, 0.5, 0.75, 1.0);
            glColorMask(1, 0, 0, 0);
            glClear(GL_COLOR_BUFFER_BIT);
        }
        let c = clears();
        assert_eq!(c.len(), 1, "{}: one clear", r.name);
        let s = &c[0];
        assert!(s.needs_quad, "{}: a partial mask is a quad", r.name);
        assert_eq!(s.refused, None, "{}", r.name);
        assert!(hidden_channels_clean(&r, s, s.targets[0].3), "{}: mask R only wrote a hidden channel: {s:?}", r.name);

        // SAFETY: plain GL calls.
        unsafe {
            glColorMask(1, 1, 1, 1);
            glEnable(GL_SCISSOR_TEST);
            glScissor(1, 1, 3, 3);
            glClear(GL_COLOR_BUFFER_BIT);
            glDisable(GL_SCISSOR_TEST);
        }
        let c = clears();
        assert_eq!(c.len(), 1, "{}: one clear", r.name);
        let s = &c[0];
        assert!(s.needs_quad && s.clear_quad, "{}: a scissored clear is a quad", r.name);
        assert_eq!(s.refused, None, "{}", r.name);
        // the mask here is the full RGBA: only narrowed values keep the store clean
        assert!(
            words(s.targets[0].3, s.clear_bits) == words(s.targets[0].3, narrowed_bits(&r, s.clear_bits)),
            "{}: the scissored quad carries B/A into the hidden channels: {s:?}",
            r.name
        );
        let _ = f.w;
    }
}

/// An R8I texture at attachment 1 beside an RGBA8 renderbuffer at attachment 0
/// under glClear: RT 0's clear value is the application's float colour,
/// unchanged by the integer RT; RT 1 is narrowed (the clear words of its wide
/// store hold 0, 0, 1 in G, B, A) or the clear is refused with a named cause:
/// by the back end (`state::refusal` / `state::unsupported`), or by the GL
/// core before it (one clear value cannot be right for both targets: no
/// clear reaches the back end and GL_INVALID_OPERATION is raised, with the
/// "clear refused" log line). (Which of a float colour's bits an integer RT
/// takes is undefined by the spec; the hidden channels are not.)
#[test]
fn r8i_texture_beside_a_float_target_under_glclear_keeps_rt0_and_narrows_or_refuses_rt1() {
    let _g = context();
    let r = ROWS[0];
    let _f = fbo_float_then(&r, Kind::Texture2d);
    assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE);
    // SAFETY: plain GL calls.
    unsafe {
        glClearColor(0.25, 0.5, 0.75, 1.0);
        glClear(GL_COLOR_BUFFER_BIT);
    }
    let _ = err();
    let c = clears();
    if c.is_empty() {
        assert_eq!(err(), GL_INVALID_OPERATION, "no clear reached the back end, and none was refused");
        return;
    }
    for s in &c {
        match s.refused {
            Some(why) => assert!(!why.is_empty(), "a refusal without a cause"),
            None => {
                assert_eq!(s.targets.len(), 2);
                let want = [0.25f32, 0.5, 0.75, 1.0].map(f32::to_bits);
                assert_eq!(
                    words(s.targets[0].3, s.clear_bits),
                    words(s.targets[0].3, want),
                    "RT 0's clear value is unchanged"
                );
                assert!(
                    hidden_channels_clean(&r, s, s.targets[1].3),
                    "RT 1 carries B/A into the hidden channels: {s:?}"
                );
            }
        }
    }
}

/// The same, clearing draw buffer 1 alone with glClearBufferiv: RT 0 is not in
/// the clear, RT 1's clear words are narrowed.
#[test]
fn r8i_texture_at_attachment_1_clearbuffer_leaves_rt0_out_and_narrows_rt1() {
    let _g = context();
    let r = ROWS[0];
    let _f = fbo_float_then(&r, Kind::Texture2d);
    assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE);
    SEEN.lock().unwrap().clear();
    clear_buffer(&r, 1);
    assert_eq!(err(), GL_NO_ERROR);
    let c = clears();
    assert_eq!(c.len(), 1, "one clear");
    let s = &c[0];
    assert_eq!(s.refused, None);
    assert_eq!(s.clear_target_mask, 0b10, "RT 0 is not cleared");
    assert!(hidden_channels_clean(&r, s, s.targets[1].3), "RT 1 carries B/A into the hidden channels: {s:?}");
}
