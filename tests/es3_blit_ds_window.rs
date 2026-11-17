// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! glBlitFramebuffer depth and stencil to and from the window (plan item
//! window-tests; docs/notes/blit-depth-window.md, "Design: the window in
//! `depth_source` and `copy_depth`"; ES 3.0.6 4.3.3, dEQP-GLES3
//! fbo.blit.depth_stencil and the default-framebuffer cases).
//!
//! The window is the default framebuffer (id 0). Its depth and stencil are
//! one object a back end keeps under the id `0x8000_0000 |
//! (default_surface.gpu >> 12)` (the formula of gl/draw.rs), read with
//! `depth_resolve` / `stencil_resolve` and written with `ds_write`. The
//! window is stored top-down unless `GlContext::default_y_up`, so the GL core
//! reverses the rows of a plane it reads, and writes a cut rectangle
//! (x, y, w, h) of GL rows at the memory rows `target_h - (y + h) ..
//! target_h - y`.
//!
//! Every test that reads or writes the window runs it stored top-down
//! (`default_y_up` false: memory row r is GL row `WIN - 1 - r`) and y-up
//! (true: no mirror). The stand-in keeps its planes in memory order and the
//! test sets `default_y_up` on the current context, so the stand-in of
//! tests/es3_blit_ds.rs (copied here, with a 16x16 window) needed nothing
//! more.
//!
//! Expected values come from a model written apart from the code (as in
//! tests/es3_blit_ds.rs): destination pixel (x, y) of the cut destination
//! rectangle takes source texel floor(s0 + (x + 0.5 - d0) * (s1 - s0) /
//! (d1 - d0)) per axis (in f64) when that lies in the cut source, and keeps
//! its value otherwise; the window's GL row g is memory row `WIN - 1 - g`
//! when top-down; the ds_write rectangle is the destination rectangle cut to
//! the window and the scissor box (the cases chosen keep every sample point
//! inside the source), written with the pitch `WIN`.
//!
//! The tests that need the window path (`DepthSource::Window`,
//! `PlaneDst::Window`) are `#[ignore]`d for the gl-ds-layers item: today the
//! window is GL_INVALID_OPERATION on both sides. The tests that pass today
//! (refusals that must stay) are not.
//!
//! Feature sets (as tests/es3_blit_ds.rs, with one GPU feature): `cargo test
//! --features host-test,gpu-powervr --test es3_blit_ds_window`, and the same
//! with `host-test,gpu-vivante`. Plain `host-test` does not build the library.
//! Not in a `report-es2` build: glBlitFramebuffer is GL_INVALID_OPERATION
//! there (gl/framebuffer.rs `blit_framebuffer_is_es3_only`).

#![cfg(all(feature = "host-test", not(feature = "report-es2")))]

use core::ffi::{c_char, c_void};
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{self, GlesDraw, GlesHeap, YakoglBackend};
use yakogl::gl::consts::*;
use yakogl::gl::context;
use yakogl::gl::framebuffer::{
    glBindFramebuffer, glBindRenderbuffer, glBlitFramebuffer, glCheckFramebufferStatus, glDeleteFramebuffers,
    glDeleteRenderbuffers, glFramebufferRenderbuffer, glFramebufferTexture2D, glGenFramebuffers, glGenRenderbuffers,
    glRenderbufferStorage,
};
use yakogl::gl::names::{self, Kind};
use yakogl::gl::state::{glDisable, glEnable, glGetError, glPixelStorei, glScissor};
use yakogl::gl::texture::{glBindTexture, glDeleteTextures, glGenTextures, glTexImage2D};
use yakogl::gl::types::*;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};

/// The window's side: `gl_context_create(WIN, WIN)`.
const WIN: u32 = 16;
const D: u32 = backend::DS_PLANE_DEPTH;
const S: u32 = backend::DS_PLANE_STENCIL;

/* ---- a context on the host, and back ends that keep a window's planes ----- */

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

/// The planes the stand-in keeps, by (object, plane): width, height and one
/// u32 per pixel in MEMORY order (a depth float's bits, or a stencil byte).
type Planes = std::collections::BTreeMap<(u32, u32), (u32, u32, Vec<u32>)>;
static PLANES: Mutex<Planes> = Mutex::new(Planes::new());
/// The ds_write calls it answered with a non-empty rectangle:
/// (id, plane, x, y, w, h, pitch).
type Write = (u32, u32, u32, u32, u32, u32, u32);
static WRITES: Mutex<Vec<Write>> = Mutex::new(Vec::new());
/// The resolve calls it answered: (id, w, h, pitch).
static RESOLVES: Mutex<Vec<(u32, u32, u32, u32)>> = Mutex::new(Vec::new());

/// Make the plane (`id`, `plane`), pixel (x, y) in memory order being
/// `f(x, y)`.
fn keep_plane(id: u32, plane: u32, w: u32, h: u32, f: impl Fn(u32, u32) -> u32) {
    let px = (0..h).flat_map(|y| (0..w).map(move |x| (x, y))).map(|(x, y)| f(x, y)).collect();
    PLANES.lock().unwrap_or_else(|e| e.into_inner()).insert((id, plane), (w, h, px));
}

fn kept_plane(id: u32, plane: u32) -> Vec<u32> {
    PLANES.lock().unwrap_or_else(|e| e.into_inner()).get(&(id, plane)).expect("a kept plane").2.clone()
}

unsafe extern "C" fn rw_depth_resolve(id: u32, dst: *mut f32, w: u32, h: u32, pitch: u32) -> i32 {
    RESOLVES.lock().unwrap_or_else(|e| e.into_inner()).push((id, w, h, pitch));
    let planes = PLANES.lock().unwrap_or_else(|e| e.into_inner());
    let Some((pw, ph, px)) = planes.get(&(id, D)) else { return 1 };
    assert_eq!((*pw, *ph), (w, h));
    for y in 0..h {
        for x in 0..w {
            // SAFETY: the caller's contract: `h` rows of `pitch` floats.
            unsafe { dst.add((y * pitch + x) as usize).write(f32::from_bits(px[(y * w + x) as usize])) };
        }
    }
    0
}

unsafe extern "C" fn rw_stencil_resolve(id: u32, dst: *mut u8, w: u32, h: u32, pitch: u32) -> i32 {
    RESOLVES.lock().unwrap_or_else(|e| e.into_inner()).push((id, w, h, pitch));
    let planes = PLANES.lock().unwrap_or_else(|e| e.into_inner());
    let Some((pw, ph, px)) = planes.get(&(id, S)) else { return 1 };
    assert_eq!((*pw, *ph), (w, h));
    for y in 0..h {
        for x in 0..w {
            // SAFETY: the caller's contract.
            unsafe { dst.add((y * pitch + x) as usize).write(px[(y * w + x) as usize] as u8) };
        }
    }
    0
}

#[allow(clippy::too_many_arguments)]
unsafe extern "C" fn rw_ds_write(
    id: u32,
    plane: u32,
    src: *const c_void,
    x: u32,
    y: u32,
    w: u32,
    h: u32,
    pitch: u32,
) -> i32 {
    if w != 0 && h != 0 {
        WRITES.lock().unwrap_or_else(|e| e.into_inner()).push((id, plane, x, y, w, h, pitch));
    }
    let mut planes = PLANES.lock().unwrap_or_else(|e| e.into_inner());
    let Some((pw, ph, px)) = planes.get_mut(&(id, plane)) else { return 1 };
    assert!(x + w <= *pw && y + h <= *ph, "the rectangle is inside the plane");
    for j in 0..h {
        for i in 0..w {
            let at = (j * pitch + i) as usize;
            // SAFETY: the caller's contract: `h` rows of `pitch` elements.
            let v = unsafe {
                if plane == S {
                    (src as *const u8).add(at).read() as u32
                } else {
                    (src as *const f32).add(at).read().to_bits()
                }
            };
            px[((y + j) * *pw + x + i) as usize] = v;
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

type Resolve = Option<unsafe extern "C" fn(u32, *mut f32, u32, u32, u32) -> i32>;
type StencilResolve = Option<unsafe extern "C" fn(u32, *mut u8, u32, u32, u32) -> i32>;
type DsWrite = Option<unsafe extern "C" fn(u32, u32, *const c_void, u32, u32, u32, u32, u32) -> i32>;

/// A stand-in with the given entries.
const fn backend_with(
    name: *const c_char,
    depth_resolve: Resolve,
    ds_write: DsWrite,
    stencil_resolve: StencilResolve,
) -> YakoglBackend {
    YakoglBackend {
        name,
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
        depth_resolve,
        query_begin: None,
        query_end: None,
        query_result: None,
        generate_mipmap: None,
        return_mipmap_chain: None,
        linear_row_align: 0,
        uniforms_host_only: false,
        ds_write,
        stencil_resolve,
    }
}

/// Reads and writes depth and stencil.
static BACKEND_RW: YakoglBackend =
    backend_with(c"win-rw".as_ptr(), Some(rw_depth_resolve), Some(rw_ds_write), Some(rw_stencil_resolve));
/// Reads depth and stencil, cannot write (no `ds_write`).
static BACKEND_READ_ONLY: YakoglBackend =
    backend_with(c"win-read-only".as_ptr(), Some(rw_depth_resolve), None, Some(rw_stencil_resolve));
/// Writes, but reads no depth (no `depth_resolve`); reads stencil.
static BACKEND_NO_DEPTH_READ: YakoglBackend =
    backend_with(c"win-no-depth-read".as_ptr(), None, Some(rw_ds_write), Some(rw_stencil_resolve));
/// Reads and writes depth, reads no stencil (no `stencil_resolve`).
static BACKEND_NO_STENCIL_READ: YakoglBackend =
    backend_with(c"win-no-stencil-read".as_ptr(), Some(rw_depth_resolve), Some(rw_ds_write), None);

/// One context for the whole file; the tests take turns on it.
static LOCK: Mutex<()> = Mutex::new(());
static ONCE: Once = Once::new();

/// The test's context on back end `b`, with the window stored top-down.
fn context_on(b: &'static YakoglBackend) -> MutexGuard<'static, ()> {
    let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    // SAFETY: the tables are statics; the lock serialises the tests.
    unsafe {
        ONCE.call_once(|| {
            osal_set_ops(&OSAL);
            backend::g_backend = b;
            assert!(!context::gl_context_create(WIN, WIN).is_null());
        });
        backend::g_backend = b;
        glPixelStorei(GL_UNPACK_ALIGNMENT, 1);
        glPixelStorei(GL_PACK_ALIGNMENT, 1);
        glDisable(GL_SCISSOR_TEST);
        while glGetError() != GL_NO_ERROR {}
    }
    window_y_up(false);
    PLANES.lock().unwrap_or_else(|e| e.into_inner()).clear();
    WRITES.lock().unwrap_or_else(|e| e.into_inner()).clear();
    RESOLVES.lock().unwrap_or_else(|e| e.into_inner()).clear();
    g
}

fn context_rw() -> MutexGuard<'static, ()> {
    context_on(&BACKEND_RW)
}

/// Store the window bottom-up (`true`, a pbuffer) or top-down (`false`).
fn window_y_up(up: bool) {
    let c = context::current().expect("a current context");
    assert_eq!((c.target_w, c.target_h), (WIN, WIN));
    c.default_y_up = up;
}

/// The id the back ends keep the window's depth and stencil under
/// (the formula of gl/draw.rs: 0x8000_0000 | (default_surface.gpu >> 12)).
fn window_id() -> u32 {
    let c = context::current().expect("a current context");
    assert!(c.default_surface_valid);
    0x8000_0000 | (c.default_surface.gpu >> 12) as u32
}

/// The memory row of the window's GL row `g` (the mirror is an involution).
fn mem_row(g: u32, y_up: bool) -> u32 {
    if y_up {
        g
    } else {
        WIN - 1 - g
    }
}

fn err() -> GLenum {
    // SAFETY: plain GL call.
    unsafe { glGetError() }
}

fn blit(s: [i32; 4], d: [i32; 4], mask: GLbitfield, filter: GLenum) {
    // SAFETY: plain GL call.
    unsafe { glBlitFramebuffer(s[0], s[1], s[2], s[3], d[0], d[1], d[2], d[3], mask, filter) };
}

fn bind(read: GLuint, draw: GLuint) {
    // SAFETY: plain GL calls.
    unsafe {
        glBindFramebuffer(GL_READ_FRAMEBUFFER, read);
        glBindFramebuffer(GL_DRAW_FRAMEBUFFER, draw);
    }
}

/* ---- objects ----------------------------------------------------------- */

/// A `w` x `h` D24S8 renderbuffer at GL_DEPTH_STENCIL_ATTACHMENT of a new
/// framebuffer: (framebuffer, renderbuffer). The window is D24S8, so the
/// formats of 4.3.3 agree.
fn renderbuffer_fbo(w: i32, h: i32) -> (GLuint, GLuint) {
    let (mut fb, mut rb) = (0, 0);
    // SAFETY: room for one name each; plain GL calls.
    unsafe {
        glGenRenderbuffers(1, &mut rb);
        glBindRenderbuffer(GL_RENDERBUFFER, rb);
        glRenderbufferStorage(GL_RENDERBUFFER, GL_DEPTH24_STENCIL8, w, h);
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_DEPTH_STENCIL_ATTACHMENT, GL_RENDERBUFFER, rb);
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE);
    }
    (fb, rb)
}

/// A `w` x `h` D24S8 texture, depth (x, y) = `f(x, y)`, at
/// GL_DEPTH_STENCIL_ATTACHMENT of a new framebuffer: (framebuffer, texture).
fn texture_fbo(w: u32, h: u32, f: impl Fn(u32, u32) -> f32) -> (GLuint, GLuint) {
    let (mut tex, mut fb) = (0, 0);
    // SAFETY: room for one name each; no pixels (the texels are written below).
    unsafe {
        glGenTextures(1, &mut tex);
        glBindTexture(GL_TEXTURE_2D, tex);
        glTexImage2D(
            GL_TEXTURE_2D,
            0,
            GL_DEPTH24_STENCIL8 as GLint,
            w as i32,
            h as i32,
            0,
            GL_DEPTH_STENCIL,
            GL_UNSIGNED_INT_24_8,
            core::ptr::null(),
        );
        assert_eq!(err(), GL_NO_ERROR);
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        glFramebufferTexture2D(GL_FRAMEBUFFER, GL_DEPTH_STENCIL_ATTACHMENT, GL_TEXTURE_2D, tex, 0);
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE);
    }
    for y in 0..h {
        for x in 0..w {
            // SAFETY: inside the texture's host allocation (texel_ptr checks).
            unsafe { texel_ptr(tex, x, y).write(f(x, y)) };
        }
    }
    (fb, tex)
}

/// The R32F texel (x, y) of level 0 of depth texture `tex`, as stored.
fn texel_ptr(tex: GLuint, x: u32, y: u32) -> *mut f32 {
    let c = context::current().expect("a current context");
    let slot = names::slot_of(c, Kind::Texture, tex).expect("a texture name");
    let t = c.texture_object(slot).expect("a texture object");
    assert!(x < t.width && y < t.height);
    let off = t.level_off[0] as usize + (y as usize * t.level_pitch_at(0) as usize + x as usize) * 4;
    assert!(off + 4 <= t.mem.size);
    (t.mem.cpu as *mut u8).wrapping_add(off) as *mut f32
}

fn picture(tex: GLuint, w: u32, h: u32) -> Vec<f32> {
    // SAFETY: inside the texture's host allocation (texel_ptr checks).
    (0..h).flat_map(|y| (0..w).map(move |x| (x, y))).map(|(x, y)| unsafe { texel_ptr(tex, x, y).read() }).collect()
}

/// The id the back ends keep renderbuffer `rb`'s depth and stencil under.
fn renderbuffer_ds_id(rb: GLuint) -> u32 {
    let c = context::current().expect("a current context");
    let slot = names::slot_of(c, Kind::Renderbuffer, rb).expect("a renderbuffer name");
    context::ds_object_id(c.renderbuffer_object(slot).expect("a renderbuffer").serial, false)
}

fn release(fbs: &[GLuint], texs: &[GLuint], rbs: &[GLuint]) {
    // SAFETY: names these tests made.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glDeleteFramebuffers(fbs.len() as i32, fbs.as_ptr());
        glDeleteTextures(texs.len() as i32, texs.as_ptr());
        glDeleteRenderbuffers(rbs.len() as i32, rbs.as_ptr());
    }
}

/* ---- the model ----------------------------------------------------------- */

/// The source texel destination pixel `o` takes on one axis, or None when
/// its sample point is outside the source cut to `[0, n)`.
fn model_axis(o: i64, s0: i64, s1: i64, d0: i64, d1: i64, n: i64) -> Option<i64> {
    let p = s0 as f64 + (o as f64 + 0.5 - d0 as f64) * (s1 - s0) as f64 / (d1 - d0) as f64;
    let (lo, hi) = (s0.min(s1).max(0), s0.max(s1).min(n));
    let t = p.floor() as i64;
    (p >= lo as f64 && p < hi as f64).then_some(t)
}

/// The destination (GL row order, `dw` x `dh`) after the blit `s` -> `d` of
/// the `sw` x `sh` source `src` (GL row order) into a destination that held
/// `keep`, with the scissor box `scissor` (None: off). Values are f32
/// (stencil bytes ride in them exactly).
fn model(
    src: &dyn Fn(u32, u32) -> f32,
    (sw, sh): (u32, u32),
    keep: &dyn Fn(u32, u32) -> f32,
    (dw, dh): (u32, u32),
    s: [i32; 4],
    d: [i32; 4],
    scissor: Option<[i32; 4]>,
) -> Vec<f32> {
    let mut out = Vec::new();
    for y in 0..dh {
        for x in 0..dw {
            let (xi, yi) = (x as i64, y as i64);
            let in_dst = xi >= d[0].min(d[2]) as i64
                && xi < d[0].max(d[2]) as i64
                && yi >= d[1].min(d[3]) as i64
                && yi < d[1].max(d[3]) as i64;
            let in_scissor = scissor.is_none_or(|[sx, sy, w, h]| {
                xi >= sx as i64 && xi < (sx + w) as i64 && yi >= sy as i64 && yi < (sy + h) as i64
            });
            let from = (in_dst && in_scissor)
                .then(|| {
                    let tx = model_axis(xi, s[0] as i64, s[2] as i64, d[0] as i64, d[2] as i64, sw as i64)?;
                    let ty = model_axis(yi, s[1] as i64, s[3] as i64, d[1] as i64, d[3] as i64, sh as i64)?;
                    Some((tx as u32, ty as u32))
                })
                .flatten();
            out.push(match from {
                Some((tx, ty)) => src(tx, ty),
                None => keep(x, y),
            });
        }
    }
    out
}

/// The destination rectangle (x, y, w, h) in GL rows, cut to a `dw` x `dh`
/// image and the scissor box.
fn cut_rect(d: [i32; 4], scissor: Option<[i32; 4]>, (dw, dh): (i32, i32)) -> (u32, u32, u32, u32) {
    let (mut x0, mut x1) = (d[0].min(d[2]).max(0), d[0].max(d[2]).min(dw));
    let (mut y0, mut y1) = (d[1].min(d[3]).max(0), d[1].max(d[3]).min(dh));
    if let Some([sx, sy, w, h]) = scissor {
        (x0, x1, y0, y1) = (x0.max(sx), x1.min(sx + w), y0.max(sy), y1.min(sy + h));
    }
    (x0 as u32, y0 as u32, (x1 - x0) as u32, (y1 - y0) as u32)
}

/// Depth of the sources and of what the destinations held: distinct
/// everywhere, exact in f32.
fn src_depth(x: u32, y: u32) -> f32 {
    (y * 32 + x) as f32 / 1024.0
}
fn dst_depth(x: u32, y: u32) -> f32 {
    0.75 + (y * 16 + x) as f32 / 4096.0
}
/// The window's depth as the plane holds it at memory (x, r).
fn win_depth(x: u32, r: u32) -> f32 {
    (r * 16 + x + 1) as f32 / 512.0
}
/// Stencil values: a byte, never 0, different on every pixel of a side.
fn src_stencil(x: u32, y: u32) -> f32 {
    (10 + y * 8 + x) as f32
}
fn dst_stencil(x: u32, y: u32) -> f32 {
    (100 + y * 16 + x) as f32
}
fn win_stencil(x: u32, r: u32) -> f32 {
    (1 + (r * 16 + x) % 250) as f32
}

fn bits(f: impl Fn(u32, u32) -> f32) -> impl Fn(u32, u32) -> u32 {
    move |x, y| f(x, y).to_bits()
}
fn byte(f: impl Fn(u32, u32) -> f32) -> impl Fn(u32, u32) -> u32 {
    move |x, y| f(x, y) as u32
}
fn floats(v: Vec<u32>) -> Vec<f32> {
    v.into_iter().map(f32::from_bits).collect()
}
fn bytes(v: Vec<u32>) -> Vec<f32> {
    v.into_iter().map(|b| b as f32).collect()
}

/// The window's memory plane (as `f` of (x, memory row)) in GL row order.
fn gl_order(f: impl Fn(u32, u32) -> f32, y_up: bool) -> impl Fn(u32, u32) -> f32 {
    move |x, g| f(x, mem_row(g, y_up))
}

/// A GL-ordered picture (`WIN` x `WIN`) as the memory-ordered plane.
fn to_memory(pic: &[f32], y_up: bool) -> Vec<f32> {
    (0..WIN)
        .flat_map(|r| (0..WIN).map(move |x| (x, r)))
        .map(|(x, r)| pic[(mem_row(r, y_up) * WIN + x) as usize])
        .collect()
}

/// The blits of the window: whole, scaled and flipped past the edge, and cut
/// by the scissor. Every sample point of the destination rectangle (cut to the
/// window) is inside the source (a `sw` x `sh` image).
fn window_cases(sw: i32, sh: i32) -> Vec<([i32; 4], [i32; 4], Option<[i32; 4]>)> {
    vec![
        ([0, 0, sw, sh], [0, 0, WIN as i32, WIN as i32], None),
        ([0, 0, sw, sh], [20, 14, 4, 6], None),
        ([sw, 0, 0, sh], [0, 0, WIN as i32, WIN as i32], Some([3, 5, 6, 4])),
        ([1, 2, sw - 1, sh - 2], [2, 3, 10, 11], None),
    ]
}

/* ---- (a) depth from the window --------------------------------------------- */

/// Depth from the window into a depth renderbuffer, the window top-down and
/// y-up: the destination takes the model's values of the window in GL row
/// order, and the rest of its plane stays. One ds_write per blit carries the
/// cut rectangle of the renderbuffer (a renderbuffer is never mirrored).
#[test]
#[ignore = "gl-ds-layers: window depth/stencil blit (docs/notes/blit-depth-window.md)"]
fn depth_from_the_window_into_a_renderbuffer() {
    let _g = context_rw();
    let (dfb, drb) = renderbuffer_fbo(WIN as i32, WIN as i32);
    let (wid, did) = (window_id(), renderbuffer_ds_id(drb));
    for y_up in [false, true] {
        window_y_up(y_up);
        let win = gl_order(win_depth, y_up);
        for (s, d, scissor) in window_cases(WIN as i32, WIN as i32) {
            keep_plane(wid, D, WIN, WIN, bits(win_depth));
            keep_plane(did, D, WIN, WIN, bits(dst_depth));
            WRITES.lock().unwrap().clear();
            bind(0, dfb);
            if let Some([x, y, w, h]) = scissor {
                // SAFETY: plain GL calls.
                unsafe {
                    glEnable(GL_SCISSOR_TEST);
                    glScissor(x, y, w, h);
                }
            }
            blit(s, d, GL_DEPTH_BUFFER_BIT, GL_NEAREST);
            // SAFETY: plain GL call.
            unsafe { glDisable(GL_SCISSOR_TEST) };
            let what = format!("y_up {y_up}: {s:?} -> {d:?} scissor {scissor:?}");
            assert_eq!(err(), GL_NO_ERROR, "{what}");
            let want = model(&win, (WIN, WIN), &dst_depth, (WIN, WIN), s, d, scissor);
            assert_eq!(floats(kept_plane(did, D)), want, "{what}");
            let (x, y, w, h) = cut_rect(d, scissor, (WIN as i32, WIN as i32));
            assert_eq!(WRITES.lock().unwrap().as_slice(), &[(did, D, x, y, w, h, WIN)], "{what}");
            assert_eq!(floats(kept_plane(wid, D)), filled(&win_depth), "{what}: the window is read, not written");
        }
    }
    release(&[dfb], &[], &[drb]);
}

/// The same into a depth texture, which the GL core holds itself, on a back
/// end that cannot write a plane (`depth_resolve` only): the window is read
/// whole under its id, once per blit, with the mirror applied by the core.
#[test]
#[ignore = "gl-ds-layers: window depth/stencil blit (docs/notes/blit-depth-window.md)"]
fn depth_from_the_window_into_a_texture() {
    let _g = context_on(&BACKEND_READ_ONLY);
    let (dfb, tex) = texture_fbo(WIN, WIN, dst_depth);
    let wid = window_id();
    for y_up in [false, true] {
        window_y_up(y_up);
        let win = gl_order(win_depth, y_up);
        for (s, d, scissor) in window_cases(WIN as i32, WIN as i32) {
            keep_plane(wid, D, WIN, WIN, bits(win_depth));
            RESOLVES.lock().unwrap().clear();
            // the texture back to what it held
            for y in 0..WIN {
                for x in 0..WIN {
                    // SAFETY: inside the texture (texel_ptr checks).
                    unsafe { texel_ptr(tex, x, y).write(dst_depth(x, y)) };
                }
            }
            bind(0, dfb);
            if let Some([x, y, w, h]) = scissor {
                // SAFETY: plain GL calls.
                unsafe {
                    glEnable(GL_SCISSOR_TEST);
                    glScissor(x, y, w, h);
                }
            }
            blit(s, d, GL_DEPTH_BUFFER_BIT, GL_NEAREST);
            // SAFETY: plain GL call.
            unsafe { glDisable(GL_SCISSOR_TEST) };
            let what = format!("y_up {y_up}: {s:?} -> {d:?} scissor {scissor:?}");
            assert_eq!(err(), GL_NO_ERROR, "{what}");
            assert_eq!(
                RESOLVES.lock().unwrap().as_slice(),
                &[(wid, WIN, WIN, WIN)],
                "{what}: one read of the whole plane"
            );
            let want = model(&win, (WIN, WIN), &dst_depth, (WIN, WIN), s, d, scissor);
            assert_eq!(picture(tex, WIN, WIN), want, "{what}");
        }
    }
    release(&[dfb], &[tex], &[]);
}

/* ---- (b) depth into the window --------------------------------------------- */

/// What the source of a blit into the window is.
#[derive(Clone, Copy, Debug)]
enum Source {
    Renderbuffer,
    Texture,
}

/// Depth into the window from an 8x8 source, for each `window_cases` blit,
/// the window top-down and y-up. The window's plane is read whole (memory
/// order), mirrored, written by the copy, mirrored back: every pixel takes the
/// model's value (GL row order, mapped to memory rows), and the one ds_write
/// is the cut rectangle at the memory rows `WIN - (y + h) .. WIN - y` (top-down)
/// or `y .. y + h` (y-up), pitch `WIN`.
fn check_depth_into_window(source: Source) {
    let _g = context_rw();
    let wid = window_id();
    let (sfb, srb, tex) = match source {
        Source::Renderbuffer => {
            let (fb, rb) = renderbuffer_fbo(8, 8);
            (fb, rb, 0)
        }
        Source::Texture => {
            let (fb, tex) = texture_fbo(8, 8, src_depth);
            (fb, 0, tex)
        }
    };
    for y_up in [false, true] {
        window_y_up(y_up);
        let keep = gl_order(win_depth, y_up);
        for (s, d, scissor) in window_cases(8, 8) {
            keep_plane(wid, D, WIN, WIN, bits(win_depth));
            if srb != 0 {
                keep_plane(renderbuffer_ds_id(srb), D, 8, 8, bits(src_depth));
            }
            WRITES.lock().unwrap().clear();
            bind(sfb, 0);
            if let Some([x, y, w, h]) = scissor {
                // SAFETY: plain GL calls.
                unsafe {
                    glEnable(GL_SCISSOR_TEST);
                    glScissor(x, y, w, h);
                }
            }
            blit(s, d, GL_DEPTH_BUFFER_BIT, GL_NEAREST);
            // SAFETY: plain GL call.
            unsafe { glDisable(GL_SCISSOR_TEST) };
            let what = format!("{source:?}, y_up {y_up}: {s:?} -> {d:?} scissor {scissor:?}");
            assert_eq!(err(), GL_NO_ERROR, "{what}");
            let gl_pic = model(&src_depth, (8, 8), &keep, (WIN, WIN), s, d, scissor);
            assert_eq!(floats(kept_plane(wid, D)), to_memory(&gl_pic, y_up), "{what}: the window plane, memory order");
            let (x, y, w, h) = cut_rect(d, scissor, (WIN as i32, WIN as i32));
            let row = if y_up { y } else { WIN - (y + h) };
            assert_eq!(
                WRITES.lock().unwrap().as_slice(),
                &[(wid, D, x, row, w, h, WIN)],
                "{what}: the ds_write rectangle"
            );
        }
    }
    let rbs = if srb != 0 { vec![srb] } else { vec![] };
    release(&[sfb], &[tex], &rbs);
}

#[test]
#[ignore = "gl-ds-layers: window depth/stencil blit (docs/notes/blit-depth-window.md)"]
fn depth_from_a_renderbuffer_into_the_window() {
    check_depth_into_window(Source::Renderbuffer);
}

#[test]
#[ignore = "gl-ds-layers: window depth/stencil blit (docs/notes/blit-depth-window.md)"]
fn depth_from_a_texture_into_the_window() {
    check_depth_into_window(Source::Texture);
}

/* ---- (c) stencil through the window ---------------------------------------- */

/// Stencil from the window into a D24S8 renderbuffer (`stencil_resolve`, then
/// `ds_write` of the renderbuffer): the stencil plane takes the model's
/// values of the window in GL row order, the depth plane is not touched.
#[test]
#[ignore = "gl-ds-layers: window depth/stencil blit (docs/notes/blit-depth-window.md)"]
fn stencil_from_the_window() {
    let _g = context_rw();
    let (dfb, drb) = renderbuffer_fbo(WIN as i32, WIN as i32);
    let (wid, did) = (window_id(), renderbuffer_ds_id(drb));
    for y_up in [false, true] {
        window_y_up(y_up);
        let win = gl_order(win_stencil, y_up);
        for (s, d, scissor) in window_cases(WIN as i32, WIN as i32) {
            keep_plane(wid, S, WIN, WIN, byte(win_stencil));
            keep_plane(did, S, WIN, WIN, byte(dst_stencil));
            keep_plane(did, D, WIN, WIN, bits(dst_depth));
            WRITES.lock().unwrap().clear();
            bind(0, dfb);
            if let Some([x, y, w, h]) = scissor {
                // SAFETY: plain GL calls.
                unsafe {
                    glEnable(GL_SCISSOR_TEST);
                    glScissor(x, y, w, h);
                }
            }
            blit(s, d, GL_STENCIL_BUFFER_BIT, GL_NEAREST);
            // SAFETY: plain GL call.
            unsafe { glDisable(GL_SCISSOR_TEST) };
            let what = format!("y_up {y_up}: {s:?} -> {d:?} scissor {scissor:?}");
            assert_eq!(err(), GL_NO_ERROR, "{what}");
            let want = model(&win, (WIN, WIN), &dst_stencil, (WIN, WIN), s, d, scissor);
            assert_eq!(bytes(kept_plane(did, S)), want, "{what}");
            let (x, y, w, h) = cut_rect(d, scissor, (WIN as i32, WIN as i32));
            assert_eq!(WRITES.lock().unwrap().as_slice(), &[(did, S, x, y, w, h, WIN)], "{what}");
            assert_eq!(floats(kept_plane(did, D)), filled(&dst_depth), "{what}: depth untouched");
        }
    }
    release(&[dfb], &[], &[drb]);
}

/// Stencil from an 8x8 D24S8 renderbuffer into the window: the window's
/// stencil plane is read whole, mirrored, copied into, mirrored back, and the
/// cut rectangle written at its memory rows; the window's depth plane stays.
#[test]
#[ignore = "gl-ds-layers: window depth/stencil blit (docs/notes/blit-depth-window.md)"]
fn stencil_into_the_window() {
    let _g = context_rw();
    let (sfb, srb) = renderbuffer_fbo(8, 8);
    let (wid, sid) = (window_id(), renderbuffer_ds_id(srb));
    for y_up in [false, true] {
        window_y_up(y_up);
        let keep = gl_order(win_stencil, y_up);
        for (s, d, scissor) in window_cases(8, 8) {
            keep_plane(wid, S, WIN, WIN, byte(win_stencil));
            keep_plane(wid, D, WIN, WIN, bits(win_depth));
            keep_plane(sid, S, 8, 8, byte(src_stencil));
            WRITES.lock().unwrap().clear();
            bind(sfb, 0);
            if let Some([x, y, w, h]) = scissor {
                // SAFETY: plain GL calls.
                unsafe {
                    glEnable(GL_SCISSOR_TEST);
                    glScissor(x, y, w, h);
                }
            }
            blit(s, d, GL_STENCIL_BUFFER_BIT, GL_NEAREST);
            // SAFETY: plain GL call.
            unsafe { glDisable(GL_SCISSOR_TEST) };
            let what = format!("y_up {y_up}: {s:?} -> {d:?} scissor {scissor:?}");
            assert_eq!(err(), GL_NO_ERROR, "{what}");
            let gl_pic = model(&src_stencil, (8, 8), &keep, (WIN, WIN), s, d, scissor);
            assert_eq!(bytes(kept_plane(wid, S)), to_memory(&gl_pic, y_up), "{what}");
            let (x, y, w, h) = cut_rect(d, scissor, (WIN as i32, WIN as i32));
            let row = if y_up { y } else { WIN - (y + h) };
            assert_eq!(WRITES.lock().unwrap().as_slice(), &[(wid, S, x, row, w, h, WIN)], "{what}");
            assert_eq!(floats(kept_plane(wid, D)), filled(&win_depth), "{what}: depth untouched");
        }
    }
    release(&[sfb], &[], &[srb]);
}

/// Depth and stencil in one call into the window: both planes, each with its
/// own ds_write at the same memory rows.
#[test]
#[ignore = "gl-ds-layers: window depth/stencil blit (docs/notes/blit-depth-window.md)"]
fn depth_and_stencil_into_the_window_together() {
    let _g = context_rw();
    let (sfb, srb) = renderbuffer_fbo(8, 8);
    let (wid, sid) = (window_id(), renderbuffer_ds_id(srb));
    let (s, d) = ([0, 0, 8, 8], [2, 3, 10, 11]);
    for y_up in [false, true] {
        window_y_up(y_up);
        keep_plane(wid, D, WIN, WIN, bits(win_depth));
        keep_plane(wid, S, WIN, WIN, byte(win_stencil));
        keep_plane(sid, D, 8, 8, bits(src_depth));
        keep_plane(sid, S, 8, 8, byte(src_stencil));
        WRITES.lock().unwrap().clear();
        bind(sfb, 0);
        blit(s, d, GL_DEPTH_BUFFER_BIT | GL_STENCIL_BUFFER_BIT, GL_NEAREST);
        assert_eq!(err(), GL_NO_ERROR, "y_up {y_up}");
        let depth = model(&src_depth, (8, 8), &gl_order(win_depth, y_up), (WIN, WIN), s, d, None);
        let stencil = model(&src_stencil, (8, 8), &gl_order(win_stencil, y_up), (WIN, WIN), s, d, None);
        assert_eq!(floats(kept_plane(wid, D)), to_memory(&depth, y_up), "y_up {y_up}: depth");
        assert_eq!(bytes(kept_plane(wid, S)), to_memory(&stencil, y_up), "y_up {y_up}: stencil");
        let (x, y, w, h) = cut_rect(d, None, (WIN as i32, WIN as i32));
        let row = if y_up { y } else { WIN - (y + h) };
        let mut writes = WRITES.lock().unwrap().clone();
        writes.sort();
        assert_eq!(writes, [(wid, D, x, row, w, h, WIN), (wid, S, x, row, w, h, WIN)], "y_up {y_up}");
    }
    release(&[sfb], &[], &[srb]);
}

/* ---- (d) window to window --------------------------------------------------- */

/// "If the same buffer is read and written" is GL_INVALID_OPERATION: the
/// window to the window is refused whatever the entries (4.3.3), for each of
/// depth and stencil, nothing is read or written. Passes today, must stay.
#[test]
fn the_window_to_the_window_is_an_error() {
    let _g = context_rw();
    keep_plane(window_id(), D, WIN, WIN, bits(win_depth));
    keep_plane(window_id(), S, WIN, WIN, byte(win_stencil));
    for y_up in [false, true] {
        window_y_up(y_up);
        for mask in [GL_DEPTH_BUFFER_BIT, GL_STENCIL_BUFFER_BIT, GL_DEPTH_BUFFER_BIT | GL_STENCIL_BUFFER_BIT] {
            bind(0, 0);
            blit([0, 0, 8, 8], [8, 8, 16, 16], mask, GL_NEAREST);
            assert_eq!(err(), GL_INVALID_OPERATION, "y_up {y_up}, mask {mask:#x}");
        }
    }
    assert!(WRITES.lock().unwrap().is_empty(), "nothing written");
    assert!(RESOLVES.lock().unwrap().is_empty(), "nothing read");
    assert_eq!(floats(kept_plane(window_id(), D)), filled(&win_depth));
}

/* ---- (e) the refusals the missing entries keep ------------------------------ */

/// A back end without `ds_write` cannot write the window's depth or stencil:
/// GL_INVALID_OPERATION (REFUSED_DEPTH_DST / REFUSED_STENCIL), nothing read.
/// Passes today, must stay.
#[test]
fn without_ds_write_the_window_stays_refused_as_a_destination() {
    let _g = context_on(&BACKEND_READ_ONLY);
    let (sfb, srb) = renderbuffer_fbo(8, 8);
    let (tfb, tex) = texture_fbo(8, 8, src_depth);
    keep_plane(window_id(), D, WIN, WIN, bits(win_depth));
    keep_plane(window_id(), S, WIN, WIN, byte(win_stencil));
    keep_plane(renderbuffer_ds_id(srb), D, 8, 8, bits(src_depth));
    keep_plane(renderbuffer_ds_id(srb), S, 8, 8, byte(src_stencil));
    for y_up in [false, true] {
        window_y_up(y_up);
        for (read, what) in [(sfb, "a renderbuffer"), (tfb, "a texture")] {
            for mask in [GL_DEPTH_BUFFER_BIT, GL_STENCIL_BUFFER_BIT] {
                if read == tfb && mask == GL_STENCIL_BUFFER_BIT {
                    continue; // stencil out of a texture: its own refusal (gl-ds-layers)
                }
                bind(read, 0);
                blit([0, 0, 8, 8], [0, 0, 16, 16], mask, GL_NEAREST);
                assert_eq!(err(), GL_INVALID_OPERATION, "y_up {y_up}: {what}, mask {mask:#x} into the window");
            }
        }
    }
    assert_eq!(floats(kept_plane(window_id(), D)), filled(&win_depth), "the window is untouched");
    assert!(RESOLVES.lock().unwrap().is_empty(), "a refused call reads nothing");
    release(&[sfb, tfb], &[tex], &[srb]);
}

/// A back end without `depth_resolve` cannot read the window's depth
/// (REFUSED_DEPTH_SRC), though it could write: into a texture and into a
/// renderbuffer it is GL_INVALID_OPERATION. A back end without
/// `stencil_resolve` cannot read the window's stencil. Nothing is written.
/// Passes today, must stay.
#[test]
fn without_the_resolve_entries_the_window_stays_refused_as_a_source() {
    {
        let _g = context_on(&BACKEND_NO_DEPTH_READ);
        let (dfb, drb) = renderbuffer_fbo(WIN as i32, WIN as i32);
        let (tfb, tex) = texture_fbo(WIN, WIN, dst_depth);
        keep_plane(window_id(), D, WIN, WIN, bits(win_depth));
        keep_plane(renderbuffer_ds_id(drb), D, WIN, WIN, bits(dst_depth));
        for y_up in [false, true] {
            window_y_up(y_up);
            for (draw, what) in [(dfb, "a renderbuffer"), (tfb, "a texture")] {
                bind(0, draw);
                blit([0, 0, 16, 16], [0, 0, 16, 16], GL_DEPTH_BUFFER_BIT, GL_NEAREST);
                assert_eq!(err(), GL_INVALID_OPERATION, "y_up {y_up}: depth from the window into {what}");
            }
        }
        assert!(WRITES.lock().unwrap().is_empty(), "nothing written");
        assert_eq!(floats(kept_plane(renderbuffer_ds_id(drb), D)), filled(&dst_depth));
        release(&[dfb, tfb], &[tex], &[drb]);
    }
    {
        let _g = context_on(&BACKEND_NO_STENCIL_READ);
        let (dfb, drb) = renderbuffer_fbo(WIN as i32, WIN as i32);
        keep_plane(window_id(), S, WIN, WIN, byte(win_stencil));
        keep_plane(renderbuffer_ds_id(drb), S, WIN, WIN, byte(dst_stencil));
        for y_up in [false, true] {
            window_y_up(y_up);
            bind(0, dfb);
            blit([0, 0, 16, 16], [0, 0, 16, 16], GL_STENCIL_BUFFER_BIT, GL_NEAREST);
            assert_eq!(err(), GL_INVALID_OPERATION, "y_up {y_up}: stencil from the window");
        }
        assert!(WRITES.lock().unwrap().is_empty(), "nothing written");
        release(&[dfb], &[], &[drb]);
    }
}

/// A back end with `ds_write` but no `stencil_resolve` cannot write stencil
/// into the window from a renderbuffer either (the source's stencil is read
/// through `stencil_resolve`). Passes today, must stay.
#[test]
fn without_stencil_resolve_no_stencil_reaches_the_window() {
    let _g = context_on(&BACKEND_NO_STENCIL_READ);
    let (sfb, srb) = renderbuffer_fbo(8, 8);
    keep_plane(window_id(), S, WIN, WIN, byte(win_stencil));
    keep_plane(renderbuffer_ds_id(srb), S, 8, 8, byte(src_stencil));
    bind(sfb, 0);
    blit([0, 0, 8, 8], [0, 0, 16, 16], GL_STENCIL_BUFFER_BIT, GL_NEAREST);
    assert_eq!(err(), GL_INVALID_OPERATION);
    assert!(WRITES.lock().unwrap().is_empty());
    assert_eq!(bytes(kept_plane(window_id(), S)), filled(&win_stencil));
    release(&[sfb], &[], &[srb]);
}

/// The destination plane being unheld (the probe `ds_write` of an empty
/// rectangle answers 1) cannot be written: GL_INVALID_OPERATION before any
/// copy, nothing written (REFUSED_DS_UNHELD, as for a renderbuffer in
/// tests/es3_blit_ds.rs). Today it is refused as a window; after gl-ds-layers
/// it is refused as unheld: either way this stays an error.
#[test]
fn a_window_destination_the_back_end_holds_nothing_for_is_an_error() {
    let _g = context_rw();
    let (sfb, srb) = renderbuffer_fbo(8, 8);
    keep_plane(renderbuffer_ds_id(srb), D, 8, 8, bits(src_depth));
    keep_plane(renderbuffer_ds_id(srb), S, 8, 8, byte(src_stencil));
    for y_up in [false, true] {
        window_y_up(y_up);
        for mask in [GL_DEPTH_BUFFER_BIT, GL_STENCIL_BUFFER_BIT] {
            bind(sfb, 0);
            blit([0, 0, 8, 8], [0, 0, 16, 16], mask, GL_NEAREST);
            assert_eq!(err(), GL_INVALID_OPERATION, "y_up {y_up}, mask {mask:#x}");
        }
    }
    assert!(WRITES.lock().unwrap().is_empty());
    release(&[sfb], &[], &[srb]);
}

/* ---- (f) a window plane that holds nothing ---------------------------------- */

/// The back end answers 1 ("holds nothing") for the window's depth: its
/// depth is undefined, so the blit is not an error and the destination stays as
/// it was, nothing written. Into a renderbuffer and into a texture.
#[test]
#[ignore = "gl-ds-layers: window depth/stencil blit (docs/notes/blit-depth-window.md)"]
fn a_window_depth_that_holds_nothing_leaves_the_destination() {
    let _g = context_rw();
    let (dfb, drb) = renderbuffer_fbo(WIN as i32, WIN as i32);
    let (tfb, tex) = texture_fbo(WIN, WIN, dst_depth);
    let did = renderbuffer_ds_id(drb);
    for y_up in [false, true] {
        window_y_up(y_up);
        keep_plane(did, D, WIN, WIN, bits(dst_depth)); /* no plane for the window's id */
        for draw in [dfb, tfb] {
            RESOLVES.lock().unwrap().clear();
            bind(0, draw);
            blit([0, 0, 16, 16], [0, 0, 16, 16], GL_DEPTH_BUFFER_BIT, GL_NEAREST);
            assert_eq!(err(), GL_NO_ERROR, "y_up {y_up}, draw {draw}");
            assert_eq!(RESOLVES.lock().unwrap().as_slice(), &[(window_id(), WIN, WIN, WIN)], "the window was asked");
        }
        assert_eq!(floats(kept_plane(did, D)), filled(&dst_depth), "y_up {y_up}: the renderbuffer is as it was");
        assert_eq!(picture(tex, WIN, WIN), filled(&dst_depth), "y_up {y_up}: the texture is as it was");
        assert!(WRITES.lock().unwrap().is_empty(), "y_up {y_up}: nothing written");
    }
    release(&[dfb, tfb], &[tex], &[drb]);
}

/// The same for the window's stencil.
#[test]
#[ignore = "gl-ds-layers: window depth/stencil blit (docs/notes/blit-depth-window.md)"]
fn a_window_stencil_that_holds_nothing_leaves_the_destination() {
    let _g = context_rw();
    let (dfb, drb) = renderbuffer_fbo(WIN as i32, WIN as i32);
    let did = renderbuffer_ds_id(drb);
    for y_up in [false, true] {
        window_y_up(y_up);
        keep_plane(did, S, WIN, WIN, byte(dst_stencil));
        bind(0, dfb);
        blit([0, 0, 16, 16], [0, 0, 16, 16], GL_STENCIL_BUFFER_BIT, GL_NEAREST);
        assert_eq!(err(), GL_NO_ERROR, "y_up {y_up}");
        assert_eq!(bytes(kept_plane(did, S)), filled(&dst_stencil), "y_up {y_up}");
        assert!(WRITES.lock().unwrap().is_empty(), "y_up {y_up}: nothing written");
    }
    release(&[dfb], &[], &[drb]);
}

/// A `WIN` x `WIN` picture of `f`.
fn filled(f: &dyn Fn(u32, u32) -> f32) -> Vec<f32> {
    (0..WIN).flat_map(|y| (0..WIN).map(move |x| (x, y))).map(|(x, y)| f(x, y)).collect()
}
