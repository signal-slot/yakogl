// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! 4x framebuffer objects on the GC7000UL (i.MX8MP), through the GL API into
//! the Vivante back end, on the host (item msaa-pin-gc-file).
//!
//! docs/notes/es3-msaa-renderbuffers.md section (0): the GL core never makes a
//! 4x framebuffer object. `GL_MAX_SAMPLES` is 1, `glRenderbufferStorageMultisample`
//! gives `GL_INVALID_OPERATION` for 2 or more samples, one sample is plain
//! storage with `GL_RENDERBUFFER_SAMPLES` 1, and a draw into it reaches the
//! back end with `GlesDraw::samples` 1. Only the window is 4x on this core, and
//! only behind the `gc_debug_msaa` knob (note (c3), "Gate").
//!
//! The real `yakogl_backend_vivante` runs on a stand-in OSAL (host memory, every
//! kick kept), as `tests/es3_mrt4_gc.rs`; its table is copied with `draw` and
//! `attachment_gone` replaced by recorders that forward to the real ones, so
//! the test sees the `GlesDraw` and the object ids the GL core hands the back
//! end. The GPU draws nothing: no pixel of a kick exists on the host.
//!
//! Passing tests pin today. The `#[ignore]`d ones are the expected result after
//! the note's design (the contract (b), the GC7000UL's scratch pool (c3)), each
//! naming the note section and the owner: the GL core (`gl-msaa-cap`: capability,
//! counts, completeness, draw identity; `gl-msaa-resolve`: the resolve blit) or
//! the GC back end (`gc-msaa-fbo`, `src/vivante`). They fail today at
//! `glRenderbufferStorageMultisample`. Those that read pixels also need the
//! host stand-in to model the RS's 2x2 downsample (the kick is not executed
//! here), which is `gc-msaa-fbo`'s host test, or a board.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online --test es3_msaa_gc
//! ```

#![cfg(all(feature = "host-test", feature = "gpu-vivante", feature = "shaderc-online", not(feature = "report-es2")))]

use core::ffi::{c_char, c_void};
use std::collections::BTreeMap;
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
use yakogl::vivante::regs::*;

/// The online compiler's store is one static for the process: every test
/// takes this lock.
static STORE: Mutex<()> = Mutex::new(());

/* ---- the stand-in OSAL: host memory, every stream kept -------------------- */

#[derive(Default)]
struct Platform {
    allocs: BTreeMap<u64, Box<[u64]>>,
    bump: BTreeMap<u64, u64>,
    fence: i64,
    kicks: Vec<Vec<u32>>,
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
        let at = p.bump.entry(heap_base).or_insert(0);
        let gpu = heap_base + 0x1000 + *at;
        *at += (size as u64).div_ceil(4096) * 4096;
        let cpu = mem.as_mut_ptr() as *mut c_void;
        // SAFETY: the caller passes a writable descriptor.
        unsafe { *out = OsalGpuMem { cpu, gpu, phys: gpu, size, impl_: cpu } };
        p.allocs.insert(gpu, mem);
        0
    })
}

unsafe extern "C" fn p_gpu_free(mem: *mut OsalGpuMem) -> i32 {
    // SAFETY: the caller passes a live descriptor; it is cleared (the OSAL contract).
    unsafe {
        let m = *mem;
        with_platform(|p| p.allocs.remove(&m.gpu));
        *mem = OsalGpuMem::ZERO;
    }
    0
}

unsafe extern "C" fn p_cache(_cpu: *mut c_void, _n: usize) {}

/// Every `(register, value)` a LOAD_STATE of the stream writes, in order.
fn writes(words: &[u32]) -> Vec<(u32, u32)> {
    let mut out = Vec::new();
    let mut i = 0usize;
    while i < words.len() {
        let w = words[i];
        if w & 0xf800_0000 == GC_OP_LOAD_STATE {
            let count = ((w >> 16) & 0x3ff) as usize;
            let base = (w & 0xffff) * 4;
            for k in 0..count {
                if let Some(v) = words.get(i + 1 + k) {
                    out.push((base + 4 * k as u32, *v));
                }
            }
            i += 1 + count;
            continue;
        }
        i += match w & 0xf800_0000 {
            GC_OP_DRAW_INSTANCED => 4,
            GC_OP_STALL => 2,
            _ => 1,
        };
    }
    out
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
        let words: &[u32] =
            unsafe { core::slice::from_raw_parts(a.as_ptr() as *const u32, k.stream_bytes as usize / 4) };
        p.kicks.push(words.to_vec());
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
    });
    if let Err(e) = r {
        std::panic::resume_unwind(e);
    }
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

/// A bound `SIZE` x `SIZE` framebuffer with an RGBA8 renderbuffer of `samples`
/// and, when `depth` names a format, a depth renderbuffer of the same count. A
/// storage error comes back as `Err(error)`.
fn framebuffer(samples: GLsizei, depth: Option<GLenum>) -> Result<Fbo, GLenum> {
    let mut fb = 0;
    // SAFETY: plain GL calls.
    unsafe {
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
    }
    let (e, colour) = storage(samples, GL_RGBA8);
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

/// Draw the quad `[-1, right] x [-1, 1]` at depth `z` in `rgba` and hand back
/// what the back end was given for it. The NDC x of pixel edge `px` is
/// `px / SIZE * 2 - 1`.
fn draw_left_of(right: f32, z: f32, rgba: [f32; 4]) -> Seen {
    let quad = [-1.0, -1.0, z, 1.0, right, -1.0, z, 1.0, -1.0, 1.0, z, 1.0, right, 1.0, z, 1.0];
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

/* ---- today: the queries ---------------------------------------------------- */

/// `GL_MAX_SAMPLES` is 1 (`src/gl/state.rs`), below ES 3.0's minimum of 4.
#[test]
fn max_samples_is_one_today() {
    on_gc7000(|| assert_eq!(geti(GL_MAX_SAMPLES), 1));
}

/// 4 once the back end's capability is on (note (a), (b)1).
#[test]
#[ignore = "note (a)/(b)1, owner gl-msaa-cap (src/gl): GL_MAX_SAMPLES is the constant 1 until the table's max_samples is read; the GC table says 4 under msaa-fbo only after gc-msaa-fbo"]
fn max_samples_is_four() {
    on_gc7000(|| assert_eq!(geti(GL_MAX_SAMPLES), 4));
}

/// `glGetInternalformativ`: one sample count, 1, for a renderable format; no
/// counts for an integer one (ES 3.0 4.4.2.1).
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

/// The count 4 for the multisample-renderable formats, none for integer ones.
#[test]
#[ignore = "note (b)1, owner gl-msaa-cap (src/gl): glGetInternalformativ answers the count 1, not 4, whatever the back end"]
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

/* ---- today: the storage ------------------------------------------------------ */

/// 4 (and 2) samples: `GL_INVALID_OPERATION`, past `GL_MAX_SAMPLES`.
#[test]
fn four_samples_is_invalid_operation() {
    on_gc7000(|| {
        assert_eq!(storage(4, GL_RGBA8).0, GL_INVALID_OPERATION);
        assert_eq!(storage(2, GL_RGBA8).0, GL_INVALID_OPERATION);
        assert_eq!(storage(4, GL_DEPTH24_STENCIL8).0, GL_INVALID_OPERATION);
        assert_eq!(framebuffer(4, None).err(), Some(GL_INVALID_OPERATION));
    });
}

/// A negative count is `GL_INVALID_VALUE`, checked before the limit; an
/// integer format is refused at any count above 0, the spec's rule and so
/// still true after the work.
#[test]
fn negative_and_integer_counts_are_refused() {
    on_gc7000(|| {
        assert_eq!(storage(-1, GL_RGBA8).0, GL_INVALID_VALUE);
        assert_eq!(storage(1, GL_RGBA8UI).0, GL_INVALID_OPERATION);
        assert_eq!(storage(4, GL_RGBA32UI).0, GL_INVALID_OPERATION);
    });
}

/// One sample is plain storage that reads back as 1; zero reads back as 0; the
/// framebuffer of the one-sample storage has "sample buffers" (completeness).
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

/// The same for the zero-sample framebuffer.
#[test]
fn a_draw_into_zero_samples_reaches_the_back_end_single_sampled() {
    on_gc7000(|| {
        let _fbo = framebuffer(0, None).unwrap();
        assert_eq!(draw(RED).samples, 1);
    });
}

/// A resolve blit from a one-sample framebuffer into a single-sample one is
/// the plain CPU copy and gives no error; a blit *into* a one-sample
/// framebuffer (it has sample buffers) is refused by the spec.
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

/* ---- the expected result: ignored until the design lands -------------------- */

/// A 4x RGBA8 framebuffer: storage is legal, reads back 4, the framebuffer is
/// complete, and the draw reaches the back end with `samples` 4 and ends in the
/// RS's 2x2 downsample into `rb.mem` (note (c3): the scratch surface twice the
/// target on each axis, resolved at every job end).
#[test]
#[ignore = "note (b)1/(b)4 + (c3), owners gl-msaa-cap (storage, draw samples) and gc-msaa-fbo (src/vivante: scratch pool, RS resolve at job end)"]
fn rgba8_4x_fbo_draws_four_samples_and_resolves() {
    on_gc7000(|| {
        let fbo = framebuffer(4, None).expect("4 samples are legal storage");
        assert_eq!(rb_samples(fbo.colour), 4);
        assert_eq!(geti(GL_SAMPLES), 4);
        let m = kicked();
        let s = draw(RED);
        assert_eq!((s.samples, s.targets.len(), s.size), (4, 1, (SIZE, SIZE)));
        assert!(downsamples(&kicks_since(m)), "the job ends in the RS downsample");
    });
}

/// A 4x RGBA8 FBO with a half-covered edge, resolved by a blit: the pixel the
/// edge crosses averages its four samples (two red, two black), the interior
/// is exact. Pixels need the RS modelled on the host, or a board.
#[test]
#[ignore = "note (b)7/(b)9 + (c3), owners gl-msaa-cap, gl-msaa-resolve (src/gl: the blit) and gc-msaa-fbo (src/vivante: the edge survives in the scratch and the RS averages it)"]
fn a_half_covered_edge_resolves_to_the_average() {
    on_gc7000(|| {
        let fbo = framebuffer(4, None).expect("4 samples are legal storage");
        clear(BLACK);
        let s = draw_left_of(HALF_EDGE_X, 0.5, RED);
        assert_eq!(s.samples, 4);
        let (e, _dst) = resolve_blit(&fbo, GL_COLOR_BUFFER_BIT);
        assert_eq!(e, GL_NO_ERROR);
        assert_eq!(pixel(3), [255, 0, 0, 255], "interior, exact");
        assert_eq!(pixel(13), [0, 0, 0, 255], "outside, exact");
        let edge = pixel(8);
        assert!((127..=128).contains(&edge[0]), "two of four samples red: {edge:?}");
        assert_eq!((edge[1], edge[2], edge[3]), (0, 0, 255), "{edge:?}");
    });
}

/// A resolve blit that is scaled, or whose regions differ, is
/// `GL_INVALID_OPERATION` (ES 3.0 4.3.3).
#[test]
#[ignore = "note (b)9, owner gl-msaa-resolve (src/gl/blit.rs): the multisample-read blit errors are not made while no framebuffer is 4x"]
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
#[test]
#[ignore = "note (b)8 + (c3) 'Depth', owners gl-msaa-resolve (blit of DEPTH/STENCIL_BUFFER_BIT) and gc-msaa-fbo (ds_cache keyed per object at 4x); board-unverified: which sample the PE leaves in the depth buffer"]
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

/// After the storage is specified again at the same size, the back end is told
/// the object's contents are gone (`attachment_gone`), so no sample of the old
/// storage comes back: a draw after a clear shows only what was drawn since.
#[test]
#[ignore = "note (b)5/(b)6 + (c3), owners gl-msaa-cap (target_id/gen, attachment_gone for colour) and gc-msaa-fbo (src/vivante: scratch pool keyed by (target_id, gen))"]
fn no_stale_samples_after_a_same_size_respecification() {
    on_gc7000(|| {
        let fbo = framebuffer(4, None).expect("4 samples are legal storage");
        clear(BLACK);
        let _ = draw(RED);
        let gone = GONE.lock().unwrap().len();
        // SAFETY: a renderbuffer this test made; the same size and format again.
        unsafe {
            glBindRenderbuffer(GL_RENDERBUFFER, fbo.colour);
            glRenderbufferStorageMultisample(GL_RENDERBUFFER, 4, GL_RGBA8, SIZE as GLsizei, SIZE as GLsizei);
        }
        assert_eq!(err(), GL_NO_ERROR);
        assert_eq!(GONE.lock().unwrap().len(), gone + 1, "the back end is told the colour samples are gone");
        clear(BLACK);
        let s = draw_left_of(HALF_EDGE_X, 0.5, BLUE);
        assert_eq!(s.samples, 4);
        let (e, _dst) = resolve_blit(&fbo, GL_COLOR_BUFFER_BIT);
        assert_eq!(e, GL_NO_ERROR);
        assert_eq!(pixel(3), [0, 0, 255, 255], "no red from the old storage");
        let edge = pixel(8);
        assert_eq!(edge[0], 0, "no red in the edge pixel: {edge:?}");
        assert!((127..=128).contains(&edge[2]), "{edge:?}");
    });
}
