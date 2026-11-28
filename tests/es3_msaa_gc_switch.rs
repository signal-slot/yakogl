// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Draws that alternate between a 4x framebuffer object, a 1x one and the
//! window on the GC7000UL (i.MX8MP), through the GL API into the Vivante back
//! end, on the host (item gc-msaa-switch-tests). Colour only.
//!
//! The header, the stand-in OSAL and its GPU model are those of
//! `tests/es3_msaa_gc.rs` (read its module doc): the real
//! `yakogl_backend_vivante` runs on host memory, a draw into a 4x supertiled
//! scratch takes the next queued [`Layer`], an RS 2x2 downsample averages the
//! samples into the renderbuffer's storage and the GL core's resolve blit reads
//! that back. A single-sample draw (the 1x FBO, the window) is not modelled:
//! such a target is checked by what the back end was given (`samples`, target
//! address, identity), by the kicks (no 4x scratch draw, no RS resolve
//! into it) and by its storage staying what it was.
//!
//! Every test needs `msaa-fbo`; without it the file has no tests.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online,msaa-fbo --test es3_msaa_gc_switch
//! ```

#![cfg(all(
    feature = "host-test",
    feature = "gpu-vivante",
    feature = "shaderc-online",
    feature = "msaa-fbo",
    not(feature = "report-es2")
))]
#![allow(dead_code, unused_imports)]
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

/* ---- the switch tests --------------------------------------------------------- */

const GREEN: [f32; 4] = [0.0, 1.0, 0.0, 1.0];
const GREEN8: [u8; 4] = [0, 255, 0, 255];
const WHITE: [f32; 4] = [1.0, 1.0, 1.0, 1.0];

fn bind(fb: GLuint) {
    // SAFETY: a framebuffer this test made, or 0 (the window).
    unsafe { glBindFramebuffer(GL_FRAMEBUFFER, fb) };
}

/// A draw that must reach the back end single-sampled: no layer is queued, so
/// a 4x draw here would fail the model; the target it names is returned.
fn draw_single(rgba: [f32; 4]) -> Seen {
    let s = draw(rgba);
    assert_eq!(s.samples, 1, "a single-sample target is drawn single-sampled");
    s
}

/// The pixel (8, 8) of `fb`'s storage, read through the GL core.
fn read_at(fb: GLuint) -> [u8; 4] {
    // SAFETY: a framebuffer this test made.
    unsafe { glBindFramebuffer(GL_READ_FRAMEBUFFER, fb) };
    pixel(8)
}

/// One pass of the sequence: 4x (clear black, left of pixel 8's edge in `a`),
/// 1x (`b`), the window (`b`), then the 4x one again (right of the edge in
/// `b`), and its resolve. `first` says A has no slot yet. Returns A's edge
/// pixel after the resolve.
fn pass(a: &Fbo, one: &Fbo, left: ([f32; 4], [u8; 4]), right: ([f32; 4], [u8; 4]), first: bool) -> Seen4 {
    let (m, d) = (kicked(), ms_draws());
    let _ = (m, first);

    /* 4x: a whole clear, then the left half */
    bind(a.fb);
    assert_eq!(geti(GL_SAMPLES), 4);
    expect_layers([all(BLACK8), left_half_edge(left.1)]);
    clear(BLACK);
    let sa = draw_left_of(HALF_EDGE_X, 0.5, left.0);
    assert_eq!(sa.samples, 4);
    let scratch = ms_draws_since(d);
    assert_eq!(scratch.len(), 2, "the clear and the draw, into the scratch");
    let (slot, _) = yakogl::vivante::msaa_fbo::kept(sa.targets[0].0).expect("A keeps a slot");
    assert_eq!(scratch, [slot, slot]);
    let resolves_a = with_platform(|p| p.resolves.len());

    /* 1x: no 4x draw, no resolve */
    bind(one.fb);
    assert_eq!(geti(GL_SAMPLES), 0);
    let d1 = ms_draws();
    let sb = draw_single(GREEN);
    assert_eq!(ms_draws_since(d1), Vec::<u64>::new(), "the 1x draw is not a scratch draw");
    assert_ne!(sb.targets[0].0, sa.targets[0].0, "its own storage");
    assert_ne!(sb.ident, sa.ident, "its own identity");
    assert_eq!(sb.size, (SIZE, SIZE));

    /* the window */
    bind(0);
    let d2 = ms_draws();
    let sw = draw_single(WHITE);
    assert_eq!(ms_draws_since(d2), Vec::<u64>::new(), "the window draw is not a scratch draw");
    assert_ne!(sw.targets[0].0, sa.targets[0].0);
    assert_ne!(sw.targets[0].0, sb.targets[0].0);

    /* the 4x one again: draws on in the kept samples, no reload */
    bind(a.fb);
    assert_eq!(geti(GL_SAMPLES), 4);
    let d3 = ms_draws();
    expect_layers([right_half_edge(right.1)]);
    let sa2 = draw_quad(HALF_EDGE_X, 1.0, 0.5, right.0);
    assert_eq!((sa2.samples, sa2.targets[0].0, sa2.ident), (4, sa.targets[0].0, sa.ident));
    assert_eq!(ms_draws_since(d3), [slot], "one draw into the kept scratch: no reload");
    assert_eq!(yakogl::vivante::msaa_fbo::kept(sa.targets[0].0).map(|k| k.0), Some(slot));

    /* every RS resolve since A's first job went to A's storage, none to 1x */
    let _ = resolves_a;
    with_platform(|p| {
        assert!(p.resolves.iter().all(|&(s, t)| s == slot && t == sa.targets[0].0), "{:?}", p.resolves);
    });
    let (e, _dst) = resolve_blit(a, GL_COLOR_BUFFER_BIT);
    assert_eq!(e, GL_NO_ERROR);
    Seen4 { a: sa, one: sb, window: sw, edge: [pixel(3), pixel(8), pixel(13)] }
}

struct Seen4 {
    a: Seen,
    one: Seen,
    window: Seen,
    /// A's pixels (3, 8), (8, 8) and (13, 8) after the resolve.
    edge: [[u8; 4]; 3],
}

fn check_edge(e: &[[u8; 4]; 3], left: [u8; 4], right: [u8; 4]) {
    assert_eq!(e[0], left, "interior left, exact");
    assert_eq!(e[2], right, "interior right, exact");
    let mid: Vec<u8> = left.iter().zip(right).map(|(l, r)| ((u16::from(*l) + u16::from(r)) / 2) as u8).collect();
    assert!(near(e[1], [mid[0], mid[1], mid[2], 255]), "two samples of each: {:?} vs {mid:?}", e[1]);
}

/// 4x, 1x, window, 4x again, each with its own colour: the 4x FBO keeps the
/// samples of its first job across the two switches (pixel 8 is half the
/// first colour, half the second) and neither other target is its resolve's
/// destination.
#[test]
fn each_target_keeps_its_own_contents() {
    on_gc7000(|| {
        let a = framebuffer(4, None).unwrap();
        let one = framebuffer(0, None).unwrap();
        let before = read_at(one.fb);
        let r = pass(&a, &one, (RED, RED8), (BLUE, BLUE8), true);
        check_edge(&r.edge, RED8, BLUE8);
        assert_ne!(r.edge[0], GREEN8);
        assert_eq!(read_at(one.fb), before, "the 1x target is not touched by A's resolves");
        assert_ne!(r.one.targets[0].0, r.window.targets[0].0);
    });
}

/// The same sequence twice, the second with other colours, on the same three
/// targets: no state of the first (slot, scratch, MSAA or tile-status
/// registers) bleeds into the second.
#[test]
fn the_sequence_repeats_without_stale_state() {
    on_gc7000(|| {
        let a = framebuffer(4, None).unwrap();
        let one = framebuffer(0, None).unwrap();
        let r1 = pass(&a, &one, (RED, RED8), (BLUE, BLUE8), true);
        check_edge(&r1.edge, RED8, BLUE8);
        let r2 = pass(&a, &one, (GREEN, GREEN8), (RED, RED8), false);
        check_edge(&r2.edge, GREEN8, RED8);
        assert_eq!(r1.a.targets[0], r2.a.targets[0], "the same storage");
        assert_eq!(r1.a.ident, r2.a.ident, "the same identity");
        assert_eq!(r1.one.targets[0], r2.one.targets[0]);
        assert_eq!(r1.window.targets[0], r2.window.targets[0]);
        /* the PE and RS state of the 1x and window jobs does not carry the 4x
         * supertiling or downsample: every kick that names no 4x draw has no
         * downsample, checked on the last single-sample kick */
        let m = kicked();
        bind(one.fb);
        let _ = draw_single(WHITE);
        let w = kicks_since(m);
        assert!(!downsamples(&w), "a 1x job does not resolve");
        let fmt = w.iter().rev().find(|(r, _)| *r == GC_PE_COLOR_FORMAT).map(|(_, v)| *v).expect("a PE format write");
        assert_eq!(fmt & GC_PE_COLOR_FORMAT_SUPER_TILED, 0, "a 1x job is not supertiled");
    });
}

/// A third pass after a longer run keeps working: 4x, window, 1x order too.
#[test]
fn the_other_order_works_too() {
    on_gc7000(|| {
        let a = framebuffer(4, None).unwrap();
        let one = framebuffer(0, None).unwrap();
        let d = ms_draws();
        bind(0);
        let _ = draw_single(WHITE);
        bind(one.fb);
        let _ = draw_single(GREEN);
        bind(a.fb);
        expect_layers([all(BLACK8), left_half_edge(RED8)]);
        clear(BLACK);
        let sa = draw_left_of(HALF_EDGE_X, 0.5, RED);
        assert_eq!(sa.samples, 4);
        assert_eq!(ms_draws_since(d).len(), 2, "no 4x draw for the other two");
        bind(0);
        let _ = draw_single(BLUE);
        bind(a.fb);
        expect_layers([right_half_edge(BLUE8)]);
        assert_eq!(draw_quad(HALF_EDGE_X, 1.0, 0.5, BLUE).samples, 4);
        let (e, _dst) = resolve_blit(&a, GL_COLOR_BUFFER_BIT);
        assert_eq!(e, GL_NO_ERROR);
        check_edge(&[pixel(3), pixel(8), pixel(13)], RED8, BLUE8);
    });
}

/// `GL_SAMPLES` follows the bound draw framebuffer, not the read one.
#[test]
fn gl_samples_follows_the_draw_framebuffer() {
    on_gc7000(|| {
        let a = framebuffer(4, None).unwrap();
        let one = framebuffer(0, None).unwrap();
        let pair = |read: GLuint, draw: GLuint| {
            // SAFETY: framebuffers this test made, or 0.
            unsafe {
                glBindFramebuffer(GL_READ_FRAMEBUFFER, read);
                glBindFramebuffer(GL_DRAW_FRAMEBUFFER, draw);
            }
            (geti(GL_SAMPLES), geti(GL_SAMPLE_BUFFERS))
        };
        assert_eq!(pair(one.fb, a.fb), (4, 1));
        assert_eq!(pair(a.fb, one.fb), (0, 0));
        assert_eq!(pair(a.fb, a.fb), (4, 1));
        let w = pair(a.fb, 0);
        assert_eq!(pair(0, a.fb), (4, 1));
        assert_eq!(pair(0, one.fb), (0, 0));
        assert!(w.0 <= 1 || w.0 == 4, "the window's own count: {w:?}");
        bind(a.fb);
        assert_eq!(geti(GL_SAMPLES), 4);
        bind(0);
        let win = geti(GL_SAMPLES);
        bind(one.fb);
        assert_eq!(geti(GL_SAMPLES), 0);
        bind(0);
        assert_eq!(geti(GL_SAMPLES), win, "the window's count is stable");
        bind(a.fb);
        assert_eq!(geti(GL_SAMPLES), 4);
    });
}
