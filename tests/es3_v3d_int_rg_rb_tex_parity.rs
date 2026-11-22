// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! V3D (Raspberry Pi 4): an R or RG integer renderbuffer and a texture of the
//! same format (R8I .. RG32UI) are given the same draw and the same clear,
//! and the job state the stand-in records is compared (plan #383, gap G2 of
//! docs/notes/v3d-format-matrix.md).
//!
//! The renderbuffer is drawn on the format's own TLB surface; the texture as
//! the wide four-channel store of the same channel type and width
//! (`WIDE_INT_RT`, src/gl/framebuffer.rs) with the format's channels as the
//! write mask. Per format, for the draw and for the clear, this compares:
//! the fragment shader (compiled code and output count), the render
//! target's internal type, clamp and bpp class (`state::rt_row`), the
//! colour mask (`GlesDraw::color_mask`, the job's `colormask`, the per-RT
//! masks), and the clear words (`state::clear_words`, `GlesDraw::clear_rgba`).
//!
//! Everything is equal except the differences below, each asserted by name:
//!
//! 1. the render-target surface: the renderbuffer's own `GLES_SURF_*`, the
//!    texture's four-channel surface of the same type and width;
//! 2. the internal bpp class, where the wide pixel is larger than the
//!    format's own: 16-bit channels 32 -> 64, 32-bit 32/64 -> 128 (the 8-bit
//!    formats stay 32). The internal type and the clamp are the same;
//! 3. the draw's colour mask: the renderbuffer all four channels (the
//!    application's mask), the texture the format's channels only (the
//!    masked-off channels); the job's `colormask` is 0x7 for the
//!    renderbuffer (no alpha written) and the format's channels for the
//!    texture, and no per-RT masks in either (one target);
//! 4. the clear's values for the channels the format lacks: the renderbuffer
//!    takes the application's raw values, the texture the 0, 0, 1 of the
//!    store, so the clear words differ there only. The clear colour mask is
//!    the same (all four) for both.
//!
//! What it cannot see: the GPU. The back end here is a stand-in that records
//! each draw and clear; nothing is rasterized and nothing is verified on
//! hardware.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_v3d_int_rg_rb_tex_parity
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
use yakogl::v3d::ffi::*;
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
#[derive(Clone, Debug, PartialEq)]
struct Seen {
    /// An application draw (vertices); false: a clear alone.
    draws: bool,
    do_clear: bool,
    /// `targets[0].format`.
    surf: u8,
    num_targets: usize,
    /// The fragment shader's code and output count (None: no shader).
    shader: Option<(Vec<u32>, u8)>,
    color_mask: u8,
    clear_color_mask: u8,
    per_target_mask: bool,
    clear_rgba: [u32; 4],
    /// `state::clear_words` of the target's surface and the draw's clear value.
    clear_words: [u32; 4],
    /// The job state's (colormask, rt_colormask_own, rt_colormask) of a draw.
    job: Option<(u32, bool, u32)>,
    /// `state::refusal` / `state::unsupported` or the job builder's reason.
    refused: Option<&'static str>,
}

static SEEN: Mutex<Vec<Seen>> = Mutex::new(Vec::new());

/// Records each call; a clear of an integer target stores the low bits of
/// the four raw values in the target's layout (the stand-in's own layout:
/// the GL core reads back what it was told).
unsafe extern "C" fn b_draw(d: *const GlesDraw) -> i32 {
    let d = &*d;
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
    let t = &d.targets[0];
    let shader = (!d.fs.is_null() && !(*d.fs).usc.is_null())
        .then(|| (core::slice::from_raw_parts((*d.fs).usc, (*d.fs).usc_dw as usize).to_vec(), (*d.fs).num_outputs));
    SEEN.lock().unwrap().push(Seen {
        draws: d.index_count != 0,
        do_clear: d.do_clear,
        surf: t.format,
        num_targets: (d.num_targets.max(1) as usize).min(GLES_MAX_TARGETS),
        shader,
        color_mask: d.color_mask,
        clear_color_mask: d.clear_color_mask,
        per_target_mask: d.per_target_mask,
        clear_rgba: d.clear_rgba.map(f32::to_bits),
        clear_words: state::clear_words(t.format, d.clear_rgba),
        job,
        refused,
    });
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
    name: c"es3-v3d-int-rg-rb-tex-parity".as_ptr() as *const c_char,
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
    /// The channels (bit 0 R .. bit 3 A) of the format.
    fn own_mask(&self) -> u8 {
        (1u8 << self.channels) - 1
    }
}

/// Which of the two carries the format at attachment 0.
#[derive(Clone, Copy, Debug)]
enum Kind {
    Renderbuffer,
    Texture,
}

/// A bound framebuffer with the format at attachment 0; dropped, it unbinds
/// and deletes it.
struct Fbo {
    fb: GLuint,
    obj: GLuint,
    kind: Kind,
}

impl Drop for Fbo {
    fn drop(&mut self) {
        // SAFETY: names this test made.
        unsafe {
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
            glDeleteFramebuffers(1, &self.fb);
            match self.kind {
                Kind::Renderbuffer => glDeleteRenderbuffers(1, &self.obj),
                Kind::Texture => glDeleteTextures(1, &self.obj),
            }
            glUseProgram(0);
        }
    }
}

fn fbo(r: &Row, kind: Kind) -> Fbo {
    let (mut fb, mut obj) = (0, 0);
    // SAFETY: room for one name each.
    unsafe {
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        match kind {
            Kind::Renderbuffer => {
                glGenRenderbuffers(1, &mut obj);
                glBindRenderbuffer(GL_RENDERBUFFER, obj);
                glRenderbufferStorage(GL_RENDERBUFFER, r.ifmt, 8, 8);
                glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_RENDERBUFFER, obj);
            }
            Kind::Texture => {
                glGenTextures(1, &mut obj);
                glBindTexture(GL_TEXTURE_2D, obj);
                glTexStorage2D(GL_TEXTURE_2D, 1, r.ifmt, 8, 8);
                glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, obj, 0);
            }
        }
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE, "{} {kind:?}", r.name);
    }
    assert_eq!(err(), GL_NO_ERROR, "{} {kind:?}: storage and attach", r.name);
    Fbo { fb, obj, kind }
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

/// Make the program writing a known value of the format's type current.
fn program_one(r: &Row) {
    let val = if r.signed { "ivec4(1, -2, 3, -4)" } else { "uvec4(1u, 2u, 3u, 4u)" };
    let fs = format!(
        "#version 300 es\nprecision highp int;\nlayout(location = 0) out highp {} o0;\n\
         void main() {{ o0 = {val}; }}\n",
        r.ty()
    );
    let v = shader(GL_VERTEX_SHADER, "#version 300 es\nin vec4 p;\nvoid main() { gl_Position = p; }\n");
    let f = shader(GL_FRAGMENT_SHADER, &fs);
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

/// Draw one triangle; the draw the back end was handed, and the GL error.
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

/// glClearBuffer{i,ui}v of attachment 0 with 5, 6, 7, 8; the clear the back
/// end was handed.
fn clear(r: &Row) -> Seen {
    let v: [GLint; 4] = [5, 6, 7, 8];
    SEEN.lock().unwrap().clear();
    // SAFETY: four values.
    unsafe {
        if r.signed {
            glClearBufferiv(GL_COLOR, 0, v.as_ptr());
        } else {
            glClearBufferuiv(GL_COLOR, 0, v.as_ptr() as *const GLuint);
        }
        glFinish();
    }
    assert_eq!(err(), GL_NO_ERROR, "{}: clear", r.name);
    let s = core::mem::take(&mut *SEEN.lock().unwrap());
    s.into_iter().rev().find(|s| s.do_clear).unwrap_or_else(|| panic!("{}: the back end saw no clear", r.name))
}

/* ---- the comparison ------------------------------------------------------ */

/// What the draw and the clear of one of the two carry.
fn capture(r: &Row, kind: Kind) -> (Seen, Seen) {
    let _f = fbo(r, kind);
    program_one(r);
    let (d, e) = draw();
    assert_eq!(e, GL_NO_ERROR, "{} {kind:?}: the draw", r.name);
    let d = d.unwrap_or_else(|| panic!("{} {kind:?}: the back end saw no draw", r.name));
    assert_eq!(d.refused, None, "{} {kind:?}", r.name);
    (d, clear(r))
}

/// Channel `i` of a clear's words in a tile buffer of `bits`-bit channels.
fn channel(words: [u32; 4], bits: u32, i: u32) -> u32 {
    match bits {
        8 => (words[0] >> (8 * i)) & 0xff,
        16 => (words[(i / 2) as usize] >> (16 * (i % 2))) & 0xffff,
        _ => words[i as usize],
    }
}

/// The EXPECTED difference 2: (renderbuffer's, texture's) internal bpp class.
fn bpp_pair(bits: u32, channels: u8) -> (u32, u32) {
    match (bits, channels) {
        (8, _) => (V3D_INTERNAL_BPP_32, V3D_INTERNAL_BPP_32),
        (16, _) => (V3D_INTERNAL_BPP_32, V3D_INTERNAL_BPP_64),
        (32, 1) => (V3D_INTERNAL_BPP_32, V3D_INTERNAL_BPP_128),
        _ => (V3D_INTERNAL_BPP_64, V3D_INTERNAL_BPP_128),
    }
}

#[test]
fn an_r_or_rg_integer_renderbuffer_and_texture_draw_the_same_but_for_the_wide_store() {
    let _g = context();
    for r in ROWS {
        let (rb_draw, rb_clear) = capture(&r, Kind::Renderbuffer);
        let (tx_draw, tx_clear) = capture(&r, Kind::Texture);
        let want = backend::gles_surf_int_layout(r.surf).unwrap();
        let bits = u32::from(want.bits);
        let n = r.name;

        /* ---- equal ---- */
        for (what, rb, tx) in [("draw", &rb_draw, &tx_draw), ("clear", &rb_clear, &tx_clear)] {
            assert_eq!(rb.num_targets, 1, "{n} {what}");
            assert_eq!(tx.num_targets, 1, "{n} {what}");
            assert!(!rb.per_target_mask && !tx.per_target_mask, "{n} {what}: one target, one mask");
        }
        assert!(rb_draw.shader.is_some(), "{n}: the shader reached the back end");
        assert_eq!(rb_draw.shader, tx_draw.shader, "{n}: the same compiled fragment shader and outputs");
        assert_eq!(rb_clear.clear_color_mask, tx_clear.clear_color_mask, "{n}: the clear writes the same channels");
        assert_eq!(rb_clear.color_mask, tx_clear.color_mask, "{n}: the clear's colour mask");
        assert_eq!(rb_clear.draws, tx_clear.draws, "{n}: both clears are clears alone");

        /* ---- EXPECTED 1: the render-target surface ---- */
        assert_eq!(rb_draw.surf, r.surf, "{n}: the renderbuffer's own surface");
        assert_eq!(rb_clear.surf, r.surf, "{n}: the renderbuffer's own surface (clear)");
        assert_ne!(tx_draw.surf, r.surf, "{n}: the texture's surface is the wide one");
        assert_eq!(tx_clear.surf, tx_draw.surf, "{n}: the clear is on the draw's surface");
        let wide = backend::gles_surf_int_layout(tx_draw.surf).unwrap();
        assert_eq!((wide.bits, wide.signed, wide.channels), (want.bits, want.signed, 4), "{n}: the store");

        /* ---- the internal type and clamp are equal; EXPECTED 2: the bpp class ---- */
        let (rt_rb, rt_tx) = (state::rt_row(rb_draw.surf), state::rt_row(tx_draw.surf));
        assert_eq!(rt_rb.internal_type, rt_tx.internal_type, "{n}: internal type");
        assert_eq!(rt_rb.clamp, rt_tx.clamp, "{n}: clamp");
        assert_eq!(rt_rb.clamp, V3D_RENDER_TARGET_CLAMP_INT, "{n}: clamp INT");
        assert_ne!(rt_rb.output, rt_tx.output, "{n}: the store's image format differs (the surface)");
        assert_eq!(
            (rt_rb.internal_bpp, rt_tx.internal_bpp),
            bpp_pair(bits, r.channels),
            "{n}: internal bpp class (renderbuffer, texture)"
        );

        /* ---- EXPECTED 3: the draw's colour mask ---- */
        assert_eq!(rb_draw.color_mask, 0xf, "{n}: the renderbuffer takes the application's mask");
        assert_eq!(tx_draw.color_mask, r.own_mask(), "{n}: the texture the format's channels");
        let job_rb = rb_draw.job.unwrap_or_else(|| panic!("{n}: the V3D refused the renderbuffer draw"));
        let job_tx = tx_draw.job.unwrap_or_else(|| panic!("{n}: the V3D refused the texture draw"));
        /* the job state drops alpha of the R / RG renderbuffer's format
         * (0xf -> 0x7: no alpha to write, one that reads 1); the texture's mask
         * is the format's channels, no alpha either */
        assert_eq!(job_rb, (0x7, false, 0), "{n}: renderbuffer job mask");
        assert_eq!(job_tx, (u32::from(r.own_mask()), false, 0), "{n}: texture job mask");
        assert_eq!(job_rb.0 & u32::from(r.own_mask()), job_tx.0, "{n}: the format's channels are written alike");

        /* ---- EXPECTED 4: the clear's absent channels ---- */
        let (raw_rb, raw_tx) = (rb_clear.clear_rgba, tx_clear.clear_rgba);
        assert_eq!(raw_rb, [5, 6, 7, 8], "{n}: the renderbuffer is cleared with the application's values");
        for i in 0..4usize {
            if i < r.channels as usize {
                assert_eq!(raw_tx[i], raw_rb[i], "{n}: channel {i} of the clear");
            } else {
                assert_eq!(raw_tx[i], u32::from(i == 3), "{n}: absent channel {i}: 0, 0, 1 for the store");
            }
        }
        /* the words as the tile buffer takes them: the format's channels alike,
         * the others the raw values (renderbuffer) or 0, 0, 1 (texture) */
        assert_eq!(rb_clear.clear_words, state::clear_words(r.surf, raw_rb.map(f32::from_bits)), "{n}");
        assert_eq!(tx_clear.clear_words, state::clear_words(tx_clear.surf, raw_tx.map(f32::from_bits)), "{n}");
        let mask = if bits == 32 { u32::MAX } else { (1 << bits) - 1 };
        for i in 0..4u32 {
            let (a, b) = (channel(rb_clear.clear_words, bits, i), channel(tx_clear.clear_words, bits, i));
            if i < u32::from(r.channels) {
                assert_eq!(a, b, "{n}: channel {i} of the clear words");
            } else {
                assert_eq!(a, [5, 6, 7, 8][i as usize] & mask, "{n}: renderbuffer word {i}");
                assert_eq!(b, u32::from(i == 3), "{n}: texture word {i}");
            }
        }
    }
}
