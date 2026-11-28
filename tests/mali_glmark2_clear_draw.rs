// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GL-level host test: glmark2's per-frame clear, viewport and scissor state
//! on the Mali back end (plan #367). Every glmark2 scene starts a frame with
//! `glClearColor` + `glClear(COLOR | DEPTH)`, sets the viewport, and some
//! enable the scissor test; the Mali back end must take each of those without
//! a GL error and be handed the clear the application asked for.
//!
//! Level reached: GL core to the back end's `draw` entry. The host has no
//! Mali OSAL and no shader runs, so no pixel is written. What is checked is
//! what the stand-in back end is handed for each clear (colour, depth, a
//! scissored clear as a quad with its rectangle, the viewport) and that
//! `glGetError` is clean after every step, glReadPixels inside and outside
//! the scissor included. Pixel values are not checked (the stand-in writes
//! none); the board run covers them.
//!
//! Cases: many frames of changing clear colours; a clear under a scissor
//! rectangle; a clear after a viewport change; a draw between two clears
//! with the depth test on. The shaders are written for this test.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_glmark2_clear_draw
//! ```

#![cfg(all(feature = "host-test", feature = "shaderc-online", feature = "gpu-mali", not(feature = "report-es2")))]

use core::ffi::{c_char, c_void};
use std::ffi::CString;
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{self, GlesDraw, GlesHeap, YakoglBackend};
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
use yakogl::mali::backend::{mali_draw, yakogl_backend_mali};
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};

/* ---- a context on the host: the smallest OSAL and back end that make one -- */

unsafe extern "C" fn m_gpu_alloc(_heap: u64, size: usize, _flags: u32, out: *mut OsalGpuMem) -> i32 {
    let p = Box::leak(vec![0u8; size].into_boxed_slice()).as_mut_ptr();
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

/// What one back-end draw call carried: a clear alone has `index_count` 0.
#[derive(Clone, Copy, Debug)]
struct Seen {
    index_count: u32,
    do_clear: bool,
    clear_rgba: [f32; 4],
    clear_quad: bool,
    do_clear_depth: bool,
    clear_depth: f32,
    scissor: bool,
    scissor_rect: [i32; 4],
    viewport: [i32; 4],
    depth_test: bool,
}

static SEEN: Mutex<Vec<Seen>> = Mutex::new(Vec::new());

unsafe extern "C" fn b_draw(d: *const GlesDraw) -> i32 {
    let d = &*d;
    SEEN.lock().unwrap().push(Seen {
        index_count: d.index_count,
        do_clear: d.do_clear,
        clear_rgba: d.clear_rgba,
        clear_quad: d.clear_quad,
        do_clear_depth: d.do_clear_depth,
        clear_depth: d.clear_depth,
        scissor: d.scissor,
        scissor_rect: d.scissor_rect,
        viewport: d.viewport,
        depth_test: d.depth_test,
    });
    if d.index_count == 0 {
        return 0;
    }
    /* the Mali takes the draw and ends at -ENODEV; no shader executes */
    let _ = mali_draw(d);
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

/// The Mali's table with the stand-in's own entries (a draw that records, a
/// heap base, no hardware to start or wait for).
fn stand_in() -> &'static YakoglBackend {
    let m = &yakogl_backend_mali;
    Box::leak(Box::new(YakoglBackend {
        name: m.name,
        native_attrib_formats: m.native_attrib_formats,
        rt_rgba8888_is_bgra: m.rt_rgba8888_is_bgra,
        mip_level_align: m.mip_level_align,
        compressed_blocks_in_rows: m.compressed_blocks_in_rows,
        native_layered_textures: m.native_layered_textures,
        front_facing_shader_variant: m.front_facing_shader_variant,
        etc1_decoded_at_upload: m.etc1_decoded_at_upload,
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
        max_instance_divisor: m.max_instance_divisor,
        wait_kick: None,
        samples_texel_copies: m.samples_texel_copies,
        attachment_gone: None,
        depth_resolve: None,
        query_begin: None,
        query_end: None,
        query_result: None,
        generate_mipmap: m.generate_mipmap,
        return_mipmap_chain: m.return_mipmap_chain,
        linear_row_align: m.linear_row_align,
        uniforms_host_only: m.uniforms_host_only,
        ds_write: None,
        stencil_resolve: None,
        max_samples: m.max_samples,
    }))
}

/// One context for the whole file; the tests take turns on it.
static LOCK: Mutex<()> = Mutex::new(());
static ONCE: Once = Once::new();

fn context() -> MutexGuard<'static, ()> {
    let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    ONCE.call_once(|| {
        // SAFETY: the tables are statics or leaked; the lock above serialises
        // the tests.
        unsafe {
            osal_set_ops(&OSAL);
            backend::g_backend = stand_in();
            yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
            assert!(!yakogl::gl::context::gl_context_create(8, 8).is_null());
        }
    });
    SEEN.lock().unwrap().clear();
    // SAFETY: plain GL call.
    while unsafe { glGetError() } != GL_NO_ERROR {}
    g
}

fn err() -> GLenum {
    // SAFETY: plain GL call.
    unsafe { glGetError() }
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
        let mut log = [0u8; 512];
        let mut n: GLsizei = 0;
        glGetShaderInfoLog(s, log.len() as GLsizei, &mut n, log.as_mut_ptr().cast());
        assert_eq!(ok, 1, "{text:?} did not compile: {}", String::from_utf8_lossy(&log[..n as usize]));
        s
    }
}

const VS: &str = "attribute vec3 a_pos;
attribute vec2 a_uv;
varying mediump vec2 v_uv;
void main()
{
    gl_Position = vec4(a_pos, 1.0);
    v_uv = a_uv;
}
";
const FS: &str = "precision mediump float;
varying mediump vec2 v_uv;
void main()
{
    gl_FragColor = vec4(v_uv, 0.5, 1.0);
}
";

fn program() {
    let (v, f) = (shader(GL_VERTEX_SHADER, VS), shader(GL_FRAGMENT_SHADER, FS));
    // SAFETY: plain GL calls.
    unsafe {
        let p = glCreateProgram();
        glAttachShader(p, v);
        glAttachShader(p, f);
        glBindAttribLocation(p, 0, c"a_pos".as_ptr());
        glBindAttribLocation(p, 1, c"a_uv".as_ptr());
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        assert_eq!(ok, 1, "link failed");
        glUseProgram(p);
    }
}

const W: i32 = 8;

/// An RGBA8 texture and a 16-bit depth renderbuffer as the framebuffer,
/// glmark2's offscreen shape; the colour and depth are both clearable.
fn target() -> (GLuint, GLuint, GLuint) {
    let (mut fbo, mut tex, mut rb) = (0, 0, 0);
    // SAFETY: room for one name each; null texels allocate the image.
    unsafe {
        glGenTextures(1, &mut tex);
        glBindTexture(GL_TEXTURE_2D, tex);
        glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA as GLint, W, W, 0, GL_RGBA, GL_UNSIGNED_BYTE, core::ptr::null());
        glGenRenderbuffers(1, &mut rb);
        glBindRenderbuffer(GL_RENDERBUFFER, rb);
        glRenderbufferStorage(GL_RENDERBUFFER, GL_DEPTH_COMPONENT16, W, W);
        glGenFramebuffers(1, &mut fbo);
        glBindFramebuffer(GL_FRAMEBUFFER, fbo);
        glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex, 0);
        glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_DEPTH_ATTACHMENT, GL_RENDERBUFFER, rb);
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE);
        glViewport(0, 0, W, W);
    }
    assert_eq!(err(), GL_NO_ERROR, "target set-up");
    (fbo, tex, rb)
}

fn teardown(fbo: GLuint, tex: GLuint, rb: GLuint) {
    // SAFETY: names this test made.
    unsafe {
        glDisable(GL_SCISSOR_TEST);
        glDisable(GL_DEPTH_TEST);
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glDeleteFramebuffers(1, &fbo);
        glDeleteTextures(1, &tex);
        glDeleteRenderbuffers(1, &rb);
    }
    assert_eq!(err(), GL_NO_ERROR, "teardown");
}

/// Read one pixel; the host stand-in writes none, so only the call's error
/// is checked (inside and outside the scissor alike).
fn read(x: i32, y: i32) -> GLenum {
    let mut px = [0u8; 4];
    // SAFETY: one pixel into `px`.
    unsafe { glReadPixels(x, y, 1, 1, GL_RGBA, GL_UNSIGNED_BYTE, px.as_mut_ptr().cast()) };
    err()
}

fn finish() {
    // SAFETY: plain GL call.
    unsafe { glFinish() };
}

fn take() -> Vec<Seen> {
    core::mem::take(&mut *SEEN.lock().unwrap())
}

/// Two triangles over the whole target; the quad's depth is z = 0.
static POS: [f32; 18] =
    [-1.0, -1.0, 0.0, 1.0, -1.0, 0.0, -1.0, 1.0, 0.0, -1.0, 1.0, 0.0, 1.0, -1.0, 0.0, 1.0, 1.0, 0.0];
static UV: [f32; 12] = [0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 1.0, 1.0, 0.0, 1.0, 1.0];

fn draw_quad() {
    // SAFETY: the arrays are static.
    unsafe {
        glBindBuffer(GL_ARRAY_BUFFER, 0);
        glVertexAttribPointer(0, 3, GL_FLOAT, 0, 0, POS.as_ptr() as *const c_void);
        glEnableVertexAttribArray(0);
        glVertexAttribPointer(1, 2, GL_FLOAT, 0, 0, UV.as_ptr() as *const c_void);
        glEnableVertexAttribArray(1);
        glDrawArrays(GL_TRIANGLES, 0, 6);
    }
}

fn clear(rgba: [f32; 4], depth: f32) {
    // SAFETY: plain GL calls.
    unsafe {
        glClearColor(rgba[0], rgba[1], rgba[2], rgba[3]);
        glClearDepthf(depth);
        glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT);
    }
}

#[test]
fn many_frames_of_changing_clear_colours() {
    let _g = context();
    let (fbo, tex, rb) = target();
    for frame in 0..40u32 {
        let f = frame as f32;
        let rgba = [(f * 0.025) % 1.0, 1.0 - f * 0.02, ((frame * 7) % 11) as f32 / 10.0, 1.0];
        let depth = if frame % 2 == 0 { 1.0 } else { 0.5 };
        clear(rgba, depth);
        assert_eq!(err(), GL_NO_ERROR, "frame {frame}: glClear");
        finish();
        assert_eq!(err(), GL_NO_ERROR, "frame {frame}: glFinish");
        let seen = take();
        let s = seen.last().unwrap_or_else(|| panic!("frame {frame}: the clear reached the back end"));
        assert_eq!(s.index_count, 0, "frame {frame}: a clear alone");
        assert!(s.do_clear && s.do_clear_depth, "frame {frame}: colour and depth are cleared");
        assert!(!s.clear_quad && !s.scissor, "frame {frame}: a whole-target clear is no quad");
        assert_eq!(s.clear_rgba, rgba, "frame {frame}: the colour asked for");
        assert_eq!(s.clear_depth, depth, "frame {frame}: the depth asked for");
        assert_eq!(read(0, 0), GL_NO_ERROR, "frame {frame}: read");
        assert_eq!(read(W - 1, W - 1), GL_NO_ERROR, "frame {frame}: read");
    }
    teardown(fbo, tex, rb);
}

#[test]
fn clear_under_a_scissor_rectangle() {
    let _g = context();
    let (fbo, tex, rb) = target();
    let rgba = [0.25, 0.5, 0.75, 1.0];
    // SAFETY: plain GL calls.
    unsafe {
        glEnable(GL_SCISSOR_TEST);
        glScissor(2, 3, 4, 2);
    }
    assert_eq!(err(), GL_NO_ERROR, "scissor");
    clear(rgba, 1.0);
    assert_eq!(err(), GL_NO_ERROR, "scissored glClear");
    finish();
    assert_eq!(err(), GL_NO_ERROR, "glFinish");
    let seen = take();
    let s = seen.last().expect("the scissored clear reached the back end");
    assert_eq!(s.index_count, 0);
    assert!(s.clear_quad, "a scissored clear cannot be the background object");
    assert!(s.scissor && s.do_clear && s.do_clear_depth);
    assert_eq!(s.scissor_rect, [2, 3, 4, 2]);
    assert_eq!(s.clear_rgba, rgba);
    /* inside and outside the rectangle */
    for (x, y) in [(2, 3), (5, 4), (3, 3), (0, 0), (7, 7), (1, 3), (6, 4), (2, 5)] {
        assert_eq!(read(x, y), GL_NO_ERROR, "read ({x},{y})");
    }
    /* a scissor that holds the whole target is no scissored clear */
    // SAFETY: plain GL calls.
    unsafe { glScissor(0, 0, W, W) };
    clear([1.0, 0.0, 0.0, 1.0], 1.0);
    finish();
    assert_eq!(err(), GL_NO_ERROR, "full-target scissor clear");
    let seen = take();
    assert!(!seen.last().unwrap().clear_quad, "a box that holds the target is the background object");
    /* an empty box clears nothing and raises no error */
    // SAFETY: plain GL call.
    unsafe { glScissor(3, 3, 0, 0) };
    clear([0.0, 1.0, 0.0, 1.0], 1.0);
    finish();
    assert_eq!(err(), GL_NO_ERROR, "empty scissor clear");
    assert_eq!(read(3, 3), GL_NO_ERROR);
    teardown(fbo, tex, rb);
}

#[test]
fn clear_after_a_viewport_change() {
    let _g = context();
    let (fbo, tex, rb) = target();
    // SAFETY: plain GL calls.
    unsafe { glViewport(2, 2, 4, 4) };
    assert_eq!(err(), GL_NO_ERROR, "glViewport");
    let rgba = [0.5, 0.25, 0.125, 1.0];
    clear(rgba, 1.0);
    finish();
    assert_eq!(err(), GL_NO_ERROR, "clear after the viewport change");
    let seen = take();
    let s = seen.last().expect("the clear reached the back end");
    /* the viewport does not limit a clear: the whole target, no quad */
    assert!(s.do_clear && !s.clear_quad && !s.scissor, "viewport is not a scissor: {s:?}");
    assert_eq!(s.clear_rgba, rgba);
    assert_eq!(read(0, 0), GL_NO_ERROR, "outside the viewport");
    assert_eq!(read(3, 3), GL_NO_ERROR, "inside the viewport");
    /* a draw after it carries the new viewport */
    program();
    draw_quad();
    finish();
    assert_eq!(err(), GL_NO_ERROR, "draw under the new viewport");
    let seen = take();
    let d = seen.iter().rev().find(|s| s.index_count > 0).expect("the draw reached the back end");
    assert_eq!(d.viewport, [2, 2, 4, 4]);
    /* and back to the full one */
    // SAFETY: plain GL call.
    unsafe { glViewport(0, 0, W, W) };
    clear([0.0, 0.0, 0.0, 1.0], 1.0);
    finish();
    assert_eq!(err(), GL_NO_ERROR);
    teardown(fbo, tex, rb);
}

#[test]
fn a_draw_between_clears() {
    let _g = context();
    let (fbo, tex, rb) = target();
    program();
    // SAFETY: plain GL calls.
    unsafe {
        glEnable(GL_DEPTH_TEST);
        glDepthFunc(GL_LESS);
    }
    for frame in 0..8u32 {
        let first = [frame as f32 / 8.0, 0.2, 0.4, 1.0];
        clear(first, 1.0);
        assert_eq!(err(), GL_NO_ERROR, "frame {frame}: first clear");
        draw_quad();
        assert_eq!(err(), GL_NO_ERROR, "frame {frame}: draw");
        /* the clear before the draw went out with it */
        let seen = take();
        assert!(seen.iter().any(|s| s.index_count > 0 && s.depth_test), "frame {frame}: a depth-tested draw");
        let c = seen.iter().find(|s| s.do_clear).unwrap_or_else(|| panic!("frame {frame}: the first clear went out"));
        assert_eq!(c.clear_rgba, first, "frame {frame}: first colour");
        assert!(c.do_clear_depth && c.clear_depth == 1.0, "frame {frame}: depth cleared for the test");
        /* a second clear, a different colour, with nothing drawn after it */
        let second = [0.9, frame as f32 / 8.0, 0.1, 1.0];
        clear(second, 0.25);
        assert_eq!(err(), GL_NO_ERROR, "frame {frame}: second clear");
        finish();
        assert_eq!(err(), GL_NO_ERROR, "frame {frame}: glFinish");
        let seen = take();
        let s =
            seen.iter().rev().find(|s| s.do_clear).unwrap_or_else(|| panic!("frame {frame}: second clear went out"));
        assert_eq!(s.clear_rgba, second, "frame {frame}: second colour");
        assert!(s.do_clear_depth);
        assert_eq!(s.clear_depth, 0.25, "frame {frame}: the depth the second clear asked for");
        assert_eq!(read(4, 4), GL_NO_ERROR, "frame {frame}: read");
        assert_eq!(read(0, 7), GL_NO_ERROR, "frame {frame}: read");
    }
    teardown(fbo, tex, rb);
}
