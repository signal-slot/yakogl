// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The GL_EXT/OES_draw_buffers_indexed entry points (glEnableiEXT, glDisableiEXT,
//! glIsEnablediEXT, glBlendEquation[Separate]iEXT, glBlendFunc[Separate]iEXT,
//! glColorMaskiEXT and the OES twins) and the indexed queries
//! (glGetIntegeri_v of the blend pnames, glGetBooleani_v of the colour mask),
//! over `context::dbi`, at four draw targets (the HOST_DRAW_TARGETS stand-in).
//! The extension is not advertised and draws do not read the state yet.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_dbi_api
//! ```

#![cfg(all(feature = "host-test", feature = "shaderc-online", feature = "gpu-v3d", not(feature = "report-es2")))]

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
    name: c"es3-dbi-api".as_ptr() as *const c_char,
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

fn error() -> GLenum {
    // SAFETY: plain GL call.
    unsafe { glGetError() }
}
fn error_clear() {
    while error() != GL_NO_ERROR {}
}
fn geti(pname: GLenum, index: GLuint) -> (GLenum, i32) {
    error_clear();
    let mut v = i32::MIN;
    // SAFETY: one value of room.
    unsafe { glGetIntegeri_v(pname, index, &mut v) };
    (error(), v)
}
fn getb(pname: GLenum, index: GLuint) -> (GLenum, [u8; 4]) {
    error_clear();
    let mut v = [7u8; 4];
    // SAFETY: four values of room.
    unsafe { glGetBooleani_v(pname, index, v.as_mut_ptr()) };
    (error(), v)
}
fn blend(index: GLuint) -> [i32; 6] {
    [
        GL_BLEND_EQUATION_RGB,
        GL_BLEND_EQUATION_ALPHA,
        GL_BLEND_SRC_RGB,
        GL_BLEND_DST_RGB,
        GL_BLEND_SRC_ALPHA,
        GL_BLEND_DST_ALPHA,
    ]
    .map(|p| {
        let (e, v) = geti(p, index);
        assert_eq!(e, GL_NO_ERROR, "pname 0x{p:x} index {index}");
        v
    })
}
fn mask(index: GLuint) -> [u8; 4] {
    let (e, v) = getb(GL_COLOR_WRITEMASK, index);
    assert_eq!(e, GL_NO_ERROR);
    v
}
fn enabled(index: GLuint) -> bool {
    // SAFETY: plain GL call.
    unsafe { glIsEnablediEXT(GL_BLEND, index) != 0 }
}

/// Every index back at the initial state, through the non-indexed calls.
fn reset_all() {
    // SAFETY: plain GL calls.
    unsafe {
        glDisable(GL_BLEND);
        glBlendFunc(GL_ONE, GL_ZERO);
        glBlendEquation(GL_FUNC_ADD);
        glColorMask(1, 1, 1, 1);
    }
    error_clear();
}

const INITIAL: [i32; 6] =
    [GL_FUNC_ADD as i32, GL_FUNC_ADD as i32, GL_ONE as i32, GL_ZERO as i32, GL_ONE as i32, GL_ZERO as i32];

/* ---- tests ---------------------------------------------------------------------- */

#[test]
fn initial_state_at_every_index() {
    let _lib = context();
    reset_all();
    for i in 0..4 {
        assert_eq!(blend(i), INITIAL);
        assert_eq!(mask(i), [1, 1, 1, 1]);
        assert!(!enabled(i));
    }
}

#[test]
fn state_is_set_and_queried_per_index() {
    let _lib = context();
    reset_all();
    // SAFETY: plain GL calls.
    unsafe {
        glEnableiEXT(GL_BLEND, 3);
        glBlendEquationSeparateiEXT(3, GL_FUNC_SUBTRACT, GL_MAX);
        glBlendFuncSeparateiEXT(3, GL_SRC_ALPHA, GL_ONE_MINUS_SRC_ALPHA, GL_ONE, GL_DST_ALPHA);
        glColorMaskiEXT(3, 1, 0, 1, 0);
        glBlendFunciEXT(1, GL_DST_COLOR, GL_ONE);
        glBlendEquationiEXT(1, GL_FUNC_REVERSE_SUBTRACT);
        glColorMaskiOES(2, 0, 0, 0, 0);
        glEnableiOES(GL_BLEND, 1);
        glDisableiOES(GL_BLEND, 1);
        glEnableiOES(GL_BLEND, 2);
    }
    assert_eq!(error(), GL_NO_ERROR);
    assert_eq!(
        blend(3),
        [
            GL_FUNC_SUBTRACT as i32,
            GL_MAX as i32,
            GL_SRC_ALPHA as i32,
            GL_ONE_MINUS_SRC_ALPHA as i32,
            GL_ONE as i32,
            GL_DST_ALPHA as i32
        ]
    );
    assert_eq!(mask(3), [1, 0, 1, 0]);
    assert_eq!(
        blend(1),
        [
            GL_FUNC_REVERSE_SUBTRACT as i32,
            GL_FUNC_REVERSE_SUBTRACT as i32,
            GL_DST_COLOR as i32,
            GL_ONE as i32,
            GL_DST_COLOR as i32,
            GL_ONE as i32
        ]
    );
    assert_eq!(mask(2), [0, 0, 0, 0]);
    assert_eq!([enabled(0), enabled(1), enabled(2), enabled(3)], [false, false, true, true]);
    /* untouched index 0 */
    assert_eq!(blend(0), INITIAL);
    assert_eq!(mask(0), [1, 1, 1, 1]);
    reset_all();
}

#[test]
fn non_indexed_calls_reset_every_index() {
    let _lib = context();
    reset_all();
    // SAFETY: plain GL calls.
    unsafe {
        for i in 0..4 {
            glEnableiEXT(GL_BLEND, i);
            glBlendFuncSeparateiEXT(i, GL_ZERO, GL_ONE, GL_ONE, GL_ZERO);
            glBlendEquationiEXT(i, GL_MIN);
            glColorMaskiEXT(i, 0, 1, 0, 1);
        }
        glDisable(GL_BLEND);
        glBlendFunc(GL_SRC_COLOR, GL_ONE_MINUS_SRC_COLOR);
        glBlendEquationSeparate(GL_FUNC_ADD, GL_FUNC_SUBTRACT);
        glColorMask(1, 1, 0, 0);
    }
    assert_eq!(error(), GL_NO_ERROR);
    let want = [
        GL_FUNC_ADD as i32,
        GL_FUNC_SUBTRACT as i32,
        GL_SRC_COLOR as i32,
        GL_ONE_MINUS_SRC_COLOR as i32,
        GL_SRC_COLOR as i32,
        GL_ONE_MINUS_SRC_COLOR as i32,
    ];
    for i in 0..4 {
        assert_eq!(blend(i), want, "index {i}");
        assert_eq!(mask(i), [1, 1, 0, 0]);
        assert!(!enabled(i));
    }
    // SAFETY: plain GL call.
    unsafe { glEnable(GL_BLEND) };
    assert!((0..4).all(enabled));
    // the non-indexed queries still answer
    let mut v = 0;
    // SAFETY: one value of room.
    unsafe { glGetIntegerv(GL_BLEND_SRC_RGB, &mut v) };
    assert_eq!(v, GL_SRC_COLOR as i32);
    reset_all();
}

#[test]
fn index_at_or_past_draw_targets_is_invalid_value() {
    let _lib = context();
    reset_all();
    for idx in [4u32, 5, u32::MAX] {
        // SAFETY: plain GL calls.
        unsafe {
            glEnableiEXT(GL_BLEND, idx);
            assert_eq!(error(), GL_INVALID_VALUE);
            glDisableiEXT(GL_BLEND, idx);
            assert_eq!(error(), GL_INVALID_VALUE);
            assert_eq!(glIsEnablediEXT(GL_BLEND, idx), 0);
            assert_eq!(error(), GL_INVALID_VALUE);
            glBlendEquationiEXT(idx, GL_FUNC_ADD);
            assert_eq!(error(), GL_INVALID_VALUE);
            glBlendEquationSeparateiEXT(idx, GL_FUNC_ADD, GL_FUNC_ADD);
            assert_eq!(error(), GL_INVALID_VALUE);
            glBlendFunciEXT(idx, GL_ONE, GL_ONE);
            assert_eq!(error(), GL_INVALID_VALUE);
            glBlendFuncSeparateiEXT(idx, GL_ONE, GL_ONE, GL_ONE, GL_ONE);
            assert_eq!(error(), GL_INVALID_VALUE);
            glColorMaskiEXT(idx, 0, 0, 0, 0);
            assert_eq!(error(), GL_INVALID_VALUE);
        }
        assert_eq!(geti(GL_BLEND_SRC_RGB, idx), (GL_INVALID_VALUE, i32::MIN));
        assert_eq!(getb(GL_COLOR_WRITEMASK, idx), (GL_INVALID_VALUE, [7; 4]));
    }
    /* nothing changed */
    assert_eq!(blend(3), INITIAL);
}

#[test]
fn other_caps_and_bad_enums_are_invalid_enum() {
    let _lib = context();
    reset_all();
    // SAFETY: plain GL calls.
    unsafe {
        glEnableiEXT(GL_DEPTH_TEST, 0);
        assert_eq!(error(), GL_INVALID_ENUM);
        glDisableiEXT(GL_SCISSOR_TEST, 0);
        assert_eq!(error(), GL_INVALID_ENUM);
        assert_eq!(glIsEnablediEXT(GL_CULL_FACE, 0), 0);
        assert_eq!(error(), GL_INVALID_ENUM);
        glBlendFunciEXT(0, 0x1234, GL_ONE);
        assert_eq!(error(), GL_INVALID_ENUM);
        glBlendEquationiEXT(0, 0x1234);
        assert_eq!(error(), GL_INVALID_ENUM);
    }
    assert_eq!(geti(GL_BLEND, 0).0, GL_INVALID_ENUM);
    assert_eq!(geti(GL_COLOR_WRITEMASK, 0).0, GL_INVALID_ENUM);
    assert_eq!(geti(GL_DEPTH_FUNC, 0).0, GL_INVALID_ENUM);
    assert_eq!(getb(GL_BLEND, 0).0, GL_INVALID_ENUM);
    assert_eq!(blend(0), INITIAL);
}

#[test]
fn gl_extensions_is_unchanged() {
    let _lib = context();
    // SAFETY: NUL-terminated static.
    let s =
        unsafe { std::ffi::CStr::from_ptr(glGetString(GL_EXTENSIONS) as *const c_char) }.to_string_lossy().into_owned();
    assert!(!s.contains("draw_buffers_indexed"), "{s}");
}
