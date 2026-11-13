// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The float colour-buffer decision of plan #360 on the `report-es2` surface
//! (item cbf-es2-test, issue #5227).
//!
//! The decision (docs/es3/sized-formats.md, "The plan #360 decision"): a
//! `report-es2` build claims neither float colour-buffer extension. ES 2.0 core
//! has no float colour target -- 4.4.3 names no sized float renderbuffer format
//! and no claimed extension adds one, so `GL_EXTENSIONS` lists neither
//! `GL_EXT_color_buffer_float` nor `GL_EXT_color_buffer_half_float`, and
//! glRenderbufferStorage answers `GL_INVALID_ENUM` for `RGBA16F` and `R32F`
//! alike (4.4.2's format rule; the claim is what makes a float renderbuffer
//! legal at all).
//!
//! Today `GL_EXT_color_buffer_half_float` is claimed (docs/notes/es3-float-targets.md
//! sections 3 and 4, option B: "both builds list" it, the ES 2.0 surface among
//! them -- `src/gl/state.rs`, the base list), pinned by tests/es2_report_guard.rs
//! `extension_list_is_exact`, and `renderbuffer_format_advertised`
//! (`src/gl/framebuffer.rs`) takes `RGBA16F` storage against that claim. So
//! `color_buffer_half_float_is_not_listed` and `rgba16f_renderbuffer_is_invalid_enum`
//! fail today and are `#[ignore]`d naming that state: leaving the claim off the
//! ES 2.0 report moves an ES 2.0 answer -- the exact list the guard pins, and the
//! dEQP-GLES2 16F rows gated on it -- and that is the guard's owner (plan #351,
//! which holds tests/es2_report_guard.rs) to decide, not this item, which changes
//! no `src/`. The two tests that hold under both answers pass and pin them here;
//! the ignored two flip when the claim leaves the ES 2.0 list.
//!
//! The `GL_EXT_color_buffer_float` arm already holds: no GPU claims it (the
//! decision's per-GPU lines), no build lists it, and the ES 2.0 report refuses
//! `R32F` storage. This file pins that arm from the extension list and the
//! storage rule. Standalone: tests/es2_*.rs belongs to another plan; the context
//! is the stand-in of tests/es2_report_guard.rs (as in tests/es3_tf_api.rs).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,report-es2 --test es3_cbf_report_es2
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online,report-es2 --test es3_cbf_report_es2
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online,report-es2 --test es3_cbf_report_es2
//! cargo test --no-default-features --features host-test,gpu-vivante,report-es2 --test es3_cbf_report_es2
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table,report-es2 --test es3_cbf_report_es2
//! ```
//!
//! Compiled only with `report-es2`: other builds see an empty file, as
//! tests/es2_report_guard.rs does.

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

/// glRenderbufferStorage of `format` on a fresh renderbuffer: its error.
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

/// Is `name` one word of GL_EXTENSIONS?
fn listed(name: &str) -> bool {
    // SAFETY: GL_EXTENSIONS is a NUL-terminated static string.
    let ext = unsafe { CStr::from_ptr(glGetString(GL_EXTENSIONS) as *const c_char) }.to_str().unwrap();
    ext.split_whitespace().any(|n| n == name)
}

/// `GL_EXT_color_buffer_float` is claimed nowhere, this build included (the
/// decision's per-GPU lines; `GL_EXT_color_buffer_float` is a constant only,
/// src/gl/consts.rs). Holds today.
#[test]
fn color_buffer_float_is_not_listed() {
    let _g = context();
    assert!(!listed("GL_EXT_color_buffer_float"));
}

/// The decision's half-float arm, failing today: `report-es2` lists
/// `GL_EXT_color_buffer_half_float` (option B, docs/notes/es3-float-targets.md
/// section 4), pinned by tests/es2_report_guard.rs `extension_list_is_exact`.
/// Turns when the ES 2.0 report leaves the claim off, if its owner decides that.
#[test]
#[ignore = "report-es2 lists GL_EXT_color_buffer_half_float today (option B; tests/es2_report_guard.rs extension_list_is_exact pins it): leaving it off the ES 2.0 report is the guard owner's decision, issue #5227"]
fn color_buffer_half_float_is_not_listed() {
    let _g = context();
    assert!(!listed("GL_EXT_color_buffer_half_float"));
}

/// ES 2.0 4.4.3 names no `R32F` renderbuffer format and no claimed extension
/// adds it: GL_INVALID_ENUM (4.4.2). Holds today -- `R32F` is not in the
/// advertised set of a `report-es2` build (src/gl/framebuffer.rs,
/// `renderbuffer_format_advertised`), as tests/es2_report_guard.rs
/// `es3_only_renderbuffer_formats_are_invalid_enum` already pins.
#[test]
fn r32f_renderbuffer_is_invalid_enum() {
    let _g = context();
    assert_eq!(renderbuffer_storage(GL_R32F), GL_INVALID_ENUM);
}

/// The same rule for `RGBA16F`, failing today: it is advertised while
/// `GL_EXT_color_buffer_half_float` is claimed (`renderbuffer_format_advertised`).
/// Turns with the claim, as `color_buffer_half_float_is_not_listed` does.
#[test]
#[ignore = "RGBA16F renderbuffer storage is accepted today because report-es2 claims GL_EXT_color_buffer_half_float (option B): it turns with the claim, issue #5227"]
fn rgba16f_renderbuffer_is_invalid_enum() {
    let _g = context();
    assert_eq!(renderbuffer_storage(GL_RGBA16F), GL_INVALID_ENUM);
}
