// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! agl-slint-cluster on Zephyr: the Rust staticlib of the cluster sample
//! (../src/main.c, on the board layer, every board).
//!
//! Draws the cluster's UI (signal-slot/agl-slint-cluster `ui/app-window.slint`,
//! re-exported by ../ui/zephyr-cluster.slint) with Slint's FemtoVG OpenGL ES 2.0
//! renderer on YakoGL, over the GL context the C side made current. The
//! platform, window adapter and runtime glue are gl_slint's
//! (../../gl_slint/rust/src/lib.rs); the application layer is the cluster's
//! own, ported in cluster.rs. Fonts and images are embedded at compile time
//! (build.rs).
//!
//! C entry points, all from one thread:
//! - `slint_cluster_draw_frame(get_proc, w, h)`: one frame; the first call
//!   creates the platform and the component;
//! - `slint_cluster_advance(n)`: run n telemetry periods (50 ms each) of the
//!   drive cycle and push the last frame into the UI;
//! - the cluster sample's check, present accounting and records (board.rs).
#![no_std]

extern crate alloc;

use alloc::boxed::Box;
use alloc::rc::{Rc, Weak};
use core::alloc::{GlobalAlloc, Layout};
use core::cell::UnsafeCell;
use core::ffi::{c_char, c_void, CStr};
use core::num::NonZeroU32;

use slint::platform::{Platform, Renderer, WindowAdapter};
use slint::{ComponentHandle, PhysicalSize, PlatformError, Window};

use i_slint_renderer_femtovg::opengl::OpenGLInterface;
use i_slint_renderer_femtovg::FemtoVGOpenGLRenderer;

// The Slint compiler output (build.rs): the cluster's `AppWindow`, unchanged,
// and `FontWeights`, which only makes the compiler embed the Light and
// ExtraLight faces (../ui/zephyr-cluster.slint).
slint::include_modules!();

mod board;
mod cluster;

// ---------------------------------------------------------------------------
// Zephyr runtime glue (gl_slint's)
// ---------------------------------------------------------------------------

extern "C" {
    /// The UI heap: a sys_heap over a static arena owned by main.c.
    fn slint_heap_alloc(alignment: usize, size: usize) -> *mut c_void;
    fn slint_heap_free(ptr: *mut c_void);
    fn printk(fmt: *const c_char, ...);
    /// Slint's animation clock in milliseconds, set by main.c.
    fn slint_zephyr_uptime_ms() -> u64;
}

struct ZephyrAlloc;

unsafe impl GlobalAlloc for ZephyrAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let align = layout.align().max(core::mem::size_of::<usize>());
        slint_heap_alloc(align, layout.size().max(1)) as *mut u8
    }

    unsafe fn dealloc(&self, ptr: *mut u8, _layout: Layout) {
        slint_heap_free(ptr as *mut c_void);
    }
}

#[global_allocator]
static GLOBAL: ZephyrAlloc = ZephyrAlloc;

/// A core::fmt sink that prints through printk, flushed per line; a line
/// longer than N - 1 bytes is split.
struct Pk<const N: usize = 192> {
    buf: [u8; N],
    n: usize,
}

impl<const N: usize> Pk<N> {
    const fn new() -> Self {
        Pk { buf: [0; N], n: 0 }
    }

    fn flush(&mut self) {
        if self.n > 0 {
            self.buf[self.n] = 0;
            unsafe {
                printk(b"%s\n\0".as_ptr() as *const c_char, self.buf.as_ptr() as *const c_char);
            }
            self.n = 0;
        }
    }
}

impl<const N: usize> core::fmt::Write for Pk<N> {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        for &b in s.as_bytes() {
            if b == b'\n' || self.n >= self.buf.len() - 1 {
                self.flush();
                if b == b'\n' {
                    continue;
                }
            }
            self.buf[self.n] = b;
            self.n += 1;
        }
        Ok(())
    }
}

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    // panic = "abort": log the location and message and spin; the board's
    // watchdog resets the SoC.
    use core::fmt::Write;
    let mut w: Pk = Pk::new();
    let _ = w.write_str("slint_cluster: rust panic: ");
    if let Some(loc) = info.location() {
        let _ = write!(w, "{}:{}:{}: ", loc.file(), loc.line(), loc.column());
    }
    let _ = write!(w, "{}", info.message());
    w.flush();
    loop {
        core::hint::spin_loop();
    }
}

// ---------------------------------------------------------------------------
// Single-threaded process state (the `unsafe-single-threaded` contract: the C
// side calls every entry point from one thread).
// ---------------------------------------------------------------------------

type GetProc = extern "C" fn(*const c_char) -> *const c_void;

struct Single<T>(UnsafeCell<T>);
// Safety: only ever accessed from the single UI thread.
unsafe impl<T> Sync for Single<T> {}
impl<T> Single<T> {
    const fn new(v: T) -> Self {
        Single(UnsafeCell::new(v))
    }
    #[allow(clippy::mut_from_ref)]
    unsafe fn get(&self) -> &mut T {
        &mut *self.0.get()
    }
}

struct State {
    get_proc: Option<GetProc>,
    adapter: Option<Rc<ClusterWindowAdapter>>,
    ui: Option<AppWindow>,
    platform_set: bool,
    drive: cluster::Drive,
}

static STATE: Single<State> =
    Single::new(State { get_proc: None, adapter: None, ui: None, platform_set: false, drive: cluster::Drive::new() });

type BoxErr = Box<dyn core::error::Error + Send + Sync>;

/// The GL context bridge: every lookup goes to the C hook, which made the
/// context current before `slint_cluster_draw_frame`.
struct GlContext;

#[allow(unsafe_code)]
unsafe impl OpenGLInterface for GlContext {
    fn ensure_current(&self) -> Result<(), BoxErr> {
        Ok(())
    }
    fn swap_buffers(&self) -> Result<(), BoxErr> {
        Ok(())
    }
    fn resize(&self, _width: NonZeroU32, _height: NonZeroU32) -> Result<(), BoxErr> {
        Ok(())
    }
    fn get_proc_address(&self, name: &CStr) -> *const c_void {
        match unsafe { STATE.get() }.get_proc {
            Some(f) => f(name.as_ptr()),
            None => core::ptr::null(),
        }
    }
}

struct ClusterWindowAdapter {
    window: Window,
    renderer: FemtoVGOpenGLRenderer,
    size: PhysicalSize,
}

impl WindowAdapter for ClusterWindowAdapter {
    fn window(&self) -> &Window {
        &self.window
    }
    fn size(&self) -> PhysicalSize {
        self.size
    }
    fn renderer(&self) -> &dyn Renderer {
        &self.renderer
    }
}

struct ClusterPlatform {
    size: PhysicalSize,
}

impl Platform for ClusterPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        let renderer = FemtoVGOpenGLRenderer::new(GlContext)?;
        let size = self.size;
        let adapter = Rc::new_cyclic(|weak: &Weak<ClusterWindowAdapter>| ClusterWindowAdapter {
            window: Window::new(weak.clone()),
            renderer,
            size,
        });
        unsafe { STATE.get() }.adapter = Some(adapter.clone());
        Ok(adapter)
    }

    fn duration_since_start(&self) -> core::time::Duration {
        core::time::Duration::from_millis(unsafe { slint_zephyr_uptime_ms() })
    }
}

/// Run `n` telemetry periods of the drive cycle and push the last frame into
/// the UI. 1 before the first `slint_cluster_draw_frame` (no component yet).
#[no_mangle]
pub extern "C" fn slint_cluster_advance(n: u32) -> i32 {
    let state = unsafe { STATE.get() };
    let Some(ui) = state.ui.as_ref() else {
        return 1;
    };
    state.drive.advance(ui, n);
    0
}

/// The speed the last pushed frame carries, in tenths of km/h (-1: none), so
/// the C side can print the state the frame was drawn in.
#[no_mangle]
pub extern "C" fn slint_cluster_speed_x10() -> i32 {
    match unsafe { STATE.get() }.drive.last.as_ref() {
        Some(t) => (t.speed_kmh * 10.0) as i32,
        None => -1,
    }
}

/// Draw one frame through the caller's current GL ES 2.0 context at
/// `width` x `height`. The first call creates the platform and the component.
/// 0 on success; 1 set_platform, 2 component build, 3 show, 4 no adapter,
/// 5 render (gl_slint's codes).
#[no_mangle]
pub extern "C" fn slint_cluster_draw_frame(get_proc: GetProc, width: u32, height: u32) -> i32 {
    let state = unsafe { STATE.get() };
    state.get_proc = Some(get_proc);
    let size = PhysicalSize::new(width, height);

    if !state.platform_set {
        if slint::platform::set_platform(Box::new(ClusterPlatform { size })).is_err() {
            return 1;
        }
        state.platform_set = true;
    }
    if state.ui.is_none() {
        match AppWindow::new() {
            Ok(ui) => state.ui = Some(ui),
            Err(_) => return 2,
        }
    }
    let ui = state.ui.as_ref().unwrap();
    ui.window().set_size(size);
    if ui.show().is_err() {
        return 3;
    }
    slint::platform::update_timers_and_animations();
    let Some(adapter) = state.adapter.as_ref() else {
        return 4;
    };
    if adapter.renderer.render().is_err() {
        return 5;
    }
    0
}
