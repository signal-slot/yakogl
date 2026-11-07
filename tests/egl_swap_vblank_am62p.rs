// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The AM62P's swap interval, pinned unwired (docs/notes/am62p-flip-latch.md,
//! "Decision: wait_vblank (2026-11)").
//!
//! The AM62P's OSAL table (src/osal/zephyr.c) has `.wait_vblank`, but it
//! calls the weak `yakogl_zephyr_wait_vblank`, which answers -ENOTSUP: the
//! board library (driver/zephyr/lib/board/am62p, common) defines none, and
//! the DSS driver's `am62p_dss_wait_vsync` is not wired to EGL. So
//! `eglSwapInterval` is accepted and ignored there, and no swap waits for a
//! vertical period.
//!
//! Why it stays so. Wiring it would honour interval 1 for every client that
//! does not ask for 0, and EGL's default is 1 (src/gl/egl.rs, `swap_interval:
//! 1`). glmark2 asks for 0 (gl-state-egl.cpp, `eglSwapInterval(egl_display_,
//! 0)` unless --swap-mode fifo, and the image passes no --swap-mode), so its
//! frame times would not move. But gl_test never calls eglSwapInterval and
//! swaps pbuffers, single-buffered windows and the panel cube's 120-frame
//! loop at the default 1, and eglSwapBuffers runs the interval loop for every
//! surface kind: each of those swaps would gain a vertical period (about 17
//! ms on the panel). The rule for the wiring was "no benchmark or
//! conformance frame time moves", and the gl_test image's would. dEQP swaps
//! nothing (its runs use a pbuffer and RenderContext::postIterate swaps
//! windows only), so it is not the reason.
//!
//! What it pins, on a fake of the AM62P's OSAL (present that does not wait,
//! `display_busy`, `wait_vblank` that answers -ENOTSUP, a clock):
//!
//! * interval 0 (glmark2): no `wait_vblank` call in any swap;
//! * the default interval and interval 1 (gl_test, and any other client):
//!   one `wait_vblank` call a swap, which finds no source, so the swap
//!   returns without waiting; a pbuffer's swap the same;
//! * `next_window_buffer` with every window buffer held: it asks
//!   `wait_vblank` once a round, counts no vertical period, and asks the
//!   display again until a buffer is free, or for `BUSY_WAIT_NS` (100 ms) of
//!   the OSAL clock, then takes the ring's buffer;
//! * the sources: no `yakogl_zephyr_wait_vblank` in the AM62P board files or
//!   common/board.c, and zephyr.c's weak one answers -ENOTSUP. A change that
//!   wires it has to change this test and the decision with it.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr --test egl_swap_vblank_am62p
//! ```
#![cfg(all(feature = "host-test", any(feature = "gpu-powervr", feature = "gpu-vivante", feature = "gpu-v3d")))]

use core::ffi::c_void;
use std::sync::Mutex;

use yakogl::backend::{self, GlesDraw, YakoglBackend, GLES_HEAP_PROGRAM, GLES_HEAP_SHADER};
use yakogl::gl::consts::GL_COLOR_BUFFER_BIT;
use yakogl::gl::egl::*;
use yakogl::gl::egl_consts::*;
use yakogl::gl::types::*;
use yakogl::osal::{OsalGpuMem, OsalOps, OSAL_QUERY_CLOCK_NS};

/// One test at a time: the EGL state, the OSAL table and the fake are
/// process-wide.
static SERIAL: Mutex<()> = Mutex::new(());

/// What a `display_busy` call costs on the fake clock (the time between two
/// asks of the swap's busy loop).
const ASK_NS: u64 = 1_000_000;

#[derive(Default)]
struct Fake {
    now_ns: u64,
    /// Calls of `wait_vblank` (each answers -ENOTSUP: no period passes).
    vblank_calls: u32,
    presents: u32,
    busy_calls: u32,
    /// The next `busy_all` calls of `display_busy` report every buffer held.
    busy_all: u32,
    /// Kicks submitted, and completed through.
    submitted: u32,
    completed: u32,
    open: bool,
}

static FAKE: Mutex<Option<Fake>> = Mutex::new(None);

fn with_fake<R>(f: impl FnOnce(&mut Fake) -> R) -> R {
    f(FAKE.lock().unwrap_or_else(|e| e.into_inner()).as_mut().expect("the fake is installed"))
}

/* ---- the platform: zephyr.c on the AM62P ----------------------------------------- */

unsafe extern "C" fn m_init(_arg: *mut c_void) -> i32 {
    0
}

unsafe extern "C" fn m_gpu_alloc(_heap: u64, size: usize, _flags: u32, out: *mut OsalGpuMem) -> i32 {
    if out.is_null() || size == 0 {
        return -22;
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

/// The OSAL clock (z_query, OSAL_QUERY_CLOCK_NS); nothing else is answered.
unsafe extern "C" fn m_query(what: u32, out: *mut c_void, out_sz: usize) -> i32 {
    if what != OSAL_QUERY_CLOCK_NS || out_sz != 8 {
        return -yakogl::util::ENOTSUP;
    }
    let now = with_fake(|f| f.now_ns);
    // SAFETY: the library passes a u64 out-parameter of the size checked.
    unsafe { *out.cast::<u64>() = now };
    0
}

/// board_am62p.c board_show -> sample_glue.c pvr_sample_flip_window: the
/// DSS takes the address at its next frame end; the call does not wait.
unsafe extern "C" fn m_present(_mem: *const OsalGpuMem, _w: u32, _h: u32, _stride: u32, _format: u32) -> i32 {
    with_fake(|f| f.presents += 1);
    0
}

/// zephyr.c z_wait_vblank -> the weak yakogl_zephyr_wait_vblank: no source.
unsafe extern "C" fn m_wait_vblank(_timeout_us: i64) -> i32 {
    with_fake(|f| f.vblank_calls += 1);
    -yakogl::util::ENOTSUP
}

/// common/board.c yakogl_zephyr_display_busy -> board_am62p.c
/// board_display_busy: nothing held, unless the test holds every buffer.
unsafe extern "C" fn m_display_busy(_phys: *const u64, n: usize) -> u32 {
    assert!((1..32).contains(&n));
    with_fake(|f| {
        f.busy_calls += 1;
        f.now_ns += ASK_NS;
        if f.busy_all > 0 {
            f.busy_all -= 1;
            (1u32 << n) - 1
        } else {
            0
        }
    })
}

/// src/osal/zephyr.c osal_zephyr_ops on the AM62P, in what the swap uses.
static OSAL_AM62P: OsalOps = OsalOps {
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
    query: Some(m_query),
    log: None,
    wait_vblank: Some(m_wait_vblank),
    present: Some(m_present),
    lock: None,
    unlock: None,
    thread_id: None,
    display_busy: Some(m_display_busy),
};

/* ---- the fake GPU: a kick ends when the library waits for it ------------------- */

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

unsafe extern "C" fn b_draw(_d: *const GlesDraw) -> i32 {
    with_fake(|f| f.open = true);
    0
}

fn submit(f: &mut Fake) {
    if f.open {
        f.open = false;
        f.submitted += 1;
    }
}

unsafe extern "C" fn b_flush() -> i32 {
    with_fake(submit);
    0
}

unsafe extern "C" fn b_finish() -> i32 {
    with_fake(|f| {
        submit(f);
        f.completed = f.submitted;
    });
    0
}

unsafe extern "C" fn b_finish_previous() -> i32 {
    with_fake(|f| f.completed = f.completed.max(f.submitted.saturating_sub(1)));
    0
}

unsafe extern "C" fn b_kick_id() -> u32 {
    with_fake(|f| f.submitted + 1)
}

unsafe extern "C" fn b_kick_pending(id: u32) -> bool {
    with_fake(|f| id > f.completed && (id <= f.submitted || f.open))
}

fn backend_table() -> YakoglBackend {
    YakoglBackend {
        name: c"egl_swap_vblank_am62p".as_ptr(),
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

#[derive(Clone, Copy, PartialEq)]
enum Surface {
    /// No attribute list: EGL_BACK_BUFFER, what glmark2 asks for.
    Back,
    /// EGL_RENDER_BUFFER EGL_SINGLE_BUFFER: gl_test's windows.
    Single,
    /// gl_test's pbuffer cases.
    Pbuffer,
}

struct Setup {
    surface: Surface,
    /// Swap interval to set (None: EGL's default, 1).
    interval: Option<EGLint>,
    swaps: u32,
    /// Before swap `at`, the display holds every window buffer for `asks`
    /// asks (u32::MAX: for good).
    busy: Option<(u32, u32)>,
}

/// What a run saw, and the fake clock's time spent inside each swap.
struct Run {
    fake: Fake,
    swap_ns: Vec<u64>,
}

fn run(setup: Setup) -> Run {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    *FAKE.lock().unwrap_or_else(|e| e.into_inner()) = Some(Fake::default());
    let table: &'static YakoglBackend = Box::leak(Box::new(backend_table()));
    let mut pixels = vec![0u8; (W * H * 4) as usize + 4096];
    let phys = (pixels.as_mut_ptr() as u64 + 4095) & !4095;
    let mut win =
        GlesNativeWindow { phys, width: W, height: H, stride_bytes: W * 4, format: GLES_NATIVE_FORMAT_XRGB8888 };
    let single = [EGL_RENDER_BUFFER, EGL_SINGLE_BUFFER, EGL_NONE];
    let pb_attribs = [EGL_WIDTH, W as EGLint, EGL_HEIGHT, H as EGLint, EGL_NONE];
    let kind = if setup.surface == Surface::Pbuffer { EGL_PBUFFER_BIT } else { EGL_WINDOW_BIT };
    #[rustfmt::skip]
    let cfg_list = [
        EGL_RED_SIZE, 8, EGL_GREEN_SIZE, 8, EGL_BLUE_SIZE, 8,
        EGL_SURFACE_TYPE, kind, EGL_RENDERABLE_TYPE, EGL_OPENGL_ES2_BIT, EGL_NONE,
    ];
    let ctx_list = [EGL_CONTEXT_CLIENT_VERSION, 2, EGL_NONE];
    let mut swap_ns = Vec::new();
    // SAFETY: tests are serialised; the tables are static or leaked; every
    // pointer passed is live for its call.
    unsafe {
        yakogl::osal::osal_set_ops(&OSAL_AM62P);
        backend::g_backend = table;
        let dpy = eglGetDisplay(core::ptr::null_mut());
        assert_eq!(eglInitialize(dpy, core::ptr::null_mut(), core::ptr::null_mut()), 1);
        let mut cfg = core::ptr::null_mut();
        let mut n = 0;
        assert_eq!(eglChooseConfig(dpy, cfg_list.as_ptr(), &mut cfg, 1, &mut n), 1);
        assert_eq!(n, 1, "a config");
        let s = match setup.surface {
            Surface::Back => eglCreateWindowSurface(dpy, cfg, &mut win, core::ptr::null()),
            Surface::Single => eglCreateWindowSurface(dpy, cfg, &mut win, single.as_ptr()),
            Surface::Pbuffer => eglCreatePbufferSurface(dpy, cfg, pb_attribs.as_ptr()),
        };
        assert!(!s.is_null(), "surface: {:#x}", eglGetError());
        let c = eglCreateContext(dpy, cfg, core::ptr::null_mut(), ctx_list.as_ptr());
        assert!(!c.is_null());
        assert_eq!(eglMakeCurrent(dpy, s, s, c), 1);
        if setup.surface != Surface::Pbuffer {
            let mut rb = 0;
            assert_eq!(eglQuerySurface(dpy, s, EGL_RENDER_BUFFER, &mut rb), 1);
            let want = if setup.surface == Surface::Back { EGL_BACK_BUFFER } else { EGL_SINGLE_BUFFER };
            assert_eq!(rb, want, "EGL_RENDER_BUFFER");
        }
        if let Some(i) = setup.interval {
            assert_eq!(eglSwapInterval(dpy, i), 1);
        }
        for frame in 0..setup.swaps {
            yakogl::gl::state::glClearColor(0.0, 0.0, (frame % 7) as f32 / 7.0, 1.0);
            yakogl::gl::state::glClear(GL_COLOR_BUFFER_BIT);
            if let Some((at, asks)) = setup.busy {
                if at == frame {
                    with_fake(|f| f.busy_all = asks);
                }
            }
            let t0 = with_fake(|f| f.now_ns);
            assert_eq!(eglSwapBuffers(dpy, s), 1, "eglSwapBuffers of frame {frame}");
            swap_ns.push(with_fake(|f| f.now_ns) - t0);
        }
        let fake = FAKE.lock().unwrap_or_else(|e| e.into_inner()).replace(Fake::default()).expect("the fake");
        /* the interval is the display's, and survives eglTerminate (one
         * EglState a process): put EGL's default back for the next run */
        assert_eq!(eglSwapInterval(dpy, 1), 1);
        eglMakeCurrent(dpy, core::ptr::null_mut(), core::ptr::null_mut(), core::ptr::null_mut());
        eglDestroyContext(dpy, c);
        eglDestroySurface(dpy, s);
        eglTerminate(dpy);
        drop(pixels);
        Run { fake, swap_ns }
    }
}

const SWAPS: u32 = 40;

/* ---- the swap interval -------------------------------------------------------------- */

/// glmark2 sets interval 0 (gl-state-egl.cpp GLStateEGL::valid): not one
/// `wait_vblank` call, so wiring a source could not slow it; and no swap
/// spends time (the busy query is the only clock cost, none when free).
#[test]
fn interval_0_the_glmark2_path_makes_no_wait_vblank_call() {
    let r = run(Setup { surface: Surface::Back, interval: Some(0), swaps: SWAPS, busy: None });
    assert_eq!(r.fake.vblank_calls, 0);
    assert_eq!(r.fake.presents, SWAPS - 1, "every swap but the first shows the frame before it");
    assert!(r.fake.busy_calls >= SWAPS - 1, "the four-buffer path asks the display: {}", r.fake.busy_calls);
    assert!(r.swap_ns.iter().all(|&t| t == ASK_NS || t == 0), "{:?}", r.swap_ns);
}

/// EGL's default (1) and an explicit 1: one `wait_vblank` call a swap,
/// which finds no source on the AM62P, so the swap returns at once. This is
/// the accept-and-ignore the decision keeps. With `am62p_dss_wait_vsync`
/// wired, each of these swaps would wait a vertical period.
#[test]
fn interval_1_and_the_default_are_ignored_one_call_a_swap_no_wait() {
    for interval in [None, Some(1)] {
        let r = run(Setup { surface: Surface::Back, interval, swaps: SWAPS, busy: None });
        assert_eq!(r.fake.vblank_calls, SWAPS, "interval {interval:?}");
        assert_eq!(r.fake.presents, SWAPS - 1);
        assert!(r.swap_ns.iter().all(|&t| t == ASK_NS || t == 0), "{:?}", r.swap_ns);
    }
}

/// The reason not to wire it: gl_test never calls eglSwapInterval, and it
/// swaps pbuffers and single-buffered windows. Both run the interval loop
/// at the default 1 (one call a swap here), so a wired source would add a
/// vertical period to each of gl_test's swaps.
#[test]
fn gl_test_swaps_pbuffers_and_single_windows_at_the_default_interval() {
    for surface in [Surface::Pbuffer, Surface::Single] {
        let r = run(Setup { surface, interval: None, swaps: SWAPS, busy: None });
        assert_eq!(r.fake.vblank_calls, SWAPS, "one call a swap");
        assert_eq!(r.fake.presents, 0, "EGL shows neither");
        assert!(r.swap_ns.iter().all(|&t| t == 0), "no swap waits: {:?}", r.swap_ns);
    }
}

/* ---- next_window_buffer with every buffer held --------------------------------------- */

/// A report that holds every buffer for three asks: the swap asks
/// `wait_vblank` once a round (no source: no period counted) and asks the
/// display again; the fourth answer frees one. Interval 0, as glmark2.
#[test]
fn every_buffer_held_for_a_moment_asks_again_without_a_vertical_period() {
    let r = run(Setup { surface: Surface::Back, interval: Some(0), swaps: 8, busy: Some((4, 3)) });
    assert_eq!(r.fake.vblank_calls, 3, "one ask of wait_vblank a round");
    assert_eq!(r.swap_ns[4], 4 * ASK_NS, "three held answers and the free one: {:?}", r.swap_ns);
    assert_eq!(r.fake.presents, 7);
}

/// Held for good: the swap asks again for `BUSY_WAIT_NS` (100 ms) of the
/// OSAL clock, as where there is no vertical-sync source, then takes the
/// ring's buffer; the `BUSY_WAIT_PERIODS` (6) limit never applies.
#[test]
fn every_buffer_held_for_good_gives_up_after_100_ms_of_the_clock() {
    let r = run(Setup { surface: Surface::Back, interval: Some(0), swaps: 6, busy: Some((3, u32::MAX)) });
    let t = r.swap_ns[3];
    assert!((100_000_000..=102_000_000).contains(&t), "the swap asked for {t} ns");
    assert!(r.fake.vblank_calls > 6, "more rounds than BUSY_WAIT_PERIODS: {}", r.fake.vblank_calls);
}

/* ---- the sources: unwired ---------------------------------------------------------- */

/// The board library defines no `yakogl_zephyr_wait_vblank` for the AM62P,
/// so zephyr.c's weak one (-ENOTSUP) is the one linked.
#[test]
fn the_am62p_board_library_defines_no_wait_vblank() {
    let boards = [
        ("board_am62p.c", include_str!("../driver/zephyr/lib/board/am62p/board_am62p.c")),
        ("common/board.c", include_str!("../driver/zephyr/lib/board/common/board.c")),
        ("common/sample_glue.c", include_str!("../driver/zephyr/lib/board/common/sample_glue.c")),
    ];
    for (name, src) in boards {
        assert!(
            !src.contains("yakogl_zephyr_wait_vblank") && !src.contains("am62p_dss_wait_vsync"),
            "{name} wires a vertical-sync source for EGL: see docs/notes/am62p-flip-latch.md, \
             'Decision: wait_vblank (2026-11)', before changing this test"
        );
    }
    let zephyr = include_str!("../src/osal/zephyr.c");
    let weak = zephyr.find("__weak int yakogl_zephyr_wait_vblank(").expect("zephyr.c's weak wait_vblank");
    let body = &zephyr[weak..weak + zephyr[weak..].find('}').expect("its body")];
    assert!(body.contains("return -ENOTSUP;"), "{body}");
    assert!(zephyr.contains(".wait_vblank = z_wait_vblank,"));
}
