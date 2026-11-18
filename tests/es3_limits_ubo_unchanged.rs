// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The uniform answers of the GPUs whose uniform-block limits the V3D raise
//! did not move: PowerVR, Mali and GC7000UL (Vivante) keep, under ES 3.0,
//! `GL_MAX_UNIFORM_BLOCK_SIZE` = `MAX_UNIFORM_DWORDS * 4` = 6144,
//! `GL_MAX_COMBINED_{VERTEX,FRAGMENT}_UNIFORM_COMPONENTS` = 1536 and
//! `GL_MAX_{VERTEX,FRAGMENT}_UNIFORM_COMPONENTS` = 1536. Each pname is read
//! through `glGetIntegerv`, `glGetFloatv` and `glGetInteger64v`, which must
//! agree. The V3D values are pinned in `tests/es3_limits_v3d.rs`.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test es3_limits_ubo_unchanged
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online --test es3_limits_ubo_unchanged
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test es3_limits_ubo_unchanged
//! ```

#![cfg(all(
    feature = "host-test",
    any(feature = "gpu-powervr", feature = "gpu-mali", feature = "gpu-vivante"),
    not(feature = "report-es2")
))]

use core::ffi::{c_char, c_void};
use std::sync::Mutex;
use std::sync::MutexGuard;
use std::sync::Once;
use yakogl::backend::{GlesHeap, YakoglBackend};
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
    name: c"es3-limits-ubo-unchanged".as_ptr() as *const c_char,
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
            assert!(!yakogl::gl::context::gl_context_create(8, 8).is_null());
        }
    });
    guard
}

/* ---- the pnames and their unchanged values -------------------------------- */

const PNAMES: [(&str, GLenum, i64); 5] = [
    ("MAX_UNIFORM_BLOCK_SIZE", GL_MAX_UNIFORM_BLOCK_SIZE, 6144),
    ("MAX_COMBINED_VERTEX_UNIFORM_COMPONENTS", GL_MAX_COMBINED_VERTEX_UNIFORM_COMPONENTS, 1536),
    ("MAX_COMBINED_FRAGMENT_UNIFORM_COMPONENTS", GL_MAX_COMBINED_FRAGMENT_UNIFORM_COMPONENTS, 1536),
    ("MAX_VERTEX_UNIFORM_COMPONENTS", GL_MAX_VERTEX_UNIFORM_COMPONENTS, 1536),
    ("MAX_FRAGMENT_UNIFORM_COMPONENTS", GL_MAX_FRAGMENT_UNIFORM_COMPONENTS, 1536),
];

fn error_clear() {
    // SAFETY: plain GL call.
    unsafe { while glGetError() != GL_NO_ERROR {} }
}

#[test]
fn uniform_limits_unchanged_glgetintegerv() {
    let _g = context();
    for (name, pname, want) in PNAMES {
        error_clear();
        let mut v = i32::MIN;
        // SAFETY: one value.
        unsafe { glGetIntegerv(pname, &mut v) };
        assert_eq!(i64::from(v), want, "glGetIntegerv {name}");
        // SAFETY: plain GL call.
        assert_eq!(unsafe { glGetError() }, GL_NO_ERROR, "{name}");
    }
}

#[test]
fn uniform_limits_unchanged_glgetfloatv() {
    let _g = context();
    for (name, pname, want) in PNAMES {
        error_clear();
        let mut v = f32::NAN;
        // SAFETY: one value.
        unsafe { glGetFloatv(pname, &mut v) };
        assert_eq!(v, want as f32, "glGetFloatv {name}");
        // SAFETY: plain GL call.
        assert_eq!(unsafe { glGetError() }, GL_NO_ERROR, "{name}");
    }
}

#[test]
fn uniform_limits_unchanged_glgetinteger64v() {
    let _g = context();
    for (name, pname, want) in PNAMES {
        error_clear();
        let mut v = i64::MIN;
        // SAFETY: one value.
        unsafe { glGetInteger64v(pname, &mut v) };
        assert_eq!(v, want, "glGetInteger64v {name}");
        // SAFETY: plain GL call.
        assert_eq!(unsafe { glGetError() }, GL_NO_ERROR, "{name}");
    }
}
