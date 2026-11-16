// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The V3D under `report-es2` keeps its ES 2.0 uniform answers.
//!
//! The V3D's uniform-block limits (16384-byte blocks, a 50688-dword stage
//! store) are cfg-gated in `src/gl/context.rs` on
//! `all(feature = "gpu-v3d", not(feature = "report-es2"))`;
//! tests/es2_report_guard.rs guards the ES 2.0 report on the PowerVR only.
//! This file pins, for the V3D with `report-es2`: 384 default-block vectors
//! per stage, the block-size and combined-component queries refused as
//! ES 3.0-only (GL_INVALID_ENUM, output untouched; src/gl/state.rs
//! `es3_only_pname`), and the two context constants at their ES 2.0 values.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table,report-es2 --test es2_report_v3d_uniforms
//! ```

#![cfg(all(feature = "host-test", feature = "gpu-v3d", feature = "report-es2", feature = "shaderc-online"))]

use core::ffi::{c_char, c_void};
use std::sync::Once;
use yakogl::backend::{GlesHeap, YakoglBackend};
use yakogl::gl::consts::*;
use yakogl::gl::state::*;
use yakogl::gl::types::*;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};

/* ---- a context on the host: the smallest OSAL and back end that make one --- */

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
unsafe extern "C" fn b_ok() -> i32 {
    0
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
    name: c"es2-report-v3d-uniforms".as_ptr() as *const c_char,
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
    flush: Some(b_ok),
    finish: Some(b_ok),
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
};

static ONCE: Once = Once::new();

/// The context, current, with no error pending.
fn context() {
    ONCE.call_once(|| {
        // SAFETY: both tables are statics; the one test below runs alone.
        unsafe {
            osal_set_ops(&OSAL);
            yakogl::backend::g_backend = &BACKEND;
            yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
            assert!(!yakogl::gl::context::gl_context_create(8, 8).is_null());
        }
    });
    // SAFETY: a plain GL call on the one context.
    unsafe { while glGetError() != GL_NO_ERROR {} }
}

/// glGetIntegerv: the first value, or the error it raised (the output must
/// then be untouched).
fn geti(pname: GLenum) -> Result<GLint, GLenum> {
    let mut v = [0x5A5A; 4];
    // SAFETY: room for four values, the most any pname yields.
    unsafe { glGetIntegerv(pname, v.as_mut_ptr()) };
    // SAFETY: a plain GL call.
    match unsafe { glGetError() } {
        GL_NO_ERROR => Ok(v[0]),
        e => {
            assert_eq!(v, [0x5A5A; 4], "glGetIntegerv(0x{pname:x}) wrote while failing");
            Err(e)
        }
    }
}

#[test]
fn v3d_keeps_its_es2_uniform_answers() {
    context();
    assert_eq!(geti(GL_MAX_VERTEX_UNIFORM_VECTORS), Ok(384));
    assert_eq!(geti(GL_MAX_FRAGMENT_UNIFORM_VECTORS), Ok(384));
    for (n, p) in [
        ("GL_MAX_UNIFORM_BLOCK_SIZE", GL_MAX_UNIFORM_BLOCK_SIZE),
        ("GL_MAX_COMBINED_VERTEX_UNIFORM_COMPONENTS", GL_MAX_COMBINED_VERTEX_UNIFORM_COMPONENTS),
        ("GL_MAX_COMBINED_FRAGMENT_UNIFORM_COMPONENTS", GL_MAX_COMBINED_FRAGMENT_UNIFORM_COMPONENTS),
    ] {
        assert_eq!(geti(p), Err(GL_INVALID_ENUM), "{n}");
    }
    assert_eq!(yakogl::gl::context::MAX_UNIFORM_BLOCK_SIZE, 6144);
    assert_eq!(yakogl::gl::context::MAX_STAGE_UNIFORM_DWORDS, 1536);
}
