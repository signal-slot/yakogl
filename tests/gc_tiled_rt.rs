// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GC7000 (i.MX8MP): the tiled render targets and early depth of
//! docs/notes/gc-tiled-wire.md W1-W5 (`src/vivante/tiled_rt.rs`, behind the
//! `gc_tiled_rt` test knob, `gc_debug_set_tiled_rt`), seen through the GL
//! API. The lib tests (`src/vivante/backend_tests_tiled_rt.rs`) hand the back
//! end a `GlesDraw`; these make the GL calls an application makes and read
//! the streams the back end submits.
//!
//! The scene is a depth-tested quad into a framebuffer object: an RGBA8
//! texture with a DEPTH_COMPONENT16 renderbuffer, the shape glmark2's
//! shadow and refract draw into. With the knob off it is the stream of
//! before W4; with it on the draw goes to a TILED shadow under
//! SINGLE_BUFFER(2) with early depth, and the same stream holds the resolve
//! engine's tile-in and copy back.
//!
//! How it runs: the real GL core and Vivante back end over a stand-in OSAL,
//! as `gc_tiled_copy_reuse.rs` does. The stand-in also runs a small model of
//! the GPU over each submitted stream, in stream order: the resolve engine's
//! copies and fills (each side linear or 4x4-tiled as RS_CONFIG says, a tiled
//! side's stride times four, Mesa's `etna_compile_rs_state`) and, at each
//! DRAW_INSTANCED, a pixel engine that paints every pixel of the colour
//! surface with a value made from its position, in the layout the PE words
//! name (linear under SINGLE_BUFFER(1), 4x4-tiled under 2). So glReadPixels
//! returns the pattern only when the PE words, the RS jobs and their order
//! agree. It shows the words agree with each other; it does not show what
//! the hardware does (that is the X2's session A).
//!
//! The stand-in hands out the same addresses in every run (the bump
//! allocator restarts with each context, after one warm-up run in the
//! process), so two runs of one scene can be compared word for word.
//!
//! Built without `report-es2` (the library reports ES 3.0); nothing here
//! depends on the report.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online,test-knobs --test gc_tiled_rt
//! ```

#![cfg(all(feature = "host-test", feature = "gpu-vivante", feature = "shaderc-online", feature = "test-knobs"))]

use core::ffi::{c_char, c_void};
use std::collections::BTreeMap;
use std::ffi::CString;
use std::sync::Mutex;
use yakogl::backend::GLES_SURF_RGB565;
use yakogl::gl::consts::*;
use yakogl::gl::draw::*;
use yakogl::gl::framebuffer::*;
use yakogl::gl::program::*;
use yakogl::gl::shader::*;
use yakogl::gl::state::*;
use yakogl::gl::texture::*;
use yakogl::gl::types::*;
use yakogl::gl::vertex::*;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};
use yakogl::vivante::backend::gc_debug_set_tiled_rt;
use yakogl::vivante::kick::{GcKick, GC_KICK_MAGIC};
use yakogl::vivante::regs::*;

/* ---- the stand-in OSAL: host memory, the streams, and a model of the GPU -----------
 *
 * The allocator and submit of tests/gc_tiled_copy_reuse.rs, with the model
 * of the RS and the PE run over each stream in `p_submit`.
 */

struct Alloc {
    mem: Box<[u64]>,
    size: usize,
}

#[derive(Default)]
struct Platform {
    allocs: BTreeMap<u64, Alloc>,
    bump: BTreeMap<u64, u64>,
    fence: i64,
    kicks: Vec<Vec<u32>>,
    faults: Vec<String>,
    /// Run the model over each stream; the PE paints a `w` x `h` window.
    paint: Option<(u32, u32)>,
}

static PLATFORM: Mutex<Option<Platform>> = Mutex::new(None);
/// Serialises the tests: the OSAL pointer, the back end, the knob and
/// `g_backend` are process-global. Holds whether the warm-up ran.
static SERIAL: Mutex<bool> = Mutex::new(false);

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
        p.allocs.insert(gpu, Alloc { mem, size });
        0
    })
}

unsafe extern "C" fn p_gpu_free(mem: *mut OsalGpuMem) -> i32 {
    // SAFETY: the caller passes a live descriptor.
    let m = unsafe { *mem };
    with_platform(|p| {
        if !m.impl_.is_null() && p.allocs.remove(&m.gpu).is_none() {
            p.faults.push(format!("gpu_free of 0x{:x}, which is no live allocation", m.gpu));
        }
    });
    // SAFETY: as above; a freed descriptor is cleared (the OSAL contract).
    unsafe { *mem = OsalGpuMem::ZERO };
    0
}

unsafe extern "C" fn p_cache(_cpu: *mut c_void, _n: usize) {}

unsafe extern "C" fn p_submit(kick: *const c_void, len: usize) -> i64 {
    if len != core::mem::size_of::<GcKick>() {
        with_platform(|p| p.faults.push(format!("submit of a {len}-byte descriptor")));
        return -22;
    }
    // SAFETY: the back end passes a live GcKick of that size.
    let k = unsafe { *(kick as *const GcKick) };
    with_platform(|p| {
        if k.magic != GC_KICK_MAGIC {
            p.faults.push(format!("submit with magic 0x{:x}", k.magic));
            return -22;
        }
        let Some(a) = p.allocs.get(&k.stream_gpu) else {
            p.faults.push(format!("the kick's stream 0x{:x} is no allocation", k.stream_gpu));
            return -22;
        };
        // SAFETY: the allocation holds at least `stream_bytes` bytes.
        let words: Vec<u32> =
            unsafe { core::slice::from_raw_parts(a.mem.as_ptr() as *const u32, k.stream_bytes as usize / 4) }.to_vec();
        if let Some(size) = p.paint {
            run_model(p, &words, size);
        }
        p.kicks.push(words);
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

/* ---- the stream ---------------------------------------------------------------------- */

/// Every LOAD_STATE write of the stream, `(register, value, word index)`, and
/// each DRAW_INSTANCED as `(u32::MAX, 0, word index)`, in order. The part of
/// `backend_tests.rs`'s `walk` this file needs.
fn events(words: &[u32]) -> Vec<(u32, u32, usize)> {
    let mut out = Vec::new();
    let mut i = 0usize;
    while i < words.len() {
        let w = words[i];
        if w & 0xf800_0000 == GC_OP_LOAD_STATE {
            let count = ((w >> 16) & 0x3ff) as usize;
            let base = (w & 0xffff) * 4;
            for k in 0..count {
                if let Some(v) = words.get(i + 1 + k) {
                    out.push((base + 4 * k as u32, *v, i));
                }
            }
            i += 1 + count;
            continue;
        }
        if w & 0xf800_0000 == GC_OP_DRAW_INSTANCED {
            out.push((DRAW, 0, i));
        }
        i += match w & 0xf800_0000 {
            GC_OP_DRAW_INSTANCED => 4,
            GC_OP_STALL => 2,
            _ => 1, /* NOP and the pad words */
        };
    }
    out
}

/// [`events`]' marker of a DRAW_INSTANCED.
const DRAW: u32 = u32::MAX;

/// The value `reg` had at the stream's first draw.
fn at_first_draw(words: &[u32], reg: u32) -> Option<u32> {
    let ev = events(words);
    let d = ev.iter().position(|(r, _, _)| *r == DRAW).expect("a draw");
    ev[..d].iter().rev().find(|(r, _, _)| *r == reg).map(|(_, v, _)| *v)
}

/// Word index of each DRAW_INSTANCED.
fn draws_at(words: &[u32]) -> Vec<usize> {
    events(words).iter().filter(|(r, _, _)| *r == DRAW).map(|(_, _, i)| *i).collect()
}

/// What the resolve engine was told at one RS_KICKER write, and where.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct RsJob {
    config: u32,
    src: u32,
    src_stride: u32,
    dst: u32,
    dst_stride: u32,
    window: u32,
    clear_control: u32,
    fill: u32,
    at: usize,
}

impl RsJob {
    fn note(&mut self, reg: u32, v: u32) {
        match reg {
            GC_RS_CONFIG => self.config = v,
            GC_RS_SOURCE_STRIDE => self.src_stride = v,
            GC_RS_DEST_STRIDE => self.dst_stride = v,
            GC_RS_WINDOW_SIZE => self.window = v,
            GC_RS_CLEAR_CONTROL => self.clear_control = v,
            r if r == rs_fill_value(0) => self.fill = v,
            r if r == rs_pipe_source_addr(0) => self.src = v,
            r if r == rs_pipe_dest_addr(0) => self.dst = v,
            _ => {}
        }
    }
}

/// Every RS kick of the stream, in order.
fn rs_jobs(words: &[u32]) -> Vec<RsJob> {
    let mut cur = RsJob::default();
    let mut out = Vec::new();
    for (reg, v, i) in events(words) {
        cur.note(reg, v);
        if reg == GC_RS_KICKER {
            out.push(RsJob { at: i, ..cur });
        }
    }
    out
}

/* ---- the model of the GPU ---------------------------------------------------------- */

/// Byte offset of 32-bit pixel (x, y) in a surface of `stride`-byte pixel
/// rows: linear, or 4x4 tiles row-major with the pixels row-major inside a
/// tile (etnaviv's TILED; `layout.rs`'s `Layout::Tiled`).
fn offset(tiled: bool, x: u32, y: u32, stride: u32) -> usize {
    if tiled {
        let tile = (y / 4) * (stride / 16) + x / 4;
        (tile * 64 + ((y % 4) * 4 + x % 4) * 4) as usize
    } else {
        (y * stride + x * 4) as usize
    }
}

/// The value the model's PE writes at (x, y): different at every pixel of a
/// target up to 1024x1024, alpha 0xff.
fn paint(x: u32, y: u32) -> u32 {
    0xff00_0000 | (y & 0x3ff) << 10 | (x & 0x3ff)
}

/// The 4 bytes at GPU address `gpu + off` of the stand-in's memory.
fn word_at(p: &mut Platform, gpu: u64, off: usize) -> Option<&mut [u8; 4]> {
    let at = gpu + off as u64;
    let (base, a) = p.allocs.range_mut(..=at).next_back()?;
    let o = (at - base) as usize;
    if o + 4 > a.size {
        return None;
    }
    // SAFETY: inside the allocation (checked just above); any alignment.
    Some(unsafe { &mut *((a.mem.as_mut_ptr() as *mut u8).add(o) as *mut [u8; 4]) })
}

fn run_model(p: &mut Platform, words: &[u32], (w, h): (u32, u32)) {
    let mut regs: BTreeMap<u32, u32> = BTreeMap::new();
    let mut rs = RsJob::default();
    for (reg, v, _) in events(words) {
        if reg == DRAW {
            let reg = |r: u32| regs.get(&r).copied().unwrap_or(0);
            let (addr, stride) = (u64::from(reg(pe_pipe_color_addr(0))), reg(GC_PE_COLOR_STRIDE));
            if reg(GC_PE_COLOR_FORMAT) & (GC_PE_COLOR_FORMAT_SUPER_TILED | GC_PE_COLOR_FORMAT_SUPER_TILED_NEW) != 0 {
                p.faults.push("the model paints no super-tiled colour surface".into());
                continue;
            }
            let tiled = (reg(GC_PE_LOGIC_OP) >> 8) & 3 != 1;
            for y in 0..h {
                for x in 0..w {
                    match word_at(p, addr, offset(tiled, x, y, stride)) {
                        Some(px) => *px = paint(x, y).to_ne_bytes(),
                        None => {
                            p.faults.push(format!("the PE painted ({x},{y}) past its surface at 0x{addr:x}"));
                            return;
                        }
                    }
                }
            }
            continue;
        }
        regs.insert(reg, v);
        rs.note(reg, v);
        if reg == GC_RS_KICKER {
            rs_model(p, &rs);
        }
    }
}

/// The resolve engine: a 32-bit copy or fill of a window, each side linear
/// or tiled as the job says (the stride times 4 for a tiled side).
fn rs_model(p: &mut Platform, j: &RsJob) {
    let (w, h) = (j.window & 0xffff, j.window >> 16);
    let (st, dt) = (j.config & GC_RS_SOURCE_TILED != 0, j.config & GC_RS_DEST_TILED != 0);
    let side = |tiled: bool, stride: u32| if tiled { stride >> 2 } else { stride };
    let (ss, ds) = (side(st, j.src_stride), side(dt, j.dst_stride));
    let fill = j.clear_control & GC_RS_CLEAR_CONTROL_MODE_ENABLED1 != 0;
    for y in 0..h {
        for x in 0..w {
            let v = if fill {
                j.fill.to_ne_bytes()
            } else {
                match word_at(p, u64::from(j.src), offset(st, x, y, ss)) {
                    Some(v) => *v,
                    None => {
                        p.faults.push(format!("the RS read ({x},{y}) past its source 0x{:x}", j.src));
                        return;
                    }
                }
            };
            match word_at(p, u64::from(j.dst), offset(dt, x, y, ds)) {
                Some(d) => *d = v,
                None => {
                    p.faults.push(format!("the RS wrote ({x},{y}) past its destination 0x{:x}", j.dst));
                    return;
                }
            }
        }
    }
}

/* ---- the rig ------------------------------------------------------------------------ */

/// Run `case` with the Vivante back end up on the stand-in OSAL and a fresh
/// `w` x `h` context, the allocator restarted: its window RGBA8888, or
/// RGB565 in a surface this function allocates when `rgb565`. The knob is
/// put back to off at the end.
fn on_gc7000<R>(w: u32, h: u32, paint: bool, rgb565: bool, case: impl FnOnce() -> R) -> R {
    with_platform(|p| {
        p.bump.clear();
        p.kicks.clear();
        p.paint = paint.then_some((w, h));
    });
    // SAFETY: the caller holds SERIAL; the tables are statics.
    let ctx = unsafe {
        osal_set_ops(&OSAL);
        yakogl::backend::g_backend = &yakogl::vivante::backend::yakogl_backend_vivante;
        yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
        assert_eq!(yakogl::vivante::backend::gc_init(), 0, "gc_init");
        let c = if rgb565 {
            let mut surface = OsalGpuMem::ZERO;
            assert_eq!(p_gpu_alloc(WINDOW_HEAP, (w * h * 2) as usize, 0, &mut surface), 0);
            let group = yakogl::gl::context::GROUP_NONE;
            yakogl::gl::context::gl_context_create_for_surface(&mut surface, w, h, GLES_SURF_RGB565, 1, group)
        } else {
            yakogl::gl::context::gl_context_create(w, h)
        };
        assert!(!c.is_null(), "context pool exhausted");
        c
    };
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(case));
    // SAFETY: the context made above.
    unsafe {
        yakogl::gl::context::gl_context_destroy(ctx);
        yakogl::vivante::backend::gc_shutdown();
    }
    gc_debug_set_tiled_rt(0);
    let faults = with_platform(|p| {
        /* leaked, not freed, as in gc_npot_repeat.rs: nothing still pointing
         * at the memory of a run reads freed memory */
        for (_, a) in core::mem::take(&mut p.allocs) {
            core::mem::forget(a.mem);
        }
        p.paint = None;
        core::mem::take(&mut p.faults)
    });
    let r = r.unwrap_or_else(|e| std::panic::resume_unwind(e));
    assert!(faults.is_empty(), "the back end broke the OSAL contract: {faults:?}");
    r
}

/// The lock of [`SERIAL`], after one warm-up scene in the process: the back
/// end's first run makes a page it keeps for the life of the process, so
/// only the runs after it hand out the same addresses.
/// Where the RGB565 window surface is allocated: apart from the back end's
/// heaps.
const WINDOW_HEAP: u64 = 0x7000_0000;

fn serial() -> std::sync::MutexGuard<'static, bool> {
    let mut g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    if !*g {
        draw_scene(64, 64, SCENE);
        *g = true;
    }
    g
}

fn no_error(what: &str) {
    // SAFETY: a plain GL call.
    let e = unsafe { glGetError() };
    assert_eq!(e, GL_NO_ERROR, "{what}: 0x{e:x}");
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

fn program(vs: &str, fs: &str) -> GLuint {
    // SAFETY: plain GL calls.
    unsafe {
        let p = glCreateProgram();
        glAttachShader(p, shader(GL_VERTEX_SHADER, vs));
        glAttachShader(p, shader(GL_FRAGMENT_SHADER, fs));
        glBindAttribLocation(p, 0, c"p".as_ptr());
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        assert_eq!(ok, 1, "link failed");
        p
    }
}

const VS: &str = "attribute vec4 p;\nvoid main() { gl_Position = p; }\n";
const FS: &str = "precision mediump float;\nvoid main() { gl_FragColor = vec4(0.5, 0.25, 0.125, 1.0); }\n";

static QUAD: [f32; 16] = [-1.0, -1.0, 0.5, 1.0, 1.0, -1.0, 0.5, 1.0, -1.0, 1.0, 0.5, 1.0, 1.0, 1.0, 0.5, 1.0];

/// Where the scene draws.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Target {
    /// An RGBA8 texture with a DEPTH_COMPONENT16 renderbuffer: the draw W4
    /// takes.
    Rgba8,
    /// The window (the context's own colour and depth), RGBA8888.
    Window,
    /// An RGB565 window: 16 bits a pixel. The GL core makes no other 16-bit
    /// colour target: an RGB565 texture or colour renderbuffer is stored as
    /// X8R8G8B8 (and takes the shadow), and an RGB565 EGLImage texture is no
    /// complete attachment.
    Window565,
}

/// One scene: the knob (`None`: never set by this run), the target, the
/// depth test, and how many frames (a glFinish after each).
#[derive(Clone, Copy, Debug)]
struct Scene {
    knob: Option<bool>,
    target: Target,
    depth_test: bool,
    frames: u32,
    /// The knob set to this before frame `.0`.
    knob_at: Option<(u32, bool)>,
    /// The depth test set to this before frame `.0`.
    depth_test_at: Option<(u32, bool)>,
}

const SCENE: Scene =
    Scene { knob: None, target: Target::Rgba8, depth_test: true, frames: 1, knob_at: None, depth_test_at: None };

/// What a scene left: each frame's kicks, and the target's pixels read back
/// after the last frame.
struct Run {
    frames: Vec<Vec<Vec<u32>>>,
    pixels: Vec<u8>,
}

impl Run {
    /// The one kick of frame `n`.
    fn kick(&self, n: usize) -> &[u32] {
        assert_eq!(self.frames[n].len(), 1, "frame {n} is one kick");
        &self.frames[n][0]
    }
}

/// Draw `s` into a fresh `w` x `h` context: per frame, a depth clear, then
/// the quad with the depth test as asked, then glFinish.
fn draw_scene(w: u32, h: u32, s: Scene) -> Run {
    on_gc7000(w, h, s.target == Target::Rgba8, s.target == Target::Window565, || {
        if let Some(on) = s.knob {
            gc_debug_set_tiled_rt(on as i32);
        }
        // SAFETY: plain GL calls; null texel data; QUAD is a static.
        unsafe {
            if s.target == Target::Rgba8 {
                let (mut t, mut rb, mut fb) = (0, 0, 0);
                glGenFramebuffers(1, &mut fb);
                glBindFramebuffer(GL_FRAMEBUFFER, fb);
                glGenTextures(1, &mut t);
                glBindTexture(GL_TEXTURE_2D, t);
                glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST as GLint);
                glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST as GLint);
                let (fw, fh) = (w as GLsizei, h as GLsizei);
                glTexImage2D(
                    GL_TEXTURE_2D,
                    0,
                    GL_RGBA as GLint,
                    fw,
                    fh,
                    0,
                    GL_RGBA,
                    GL_UNSIGNED_BYTE,
                    core::ptr::null(),
                );
                glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, t, 0);
                glGenRenderbuffers(1, &mut rb);
                glBindRenderbuffer(GL_RENDERBUFFER, rb);
                glRenderbufferStorage(GL_RENDERBUFFER, GL_DEPTH_COMPONENT16, w as GLsizei, h as GLsizei);
                glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_DEPTH_ATTACHMENT, GL_RENDERBUFFER, rb);
                assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE);
            }
            glViewport(0, 0, w as GLsizei, h as GLsizei);
            let p = program(VS, FS);
            glUseProgram(p);
            glEnableVertexAttribArray(0);
            glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, QUAD.as_ptr() as *const c_void);
            if s.depth_test {
                glEnable(GL_DEPTH_TEST);
                glDepthFunc(GL_LEQUAL);
            }
        }
        no_error("setup");
        let mut frames = Vec::new();
        for f in 0..s.frames {
            if let Some((n, on)) = s.knob_at {
                if n == f {
                    gc_debug_set_tiled_rt(on as i32);
                }
            }
            if let Some((n, on)) = s.depth_test_at {
                if n == f {
                    // SAFETY: plain GL calls.
                    unsafe {
                        if on {
                            glEnable(GL_DEPTH_TEST)
                        } else {
                            glDisable(GL_DEPTH_TEST)
                        }
                    }
                }
            }
            let k0 = with_platform(|p| p.kicks.len());
            // SAFETY: plain GL calls.
            unsafe {
                glClear(GL_DEPTH_BUFFER_BIT);
                glDrawArrays(GL_TRIANGLE_STRIP, 0, 4);
                glFinish();
            }
            no_error(&format!("frame {f}"));
            frames.push(with_platform(|p| p.kicks[k0..].to_vec()));
        }
        let mut pixels = vec![0u8; (w * h * 4) as usize];
        if s.target == Target::Rgba8 {
            // SAFETY: room for w x h RGBA8 pixels.
            unsafe {
                glPixelStorei(GL_PACK_ALIGNMENT, 1);
                glReadPixels(0, 0, w as GLsizei, h as GLsizei, GL_RGBA, GL_UNSIGNED_BYTE, pixels.as_mut_ptr().cast());
            }
            no_error("glReadPixels");
        }
        Run { frames, pixels }
    })
}

/* ---- what the tests read ------------------------------------------------------------ */

/// The resolve engine's copies of a stream (its fills left out: the depth
/// clear is one).
fn rs_copies(words: &[u32]) -> Vec<RsJob> {
    rs_jobs(words).into_iter().filter(|j| j.clear_control & GC_RS_CLEAR_CONTROL_MODE_ENABLED1 == 0).collect()
}

/// PE_LOGIC_OP's SINGLE_BUFFER at the stream's first draw.
fn single_buffer(words: &[u32]) -> u32 {
    (reg_at_draw(words, GC_PE_LOGIC_OP) >> 8) & 3
}

fn reg_at_draw(words: &[u32], reg: u32) -> u32 {
    at_first_draw(words, reg).unwrap_or_else(|| panic!("register 0x{reg:x} was not written before the draw"))
}

const SUPER: u32 = GC_PE_COLOR_FORMAT_SUPER_TILED | GC_PE_COLOR_FORMAT_SUPER_TILED_NEW;
const RGBA8: u32 = GC_RS_FORMAT_A8R8G8B8;

/// `gc_debug_tiled_rt` as it is now.
fn knob() -> i32 {
    // SAFETY: a plain read of an int; every caller holds SERIAL.
    unsafe { core::ptr::addr_of!(yakogl::vivante::backend::gc_debug_tiled_rt).read() }
}

/// Every pixel read back is a different one of the model's: none was lost
/// or written twice on the way.
fn all_different(pixels: &[u8], n: usize) {
    let set: std::collections::BTreeSet<&[u8]> = pixels.chunks(4).collect();
    assert_eq!(set.len(), n, "the pixels read back are not the {n} the model painted");
}

/* ---- the cases ---------------------------------------------------------------------- */

/// (1) Knob off: the depth-tested RGBA8 draw records the stream a run that
/// never touched the knob records, word for word, after a run with the knob
/// on (which took the shadow). The stream of before W4: the storage under
/// SINGLE_BUFFER(1), a linear depth, late depth, no RS copy.
#[test]
fn knob_off_records_the_stream_of_a_knob_never_set() {
    let _g = serial();
    assert_eq!(knob(), 0, "the knob's default");
    let never = draw_scene(64, 64, SCENE);
    let on = draw_scene(64, 64, Scene { knob: Some(true), ..SCENE });
    assert_eq!(single_buffer(on.kick(0)), 2, "the knob-on run took the shadow");
    let off = draw_scene(64, 64, Scene { knob: Some(false), ..SCENE });
    assert_eq!(off.kick(0), never.kick(0), "knob off and knob never set record different words");
    assert_eq!(off.pixels, never.pixels);

    let k = never.kick(0);
    assert_eq!(single_buffer(k), 1, "a linear target: SINGLE_BUFFER(1)");
    assert_eq!(reg_at_draw(k, GC_PE_COLOR_FORMAT) & SUPER, 0);
    assert_eq!(reg_at_draw(k, GC_PE_DEPTH_CONFIG) & (GC_PE_DEPTH_SUPER_TILED | GC_PE_DEPTH_EARLY_Z), 0);
    assert!(rs_copies(k).is_empty(), "no RS copy: {:x?}", rs_copies(k));
    all_different(&never.pixels, 64 * 64);
}

/// (2) Knob on: the same draw renders into a TILED shadow (not the storage,
/// page-aligned, SINGLE_BUFFER(2), no super-tiled bit), and the same kick
/// holds the RS tile-in of the storage before the draw and the RS copy back
/// into the storage after it, at Mesa's strides (a tiled side's is its row
/// stride times four, etnaviv_rs.c:84-132). glReadPixels returns what the
/// model's PE painted into the shadow: the pixels of the knob-off run.
///
/// Deliberate break: drop the copy back (`frame_submit`'s untile) and the
/// pixels read back are the tile-in's, all zero.
#[test]
fn knob_on_draws_into_a_tiled_shadow_the_rs_tiles_in_and_copies_back() {
    let _g = serial();
    let off = draw_scene(64, 64, SCENE);
    let storage = reg_at_draw(off.kick(0), pe_pipe_color_addr(0));
    let on = draw_scene(64, 64, Scene { knob: Some(true), ..SCENE });
    let k = on.kick(0);

    let shadow = reg_at_draw(k, pe_pipe_color_addr(0));
    assert_ne!(shadow, storage, "the draw goes to the shadow");
    assert_eq!(shadow & 0xfff, 0, "a page");
    assert_eq!(reg_at_draw(k, pe_pipe_color_addr(1)), shadow, "both pixel pipes");
    assert_eq!(single_buffer(k), 2, "SINGLE_BUFFER(2)");
    assert_eq!(reg_at_draw(k, GC_PE_COLOR_FORMAT) & SUPER, 0, "TILED, not super-tiled");
    assert_eq!(reg_at_draw(k, GC_PE_COLOR_STRIDE), 64 * 4);
    assert_ne!(reg_at_draw(k, GC_PE_DEPTH_CONFIG) & GC_PE_DEPTH_SUPER_TILED, 0, "a super-tiled depth surface");

    let jobs = rs_copies(k);
    assert_eq!(jobs.len(), 2, "the tile-in and the copy back: {jobs:x?}");
    let (tile_in, back) = (jobs[0], jobs[1]);
    let draws = draws_at(k);
    assert_eq!(draws.len(), 1);
    assert!(tile_in.at < draws[0] && back.at > draws[0], "tile in, draw, copy back");
    assert_eq!((tile_in.src, tile_in.dst), (storage, shadow));
    assert_eq!(tile_in.config, rs_source_format(RGBA8) | rs_dest_format(RGBA8) | GC_RS_DEST_TILED);
    assert_eq!((tile_in.src_stride, tile_in.dst_stride), (64 * 4, (64 * 4) << 2));
    assert_eq!((back.src, back.dst), (shadow, storage));
    assert_eq!(back.config, rs_source_format(RGBA8) | rs_dest_format(RGBA8) | GC_RS_SOURCE_TILED);
    assert_eq!((back.src_stride, back.dst_stride), ((64 * 4) << 2, 64 * 4));
    for j in [tile_in, back] {
        assert_eq!(j.window, rs_window_width(64) | rs_window_height(64));
    }

    all_different(&on.pixels, 64 * 64);
    assert!(on.pixels == off.pixels, "glReadPixels after the shadow draw differs from the linear draw's");
}

/// (3) Knob on: the shadow draw takes early depth, Mesa's words for a
/// depth-tested, depth-writing draw into a tiled target (golden mode 8,
/// tools/vivante/README.md "early Z"): RA_EARLY_DEPTH 0x05000031 (TEST_ENABLE,
/// no WRITE_DISABLE) and PE_DEPTH_CONFIG with EARLY_Z and DISABLE_ZS
/// (0x05051311, the golden of docs/notes/gc-slow-stream-diff.md row 4).
/// Knob off it is late: 0x15000030, and the PE tests.
#[test]
fn knob_on_the_shadow_draw_takes_early_depth() {
    let _g = serial();
    let off = draw_scene(64, 64, SCENE);
    let on = draw_scene(64, 64, Scene { knob: Some(true), ..SCENE });
    let (pe_off, pe_on) = (reg_at_draw(off.kick(0), GC_PE_DEPTH_CONFIG), reg_at_draw(on.kick(0), GC_PE_DEPTH_CONFIG));
    let (ra_off, ra_on) = (reg_at_draw(off.kick(0), GC_RA_EARLY_DEPTH), reg_at_draw(on.kick(0), GC_RA_EARLY_DEPTH));

    assert_eq!(ra_off, 0x1500_0030, "knob off: late depth, the RA writes nothing");
    assert_eq!(pe_off & (GC_PE_DEPTH_EARLY_Z | GC_PE_DEPTH_DISABLE_ZS), 0);
    assert_eq!(ra_on, 0x0500_0031, "Mesa's RA_EARLY_DEPTH");
    assert_ne!(ra_on & GC_RA_EARLY_DEPTH_TEST_ENABLE, 0);
    assert_eq!(ra_on & GC_RA_EARLY_DEPTH_WRITE_DISABLE, 0);
    assert_eq!(pe_on, pe_off | GC_PE_DEPTH_EARLY_Z | GC_PE_DEPTH_DISABLE_ZS | GC_PE_DEPTH_SUPER_TILED);
    assert_eq!(pe_on, 0x0505_1311);
}

/// (4) Knob on: a draw that does not qualify records the knob-off stream word
/// for word: a 16-bit target (an RGB565 window, the one the GL API makes), no
/// depth test, and the RGBA8888 window.
#[test]
fn draws_that_do_not_qualify_are_recorded_as_with_the_knob_off() {
    let _g = serial();
    for (what, s) in [
        ("an RGB565 window", Scene { target: Target::Window565, ..SCENE }),
        ("no depth test", Scene { depth_test: false, ..SCENE }),
        ("the window", Scene { target: Target::Window, ..SCENE }),
    ] {
        let off = draw_scene(64, 64, Scene { knob: Some(false), ..s });
        let on = draw_scene(64, 64, Scene { knob: Some(true), ..s });
        assert_eq!(on.kick(0), off.kick(0), "{what}: the knob changed the stream");
        assert!(rs_copies(on.kick(0)).is_empty(), "{what}: an RS copy");
        assert_ne!(single_buffer(on.kick(0)), 2, "{what}");
        assert_eq!(on.pixels, off.pixels, "{what}");
    }
    let w = draw_scene(64, 64, Scene { knob: Some(true), target: Target::Window565, ..SCENE });
    assert_eq!(reg_at_draw(w.kick(0), GC_PE_COLOR_FORMAT) & 0xf, GC_PE_FORMAT_R5G6B5, "a 16-bit target in the PE");
}

/// (5) The depth surface keeps its layout. Knob on, three frames: one
/// super-tiled depth surface at one address in every frame, and every
/// frame's kick tiles the storage in and copies it back (nothing carries
/// over a kick, tiled_rt.rs). Knob on only from the second frame: the
/// renderbuffer's depth, drawn linear and live, stays linear, behind a whole
/// depth clear too, so the frames record the knob-off run's words (ds_cache.rs, W3: a live
/// plane is never remapped).
#[test]
fn the_depth_surface_keeps_its_layout() {
    let _g = serial();
    let on = draw_scene(64, 64, Scene { knob: Some(true), frames: 3, ..SCENE });
    let depth = reg_at_draw(on.kick(0), pe_pipe_depth_addr(0));
    for f in 0..3 {
        let k = on.kick(f);
        assert_eq!(reg_at_draw(k, pe_pipe_depth_addr(0)), depth, "frame {f}: the same depth surface");
        assert_ne!(reg_at_draw(k, GC_PE_DEPTH_CONFIG) & GC_PE_DEPTH_SUPER_TILED, 0, "frame {f}: super-tiled");
        assert_eq!(single_buffer(k), 2, "frame {f}");
        let jobs = rs_copies(k);
        assert_eq!(jobs.len(), 2, "frame {f}: tile in, copy back: {jobs:x?}");
        assert_ne!(jobs[0].config & GC_RS_DEST_TILED, 0);
        assert_ne!(jobs[1].config & GC_RS_SOURCE_TILED, 0);
    }
    all_different(&on.pixels, 64 * 64);

    let late = draw_scene(64, 64, Scene { knob: Some(false), frames: 3, knob_at: Some((1, true)), ..SCENE });
    let depth = reg_at_draw(late.kick(0), pe_pipe_depth_addr(0));
    for f in 0..3 {
        let k = late.kick(f);
        assert_eq!(reg_at_draw(k, pe_pipe_depth_addr(0)), depth, "frame {f}: the same depth surface");
        assert_eq!(reg_at_draw(k, GC_PE_DEPTH_CONFIG) & GC_PE_DEPTH_SUPER_TILED, 0, "frame {f}: still linear");
        assert_eq!(single_buffer(k), 1, "frame {f}: no shadow without a super-tiled depth");
        assert!(rs_copies(k).is_empty(), "frame {f}");
    }
    let off = draw_scene(64, 64, Scene { knob: Some(false), frames: 3, ..SCENE });
    for f in 1..3 {
        assert_eq!(late.kick(f), off.kick(f), "frame {f}: the knob changed the stream");
    }
}

/// Mesa's PE words for a TILED 800x600 RGBA8 target, single-sampled, no
/// blend, all channels written (Mesa 26.2.1, src/gallium/drivers/etnaviv):
///
/// - PE_COLOR_FORMAT = FORMAT(A8R8G8B8): R8G8B8A8_UNORM's PE format is
///   A8B8G8R8 (etnaviv_format.c:155), A8R8G8B8 | RB_SWAP (:55), and
///   `translate_pe_format` keeps PE_FORMAT_MASK of it (:51, :397); with
///   COMPONENTS__MASK (etnaviv_state.c:287-288), no SUPER_TILED (:247, :289,
///   :295-296: the layout is not super-tiled) and OVERWRITE for one sample
///   (:292-293), merged with the blend state's COMPONENTS(0xf) | OVERWRITE
///   (etnaviv_blend.c:168-169): 0x00010f06, the super-tiled golden's
///   0x00112f06 (tools/vivante/README.md, the PE's view) without its two
///   super-tiled bits.
/// - PE_COLOR_STRIDE = level->stride (etnaviv_state.c:302): the padded width
///   (16 for an RS-aligned TILED layout, etnaviv_resource.c:384-385) times 4
///   bytes (:327-329): 3200.
/// - PE_LOGIC_OP's SINGLE_BUFFER = 2 for a 32-bit target that is not linear
///   (etnaviv_state.c:536-539).
///
/// So pe_overwrite.rs (which owns OVERWRITE and COMPONENTS) and pe_zs.rs need
/// no change for this target: the words are Mesa's. Knob off the same target
/// differs in SINGLE_BUFFER only.
#[test]
fn a_tiled_800x600_rgba8_target_has_mesas_pe_words() {
    let _g = serial();
    let on = draw_scene(800, 600, Scene { knob: Some(true), ..SCENE });
    let k = on.kick(0);
    assert_eq!(reg_at_draw(k, GC_PE_COLOR_FORMAT), 0x0001_0f06);
    assert_eq!(
        reg_at_draw(k, GC_PE_COLOR_FORMAT),
        pe_color_format_format(GC_PE_FORMAT_A8R8G8B8) | pe_color_format_components(0xf) | GC_PE_COLOR_FORMAT_OVERWRITE
    );
    assert_eq!(reg_at_draw(k, GC_PE_COLOR_STRIDE), 3200);
    assert_eq!(single_buffer(k), 2);
    assert_eq!(rs_copies(k).len(), 2, "the tile-in and the copy back");
    all_different(&on.pixels, 800 * 600);

    let off = draw_scene(800, 600, Scene { knob: Some(false), ..SCENE });
    let o = off.kick(0);
    assert_eq!(reg_at_draw(o, GC_PE_COLOR_FORMAT), reg_at_draw(k, GC_PE_COLOR_FORMAT));
    assert_eq!(reg_at_draw(o, GC_PE_COLOR_STRIDE), 3200);
    assert_eq!(
        reg_at_draw(o, GC_PE_LOGIC_OP) ^ reg_at_draw(k, GC_PE_LOGIC_OP),
        pe_logic_op_single_buffer(1) ^ pe_logic_op_single_buffer(2),
        "PE_LOGIC_OP differs in SINGLE_BUFFER only"
    );
    assert!(on.pixels == off.pixels);
}

/// (6) Knob on, a frame with the depth test off into the framebuffer whose
/// depth a shadow frame drew super-tiled. Drawn linear it would ask for the
/// linear layout of a live super-tiled surface, which `ds_cache` refuses
/// (GL_INVALID_OPERATION, nothing recorded, its depth clear lost too); so it
/// keeps the shadow and the surface's layout, with late depth (no test).
///
/// Deliberate break: drop `depth_super` from `tiled_rt_spec`'s condition and
/// the second frame is refused.
#[test]
fn a_draw_without_the_depth_test_after_a_shadow_frame_is_drawn() {
    let _g = serial();
    let r = draw_scene(64, 64, Scene { knob: Some(true), frames: 2, depth_test_at: Some((1, false)), ..SCENE });
    assert_eq!(r.frames[1].len(), 1, "the second frame was drawn");
    let k = r.kick(1);
    assert_eq!(draws_at(k).len(), 1);
    assert_eq!(single_buffer(k), 2, "the shadow is kept");
    assert_eq!(reg_at_draw(k, pe_pipe_depth_addr(0)), reg_at_draw(r.kick(0), pe_pipe_depth_addr(0)));
    assert_ne!(reg_at_draw(k, GC_PE_DEPTH_CONFIG) & GC_PE_DEPTH_SUPER_TILED, 0, "the depth stays super-tiled");
    assert_eq!(reg_at_draw(k, GC_PE_DEPTH_CONFIG) & GC_PE_DEPTH_EARLY_Z, 0, "no early depth without the test");
    assert_eq!(rs_copies(k).len(), 2, "tile in, copy back");
    all_different(&r.pixels, 64 * 64);
}
