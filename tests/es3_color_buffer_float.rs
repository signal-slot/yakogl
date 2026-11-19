// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Plan #360, items `cbf-test-draft` and `core-surf`: the
//! `GL_EXT_color_buffer_float` contract. What holds today, with the extension
//! unclaimed, is pinned by passing tests. What the GL core does once the
//! extension is listed (completeness, the two storage calls, glReadPixels'
//! pairs and values, the getters, the 32F blend rule) runs in every build
//! through the host knob (`framebuffer::HOST_R32F_RT` with
//! `backend::HOST_FLOAT_RT_FORMATS`, the `Claimed` guard); the claim itself
//! (the `color-buffer-float` Cargo feature on a back end that draws the
//! formats, item `v3d-arm`) is checked by `claimed_path`. No test is
//! `#[ignore]`d (the gate refuses an ignored test) and none is `#[cfg]`ed out
//! (`tests_as_declared` reads it as "declared, did not run"): every `#[test]`
//! exists in every build and the feature-dependent part of its body is
//! `#[cfg]`ed.
//!
//! Rules (docs/notes/es3-float-targets.md; ES 3.0.6 table 3.13, 4.3.2, 4.4.2,
//! 4.4.4.1; EXT_color_buffer_float):
//!
//! - Without the extension R32F, RG32F, RGBA32F and R11F_G11F_B10F are not
//!   colour-renderable: a texture attachment is
//!   `GL_FRAMEBUFFER_INCOMPLETE_ATTACHMENT`, `glRenderbufferStorage` is
//!   `GL_INVALID_ENUM` (R32F was complete until plan #325's board-mrt).
//! - With it, all seven formats (R16F, RG16F, RGBA16F, R32F, RG32F, RGBA32F,
//!   R11F_G11F_B10F) are complete, and `glReadPixels` takes RGBA/FLOAT and
//!   the implementation pair, RGBA/UNSIGNED_BYTE for every float buffer.
//!   RG32F, RGBA32F and R11F_G11F_B10F also need their surface drawable
//!   (`backend::rt_format_drawable`), which the claim implies.
//! - A draw with GL_BLEND on and an R32F, RG32F or RGBA32F draw buffer is
//!   `GL_INVALID_OPERATION` (no `GL_EXT_float_blend`).
//! - `GL_EXT_color_buffer_half_float` is claimed today; the 16F answers of
//!   the current tip are recorded as they are.
//!
//! The `color-buffer-float` feature is declared in Cargo.toml (off by
//! default; `gl/state.rs` lists the extension only under it, on a back end
//! where `backend::color_buffer_float_capable()` holds): the "not claimed" assertions are
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
use std::sync::atomic::Ordering;
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{self, GlesDraw, GlesHeap, YakoglBackend};
use yakogl::gl::consts::*;
use yakogl::gl::context;
use yakogl::gl::float_pack;
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
    max_samples: 0,
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
            #[cfg(feature = "shaderc-online")]
            {
                yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
            }
            assert!(!context::gl_context_create(16, 16).is_null());
        });
        /* the host knob is off unless a test's `Claimed` guard is alive */
        HOST_R32F_RT.store(false, Ordering::Relaxed);
        backend::HOST_FLOAT_RT_FORMATS.store(0, Ordering::Relaxed);
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

/// With the `color-buffer-float` feature on, the extension is still unlisted
/// while the build's back end cannot draw the formats
/// (`backend::color_buffer_float_capable()`, false on the PowerVR and the
/// GC7000UL): the feature alone claims nothing. The V3D draws them (item
/// `v3d-arm`): its build is capable with the feature, and only with it, so
/// the V3D build without the feature answers as before.
#[test]
fn feature_on_not_claimed_while_incapable() {
    let _g = context();
    assert_eq!(
        in_extension_string("GL_EXT_color_buffer_float"),
        cfg!(feature = "color-buffer-float") && backend::color_buffer_float_capable(),
        "{GPU}: glGetString"
    );
    assert_eq!(
        in_extension_list("GL_EXT_color_buffer_float"),
        cfg!(feature = "color-buffer-float") && backend::color_buffer_float_capable(),
        "{GPU}: glGetStringi"
    );
    #[cfg(not(feature = "gpu-v3d"))]
    assert!(!backend::color_buffer_float_capable(), "{GPU}: capability");
    #[cfg(feature = "gpu-v3d")]
    {
        assert_eq!(backend::color_buffer_float_capable(), cfg!(feature = "color-buffer-float"), "{GPU}: capability");
        assert_eq!(backend::FLOAT_RT_FORMATS, if cfg!(feature = "color-buffer-float") { 0b111 } else { 0 }, "{GPU}");
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
fn r32f_rg32f_rgba32f_r11g11b10_incomplete() {
    let _g = context();
    #[cfg(not(feature = "color-buffer-float"))]
    for f in [SEVEN[3], SEVEN[4], SEVEN[5], SEVEN[6]] {
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
        assert_eq!(multisample_storage_error(f.0), GL_INVALID_ENUM, "{GPU}: {} multisample storage", f.3);
    }
}

/* ---- (c) R16F, RG16F and RGBA16F, which the half-float extension gives -- */

/// The three 16F formats are complete as texture and as renderbuffer
/// (`GL_EXT_color_buffer_half_float`).
#[test]
fn r16f_rg16f_rgba16f_complete() {
    let _g = context();
    for f in [SEVEN[0], SEVEN[1], SEVEN[2]] {
        assert_eq!(texture_target_status(f), GL_FRAMEBUFFER_COMPLETE, "{GPU}: {} texture", f.3);
        assert_eq!(renderbuffer_target(f.0), (GL_NO_ERROR, GL_FRAMEBUFFER_COMPLETE), "{GPU}: {} renderbuffer", f.3);
    }
}

/* ---- the claimed path, through the host knob ----------------------------- */

/// The GL core as a build that lists GL_EXT_color_buffer_float on a back end
/// that draws every surface of it: `framebuffer::HOST_R32F_RT` (the
/// extension's rule) and `backend::HOST_FLOAT_RT_FORMATS` (the RG32F, RGBA32F
/// and R11F_G11F_B10F surfaces drawable), on while the guard lives. The
/// extension string is not changed; the claim itself (item `v3d-arm`) is
/// checked by [`claimed_path`].
struct Claimed;

impl Claimed {
    fn on() -> Claimed {
        HOST_R32F_RT.store(true, Ordering::Relaxed);
        backend::HOST_FLOAT_RT_FORMATS.store(0b111, Ordering::Relaxed);
        Claimed
    }
}

impl Drop for Claimed {
    fn drop(&mut self) {
        HOST_R32F_RT.store(false, Ordering::Relaxed);
        backend::HOST_FLOAT_RT_FORMATS.store(0, Ordering::Relaxed);
    }
}

/// glRenderbufferStorageMultisample of `internalformat` (0 samples) into a
/// new bound renderbuffer: its error.
fn multisample_storage_error(internalformat: GLenum) -> GLenum {
    let mut rb = 0;
    // SAFETY: room for one name; plain GL calls.
    unsafe {
        glGenRenderbuffers(1, &mut rb);
        glBindRenderbuffer(GL_RENDERBUFFER, rb);
        glRenderbufferStorageMultisample(GL_RENDERBUFFER, 0, internalformat, 16, 16);
        let e = err();
        glDeleteRenderbuffers(1, &rb);
        e
    }
}

/// With the extension's rule all seven formats are complete colour
/// attachments, as textures and as renderbuffers (ES 3.0.6 4.4.4.1 with
/// EXT_color_buffer_float), and both storage calls take them.
#[test]
fn claimed_all_seven_complete() {
    let _g = context();
    let _c = Claimed::on();
    for f in SEVEN {
        assert_eq!(texture_target_status(f), GL_FRAMEBUFFER_COMPLETE, "{GPU}: {} texture", f.3);
        assert_eq!(renderbuffer_target(f.0), (GL_NO_ERROR, GL_FRAMEBUFFER_COMPLETE), "{GPU}: {} renderbuffer", f.3);
        assert_eq!(multisample_storage_error(f.0), GL_NO_ERROR, "{GPU}: {} multisample storage", f.3);
    }
}

/// The rule needs the surface too: with the extension's rule but no RG32F,
/// RGBA32F or R11F_G11F_B10F surface the back end draws, those three stay
/// out (GL_INVALID_ENUM, incomplete) and R32F is in.
#[test]
fn claimed_rule_without_a_surface_keeps_the_three_out() {
    let _g = context();
    let _c = Claimed::on();
    backend::HOST_FLOAT_RT_FORMATS.store(0, Ordering::Relaxed);
    if backend::FLOAT_RT_FORMATS != 0 {
        return; // a back end that draws them (v3d-arm): nothing to take out
    }
    assert_eq!(renderbuffer_target(GL_R32F), (GL_NO_ERROR, GL_FRAMEBUFFER_COMPLETE), "{GPU}: R32F");
    for f in [SEVEN[4], SEVEN[5], SEVEN[6]] {
        assert_eq!(texture_target_status(f), GL_FRAMEBUFFER_INCOMPLETE_ATTACHMENT, "{GPU}: {} texture", f.3);
        assert_eq!(
            renderbuffer_target(f.0),
            (GL_INVALID_ENUM, GL_FRAMEBUFFER_INCOMPLETE_ATTACHMENT),
            "{GPU}: {} renderbuffer",
            f.3
        );
    }
}

/// A 4 x 4 texture of `f` filled with one pixel's bytes, attached as colour
/// 0 of a new bound framebuffer: (framebuffer, texture).
fn filled_texture_target(f: (GLenum, GLenum, GLenum, &str), pixel: &[u8]) -> (GLuint, GLuint) {
    let (mut fb, mut t) = (0, 0);
    let data: Vec<u8> = pixel.iter().copied().cycle().take(pixel.len() * 16).collect();
    // SAFETY: room for one name each; `data` holds 16 pixels; plain GL calls.
    unsafe {
        glGenTextures(1, &mut t);
        glBindTexture(GL_TEXTURE_2D, t);
        glTexImage2D(GL_TEXTURE_2D, 0, f.0 as GLint, 4, 4, 0, f.1, f.2, data.as_ptr() as *const c_void);
        assert_eq!(err(), GL_NO_ERROR, "{GPU}: {} texture", f.3);
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, t, 0);
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE, "{GPU}: {}", f.3);
    }
    (fb, t)
}

fn drop_target(fb: GLuint, t: GLuint) {
    // SAFETY: names made by `filled_texture_target`.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glDeleteFramebuffers(1, &fb);
        glDeleteTextures(1, &t);
    }
}

fn halves(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|&x| float_pack::f32_to_half(x).to_le_bytes()).collect()
}

fn floats(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|x| x.to_le_bytes()).collect()
}

/// glReadPixels of each of the seven, with the extension's rule: RGBA/FLOAT
/// gives the stored values, the channels the format lacks 0, 0, 1 (ES 3.0.6
/// 4.3.2; the 16F and R11F_G11F_B10F ones decoded by `gl/float_pack.rs`);
/// GL_IMPLEMENTATION_COLOR_READ_FORMAT / _TYPE is RGBA / UNSIGNED_BYTE for
/// every float buffer (`framebuffer::implementation_color_read`) and that
/// pair reads the values clamped; a pair of neither kind is
/// GL_INVALID_OPERATION.
#[test]
fn claimed_readpixels_float_pairs() {
    let _g = context();
    let _c = Claimed::on();
    let r11 = float_pack::pack_r11g11b10f([0.5, 0.25, 0.75]).to_le_bytes().to_vec();
    let cases: [(usize, Vec<u8>, [f32; 4]); 7] = [
        (0, halves(&[0.5]), [0.5, 0.0, 0.0, 1.0]),
        (1, halves(&[0.5, -0.25]), [0.5, -0.25, 0.0, 1.0]),
        (2, halves(&[0.5, -0.25, 0.75, 0.125]), [0.5, -0.25, 0.75, 0.125]),
        (3, floats(&[0.1]), [0.1, 0.0, 0.0, 1.0]),
        (4, floats(&[0.1, -3.5]), [0.1, -3.5, 0.0, 1.0]),
        (5, floats(&[0.1, -3.5, 1.0e5, 0.125]), [0.1, -3.5, 1.0e5, 0.125]),
        (6, r11, [0.5, 0.25, 0.75, 1.0]),
    ];
    for (i, pixel, want) in cases {
        let f = SEVEN[i];
        let (fb, t) = filled_texture_target(f, &pixel);
        let mut px = [f32::NAN; 4];
        let mut bytes = [0u8; 4];
        let (mut rf, mut rt) = (0, 0);
        // SAFETY: room for one RGBA float pixel, one RGBA byte pixel and one
        // integer each; plain GL calls.
        unsafe {
            glReadPixels(1, 2, 1, 1, GL_RGBA, GL_FLOAT, px.as_mut_ptr() as *mut c_void);
            assert_eq!(err(), GL_NO_ERROR, "{GPU}: {} RGBA/FLOAT", f.3);
            assert_eq!(px, want, "{GPU}: {} RGBA/FLOAT", f.3);
            glGetIntegerv(GL_IMPLEMENTATION_COLOR_READ_FORMAT, &mut rf);
            glGetIntegerv(GL_IMPLEMENTATION_COLOR_READ_TYPE, &mut rt);
            assert_eq!((rf as GLenum, rt as GLenum), (GL_RGBA, GL_UNSIGNED_BYTE), "{GPU}: {} pair", f.3);
            glReadPixels(1, 2, 1, 1, GL_RGBA, GL_UNSIGNED_BYTE, bytes.as_mut_ptr() as *mut c_void);
            assert_eq!(err(), GL_NO_ERROR, "{GPU}: {} RGBA/UNSIGNED_BYTE", f.3);
            let clamped = want.map(|v| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8);
            assert_eq!(bytes, clamped, "{GPU}: {} RGBA/UNSIGNED_BYTE", f.3);
            for (format, type_) in [(GL_RGBA_INTEGER, GL_INT), (GL_RGBA, GL_HALF_FLOAT), (GL_RGB, GL_FLOAT)] {
                glReadPixels(1, 2, 1, 1, format, type_, px.as_mut_ptr() as *mut c_void);
                assert_eq!(err(), GL_INVALID_OPERATION, "{GPU}: {} {format:#x}/{type_:#x}", f.3);
            }
        }
        drop_target(fb, t);
    }
}

/// glGetFramebufferAttachmentParameteriv of colour attachment 0.
fn attachment_param(pname: GLenum) -> GLint {
    let mut v = -1;
    // SAFETY: one integer out.
    unsafe { glGetFramebufferAttachmentParameteriv(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, pname, &mut v) };
    assert_eq!(err(), GL_NO_ERROR, "{GPU}: {pname:#x}");
    v
}

/// The four channel sizes of colour attachment 0.
fn attachment_sizes() -> [GLint; 4] {
    [
        GL_FRAMEBUFFER_ATTACHMENT_RED_SIZE,
        GL_FRAMEBUFFER_ATTACHMENT_GREEN_SIZE,
        GL_FRAMEBUFFER_ATTACHMENT_BLUE_SIZE,
        GL_FRAMEBUFFER_ATTACHMENT_ALPHA_SIZE,
    ]
    .map(attachment_param)
}

/// glGetRenderbufferParameteriv of the bound renderbuffer.
fn renderbuffer_param(pname: GLenum) -> GLint {
    let mut v = -1;
    // SAFETY: one integer out.
    unsafe { glGetRenderbufferParameteriv(GL_RENDERBUFFER, pname, &mut v) };
    v
}

/// The channel sizes and component type each of the seven reports, as a
/// renderbuffer (glGetRenderbufferParameteriv and the attachment query) and
/// as a texture attachment: GL_FLOAT, the format's own sizes.
#[test]
fn claimed_getters_match_the_formats() {
    let _g = context();
    let _c = Claimed::on();
    let sizes: [[GLint; 4]; 7] = [
        [16, 0, 0, 0],
        [16, 16, 0, 0],
        [16, 16, 16, 16],
        [32, 0, 0, 0],
        [32, 32, 0, 0],
        [32, 32, 32, 32],
        [11, 11, 10, 0],
    ];
    for (f, want) in SEVEN.into_iter().zip(sizes) {
        let (mut fb, mut rb) = (0, 0);
        // SAFETY: room for one name each; plain GL calls.
        unsafe {
            glGenRenderbuffers(1, &mut rb);
            glBindRenderbuffer(GL_RENDERBUFFER, rb);
            glRenderbufferStorage(GL_RENDERBUFFER, f.0, 4, 4);
            glGenFramebuffers(1, &mut fb);
            glBindFramebuffer(GL_FRAMEBUFFER, fb);
            glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_RENDERBUFFER, rb);
        }
        assert_eq!(err(), GL_NO_ERROR, "{GPU}: {} renderbuffer", f.3);
        let rb_sizes = [
            GL_RENDERBUFFER_RED_SIZE,
            GL_RENDERBUFFER_GREEN_SIZE,
            GL_RENDERBUFFER_BLUE_SIZE,
            GL_RENDERBUFFER_ALPHA_SIZE,
        ]
        .map(renderbuffer_param);
        assert_eq!(rb_sizes, want, "{GPU}: {} renderbuffer sizes", f.3);
        assert_eq!(
            renderbuffer_param(GL_RENDERBUFFER_INTERNAL_FORMAT) as GLenum,
            f.0,
            "{GPU}: {} internal format",
            f.3
        );
        assert_eq!(attachment_sizes(), want, "{GPU}: {} renderbuffer attachment sizes", f.3);
        let ct = attachment_param(GL_FRAMEBUFFER_ATTACHMENT_COMPONENT_TYPE) as GLenum;
        assert_eq!(ct, GL_FLOAT, "{GPU}: {} renderbuffer component type", f.3);
        // SAFETY: names made above.
        unsafe {
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
            glDeleteFramebuffers(1, &fb);
            glDeleteRenderbuffers(1, &rb);
        }
        let (fb, t) = filled_texture_target(f, &[0u8; 16][..pixel_bytes(f)]);
        assert_eq!(attachment_sizes(), want, "{GPU}: {} texture attachment sizes", f.3);
        let ct = attachment_param(GL_FRAMEBUFFER_ATTACHMENT_COMPONENT_TYPE) as GLenum;
        assert_eq!(ct, GL_FLOAT, "{GPU}: {} texture component type", f.3);
        drop_target(fb, t);
    }
}

/// The bytes of one pixel of `f`'s (format, type) in [`SEVEN`].
fn pixel_bytes(f: (GLenum, GLenum, GLenum, &str)) -> usize {
    let channels = match f.1 {
        GL_RED => 1,
        GL_RG => 2,
        _ => 4,
    };
    match f.2 {
        GL_HALF_FLOAT => 2 * channels,
        GL_FLOAT => 4 * channels,
        _ => 4, /* the packed R11F_G11F_B10F word */
    }
}

/// EXT_color_buffer_float: with GL_BLEND on and a draw buffer of R32F, RG32F
/// or RGBA32F, a draw is GL_INVALID_OPERATION (no GL_EXT_float_blend); the
/// 16F formats and R11F_G11F_B10F blend; a 32F image behind a GL_NONE draw
/// buffer does not count. The draws have no vertices: the rule is checked
/// before anything reaches the back end. The program needs the in-process
/// compiler (`shaderc-online`, in every feature set of the header).
#[test]
fn claimed_blend_of_a_32f_draw_buffer_is_invalid_operation() {
    let _g = context();
    #[cfg(feature = "shaderc-online")]
    {
        let _c = Claimed::on();
        assert!(!in_extension_string("GL_EXT_float_blend"), "{GPU}: GL_EXT_float_blend is not claimed");
        draws::use_program();
        for f in SEVEN {
            let target = draws::bound_renderbuffer_target(&[f.0]);
            let blended =
                if matches!(f.0, GL_R32F | GL_RG32F | GL_RGBA32F) { GL_INVALID_OPERATION } else { GL_NO_ERROR };
            assert_eq!(draws::draw_nothing(false), GL_NO_ERROR, "{GPU}: {} unblended", f.3);
            assert_eq!(draws::draw_nothing(true), blended, "{GPU}: {} blended", f.3);
            draws::drop_renderbuffer_target(target);
        }
        /* RGBA16F at 0, RGBA32F at 1: the second draw buffer counts, until
         * glDrawBuffers takes it out */
        let target = draws::bound_renderbuffer_target(&[GL_RGBA16F, GL_RGBA32F]);
        // SAFETY: plain GL calls with two-element arrays.
        unsafe { glDrawBuffers(2, [GL_COLOR_ATTACHMENT0, GL_COLOR_ATTACHMENT1].as_ptr()) };
        assert_eq!(draws::draw_nothing(true), GL_INVALID_OPERATION, "{GPU}: RGBA32F as draw buffer 1");
        // SAFETY: as above.
        unsafe { glDrawBuffers(2, [GL_COLOR_ATTACHMENT0, GL_NONE].as_ptr()) };
        assert_eq!(draws::draw_nothing(true), GL_NO_ERROR, "{GPU}: RGBA32F behind GL_NONE");
        draws::drop_renderbuffer_target(target);
        // SAFETY: plain GL call.
        unsafe { yakogl::gl::program::glUseProgram(0) };
    }
}

/// What the blend test draws with.
#[cfg(feature = "shaderc-online")]
mod draws {
    use super::*;
    use yakogl::gl::draw::glDrawArrays;
    use yakogl::gl::program::*;
    use yakogl::gl::shader::*;
    use yakogl::gl::state::{glDisable, glEnable};

    const VS: &str = "#version 300 es\nin vec4 p;\nvoid main() { gl_Position = p; }\n";
    const FS: &str = concat!(
        "#version 300 es\nprecision highp float;\n",
        "layout(location = 0) out vec4 o0;\nlayout(location = 1) out vec4 o1;\n",
        "void main() { o0 = vec4(0.5); o1 = vec4(0.25); }\n"
    );

    fn shader(ty: GLenum, text: &str) -> GLuint {
        let text = std::ffi::CString::new(text).unwrap();
        // SAFETY: plain GL calls; one NUL-terminated source.
        unsafe {
            let s = glCreateShader(ty);
            let p: [*const c_char; 1] = [text.as_ptr()];
            glShaderSource(s, 1, p.as_ptr(), core::ptr::null());
            glCompileShader(s);
            let mut ok: GLint = 0;
            glGetShaderiv(s, GL_COMPILE_STATUS, &mut ok);
            assert_eq!(ok, 1, "{GPU}: shader did not compile");
            s
        }
    }

    pub fn use_program() {
        let (v, f) = (shader(GL_VERTEX_SHADER, VS), shader(GL_FRAGMENT_SHADER, FS));
        // SAFETY: plain GL calls.
        unsafe {
            let p = glCreateProgram();
            glAttachShader(p, v);
            glAttachShader(p, f);
            glLinkProgram(p);
            let mut ok: GLint = 0;
            glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
            assert_eq!(ok, 1, "{GPU}: link");
            glUseProgram(p);
        }
    }

    /// A new bound framebuffer whose colour attachments 0.. are new 4 x 4
    /// renderbuffers of `formats`, complete: (framebuffer, renderbuffers).
    pub fn bound_renderbuffer_target(formats: &[GLenum]) -> (GLuint, Vec<GLuint>) {
        let mut fb = 0;
        let mut rbs = vec![0; formats.len()];
        // SAFETY: room for the names; plain GL calls.
        unsafe {
            glGenFramebuffers(1, &mut fb);
            glBindFramebuffer(GL_FRAMEBUFFER, fb);
            glGenRenderbuffers(rbs.len() as GLsizei, rbs.as_mut_ptr());
            for (n, (&rb, &f)) in rbs.iter().zip(formats).enumerate() {
                glBindRenderbuffer(GL_RENDERBUFFER, rb);
                glRenderbufferStorage(GL_RENDERBUFFER, f, 4, 4);
                glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0 + n as GLenum, GL_RENDERBUFFER, rb);
            }
            assert_eq!(err(), GL_NO_ERROR, "{GPU}: {formats:#x?}");
            assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE, "{GPU}: {formats:#x?}");
        }
        (fb, rbs)
    }

    pub fn drop_renderbuffer_target((fb, rbs): (GLuint, Vec<GLuint>)) {
        // SAFETY: names made by `bound_renderbuffer_target`.
        unsafe {
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
            glDeleteFramebuffers(1, &fb);
            glDeleteRenderbuffers(rbs.len() as GLsizei, rbs.as_ptr());
        }
    }

    /// glDrawArrays of no vertices with GL_BLEND on or off: its error.
    pub fn draw_nothing(blend: bool) -> GLenum {
        // SAFETY: plain GL calls.
        unsafe {
            if blend {
                glEnable(GL_BLEND);
            } else {
                glDisable(GL_BLEND);
            }
            glDrawArrays(GL_TRIANGLES, 0, 0);
            let e = glGetError();
            glDisable(GL_BLEND);
            e
        }
    }
}

/// The claim itself, where a build makes it (the `color-buffer-float`
/// feature on a back end that draws the formats, item `v3d-arm`): both names
/// are listed and the seven formats are complete without the host knob.
#[test]
fn claimed_path() {
    let _g = context();
    #[cfg(feature = "color-buffer-float")]
    if backend::color_buffer_float_capable() {
        assert!(in_extension_string("GL_EXT_color_buffer_float"), "{GPU}: glGetString");
        assert!(in_extension_list("GL_EXT_color_buffer_float"), "{GPU}: glGetStringi");
        for f in SEVEN {
            assert_eq!(texture_target_status(f), GL_FRAMEBUFFER_COMPLETE, "{GPU}: {} texture", f.3);
            assert_eq!(renderbuffer_target(f.0), (GL_NO_ERROR, GL_FRAMEBUFFER_COMPLETE), "{GPU}: {} renderbuffer", f.3);
        }
    }
}
