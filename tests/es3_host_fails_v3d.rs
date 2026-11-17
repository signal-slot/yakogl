// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The host-side Fails of the Pi 4's A1 and M1 dEQP-GLES3 images on the V3D
//! build, pinned through the GL entry points (plan #380, item pin-a1m1).
//!
//! Images (tools/cts/pi4-gles3-tree.txt) and cases. The rerun map finds 2
//! Fails in A1 and 2 API Fails in M1 that main has not fixed; the other board
//! Fails of the two images (the uniform-count and varying rows,
//! `negative_api.texture.texsubimage*`, `negative_api.shader.uniformui*`, the
//! compressed-format rows) are fixed on the host and are not here.
//!
//! * A1 `implementation_limits.max_draw_buffers` and
//!   `implementation_limits.max_color_attachments`: dEQP reads the limit
//!   with `glGetIntegerv` and wants at least 4 (ES 3.0 table 6.33). The V3D
//!   build answers 3 (`DRAW_TARGETS` in src/backend.rs, read by
//!   `max_draw_buffers` and `max_color_attachments` in src/gl/context.rs).
//!   One cause, one test: `four_draw_buffers_are_not_offered`.
//! * M1 `negative_api.buffer.blit_framebuffer_multisample` and
//!   `negative_api.buffer.read_pixels_fbo_format_mismatch`: both first make
//!   a 4-sample RGBA8 renderbuffer with `glRenderbufferStorageMultisample`
//!   and `expectError(GL_NO_ERROR)` ("Got invalid error"). The GL core
//!   answers GL_INVALID_OPERATION, because `msaa::backend_max_samples`
//!   (src/gl/msaa.rs) is 0 and `msaa::stored_samples` refuses any count
//!   above `GL_MAX_SAMPLES` (1). `glRenderbufferStorageMultisample`
//!   (src/gl/framebuffer.rs) raises it. One cause, one test:
//!   `a_four_sample_renderbuffer_is_refused`.
//!
//! Not pinned: M1's `multisample.fbo_max_samples.*` Fails are pixel verdicts
//! (the host reads black and cannot show them), and
//! `negative_api.buffer.blit_framebuffer` is not a host Fail. The
//! `multisample.*` NotSupported rows are no Fail.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_host_fails_v3d
//! ```
#![cfg(all(feature = "host-test", feature = "gpu-v3d", not(feature = "report-es2")))]

use core::ffi::{c_char, c_void};
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{GlesHeap, YakoglBackend};
use yakogl::gl::consts::*;
use yakogl::gl::framebuffer::{
    glBindFramebuffer, glBindRenderbuffer, glCheckFramebufferStatus, glDrawBuffers, glFramebufferRenderbuffer,
    glGenFramebuffers, glGenRenderbuffers, glGetRenderbufferParameteriv, glRenderbufferStorage,
    glRenderbufferStorageMultisample,
};
use yakogl::gl::state::{glGetError, glGetIntegerv};
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
    name: c"es3-host-fails-v3d".as_ptr() as *const c_char,
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

fn clear_errors() {
    // SAFETY: plain GL call.
    unsafe { while glGetError() != GL_NO_ERROR {} }
}

fn error() -> GLenum {
    // SAFETY: plain GL call.
    unsafe { glGetError() }
}

fn geti(pname: GLenum) -> i32 {
    clear_errors();
    let mut v = i32::MIN;
    // SAFETY: one value.
    unsafe { glGetIntegerv(pname, &mut v) };
    assert_ne!(v, i32::MIN, "glGetIntegerv(0x{pname:x}) wrote nothing");
    v
}

/// A1: implementation_limits.max_draw_buffers and .max_color_attachments.
/// dEQP's check is `glGetIntegerv(pname) >= 4` (ES 3.0 table 6.33). The V3D
/// build answers 3, and the limit is enforced where an application would use
/// a fourth point: a fourth `glDrawBuffers` entry is GL_INVALID_VALUE, an
/// attachment 3 is GL_INVALID_ENUM (src/gl/framebuffer.rs reads
/// `context::max_color_attachments`). The cause is `DRAW_TARGETS` (3 for
/// `gpu-v3d`) in src/backend.rs, which `context::max_draw_buffers` returns.
///
/// A fix (the TLB takes four, the compiler writes four: plan #376 and
/// docs/notes/v3d-mrt-clear.md) flips the two `3`s and the two refusals, and
/// the ignored `four_draw_buffers_and_colour_attachments` of
/// tests/es3_limits_v3d.rs.
#[test]
fn four_draw_buffers_are_not_offered() {
    let _lib = context();
    assert_eq!(geti(GL_MAX_DRAW_BUFFERS), 3, "flip to 4 when the V3D draws four targets");
    assert_eq!(geti(GL_MAX_COLOR_ATTACHMENTS), 3, "flip to 4 when the V3D draws four targets");
    assert_eq!(error(), GL_NO_ERROR);

    // SAFETY: plain GL calls on objects of this test.
    unsafe {
        let mut fbo: GLuint = 0;
        let mut rbo: GLuint = 0;
        glGenFramebuffers(1, &mut fbo);
        glGenRenderbuffers(1, &mut rbo);
        glBindFramebuffer(GL_FRAMEBUFFER, fbo);
        glBindRenderbuffer(GL_RENDERBUFFER, rbo);
        glRenderbufferStorage(GL_RENDERBUFFER, GL_RGBA8, 4, 4);
        clear_errors();

        // the control: the third point (index 2) is accepted
        glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0 + 2, GL_RENDERBUFFER, rbo);
        assert_eq!(error(), GL_NO_ERROR, "attachment 2 is within the limit");

        // dEQP's limit is 4, so attachment 3 must be accepted; it is not
        glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0 + 3, GL_RENDERBUFFER, rbo);
        assert_eq!(error(), GL_INVALID_ENUM, "attachment 3 is past MAX_COLOR_ATTACHMENTS (3)");

        let four = [GL_COLOR_ATTACHMENT0, GL_COLOR_ATTACHMENT0 + 1, GL_COLOR_ATTACHMENT0 + 2, GL_COLOR_ATTACHMENT0 + 3];
        glDrawBuffers(4, four.as_ptr());
        assert_eq!(error(), GL_INVALID_VALUE, "four draw buffers are past MAX_DRAW_BUFFERS (3)");
        glDrawBuffers(3, four.as_ptr());
        assert_eq!(error(), GL_NO_ERROR, "three are accepted");
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
    }
}

/// M1: negative_api.buffer.blit_framebuffer_multisample and
/// negative_api.buffer.read_pixels_fbo_format_mismatch. Both call
/// `glRenderbufferStorageMultisample(GL_RENDERBUFFER, 4, GL_RGBA8, 32, 32)`
/// and `expectError(GL_NO_ERROR)`; dEQP's message is "Got invalid error"
/// (the second then also needs `GL_SAMPLE_BUFFERS` 1 on the framebuffer it
/// attaches to). The GL core answers GL_INVALID_OPERATION:
/// `msaa::backend_max_samples` (src/gl/msaa.rs) is 0, `GL_MAX_SAMPLES` is 1,
/// and `msaa::stored_samples` refuses a count above it (called by
/// `glRenderbufferStorageMultisample`, src/gl/framebuffer.rs). No back-end
/// code is involved, so the V3D's own MSAA gaps (docs/notes/v3d-msaa-es3.md)
/// are not what the host sees.
///
/// The fix (a `max_samples` capability of the back end, plan #339 and
/// docs/notes/es3-negative-api.md section 3) flips the error to GL_NO_ERROR,
/// `GL_MAX_SAMPLES` to 4 and the stored sample count to 4; it also flips
/// `max_samples_is_one_today` of tests/es3_msaa_v3d.rs.
#[test]
fn a_four_sample_renderbuffer_is_refused() {
    let _lib = context();
    assert_eq!(geti(GL_MAX_SAMPLES), 1, "flip to 4 when the back end keeps 4 samples");

    // SAFETY: plain GL calls on objects of this test.
    unsafe {
        let mut fbo: GLuint = 0;
        let mut rbo: GLuint = 0;
        glGenFramebuffers(1, &mut fbo);
        glGenRenderbuffers(1, &mut rbo);
        glBindFramebuffer(GL_FRAMEBUFFER, fbo);
        glBindRenderbuffer(GL_RENDERBUFFER, rbo);
        clear_errors();

        glRenderbufferStorageMultisample(GL_RENDERBUFFER, 4, GL_RGBA8, 32, 32);
        assert_eq!(error(), GL_INVALID_OPERATION, "dEQP wants GL_NO_ERROR");

        // the control: one sample is accepted (single-sample storage)
        glRenderbufferStorageMultisample(GL_RENDERBUFFER, 1, GL_RGBA8, 32, 32);
        assert_eq!(error(), GL_NO_ERROR);

        // the refused call left that storage in place: attached, the
        // framebuffer's renderbuffer reports one sample, not 4
        glRenderbufferStorageMultisample(GL_RENDERBUFFER, 4, GL_RGBA8, 32, 32);
        clear_errors();
        glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_RENDERBUFFER, rbo);
        assert_eq!(error(), GL_NO_ERROR);
        let _ = glCheckFramebufferStatus(GL_FRAMEBUFFER);
        assert_eq!(error(), GL_NO_ERROR);
        let mut samples = -1;
        glGetRenderbufferParameteriv(GL_RENDERBUFFER, GL_RENDERBUFFER_SAMPLES, &mut samples);
        assert_eq!(samples, 1, "flip to 4 when the back end keeps 4 samples");
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
    }
}

/// Integer formats stay without multisampled storage (ES 3.0 4.4.2): a
/// correct refusal, not part of the Fail, so the fix does not flip it.
#[test]
fn an_integer_four_sample_renderbuffer_stays_refused() {
    let _lib = context();
    // SAFETY: plain GL calls.
    unsafe {
        let mut rbo: GLuint = 0;
        glGenRenderbuffers(1, &mut rbo);
        glBindRenderbuffer(GL_RENDERBUFFER, rbo);
        clear_errors();
        glRenderbufferStorageMultisample(GL_RENDERBUFFER, 4, GL_RGBA8I, 8, 8);
        assert_eq!(error(), GL_INVALID_OPERATION);
    }
}
