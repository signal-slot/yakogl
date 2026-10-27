// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! ES 2.0 error rules, draw part (the audit is docs/notes/es2-error-rules.md,
//! Table A row 11).
//!
//! ES 2.0 2.8 takes `GL_UNSIGNED_BYTE` and `GL_UNSIGNED_SHORT` as the index
//! type of `glDrawElements`. `GL_UNSIGNED_INT` is `GL_OES_element_index_uint`,
//! which a `report-es2` build does not advertise, so there `glDrawElements`,
//! `glDrawElementsInstanced` and `glDrawRangeElements` give `GL_INVALID_ENUM`
//! for it. An ES 3.0 build takes it. The type is checked before the program,
//! so no program is needed to see the answer: with no program bound, a type
//! that passes ends in `GL_INVALID_OPERATION`.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,report-es2 --test es2_error_rules_draw
//! cargo test --no-default-features --features host-test,gpu-powervr --test es2_error_rules_draw
//! ```

#![cfg(feature = "host-test")]

use core::ffi::{c_char, c_void};
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{GlesDraw, GlesHeap, YakoglBackend};
use yakogl::gl::consts::*;
use yakogl::gl::draw::*;
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
unsafe extern "C" fn b_draw(_d: *const GlesDraw) -> i32 {
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
    name: c"es2-error-rules-draw".as_ptr() as *const c_char,
    native_attrib_formats: 0,
    rt_rgba8888_is_bgra: false,
    mip_level_align: 0,
    compressed_blocks_in_rows: false,
    native_layered_textures: false,
    front_facing_shader_variant: false,
    etc1_decoded_at_upload: false,
    init: Some(b_init),
    shutdown: None,
    draw: Some(b_draw),
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

fn context() -> MutexGuard<'static, ()> {
    let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    ONCE.call_once(|| {
        // SAFETY: both tables are statics; the lock above serialises the tests.
        unsafe {
            osal_set_ops(&OSAL);
            yakogl::backend::g_backend = &BACKEND;
            assert!(!yakogl::gl::context::gl_context_create(8, 8).is_null());
        }
    });
    drain();
    g
}

const ES2: bool = cfg!(feature = "report-es2");
const INDICES: [u16; 3] = [0, 1, 2];

fn drain() {
    // SAFETY: plain GL call.
    while unsafe { glGetError() } != GL_NO_ERROR {}
}

/// The first error; the queue is empty again afterwards.
fn err() -> GLenum {
    // SAFETY: plain GL call.
    let e = unsafe { glGetError() };
    drain();
    e
}

/// What the type check alone answers: `GL_INVALID_ENUM` for a refused type,
/// `GL_INVALID_OPERATION` (no program) for one that passed it.
fn passes_type_check(e: GLenum) -> bool {
    assert!(e == GL_INVALID_ENUM || e == GL_INVALID_OPERATION, "unexpected error 0x{e:x}");
    e != GL_INVALID_ENUM
}

/// The three entry points that take an index type, as `(name, draw)`.
type DrawCall = (&'static str, fn(GLenum));

fn calls() -> [DrawCall; 3] {
    // SAFETY (all three): plain GL calls; `INDICES` is a static.
    [
        ("glDrawElements", |t| unsafe { glDrawElements(GL_TRIANGLES, 3, t, INDICES.as_ptr() as *const c_void) }),
        ("glDrawElementsInstanced", |t| unsafe {
            glDrawElementsInstanced(GL_TRIANGLES, 3, t, INDICES.as_ptr() as *const c_void, 2)
        }),
        ("glDrawRangeElements", |t| unsafe {
            glDrawRangeElements(GL_TRIANGLES, 0, 2, 3, t, INDICES.as_ptr() as *const c_void)
        }),
    ]
}

/// Row 11: the two ES 2.0 index types pass the type check in both builds.
#[test]
fn es2_index_types_are_accepted() {
    let _g = context();
    for (name, draw) in calls() {
        for t in [GL_UNSIGNED_BYTE, GL_UNSIGNED_SHORT] {
            draw(t);
            assert!(passes_type_check(err()), "{name} type 0x{t:x}");
        }
    }
}

/// Row 11: `GL_UNSIGNED_INT` is `GL_INVALID_ENUM` in `report-es2` and passes the
/// type check in an ES 3.0 build.
#[test]
fn unsigned_int_index_type_by_build() {
    let _g = context();
    for (name, draw) in calls() {
        draw(GL_UNSIGNED_INT);
        assert_eq!(passes_type_check(err()), !ES2, "{name}");
    }
}

/// Row 11: a type that is no index type in either version is `GL_INVALID_ENUM`
/// (`GL_FLOAT`, `GL_INT`, `GL_HALF_FLOAT`, zero).
#[test]
fn other_index_types_are_invalid_enum() {
    let _g = context();
    for (name, draw) in calls() {
        for t in [GL_FLOAT, GL_INT, GL_SHORT, GL_BYTE, GL_HALF_FLOAT, 0] {
            draw(t);
            assert_eq!(err(), GL_INVALID_ENUM, "{name} type 0x{t:x}");
        }
    }
}
