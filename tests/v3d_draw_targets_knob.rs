// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! `v3d_debug_set_draw_targets` (src/v3d/debug.rs), the setter the Pi 4
//! gl_test image calls under CONFIG_V3D_GL_TEST_MRT4 and the deqp image under
//! CONFIG_V3D_DEQP_MRT4 (docs/notes/mrt4-v3d-pvr.md section 3), the V3D twin
//! of tests/pvr_draw_targets_knob.rs: the default is the build's own count
//! (`backend::DRAW_TARGETS`: 3 on the V3D, ES 3.0 and ES 2.0 report alike),
//! a set count is answered by GL_MAX_DRAW_BUFFERS and
//! GL_MAX_COLOR_ATTACHMENTS, the previous count is returned, 0 restores, out
//! of range is refused.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table,test-knobs --test v3d_draw_targets_knob
//! ```

#![cfg(all(feature = "host-test", feature = "gpu-v3d", feature = "test-knobs"))]

use core::ffi::{c_char, c_void};
use yakogl::backend::{self, GlesHeap, YakoglBackend};
use yakogl::gl::consts::*;
use yakogl::gl::state::*;
use yakogl::gl::types::*;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};

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
    name: c"v3d-draw-targets-knob".as_ptr() as *const c_char,
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

fn geti(pname: GLenum) -> GLint {
    let mut v: GLint = -777;
    // SAFETY: plain GL calls; one GLint out.
    unsafe {
        while glGetError() != GL_NO_ERROR {}
        glGetIntegerv(pname, &mut v);
        assert_eq!(glGetError(), GL_NO_ERROR);
    }
    v
}

#[test]
fn the_knob_sets_a_count_and_restores_the_builds() {
    // SAFETY: statics; this file has one test, so nothing else calls the library.
    unsafe {
        osal_set_ops(&OSAL);
        backend::g_backend = &BACKEND;
        assert!(!yakogl::gl::context::gl_context_create(8, 8).is_null());
    }
    use yakogl::v3d::debug::v3d_debug_set_draw_targets as set;
    /* the build's own count: three on the V3D in either report
     * (`backend::DRAW_TARGETS`) */
    let own = backend::DRAW_TARGETS as i32;
    assert_eq!(own, 3);
    assert_eq!(backend::draw_targets() as i32, own);
    assert_eq!(geti(GL_MAX_COLOR_ATTACHMENTS), own);

    /* each count is answered, and the call returns the one before it */
    let mut was = own;
    for n in [4, 1, 4] {
        assert_eq!(set(n), was, "returns the previous count");
        assert_eq!(backend::draw_targets() as i32, n);
        assert_eq!(geti(GL_MAX_COLOR_ATTACHMENTS), n);
        if !cfg!(feature = "report-es2") {
            assert_eq!(geti(GL_MAX_DRAW_BUFFERS), n);
        }
        was = n;
    }

    assert_eq!(set(5), 4, "out of range is refused");
    assert_eq!(set(-1), 4);
    assert_eq!(geti(GL_MAX_COLOR_ATTACHMENTS), 4);

    assert_eq!(set(0), 4, "0 restores the default");
    assert_eq!(backend::draw_targets() as i32, own);
    assert_eq!(geti(GL_MAX_COLOR_ATTACHMENTS), own);
}
