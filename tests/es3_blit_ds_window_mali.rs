// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! glBlitFramebuffer depth and stencil to and from the window (the default
//! framebuffer) over the Mali (Valhall G610) back end's table, on the host:
//! what the GL core answers today, pinned, and `#[ignore]`d twins that hold
//! what a working blit leaves. The window counterpart of
//! tests/es3_blit_ds_mali.rs (renderbuffer blits); the window path itself
//! (`DepthSource::Window`, `PlaneDst::Window`; docs/notes/blit-depth-window.md,
//! "Design") is pinned on a stand-in with all three entries in
//! tests/es3_blit_ds_window.rs.
//!
//! HOST-ONLY: an API verdict of the GL core on the Mali's table; no Mali
//! frame runs and no picture is read (tools/cts/README.md 2b).
//!
//! The Mali's table (`yakogl_backend_mali`, src/mali/backend.rs) has
//! `depth_resolve` but no `ds_write` and no `stencil_resolve` (plan #152).
//! So, with or without the GL core's window path:
//!   - depth or stencil INTO the window needs `ds_write` of the window's
//!     plane: GL_INVALID_OPERATION (REFUSED_DEPTH_DST / REFUSED_STENCIL);
//!   - stencil FROM the window needs `stencil_resolve`: refused;
//!   - depth or stencil from the window into a RENDERBUFFER needs `ds_write`
//!     of the renderbuffer (a renderbuffer is written through the back end
//!     only): refused;
//!   - depth from the window into a depth TEXTURE needs only `depth_resolve`
//!     (the GL core holds the texels): it works through the GL core's window
//!     path (plan item gl-ds-window), Mali entries or not, and is pinned
//!     unignored on the Mali's own table (`depth_from_the_window_into_a_texture`).
//!
//! Window -> window is GL_INVALID_OPERATION whatever the table (ES 3.0.6
//! 4.3.3: the same buffer read and written). A blit whose mask is only
//! GL_COLOR_BUFFER_BIT raises nothing. A refused call fails whole, the
//! colour too.
//!
//! The rig is tests/es3_blit_ds_mali.rs's: the GL core and a stand-in table
//! with the Mali's entries (`ds_write` and `stencil_resolve` none; a test
//! checks that against `yakogl_backend_mali`) and a `depth_resolve` that
//! answers planes the test seeds (the real one needs a Mali frame). The
//! model and helpers (`model_axis`, `model`, `cut_rect`, `mem_row`) are
//! tests/es3_blit_ds_window.rs's. The window is stored top-down unless
//! `GlContext::default_y_up`; each twin runs both.
//!
//! The twins run on `BACKEND_LIFTED`: the same table with a stand-in
//! `ds_write` and `stencil_resolve` that keep planes in memory. To lift
//! (plan #152 gives the Mali the entries; gl-ds-window has landed): remove
//! the `#[ignore]`s and replace `BACKEND_LIFTED`'s two entries with
//! `yakogl_backend_mali`'s on a Mali OSAL rig that reads the planes back
//! through them; the seeded `depth_resolve` stands until then.
//!
//! Not in a `report-es2` build: glBlitFramebuffer is GL_INVALID_OPERATION
//! there.
//!
//! Run: `cargo test --no-default-features --features
//! host-test,gpu-mali,shaderc-online,no-shader-table --test
//! es3_blit_ds_window_mali` (add `-- --ignored` for the twins).

#![cfg(all(feature = "host-test", feature = "shaderc-online", feature = "gpu-mali", not(feature = "report-es2")))]

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
use yakogl::gl::state::{glClear, glDisable, glEnable, glGetError, glPixelStorei, glScissor};
use yakogl::gl::texture::{glBindTexture, glDeleteTextures, glGenTextures, glTexImage2D};
use yakogl::gl::types::*;
use yakogl::mali::backend::yakogl_backend_mali;
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

/// The Mali's table as the GL core sees it: `depth_resolve` (seeded here),
/// no `ds_write`, no `stencil_resolve` (plan #152).
static BACKEND: YakoglBackend = backend_with(c"win-mali".as_ptr(), Some(rw_depth_resolve), None, None);
/// The same with the entries #152 adds, as a stand-in keeping planes.
static BACKEND_LIFTED: YakoglBackend =
    backend_with(c"win-mali-lifted".as_ptr(), Some(rw_depth_resolve), Some(rw_ds_write), Some(rw_stencil_resolve));

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

/// The context on the lifted table (the twins).
fn context_lifted() -> MutexGuard<'static, ()> {
    context_on(&BACKEND_LIFTED)
}

/// The context on the Mali's table (the refusals).
fn context_mali() -> MutexGuard<'static, ()> {
    context_on(&BACKEND)
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

/// One blit: the source rectangle, the destination rectangle, the scissor.
type WindowCase = ([i32; 4], [i32; 4], Option<[i32; 4]>);

/// The blits of the window: whole, scaled and flipped past the edge, and cut
/// by the scissor. Every sample point of the destination rectangle (cut to the
/// window) is inside the source (a `sw` x `sh` image).
fn window_cases(sw: i32, sh: i32) -> Vec<WindowCase> {
    vec![
        ([0, 0, sw, sh], [0, 0, WIN as i32, WIN as i32], None),
        ([0, 0, sw, sh], [20, 14, 4, 6], None),
        ([sw, 0, 0, sh], [0, 0, WIN as i32, WIN as i32], Some([3, 5, 6, 4])),
        ([1, 2, sw - 1, sh - 2], [2, 3, 10, 11], None),
    ]
}

/// A `WIN` x `WIN` picture of `f`.
fn filled(f: &dyn Fn(u32, u32) -> f32) -> Vec<f32> {
    (0..WIN).flat_map(|y| (0..WIN).map(move |x| (x, y))).map(|(x, y)| f(x, y)).collect()
}

/// An 8x8 picture of `f`.
fn filled_8(f: &dyn Fn(u32, u32) -> f32) -> Vec<f32> {
    (0..8).flat_map(|y| (0..8).map(move |x| (x, y))).map(|(x, y)| f(x, y)).collect()
}

/// `blit` with the scissor box of `scissor` (None: off), restored after.
fn blit_scissored(s: [i32; 4], d: [i32; 4], mask: GLbitfield, scissor: Option<[i32; 4]>) {
    if let Some([x, y, w, h]) = scissor {
        // SAFETY: plain GL calls.
        unsafe {
            glEnable(GL_SCISSOR_TEST);
            glScissor(x, y, w, h);
        }
    }
    blit(s, d, mask, GL_NEAREST);
    // SAFETY: plain GL call.
    unsafe { glDisable(GL_SCISSOR_TEST) };
}

/// The window's two planes as seeded (memory order).
fn seed_window(wid: u32) {
    keep_plane(wid, D, WIN, WIN, bits(win_depth));
    keep_plane(wid, S, WIN, WIN, byte(win_stencil));
}

/// The table is what this file says it is: the Mali's has no `ds_write` and
/// no `stencil_resolve`, and the stand-in `BACKEND` agrees with it; only
/// `BACKEND_LIFTED` has both.
#[test]
fn the_mali_table_has_neither_entry() {
    assert!(yakogl_backend_mali.depth_resolve.is_some());
    assert!(yakogl_backend_mali.ds_write.is_none(), "the Mali's table has a ds_write");
    assert!(yakogl_backend_mali.stencil_resolve.is_none(), "the Mali's table has a stencil_resolve");
    assert_eq!(BACKEND.ds_write.is_some(), yakogl_backend_mali.ds_write.is_some());
    assert_eq!(BACKEND.stencil_resolve.is_some(), yakogl_backend_mali.stencil_resolve.is_some());
    assert!(BACKEND_LIFTED.ds_write.is_some() && BACKEND_LIFTED.stencil_resolve.is_some());
}

/* ---- (a) refused today, and after the GL core's window path lands too ------- */

/// Depth and stencil into the window from a renderbuffer and from a texture
/// (stencil from a renderbuffer only): the Mali has no `ds_write`, so the
/// window cannot be written: exactly one GL_INVALID_OPERATION, no plane of
/// either side changed, nothing written. Mixed with GL_COLOR_BUFFER_BIT the
/// call fails whole (still one error). The context stays usable.
#[test]
fn depth_and_stencil_into_the_window_are_refused() {
    let _g = context_mali();
    let (rfb, rb) = renderbuffer_fbo(8, 8);
    let (tfb, tex) = texture_fbo(8, 8, src_depth);
    let (wid, rid) = (window_id(), renderbuffer_ds_id(rb));
    let both = GL_DEPTH_BUFFER_BIT | GL_STENCIL_BUFFER_BIT;
    for y_up in [false, true] {
        window_y_up(y_up);
        for (read, what, masks) in [
            (rfb, "a renderbuffer", vec![GL_DEPTH_BUFFER_BIT, GL_STENCIL_BUFFER_BIT, both]),
            (tfb, "a texture", vec![GL_DEPTH_BUFFER_BIT]),
        ] {
            for mask in masks.into_iter().flat_map(|m| [m, m | GL_COLOR_BUFFER_BIT]) {
                seed_window(wid);
                keep_plane(rid, D, 8, 8, bits(src_depth));
                keep_plane(rid, S, 8, 8, byte(src_stencil));
                bind(read, 0);
                blit([0, 0, 8, 8], [2, 3, 10, 11], mask, GL_NEAREST);
                let what = format!("y_up {y_up}: from {what}, mask {mask:#x}");
                assert_eq!(err(), GL_INVALID_OPERATION, "{what}");
                assert_eq!(err(), GL_NO_ERROR, "{what}: exactly one error");
                assert_eq!(floats(kept_plane(wid, D)), filled(&win_depth), "{what}: window depth");
                assert_eq!(bytes(kept_plane(wid, S)), filled(&win_stencil), "{what}: window stencil");
                assert_eq!(floats(kept_plane(rid, D)), filled_8(&src_depth), "{what}: source depth");
                assert!(WRITES.lock().unwrap().is_empty(), "{what}: nothing written");
            }
        }
        /* usable: a clear raises nothing */
        bind(rfb, 0);
        // SAFETY: plain GL call.
        unsafe { glClear(GL_DEPTH_BUFFER_BIT | GL_STENCIL_BUFFER_BIT) };
        assert_eq!(err(), GL_NO_ERROR, "y_up {y_up}: a clear after the refusals");
    }
    release(&[rfb, tfb], &[tex], &[rb]);
}

/// Depth and stencil from the window into a renderbuffer: the Mali can read
/// the window's depth (`depth_resolve`) but not its stencil
/// (`stencil_resolve`), and cannot write the renderbuffer's plane
/// (`ds_write`): refused whatever the mask, the destination unchanged.
#[test]
fn depth_and_stencil_from_the_window_into_a_renderbuffer_are_refused() {
    let _g = context_mali();
    let (dfb, drb) = renderbuffer_fbo(WIN as i32, WIN as i32);
    let (wid, did) = (window_id(), renderbuffer_ds_id(drb));
    for y_up in [false, true] {
        window_y_up(y_up);
        for mask in [GL_DEPTH_BUFFER_BIT, GL_STENCIL_BUFFER_BIT, GL_DEPTH_BUFFER_BIT | GL_STENCIL_BUFFER_BIT] {
            seed_window(wid);
            keep_plane(did, D, WIN, WIN, bits(dst_depth));
            keep_plane(did, S, WIN, WIN, byte(dst_stencil));
            bind(0, dfb);
            blit([0, 0, 16, 16], [0, 0, 16, 16], mask, GL_NEAREST);
            let what = format!("y_up {y_up}: mask {mask:#x}");
            assert_eq!(err(), GL_INVALID_OPERATION, "{what}");
            assert_eq!(err(), GL_NO_ERROR, "{what}: exactly one error");
            assert_eq!(floats(kept_plane(did, D)), filled(&dst_depth), "{what}: destination depth");
            assert_eq!(bytes(kept_plane(did, S)), filled(&dst_stencil), "{what}: destination stencil");
            assert!(WRITES.lock().unwrap().is_empty(), "{what}: nothing written");
        }
    }
    release(&[dfb], &[], &[drb]);
}

/// Stencil from the window into a framebuffer with a depth texture: the Mali
/// has no `stencil_resolve`, so the window's stencil cannot be read (and a
/// texture takes no stencil blit at this base either): refused.
#[test]
fn stencil_from_the_window_into_a_texture_is_refused() {
    let _g = context_mali();
    let (tfb, tex) = texture_fbo(WIN, WIN, dst_depth);
    seed_window(window_id());
    for y_up in [false, true] {
        window_y_up(y_up);
        bind(0, tfb);
        blit([0, 0, 16, 16], [0, 0, 16, 16], GL_STENCIL_BUFFER_BIT, GL_NEAREST);
        assert_eq!(err(), GL_INVALID_OPERATION, "y_up {y_up}");
        assert_eq!(err(), GL_NO_ERROR, "y_up {y_up}: exactly one error");
        assert_eq!(picture(tex, WIN, WIN), filled(&dst_depth), "y_up {y_up}: the texture is as it was");
    }
    assert!(WRITES.lock().unwrap().is_empty(), "nothing written");
    release(&[tfb], &[tex], &[]);
}

/// The window to the window is GL_INVALID_OPERATION (4.3.3, the same buffer)
/// for each of depth and stencil, nothing read or written.
#[test]
fn the_window_to_the_window_is_an_error() {
    let _g = context_mali();
    seed_window(window_id());
    for y_up in [false, true] {
        window_y_up(y_up);
        for mask in [GL_DEPTH_BUFFER_BIT, GL_STENCIL_BUFFER_BIT, GL_DEPTH_BUFFER_BIT | GL_STENCIL_BUFFER_BIT] {
            bind(0, 0);
            blit([0, 0, 8, 8], [8, 8, 16, 16], mask, GL_NEAREST);
            assert_eq!(err(), GL_INVALID_OPERATION, "y_up {y_up}, mask {mask:#x}");
            assert_eq!(err(), GL_NO_ERROR, "y_up {y_up}, mask {mask:#x}: exactly one error");
        }
    }
    assert!(WRITES.lock().unwrap().is_empty(), "nothing written");
    assert_eq!(floats(kept_plane(window_id(), D)), filled(&win_depth));
}

/// A colour-only blit into the window raises no error and writes no plane:
/// the refusals above are of the depth and stencil bits only.
#[test]
fn a_colour_only_blit_into_the_window_is_no_error() {
    let _g = context_mali();
    let (mut fb, mut rb) = (0, 0);
    // SAFETY: room for one name each; plain GL calls.
    unsafe {
        glGenRenderbuffers(1, &mut rb);
        glBindRenderbuffer(GL_RENDERBUFFER, rb);
        glRenderbufferStorage(GL_RENDERBUFFER, GL_RGBA8, 8, 8);
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_RENDERBUFFER, rb);
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE);
    }
    seed_window(window_id());
    for y_up in [false, true] {
        window_y_up(y_up);
        bind(fb, 0);
        blit([0, 0, 8, 8], [0, 0, 16, 16], GL_COLOR_BUFFER_BIT, GL_NEAREST);
        assert_eq!(err(), GL_NO_ERROR, "y_up {y_up}");
    }
    assert!(WRITES.lock().unwrap().is_empty(), "no plane written");
    assert_eq!(floats(kept_plane(window_id(), D)), filled(&win_depth));
    release(&[fb], &[], &[rb]);
}

/* ---- (b) the twins: what a working blit leaves ------------------------------- */

/// Depth from the window into a depth renderbuffer, the window top-down and
/// y-up: the destination takes the model's values of the window in GL row
/// order; one ds_write per blit carries the cut rectangle; the window is read,
/// not written.
#[test]
#[ignore = "needs Mali ds_write/stencil_resolve (#152)"]
fn depth_from_the_window_into_a_renderbuffer() {
    let _g = context_lifted();
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
            blit_scissored(s, d, GL_DEPTH_BUFFER_BIT, scissor);
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

/// Depth from the window into a depth texture, which the GL core holds
/// itself: the Mali's table as it is (`depth_resolve` only) is enough
/// through the GL core's window path (gl-ds-window), without #152. The
/// window is read whole under its id, once per blit, mirrored by the core.
#[test]
fn depth_from_the_window_into_a_texture() {
    let _g = context_mali();
    let (dfb, tex) = texture_fbo(WIN, WIN, dst_depth);
    let wid = window_id();
    for y_up in [false, true] {
        window_y_up(y_up);
        let win = gl_order(win_depth, y_up);
        for (s, d, scissor) in window_cases(WIN as i32, WIN as i32) {
            keep_plane(wid, D, WIN, WIN, bits(win_depth));
            RESOLVES.lock().unwrap().clear();
            for y in 0..WIN {
                for x in 0..WIN {
                    // SAFETY: inside the texture (texel_ptr checks).
                    unsafe { texel_ptr(tex, x, y).write(dst_depth(x, y)) };
                }
            }
            bind(0, dfb);
            blit_scissored(s, d, GL_DEPTH_BUFFER_BIT, scissor);
            let what = format!("y_up {y_up}: {s:?} -> {d:?} scissor {scissor:?}");
            assert_eq!(err(), GL_NO_ERROR, "{what}");
            assert_eq!(RESOLVES.lock().unwrap().as_slice(), &[(wid, WIN, WIN, WIN)], "{what}: one read");
            let want = model(&win, (WIN, WIN), &dst_depth, (WIN, WIN), s, d, scissor);
            assert_eq!(picture(tex, WIN, WIN), want, "{what}");
        }
    }
    release(&[dfb], &[tex], &[]);
}

/// What the source of a blit into the window is.
#[derive(Clone, Copy, Debug)]
enum Source {
    Renderbuffer,
    Texture,
}

/// Depth into the window from an 8x8 source, for each `window_cases` blit,
/// top-down and y-up: every pixel takes the model's value (GL row order,
/// mapped to memory rows) and the one ds_write is the cut rectangle at the
/// memory rows `WIN - (y + h) .. WIN - y` (top-down) or `y .. y + h` (y-up).
fn check_depth_into_window(source: Source) {
    let _g = context_lifted();
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
            blit_scissored(s, d, GL_DEPTH_BUFFER_BIT, scissor);
            let what = format!("{source:?}, y_up {y_up}: {s:?} -> {d:?} scissor {scissor:?}");
            assert_eq!(err(), GL_NO_ERROR, "{what}");
            let gl_pic = model(&src_depth, (8, 8), &keep, (WIN, WIN), s, d, scissor);
            assert_eq!(floats(kept_plane(wid, D)), to_memory(&gl_pic, y_up), "{what}: the window plane, memory order");
            let (x, y, w, h) = cut_rect(d, scissor, (WIN as i32, WIN as i32));
            let row = if y_up { y } else { WIN - (y + h) };
            assert_eq!(WRITES.lock().unwrap().as_slice(), &[(wid, D, x, row, w, h, WIN)], "{what}: the ds_write");
        }
    }
    let rbs = if srb != 0 { vec![srb] } else { vec![] };
    release(&[sfb], &[tex], &rbs);
}

#[test]
#[ignore = "needs Mali ds_write/stencil_resolve (#152)"]
fn depth_from_a_renderbuffer_into_the_window() {
    check_depth_into_window(Source::Renderbuffer);
}

#[test]
#[ignore = "needs Mali ds_write/stencil_resolve (#152)"]
fn depth_from_a_texture_into_the_window() {
    check_depth_into_window(Source::Texture);
}

/// Stencil from the window into a D24S8 renderbuffer (`stencil_resolve`, then
/// `ds_write` of the renderbuffer): the stencil plane takes the model's values
/// of the window in GL row order; the depth plane is not touched.
#[test]
#[ignore = "needs Mali ds_write/stencil_resolve (#152)"]
fn stencil_from_the_window() {
    let _g = context_lifted();
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
            blit_scissored(s, d, GL_STENCIL_BUFFER_BIT, scissor);
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

/// Stencil from an 8x8 D24S8 renderbuffer into the window: the cut rectangle
/// written at its memory rows; the window's depth plane stays.
#[test]
#[ignore = "needs Mali ds_write/stencil_resolve (#152)"]
fn stencil_into_the_window() {
    let _g = context_lifted();
    let (sfb, srb) = renderbuffer_fbo(8, 8);
    let (wid, sid) = (window_id(), renderbuffer_ds_id(srb));
    for y_up in [false, true] {
        window_y_up(y_up);
        let keep = gl_order(win_stencil, y_up);
        for (s, d, scissor) in window_cases(8, 8) {
            seed_window(wid);
            keep_plane(sid, S, 8, 8, byte(src_stencil));
            WRITES.lock().unwrap().clear();
            bind(sfb, 0);
            blit_scissored(s, d, GL_STENCIL_BUFFER_BIT, scissor);
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
#[ignore = "needs Mali ds_write/stencil_resolve (#152)"]
fn depth_and_stencil_into_the_window_together() {
    let _g = context_lifted();
    let (sfb, srb) = renderbuffer_fbo(8, 8);
    let (wid, sid) = (window_id(), renderbuffer_ds_id(srb));
    let (s, d) = ([0, 0, 8, 8], [2, 3, 10, 11]);
    for y_up in [false, true] {
        window_y_up(y_up);
        seed_window(wid);
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
