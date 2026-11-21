// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! ES 3.0 multisample renderbuffers: blits between a framebuffer object and
//! the window, on the host (docs/notes/es3-msaa-renderbuffers.md; ES 3.0.6
//! section 4.3.3 "BlitFramebuffer").
//!
//! The host can model a 4x window: `gl_context_bind_surface` takes the
//! surface's sample count (the call EGL makes for a 4x config), which sets
//! `GlContext::default_samples`. The stand-in back end says `max_samples` 4
//! and, as in tests/es3_msaa_gl.rs, its `finish` stands in for the job-end
//! resolve: it writes the resolved colour into the memory of every 4x target
//! drawn (a 4x window's surface included).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_msaa_gl_window
//! ```

#![cfg(all(feature = "host-test", feature = "shaderc-online", not(feature = "report-es2")))]

use core::ffi::{c_char, c_void};
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

/// The 4x targets drawn since the last `finish`: (`target_gpu`, w, h).
static PENDING: Mutex<Vec<(u64, u32, u32)>> = Mutex::new(Vec::new());

/// The colour the stand-in's job-end resolve leaves in an RGBA8 target.
const RESOLVED: [u8; 4] = [0x40, 0x80, 0xc0, 0xff];

unsafe extern "C" fn b_draw(d: *const GlesDraw) -> i32 {
    let d = &*d;
    if d.samples > 1 {
        PENDING.lock().unwrap().push((d.target_gpu, d.target_w, d.target_h));
    }
    0
}

/// The job-end resolve of the stand-in: every 4x target drawn holds the
/// resolved colour when the jobs are complete, and not before.
unsafe extern "C" fn b_finish_resolve() -> i32 {
    for (gpu, w, h) in core::mem::take(&mut *PENDING.lock().unwrap()) {
        /* the stand-in OSAL's gpu address is the CPU pointer; RGBA8 rows of w*4 */
        let px = core::slice::from_raw_parts_mut(gpu as *mut [u8; 4], (w * h) as usize);
        px.fill(RESOLVED);
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
    name: c"es3-msaa-gl-window".as_ptr() as *const c_char,
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

/// One context for the whole file; the tests take turns on it.
static LOCK: Mutex<()> = Mutex::new(());
static ONCE: Once = Once::new();

/// The context with `table` as the back end.
fn context_on(table: &'static YakoglBackend) -> MutexGuard<'static, ()> {
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
        backend::g_backend = table;
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glFinish();
    }
    PENDING.lock().unwrap().clear();
    // SAFETY: plain GL call.
    while unsafe { glGetError() } != GL_NO_ERROR {}
    g
}

/// The context with a stand-in back end that says `max_samples` 4.
fn context_ms4() -> MutexGuard<'static, ()> {
    context_on(&BACKEND_MS4)
}

fn err() -> GLenum {
    // SAFETY: plain GL call.
    unsafe { glGetError() }
}

fn geti(pname: GLenum) -> GLint {
    let mut v: GLint = -777;
    // SAFETY: one GLint out.
    unsafe { glGetIntegerv(pname, &mut v) };
    assert_eq!(err(), GL_NO_ERROR);
    v
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

/// The window's pixels, as a test sees them in the memory.
fn window_pixels() -> &'static mut [[u8; 4]] {
    let m = window_mem();
    // SAFETY: 64 RGBA8 pixels, leaked.
    unsafe { core::slice::from_raw_parts_mut(m.cpu as *mut [u8; 4], 64) }
}

/// A window of `samples` per pixel bound as the default framebuffer, cleared
/// to zeros; it goes back to one sample when dropped.
struct Window;

impl Window {
    fn new(samples: u8) -> Window {
        window_pixels().fill([0; 4]);
        PENDING.lock().unwrap().clear();
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

/// A bound 8x8 RGBA8 framebuffer object of `samples` (0 or 1: plain).
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

/// Draw one triangle over the bound framebuffer, the job left unfinished.
fn draw_unfinished() {
    static TRI: [f32; 12] = [-1.0, -1.0, 0.0, 1.0, 3.0, -1.0, 0.0, 1.0, -1.0, 3.0, 0.0, 1.0];
    // SAFETY: TRI is static and outlives the draw.
    unsafe {
        glBindBuffer(GL_ARRAY_BUFFER, 0);
        glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, TRI.as_ptr() as *const c_void);
        glEnableVertexAttribArray(0);
        glDrawArrays(GL_TRIANGLES, 0, 3);
    }
    assert_eq!(err(), GL_NO_ERROR);
}

/// A 4x framebuffer object drawn into, its job pending.
fn drawn_4x() -> Fbo {
    let f = fbo(4);
    program();
    draw_unfinished();
    assert_eq!(PENDING.lock().unwrap().len(), 1, "the 4x draw's job is pending");
    f
}

/// glBlitFramebuffer of colour from `read` to `draw_fb` (0 = the window);
/// the GL error.
fn blit(read: GLuint, draw_fb: GLuint, src: [GLint; 4], dst: [GLint; 4]) -> GLenum {
    // SAFETY: plain GL calls on names this test made.
    unsafe {
        glBindFramebuffer(GL_READ_FRAMEBUFFER, read);
        glBindFramebuffer(GL_DRAW_FRAMEBUFFER, draw_fb);
        while glGetError() != GL_NO_ERROR {}
        glBlitFramebuffer(
            src[0],
            src[1],
            src[2],
            src[3],
            dst[0],
            dst[1],
            dst[2],
            dst[3],
            GL_COLOR_BUFFER_BIT,
            GL_NEAREST,
        );
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

const FULL: [GLint; 4] = [0, 0, 8, 8];

/* ---- GL_SAMPLES and GL_SAMPLE_BUFFERS ------------------------------------ */

/// The window's sample count is the surface's; a 1x window has no sample
/// buffer.
#[test]
fn samples_of_the_window() {
    let _g = context_ms4();
    {
        let _w = Window::new(1);
        assert_eq!((geti(GL_SAMPLES), geti(GL_SAMPLE_BUFFERS)), (0, 0), "1x window");
    }
    let _w = Window::new(4);
    assert_eq!((geti(GL_SAMPLES), geti(GL_SAMPLE_BUFFERS)), (4, 1), "4x window");
}

/// The queries follow the bound draw framebuffer: a 4x object is 4 and 1
/// whatever the window is, a plain one 0 and 0 even beside a 4x window.
#[test]
fn samples_follow_the_bound_framebuffer() {
    let _g = context_ms4();
    for win in [1, 4] {
        let _w = Window::new(win);
        let ms = fbo(4);
        assert_eq!((geti(GL_SAMPLES), geti(GL_SAMPLE_BUFFERS)), (4, 1), "4x FBO, {win}x window");
        let single = fbo(0);
        assert_eq!((geti(GL_SAMPLES), geti(GL_SAMPLE_BUFFERS)), (0, 0), "1x FBO, {win}x window");
        // SAFETY: plain GL call.
        unsafe { glBindFramebuffer(GL_FRAMEBUFFER, 0) };
        assert_eq!(geti(GL_SAMPLE_BUFFERS), (win > 1) as GLint, "the window again, {win}x");
        drop((ms, single));
    }
}

/// A draw into the 4x window reaches the back end with 4 samples.
#[test]
fn draw_into_the_window_has_the_windows_samples() {
    let _g = context_ms4();
    let _w = Window::new(4);
    program();
    draw_unfinished();
    assert_eq!(PENDING.lock().unwrap().len(), 1, "a 4x job: the window's");
    // SAFETY: plain GL call.
    unsafe { glFinish() };
}

/* ---- blits between an FBO and the window ----------------------------------- */

/// 4.3.3: the draw framebuffer's sample buffers make a blit
/// GL_INVALID_OPERATION: a 4x FBO to the 4x window, a 1x one, a 4x window to a
/// 4x FBO.
#[test]
fn blit_into_the_4x_window_or_a_4x_fbo_is_invalid_operation() {
    let _g = context_ms4();
    let _w = Window::new(4);
    let ms = fbo(4);
    let single = fbo(0);
    assert_eq!(blit(ms.fb, 0, FULL, FULL), GL_INVALID_OPERATION, "4x FBO to 4x window");
    assert_eq!(blit(single.fb, 0, FULL, FULL), GL_INVALID_OPERATION, "1x FBO to 4x window");
    assert_eq!(blit(0, ms.fb, FULL, FULL), GL_INVALID_OPERATION, "4x window to 4x FBO");
    assert_eq!(blit(0, 0, FULL, FULL), GL_INVALID_OPERATION, "4x window to itself");
    assert!(window_pixels().iter().all(|p| *p == [0; 4]), "nothing copied");
}

/// 4.3.3: a 4x window read into a 1x FBO is a resolve: allowed for the same
/// formats and rectangles, the 4x window's job waited for.
#[test]
fn resolve_the_4x_window_into_a_1x_fbo() {
    let _g = context_ms4();
    let _w = Window::new(4);
    let dst = fbo(0);
    // SAFETY: plain GL call.
    unsafe { glBindFramebuffer(GL_FRAMEBUFFER, 0) };
    program();
    draw_unfinished();
    assert_eq!(PENDING.lock().unwrap().len(), 1);
    assert_eq!(blit(0, dst.fb, FULL, FULL), GL_NO_ERROR);
    assert!(PENDING.lock().unwrap().is_empty(), "the blit waited for the window's job");
    assert!(read(dst.fb).iter().all(|p| *p == RESOLVED), "the resolved colour");
    /* the same errors as any resolve */
    assert_eq!(blit(0, dst.fb, FULL, [0, 0, 4, 4]), GL_INVALID_OPERATION, "scaled");
    assert_eq!(blit(0, dst.fb, [0, 0, 4, 4], [4, 4, 8, 8]), GL_INVALID_OPERATION, "moved");
}

/// A 1x FBO to a 1x window copies, and back (the stand-in draws nothing, so
/// the colour is put in the window's memory).
#[test]
fn blit_a_1x_fbo_to_a_1x_window() {
    let _g = context_ms4();
    let _w = Window::new(1);
    let f = fbo(0);
    window_pixels().fill([1, 2, 3, 4]);
    assert_eq!(blit(0, f.fb, FULL, FULL), GL_NO_ERROR, "1x window to 1x FBO");
    assert!(read(f.fb).iter().all(|p| *p == [1, 2, 3, 4]), "the FBO holds the window's colour");
    window_pixels().fill([0; 4]);
    assert_eq!(blit(f.fb, 0, FULL, FULL), GL_NO_ERROR, "1x FBO to 1x window");
    assert!(window_pixels().iter().all(|p| *p == [1, 2, 3, 4]), "the window holds it again");
}

/// A 4x FBO to a 1x window is a resolve into the window (the format of the
/// window is RGBA8888, the FBO's RGBA8).
#[test]
fn resolve_a_4x_fbo_into_a_1x_window() {
    let _g = context_ms4();
    let _w = Window::new(1);
    let src = drawn_4x();
    assert_eq!(blit(src.fb, 0, FULL, [0, 0, 4, 4]), GL_INVALID_OPERATION, "scaled");
    assert_eq!(blit(src.fb, 0, FULL, FULL), GL_NO_ERROR);
    assert!(PENDING.lock().unwrap().is_empty(), "the blit waited for the 4x job");
    assert!(read(0).iter().all(|p| *p == RESOLVED), "the resolved colour, every pixel");
}
