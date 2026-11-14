// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The Pi 4-only sRGB blit rows of dEQP-GLES3 on the V3D host back end
//! (t-srgb-blit-v3d): `fbo.blit.conversion.*srgb8_alpha8*` and
//! `fbo.blit.default_framebuffer.srgb8_alpha8_*`, 29 rows that pass on the
//! AM62P and fail on the Pi 4 (docs/notes/v3d-fbo-blit.md 3.3).
//!
//! The blit codec is the GL core's and shared (`src/gl/blit.rs`): an sRGB
//! read buffer decodes to linear, an sRGB draw buffer encodes, alpha stays a
//! linear byte, sRGB to sRGB copies the bytes. What the V3D adds is the
//! render target: the TLB row of `GLES_SURF_SRGB8_A8` (`state::rt_format`),
//! the job state of a draw into it, and the clear. The GL core is driven
//! through the API into a stand-in back end which, at each draw, runs the
//! V3D's own `state::refusal`, `state::unsupported` and `state::job_state`
//! on what it is handed and then stores the pixels the way the TLB would for
//! the row `state::rt_format` names: RGBA8 as linear bytes, SRGB8_ALPHA8
//! with red, green and blue encoded. A draw is a fixed gradient standing in
//! for the fragment shader (no QPU runs here); the TLB conversion itself is
//! not run, so the "right encode" these tests hold is the row's contract.
//!
//! What this file settles for the 29 rows (the tolerance of a conversion
//! blit is one 8-bit step in linear space; decode and encode are the byte
//! tables of `gl::srgb`, the nearest byte on the curve):
//!
//! - everything passes: the V3D takes a draw and a clear into an sRGB
//!   renderbuffer and texture (no refusal, TLB row SRGB8_ALPHA8 on a 16F tile
//!   buffer); an sRGB image blitted to RGBA8 reads back decoded, RGBA8 back
//!   into sRGB re-encoded, sRGB to sRGB keeps the bytes, a LINEAR scale
//!   filters after the decode; an sRGB texture cleared to yellow blits into
//!   the default framebuffer, and the default framebuffer into an sRGB
//!   renderbuffer encoded.
//! - so there is no ignored test and no refusal, wrong encode/decode step or
//!   blit-codec fault to name from the host: nothing the host can run
//!   explains the Pi 4 rows. The cause stays one the note's 3.3 lists that
//!   only a board shows (the TLB conversion at store, the 16F tile buffer,
//!   the sRGB decode of the texture sources), or an image older than main;
//!   that is plan #320's FB2 rerun.
//! - the conversion rows whose other side is not an RGBA8-byte format (R8,
//!   RG8, RGB565, RGBA4, RGB5_A1, RGB10_A2, the float formats) need that
//!   side's own render, which this stand-in does not store:
//!   `other_side_formats_are_complete_colour_buffers` holds only that each is
//!   a complete colour renderbuffer; the blit values of those pairs stay with
//!   the shared `src/gl/blit.rs` tests. R32F is the one of them whose
//!   completeness follows the build rather than one answer: ES 3.0 makes it
//!   colour-renderable only under GL_EXT_color_buffer_float, which no back
//!   end claims (docs/notes/es3-float-targets.md), while today's core
//!   completes it anyway. That test probes a 1x1 R32F colour renderbuffer
//!   and holds the row as today while it is complete; once the core follows
//!   the extension the row asserts the refusal instead (no extension listed,
//!   storage GL_INVALID_ENUM) and skips (plan #360, r32f-gate-b).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_srgb_blit_v3d
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

/// What the V3D made of one draw or clear.
#[derive(Clone, Debug)]
struct Seen {
    /// An application draw (vertices), not a clear alone.
    draws: bool,
    /// `targets[..n]`: (gpu, format).
    targets: Vec<(u64, u8)>,
    /// `state::refusal` / `state::unsupported`.
    refused: Option<String>,
    /// `state::job_state`, when the V3D takes the draw.
    job: Option<V3dJobState>,
}

static SEEN: Mutex<Vec<Seen>> = Mutex::new(Vec::new());

/// The gradient a draw stands for: linear red rising and green falling with
/// x, blue 0.25, opaque, over a row of `w`.
fn gradient(x: usize, w: usize) -> [f32; 4] {
    let t = x as f32 / (w - 1) as f32;
    [t, 1.0 - t, 0.25, 1.0]
}

fn unorm(v: f32) -> u8 {
    (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8
}

/// The bytes the TLB stores for linear `c` into the row `surf` names; `None`
/// for a surface this stand-in does not store.
fn tlb_store(surf: u8, c: [f32; 4]) -> Option<[u8; 4]> {
    let row = state::rt_format(surf)?;
    match row.output {
        V3D_OUTPUT_IMAGE_FORMAT_SRGB8_ALPHA8 => Some([
            yakogl::gl::srgb::encode(c[0]),
            yakogl::gl::srgb::encode(c[1]),
            yakogl::gl::srgb::encode(c[2]),
            unorm(c[3]),
        ]),
        V3D_OUTPUT_IMAGE_FORMAT_RGBA8 if surf == GLES_SURF_RGBA8888 || surf == GLES_SURF_XRGB8888 => {
            Some([unorm(c[0]), unorm(c[1]), unorm(c[2]), unorm(c[3])])
        }
        _ => None,
    }
}

unsafe extern "C" fn b_draw(d: *const GlesDraw) -> i32 {
    let d = &*d;
    let n = d.num_targets as usize;
    /* the default framebuffer comes as the frame's own target */
    let mut ts: Vec<(u64, u32, u32, u8)> =
        d.targets[..n.min(GLES_MAX_TARGETS)].iter().map(|t| (t.gpu, t.w, t.h, t.format)).collect();
    if ts.is_empty() {
        ts.push((d.target_gpu, d.target_w, d.target_h, d.target_format));
    }
    let refused = state::refusal(d).or_else(|| state::unsupported(d)).map(|r| format!("{r:?}"));
    let draws = d.index_count != 0;
    let job = if draws { state::job_state(d).ok() } else { None };
    SEEN.lock().unwrap().push(Seen {
        draws,
        targets: ts.iter().map(|t| (t.0, t.3)).collect(),
        refused: refused.clone(),
        job,
    });
    if refused.is_some() {
        return 0;
    }
    for (i, &(gpu, w, h, fmt)) in ts.iter().enumerate() {
        if gpu == 0 {
            continue;
        }
        let clear = d.do_clear && (d.clear_target_mask == 0 || d.clear_target_mask & (1 << i) != 0);
        if !clear && !draws {
            continue;
        }
        let pitch = ((w + 1) & !1) as usize;
        for y in 0..h as usize {
            for x in 0..w as usize {
                let c = if draws { gradient(x, w as usize) } else { d.clear_rgba };
                if let Some(px) = tlb_store(fmt, c) {
                    core::ptr::copy_nonoverlapping(px.as_ptr(), (gpu as *mut u8).add((y * pitch + x) * 4), 4);
                }
            }
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
    name: c"es3-srgb-blit".as_ptr() as *const c_char,
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
};

/// One context for the whole file; the tests take turns on it.
static LOCK: Mutex<()> = Mutex::new(());
static ONCE: Once = Once::new();

const W: GLsizei = 8;

fn context() -> MutexGuard<'static, ()> {
    let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    ONCE.call_once(|| {
        // SAFETY: both tables are statics; the lock above serialises the tests.
        unsafe {
            osal_set_ops(&OSAL);
            backend::g_backend = &BACKEND;
            yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
            assert!(!yakogl::gl::context::gl_context_create(W as u32, W as u32).is_null());
        }
    });
    // SAFETY: plain GL calls.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glDisable(GL_BLEND);
        glDisable(GL_SCISSOR_TEST);
        glFinish();
    }
    backend::HOST_SRGB_RT.store(false, Ordering::Relaxed);
    SEEN.lock().unwrap().clear();
    while err() != GL_NO_ERROR {}
    g
}

fn err() -> GLenum {
    // SAFETY: plain GL call.
    unsafe { glGetError() }
}

/* ---- objects ------------------------------------------------------------- */

/// A renderbuffer in a framebuffer of its own.
struct Target {
    fb: GLuint,
    rb: GLuint,
}

impl Drop for Target {
    fn drop(&mut self) {
        // SAFETY: names this test made.
        unsafe {
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
            glDeleteFramebuffers(1, &self.fb);
            glDeleteRenderbuffers(1, &self.rb);
        }
    }
}

fn renderbuffer_target(format: GLenum) -> Target {
    let (mut fb, mut rb) = (0, 0);
    // SAFETY: room for one name each.
    unsafe {
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        glGenRenderbuffers(1, &mut rb);
        glBindRenderbuffer(GL_RENDERBUFFER, rb);
        glRenderbufferStorage(GL_RENDERBUFFER, format, W, W);
        assert_eq!(err(), GL_NO_ERROR, "storage {format:#x}");
        glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_RENDERBUFFER, rb);
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE, "{format:#x}");
    }
    Target { fb, rb }
}

/// A `W` x `W` texture as colour attachment 0 of a framebuffer of its own.
struct TexTarget {
    fb: GLuint,
    tex: GLuint,
}

impl Drop for TexTarget {
    fn drop(&mut self) {
        // SAFETY: names this test made.
        unsafe {
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
            glDeleteFramebuffers(1, &self.fb);
            glDeleteTextures(1, &self.tex);
        }
    }
}

fn texture_target(internal: GLenum) -> TexTarget {
    let (mut fb, mut tex) = (0, 0);
    // SAFETY: room for one name each; null pixels only allocate.
    unsafe {
        glGenTextures(1, &mut tex);
        glBindTexture(GL_TEXTURE_2D, tex);
        glTexImage2D(GL_TEXTURE_2D, 0, internal as GLint, W, W, 0, GL_RGBA, GL_UNSIGNED_BYTE, core::ptr::null());
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex, 0);
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE, "{internal:#x}");
    }
    TexTarget { fb, tex }
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

/// Draw one full-target triangle with a plain colour program; the stand-in
/// stores the gradient in its place. Returns what the V3D made of the draw.
fn draw() -> Seen {
    static TRI: [f32; 12] = [-1.0, -1.0, 0.0, 1.0, 3.0, -1.0, 0.0, 1.0, -1.0, 3.0, 0.0, 1.0];
    let v = shader(GL_VERTEX_SHADER, "#version 300 es\nin vec4 p;\nvoid main() { gl_Position = p; }\n");
    let f = shader(
        GL_FRAGMENT_SHADER,
        "#version 300 es\nprecision highp float;\nlayout(location = 0) out vec4 o;\nvoid main() { o = vec4(0.5, 0.5, 0.25, 1.0); }\n",
    );
    SEEN.lock().unwrap().clear();
    // SAFETY: plain GL calls; TRI is static and outlives the draw.
    unsafe {
        let p = glCreateProgram();
        glAttachShader(p, v);
        glAttachShader(p, f);
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        assert_eq!(ok, 1);
        glUseProgram(p);
        glBindBuffer(GL_ARRAY_BUFFER, 0);
        glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, TRI.as_ptr() as *const c_void);
        glEnableVertexAttribArray(0);
        glDrawArrays(GL_TRIANGLES, 0, 3);
        glFinish();
        glUseProgram(0);
    }
    assert_eq!(err(), GL_NO_ERROR);
    let s = core::mem::take(&mut *SEEN.lock().unwrap());
    s.into_iter().rev().find(|s| s.draws).expect("the draw reached the back end")
}

fn clear(rgba: [f32; 4]) -> Seen {
    SEEN.lock().unwrap().clear();
    // SAFETY: plain GL calls.
    unsafe {
        glClearColor(rgba[0], rgba[1], rgba[2], rgba[3]);
        glClear(GL_COLOR_BUFFER_BIT);
        glFinish();
    }
    assert_eq!(err(), GL_NO_ERROR);
    let s = core::mem::take(&mut *SEEN.lock().unwrap());
    s.into_iter().next_back().expect("the clear reached the back end")
}

/// The bytes glReadPixels returns for the bound read framebuffer, rows
/// bottom-up as ES hands them.
fn read() -> Vec<[u8; 4]> {
    let mut px = vec![0x77u8; (W * W * 4) as usize];
    // SAFETY: room for W x W RGBA pixels.
    unsafe { glReadPixels(0, 0, W, W, GL_RGBA, GL_UNSIGNED_BYTE, px.as_mut_ptr() as *mut c_void) };
    assert_eq!(err(), GL_NO_ERROR);
    px.chunks(4).map(|c| [c[0], c[1], c[2], c[3]]).collect()
}

/// Blit the whole `W` x `W` read framebuffer to the draw framebuffer.
fn blit_all(filter: GLenum) {
    blit([0, 0, W, W], [0, 0, W, W], filter);
}

fn blit(s: [GLint; 4], d: [GLint; 4], filter: GLenum) {
    // SAFETY: plain GL call.
    unsafe {
        glBlitFramebuffer(s[0], s[1], s[2], s[3], d[0], d[1], d[2], d[3], GL_COLOR_BUFFER_BIT, filter);
        glFinish();
    }
}

fn bind(read_fb: GLuint, draw_fb: GLuint) {
    // SAFETY: plain GL calls.
    unsafe {
        glBindFramebuffer(GL_READ_FRAMEBUFFER, read_fb);
        glBindFramebuffer(GL_DRAW_FRAMEBUFFER, draw_fb);
    }
}

/* ---- ES 3.0's conversion, from the formulas ----------------------------- */

/// The tolerance of a conversion blit: one 8-bit step, in linear space.
const STEP: f64 = 1.0 / 255.0;

fn dec(i: u8) -> f64 {
    let c = i as f64 / 255.0;
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

fn enc(l: f64) -> u8 {
    let l = l.clamp(0.0, 1.0);
    let s = if l <= 0.0031308 { 12.92 * l } else { 1.055 * l.powf(1.0 / 2.4) - 0.055 };
    (s * 255.0 + 0.5).floor() as u8
}

/// The stored bytes of the gradient at column `x`, as the sRGB row writes
/// them.
fn stored_gradient(x: usize) -> [u8; 4] {
    let c = gradient(x, W as usize);
    [enc(c[0] as f64), enc(c[1] as f64), enc(c[2] as f64), unorm(c[3])]
}

fn near(got: u8, want: f64) -> bool {
    (got as f64 / 255.0 - want).abs() <= STEP
}

/// The pixels of column `x` of a bottom-up read; the gradient varies along x
/// only so the orientation does not matter.
fn column(px: &[[u8; 4]], x: usize) -> Vec<[u8; 4]> {
    (0..W as usize).map(|y| px[y * W as usize + x]).collect()
}

/* ---- the V3D takes the target ------------------------------------------- */

/// The TLB row of the sRGB surface: output SRGB8_ALPHA8, a 16F linear tile
/// buffer at 64 bpp, clamped to [0, 1] (v3dx_format_table.c:250-259).
#[test]
fn the_srgb_surface_has_mesas_tlb_row() {
    let row = state::rt_format(GLES_SURF_SRGB8_A8).expect("a row");
    assert_eq!(row.output, V3D_OUTPUT_IMAGE_FORMAT_SRGB8_ALPHA8);
    assert_eq!(row.internal_type, V3D_INTERNAL_TYPE_16F);
    assert_eq!(row.internal_bpp, V3D_INTERNAL_BPP_64);
    assert_eq!(row.clamp, V3D_RENDER_TARGET_CLAMP_NORM);
    assert_eq!(row.bytes, 4);
    assert!(backend::rt_format_drawable(GLES_SURF_SRGB8_A8));
}

/// A draw into an SRGB8_ALPHA8 renderbuffer reaches the back end as
/// `GLES_SURF_SRGB8_A8` and the V3D takes it: no refusal, a job state. The
/// stored gradient is encoded, the read-back returns the stored bytes.
#[test]
fn a_gradient_drawn_into_srgb_is_taken_and_reads_back_encoded() {
    let _g = context();
    let t = renderbuffer_target(GL_SRGB8_ALPHA8);
    let s = draw();
    assert_eq!(s.refused, None);
    assert_eq!(s.targets[0].1, GLES_SURF_SRGB8_A8);
    assert!(s.job.is_some(), "job_state accepts the draw");
    let px = read();
    for x in 0..W as usize {
        for p in column(&px, x) {
            assert_eq!(p, stored_gradient(x), "column {x}");
        }
    }
    drop(t);
}

/// A clear of an sRGB renderbuffer is taken (no refusal) and stores the
/// clear colour encoded, alpha linear.
#[test]
fn a_clear_of_srgb_is_taken_and_reads_back_encoded() {
    let _g = context();
    let t = renderbuffer_target(GL_SRGB8_ALPHA8);
    let s = clear([0.5, 0.25, 0.0, 0.5]);
    assert_eq!(s.refused, None);
    assert_eq!(s.targets[0].1, GLES_SURF_SRGB8_A8);
    for p in read() {
        assert_eq!(p, [188, 137, 0, 128]);
    }
    drop(t);
}

/* ---- fbo.blit.conversion.srgb8_alpha8_to_rgba8 and back ------------------ */

/// Draw a gradient into SRGB8_ALPHA8, blit it to RGBA8 (decode), blit that
/// back into a second SRGB8_ALPHA8 (encode): the middle image is the linear
/// gradient within one 8-bit step, the last is the nearest sRGB byte of the
/// middle one and within one step of the first in linear space.
#[test]
fn srgb_to_rgba8_and_back_follows_the_conversion() {
    let _g = context();
    let src = renderbuffer_target(GL_SRGB8_ALPHA8);
    draw();
    let mid = renderbuffer_target(GL_RGBA8);
    let back = renderbuffer_target(GL_SRGB8_ALPHA8);

    bind(src.fb, mid.fb);
    blit_all(GL_NEAREST);
    assert_eq!(err(), GL_NO_ERROR);
    bind(mid.fb, mid.fb);
    let m = read();
    for x in 0..W as usize {
        let s = stored_gradient(x);
        for p in column(&m, x) {
            for ch in 0..3 {
                assert!(near(p[ch], dec(s[ch])), "decoded column {x} channel {ch}: {} for {}", p[ch], s[ch]);
            }
            assert_eq!(p[3], s[3], "alpha is a linear byte");
        }
    }

    bind(mid.fb, back.fb);
    blit_all(GL_NEAREST);
    assert_eq!(err(), GL_NO_ERROR);
    bind(back.fb, back.fb);
    let b = read();
    for x in 0..W as usize {
        let s = stored_gradient(x);
        for (p, q) in column(&b, x).into_iter().zip(column(&m, x)) {
            for ch in 0..3 {
                assert_eq!(p[ch], enc(q[ch] as f64 / 255.0), "encoded column {x} channel {ch}");
                assert!((dec(p[ch]) - dec(s[ch])).abs() <= STEP, "round trip column {x} channel {ch}");
            }
            assert_eq!(p[3], s[3]);
        }
    }
}

/// sRGB to sRGB keeps every byte (srgb8_alpha8_to_srgb8_alpha8).
#[test]
fn srgb_to_srgb_keeps_the_bytes() {
    let _g = context();
    let src = renderbuffer_target(GL_SRGB8_ALPHA8);
    draw();
    let dst = renderbuffer_target(GL_SRGB8_ALPHA8);
    bind(src.fb, dst.fb);
    blit_all(GL_NEAREST);
    assert_eq!(err(), GL_NO_ERROR);
    bind(dst.fb, dst.fb);
    let px = read();
    for x in 0..W as usize {
        for p in column(&px, x) {
            assert_eq!(p, stored_gradient(x), "column {x}");
        }
    }
}

/// An sRGB texture level as the source, cleared by the V3D: the same decode.
#[test]
fn an_srgb_texture_blits_to_rgba8_decoded() {
    let _g = context();
    let src = texture_target(GL_SRGB8_ALPHA8);
    let s = clear([0.5, 0.25, 0.0, 0.5]);
    assert_eq!(s.refused, None);
    assert_eq!(s.targets[0].1, GLES_SURF_SRGB8_A8);
    let dst = renderbuffer_target(GL_RGBA8);
    bind(src.fb, dst.fb);
    blit_all(GL_NEAREST);
    assert_eq!(err(), GL_NO_ERROR);
    bind(dst.fb, dst.fb);
    for p in read() {
        assert!(near(p[0], 0.5) && near(p[1], 0.25) && p[2] == 0, "{p:?}");
        assert_eq!(p[3], 128);
    }
}

/// A LINEAR 2:1 reduction of the sRGB gradient filters after the decode.
#[test]
fn a_linear_reduction_filters_in_linear_space() {
    let _g = context();
    let src = renderbuffer_target(GL_SRGB8_ALPHA8);
    draw();
    let dst = renderbuffer_target(GL_RGBA8);
    bind(src.fb, dst.fb);
    blit([0, 0, W, W], [0, 0, W / 2, W / 2], GL_LINEAR);
    assert_eq!(err(), GL_NO_ERROR);
    bind(dst.fb, dst.fb);
    let mut got = vec![0x77u8; 4 * 4 * 4];
    // SAFETY: room for 4 x 4 RGBA pixels.
    unsafe { glReadPixels(0, 0, W / 2, W / 2, GL_RGBA, GL_UNSIGNED_BYTE, got.as_mut_ptr() as *mut c_void) };
    for x in 0..(W / 2) as usize {
        /* the pair of source columns the output column is centred on */
        let lin = (dec(stored_gradient(2 * x)[0]) + dec(stored_gradient(2 * x + 1)[0])) / 2.0;
        let g = got[x * 4];
        assert!((g as f64 / 255.0 - lin).abs() <= 2.0 * STEP, "column {x}: {g} for {lin}");
    }
}

/* ---- fbo.blit.default_framebuffer.srgb8_alpha8_* ------------------------- */

/// An sRGB texture cleared to yellow blitted into the default framebuffer
/// (linear: decoded) and read back; the default framebuffer blitted into an
/// sRGB renderbuffer comes out encoded. Yellow is 0 or 1 in every channel, so
/// that direction is exact.
#[test]
fn a_yellow_srgb_texture_blits_to_and_from_the_default_framebuffer() {
    let _g = context();
    let tex = texture_target(GL_SRGB8_ALPHA8);
    let s = clear([1.0, 1.0, 0.0, 1.0]);
    assert_eq!(s.refused, None);
    /* the default framebuffer: opaque black first, so a missed blit shows */
    bind(0, 0);
    let c = clear([0.0, 0.0, 0.0, 1.0]);
    assert_eq!(c.refused, None);
    bind(tex.fb, 0);
    blit_all(GL_NEAREST);
    assert_eq!(err(), GL_NO_ERROR, "blit_to_default");
    bind(0, 0);
    for p in read() {
        assert_eq!(p, [255, 255, 0, 255]);
    }
    /* and back: default -> sRGB renderbuffer, linear 0.5 -> a byte near 188 */
    bind(0, 0);
    clear([0.5, 0.5, 0.5, 1.0]);
    let rb = renderbuffer_target(GL_SRGB8_ALPHA8);
    bind(0, rb.fb);
    blit_all(GL_NEAREST);
    assert_eq!(err(), GL_NO_ERROR, "blit_from_default");
    bind(rb.fb, rb.fb);
    for p in read() {
        for ch in 0..3 {
            assert!((dec(p[ch]) - 0.5).abs() <= STEP * 2.0, "{p:?}");
        }
        assert_eq!(p[3], 255);
    }
}

/// `*_nearest_scale_*` and `*_linear_scale_*`: an sRGB source scaled up 2:1
/// into the default framebuffer decodes first.
#[test]
fn scaled_blits_to_the_default_framebuffer_decode() {
    let _g = context();
    let src = renderbuffer_target(GL_SRGB8_ALPHA8);
    draw();
    for filter in [GL_NEAREST, GL_LINEAR] {
        bind(0, 0);
        clear([0.0, 0.0, 0.0, 1.0]);
        bind(src.fb, 0);
        blit([0, 0, W / 2, W / 2], [0, 0, W, W], filter);
        assert_eq!(err(), GL_NO_ERROR, "filter {filter:#x}");
        bind(0, 0);
        let px = read();
        for (x, p) in px.iter().enumerate().take(W as usize) {
            let want = dec(stored_gradient(x / 2)[0]);
            let got = p[0] as f64 / 255.0;
            /* LINEAR also mixes in the neighbouring column: bounded by the
             * gradient's own step (1/7 of the range a source column) */
            let tol = if filter == GL_NEAREST { STEP } else { 0.2 };
            assert!((got - want).abs() <= tol, "filter {filter:#x} column {x}: {got} for {want}");
        }
    }
}

/// `*_out_of_bounds_blit_*`: a destination rectangle half outside the
/// default framebuffer is clipped, no error, and the inside part is the
/// source.
#[test]
fn an_out_of_bounds_blit_to_the_default_framebuffer_is_clipped() {
    let _g = context();
    let src = renderbuffer_target(GL_SRGB8_ALPHA8);
    clear([1.0, 1.0, 0.0, 1.0]);
    bind(0, 0);
    clear([0.0, 0.0, 0.0, 1.0]);
    bind(src.fb, 0);
    blit([0, 0, W, W], [W / 2, W / 2, W + W / 2, W + W / 2], GL_NEAREST);
    assert_eq!(err(), GL_NO_ERROR);
    bind(0, 0);
    let px = read();
    for y in 0..W as usize {
        for x in 0..W as usize {
            let inside = x >= (W / 2) as usize && y >= (W / 2) as usize;
            let want = if inside { [255, 255, 0, 255] } else { [0, 0, 0, 255] };
            assert_eq!(px[y * W as usize + x], want, "({x}, {y})");
        }
    }
}

/* ---- the other side of the conversion rows ------------------------------- */

/// Is `token` in this context's GL_EXTENSIONS (what dEQP's completeness cases
/// read to decide what a float colour format must be)?
fn ext_claimed(token: &str) -> bool {
    // SAFETY: GL_EXTENSIONS is a static NUL-terminated string the GL owns.
    unsafe {
        let s = core::ffi::CStr::from_ptr(glGetString(GL_EXTENSIONS) as *const _);
        s.to_str().unwrap_or_default().split(' ').any(|t| t == token)
    }
}

/// Is GL_R32F colour renderbuffer storage of this build: a 1x1 GL_R32F
/// colour renderbuffer whose storage was accepted and that completes its
/// framebuffer? ES 3.0 makes R32F colour-renderable under
/// GL_EXT_color_buffer_float only, which this build does not claim; the core
/// nevertheless completes it today, and plan #360's core-claim gates it on
/// the extension (docs/notes/es3-float-targets.md). The probe answers for
/// both states, so the r32f row below runs as today while the target is
/// renderable and holds the extension's ES 3.0 refusal once it is not.
fn r32f_colour_renderable() -> bool {
    let (mut fb, mut rb) = (0, 0);
    // SAFETY: plain GL calls; room for one name each; null pixels only
    // allocate.
    let (storage_ok, complete) = unsafe {
        glGenRenderbuffers(1, &mut rb);
        glBindRenderbuffer(GL_RENDERBUFFER, rb);
        glRenderbufferStorage(GL_RENDERBUFFER, GL_R32F, 1, 1);
        let storage_ok = err() == GL_NO_ERROR;
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_RENDERBUFFER, rb);
        let complete = glCheckFramebufferStatus(GL_FRAMEBUFFER) == GL_FRAMEBUFFER_COMPLETE;
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glDeleteFramebuffers(1, &fb);
        glDeleteRenderbuffers(1, &rb);
        (storage_ok, complete)
    };
    while err() != GL_NO_ERROR {}
    storage_ok || complete
}

/// The other formats of `fbo.blit.conversion.*_to_srgb8_alpha8` and
/// `srgb8_alpha8_to_*`: each is a complete colour renderbuffer, except the
/// four that ES 3.0 does not make colour-renderable (R11F_G11F_B10F, R32F, RG32F,
/// RGBA32F: no EXT_color_buffer_float). Those are the `NotSupported` rows of
/// the Pi 4 list (docs/es3/pi4-gles3-fails.tsv), and stay
/// incomplete here: they are not failures. Only completeness is held; the
/// stand-in stores no pixels for these formats (see the file header).
///
/// The r32f row follows the build, not one answer. ES 3.0 names R32F
/// colour-renderable only under GL_EXT_color_buffer_float, unclaimed here,
/// but the core today completes a GL_R32F renderbuffer anyway; core-claim
/// stops that. [`r32f_colour_renderable`] probes a 1x1 R32F colour
/// renderbuffer: while it is complete the row runs as today, and once it is
/// not the row asserts the ES 3.0 answer instead (no extension listed,
/// storage GL_INVALID_ENUM) and skips.
#[test]
fn other_side_formats_are_complete_colour_buffers() {
    let _g = context();
    let r32f = r32f_colour_renderable();
    let formats: [(GLenum, &str); 15] = [
        (GL_R8, "r8"),
        (GL_RG8, "rg8"),
        (GL_RGB8, "rgb8"),
        (GL_RGB565, "rgb565"),
        (GL_RGBA4, "rgba4"),
        (GL_RGB5_A1, "rgb5_a1"),
        (GL_RGBA8, "rgba8"),
        (GL_RGB10_A2, "rgb10_a2"),
        (GL_R16F, "r16f"),
        (GL_RG16F, "rg16f"),
        (GL_RGBA16F, "rgba16f"),
        (GL_R11F_G11F_B10F, "r11f_g11f_b10f"),
        (GL_R32F, "r32f"),
        (GL_RG32F, "rg32f"),
        (GL_RGBA32F, "rgba32f"),
    ];
    let mut bad = Vec::new();
    for (f, name) in formats {
        if f == GL_R32F && !r32f {
            /* ES 3.0 4.4.2 with no GL_EXT_color_buffer_float in
             * GL_EXTENSIONS: R32F is not colour-renderable and its
             * renderbuffer storage is GL_INVALID_ENUM. Hold that much, then
             * skip the row: it is neither a pass nor a failure then. */
            assert!(
                !ext_claimed("GL_EXT_color_buffer_float"),
                "the r32f row skips only while GL_EXT_color_buffer_float is unclaimed"
            );
            let mut rb = 0;
            // SAFETY: room for one name; plain GL calls.
            unsafe {
                glGenRenderbuffers(1, &mut rb);
                glBindRenderbuffer(GL_RENDERBUFFER, rb);
                glRenderbufferStorage(GL_RENDERBUFFER, GL_R32F, W, W);
                glDeleteRenderbuffers(1, &rb);
            }
            assert_eq!(err(), GL_INVALID_ENUM, "r32f renderbuffer storage");
            eprintln!("skipping r32f: GL_R32F is no colour renderbuffer of this build (GL_EXT_color_buffer_float not claimed)");
            continue;
        }
        let (mut fb, mut rb) = (0, 0);
        // SAFETY: plain GL calls; room for one name each.
        let status = unsafe {
            glGenFramebuffers(1, &mut fb);
            glBindFramebuffer(GL_FRAMEBUFFER, fb);
            glGenRenderbuffers(1, &mut rb);
            glBindRenderbuffer(GL_RENDERBUFFER, rb);
            glRenderbufferStorage(GL_RENDERBUFFER, f, W, W);
            glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_RENDERBUFFER, rb);
            let s = glCheckFramebufferStatus(GL_FRAMEBUFFER);
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
            glDeleteFramebuffers(1, &fb);
            glDeleteRenderbuffers(1, &rb);
            s
        };
        let _ = err();
        let not_renderable = matches!(f, GL_R11F_G11F_B10F | GL_R32F | GL_RG32F | GL_RGBA32F);
        if (status == GL_FRAMEBUFFER_COMPLETE) == not_renderable {
            bad.push(format!("{name}: status {status:#x}"));
        }
    }
    assert!(bad.is_empty(), "{bad:?}");
}
