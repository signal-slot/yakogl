// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The timeline of `eglSwapBuffers` on a window surface, recorded by a
//! counting fake platform: per swap, the GPU submit, the wait for a frame's
//! end, the wait for the frames before it, the show (`present`) and the
//! vertical-period waits; between swaps, the application's CPU work and
//! whether the GPU is still drawing the frame before it.
//! docs/notes/egl-swap-timeline.md explains the timelines and what they
//! mean for the glmark2 scores of the AM62P and the Pi 4.
//!
//! What it pins:
//!
//! * A window EGL double-buffers (EGL_BACK_BUFFER, the default where the
//!   platform has `present`): the swap of frame N submits N and waits only
//!   for N-1, so the CPU records frame N+1 while the GPU draws N; it never
//!   waits for N's own end while a buffer is free; it shows N-1's buffer
//!   once N-1 is complete; frame N+1 goes into neither N's buffer (being
//!   drawn) nor N-1's (just shown); and a swap waits at most one vertical
//!   period, also on a platform whose show itself waits for one.
//! * A swap with nothing drawn since the last one still shows a complete
//!   buffer (its previous frame is the last kick, which "all but the last
//!   kick" would leave running).
//! * The window rendered in place (EGL_SINGLE_BUFFER: #925, 0247ac66) waits
//!   for its frame's end in every swap: no overlap. This is the timeline the
//!   AM62P's 289 and the Pi 4's 157 were measured with.
//! * GLES_NATIVE_WINDOW_PIPELINED: the overlap without EGL's buffers (the
//!   application double-buffers the window itself), the timeline of the
//!   AM62P's 462 and the Pi 4's 188.
//!
//! The boards are modelled by what their show and vertical sync do (the
//! code cited at each [`Board`]); the GPU is a fake backend whose kicks
//! complete only when the library waits for them, so "still running" is
//! exact. Run it with any GPU feature (the backend is the fake's):
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr --test egl_swap_timeline
//! ```
#![cfg(all(feature = "host-test", any(feature = "gpu-powervr", feature = "gpu-vivante", feature = "gpu-v3d")))]

use core::ffi::c_void;
use std::collections::BTreeMap;
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

/* ---- the timeline ------------------------------------------------------------ */

/// One thing that happened, in order.
#[derive(Clone, Debug, PartialEq)]
enum Ev {
    /// The application's CPU work for frame `frame` (its GL calls), with
    /// the kicks the GPU was still drawing at that moment.
    Cpu {
        frame: u32,
        running: Vec<u32>,
    },
    /// `eglSwapBuffers` of frame `frame` begins / returns.
    Swap(u32),
    SwapDone(u32),
    /// A draw recorded into kick `kick`, rendering into `target`.
    Draw {
        kick: u32,
        target: u64,
    },
    /// Kick `kick` handed to the GPU.
    Submit(u32),
    /// `finish`: the wait for every kick submitted, through `through`
    /// (the end of the frame just submitted).
    WaitEnd {
        through: u32,
    },
    /// `finish_previous`: the wait for every kick but the last, through
    /// `through`.
    WaitPrevious {
        through: u32,
    },
    /// The display took buffer `gpu`; `running` when a kick that renders
    /// into it had not completed (a torn or partial picture); `waited` when
    /// this show waited for a vertical period.
    Present {
        gpu: u64,
        running: bool,
        waited: bool,
    },
    /// A vertical period waited for (`wait_vblank`).
    Vblank,
}

/// The fake GPU and display.
#[derive(Default)]
struct Fake {
    log: Vec<Ev>,
    /// Kicks handed to the GPU, and completed: kicks complete only when the
    /// library waits for them.
    submitted: u32,
    completed: u32,
    /// The target of the kick being recorded, if a draw opened one.
    open: Option<u64>,
    /// The target of each kick submitted.
    targets: BTreeMap<u32, u64>,
    /// The board's show waits for a vertical period (and says so).
    present_waits: bool,
}

static FAKE: Mutex<Option<Fake>> = Mutex::new(None);

fn with_fake<R>(f: impl FnOnce(&mut Fake) -> R) -> R {
    f(FAKE.lock().unwrap_or_else(|e| e.into_inner()).as_mut().expect("the fake is installed"))
}

impl Fake {
    fn running(&self) -> Vec<u32> {
        (self.completed + 1..=self.submitted).collect()
    }

    fn submit(&mut self) {
        if let Some(t) = self.open.take() {
            self.submitted += 1;
            self.targets.insert(self.submitted, t);
            self.log.push(Ev::Submit(self.submitted));
        }
    }

    fn complete(&mut self, through: u32) {
        self.completed = self.completed.max(through.min(self.submitted));
    }
}

/* ---- the platform: host memory, a counting display --------------------------- */

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

unsafe extern "C" fn m_gpu_write(_m: *mut OsalGpuMem, _off: usize, _src: *const c_void, _n: usize) -> i32 {
    0
}

unsafe extern "C" fn m_gpu_read(_m: *mut OsalGpuMem, _off: usize, _dst: *mut c_void, _n: usize) -> i32 {
    0
}

unsafe extern "C" fn m_cache(_cpu: *mut c_void, _n: usize) {}

/// The show (`gles_osal.h`, present): recorded with whether a kick still
/// renders into the buffer. A board whose show waits for a vertical period
/// returns 1 (the contract: the swap counts it against its interval).
unsafe extern "C" fn m_present(mem: *const OsalGpuMem, _w: u32, _h: u32, _stride: u32, _format: u32) -> i32 {
    // SAFETY: the library passes one of its buffers.
    let gpu = unsafe { (*mem).gpu };
    with_fake(|f| {
        let running = f.targets.iter().any(|(&k, &t)| t == gpu && k > f.completed) || f.open == Some(gpu);
        let waited = f.present_waits;
        f.log.push(Ev::Present { gpu, running, waited });
        i32::from(waited)
    })
}

unsafe extern "C" fn m_wait_vblank(_timeout_us: i64) -> i32 {
    with_fake(|f| f.log.push(Ev::Vblank));
    0
}

/// The OSAL of a board with no vertical-sync source for EGL (the AM62P's
/// zephyr.c weak `yakogl_zephyr_wait_vblank`, the Pi 4's zephyr_v3d.c and
/// the i.MX8MP's zephyr_gc.c, which have no `.wait_vblank`).
static OSAL_NO_VBLANK: OsalOps = OsalOps {
    init: Some(m_init),
    shutdown: None,
    gpu_alloc: Some(m_gpu_alloc),
    gpu_free: Some(m_gpu_free),
    mmu_map: None,
    map_phys: Some(m_map_phys),
    gpu_write: Some(m_gpu_write),
    gpu_read: Some(m_gpu_read),
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

/// The same with a vertical-sync source (the i.MX8MP once #737 wires its
/// `yakogl_zephyr_wait_vblank` into the GC OSAL).
static OSAL_VBLANK: OsalOps = OsalOps { wait_vblank: Some(m_wait_vblank), ..OSAL_NO_VBLANK };

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
        let kick = f.submitted + 1;
        f.log.push(Ev::Draw { kick, target });
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
        let through = f.submitted;
        f.complete(through);
        f.log.push(Ev::WaitEnd { through });
    });
    0
}

/// Every kick but the last submitted (src/pvr/backend.rs
/// `pvr_backend_finish_previous`; the V3D's flush waits for the job in
/// flight before it submits the next, which leaves the same one running).
unsafe extern "C" fn b_finish_previous() -> i32 {
    with_fake(|f| {
        let through = f.submitted.saturating_sub(1);
        f.complete(through);
        f.log.push(Ev::WaitPrevious { through });
    });
    0
}

unsafe extern "C" fn b_kick_id() -> u32 {
    with_fake(|f| f.submitted + 1)
}

unsafe extern "C" fn b_kick_pending(id: u32) -> bool {
    with_fake(|f| id > f.completed && (id <= f.submitted || f.open.is_some()))
}

fn backend_table() -> YakoglBackend {
    YakoglBackend {
        name: c"egl_swap_timeline".as_ptr(),
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

/* ---- the boards --------------------------------------------------------------- */

/// What a board's show and vertical sync do, from its code.
#[derive(Clone, Copy)]
struct Board {
    name: &'static str,
    /// The show waits for a vertical period before it returns.
    present_waits: bool,
    /// EGL has a vertical-sync source (`wait_vblank`).
    vblank: bool,
}

/// board_am62p.c board_show -> sample_glue.c pvr_sample_flip_window ->
/// soc/am62p/dss.rs set_app_buffer: the plane's registers and GO, latched at
/// the next vsync, no wait. No `yakogl_zephyr_wait_vblank`.
const AM62P: Board = Board { name: "AM62P", present_waits: false, vblank: false };
/// board_rpi4.c board_show: a copy into the firmware framebuffer (the GM
/// image has no SCANOUT_VIEW), no wait; zephyr_v3d.c has no `.wait_vblank`.
const PI4: Board = Board { name: "Pi 4", present_waits: false, vblank: false };
/// board_imx8mp.c board_show with YAKOGL_BOARD_IMX8MP_SCREEN_BUFFERS 1 (the
/// default): the show waits for the LCDIF latch; zephyr_gc.c has no
/// `.wait_vblank` yet.
const IMX8MP: Board = Board { name: "i.MX8MP", present_waits: true, vblank: false };
/// The i.MX8MP once `.wait_vblank` is wired (#737) and the show still waits.
const IMX8MP_VBLANK: Board = Board { name: "i.MX8MP with .wait_vblank", present_waits: true, vblank: true };

/* ---- the run -------------------------------------------------------------------- */

const W: u32 = 8;
const H: u32 = 4;
const FRAMES: u32 = 6;

/// How the run asks for its window.
#[derive(Clone, Copy, PartialEq)]
enum Window {
    /// No attribute list: EGL_BACK_BUFFER, what glmark2 asks for.
    Back,
    /// EGL_RENDER_BUFFER EGL_SINGLE_BUFFER: rendered in place, #925.
    Single,
    /// GLES_NATIVE_WINDOW_PIPELINED in the native window's format.
    Pipelined,
}

/// One run: the timeline and the window's own buffer address.
struct Run {
    log: Vec<Ev>,
    window_gpu: u64,
}

/// `FRAMES` frames on `board`: per frame the CPU work (a clear), then the
/// swap; `skip_draw` names a frame whose application draws nothing.
fn run(board: Board, window: Window, interval: Option<EGLint>, skip_draw: Option<u32>) -> Run {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    *FAKE.lock().unwrap_or_else(|e| e.into_inner()) =
        Some(Fake { present_waits: board.present_waits, ..Fake::default() });
    let table: &'static YakoglBackend = Box::leak(Box::new(backend_table()));
    let mut pixels = vec![0u8; (W * H * 4) as usize + 4096];
    let phys = (pixels.as_mut_ptr() as u64 + 4095) & !4095;
    let format =
        GLES_NATIVE_FORMAT_XRGB8888 | if window == Window::Pipelined { GLES_NATIVE_WINDOW_PIPELINED } else { 0 };
    let mut win = GlesNativeWindow { phys, width: W, height: H, stride_bytes: W * 4, format };
    let single = [EGL_RENDER_BUFFER, EGL_SINGLE_BUFFER, EGL_NONE];
    /* an XRGB8888 window needs an 8-bit config (a 565 one is EGL_BAD_MATCH) */
    #[rustfmt::skip]
    let cfg_list = [
        EGL_RED_SIZE, 8, EGL_GREEN_SIZE, 8, EGL_BLUE_SIZE, 8,
        EGL_SURFACE_TYPE, EGL_WINDOW_BIT, EGL_RENDERABLE_TYPE, EGL_OPENGL_ES2_BIT, EGL_NONE,
    ];
    let ctx_list = [EGL_CONTEXT_CLIENT_VERSION, 2, EGL_NONE];
    // SAFETY: tests are serialised; the tables are static or leaked; every
    // pointer passed is live for its call.
    unsafe {
        yakogl::osal::osal_set_ops(if board.vblank { &OSAL_VBLANK } else { &OSAL_NO_VBLANK });
        backend::g_backend = table;
        let dpy = eglGetDisplay(core::ptr::null_mut());
        assert_eq!(eglInitialize(dpy, core::ptr::null_mut(), core::ptr::null_mut()), 1);
        let mut cfg = core::ptr::null_mut();
        let mut n = 0;
        assert_eq!(eglChooseConfig(dpy, cfg_list.as_ptr(), &mut cfg, 1, &mut n), 1);
        assert_eq!(n, 1, "a window config");
        let attribs = if window == Window::Single { single.as_ptr() } else { core::ptr::null() };
        let s = eglCreateWindowSurface(dpy, cfg, &mut win, attribs);
        assert!(!s.is_null(), "eglCreateWindowSurface: {:#x}", eglGetError());
        let c = eglCreateContext(dpy, cfg, core::ptr::null_mut(), ctx_list.as_ptr());
        assert!(!c.is_null());
        assert_eq!(eglMakeCurrent(dpy, s, s, c), 1);
        let mut rb = 0;
        assert_eq!(eglQuerySurface(dpy, s, EGL_RENDER_BUFFER, &mut rb), 1);
        let want = if window == Window::Back { EGL_BACK_BUFFER } else { EGL_SINGLE_BUFFER };
        assert_eq!(rb, want, "{}: EGL_RENDER_BUFFER", board.name);
        if let Some(i) = interval {
            assert_eq!(eglSwapInterval(dpy, i), 1);
        }
        with_fake(|f| f.log.clear());
        for frame in 0..FRAMES {
            with_fake(|f| {
                let running = f.running();
                f.log.push(Ev::Cpu { frame, running });
            });
            if skip_draw != Some(frame) {
                yakogl::gl::state::glClearColor(0.0, 0.0, frame as f32 / FRAMES as f32, 1.0);
                yakogl::gl::state::glClear(GL_COLOR_BUFFER_BIT);
            }
            with_fake(|f| f.log.push(Ev::Swap(frame)));
            assert_eq!(eglSwapBuffers(dpy, s), 1, "{}: eglSwapBuffers of frame {frame}", board.name);
            with_fake(|f| f.log.push(Ev::SwapDone(frame)));
        }
        let log = with_fake(|f| core::mem::take(&mut f.log));
        eglMakeCurrent(dpy, core::ptr::null_mut(), core::ptr::null_mut(), core::ptr::null_mut());
        eglDestroyContext(dpy, c);
        eglDestroySurface(dpy, s);
        eglTerminate(dpy);
        drop(pixels);
        Run { log, window_gpu: phys }
    }
}

/// The events of swap `frame`, from its begin to its return.
fn swap_events(log: &[Ev], frame: u32) -> &[Ev] {
    let a = log.iter().position(|e| *e == Ev::Swap(frame)).expect("the swap began");
    let b = log.iter().position(|e| *e == Ev::SwapDone(frame)).expect("the swap returned");
    &log[a + 1..b]
}

/// The kick frame `frame`'s draws went into, and its target.
fn frame_kick(log: &[Ev], frame: u32) -> Option<(u32, u64)> {
    swap_events(log, frame).iter().find_map(|e| match e {
        Ev::Draw { kick, target } => Some((*kick, *target)),
        _ => None,
    })
}

/// The vertical periods swap `frame` waited: its `wait_vblank` calls and a
/// show that waited for one.
fn periods(log: &[Ev], frame: u32) -> usize {
    swap_events(log, frame).iter().filter(|e| matches!(e, Ev::Vblank | Ev::Present { waited: true, .. })).count()
}

fn dump(log: &[Ev]) -> String {
    log.iter().map(|e| format!("{e:?}")).collect::<Vec<_>>().join("\n  ")
}

/* ---- the EGL double buffer ------------------------------------------------------ */

/// The double-buffered window's timeline on `board`, checked frame by frame.
fn double_buffer_timeline(board: Board, interval: Option<EGLint>) {
    let r = run(board, Window::Back, interval, None);
    let log = &r.log;
    let who = format!("{} (interval {interval:?})", board.name);
    let mut shown_before: Option<u64> = None;
    for frame in 0..FRAMES {
        let (kick, target) =
            frame_kick(log, frame).unwrap_or_else(|| panic!("{who}: frame {frame} drew\n  {}", dump(log)));
        let ev = swap_events(log, frame);
        /* no wait for the frame's own end while a buffer is free */
        assert!(
            !ev.iter().any(|e| matches!(e, Ev::WaitEnd { .. })),
            "{who}: swap {frame} waited for its frame's end\n  {}",
            dump(ev)
        );
        assert!(ev.contains(&Ev::Submit(kick)), "{who}: swap {frame} submits its frame");
        /* the swap returns with the frame running: the next frame's CPU
         * work overlaps it */
        if frame + 1 < FRAMES {
            let next_cpu = log
                .iter()
                .find_map(|e| match e {
                    Ev::Cpu { frame: f, running } if *f == frame + 1 => Some(running.clone()),
                    _ => None,
                })
                .expect("the next frame's CPU work");
            assert_eq!(next_cpu, [kick], "{who}: frame {}'s CPU work overlaps frame {frame}'s GPU", frame + 1);
        }
        /* at most one vertical period per swap */
        assert!(periods(log, frame) <= 1, "{who}: swap {frame} waited {} periods\n  {}", periods(log, frame), dump(ev));
        /* the show: the frame before, complete */
        let shown: Vec<(u64, bool)> = ev
            .iter()
            .filter_map(|e| match e {
                Ev::Present { gpu, running, .. } => Some((*gpu, *running)),
                _ => None,
            })
            .collect();
        if frame == 0 {
            assert_eq!(shown, [], "{who}: nothing complete to show at the first swap");
            assert_eq!(target, r.window_gpu, "{who}: frame 0 is drawn into the window's own buffer");
        } else {
            let (_, prev_target) = frame_kick(log, frame - 1).expect("the frame before drew");
            assert_eq!(
                shown,
                [(prev_target, false)],
                "{who}: swap {frame} shows frame {}'s complete buffer",
                frame - 1
            );
            /* the frame drawn now is in neither the buffer the GPU draws
             * (frame - 1 was shown at this swap; frame was drawn in) nor
             * the one shown at the swap before it */
            assert_ne!(target, prev_target, "{who}: frame {frame} draws into the buffer being shown");
        }
        if let Some(s) = shown_before {
            assert_ne!(target, s, "{who}: frame {frame} draws into the buffer shown one swap earlier");
        }
        shown_before = shown.first().map(|x| x.0);
    }
}

#[test]
fn am62p_double_buffer_overlaps_and_never_waits_for_the_frames_end() {
    double_buffer_timeline(AM62P, None);
}

#[test]
fn pi4_double_buffer_overlaps_and_never_waits_for_the_frames_end() {
    double_buffer_timeline(PI4, None);
}

#[test]
fn glmark2_interval_0_overlaps_on_every_board() {
    for board in [AM62P, PI4, IMX8MP, IMX8MP_VBLANK] {
        double_buffer_timeline(board, Some(0));
    }
}

#[test]
fn a_show_that_waits_for_a_period_is_the_one_period_of_interval_1() {
    /* the i.MX8MP's show waits for the LCDIF latch; once EGL also has its
     * vertical sync, swap interval 1 must not add a second period */
    double_buffer_timeline(IMX8MP, Some(1));
    double_buffer_timeline(IMX8MP_VBLANK, Some(1));
}

#[test]
fn interval_2_waits_two_periods_counting_the_show() {
    let r = run(IMX8MP_VBLANK, Window::Back, Some(2), None);
    for frame in 1..FRAMES {
        assert_eq!(periods(&r.log, frame), 2, "swap {frame}\n  {}", dump(swap_events(&r.log, frame)));
    }
    /* the first swap shows nothing and waits its two periods itself */
    assert_eq!(periods(&r.log, 0), 2);
}

#[test]
fn a_swap_with_nothing_drawn_still_shows_a_complete_buffer() {
    /* frame 3 draws nothing: its swap submits no kick, so the last kick is
     * frame 2's and "all but the last" would show frame 2 still running */
    for board in [AM62P, PI4] {
        let r = run(board, Window::Back, None, Some(3));
        let ev = swap_events(&r.log, 3);
        let (_, prev_target) = frame_kick(&r.log, 2).expect("frame 2 drew");
        let shown: Vec<_> = ev.iter().filter(|e| matches!(e, Ev::Present { .. })).cloned().collect();
        assert_eq!(
            shown,
            [Ev::Present { gpu: prev_target, running: false, waited: false }],
            "{}: swap 3 shows frame 2 complete\n  {}",
            board.name,
            dump(ev)
        );
        /* and the frames after it keep the overlap */
        double_buffer_check_tail(&r, board, 4);
    }
}

/// From `from` on, every swap leaves its frame running.
fn double_buffer_check_tail(r: &Run, board: Board, from: u32) {
    for frame in from..FRAMES {
        let ev = swap_events(&r.log, frame);
        assert!(!ev.iter().any(|e| matches!(e, Ev::WaitEnd { .. })), "{}: swap {frame}\n  {}", board.name, dump(ev));
    }
}

/* ---- the other two windows --------------------------------------------------------- */

#[test]
fn a_single_buffered_window_waits_for_its_frames_end_in_every_swap() {
    /* #925: the window is shown in place right after the swap, so the swap
     * completes the frame. The CPU work of frame N+1 then starts on an idle
     * GPU: the timeline of the AM62P's 289 and the Pi 4's 157 */
    for board in [AM62P, PI4] {
        let r = run(board, Window::Single, None, None);
        for frame in 0..FRAMES {
            let (kick, target) = frame_kick(&r.log, frame).expect("the frame drew");
            assert_eq!(target, r.window_gpu, "in place");
            let ev = swap_events(&r.log, frame);
            assert!(ev.contains(&Ev::WaitEnd { through: kick }), "{}: swap {frame}\n  {}", board.name, dump(ev));
            assert!(!ev.iter().any(|e| matches!(e, Ev::Present { .. })), "the application shows it");
        }
        for e in &r.log {
            if let Ev::Cpu { running, frame } = e {
                assert_eq!(
                    *running,
                    [] as [u32; 0],
                    "{}: frame {frame}'s CPU work has no GPU work beside it",
                    board.name
                );
            }
        }
    }
}

#[test]
fn a_pipelined_window_overlaps_without_egls_buffers() {
    /* GLES_NATIVE_WINDOW_PIPELINED: the swap waits only for the frame before,
     * as before #925 (the AM62P's 462, the Pi 4's 188); EGL shows nothing
     * and every frame goes into the application's window */
    for board in [AM62P, PI4] {
        let r = run(board, Window::Pipelined, None, None);
        for frame in 0..FRAMES {
            let (kick, target) = frame_kick(&r.log, frame).expect("the frame drew");
            assert_eq!(target, r.window_gpu);
            let ev = swap_events(&r.log, frame);
            assert!(!ev.iter().any(|e| matches!(e, Ev::WaitEnd { .. } | Ev::Present { .. })), "{}", dump(ev));
            assert!(
                ev.contains(&Ev::WaitPrevious { through: kick - 1 }),
                "{}: swap {frame}\n  {}",
                board.name,
                dump(ev)
            );
        }
    }
}
