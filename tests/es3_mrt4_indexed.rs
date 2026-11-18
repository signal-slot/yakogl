// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GL_EXT_draw_buffers_indexed / GL_OES_draw_buffers_indexed at index 3 under
//! a four-target stand-in (`backend::HOST_DRAW_TARGETS` 4, what
//! `testutil::backend_draws_targets(4)` sets), on the V3D and PowerVR builds:
//! what the GL core answers today. These are the 76
//! `functional.draw_buffers_indexed.*` NotSupported rows of RC14
//! (docs/es3-cts-failures.md).
//!
//! Today: neither extension name is in GL_EXTENSIONS, the entry points
//! (`glEnableiEXT`, `glDisableiEXT`, `glIsEnablediEXT`, `glBlendEquationiEXT`,
//! `glBlendEquationSeparateiEXT`, `glBlendFunciEXT`,
//! `glBlendFuncSeparateiEXT`, `glColorMaskiEXT`) do not exist in `src/gl`
//! (so no Rust test can call them: the file pins their absence by reading the
//! sources), and the indexed state queries `glGetIntegeri_v` of the blend and
//! colour-mask pnames answer GL_INVALID_ENUM at index 0..3 and 4 alike.
//! The stand-in does reach the core: GL_MAX_DRAW_BUFFERS is 4.
//!
//! The `#[ignore]`d tests are the expected behaviour once Draft 1 of
//! docs/notes/v3d-draw-buffers-indexed.md (owner: the GL core) lands: the
//! extension is listed, the indexed queries answer for 0..3 and give
//! GL_INVALID_VALUE for index 4. Per-index calls are tested through the
//! queries only, until the entry points exist and can be named here.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_mrt4_indexed
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test es3_mrt4_indexed
//! ```

#![cfg(all(
    feature = "host-test",
    feature = "shaderc-online",
    any(feature = "gpu-v3d", feature = "gpu-powervr"),
    not(feature = "report-es2")
))]

use core::ffi::{c_char, c_void};
use core::sync::atomic::Ordering;
use std::sync::Mutex;
use std::sync::MutexGuard;
use std::sync::Once;
use yakogl::backend::{GlesHeap, YakoglBackend, HOST_DRAW_TARGETS};
use yakogl::gl::consts::*;
use yakogl::gl::state::*;
use yakogl::gl::types::*;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};

/* ---- a context on the host: the smallest OSAL and back end that make one -- */

unsafe extern "C" fn m_gpu_alloc(_heap: u64, size: usize, _flags: u32, out: *mut OsalGpuMem) -> i32 {
    let mem = Box::leak(vec![0u8; size].into_boxed_slice());
    let p = mem.as_mut_ptr();
    *out = OsalGpuMem { cpu: p as *mut c_void, gpu: p as u64, phys: p as u64, size, impl_: p as *mut c_void };
    0
}
unsafe extern "C" fn m_gpu_free(_mem: *mut OsalGpuMem) -> i32 {
    0
}
unsafe extern "C" fn m_cache(_cpu: *mut c_void, _n: usize) {}
unsafe extern "C" fn b_init() -> i32 {
    0
}
unsafe extern "C" fn b_heap_base(_heap: GlesHeap) -> u64 {
    0x1000_0000
}

static OSAL: OsalOps = OsalOps {
    init: None,
    shutdown: None,
    gpu_alloc: Some(m_gpu_alloc),
    gpu_free: Some(m_gpu_free),
    mmu_map: None,
    map_phys: None,
    gpu_write: None,
    gpu_read: None,
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
    present: None,
    lock: None,
    unlock: None,
    thread_id: None,
    display_busy: None,
};

static BACKEND: YakoglBackend = YakoglBackend {
    name: c"es3-mrt4-indexed".as_ptr() as *const c_char,
    native_attrib_formats: 0,
    rt_rgba8888_is_bgra: false,
    mip_level_align: 0,
    compressed_blocks_in_rows: false,
    native_layered_textures: false,
    front_facing_shader_variant: false,
    etc1_decoded_at_upload: false,
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
    linear_row_align: 0,
    uniforms_host_only: false,
    ds_write: None,
    stencil_resolve: None,
    max_samples: 0,
};

static LIBRARY: Mutex<()> = Mutex::new(());
static CONTEXT: Once = Once::new();

/// The context, made once; the guard keeps the tests of this file off the
/// library (single-threaded by contract) one at a time.
fn context() -> MutexGuard<'static, ()> {
    let guard = LIBRARY.lock().unwrap_or_else(|e| e.into_inner());
    CONTEXT.call_once(|| {
        // SAFETY: both tables are statics, and the guard above is held.
        unsafe {
            osal_set_ops(&OSAL);
            yakogl::backend::g_backend = &BACKEND;
            /* the four-target stand-in, set before the context is made (what
             * testutil::backend_draws_targets(4) does) */
            HOST_DRAW_TARGETS.store(4, Ordering::Relaxed);
            assert!(!yakogl::gl::context::gl_context_create(8, 8).is_null());
        }
    });
    guard
}

/* ---- helpers ---------------------------------------------------------------- */

fn error_clear() {
    // SAFETY: plain GL call.
    unsafe { while glGetError() != GL_NO_ERROR {} }
}

fn extensions() -> Vec<String> {
    let mut out = Vec::new();
    let mut n = 0;
    // SAFETY: plain GL call, one value of room.
    unsafe { glGetIntegerv(GL_NUM_EXTENSIONS, &mut n) };
    for i in 0..n {
        // SAFETY: in-range, NUL-terminated static.
        let s = unsafe { std::ffi::CStr::from_ptr(glGetStringi(GL_EXTENSIONS, i as GLuint) as *const c_char) };
        out.push(s.to_string_lossy().into_owned());
    }
    out
}

/// The indexed pnames the extension adds, as glGetIntegeri_v targets.
const INDEXED_PNAMES: [(&str, GLenum); 6] = [
    ("GL_BLEND_EQUATION_RGB", GL_BLEND_EQUATION_RGB),
    ("GL_BLEND_EQUATION_ALPHA", GL_BLEND_EQUATION_ALPHA),
    ("GL_BLEND_SRC_RGB", GL_BLEND_SRC_RGB),
    ("GL_BLEND_DST_RGB", GL_BLEND_DST_RGB),
    ("GL_BLEND_SRC_ALPHA", GL_BLEND_SRC_ALPHA),
    ("GL_BLEND_DST_ALPHA", GL_BLEND_DST_ALPHA),
];

/// glGetIntegeri_v: the error and the value (the sentinel when not written).
fn integeri(pname: GLenum, index: GLuint) -> (GLenum, i32) {
    error_clear();
    let mut v = i32::MIN;
    // SAFETY: one value of room.
    unsafe { glGetIntegeri_v(pname, index, &mut v) };
    // SAFETY: plain GL call.
    (unsafe { glGetError() }, v)
}

/* ---- today ------------------------------------------------------------------ */

/// The stand-in reaches the core: four draw buffers, so index 3 is in range
/// once the extension exists (and index 4 is the first out of range).
#[test]
fn the_stand_in_gives_four_draw_buffers() {
    let _lib = context();
    let mut v = i32::MIN;
    // SAFETY: one value of room.
    unsafe { glGetIntegerv(GL_MAX_DRAW_BUFFERS, &mut v) };
    assert_eq!(v, 4);
}

/// Neither name is advertised, in the string or the list.
#[test]
fn neither_indexed_extension_is_advertised() {
    let _lib = context();
    error_clear();
    // SAFETY: NUL-terminated static.
    let s =
        unsafe { std::ffi::CStr::from_ptr(glGetString(GL_EXTENSIONS) as *const c_char) }.to_string_lossy().into_owned();
    assert!(!s.contains("draw_buffers_indexed"), "in GL_EXTENSIONS: re-pin: {s}");
    assert!(!extensions().iter().any(|e| e.contains("draw_buffers_indexed")), "in the list: re-pin");
}

/// The entry points are missing from the GL core (the sources are read, since
/// a call to a missing function cannot link). The plain ones stay.
#[test]
fn the_per_target_entry_points_do_not_exist() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/gl");
    let mut text = String::new();
    for e in std::fs::read_dir(dir).unwrap() {
        let p = e.unwrap().path();
        if p.extension().is_some_and(|x| x == "rs") {
            text += &std::fs::read_to_string(p).unwrap();
        }
    }
    for name in [
        "glEnableiEXT",
        "glDisableiEXT",
        "glIsEnablediEXT",
        "glBlendEquationiEXT",
        "glBlendEquationSeparateiEXT",
        "glBlendFunciEXT",
        "glBlendFuncSeparateiEXT",
        "glColorMaskiEXT",
        "glEnableiOES",
        "glColorMaskiOES",
    ] {
        assert!(!text.contains(name), "{name} exists now: call it from this file and re-pin");
    }
    for name in ["fn glEnable(", "fn glBlendFunc(", "fn glBlendEquation(", "fn glColorMask(", "fn glIsEnabled("] {
        assert!(text.contains(name), "{name} is gone");
    }
}

/// The indexed blend queries are not indexed state: GL_INVALID_ENUM at index
/// 0..3 and at 4, and nothing written.
#[test]
fn the_indexed_blend_queries_are_invalid_enum_at_every_index() {
    let _lib = context();
    let extra = [("GL_COLOR_WRITEMASK", GL_COLOR_WRITEMASK), ("GL_BLEND", GL_BLEND)];
    for (what, pname) in INDEXED_PNAMES.iter().chain(extra.iter()) {
        for index in 0..=4 {
            let (err, v) = integeri(*pname, index);
            assert_eq!(err, GL_INVALID_ENUM, "glGetIntegeri_v({what}, {index})");
            assert_eq!(v, i32::MIN, "glGetIntegeri_v({what}, {index}) wrote despite the error");
        }
    }
}

/// The plain calls the indexed ones would mirror: one blend state for all
/// targets, no error at the four-target count.
#[test]
fn the_plain_blend_state_is_one_state_for_all_targets() {
    let _lib = context();
    error_clear();
    // SAFETY: plain GL calls.
    unsafe {
        glEnable(GL_BLEND);
        assert_eq!(glIsEnabled(GL_BLEND), 1);
        glBlendFunc(GL_ONE, GL_ZERO);
        glBlendEquation(GL_FUNC_ADD);
        glColorMask(1, 0, 1, 0);
        assert_eq!(glGetError(), GL_NO_ERROR);
        glColorMask(1, 1, 1, 1);
        glDisable(GL_BLEND);
        assert_eq!(glIsEnabled(GL_BLEND), 0);
    }
}

/* ---- ES 3.0 + GL_EXT_draw_buffers_indexed, ignored until Draft 1 ------------- */

/// Owner: docs/notes/v3d-draw-buffers-indexed.md Draft 1 (5): the names are
/// listed once the back end takes the per-target array.
#[test]
#[ignore = "docs/notes/v3d-draw-buffers-indexed.md Draft 1: the extension is not listed"]
fn expected_the_extension_is_advertised() {
    let _lib = context();
    let all = extensions();
    assert!(all.iter().any(|e| e == "GL_EXT_draw_buffers_indexed"), "{all:?}");
    assert!(all.iter().any(|e| e == "GL_OES_draw_buffers_indexed"), "{all:?}");
}

/// Draft 1 (3): the indexed queries answer for 0..3 (blend defaults: ADD, ONE,
/// ZERO) and give GL_INVALID_VALUE for index 4.
#[test]
#[ignore = "docs/notes/v3d-draw-buffers-indexed.md Draft 1: glGetIntegeri_v of blend state is GL_INVALID_ENUM"]
fn expected_indexed_blend_queries_answer_at_three_and_refuse_four() {
    let _lib = context();
    let defaults = [GL_FUNC_ADD, GL_FUNC_ADD, GL_ONE, GL_ZERO, GL_ONE, GL_ZERO];
    for ((what, pname), want) in INDEXED_PNAMES.iter().zip(defaults) {
        for index in 0..4 {
            assert_eq!(integeri(*pname, index), (GL_NO_ERROR, want as i32), "{what} at {index}");
        }
        let (err, v) = integeri(*pname, 4);
        assert_eq!((err, v), (GL_INVALID_VALUE, i32::MIN), "{what} at 4");
    }
}
