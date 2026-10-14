// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The depth/stencil half of glBlitFramebuffer (plan item blit-ds; ES 3.0.6
//! 4.3.3, dEQP-GLES3 fbo.blit.depth_stencil).
//!
//! The GL core copies depth on the CPU into a plane it holds itself: the
//! R32F texels of a depth texture (gl/blit.rs `copy_depth`), read from a
//! depth texture's texels or from the depth a back end keeps for a depth
//! renderbuffer (`backend::depth_resolve`). The copies it cannot make --
//! into a depth renderbuffer or the window, from the window, any stencil --
//! need a back-end entry that writes a buffer's plane, and stay
//! GL_INVALID_OPERATION, named once in the log.
//!
//! Every expected picture comes from a model written apart from the code:
//! destination pixel (x, y) of the cut destination rectangle takes source
//! texel floor(s0 + (x + 0.5 - d0) * (s1 - s0) / (d1 - d0)) per axis (in
//! f64; the sample points are chosen where f32 and f64 agree) when that lies
//! in the cut source, and keeps its value otherwise.
//!
//! No GPU runs: the stand-in back end draws nothing; its `depth_resolve`
//! answers a known ramp for the renderbuffers a test says it holds.
//!
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
    glReadPixels, glRenderbufferStorage,
};
use yakogl::gl::names::{self, Kind};
use yakogl::gl::state::{glDisable, glEnable, glGetError, glPixelStorei, glScissor};
use yakogl::gl::texture::{glBindTexture, glDeleteTextures, glGenTextures, glTexImage2D};
use yakogl::gl::types::*;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};

/* ---- a context on the host, and a back end that keeps some depth -------- */

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

/// The depth the stand-in keeps for object `id` at (x, y), when it keeps
/// any (`HELD`).
fn held_depth(id: u32, x: u32, y: u32) -> f32 {
    ((id & 0xff) as f32 * 1000.0 + y as f32 * 64.0 + x as f32) / 65536.0
}

/// The objects the stand-in keeps depth for, and the calls it answered:
/// (id, w, h, pitch).
static HELD: Mutex<Vec<u32>> = Mutex::new(Vec::new());
static RESOLVES: Mutex<Vec<(u32, u32, u32, u32)>> = Mutex::new(Vec::new());

unsafe extern "C" fn b_depth_resolve(id: u32, dst: *mut f32, w: u32, h: u32, pitch: u32) -> i32 {
    RESOLVES.lock().unwrap_or_else(|e| e.into_inner()).push((id, w, h, pitch));
    if !HELD.lock().unwrap_or_else(|e| e.into_inner()).contains(&id) {
        return 1;
    }
    for y in 0..h {
        for x in 0..w {
            // SAFETY: the caller's contract: `h` rows of `pitch` floats.
            unsafe { dst.add((y * pitch + x) as usize).write(held_depth(id, x, y)) };
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
    lock: None,
    unlock: None,
    thread_id: None,
};

static BACKEND: YakoglBackend = YakoglBackend {
    name: c"es3-blit-ds".as_ptr() as *const c_char,
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
    depth_resolve: Some(b_depth_resolve),
    query_begin: None,
    query_end: None,
    query_result: None,
    generate_mipmap: None,
    return_mipmap_chain: None,
};

/// One context for the whole file; the tests take turns on it.
static LOCK: Mutex<()> = Mutex::new(());
static ONCE: Once = Once::new();

fn context() -> MutexGuard<'static, ()> {
    let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    // SAFETY: the tables are statics; the lock serialises the tests.
    unsafe {
        ONCE.call_once(|| {
            osal_set_ops(&OSAL);
            backend::g_backend = &BACKEND;
            assert!(!context::gl_context_create(16, 16).is_null());
        });
        glPixelStorei(GL_UNPACK_ALIGNMENT, 1);
        glPixelStorei(GL_PACK_ALIGNMENT, 1);
        glDisable(GL_SCISSOR_TEST);
        while glGetError() != GL_NO_ERROR {}
    }
    HELD.lock().unwrap_or_else(|e| e.into_inner()).clear();
    RESOLVES.lock().unwrap_or_else(|e| e.into_inner()).clear();
    g
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

/// How a depth texture is specified: (internal format, format, type).
const D32F: (GLenum, GLenum, GLenum) = (GL_DEPTH_COMPONENT32F, GL_DEPTH_COMPONENT, GL_FLOAT);
const D24: (GLenum, GLenum, GLenum) = (GL_DEPTH_COMPONENT24, GL_DEPTH_COMPONENT, GL_UNSIGNED_INT);
const D24S8: (GLenum, GLenum, GLenum) = (GL_DEPTH24_STENCIL8, GL_DEPTH_STENCIL, GL_UNSIGNED_INT_24_8);

/// A `w` x `h` depth texture, depth (x, y) = `f(x, y)` (a D32F's floats as
/// they are; the other formats are filled by the CPU after it is made).
fn depth_texture(fmt: (GLenum, GLenum, GLenum), w: u32, h: u32, f: impl Fn(u32, u32) -> f32) -> GLuint {
    let mut tex = 0;
    // SAFETY: room for one name; no pixels (the texels are written below).
    unsafe {
        glGenTextures(1, &mut tex);
        glBindTexture(GL_TEXTURE_2D, tex);
        glTexImage2D(GL_TEXTURE_2D, 0, fmt.0 as GLint, w as i32, h as i32, 0, fmt.1, fmt.2, core::ptr::null());
    }
    assert_eq!(err(), GL_NO_ERROR, "a {:#x} depth texture", fmt.0);
    for y in 0..h {
        for x in 0..w {
            set_texel(tex, x, y, f(x, y));
        }
    }
    tex
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

fn texel(tex: GLuint, x: u32, y: u32) -> f32 {
    // SAFETY: inside the texture's host allocation (texel_ptr checks).
    unsafe { texel_ptr(tex, x, y).read() }
}

fn set_texel(tex: GLuint, x: u32, y: u32, v: f32) {
    // SAFETY: as in `texel`.
    unsafe { texel_ptr(tex, x, y).write(v) }
}

/// A framebuffer with `tex` at GL_DEPTH_ATTACHMENT (or with `stencil_too`
/// at GL_DEPTH_STENCIL_ATTACHMENT), checked complete.
fn texture_fbo(tex: GLuint, stencil_too: bool) -> GLuint {
    let mut fb = 0;
    let point = if stencil_too { GL_DEPTH_STENCIL_ATTACHMENT } else { GL_DEPTH_ATTACHMENT };
    // SAFETY: room for one name; plain GL calls.
    unsafe {
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        glFramebufferTexture2D(GL_FRAMEBUFFER, point, GL_TEXTURE_2D, tex, 0);
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE);
    }
    fb
}

/// A `w` x `h` renderbuffer of `format` at `point` of a new framebuffer:
/// (framebuffer, renderbuffer).
fn renderbuffer_fbo(format: GLenum, point: GLenum, w: i32, h: i32) -> (GLuint, GLuint) {
    let (mut fb, mut rb) = (0, 0);
    // SAFETY: room for one name each; plain GL calls.
    unsafe {
        glGenRenderbuffers(1, &mut rb);
        glBindRenderbuffer(GL_RENDERBUFFER, rb);
        glRenderbufferStorage(GL_RENDERBUFFER, format, w, h);
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        glFramebufferRenderbuffer(GL_FRAMEBUFFER, point, GL_RENDERBUFFER, rb);
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE);
    }
    (fb, rb)
}

/// The id the back ends keep renderbuffer `rb`'s depth under
/// (`GlesDraw::depth_id`: its serial).
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

/// The destination after the blit `s` -> `d` of a `sw` x `sh` source whose
/// depth is `src` into a `dw` x `dh` destination that held `keep`, with the
/// scissor box `scissor` (None: off).
#[allow(clippy::too_many_arguments)]
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

/// A `w` x `h` picture of `f`.
fn filled(f: &dyn Fn(u32, u32) -> f32, (w, h): (u32, u32)) -> Vec<f32> {
    (0..h).flat_map(|y| (0..w).map(move |x| (x, y))).map(|(x, y)| f(x, y)).collect()
}

fn picture(tex: GLuint, w: u32, h: u32) -> Vec<f32> {
    (0..h).flat_map(|y| (0..w).map(move |x| (x, y))).map(|(x, y)| texel(tex, x, y)).collect()
}

/// Source and destination depth of the texture-to-texture cases: distinct
/// everywhere, exact in f32.
fn src_depth(x: u32, y: u32) -> f32 {
    (y * 32 + x) as f32 / 1024.0
}
fn dst_depth(x: u32, y: u32) -> f32 {
    0.75 + (y * 16 + x) as f32 / 4096.0
}

/// Blit depth texture to depth texture (`sw` x `sh` into `dw` x `dh`) and
/// compare every destination texel with the model.
fn check_texture_blit(sz: (u32, u32), dz: (u32, u32), s: [i32; 4], d: [i32; 4], scissor: Option<[i32; 4]>) {
    let src = depth_texture(D32F, sz.0, sz.1, src_depth);
    let dst = depth_texture(D32F, dz.0, dz.1, dst_depth);
    let (sfb, dfb) = (texture_fbo(src, false), texture_fbo(dst, false));
    bind(sfb, dfb);
    if let Some([x, y, w, h]) = scissor {
        // SAFETY: plain GL calls.
        unsafe {
            glEnable(GL_SCISSOR_TEST);
            glScissor(x, y, w, h);
        }
    }
    blit(s, d, GL_DEPTH_BUFFER_BIT, GL_NEAREST);
    assert_eq!(err(), GL_NO_ERROR, "{s:?} -> {d:?}");
    let want = model(&src_depth, sz, &dst_depth, dz, s, d, scissor);
    assert_eq!(picture(dst, dz.0, dz.1), want, "{s:?} -> {d:?} scissor {scissor:?}");
    assert_eq!(picture(src, sz.0, sz.1), filled(&src_depth, sz), "the source is untouched");
    release(&[sfb, dfb], &[src, dst], &[]);
}

/* ---- copies -------------------------------------------------------------- */

/// 1:1, whole and a part placed elsewhere.
#[test]
fn a_depth_texture_copies_one_to_one() {
    let _g = context();
    check_texture_blit((8, 8), (8, 8), [0, 0, 8, 8], [0, 0, 8, 8], None);
    check_texture_blit((8, 8), (8, 8), [1, 2, 5, 6], [3, 1, 7, 5], None);
}

/// NEAREST scaling, up and down, and different scales per axis.
#[test]
fn a_depth_texture_copies_scaled() {
    let _g = context();
    check_texture_blit((4, 4), (8, 8), [0, 0, 4, 4], [0, 0, 8, 8], None);
    check_texture_blit((8, 8), (4, 4), [0, 0, 8, 8], [0, 0, 4, 4], None);
    check_texture_blit((8, 8), (8, 8), [0, 0, 4, 8], [0, 0, 8, 4], None);
}

/// Reversed corners flip, on either rectangle and either axis.
#[test]
fn reversed_rectangles_flip_the_depth() {
    let _g = context();
    check_texture_blit((8, 8), (8, 8), [0, 0, 8, 8], [8, 0, 0, 8], None);
    check_texture_blit((8, 8), (8, 8), [0, 8, 8, 0], [0, 0, 8, 8], None);
    check_texture_blit((8, 8), (8, 8), [8, 8, 0, 0], [0, 0, 4, 4], None);
}

/// Rectangles past either image are cut, at the uncut rectangles' scale;
/// a destination pixel whose sample point leaves the source keeps its depth.
#[test]
fn rectangles_past_the_images_are_cut() {
    let _g = context();
    check_texture_blit((8, 8), (8, 8), [-4, -4, 12, 12], [0, 0, 8, 8], None);
    check_texture_blit((8, 8), (8, 8), [0, 0, 8, 8], [-2, 3, 6, 11], None);
    check_texture_blit((8, 8), (8, 8), [4, 4, 12, 12], [0, 0, 8, 8], None);
    check_texture_blit((4, 4), (8, 8), [-2, 0, 6, 4], [0, 0, 8, 8], None);
}

/// The scissor box cuts the destination, as for colour.
#[test]
fn the_scissor_cuts_the_depth_destination() {
    let _g = context();
    check_texture_blit((8, 8), (8, 8), [0, 0, 8, 8], [0, 0, 8, 8], Some([2, 1, 3, 5]));
    check_texture_blit((4, 4), (8, 8), [0, 0, 4, 4], [8, 8, 0, 0], Some([0, 4, 8, 4]));
}

/// A D24 texture copies too (its texels are floats as well), and a
/// depth-only blit into a D24S8 texture copies its depth.
#[test]
fn d24_and_d24s8_textures_copy_their_depth() {
    let _g = context();
    for fmt in [D24, D24S8] {
        let src = depth_texture(fmt, 4, 4, src_depth);
        let dst = depth_texture(fmt, 4, 4, dst_depth);
        let stencil = fmt == D24S8;
        let (sfb, dfb) = (texture_fbo(src, stencil), texture_fbo(dst, stencil));
        bind(sfb, dfb);
        blit([0, 0, 4, 4], [4, 0, 0, 4], GL_DEPTH_BUFFER_BIT, GL_NEAREST);
        assert_eq!(err(), GL_NO_ERROR);
        let want = model(&src_depth, (4, 4), &dst_depth, (4, 4), [0, 0, 4, 4], [4, 0, 0, 4], None);
        assert_eq!(picture(dst, 4, 4), want, "{:#x}", fmt.0);
        release(&[sfb, dfb], &[src, dst], &[]);
    }
}

/// From a depth renderbuffer: the depth the back end keeps for it, asked
/// under the renderbuffer's id, whole (the source rectangle maps into it).
#[test]
fn a_depth_renderbuffer_is_read_through_the_back_end() {
    let _g = context();
    let (sfb, rb) = renderbuffer_fbo(GL_DEPTH_COMPONENT32F, GL_DEPTH_ATTACHMENT, 8, 6);
    let id = renderbuffer_ds_id(rb);
    HELD.lock().unwrap().push(id);
    let dst = depth_texture(D32F, 8, 8, dst_depth);
    let dfb = texture_fbo(dst, false);
    bind(sfb, dfb);
    let (s, d) = ([1, 0, 7, 6], [0, 8, 8, 0]);
    blit(s, d, GL_DEPTH_BUFFER_BIT, GL_NEAREST);
    assert_eq!(err(), GL_NO_ERROR);
    assert_eq!(RESOLVES.lock().unwrap().as_slice(), &[(id, 8, 6, 8)], "one read of the whole plane");
    let src = |x, y| held_depth(id, x, y);
    assert_eq!(picture(dst, 8, 8), model(&src, (8, 6), &dst_depth, (8, 8), s, d, None));
    release(&[sfb, dfb], &[dst], &[rb]);
}

/// A renderbuffer the back end keeps nothing for has undefined depth: the
/// destination stays as it was, with no error.
#[test]
fn an_undefined_source_leaves_the_destination() {
    let _g = context();
    let (sfb, rb) = renderbuffer_fbo(GL_DEPTH_COMPONENT24, GL_DEPTH_ATTACHMENT, 4, 4);
    let dst = depth_texture(D24, 4, 4, dst_depth);
    let dfb = texture_fbo(dst, false);
    bind(sfb, dfb);
    blit([0, 0, 4, 4], [0, 0, 4, 4], GL_DEPTH_BUFFER_BIT, GL_NEAREST);
    assert_eq!(err(), GL_NO_ERROR);
    assert_eq!(RESOLVES.lock().unwrap().len(), 1);
    assert_eq!(picture(dst, 4, 4), filled(&dst_depth, (4, 4)));
    release(&[sfb, dfb], &[dst], &[rb]);
}

/// Colour and depth in one call: both copied, each with its own images.
#[test]
fn colour_and_depth_copy_in_one_call() {
    let _g = context();
    let src = depth_texture(D32F, 4, 4, src_depth);
    let dst = depth_texture(D32F, 4, 4, dst_depth);
    let (sfb, dfb) = (texture_fbo(src, false), texture_fbo(dst, false));
    let mut colour = [0u32; 2];
    // SAFETY: room for two names; plain GL calls; 16 RGBA pixels uploaded.
    unsafe {
        glGenTextures(2, colour.as_mut_ptr());
        for (i, (&tex, fb)) in colour.iter().zip([sfb, dfb]).enumerate() {
            let px = [i as u8 * 100 + 10; 64];
            glBindTexture(GL_TEXTURE_2D, tex);
            glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA as GLint, 4, 4, 0, GL_RGBA, GL_UNSIGNED_BYTE, px.as_ptr() as _);
            glBindFramebuffer(GL_FRAMEBUFFER, fb);
            glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex, 0);
            assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE);
        }
    }
    bind(sfb, dfb);
    blit([0, 0, 4, 4], [0, 0, 4, 4], GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT, GL_NEAREST);
    assert_eq!(err(), GL_NO_ERROR);
    assert_eq!(picture(dst, 4, 4), picture(src, 4, 4));
    let mut px = [0u8; 4];
    bind(dfb, dfb);
    // SAFETY: room for one RGBA pixel.
    unsafe { glReadPixels(2, 3, 1, 1, GL_RGBA, GL_UNSIGNED_BYTE, px.as_mut_ptr() as *mut c_void) };
    assert_eq!(px, [10; 4], "the source's colour");
    release(&[sfb, dfb], &[src, dst, colour[0], colour[1]], &[]);
}

/// "If a buffer is specified in mask and does not exist in both the read
/// and draw framebuffers, the corresponding bit is silently ignored."
#[test]
fn a_depth_bit_without_depth_on_one_side_is_ignored() {
    let _g = context();
    let src = depth_texture(D32F, 4, 4, src_depth);
    let sfb = texture_fbo(src, false);
    let (dfb, rb) = renderbuffer_fbo(GL_RGBA8, GL_COLOR_ATTACHMENT0, 4, 4);
    bind(sfb, dfb);
    blit([0, 0, 4, 4], [0, 0, 4, 4], GL_DEPTH_BUFFER_BIT | GL_STENCIL_BUFFER_BIT, GL_NEAREST);
    assert_eq!(err(), GL_NO_ERROR);
    release(&[sfb, dfb], &[src], &[rb]);
}

/* ---- ES 3.0.6 4.3.3 errors ------------------------------------------------- */

/// A filter other than NEAREST with depth or stencil, formats that differ,
/// and one image read and written are GL_INVALID_OPERATION, and nothing is
/// written.
#[test]
fn the_errors_of_4_3_3_for_depth_and_stencil() {
    let _g = context();
    let a = depth_texture(D32F, 4, 4, src_depth);
    let b = depth_texture(D32F, 4, 4, dst_depth);
    let c24 = depth_texture(D24, 4, 4, dst_depth);
    let (afb, bfb, cfb) = (texture_fbo(a, false), texture_fbo(b, false), texture_fbo(c24, false));
    let (sfb, srb) = renderbuffer_fbo(GL_DEPTH24_STENCIL8, GL_DEPTH_STENCIL_ATTACHMENT, 4, 4);
    let (s32fb, s32rb) = renderbuffer_fbo(GL_DEPTH32F_STENCIL8, GL_DEPTH_STENCIL_ATTACHMENT, 4, 4);
    let cases = [
        (afb, bfb, GL_DEPTH_BUFFER_BIT, GL_LINEAR, "LINEAR with depth"),
        (sfb, s32fb, GL_STENCIL_BUFFER_BIT, GL_LINEAR, "LINEAR with stencil"),
        (afb, cfb, GL_DEPTH_BUFFER_BIT, GL_NEAREST, "D32F into D24"),
        (sfb, s32fb, GL_STENCIL_BUFFER_BIT, GL_NEAREST, "stencil between different depth formats"),
        (sfb, s32fb, GL_DEPTH_BUFFER_BIT, GL_NEAREST, "D24S8 depth into D32F_S8"),
        (afb, afb, GL_DEPTH_BUFFER_BIT, GL_NEAREST, "the same depth texture read and written"),
        (sfb, sfb, GL_STENCIL_BUFFER_BIT, GL_NEAREST, "the same stencil read and written"),
        (0, 0, GL_DEPTH_BUFFER_BIT, GL_NEAREST, "the window's depth read and written"),
    ];
    for (read, draw, mask, filter, what) in cases {
        bind(read, draw);
        blit([0, 0, 4, 4], [0, 0, 4, 4], mask, filter);
        assert_eq!(err(), GL_INVALID_OPERATION, "{what}");
    }
    assert_eq!(picture(b, 4, 4), filled(&dst_depth, (4, 4)));
    assert_eq!(picture(c24, 4, 4), filled(&dst_depth, (4, 4)));
    assert!(RESOLVES.lock().unwrap().is_empty(), "nothing was read");
    /* and a format check passes where the formats agree */
    bind(afb, bfb);
    blit([0, 0, 4, 4], [0, 0, 4, 4], GL_DEPTH_BUFFER_BIT, GL_NEAREST);
    assert_eq!(err(), GL_NO_ERROR);
    release(&[afb, bfb, cfb, sfb, s32fb], &[a, b, c24], &[srb, s32rb]);
}

/* ---- the copies still refused (a back-end entry missing) -------------------- */

/// Into a depth renderbuffer or the window, from the window, and any
/// stencil: GL_INVALID_OPERATION, nothing read or written (the log line
/// of each cause is gl/blit.rs's unit test).
#[test]
fn copies_without_a_plane_the_core_holds_are_refused_by_name() {
    let _g = context();
    let (rfb, rrb) = renderbuffer_fbo(GL_DEPTH24_STENCIL8, GL_DEPTH_STENCIL_ATTACHMENT, 4, 4);
    let (r2fb, r2rb) = renderbuffer_fbo(GL_DEPTH24_STENCIL8, GL_DEPTH_STENCIL_ATTACHMENT, 4, 4);
    let tex = depth_texture(D24S8, 4, 4, dst_depth);
    let tfb = texture_fbo(tex, true);
    HELD.lock().unwrap().push(renderbuffer_ds_id(rrb));
    for _ in 0..2 {
        let cases = [
            (rfb, r2fb, GL_DEPTH_BUFFER_BIT, "into a renderbuffer"),
            (tfb, 0, GL_DEPTH_BUFFER_BIT, "into the window"),
            (0, tfb, GL_DEPTH_BUFFER_BIT, "from the window"),
            (rfb, r2fb, GL_STENCIL_BUFFER_BIT, "stencil between renderbuffers"),
            (rfb, tfb, GL_STENCIL_BUFFER_BIT, "stencil into a texture"),
            (rfb, tfb, GL_DEPTH_BUFFER_BIT | GL_STENCIL_BUFFER_BIT, "depth with stencil"),
        ];
        for (read, draw, mask, what) in cases {
            bind(read, draw);
            blit([0, 0, 4, 4], [0, 0, 4, 4], mask, GL_NEAREST);
            assert_eq!(err(), GL_INVALID_OPERATION, "{what}");
        }
    }
    assert_eq!(picture(tex, 4, 4), filled(&dst_depth, (4, 4)));
    assert!(RESOLVES.lock().unwrap().is_empty(), "a refused call reads nothing");
    release(&[rfb, r2fb, tfb], &[tex], &[rrb, r2rb]);
}
