// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The float colour target decision of docs/notes/es3-float-targets.md
//! (plan #343, item fix-float), through the GL API on the host: dEQP sees
//! NotSupported or a working target, never an incomplete framebuffer it did
//! not ask for.
//!
//! The note (section 5) keeps `GL_EXT_color_buffer_float` unclaimed on every
//! back end (PowerVR, GC7000UL, V3D: none renders RG32F or RGBA32F today) and
//! keeps `GL_EXT_color_buffer_half_float` claimed. So dEQP reads the extension
//! string and answers NotSupported for the 32-bit and r11f_g11f_b10f rows, and
//! the one place it asks for the ES 3.0 core answer is
//! `fbo.completeness.renderable`: a format that is not colour-renderable is
//! `GL_FRAMEBUFFER_INCOMPLETE_ATTACHMENT` as a texture and `GL_INVALID_ENUM`
//! at `glRenderbufferStorage`. Pinned here:
//!
//! - the extension is absent from `glGetString(GL_EXTENSIONS)` and every
//!   `glGetStringi`; the half-float one is listed;
//! - RGBA32F, RG32F, RGB32F, R11F_G11F_B10F as a texture attachment are
//!   INCOMPLETE_ATTACHMENT (never UNSUPPORTED, never a silent RGBA16F), as a
//!   renderbuffer INVALID_ENUM;
//! - RGBA16F, RGB16F, RG16F, R16F, and the RGBA8 and integer targets the
//!   shaders.derivate cases use (RGBA8, RGBA32UI) stay complete.
//!
//! R32F is the one the note says is wrong today (section 5, the contract of
//! fix-float: complete without the extension, `src/gl/framebuffer.rs`
//! `is_color_format`). Its tests are `#[ignore]`d with that section; the
//! passing `r32f_today_is_complete` pins the current answer so the change
//! has to move it on purpose. No back end claims the extension, so there is
//! no render and readback test for a claimed float target.
//!
//! The rule is GL core and the same for every back end; the GL core is driven
//! into a stand-in back end, so the file runs in each GPU row of tools/check.sh
//! (`gpu-powervr`, `gpu-vivante`, V3D) and names the one it ran for.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_float_target
//! ```

#![cfg(all(feature = "host-test", not(feature = "report-es2")))]

use core::ffi::{c_char, c_void, CStr};
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{self, GlesDraw, GlesHeap, YakoglBackend};
use yakogl::gl::consts::*;
use yakogl::gl::framebuffer::*;
use yakogl::gl::state::*;
use yakogl::gl::texture::*;
use yakogl::gl::types::*;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};

/// The back end this build was made for, for the failure messages.
const GPU: &str = if cfg!(feature = "gpu-powervr") {
    "powervr"
} else if cfg!(feature = "gpu-vivante") {
    "vivante"
} else if cfg!(feature = "gpu-v3d") {
    "v3d"
} else {
    "none"
};

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
    name: c"es3-float-target".as_ptr() as *const c_char,
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

static LOCK: Mutex<()> = Mutex::new(());
static ONCE: Once = Once::new();

fn context() -> MutexGuard<'static, ()> {
    let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    ONCE.call_once(|| {
        // SAFETY: both tables are statics; the lock above serialises the tests.
        unsafe {
            osal_set_ops(&OSAL);
            backend::g_backend = &BACKEND;
            assert!(!yakogl::gl::context::gl_context_create(8, 8).is_null());
        }
    });
    // SAFETY: plain GL call.
    unsafe {
        glFinish();
        while glGetError() != GL_NO_ERROR {}
    }
    g
}

fn err() -> GLenum {
    // SAFETY: plain GL call.
    unsafe { glGetError() }
}

/// A 4x4 texture of `internal` (with a format and type that upload it),
/// attached to colour 0 of a new bound framebuffer. Returns the framebuffer
/// status, the GL error of the upload or attach, and the names to delete.
fn texture_status(internal: GLenum, format: GLenum, type_: GLenum) -> (GLenum, GLenum, [GLuint; 2]) {
    let (mut tex, mut fb) = (0, 0);
    // SAFETY: room for one name each; null pixels are a storage-only upload.
    unsafe {
        glGenTextures(1, &mut tex);
        glBindTexture(GL_TEXTURE_2D, tex);
        glTexImage2D(GL_TEXTURE_2D, 0, internal as GLint, 4, 4, 0, format, type_, core::ptr::null());
        let upload = glGetError();
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex, 0);
        let attach = glGetError();
        let status = glCheckFramebufferStatus(GL_FRAMEBUFFER);
        (status, if upload != GL_NO_ERROR { upload } else { attach }, [tex, fb])
    }
}

/// A 4x4 renderbuffer of `format` attached to colour 0 of a new bound
/// framebuffer. Returns the status, the GL error of the storage call and the
/// names to delete.
fn renderbuffer_status(format: GLenum) -> (GLenum, GLenum, [GLuint; 2]) {
    let (mut rb, mut fb) = (0, 0);
    // SAFETY: room for one name each.
    unsafe {
        glGenRenderbuffers(1, &mut rb);
        glBindRenderbuffer(GL_RENDERBUFFER, rb);
        glRenderbufferStorage(GL_RENDERBUFFER, format, 4, 4);
        let storage = glGetError();
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_RENDERBUFFER, rb);
        while glGetError() != GL_NO_ERROR {}
        (glCheckFramebufferStatus(GL_FRAMEBUFFER), storage, [rb, fb])
    }
}

fn release_texture(n: [GLuint; 2]) {
    // SAFETY: names this test made.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glDeleteFramebuffers(1, &n[1]);
        glDeleteTextures(1, &n[0]);
    }
}

fn release_renderbuffer(n: [GLuint; 2]) {
    // SAFETY: names this test made.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glDeleteFramebuffers(1, &n[1]);
        glDeleteRenderbuffers(1, &n[0]);
    }
}

/// The formats that are not colour-renderable without GL_EXT_color_buffer_float
/// (ES 3.0 table 3.13), with a texture format and type that upload them. R32F
/// is separate: see the R32F tests.
const NOT_RENDERABLE: [(GLenum, &str, GLenum, GLenum); 4] = [
    (GL_RGBA32F, "RGBA32F", GL_RGBA, GL_FLOAT),
    (GL_RG32F, "RG32F", GL_RG, GL_FLOAT),
    (GL_RGB32F, "RGB32F", GL_RGB, GL_FLOAT),
    (GL_R11F_G11F_B10F, "R11F_G11F_B10F", GL_RGB, GL_FLOAT),
];

/// The formats colour-renderable by the half-float extension.
const HALF: [(GLenum, &str, GLenum, GLenum); 4] = [
    (GL_RGBA16F, "RGBA16F", GL_RGBA, GL_HALF_FLOAT),
    (GL_RGB16F, "RGB16F", GL_RGB, GL_HALF_FLOAT),
    (GL_RG16F, "RG16F", GL_RG, GL_HALF_FLOAT),
    (GL_R16F, "R16F", GL_RED, GL_HALF_FLOAT),
];

/* ---- (a) the extension string ---- */

fn extensions() -> Vec<String> {
    // SAFETY: GL strings are NUL-terminated statics.
    let all = unsafe { CStr::from_ptr(glGetString(GL_EXTENSIONS) as *const c_char) }.to_str().unwrap().to_owned();
    all.split_whitespace().map(str::to_owned).collect()
}

fn extensions_indexed() -> Vec<String> {
    let mut n = 0;
    // SAFETY: room for one value; the indices are in range; strings are statics.
    unsafe {
        glGetIntegerv(GL_NUM_EXTENSIONS, &mut n);
        (0..n as GLuint)
            .map(|i| CStr::from_ptr(glGetStringi(GL_EXTENSIONS, i) as *const c_char).to_str().unwrap().to_owned())
            .collect()
    }
}

#[test]
fn color_buffer_float_is_not_claimed() {
    let _g = context();
    for (how, list) in [("glGetString", extensions()), ("glGetStringi", extensions_indexed())] {
        assert!(
            !list.iter().any(|e| e == "GL_EXT_color_buffer_float"),
            "{GPU}: {how} lists GL_EXT_color_buffer_float, which note section 5 keeps unclaimed (no back end renders RGBA32F)"
        );
    }
    assert_eq!(err(), GL_NO_ERROR);
}

#[test]
fn half_float_extension_stays_claimed() {
    let _g = context();
    for (how, list) in [("glGetString", extensions()), ("glGetStringi", extensions_indexed())] {
        assert!(
            list.iter().any(|e| e == "GL_EXT_color_buffer_half_float"),
            "{GPU}: {how} lacks GL_EXT_color_buffer_half_float (note section 5, option B)"
        );
    }
}

#[test]
fn string_and_indexed_lists_agree() {
    let _g = context();
    let (mut a, mut b) = (extensions(), extensions_indexed());
    a.sort();
    b.sort();
    assert_eq!(a, b, "{GPU}");
}

/* ---- (b) the formats that are not renderable follow ES 3.0 core ---- */

#[test]
fn float32_texture_attachment_is_incomplete_attachment() {
    let _g = context();
    for (f, name, fmt, ty) in NOT_RENDERABLE {
        let (status, e, n) = texture_status(f, fmt, ty);
        assert_eq!(e, GL_NO_ERROR, "{GPU}: {name} texture upload or attach gave 0x{e:x}");
        assert_eq!(
            status, GL_FRAMEBUFFER_INCOMPLETE_ATTACHMENT,
            "{GPU}: {name} texture colour attachment must be INCOMPLETE_ATTACHMENT (ES 3.0 4.4.4.1), not 0x{status:x}"
        );
        release_texture(n);
    }
}

#[test]
fn float32_renderbuffer_storage_is_invalid_enum() {
    let _g = context();
    for (f, name, _, _) in NOT_RENDERABLE {
        let (status, storage, n) = renderbuffer_status(f);
        assert_eq!(
            storage, GL_INVALID_ENUM,
            "{GPU}: glRenderbufferStorage({name}) must be INVALID_ENUM, got 0x{storage:x}"
        );
        assert_ne!(
            status, GL_FRAMEBUFFER_COMPLETE,
            "{GPU}: a {name} renderbuffer without storage must not be complete"
        );
        release_renderbuffer(n);
    }
}

#[test]
fn float32_texture_is_never_unsupported() {
    // dEQP's completeness.renderable rows accept COMPLETE or the ES 3.0
    // answer; UNSUPPORTED is neither.
    let _g = context();
    for (f, name, fmt, ty) in NOT_RENDERABLE {
        let (status, _, n) = texture_status(f, fmt, ty);
        assert_ne!(status, GL_FRAMEBUFFER_UNSUPPORTED, "{GPU}: {name}");
        release_texture(n);
    }
}

/* ---- (c) the half-float family and the targets the derivate cases use ---- */

#[test]
fn half_float_targets_are_complete() {
    // Every renderbuffer, and the textures whose store is a 16F class of its
    // own (RGBA16F, R16F). RG16F and RGB16F textures: see the next test.
    let _g = context();
    for (f, name, fmt, ty) in HALF {
        if f == GL_RGBA16F || f == GL_R16F {
            let (status, e, n) = texture_status(f, fmt, ty);
            assert_eq!(e, GL_NO_ERROR, "{GPU}: {name} texture gave 0x{e:x}");
            assert_eq!(status, GL_FRAMEBUFFER_COMPLETE, "{GPU}: {name} texture target, 0x{status:x}");
            release_texture(n);
        }
        let (status, storage, n) = renderbuffer_status(f);
        assert_eq!(storage, GL_NO_ERROR, "{GPU}: {name} renderbuffer storage gave 0x{storage:x}");
        assert_eq!(status, GL_FRAMEBUFFER_COMPLETE, "{GPU}: {name} renderbuffer target, 0x{status:x}");
        release_renderbuffer(n);
    }
}

#[test]
#[ignore = "docs/notes/es3-float-targets.md sections 3 and 5 (option B keeps GL_EXT_color_buffer_half_float): RG16F and RGB16F textures are INCOMPLETE_ATTACHMENT today (attachment_storage in src/gl/framebuffer.rs admits only the RGBA16F, R16F, R32F texture classes); a code change if dEQP's completeness.renderable rg16f/rgb16f texture rows are to pass"]
fn rg16f_rgb16f_texture_attachments_are_complete() {
    let _g = context();
    for (f, name, fmt, ty) in HALF {
        if f == GL_RG16F || f == GL_RGB16F {
            let (status, e, n) = texture_status(f, fmt, ty);
            assert_eq!(e, GL_NO_ERROR, "{GPU}: {name} texture gave 0x{e:x}");
            assert_eq!(status, GL_FRAMEBUFFER_COMPLETE, "{GPU}: {name} texture target, 0x{status:x}");
            release_texture(n);
        }
    }
}

#[test]
fn derivate_targets_stay_complete() {
    // shaders.derivate renders into RGBA8 (the msaa cases resolve into one)
    // and, for fbo_float, RGBA32UI.
    let _g = context();
    for (f, name) in [(GL_RGBA8, "RGBA8"), (GL_RGBA32UI, "RGBA32UI"), (GL_RGBA8UI, "RGBA8UI"), (GL_RGBA32I, "RGBA32I")]
    {
        let (status, storage, n) = renderbuffer_status(f);
        assert_eq!(storage, GL_NO_ERROR, "{GPU}: {name} renderbuffer storage gave 0x{storage:x}");
        assert_eq!(status, GL_FRAMEBUFFER_COMPLETE, "{GPU}: {name} renderbuffer target, 0x{status:x}");
        release_renderbuffer(n);
    }
    let (status, e, n) = texture_status(GL_RGBA8, GL_RGBA, GL_UNSIGNED_BYTE);
    assert_eq!((e, status), (GL_NO_ERROR, GL_FRAMEBUFFER_COMPLETE), "{GPU}: RGBA8 texture target");
    release_texture(n);
}

/* ---- R32F: today's answer, and the one the note asks for ---- */

#[test]
fn r32f_today_is_complete() {
    // docs/notes/es3-float-targets.md sections 3 and 5: R32F is complete
    // without the extension. This pin moves with the change of section 5.
    let _g = context();
    let (status, e, n) = texture_status(GL_R32F, GL_RED, GL_FLOAT);
    assert_eq!((e, status), (GL_NO_ERROR, GL_FRAMEBUFFER_COMPLETE), "{GPU}");
    release_texture(n);
}

#[test]
#[ignore = "docs/notes/es3-float-targets.md section 5 (A+B): R32F texture attachment becomes INCOMPLETE_ATTACHMENT; needs src/gl/framebuffer.rs is_color_format"]
fn r32f_texture_attachment_is_incomplete_attachment() {
    let _g = context();
    let (status, e, n) = texture_status(GL_R32F, GL_RED, GL_FLOAT);
    assert_eq!(e, GL_NO_ERROR, "{GPU}");
    assert_eq!(status, GL_FRAMEBUFFER_INCOMPLETE_ATTACHMENT, "{GPU}");
    release_texture(n);
}

#[test]
#[ignore = "docs/notes/es3-float-targets.md section 5 (A+B): R32F glRenderbufferStorage becomes INVALID_ENUM; needs src/gl/framebuffer.rs is_renderbuffer_color_format"]
fn r32f_renderbuffer_storage_is_invalid_enum() {
    let _g = context();
    let (_, storage, n) = renderbuffer_status(GL_R32F);
    assert_eq!(storage, GL_INVALID_ENUM, "{GPU}");
    release_renderbuffer(n);
}

#[test]
fn r32f_texture_still_uploads() {
    // Section 5, the texture side: R32F stays creatable; only the colour
    // attachment changes.
    let _g = context();
    let px = [1.0f32; 16];
    let mut tex = 0;
    // SAFETY: 16 floats are a 4x4 R32F image.
    unsafe {
        glGenTextures(1, &mut tex);
        glBindTexture(GL_TEXTURE_2D, tex);
        glTexImage2D(GL_TEXTURE_2D, 0, GL_R32F as GLint, 4, 4, 0, GL_RED, GL_FLOAT, px.as_ptr() as *const c_void);
    }
    assert_eq!(err(), GL_NO_ERROR, "{GPU}");
    // SAFETY: name made above.
    unsafe { glDeleteTextures(1, &tex) };
}
