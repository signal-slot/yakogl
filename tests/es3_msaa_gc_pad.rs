// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! 4x RGBA8 renderbuffer FBOs of the sizes dEQP draws at (119x131, 40x40) and
//! of the corner sizes (1x1, 17x5) on the GC7000UL (i.MX8MP), through the GL
//! API into the Vivante back end, on the host (item gc-pad-tests).
//!
//! gc-msaa-pad removed the limit that refused a 4x FBO whose width was not a
//! multiple of 16 or whose height was not a multiple of 4: the job ends in
//! the RS's 2x2 downsample over the padded rectangle
//! (`msaa_hw::gc::rs_resolve_extent`) into a temporary, and
//! `copy_resolved_rect` moves exactly `w` x `h` pixels into the storage.
//! `tests/es3_msaa_gc.rs` pins 119x131 and 40x40 at the job end; this file
//! goes on to the resolve blit into a single-sample FBO of the same size:
//!
//! * the pixels `glReadPixels` returns, the last column and the last row
//!   (the pixels next to the RS window's padding) included;
//! * the destination's whole allocation, filled with a guard pattern first:
//!   not one byte outside the blitted rectangle changes (the odd width's
//!   pitch column and the bytes past the last row included);
//! * a sub-rectangle with an unaligned origin, and one that runs to the
//!   target's far corner.
//!
//! The stand-in OSAL and the GPU model are those of `tests/es3_msaa_gc.rs`
//! (copied, not shared: a test file is its own crate); the model writes
//! garbage into the scratch's padding (`Platform::pad_garbage`), which must
//! reach nothing. The kick is not executed on the host: how the hardware
//! averages and which sample pattern it uses is a board fact (note (f3)).
//!
//! Every test takes the lock: the online compiler's store is one static.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online,msaa-fbo --test es3_msaa_gc_pad
//! ```

#![cfg(all(
    feature = "host-test",
    feature = "gpu-vivante",
    feature = "shaderc-online",
    feature = "msaa-fbo",
    not(feature = "report-es2")
))]
/* the copied model serves more than this file's cases */
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

/* ---- the cases -------------------------------------------------------------- */

const RED: [f32; 4] = [1.0, 0.0, 0.0, 1.0];
const GREEN: [f32; 4] = [0.0, 1.0, 0.0, 1.0];
const RED8: [u8; 4] = [255, 0, 0, 255];
const BLUE8: [u8; 4] = [0, 0, 255, 255];
const BLACK8: [u8; 4] = [0, 0, 0, 255];
/// What a test fills the destination's whole allocation with before the
/// blit: the bytes outside the blitted rectangle must keep it.
const SENTINEL: u8 = 0x5a;
/// What the model writes into the padding of a 4x scratch.
const GARBAGE8: [u8; 4] = [0x11, 0x22, 0x33, 0x44];

fn near(got: [u8; 4], want: [u8; 4]) -> bool {
    got.iter().zip(want).all(|(g, w)| g.abs_diff(w) <= 1)
}

fn expect_layers(layers: impl IntoIterator<Item = Layer>) {
    with_platform(|p| p.layers.extend(layers));
}

fn all(rgba: [u8; 4]) -> Layer {
    Layer::Paint(Box::new(move |_, _| Some(rgba)))
}

/// A bound `w` x `h` framebuffer with a `samples` RGBA8 colour renderbuffer.
fn framebuffer_sized(samples: GLsizei, w: u32, h: u32) -> GLuint {
    let (mut fb, mut rb) = (0, 0);
    // SAFETY: plain GL calls on names this test makes.
    unsafe {
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        glGenRenderbuffers(1, &mut rb);
        glBindRenderbuffer(GL_RENDERBUFFER, rb);
        glRenderbufferStorageMultisample(GL_RENDERBUFFER, samples, GL_RGBA8, w as GLsizei, h as GLsizei);
        glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_RENDERBUFFER, rb);
    }
    assert_eq!(err(), GL_NO_ERROR, "{w}x{h} at {samples}");
    // SAFETY: a plain GL call.
    assert_eq!(unsafe { glCheckFramebufferStatus(GL_FRAMEBUFFER) }, GL_FRAMEBUFFER_COMPLETE, "{w}x{h}");
    let mut got = -1;
    // SAFETY: one value.
    unsafe { glGetRenderbufferParameteriv(GL_RENDERBUFFER, GL_RENDERBUFFER_SAMPLES, &mut got) };
    assert_eq!(got, samples, "{w}x{h}: the renderbuffer keeps its real count");
    fb
}

/// Draw the whole-target quad in the bound framebuffer; what the back end
/// got for it. The job is not finished.
fn draw_whole(w: u32, h: u32, rgba: [f32; 4]) -> Seen {
    let quad = [-1.0f32, -1.0, 0.5, 1.0, 1.0, -1.0, 0.5, 1.0, -1.0, 1.0, 0.5, 1.0, 1.0, 1.0, 0.5, 1.0];
    colour_program(rgba);
    SEEN.lock().unwrap().clear();
    // SAFETY: plain GL calls; `quad` outlives the finish the caller makes
    // (the attribute pointer is read at the draw).
    unsafe {
        glViewport(0, 0, w as GLsizei, h as GLsizei);
        glEnableVertexAttribArray(0);
        glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, quad.as_ptr() as *const c_void);
        glDrawArrays(GL_TRIANGLE_STRIP, 0, 4);
        glFinish();
    }
    assert_eq!(err(), GL_NO_ERROR);
    let s = SEEN.lock().unwrap().iter().rev().find(|s| s.draws).cloned();
    s.expect("the draw reached the back end")
}

/// The resolved pixel `(x, y)`, as GL R,G,B,A: blue in the last column and
/// the last row (the RS window's padding begins right after them), red left
/// of pixel 10, pixel 10 half red, black elsewhere.
fn expected(w: u32, h: u32, x: u32, y: u32) -> [u8; 4] {
    if x == w - 1 || y == h - 1 {
        BLUE8
    } else if x < 10 {
        RED8
    } else if x == 10 {
        [128, 0, 0, 255]
    } else {
        BLACK8
    }
}

/// The samples the PE writes for [`expected`] over a black clear: pixel 10's
/// left two samples (sample column 20) red, its right two untouched.
fn pattern(w: u32, h: u32) -> Layer {
    Layer::Paint(Box::new(
        move |sx, sy| {
            if sx / 2 == w - 1 || sy / 2 == h - 1 {
                Some(BLUE8)
            } else {
                (sx <= 20).then_some(RED8)
            }
        },
    ))
}

/// A `w` x `h` 4x FBO, drawn (a black clear, then [`pattern`] over it) and
/// resolved with `glBlitFramebuffer(rect)` into a single-sample FBO of the
/// same size whose allocation was filled with [`SENTINEL`]: the pixels read
/// back and every byte of the allocation are checked. `rect` is
/// `(x0, y0, x1, y1)`, the same on both sides.
fn pad_case(w: u32, h: u32, rect: (u32, u32, u32, u32)) {
    use yakogl::msaa_hw::gc::rs_resolvable;
    assert_eq!(rs_resolvable(w, h), w % 16 == 0 && h % 4 == 0);
    on_gc7000(|| {
        with_platform(|p| {
            p.extent = Some((w, h));
            p.pad_garbage = Some(GARBAGE8);
        });
        let src = framebuffer_sized(4, w, h);
        // SAFETY: plain GL calls.
        unsafe {
            glClearColor(0.0, 0.0, 0.0, 1.0);
            glClear(GL_COLOR_BUFFER_BIT);
        }
        expect_layers([all(BLACK8), pattern(w, h)]);
        /* the layers are taken at the finish inside draw_whole */
        let s = draw_whole(w, h, RED);
        assert_eq!((s.samples, s.targets.len(), s.size), (4, 1, (w, h)), "{w}x{h} keeps 4 samples");
        with_platform(|p| {
            assert!(p.model_errors.is_empty(), "the GPU model: {:?}", p.model_errors);
            assert!(p.layers.is_empty(), "{} queued 4x draws were not made", p.layers.len());
        });

        /* the destination: a draw tells where its storage is; the allocation
         * is filled with the guard pattern afterwards */
        let dst = framebuffer_sized(0, w, h);
        let t = draw_whole(w, h, GREEN);
        assert_eq!(t.samples, 1);
        let target = t.targets[0].0;
        let (base, len) = with_platform(|p| {
            let (&base, a) = p.allocs.range_mut(..=target).next_back().expect("the storage's allocation");
            // SAFETY: the allocation's own bytes.
            let all = unsafe { core::slice::from_raw_parts_mut(a.as_mut_ptr() as *mut u8, a.len() * 8) };
            all.fill(SENTINEL);
            (base, all.len())
        });
        let stride = ((w + 1) & !1) * 4;
        assert!(target - base + u64::from(stride * h) <= len as u64, "the storage fits its allocation");

        let (x0, y0, x1, y1) = rect;
        // SAFETY: plain GL calls on names this test made.
        unsafe {
            glBindFramebuffer(GL_READ_FRAMEBUFFER, src);
            glBindFramebuffer(GL_DRAW_FRAMEBUFFER, dst);
            glReadBuffer(GL_COLOR_ATTACHMENT0);
            glBlitFramebuffer(
                x0 as GLint,
                y0 as GLint,
                x1 as GLint,
                y1 as GLint,
                x0 as GLint,
                y0 as GLint,
                x1 as GLint,
                y1 as GLint,
                GL_COLOR_BUFFER_BIT,
                GL_NEAREST,
            );
        }
        assert_eq!(err(), GL_NO_ERROR, "{w}x{h} blit {rect:?}");
        let inside = |x: u32, y: u32| (x0..x1).contains(&x) && (y0..y1).contains(&y);

        /* the pixels, the last column and row included */
        let mut px = vec![0u8; (w * h * 4) as usize];
        // SAFETY: room for w x h RGBA8 pixels.
        unsafe {
            glBindFramebuffer(GL_READ_FRAMEBUFFER, dst);
            glReadPixels(0, 0, w as GLsizei, h as GLsizei, GL_RGBA, GL_UNSIGNED_BYTE, px.as_mut_ptr() as *mut c_void);
        }
        assert_eq!(err(), GL_NO_ERROR);
        for y in 0..h {
            for x in 0..w {
                let i = ((y * w + x) * 4) as usize;
                let got: [u8; 4] = px[i..i + 4].try_into().unwrap();
                let want = if inside(x, y) { expected(w, h, x, y) } else { [SENTINEL; 4] };
                assert!(near(got, want), "{w}x{h} blit {rect:?} pixel ({x}, {y}): {got:?}, want {want:?}");
            }
        }

        /* the allocation: the rectangle, and not a byte else */
        let all = with_platform(|p| bytes(p, base, len).to_vec());
        let mut wrong = Vec::new();
        for (i, &b) in all.iter().enumerate() {
            let off = i as i64 - (target - base) as i64;
            let (y, x) = (off.div_euclid(i64::from(stride)), off.rem_euclid(i64::from(stride)) / 4);
            let want = if off >= 0 && (y as u32) < h && (x as u32) < w && inside(x as u32, y as u32) {
                let [r, g, b, a] = expected(w, h, x as u32, y as u32);
                [b, g, r, a][(off % 4) as usize]
            } else {
                SENTINEL
            };
            if b.abs_diff(want) > 1 && wrong.len() < 8 {
                wrong.push((x, y, off % 4, b, want));
            }
        }
        assert!(wrong.is_empty(), "{w}x{h} blit {rect:?}: (x, y, byte, got, want) {wrong:?}");
    });
}

/// The whole target, an inner rectangle with an unaligned origin, and one
/// from an unaligned origin to the far corner.
fn pad_size(w: u32, h: u32) {
    pad_case(w, h, (0, 0, w, h));
    if w >= 4 && h >= 3 {
        pad_case(w, h, (3, 1, w - 1, h - 1));
        pad_case(w, h, (3, 1, w, h));
    }
}

/// dEQP fbo.msaa's size: neither axis aligned.
#[test]
fn a_119x131_4x_fbo_resolves_into_a_1x_fbo() {
    pad_size(119, 131);
}

/// dEQP renderbuffer_resize's size: the height aligned, the width not.
#[test]
fn a_40x40_4x_fbo_resolves_into_a_1x_fbo() {
    pad_size(40, 40);
}

/// The smallest target: one pixel, all of it the last column and row.
#[test]
fn a_1x1_4x_fbo_resolves_into_a_1x_fbo() {
    pad_size(1, 1);
}

/// An odd width and a height of 5: the storage's pitch has a pad column.
#[test]
fn a_17x5_4x_fbo_resolves_into_a_1x_fbo() {
    pad_size(17, 5);
}
