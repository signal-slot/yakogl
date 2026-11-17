// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! glBlitFramebuffer depth and stencil to and from the window (the default
//! framebuffer) through the GL API over the GC7000UL (Vivante) host back end,
//! item window-test-gc. The window path of the GL core (`DepthSource::Window`,
//! `PlaneDst::Window`; docs/notes/blit-depth-window.md, "Design") is pinned
//! on a stand-in back end in tests/es3_blit_ds_window.rs; the renderbuffer
//! blits over this table in tests/es3_blit_ds_gc.rs (the rig here is its rig).
//!
//! The Vivante table has `depth_resolve` (`gc_depth_resolve`) but
//! `ds_write: None` and `stencil_resolve: None` (src/vivante/backend.rs).
//! What src/gl/blit.rs does with the window at this base, per blit:
//!
//! - stencil, either way: REFUSED_STENCIL (a stencil copy needs both entries
//!   and renderbuffers on both sides).
//! - depth into the window, or into a renderbuffer (no `ds_write`): the
//!   destination cannot be written, REFUSED_DEPTH_DST.
//! - depth from the window into a depth *texture* (the GL core holds the
//!   texels itself, so no `ds_write` is needed): `depth_source` reads
//!   textures and renderbuffers only, so the window is REFUSED_DEPTH_SRC.
//!   After the window lift this blit needs only `depth_resolve`, which the
//!   table has; its twin below waits for the GL core, not for a table entry.
//! - window to window: GL_INVALID_OPERATION by ES 3.0.6 4.3.3 (the same
//!   buffer is read and written), whatever the entries; stays after the lift.
//! - colour only to the window: unaffected, no error.
//!
//! Every refusal is exactly one GL_INVALID_OPERATION, nothing copied, the
//! planes the back end holds as they were, and the context usable afterwards.
//! The stencil plane cannot be read on this back end, so refusals do not
//! check it.
//!
//! `#[ignore]`d twins, for the lift (the `ds_write` and `stencil_resolve`
//! entries in src/vivante/backend.rs, and the window path in src/gl/blit.rs):
//! the same shapes with the expected pixels of tests/es3_blit_ds_window.rs's
//! model, the window stored top-down (`default_y_up` false: memory row r is
//! GL row `WIN - 1 - r`) and y-up, read back through the table's entries.
//! Removing the `#[ignore]` is all the lift needs.
//!
//! HOST-ONLY: API verdicts on the host back end, never pictures and not a
//! board result (tools/cts/README.md 2b).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online --test es3_blit_ds_window_gc
//! ```

#![cfg(all(feature = "host-test", feature = "gpu-vivante", feature = "shaderc-online", not(feature = "report-es2")))]

use core::ffi::c_void;
use std::collections::BTreeMap;
use std::sync::Mutex;
use yakogl::gl::consts::*;
use yakogl::gl::context;
use yakogl::gl::framebuffer::*;
use yakogl::gl::names::{self, Kind};
use yakogl::gl::state::*;
use yakogl::gl::texture::{glBindTexture, glDeleteTextures, glGenTextures, glTexImage2D};
use yakogl::gl::types::*;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};
use yakogl::vivante::backend::{gc_depth_resolve, yakogl_backend_vivante};
use yakogl::vivante::kick::{GcKick, GC_KICK_MAGIC};

/// The online compiler's store is one static for the process: every test
/// takes this lock.
static STORE: Mutex<()> = Mutex::new(());

/* ---- the stand-in OSAL: host memory, every kick accepted ------------------- */

#[derive(Default)]
struct Platform {
    allocs: BTreeMap<u64, Box<[u64]>>,
    bump: BTreeMap<u64, u64>,
    fence: i64,
}

static PLATFORM: Mutex<Option<Platform>> = Mutex::new(None);

fn with_platform<R>(f: impl FnOnce(&mut Platform) -> R) -> R {
    let mut g = PLATFORM.lock().unwrap_or_else(|e| e.into_inner());
    f(g.get_or_insert_with(Platform::default))
}

unsafe extern "C" fn p_gpu_alloc(heap_base: u64, size: usize, _flags: u32, out: *mut OsalGpuMem) -> i32 {
    if size == 0 || size > 1 << 30 {
        return -12;
    }
    with_platform(|p| {
        let mut mem = vec![0u64; size.div_ceil(8)].into_boxed_slice();
        let at = p.bump.entry(heap_base).or_insert(0);
        let gpu = heap_base + 0x1000 + *at;
        *at += (size as u64).div_ceil(4096) * 4096;
        let cpu = mem.as_mut_ptr() as *mut c_void;
        // SAFETY: the caller passes a writable descriptor.
        unsafe { *out = OsalGpuMem { cpu, gpu, phys: gpu, size, impl_: cpu } };
        p.allocs.insert(gpu, mem);
        0
    })
}

unsafe extern "C" fn p_gpu_free(mem: *mut OsalGpuMem) -> i32 {
    // SAFETY: the caller passes a live descriptor; it is cleared (the OSAL contract).
    unsafe {
        let m = *mem;
        with_platform(|p| p.allocs.remove(&m.gpu));
        *mem = OsalGpuMem::ZERO;
    }
    0
}

unsafe extern "C" fn p_cache(_cpu: *mut c_void, _n: usize) {}

unsafe extern "C" fn p_submit(kick: *const c_void, len: usize) -> i64 {
    if len != core::mem::size_of::<GcKick>() {
        return -22;
    }
    // SAFETY: the back end passes a live GcKick of that size.
    let k = unsafe { *(kick as *const GcKick) };
    with_platform(|p| {
        if k.magic != GC_KICK_MAGIC || !p.allocs.contains_key(&k.stream_gpu) {
            return -22;
        }
        p.fence += 1;
        p.fence
    })
}

unsafe extern "C" fn p_wait(_fence: i64, _timeout_us: i64) -> i32 {
    0
}

unsafe extern "C" fn p_completed() -> i64 {
    with_platform(|p| p.fence)
}

unsafe extern "C" fn p_query(what: u32, out: *mut c_void, out_sz: usize) -> i32 {
    use yakogl::osal::{OSAL_QUERY_DRAW_STATS, OSAL_QUERY_GC_FRAME_OPEN};
    if (what == OSAL_QUERY_GC_FRAME_OPEN && out_sz == 4 && !out.is_null()) || what == OSAL_QUERY_DRAW_STATS {
        return 0;
    }
    -22
}

static OSAL: OsalOps = OsalOps {
    init: None,
    shutdown: None,
    gpu_alloc: Some(p_gpu_alloc),
    gpu_free: Some(p_gpu_free),
    mmu_map: None,
    map_phys: None,
    gpu_write: None,
    gpu_read: None,
    cache_flush: Some(p_cache),
    cache_invd: Some(p_cache),
    submit: Some(p_submit),
    wait: Some(p_wait),
    completed: Some(p_completed),
    reg_read: None,
    reg_write: None,
    query: Some(p_query),
    log: None,
    wait_vblank: None,
    present: None,
    lock: None,
    unlock: None,
    thread_id: None,
    display_busy: None,
};

/// Run `case` with the Vivante back end up on the stand-in OSAL and a fresh
/// 64x64 context; the lock serialises the tests.
fn on_gc7000(case: impl FnOnce()) {
    let _g = STORE.lock().unwrap_or_else(|e| e.into_inner());
    // SAFETY: the guard serialises the tests; the tables are statics.
    let ctx = unsafe {
        osal_set_ops(&OSAL);
        yakogl::backend::g_backend = &yakogl_backend_vivante;
        yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
        assert_eq!(yakogl::vivante::backend::gc_init(), 0, "gc_init");
        let c = yakogl::gl::context::gl_context_create(64, 64);
        assert!(!c.is_null(), "context pool exhausted");
        c
    };
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(case));
    // SAFETY: the context made above.
    unsafe {
        yakogl::gl::context::gl_context_destroy(ctx);
        yakogl::vivante::backend::gc_shutdown();
    }
    with_platform(|p| {
        /* leaked, not freed, as tests/es3_mrt4_gc.rs */
        for (_, m) in core::mem::take(&mut p.allocs) {
            core::mem::forget(m);
        }
    });
    if let Err(e) = r {
        std::panic::resume_unwind(e);
    }
}

/* ---- the window and the sides ---------------------------------------------- */

/// The window's side: `gl_context_create(64, 64)` in `on_gc7000`.
const WIN: i32 = 64;
const DEPTH: GLbitfield = GL_DEPTH_BUFFER_BIT;
const STENCIL: GLbitfield = GL_STENCIL_BUFFER_BIT;
/// Clears: (depth, stencil) of the window, of the blit source and of a
/// destination renderbuffer. Constants, so a blit shows only as which pixels
/// took the source's value (`model` below).
const WIN_CLEAR: (f32, i32) = (0.5, 0x33);
const SRC_CLEAR: (f32, i32) = (0.25, 0x5a);
const DST_CLEAR: (f32, i32) = (0.75, 0x11);

fn err() -> GLenum {
    // SAFETY: a plain GL call.
    unsafe { glGetError() }
}

fn drain() {
    while err() != GL_NO_ERROR {}
}

/// Store the window bottom-up (`true`) or top-down (`false`).
fn window_y_up(up: bool) {
    let c = context::current().expect("a current context");
    assert_eq!((c.target_w, c.target_h), (WIN as u32, WIN as u32));
    c.default_y_up = up;
}

/// The id the back end keeps the window's depth and stencil under (the
/// formula of gl/draw.rs).
fn window_id() -> u32 {
    let c = context::current().expect("a current context");
    assert!(c.default_surface_valid, "the context has a window");
    0x8000_0000 | (c.default_surface.gpu >> 12) as u32
}

/// The memory row of the window's GL row `g` (the mirror is an involution).
fn mem_row(g: u32, y_up: bool) -> u32 {
    if y_up {
        g
    } else {
        WIN as u32 - 1 - g
    }
}

/// Clear the window's depth, stencil and colour to `WIN_CLEAR`.
fn clear_window() {
    // SAFETY: plain GL calls.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glDisable(GL_SCISSOR_TEST);
        glClearDepthf(WIN_CLEAR.0);
        glClearStencil(WIN_CLEAR.1);
        glClear(DEPTH | STENCIL | GL_COLOR_BUFFER_BIT);
        glFinish();
    }
    assert_eq!(err(), GL_NO_ERROR, "clearing the window");
}

/// A `w` x `h` D24S8 renderbuffer at GL_DEPTH_STENCIL_ATTACHMENT of a new
/// framebuffer, cleared to `clear`: (framebuffer, renderbuffer).
fn ds_fbo(w: i32, h: i32, clear: (f32, i32)) -> (GLuint, GLuint) {
    let (mut fb, mut rb) = (0, 0);
    // SAFETY: room for one name each; plain GL calls.
    unsafe {
        glGenRenderbuffers(1, &mut rb);
        glBindRenderbuffer(GL_RENDERBUFFER, rb);
        glRenderbufferStorage(GL_RENDERBUFFER, GL_DEPTH24_STENCIL8, w, h);
        assert_eq!(glGetError(), GL_NO_ERROR, "storage");
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_DEPTH_STENCIL_ATTACHMENT, GL_RENDERBUFFER, rb);
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE);
        glClearDepthf(clear.0);
        glClearStencil(clear.1);
        glClear(DEPTH | STENCIL);
        glFinish();
    }
    assert_eq!(err(), GL_NO_ERROR, "clearing the renderbuffer");
    (fb, rb)
}

/// A `w` x `h` D24S8 *texture* at GL_DEPTH_STENCIL_ATTACHMENT of a new
/// framebuffer, depth cleared to `depth`: (framebuffer, texture).
fn tex_fbo(w: i32, h: i32, depth: f32) -> (GLuint, GLuint) {
    let (mut fb, mut tex) = (0, 0);
    // SAFETY: room for one name each; no pixels.
    unsafe {
        glGenTextures(1, &mut tex);
        glBindTexture(GL_TEXTURE_2D, tex);
        glTexImage2D(
            GL_TEXTURE_2D,
            0,
            GL_DEPTH24_STENCIL8 as GLint,
            w,
            h,
            0,
            GL_DEPTH_STENCIL,
            GL_UNSIGNED_INT_24_8,
            core::ptr::null(),
        );
        assert_eq!(glGetError(), GL_NO_ERROR, "texture storage");
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        glFramebufferTexture2D(GL_FRAMEBUFFER, GL_DEPTH_STENCIL_ATTACHMENT, GL_TEXTURE_2D, tex, 0);
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE);
        glClearDepthf(depth);
        glClear(DEPTH);
        glFinish();
    }
    assert_eq!(err(), GL_NO_ERROR, "clearing the texture");
    (fb, tex)
}

/// A `w` x `h` RGBA8 renderbuffer in a new framebuffer, cleared to red.
fn colour_fbo(w: i32, h: i32) -> (GLuint, GLuint) {
    let (mut fb, mut rb) = (0, 0);
    // SAFETY: room for one name each; plain GL calls.
    unsafe {
        glGenRenderbuffers(1, &mut rb);
        glBindRenderbuffer(GL_RENDERBUFFER, rb);
        glRenderbufferStorage(GL_RENDERBUFFER, GL_RGBA8, w, h);
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_RENDERBUFFER, rb);
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE);
        glClearColor(1.0, 0.0, 0.0, 1.0);
        glClear(GL_COLOR_BUFFER_BIT);
        glFinish();
    }
    assert_eq!(err(), GL_NO_ERROR, "clearing the colour renderbuffer");
    (fb, rb)
}

fn ds_id(rb: GLuint) -> u32 {
    let c = context::current().expect("a current context");
    let slot = names::slot_of(c, Kind::Renderbuffer, rb).expect("a renderbuffer name");
    context::ds_object_id(c.renderbuffer_object(slot).expect("a renderbuffer").serial, false)
}

fn bind(read: GLuint, draw: GLuint) {
    // SAFETY: plain GL calls.
    unsafe {
        glBindFramebuffer(GL_READ_FRAMEBUFFER, read);
        glBindFramebuffer(GL_DRAW_FRAMEBUFFER, draw);
    }
}

fn blit(s: [i32; 4], d: [i32; 4], mask: GLbitfield) {
    drain();
    // SAFETY: a plain GL call.
    unsafe { glBlitFramebuffer(s[0], s[1], s[2], s[3], d[0], d[1], d[2], d[3], mask, GL_NEAREST) };
}

fn with_scissor(scissor: Option<[i32; 4]>, f: impl FnOnce()) {
    // SAFETY: plain GL calls.
    unsafe {
        if let Some([x, y, w, h]) = scissor {
            glEnable(GL_SCISSOR_TEST);
            glScissor(x, y, w, h);
        }
        f();
        glDisable(GL_SCISSOR_TEST);
    }
}

fn release(fbs: &[GLuint], texs: &[GLuint], rbs: &[GLuint]) {
    // SAFETY: names the test made.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glDeleteFramebuffers(fbs.len() as i32, fbs.as_ptr());
        glDeleteTextures(texs.len() as i32, texs.as_ptr());
        glDeleteRenderbuffers(rbs.len() as i32, rbs.as_ptr());
    }
}

/// Depth plane `id` (`w` x `h`, memory order) through the table's
/// `depth_resolve`; None when the back end holds nothing under that id.
fn try_depth(id: u32, w: i32, h: i32) -> Option<Vec<f32>> {
    let mut v = vec![-1.0f32; (w * h) as usize];
    // SAFETY: `h` rows of `w` floats.
    let r = unsafe { gc_depth_resolve(id, v.as_mut_ptr(), w as u32, h as u32, w as u32) };
    (r == 0).then_some(v)
}

fn read_depth(id: u32, w: i32, h: i32) -> Vec<f32> {
    try_depth(id, w, h).expect("the back end holds the depth plane")
}

/// Stencil plane `id` through the table's `stencil_resolve` (the entry the
/// lift adds; `None` today).
fn read_stencil(id: u32, w: i32, h: i32) -> Vec<u8> {
    let f = yakogl_backend_vivante.stencil_resolve.expect("the GC table has a stencil_resolve entry");
    let mut v = vec![0xffu8; (w * h) as usize];
    // SAFETY: `h` rows of `w` bytes.
    let r = unsafe { f(id, v.as_mut_ptr(), w as u32, h as u32, w as u32) };
    assert_eq!(r, 0, "the back end holds the stencil plane");
    v
}

/// A window plane (memory order) as GL rows.
fn window_gl_rows<T: Copy>(mem: &[T], y_up: bool) -> Vec<T> {
    (0..WIN as u32)
        .flat_map(|g| (0..WIN as u32).map(move |x| (x, g)))
        .map(|(x, g)| mem[(mem_row(g, y_up) * WIN as u32 + x) as usize])
        .collect()
}

/// The depth texels of level 0 of depth texture `tex`, GL row order, as the
/// GL core holds them (R32F).
fn texture_depth(tex: GLuint, w: i32, h: i32) -> Vec<f32> {
    let c = context::current().expect("a current context");
    let slot = names::slot_of(c, Kind::Texture, tex).expect("a texture name");
    let t = c.texture_object(slot).expect("a texture object");
    (0..h)
        .flat_map(|y| (0..w).map(move |x| (x, y)))
        .map(|(x, y)| {
            let off = t.level_off[0] as usize + (y as usize * t.level_pitch_at(0) as usize + x as usize) * 4;
            assert!(off + 4 <= t.mem.size);
            // SAFETY: inside the texture's host allocation (checked above).
            unsafe { ((t.mem.cpu as *const u8).add(off) as *const f32).read() }
        })
        .collect()
}

/* ---- the model (tests/es3_blit_ds_window.rs) -------------------------------- */

/// The source texel destination pixel `o` takes on one axis, or None when
/// its sample point is outside the source cut to `[0, n)`.
fn model_axis(o: i64, s0: i64, s1: i64, d0: i64, d1: i64, n: i64) -> Option<i64> {
    let p = s0 as f64 + (o as f64 + 0.5 - d0 as f64) * (s1 - s0) as f64 / (d1 - d0) as f64;
    let (lo, hi) = (s0.min(s1).max(0), s0.max(s1).min(n));
    let t = p.floor() as i64;
    (p >= lo as f64 && p < hi as f64).then_some(t)
}

/// The destination (GL row order, `dw` x `dh`) after the blit `s` -> `d` of
/// a `sw` x `sh` source whose every texel is `src`, into a destination that
/// held `keep` everywhere, with the scissor box `scissor` (None: off).
fn model(
    src: f32,
    (sw, sh): (u32, u32),
    keep: f32,
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
            let hit = in_dst
                && in_scissor
                && model_axis(xi, s[0] as i64, s[2] as i64, d[0] as i64, d[2] as i64, sw as i64).is_some()
                && model_axis(yi, s[1] as i64, s[3] as i64, d[1] as i64, d[3] as i64, sh as i64).is_some();
            out.push(if hit { src } else { keep });
        }
    }
    out
}

/// One blit: the source rectangle, the destination rectangle, the scissor.
type Blit = ([i32; 4], [i32; 4], Option<[i32; 4]>);

/// The blits between a `sw` x `sh` image and the window: whole, scaled and
/// cut by the window's edge, flipped under a scissor box, and a
/// sub-rectangle. Every sample point of the destination rectangle cut to its
/// image is inside the source.
fn cases(sw: i32, sh: i32) -> Vec<Blit> {
    vec![
        ([0, 0, sw, sh], [0, 0, WIN, WIN], None),
        ([0, 0, sw, sh], [WIN - 6, WIN - 10, 12, 20], None),
        ([sw, 0, 0, sh], [0, 0, WIN, WIN], Some([11, 17, 20, 9])),
        ([1, 2, sw - 1, sh - 2], [5, 9, 30, 40], None),
    ]
}

/* ---- (a) refused today ------------------------------------------------------- */

/// What is on the other side of the blit from the window.
#[derive(Clone, Copy, Debug)]
enum Other {
    Renderbuffer,
    Texture,
}

/// One refusal: blit `mask` between the window and `other` (32x32),
/// `to_window` choosing the direction, for each of `cases`, the window
/// top-down and y-up. Exactly one GL_INVALID_OPERATION, the planes the back
/// end holds as they were, and a later clear lands.
///
/// Why each shape is refused is in the header; the expectation is the same
/// for all of them: refused whole, nothing changes.
fn check_refused(mask: GLbitfield, other: Other, to_window: bool) {
    on_gc7000(|| {
        let wid = window_id();
        for y_up in [false, true] {
            window_y_up(y_up);
            clear_window();
            let (fb, rb, tex) = match other {
                Other::Renderbuffer => {
                    let (fb, rb) = ds_fbo(32, 32, if to_window { SRC_CLEAR } else { DST_CLEAR });
                    (fb, rb, 0)
                }
                Other::Texture => {
                    let (fb, tex) = tex_fbo(32, 32, if to_window { SRC_CLEAR.0 } else { DST_CLEAR.0 });
                    (fb, 0, tex)
                }
            };
            let win_before = try_depth(wid, WIN, WIN);
            let other_before = (rb != 0).then(|| read_depth(ds_id(rb), 32, 32));
            for (s, d, scissor) in cases(32, 32) {
                /* from the window: the 32x32 rectangle is the destination */
                let (s, d) = if to_window { (s, d) } else { (d, s) };
                if to_window {
                    bind(fb, 0);
                } else {
                    bind(0, fb);
                }
                with_scissor(scissor, || blit(s, d, mask));
                let what = format!("{other:?}, window dst {to_window}, mask {mask:#x}, y_up {y_up}: {s:?} -> {d:?}");
                assert_eq!(err(), GL_INVALID_OPERATION, "{what}: the blit is refused");
                assert_eq!(err(), GL_NO_ERROR, "{what}: exactly one error");
                assert_eq!(try_depth(wid, WIN, WIN), win_before, "{what}: the window depth is untouched");
                if let Some(before) = &other_before {
                    assert_eq!(&read_depth(ds_id(rb), 32, 32), before, "{what}: the renderbuffer depth is untouched");
                }
            }
            /* usable afterwards: a clear of the window lands (depth when readable) */
            clear_window();
            assert_eq!(err(), GL_NO_ERROR, "the context is usable after the refusals");
            if let Some(w) = try_depth(wid, WIN, WIN) {
                assert!(w.iter().all(|&d| d == WIN_CLEAR.0), "a later clear of the window lands");
            }
            release(&[fb], &[tex], &[rb]);
        }
    });
}

/// Depth from a renderbuffer into the window: no `ds_write` to write the
/// window with (REFUSED_DEPTH_DST).
#[test]
fn depth_from_a_renderbuffer_into_the_window_is_refused_today() {
    check_refused(DEPTH, Other::Renderbuffer, true);
}

/// Depth from a depth texture into the window: the same (REFUSED_DEPTH_DST).
#[test]
fn depth_from_a_texture_into_the_window_is_refused_today() {
    check_refused(DEPTH, Other::Texture, true);
}

/// Depth from the window into a renderbuffer: no `ds_write` for the
/// renderbuffer (REFUSED_DEPTH_DST), and the GL core does not read the
/// window at this base either.
#[test]
fn depth_from_the_window_into_a_renderbuffer_is_refused_today() {
    check_refused(DEPTH, Other::Renderbuffer, false);
}

/// Depth from the window into a depth texture: the destination could be
/// written (the GL core holds texels itself), the source cannot be read at
/// this base (REFUSED_DEPTH_SRC).
#[test]
fn depth_from_the_window_into_a_texture_is_refused_today() {
    check_refused(DEPTH, Other::Texture, false);
}

/// Stencil into the window from a renderbuffer (REFUSED_STENCIL).
#[test]
fn stencil_into_the_window_is_refused_today() {
    check_refused(STENCIL, Other::Renderbuffer, true);
}

/// Stencil from the window into a renderbuffer (REFUSED_STENCIL).
#[test]
fn stencil_from_the_window_is_refused_today() {
    check_refused(STENCIL, Other::Renderbuffer, false);
}

/// Depth and stencil together in either direction: refused whole.
#[test]
fn depth_and_stencil_with_the_window_are_refused_today() {
    check_refused(DEPTH | STENCIL, Other::Renderbuffer, true);
    check_refused(DEPTH | STENCIL, Other::Renderbuffer, false);
}

/// The window to the window is GL_INVALID_OPERATION by ES 3.0.6 4.3.3 (the
/// same buffer is read and written), for each of depth and stencil, whatever
/// the entries the table has. Must stay after the lift.
#[test]
fn the_window_to_the_window_is_an_error() {
    on_gc7000(|| {
        for y_up in [false, true] {
            window_y_up(y_up);
            clear_window();
            for mask in [DEPTH, STENCIL, DEPTH | STENCIL] {
                bind(0, 0);
                blit([0, 0, 32, 32], [32, 32, 64, 64], mask);
                assert_eq!(err(), GL_INVALID_OPERATION, "y_up {y_up}, mask {mask:#x}");
            }
            if let Some(w) = try_depth(window_id(), WIN, WIN) {
                assert!(w.iter().all(|&d| d == WIN_CLEAR.0), "the window depth is untouched");
            }
        }
    });
}

/// A colour-only blit into the window is not touched by any of this: no
/// error, and the window keeps the depth it had.
#[test]
fn a_colour_only_blit_to_the_window_works() {
    on_gc7000(|| {
        let wid = window_id();
        for y_up in [false, true] {
            window_y_up(y_up);
            clear_window();
            let before = try_depth(wid, WIN, WIN);
            let (fb, rb) = colour_fbo(32, 32);
            bind(fb, 0);
            blit([0, 0, 32, 32], [0, 0, WIN, WIN], GL_COLOR_BUFFER_BIT);
            assert_eq!(err(), GL_NO_ERROR, "y_up {y_up}: colour into the window");
            assert_eq!(try_depth(wid, WIN, WIN), before, "y_up {y_up}: the window depth is untouched");
            release(&[fb], &[], &[rb]);
        }
    });
}

/// The table entries the refusals rest on; the line the lift changes.
#[test]
fn the_table_lacks_ds_write_and_stencil_resolve_today() {
    assert!(yakogl_backend_vivante.ds_write.is_none(), "ds_write (src/vivante/backend.rs)");
    assert!(yakogl_backend_vivante.stencil_resolve.is_none(), "stencil_resolve (src/vivante/backend.rs)");
    assert!(yakogl_backend_vivante.depth_resolve.is_some(), "depth_resolve stays");
}

/* ---- (b) the lift: expected pixels from the model -------------------------- */

fn assert_depth_eq(what: &str, got: &[f32], want: &[f32]) {
    for (i, (g, e)) in got.iter().zip(want).enumerate() {
        assert!((g - e).abs() <= 0.5 / 16_777_215.0, "{what}: depth pixel {i}: {g} want {e}");
    }
}

fn as_bytes(v: Vec<f32>) -> Vec<u8> {
    v.into_iter().map(|b| b as u8).collect()
}

/// Blit `mask` from a 32x32 D24S8 renderbuffer (`SRC_CLEAR`) or depth
/// texture into the window (`WIN_CLEAR`), each of `cases`, the window
/// top-down and y-up: the window's plane read through the table (memory
/// order, mirrored back to GL rows) is the model's picture. Depth is read
/// through `depth_resolve`, stencil through `stencil_resolve`.
fn check_into_window(mask: GLbitfield, other: Other) {
    on_gc7000(|| {
        let wid = window_id();
        let (fb, rb, tex) = match other {
            Other::Renderbuffer => {
                let (fb, rb) = ds_fbo(32, 32, SRC_CLEAR);
                (fb, rb, 0)
            }
            Other::Texture => {
                let (fb, tex) = tex_fbo(32, 32, SRC_CLEAR.0);
                (fb, 0, tex)
            }
        };
        for y_up in [false, true] {
            window_y_up(y_up);
            for (s, d, scissor) in cases(32, 32) {
                clear_window();
                bind(fb, 0);
                with_scissor(scissor, || blit(s, d, mask));
                let what = format!("{other:?}, y_up {y_up}: {s:?} -> {d:?} scissor {scissor:?}");
                assert_eq!(err(), GL_NO_ERROR, "{what}");
                let wh = (WIN as u32, WIN as u32);
                if mask & DEPTH != 0 {
                    let want = model(SRC_CLEAR.0, (32, 32), WIN_CLEAR.0, wh, s, d, scissor);
                    assert_depth_eq(&what, &window_gl_rows(&read_depth(wid, WIN, WIN), y_up), &want);
                }
                if mask & STENCIL != 0 {
                    let want = as_bytes(model(SRC_CLEAR.1 as f32, (32, 32), WIN_CLEAR.1 as f32, wh, s, d, scissor));
                    assert_eq!(window_gl_rows(&read_stencil(wid, WIN, WIN), y_up), want, "{what}: stencil");
                }
            }
        }
        release(&[fb], &[tex], &[rb]);
    });
}

#[test]
#[ignore = "needs GC7000UL ds_write/stencil_resolve"]
fn depth_from_a_renderbuffer_into_the_window_reads_back_the_source() {
    check_into_window(DEPTH, Other::Renderbuffer);
}

#[test]
#[ignore = "needs GC7000UL ds_write/stencil_resolve"]
fn depth_from_a_texture_into_the_window_reads_back_the_source() {
    check_into_window(DEPTH, Other::Texture);
}

#[test]
#[ignore = "needs GC7000UL ds_write/stencil_resolve"]
fn stencil_from_a_renderbuffer_into_the_window_reads_back_the_source() {
    check_into_window(STENCIL, Other::Renderbuffer);
}

#[test]
#[ignore = "needs GC7000UL ds_write/stencil_resolve"]
fn depth_and_stencil_from_a_renderbuffer_into_the_window_read_back_the_source() {
    check_into_window(DEPTH | STENCIL, Other::Renderbuffer);
}

/// Blit `mask` from the window (`WIN_CLEAR`) into a 64x64 D24S8 renderbuffer
/// or depth texture (`DST_CLEAR`), each of `cases`, the window top-down and
/// y-up: the destination (never mirrored) is the model's picture of the
/// window in GL rows.
fn check_from_window(mask: GLbitfield, other: Other) {
    on_gc7000(|| {
        let (fb, rb, tex) = match other {
            Other::Renderbuffer => {
                let (fb, rb) = ds_fbo(WIN, WIN, DST_CLEAR);
                (fb, rb, 0)
            }
            Other::Texture => {
                let (fb, tex) = tex_fbo(WIN, WIN, DST_CLEAR.0);
                (fb, 0, tex)
            }
        };
        for y_up in [false, true] {
            window_y_up(y_up);
            for (s, d, scissor) in cases(WIN, WIN) {
                clear_window();
                // SAFETY: plain GL calls on live names.
                unsafe {
                    glBindFramebuffer(GL_FRAMEBUFFER, fb);
                    glClearDepthf(DST_CLEAR.0);
                    glClearStencil(DST_CLEAR.1);
                    glClear(DEPTH | STENCIL);
                }
                bind(0, fb);
                with_scissor(scissor, || blit(s, d, mask));
                let what = format!("{other:?}, y_up {y_up}: {s:?} -> {d:?} scissor {scissor:?}");
                assert_eq!(err(), GL_NO_ERROR, "{what}");
                let wh = (WIN as u32, WIN as u32);
                if mask & DEPTH != 0 {
                    let want = model(WIN_CLEAR.0, wh, DST_CLEAR.0, wh, s, d, scissor);
                    let got = match other {
                        Other::Renderbuffer => read_depth(ds_id(rb), WIN, WIN),
                        Other::Texture => texture_depth(tex, WIN, WIN),
                    };
                    assert_depth_eq(&what, &got, &want);
                }
                if mask & STENCIL != 0 {
                    let want = as_bytes(model(WIN_CLEAR.1 as f32, wh, DST_CLEAR.1 as f32, wh, s, d, scissor));
                    assert_eq!(read_stencil(ds_id(rb), WIN, WIN), want, "{what}: stencil");
                }
            }
        }
        release(&[fb], &[tex], &[rb]);
    });
}

#[test]
#[ignore = "needs GC7000UL ds_write/stencil_resolve"]
fn depth_from_the_window_into_a_renderbuffer_reads_back_the_window() {
    check_from_window(DEPTH, Other::Renderbuffer);
}

/// Needs only the window path of the GL core (`depth_resolve` exists, the
/// texture is written by the GL core), but the same lift.
#[test]
#[ignore = "needs GC7000UL ds_write/stencil_resolve"]
fn depth_from_the_window_into_a_texture_reads_back_the_window() {
    check_from_window(DEPTH, Other::Texture);
}

#[test]
#[ignore = "needs GC7000UL ds_write/stencil_resolve"]
fn stencil_from_the_window_into_a_renderbuffer_reads_back_the_window() {
    check_from_window(STENCIL, Other::Renderbuffer);
}

#[test]
#[ignore = "needs GC7000UL ds_write/stencil_resolve"]
fn depth_and_stencil_from_the_window_into_a_renderbuffer_read_back_the_window() {
    check_from_window(DEPTH | STENCIL, Other::Renderbuffer);
}
