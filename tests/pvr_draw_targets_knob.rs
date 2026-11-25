// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! `pvr_debug_set_draw_targets`, the setter driver/samples/zephyr/deqp calls
//! under CONFIG_PVR_DEQP_MRT4 (docs/notes/mrt4-deqp-cases.md): default 3,
//! set 4 answered by GL_MAX_DRAW_BUFFERS and GL_MAX_COLOR_ATTACHMENTS,
//! previous count returned, 0 restores, out of range refused.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,test-knobs --test pvr_draw_targets_knob
//! ```

#![cfg(all(feature = "host-test", feature = "gpu-powervr", feature = "test-knobs"))]

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
    name: c"pvr-draw-targets-knob".as_ptr() as *const c_char,
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
fn the_knob_sets_four_and_restores_three() {
    // SAFETY: statics; this file has one test, so nothing else calls the library.
    unsafe {
        osal_set_ops(&OSAL);
        backend::g_backend = &BACKEND;
        assert!(!yakogl::gl::context::gl_context_create(8, 8).is_null());
    }
    use yakogl::pvr::debug::pvr_debug_set_draw_targets as set;
    assert_eq!(backend::draw_targets(), 3);
    assert_eq!(geti(GL_MAX_DRAW_BUFFERS), 3);
    assert_eq!(geti(GL_MAX_COLOR_ATTACHMENTS), 3);

    assert_eq!(set(4), 3, "returns the previous count");
    assert_eq!(backend::draw_targets(), 4);
    assert_eq!(geti(GL_MAX_DRAW_BUFFERS), 4);
    assert_eq!(geti(GL_MAX_COLOR_ATTACHMENTS), 4);

    assert_eq!(set(5), 4, "out of range is refused");
    assert_eq!(set(-1), 4);
    assert_eq!(geti(GL_MAX_DRAW_BUFFERS), 4);

    assert_eq!(set(0), 4, "0 restores the default");
    assert_eq!(backend::draw_targets(), 3);
    assert_eq!(geti(GL_MAX_DRAW_BUFFERS), 3);
    assert_eq!(geti(GL_MAX_COLOR_ATTACHMENTS), 3);
}
