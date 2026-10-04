// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Slint meter cluster on the PowerVR GPU: the Zephyr-side Rust staticlib.
//!
//! Draws the cluster UI (`../ui/cluster.slint`, the Slint port of the SafeUI C
//! cluster) through Slint's FemtoVG (OpenGL ES 2.0) renderer over an app-supplied
//! GL context (YakoGL + EGL on the driver). The C side creates and makes current
//! a GL context and passes a get-proc-address function pointer; this crate installs
//! a custom `slint::platform::Platform` whose `WindowAdapter` owns a
//! `FemtoVGOpenGLRenderer` built over that context, instantiates the compiled
//! component, feeds it the vehicle state (`slint_set_vehicle`) and renders one frame
//! per `slint_draw_frame`. Fonts and images are embedded at compile time (build.rs).
//!
//! The crate also carries the Zephyr runtime glue: a global allocator on the libc
//! heap and a panic handler that prints through printk.
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

// The Slint compiler output (build.rs / ui/cluster.slint). Exports `MainWindow`.
slint::include_modules!();

// ---------------------------------------------------------------------------
// Zephyr runtime glue
// ---------------------------------------------------------------------------

extern "C" {
    /// The UI heap: a sys_heap over a static arena owned by main.c, so it can be
    /// validated (sys_heap_validate) between frames when hunting corruption.
    fn slint_heap_alloc(alignment: usize, size: usize) -> *mut c_void;
    fn slint_heap_free(ptr: *mut c_void);
    fn printk(fmt: *const c_char, ...);
    /// Monotonic milliseconds since boot, supplied by main.c (k_uptime_get); drives
    /// Slint's animations.
    fn slint_zephyr_uptime_ms() -> u64;
}

/// Global allocator backed by Zephyr's libc heap.
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

/// A core::fmt sink that prints through Zephyr printk. Buffers into a fixed
/// stack array and flushes a null-terminated "%s" on newline or when full, so
/// the panic message + location reach the console without std.
struct Pk {
    buf: [u8; 192],
    n: usize,
}

impl Pk {
    fn flush(&mut self) {
        if self.n > 0 {
            self.buf[self.n] = 0;
            unsafe {
                printk(b"%s\0".as_ptr() as *const c_char, self.buf.as_ptr() as *const c_char);
            }
            self.n = 0;
        }
    }
}

impl core::fmt::Write for Pk {
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
    // No unwinding (panic = "abort"): log the location + message and spin. The
    // board's watchdog resets the SoC if we ever get here.
    use core::fmt::Write;
    let mut w = Pk { buf: [0; 192], n: 0 };
    let _ = w.write_str("slint_zephyr: rust panic: ");
    if let Some(loc) = info.location() {
        let _ = write!(w, "{}:{}:{}: ", loc.file(), loc.line(), loc.column());
    }
    let _ = write!(w, "{}", info.message());
    let _ = w.write_str("\n");
    w.flush();
    loop {
        core::hint::spin_loop();
    }
}

// ---------------------------------------------------------------------------
// Single-threaded process state.
//
// Slint is used here under the `unsafe-single-threaded` contract: the C side
// calls every entry point from exactly one thread. `Single` is a Sync wrapper
// around that state (the live component, the window adapter and the get-proc
// hook) so we can hold non-Send types (`Rc`, `MainWindow`) in a `static`
// without `std::thread_local!`.
// ---------------------------------------------------------------------------

type GetProc = extern "C" fn(*const c_char) -> *const c_void;

struct Single<T>(UnsafeCell<T>);
// Safety: only ever accessed from the single UI thread (see above).
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

/// A C function that draws under the UI: see `slint_set_underlay`.
type Underlay = extern "C" fn();

struct State {
    get_proc: Option<GetProc>,
    adapter: Option<Rc<AppWindowAdapter>>,
    ui: Option<MainWindow>,
    platform_set: bool,
    underlay: Option<Underlay>,
    underlay_covers: bool,
}

static STATE: Single<State> = Single::new(State {
    get_proc: None,
    adapter: None,
    ui: None,
    platform_set: false,
    underlay: None,
    underlay_covers: false,
});

type BoxErr = Box<dyn core::error::Error + Send + Sync>;

/// OpenGL ES 2.0 context bridge: forward every entry-point lookup to the C hook.
/// The C side has made the context current before calling `slint_draw_frame`,
/// so `ensure_current` is a no-op; the caller presents the frame after we return.
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
        // Safety: single-threaded; get_proc is set before the platform is used.
        match unsafe { STATE.get() }.get_proc {
            Some(f) => f(name.as_ptr()),
            None => core::ptr::null(),
        }
    }
}

/// Window adapter owning the FemtoVG renderer. `renderer()` hands Slint the
/// renderer for this window; `size()` reports the fixed drawable size.
struct AppWindowAdapter {
    window: Window,
    renderer: FemtoVGOpenGLRenderer,
    size: PhysicalSize,
}

impl WindowAdapter for AppWindowAdapter {
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

/// Custom platform: builds our FemtoVG window adapter. No event loop; the
/// application drives one render per `slint_draw_frame`.
struct AppPlatform {
    size: PhysicalSize,
}

impl Platform for AppPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        let renderer = FemtoVGOpenGLRenderer::new(GlContext)?;
        let size = self.size;
        let adapter = Rc::new_cyclic(|weak: &Weak<AppWindowAdapter>| AppWindowAdapter {
            window: Window::new(weak.clone()),
            renderer,
            size,
        });
        // Keep a strong handle so we can drive render() outside an event loop.
        // Safety: single-threaded (see above).
        unsafe { STATE.get() }.adapter = Some(adapter.clone());
        Ok(adapter)
    }

    /// The animation clock: Zephyr uptime (the gauge fractions animate).
    fn duration_since_start(&self) -> core::time::Duration {
        core::time::Duration::from_millis(unsafe { slint_zephyr_uptime_ms() })
    }
}

/// The vehicle state the C side feeds to the UI each frame. Mirrors the `in`
/// properties of `MainWindow` in ui/cluster.slint; all fields are i32 so the
/// struct has one obvious C layout (bools are 0/1).
#[repr(C)]
pub struct SlintVehicle {
    pub speed_x10: i32,
    pub rpm: i32,
    pub fuel_ml: i32,
    pub tank_ml: i32,
    pub trip_m: i32,
    pub trip_valid: i32,
    pub odo_m: i32,
    pub gear: i32,
    pub engine_on: i32,
    pub engine_warn: i32,
    pub turn_left: i32,
    pub turn_right: i32,
    pub lights: i32,
    pub fuel_low: i32,
    pub fuel_empty: i32,
    pub uptime_secs: i32,
    pub frame: i32,
    /// Also draw the R5F-owned elements (speed digits, gear, telltales, fuel).
    pub standalone: i32,
}

/// Pushes the vehicle state into the UI. A no-op until the first
/// `slint_draw_frame` has created the component.
///
/// # Safety
/// `v` must point to a valid `SlintVehicle`; single-threaded like the rest.
#[no_mangle]
pub unsafe extern "C" fn slint_set_vehicle(v: *const SlintVehicle) -> i32 {
    let Some(ui) = (unsafe { STATE.get() }).ui.as_ref() else {
        return 1;
    };
    let v = unsafe { &*v };
    ui.set_speed_x10(v.speed_x10);
    ui.set_rpm(v.rpm);
    ui.set_fuel_ml(v.fuel_ml);
    ui.set_tank_ml(v.tank_ml);
    ui.set_trip_m(v.trip_m);
    ui.set_trip_valid(v.trip_valid != 0);
    ui.set_odo_m(v.odo_m);
    ui.set_gear(v.gear);
    ui.set_engine_on(v.engine_on != 0);
    ui.set_engine_warn(v.engine_warn != 0);
    ui.set_turn_left(v.turn_left != 0);
    ui.set_turn_right(v.turn_right != 0);
    ui.set_lights(v.lights != 0);
    ui.set_fuel_low(v.fuel_low != 0);
    ui.set_fuel_empty(v.fuel_empty != 0);
    ui.set_uptime_secs(v.uptime_secs);
    ui.set_frame(v.frame);
    ui.set_standalone(v.standalone != 0);
    0
}

/// Draw something under the UI in every frame: `f` is called with the window's
/// framebuffer bound, after the renderer has cleared it and before the first
/// item is drawn, with the caller's GL context current. It must leave the GL
/// state it changes as FemtoVG expects it (blending on, program 0, no array
/// buffer bound, vertex attribute arrays disabled).
///
/// Route 3 on the i.MX8MP uses it to draw the background picture the safety
/// core rendered once at its boot, so the window's own background can stay
/// transparent and the GPU never draws the gradient itself.
///
/// Takes effect only when called before the first `slint_draw_frame`, which is
/// when the component and its window are made; a build that never calls it
/// renders exactly as before, with no notifier installed. Returns 0, or 1 when
/// the component already exists.
#[no_mangle]
pub extern "C" fn slint_set_underlay(f: Option<Underlay>) -> i32 {
    // Safety: single-threaded; STATE is only touched from this thread.
    let state = unsafe { STATE.get() };
    if state.ui.is_some() {
        return 1;
    }
    state.underlay = f;
    0
}

/// Say that the underlay (`slint_set_underlay`) together with what the UI
/// draws opaquely covers every pixel of the window, so the renderer need not
/// clear the window first: it skips the clear, which is a full-screen write
/// (4.2 ms of GPU a frame on the i.MX8MP's GC7000 at 1280x800). Neither the
/// colour nor the stencil buffer is cleared then; the underlay clears the
/// stencil itself if it wants it cleared. 0 restores the clear. Takes effect
/// from the next frame.
#[no_mangle]
pub extern "C" fn slint_set_underlay_covers(covers: i32) {
    // Safety: single-threaded; STATE is only touched from this thread.
    unsafe { STATE.get() }.underlay_covers = covers != 0;
}

/// Draw one Slint frame through the caller's current GL ES 2.0 context.
///
/// `get_proc` is the app's get-proc-address hook (EGL/GLES style). `width`/
/// `height` are the drawable size in pixels. The first call creates the platform
/// and the component. Returns 0 on success, non-zero on failure (1: set_platform,
/// 2: component build, 3: show, 4: no adapter, 5: render, 6: the underlay's
/// rendering notifier could not be installed).
#[no_mangle]
pub extern "C" fn slint_draw_frame(get_proc: GetProc, width: u32, height: u32) -> i32 {
    // Safety: single-threaded; STATE is only touched from this thread.
    let state = unsafe { STATE.get() };
    state.get_proc = Some(get_proc);

    let size = PhysicalSize::new(width, height);

    if !state.platform_set {
        if slint::platform::set_platform(Box::new(AppPlatform { size })).is_err() {
            return 1;
        }
        state.platform_set = true;
    }

    if state.ui.is_none() {
        match MainWindow::new() {
            Ok(ui) => {
                if state.underlay.is_some() {
                    // After the renderer's clear and before the first item:
                    // the one point at which a picture drawn into the window
                    // ends up under the whole UI (FemtoVG flushes the clear
                    // first, so it cannot wipe what this draws).
                    let installed =
                        ui.window().set_rendering_notifier(|rendering_state, _api| {
                            if let slint::RenderingState::BeforeRendering = rendering_state {
                                // Safety: single-threaded (see above).
                                if let Some(f) = unsafe { STATE.get() }.underlay {
                                    f();
                                }
                            }
                        });
                    if installed.is_err() {
                        return 6;
                    }
                }
                state.ui = Some(ui)
            }
            Err(_) => return 2,
        }
    }

    let ui = state.ui.as_ref().unwrap();
    // A drawable wider than the 1280x800 design (the Armadillo-X2's 1920x1200
    // HDMI monitor, same 8:5 shape) shows it scaled up; narrower ones are
    // unchanged (scale 1).
    if width > 1280 {
        ui.window().dispatch_event(slint::platform::WindowEvent::ScaleFactorChanged {
            scale_factor: width as f32 / 1280.0,
        });
    }
    ui.window().set_size(size);
    if ui.show().is_err() {
        return 3;
    }

    // Advance animations to the current uptime before rendering the frame.
    slint::platform::update_timers_and_animations();

    let adapter = match state.adapter.as_ref() {
        Some(a) => a,
        None => return 4,
    };
    adapter.renderer.set_skip_window_clear(state.underlay.is_some() && state.underlay_covers);
    if adapter.renderer.render().is_err() {
        return 5;
    }
    0
}

// ---------------------------------------------------------------------------
// Medical monitor scene (ui/medmon.slint)
// ---------------------------------------------------------------------------

/// The patient state the C side feeds each frame in the medical scene.
/// Mirrors the medical `in` properties of `MainWindow`; i32 fields, bools 0/1.
#[repr(C)]
pub struct SlintPatient {
    pub hr: i32,
    pub hr_valid: i32,
    pub spo2: i32,
    pub spo2_valid: i32,
    pub resp: i32,
    pub resp_valid: i32,
    pub lead_off: i32,
    pub signal_invalid: i32,
    pub hr_limit: i32,
    pub spo2_low: i32,
    pub blink: i32,
    pub lead_mask: i32,
    pub cursor_x: i32,
    pub uptime_secs: i32,
    pub frame: i32,
    pub standalone: i32,
}

/// Select the design: 0 = meter cluster, 1 = medical monitor. A no-op until
/// the first `slint_draw_frame` has created the component.
#[no_mangle]
pub extern "C" fn slint_set_scene(scene: i32) -> i32 {
    let Some(ui) = (unsafe { STATE.get() }).ui.as_ref() else {
        return 1;
    };
    ui.set_scene(scene);
    0
}

/// Push the patient state into the medical scene.
///
/// # Safety
/// `p` must point to a valid `SlintPatient`; single-threaded like the rest.
#[no_mangle]
pub unsafe extern "C" fn slint_set_patient(p: *const SlintPatient) -> i32 {
    let Some(ui) = (unsafe { STATE.get() }).ui.as_ref() else {
        return 1;
    };
    let p = unsafe { &*p };
    ui.set_hr(p.hr);
    ui.set_hr_valid(p.hr_valid != 0);
    ui.set_spo2(p.spo2);
    ui.set_spo2_valid(p.spo2_valid != 0);
    ui.set_resp(p.resp);
    ui.set_resp_valid(p.resp_valid != 0);
    ui.set_lead_off(p.lead_off != 0);
    ui.set_signal_invalid(p.signal_invalid != 0);
    ui.set_hr_limit(p.hr_limit != 0);
    ui.set_spo2_low(p.spo2_low != 0);
    ui.set_blink(p.blink != 0);
    ui.set_lead_mask(p.lead_mask);
    ui.set_cursor_x(p.cursor_x);
    ui.set_uptime_secs(p.uptime_secs);
    ui.set_frame(p.frame);
    ui.set_standalone(p.standalone != 0);
    0
}

/// Set one waveform channel (0..4) from SVG path commands in the channel's
/// 688 x 100 viewbox (built by the C side from the ECG stream).
///
/// # Safety
/// `commands` must be a NUL-terminated UTF-8 string that outlives the call.
#[no_mangle]
pub unsafe extern "C" fn slint_set_wave(channel: i32, commands: *const c_char) -> i32 {
    let Some(ui) = (unsafe { STATE.get() }).ui.as_ref() else {
        return 1;
    };
    let s = match unsafe { CStr::from_ptr(commands) }.to_str() {
        Ok(s) => slint::SharedString::from(s),
        Err(_) => return 2,
    };
    match channel {
        0 => ui.set_wave0(s),
        1 => ui.set_wave1(s),
        2 => ui.set_wave2(s),
        3 => ui.set_wave3(s),
        4 => ui.set_wave4(s),
        _ => return 3,
    }
    0
}
