// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! gl_test case 20 (`etc`, driver/tools/pvrlab/gl_test.c `case_etc`): its
//! compressed-format checks, run through the GL API on the host for each
//! GPU build. The core promises the count and list of
//! GL_COMPRESSED_TEXTURE_FORMATS, not the old "3 formats, ETC1 first":
//!
//! - under `report-es2`: exactly 3 (ETC1, RGB8_ETC2, RGBA8_ETC2_EAC);
//! - otherwise (ES 3.0): those three, in any order, plus either both sRGB
//!   ETC2 formats or neither, as the back end samples them
//!   (src/gl/texture.rs `stored_compressed_formats`; tests/es3_caps.rs
//!   `known_gaps` keeps the five-format list as the intended ES 3.0 one);
//! - GL_ETC1_RGB8_OES appears somewhere in the list, the extension string
//!   has GL_OES_compressed_ETC1_RGB8_texture, and no GL error is raised.
//!
//! The count each build must give is derived from the core's own rule
//! (`report-es2` and the back end's `sampled_texfmts`), not a number. One
//! line per build prints the count and the list (--nocapture).
//!
//! cargo test --no-default-features --features host-test,gpu-powervr --test gltest_case20_formats -- --nocapture
//! cargo test --no-default-features --features host-test,gpu-powervr,report-es2 --test gltest_case20_formats -- --nocapture
//! cargo test --no-default-features --features host-test,gpu-vivante --test gltest_case20_formats -- --nocapture
//! cargo test --no-default-features --features host-test,gpu-vivante,report-es2 --test gltest_case20_formats -- --nocapture
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test gltest_case20_formats -- --nocapture
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table,report-es2 --test gltest_case20_formats -- --nocapture

#![cfg(feature = "host-test")]

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
    name: c"es3-caps".as_ptr() as *const c_char,
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

const ETC1_EXT: &str = "GL_OES_compressed_ETC1_RGB8_texture";
const BASE: [GLenum; 3] = [GL_ETC1_RGB8_OES, GL_COMPRESSED_RGB8_ETC2, GL_COMPRESSED_RGBA8_ETC2_EAC];
const SRGB: [GLenum; 2] = [GL_COMPRESSED_SRGB8_ETC2, GL_COMPRESSED_SRGB8_ALPHA8_ETC2_EAC];

const GPU: &str = if cfg!(feature = "gpu-v3d") {
    "V3D 4.2 (Pi 4)"
} else if cfg!(feature = "gpu-vivante") {
    "GC7000UL (i.MX8MP)"
} else {
    "BXS-4-64 (AM62P)"
};

/// The six formats the GL core decodes at upload and the class each lands
/// in (docs/notes/etc2-eac.md): stored in an ES 3.0 report on a back end
/// that samples the class.
fn decoded() -> [(GLenum, u8); 6] {
    use yakogl::backend::*;
    [
        (GL_COMPRESSED_R11_EAC, GLES_TEXFMT_R16F),
        (GL_COMPRESSED_SIGNED_R11_EAC, GLES_TEXFMT_R16F),
        (GL_COMPRESSED_RG11_EAC, GLES_TEXFMT_RGBA16F),
        (GL_COMPRESSED_SIGNED_RG11_EAC, GLES_TEXFMT_RGBA16F),
        (GL_COMPRESSED_RGB8_PUNCHTHROUGH_ALPHA1_ETC2, GLES_TEXFMT_RGBA8),
        (GL_COMPRESSED_SRGB8_PUNCHTHROUGH_ALPHA1_ETC2, GLES_TEXFMT_SRGB8_A8),
    ]
}

/// The count the core's rule gives this build: the sRGB pair is stored only
/// in an ES 3.0 report and where the back end samples both, and each decoded
/// format where the back end samples its class.
fn expected_count() -> usize {
    use yakogl::backend::*;
    let sampled = sampled_texfmts();
    let srgb = !cfg!(feature = "report-es2")
        && sampled & gles_texfmt_bit(GLES_TEXFMT_ETC2_SRGB_RGB) != 0
        && sampled & gles_texfmt_bit(GLES_TEXFMT_ETC2_SRGB_RGBA) != 0;
    let dec = decoded()
        .iter()
        .filter(|&&(_, class)| !cfg!(feature = "report-es2") && sampled & gles_texfmt_bit(class) != 0)
        .count();
    BASE.len() + if srgb { SRGB.len() } else { 0 } + dec
}

#[test]
fn case20_compressed_formats() {
    let _g = context();
    // SAFETY: plain GL calls on the context made above; the buffer is sized
    // from the count the core reports.
    unsafe {
        while glGetError() != GL_NO_ERROR {}
        let mut n = 0;
        glGetIntegerv(GL_NUM_COMPRESSED_TEXTURE_FORMATS, &mut n);
        let count = n.max(0) as usize;
        let mut fmts = vec![0 as GLint; count + 8];
        glGetIntegerv(GL_COMPRESSED_TEXTURE_FORMATS, fmts.as_mut_ptr());
        let list: Vec<GLenum> = fmts[..count].iter().map(|&f| f as GLenum).collect();
        let ext = std::ffi::CStr::from_ptr(glGetString(GL_EXTENSIONS) as *const c_char).to_string_lossy().into_owned();
        let err = glGetError();

        println!(
            "case 20 {GPU}{}: {n} compressed formats: {}",
            if cfg!(feature = "report-es2") { " report-es2" } else { "" },
            list.iter().map(|f| format!("0x{f:04x}")).collect::<Vec<_>>().join(" ")
        );

        assert_eq!(err, GL_NO_ERROR, "GL error 0x{err:x}");
        if cfg!(feature = "report-es2") {
            assert_eq!(count, 3, "ES 2.0 report");
        }
        if !cfg!(feature = "report-es2") && !cfg!(feature = "gpu-mali") {
            assert_eq!(count, 11, "ETC1 and the ten ES 3.0 ETC2/EAC formats ({GPU})");
        }
        assert_eq!(count, expected_count(), "count the core's rule gives this build");
        for f in BASE {
            assert_eq!(list.iter().filter(|&&x| x == f).count(), 1, "0x{f:x} once in the list");
        }
        let srgb = SRGB.iter().filter(|f| list.contains(f)).count();
        assert!(srgb == 0 || srgb == 2, "sRGB ETC2 pair is both or neither");
        let dec = decoded().iter().filter(|(f, _)| list.contains(f)).count();
        assert_eq!(list.len(), BASE.len() + srgb + dec, "nothing else in the list");
        assert!(list.contains(&GL_ETC1_RGB8_OES), "ETC1 in the list");
        assert!(ext.split_whitespace().any(|e| e == ETC1_EXT), "{ETC1_EXT} in GL_EXTENSIONS: {ext}");
    }
}
