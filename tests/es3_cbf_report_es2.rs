// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Plan #360, item `cbf-es2-test`: what a `report-es2` build says about
//! float colour targets.
//!
//! The decision (docs/notes/es3-float-targets.md section 7;
//! docs/es3/sized-formats.md "The plan #360 decision";
//! docs/es3/backend-caps.md `GL_EXT_color_buffer_float`) reads for this
//! surface: `GL_EXT_color_buffer_float` is claimed by no build today (its
//! only claimant is the V3D, behind the `color-buffer-float` Cargo feature,
//! off by default), and a `report-es2` build claims no float colour-buffer
//! extension of its own -- the ES 2.0 core has no float renderbuffer format
//! (4.4.3 names none, and an unclaimed extension adds none), so
//! `glGetString(GL_EXTENSIONS)` lists neither `GL_EXT_color_buffer_float`
//! nor `GL_EXT_color_buffer_half_float`, and glRenderbufferStorage answers
//! `GL_INVALID_ENUM` for `RGBA16F` and `R32F` alike.
//!
//! Two of those four assertions hold today and pin here. Two fail today,
//! and those are `#[ignore]`d with the reason, per the item -- this file
//! changes no `src/`:
//!
//! - `GL_EXT_color_buffer_half_float` is claimed in every build, this
//!   surface among them (`src/gl/state.rs:112`, `:252`; the note's section 7
//!   rule keeps it in every ES 3.0 build), and tests/es2_report_guard.rs
//!   `extension_list_is_exact` pins it inside that file's exact 19-name
//!   `GL_EXTENSIONS`. Leaving it off the ES 2.0 report moves an ES 2.0
//!   answer -- the exact list the guard pins, and the 16F renderbuffer
//!   answers gated on it -- and tests/es2_report_guard.rs belongs to plan
//!   #351. Whom the claim leaves out is that owner's call, not this item's.
//! - `RGBA16F` renderbuffer storage is accepted while the half-float claim
//!   stands (`src/gl/framebuffer.rs`, `renderbuffer_format_advertised`;
//!   tests/es2_report_guard.rs `renderbuffer_formats` pins it accepted),
//!   and turns with the claim.
//!
//! The `GL_EXT_color_buffer_float` arm and the `R32F` arm hold: the name is
//! listed nowhere (it is a constant only, `src/gl/consts.rs`), and `R32F`
//! is not in a `report-es2` build's advertised storage formats. Those two
//! are ordinary passing tests; the ignored two flip if the claim ever
//! leaves the ES 2.0 list. All four `#[test]`s exist in every `report-es2`
//! build -- a failing answer is ignored at run time with its reason named,
//! not `#[cfg]`ed away.
//!
//! Standalone: tests/es2_report_guard.rs is another plan's file and is not
//! touched here; the stand-in OSAL and back end are that file's pattern (as
//! in tests/es3_tf_api.rs), so this file runs on its own. No GPU draws: the
//! back end draws nothing.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,report-es2 --test es3_cbf_report_es2
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online,report-es2 --test es3_cbf_report_es2
//! cargo test --no-default-features --features host-test,gpu-vivante,report-es2 --test es3_cbf_report_es2
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online,report-es2 --test es3_cbf_report_es2
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table,report-es2 --test es3_cbf_report_es2
//! ```
//!
//! The three `report-es2` bulk configurations of tools/check.sh are rows 2,
//! 3 and 5 above; line 3 is the same ES 2.0 surface without the compiler
//! (a `host-test` build with no `no-shader-table` carries the Mesa
//! reference, so it has a shader back end either way), and line 5 is the
//! gate's `V3D_ES2_CONFIG` (a `gpu-v3d` build needs `shaderc-online`, and
//! the V3D takes no table: src/lib.rs, "no shader back end" and the
//! `gpu-v3d` refusals). Nightly lints this file under that configuration
//! with `clippy --all-targets`. Compiled only with `report-es2`: other
//! builds see an empty file, as tests/es2_report_guard.rs does.

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

/// An error word for the messages.
fn name_of(e: GLenum) -> String {
    match e {
        GL_NO_ERROR => "GL_NO_ERROR".into(),
        GL_INVALID_ENUM => "GL_INVALID_ENUM".into(),
        GL_INVALID_VALUE => "GL_INVALID_VALUE".into(),
        GL_INVALID_OPERATION => "GL_INVALID_OPERATION".into(),
        e => format!("0x{e:x}"),
    }
}

fn err() -> GLenum {
    // SAFETY: a plain GL call.
    unsafe { glGetError() }
}

/// The error of glRenderbufferStorage(`format`) on a fresh renderbuffer.
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
    let ext = unsafe { CStr::from_ptr(glGetString(GL_EXTENSIONS).cast::<c_char>()) }.to_string_lossy();
    assert_eq!(err(), GL_NO_ERROR, "glGetString(GL_EXTENSIONS)");
    ext.split_whitespace().any(|n| n == name)
}

/* ---- the arms that hold today ------------------------------------------------- */

/// `GL_EXT_color_buffer_float` is listed by no build, this surface among
/// them (the decision's per-GPU lines; the constant is named nowhere in a
/// GL_EXTENSIONS string, `src/gl/state.rs`). ES 2.0 4.4.3: no claimed
/// extension adds a float renderbuffer format without it.
#[test]
fn color_buffer_float_is_not_listed() {
    let _g = context();
    assert!(!listed("GL_EXT_color_buffer_float"), "GL_EXTENSIONS lists GL_EXT_color_buffer_float");
}

/// ES 2.0 4.4.3: `R32F` is renderbuffer storage only with
/// `GL_EXT_color_buffer_float` (an ES 3.0 extension, not claimed here), so
/// GL_INVALID_ENUM (4.4.2's format rule). Today it is not in the
/// advertised set of a `report-es2` build (`src/gl/framebuffer.rs`,
/// `renderbuffer_format_advertised`) -- tests/es2_report_guard.rs
/// `es3_only_renderbuffer_formats_are_invalid_enum` pins the same answer.
#[test]
fn r32f_renderbuffer_is_invalid_enum() {
    let _g = context();
    assert_eq!(name_of(renderbuffer_storage(GL_R32F)), name_of(GL_INVALID_ENUM), "glRenderbufferStorage(GL_R32F)");
}

/* ---- the half-float arm, failing today: recorded, not pinned ------------------- */

/// The decision's half-float arm as written: `GL_EXT_color_buffer_half_float`
/// is claimed in every build today, this surface among them, and
/// tests/es2_report_guard.rs pins it inside its exact list. Leaving it off
/// the ES 2.0 report moves a pinned ES 2.0 answer, and that file belongs to
/// plan #351: which build stops listing it is the guard owner's decision,
/// not this item's. The assertion flips with the claim.
#[test]
#[ignore = "GL_EXT_color_buffer_half_float is claimed by every build today (src/gl/state.rs:112, :252) and pinned \
            in tests/es2_report_guard.rs extension_list_is_exact (plan #351's file): the strict ES 2.0 answer of \
            docs/es3/sized-formats.md \"The plan #360 decision\" waits for that owner's decision"]
fn color_buffer_half_float_is_not_listed() {
    let _g = context();
    assert!(!listed("GL_EXT_color_buffer_half_float"), "GL_EXTENSIONS lists GL_EXT_color_buffer_half_float");
}

/// The same rule for `RGBA16F`: ES 2.0 4.4.3 names no sized float
/// renderbuffer format, so GL_INVALID_ENUM unless a claimed extension adds
/// it. Today the half-float claim stands and it is advertised
/// (`src/gl/framebuffer.rs`, `renderbuffer_format_advertised`;
/// tests/es2_report_guard.rs `renderbuffer_formats` pins it accepted).
/// The assertion flips with the claim, as `color_buffer_half_float_is_not_listed` does.
#[test]
#[ignore = "GL_RGBA16F renderbuffer storage is accepted today because report-es2 claims GL_EXT_color_buffer_half_float \
            (src/gl/framebuffer.rs renderbuffer_format_advertised; tests/es2_report_guard.rs renderbuffer_formats pins \
            it accepted, plan #351's file): it turns with the claim, the same decision as the extension list"]
fn rgba16f_renderbuffer_is_invalid_enum() {
    let _g = context();
    assert_eq!(
        name_of(renderbuffer_storage(GL_RGBA16F)),
        name_of(GL_INVALID_ENUM),
        "glRenderbufferStorage(GL_RGBA16F)"
    );
}
