// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! glReadPixels answers GL_INVALID_OPERATION for a (format, type) pair that
//! does not match the read buffer's colour class (ES 3.0.6 4.3.2;
//! dEQP-GLES3 negative_api.buffer.read_pixels_fbo_format_mismatch, #364).
//! One framebuffer per class, every pair of the spec checked through the GL
//! API: normalised fixed point takes RGBA/UNSIGNED_BYTE, signed integer
//! RGBA_INTEGER/INT, unsigned integer RGBA_INTEGER/UNSIGNED_INT, RGB10_A2
//! also RGBA/UNSIGNED_INT_2_10_10_10_REV where the back end stores the class;
//! the implementation's own pair (GL_IMPLEMENTATION_COLOR_READ_FORMAT/TYPE)
//! is always taken. Anything else is GL_INVALID_OPERATION, an unknown enum
//! GL_INVALID_ENUM.
//!
//! Backend-neutral: the GL core's check, the same on PowerVR and V3D. The
//! stand-in back end draws nothing. Not in a `report-es2` build (no integer
//! formats there; the ES 2.0 pairs are pinned by tests/es2_report_guard.rs).
//! The multisample read error (RC8) is tests/es3_negapi_readpixels.rs's.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test es3_readpix_mismatch
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online --test es3_readpix_mismatch
//! ```

#![cfg(all(feature = "host-test", not(feature = "report-es2")))]

use core::ffi::{c_char, c_void};
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{self, GlesDraw, GlesHeap, YakoglBackend};
use yakogl::gl::consts::*;
use yakogl::gl::context;
use yakogl::gl::framebuffer::*;
use yakogl::gl::state::{glGetError, glGetIntegerv};
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

/// A back end that keeps 4 samples (`max_samples` 4): GL_MAX_SAMPLES 4, so
/// the 4-sample test runs dEQP's whole sequence here.
static BACKEND: YakoglBackend = YakoglBackend {
    name: c"es3-readpix-mismatch".as_ptr() as *const c_char,
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
    max_samples: 4,
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
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        while glGetError() != GL_NO_ERROR {}
    }
    g
}

fn err() -> GLenum {
    // SAFETY: plain GL call.
    unsafe { glGetError() }
}

fn geti(pname: GLenum) -> GLint {
    let mut v: GLint = -777;
    // SAFETY: one GLint out.
    unsafe { glGetIntegerv(pname, &mut v) };
    assert_eq!(err(), GL_NO_ERROR, "glGetIntegerv({pname:#x})");
    v
}

/// A framebuffer object bound to GL_FRAMEBUFFER, and what is attached to its
/// colour 0; all of it deleted and the default framebuffer bound on drop.
struct Fbo {
    fb: GLuint,
    tex: GLuint,
    rb: GLuint,
}

impl Fbo {
    fn new() -> Fbo {
        let mut fb = 0;
        // SAFETY: room for one name; plain GL calls.
        unsafe {
            glGenFramebuffers(1, &mut fb);
            glBindFramebuffer(GL_FRAMEBUFFER, fb);
        }
        assert_eq!(err(), GL_NO_ERROR);
        Fbo { fb, tex: 0, rb: 0 }
    }
}

impl Drop for Fbo {
    fn drop(&mut self) {
        // SAFETY: names this test made (0 is ignored).
        unsafe {
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
            glDeleteFramebuffers(1, &self.fb);
            glDeleteTextures(1, &self.tex);
            glDeleteRenderbuffers(1, &self.rb);
        }
    }
}

/// The error of a one-pixel glReadPixels at the origin.
fn read(format: GLenum, type_: GLenum) -> GLenum {
    let mut px = [0u8; 16];
    // SAFETY: one pixel of at most 16 bytes into `px`.
    unsafe { glReadPixels(0, 0, 1, 1, format, type_, px.as_mut_ptr() as *mut c_void) };
    err()
}

/// A complete framebuffer with a 32 x 32 renderbuffer of `internalformat` as
/// colour 0, or None (said on stderr) where this back end does not render
/// the class.
fn class_fbo(internalformat: GLenum) -> Option<Fbo> {
    let mut f = Fbo::new();
    // SAFETY: room for one name; plain GL calls.
    unsafe {
        glGenRenderbuffers(1, &mut f.rb);
        glBindRenderbuffer(GL_RENDERBUFFER, f.rb);
        glRenderbufferStorage(GL_RENDERBUFFER, internalformat, 32, 32);
        let e = glGetError();
        glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_RENDERBUFFER, f.rb);
        if e != GL_NO_ERROR || glCheckFramebufferStatus(GL_FRAMEBUFFER) != GL_FRAMEBUFFER_COMPLETE {
            while glGetError() != GL_NO_ERROR {}
            eprintln!("renderbuffer {internalformat:#x}: not a complete target on this back end, skipped");
            return None;
        }
    }
    assert_eq!(err(), GL_NO_ERROR);
    Some(f)
}

const FORMATS: [GLenum; 4] = [GL_RGBA, GL_RGBA_INTEGER, GL_RGB, GL_RED];
const TYPES: [GLenum; 6] =
    [GL_UNSIGNED_BYTE, GL_FLOAT, GL_INT, GL_UNSIGNED_INT, GL_UNSIGNED_SHORT, GL_UNSIGNED_INT_2_10_10_10_REV];

/// Every (format, type) of the table against `ok`: the pairs in `ok` read
/// without error, every other known pair is GL_INVALID_OPERATION, except the
/// implementation's own pair, which is always taken.
fn check_class(internalformat: GLenum, ok: &[(GLenum, GLenum)]) -> bool {
    let Some(_f) = class_fbo(internalformat) else { return false };
    let (ifmt, ityp) =
        (geti(GL_IMPLEMENTATION_COLOR_READ_FORMAT) as GLenum, geti(GL_IMPLEMENTATION_COLOR_READ_TYPE) as GLenum);
    assert!(
        ok.contains(&(ifmt, ityp)),
        "{internalformat:#x}: implementation pair {ifmt:#x}/{ityp:#x} not in the class's"
    );
    for &fmt in &FORMATS {
        for &ty in &TYPES {
            let want = if ok.contains(&(fmt, ty)) { GL_NO_ERROR } else { GL_INVALID_OPERATION };
            assert_eq!(read(fmt, ty), want, "{internalformat:#x} read as {fmt:#x}/{ty:#x}");
        }
    }
    true
}

#[test]
fn normalised_targets_take_rgba_unsigned_byte_only() {
    let _g = context();
    for f in [GL_RGBA8, GL_RGB8, GL_RGB565, GL_RGBA4, GL_RGB5_A1] {
        if f == GL_RGBA8 {
            assert!(check_class(f, &[(GL_RGBA, GL_UNSIGNED_BYTE)]), "RGBA8 is a target everywhere");
        } else {
            check_class(f, &[(GL_RGBA, GL_UNSIGNED_BYTE)]);
        }
    }
}

#[test]
fn signed_integer_targets_take_rgba_integer_int_only() {
    let _g = context();
    for f in [GL_RGBA8I, GL_RGBA16I, GL_RGBA32I] {
        check_class(f, &[(GL_RGBA_INTEGER, GL_INT)]);
    }
}

#[test]
fn unsigned_integer_targets_take_rgba_integer_unsigned_int_only() {
    let _g = context();
    for f in [GL_RGBA8UI, GL_RGBA16UI, GL_RGBA32UI] {
        check_class(f, &[(GL_RGBA_INTEGER, GL_UNSIGNED_INT)]);
    }
}

#[test]
fn rgb10_a2_takes_unsigned_byte_and_its_packed_word_where_stored() {
    let _g = context();
    let Some(_f) = class_fbo(GL_RGB10_A2) else { return };
    let packed = geti(GL_IMPLEMENTATION_COLOR_READ_TYPE) as GLenum == GL_UNSIGNED_INT_2_10_10_10_REV;
    let mut ok = vec![(GL_RGBA, GL_UNSIGNED_BYTE)];
    if packed {
        ok.push((GL_RGBA, GL_UNSIGNED_INT_2_10_10_10_REV));
    }
    drop(_f);
    check_class(GL_RGB10_A2, &ok);
}

/// dEQP's three steps, in its order, on one texture re-specified three times.
#[test]
fn deqp_sequence_rgba8_then_rgba32i_then_rgba32ui_read_as_float() {
    let _g = context();
    let mut f = Fbo::new();
    for (internalformat, format, type_) in [
        (GL_RGBA8, GL_RGBA, GL_UNSIGNED_BYTE),
        (GL_RGBA32I, GL_RGBA_INTEGER, GL_INT),
        (GL_RGBA32UI, GL_RGBA_INTEGER, GL_UNSIGNED_INT),
    ] {
        // SAFETY: room for one name; no pixels; plain GL calls.
        unsafe {
            if f.tex == 0 {
                glGenTextures(1, &mut f.tex);
            }
            glBindTexture(GL_TEXTURE_2D, f.tex);
            glTexImage2D(GL_TEXTURE_2D, 0, internalformat as GLint, 32, 32, 0, format, type_, core::ptr::null());
            let e = glGetError();
            glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, f.tex, 0);
            if e != GL_NO_ERROR || glCheckFramebufferStatus(GL_FRAMEBUFFER) != GL_FRAMEBUFFER_COMPLETE {
                while glGetError() != GL_NO_ERROR {}
                eprintln!("texture {internalformat:#x}: not a complete target here, step skipped");
                continue;
            }
        }
        assert_eq!(read(GL_RGBA, GL_FLOAT), GL_INVALID_OPERATION, "{internalformat:#x}");
    }
}

#[test]
fn unknown_enums_are_invalid_enum() {
    let _g = context();
    let _f = class_fbo(GL_RGBA8).expect("RGBA8");
    assert_eq!(read(0x1234, GL_UNSIGNED_BYTE), GL_INVALID_ENUM);
    assert_eq!(read(GL_RGBA, 0x1234), GL_INVALID_ENUM);
}
