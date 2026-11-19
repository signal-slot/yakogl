// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The PowerVR (AM62P, BXS-4-64) ES 3.0 uniform limits after the
//! uniform-block raise (plan #378,
//! `docs/notes/ubo-limits.md` section 4, `docs/notes/pvr-ubo-memory-load.md`).
//!
//! `GL_MAX_UNIFORM_BLOCK_SIZE` was 6144 and
//! `GL_MAX_COMBINED_{VERTEX,FRAGMENT}_UNIFORM_COMPONENTS` 1536; they are now
//! 16384 and 50688 (`MAX_UNIFORM_BLOCKS * 16384 / 4 + 1536`,
//! `combined_uniform_components` in `src/gl/limits.rs`). The default-block
//! limits `GL_MAX_{VERTEX,FRAGMENT}_UNIFORM_COMPONENTS` (1536) and the
//! `..._UNIFORM_VECTORS` do not move. Each pname is read through
//! `glGetIntegerv`, `glGetFloatv` and `glGetInteger64v`, which must agree.
//!
//! The PowerVR arm in `src/gl/context.rs` reports the raised values; the
//! default-block limits stay.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test es3_limits_pvr_ubo
//! ```

#![cfg(all(feature = "host-test", feature = "gpu-powervr", not(feature = "report-es2")))]

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
    name: c"es3-limits-pvr-ubo".as_ptr() as *const c_char,
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

/* ---- reads ---------------------------------------------------------------- */

/// MAX_{VERTEX,FRAGMENT}_UNIFORM_VECTORS as they read today; they do not move.
const VECTORS: i64 = 384;

fn error_clear() {
    // SAFETY: plain GL call.
    unsafe { while glGetError() != GL_NO_ERROR {} }
}

/// One pname through the three getters; they must agree and raise no error.
fn read(name: &str, pname: GLenum) -> i64 {
    error_clear();
    let mut i = i32::MIN;
    let mut f = f32::NAN;
    let mut l = i64::MIN;
    // SAFETY: one value each.
    unsafe {
        glGetIntegerv(pname, &mut i);
        glGetFloatv(pname, &mut f);
        glGetInteger64v(pname, &mut l);
        assert_eq!(glGetError(), GL_NO_ERROR, "{name}");
    }
    assert_eq!(i64::from(i), l, "{name}: glGetIntegerv vs glGetInteger64v");
    assert_eq!(f, l as f32, "{name}: glGetFloatv vs glGetInteger64v");
    l
}

const COMBINED_VERTEX: (&str, GLenum) =
    ("MAX_COMBINED_VERTEX_UNIFORM_COMPONENTS", GL_MAX_COMBINED_VERTEX_UNIFORM_COMPONENTS);
const COMBINED_FRAGMENT: (&str, GLenum) =
    ("MAX_COMBINED_FRAGMENT_UNIFORM_COMPONENTS", GL_MAX_COMBINED_FRAGMENT_UNIFORM_COMPONENTS);

/* ---- unchanged by the raise (live, stays) --------------------------------- */

#[test]
fn per_stage_uniform_limits_stay() {
    let _g = context();
    assert_eq!(read("MAX_VERTEX_UNIFORM_COMPONENTS", GL_MAX_VERTEX_UNIFORM_COMPONENTS), 1536);
    assert_eq!(read("MAX_FRAGMENT_UNIFORM_COMPONENTS", GL_MAX_FRAGMENT_UNIFORM_COMPONENTS), 1536);
    assert_eq!(read("MAX_VERTEX_UNIFORM_VECTORS", GL_MAX_VERTEX_UNIFORM_VECTORS), VECTORS);
    assert_eq!(read("MAX_FRAGMENT_UNIFORM_VECTORS", GL_MAX_FRAGMENT_UNIFORM_VECTORS), VECTORS);
}

/* ---- raised --------------------------------------------------------------- */

const ES3_MIN_COMBINED_VERTEX: i64 = 50176;
const ES3_MIN_COMBINED_FRAGMENT: i64 = 50048;

#[test]
fn raised_block_size() {
    let _g = context();
    // read() checks the int, float and 64-bit reads agree.
    assert_eq!(read("MAX_UNIFORM_BLOCK_SIZE", GL_MAX_UNIFORM_BLOCK_SIZE), 16384);
}

#[test]
fn raised_combined_components() {
    let _g = context();
    let size = read("MAX_UNIFORM_BLOCK_SIZE", GL_MAX_UNIFORM_BLOCK_SIZE);
    let comps = |blocks: GLenum, per_stage: GLenum| read("blocks", blocks) * size / 4 + read("components", per_stage);
    let v = comps(GL_MAX_VERTEX_UNIFORM_BLOCKS, GL_MAX_VERTEX_UNIFORM_COMPONENTS);
    let f = comps(GL_MAX_FRAGMENT_UNIFORM_BLOCKS, GL_MAX_FRAGMENT_UNIFORM_COMPONENTS);
    assert_eq!(v, 50688, "formula, vertex");
    assert_eq!(f, 50688, "formula, fragment");
    assert!(v >= ES3_MIN_COMBINED_VERTEX && f >= ES3_MIN_COMBINED_FRAGMENT);
    assert_eq!(read(COMBINED_VERTEX.0, COMBINED_VERTEX.1), v);
    assert_eq!(read(COMBINED_FRAGMENT.0, COMBINED_FRAGMENT.1), f);
}
