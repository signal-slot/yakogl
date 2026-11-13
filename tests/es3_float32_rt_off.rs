// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! RG32F, RGBA32F and R11F_G11F_B10F are not colour-renderable without
//! `GL_EXT_color_buffer_float` (ES 3.0.6 4.4.4 and table 3.13). No build
//! claims the extension today, and a later capability keeps it off by
//! default, so in the ES 3.0 host build of every GPU:
//!
//! - (a) the extension is in neither `GL_EXTENSIONS` nor `glGetStringi`;
//! - (b) a texture of each format as `GL_COLOR_ATTACHMENT0` is
//!   `GL_FRAMEBUFFER_INCOMPLETE_ATTACHMENT`;
//! - (c) `glRenderbufferStorage` of each format is `GL_INVALID_ENUM` (4.4.2.1:
//!   the format must be colour-, depth- or stencil-renderable);
//! - (d) `glReadPixels` and `glClear` on the incomplete framebuffer are
//!   `GL_INVALID_FRAMEBUFFER_OPERATION` (4.4.4). `glClear` only records a
//!   pending clear today and raises it later: that test is `#[ignore]`d.
//!
//! R32F is wrongly complete today (src/gl/framebuffer.rs `is_color_format`);
//! its case asserts the spec answer and is `#[ignore]`d until plan #325's
//! board-mrt lands.
//!
//! Feature sets (the rule is GL core, the same on every GPU):
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test es3_float32_rt_off
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online --test es3_float32_rt_off
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_float32_rt_off
//! ```
//!
//! No GPU runs: the stand-in back end draws nothing.
//!
//! Not in a `report-es2` build: ES 2.0 has none of the sized float names.

#![cfg(all(feature = "host-test", not(feature = "report-es2")))]

use core::ffi::{c_char, c_void, CStr};
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{self, GlesDraw, GlesHeap, YakoglBackend};
use yakogl::gl::consts::*;
use yakogl::gl::context;
use yakogl::gl::framebuffer::*;
use yakogl::gl::state::{glClear, glGetError, glGetIntegerv, glGetString, glGetStringi, glPixelStorei};
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
    name: c"es3-float32-rt-off".as_ptr() as *const c_char,
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

/// (internalformat, glTexImage2D format, type, label)
type Fmt = (GLenum, GLenum, GLenum, &'static str);

/// The formats that need `GL_EXT_color_buffer_float`.
const FLOAT32: [Fmt; 3] = [
    (GL_RGBA32F, GL_RGBA, GL_FLOAT, "RGBA32F"),
    (GL_RG32F, GL_RG, GL_FLOAT, "RG32F"),
    (GL_R11F_G11F_B10F, GL_RGB, GL_UNSIGNED_INT_10F_11F_11F_REV, "R11F_G11F_B10F"),
];

/// Apart: wrongly complete today, see the module text.
const R32F: Fmt = (GL_R32F, GL_RED, GL_FLOAT, "R32F");

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

/// Make a framebuffer whose colour 0 is a new 16 x 16 texture of `f`, leave it
/// bound, and give its status to `body`; then delete it all.
fn with_texture_target<R>((internalformat, format, type_, label): Fmt, body: impl FnOnce(GLenum) -> R) -> R {
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
        let r = body(status);
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glDeleteFramebuffers(1, &fb);
        glDeleteTextures(1, &t);
        while glGetError() != GL_NO_ERROR {}
        r
    }
}

fn texture_target_status(f: Fmt) -> GLenum {
    with_texture_target(f, |s| s)
}

/// glRenderbufferStorage of a 16 x 16 `internalformat` into a new bound
/// renderbuffer attached as colour 0 of a new framebuffer: the storage call's
/// error, and the framebuffer's status after it.
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

/// glClear on the bound framebuffer: its error.
fn clear_error() -> GLenum {
    // SAFETY: plain GL calls.
    unsafe {
        while glGetError() != GL_NO_ERROR {}
        glClear(GL_COLOR_BUFFER_BIT);
        err()
    }
}

/// glReadPixels (RGBA / UNSIGNED_BYTE, 1 x 1) on the bound framebuffer: its error.
fn read_error() -> GLenum {
    let mut px = [0u8; 4];
    // SAFETY: a 1 x 1 RGBA8 read into four bytes; plain GL calls.
    unsafe {
        while glGetError() != GL_NO_ERROR {}
        glReadPixels(0, 0, 1, 1, GL_RGBA, GL_UNSIGNED_BYTE, px.as_mut_ptr() as *mut c_void);
        err()
    }
}

/* ---- (a) the extension strings ------------------------------------------ */

#[test]
fn color_buffer_float_not_listed() {
    let _g = context();
    assert!(!in_extension_string("GL_EXT_color_buffer_float"), "{GPU}: glGetString lists GL_EXT_color_buffer_float");
    assert!(!in_extension_list("GL_EXT_color_buffer_float"), "{GPU}: glGetStringi lists GL_EXT_color_buffer_float");
}

/* ---- (b) texture attachments -------------------------------------------- */

#[test]
fn float32_texture_attachment_incomplete() {
    let _g = context();
    for f in FLOAT32 {
        assert_eq!(texture_target_status(f), GL_FRAMEBUFFER_INCOMPLETE_ATTACHMENT, "{GPU}: {} texture", f.3);
    }
}

/* ---- (c) renderbuffer storage ------------------------------------------- */

#[test]
fn float32_renderbuffer_storage_invalid_enum() {
    let _g = context();
    for (f, _, _, label) in FLOAT32 {
        assert_eq!(
            renderbuffer_target(f),
            (GL_INVALID_ENUM, GL_FRAMEBUFFER_INCOMPLETE_ATTACHMENT),
            "{GPU}: {label} renderbuffer"
        );
    }
}

/* ---- (d) Clear and ReadPixels on the incomplete framebuffer ------------- */

#[test]
fn float32_incomplete_read_invalid_operation() {
    let _g = context();
    for f in FLOAT32 {
        assert_eq!(
            with_texture_target(f, |_| read_error()),
            GL_INVALID_FRAMEBUFFER_OPERATION,
            "{GPU}: {} glReadPixels",
            f.3
        );
    }
}

/// ES 3.0.6 4.4.4: glClear on an incomplete framebuffer is
/// `GL_INVALID_FRAMEBUFFER_OPERATION` at the call. Today glClear only records
/// a pending clear (src/gl/state.rs glClear) and the error comes with the
/// next draw or flush (src/gl/draw.rs emit_pending_clear), so the call itself
/// is `GL_NO_ERROR`.
#[test]
#[ignore = "defect: glClear on an incomplete framebuffer raises no error at the call, only when the pending clear is emitted (src/gl/state.rs glClear)"]
fn float32_incomplete_clear_invalid_operation() {
    let _g = context();
    for f in FLOAT32 {
        assert_eq!(
            with_texture_target(f, |_| clear_error()),
            GL_INVALID_FRAMEBUFFER_OPERATION,
            "{GPU}: {} glClear",
            f.3
        );
    }
}

/* ---- R32F: the spec answer, wrong today --------------------------------- */

#[test]
#[ignore = "R32F incomplete without the extension: plan #325 board-mrt"]
fn r32f_refused_without_extension() {
    let _g = context();
    assert_eq!(texture_target_status(R32F), GL_FRAMEBUFFER_INCOMPLETE_ATTACHMENT, "{GPU}: R32F texture");
    assert_eq!(
        renderbuffer_target(GL_R32F),
        (GL_INVALID_ENUM, GL_FRAMEBUFFER_INCOMPLETE_ATTACHMENT),
        "{GPU}: R32F renderbuffer"
    );
    assert_eq!(
        with_texture_target(R32F, |_| read_error()),
        GL_INVALID_FRAMEBUFFER_OPERATION,
        "{GPU}: R32F glReadPixels"
    );
}
