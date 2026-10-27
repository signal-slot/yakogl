// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Why the Pi 4's `glmark2-frame:` line says "end waited for in N of N" in
//! terrain, shadow, refract, desktop blur and desktop shadow under the EGL
//! double buffer, whose swap waits only for the frame before
//! (docs/notes/egl-swap-timeline.md, "Why the Pi 4 still reads 'end waited
//! for in N of N'").
//!
//! What the counter counts. After each `eglSwapBuffers` the glmark2 sample
//! (`driver/samples/zephyr/glmark2/src/main.cpp`, `GLStateTimed::swap`)
//! reads the LAST job that completed (`osal_v3d_last_completed_job_us`) and
//! counts the frame as "end waited for" when that job's `render_late` is 0:
//! the OSAL found the render still running and waited for its end
//! (`src/osal/zephyr_v3d.c` `finish_inflight`). It names no call and no
//! frame: any wait of the CPU for a running job counts, wherever it was
//! taken.
//!
//! The run here: the real GL core and the real `eglSwapBuffers` (a window
//! EGL double-buffers, as glmark2's), on a stand-in back end that waits
//! where `src/v3d/backend.rs` waits, with a clock:
//!
//! * one job in flight: a submit first waits for the job in flight
//!   (`flush_job` -> `wait_in_flight`, the OSAL runs one job at a time);
//! * `finish` submits the open job and waits for it (`v3d_finish`);
//!   `finish_previous` waits for nothing (`v3d_finish_previous`: at most the
//!   last job runs already);
//! * `kick_pending` asks the fence and completes a job that is over without
//!   a wait, as `z_completed` does: such an end is seen late;
//! * `generate_mipmap` submits the job rendering into the target, waits for
//!   the job in flight and runs the TFU (`v3d_generate_mipmap`);
//! * a draw that samples a render target reads its render image: no wait
//!   (`tex_state_for`, `rt_sample`).
//!
//! The GL core's depth resolve flushes and finishes the depth pass itself
//! (`src/gl/texture.rs` `resolve_depth`) before `depth_resolve`.
//!
//! The clock: each job has a GPU time and runs after the one before it; the
//! CPU's time moves only where the run says (`cpu`), by a wait, by the
//! depth resolve's detile and by a TFU job (the OSAL polls it). A wait for a
//! job whose end is past the CPU's time is "waited for" (`render_late` 0),
//! one for a job already over is "seen late". The times are host model
//! times, not the board's: what the test pins is where the waits fall,
//! which does not depend on them as long as the scene's passes take longer
//! on the GPU than the CPU takes to record the next one.
//!
//! What it pins, per scene shape (one frame each, `FRAMES` frames):
//!
//! * single pass, CPU-bound: 0 of N. The swap's flush finds the frame
//!   before over. The double buffer gives the overlap.
//! * single pass, GPU-bound: N of N, every one taken in `eglSwapBuffers`
//!   for the frame before: the designed wait.
//! * shadow (depth pass, then a window pass sampling the depth texture):
//!   N of N, taken in the ground's `glDrawArrays` (the depth resolve waits
//!   for the depth pass of the same frame).
//! * terrain (a pass into a mipmapped target, `glGenerateMipmap`, a window
//!   pass sampling it): N of N, taken in `glGenerateMipmap` for the pass
//!   of the same frame.
//! * refract (a depth pass, a colour pass into a mipmapped target,
//!   `glGenerateMipmap`, a window pass sampling both): N of N, taken in
//!   `glGenerateMipmap` for the colour pass of the same frame.
//! * desktop blur (passes ping-ponging between two render targets, then
//!   the window pass) and desktop shadow (one pass into the desktop, then
//!   the window pass): N of N, taken in `eglSwapBuffers`, in the back end's
//!   flush, which waits for the job in flight (the frame's own last
//!   off-screen pass) before it submits the window's job.
//! * in every shape: no swap waits for the window job it submits (its
//!   frame's end), and no swap calls `finish`.
//!
//! The run also shows what the one job in flight costs the overlap: in
//! every multi-pass shape the window job of frame N is waited for at frame
//! N+1's first off-screen submit, so frame N+1's CPU work beside it is only
//! what comes before that submit.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test egl_swap_end_wait -- --nocapture
//! ```

#![cfg(all(feature = "host-test", feature = "shaderc-online"))]

use core::ffi::{c_char, c_void};
use std::ffi::CString;
use std::sync::Mutex;

use yakogl::backend::{self, GlesDraw, GlesHeap, GlesTextureBind, YakoglBackend};
use yakogl::gl::buffer::*;
use yakogl::gl::consts::*;
use yakogl::gl::draw::*;
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

/// One test at a time: the EGL state, the OSAL table and the rig are
/// process-wide.
static SERIAL: Mutex<()> = Mutex::new(());

/* ---- the stand-in V3D, with a clock --------------------------------------------- */

/// One job handed to the GPU.
#[derive(Clone, Debug)]
struct Job {
    kick: u32,
    /// The frame whose GL calls recorded it, and the pass.
    frame: u32,
    pass: &'static str,
    gpu_us: u64,
    /// When the GPU ends it: after its submit and after the job before.
    end_us: u64,
}

/// How a job's completion was seen.
#[derive(Clone, Debug, PartialEq)]
struct Done {
    kick: u32,
    /// `render_late` 0: the CPU waited for the render's end.
    waited: bool,
    /// The call the CPU was in (`at`).
    call: &'static str,
}

#[derive(Default)]
struct Rig {
    now_us: u64,
    /// The GL call the run is in (`at`), and the frame and pass it records.
    call: &'static str,
    frame: u32,
    pass: &'static str,
    /// The GPU time a job opened now takes (`pass`).
    pass_gpu_us: u64,
    /// The depth resolve's CPU time (the detile).
    resolve_us: u64,
    /// One TFU job's time; the OSAL polls it, so it is the CPU's too.
    tfu_us: u64,
    /// The open job: (target, depth id), its pass and GPU time.
    open: Option<((u64, u32), &'static str, u64)>,
    in_flight: Option<Job>,
    /// The GPU's end of the last job submitted (the next one runs after it).
    gpu_free_us: u64,
    kicks: u32,
    jobs: Vec<Job>,
    done: Vec<Done>,
    /// Calls of the back end's entry points that wait or might.
    finishes: Vec<&'static str>,
    mipmaps_on_gpu: u32,
    resolves: u32,
}

static RIG: Mutex<Option<Rig>> = Mutex::new(None);

fn rig<R>(f: impl FnOnce(&mut Rig) -> R) -> R {
    f(RIG.lock().unwrap_or_else(|e| e.into_inner()).as_mut().expect("the rig is installed"))
}

impl Rig {
    /// The job in flight is over: completed, waited for when its end is
    /// still ahead of the CPU (`finish_inflight`'s `render_late`).
    fn complete_in_flight(&mut self) {
        if let Some(j) = self.in_flight.take() {
            let waited = j.end_us > self.now_us;
            self.now_us = self.now_us.max(j.end_us);
            self.done.push(Done { kick: j.kick, waited, call: self.call });
        }
    }

    /// `flush_job`: the job in flight first, then the open one submitted.
    fn flush_job(&mut self) {
        let Some((_, pass, gpu_us)) = self.open.take() else { return };
        self.complete_in_flight();
        self.kicks += 1;
        let end_us = self.now_us.max(self.gpu_free_us) + gpu_us;
        self.gpu_free_us = end_us;
        let job = Job { kick: self.kicks, frame: self.frame, pass, gpu_us, end_us };
        self.jobs.push(job.clone());
        self.in_flight = Some(job);
    }

    /// `fence_done` through `z_completed`: a job that is over completes
    /// without a wait (its end seen late).
    fn poll(&mut self) {
        if self.in_flight.as_ref().is_some_and(|j| j.end_us <= self.now_us) {
            self.complete_in_flight();
        }
    }
}

unsafe extern "C" fn s_init() -> i32 {
    0
}

unsafe extern "C" fn s_heap_base(_h: GlesHeap) -> u64 {
    0x1000_0000
}

/// A draw into another target submits the open job first (`flush_job`), then
/// opens its own. A sampled render target reads its render image: no wait.
unsafe extern "C" fn s_draw(d: *const GlesDraw) -> i32 {
    // SAFETY: the core passes a valid draw.
    let d = unsafe { &*d };
    let key = (d.target_gpu, d.depth_id);
    rig(|r| {
        if r.open.is_some_and(|o| o.0 != key) {
            r.flush_job();
        }
        if r.open.is_none() {
            /* the window's clear at a frame's start goes out alone when the
             * next draw is into a render target (no shaders: a clear) */
            let window_clear = d.vs.is_null() && d.target_w == W as u32;
            r.open =
                Some(if window_clear { (key, "window clear", CLEAR_GPU_US) } else { (key, r.pass, r.pass_gpu_us) });
        }
    });
    0
}

unsafe extern "C" fn s_flush() -> i32 {
    rig(|r| r.flush_job());
    0
}

unsafe extern "C" fn s_finish() -> i32 {
    rig(|r| {
        r.finishes.push(r.call);
        r.flush_job();
        r.complete_in_flight();
    });
    0
}

unsafe extern "C" fn s_finish_previous() -> i32 {
    0
}

unsafe extern "C" fn s_kick_id() -> u32 {
    rig(|r| r.kicks + 1)
}

unsafe extern "C" fn s_kick_pending(id: u32) -> bool {
    rig(|r| {
        r.poll();
        id != 0 && (id > r.kicks || r.in_flight.as_ref().is_some_and(|j| id >= j.kick))
    })
}

/// The detile of the depth plane into the R32F texels: CPU time, after the
/// GL core waited for the depth pass.
unsafe extern "C" fn s_depth_resolve(_id: u32, dst: *mut f32, w: u32, h: u32, pitch: u32) -> i32 {
    for y in 0..h {
        for x in 0..w {
            // SAFETY: the GL core's scratch of `pitch` x `h` floats.
            unsafe { *dst.add((y * pitch + x) as usize) = 0.5 };
        }
    }
    rig(|r| {
        r.resolves += 1;
        r.now_us += r.resolve_us;
    });
    0
}

/// `v3d_generate_mipmap`: the job rendering into the target submitted, the
/// job in flight waited for, the TFU job run (polled by the CPU).
unsafe extern "C" fn s_generate_mipmap(_b: *const GlesTextureBind, _levels: u32) -> i32 {
    rig(|r| {
        r.flush_job();
        r.complete_in_flight();
        r.now_us += r.tfu_us;
        r.mipmaps_on_gpu += 1;
    });
    1
}

unsafe extern "C" fn s_return_mipmap_chain(_b: *const GlesTextureBind, _levels: u32) {}

fn backend_table() -> YakoglBackend {
    YakoglBackend {
        name: c"egl_swap_end_wait".as_ptr() as *const c_char,
        native_attrib_formats: 0,
        rt_rgba8888_is_bgra: false,
        mip_level_align: 0,
        compressed_blocks_in_rows: false,
        native_layered_textures: false,
        front_facing_shader_variant: false,
        etc1_decoded_at_upload: false,
        init: Some(s_init),
        shutdown: None,
        draw: Some(s_draw),
        flush: Some(s_flush),
        finish: Some(s_finish),
        finish_previous: Some(s_finish_previous),
        kick_id: Some(s_kick_id),
        kick_pending: Some(s_kick_pending),
        heap_base: Some(s_heap_base),
        counters: None,
        texture_released: None,
        surface_swapped: None,
        texture_uploaded: None,
        max_instance_divisor: 1,
        wait_kick: None,
        samples_texel_copies: false,
        attachment_gone: None,
        depth_resolve: Some(s_depth_resolve),
        query_begin: None,
        query_end: None,
        query_result: None,
        generate_mipmap: Some(s_generate_mipmap),
        return_mipmap_chain: Some(s_return_mipmap_chain),
        linear_row_align: 0,
        uniforms_host_only: false,
        ds_write: None,
        stencil_resolve: None,
    }
}

/* ---- the platform: host memory, a display that does not wait ---------------------- */

/// Pointer-valued device addresses (the core reads and writes the storage).
unsafe extern "C" fn o_gpu_alloc(_heap: u64, size: usize, _flags: u32, out: *mut OsalGpuMem) -> i32 {
    if out.is_null() || size == 0 {
        return -22;
    }
    let mem = Box::leak(vec![0u64; size.div_ceil(8)].into_boxed_slice());
    let cpu = mem.as_mut_ptr() as *mut c_void;
    // SAFETY: `out` is a valid out-parameter of the library's call.
    unsafe { *out = OsalGpuMem { cpu, gpu: cpu as u64, phys: cpu as u64, size, impl_: cpu } };
    0
}

unsafe extern "C" fn o_gpu_free(_m: *mut OsalGpuMem) -> i32 {
    0
}

unsafe extern "C" fn o_map_phys(_heap: u64, phys: u64, size: usize, _flags: u32, out: *mut OsalGpuMem) -> i32 {
    // SAFETY: `out` is a valid out-parameter of the library's call.
    unsafe { *out = OsalGpuMem { cpu: phys as *mut c_void, gpu: phys, phys, size, impl_: core::ptr::null_mut() } };
    0
}

unsafe extern "C" fn o_cache(_cpu: *mut c_void, _n: usize) {}

/// The Pi 4 GM image's show: a copy into the firmware framebuffer, no wait
/// (board_rpi4.c board_show; tests/egl_swap_timeline.rs `PI4`).
unsafe extern "C" fn o_present(_m: *const OsalGpuMem, _w: u32, _h: u32, _stride: u32, _format: u32) -> i32 {
    0
}

unsafe extern "C" fn o_init(_arg: *mut c_void) -> i32 {
    0
}

/// zephyr_v3d.c: no `.wait_vblank`.
static OSAL: OsalOps = OsalOps {
    init: Some(o_init),
    shutdown: None,
    gpu_alloc: Some(o_gpu_alloc),
    gpu_free: Some(o_gpu_free),
    mmu_map: None,
    map_phys: Some(o_map_phys),
    gpu_write: None,
    gpu_read: None,
    cache_flush: Some(o_cache),
    cache_invd: Some(o_cache),
    submit: None,
    wait: None,
    completed: None,
    reg_read: None,
    reg_write: None,
    query: None,
    log: None,
    wait_vblank: None,
    present: Some(o_present),
    lock: None,
    unlock: None,
    thread_id: None,
    display_busy: None,
};

/* ---- GL plumbing ----------------------------------------------------------------- */

/// The window, and the render targets (smaller, so a job's target tells
/// the window's clear apart).
const W: i32 = 16;
const H: i32 = 16;
const TW: i32 = 8;
const TH: i32 = 8;
const FRAMES: u32 = 8;

/// Run `f` as GL call `call`: the waits it takes are booked under that name.
fn at<R>(call: &'static str, f: impl FnOnce() -> R) -> R {
    rig(|r| r.call = call);
    let v = f();
    rig(|r| r.call = "");
    v
}

/// CPU work of the application between GL calls.
fn cpu(us: u64) {
    rig(|r| r.now_us += us);
}

/// The jobs opened from now on are pass `pass`, `gpu_us` long on the GPU.
fn pass(pass: &'static str, gpu_us: u64) {
    rig(|r| {
        r.pass = pass;
        r.pass_gpu_us = gpu_us;
    });
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

fn program(fs: &str) -> GLuint {
    // SAFETY: plain GL calls.
    unsafe {
        let p = glCreateProgram();
        glAttachShader(p, shader(GL_VERTEX_SHADER, VS));
        glAttachShader(p, shader(GL_FRAGMENT_SHADER, fs));
        glBindAttribLocation(p, 0, c"p".as_ptr());
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        assert_eq!(ok, 1, "link failed");
        p
    }
}

const VS: &str = "attribute vec4 p;\nvarying vec2 v;\nvoid main() { v = p.xy * 0.5 + 0.5; gl_Position = p; }\n";
const FS_FLAT: &str = "precision mediump float;\nvoid main() { gl_FragColor = vec4(1.0); }\n";
const FS_SAMPLE: &str = "precision mediump float;\nuniform sampler2D t;\nvarying vec2 v;\n\
    void main() { gl_FragColor = texture2D(t, v); }\n";

/// A texture `W` x `H` of `format`, with an FBO rendering into it.
fn target(depth: bool, mipmapped: bool) -> (GLuint, GLuint) {
    let (mut tex, mut fbo) = (0, 0);
    // SAFETY: plain GL calls on the current context.
    unsafe {
        glGenTextures(1, &mut tex);
        glBindTexture(GL_TEXTURE_2D, tex);
        let min = if mipmapped { GL_LINEAR_MIPMAP_LINEAR } else { GL_NEAREST };
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, min as GLint);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST as GLint);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE as GLint);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE as GLint);
        let (internal, format, ty) = if depth {
            (
                if cfg!(feature = "report-es2") { GL_DEPTH_COMPONENT } else { GL_DEPTH_COMPONENT24 },
                GL_DEPTH_COMPONENT,
                GL_UNSIGNED_INT,
            )
        } else {
            (GL_RGBA, GL_RGBA, GL_UNSIGNED_BYTE)
        };
        glTexImage2D(GL_TEXTURE_2D, 0, internal as GLint, TW, TH, 0, format, ty, core::ptr::null());
        assert_eq!(glGetError(), GL_NO_ERROR, "glTexImage2D depth {depth}");
        if mipmapped {
            glGenerateMipmap(GL_TEXTURE_2D);
        }
        glBindTexture(GL_TEXTURE_2D, 0);
        glGenFramebuffers(1, &mut fbo);
        glBindFramebuffer(GL_FRAMEBUFFER, fbo);
        let attach = if depth { GL_DEPTH_ATTACHMENT } else { GL_COLOR_ATTACHMENT0 };
        glFramebufferTexture2D(GL_FRAMEBUFFER, attach, GL_TEXTURE_2D, tex, 0);
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE, "depth {depth}");
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        assert_eq!(glGetError(), GL_NO_ERROR);
    }
    (tex, fbo)
}

/// A full-screen quad with `prog`, sampling `tex` when it is not 0.
fn quad(prog: GLuint, tex: GLuint) {
    // SAFETY: plain GL calls on the current context.
    unsafe {
        glUseProgram(prog);
        glActiveTexture(GL_TEXTURE0);
        glBindTexture(GL_TEXTURE_2D, tex);
        if tex != 0 {
            glUniform1i(glGetUniformLocation(prog, c"t".as_ptr()), 0);
        }
        glDrawArrays(GL_TRIANGLE_STRIP, 0, 4);
        assert_eq!(glGetError(), GL_NO_ERROR);
    }
}

/// glmark2's canvas clear at a frame's start: the window's colour and depth.
fn clear_window() {
    // SAFETY: plain GL calls on the current context.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glViewport(0, 0, W, H);
        glColorMask(1, 1, 1, 1);
        glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT);
    }
}

/// Binds `fbo` (0: the window) with its whole size as the viewport.
fn bind(fbo: GLuint) {
    let (w, h) = if fbo == 0 { (W, H) } else { (TW, TH) };
    // SAFETY: plain GL calls on the current context.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, fbo);
        glViewport(0, 0, w, h);
    }
}

/* ---- the scene shapes ------------------------------------------------------------- */

/// What a scene's GL objects are; `frame` records one frame of it.
struct Objects {
    flat: GLuint,
    sample: GLuint,
    /// Render targets: (texture, FBO).
    a: (GLuint, GLuint),
    b: (GLuint, GLuint),
    depth: (GLuint, GLuint),
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Shape {
    /// One window pass; the CPU's work is larger than the GPU's.
    SinglePassCpuBound,
    /// One window pass; the GPU's work is larger than the CPU's.
    SinglePassGpuBound,
    /// scene-shadow.cpp: a depth pass, then the window pass sampling it.
    Shadow,
    /// scene-terrain.cpp's renderer chain, reduced: a pass into a mipmapped
    /// target, glGenerateMipmap, the window pass sampling it.
    Terrain,
    /// scene-refract.cpp: a depth pass, a colour pass into a mipmapped
    /// target, glGenerateMipmap, the window pass sampling both.
    Refract,
    /// scene-desktop.cpp blur: passes ping-pong between the desktop and a
    /// window texture, then the desktop is drawn to the screen.
    DesktopBlur,
    /// scene-desktop.cpp shadow: the windows and their shadows are drawn
    /// into the desktop (one off-screen pass), then the desktop to the
    /// screen.
    DesktopShadow,
}

/// Host model times, in microseconds.
const CLEAR_GPU_US: u64 = 200;
const PASS_GPU_US: u64 = 3_000;
const WINDOW_GPU_US: u64 = 2_000;

fn frame(shape: Shape, o: &Objects) {
    at("glClear", clear_window);
    match shape {
        Shape::SinglePassCpuBound | Shape::SinglePassGpuBound => {
            let (cpu_us, gpu_us) = if shape == Shape::SinglePassCpuBound { (20_000, 2_000) } else { (1_000, 10_000) };
            cpu(cpu_us);
            pass("window", gpu_us);
            at("glDrawArrays (window)", || quad(o.flat, 0));
        }
        Shape::Shadow => {
            /* DepthRenderTarget::enable: the depth pass */
            at("glBindFramebuffer (depth pass)", || bind(o.depth.1));
            pass("depth pass", PASS_GPU_US);
            // SAFETY: plain GL calls on the current context.
            at("glClear (depth pass)", || unsafe {
                glColorMask(0, 0, 0, 0);
                glClear(GL_DEPTH_BUFFER_BIT);
                glEnable(GL_DEPTH_TEST);
            });
            at("glDrawArrays (depth pass)", || quad(o.flat, 0));
            /* back to the window: the ground samples the depth texture */
            // SAFETY: plain GL calls on the current context.
            at("glBindFramebuffer (window)", || unsafe {
                bind(0);
                glColorMask(1, 1, 1, 1);
                glDisable(GL_DEPTH_TEST);
            });
            cpu(500);
            pass("window", WINDOW_GPU_US);
            at("glDrawArrays (ground, samples the depth)", || quad(o.sample, o.depth.0));
        }
        Shape::Refract => {
            at("glBindFramebuffer (depth pass)", || bind(o.depth.1));
            pass("depth pass", PASS_GPU_US);
            // SAFETY: plain GL calls on the current context.
            at("glClear (depth pass)", || unsafe {
                glColorMask(0, 0, 0, 0);
                glClear(GL_DEPTH_BUFFER_BIT);
                glEnable(GL_DEPTH_TEST);
            });
            at("glDrawArrays (depth pass)", || quad(o.flat, 0));
            // SAFETY: plain GL calls on the current context.
            at("glBindFramebuffer (colour pass)", || unsafe {
                bind(o.a.1);
                glColorMask(1, 1, 1, 1);
                glDisable(GL_DEPTH_TEST);
            });
            pass("colour pass", PASS_GPU_US);
            at("glDrawArrays (colour pass)", || quad(o.flat, 0));
            // SAFETY: plain GL calls on the current context.
            at("glGenerateMipmap", || unsafe {
                glBindTexture(GL_TEXTURE_2D, o.a.0);
                glGenerateMipmap(GL_TEXTURE_2D);
                glBindTexture(GL_TEXTURE_2D, 0);
            });
            at("glBindFramebuffer (window)", || bind(0));
            cpu(500);
            pass("window", WINDOW_GPU_US);
            at("glDrawArrays (window, samples the depth)", || quad(o.sample, o.depth.0));
            at("glDrawArrays (window, samples the colour)", || quad(o.sample, o.a.0));
        }
        Shape::Terrain => {
            at("glBindFramebuffer (target)", || bind(o.a.1));
            pass("pass into the mipmapped target", PASS_GPU_US);
            at("glDrawArrays (target)", || quad(o.flat, 0));
            // SAFETY: plain GL calls on the current context.
            at("glGenerateMipmap", || unsafe {
                glBindTexture(GL_TEXTURE_2D, o.a.0);
                glGenerateMipmap(GL_TEXTURE_2D);
                glBindTexture(GL_TEXTURE_2D, 0);
            });
            at("glBindFramebuffer (window)", || bind(0));
            cpu(500);
            pass("window", WINDOW_GPU_US);
            at("glDrawArrays (window, samples the target)", || quad(o.sample, o.a.0));
        }
        Shape::DesktopBlur | Shape::DesktopShadow => {
            /* desktop.clear(), then per blur pass render_from / render_to;
             * the shadow's windows draw into the desktop itself */
            at("glBindFramebuffer (desktop)", || bind(o.a.1));
            pass("desktop", PASS_GPU_US);
            at("glDrawArrays (desktop)", || quad(o.flat, 0));
            let blur_passes = if shape == Shape::DesktopBlur { 2 } else { 0 };
            for i in 0..blur_passes {
                at("glBindFramebuffer (window texture)", || bind(o.b.1));
                pass(if i == 0 { "blur h 0" } else { "blur h 1" }, PASS_GPU_US);
                at("glDrawArrays (render_from)", || quad(o.sample, o.a.0));
                at("glBindFramebuffer (desktop)", || bind(o.a.1));
                pass(if i == 0 { "blur v 0" } else { "blur v 1" }, PASS_GPU_US);
                at("glDrawArrays (render_to)", || quad(o.sample, o.b.0));
            }
            at("glBindFramebuffer (window)", || bind(0));
            cpu(500);
            pass("window", WINDOW_GPU_US);
            at("glDrawArrays (desktop to screen)", || quad(o.sample, o.a.0));
        }
    }
}

/* ---- the run ------------------------------------------------------------------------ */

/// One swap, as the glmark2 sample reads it afterwards.
#[derive(Debug)]
struct SwapRead {
    frame: u32,
    /// The jobs that completed since the swap before, in order.
    done: Vec<Done>,
}

struct Run {
    jobs: Vec<Job>,
    swaps: Vec<SwapRead>,
    finishes: Vec<&'static str>,
    mipmaps_on_gpu: u32,
    resolves: u32,
}

impl Run {
    fn job(&self, kick: u32) -> &Job {
        self.jobs.iter().find(|j| j.kick == kick).expect("a submitted job")
    }

    /// The frames counted (the first two are the scene's set-up: the
    /// sample resets its totals after the first frame), and of them those
    /// whose last completed job was waited for: the sample's
    /// "end waited for in N of M".
    fn end_waited(&self) -> (usize, usize) {
        let counted: Vec<&SwapRead> = self.swaps.iter().filter(|s| s.frame >= 2 && !s.done.is_empty()).collect();
        (counted.iter().filter(|s| s.done.last().is_some_and(|d| d.waited)).count(), counted.len())
    }

    /// Per counted frame, the wait the sample counted: the call that took
    /// it, the frame and pass of the job it waited for.
    fn counted_waits(&self) -> Vec<(u32, &'static str, u32, &'static str)> {
        self.swaps
            .iter()
            .filter(|s| s.frame >= 2)
            .filter_map(|s| s.done.last().filter(|d| d.waited).map(|d| (s, d)))
            .map(|(s, d)| {
                let j = self.job(d.kick);
                (s.frame, d.call, j.frame, j.pass)
            })
            .collect()
    }

    fn report(&self, shape: Shape) -> String {
        let (n, of) = self.end_waited();
        let mut out = format!("{shape:?}: end waited for in {n} of {of}\n");
        for s in &self.swaps {
            let line: Vec<String> = s
                .done
                .iter()
                .map(|d| {
                    let j = self.job(d.kick);
                    format!(
                        "kick {} (frame {} {}, {} us) {} in {}",
                        d.kick,
                        j.frame,
                        j.pass,
                        j.gpu_us,
                        if d.waited { "WAITED" } else { "seen late" },
                        if d.call.is_empty() { "-" } else { d.call }
                    )
                })
                .collect();
            out += &format!("  swap {}: {}\n", s.frame, line.join("; "));
        }
        out
    }
}

fn run(shape: Shape) -> Run {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    *RIG.lock().unwrap_or_else(|e| e.into_inner()) = Some(Rig { resolve_us: 6_000, tfu_us: 1_000, ..Rig::default() });
    let table: &'static YakoglBackend = Box::leak(Box::new(backend_table()));
    let mut pixels = vec![0u8; (W * H * 4) as usize + 4096];
    let phys = (pixels.as_mut_ptr() as u64 + 4095) & !4095;
    let mut win = GlesNativeWindow {
        phys,
        width: W as u32,
        height: H as u32,
        stride_bytes: W as u32 * 4,
        format: GLES_NATIVE_FORMAT_XRGB8888,
    };
    #[rustfmt::skip]
    let cfg_list = [
        EGL_RED_SIZE, 8, EGL_GREEN_SIZE, 8, EGL_BLUE_SIZE, 8, EGL_DEPTH_SIZE, 16,
        EGL_SURFACE_TYPE, EGL_WINDOW_BIT, EGL_RENDERABLE_TYPE, EGL_OPENGL_ES2_BIT, EGL_NONE,
    ];
    let ctx_list = [EGL_CONTEXT_CLIENT_VERSION, 2, EGL_NONE];
    // SAFETY: tests are serialised; the tables are static or leaked; every
    // pointer passed is live for its call.
    unsafe {
        osal_set_ops(&OSAL);
        backend::g_backend = table;
        yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
        let dpy = eglGetDisplay(core::ptr::null_mut());
        assert_eq!(eglInitialize(dpy, core::ptr::null_mut(), core::ptr::null_mut()), 1);
        let mut cfg = core::ptr::null_mut();
        let mut n = 0;
        assert_eq!(eglChooseConfig(dpy, cfg_list.as_ptr(), &mut cfg, 1, &mut n), 1);
        assert_eq!(n, 1, "a window config");
        /* no attribute list: EGL_BACK_BUFFER, what glmark2 gets */
        let s = eglCreateWindowSurface(dpy, cfg, &mut win, core::ptr::null());
        assert!(!s.is_null(), "eglCreateWindowSurface: {:#x}", eglGetError());
        let c = eglCreateContext(dpy, cfg, core::ptr::null_mut(), ctx_list.as_ptr());
        assert!(!c.is_null());
        assert_eq!(eglMakeCurrent(dpy, s, s, c), 1);
        let mut rb = 0;
        assert_eq!(eglQuerySurface(dpy, s, EGL_RENDER_BUFFER, &mut rb), 1);
        assert_eq!(rb, EGL_BACK_BUFFER, "the window EGL double-buffers");
        assert_eq!(eglSwapInterval(dpy, 0), 1, "glmark2's interval");

        let mut vbo = 0;
        let quad: [f32; 16] = [-1.0, -1.0, 0.5, 1.0, 1.0, -1.0, 0.5, 1.0, -1.0, 1.0, 0.5, 1.0, 1.0, 1.0, 0.5, 1.0];
        glGenBuffers(1, &mut vbo);
        glBindBuffer(GL_ARRAY_BUFFER, vbo);
        glBufferData(GL_ARRAY_BUFFER, 64, quad.as_ptr() as *const c_void, GL_STATIC_DRAW);
        glEnableVertexAttribArray(0);
        glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, core::ptr::null());
        let o = Objects {
            flat: program(FS_FLAT),
            sample: program(FS_SAMPLE),
            a: target(false, matches!(shape, Shape::Terrain | Shape::Refract)),
            b: target(false, false),
            depth: target(true, false),
        };
        rig(|r| {
            r.done.clear();
            r.finishes.clear();
            r.mipmaps_on_gpu = 0;
        });

        let mut swaps = Vec::new();
        for f in 0..FRAMES {
            rig(|r| r.frame = f);
            frame(shape, &o);
            assert_eq!(at("eglSwapBuffers", || eglSwapBuffers(dpy, s)), 1, "{shape:?}: swap {f}");
            let done = rig(|r| core::mem::take(&mut r.done));
            swaps.push(SwapRead { frame: f, done });
        }
        let (jobs, finishes, mipmaps_on_gpu, resolves) =
            rig(|r| (r.jobs.clone(), r.finishes.clone(), r.mipmaps_on_gpu, r.resolves));
        eglMakeCurrent(dpy, core::ptr::null_mut(), core::ptr::null_mut(), core::ptr::null_mut());
        eglDestroyContext(dpy, c);
        eglDestroySurface(dpy, s);
        eglTerminate(dpy);
        drop(pixels);
        Run { jobs, swaps, finishes, mipmaps_on_gpu, resolves }
    }
}

/// In every shape: no swap waits for the window job it submits, and no swap
/// calls `finish` (the double buffer's swap waits only for the frame
/// before).
fn swap_never_waits_for_its_own_window_job(r: &Run, shape: Shape) {
    let report = r.report(shape);
    for s in &r.swaps {
        for d in s.done.iter().filter(|d| d.call == "eglSwapBuffers" && d.waited) {
            let j = r.job(d.kick);
            assert!(
                !(j.frame == s.frame && j.pass == "window"),
                "{shape:?}: swap {} waited for its own frame's window job\n{report}",
                s.frame
            );
        }
    }
    assert!(!r.finishes.contains(&"eglSwapBuffers"), "{shape:?}: a swap called finish\n{report}");
}

/// The counted wait of every counted frame: taken in `call`, for a job of
/// the same frame (`own`) or of the frame before, in pass `pass` (any pass
/// when `None`).
fn every_counted_wait(r: &Run, shape: Shape, call: &str, own: bool, pass: Option<&str>) {
    let report = r.report(shape);
    let (n, of) = r.end_waited();
    assert_eq!(n, of, "{shape:?}: end waited for in every frame\n{report}");
    assert!(of >= FRAMES as usize - 2, "{shape:?}: every frame counted\n{report}");
    for (frame, c, job_frame, job_pass) in r.counted_waits() {
        assert_eq!(c, call, "{shape:?}: swap {frame}'s counted wait\n{report}");
        assert_eq!(
            job_frame == frame,
            own,
            "{shape:?}: swap {frame}'s counted wait is for frame {job_frame}\n{report}"
        );
        if let Some(p) = pass {
            assert_eq!(job_pass, p, "{shape:?}: swap {frame}'s counted wait\n{report}");
        }
    }
}

#[test]
fn a_cpu_bound_single_pass_scene_reads_0_of_n() {
    let r = run(Shape::SinglePassCpuBound);
    eprintln!("{}", r.report(Shape::SinglePassCpuBound));
    swap_never_waits_for_its_own_window_job(&r, Shape::SinglePassCpuBound);
    let (n, of) = r.end_waited();
    assert!(of >= FRAMES as usize - 2, "{}", r.report(Shape::SinglePassCpuBound));
    assert_eq!(n, 0, "the frame before is over when the swap's flush looks\n{}", r.report(Shape::SinglePassCpuBound));
}

#[test]
fn a_gpu_bound_single_pass_scene_waits_in_the_swap_for_the_frame_before() {
    let r = run(Shape::SinglePassGpuBound);
    eprintln!("{}", r.report(Shape::SinglePassGpuBound));
    swap_never_waits_for_its_own_window_job(&r, Shape::SinglePassGpuBound);
    every_counted_wait(&r, Shape::SinglePassGpuBound, "eglSwapBuffers", false, Some("window"));
}

#[test]
fn shadow_waits_in_the_depth_resolve_for_its_own_depth_pass() {
    let r = run(Shape::Shadow);
    eprintln!("{}", r.report(Shape::Shadow));
    assert_eq!(r.resolves, FRAMES, "one depth resolve a frame\n{}", r.report(Shape::Shadow));
    swap_never_waits_for_its_own_window_job(&r, Shape::Shadow);
    every_counted_wait(&r, Shape::Shadow, "glDrawArrays (ground, samples the depth)", true, Some("depth pass"));
}

#[test]
fn terrain_waits_in_glgeneratemipmap_for_its_own_target_pass() {
    let r = run(Shape::Terrain);
    eprintln!("{}", r.report(Shape::Terrain));
    assert_eq!(r.mipmaps_on_gpu, FRAMES, "the chain made by the back end every frame\n{}", r.report(Shape::Terrain));
    swap_never_waits_for_its_own_window_job(&r, Shape::Terrain);
    every_counted_wait(&r, Shape::Terrain, "glGenerateMipmap", true, Some("pass into the mipmapped target"));
}

#[test]
fn refract_waits_in_glgeneratemipmap_for_its_own_colour_pass() {
    let r = run(Shape::Refract);
    eprintln!("{}", r.report(Shape::Refract));
    assert_eq!(r.mipmaps_on_gpu, FRAMES, "the chain made by the back end every frame\n{}", r.report(Shape::Refract));
    assert_eq!(r.resolves, FRAMES, "one depth resolve a frame\n{}", r.report(Shape::Refract));
    swap_never_waits_for_its_own_window_job(&r, Shape::Refract);
    every_counted_wait(&r, Shape::Refract, "glGenerateMipmap", true, Some("colour pass"));
}

#[test]
fn desktop_blur_waits_in_the_swaps_flush_for_its_own_last_offscreen_pass() {
    let r = run(Shape::DesktopBlur);
    eprintln!("{}", r.report(Shape::DesktopBlur));
    swap_never_waits_for_its_own_window_job(&r, Shape::DesktopBlur);
    every_counted_wait(&r, Shape::DesktopBlur, "eglSwapBuffers", true, Some("blur v 1"));
}

#[test]
fn desktop_shadow_waits_in_the_swaps_flush_for_its_own_desktop_pass() {
    let r = run(Shape::DesktopShadow);
    eprintln!("{}", r.report(Shape::DesktopShadow));
    swap_never_waits_for_its_own_window_job(&r, Shape::DesktopShadow);
    every_counted_wait(&r, Shape::DesktopShadow, "eglSwapBuffers", true, Some("desktop"));
}
