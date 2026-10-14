// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The EGL calls of the three benchmarks, with their exact attribute lists,
//! against the library's EGL of each GPU build: does `eglChooseConfig`
//! return a config with the colour, depth and stencil the benchmark needs,
//! and do a window surface of the board's size and an ES 2 context come out
//! of it?
//!
//! Where the lists come from (the samples under driver/samples/zephyr/):
//!
//! * Slint (`slint/src/main.c`, `egl_up`) and the AGL cluster
//!   (`cluster/src/main.c`, `egl_up`) pass the same list: RGBA8888, stencil 8
//!   (femtovg fills concave paths and strokes through the stencil), window
//!   bit, ES 2 bit, one config asked for, and they REFUSE to start unless
//!   `n == 1`. Context attributes: `EGL_CONTEXT_CLIENT_VERSION 2`. Slint's
//!   window is 1024x600 on every board (`WIN_W`/`WIN_H`); the cluster makes
//!   two window surfaces, the two halves of one buffer, each the panel's size
//!   (`CONFIG_CLUSTER_PANEL_WIDTH/HEIGHT`, boards/*.conf: 1024x600 AM62P,
//!   1280x800 i.MX8MP, 1920x720 Pi 4), packed rows (it refuses any other
//!   stride), half 1 right after half 0.
//! * glmark2 is upstream's `GLStateEGL` (gl-state-egl.cpp at the pinned
//!   `GLMARK2_COMMIT` 22c527cb, glmark2/CMakeLists.txt), built with
//!   `GLMARK2_USE_GLESv2` and `GLMARK2_USE_EGL`. `gotValidConfig` passes
//!   `EGL_RENDERABLE_TYPE, EGL_OPENGL_ES2_BIT` and nothing else: no surface
//!   type, no sizes. It asks for the count first (`configs` NULL, size 0),
//!   then for all of them, and picks the one with the best
//!   `GLVisualConfig::match_score` against the requested visual config, the
//!   default one unless `--visual-config` is given (CONFIG_GLMARK2_ARGS is ""
//!   on every board): r=g=b=a=1, depth 1, stencil -1 (don't care), buffer 1,
//!   samples 0. A best score of 0 or less is a warning (a refusal with
//!   `--good-config`). The window surface has no attribute list; the context
//!   list is `EGL_CONTEXT_CLIENT_VERSION 2`. The window is
//!   `CONFIG_GLMARK2_SIZE`, 800x600 (glmark2/Kconfig; no board overrides
//!   it). The scenes need a depth buffer (terrain, refract, shadow, ...), so
//!   the test also requires 16 or more depth bits on the config picked.
//!
//! The board is the GPU feature's: AM62P for `gpu-powervr`, i.MX8MP for
//! `gpu-vivante`, Pi 4 for `gpu-v3d`, with the window formats its board layer
//! hands out (driver/zephyr/lib/board/<board>/: XRGB8888 everywhere, plus the
//! PowerVR samples' bottom-up flag on the AM62P and RGBA8888 for a Pi 4
//! framebuffer in that byte order; RGB565 only with
//! CONFIG_YAKOGL_BOARD_WINDOW_RGB565, which no benchmark image sets).
//!
//! The library has no GPU here: the test installs a minimal OSAL (memory from
//! the host heap, a `map_phys` that maps nothing) and a backend table with the
//! capabilities of the GPU feature under test, as `testutil::TestGl` does for
//! the unit tests (which an integration test cannot use: it is `cfg(test)`).
//! Run it per GPU:
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr --test egl_bench_configs
//! cargo test --no-default-features --features host-test,gpu-vivante --test egl_bench_configs
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test egl_bench_configs
//! ```
#![cfg(all(feature = "host-test", any(feature = "gpu-powervr", feature = "gpu-vivante", feature = "gpu-v3d")))]

use core::ffi::c_void;
use std::sync::Mutex;

use yakogl::backend::{self, YakoglBackend, GLES_ATTRIBF_FLOAT32, GLES_HEAP_PROGRAM, GLES_HEAP_SHADER};
use yakogl::gl::egl::*;
use yakogl::gl::egl_consts::*;
use yakogl::gl::types::*;
use yakogl::osal::{OsalGpuMem, OsalOps};

/// One test at a time: the EGL state and the OSAL table are process-wide.
static SERIAL: Mutex<()> = Mutex::new(());

/* ---- the board of the GPU feature ----------------------------------------- */

/// The board, its cluster panel and the window formats its board layer gives.
struct Board {
    name: &'static str,
    panel: (u32, u32),
    formats: &'static [u32],
}

#[cfg(feature = "gpu-powervr")]
const BOARD: Board = Board {
    name: "AM62P",
    panel: (1024, 600),
    /* board_am62p.c: XRGB8888 | pvr_sample_window_flags() (bottom-up where
     * the PowerVR samples flip) */
    formats: &[GLES_NATIVE_FORMAT_XRGB8888, GLES_NATIVE_FORMAT_XRGB8888 | GLES_NATIVE_WINDOW_BOTTOM_UP],
};

#[cfg(feature = "gpu-vivante")]
const BOARD: Board = Board {
    name: "i.MX8MP",
    panel: (1280, 800),
    /* board_imx8mp.c: XRGB8888, bottom-up refused */
    formats: &[GLES_NATIVE_FORMAT_XRGB8888],
};

#[cfg(feature = "gpu-v3d")]
const BOARD: Board = Board {
    name: "Pi 4",
    panel: (1920, 720),
    /* board_rpi4.c: the framebuffer's byte order, XRGB8888 or RGBA8888 */
    formats: &[GLES_NATIVE_FORMAT_XRGB8888, GLES_NATIVE_FORMAT_RGBA8888],
};

/// Slint's window on every board (slint/src/main.c `WIN_W`/`WIN_H`).
const SLINT_WINDOW: (u32, u32) = (1024, 600);
/// glmark2's `CONFIG_GLMARK2_SIZE` default; no board overrides it.
const GLMARK2_WINDOW: (u32, u32) = (800, 600);

/* ---- the platform: host memory, no GPU ------------------------------------- */

unsafe extern "C" fn m_init(_arg: *mut c_void) -> i32 {
    0
}

unsafe extern "C" fn m_gpu_alloc(_heap: u64, size: usize, _flags: u32, out: *mut OsalGpuMem) -> i32 {
    if out.is_null() || size == 0 {
        return -22;
    }
    let layout = std::alloc::Layout::from_size_align(size, 4096).expect("a layout");
    // SAFETY: a non-zero size. The memory is kept for the process: a test
    // allocates a few buffers, and the library's free path is not under test.
    let p = unsafe { std::alloc::alloc_zeroed(layout) };
    assert!(!p.is_null(), "host allocation of {size} bytes");
    // SAFETY: `out` is a valid out-parameter of the library's call.
    unsafe { *out = OsalGpuMem { cpu: p.cast(), gpu: p as u64, phys: p as u64, size, impl_: core::ptr::null_mut() } };
    0
}

unsafe extern "C" fn m_gpu_free(_mem: *mut OsalGpuMem) -> i32 {
    0
}

/// A window's pages mapped at a device address: here the address is the
/// physical one and there is no CPU view.
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

static OSAL: OsalOps = OsalOps {
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
    lock: None,
    unlock: None,
    thread_id: None,
};

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

/// The backend table of the GPU feature: the capabilities that decide how the
/// GL core lays out a surface (the real tables' values, src/{pvr,vivante,
/// v3d}/backend.rs), the draw entry points absent: no call here draws.
fn backend_table() -> YakoglBackend {
    let (native_attrib_formats, bgra, mip_align, blocks, layered, etc1) = if cfg!(feature = "gpu-vivante") {
        ((1u32 << backend::GLES_ATTRIBF_COUNT) - 1, true, 64, true, true, false)
    } else if cfg!(feature = "gpu-v3d") {
        (1 << GLES_ATTRIBF_FLOAT32, false, 0, false, true, true)
    } else {
        (0, false, 0, false, false, false)
    };
    YakoglBackend {
        name: c"egl_bench_configs".as_ptr(),
        native_attrib_formats,
        rt_rgba8888_is_bgra: bgra,
        mip_level_align: mip_align,
        compressed_blocks_in_rows: blocks,
        native_layered_textures: layered,
        front_facing_shader_variant: false,
        etc1_decoded_at_upload: etc1,
        init: Some(b_init),
        shutdown: None,
        draw: None,
        flush: None,
        finish: None,
        finish_previous: None,
        kick_id: None,
        kick_pending: None,
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
    }
}

/// The platform and backend installed, the lock held, a display up;
/// `eglTerminate` when dropped.
struct Egl {
    dpy: EGLDisplay,
    _serial: std::sync::MutexGuard<'static, ()>,
}

fn egl() -> Egl {
    let serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    // The table lives for the process; each test installs an equal one.
    let table: &'static YakoglBackend = Box::leak(Box::new(backend_table()));
    // SAFETY: tests are serialised by SERIAL; the tables are static.
    unsafe {
        yakogl::osal::osal_set_ops(&OSAL);
        backend::g_backend = table;
        let dpy = eglGetDisplay(core::ptr::null_mut());
        assert!(!dpy.is_null());
        assert_eq!(eglInitialize(dpy, core::ptr::null_mut(), core::ptr::null_mut()), 1, "eglInitialize");
        Egl { dpy, _serial: serial }
    }
}

impl Drop for Egl {
    fn drop(&mut self) {
        // SAFETY: the display this test initialised.
        unsafe { eglTerminate(self.dpy) };
    }
}

/* ---- the benchmarks' lists ------------------------------------------------- */

/// `slint/src/main.c` and `cluster/src/main.c` `egl_up`: cfg_attribs.
#[rustfmt::skip]
const FEMTOVG_CFG: &[EGLint] = &[
    EGL_RED_SIZE, 8, EGL_GREEN_SIZE, 8, EGL_BLUE_SIZE, 8, EGL_ALPHA_SIZE, 8,
    EGL_STENCIL_SIZE, 8, EGL_SURFACE_TYPE, EGL_WINDOW_BIT,
    EGL_RENDERABLE_TYPE, EGL_OPENGL_ES2_BIT, EGL_NONE,
];

/// glmark2's `GLStateEGL::gotValidConfig` config_attribs (GLESv2, not the
/// null platform): the renderable type alone.
const GLMARK2_CFG: &[EGLint] = &[EGL_RENDERABLE_TYPE, EGL_OPENGL_ES2_BIT, EGL_NONE];

/// The context list all three pass.
const CTX_ES2: &[EGLint] = &[EGL_CONTEXT_CLIENT_VERSION, 2, EGL_NONE];

/* ---- helpers ---------------------------------------------------------------- */

fn attrib(e: &Egl, cfg: EGLConfig, a: EGLint) -> EGLint {
    let mut v = -1;
    // SAFETY: a valid config of the display and a live out-parameter.
    assert_eq!(unsafe { eglGetConfigAttrib(e.dpy, cfg, a, &mut v) }, 1, "eglGetConfigAttrib {a:#x}");
    v
}

/// The configs `list` returns, up to `max` of them.
fn choose(e: &Egl, list: &[EGLint], max: usize) -> Vec<EGLConfig> {
    let mut cfgs = vec![core::ptr::null_mut(); max];
    let mut n = -1;
    // SAFETY: the list is EGL_NONE-terminated; `cfgs` holds `max` entries.
    let ok = unsafe { eglChooseConfig(e.dpy, list.as_ptr(), cfgs.as_mut_ptr(), max as EGLint, &mut n) };
    assert_eq!(ok, 1, "eglChooseConfig");
    cfgs.truncate(n as usize);
    cfgs
}

/// The configs a benchmark's list returned, for a failure message.
fn describe(e: &Egl, cfgs: &[EGLConfig]) -> String {
    cfgs.iter()
        .map(|&c| {
            let a = |x| attrib(e, c, x);
            format!(
                "id {} rgba {}{}{}{} depth {} stencil {} samples {} surface {:#x}",
                a(EGL_CONFIG_ID),
                a(EGL_RED_SIZE),
                a(EGL_GREEN_SIZE),
                a(EGL_BLUE_SIZE),
                a(EGL_ALPHA_SIZE),
                a(EGL_DEPTH_SIZE),
                a(EGL_STENCIL_SIZE),
                a(EGL_SAMPLES),
                a(EGL_SURFACE_TYPE)
            )
        })
        .collect::<Vec<_>>()
        .join("; ")
}

/// Colour, depth and stencil of `cfg` meet the minimums, and it is a window
/// config with ES 2.
fn assert_sizes(e: &Egl, who: &str, cfg: EGLConfig, rgba: i32, depth: i32, stencil: i32, all: &[EGLConfig]) {
    let got = [EGL_RED_SIZE, EGL_GREEN_SIZE, EGL_BLUE_SIZE, EGL_ALPHA_SIZE].map(|a| attrib(e, cfg, a));
    let ctx = || format!("{who}: configs returned: {}", describe(e, all));
    assert!(got.iter().all(|&c| c >= rgba), "colour {got:?} < {rgba} each; {}", ctx());
    assert!(attrib(e, cfg, EGL_DEPTH_SIZE) >= depth, "depth < {depth}; {}", ctx());
    assert!(attrib(e, cfg, EGL_STENCIL_SIZE) >= stencil, "stencil < {stencil}; {}", ctx());
    assert!(attrib(e, cfg, EGL_SURFACE_TYPE) & EGL_WINDOW_BIT != 0, "no window bit; {}", ctx());
    assert!(attrib(e, cfg, EGL_RENDERABLE_TYPE) & EGL_OPENGL_ES2_BIT != 0, "no ES 2 bit; {}", ctx());
}

/// A window of `w` x `h`, packed rows of 4 bytes a pixel, in `format`; the
/// physical address is the board layer's job and is any aligned one here.
fn window(phys: u64, w: u32, h: u32, format: u32) -> GlesNativeWindow {
    GlesNativeWindow { phys, width: w, height: h, stride_bytes: w * 4, format }
}

/// One surface per window on `cfg`, each of its size, and an ES 2 context
/// made current on the first: the GL core exists and reports no error.
fn surfaces_and_context(e: &Egl, who: &str, cfg: EGLConfig, wins: &mut [GlesNativeWindow]) {
    let mut surfs = Vec::new();
    for w in wins.iter_mut() {
        // SAFETY: a valid window description; no attribute list.
        let s = unsafe { eglCreateWindowSurface(e.dpy, cfg, w, core::ptr::null()) };
        // SAFETY: plain call.
        let err = unsafe { eglGetError() };
        assert!(
            !s.is_null(),
            "{who}: eglCreateWindowSurface {}x{} format {:#x}: EGL error {err:#x}",
            w.width,
            w.height,
            w.format
        );
        for (a, want) in [(EGL_WIDTH, w.width), (EGL_HEIGHT, w.height)] {
            let mut v = 0;
            // SAFETY: a live surface and out-parameter.
            assert_eq!(unsafe { eglQuerySurface(e.dpy, s, a, &mut v) }, 1);
            assert_eq!(v as u32, want, "{who}: surface attribute {a:#x}");
        }
        surfs.push(s);
    }
    // SAFETY: the context list is EGL_NONE-terminated.
    let ctx = unsafe { eglCreateContext(e.dpy, cfg, core::ptr::null_mut(), CTX_ES2.as_ptr()) };
    // SAFETY: plain call.
    let err = unsafe { eglGetError() };
    assert!(!ctx.is_null(), "{who}: eglCreateContext: EGL error {err:#x}");
    let mut version = 0;
    // SAFETY: a live context and out-parameter.
    assert_eq!(unsafe { eglQueryContext(e.dpy, ctx, EGL_CONTEXT_CLIENT_VERSION, &mut version) }, 1);
    /* an ES 2 request may get a backwards-compatible later version (EGL 1.5
     * 3.7.1.1): this EGL's one kind of context reports 3, in report-es2
     * builds too (GL_VERSION there says 2.0) */
    assert!(version >= 2, "{who}: the context's client version is {version}");
    // SAFETY: live surface and context of the display.
    let made = unsafe { eglMakeCurrent(e.dpy, surfs[0], surfs[0], ctx) };
    // SAFETY: plain call.
    let err = unsafe { eglGetError() };
    assert_eq!(made, 1, "{who}: eglMakeCurrent: EGL error {err:#x}");
    // SAFETY: a context is current.
    assert_eq!(unsafe { yakogl::gl::state::glGetError() }, 0, "{who}: GL error after eglMakeCurrent");
    // SAFETY: tear down what this call made.
    unsafe {
        eglMakeCurrent(e.dpy, core::ptr::null_mut(), core::ptr::null_mut(), core::ptr::null_mut());
        eglDestroyContext(e.dpy, ctx);
        for s in surfs {
            eglDestroySurface(e.dpy, s);
        }
    }
}

/* ---- Slint and the cluster (femtovg) ---------------------------------------- */

/// Slint and the cluster: the one config they ask for (`n == 1` is their
/// check), RGBA8888 with an 8-bit stencil, and their windows: `count`
/// windows of `size`, back to back in one buffer.
fn femtovg_case(who: &str, size: (u32, u32), count: u64) {
    let who = format!("{who} on the {} ({count} x {}x{})", BOARD.name, size.0, size.1);
    for &format in BOARD.formats {
        let e = egl();
        let all = choose(&e, FEMTOVG_CFG, 8);
        let mut cfg = core::ptr::null_mut();
        let mut n = 0;
        // SAFETY: as the samples call it: one config asked for.
        let ok = unsafe { eglChooseConfig(e.dpy, FEMTOVG_CFG.as_ptr(), &mut cfg, 1, &mut n) };
        assert_eq!(ok, 1, "{who}: eglChooseConfig");
        assert_eq!(n, 1, "{who}: the sample refuses to start unless n == 1; configs: {}", describe(&e, &all));
        assert_eq!(cfg, all[0], "{who}: one config asked for is the first of the list");
        assert_sizes(&e, &who, cfg, 8, 0, 8, &all);
        let half = u64::from(size.0) * u64::from(size.1) * 4;
        let mut wins: Vec<_> = (0..count).map(|k| window(0x4000_0000 + k * half, size.0, size.1, format)).collect();
        surfaces_and_context(&e, &format!("{who} format {format:#x}"), cfg, &mut wins);
    }
}

#[test]
fn slint_gets_an_rgba8888_stencil8_config_and_its_1024x600_window() {
    femtovg_case("slint", SLINT_WINDOW, 1);
}

#[test]
fn the_cluster_gets_its_config_and_two_panel_sized_halves() {
    femtovg_case("cluster", BOARD.panel, 2);
}

/* ---- glmark2 ------------------------------------------------------------------ */

/// glmark2's `GLVisualConfig` (gl-visual-config.h): the default is
/// r=g=b=a=1, depth 1, stencil -1, buffer 1, samples 0.
#[derive(Clone, Copy, Debug)]
struct VisualConfig {
    red: i32,
    green: i32,
    blue: i32,
    alpha: i32,
    depth: i32,
    stencil: i32,
    buffer: i32,
    samples: i32,
}

const GLMARK2_DEFAULT_VISUAL: VisualConfig =
    VisualConfig { red: 1, green: 1, blue: 1, alpha: 1, depth: 1, stencil: -1, buffer: 1, samples: 0 };

/// `GLVisualConfig::score_component`, as upstream has it.
fn score_component(component: i32, target: i32, scale: i32) -> i32 {
    const MAX: i32 = 32;
    const PENALTY: i32 = -1000;
    if (component > 0 && target == 0) || (component == 0 && target > 0) {
        PENALTY
    } else if component == target {
        MAX
    } else if component > 8 && target <= 8 && scale > 1 {
        PENALTY
    } else {
        let diff = scale.abs() * (component - target);
        if diff > 0 {
            let s = if scale < 0 || target < 0 { MAX - diff } else { diff };
            s.clamp(0, MAX)
        } else {
            0
        }
    }
}

/// `GLVisualConfig::match_score` (no target id).
fn match_score(vc: VisualConfig, target: VisualConfig) -> i32 {
    score_component(vc.red, target.red, 4)
        + score_component(vc.green, target.green, 4)
        + score_component(vc.blue, target.blue, 4)
        + score_component(vc.alpha, target.alpha, 4)
        + score_component(vc.depth, target.depth, 1)
        + score_component(vc.stencil, target.stencil, 1)
        + score_component(vc.buffer, target.buffer, 1)
        + score_component(vc.samples, target.samples, -1)
}

/// `GLStateEGL::get_glvisualconfig`.
fn visual_of(e: &Egl, cfg: EGLConfig) -> VisualConfig {
    let a = |x| attrib(e, cfg, x);
    VisualConfig {
        red: a(EGL_RED_SIZE),
        green: a(EGL_GREEN_SIZE),
        blue: a(EGL_BLUE_SIZE),
        alpha: a(EGL_ALPHA_SIZE),
        depth: a(EGL_DEPTH_SIZE),
        stencil: a(EGL_STENCIL_SIZE),
        buffer: a(EGL_BUFFER_SIZE),
        samples: a(EGL_SAMPLES),
    }
}

#[test]
fn glmark2_picks_a_window_config_with_colour_and_depth_for_its_800x600_window() {
    let who = format!("glmark2 on the {} ({}x{})", BOARD.name, GLMARK2_WINDOW.0, GLMARK2_WINDOW.1);
    for &format in BOARD.formats {
        let e = egl();
        // The count query: configs NULL, size 0.
        let mut count = -1;
        // SAFETY: the list is EGL_NONE-terminated; no array, as upstream calls it.
        let ok = unsafe { eglChooseConfig(e.dpy, GLMARK2_CFG.as_ptr(), core::ptr::null_mut(), 0, &mut count) };
        assert_eq!(ok, 1, "{who}: eglChooseConfig (count query)");
        assert!(count > 0, "{who}: eglChooseConfig didn't return any configs");
        let all = choose(&e, GLMARK2_CFG, count as usize);
        assert_eq!(all.len(), count as usize, "{who}: the second call returns as many as the count");

        // select_best_config: the first config with the highest score.
        let scored: Vec<_> = all.iter().map(|&c| (c, match_score(visual_of(&e, c), GLMARK2_DEFAULT_VISUAL))).collect();
        let (cfg, best) =
            scored.iter().fold((core::ptr::null_mut(), i32::MIN), |b, &(c, s)| if s > b.1 { (c, s) } else { b });
        let scores = || {
            scored
                .iter()
                .map(|&(c, s)| format!("id {} score {s}", attrib(&e, c, EGL_CONFIG_ID)))
                .collect::<Vec<_>>()
                .join(", ")
        };
        assert!(best > 0, "{who}: no good config (best score {best}: {}); configs: {}", scores(), describe(&e, &all));

        // The config picked: 8-bit colour for a 32-bit window, a window
        // config (glmark2 does not ask for the window bit), a real depth
        // buffer for the scenes' depth test, single-sampled.
        let ctx = || {
            format!(
                "{who}: picked id {} ({}); configs: {}",
                attrib(&e, cfg, EGL_CONFIG_ID),
                scores(),
                describe(&e, &all)
            )
        };
        assert_sizes(&e, &who, cfg, 8, 16, 0, &all);
        assert_eq!(attrib(&e, cfg, EGL_BUFFER_SIZE), 32, "{}", ctx());
        assert_eq!(attrib(&e, cfg, EGL_SAMPLES), 0, "{}", ctx());

        let mut wins = [window(0x4000_0000, GLMARK2_WINDOW.0, GLMARK2_WINDOW.1, format)];
        surfaces_and_context(&e, &format!("{who} format {format:#x}"), cfg, &mut wins);
    }
}
