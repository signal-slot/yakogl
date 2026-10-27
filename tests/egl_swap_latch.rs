// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! A double-buffered window on a display whose flip does not wait (the
//! AM62P's DSS): `present` writes the new address into a shadow register,
//! and the display takes it at its next frame end. A present before that
//! frame end replaces the address waiting in the shadow (a mailbox). The
//! panel can then still scan a buffer the swap has handed back for the next
//! frame. docs/notes/am62p-flip-latch.md explains the defect and the fix:
//! four buffers and `display_busy`, the display's answer to "which of these
//! buffers may you still scan".
//!
//! What it pins:
//!
//! * Without `display_busy` (the ring of three, which the Pi 4 and the
//!   i.MX8MP keep) the GPU draws into a buffer the display scans or is about
//!   to latch, once there are more than about two swaps per vertical
//!   period. This is the defect, shown on the model.
//! * With `display_busy` (the AM62P): over 5000 swaps at 0.5 to 100 swaps a
//!   period, with the vertical periods at random offsets, no frame is drawn
//!   into a buffer the display scans or holds in its shadow, from the kick
//!   to the frame's end. No swap waits for a vertical period, and no swap
//!   waits for its own frame's end.
//! * Without it: three buffers (two allocated besides the window's) in
//!   today's ring order. With it: four (three allocated). When the fourth
//!   cannot be allocated, the window is rendered in place, as when the
//!   second cannot.
//! * A report that holds every buffer for a moment (allowed: too large is
//!   safe) makes the swap wait for one vertical period, not draw into a
//!   held buffer.
//!
//! The GPU is a fake: a kick runs from its submit for `GPU_NS`, or until the
//! library waits for it. Time is fake too: the application's CPU work
//! advances it by the frame time between swaps. Run it with any GPU feature
//! (the backend is the fake's):
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr --test egl_swap_latch
//! ```
#![cfg(all(feature = "host-test", any(feature = "gpu-powervr", feature = "gpu-vivante", feature = "gpu-v3d")))]

use core::ffi::c_void;
use std::sync::Mutex;

use yakogl::backend::{self, GlesDraw, YakoglBackend, GLES_HEAP_PROGRAM, GLES_HEAP_SHADER};
use yakogl::gl::consts::GL_COLOR_BUFFER_BIT;
use yakogl::gl::egl::*;
use yakogl::gl::egl_consts::*;
use yakogl::gl::types::*;
use yakogl::osal::{OsalGpuMem, OsalOps};

/// One test at a time: the EGL state, the OSAL table and the fake are
/// process-wide.
static SERIAL: Mutex<()> = Mutex::new(());

/// The vertical period: the AM62P panel runs at about 58.8 Hz.
const PERIOD_NS: u64 = 17_000_000;
/// A kick's GPU time (a light glmark2 scene).
const GPU_NS: u64 = 300_000;

/* ---- the fake display and GPU -------------------------------------------------- */

/// One kick handed to the GPU.
struct Kick {
    target: u64,
    submit_ns: u64,
    end_ns: u64,
}

#[derive(Default)]
struct Fake {
    now_ns: u64,
    /// The next frame end of the display.
    next_vsync_ns: u64,
    /// The address waiting for the next frame end, and the one the panel scans.
    shadow: Option<u64>,
    scanned: Option<u64>,
    /// The target of the kick being recorded, if a draw opened one.
    open: Option<u64>,
    kicks: Vec<Kick>,
    /// Times the GPU drew into a buffer the display held, and the first few.
    violations: u32,
    first: Vec<String>,
    /// Calls made by the library.
    vblanks: u32,
    busy_calls: u32,
    allocs: u32,
    /// Swaps that waited for the end of the frame they submitted.
    end_waits: u32,
    /// The first `busy_all` calls of `display_busy` report every buffer busy.
    busy_all: u32,
    /// `gpu_alloc` number `fail_alloc` (1-based) fails.
    fail_alloc: Option<u32>,
    /// The swap in progress (for the violation messages).
    swap: u32,
}

static FAKE: Mutex<Option<Fake>> = Mutex::new(None);

fn with_fake<R>(f: impl FnOnce(&mut Fake) -> R) -> R {
    f(FAKE.lock().unwrap_or_else(|e| e.into_inner()).as_mut().expect("the fake is installed"))
}

impl Fake {
    fn violation(&mut self, what: String) {
        self.violations += 1;
        if self.first.len() < 4 {
            self.first.push(what);
        }
    }

    /// Kicks the GPU runs now.
    fn running(&self) -> impl Iterator<Item = &Kick> {
        self.kicks.iter().filter(|k| k.submit_ns <= self.now_ns && self.now_ns < k.end_ns)
    }

    /// The invariant: no running kick renders into a buffer the display
    /// scans or holds in its shadow.
    fn check(&mut self, at: &str) {
        let held = [self.scanned, self.shadow];
        let bad: Vec<u64> = self.running().map(|k| k.target).filter(|t| held.contains(&Some(*t))).collect();
        for t in bad {
            let what = format!(
                "swap {} at {} us ({at}): the GPU draws into {t:#x}; scanned {:x?}, shadow {:x?}",
                self.swap,
                self.now_ns / 1000,
                self.scanned,
                self.shadow
            );
            self.violation(what);
        }
    }

    /// Let `dt` pass: every frame end on the way latches the shadow.
    fn advance(&mut self, dt: u64) {
        let end = self.now_ns + dt;
        while self.next_vsync_ns <= end {
            self.now_ns = self.next_vsync_ns;
            if let Some(a) = self.shadow.take() {
                self.scanned = Some(a);
            }
            self.check("frame end");
            self.next_vsync_ns += PERIOD_NS;
        }
        self.now_ns = end;
    }

    fn submit(&mut self) {
        if let Some(target) = self.open.take() {
            let now = self.now_ns;
            self.kicks.push(Kick { target, submit_ns: now, end_ns: now + GPU_NS });
            self.check("submit");
        }
    }

    /// The library waits for every kick but the last `keep`.
    fn wait_for(&mut self, keep: usize) {
        let n = self.kicks.len().saturating_sub(keep);
        if let Some(end) = self.kicks[..n].iter().map(|k| k.end_ns).max() {
            if end > self.now_ns {
                self.advance(end - self.now_ns);
            }
        }
        let now = self.now_ns;
        for k in &mut self.kicks[..n] {
            k.end_ns = k.end_ns.min(now);
        }
    }

    fn completed(&self) -> u32 {
        self.kicks.iter().take_while(|k| k.end_ns <= self.now_ns).count() as u32
    }
}

/* ---- the platform: host memory, the latching display ---------------------------- */

unsafe extern "C" fn m_init(_arg: *mut c_void) -> i32 {
    0
}

unsafe extern "C" fn m_gpu_alloc(_heap: u64, size: usize, _flags: u32, out: *mut OsalGpuMem) -> i32 {
    if out.is_null() || size == 0 {
        return -22;
    }
    let fail = with_fake(|f| {
        f.allocs += 1;
        f.fail_alloc == Some(f.allocs)
    });
    if fail {
        return -12;
    }
    let layout = std::alloc::Layout::from_size_align(size, 4096).expect("a layout");
    // SAFETY: a non-zero size. The memory is kept for the process (a test
    // allocates a few small buffers; the free path is not under test).
    let p = unsafe { std::alloc::alloc_zeroed(layout) };
    assert!(!p.is_null(), "host allocation of {size} bytes");
    // SAFETY: `out` is a valid out-parameter of the library's call.
    unsafe { *out = OsalGpuMem { cpu: p.cast(), gpu: p as u64, phys: p as u64, size, impl_: core::ptr::null_mut() } };
    0
}

unsafe extern "C" fn m_gpu_free(_mem: *mut OsalGpuMem) -> i32 {
    0
}

unsafe extern "C" fn m_map_phys(_heap: u64, phys: u64, size: usize, _flags: u32, out: *mut OsalGpuMem) -> i32 {
    // SAFETY: `out` is a valid out-parameter of the library's call.
    unsafe { *out = OsalGpuMem { cpu: core::ptr::null_mut(), gpu: phys, phys, size, impl_: core::ptr::null_mut() } };
    0
}

unsafe extern "C" fn m_gpu_rw(_m: *mut OsalGpuMem, _off: usize, _p: *mut c_void, _n: usize) -> i32 {
    0
}

unsafe extern "C" fn m_gpu_write(m: *mut OsalGpuMem, off: usize, src: *const c_void, n: usize) -> i32 {
    // SAFETY: forwarded as received; the fake does not touch the memory.
    unsafe { m_gpu_rw(m, off, src.cast_mut(), n) }
}

unsafe extern "C" fn m_cache(_cpu: *mut c_void, _n: usize) {}

/// The DSS flip (soc/am62p/dss.rs set_app_buffer): the address goes to the
/// shadow and the call returns at once. A shadow address not yet latched is
/// replaced.
unsafe extern "C" fn m_present(mem: *const OsalGpuMem, _w: u32, _h: u32, _stride: u32, _format: u32) -> i32 {
    // SAFETY: the library passes one of its buffers.
    let phys = unsafe { (*mem).phys };
    with_fake(|f| {
        f.shadow = Some(phys);
        f.check("present");
    });
    0
}

/// A vertical period: the time runs to the next frame end.
unsafe extern "C" fn m_wait_vblank(_timeout_us: i64) -> i32 {
    with_fake(|f| {
        f.vblanks += 1;
        let dt = f.next_vsync_ns - f.now_ns;
        f.advance(dt);
    });
    0
}

/// The display's answer: the buffers it scans or holds in its shadow (exact).
unsafe extern "C" fn m_display_busy(phys: *const u64, n: usize) -> u32 {
    assert!(n <= 32);
    // SAFETY: the library passes `n` addresses.
    let phys = unsafe { core::slice::from_raw_parts(phys, n) };
    with_fake(|f| {
        f.busy_calls += 1;
        if f.busy_all > 0 {
            f.busy_all -= 1;
            return (1u32 << n) - 1;
        }
        let held = [f.scanned, f.shadow];
        phys.iter().enumerate().filter(|(_, p)| held.contains(&Some(**p))).fold(0, |m, (k, _)| m | 1 << k)
    })
}

/// The AM62P before this change: a flip that does not wait, no vertical
/// sync for EGL, no `display_busy` (also the shape of the Pi 4's and the
/// i.MX8MP's tables, whose displays differ).
static OSAL_RING: OsalOps = OsalOps {
    init: Some(m_init),
    shutdown: None,
    gpu_alloc: Some(m_gpu_alloc),
    gpu_free: Some(m_gpu_free),
    mmu_map: None,
    map_phys: Some(m_map_phys),
    gpu_write: Some(m_gpu_write),
    gpu_read: Some(m_gpu_rw),
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
    present: Some(m_present),
    lock: None,
    unlock: None,
    thread_id: None,
    display_busy: None,
};

/// The AM62P with this change: `display_busy` (board_am62p.c over the DSS
/// driver), still no vertical sync for EGL.
static OSAL_BUSY: OsalOps = OsalOps { display_busy: Some(m_display_busy), ..OSAL_RING };

/// The same with a vertical-sync source, so that a swap that waits for a
/// period can be counted (the runs set swap interval 0, as glmark2 does).
static OSAL_BUSY_VBLANK: OsalOps = OsalOps { wait_vblank: Some(m_wait_vblank), ..OSAL_BUSY };

/* ---- the fake GPU ------------------------------------------------------------ */

unsafe extern "C" fn b_init() -> i32 {
    0
}

unsafe extern "C" fn b_heap_base(heap: u32) -> u64 {
    match heap {
        GLES_HEAP_SHADER => 0x1000_0000,
        GLES_HEAP_PROGRAM => 0x2000_0000,
        _ => 0x4000_0000,
    }
}

/// A draw opens the kick being recorded; a draw into another target submits
/// the open one first (as the backends start a frame per target).
unsafe extern "C" fn b_draw(d: *const GlesDraw) -> i32 {
    // SAFETY: the core passes a valid draw.
    let target = unsafe { (*d).target_gpu };
    with_fake(|f| {
        if f.open.is_some_and(|t| t != target) {
            f.submit();
        }
        f.open = Some(target);
    });
    0
}

unsafe extern "C" fn b_flush() -> i32 {
    with_fake(|f| f.submit());
    0
}

unsafe extern "C" fn b_finish() -> i32 {
    with_fake(|f| {
        f.submit();
        f.end_waits += 1;
        f.wait_for(0);
    });
    0
}

unsafe extern "C" fn b_finish_previous() -> i32 {
    with_fake(|f| f.wait_for(1));
    0
}

unsafe extern "C" fn b_kick_id() -> u32 {
    with_fake(|f| f.kicks.len() as u32 + 1)
}

unsafe extern "C" fn b_kick_pending(id: u32) -> bool {
    with_fake(|f| id > f.completed() && (id as usize <= f.kicks.len() || f.open.is_some()))
}

fn backend_table() -> YakoglBackend {
    YakoglBackend {
        name: c"egl_swap_latch".as_ptr(),
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
        flush: Some(b_flush),
        finish: Some(b_finish),
        finish_previous: Some(b_finish_previous),
        kick_id: Some(b_kick_id),
        kick_pending: Some(b_kick_pending),
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
    }
}

/* ---- the run -------------------------------------------------------------------- */

const W: u32 = 8;
const H: u32 = 4;

/// A small deterministic generator (no crate for it).
struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        self.0 >> 33
    }
}

struct Setup {
    osal: &'static OsalOps,
    /// Swap interval to set (None: the default, 1).
    interval: Option<EGLint>,
    swaps: u32,
    /// Swaps per vertical period.
    ratio: f64,
    seed: u64,
    busy_all: u32,
    fail_alloc: Option<u32>,
}

impl Setup {
    fn new(osal: &'static OsalOps, swaps: u32, ratio: f64, seed: u64) -> Setup {
        Setup { osal, interval: None, swaps, ratio, seed, busy_all: 0, fail_alloc: None }
    }
}

struct Run {
    fake: Fake,
    /// The target of each frame's kick, in order (a clear is one kick,
    /// recorded at the swap that flushes it).
    targets: Vec<u64>,
    window_phys: u64,
    /// `EGL_RENDER_BUFFER` after the last swap.
    render_buffer: EGLint,
}

/// `setup.swaps` frames: per frame a clear, then the CPU work of the frame
/// time (`PERIOD_NS / ratio`, jittered by up to a fifth either way), then
/// the swap.
fn run(setup: Setup) -> Run {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let mut rng = Lcg(setup.seed);
    let offset = rng.next() % PERIOD_NS;
    *FAKE.lock().unwrap_or_else(|e| e.into_inner()) =
        Some(Fake { next_vsync_ns: offset, busy_all: setup.busy_all, fail_alloc: setup.fail_alloc, ..Fake::default() });
    let table: &'static YakoglBackend = Box::leak(Box::new(backend_table()));
    let mut pixels = vec![0u8; (W * H * 4) as usize + 4096];
    let phys = (pixels.as_mut_ptr() as u64 + 4095) & !4095;
    let mut win =
        GlesNativeWindow { phys, width: W, height: H, stride_bytes: W * 4, format: GLES_NATIVE_FORMAT_XRGB8888 };
    #[rustfmt::skip]
    let cfg_list = [
        EGL_RED_SIZE, 8, EGL_GREEN_SIZE, 8, EGL_BLUE_SIZE, 8,
        EGL_SURFACE_TYPE, EGL_WINDOW_BIT, EGL_RENDERABLE_TYPE, EGL_OPENGL_ES2_BIT, EGL_NONE,
    ];
    let ctx_list = [EGL_CONTEXT_CLIENT_VERSION, 2, EGL_NONE];
    let frame_ns = (PERIOD_NS as f64 / setup.ratio) as u64;
    // SAFETY: tests are serialised; the tables are static or leaked; every
    // pointer passed is live for its call.
    unsafe {
        yakogl::osal::osal_set_ops(setup.osal);
        backend::g_backend = table;
        let dpy = eglGetDisplay(core::ptr::null_mut());
        assert_eq!(eglInitialize(dpy, core::ptr::null_mut(), core::ptr::null_mut()), 1);
        let mut cfg = core::ptr::null_mut();
        let mut n = 0;
        assert_eq!(eglChooseConfig(dpy, cfg_list.as_ptr(), &mut cfg, 1, &mut n), 1);
        let s = eglCreateWindowSurface(dpy, cfg, &mut win, core::ptr::null());
        assert!(!s.is_null(), "eglCreateWindowSurface: {:#x}", eglGetError());
        let c = eglCreateContext(dpy, cfg, core::ptr::null_mut(), ctx_list.as_ptr());
        assert!(!c.is_null());
        assert_eq!(eglMakeCurrent(dpy, s, s, c), 1);
        if let Some(i) = setup.interval {
            assert_eq!(eglSwapInterval(dpy, i), 1);
        }
        for frame in 0..setup.swaps {
            yakogl::gl::state::glClearColor(0.0, 0.0, (frame % 7) as f32 / 7.0, 1.0);
            yakogl::gl::state::glClear(GL_COLOR_BUFFER_BIT);
            let jitter = frame_ns / 5;
            let dt = frame_ns - jitter + if jitter == 0 { 0 } else { rng.next() % (2 * jitter) };
            with_fake(|f| {
                f.advance(dt);
                f.swap = frame;
            });
            assert_eq!(eglSwapBuffers(dpy, s), 1, "eglSwapBuffers of frame {frame}");
        }
        let mut rb = 0;
        assert_eq!(eglQuerySurface(dpy, s, EGL_RENDER_BUFFER, &mut rb), 1);
        /* the teardown's waits are not the run's: it gets a fresh fake */
        let fake = FAKE.lock().unwrap_or_else(|e| e.into_inner()).replace(Fake::default()).expect("the fake");
        eglMakeCurrent(dpy, core::ptr::null_mut(), core::ptr::null_mut(), core::ptr::null_mut());
        eglDestroyContext(dpy, c);
        eglDestroySurface(dpy, s);
        eglTerminate(dpy);
        drop(pixels);
        let targets = fake.kicks.iter().map(|k| k.target).collect();
        Run { fake, targets, window_phys: phys, render_buffer: rb }
    }
}

const RATIOS: [f64; 6] = [0.5, 1.0, 2.0, 3.0, 10.0, 100.0];

/* ---- without display_busy: the defect ----------------------------------------------- */

#[test]
fn the_ring_of_three_draws_into_a_buffer_the_display_holds_above_two_swaps_a_period() {
    for ratio in [3.0, 10.0] {
        let r = run(Setup::new(&OSAL_RING, 200, ratio, 1));
        assert!(
            r.fake.violations > 0,
            "{ratio} swaps a period: the blind ring of three was expected to draw into a held buffer"
        );
    }
    /* at one swap per two periods a frame end always comes between a
     * present and the next kick */
    let r = run(Setup::new(&OSAL_RING, 200, 0.5, 1));
    assert_eq!(r.fake.violations, 0, "{:?}", r.fake.first);
}

/* ---- with display_busy: the fix ------------------------------------------------------ */

#[test]
fn with_display_busy_no_frame_is_drawn_into_a_buffer_the_display_holds() {
    for ratio in RATIOS {
        for seed in 1..=3 {
            let mut setup = Setup::new(&OSAL_BUSY_VBLANK, 5000, ratio, seed);
            setup.interval = Some(0);
            let r = run(setup);
            let who = format!("{ratio} swaps a period, seed {seed}");
            assert_eq!(r.fake.violations, 0, "{who}: {:?}", r.fake.first);
            assert_eq!(r.fake.vblanks, 0, "{who}: a swap waited for a vertical period");
            assert_eq!(r.fake.end_waits, 0, "{who}: a swap waited for its own frame's end");
            assert!(r.fake.busy_calls >= 4999, "{who}: the swap asks the display ({})", r.fake.busy_calls);
            assert_eq!(r.render_buffer, EGL_BACK_BUFFER);
        }
    }
}

#[test]
fn the_am62p_table_without_vertical_sync_holds_the_same() {
    /* the board's own shape: no wait_vblank, the default interval */
    for ratio in [1.0, 10.0, 100.0] {
        let r = run(Setup::new(&OSAL_BUSY, 2000, ratio, 7));
        assert_eq!(r.fake.violations, 0, "{ratio}: {:?}", r.fake.first);
        assert_eq!(r.fake.end_waits, 0, "{ratio}");
    }
}

#[test]
fn a_report_that_holds_every_buffer_waits_one_period_instead_of_drawing_into_one() {
    /* the first two answers say "all busy": the first swap waits a period,
     * asks again, waits another, and then takes the buffer that is free */
    let mut setup = Setup::new(&OSAL_BUSY_VBLANK, 50, 10.0, 3);
    setup.interval = Some(0);
    setup.busy_all = 2;
    let r = run(setup);
    assert_eq!(r.fake.violations, 0, "{:?}", r.fake.first);
    assert_eq!(r.fake.vblanks, 2);
    assert_eq!(r.fake.end_waits, 0);
}

/* ---- buffers and order ------------------------------------------------------------- */

#[test]
fn without_display_busy_three_buffers_in_todays_ring() {
    let r = run(Setup::new(&OSAL_RING, 9, 1.0, 1));
    assert_eq!(r.fake.allocs, 2, "two buffers besides the window's");
    assert_eq!(r.targets[0], r.window_phys, "frame 0 goes into the window");
    let ring = &r.targets[..3];
    assert!(ring[0] != ring[1] && ring[1] != ring[2] && ring[0] != ring[2], "{ring:x?}");
    for (i, t) in r.targets.iter().enumerate() {
        assert_eq!(*t, ring[i % 3], "frame {i}: the ring order");
    }
}

#[test]
fn with_display_busy_four_buffers() {
    let r = run(Setup::new(&OSAL_BUSY, 40, 10.0, 1));
    assert_eq!(r.fake.allocs, 3, "three buffers besides the window's");
    let mut used = r.targets.clone();
    used.sort_unstable();
    used.dedup();
    assert_eq!(used.len(), 4, "{used:x?}");
}

#[test]
fn no_memory_for_the_fourth_buffer_renders_the_window_in_place() {
    let mut setup = Setup::new(&OSAL_BUSY, 6, 1.0, 1);
    setup.fail_alloc = Some(3);
    let r = run(setup);
    assert_eq!(r.fake.allocs, 3);
    assert!(r.targets.iter().all(|t| *t == r.window_phys), "{:x?}", r.targets);
    assert_eq!(r.fake.shadow, None, "nothing presented");
    assert_eq!(r.fake.end_waits, 6, "every swap completes its frame");
}
