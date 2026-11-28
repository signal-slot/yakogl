// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Resolve blits between a 4x framebuffer object and the default framebuffer
//! on the GC7000UL (i.MX8MP), through the GL API into the Vivante back end,
//! on the host (item gc-msaa-window-tests; docs/notes/es3-msaa-renderbuffers.md,
//! GC7000UL section; ES 3.0.6 section 4.3.3 "BlitFramebuffer"). Colour only.
//!
//! The harness (the stand-in OSAL, its GPU model, the recorders and the GL
//! helpers) is copied from tests/es3_msaa_gc.rs; see its header for the
//! model. The host context's window is 64x64 and single-sample: the window
//! here is a plain target for the resolve, drawn nowhere else. The 1x
//! reference is analytic (the stand-in executes no 1x draw): a pixel the quad
//! covers is its colour, one it misses is the clear colour, and a half-covered
//! pixel of a 4x resolve is the average of its samples, within the edge
//! tolerance `near`.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online,msaa-fbo --test es3_msaa_gc_window
//! ```

#![cfg(all(
    feature = "host-test",
    feature = "gpu-vivante",
    feature = "shaderc-online",
    feature = "msaa-fbo",
    not(feature = "report-es2")
))]
#![allow(dead_code)]

use core::ffi::{c_char, c_void};
use std::collections::{BTreeMap, VecDeque};
use std::ffi::CString;
use std::sync::atomic::Ordering;
use std::sync::{Mutex, OnceLock};
use yakogl::backend::*;
use yakogl::gl::consts::*;
use yakogl::gl::draw::*;
use yakogl::gl::framebuffer::*;
use yakogl::gl::program::*;
use yakogl::gl::shader::*;
use yakogl::gl::state::*;
use yakogl::gl::types::*;
use yakogl::gl::vertex::*;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};
use yakogl::vivante::backend::{gc_attachment_gone, gc_draw, yakogl_backend_vivante};
use yakogl::vivante::kick::{GcKick, GC_KICK_MAGIC};
use yakogl::vivante::layout::{self, Layout};
use yakogl::vivante::regs::*;

/// The online compiler's store is one static for the process: every test
/// takes this lock.
static STORE: Mutex<()> = Mutex::new(());

/* ---- the stand-in OSAL: host memory, every stream kept -------------------- */

/// What the PE writes into the samples of a 4x scratch for one draw, in
/// sample coordinates (`sx`, `sy` in `[0, 2 * SIZE)`; pixel `(x, y)` holds
/// samples `(2x..2x+1, 2y..2y+1)`).
enum Layer {
    /// The draw's colour where its coverage is, as GL R,G,B,A (the PE stores
    /// an RGBA8888 target as B,G,R,A, `rt_rgba8888_is_bgra`).
    Paint(Box<dyn Fn(u32, u32) -> Option<[u8; 4]> + Send>),
    /// The reload draw: each pixel's texel of the storage at `src` (rows of
    /// `SIZE` pixels) written into its four samples, bytes as they lie.
    Broadcast { src: u64 },
}

#[derive(Default)]
struct Platform {
    allocs: BTreeMap<u64, Box<[u64]>>,
    /// Each live allocation's heap and size.
    sizes: BTreeMap<u64, (u64, usize)>,
    /// Freed allocations, handed out again to the same heap and size.
    free: Vec<(u64, u64, usize)>,
    bump: BTreeMap<u64, u64>,
    fence: i64,
    kicks: Vec<Vec<u32>>,
    /// The layers the next draws into a 4x scratch take, in order.
    layers: VecDeque<Layer>,
    /// The scratch address of every draw into a 4x scratch, in order.
    ms_draws: Vec<u64>,
    /// Every 2x2 RS resolve: (source, destination).
    resolves: Vec<(u64, u64)>,
    /// What the model could not do (a panic inside the OSAL's `submit`
    /// would abort the process): checked by the test after the case.
    model_errors: Vec<String>,
    /// The 4x target's size in pixels, when it is not `SIZE` x `SIZE`: the
    /// samples a layer paints and the storage pitch a reload reads.
    extent: Option<(u32, u32)>,
    /// When set, every draw into a 4x scratch also writes this into the
    /// scratch's padding (the samples past the target's `2w` x `2h`): what
    /// the padding of a real scratch may hold, which no resolve may carry
    /// into the target.
    pad_garbage: Option<[u8; 4]>,
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
        let gpu = match p.free.iter().rposition(|&(h, _, s)| h == heap_base && s == size) {
            Some(i) => p.free.remove(i).1,
            None => {
                let at = p.bump.entry(heap_base).or_insert(0);
                let gpu = heap_base + 0x1000 + *at;
                *at += (size as u64).div_ceil(4096) * 4096;
                gpu
            }
        };
        let cpu = mem.as_mut_ptr() as *mut c_void;
        // SAFETY: the caller passes a writable descriptor.
        unsafe { *out = OsalGpuMem { cpu, gpu, phys: gpu, size, impl_: cpu } };
        p.allocs.insert(gpu, mem);
        p.sizes.insert(gpu, (heap_base, size));
        0
    })
}

unsafe extern "C" fn p_gpu_free(mem: *mut OsalGpuMem) -> i32 {
    // SAFETY: the caller passes a live descriptor; it is cleared (the OSAL contract).
    unsafe {
        let m = *mem;
        with_platform(|p| {
            /* leaked, not dropped: the back end may still hold a CPU view */
            if let Some(a) = p.allocs.remove(&m.gpu) {
                core::mem::forget(a);
            }
            if let Some((heap, size)) = p.sizes.remove(&m.gpu) {
                p.free.push((heap, m.gpu, size));
            }
        });
        *mem = OsalGpuMem::ZERO;
    }
    0
}

unsafe extern "C" fn p_cache(_cpu: *mut c_void, _n: usize) {}

/// Every `(register, value)` a LOAD_STATE of the stream writes, in order.
fn writes(words: &[u32]) -> Vec<(u32, u32)> {
    let mut out = Vec::new();
    walk(words, |reg, v| out.push((reg, v)), || ());
    out
}

/// Walk a stream: `state` for every register write, `draw` for every
/// DRAW_INSTANCED, in order.
fn walk(words: &[u32], mut state: impl FnMut(u32, u32), mut draw: impl FnMut()) {
    let mut i = 0usize;
    while i < words.len() {
        let w = words[i];
        if w & 0xf800_0000 == GC_OP_LOAD_STATE {
            let count = ((w >> 16) & 0x3ff) as usize;
            let base = (w & 0xffff) * 4;
            for k in 0..count {
                if let Some(v) = words.get(i + 1 + k) {
                    state(base + 4 * k as u32, *v);
                }
            }
            i += 1 + count;
            continue;
        }
        i += match w & 0xf800_0000 {
            GC_OP_DRAW_INSTANCED => {
                draw();
                4
            }
            GC_OP_STALL => 2,
            _ => 1,
        };
    }
}

/// The bytes `[gpu, gpu + len)` of the allocation holding them.
fn bytes(p: &mut Platform, gpu: u64, len: usize) -> &mut [u8] {
    let (&base, a) = p.allocs.range_mut(..=gpu).next_back().expect("an address in no allocation");
    // SAFETY: the allocation's own bytes.
    let all = unsafe { core::slice::from_raw_parts_mut(a.as_mut_ptr() as *mut u8, a.len() * 8) };
    let off = (gpu - base) as usize;
    &mut all[off..off + len]
}

/// The GPU model of the module doc, over one submitted stream.
fn run_model(words: &[u32], p: &mut Platform) {
    let regs: std::cell::RefCell<BTreeMap<u32, u32>> = Default::default();
    let jobs: std::cell::RefCell<Vec<Job>> = Default::default();
    enum Job {
        Draw(BTreeMap<u32, u32>),
        Rs(BTreeMap<u32, u32>),
    }
    walk(
        words,
        |reg, v| {
            regs.borrow_mut().insert(reg, v);
            if reg == GC_RS_KICKER {
                jobs.borrow_mut().push(Job::Rs(regs.borrow().clone()));
            }
        },
        || jobs.borrow_mut().push(Job::Draw(regs.borrow().clone())),
    );
    let r = |m: &BTreeMap<u32, u32>, reg: u32| m.get(&reg).copied().unwrap_or(0);
    for job in jobs.into_inner() {
        match job {
            Job::Draw(m) => {
                if r(&m, GC_PE_COLOR_FORMAT) & GC_PE_COLOR_FORMAT_SUPER_TILED == 0 {
                    continue; /* a single-sampled draw: not modelled */
                }
                let (addr, pitch) = (u64::from(r(&m, pe_pipe_color_addr(0))), r(&m, GC_PE_COLOR_STRIDE));
                p.ms_draws.push(addr);
                let layer = p.layers.pop_front().expect("a 4x draw the test did not expect");
                let (tw, th) = p.extent.unwrap_or((SIZE, SIZE));
                let (nx, ny) = (2 * tw, 2 * th);
                /* the storage's row in pixels: whole pixel pairs (gl/context.rs `tex_pitch`) */
                let storage_px = (tw + 1) & !1;
                let garbage = p.pad_garbage;
                let (gx, gy) = match garbage {
                    Some(_) => (pitch / 4, yakogl::msaa_hw::gc::scratch_rows(th)),
                    None => (nx, ny),
                };
                for sy in 0..gy {
                    for sx in 0..gx {
                        if sx >= nx || sy >= ny {
                            let off = layout::offset(Layout::SuperTiled, sx, sy, 4, pitch) as u64;
                            bytes(p, addr + off, 4).copy_from_slice(&garbage.unwrap());
                            continue;
                        }
                        let px = match &layer {
                            Layer::Paint(f) => match f(sx, sy) {
                                Some([r, g, b, a]) => [b, g, r, a],
                                None => continue,
                            },
                            Layer::Broadcast { src } => {
                                let at = *src + u64::from((sy / 2) * storage_px * 4 + (sx / 2) * 4);
                                bytes(p, at, 4).try_into().unwrap()
                            }
                        };
                        let off = layout::offset(Layout::SuperTiled, sx, sy, 4, pitch) as u64;
                        bytes(p, addr + off, 4).copy_from_slice(&px);
                    }
                }
            }
            Job::Rs(m) => {
                let both = GC_RS_DOWNSAMPLE_X | GC_RS_DOWNSAMPLE_Y;
                let config = r(&m, GC_RS_CONFIG);
                if config & both != both {
                    continue; /* not a resolve: not modelled */
                }
                assert_ne!(config & GC_RS_SOURCE_TILED, 0, "a resolve reads the supertiled scratch");
                assert_eq!(config & 0x1f, GC_RS_FORMAT_A8R8G8B8, "RGBA8 and sRGB resolve as A8R8G8B8");
                let src = u64::from(r(&m, rs_pipe_source_addr(0)));
                let dst = u64::from(r(&m, rs_pipe_dest_addr(0)));
                let sstride = (r(&m, GC_RS_SOURCE_STRIDE) & !(0x8000_0000 | 0x0800_0000)) >> 2;
                let dstride = r(&m, GC_RS_DEST_STRIDE);
                let win = r(&m, GC_RS_WINDOW_SIZE);
                let (w, h) = (win & 0xffff, win >> 16);
                p.resolves.push((src, dst));
                for y in 0..h / 2 {
                    for x in 0..w / 2 {
                        let mut sum = [0u32; 4];
                        for (i, j) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                            let off = layout::offset(Layout::SuperTiled, 2 * x + i, 2 * y + j, 4, sstride) as u64;
                            for (s, b) in sum.iter_mut().zip(bytes(p, src + off, 4).iter()) {
                                *s += u32::from(*b);
                            }
                        }
                        let px = sum.map(|s| ((s + 2) / 4) as u8);
                        bytes(p, dst + u64::from(y * dstride + x * 4), 4).copy_from_slice(&px);
                    }
                }
            }
        }
    }
}

unsafe extern "C" fn p_submit(kick: *const c_void, len: usize) -> i64 {
    if len != core::mem::size_of::<GcKick>() {
        return -22;
    }
    // SAFETY: the back end passes a live GcKick of that size.
    let k = unsafe { *(kick as *const GcKick) };
    with_platform(|p| {
        if k.magic != GC_KICK_MAGIC {
            return -22;
        }
        let Some(a) = p.allocs.get(&k.stream_gpu) else { return -22 };
        // SAFETY: the allocation holds at least `stream_bytes` bytes.
        let words: Vec<u32> =
            unsafe { core::slice::from_raw_parts(a.as_ptr() as *const u32, k.stream_bytes as usize / 4) }.to_vec();
        if let Err(e) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run_model(&words, p))) {
            let why = e.downcast_ref::<String>().cloned().or_else(|| e.downcast_ref::<&str>().map(|s| s.to_string()));
            p.model_errors.push(why.unwrap_or_else(|| "the GPU model failed".into()));
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

/* ---- the recorders: what the GL core hands the back end -------------------- */

/// One draw as the back end got it.
#[derive(Clone, Debug)]
struct Seen {
    /// An application draw (vertices), not a clear alone.
    draws: bool,
    /// `GlesDraw::samples`.
    samples: u8,
    /// `targets[..num_targets]`: (gpu, format).
    targets: Vec<(u64, u8)>,
    /// `GlesDraw::target_w`, `target_h`.
    size: (u32, u32),
    /// `GlesDraw::target_id`, `target_gen`.
    ident: (u32, u32),
}

static SEEN: Mutex<Vec<Seen>> = Mutex::new(Vec::new());
/// The ids `attachment_gone` was called with.
static GONE: Mutex<Vec<u32>> = Mutex::new(Vec::new());

unsafe extern "C" fn rec_draw(d: *const GlesDraw) -> i32 {
    let g = &*d;
    let n = (g.num_targets.max(1) as usize).min(GLES_MAX_TARGETS);
    SEEN.lock().unwrap().push(Seen {
        draws: g.index_count != 0,
        samples: g.samples,
        targets: g.targets[..n].iter().map(|t| (t.gpu, t.format)).collect(),
        size: (g.target_w, g.target_h),
        ident: (g.target_id, g.target_gen),
    });
    gc_draw(d)
}

unsafe extern "C" fn rec_gone(id: u32) {
    GONE.lock().unwrap().push(id);
    gc_attachment_gone(id)
}

/// The Vivante table with the two recorders in; built once, from the real
/// table, so a field added to `YakoglBackend` needs no change here.
fn table() -> &'static YakoglBackend {
    static T: OnceLock<usize> = OnceLock::new();
    let p = *T.get_or_init(|| {
        // SAFETY: a bitwise copy of a static table of plain data (pointers and
        // function pointers); the copy is leaked and never mutated again.
        let mut t = unsafe { core::ptr::read(&yakogl_backend_vivante) };
        t.draw = Some(rec_draw);
        t.attachment_gone = Some(rec_gone);
        Box::leak(Box::new(t)) as *const YakoglBackend as usize
    });
    // SAFETY: the leaked table above.
    unsafe { &*(p as *const YakoglBackend) }
}

/// Run `case` with the Vivante back end up on the stand-in OSAL and a fresh
/// 64x64 context; the lock serialises the tests.
fn on_gc7000(case: impl FnOnce()) {
    let _g = STORE.lock().unwrap_or_else(|e| e.into_inner());
    SEEN.lock().unwrap().clear();
    GONE.lock().unwrap().clear();
    // SAFETY: the guard serialises the tests; the tables are statics.
    let ctx = unsafe {
        osal_set_ops(&OSAL);
        yakogl::backend::g_backend = table();
        yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
        assert_eq!(yakogl::vivante::backend::gc_init(), 0, "gc_init");
        let c = yakogl::gl::context::gl_context_create(64, 64);
        assert!(!c.is_null(), "context pool exhausted");
        c
    };
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(case));
    let model_errors = with_platform(|p| core::mem::take(&mut p.model_errors));
    // SAFETY: the context made above.
    unsafe {
        yakogl::gl::context::gl_context_destroy(ctx);
        yakogl::vivante::backend::gc_shutdown();
    }
    HOST_DRAW_TARGETS.store(0, Ordering::Relaxed);
    with_platform(|p| {
        /* leaked, not freed, as tests/es3_mrt4_gc.rs */
        for (_, m) in core::mem::take(&mut p.allocs) {
            core::mem::forget(m);
        }
        p.sizes.clear();
        p.free.clear();
        p.layers.clear();
        p.ms_draws.clear();
        p.resolves.clear();
        p.extent = None;
        p.pad_garbage = None;
    });
    if let Err(e) = r {
        std::panic::resume_unwind(e);
    }
    assert!(model_errors.is_empty(), "the GPU model: {model_errors:?}");
}

/* ---- GL helpers ------------------------------------------------------------ */

/// A multiple of 16 wide and of 4 high: the RS can resolve it (`ms_resolvable`).
const SIZE: u32 = 16;

fn err() -> GLenum {
    // SAFETY: a plain GL call.
    unsafe { glGetError() }
}

fn clear_errors() {
    while err() != GL_NO_ERROR {}
}

fn geti(pname: GLenum) -> GLint {
    let mut v = -1;
    // SAFETY: one value.
    unsafe { glGetIntegerv(pname, &mut v) };
    v
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

/// Use a program writing the constant `rgba`.
fn colour_program(rgba: [f32; 4]) {
    const VS: &str = "#version 300 es\nin vec4 p;\nvoid main() { gl_Position = p; }\n";
    let fs = format!(
        "#version 300 es\nprecision highp float;\nlayout(location = 0) out vec4 o;\nvoid main() {{ o = vec4({:?}, {:?}, {:?}, {:?}); }}\n",
        rgba[0], rgba[1], rgba[2], rgba[3]
    );
    // SAFETY: plain GL calls.
    unsafe {
        let p = glCreateProgram();
        glAttachShader(p, shader(GL_VERTEX_SHADER, VS));
        glAttachShader(p, shader(GL_FRAGMENT_SHADER, &fs));
        glBindAttribLocation(p, 0, c"p".as_ptr());
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        assert_eq!(ok, 1, "link failed");
        glUseProgram(p);
    }
}

/// The error of `glRenderbufferStorageMultisample(.., samples, format, SIZE, SIZE)`
/// on a fresh renderbuffer, and the renderbuffer.
fn storage(samples: GLsizei, format: GLenum) -> (GLenum, GLuint) {
    let mut rb = 0;
    // SAFETY: plain GL calls.
    unsafe {
        glGenRenderbuffers(1, &mut rb);
        glBindRenderbuffer(GL_RENDERBUFFER, rb);
    }
    clear_errors();
    // SAFETY: a plain GL call.
    unsafe { glRenderbufferStorageMultisample(GL_RENDERBUFFER, samples, format, SIZE as GLsizei, SIZE as GLsizei) };
    (err(), rb)
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

struct Fbo {
    fb: GLuint,
    colour: GLuint,
    depth: Option<GLuint>,
}

/// A bound `SIZE` x `SIZE` framebuffer with a `colour_format` renderbuffer of
/// `samples` and, when `depth` names a format, a depth renderbuffer of the
/// same count. A storage error comes back as `Err(error)`.
fn framebuffer_of(samples: GLsizei, colour_format: GLenum, depth: Option<GLenum>) -> Result<Fbo, GLenum> {
    let mut fb = 0;
    // SAFETY: plain GL calls.
    unsafe {
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
    }
    let (e, colour) = storage(samples, colour_format);
    if e != GL_NO_ERROR {
        return Err(e);
    }
    // SAFETY: a renderbuffer this test made.
    unsafe { glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_RENDERBUFFER, colour) };
    let depth = match depth {
        Some(f) => {
            let (e, rb) = storage(samples, f);
            if e != GL_NO_ERROR {
                return Err(e);
            }
            let attach = if f == GL_DEPTH24_STENCIL8 { GL_DEPTH_STENCIL_ATTACHMENT } else { GL_DEPTH_ATTACHMENT };
            // SAFETY: a renderbuffer this test made.
            unsafe { glFramebufferRenderbuffer(GL_FRAMEBUFFER, attach, GL_RENDERBUFFER, rb) };
            Some(rb)
        }
        None => None,
    };
    // SAFETY: a plain GL call.
    let status = unsafe { glCheckFramebufferStatus(GL_FRAMEBUFFER) };
    assert_eq!(status, GL_FRAMEBUFFER_COMPLETE, "samples {samples}");
    Ok(Fbo { fb, colour, depth })
}

/// [`framebuffer_of`] with an RGBA8 colour renderbuffer.
fn framebuffer(samples: GLsizei, depth: Option<GLenum>) -> Result<Fbo, GLenum> {
    framebuffer_of(samples, GL_RGBA8, depth)
}

/// Draw the quad `[left, right] x [-1, 1]` at depth `z` in `rgba` and hand
/// back what the back end was given for it. The NDC x of pixel edge `px` is
/// `px / SIZE * 2 - 1`.
fn draw_quad(left: f32, right: f32, z: f32, rgba: [f32; 4]) -> Seen {
    let quad = [left, -1.0, z, 1.0, right, -1.0, z, 1.0, left, 1.0, z, 1.0, right, 1.0, z, 1.0];
    colour_program(rgba);
    SEEN.lock().unwrap().clear();
    // SAFETY: plain GL calls; `quad` outlives the finish.
    unsafe {
        glViewport(0, 0, SIZE as GLsizei, SIZE as GLsizei);
        glEnableVertexAttribArray(0);
        glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, quad.as_ptr() as *const c_void);
        glDrawArrays(GL_TRIANGLE_STRIP, 0, 4);
        glFinish();
    }
    assert_eq!(err(), GL_NO_ERROR);
    let s = core::mem::take(&mut *SEEN.lock().unwrap());
    s.into_iter().rev().find(|s| s.draws).expect("the draw reached the back end")
}

/// Draw the quad `[-1, right] x [-1, 1]` (see [`draw_quad`]).
fn draw_left_of(right: f32, z: f32, rgba: [f32; 4]) -> Seen {
    draw_quad(-1.0, right, z, rgba)
}

/// A whole-target draw of `rgba`.
fn draw(rgba: [f32; 4]) -> Seen {
    draw_left_of(1.0, 0.5, rgba)
}

fn clear(rgba: [f32; 4]) {
    // SAFETY: plain GL calls.
    unsafe {
        glClearColor(rgba[0], rgba[1], rgba[2], rgba[3]);
        glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT | GL_STENCIL_BUFFER_BIT);
    }
}

/// Resolve blit of `read` into a fresh single-sample framebuffer of the same
/// size (with a depth/stencil renderbuffer), `mask` as the buffers; the GL
/// error and the target, left bound as the read framebuffer.
fn resolve_blit(read: &Fbo, mask: GLbitfield) -> (GLenum, Fbo) {
    let dst = framebuffer(0, Some(GL_DEPTH24_STENCIL8)).expect("a single-sample target");
    // SAFETY: plain GL calls on names this test made.
    unsafe {
        glBindFramebuffer(GL_READ_FRAMEBUFFER, read.fb);
        glBindFramebuffer(GL_DRAW_FRAMEBUFFER, dst.fb);
        glReadBuffer(GL_COLOR_ATTACHMENT0);
    }
    clear_errors();
    let n = SIZE as GLint;
    // SAFETY: a plain GL call.
    unsafe { glBlitFramebuffer(0, 0, n, n, 0, 0, n, n, mask, GL_NEAREST) };
    let e = err();
    // SAFETY: a plain GL call.
    unsafe { glBindFramebuffer(GL_READ_FRAMEBUFFER, dst.fb) };
    (e, dst)
}

/// RGBA8 of pixel (`x`, 8) of the bound read framebuffer.
fn pixel(x: i32) -> [u8; 4] {
    let mut p = [0xeeu8; 4];
    // SAFETY: four writable bytes.
    unsafe { glReadPixels(x, 8, 1, 1, GL_RGBA, GL_UNSIGNED_BYTE, p.as_mut_ptr() as *mut c_void) };
    p
}

/// The register writes of the streams kicked since `mark`, after a finish.
fn kicks_since(mark: usize) -> Vec<(u32, u32)> {
    // SAFETY: a plain GL call.
    unsafe { glFinish() };
    with_platform(|p| p.kicks[mark..].iter().flat_map(|k| writes(k)).collect())
}

/// Finish, and the number of streams kicked so far.
fn kicked() -> usize {
    // SAFETY: a plain GL call.
    unsafe { glFinish() };
    with_platform(|p| p.kicks.len())
}

/// Does a write of RS_CONFIG ask for the 2x2 downsample?
fn downsamples(w: &[(u32, u32)]) -> bool {
    let both = GC_RS_DOWNSAMPLE_X | GC_RS_DOWNSAMPLE_Y;
    w.iter().any(|(r, v)| *r == GC_RS_CONFIG && v & both == both)
}

const RED: [f32; 4] = [1.0, 0.0, 0.0, 1.0];
const BLUE: [f32; 4] = [0.0, 0.0, 1.0, 1.0];
const BLACK: [f32; 4] = [0.0, 0.0, 0.0, 1.0];

/// NDC x of the right edge of the quad that half-covers pixel column 8: the
/// edge runs through the middle of the pixel, so two of its four samples (any
/// of the 4x patterns split a pixel evenly on a vertical line) are inside.
const HALF_EDGE_X: f32 = 8.5 / SIZE as f32 * 2.0 - 1.0;

/* ---- msaa-fbo: 4x framebuffer objects ----------------------------------------- */

/// The pixel `[r, g, b, a]` lies within one step of `want` on each channel
/// (the RS's rounding of an average of two is a board fact).
fn near(got: [u8; 4], want: [u8; 4]) -> bool {
    got.iter().zip(want).all(|(g, w)| g.abs_diff(w) <= 1)
}

/// Queue the layers the next 4x draws write (module doc, the GPU model).
fn expect_layers(layers: impl IntoIterator<Item = Layer>) {
    with_platform(|p| p.layers.extend(layers));
}

/// Every sample in `rgba`.
fn all(rgba: [u8; 4]) -> Layer {
    Layer::Paint(Box::new(move |_, _| Some(rgba)))
}

/// The samples of the quad `[-1, HALF_EDGE_X]`: sample columns 0..=16, pixel
/// 8's left two samples (the edge at pixel 8.5 puts sample column 16, centre
/// 8.25, inside and column 17, centre 8.75, outside).
fn left_half_edge(rgba: [u8; 4]) -> Layer {
    Layer::Paint(Box::new(move |sx, _| (sx <= 16).then_some(rgba)))
}

/// The samples of the quad `[HALF_EDGE_X, 1]`: sample columns 17 and on.
fn right_half_edge(rgba: [u8; 4]) -> Layer {
    Layer::Paint(Box::new(move |sx, _| (sx >= 17).then_some(rgba)))
}

const RED8: [u8; 4] = [255, 0, 0, 255];
const BLUE8: [u8; 4] = [0, 0, 255, 255];
const BLACK8: [u8; 4] = [0, 0, 0, 255];

/// The draws into a 4x scratch since `mark` (their scratch addresses), and
/// whether every queued layer was taken.
fn ms_draws_since(mark: usize) -> Vec<u64> {
    // SAFETY: a plain GL call.
    unsafe { glFinish() };
    with_platform(|p| {
        assert!(p.model_errors.is_empty(), "the GPU model: {:?}", p.model_errors);
        assert!(p.layers.is_empty(), "{} queued 4x draws were not made", p.layers.len());
        p.ms_draws[mark..].to_vec()
    })
}

fn ms_draws() -> usize {
    // SAFETY: a plain GL call.
    unsafe { glFinish() };
    with_platform(|p| p.ms_draws.len())
}

/* ---- the window ---------------------------------------------------------------- */

/// RGBA8 of the window's pixels `[0, SIZE)` x `[0, SIZE)`, row by row.
fn window_block() -> Vec<[u8; 4]> {
    let mut out = Vec::new();
    // SAFETY: plain GL calls; four writable bytes per read.
    unsafe { glBindFramebuffer(GL_READ_FRAMEBUFFER, 0) };
    for y in 0..SIZE as i32 {
        for x in 0..SIZE as i32 {
            let mut p = [0xeeu8; 4];
            // SAFETY: four writable bytes.
            unsafe { glReadPixels(x, y, 1, 1, GL_RGBA, GL_UNSIGNED_BYTE, p.as_mut_ptr() as *mut c_void) };
            out.push(p);
        }
    }
    assert_eq!(err(), GL_NO_ERROR, "reading the window");
    out
}

fn at(block: &[[u8; 4]], x: usize, y: usize) -> [u8; 4] {
    block[y * SIZE as usize + x]
}

/// `glBlitFramebuffer` from `read` to the window (`dst` = x0, y0, x1, y1) of
/// the colour buffer; the GL error.
fn blit_to_window(read: GLuint, src: [GLint; 4], dst: [GLint; 4]) -> GLenum {
    // SAFETY: plain GL calls on names this test made.
    unsafe {
        glBindFramebuffer(GL_READ_FRAMEBUFFER, read);
        glBindFramebuffer(GL_DRAW_FRAMEBUFFER, 0);
        glReadBuffer(GL_COLOR_ATTACHMENT0);
    }
    clear_errors();
    // SAFETY: a plain GL call.
    unsafe {
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
        )
    };
    err()
}

const FULL: [GLint; 4] = [0, 0, SIZE as GLint, SIZE as GLint];

/// A 4x FBO drawn whole red (the reload draw, then the draw) and left bound.
fn drawn_red_4x() -> Fbo {
    let fbo = framebuffer(4, None).expect("4 samples are legal storage");
    colour_program(RED);
    let quad = [-1.0f32, -1.0, 0.5, 1.0, 1.0, -1.0, 0.5, 1.0, -1.0, 1.0, 0.5, 1.0, 1.0, 1.0, 0.5, 1.0];
    SEEN.lock().unwrap().clear();
    // SAFETY: plain GL calls; `quad` outlives the finish below.
    unsafe {
        glViewport(0, 0, SIZE as GLsizei, SIZE as GLsizei);
        glEnableVertexAttribArray(0);
        glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, quad.as_ptr() as *const c_void);
        glDrawArrays(GL_TRIANGLE_STRIP, 0, 4);
    }
    let target =
        SEEN.lock().unwrap().iter().rev().find(|s| s.draws).expect("the draw reached the back end").targets[0].0;
    expect_layers([Layer::Broadcast { src: target }, all(RED8)]);
    // SAFETY: a plain GL call.
    unsafe { glFinish() };
    fbo
}

/* ---- (a) a 4x FBO resolves into the window --------------------------------------- */

/// A 4x FBO drawn whole red, blitted to framebuffer 0 with the same
/// rectangle: every window pixel of the rectangle is the 1x reference (red),
/// and the window beyond the rectangle is untouched.
#[test]
fn a_4x_draw_resolves_into_the_window() {
    on_gc7000(|| {
        let before = {
            let b = window_block();
            // SAFETY: a plain GL call.
            unsafe { glBindFramebuffer(GL_READ_FRAMEBUFFER, 0) };
            let mut p = [0xeeu8; 4];
            // SAFETY: four writable bytes.
            unsafe { glReadPixels(SIZE as GLint, 8, 1, 1, GL_RGBA, GL_UNSIGNED_BYTE, p.as_mut_ptr() as *mut c_void) };
            (b, p)
        };
        let fbo = drawn_red_4x();
        assert_eq!(blit_to_window(fbo.fb, FULL, FULL), GL_NO_ERROR);
        assert!(window_block().iter().all(|p| near(*p, RED8)), "the window holds the red draw");
        // SAFETY: four writable bytes.
        let mut p = [0xeeu8; 4];
        unsafe { glReadPixels(SIZE as GLint, 8, 1, 1, GL_RGBA, GL_UNSIGNED_BYTE, p.as_mut_ptr() as *mut c_void) };
        assert_eq!(p, before.1, "a pixel outside the rectangle is untouched");
    });
}

/// A half-covered edge: the 4x FBO is cleared black and drawn red left of
/// pixel 8.5; its resolve into the window is red inside, black outside and
/// the average (two of four samples) at the edge pixel.
#[test]
fn a_half_covered_edge_resolves_into_the_window() {
    on_gc7000(|| {
        let fbo = framebuffer(4, None).expect("4 samples are legal storage");
        expect_layers([all(BLACK8), left_half_edge(RED8)]);
        clear(BLACK);
        let s = draw_left_of(HALF_EDGE_X, 0.5, RED);
        assert_eq!(s.samples, 4);
        assert_eq!(blit_to_window(fbo.fb, FULL, FULL), GL_NO_ERROR);
        let w = window_block();
        for y in [0, 8, 15] {
            assert!(near(at(&w, 3, y), RED8), "interior {:?}", at(&w, 3, y));
            assert!(near(at(&w, 13, y), BLACK8), "outside {:?}", at(&w, 13, y));
            assert!(near(at(&w, 8, y), [128, 0, 0, 255]), "edge {:?}", at(&w, 8, y));
        }
    });
}

/// A sub-rectangle with the same source and destination corners resolves
/// only those pixels: the rest of the window keeps what it had.
#[test]
fn a_matching_sub_rectangle_resolves_only_itself() {
    on_gc7000(|| {
        let fbo = drawn_red_4x();
        let before = window_block();
        assert_eq!(blit_to_window(fbo.fb, [4, 4, 12, 12], [4, 4, 12, 12]), GL_NO_ERROR);
        let w = window_block();
        for y in 0..SIZE as usize {
            for x in 0..SIZE as usize {
                if (4..12).contains(&x) && (4..12).contains(&y) {
                    assert!(near(at(&w, x, y), RED8), "inside ({x},{y}) {:?}", at(&w, x, y));
                } else {
                    assert_eq!(at(&w, x, y), at(&before, x, y), "outside ({x},{y}) changed");
                }
            }
        }
    });
}

/* ---- (b) the ES 3.0 errors of a resolve into the window ---------------------------- */

/// A scaled rectangle, a rectangle at another place, and a 4x format the
/// window does not have each give `GL_INVALID_OPERATION` and leave the window
/// as it was (here: red from a first, legal resolve).
#[test]
fn an_invalid_resolve_into_the_window_changes_nothing() {
    on_gc7000(|| {
        let fbo = drawn_red_4x();
        assert_eq!(blit_to_window(fbo.fb, FULL, FULL), GL_NO_ERROR);
        let red = window_block();

        /* scaled: the destination is twice the source */
        assert_eq!(
            blit_to_window(fbo.fb, FULL, [0, 0, 2 * SIZE as GLint, 2 * SIZE as GLint]),
            GL_INVALID_OPERATION,
            "scaled"
        );
        assert_eq!(window_block(), red, "scaled: the window is unchanged");

        /* scaled the other way, and mirrored */
        assert_eq!(
            blit_to_window(fbo.fb, FULL, [0, 0, SIZE as GLint / 2, SIZE as GLint / 2]),
            GL_INVALID_OPERATION,
            "shrunk"
        );
        assert_eq!(
            blit_to_window(fbo.fb, FULL, [SIZE as GLint, 0, 0, SIZE as GLint]),
            GL_INVALID_OPERATION,
            "mirrored"
        );
        assert_eq!(window_block(), red, "shrunk, mirrored: the window is unchanged");

        /* mismatched: the same size at another place */
        assert_eq!(blit_to_window(fbo.fb, [0, 0, 8, 8], [8, 8, 16, 16]), GL_INVALID_OPERATION, "moved");
        assert_eq!(window_block(), red, "moved: the window is unchanged");

        /* mismatched format: an RGB565 4x FBO into the RGBA8 window */
        let rgb565 = framebuffer_of(4, GL_RGB565, None).expect("a 4x RGB565 FBO");
        assert_eq!(blit_to_window(rgb565.fb, FULL, FULL), GL_INVALID_OPERATION, "format");
        assert_eq!(window_block(), red, "format: the window is unchanged");

        /* and the legal blit still works after the errors */
        assert_eq!(blit_to_window(fbo.fb, FULL, FULL), GL_NO_ERROR);
        assert_eq!(window_block(), red);
    });
}

/* ---- (c) the window into a single-sample FBO ----------------------------------------- */

/// A blit from the window into a 1x FBO is a plain copy, as before the 4x
/// FBOs: no error, and the RS does not downsample (the window is single
/// sample here), the same rectangle or scaled.
#[test]
fn a_window_blit_into_a_1x_fbo_is_a_plain_copy() {
    on_gc7000(|| {
        let ms = drawn_red_4x();
        assert_eq!(blit_to_window(ms.fb, FULL, FULL), GL_NO_ERROR);
        let one = framebuffer(0, None).expect("a single-sample FBO");
        let m = kicked();
        // SAFETY: plain GL calls on names this test made.
        unsafe {
            glBindFramebuffer(GL_READ_FRAMEBUFFER, 0);
            glBindFramebuffer(GL_DRAW_FRAMEBUFFER, one.fb);
        }
        clear_errors();
        let n = SIZE as GLint;
        // SAFETY: a plain GL call.
        unsafe { glBlitFramebuffer(0, 0, n, n, 0, 0, n, n, GL_COLOR_BUFFER_BIT, GL_NEAREST) };
        assert_eq!(err(), GL_NO_ERROR, "same rectangle");
        // SAFETY: a plain GL call.
        unsafe { glBindFramebuffer(GL_READ_FRAMEBUFFER, one.fb) };
        assert!(near(pixel(3), RED8), "the copy holds the window's red: {:?}", pixel(3));
        assert!(!downsamples(&kicks_since(m)), "a single-sample read makes no 4x resolve");
        // SAFETY: plain GL calls.
        unsafe {
            glBindFramebuffer(GL_READ_FRAMEBUFFER, 0);
            glBlitFramebuffer(0, 0, n, n, 0, 0, n / 2, n / 2, GL_COLOR_BUFFER_BIT, GL_NEAREST);
        }
        assert_eq!(err(), GL_NO_ERROR, "a scaled copy of a single-sample window is legal");
    });
}
