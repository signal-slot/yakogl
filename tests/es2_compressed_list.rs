// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GL_NUM_COMPRESSED_TEXTURE_FORMATS / GL_COMPRESSED_TEXTURE_FORMATS under
//! `report-es2` and under ES 3.0 (plan #354, item state-values-16).
//!
//! Commit 455c3bb5 grew the list to the ten ES 3.0 ETC2/EAC names. The ES 2.0
//! context must answer what main answered at 6f9e52d4: three formats, in the
//! order ETC1, RGB8_ETC2, RGBA8_ETC2_EAC (the three compressed-texture
//! extensions GL_EXTENSIONS claims; no sRGB, no EAC R11/RG11, no punch-through
//! alpha in an ES 2.0 report). tests/es2_report_guard.rs pins the same on the
//! PowerVR only; this file pins it on all three GPU builds. Without
//! `report-es2` the list holds the ten names (ES 3.0 table 6.14 asks for
//! them), in any order, and at least ten entries; ETC1 may come on top.
//!
//! Same stand-in context as tests/es2_report_limits.rs.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table,report-es2 --test es2_compressed_list
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online,report-es2 --test es2_compressed_list
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online,report-es2 --test es2_compressed_list
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es2_compressed_list
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test es2_compressed_list
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online --test es2_compressed_list
//! (no Mali build: it samples no ETC2 class and keeps its three names)
//! ```

#![cfg(all(feature = "host-test", feature = "shaderc-online", not(feature = "gpu-mali")))]

use core::ffi::{c_char, c_void};
use std::sync::{Mutex, MutexGuard, Once};
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
    name: c"es2-compressed-list".as_ptr() as *const c_char,
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
    max_samples: 0,
};

/// One context for the whole file; the tests take turns on it.
static LOCK: Mutex<()> = Mutex::new(());
static ONCE: Once = Once::new();

fn context() -> MutexGuard<'static, ()> {
    let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    ONCE.call_once(|| {
        // SAFETY: both tables are statics; the lock above serialises the tests.
        unsafe {
            osal_set_ops(&OSAL);
            yakogl::backend::g_backend = &BACKEND;
            yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
            assert!(!yakogl::gl::context::gl_context_create(8, 8).is_null());
        }
    });
    // SAFETY: a plain GL call on the one context.
    unsafe { while glGetError() != GL_NO_ERROR {} }
    g
}

/// (count, list) as the context answers; the list is read with room for 32.
fn answer() -> (GLint, Vec<GLenum>) {
    let mut n: GLint = -1;
    let mut v = [0 as GLint; 32];
    // SAFETY: room for 32 values, far above any list this build has.
    unsafe {
        glGetIntegerv(GL_NUM_COMPRESSED_TEXTURE_FORMATS, &mut n);
        assert_eq!(glGetError(), GL_NO_ERROR);
        glGetIntegerv(GL_COMPRESSED_TEXTURE_FORMATS, v.as_mut_ptr());
        assert_eq!(glGetError(), GL_NO_ERROR);
    }
    (n, v[..n as usize].iter().map(|&f| f as GLenum).collect())
}

/// The answer of 6f9e52d4 under `report-es2`: the three extension formats.
#[cfg(feature = "report-es2")]
#[test]
fn es2_list_is_the_three_extension_formats() {
    let _g = context();
    let (n, list) = answer();
    assert_eq!(n, 3, "GL_NUM_COMPRESSED_TEXTURE_FORMATS");
    assert_eq!(
        list,
        [GL_ETC1_RGB8_OES, GL_COMPRESSED_RGB8_ETC2, GL_COMPRESSED_RGBA8_ETC2_EAC],
        "GL_COMPRESSED_TEXTURE_FORMATS"
    );
}

/// The ten ES 3.0 names, and a count of at least ten, without `report-es2`.
#[cfg(not(feature = "report-es2"))]
#[test]
fn es3_list_has_the_ten_etc2_eac_names() {
    let _g = context();
    let (n, list) = answer();
    assert!(n >= 10, "GL_NUM_COMPRESSED_TEXTURE_FORMATS = {n}");
    for want in [
        GL_COMPRESSED_R11_EAC,
        GL_COMPRESSED_SIGNED_R11_EAC,
        GL_COMPRESSED_RG11_EAC,
        GL_COMPRESSED_SIGNED_RG11_EAC,
        GL_COMPRESSED_RGB8_ETC2,
        GL_COMPRESSED_SRGB8_ETC2,
        GL_COMPRESSED_RGB8_PUNCHTHROUGH_ALPHA1_ETC2,
        GL_COMPRESSED_SRGB8_PUNCHTHROUGH_ALPHA1_ETC2,
        GL_COMPRESSED_RGBA8_ETC2_EAC,
        GL_COMPRESSED_SRGB8_ALPHA8_ETC2_EAC,
    ] {
        assert!(list.contains(&want), "0x{want:x} missing from {list:x?}");
    }
}
