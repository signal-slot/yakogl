// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! A `report-es2` build claims neither float colour-buffer extension (item
//! cbf-es2-test, plan #360).
//!
//! ES 2.0 has no float colour targets: GL_EXT_color_buffer_float and
//! GL_EXT_color_buffer_half_float belong to the ES 3.0 build only, so
//! GL_EXTENSIONS must not list them and a float renderbuffer is refused with
//! the ES 2.0 error (GL_INVALID_ENUM, 4.4.3). Standalone: tests/es2_*.rs is
//! owned by another plan. The context is the stand-in of
//! tests/es2_report_guard.rs.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,report-es2 --test es3_cbf_report_es2
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online,report-es2 --test es3_cbf_report_es2
//! cargo test --no-default-features --features host-test,gpu-vivante,report-es2 --test es3_cbf_report_es2
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table,report-es2--test es3_cbf_report_es2
//! ```
//!
//! Compiled only with `report-es2`; other builds see an empty file.

#![cfg(all(feature = "host-test", feature = "report-es2"))]

use core::ffi::{c_char, c_void};
use std::ffi::CStr;
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{GlesHeap, YakoglBackend};
use yakogl::gl::consts::*;
use yakogl::gl::framebuffer::*;
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
    name: c"es3-cbf-report-es2".as_ptr() as *const c_char,
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

/// One context for the whole file; the tests take turns on it.
static LOCK: Mutex<()> = Mutex::new(());
static ONCE: Once = Once::new();

/// The shared setup: the context, current, with no error pending.
fn context() -> MutexGuard<'static, ()> {
    let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    ONCE.call_once(|| {
        // SAFETY: both tables are statics; the lock above serialises the tests.
        unsafe {
            osal_set_ops(&OSAL);
            yakogl::backend::g_backend = &BACKEND;
            #[cfg(feature = "shaderc-online")]
            {
                yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
            }
            assert!(!yakogl::gl::context::gl_context_create(8, 8).is_null());
        }
    });
    // SAFETY: a plain GL call on the one context.
    unsafe { while glGetError() != GL_NO_ERROR {} }
    g
}

fn err() -> GLenum {
    // SAFETY: a plain GL call.
    unsafe { glGetError() }
}

fn renderbuffer_storage(format: GLenum) -> GLenum {
    let mut rb = 0;
    // SAFETY: one name; plain GL calls on it.
    unsafe {
        glGenRenderbuffers(1, &mut rb);
        glBindRenderbuffer(GL_RENDERBUFFER, rb);
        glRenderbufferStorage(GL_RENDERBUFFER, format, 4, 4);
        let e = err();
        glDeleteRenderbuffers(1, &rb);
        e
    }
}

fn listed(name: &str) -> bool {
    // SAFETY: GL_EXTENSIONS is a NUL-terminated static string.
    let ext = unsafe { CStr::from_ptr(glGetString(GL_EXTENSIONS) as *const c_char) }.to_str().unwrap();
    ext.split_whitespace().any(|n| n == name)
}

#[test]
fn color_buffer_float_is_not_listed() {
    let _g = context();
    assert!(!listed("GL_EXT_color_buffer_float"));
}

/// CONFLICT, reported to the planner: report-es2 lists
/// GL_EXT_color_buffer_half_float today, on purpose (tests/es2_report_guard.rs
/// `extension_list_is_exact` pins it; the ES 2.0 flavour of that extension
/// makes RGBA16F and RGB16F renderbuffers legal). The item's rule contradicts
/// that pin; the owner of es2_report_guard decides.
#[test]
#[ignore = "report-es2 lists GL_EXT_color_buffer_half_float on purpose (es2_report_guard pins it)"]
fn color_buffer_half_float_is_not_listed() {
    let _g = context();
    assert!(!listed("GL_EXT_color_buffer_half_float"));
}

/// ES 2.0 4.4.3 has no R32F renderbuffer format: GL_INVALID_ENUM.
#[test]
fn r32f_renderbuffer_is_invalid_enum() {
    let _g = context();
    assert_eq!(renderbuffer_storage(GL_R32F), GL_INVALID_ENUM);
}

/// Accepted today because GL_EXT_color_buffer_half_float is listed (above).
#[test]
#[ignore = "RGBA16F is accepted: report-es2 lists GL_EXT_color_buffer_half_float on purpose"]
fn rgba16f_renderbuffer_is_invalid_enum() {
    let _g = context();
    assert_eq!(renderbuffer_storage(GL_RGBA16F), GL_INVALID_ENUM);
}
