// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Draws that alternate between a 4x FBO, a 1x FBO and the window, in one
//! context, on the 4x stand-in (docs/notes/es3-msaa-renderbuffers.md (b)4,
//! (b)5, (b)7).
//!
//! Helpers are copied from tests/es3_msaa_gl_draws.rs and
//! tests/es3_msaa_gl_window.rs (tests/es3_msaa_gl.rs says what the stand-in
//! is). Here the stand-in also models the pixels: a draw is a covering
//! triangle, and the colour it leaves in its target is the colour of the
//! clear that came with it (the host cannot run the shader). A 1x target
//! holds it at once; a 4x target holds it only after `finish`, in the memory
//! of that target, as the job-end resolve of a real back end leaves it. A draw
//! that reached the wrong target, or a 4x target that took the colour of a
//! later draw to another one, shows in the pixels.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_msaa_gl_switch
//! ```

#![allow(dead_code)]
#![cfg(all(feature = "host-test", feature = "shaderc-online", not(feature = "report-es2")))]

use core::ffi::{c_char, c_void};
use std::collections::HashMap;
use std::ffi::CString;
use std::sync::atomic::Ordering;
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{self, GlesDraw, GlesHeap, YakoglBackend, GLES_SURF_RGBA8888};
use yakogl::gl::buffer::glBindBuffer;
use yakogl::gl::consts::*;
use yakogl::gl::context::gl_context_bind_surface;
use yakogl::gl::draw::*;
use yakogl::gl::framebuffer::*;
use yakogl::gl::program::*;
use yakogl::gl::shader::*;
use yakogl::gl::state::*;
use yakogl::gl::types::*;
use yakogl::gl::vertex::{glEnableVertexAttribArray, glVertexAttribPointer};
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};

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

/// What the back end was handed by one application draw.
#[derive(Clone, Debug, PartialEq)]
struct Seen {
    samples: u8,
    w: u32,
    h: u32,
    gpu: u64,
    id: u32,
    gen: u32,
}

/// Every application draw (vertices), in order.
static DRAWS: Mutex<Vec<Seen>> = Mutex::new(Vec::new());

/// 4x targets with a colour waiting for the job-end resolve: gpu -> (w, h, rgba).
type Pending = HashMap<u64, (u32, u32, [u8; 4])>;
static PENDING: Mutex<Option<Pending>> = Mutex::new(None);

fn to_u8(c: [f32; 4]) -> [u8; 4] {
    [0, 1, 2, 3].map(|i| (c[i] * 255.0).round() as u8)
}

unsafe extern "C" fn b_draw(d: *const GlesDraw) -> i32 {
    let d = &*d;
    if d.do_clear {
        let rgba = to_u8(d.clear_rgba);
        if d.samples > 1 {
            PENDING
                .lock()
                .unwrap()
                .get_or_insert_with(HashMap::new)
                .insert(d.target_gpu, (d.target_w, d.target_h, rgba));
        } else {
            /* the stand-in OSAL's gpu address is the CPU pointer; RGBA8 rows of w*4 */
            let px = core::slice::from_raw_parts_mut(d.target_gpu as *mut [u8; 4], (d.target_w * d.target_h) as usize);
            px.fill(rgba);
        }
    }
    if d.index_count != 0 {
        DRAWS.lock().unwrap().push(Seen {
            samples: d.samples,
            w: d.target_w,
            h: d.target_h,
            gpu: d.target_gpu,
            id: d.target_id,
            gen: d.target_gen,
        });
    }
    0
}

/// The job-end resolve of the stand-in: every 4x target holds its own colour
/// when the jobs are complete, and not before.
unsafe extern "C" fn b_finish_resolve() -> i32 {
    let p = PENDING.lock().unwrap().take().unwrap_or_default();
    for (gpu, (w, h, rgba)) in p {
        let px = core::slice::from_raw_parts_mut(gpu as *mut [u8; 4], (w * h) as usize);
        px.fill(rgba);
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
    name: c"es3-msaa-gl-switch".as_ptr() as *const c_char,
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

/// [`BACKEND`] with the multisample capability on: it keeps 4 samples.
static BACKEND_MS4: YakoglBackend = YakoglBackend { max_samples: 4, finish: Some(b_finish_resolve), ..BACKEND };

static LOCK: Mutex<()> = Mutex::new(());
static ONCE: Once = Once::new();

/// The context with the 4x stand-in as the back end.
fn context_ms4() -> MutexGuard<'static, ()> {
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
    // SAFETY: a static table; the lock above serialises the tests. Plain GL calls.
    unsafe {
        backend::g_backend = &BACKEND_MS4;
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glFinish();
    }
    DRAWS.lock().unwrap().clear();
    *PENDING.lock().unwrap() = None;
    // SAFETY: plain GL call.
    while unsafe { glGetError() } != GL_NO_ERROR {}
    g
}

fn err() -> GLenum {
    // SAFETY: plain GL call.
    unsafe { glGetError() }
}

/* ---- the window ---------------------------------------------------------- */

/// The memory of the window: 8x8 RGBA8, the same for every sample count.
fn window_mem() -> &'static mut OsalGpuMem {
    static MEM: Mutex<usize> = Mutex::new(0);
    let mut m = MEM.lock().unwrap();
    if *m == 0 {
        let mut mem = OsalGpuMem { cpu: core::ptr::null_mut(), gpu: 0, phys: 0, size: 0, impl_: core::ptr::null_mut() };
        // SAFETY: the stand-in allocator writes `mem`.
        assert_eq!(unsafe { m_gpu_alloc(0, 8 * 8 * 4, 0, &mut mem) }, 0);
        *m = Box::into_raw(Box::new(mem)) as usize;
    }
    // SAFETY: leaked and never freed; the tests take turns (LOCK).
    unsafe { &mut *(*m as *mut OsalGpuMem) }
}

/// A window of `samples` per pixel bound as the default framebuffer; it goes
/// back to one sample when dropped.
struct Window;

impl Window {
    fn new(samples: u8) -> Window {
        // SAFETY: the memory is leaked and holds 8x8 RGBA8888 pixels.
        let r = unsafe { gl_context_bind_surface(window_mem(), 8, 8, GLES_SURF_RGBA8888, samples) };
        assert_eq!(r, 0);
        Window
    }
}

impl Drop for Window {
    fn drop(&mut self) {
        // SAFETY: as in `new`.
        unsafe {
            glFinish();
            gl_context_bind_surface(window_mem(), 8, 8, GLES_SURF_RGBA8888, 1);
        }
    }
}

/* ---- framebuffers, the program, the draw ------------------------------- */

struct Fbo {
    fb: GLuint,
    rb: GLuint,
}

impl Drop for Fbo {
    fn drop(&mut self) {
        // SAFETY: names this test made.
        unsafe {
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
            glDeleteFramebuffers(1, &self.fb);
            glDeleteRenderbuffers(1, &self.rb);
            glUseProgram(0);
        }
    }
}

/// A bound 8x8 RGBA8 framebuffer object of `samples` (0: plain; a request of
/// 1 is rounded up to the supported 4).
fn fbo(samples: GLsizei) -> Fbo {
    let (mut fb, mut rb) = (0, 0);
    // SAFETY: plain GL calls; one name each out.
    unsafe {
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        glGenRenderbuffers(1, &mut rb);
        glBindRenderbuffer(GL_RENDERBUFFER, rb);
        glRenderbufferStorageMultisample(GL_RENDERBUFFER, samples, GL_RGBA8, 8, 8);
        assert_eq!(glGetError(), GL_NO_ERROR);
        glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_RENDERBUFFER, rb);
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE);
    }
    assert_eq!(err(), GL_NO_ERROR);
    Fbo { fb, rb }
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

/// The current program: one float output.
fn program() {
    let v = shader(GL_VERTEX_SHADER, "#version 300 es\nin vec4 p;\nvoid main() { gl_Position = p; }\n");
    let f = shader(
        GL_FRAGMENT_SHADER,
        "#version 300 es\nprecision highp float;\nlayout(location = 0) out vec4 o;\nvoid main() { o = vec4(0.25, 0.5, 0.75, 1.0); }\n",
    );
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

/// Bind `fb` (0 = the window), clear it to `c` and draw one covering
/// triangle, the job left unfinished: what the back end was handed for the
/// draw.
fn draw_on(fb: GLuint, c: [f32; 4]) -> Seen {
    static TRI: [f32; 12] = [-1.0, -1.0, 0.0, 1.0, 3.0, -1.0, 0.0, 1.0, -1.0, 3.0, 0.0, 1.0];
    let before = DRAWS.lock().unwrap().len();
    // SAFETY: TRI is static and outlives the draw; plain GL calls.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        glClearColor(c[0], c[1], c[2], c[3]);
        glClear(GL_COLOR_BUFFER_BIT);
        glBindBuffer(GL_ARRAY_BUFFER, 0);
        glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, TRI.as_ptr() as *const c_void);
        glEnableVertexAttribArray(0);
        glDrawArrays(GL_TRIANGLES, 0, 3);
        glFlush();
    }
    assert_eq!(err(), GL_NO_ERROR);
    let d = DRAWS.lock().unwrap();
    assert_eq!(d.len(), before + 1, "one application draw reached the back end");
    d[before].clone()
}

/// glBlitFramebuffer of colour from `read` to `draw_fb` over the whole 8x8;
/// the GL error.
fn blit(read: GLuint, draw_fb: GLuint) -> GLenum {
    // SAFETY: plain GL calls on names this test made.
    unsafe {
        glBindFramebuffer(GL_READ_FRAMEBUFFER, read);
        glBindFramebuffer(GL_DRAW_FRAMEBUFFER, draw_fb);
        while glGetError() != GL_NO_ERROR {}
        glBlitFramebuffer(0, 0, 8, 8, 0, 0, 8, 8, GL_COLOR_BUFFER_BIT, GL_NEAREST);
    }
    err()
}

/// The 8x8 RGBA8 pixels of `fb` (0 = the window) through glReadPixels.
fn read(fb: GLuint) -> Vec<[u8; 4]> {
    let mut px = vec![[0x11u8; 4]; 64];
    // SAFETY: room for 8x8 RGBA/UNSIGNED_BYTE pixels.
    unsafe {
        glBindFramebuffer(GL_READ_FRAMEBUFFER, fb);
        glReadPixels(0, 0, 8, 8, GL_RGBA, GL_UNSIGNED_BYTE, px.as_mut_ptr() as *mut c_void);
    }
    assert_eq!(err(), GL_NO_ERROR);
    px
}

/* ---- the sequence --------------------------------------------------------- */

// Every channel is a multiple of 0.2, so it survives the round trip to u8.
const MS_A: [f32; 4] = [1.0, 0.2, 0.0, 1.0];
const ONE_A: [f32; 4] = [0.0, 0.4, 1.0, 1.0];
const WIN: [f32; 4] = [0.6, 0.0, 0.8, 1.0];
const MS_B: [f32; 4] = [0.0, 1.0, 0.4, 1.0];
const ONE_B: [f32; 4] = [0.8, 0.8, 0.0, 1.0];

/// 4x, 1x, window, 4x, 1x: each draw sees its own target's samples, size and
/// identity; the 4x FBO resolves to the colour of its last draw and the 1x FBO
/// holds the colour of its own.
fn sequence(window_samples: u8) {
    let _g = context_ms4();
    let _w = Window::new(window_samples);
    program();
    let ms = fbo(4);
    let one = fbo(0);

    let d1 = draw_on(ms.fb, MS_A);
    let d2 = draw_on(one.fb, ONE_A);
    let d3 = draw_on(0, WIN);
    let d4 = draw_on(ms.fb, MS_B);
    let d5 = draw_on(one.fb, ONE_B);

    for (i, d) in [&d1, &d4].into_iter().enumerate() {
        assert_eq!((d.samples, d.w, d.h), (4, 8, 8), "4x draw #{i}");
        assert_ne!(d.id, 0, "4x draw #{i}: the colour renderbuffer's id");
        assert_ne!(d.gen, 0, "4x draw #{i}: a storage generation");
    }
    assert_eq!((d1.id, d1.gen, d1.gpu), (d4.id, d4.gen, d4.gpu), "the 4x target is the same one both times");
    for (i, d) in [&d2, &d5].into_iter().enumerate() {
        assert_eq!((d.samples, d.w, d.h), (1, 8, 8), "1x draw #{i}");
        assert_ne!(d.id, 0, "1x draw #{i}: the colour renderbuffer's id");
    }
    assert_eq!((d2.id, d2.gen, d2.gpu), (d5.id, d5.gen, d5.gpu), "the 1x target is the same one both times");
    assert_ne!(d1.id, d2.id, "4x and 1x are different renderbuffers");
    assert_ne!(d1.gpu, d2.gpu, "4x and 1x are different memory");
    assert_eq!((d3.samples, d3.w, d3.h), (window_samples, 8, 8), "window draw");
    assert_eq!((d3.id, d3.gen), (0, 0), "the window has no renderbuffer identity");
    assert_eq!(d3.gpu, window_mem().gpu, "the window's own memory");

    // The 4x FBO resolves to a fresh 1x FBO: the colour of its last draw.
    let dst = fbo(0);
    assert_eq!(blit(ms.fb, dst.fb), GL_NO_ERROR);
    unsafe { glFinish() };
    let want = to_u8(MS_B);
    assert_eq!(read(dst.fb), vec![want; 64], "the 4x FBO holds the last 4x draw, not another target's");
    assert_ne!(want, to_u8(MS_A));
    // The 1x FBO still holds its own last colour.
    assert_eq!(read(one.fb), vec![to_u8(ONE_B); 64], "the 1x FBO holds its last draw");
    unsafe { glFinish() };
}

#[test]
fn alternating_with_a_1x_window() {
    sequence(1);
}

#[test]
fn alternating_with_a_4x_window() {
    sequence(4);
}
