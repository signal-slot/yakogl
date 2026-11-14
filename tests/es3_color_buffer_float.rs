// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Plan #360, item `cbf-test-draft`: the `GL_EXT_color_buffer_float`
//! contract. What holds today, with the extension unclaimed, is pinned by
//! passing tests. What holds after the per-GPU claim (the `color-buffer-float`
//! Cargo feature, item `core-claim`) is written as code compiled only under
//! that feature, with the expected values. No test is `#[ignore]`d (the gate
//! refuses an ignored test) and none is `#[cfg]`ed out (`tests_as_declared`
//! reads it as "declared, did not run"): every `#[test]` exists in every
//! build and the claim-dependent part of its body is `#[cfg]`ed.
//!
//! Rules (docs/notes/es3-float-targets.md; ES 3.0.6 table 3.13, 4.3.2, 4.4.2,
//! 4.4.4.1; EXT_color_buffer_float):
//!
//! - Without the extension RG32F, RGBA32F and R11F_G11F_B10F are not
//!   colour-renderable: a texture attachment is
//!   `GL_FRAMEBUFFER_INCOMPLETE_ATTACHMENT`, `glRenderbufferStorage` is
//!   `GL_INVALID_ENUM`. R32F should follow; it is complete today (recorded
//!   as it is, not asserted incomplete until core-claim changes the GL core).
//! - With it, all seven formats (R16F, RG16F, RGBA16F, R32F, RG32F, RGBA32F,
//!   R11F_G11F_B10F) are complete, and `glReadPixels` takes RGBA/FLOAT.
//! - `GL_EXT_color_buffer_half_float` is claimed today; the 16F answers of
//!   the current tip are recorded as they are.
//!
//! The `color-buffer-float` feature is declared in Cargo.toml (off by
//! default; no library code reads it yet): the "not claimed" assertions are
//! `cfg(not(feature = "color-buffer-float"))`.
//!
//! Feature sets (each with `--no-default-features`):
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test es3_color_buffer_float
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online --test es3_color_buffer_float
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_color_buffer_float
//! ```
//!
//! No GPU runs: the stand-in back end draws nothing. Not in a `report-es2`
//! build (ES 2.0 has none of the sized float names; tests/es2_report_guard.rs).

#![cfg(all(feature = "host-test", not(feature = "report-es2")))]

use core::ffi::{c_char, c_void, CStr};
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{self, GlesDraw, GlesHeap, YakoglBackend};
use yakogl::gl::consts::*;
use yakogl::gl::context;
use yakogl::gl::framebuffer::*;
use yakogl::gl::state::{glGetError, glGetIntegerv, glGetString, glGetStringi, glPixelStorei};
use yakogl::gl::texture::{glBindTexture, glDeleteTextures, glGenTextures, glTexImage2D};
use yakogl::gl::types::*;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};

/* ---- a context on the host ---------------------------------------------- */

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
    name: c"es3-color-buffer-float".as_ptr() as *const c_char,
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
    // SAFETY: the tables are statics; the lock serialises the tests.
    unsafe {
        ONCE.call_once(|| {
            osal_set_ops(&OSAL);
            backend::g_backend = &BACKEND;
            assert!(!context::gl_context_create(16, 16).is_null());
        });
        glPixelStorei(GL_UNPACK_ALIGNMENT, 1);
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        while glGetError() != GL_NO_ERROR {}
    }
    g
}

fn err() -> GLenum {
    // SAFETY: plain GL call.
    unsafe { glGetError() }
}

const NULL: *const c_void = core::ptr::null();

/// The GPU this build is for, as the messages name it.
const GPU: &str = if cfg!(feature = "gpu-powervr") {
    "powervr"
} else if cfg!(feature = "gpu-vivante") {
    "vivante"
} else if cfg!(feature = "gpu-v3d") {
    "v3d"
} else {
    "other"
};

/// Is `name` in `glGetString(GL_EXTENSIONS)`?
fn in_extension_string(name: &str) -> bool {
    // SAFETY: plain GL call; the string is the build's static, NUL-terminated.
    let s = unsafe { glGetString(GL_EXTENSIONS) };
    assert!(!s.is_null(), "{GPU}: glGetString(GL_EXTENSIONS)");
    // SAFETY: non-null and NUL-terminated (above).
    let s = unsafe { CStr::from_ptr(s as *const c_char) }.to_str().unwrap();
    s.split(' ').any(|e| e == name)
}

/// Is `name` one of the `glGetStringi(GL_EXTENSIONS, i)` names?
fn in_extension_list(name: &str) -> bool {
    let mut n: GLint = 0;
    // SAFETY: room for one integer.
    unsafe { glGetIntegerv(GL_NUM_EXTENSIONS, &mut n) };
    assert_eq!(err(), GL_NO_ERROR, "{GPU}: GL_NUM_EXTENSIONS");
    assert!(n > 0, "{GPU}: GL_NUM_EXTENSIONS is {n}");
    (0..n as GLuint).any(|i| {
        // SAFETY: plain GL call; `i` is below GL_NUM_EXTENSIONS.
        let s = unsafe { glGetStringi(GL_EXTENSIONS, i) };
        assert!(!s.is_null(), "{GPU}: glGetStringi(GL_EXTENSIONS, {i})");
        // SAFETY: non-null and NUL-terminated.
        unsafe { CStr::from_ptr(s as *const c_char) }.to_bytes() == name.as_bytes()
    })
}

/// The status of a new framebuffer whose colour attachment 0 is a new 16 x 16
/// texture of `internalformat`; the texture is made with no error.
fn texture_target_status((internalformat, format, type_, label): (GLenum, GLenum, GLenum, &str)) -> GLenum {
    let (mut fb, mut t) = (0, 0);
    // SAFETY: room for one name each; no pixels; plain GL calls.
    unsafe {
        glGenTextures(1, &mut t);
        glBindTexture(GL_TEXTURE_2D, t);
        glTexImage2D(GL_TEXTURE_2D, 0, internalformat as GLint, 16, 16, 0, format, type_, NULL);
        assert_eq!(err(), GL_NO_ERROR, "{GPU}: {label} texture");
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, t, 0);
        assert_eq!(err(), GL_NO_ERROR, "{GPU}: {label} attached");
        let status = glCheckFramebufferStatus(GL_FRAMEBUFFER);
        assert_eq!(err(), GL_NO_ERROR, "{GPU}: {label} status");
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glDeleteFramebuffers(1, &fb);
        glDeleteTextures(1, &t);
        status
    }
}

/// glRenderbufferStorage of a 16 x 16 `internalformat` into a new bound
/// renderbuffer attached as colour 0 of a new framebuffer: the storage
/// call's error, and the framebuffer's status after it.
fn renderbuffer_target(internalformat: GLenum) -> (GLenum, GLenum) {
    let (mut fb, mut rb) = (0, 0);
    // SAFETY: room for one name each; plain GL calls.
    unsafe {
        glGenRenderbuffers(1, &mut rb);
        glBindRenderbuffer(GL_RENDERBUFFER, rb);
        glRenderbufferStorage(GL_RENDERBUFFER, internalformat, 16, 16);
        let storage = err();
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_RENDERBUFFER, rb);
        let status = glCheckFramebufferStatus(GL_FRAMEBUFFER);
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glDeleteFramebuffers(1, &fb);
        glDeleteRenderbuffers(1, &rb);
        while glGetError() != GL_NO_ERROR {}
        (storage, status)
    }
}

/// The seven formats EXT_color_buffer_float (with half_float) makes
/// colour-renderable, each with a (format, type) glTexImage2D takes.
const SEVEN: [(GLenum, GLenum, GLenum, &str); 7] = [
    (GL_R16F, GL_RED, GL_HALF_FLOAT, "R16F"),
    (GL_RG16F, GL_RG, GL_HALF_FLOAT, "RG16F"),
    (GL_RGBA16F, GL_RGBA, GL_HALF_FLOAT, "RGBA16F"),
    (GL_R32F, GL_RED, GL_FLOAT, "R32F"),
    (GL_RG32F, GL_RG, GL_FLOAT, "RG32F"),
    (GL_RGBA32F, GL_RGBA, GL_FLOAT, "RGBA32F"),
    (GL_R11F_G11F_B10F, GL_RGB, GL_UNSIGNED_INT_10F_11F_11F_REV, "R11F_G11F_B10F"),
];

/* ---- (a) not claimed by default ----------------------------------------- */

/// `GL_EXT_color_buffer_float` is in neither GL_EXTENSIONS nor glGetStringi
/// without the claim. `GL_EXT_color_buffer_half_float` IS listed today (note
/// section 5, option B): it is recorded by the next test, not asserted absent.
#[test]
fn extension_not_claimed_by_default() {
    let _g = context();
    #[cfg(not(feature = "color-buffer-float"))]
    {
        assert!(
            !in_extension_string("GL_EXT_color_buffer_float"),
            "{GPU}: glGetString lists GL_EXT_color_buffer_float"
        );
        assert!(!in_extension_list("GL_EXT_color_buffer_float"), "{GPU}: glGetStringi lists GL_EXT_color_buffer_float");
    }
}

/// Today's tip: `GL_EXT_color_buffer_half_float` is claimed, in both forms.
#[test]
fn half_float_extension_today() {
    let _g = context();
    assert!(in_extension_string("GL_EXT_color_buffer_half_float"), "{GPU}: glGetString");
    assert!(in_extension_list("GL_EXT_color_buffer_half_float"), "{GPU}: glGetStringi");
}

/* ---- (b) the 32-bit formats are incomplete without the extension -------- */

#[test]
fn rg32f_rgba32f_r11g11b10_incomplete() {
    let _g = context();
    #[cfg(not(feature = "color-buffer-float"))]
    for f in [SEVEN[4], SEVEN[5], SEVEN[6]] {
        assert_eq!(
            texture_target_status(f),
            GL_FRAMEBUFFER_INCOMPLETE_ATTACHMENT,
            "{GPU}: {} texture as colour 0",
            f.3
        );
        assert_eq!(
            renderbuffer_target(f.0),
            (GL_INVALID_ENUM, GL_FRAMEBUFFER_INCOMPLETE_ATTACHMENT),
            "{GPU}: {} renderbuffer as colour 0",
            f.3
        );
    }
}

/* ---- (c) R16F, RG16F, RGBA16F and R32F as the tip has them -------------- */

/// Recorded as it is: the three 16F formats are complete as texture and as
/// renderbuffer, and so is R32F (the mismatch of the note's section 3).
#[test]
fn r16f_rg16f_rgba16f_r32f_complete_today() {
    let _g = context();
    for f in [SEVEN[0], SEVEN[1], SEVEN[2], SEVEN[3]] {
        assert_eq!(texture_target_status(f), GL_FRAMEBUFFER_COMPLETE, "{GPU}: {} texture", f.3);
        assert_eq!(renderbuffer_target(f.0), (GL_NO_ERROR, GL_FRAMEBUFFER_COMPLETE), "{GPU}: {} renderbuffer", f.3);
    }
}

/* ---- the claimed path (compiled with `color-buffer-float`) -------------- */

/// With the claim (a build with `color-buffer-float`, on a GPU that draws
/// the formats) both names are listed and all seven formats are complete.
#[cfg(feature = "color-buffer-float")]
fn claimed_all_seven_complete() {
    assert!(in_extension_string("GL_EXT_color_buffer_float"), "{GPU}: glGetString");
    assert!(in_extension_list("GL_EXT_color_buffer_float"), "{GPU}: glGetStringi");
    for f in SEVEN {
        assert_eq!(texture_target_status(f), GL_FRAMEBUFFER_COMPLETE, "{GPU}: {} texture", f.3);
        assert_eq!(renderbuffer_target(f.0), (GL_NO_ERROR, GL_FRAMEBUFFER_COMPLETE), "{GPU}: {} renderbuffer", f.3);
    }
}

/// glReadPixels from a complete RGBA32F target takes RGBA/FLOAT (ES 3.0.6
/// 4.3.2: a floating-point colour buffer reads as GL_RGBA / GL_FLOAT).
#[cfg(feature = "color-buffer-float")]
fn claimed_readpixels_rgba_float_accepted() {
    let (mut fb, mut rb) = (0, 0);
    let mut px = [0f32; 4];
    // SAFETY: room for one name each and for one RGBA float pixel; plain GL calls.
    unsafe {
        glGenRenderbuffers(1, &mut rb);
        glBindRenderbuffer(GL_RENDERBUFFER, rb);
        glRenderbufferStorage(GL_RENDERBUFFER, GL_RGBA32F, 16, 16);
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_RENDERBUFFER, rb);
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE, "{GPU}: RGBA32F");
        assert_eq!(err(), GL_NO_ERROR, "{GPU}: setup");
        glReadPixels(0, 0, 1, 1, GL_RGBA, GL_FLOAT, px.as_mut_ptr() as *mut c_void);
        assert_eq!(err(), GL_NO_ERROR, "{GPU}: glReadPixels RGBA/FLOAT of RGBA32F");
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glDeleteFramebuffers(1, &fb);
        glDeleteRenderbuffers(1, &rb);
    }
}

// Not written yet (core-claim, note item blend-32f-map): a draw with GL_BLEND
// into an RGBA32F target is GL_INVALID_OPERATION without EXT_float_blend. The
// stand-in back end has no program here, so it needs the draw path.

/// The claimed path in one test; without the feature it has nothing to check.
#[test]
fn claimed_path() {
    let _g = context();
    #[cfg(feature = "color-buffer-float")]
    {
        claimed_all_seven_complete();
        claimed_readpixels_rgba_float_accepted();
    }
}
