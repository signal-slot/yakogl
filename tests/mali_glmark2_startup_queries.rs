// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The queries glmark2-es2 (22c527cb) makes at start-up, on the Mali back
//! end through the GL API on the host. Each must give `GL_NO_ERROR` and a
//! value within the ES 2.0 minimums (ES 2.0 spec table 6.18/6.20). The
//! extension list glmark2 looks for is pinned, so that a change shows.
//!
//! What glmark2 asks (src/canvas-generic.cpp, gl-headers.cpp,
//! scene-*.cpp, text-renderer.cpp):
//!
//! * `glGetString(GL_VENDOR, GL_RENDERER, GL_VERSION)`: the report, and
//!   `CanvasGeneric::supports_gl2` reads the major number out of
//!   GL_VERSION (it must be >= 2, glmark2 refuses to run otherwise).
//!   GL_SHADING_LANGUAGE_VERSION is not read by glmark2 but is part of
//!   the same start-up set of ES queries and is checked here too.
//! * `GL_EXTENSIONS` (`GLExtensions::support`, a whole-token match):
//!   - `GL_OES_depth_texture` (or `GL_ARB_depth_texture`): scene-shadow
//!     and scene-refract `supported()`; without it both scenes are
//!     skipped ("We do not have the depth texture extension"). With it
//!     they make their depth `GL_DEPTH_COMPONENT` / `GL_UNSIGNED_INT`
//!     texture.
//!   - `GL_OES_rgb8_rgba8` / `GL_ARM_rgba8`: canvas-generic picks
//!     `GL_RGBA8` (else `GL_RGBA4`) for the 32-bit offscreen colour
//!     renderbuffer, and with `GL_OES_rgb8_rgba8` `GL_RGB8` (else
//!     `GL_RGB565`) for 24-bit.
//!   - `GL_OES_depth24` / `GL_OES_depth32`: the offscreen depth
//!     renderbuffer is `GL_DEPTH_COMPONENT24` / `32` when advertised,
//!     else `GL_DEPTH_COMPONENT16`.
//!   - `GL_OES_mapbuffer`: not read through `support()`; glmark2 binds
//!     `glMapBufferOES` / `glUnmapBufferOES` at load (gl-state-egl.cpp,
//!     `init_gl_extensions`) for the buffer scene's map path.
//!   - `GL_OES_texture_npot`: not read by glmark2; it decides whether
//!     refract's and the desktop scene's NPOT mip chain is complete.
//! * `glGetIntegerv`: `GL_MAX_TEXTURE_SIZE` (shadow and refract clamp the
//!   depth/RTT texture to it), `GL_MAX_VERTEX_TEXTURE_IMAGE_UNITS` (terrain
//!   needs > 0), `GL_FRAMEBUFFER_BINDING` (desktop), `GL_CURRENT_PROGRAM`,
//!   `GL_ARRAY_BUFFER_BINDING`, `GL_ELEMENT_ARRAY_BUFFER_BINDING`,
//!   `GL_BLEND_SRC_RGB/DST_RGB/SRC_ALPHA/DST_ALPHA` (jellyfish, the text
//!   renderer), and `glGetBooleanv(GL_BLEND, GL_DEPTH_TEST)`. The
//!   limits `GL_MAX_VARYING_VECTORS`, `GL_MAX_VERTEX_ATTRIBS`,
//!   `GL_MAX_TEXTURE_IMAGE_UNITS` and the colour/depth bit counts are not
//!   read by glmark2's source but by the start-up set of every ES client
//!   and by the ES 2.0 minimums; they are checked too.
//!
//! The values here are ES spec facts, not glmark2 text.
//!
//! Which report: the extension list differs. The default (ES 3.0) report
//! has the depth texture and NPOT in core and does not list
//! `GL_OES_depth_texture` or `GL_OES_texture_npot`; `report-es2`, the
//! glmark2-es2 board library's report, lists them. The test pins both.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_glmark2_startup_queries
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table,report-es2 --test mali_glmark2_startup_queries
//! ```

#![cfg(all(feature = "host-test", feature = "shaderc-online", feature = "gpu-mali"))]

use core::ffi::{c_char, c_void};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{self, YakoglBackend};
use yakogl::gl::consts::*;
use yakogl::gl::state::*;
use yakogl::gl::types::*;
use yakogl::osal::mali::{OsalMaliGpu, OsalMaliHeap, OsalMaliKick, OSAL_QUERY_MALI_GPU, OSAL_QUERY_MALI_TILER_HEAP};
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};
use yakogl::util::ENOTSUP;

/* ---- the Mali over a mock OSAL ------------------------------------------ */

const GPU: OsalMaliGpu = OsalMaliGpu::T6;
const HEAP: OsalMaliHeap =
    OsalMaliHeap { ctx_gpu: 0x8000_0001_5000, first_chunk_gpu: 0x8000_00a0_0000, chunk_size: 2 << 20, reserved: 0 };

static FENCES: AtomicI64 = AtomicI64::new(0);

/// Buffers made so far: (GPU VA, CPU address, bytes). Each VA is its number
/// times 16 MiB, far from any CPU address.
static BUFFERS: Mutex<Vec<(u64, usize, usize)>> = Mutex::new(Vec::new());

unsafe extern "C" fn m_alloc(_heap: u64, size: usize, _flags: u32, out: *mut OsalGpuMem) -> i32 {
    assert!(size <= 16 << 20);
    let cpu = Box::leak(vec![0u64; size.div_ceil(8)].into_boxed_slice()).as_mut_ptr().cast::<c_void>();
    let mut b = BUFFERS.lock().unwrap();
    let gpu = 0x40_0000_0000 + (b.len() as u64 + 1) * (16 << 20);
    b.push((gpu, cpu as usize, size));
    *out = OsalGpuMem { cpu, gpu, phys: 0, size, impl_: cpu };
    0
}
unsafe extern "C" fn m_free(mem: *mut OsalGpuMem) -> i32 {
    *mem = OsalGpuMem::ZERO;
    0
}
unsafe extern "C" fn m_cache(_cpu: *mut c_void, _n: usize) {}
unsafe extern "C" fn m_init(_arg: *mut c_void) -> i32 {
    0
}

unsafe extern "C" fn m_submit(kick: *const c_void, len: usize) -> i64 {
    if len == core::mem::size_of::<OsalMaliKick>() {
        let _ = *kick.cast::<OsalMaliKick>();
    }
    FENCES.fetch_add(1, Ordering::Relaxed) + 1
}
unsafe extern "C" fn m_wait(_fence: i64, _timeout_us: i64) -> i32 {
    0
}
unsafe extern "C" fn m_completed() -> i64 {
    FENCES.load(Ordering::Relaxed)
}
unsafe extern "C" fn m_query(what: u32, out: *mut c_void, sz: usize) -> i32 {
    match what {
        OSAL_QUERY_MALI_GPU if sz == core::mem::size_of::<OsalMaliGpu>() => *out.cast::<OsalMaliGpu>() = GPU,
        OSAL_QUERY_MALI_TILER_HEAP if sz == core::mem::size_of::<OsalMaliHeap>() => *out.cast::<OsalMaliHeap>() = HEAP,
        _ => return -ENOTSUP,
    }
    0
}

/// The OSAL's log hook: the library only calls it as `log("%s", line)`.
unsafe extern "C" fn m_log(_fmt: *const c_char, line: *const c_char) {
    eprintln!("osal log: {}", core::ffi::CStr::from_ptr(line).to_string_lossy());
}

static OSAL: OsalOps = OsalOps {
    init: Some(m_init),
    shutdown: None,
    gpu_alloc: Some(m_alloc),
    gpu_free: Some(m_free),
    mmu_map: None,
    map_phys: None,
    gpu_write: None,
    gpu_read: None,
    cache_flush: Some(m_cache),
    cache_invd: Some(m_cache),
    submit: Some(m_submit),
    wait: Some(m_wait),
    completed: Some(m_completed),
    reg_read: None,
    reg_write: None,
    query: Some(m_query),
    // SAFETY: see `m_log`: the same calling convention for these arguments.
    log: Some(unsafe {
        core::mem::transmute::<
            unsafe extern "C" fn(*const c_char, *const c_char),
            unsafe extern "C" fn(*const c_char, ...),
        >(m_log)
    }),
    wait_vblank: None,
    present: None,
    lock: None,
    unlock: None,
    thread_id: None,
    display_busy: None,
};

static LOCK: Mutex<()> = Mutex::new(());
static ONCE: Once = Once::new();

fn context() -> MutexGuard<'static, ()> {
    let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    ONCE.call_once(|| {
        backend::HOST_DRAW_TARGETS.store(1, Ordering::Relaxed);
        // SAFETY: the Mali table copied bit for bit; the OSAL and the copy
        // are leaked; the lock serialises the tests.
        unsafe {
            osal_set_ops(&OSAL);
            let b: YakoglBackend = core::ptr::read(&yakogl::mali::backend::yakogl_backend_mali);
            backend::g_backend = Box::leak(Box::new(b));
            assert_eq!(yakogl::mali::backend::mali_init(), 0);
            yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
            assert!(!yakogl::gl::context::gl_context_create(64, 64).is_null());
        }
    });
    // SAFETY: plain GL call.
    while unsafe { glGetError() } != GL_NO_ERROR {}
    g
}

fn ok(what: &str) {
    // SAFETY: plain GL call.
    let e = unsafe { glGetError() };
    assert_eq!(e, GL_NO_ERROR, "{what}: glGetError 0x{e:x}");
}

fn string(name: GLenum, what: &str) -> String {
    // SAFETY: a NUL-terminated static string or null.
    let p = unsafe { glGetString(name) };
    ok(what);
    assert!(!p.is_null(), "{what}: null");
    // SAFETY: as above.
    unsafe { core::ffi::CStr::from_ptr(p.cast()) }.to_str().unwrap().to_owned()
}

fn int(name: GLenum, what: &str) -> GLint {
    let mut v: GLint = i32::MIN;
    // SAFETY: room for one value.
    unsafe { glGetIntegerv(name, &mut v) };
    ok(what);
    assert_ne!(v, i32::MIN, "{what}: nothing written");
    v
}

/// The major number glmark2's `supports_gl2` takes from GL_VERSION: the
/// digits before the first '.', after the last space before it.
fn glmark2_major(version: &str) -> i32 {
    let point = version.find('.').expect("no '.' in GL_VERSION");
    let start = version[..point].rfind(' ').map_or(0, |i| i + 1);
    version[start..point].parse().expect("major number")
}

/// `GLExtensions::support`: a whole-token match in GL_EXTENSIONS.
fn supported(exts: &str, name: &str) -> bool {
    exts.split(' ').any(|t| t == name)
}

#[test]
fn strings() {
    let _g = context();
    let vendor = string(GL_VENDOR, "GL_VENDOR");
    let renderer = string(GL_RENDERER, "GL_RENDERER");
    let version = string(GL_VERSION, "GL_VERSION");
    let glsl = string(GL_SHADING_LANGUAGE_VERSION, "GL_SHADING_LANGUAGE_VERSION");
    assert!(!vendor.is_empty());
    assert!(renderer.contains("Mali"), "GL_RENDERER {renderer:?} does not name the Mali");
    assert!(version.starts_with("OpenGL ES "), "GL_VERSION {version:?}");
    assert!(glmark2_major(&version) >= 2, "glmark2 refuses GL_VERSION {version:?}");
    let (major, glsl_prefix) =
        if cfg!(feature = "report-es2") { (2, "OpenGL ES GLSL ES 1.00") } else { (3, "OpenGL ES GLSL ES 3.00") };
    assert_eq!(glmark2_major(&version), major, "{version:?}");
    assert!(glsl.starts_with(glsl_prefix), "{glsl:?}");
    // an unknown name is an error, not a string
    // SAFETY: plain GL call.
    unsafe { glGetString(0x1234) };
    // SAFETY: plain GL call.
    assert_eq!(unsafe { glGetError() }, GL_INVALID_ENUM);
}

/// Every extension glmark2-es2 looks for, with whether it is advertised
/// today (ES 3.0 report, `report-es2` report). A change flips a row and
/// fails here.
const GLMARK2_EXTENSIONS: &[(&str, bool, bool)] = &[
    // shadow and refract `supported()`: the depth-texture scenes
    ("GL_OES_depth_texture", false, true),
    ("GL_ARB_depth_texture", false, false), // desktop GL's name, the OES one is looked up first
    // canvas-generic offscreen colour format
    ("GL_ARM_rgba8", false, false),
    ("GL_OES_rgb8_rgba8", true, true),
    // canvas-generic offscreen depth format
    ("GL_OES_depth24", false, false),
    ("GL_OES_depth32", false, false),
    // gl-state-egl: glMapBufferOES / glUnmapBufferOES
    ("GL_OES_mapbuffer", true, true),
    // NPOT mip chains (refract, desktop)
    ("GL_OES_texture_npot", false, true),
];

#[test]
fn extensions_glmark2_looks_for() {
    let _g = context();
    let exts = string(GL_EXTENSIONS, "GL_EXTENSIONS");
    for &(name, es3, es2) in GLMARK2_EXTENSIONS {
        let want = if cfg!(feature = "report-es2") { es2 } else { es3 };
        assert_eq!(supported(&exts, name), want, "{name}: advertised changed (extensions: {exts})");
    }
    // a prefix is not the extension (glmark2 matches whole tokens)
    assert!(!supported(&exts, "GL_OES_depth"));
    // no token twice, none empty
    let mut seen = std::collections::BTreeSet::new();
    for t in exts.split(' ') {
        assert!(!t.is_empty(), "empty token in {exts:?}");
        assert!(seen.insert(t), "{t} listed twice");
    }
    // glGetStringi lists the same names
    let n = seen.len();
    for i in 0..n as GLuint {
        // SAFETY: plain GL call, a static NUL-terminated string.
        let p = unsafe { glGetStringi(GL_EXTENSIONS, i) };
        if p.is_null() {
            // the ES 2.0 report has no glGetStringi (a 3.0 entry point)
            assert!(cfg!(feature = "report-es2"), "glGetStringi({i}) null");
            // SAFETY: plain GL call.
            unsafe { glGetError() };
            return;
        }
        // SAFETY: as above.
        let s = unsafe { core::ffi::CStr::from_ptr(p.cast()) }.to_str().unwrap();
        assert!(seen.contains(s), "glGetStringi gave {s}, not in GL_EXTENSIONS");
    }
}

#[test]
fn implementation_limits() {
    let _g = context();
    // ES 2.0 minimums (table 6.20, "Implementation Dependent Values")
    let at_least = |name, what: &str, min: GLint| {
        let v = int(name, what);
        assert!(v >= min, "{what} = {v}, ES 2.0 minimum {min}");
        v
    };
    at_least(GL_MAX_TEXTURE_SIZE, "GL_MAX_TEXTURE_SIZE", 64);
    at_least(GL_MAX_RENDERBUFFER_SIZE, "GL_MAX_RENDERBUFFER_SIZE", 1);
    at_least(GL_MAX_CUBE_MAP_TEXTURE_SIZE, "GL_MAX_CUBE_MAP_TEXTURE_SIZE", 16);
    at_least(GL_MAX_VERTEX_ATTRIBS, "GL_MAX_VERTEX_ATTRIBS", 8);
    at_least(GL_MAX_VERTEX_UNIFORM_VECTORS, "GL_MAX_VERTEX_UNIFORM_VECTORS", 128);
    at_least(GL_MAX_FRAGMENT_UNIFORM_VECTORS, "GL_MAX_FRAGMENT_UNIFORM_VECTORS", 16);
    at_least(GL_MAX_VARYING_VECTORS, "GL_MAX_VARYING_VECTORS", 8);
    let frag = at_least(GL_MAX_TEXTURE_IMAGE_UNITS, "GL_MAX_TEXTURE_IMAGE_UNITS", 8);
    let vert = at_least(GL_MAX_VERTEX_TEXTURE_IMAGE_UNITS, "GL_MAX_VERTEX_TEXTURE_IMAGE_UNITS", 0);
    let comb = at_least(GL_MAX_COMBINED_TEXTURE_IMAGE_UNITS, "GL_MAX_COMBINED_TEXTURE_IMAGE_UNITS", 8);
    assert!(comb >= frag.max(vert), "combined {comb} < fragment {frag} / vertex {vert}");
    // glmark2's terrain scene needs vertex texture fetch
    assert!(vert > 0, "terrain: GL_MAX_VERTEX_TEXTURE_IMAGE_UNITS is {vert}");
}

#[test]
fn framebuffer_bits() {
    let _g = context();
    // the default framebuffer's bits (ES 2.0 table 6.21: any non-negative)
    let (r, g, b, a) =
        (int(GL_RED_BITS, "RED"), int(GL_GREEN_BITS, "GREEN"), int(GL_BLUE_BITS, "BLUE"), int(GL_ALPHA_BITS, "ALPHA"));
    let (d, s) = (int(GL_DEPTH_BITS, "DEPTH"), int(GL_STENCIL_BITS, "STENCIL"));
    for (n, v) in [("R", r), ("G", g), ("B", b), ("A", a), ("D", d), ("S", s)] {
        assert!((0..=32).contains(&v), "{n} bits {v}");
    }
    // glmark2 matches a visual config of 8/8/8/8 (buffer 32) or 5/6/5, with
    // depth >= 16: a colour buffer it can use and a depth buffer for the
    // depth-tested scenes.
    assert!(r >= 5 && g >= 6 && b >= 5, "colour bits {r}/{g}/{b}");
    assert!(d >= 16, "depth bits {d}");
}

#[test]
fn state_glmark2_saves_and_restores() {
    let _g = context();
    // desktop: the framebuffer binding it restores; default is 0
    assert_eq!(int(GL_FRAMEBUFFER_BINDING, "GL_FRAMEBUFFER_BINDING"), 0);
    // text renderer and jellyfish: program, buffers, blend factors
    assert_eq!(int(GL_CURRENT_PROGRAM, "GL_CURRENT_PROGRAM"), 0);
    assert_eq!(int(GL_ARRAY_BUFFER_BINDING, "GL_ARRAY_BUFFER_BINDING"), 0);
    assert_eq!(int(GL_ELEMENT_ARRAY_BUFFER_BINDING, "GL_ELEMENT_ARRAY_BUFFER_BINDING"), 0);
    assert_eq!(int(GL_BLEND_SRC_RGB, "GL_BLEND_SRC_RGB") as GLenum, GL_ONE);
    assert_eq!(int(GL_BLEND_DST_RGB, "GL_BLEND_DST_RGB") as GLenum, GL_ZERO);
    assert_eq!(int(GL_BLEND_SRC_ALPHA, "GL_BLEND_SRC_ALPHA") as GLenum, GL_ONE);
    assert_eq!(int(GL_BLEND_DST_ALPHA, "GL_BLEND_DST_ALPHA") as GLenum, GL_ZERO);
    let (mut blend, mut depth) = (7u8, 7u8);
    // SAFETY: room for one value each.
    unsafe {
        glGetBooleanv(GL_BLEND, &mut blend);
        glGetBooleanv(GL_DEPTH_TEST, &mut depth);
    }
    ok("glGetBooleanv");
    assert_eq!((blend, depth), (0, 0), "blend and depth test default off");
}
