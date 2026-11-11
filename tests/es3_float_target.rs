// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The ES 3.0 float colour target decision of docs/notes/es3-float-targets.md
//! (option A+B, sections 4 and 5): dEQP must see NotSupported or a working
//! float target, never an incomplete framebuffer it did not ask for.
//!
//! - `GL_EXT_color_buffer_float` is claimed on no GPU (PowerVR, GC7000UL,
//!   V3D: section 3 has no 32-bit float target on any of our back ends), so
//!   it is absent from `glGetString(GL_EXTENSIONS)` and `glGetStringi`, and
//!   dEQP answers its 32-bit and `r11f_g11f_b10f` rows NotSupported.
//! - `GL_EXT_color_buffer_half_float` stays claimed (option B), so the 16F
//!   targets are complete.
//! - Without the extension the 32-bit float formats follow ES 3.0 core for a
//!   format that is not colour-renderable (ES 3.0.6 table 3.13, 4.4.2,
//!   4.4.4.1): a texture attachment is `GL_FRAMEBUFFER_INCOMPLETE_ATTACHMENT`
//!   and `glRenderbufferStorage` is `GL_INVALID_ENUM`.
//! - The RGBA8 and RGBA32UI targets of `shaders.derivate` stay complete.
//!
//! The rule is GL core and the same on every GPU, so the file runs in every
//! ES 3.0 host build: the PowerVR and GC7000 rows of tools/check.sh take it
//! with the rest of tests/, and the V3D build by hand:
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_float_target
//! ```
//!
//! R32F is the one format where today's code and the note differ: it is
//! complete without the extension (section 3, "Mismatch"). The test that
//! pins the note's answer for it is `#[ignore]`d, naming section 5, and a
//! passing test pins today's answer, so the fix-float code change has to
//! turn both. No GPU claims the extension, so there is no render and
//! readback test of a 32-bit float target here (section 4, C and D).
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

/// The 32-bit float colour formats ES 3.0 makes colour-renderable only with
/// `GL_EXT_color_buffer_float` (R11F_G11F_B10F with them), with a (format,
/// type) glTexImage2D takes for each. R32F is apart: see the module text.
const FLOAT32: [(GLenum, GLenum, GLenum, &str); 3] = [
    (GL_RGBA32F, GL_RGBA, GL_FLOAT, "RGBA32F"),
    (GL_RG32F, GL_RG, GL_FLOAT, "RG32F"),
    (GL_R11F_G11F_B10F, GL_RGB, GL_UNSIGNED_INT_10F_11F_11F_REV, "R11F_G11F_B10F"),
];

/// RGB32F: a texture format only, colour-renderable with no extension at
/// all (table 3.13), so incomplete and no renderbuffer storage whatever is
/// claimed.
const RGB32F: (GLenum, GLenum, GLenum, &str) = (GL_RGB32F, GL_RGB, GL_FLOAT, "RGB32F");

/// The half-float formats `GL_EXT_color_buffer_half_float` makes
/// colour-renderable, and RGB16F, which that extension adds as well.
const FLOAT16: [(GLenum, GLenum, GLenum, &str); 4] = [
    (GL_RGBA16F, GL_RGBA, GL_HALF_FLOAT, "RGBA16F"),
    (GL_RGB16F, GL_RGB, GL_HALF_FLOAT, "RGB16F"),
    (GL_RG16F, GL_RG, GL_HALF_FLOAT, "RG16F"),
    (GL_R16F, GL_RED, GL_HALF_FLOAT, "R16F"),
];

const R32F: (GLenum, GLenum, GLenum, &str) = (GL_R32F, GL_RED, GL_FLOAT, "R32F");

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

/* ---- (a) the extension strings ------------------------------------------ */

/// Section 5: no GPU claims `GL_EXT_color_buffer_float`, every one keeps
/// `GL_EXT_color_buffer_half_float`, in both forms of the list.
#[test]
fn color_buffer_float_unclaimed_half_float_claimed() {
    let _g = context();
    assert!(!in_extension_string("GL_EXT_color_buffer_float"), "{GPU}: glGetString lists GL_EXT_color_buffer_float");
    assert!(!in_extension_list("GL_EXT_color_buffer_float"), "{GPU}: glGetStringi lists GL_EXT_color_buffer_float");
    assert!(in_extension_string("GL_EXT_color_buffer_half_float"), "{GPU}: glGetString");
    assert!(in_extension_list("GL_EXT_color_buffer_half_float"), "{GPU}: glGetStringi");
}

/* ---- (b) the 32-bit float formats follow ES 3.0 core -------------------- */

/// ES 3.0.6 4.4.4.1: an image that is not colour-renderable makes the
/// attachment incomplete; `GL_FRAMEBUFFER_UNSUPPORTED` would be wrong.
#[test]
fn float32_texture_attachment_incomplete() {
    let _g = context();
    for f in FLOAT32.into_iter().chain([RGB32F]) {
        assert_eq!(
            texture_target_status(f),
            GL_FRAMEBUFFER_INCOMPLETE_ATTACHMENT,
            "{GPU}: {} texture as colour 0 without GL_EXT_color_buffer_float",
            f.3
        );
    }
}

/// ES 3.0.6 4.4.2: glRenderbufferStorage of a format that is not colour-,
/// depth- or stencil-renderable is `GL_INVALID_ENUM`, and the renderbuffer,
/// which then has no storage, is an incomplete attachment.
#[test]
fn float32_renderbuffer_storage_invalid_enum() {
    let _g = context();
    for (f, _, _, label) in FLOAT32.into_iter().chain([RGB32F]) {
        assert_eq!(
            renderbuffer_target(f),
            (GL_INVALID_ENUM, GL_FRAMEBUFFER_INCOMPLETE_ATTACHMENT),
            "{GPU}: {label} renderbuffer without GL_EXT_color_buffer_float"
        );
    }
}

/// The note's answer for R32F (section 5, contract of fix-float: R32F
/// leaves `is_color_format` and joins RG32F and RGBA32F). Fails today, see
/// [`r32f_target_complete_today`].
#[test]
#[ignore = "docs/notes/es3-float-targets.md section 5 (fix-float code change): R32F is a complete colour target without GL_EXT_color_buffer_float, src/gl/framebuffer.rs is_color_format"]
fn r32f_target_refused_without_extension() {
    let _g = context();
    assert_eq!(texture_target_status(R32F), GL_FRAMEBUFFER_INCOMPLETE_ATTACHMENT, "{GPU}: R32F texture");
    assert_eq!(
        renderbuffer_target(GL_R32F),
        (GL_INVALID_ENUM, GL_FRAMEBUFFER_INCOMPLETE_ATTACHMENT),
        "{GPU}: R32F renderbuffer"
    );
}

/// Today's answer for R32F, the mismatch of section 3: complete as a texture
/// and as a renderbuffer, though the extension is not claimed (dEQP
/// `fbo.completeness.renderable.{renderbuffer,texture}.color0.r32f` fail on
/// it). The fix-float code change turns this test and the ignored one above.
#[test]
fn r32f_target_complete_today() {
    let _g = context();
    assert_eq!(texture_target_status(R32F), GL_FRAMEBUFFER_COMPLETE, "{GPU}: R32F texture");
    assert_eq!(renderbuffer_target(GL_R32F), (GL_NO_ERROR, GL_FRAMEBUFFER_COMPLETE), "{GPU}: R32F renderbuffer");
}

/* ---- the half-float targets the claimed extension promises -------------- */

/// Option B: `GL_EXT_color_buffer_half_float` is claimed, so each 16F format
/// is a complete colour target: all four as a renderbuffer, and RGBA16F,
/// RG16F and R16F as a texture (RGB16F's texture: the two tests below).
#[test]
fn float16_targets_complete() {
    let _g = context();
    for f in FLOAT16 {
        if f.0 != GL_RGB16F {
            assert_eq!(texture_target_status(f), GL_FRAMEBUFFER_COMPLETE, "{GPU}: {} texture", f.3);
        }
        assert_eq!(renderbuffer_target(f.0), (GL_NO_ERROR, GL_FRAMEBUFFER_COMPLETE), "{GPU}: {} renderbuffer", f.3);
    }
}

/// The note's list of what stays complete names RGB16F for both kinds
/// (section 5, "Tests" (3)), and so does Mesa with the half-float extension
/// (src/mesa/main/fbobject.c `is_format_color_renderable`, GL_RGB16F). Fails
/// today, see [`rgb16f_texture_incomplete_today`].
#[test]
#[ignore = "docs/notes/es3-float-targets.md section 5, Tests (3): an RGB16F texture is GL_FRAMEBUFFER_INCOMPLETE_ATTACHMENT today (src/gl/framebuffer.rs attachment_storage); no dEQP row is measured failing on it"]
fn rgb16f_texture_complete() {
    let _g = context();
    assert_eq!(texture_target_status(FLOAT16[1]), GL_FRAMEBUFFER_COMPLETE, "{GPU}: RGB16F texture");
}

/// Today's answer for an RGB16F texture attachment: incomplete, while the
/// RGB16F renderbuffer is complete. No board list has
/// `fbo.completeness.renderable.texture.color0.rgb16f` failing.
#[test]
fn rgb16f_texture_incomplete_today() {
    let _g = context();
    assert_eq!(texture_target_status(FLOAT16[1]), GL_FRAMEBUFFER_INCOMPLETE_ATTACHMENT, "{GPU}: RGB16F texture");
}

/* ---- (c) the shaders.derivate targets stay complete --------------------- */

/// `shaders.derivate` renders into one-sample RGBA8 (`fbo`, and the resolve
/// target of `fbo_msaa*`) and RGBA32UI (`fbo_float`, `float_*`): both stay
/// complete colour targets. Whether a GPU then draws into RGBA32UI is the
/// back end's (the GC7000UL refuses it, section 1, plan #335), not this rule.
/// The GC7000UL samples no RGBA32UI texture either (`GC_SAMPLED_TEXFMTS`:
/// glTexImage2D is GL_INVALID_OPERATION there), so it has the renderbuffer
/// half only; that is the integer-target work of #335, not a float question.
#[test]
fn derivate_targets_complete() {
    let _g = context();
    for f in
        [(GL_RGBA8, GL_RGBA, GL_UNSIGNED_BYTE, "RGBA8"), (GL_RGBA32UI, GL_RGBA_INTEGER, GL_UNSIGNED_INT, "RGBA32UI")]
    {
        if !(cfg!(feature = "gpu-vivante") && f.0 == GL_RGBA32UI) {
            assert_eq!(texture_target_status(f), GL_FRAMEBUFFER_COMPLETE, "{GPU}: {} texture", f.3);
        }
        assert_eq!(renderbuffer_target(f.0), (GL_NO_ERROR, GL_FRAMEBUFFER_COMPLETE), "{GPU}: {} renderbuffer", f.3);
    }
}
