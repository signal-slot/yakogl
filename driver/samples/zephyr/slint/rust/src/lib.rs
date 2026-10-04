// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The one Slint sample (driver/samples/zephyr/slint), its Rust half: the
//! same on the Raspberry Pi 4 (V3D), the Verdin AM62P (PowerVR) and the
//! Verdin iMX8M Plus (GC7000) (driver/docs/one-method.md, section 5, step 1).
//!
//! The C half (../src/main.c) gets the window from the board layer, brings
//! EGL/GL up through YakoGL and calls, in this order:
//!
//! 1. [`slint_sample_first_frame`]: `demo_vehicle` at t = 0 in two
//!    `slint_draw_frame`s (the first builds the component and draws its
//!    default state, the second the state set in between), Slint's clock at
//!    0 and then [`FRAME2_CLOCK_MS`]; then C swaps;
//! 2. [`slint_sample_check`]: the one check (crate `slint_check`), which
//!    also packs the PROBES and FINGERPRINT words;
//! 3. [`slint_sample_animate_frame`] for CONFIG_YAKOGL_BOARD_ANIMATE_S.
//!
//! The platform part (allocator, panic handler, the FemtoVG window adapter)
//! is gl_slint/rust/src/lib.rs's, without the parts only gc_slint uses
//! (underlay, the medical scene, the scale for wide panels); it is the one
//! left when gl_slint goes. The frame sequence and the clock are
//! v3d_gl_slint/src/main.c's (:118-135, :476-497), and `demo_vehicle` is
//! gl_slint/src/main.c's (:217-265), here in Rust.
#![no_std]

extern crate alloc;

use alloc::boxed::Box;
use alloc::rc::{Rc, Weak};
use core::alloc::{GlobalAlloc, Layout};
use core::cell::UnsafeCell;
use core::ffi::{c_char, c_void, CStr};
use core::fmt::Write;
use core::num::NonZeroU32;

use slint::platform::{Platform, Renderer, WindowAdapter};
use slint::{ComponentHandle, PhysicalSize, PlatformError, Window};

use i_slint_renderer_femtovg::opengl::OpenGLInterface;
use i_slint_renderer_femtovg::FemtoVGOpenGLRenderer;

use slint_check::{check, Frame, REFERENCE, TOLERANCE};

slint::include_modules!();

// ---------------------------------------------------------------------------
// Zephyr runtime glue
// ---------------------------------------------------------------------------

extern "C" {
    /// The UI heap, a sys_heap over a static arena in main.c.
    fn slint_heap_alloc(alignment: usize, size: usize) -> *mut c_void;
    fn slint_heap_free(ptr: *mut c_void);
    fn printk(fmt: *const c_char, ...);
}

struct ZephyrAlloc;

unsafe impl GlobalAlloc for ZephyrAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let align = layout.align().max(core::mem::size_of::<usize>());
        // SAFETY: main.c's sys_heap wrapper; any size and power-of-two alignment.
        unsafe { slint_heap_alloc(align, layout.size().max(1)) as *mut u8 }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, _layout: Layout) {
        // SAFETY: `ptr` came from `alloc` above.
        unsafe { slint_heap_free(ptr as *mut c_void) }
    }
}

#[global_allocator]
static GLOBAL: ZephyrAlloc = ZephyrAlloc;

/// A `core::fmt` sink that prints through printk, a line at a time (the
/// buffer flushes on a newline or when full).
struct Pk {
    buf: [u8; 192],
    n: usize,
}

impl Pk {
    const fn new() -> Self {
        Pk { buf: [0; 192], n: 0 }
    }

    fn flush(&mut self) {
        if self.n > 0 {
            self.buf[self.n] = 0;
            // SAFETY: a NUL-terminated buffer through "%s".
            unsafe { printk(c"%s\n".as_ptr(), self.buf.as_ptr() as *const c_char) };
            self.n = 0;
        }
    }
}

impl Write for Pk {
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

/// One console line.
macro_rules! line {
    ($($arg:tt)*) => {{
        let mut pk = Pk::new();
        let _ = write!(pk, $($arg)*);
        pk.flush();
    }};
}

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    // panic = "abort": print and spin; the board's watchdog resets the SoC
    // (CONFIG_YAKOGL_BOARD_ALIVE_S once the sample fed it, else the lifetime).
    let mut w = Pk::new();
    let _ = w.write_str("slint: rust panic: ");
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
// Single-threaded state (Slint's `unsafe-single-threaded` contract: main.c
// calls every entry point from its main thread only).
// ---------------------------------------------------------------------------

type GetProc = extern "C" fn(*const c_char) -> *const c_void;

struct Single<T>(UnsafeCell<T>);
// SAFETY: only ever accessed from the one UI thread (see above).
unsafe impl<T> Sync for Single<T> {}
impl<T> Single<T> {
    const fn new(v: T) -> Self {
        Single(UnsafeCell::new(v))
    }
    /// # Safety
    /// The single UI thread only.
    #[allow(clippy::mut_from_ref)]
    unsafe fn get(&self) -> &mut T {
        // SAFETY: the caller's contract.
        unsafe { &mut *self.0.get() }
    }
}

struct State {
    get_proc: Option<GetProc>,
    adapter: Option<Rc<AppWindowAdapter>>,
    ui: Option<MainWindow>,
    platform_set: bool,
    /// Slint's clock (Platform::duration_since_start), set by the sample so
    /// that the checked frame is the same picture on every run and board.
    clock_ms: u64,
}

static STATE: Single<State> =
    Single::new(State { get_proc: None, adapter: None, ui: None, platform_set: false, clock_ms: 0 });

/// The state. As gl_slint's crate: the entry points and Slint's callbacks
/// all run on the UI thread, one at a time.
fn state() -> &'static mut State {
    // SAFETY: the single-threaded contract above.
    unsafe { STATE.get() }
}

type BoxErr = Box<dyn core::error::Error + Send + Sync>;

/// The caller's current GL ES 2.0 context; every entry point is looked up
/// through its get-proc hook.
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
        match state().get_proc {
            Some(f) => f(name.as_ptr()),
            None => core::ptr::null(),
        }
    }
}

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
        state().adapter = Some(adapter.clone());
        Ok(adapter)
    }

    fn duration_since_start(&self) -> core::time::Duration {
        core::time::Duration::from_millis(state().clock_ms)
    }
}

// ---------------------------------------------------------------------------
// The vehicle: the `in` properties of MainWindow (cluster.slint).
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Vehicle {
    speed_x10: i32,
    rpm: i32,
    fuel_ml: i32,
    tank_ml: i32,
    trip_m: i32,
    trip_valid: bool,
    odo_m: i32,
    gear: i32,
    engine_on: bool,
    engine_warn: bool,
    turn_left: bool,
    turn_right: bool,
    lights: bool,
    fuel_low: bool,
    fuel_empty: bool,
    uptime_secs: i32,
    frame: i32,
    standalone: bool,
}

/// gl_slint's drive (gl_slint/src/main.c:217-265): the engine idles, the car
/// accelerates through the gears to about 120 km/h, cruises and brakes; the
/// fuel drains slowly and the left turn signal blinks on the way. A function
/// of `t_ms` only, so a frame's picture follows from its time stamp.
fn demo_vehicle(t_ms: i64, frame: u32) -> Vehicle {
    let t = t_ms % 30000;
    let (kmh_x10, rpm) = if t < 3000 {
        (0, 800 + (t * 200 / 3000) as i32)
    } else if t < 15000 {
        let u = t - 3000;
        ((1200 * u / 12000) as i32, 1500 + ((u % 3000) * 4500 / 3000) as i32)
    } else if t < 22000 {
        (1200 + (((t - 15000) % 2000) * 30 / 2000) as i32, 3200)
    } else {
        let u = t - 22000;
        ((1200 - 1200 * u / 8000) as i32, 2500 - (1700 * u / 8000) as i32)
    };
    let fuel_ml = 27500 - (t_ms / 1000) as i32 * 20;
    Vehicle {
        speed_x10: kmh_x10,
        rpm,
        engine_on: true,
        gear: if kmh_x10 <= 0 { 0 } else { (1 + kmh_x10 / 300).min(6) },
        tank_ml: 45000,
        fuel_ml,
        fuel_low: fuel_ml < 6000,
        fuel_empty: fuel_ml <= 0,
        trip_valid: true,
        trip_m: 12340 + (t_ms / 100) as i32,
        odo_m: 123456 + (t_ms / 100) as i32,
        engine_warn: false,
        turn_left: (5000..11000).contains(&t) && (t / 400) & 1 == 1,
        turn_right: false,
        lights: true,
        uptime_secs: (t_ms / 1000) as i32,
        frame: frame as i32,
        standalone: true,
    }
}

fn set_vehicle(ui: &MainWindow, v: &Vehicle) {
    ui.set_speed_x10(v.speed_x10);
    ui.set_rpm(v.rpm);
    ui.set_fuel_ml(v.fuel_ml);
    ui.set_tank_ml(v.tank_ml);
    ui.set_trip_m(v.trip_m);
    ui.set_trip_valid(v.trip_valid);
    ui.set_odo_m(v.odo_m);
    ui.set_gear(v.gear);
    ui.set_engine_on(v.engine_on);
    ui.set_engine_warn(v.engine_warn);
    ui.set_turn_left(v.turn_left);
    ui.set_turn_right(v.turn_right);
    ui.set_lights(v.lights);
    ui.set_fuel_low(v.fuel_low);
    ui.set_fuel_empty(v.fuel_empty);
    ui.set_uptime_secs(v.uptime_secs);
    ui.set_frame(v.frame);
    ui.set_standalone(v.standalone);
}

// ---------------------------------------------------------------------------
// Drawing
// ---------------------------------------------------------------------------

/// Slint's clock at the second checked frame: past the ~150 ms the cluster
/// animates into a new state (measured on the host, v3d_gl_slint/src/main.c
/// :118-128), so the checked picture does not depend on how long frame 1
/// took.
pub const FRAME2_CLOCK_MS: u64 = 1000;

/// One Slint frame into the current GL context. Returns 0, or 1
/// set_platform, 2 the component was not built, 3 show, 4 no window
/// adapter, 5 render (gl_slint's codes).
fn draw(get_proc: GetProc, width: u32, height: u32) -> i32 {
    let size = PhysicalSize::new(width, height);
    {
        let st = state();
        st.get_proc = Some(get_proc);
        if !st.platform_set {
            if slint::platform::set_platform(Box::new(AppPlatform { size })).is_err() {
                return 1;
            }
            st.platform_set = true;
        }
    }
    if state().ui.is_none() {
        match MainWindow::new() {
            Ok(ui) => state().ui = Some(ui),
            Err(_) => return 2,
        }
    }
    let Some(ui) = state().ui.as_ref() else {
        return 2;
    };
    ui.window().set_size(size);
    if ui.show().is_err() {
        return 3;
    }
    slint::platform::update_timers_and_animations();
    let Some(adapter) = state().adapter.as_ref() else {
        return 4;
    };
    if adapter.renderer.render().is_err() {
        return 5;
    }
    0
}

/// The draw's return code in words.
fn draw_rc_what(rc: i32) -> &'static str {
    match rc {
        0 => "ok",
        1 => "set_platform failed",
        2 => "the component was not built (a femtovg program that does not link panics instead)",
        3 => "show failed",
        4 => "no window adapter",
        5 => "the renderer's render() failed",
        _ => "an unknown return code",
    }
}

/// The checked frame: `demo_vehicle` at t = 0 in two draws (Slint clock 0,
/// then [`FRAME2_CLOCK_MS`]). The caller swaps afterwards. Returns 0, or the
/// failed draw's code (1-5, printed).
#[no_mangle]
pub extern "C" fn slint_sample_first_frame(get_proc: GetProc, width: u32, height: u32) -> i32 {
    state().clock_ms = 0;
    let rc = draw(get_proc, width, height);
    line!("slint: frame 1 (build + default state): rc {rc}, {}", draw_rc_what(rc));
    if rc != 0 {
        return rc;
    }
    if let Some(ui) = state().ui.as_ref() {
        set_vehicle(ui, &demo_vehicle(0, 0));
    }
    state().clock_ms = FRAME2_CLOCK_MS;
    let rc = draw(get_proc, width, height);
    line!("slint: frame 2 (demo_vehicle t=0, clock {FRAME2_CLOCK_MS} ms): rc {rc}, {}", draw_rc_what(rc));
    rc
}

/// One animation frame `t_ms` after the animation started (Slint's clock
/// runs on from [`FRAME2_CLOCK_MS`]). The caller swaps afterwards. Returns
/// 0 or the draw's code (printed).
#[no_mangle]
pub extern "C" fn slint_sample_animate_frame(get_proc: GetProc, width: u32, height: u32, t_ms: u32, frame: u32) -> i32 {
    let Some(ui) = state().ui.as_ref() else {
        line!("slint: animation frame {frame}: no component (the first frame did not run)");
        return 2;
    };
    set_vehicle(ui, &demo_vehicle(t_ms as i64, frame));
    state().clock_ms = FRAME2_CLOCK_MS + t_ms as u64;
    let rc = draw(get_proc, width, height);
    if rc != 0 {
        line!("slint: animation frame {frame} at {t_ms} ms: rc {rc}, {}", draw_rc_what(rc));
    }
    rc
}

// ---------------------------------------------------------------------------
// The check
// ---------------------------------------------------------------------------

/// What C stores at the end of the run: board_result(FINGERPRINT,
/// fingerprint...) and board_result_tag(PROBES, status, probes...). The
/// layout is main.c's `struct slint_sample_result`.
#[repr(C)]
pub struct SampleResult {
    /// record STATUS_PASS (1) or STATUS_FAIL (2): the whole check.
    pub status: u32,
    pub probes: [u32; 3],
    pub fingerprint: [u32; 3],
}

const _: () = assert!(core::mem::size_of::<SampleResult>() == 28);

const GL_RGBA: u32 = 0x1908;
const GL_UNSIGNED_BYTE: u32 = 0x1401;
type ReadPixels = extern "C" fn(i32, i32, i32, i32, u32, u32, *mut c_void);

/// Check the window as it stands after the first frame's swap. `cpu` is the
/// window's memory as board_window gave it: h + guard_rows rows of w
/// pixels, `stride` pixels apart (w, or wider for a view into the scan-out
/// buffer), the guard rows after the window. `guard_before` is the guard
/// rows as the sample read them before the frame (`guard_rows * w` words;
/// null: [`slint_check::GUARD`]). `format` is the native window's format
/// word. The GL context that drew it is current.
///
/// Returns 0 for a pass, 1 for a failed check (the printed line names what
/// failed), -1 when the frame could not be checked (the reason is printed;
/// `out` then holds a FAIL status and zero words).
///
/// # Safety
/// `cpu` is readable for (h + guard_rows - 1) * stride + w words and
/// nothing writes it during the call; `guard_before` is null or readable
/// for guard_rows * w words; `out` is writable.
#[no_mangle]
pub unsafe extern "C" fn slint_sample_check(
    cpu: *const u32,
    w: u32,
    h: u32,
    stride: u32,
    guard_rows: u32,
    guard_before: *const u32,
    format: u32,
    out: *mut SampleResult,
) -> i32 {
    // SAFETY: the caller's contract.
    let out = unsafe { &mut *out };
    *out = SampleResult { status: slint_check::record::STATUS_FAIL, probes: [0; 3], fingerprint: [0; 3] };
    if stride < w || h == 0 {
        line!("slint: check: not checked: stride {stride} below the width {w}, or no rows");
        return -1;
    }
    let n = (h as usize + guard_rows as usize - 1) * stride as usize + w as usize;
    // SAFETY: the caller's contract.
    let pixels = unsafe { core::slice::from_raw_parts(cpu, n) };
    let frame = Frame::with_stride(pixels, w, h, stride, guard_rows, format).and_then(|f| {
        if guard_before.is_null() {
            Ok(f)
        } else {
            // SAFETY: the caller's contract.
            f.with_guard(unsafe { core::slice::from_raw_parts(guard_before, (guard_rows * w) as usize) })
        }
    });
    let frame = match frame {
        Ok(f) => f,
        Err(e) => {
            line!("slint: check: not checked: {} ({w}x{h}, format {format:#x})", e.what());
            return -1;
        }
    };
    let read_pixels: Option<ReadPixels> = state().get_proc.and_then(|gp| {
        let p = gp(c"glReadPixels".as_ptr());
        // SAFETY: glReadPixels' C signature, as eglGetProcAddress returns it.
        (!p.is_null()).then(|| unsafe { core::mem::transmute::<*const c_void, ReadPixels>(p) })
    });
    let Some(read_pixels) = read_pixels else {
        line!("slint: check: not checked: glReadPixels was not resolved");
        return -1;
    };
    let o = check(&frame, |x, y| {
        let mut px = [0u8; 4];
        read_pixels(x as i32, y as i32, 1, 1, GL_RGBA, GL_UNSIGNED_BYTE, px.as_mut_ptr() as *mut c_void);
        px
    });
    for (i, p) in REFERENCE.iter().enumerate() {
        let (m, g) = (o.points[i], o.via_gl[i]);
        line!(
            "slint: point {:<11} ({:3},{:3}) = {},{},{},{}: AM62P {},{},{} (diff {}){}{}",
            p.what,
            p.x,
            p.y,
            m[0],
            m[1],
            m[2],
            m[3],
            p.want[0],
            p.want[1],
            p.want[2],
            o.diff[i],
            if o.diff[i] > TOLERANCE { "  WRONG" } else { "" },
            if g != m { "  glReadPixels DIFFERENT" } else { "" }
        );
        if g != m {
            line!("slint:   glReadPixels {},{},{},{}", g[0], g[1], g[2], g[3]);
        }
    }
    line!(
        "slint: window {w}x{h}: {} sentinel pixels left, {} with alpha != 255, guard rows {} of {} changed, crc32 0x{:08x}",
        o.sentinel_left,
        o.alpha_bad,
        o.guard_bad,
        w * guard_rows,
        o.crc32
    );
    out.status = o.status() as u32;
    out.probes = o.probes_words();
    out.fingerprint = o.fingerprint_words(w, h);
    match o.verdict() {
        None => {
            line!("slint: check PASS (tolerance {TOLERANCE} per channel)");
            0
        }
        Some(f) => {
            line!("slint: check FAIL: {}", f.what());
            1
        }
    }
}
