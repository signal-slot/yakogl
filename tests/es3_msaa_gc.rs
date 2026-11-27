// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! 4x framebuffer objects on the GC7000UL (i.MX8MP), through the GL API into
//! the Vivante back end, on the host (items msaa-pin-gc-file, gc-msaa-fbo).
//!
//! docs/notes/es3-msaa-renderbuffers.md (a), (c3): the back end's
//! `max_samples` is 4 only in a `msaa-fbo` build (src/vivante/backend.rs),
//! and then a 4x renderbuffer's draws render into a supertiled scratch of
//! their own, kept across jobs (src/vivante/msaa_fbo.rs, keyed on the colour
//! target's address), resolved by the RS into the renderbuffer's storage at
//! every job end. Without the feature `max_samples` is 0: `GL_MAX_SAMPLES` is
//! 1, `glRenderbufferStorageMultisample` gives `GL_INVALID_OPERATION` for 2
//! or more samples, one sample is plain storage and draws reach the back end
//! single-sampled. The window's 4x stays behind the `gc_debug_msaa` knob in
//! both builds.
//!
//! The real `yakogl_backend_vivante` runs on a stand-in OSAL (host memory,
//! every kick kept), as `tests/es3_mrt4_gc.rs`; its table is copied with
//! `draw` and `attachment_gone` replaced by recorders that forward to the
//! real ones. The stand-in hands a freed allocation's address out again to
//! the next allocation of the same size, as a real pool may, so a
//! respecified renderbuffer comes back at its old address.
//!
//! **The GPU model** (the kick is not executed here). At each submit the
//! stand-in walks the stream: a draw into a SUPER_TILED colour surface (a 4x
//! scratch) takes the next of the test's queued [`Layer`]s and writes it into
//! the scratch's samples, at `PE_PIPE_COLOR_ADDR` with `PE_COLOR_STRIDE` as
//! the padded row (`vivante::layout::offset`, SUPER_TILED); an RS job with
//! `DOWNSAMPLE_X`/`_Y` averages each 2x2 of samples of its supertiled source
//! (its `RS_SOURCE_STRIDE` without the tiling bits, over 4) into the linear
//! destination (`RS_DEST_STRIDE`), rounding to nearest. What the PE rasterises
//! is the test's to say (each layer is the coverage the GL draw has at 4x);
//! what is checked is the back end's wiring: the scratch it draws into, that
//! the RS reads the same scratch at the same pitch and writes the
//! renderbuffer's storage, which draws it records (a reload draw or none),
//! and the pixels the GL core's resolve blit then reads. How the hardware
//! averages and which sample pattern it uses is a board fact (note (f3)).
//!
//! Every test takes the lock: the online compiler's store is one static.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online --test es3_msaa_gc
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online,msaa-fbo --test es3_msaa_gc
//! ```

#![cfg(all(feature = "host-test", feature = "gpu-vivante", feature = "shaderc-online", not(feature = "report-es2")))]
/* the GPU model and the pixel helpers serve the `msaa-fbo` tests */
#![cfg_attr(not(feature = "msaa-fbo"), allow(dead_code))]

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
                let n = 2 * SIZE;
                for sy in 0..n {
                    for sx in 0..n {
                        let px = match &layer {
                            Layer::Paint(f) => match f(sx, sy) {
                                Some([r, g, b, a]) => [b, g, r, a],
                                None => continue,
                            },
                            Layer::Broadcast { src } => {
                                let at = *src + u64::from((sy / 2) * SIZE * 4 + (sx / 2) * 4);
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

/* ---- feature off: the pins of the single-sampled FBOs ----------------------- */

/// Without `msaa-fbo` the table's `max_samples` stays 0 and `GL_MAX_SAMPLES`
/// is 1 (`src/gl/msaa.rs` `max_samples_query`), below ES 3.0's minimum of 4.
#[cfg(not(feature = "msaa-fbo"))]
#[test]
fn max_samples_stays_zero_without_msaa_fbo() {
    assert_eq!(yakogl_backend_vivante.max_samples, 0);
    const { assert!(!yakogl::vivante::msaa_fbo::ON) };
    on_gc7000(|| assert_eq!(geti(GL_MAX_SAMPLES), 1));
}

/// `glGetInternalformativ`: one sample count, 1, for a renderable format; no
/// counts for an integer one (ES 3.0 4.4.2.1).
#[cfg(not(feature = "msaa-fbo"))]
#[test]
fn internalformat_counts_today() {
    on_gc7000(|| {
        for f in [GL_RGBA8, GL_SRGB8_ALPHA8, GL_RGB10_A2, GL_RGB565, GL_R8, GL_RG8, GL_RGBA4, GL_RGB5_A1, GL_RGB8] {
            assert_eq!(internalformat(f, GL_NUM_SAMPLE_COUNTS), 1, "{f:#x}");
            assert_eq!(internalformat(f, GL_SAMPLES), 1, "{f:#x}");
        }
        for f in [GL_R8I, GL_RG16UI, GL_RGBA32I, GL_RGB10_A2UI] {
            assert_eq!(internalformat(f, GL_NUM_SAMPLE_COUNTS), 0, "{f:#x}");
        }
    });
}

/// 4 (and 2) samples: `GL_INVALID_OPERATION`, past `GL_MAX_SAMPLES`.
#[cfg(not(feature = "msaa-fbo"))]
#[test]
fn four_samples_is_invalid_operation() {
    on_gc7000(|| {
        assert_eq!(storage(4, GL_RGBA8).0, GL_INVALID_OPERATION);
        assert_eq!(storage(2, GL_RGBA8).0, GL_INVALID_OPERATION);
        assert_eq!(storage(4, GL_DEPTH24_STENCIL8).0, GL_INVALID_OPERATION);
        assert_eq!(framebuffer(4, None).err(), Some(GL_INVALID_OPERATION));
    });
}

/// One sample is plain storage that reads back as 1; zero reads back as 0; the
/// framebuffer of the one-sample storage has "sample buffers" (completeness).
#[cfg(not(feature = "msaa-fbo"))]
#[test]
fn one_sample_is_plain_storage_and_reads_back() {
    on_gc7000(|| {
        let none = framebuffer(0, None).unwrap();
        assert_eq!(rb_samples(none.colour), 0);
        assert_eq!((geti(GL_SAMPLE_BUFFERS), geti(GL_SAMPLES)), (0, 0));
        let one = framebuffer(1, None).unwrap();
        assert_eq!(rb_samples(one.colour), 1);
        assert_eq!((geti(GL_SAMPLE_BUFFERS), geti(GL_SAMPLES)), (1, 1));
    });
}

/// A draw into a one-sample framebuffer reaches the back end with
/// `GlesDraw::samples` 1, one target of the renderbuffer's own size, and the
/// RS does not downsample in its kicks.
#[cfg(not(feature = "msaa-fbo"))]
#[test]
fn a_draw_into_one_sample_reaches_the_back_end_single_sampled() {
    on_gc7000(|| {
        let _fbo = framebuffer(1, Some(GL_DEPTH24_STENCIL8)).unwrap();
        let m = kicked();
        let s = draw(RED);
        assert_eq!(s.samples, 1);
        assert_eq!(s.targets.len(), 1);
        assert_eq!(s.size, (SIZE, SIZE));
        assert!(!downsamples(&kicks_since(m)), "a single-sample draw makes no 4x resolve");
    });
}

/// A resolve blit from a one-sample framebuffer into a single-sample one is
/// the plain CPU copy and gives no error; a blit *into* a one-sample
/// framebuffer (it has sample buffers) is refused by the spec.
#[cfg(not(feature = "msaa-fbo"))]
#[test]
fn one_sample_blits_today() {
    on_gc7000(|| {
        let one = framebuffer(1, None).unwrap();
        let _ = draw(RED);
        let (e, _dst) = resolve_blit(&one, GL_COLOR_BUFFER_BIT);
        assert_eq!(e, GL_NO_ERROR);
        let single = framebuffer(0, None).unwrap();
        // SAFETY: plain GL calls on names this test made.
        unsafe {
            glBindFramebuffer(GL_READ_FRAMEBUFFER, single.fb);
            glBindFramebuffer(GL_DRAW_FRAMEBUFFER, one.fb);
        }
        clear_errors();
        let n = SIZE as GLint;
        // SAFETY: a plain GL call.
        unsafe { glBlitFramebuffer(0, 0, n, n, 0, 0, n, n, GL_COLOR_BUFFER_BIT, GL_NEAREST) };
        assert_eq!(err(), GL_INVALID_OPERATION);
    });
}

/* ---- both builds ------------------------------------------------------------- */

/// A negative count is `GL_INVALID_VALUE`, checked before the limit; an
/// integer format is refused at any count above 0, the spec's rule.
#[test]
fn negative_and_integer_counts_are_refused() {
    on_gc7000(|| {
        assert_eq!(storage(-1, GL_RGBA8).0, GL_INVALID_VALUE);
        assert_eq!(storage(1, GL_RGBA8UI).0, GL_INVALID_OPERATION);
        assert_eq!(storage(4, GL_RGBA32UI).0, GL_INVALID_OPERATION);
    });
}

/// The same for the zero-sample framebuffer: one sample, and no slot kept.
#[test]
fn a_draw_into_zero_samples_reaches_the_back_end_single_sampled() {
    on_gc7000(|| {
        let _fbo = framebuffer(0, None).unwrap();
        assert_eq!(draw(RED).samples, 1);
        assert_eq!(yakogl::vivante::msaa_fbo::slots(), 0);
    });
}

/* ---- msaa-fbo: 4x framebuffer objects ----------------------------------------- */

/// The pixel `[r, g, b, a]` lies within one step of `want` on each channel
/// (the RS's rounding of an average of two is a board fact).
#[cfg(feature = "msaa-fbo")]
fn near(got: [u8; 4], want: [u8; 4]) -> bool {
    got.iter().zip(want).all(|(g, w)| g.abs_diff(w) <= 1)
}

/// Queue the layers the next 4x draws write (module doc, the GPU model).
#[cfg(feature = "msaa-fbo")]
fn expect_layers(layers: impl IntoIterator<Item = Layer>) {
    with_platform(|p| p.layers.extend(layers));
}

/// Every sample in `rgba`.
#[cfg(feature = "msaa-fbo")]
fn all(rgba: [u8; 4]) -> Layer {
    Layer::Paint(Box::new(move |_, _| Some(rgba)))
}

/// The samples of the quad `[-1, HALF_EDGE_X]`: sample columns 0..=16, pixel
/// 8's left two samples (the edge at pixel 8.5 puts sample column 16, centre
/// 8.25, inside and column 17, centre 8.75, outside).
#[cfg(feature = "msaa-fbo")]
fn left_half_edge(rgba: [u8; 4]) -> Layer {
    Layer::Paint(Box::new(move |sx, _| (sx <= 16).then_some(rgba)))
}

/// The samples of the quad `[HALF_EDGE_X, 1]`: sample columns 17 and on.
#[cfg(feature = "msaa-fbo")]
fn right_half_edge(rgba: [u8; 4]) -> Layer {
    Layer::Paint(Box::new(move |sx, _| (sx >= 17).then_some(rgba)))
}

#[cfg(feature = "msaa-fbo")]
const RED8: [u8; 4] = [255, 0, 0, 255];
#[cfg(feature = "msaa-fbo")]
const BLUE8: [u8; 4] = [0, 0, 255, 255];
#[cfg(feature = "msaa-fbo")]
const BLACK8: [u8; 4] = [0, 0, 0, 255];

/// The draws into a 4x scratch since `mark` (their scratch addresses), and
/// whether every queued layer was taken.
#[cfg(feature = "msaa-fbo")]
fn ms_draws_since(mark: usize) -> Vec<u64> {
    // SAFETY: a plain GL call.
    unsafe { glFinish() };
    with_platform(|p| {
        assert!(p.model_errors.is_empty(), "the GPU model: {:?}", p.model_errors);
        assert!(p.layers.is_empty(), "{} queued 4x draws were not made", p.layers.len());
        p.ms_draws[mark..].to_vec()
    })
}

#[cfg(feature = "msaa-fbo")]
fn ms_draws() -> usize {
    // SAFETY: a plain GL call.
    unsafe { glFinish() };
    with_platform(|p| p.ms_draws.len())
}

/// The table says 4 under `msaa-fbo` (note (a), (b)1).
#[cfg(feature = "msaa-fbo")]
#[test]
fn max_samples_is_four() {
    assert_eq!(yakogl_backend_vivante.max_samples, 4);
    on_gc7000(|| assert_eq!(geti(GL_MAX_SAMPLES), 4));
}

/// The count 4 for the multisample-renderable formats, none for integer ones.
#[cfg(feature = "msaa-fbo")]
#[test]
fn internalformat_counts_four() {
    on_gc7000(|| {
        for f in [GL_RGBA8, GL_SRGB8_ALPHA8, GL_RGB10_A2, GL_RGB565, GL_R8, GL_RG8] {
            assert_eq!(internalformat(f, GL_NUM_SAMPLE_COUNTS), 1, "{f:#x}");
            assert_eq!(internalformat(f, GL_SAMPLES), 4, "{f:#x}");
        }
        for f in [GL_R8I, GL_RGBA32UI] {
            assert_eq!(internalformat(f, GL_NUM_SAMPLE_COUNTS), 0, "{f:#x}");
        }
    });
}

/// A 4x RGBA8 framebuffer: storage is legal, reads back 4, the framebuffer is
/// complete, and the draw reaches the back end with `samples` 4. It renders
/// into the target's own scratch (not the window's `msaa`), whose first job
/// starts from the storage in every sample (no samples kept yet, no clear:
/// the reload draw, then the draw), and the job ends in the RS's 2x2
/// downsample of that scratch into the renderbuffer's storage, after which
/// the slot's samples count (note (c3), (b)7).
#[cfg(feature = "msaa-fbo")]
#[test]
fn rgba8_4x_fbo_draws_four_samples_and_resolves() {
    on_gc7000(|| {
        let fbo = framebuffer(4, None).expect("4 samples are legal storage");
        assert_eq!(rb_samples(fbo.colour), 4);
        assert_eq!(geti(GL_SAMPLES), 4);
        let (m, d) = (kicked(), ms_draws());
        /* the reload reads the target's storage, whose address the draw
         * names: the layers are queued between the draw and the finish,
         * since the model runs at the submit */
        colour_program(RED);
        SEEN.lock().unwrap().clear();
        let quad = [-1.0f32, -1.0, 0.5, 1.0, 1.0, -1.0, 0.5, 1.0, -1.0, 1.0, 0.5, 1.0, 1.0, 1.0, 0.5, 1.0];
        // SAFETY: plain GL calls; `quad` outlives the finish below.
        unsafe {
            glViewport(0, 0, SIZE as GLsizei, SIZE as GLsizei);
            glEnableVertexAttribArray(0);
            glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, quad.as_ptr() as *const c_void);
            glDrawArrays(GL_TRIANGLE_STRIP, 0, 4);
        }
        let s = SEEN.lock().unwrap().iter().rev().find(|s| s.draws).cloned().expect("the draw reached the back end");
        let target = s.targets[0].0;
        expect_layers([Layer::Broadcast { src: target }, all(RED8)]);
        // SAFETY: a plain GL call.
        unsafe { glFinish() };
        assert_eq!((s.samples, s.targets.len(), s.size), (4, 1, (SIZE, SIZE)));
        let w = kicks_since(m);
        assert!(downsamples(&w), "the job ends in the RS downsample");
        let (scratch, loadable) = yakogl::vivante::msaa_fbo::kept(target).expect("a slot for the 4x target");
        assert!(loadable, "the submitted job's samples count");
        assert_eq!(ms_draws_since(d), [scratch, scratch], "the reload and the draw, into the slot's scratch");
        let scratch_pe = w.iter().filter(|(r, _)| *r == pe_pipe_color_addr(0)).any(|(_, v)| u64::from(*v) == scratch);
        assert!(scratch_pe, "the PE is pointed at the slot's scratch");
        assert_eq!(with_platform(|p| p.resolves.last().copied()), Some((scratch, target)));
        let src_stride = w.iter().rev().find(|(r, _)| *r == GC_RS_SOURCE_STRIDE).unwrap().1;
        let pe_stride = w.iter().rev().find(|(r, _)| *r == GC_PE_COLOR_STRIDE).unwrap().1;
        assert_eq!((src_stride & !(0x8000_0000 | 0x0800_0000)) >> 2, pe_stride, "the RS reads the PE's pitch");
        assert_eq!(pe_stride, yakogl::msaa_hw::gc::scratch_pitch(SIZE, 4));
        let (e, _dst) = resolve_blit(&fbo, GL_COLOR_BUFFER_BIT);
        assert_eq!(e, GL_NO_ERROR);
        assert_eq!(pixel(0), RED8);
        assert_eq!(pixel(15), RED8);
    });
}

/// A 4x RGBA8 FBO cleared and drawn with a half-covered edge, resolved by a
/// blit into a 1x FBO: the pixel the edge crosses averages its four samples
/// (two red, two black), the interior is exact. A job that begins with a
/// whole clear needs no reload: the clear and the draw are its two 4x draws.
#[cfg(feature = "msaa-fbo")]
#[test]
fn a_half_covered_edge_resolves_to_the_average() {
    on_gc7000(|| {
        let fbo = framebuffer(4, None).expect("4 samples are legal storage");
        let d = ms_draws();
        expect_layers([all(BLACK8), left_half_edge(RED8)]);
        clear(BLACK);
        let s = draw_left_of(HALF_EDGE_X, 0.5, RED);
        assert_eq!(s.samples, 4);
        assert_eq!(ms_draws_since(d).len(), 2, "the clear and the draw, no reload");
        let (e, _dst) = resolve_blit(&fbo, GL_COLOR_BUFFER_BIT);
        assert_eq!(e, GL_NO_ERROR);
        assert_eq!(pixel(3), RED8, "interior, exact");
        assert_eq!(pixel(13), BLACK8, "outside, exact");
        let edge = pixel(8);
        assert!(near(edge, [128, 0, 0, 255]), "two of four samples red: {edge:?}");
    });
}

/// A second job drawing into the same 4x FBO without a clear draws on in the
/// samples the first one kept: the same scratch, no reload draw, and pixel 8
/// ends with two red samples from job 1 and two blue ones from job 2.
#[cfg(feature = "msaa-fbo")]
#[test]
fn a_second_job_draws_on_in_the_kept_samples() {
    on_gc7000(|| {
        let fbo = framebuffer(4, None).expect("4 samples are legal storage");
        let d = ms_draws();
        expect_layers([all(BLACK8), left_half_edge(RED8)]);
        clear(BLACK);
        let target = draw_left_of(HALF_EDGE_X, 0.5, RED).targets[0].0;
        let first = ms_draws_since(d);
        let (scratch, loadable) = yakogl::vivante::msaa_fbo::kept(target).expect("a slot");
        assert!(loadable);
        assert_eq!(first, [scratch, scratch]);

        /* job 2: the other half, no clear */
        let d = ms_draws();
        expect_layers([right_half_edge(BLUE8)]);
        assert_eq!(draw_quad(HALF_EDGE_X, 1.0, 0.5, BLUE).samples, 4);
        assert_eq!(ms_draws_since(d), [scratch], "one draw, into the kept scratch: no reload");
        let (e, _dst) = resolve_blit(&fbo, GL_COLOR_BUFFER_BIT);
        assert_eq!(e, GL_NO_ERROR);
        assert_eq!(pixel(3), RED8);
        assert_eq!(pixel(13), BLUE8);
        let edge = pixel(8);
        assert!(near(edge, [128, 0, 128, 255]), "two red samples kept, two blue drawn: {edge:?}");
    });
}

/// A target whose slot was evicted (more live 4x targets than
/// `msaa_store::MSAA_SLOTS`) starts its next job from its resolved storage
/// in every sample: the reload draw comes first, so pixel 8 is its resolved
/// average in two samples and blue in two.
#[cfg(feature = "msaa-fbo")]
#[test]
fn an_evicted_target_reloads_its_resolved_image() {
    on_gc7000(|| {
        let a = framebuffer(4, None).expect("4 samples are legal storage");
        expect_layers([all(BLACK8), left_half_edge(RED8)]);
        clear(BLACK);
        let target = draw_left_of(HALF_EDGE_X, 0.5, RED).targets[0].0;
        for _ in 0..yakogl::msaa_store::MSAA_SLOTS {
            let _other = framebuffer(4, None).unwrap();
            expect_layers([all(BLACK8), all(BLACK8)]);
            clear(BLACK);
            let _ = draw(BLACK);
            let _ = ms_draws_since(0);
        }
        assert!(yakogl::vivante::msaa_fbo::kept(target).is_none(), "evicted");
        // SAFETY: a framebuffer this test made.
        unsafe { glBindFramebuffer(GL_FRAMEBUFFER, a.fb) };
        let d = ms_draws();
        expect_layers([Layer::Broadcast { src: target }, right_half_edge(BLUE8)]);
        assert_eq!(draw_quad(HALF_EDGE_X, 1.0, 0.5, BLUE).samples, 4);
        assert_eq!(ms_draws_since(d).len(), 2, "the reload, then the draw");
        let (e, _dst) = resolve_blit(&a, GL_COLOR_BUFFER_BIT);
        assert_eq!(e, GL_NO_ERROR);
        assert_eq!(pixel(3), RED8);
        assert_eq!(pixel(13), BLUE8);
        let edge = pixel(8);
        assert!(near(edge, [64, 0, 128, 255]), "the resolved edge reloaded in two samples, blue in two: {edge:?}");
    });
}

/// After the storage is specified again at the same size (the stand-in hands
/// back the same address), the slot is gone with the old storage
/// (`texture_released`), so no sample of the old storage comes back: the
/// next job reloads the new, zeroed storage, and a draw after a clear shows
/// only what was drawn since. A deleted and re-made renderbuffer at the same
/// address is the same.
#[cfg(feature = "msaa-fbo")]
#[test]
fn no_stale_samples_after_a_same_size_respecification() {
    on_gc7000(|| {
        let fbo = framebuffer(4, None).expect("4 samples are legal storage");
        expect_layers([all(BLACK8), all(RED8)]);
        clear(BLACK);
        let old = draw(RED).targets[0].0;
        assert!(yakogl::vivante::msaa_fbo::kept(old).is_some_and(|k| k.1));
        let gone = GONE.lock().unwrap().len();
        // SAFETY: a renderbuffer this test made; the same size and format again.
        unsafe {
            glBindRenderbuffer(GL_RENDERBUFFER, fbo.colour);
            glRenderbufferStorageMultisample(GL_RENDERBUFFER, 4, GL_RGBA8, SIZE as GLsizei, SIZE as GLsizei);
        }
        assert_eq!(err(), GL_NO_ERROR);
        assert_eq!(GONE.lock().unwrap().len(), gone + 1, "the back end is told the colour samples are gone");
        assert!(yakogl::vivante::msaa_fbo::kept(old).is_none(), "the slot went with the storage");

        /* no clear: the job starts from the new storage, not the old samples */
        let d = ms_draws();
        expect_layers([Layer::Broadcast { src: old }, left_half_edge(BLUE8)]);
        let s = draw_left_of(HALF_EDGE_X, 0.5, BLUE);
        assert_eq!(s.targets[0].0, old, "the stand-in gave the storage its old address back");
        assert_eq!(ms_draws_since(d).len(), 2, "a reload: nothing kept");
        let (e, _dst) = resolve_blit(&fbo, GL_COLOR_BUFFER_BIT);
        assert_eq!(e, GL_NO_ERROR);
        assert_eq!(pixel(3), BLUE8, "no red from the old storage");
        assert_eq!(pixel(13), [0, 0, 0, 0], "the new storage's zeros");
        let edge = pixel(8);
        assert!(near(edge, [0, 0, 128, 128]), "no red in the edge pixel: {edge:?}");

        /* with a clear */
        // SAFETY: a framebuffer this test made.
        unsafe { glBindFramebuffer(GL_FRAMEBUFFER, fbo.fb) };
        expect_layers([all(BLACK8), left_half_edge(BLUE8)]);
        clear(BLACK);
        let _ = draw_left_of(HALF_EDGE_X, 0.5, BLUE);
        let (e, _dst) = resolve_blit(&fbo, GL_COLOR_BUFFER_BIT);
        assert_eq!(e, GL_NO_ERROR);
        assert_eq!(pixel(3), BLUE8);
        assert!(near(pixel(8), [0, 0, 128, 255]));

        /* deleted and made again: the same address, no samples */
        // SAFETY: names this test made.
        unsafe {
            glBindFramebuffer(GL_FRAMEBUFFER, fbo.fb);
            glDeleteRenderbuffers(1, &fbo.colour);
        }
        assert!(yakogl::vivante::msaa_fbo::kept(old).is_none(), "deleted: the slot went too");
        let (e, rb) = storage(4, GL_RGBA8);
        assert_eq!(e, GL_NO_ERROR);
        // SAFETY: a renderbuffer this test made.
        unsafe { glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_RENDERBUFFER, rb) };
        let d = ms_draws();
        expect_layers([Layer::Broadcast { src: old }, left_half_edge(RED8)]);
        let s = draw_left_of(HALF_EDGE_X, 0.5, RED);
        assert_eq!(s.targets[0].0, old, "the new renderbuffer at the old address");
        assert_eq!(ms_draws_since(d).len(), 2, "a reload, not the deleted one's samples");
        let fbo2 = Fbo { fb: fbo.fb, colour: rb, depth: None };
        let (e, _dst) = resolve_blit(&fbo2, GL_COLOR_BUFFER_BIT);
        assert_eq!(e, GL_NO_ERROR);
        assert_eq!(pixel(13), [0, 0, 0, 0], "no blue from the deleted renderbuffer");
    });
}

/// The slot is keyed on `(target_id, target_gen)`: respecifying the
/// renderbuffer bumps the generation and drops the samples, deleting it frees
/// the slot, and a new renderbuffer at the same address loads nothing stale.
#[cfg(feature = "msaa-fbo")]
#[test]
fn slots_follow_the_attachment_identity() {
    on_gc7000(|| {
        let fbo = framebuffer(4, None).expect("4 samples are legal storage");
        expect_layers([all(BLACK8), all(RED8)]);
        clear(BLACK);
        let s = draw(RED);
        let (id, gen) = s.ident;
        assert_ne!(id, 0, "a renderbuffer has an identity");
        let (_, kept_gen, loadable) = yakogl::vivante::msaa_fbo::kept_id(id).expect("a slot keyed on the id");
        assert_eq!(kept_gen, gen);
        assert!(loadable);
        let slots = yakogl::vivante::msaa_fbo::slots();

        /* respecified: the generation moves, the samples are dropped */
        // SAFETY: a renderbuffer this test made.
        unsafe {
            glBindRenderbuffer(GL_RENDERBUFFER, fbo.colour);
            glRenderbufferStorageMultisample(GL_RENDERBUFFER, 4, GL_RGBA8, SIZE as GLsizei, SIZE as GLsizei);
        }
        assert_eq!(err(), GL_NO_ERROR);
        assert!(yakogl::vivante::msaa_fbo::kept_id(id).is_none(), "the samples went with the old generation");
        assert_eq!(yakogl::vivante::msaa_fbo::slots(), slots - 1);
        expect_layers([Layer::Broadcast { src: s.targets[0].0 }, left_half_edge(BLUE8)]);
        let s2 = draw_left_of(HALF_EDGE_X, 0.5, BLUE);
        assert_eq!(s2.ident.0, id, "the same renderbuffer");
        assert!(s2.ident.1 > gen, "the generation moved: {} then {}", gen, s2.ident.1);
        let (_, kept_gen, _) = yakogl::vivante::msaa_fbo::kept_id(id).expect("a new slot");
        assert_eq!(kept_gen, s2.ident.1);

        /* deleted: the slot is freed */
        // SAFETY: names this test made.
        unsafe {
            glBindFramebuffer(GL_FRAMEBUFFER, fbo.fb);
            glDeleteRenderbuffers(1, &fbo.colour);
        }
        assert!(yakogl::vivante::msaa_fbo::kept_id(id).is_none(), "deleted: the slot is freed");
        assert_eq!(yakogl::vivante::msaa_fbo::slots(), slots - 1);

        /* a new renderbuffer, perhaps at the same address: another id, no stale samples */
        let (e, rb) = storage(4, GL_RGBA8);
        assert_eq!(e, GL_NO_ERROR);
        // SAFETY: a renderbuffer this test made.
        unsafe { glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_RENDERBUFFER, rb) };
        let d = ms_draws();
        expect_layers([Layer::Broadcast { src: s.targets[0].0 }, left_half_edge(RED8)]);
        let s3 = draw_left_of(HALF_EDGE_X, 0.5, RED);
        assert_ne!(s3.ident.0, id, "a new renderbuffer is a new identity");
        assert_eq!(ms_draws_since(d).len(), 2, "a reload, not the deleted one's samples");
        let fbo2 = Fbo { fb: fbo.fb, colour: rb, depth: None };
        let (e, _dst) = resolve_blit(&fbo2, GL_COLOR_BUFFER_BIT);
        assert_eq!(e, GL_NO_ERROR);
        assert_eq!(pixel(13), [0, 0, 0, 0], "no blue from the deleted renderbuffer");
    });
}

/// sRGB: the RS averages the encoded bytes as A8R8G8B8 (note (f3), (f5):
/// pinned here; whether that is inside dEQP's tolerance is a board fact),
/// so an SRGB8_ALPHA8 4x FBO resolves through the same words as RGBA8.
/// RGB10_A2 is stored RGBA8888 in this build (gl/framebuffer.rs
/// `rgb10a2_surface`: the GC7000UL draws no 10/10/10/2 surface), so its 4x
/// FBO resolves as RGBA8 too: the RS averages 8-bit channels, never the
/// packed word (f3) warns of. A 4x float FBO (RGBA16F, no RS row) draws
/// single-sampled: no downsample in its kick and no slot.
#[cfg(feature = "msaa-fbo")]
#[test]
fn srgb_and_rgb10_a2_resolve_as_a8r8g8b8_and_float_draws_single_sampled() {
    on_gc7000(|| {
        let a8r8g8b8 = yakogl::msaa_hw::gc::rs_config_resolve(yakogl::msaa_hw::gc::RS_FORMAT_A8R8G8B8);
        for (f, surf) in [(GL_SRGB8_ALPHA8, GLES_SURF_SRGB8_A8), (GL_RGB10_A2, GLES_SURF_RGBA8888)] {
            let _fbo = framebuffer_of(4, f, None).expect("4x storage");
            let m = kicked();
            expect_layers([all(BLACK8), all(RED8)]);
            clear(BLACK);
            let s = draw(RED);
            assert_eq!((s.samples, s.targets[0].1), (4, surf), "{f:#x}");
            let w = kicks_since(m);
            let cfg = w.iter().rev().find(|(r, _)| *r == GC_RS_CONFIG).map(|w| w.1);
            assert_eq!(cfg, Some(a8r8g8b8), "{f:#x}");
            assert!(yakogl::vivante::msaa_fbo::kept(s.targets[0].0).is_some(), "{f:#x}");
            assert_eq!(ms_draws_since(0).len() % 2, 0);
        }

        let _f16 = framebuffer_of(4, GL_RGBA16F, None).expect("4x RGBA16F storage");
        let m = kicked();
        let s = draw(BLUE);
        assert_eq!((s.samples, s.targets[0].1), (4, GLES_SURF_RGBA16F), "the GL core promised 4 samples");
        assert!(!downsamples(&kicks_since(m)), "drawn single-sampled: nothing to resolve");
        assert!(yakogl::vivante::msaa_fbo::kept(s.targets[0].0).is_none());
    });
}

/// A resolve blit that is scaled, or whose regions differ, is
/// `GL_INVALID_OPERATION` (ES 3.0 4.3.3).
#[cfg(feature = "msaa-fbo")]
#[test]
fn a_scaled_or_shifted_resolve_is_invalid_operation() {
    on_gc7000(|| {
        let fbo = framebuffer(4, None).expect("4 samples are legal storage");
        let dst = framebuffer(0, None).unwrap();
        // SAFETY: plain GL calls on names this test made.
        unsafe {
            glBindFramebuffer(GL_READ_FRAMEBUFFER, fbo.fb);
            glBindFramebuffer(GL_DRAW_FRAMEBUFFER, dst.fb);
        }
        for (s, d) in [([0, 0, 16, 16], [0, 0, 8, 8]), ([0, 0, 8, 8], [4, 4, 12, 12]), ([0, 0, 16, 16], [0, 0, 16, 15])]
        {
            clear_errors();
            // SAFETY: a plain GL call.
            unsafe {
                glBlitFramebuffer(s[0], s[1], s[2], s[3], d[0], d[1], d[2], d[3], GL_COLOR_BUFFER_BIT, GL_NEAREST)
            };
            assert_eq!(err(), GL_INVALID_OPERATION, "{s:?} -> {d:?}");
        }
    });
}

/// Depth and stencil of a 4x framebuffer resolve to one sample a pixel
/// (note (b)8: which sample is the implementation's, ES 3.0 4.3.3). The blit
/// is legal and the resolved framebuffer stays complete and drawable.
#[cfg(feature = "msaa-fbo")]
#[test]
#[ignore = "note (b)8 + (c3) 'Depth', not gc-msaa-fbo's: glBlitFramebuffer refuses a depth/stencil blit into a renderbuffer on this back end (GL_INVALID_OPERATION at 1x too: the GC table has no ds_write or stencil_resolve entry, gl/blit.rs refuse_ds), ds_cache::resolve_depth refuses the 4x surface (scale 2, -ENOTSUP), and this file's GPU model does not run the depth test; which sample the PE keeps is board-unverified"]
fn depth_resolve_keeps_one_sample() {
    on_gc7000(|| {
        let fbo = framebuffer(4, Some(GL_DEPTH24_STENCIL8)).expect("4 samples are legal storage");
        assert_eq!(rb_samples(fbo.depth.unwrap()), 4);
        clear(BLACK);
        // SAFETY: plain GL calls.
        unsafe {
            glEnable(GL_DEPTH_TEST);
            glDepthFunc(GL_LESS);
        }
        assert_eq!(draw_left_of(HALF_EDGE_X, 0.0, RED).samples, 4);
        let (e, dst) = resolve_blit(&fbo, GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT | GL_STENCIL_BUFFER_BIT);
        assert_eq!(e, GL_NO_ERROR);
        /* a draw at the same depth through the resolved depth fails GL_LESS
         * wherever the resolved depth is the drawn one: the interior stays red */
        // SAFETY: plain GL calls on a name this test made.
        unsafe { glBindFramebuffer(GL_FRAMEBUFFER, dst.fb) };
        assert_eq!(draw_left_of(1.0, 0.0, BLUE).samples, 1);
        assert_eq!(pixel(3), [255, 0, 0, 255], "the resolved depth kept the interior");
        // SAFETY: a plain GL call.
        unsafe { glDisable(GL_DEPTH_TEST) };
    });
}
