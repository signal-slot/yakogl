// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! A framebuffer's full pending clear stays pending across a bind to another
//! framebuffer (docs/notes/pvr-extra-kick.md, docs/notes/am62p-slow-terrain.md
//! fix 5): glmark2 clears the window, binds an FBO and draws, and the window's
//! clear went out there as a clear-only kick a frame that the window's own
//! draw would have carried (52 -> 51 kicks a PowerVR terrain frame,
//! tests/pvr_terrain_kicks.rs). The clear now waits on its framebuffer
//! (`draw::park_pending_clear`), and every reader of that framebuffer still
//! sees it:
//!
//! * (a) bind FBO A, clear it fully, bind FBO B, draw, bind A back and read
//!   A: the clear colour, and B's draw went to the back end before A's clear;
//! * (b) the same, then clear A fully again before reading: one clear kick of
//!   A reaches the back end, with the second colour;
//! * (c) the same, then swap the window: the swap carries A's clear out, and
//!   A still reads the clear colour after it;
//! * the window's own clear, kept across an FBO draw, reaches the window's
//!   buffer at the swap, or rides in the window's draw (terrain's frame);
//! * a scissored clear, and a clear of an FBO with a texture attachment,
//!   still go out at the bind as before.
//!
//! The back end is a stand-in that paints: a draw that carries a clear fills
//! its target with the clear colour, a draw with geometry then fills it with
//! `DRAWN`; each call is logged. The GL core is the one of the GPU feature,
//! so the two runs below are the PowerVR and the V3D build of it.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test gl_pending_clear_rebind
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test gl_pending_clear_rebind
//! ```

#![cfg(all(feature = "host-test", feature = "shaderc-online", any(feature = "gpu-powervr", feature = "gpu-v3d")))]

use core::ffi::{c_char, c_void};
use std::sync::{Mutex, MutexGuard, Once};

use yakogl::backend::{self, GlesDraw, YakoglBackend, GLES_HEAP_PROGRAM, GLES_HEAP_SHADER};
use yakogl::gl::consts::*;
use yakogl::gl::draw::glDrawArrays;
use yakogl::gl::egl::*;
use yakogl::gl::egl_consts::*;
use yakogl::gl::framebuffer::*;
use yakogl::gl::program::*;
use yakogl::gl::shader::*;
use yakogl::gl::state::*;
use yakogl::gl::texture::*;
use yakogl::gl::types::*;
use yakogl::gl::vertex::*;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};

/* ---- the stand-in platform ------------------------------------------------------ */

/// One call of the back end's `draw`.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Rec {
    target: u64,
    /// The draw carries a colour clear (fused, or a clear-only kick).
    clear: Option<[u8; 4]>,
    /// The draw has geometry (a clear-only kick has none).
    geometry: bool,
}

struct Log {
    draws: Vec<Rec>,
}

static LOG: Mutex<Log> = Mutex::new(Log { draws: Vec::new() });

fn log() -> MutexGuard<'static, Log> {
    LOG.lock().unwrap_or_else(|e| e.into_inner())
}

/// What a geometry draw paints (its whole target, the stand-in ignores the
/// shader).
const DRAWN: [u8; 4] = [10, 200, 30, 255];

fn unorm8(c: [f32; 4]) -> [u8; 4] {
    c.map(|v| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8)
}

unsafe extern "C" fn m_init(_arg: *mut c_void) -> i32 {
    0
}

unsafe extern "C" fn m_gpu_alloc(_heap: u64, size: usize, _flags: u32, out: *mut OsalGpuMem) -> i32 {
    if out.is_null() || size == 0 {
        return -22;
    }
    let layout = std::alloc::Layout::from_size_align(size, 4096).expect("a layout");
    // SAFETY: a non-zero size; kept for the process (a few small buffers).
    let p = unsafe { std::alloc::alloc_zeroed(layout) };
    assert!(!p.is_null(), "host allocation of {size} bytes");
    // SAFETY: `out` is a valid out-parameter of the library's call.
    unsafe { *out = OsalGpuMem { cpu: p.cast(), gpu: p as u64, phys: p as u64, size, impl_: p.cast() } };
    0
}

unsafe extern "C" fn m_gpu_free(_mem: *mut OsalGpuMem) -> i32 {
    0
}

unsafe extern "C" fn m_map_phys(_heap: u64, phys: u64, size: usize, _flags: u32, out: *mut OsalGpuMem) -> i32 {
    // SAFETY: `out` is a valid out-parameter of the library's call.
    unsafe { *out = OsalGpuMem { cpu: core::ptr::null_mut(), gpu: phys, phys, size, impl_: core::ptr::null_mut() } };
    0
}

unsafe extern "C" fn m_gpu_rw(_m: *mut OsalGpuMem, _off: usize, _p: *mut c_void, _n: usize) -> i32 {
    0
}

unsafe extern "C" fn m_gpu_write(m: *mut OsalGpuMem, off: usize, src: *const c_void, n: usize) -> i32 {
    // SAFETY: forwarded as received.
    unsafe { m_gpu_rw(m, off, src.cast_mut(), n) }
}

unsafe extern "C" fn m_cache(_cpu: *mut c_void, _n: usize) {}

/// The display: a window needs one; what it shows is not under test (the
/// swap's present may show an earlier frame's buffer).
unsafe extern "C" fn m_present(_mem: *const OsalGpuMem, _w: u32, _h: u32, _stride: u32, _format: u32) -> i32 {
    0
}

static OSAL: OsalOps = OsalOps {
    init: Some(m_init),
    shutdown: None,
    gpu_alloc: Some(m_gpu_alloc),
    gpu_free: Some(m_gpu_free),
    mmu_map: None,
    map_phys: Some(m_map_phys),
    gpu_write: Some(m_gpu_write),
    gpu_read: Some(m_gpu_rw),
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
    present: Some(m_present),
    lock: None,
    unlock: None,
    thread_id: None,
    display_busy: None,
};

unsafe extern "C" fn b_init() -> i32 {
    0
}

unsafe extern "C" fn b_heap_base(heap: u32) -> u64 {
    match heap {
        GLES_HEAP_SHADER => 0x1000_0000,
        GLES_HEAP_PROGRAM => 0x2000_0000,
        _ => 0x4000_0000,
    }
}

unsafe extern "C" fn b_ok() -> i32 {
    0
}

/// The stand-in GPU: logs the draw, then paints its 32-bit target (the
/// clear colour as RGBA bytes over the whole target for a colour clear,
/// then `DRAWN` for a draw with geometry).
unsafe extern "C" fn b_draw(d: *const GlesDraw) -> i32 {
    // SAFETY: the core passes a valid draw.
    let d = unsafe { &*d };
    let clear = (d.do_clear && d.color_write).then(|| unorm8(d.clear_rgba));
    let geometry = d.index_count != 0;
    log().draws.push(Rec { target: d.target_gpu, clear, geometry });
    if d.target_gpu == 0 {
        return 0;
    }
    let pitch = backend::draw_pitch(d) as usize;
    for px in [clear, geometry.then_some(DRAWN)].into_iter().flatten() {
        for y in 0..d.target_h as usize {
            for x in 0..d.target_w as usize {
                let p = (d.target_gpu as *mut u8).wrapping_add((y * pitch + x) * 4);
                // SAFETY: the target holds `target_h` rows of `pitch` pixels.
                unsafe { core::ptr::copy_nonoverlapping(px.as_ptr(), p, 4) };
            }
        }
    }
    0
}

static BACKEND: YakoglBackend = YakoglBackend {
    name: c"gl_pending_clear_rebind".as_ptr() as *const c_char,
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
    finish_previous: Some(b_ok),
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

/* ---- the context: an EGL window, two framebuffer objects, a program -------------- */

const W: u32 = 8;
const H: u32 = 4;
/// The framebuffer objects' size.
const N: GLsizei = 4;

const RED: [f32; 4] = [1.0, 0.0, 0.0, 1.0];
const GREEN: [f32; 4] = [0.0, 1.0, 0.0, 1.0];
const BLUE: [f32; 4] = [0.0, 0.0, 1.0, 1.0];

struct Rig {
    dpy: EGLDisplay,
    surface: EGLSurface,
    prog: GLuint,
}

// SAFETY: the handles are used under SERIAL alone.
unsafe impl Send for Rig {}

static SERIAL: Mutex<Option<Rig>> = Mutex::new(None);
static ONCE: Once = Once::new();

fn err() -> GLenum {
    // SAFETY: plain GL call.
    unsafe { glGetError() }
}

fn shader(type_: GLenum, src: &str) -> GLuint {
    let src = std::ffi::CString::new(src).unwrap();
    // SAFETY: plain GL calls; `src` is NUL-terminated.
    unsafe {
        let s = glCreateShader(type_);
        glShaderSource(s, 1, &src.as_ptr(), core::ptr::null());
        glCompileShader(s);
        let mut ok = 0;
        glGetShaderiv(s, GL_COMPILE_STATUS, &mut ok);
        assert_eq!(ok, 1, "compile");
        s
    }
}

/// The context, current, with nothing pending and the log empty.
fn rig() -> MutexGuard<'static, Option<Rig>> {
    let mut g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    ONCE.call_once(|| {
        #[rustfmt::skip]
        let cfg_list = [
            EGL_RED_SIZE, 8, EGL_GREEN_SIZE, 8, EGL_BLUE_SIZE, 8,
            EGL_SURFACE_TYPE, EGL_WINDOW_BIT, EGL_RENDERABLE_TYPE, EGL_OPENGL_ES2_BIT, EGL_NONE,
        ];
        let ctx_list = [EGL_CONTEXT_CLIENT_VERSION, 2, EGL_NONE];
        let pixels = Box::leak(vec![0u8; (W * H * 4) as usize + 4096].into_boxed_slice());
        let phys = (pixels.as_mut_ptr() as u64 + 4095) & !4095;
        let win = Box::leak(Box::new(GlesNativeWindow {
            phys,
            width: W,
            height: H,
            stride_bytes: W * 4,
            format: GLES_NATIVE_FORMAT_XRGB8888,
        }));
        // SAFETY: the tables are statics, the window leaked; every pointer
        // passed is live for its call.
        unsafe {
            osal_set_ops(&OSAL);
            backend::g_backend = &BACKEND;
            yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
            let dpy = eglGetDisplay(core::ptr::null_mut());
            assert_eq!(eglInitialize(dpy, core::ptr::null_mut(), core::ptr::null_mut()), 1);
            let mut cfg = core::ptr::null_mut();
            let mut n = 0;
            assert_eq!(eglChooseConfig(dpy, cfg_list.as_ptr(), &mut cfg, 1, &mut n), 1);
            let surface = eglCreateWindowSurface(dpy, cfg, win, core::ptr::null());
            assert!(!surface.is_null(), "eglCreateWindowSurface: {:#x}", eglGetError());
            let c = eglCreateContext(dpy, cfg, core::ptr::null_mut(), ctx_list.as_ptr());
            assert!(!c.is_null());
            assert_eq!(eglMakeCurrent(dpy, surface, surface, c), 1);
            let (v, f) = (
                shader(GL_VERTEX_SHADER, "attribute vec4 p; void main() { gl_Position = p; }"),
                shader(GL_FRAGMENT_SHADER, "precision mediump float; void main() { gl_FragColor = vec4(1.0); }"),
            );
            let prog = glCreateProgram();
            glAttachShader(prog, v);
            glAttachShader(prog, f);
            glBindAttribLocation(prog, 0, c"p".as_ptr());
            glLinkProgram(prog);
            let mut ok = 0;
            glGetProgramiv(prog, GL_LINK_STATUS, &mut ok);
            assert_eq!(ok, 1, "link");
            *g = Some(Rig { dpy, surface, prog });
        }
    });
    // SAFETY: plain GL calls.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glDisable(GL_SCISSOR_TEST);
        glFinish();
    }
    while err() != GL_NO_ERROR {}
    log().draws.clear();
    g
}

/// A framebuffer object with an RGBA8 colour renderbuffer (a texture when
/// `texture`) and a depth renderbuffer, N x N, and its colour image's
/// address (the stand-in's target).
struct Fbo {
    fb: GLuint,
    color: GLuint,
    depth: GLuint,
    texture: bool,
}

impl Fbo {
    fn new(texture: bool) -> Fbo {
        let mut f = Fbo { fb: 0, color: 0, depth: 0, texture };
        // SAFETY: room for one name each; a null image.
        unsafe {
            if texture {
                glGenTextures(1, &mut f.color);
                glBindTexture(GL_TEXTURE_2D, f.color);
                let null = core::ptr::null();
                glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA as GLint, N, N, 0, GL_RGBA, GL_UNSIGNED_BYTE, null);
                glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST as GLint);
                glBindTexture(GL_TEXTURE_2D, 0);
            } else {
                glGenRenderbuffers(1, &mut f.color);
                glBindRenderbuffer(GL_RENDERBUFFER, f.color);
                glRenderbufferStorage(GL_RENDERBUFFER, GL_RGBA8_OES, N, N);
            }
            glGenRenderbuffers(1, &mut f.depth);
            glBindRenderbuffer(GL_RENDERBUFFER, f.depth);
            glRenderbufferStorage(GL_RENDERBUFFER, GL_DEPTH_COMPONENT16, N, N);
            glGenFramebuffers(1, &mut f.fb);
            glBindFramebuffer(GL_FRAMEBUFFER, f.fb);
            if texture {
                glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, f.color, 0);
            } else {
                glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_RENDERBUFFER, f.color);
            }
            glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_DEPTH_ATTACHMENT, GL_RENDERBUFFER, f.depth);
            assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE);
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
        }
        assert_eq!(err(), GL_NO_ERROR, "framebuffer setup");
        f
    }

    fn bind(&self) {
        // SAFETY: plain GL calls on a name this test made.
        unsafe {
            glBindFramebuffer(GL_FRAMEBUFFER, self.fb);
            glViewport(0, 0, N, N);
        }
    }

    /// The distinct pixels of the bound framebuffer (glReadPixels, N x N).
    fn read(&self) -> Vec<[u8; 4]> {
        self.bind();
        let mut px = vec![0x77u8; (N * N) as usize * 4];
        // SAFETY: room for N x N RGBA8 pixels.
        unsafe { glReadPixels(0, 0, N, N, GL_RGBA, GL_UNSIGNED_BYTE, px.as_mut_ptr() as *mut c_void) };
        assert_eq!(err(), GL_NO_ERROR, "glReadPixels");
        let mut v: Vec<[u8; 4]> = px.chunks(4).map(|c| [c[0], c[1], c[2], c[3]]).collect();
        v.sort();
        v.dedup();
        v
    }
}

impl Drop for Fbo {
    fn drop(&mut self) {
        // SAFETY: names this test made.
        unsafe {
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
            glDeleteFramebuffers(1, &self.fb);
            glDeleteRenderbuffers(1, &self.depth);
            if self.texture {
                glDeleteTextures(1, &self.color);
            } else {
                glDeleteRenderbuffers(1, &self.color);
            }
        }
    }
}

/// glClear(COLOR | DEPTH) of the bound framebuffer to `c`: every
/// attachment, no scissor.
fn clear(c: [f32; 4]) {
    // SAFETY: plain GL calls.
    unsafe {
        glClearColor(c[0], c[1], c[2], c[3]);
        glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT);
    }
    assert_eq!(err(), GL_NO_ERROR, "glClear");
}

static TRI: [f32; 12] = [-1.0, -1.0, 0.0, 1.0, 3.0, -1.0, 0.0, 1.0, -1.0, 3.0, 0.0, 1.0];

fn draw(rig: &Rig) {
    // SAFETY: plain GL calls; the array is a static.
    unsafe {
        glUseProgram(rig.prog);
        glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, TRI.as_ptr() as *const c_void);
        glEnableVertexAttribArray(0);
        glDrawArrays(GL_TRIANGLES, 0, 3);
    }
    assert_eq!(err(), GL_NO_ERROR, "glDrawArrays");
}

fn swap(rig: &Rig) {
    // SAFETY: the rig's live display and surface.
    assert_eq!(unsafe { eglSwapBuffers(rig.dpy, rig.surface) }, 1, "eglSwapBuffers: {:#x}", unsafe { eglGetError() });
}

/// The draws logged so far.
fn draws() -> Vec<Rec> {
    log().draws.clone()
}

/// The target address of the draws into the bound framebuffer: the target
/// of the last draw logged (the caller has just drawn into it).
fn last_target() -> u64 {
    draws().last().expect("a draw").target
}

/// The clears that reached the back end for `target`.
fn clears_of(target: u64) -> Vec<Rec> {
    draws().into_iter().filter(|r| r.target == target && r.clear.is_some()).collect()
}

/// A's colour image's address: one draw into it (and a read, which settles
/// it), then the log is emptied.
fn target_of(rig: &Rig, f: &Fbo) -> u64 {
    f.bind();
    draw(rig);
    let t = last_target();
    f.read();
    log().draws.clear();
    t
}

/// The part every row below starts with: bind A, clear it fully to RED,
/// bind B, draw. Returns (A's target, B's target).
fn clear_a_draw_b(rig: &Rig, a: &Fbo, b: &Fbo) -> (u64, u64) {
    let (ta, tb) = (target_of(rig, a), target_of(rig, b));
    a.bind();
    clear(RED);
    b.bind();
    draw(rig);
    (ta, tb)
}

/* ---- the rows ----------------------------------------------------------------------- */

/// (a) A's full clear does not go out when B is bound: B's draw reaches the
/// back end first, and reading A afterwards gives the clear colour, through
/// one clear-only kick of A.
#[test]
fn a_read_after_the_rebind_sees_the_clear() {
    let g = rig();
    let rig = g.as_ref().unwrap();
    let (a, b) = (Fbo::new(false), Fbo::new(false));
    let (ta, tb) = clear_a_draw_b(rig, &a, &b);
    assert_eq!(
        draws(),
        [Rec { target: tb, clear: None, geometry: true }],
        "B's draw alone: A's clear waits on A (no clear-only kick at the bind)"
    );
    assert_eq!(a.read(), [unorm8(RED)], "A reads its clear");
    assert_eq!(clears_of(ta), [Rec { target: ta, clear: Some(unorm8(RED)), geometry: false }]);
    assert_eq!(b.read(), [DRAWN], "B holds its draw");
}

/// (b) A's second full clear before any draw of A replaces the first: one
/// clear kick of A reaches the back end, with the second colour.
#[test]
fn b_a_second_full_clear_replaces_the_kept_one() {
    let g = rig();
    let rig = g.as_ref().unwrap();
    let (a, b) = (Fbo::new(false), Fbo::new(false));
    let (ta, _) = clear_a_draw_b(rig, &a, &b);
    a.bind();
    clear(GREEN);
    assert_eq!(a.read(), [unorm8(GREEN)]);
    assert_eq!(
        clears_of(ta),
        [Rec { target: ta, clear: Some(unorm8(GREEN)), geometry: false }],
        "one clear kick of A, the second clear's"
    );
}

/// (b) with a draw of A: the second clear rides in A's draw, no clear-only
/// kick of A at all.
#[test]
fn b_a_draw_after_the_second_clear_carries_it() {
    let g = rig();
    let rig = g.as_ref().unwrap();
    let (a, b) = (Fbo::new(false), Fbo::new(false));
    let (ta, _) = clear_a_draw_b(rig, &a, &b);
    a.bind();
    clear(GREEN);
    draw(rig);
    assert_eq!(clears_of(ta), [Rec { target: ta, clear: Some(unorm8(GREEN)), geometry: true }]);
    assert_eq!(a.read(), [DRAWN]);
}

/// (c) A swap of the window with A's clear kept: the swap carries it out
/// (one clear kick of A), and A reads the clear colour after the swap.
#[test]
fn c_a_swap_carries_the_kept_clear_out() {
    let g = rig();
    let rig = g.as_ref().unwrap();
    let (a, b) = (Fbo::new(false), Fbo::new(false));
    let (ta, _) = clear_a_draw_b(rig, &a, &b);
    a.bind();
    // SAFETY: plain GL call.
    unsafe { glBindFramebuffer(GL_FRAMEBUFFER, 0) };
    swap(rig);
    assert_eq!(clears_of(ta), [Rec { target: ta, clear: Some(unorm8(RED)), geometry: false }], "at the swap");
    assert_eq!(a.read(), [unorm8(RED)], "A after the swap");
    assert_eq!(clears_of(ta).len(), 1, "the read found nothing left to emit");
}

/// The window's clear kept across an FBO draw reaches the frame's buffer at
/// the swap: one clear-only kick of the window.
#[test]
fn the_window_clear_kept_across_an_fbo_draw_goes_out_at_the_swap() {
    let g = rig();
    let rig = g.as_ref().unwrap();
    let b = Fbo::new(false);
    let tb = target_of(rig, &b);
    // SAFETY: plain GL calls.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glViewport(0, 0, W as GLint, H as GLint);
    }
    clear(BLUE);
    b.bind();
    draw(rig);
    assert_eq!(draws(), [Rec { target: tb, clear: None, geometry: true }], "the window's clear waits");
    swap(rig);
    let window: Vec<Rec> = draws().into_iter().filter(|r| r.target != tb).collect();
    assert_eq!(window.len(), 1, "one kick of the window: {window:?}");
    assert_eq!(window[0].clear, Some(unorm8(BLUE)));
    assert!(!window[0].geometry);
    // SAFETY: the stand-in's window buffers are host memory of W x H pixels.
    let px = unsafe { core::slice::from_raw_parts(window[0].target as *const [u8; 4], (W * H) as usize) };
    assert!(px.iter().all(|p| *p == unorm8(BLUE)), "the frame's buffer: {px:?}");
}

/// glmark2 terrain's shape: clear the window, draw into an FBO, bind the
/// window and draw. The window's clear rides in the window's draw; no
/// clear-only kick of the window (the kick docs/notes/pvr-extra-kick.md
/// removes).
#[test]
fn the_window_draw_carries_the_kept_window_clear() {
    let g = rig();
    let rig = g.as_ref().unwrap();
    let b = Fbo::new(false);
    let tb = target_of(rig, &b);
    for _ in 0..2 {
        log().draws.clear();
        // SAFETY: plain GL calls.
        unsafe {
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
            glViewport(0, 0, W as GLint, H as GLint);
        }
        clear(BLUE);
        b.bind();
        clear(GREEN);
        draw(rig);
        // SAFETY: plain GL calls.
        unsafe {
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
            glViewport(0, 0, W as GLint, H as GLint);
        }
        draw(rig);
        let tw = last_target();
        swap(rig);
        assert_eq!(
            draws(),
            [
                Rec { target: tb, clear: Some(unorm8(GREEN)), geometry: true },
                Rec { target: tw, clear: Some(unorm8(BLUE)), geometry: true },
            ],
            "two kicks: B's draw with its clear, the window's draw with the window's"
        );
    }
}

/// A scissored clear is not a full clear: it goes out when B is bound and
/// drawn, before B's draw, as before this change.
#[test]
fn a_scissored_clear_goes_out_at_the_rebind() {
    let g = rig();
    let rig = g.as_ref().unwrap();
    let (a, b) = (Fbo::new(false), Fbo::new(false));
    let (ta, tb) = (target_of(rig, &a), target_of(rig, &b));
    a.bind();
    // SAFETY: plain GL calls.
    unsafe {
        glEnable(GL_SCISSOR_TEST);
        glScissor(0, 0, 2, 2);
    }
    clear(RED);
    // SAFETY: plain GL call.
    unsafe { glDisable(GL_SCISSOR_TEST) };
    b.bind();
    draw(rig);
    let d = draws();
    assert_eq!(d.len(), 2, "{d:?}");
    assert_eq!((d[0].target, d[0].geometry), (ta, false), "A's clear first: {d:?}");
    assert_eq!(d[1], Rec { target: tb, clear: None, geometry: true });
}

/// A full clear of an FBO with a texture attachment goes out at the rebind
/// as before: the draws in between may sample the texture.
#[test]
fn a_texture_target_clear_goes_out_at_the_rebind() {
    let g = rig();
    let rig = g.as_ref().unwrap();
    let (a, b) = (Fbo::new(true), Fbo::new(false));
    let (ta, tb) = clear_a_draw_b(rig, &a, &b);
    assert_eq!(
        draws(),
        [
            Rec { target: ta, clear: Some(unorm8(RED)), geometry: false },
            Rec { target: tb, clear: None, geometry: true },
        ]
    );
    assert_eq!(a.read(), [unorm8(RED)]);
}

/// Two framebuffer objects sharing A's colour renderbuffer: B's draw into
/// it must come after A's clear, so binding B emits the kept clear.
#[test]
fn a_shared_renderbuffer_takes_the_kept_clear_out_first() {
    let g = rig();
    let rig = g.as_ref().unwrap();
    let (a, b) = (Fbo::new(false), Fbo::new(false));
    let ta = target_of(rig, &a);
    a.bind();
    clear(RED);
    b.bind();
    // SAFETY: plain GL calls on names this test made.
    unsafe { glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_RENDERBUFFER, a.color) };
    draw(rig);
    let d = draws();
    assert_eq!(
        d,
        [
            Rec { target: ta, clear: Some(unorm8(RED)), geometry: false },
            Rec { target: ta, clear: None, geometry: true }
        ],
        "A's clear, then B's draw into the shared image"
    );
    assert_eq!(a.read(), [DRAWN], "the draw is on top of the clear");
    // SAFETY: plain GL calls: give B its own image back before the drop.
    unsafe { glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_RENDERBUFFER, b.color) };
}
