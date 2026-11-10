// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! 4x multisampling of framebuffer objects on the Mali (Valhall G610), through
//! the GL API (msaa-pin-mali-file), on the host.
//!
//! docs/notes/es3-msaa-renderbuffers.md section (0) and (c4): the GL core
//! never makes a 4x framebuffer object. `GL_MAX_SAMPLES` is 1,
//! `glRenderbufferStorageMultisample` gives `GL_INVALID_OPERATION` for 2 or
//! more samples, one sample is plain storage, and `GlesDraw::samples` of an
//! FBO is the literal 1. Only the window is 4x (`default_samples`): the Mali's
//! `SampleBuffer` keeps the samples of the last 4x target and `resolve_frame`
//! averages them into the target in the same kick. A 4x FBO would map onto a
//! pool of those stores keyed by the attachment `(target_id, target_gen)`.
//!
//! The GL core is driven through the API into a stand-in back end whose draw
//! entry first runs the Mali's own `mali_draw` (its `check` and program
//! checks; no Mali OSAL on the host, so a draw it takes ends at `-ENODEV`
//! and one it refuses at `-ENOTSUP`), and then models the contract of note
//! (b) 4-7 on the host: four samples a pixel kept per target, a resolve into
//! the target's memory after every draw. The model is the stand-in's, not
//! the Mali's: pixels are only ever checked against the contract; the Mali's
//! own frames are compared with Mesa's in `src/mali/frame_msaa_tests.rs`.
//!
//! Passing tests pin today. The `#[ignore]`d ones are the expected result
//! after the design, each naming the note section and the owner: the GL core
//! (`gl-msaa-cap`, `gl-msaa-resolve`) or the Mali back end (`mali-msaa-fbo`,
//! `src/mali`). They fail today at `glRenderbufferStorageMultisample`.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test es3_msaa_mali
//! ```

#![cfg(all(feature = "host-test", feature = "shaderc-online", feature = "gpu-mali", not(feature = "report-es2")))]

use core::ffi::{c_char, c_void};
use std::ffi::CString;
use std::sync::atomic::Ordering;
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{self, GlesDraw, GlesHeap, YakoglBackend, *};
use yakogl::gl::buffer::glBindBuffer;
use yakogl::gl::consts::*;
use yakogl::gl::context::GlContext;
use yakogl::gl::draw::*;
use yakogl::gl::framebuffer::*;
use yakogl::gl::program::*;
use yakogl::gl::shader::*;
use yakogl::gl::state::*;
use yakogl::gl::types::*;
use yakogl::gl::vertex::{glEnableVertexAttribArray, glVertexAttribPointer};
use yakogl::mali::backend::mali_draw;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};
use yakogl::util::ENODEV;

/* ---- a context on the host: the smallest OSAL and back end that make one -- */

/// Freed blocks by size: `gpu_free` hands one back to the next `gpu_alloc` of
/// its size, zeroed, so that a respecified renderbuffer comes back at the
/// address it had (the case the note's identity rule is for).
static FREED: Mutex<Vec<(usize, usize)>> = Mutex::new(Vec::new());

unsafe extern "C" fn m_gpu_alloc(_heap: u64, size: usize, _flags: u32, out: *mut OsalGpuMem) -> i32 {
    let recycled = {
        let mut f = FREED.lock().unwrap();
        f.iter().position(|(s, _)| *s == size).map(|i| f.remove(i).1)
    };
    let p = match recycled {
        Some(p) => {
            core::ptr::write_bytes(p as *mut u8, 0, size);
            p as *mut u8
        }
        None => Box::leak(vec![0u8; size].into_boxed_slice()).as_mut_ptr(),
    };
    *out = OsalGpuMem { cpu: p as *mut c_void, gpu: p as u64, phys: p as u64, size, impl_: p as *mut c_void };
    0
}
unsafe extern "C" fn m_gpu_free(mem: *mut OsalGpuMem) -> i32 {
    FREED.lock().unwrap().push(((*mem).size, (*mem).cpu as usize));
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

/// What the back end was handed by one draw, and what the Mali made of it.
#[derive(Clone, Debug)]
struct Seen {
    /// An application draw (vertices), not a clear alone.
    draws: bool,
    /// `targets[..num_targets]`: (gpu, format).
    targets: Vec<(u64, u8)>,
    /// `GlesDraw::target_gpu`.
    gpu: u64,
    /// `GlesDraw::samples`.
    samples: u8,
    /// `GlesDraw::depth_id`.
    depth_id: u32,
    /// What `mali_draw` answered: `-ENODEV` is "taken, no queue on the host",
    /// `-ENOTSUP` a refusal by name.
    mali: i32,
}

static SEEN: Mutex<Vec<Seen>> = Mutex::new(Vec::new());
/// The ids `depth_resolve` was asked for.
static DEPTH_ASKED: Mutex<Vec<u32>> = Mutex::new(Vec::new());
/// The triangle the next draws rasterise in the model (clip space xyzw x3).
static TRIANGLE: Mutex<[f32; 12]> = Mutex::new([0.0; 12]);

/* The stand-in's model of note (b) 4-7: four samples a pixel of a 4x target,
 * kept per (gpu, w, h) until `attachment_gone`, loaded from the resolved
 * memory when nothing is kept, averaged into the target after every draw. */
type Pixel = [[u8; 4]; 4];
/// (gpu, w, h) and the target's pixels.
type Kept = ((u64, u32, u32), Vec<Pixel>);
static STORE: Mutex<Vec<Kept>> = Mutex::new(Vec::new());

/// The Mali's rotated 4x grid, as fractions of a pixel.
const SAMPLE_POS: [(f32, f32); 4] = [(0.375, 0.125), (0.875, 0.375), (0.125, 0.625), (0.625, 0.875)];

/// The colour the test's fragment shader writes, and the clear value.
const COLOUR: [f32; 4] = [0.25, 0.5, 0.75, 1.0];

fn bytes(rgba: [f32; 4]) -> [u8; 4] {
    rgba.map(|v| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8)
}

fn covered(tri: &[f32; 12], x: f32, y: f32) -> bool {
    let p: Vec<(f32, f32)> = (0..3).map(|i| (tri[i * 4], tri[i * 4 + 1])).collect();
    let e = |a: (f32, f32), b: (f32, f32)| (b.0 - a.0) * (y - a.1) - (b.1 - a.1) * (x - a.0);
    let (w0, w1, w2) = (e(p[0], p[1]), e(p[1], p[2]), e(p[2], p[0]));
    (w0 >= 0.0 && w1 >= 0.0 && w2 >= 0.0) || (w0 <= 0.0 && w1 <= 0.0 && w2 <= 0.0)
}

/// The clip-space position of sample `s` of pixel (x, y) in a w x h target.
fn sample_at(x: u32, y: u32, s: usize, w: u32, h: u32) -> (f32, f32) {
    let (dx, dy) = SAMPLE_POS[s];
    ((x as f32 + dx) / w as f32 * 2.0 - 1.0, (y as f32 + dy) / h as f32 * 2.0 - 1.0)
}

unsafe fn model(d: &GlesDraw) {
    if d.samples != 4 || d.target_gpu == 0 {
        return;
    }
    let (w, h, pitch) = (d.target_w, d.target_h, backend::draw_pitch(d) as usize);
    let mem = core::slice::from_raw_parts_mut(d.target_gpu as *mut u8, pitch * h as usize * 4);
    let key = (d.target_gpu, w, h);
    let mut store = STORE.lock().unwrap();
    let at = match store.iter().position(|(k, _)| *k == key) {
        Some(i) => i,
        None => {
            /* nothing kept: every sample starts as the resolved colour */
            let px = (0..w * h)
                .map(|i| {
                    let (x, y) = ((i % w) as usize, (i / w) as usize);
                    let o = (y * pitch + x) * 4;
                    [[mem[o], mem[o + 1], mem[o + 2], mem[o + 3]]; 4]
                })
                .collect();
            store.push((key, px));
            store.len() - 1
        }
    };
    let px = &mut store[at].1;
    if d.do_clear && !d.clear_quad {
        px.fill([bytes(d.clear_rgba); 4]);
    }
    if d.index_count != 0 {
        let tri = *TRIANGLE.lock().unwrap();
        for y in 0..h {
            for x in 0..w {
                let row = if d.flip_y { h - 1 - y } else { y };
                for (s, sample) in px[(row * w + x) as usize].iter_mut().enumerate() {
                    let (cx, cy) = sample_at(x, y, s, w, h);
                    if covered(&tri, cx, cy) {
                        *sample = bytes(COLOUR);
                    }
                }
            }
        }
    }
    for y in 0..h as usize {
        for x in 0..w as usize {
            let p = px[y * w as usize + x];
            for c in 0..4 {
                let sum: u32 = p.iter().map(|s| u32::from(s[c])).sum();
                mem[(y * pitch + x) * 4 + c] = ((sum + 2) / 4) as u8;
            }
        }
    }
}

unsafe extern "C" fn b_draw(d: *const GlesDraw) -> i32 {
    let d = &*d;
    let n = (d.num_targets.max(1) as usize).min(GLES_MAX_TARGETS);
    let mali = mali_draw(d);
    SEEN.lock().unwrap().push(Seen {
        draws: d.index_count != 0,
        targets: d.targets[..n].iter().map(|t| (t.gpu, t.format)).collect(),
        gpu: d.target_gpu,
        samples: d.samples,
        depth_id: d.depth_id,
        mali,
    });
    model(d);
    0
}

unsafe extern "C" fn b_gone(_id: u32) {
    /* the model keys by address: whatever it keeps is dropped, the contract
     * of note (b) 6 for the one object the GL core names */
    STORE.lock().unwrap().clear();
}

/// Sample 0 of the triangle's depth (0.5 where covered, else 1.0), for the
/// note's "sample 0 of the pixel" (b) 8.
unsafe extern "C" fn b_depth_resolve(id: u32, dst: *mut f32, w: u32, h: u32, pitch: u32) -> i32 {
    DEPTH_ASKED.lock().unwrap().push(id);
    let tri = *TRIANGLE.lock().unwrap();
    for y in 0..h {
        for x in 0..w {
            let (cx, cy) = sample_at(x, y, 0, w, h);
            *dst.add((y * pitch + x) as usize) = if covered(&tri, cx, cy) { 0.5 } else { 1.0 };
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
    name: c"es3-msaa-mali".as_ptr() as *const c_char,
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
    attachment_gone: Some(b_gone),
    depth_resolve: Some(b_depth_resolve),
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
static CTX: Mutex<usize> = Mutex::new(0);

fn context() -> MutexGuard<'static, ()> {
    let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    backend::HOST_DRAW_TARGETS.store(4, Ordering::Relaxed);
    ONCE.call_once(|| {
        // SAFETY: both tables are statics; the lock above serialises the tests.
        unsafe {
            osal_set_ops(&OSAL);
            backend::g_backend = &BACKEND;
            yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
            let c = yakogl::gl::context::gl_context_create(8, 8);
            assert!(!c.is_null());
            *CTX.lock().unwrap() = c as usize;
        }
    });
    set_window_samples(1);
    // SAFETY: plain GL calls.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glFinish();
    }
    SEEN.lock().unwrap().clear();
    DEPTH_ASKED.lock().unwrap().clear();
    STORE.lock().unwrap().clear();
    // SAFETY: plain GL call.
    while unsafe { glGetError() } != GL_NO_ERROR {}
    g
}

/// The window's sample count, as an EGL surface of the 4x config sets it
/// (`default_samples`); the lock of `context()` is held by the caller.
fn set_window_samples(n: u8) {
    let c = *CTX.lock().unwrap() as *mut GlContext;
    // SAFETY: the context `context()` made; the tests take turns.
    unsafe { (*c).default_samples = n };
}

fn err() -> GLenum {
    // SAFETY: plain GL call.
    unsafe { glGetError() }
}

/* ---- framebuffers, the program, the draw ------------------------------- */

struct Fbo {
    fb: GLuint,
    rbs: Vec<GLuint>,
}

impl Drop for Fbo {
    fn drop(&mut self) {
        // SAFETY: names this test made.
        unsafe {
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
            glDeleteFramebuffers(1, &self.fb);
            glDeleteRenderbuffers(self.rbs.len() as GLsizei, self.rbs.as_ptr());
            glUseProgram(0);
        }
    }
}

/// The error of `glRenderbufferStorageMultisample(.., samples, format, 8, 8)`
/// on a fresh renderbuffer, and the renderbuffer.
fn storage(samples: GLsizei, format: GLenum) -> (GLenum, GLuint) {
    let mut rb = 0;
    // SAFETY: plain GL calls.
    unsafe {
        glGenRenderbuffers(1, &mut rb);
        glBindRenderbuffer(GL_RENDERBUFFER, rb);
        while glGetError() != GL_NO_ERROR {}
        glRenderbufferStorageMultisample(GL_RENDERBUFFER, samples, format, 8, 8);
    }
    (err(), rb)
}

/// A bound 8x8 framebuffer with a renderbuffer of `samples` on colour 0 and,
/// when `depth` is set, a depth attachment of the same count. A storage error
/// comes back as `Err(error)`.
fn framebuffer(samples: GLsizei, depth: bool) -> Result<Fbo, GLenum> {
    let mut fb = 0;
    // SAFETY: room for one name; plain GL call.
    unsafe {
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
    }
    let mut fbo = Fbo { fb, rbs: Vec::new() };
    let attachments = [(GL_RGBA8, GL_COLOR_ATTACHMENT0), (GL_DEPTH_COMPONENT24, GL_DEPTH_ATTACHMENT)];
    for (format, point) in attachments.iter().take(if depth { 2 } else { 1 }) {
        let (e, rb) = storage(samples, *format);
        fbo.rbs.push(rb);
        if e != GL_NO_ERROR {
            return Err(e);
        }
        // SAFETY: a renderbuffer this test made.
        unsafe { glFramebufferRenderbuffer(GL_FRAMEBUFFER, *point, GL_RENDERBUFFER, rb) };
    }
    let buf = [GL_COLOR_ATTACHMENT0];
    // SAFETY: plain GL calls.
    unsafe {
        glDrawBuffers(1, buf.as_ptr());
        assert_eq!(err(), GL_NO_ERROR);
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE, "{samples} samples");
    }
    Ok(fbo)
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

/// The current program: one float output, `COLOUR`.
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

/// Covers the whole target.
static FULL: [f32; 12] = [-1.0, -1.0, 0.0, 1.0, 3.0, -1.0, 0.0, 1.0, -1.0, 3.0, 0.0, 1.0];
/// The lower-left half: the 8x8 target's pixels with x + y = 7 are cut in
/// two by its diagonal edge (two of the four samples).
static HALF: [f32; 12] = [-1.0, -1.0, 0.0, 1.0, 1.0, -1.0, 0.0, 1.0, -1.0, 1.0, 0.0, 1.0];
/// The upper-right half, the other side of the same diagonal.
static OTHER_HALF: [f32; 12] = [1.0, 1.0, 0.0, 1.0, -1.0, 1.0, 0.0, 1.0, 1.0, -1.0, 0.0, 1.0];

/// Draw one triangle and hand back what the back end was given for it.
fn draw(tri: &'static [f32; 12]) -> Seen {
    SEEN.lock().unwrap().clear();
    *TRIANGLE.lock().unwrap() = *tri;
    // SAFETY: `tri` is static and outlives the draw.
    unsafe {
        glBindBuffer(GL_ARRAY_BUFFER, 0);
        glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, tri.as_ptr() as *const c_void);
        glEnableVertexAttribArray(0);
        glDrawArrays(GL_TRIANGLES, 0, 3);
        glFinish();
    }
    assert_eq!(err(), GL_NO_ERROR);
    let s = core::mem::take(&mut *SEEN.lock().unwrap());
    s.into_iter().rev().find(|s| s.draws).expect("the draw reached the back end")
}

fn clear_black() {
    // SAFETY: plain GL calls.
    unsafe {
        glClearColor(0.0, 0.0, 0.0, 0.0);
        glClear(GL_COLOR_BUFFER_BIT);
        glFinish();
    }
}

/// Blit `read`'s colour (and depth when `depth`) into a fresh single-sample
/// framebuffer of the same size and return the GL error and that framebuffer.
fn resolve_blit(read: &Fbo, depth: bool) -> (GLenum, Fbo) {
    let dst = framebuffer(0, depth).expect("a single-sample target");
    let mask = GL_COLOR_BUFFER_BIT | if depth { GL_DEPTH_BUFFER_BIT } else { 0 };
    // SAFETY: plain GL calls on names this test made.
    unsafe {
        glBindFramebuffer(GL_READ_FRAMEBUFFER, read.fb);
        glBindFramebuffer(GL_DRAW_FRAMEBUFFER, dst.fb);
        glReadBuffer(GL_COLOR_ATTACHMENT0);
        while glGetError() != GL_NO_ERROR {}
        glBlitFramebuffer(0, 0, 8, 8, 0, 0, 8, 8, mask, GL_NEAREST);
    }
    (err(), dst)
}

/// The 8x8 RGBA8 pixels of a framebuffer.
fn read_pixels(fb: GLuint) -> Vec<[u8; 4]> {
    let mut v = vec![[0u8; 4]; 64];
    // SAFETY: room for 8x8 RGBA8; a framebuffer this test made.
    unsafe {
        glBindFramebuffer(GL_READ_FRAMEBUFFER, fb);
        glReadPixels(0, 0, 8, 8, GL_RGBA, GL_UNSIGNED_BYTE, v.as_mut_ptr() as *mut c_void);
    }
    assert_eq!(err(), GL_NO_ERROR);
    v
}

fn geti(pname: GLenum) -> GLint {
    let mut v = -1;
    // SAFETY: one value.
    unsafe { glGetIntegerv(pname, &mut v) };
    v
}

fn rb_samples(rb: GLuint) -> GLint {
    let mut v = -1;
    // SAFETY: one value; a renderbuffer this test made.
    unsafe {
        glBindRenderbuffer(GL_RENDERBUFFER, rb);
        glGetRenderbufferParameteriv(GL_RENDERBUFFER, GL_RENDERBUFFER_SAMPLES, &mut v);
    }
    v
}

fn internalformat(format: GLenum, pname: GLenum) -> GLint {
    let mut v = -1;
    // SAFETY: room for one value.
    unsafe { glGetInternalformativ(GL_RENDERBUFFER, format, pname, 1, &mut v) };
    assert_eq!(err(), GL_NO_ERROR);
    v
}

/// The resolved `pixels` of a draw of the lower-left half over black:
/// `COLOUR` exactly where a pixel is wholly inside (`inside`), black where
/// wholly outside, and the mean of the two (two of four samples, +-1) on the
/// diagonal pixels `cut` names.
fn check_halves(pixels: &[[u8; 4]], inside: impl Fn(usize, usize) -> bool, cut: impl Fn(usize, usize) -> bool) {
    let colour = bytes(COLOUR);
    for y in 0..8 {
        for x in 0..8 {
            let p = pixels[y * 8 + x];
            if cut(x, y) {
                for c in 0..4 {
                    let half = i32::from(colour[c]) / 2;
                    assert!((i32::from(p[c]) - half).abs() <= 1, "edge pixel ({x},{y}) {p:?}");
                }
            } else if inside(x, y) {
                assert_eq!(p, colour, "interior pixel ({x},{y})");
            } else {
                assert_eq!(p, [0; 4], "outside pixel ({x},{y})");
            }
        }
    }
}

/* ---- the queries the GL core answers ------------------------------------ */

/// `GL_MAX_SAMPLES` is 1 (note (0), `src/gl/state.rs`).
#[test]
fn max_samples_is_one_today() {
    let _g = context();
    assert_eq!(geti(GL_MAX_SAMPLES), 1);
}

/// `glGetInternalformativ`: one sample count, 1, for a renderable format; no
/// counts for an integer one (ES 3.0 4.4.2.1).
#[test]
fn internalformat_counts_today() {
    let _g = context();
    for f in [GL_RGBA8, GL_SRGB8_ALPHA8, GL_RGB565, GL_RGB8] {
        assert_eq!(internalformat(f, GL_NUM_SAMPLE_COUNTS), 1, "{f:#x}");
        assert_eq!(internalformat(f, GL_SAMPLES), 1, "{f:#x}");
    }
    for f in [GL_R8I, GL_RGBA32I] {
        assert_eq!(internalformat(f, GL_NUM_SAMPLE_COUNTS), 0, "{f:#x}");
    }
}

/// GL_SAMPLES and GL_SAMPLE_BUFFERS of the window and of framebuffers of
/// zero-sample and one-sample storage, and the renderbuffer's own count.
#[test]
fn renderbuffer_samples_read_back_what_is_stored() {
    let _g = context();
    assert_eq!((geti(GL_SAMPLE_BUFFERS), geti(GL_SAMPLES)), (0, 0), "the window of the 8x8 test context");
    let fbo = framebuffer(0, false).unwrap();
    assert_eq!((geti(GL_SAMPLE_BUFFERS), geti(GL_SAMPLES)), (0, 0));
    assert_eq!(rb_samples(fbo.rbs[0]), 0);
    drop(fbo);
    /* one sample is plain storage "with sample buffers" */
    let fbo = framebuffer(1, false).unwrap();
    assert_eq!((geti(GL_SAMPLE_BUFFERS), geti(GL_SAMPLES)), (1, 1));
    assert_eq!(rb_samples(fbo.rbs[0]), 1);
}

/* ---- what the application sees: the storage errors ------------------------ */

/// 4 (and 2) samples: `GL_INVALID_OPERATION`, no storage, the count being
/// past `GL_MAX_SAMPLES`; a negative count is `GL_INVALID_VALUE` first.
#[test]
fn four_samples_is_invalid_operation() {
    let _g = context();
    assert_eq!(storage(4, GL_RGBA8).0, GL_INVALID_OPERATION);
    assert_eq!(storage(2, GL_RGBA8).0, GL_INVALID_OPERATION);
    assert_eq!(storage(4, GL_DEPTH_COMPONENT24).0, GL_INVALID_OPERATION);
    assert_eq!(storage(-1, GL_RGBA8).0, GL_INVALID_VALUE);
    assert_eq!(framebuffer(4, false).err(), Some(GL_INVALID_OPERATION));
}

/// An integer format stays refused at any count: the spec's rule, not a
/// limit, so it holds after the work too.
#[test]
fn integer_formats_stay_refused() {
    let _g = context();
    for f in [GL_R8I, GL_RGBA8UI, GL_RGBA32UI] {
        assert_eq!(storage(1, f).0, GL_INVALID_OPERATION, "{f:#x}");
        assert_eq!(storage(4, f).0, GL_INVALID_OPERATION, "{f:#x}");
    }
}

/* ---- the draw as the Mali gets it ------------------------------------------ */

/// What works today: one sample. A draw into a 1-sample FBO reaches the Mali
/// with `GlesDraw::samples == 1` (not the window's count), which it takes (no
/// refusal; with no queue on the host it ends at `-ENODEV`), and a resolve
/// blit into a single-sample framebuffer is no error.
#[test]
fn a_one_sample_fbo_draws_with_samples_one() {
    let _g = context();
    set_window_samples(4);
    let fbo = framebuffer(1, false).unwrap();
    program();
    let s = draw(&FULL);
    assert_eq!(s.samples, 1, "an FBO is single-sampled whatever the window is");
    assert_eq!(s.targets.len(), 1);
    assert_eq!(s.mali, -ENODEV, "taken by the Mali's checks");
    assert_eq!(resolve_blit(&fbo, false).0, GL_NO_ERROR);
}

/// The 4x window: with the config's 4 samples the draw reaches the Mali with
/// `samples == 4` and passes its checks (`NOT_BUILT_SAMPLES` is a later check,
/// in the frame builder, which the host path cannot reach: the queue comes
/// first). The window's 4x draw itself is `src/mali/frame_msaa_tests.rs`'s.
#[test]
fn the_4x_window_draw_reaches_the_mali_with_samples_four() {
    let _g = context();
    set_window_samples(4);
    program();
    let s = draw(&FULL);
    assert_eq!(s.samples, 4);
    assert_eq!(s.mali, -ENODEV);
}

/// A blit *into* a multisample framebuffer is refused by the spec.
#[test]
fn a_blit_into_a_multisample_framebuffer_is_invalid_operation() {
    let _g = context();
    let ms = framebuffer(1, false).unwrap();
    let single = framebuffer(0, false).unwrap();
    // SAFETY: plain GL calls on names this test made.
    unsafe {
        glBindFramebuffer(GL_READ_FRAMEBUFFER, single.fb);
        glBindFramebuffer(GL_DRAW_FRAMEBUFFER, ms.fb);
        while glGetError() != GL_NO_ERROR {}
        glBlitFramebuffer(0, 0, 8, 8, 0, 0, 8, 8, GL_COLOR_BUFFER_BIT, GL_NEAREST);
    }
    assert_eq!(err(), GL_INVALID_OPERATION);
}

/* ---- the expected result: ignored until the design lands ------------------- */

/// 4 once the GL core reads the back end's capability (note (b) 1).
#[test]
#[ignore = "note (b) 1, owner GL core (gl-msaa-cap): GL_MAX_SAMPLES is the constant 1 until the Mali sets max_samples 4 under msaa-fbo"]
fn max_samples_is_four() {
    let _g = context();
    assert_eq!(geti(GL_MAX_SAMPLES), 4);
    assert_eq!(internalformat(GL_RGBA8, GL_SAMPLES), 4);
    assert_eq!(internalformat(GL_RGBA8, GL_NUM_SAMPLE_COUNTS), 1);
    assert_eq!(internalformat(GL_RGBA32I, GL_NUM_SAMPLE_COUNTS), 0);
}

/// A 4x RGBA8 FBO drawn with a half-covered triangle: the storage is legal,
/// the draw reaches the Mali with `samples == 4`, and the job-end resolve
/// leaves the averaged edge in the memory.
#[test]
#[ignore = "note (b) 1-4 and (c4), owners GL core (gl-msaa-cap: storage keeps 4, GlesDraw.samples 4) and Mali back end (mali-msaa-fbo: a sample store a target)"]
fn rgba8_4x_fbo_half_covered_edge() {
    let _g = context();
    let fbo = framebuffer(4, false).expect("4 samples are legal storage");
    assert_eq!(rb_samples(fbo.rbs[0]), 4);
    assert_eq!(geti(GL_SAMPLES), 4);
    program();
    clear_black();
    let s = draw(&HALF);
    assert_eq!((s.samples, s.mali), (4, -ENODEV));
    let (e, dst) = resolve_blit(&fbo, false);
    assert_eq!(e, GL_NO_ERROR);
    check_halves(&read_pixels(dst.fb), |x, y| x + y < 7, |x, y| x + y == 7);
}

/// The resolve blit: edge pixels are averaged, the interior is exact.
#[test]
#[ignore = "note (b) 9, owner GL core (gl-msaa-resolve): glBlitFramebuffer from a 4x FBO to a single-sample one, after the job-end resolve of (b) 7"]
fn resolve_blit_averages_the_edge_and_keeps_the_interior() {
    let _g = context();
    let fbo = framebuffer(4, false).expect("4 samples are legal storage");
    program();
    clear_black();
    draw(&HALF);
    let (e, dst) = resolve_blit(&fbo, false);
    assert_eq!(e, GL_NO_ERROR);
    let p = read_pixels(dst.fb);
    let colour = bytes(COLOUR);
    assert_eq!(p[0], colour, "interior");
    assert_eq!(p[63], [0; 4], "outside");
    for c in 0..3 {
        assert!(p[7][c] != 0 && p[7][c] != colour[c], "the diagonal pixel (7,0) is a mean: {:?}", p[7]);
    }
}

/// A draw after a draw in a later job: the edge of the first job is kept
/// (its samples), not smeared by a reload of the resolved colour.
#[test]
#[ignore = "note (c4), owner Mali back end (mali-msaa-fbo): the one-slot SampleBuffer becomes a pool keyed by (target_id, target_gen)"]
fn the_edge_of_an_earlier_job_survives_a_later_one() {
    let _g = context();
    let fbo = framebuffer(4, false).expect("4 samples are legal storage");
    program();
    clear_black();
    draw(&HALF);
    draw(&HALF);
    let (e, dst) = resolve_blit(&fbo, false);
    assert_eq!(e, GL_NO_ERROR);
    check_halves(&read_pixels(dst.fb), |x, y| x + y < 7, |x, y| x + y == 7);
}

/// A 4x depth attachment resolved by the blit keeps sample 0 of each pixel.
/// Board-unverified: the Mali keeps depth samples on chip only (note (c4)),
/// and ES 3.0 4.3.3 leaves the sample to the implementation.
#[test]
#[ignore = "note (b) 8, owners GL core (gl-msaa-resolve: depth blit from 4x) and Mali back end (mali-msaa-fbo); board-unverified: which sample the G610 resolves"]
fn depth_resolve_keeps_sample_zero() {
    let _g = context();
    let fbo = framebuffer(4, true).expect("4 samples are legal storage");
    program();
    // SAFETY: plain GL calls.
    unsafe {
        glEnable(GL_DEPTH_TEST);
        glClearDepthf(1.0);
        glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT);
    }
    let s = draw(&HALF);
    assert_eq!((s.samples, s.mali), (4, -ENODEV));
    assert_ne!(s.depth_id, 0);
    let (e, _dst) = resolve_blit(&fbo, true);
    // SAFETY: plain GL call.
    unsafe { glDisable(GL_DEPTH_TEST) };
    assert_eq!(e, GL_NO_ERROR);
    assert_eq!(
        *DEPTH_ASKED.lock().unwrap(),
        vec![s.depth_id],
        "the depth is read through depth_resolve, one sample a pixel"
    );
}

/// Respecifying a renderbuffer at its size gives it new contents: the stale
/// samples of the old storage, kept for it, are not drawn back (identity by
/// attachment, not by address: the freed memory comes back at the same GPU
/// address in this test's OSAL).
#[test]
#[ignore = "note (b) 5-6, owners GL core (gl-msaa-cap: target_id/target_gen, attachment_gone on respecification) and Mali back end (mali-msaa-fbo: pool keyed by them)"]
fn a_same_size_respecification_drops_the_stale_samples() {
    let _g = context();
    let fbo = framebuffer(4, false).expect("4 samples are legal storage");
    program();
    clear_black();
    let first = draw(&HALF);
    // SAFETY: the renderbuffer this test made, bound as the colour attachment.
    unsafe {
        glBindRenderbuffer(GL_RENDERBUFFER, fbo.rbs[0]);
        glRenderbufferStorageMultisample(GL_RENDERBUFFER, 4, GL_RGBA8, 8, 8);
        assert_eq!(glGetError(), GL_NO_ERROR);
    }
    /* the other half, with no clear: nothing of the first draw may come back */
    let second = draw(&OTHER_HALF);
    assert_eq!(second.gpu, first.gpu, "the premise: the same address and size");
    let (e, dst) = resolve_blit(&fbo, false);
    assert_eq!(e, GL_NO_ERROR);
    check_halves(&read_pixels(dst.fb), |x, y| x + y > 7, |x, y| x + y == 7);
}
