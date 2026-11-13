// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The ES 3.0 uniform limits against each other and against the spec minimum
//! (ES 3.0.6 Table 6.32, section 2.12.6), queried through glGetIntegerv:
//!
//! * `MAX_COMBINED_{VERTEX,FRAGMENT}_UNIFORM_COMPONENTS >=
//!   MAX_{VERTEX,FRAGMENT}_UNIFORM_BLOCKS * MAX_UNIFORM_BLOCK_SIZE / 4 +
//!   MAX_{VERTEX,FRAGMENT}_UNIFORM_COMPONENTS`;
//! * the absolute minimums: MAX_UNIFORM_BLOCK_SIZE 16384, combined vertex
//!   50176, combined fragment 50048;
//! * `MAX_*_UNIFORM_VECTORS * 4 <= MAX_*_UNIFORM_COMPONENTS`.
//!
//! The last rule holds on every GPU today and runs live. The formula and the
//! minimums fail today (MAX_UNIFORM_BLOCK_SIZE 6144, combined 1536: gl/state.rs
//! MAX_UNIFORM_DWORDS) and are `#[ignore]`d with the value and the item that
//! raises it; `-- --ignored` runs them and they fail until then. When a raise
//! lands, remove the `ignore` of the cases it closes. The three builds report
//! the same numbers; the reason names the GPU of the build.
//!
//! Not run under `report-es2` (the ES 3.0-only queries are INVALID_ENUM).
//!
//! cargo test --no-default-features --features host-test,gpu-powervr --test es3_uniform_formula
//! cargo test --no-default-features --features host-test,gpu-vivante --test es3_uniform_formula
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_uniform_formula

#![cfg(all(feature = "host-test", not(feature = "report-es2")))]

use core::ffi::{c_char, c_void};
use yakogl::backend::{GlesHeap, YakoglBackend};
use yakogl::gl::consts::*;
use yakogl::gl::state::*;
use yakogl::gl::types::*;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};

/* ---- a context on the host: the smallest OSAL and backend that make one --- */

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
    name: c"es3-uniform-formula".as_ptr() as *const c_char,
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
};

static LIBRARY: std::sync::Mutex<()> = std::sync::Mutex::new(());
static CONTEXT: std::sync::Once = std::sync::Once::new();

/// The context, made once; the guard keeps the tests of this file off the
/// library (single-threaded by contract) one at a time.
fn context() -> std::sync::MutexGuard<'static, ()> {
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

fn geti(pname: GLenum) -> i64 {
    let mut v = [i32::MIN; 4];
    // SAFETY: room for the four values the widest pname returns.
    unsafe { glGetIntegerv(pname, v.as_mut_ptr()) };
    assert_ne!(v[0], i32::MIN, "glGetIntegerv(0x{pname:x}) wrote nothing");
    v[0] as i64
}

/* ---- the rules ----------------------------------------------------------- */

/// (stage, combined, blocks, components, vectors, spec minimum of combined)
const STAGES: [(&str, GLenum, GLenum, GLenum, GLenum, i64); 2] = [
    (
        "VERTEX",
        GL_MAX_COMBINED_VERTEX_UNIFORM_COMPONENTS,
        GL_MAX_VERTEX_UNIFORM_BLOCKS,
        GL_MAX_VERTEX_UNIFORM_COMPONENTS,
        GL_MAX_VERTEX_UNIFORM_VECTORS,
        50_176,
    ),
    (
        "FRAGMENT",
        GL_MAX_COMBINED_FRAGMENT_UNIFORM_COMPONENTS,
        GL_MAX_FRAGMENT_UNIFORM_BLOCKS,
        GL_MAX_FRAGMENT_UNIFORM_COMPONENTS,
        GL_MAX_FRAGMENT_UNIFORM_VECTORS,
        50_048,
    ),
];

fn formula(stage: usize) {
    let _g = context();
    let (name, combined, blocks, comps, _, _) = STAGES[stage];
    let want = geti(blocks) * geti(GL_MAX_UNIFORM_BLOCK_SIZE) / 4 + geti(comps);
    let got = geti(combined);
    assert!(
        got >= want,
        "MAX_COMBINED_{name}_UNIFORM_COMPONENTS {got} < {want} (blocks * block size / 4 + components)"
    );
}

fn minimum(stage: usize) {
    let _g = context();
    let (name, combined, _, _, _, min) = STAGES[stage];
    let got = geti(combined);
    assert!(got >= min, "MAX_COMBINED_{name}_UNIFORM_COMPONENTS {got} < {min}");
}

fn block_size_minimum() {
    let _g = context();
    let got = geti(GL_MAX_UNIFORM_BLOCK_SIZE);
    assert!(got >= 16_384, "MAX_UNIFORM_BLOCK_SIZE {got} < 16384");
}

/// A case that fails today, ignored with the value and the raise, per build.
macro_rules! gap_case {
    ($name:ident, $call:expr, $what:literal) => {
        #[test]
        #[cfg_attr(
            feature = "gpu-v3d",
            ignore = concat!("V3D 4.2 reports ", $what, " today; raised by plan item ubo-v3d")
        )]
        #[cfg_attr(
            all(feature = "gpu-vivante", not(feature = "gpu-v3d")),
            ignore = concat!("GC7000UL reports ", $what, " today; raised by issue draft ubo-pvr-viv-issue (not yet filed)")
        )]
        #[cfg_attr(
            not(any(feature = "gpu-v3d", feature = "gpu-vivante")),
            ignore = concat!("PowerVR BXS-4-64 reports ", $what, " today; raised by issue draft ubo-pvr-viv-issue (not yet filed)")
        )]
        fn $name() {
            $call
        }
    };
}

gap_case!(combined_vertex_meets_formula, formula(0), "MAX_UNIFORM_BLOCK_SIZE 6144 and combined 1536");
gap_case!(combined_fragment_meets_formula, formula(1), "MAX_UNIFORM_BLOCK_SIZE 6144 and combined 1536");
gap_case!(combined_vertex_meets_minimum, minimum(0), "combined vertex 1536 (minimum 50176)");
gap_case!(combined_fragment_meets_minimum, minimum(1), "combined fragment 1536 (minimum 50048)");
gap_case!(uniform_block_size_meets_minimum, block_size_minimum(), "MAX_UNIFORM_BLOCK_SIZE 6144 (minimum 16384)");

#[test]
fn vectors_fit_in_components() {
    let _g = context();
    for (name, _, _, comps, vecs, _) in STAGES {
        let (c, v) = (geti(comps), geti(vecs));
        assert!(v * 4 <= c, "MAX_{name}_UNIFORM_VECTORS {v} * 4 > MAX_{name}_UNIFORM_COMPONENTS {c}");
    }
}

#[test]
fn combined_is_at_least_the_default_block() {
    let _g = context();
    for (name, combined, _, comps, _, _) in STAGES {
        let (cb, c) = (geti(combined), geti(comps));
        assert!(cb >= c, "MAX_COMBINED_{name}_UNIFORM_COMPONENTS {cb} < MAX_{name}_UNIFORM_COMPONENTS {c}");
    }
}
