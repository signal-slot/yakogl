// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! glReadPixels of the 16-bit float colour targets, as the ES 3.0 build
//! answers today (plan #360, item test-rp-half). `GL_EXT_color_buffer_half_float`
//! is claimed (src/gl/state.rs), so RGBA16F, R16F and RG16F texture levels
//! are complete colour targets; this file pins what a read of them does,
//! before the 32F claim changes the neighbouring rules.
//!
//! - `RGBA`/`FLOAT` is accepted (ES 3.0 4.3.2: the pair of a floating-point
//!   buffer); the stored halves come back converted to float, the channels
//!   the format lacks read as 0, 0, 1. The texture is written by the CPU
//!   (glTexImage2D) and attached as the read buffer, so the GL core's own
//!   float row converter (`read_float_row`, src/gl/framebuffer.rs) does the
//!   conversion; no GPU runs (the stand-in back end draws nothing).
//! - `GL_IMPLEMENTATION_COLOR_READ_FORMAT` / `_TYPE` is `RGBA` /
//!   `UNSIGNED_BYTE` for these targets; 4.3.2 lets an implementation choose
//!   a pair of its own, so it is consistent as long as glReadPixels accepts
//!   it, which is tested, with the clamped bytes it gives.
//! - Every other (format, type) pair of the pixel enums is
//!   `GL_INVALID_OPERATION` for a known pair and `GL_INVALID_ENUM` for a name
//!   that is none (4.3.2).
//!
//! Disagreements with the spec are `#[ignore]`d tests naming the section and
//! today's answer; src/ is not edited here.
//!
//! The rule is GL core, the same on every GPU, so the file runs in each ES
//! 3.0 host build:
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test es3_readpixels_half_float
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online --test es3_readpixels_half_float
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_readpixels_half_float
//! ```
//!
//! Not in a `report-es2` build: ES 2.0 has none of the sized float names.

#![cfg(all(feature = "host-test", not(feature = "report-es2")))]

use core::ffi::{c_char, c_void};
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{self, GlesDraw, GlesHeap, YakoglBackend};
use yakogl::gl::consts::*;
use yakogl::gl::context;
use yakogl::gl::framebuffer::*;
use yakogl::gl::state::{glGetError, glGetIntegerv, glPixelStorei};
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
    name: c"es3-readpixels-half-float".as_ptr() as *const c_char,
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

/* ---- fixtures ----------------------------------------------------------- */

/// The 16F formats complete as a texture colour target today, with the
/// glTexImage2D format and the number of channels the format has.
const FORMATS: [(GLenum, GLenum, usize, &str); 3] =
    [(GL_RGBA16F, GL_RGBA, 4, "RGBA16F"), (GL_RG16F, GL_RG, 2, "RG16F"), (GL_R16F, GL_RED, 1, "R16F")];

/// IEEE half bits of the values used; all exact in half and float.
const HALVES: [(u16, f32); 6] =
    [(0x3c00, 1.0), (0x3800, 0.5), (0xc000, -2.0), (0x4900, 10.0), (0x5640, 100.0), (0x0000, 0.0)];

/// A 2 x 2 `internalformat` texture whose texels are the `HALVES` cycled over
/// the format's channels, attached as colour 0 of a new framebuffer (left
/// bound). Returns the names and the expected RGBA floats of each texel.
fn texture_read_buffer(
    (internal, format, nch, label): (GLenum, GLenum, usize, &str),
) -> (GLuint, GLuint, Vec<[f32; 4]>) {
    let (mut fb, mut t) = (0, 0);
    let mut texels = Vec::new();
    let mut data = Vec::<u16>::new();
    for px in 0..4 {
        let mut e = [0.0, 0.0, 0.0, 1.0];
        for (ch, v) in e.iter_mut().enumerate().take(nch) {
            let (bits, f) = HALVES[(px * nch + ch) % HALVES.len()];
            data.push(bits);
            *v = f;
        }
        texels.push(e);
    }
    // SAFETY: `data` holds 2 x 2 texels of `nch` halves; plain GL calls.
    unsafe {
        glGenTextures(1, &mut t);
        glBindTexture(GL_TEXTURE_2D, t);
        glTexImage2D(
            GL_TEXTURE_2D,
            0,
            internal as GLint,
            2,
            2,
            0,
            format,
            GL_HALF_FLOAT,
            data.as_ptr() as *const c_void,
        );
        assert_eq!(err(), GL_NO_ERROR, "{GPU}: {label} texture");
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, t, 0);
        assert_eq!(err(), GL_NO_ERROR, "{GPU}: {label} attached");
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE, "{GPU}: {label} complete");
    }
    (fb, t, texels)
}

fn release(fb: GLuint, t: GLuint) {
    // SAFETY: names made by `texture_read_buffer`.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glDeleteFramebuffers(1, &fb);
        glDeleteTextures(1, &t);
    }
}

/// glReadPixels of the whole 2 x 2 into `out`; the error it raised.
fn read(format: GLenum, type_: GLenum, out: &mut [u8]) -> GLenum {
    while err() != GL_NO_ERROR {}
    // SAFETY: callers pass room for 2 x 2 pixels of the pair.
    unsafe { glReadPixels(0, 0, 2, 2, format, type_, out.as_mut_ptr() as *mut c_void) };
    err()
}

fn floats(b: &[u8]) -> Vec<f32> {
    b.as_chunks::<4>().0.iter().map(|c| f32::from_ne_bytes(*c)).collect()
}

/// Pixel format and type enums the core knows.
const FORMAT_ENUMS: [GLenum; 9] =
    [GL_RGBA, GL_RGB, GL_RG, GL_RED, GL_ALPHA, GL_RGBA_INTEGER, GL_RED_INTEGER, GL_DEPTH_COMPONENT, GL_LUMINANCE];
const TYPE_ENUMS: [GLenum; 7] =
    [GL_UNSIGNED_BYTE, GL_FLOAT, GL_HALF_FLOAT, GL_INT, GL_UNSIGNED_INT, GL_UNSIGNED_SHORT_5_6_5, GL_UNSIGNED_SHORT];

/* ---- RGBA / FLOAT ------------------------------------------------------- */

/// ES 3.0 4.3.2: RGBA/FLOAT reads a floating-point buffer; the halves come
/// back converted, the lacking channels as 0, 0, 1.
#[test]
fn rgba_float_returns_stored_values() {
    let _g = context();
    for f in FORMATS {
        let (fb, t, texels) = texture_read_buffer(f);
        let mut out = [0u8; 2 * 2 * 16];
        assert_eq!(read(GL_RGBA, GL_FLOAT, &mut out), GL_NO_ERROR, "{GPU}: {} RGBA/FLOAT", f.3);
        let want: Vec<f32> = texels.iter().flatten().copied().collect();
        assert_eq!(floats(&out), want, "{GPU}: {} RGBA/FLOAT values", f.3);
        release(fb, t);
    }
}

/// A 1 x 1 read at (1, 1) hands back that texel only.
#[test]
fn rgba_float_sub_rectangle() {
    let _g = context();
    for f in FORMATS {
        let (fb, t, texels) = texture_read_buffer(f);
        let mut out = [0u8; 16];
        // SAFETY: room for one RGBA/FLOAT pixel.
        unsafe { glReadPixels(1, 1, 1, 1, GL_RGBA, GL_FLOAT, out.as_mut_ptr() as *mut c_void) };
        assert_eq!(err(), GL_NO_ERROR, "{GPU}: {} 1x1 read", f.3);
        assert_eq!(floats(&out), texels[3].to_vec(), "{GPU}: {} texel (1, 1)", f.3);
        release(fb, t);
    }
}

/* ---- the implementation-chosen pair ------------------------------------- */

/// 4.3.2: the pair GL_IMPLEMENTATION_COLOR_READ_FORMAT / _TYPE names is one
/// glReadPixels accepts. Today it is RGBA / UNSIGNED_BYTE for every float
/// buffer.
#[test]
fn implementation_pair_is_accepted_and_consistent() {
    let _g = context();
    for f in FORMATS {
        let (fb, t, _) = texture_read_buffer(f);
        let (mut rf, mut rt): (GLint, GLint) = (0, 0);
        // SAFETY: room for one integer each.
        unsafe {
            glGetIntegerv(GL_IMPLEMENTATION_COLOR_READ_FORMAT, &mut rf);
            glGetIntegerv(GL_IMPLEMENTATION_COLOR_READ_TYPE, &mut rt);
        }
        assert_eq!(err(), GL_NO_ERROR, "{GPU}: {} getters", f.3);
        let (rf, rt) = (rf as GLenum, rt as GLenum);
        assert_eq!((rf, rt), (GL_RGBA, GL_UNSIGNED_BYTE), "{GPU}: {} implementation pair", f.3);
        let mut out = [0u8; 2 * 2 * 16];
        assert_eq!(read(rf, rt, &mut out), GL_NO_ERROR, "{GPU}: {} implementation pair accepted", f.3);
        release(fb, t);
    }
}

/// RGBA/UNSIGNED_BYTE, the implementation pair: the values clamp to [0, 1]
/// and scale to bytes.
#[test]
fn rgba_unsigned_byte_is_the_clamped_implementation_pair() {
    let _g = context();
    for f in FORMATS {
        let (fb, t, texels) = texture_read_buffer(f);
        let mut out = [0u8; 2 * 2 * 4];
        assert_eq!(read(GL_RGBA, GL_UNSIGNED_BYTE, &mut out), GL_NO_ERROR, "{GPU}: {} RGBA/UNSIGNED_BYTE", f.3);
        let want: Vec<u8> = texels.iter().flatten().map(|v| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8).collect();
        assert_eq!(out.to_vec(), want, "{GPU}: {} RGBA/UNSIGNED_BYTE values", f.3);
        release(fb, t);
    }
}

/* ---- the other pairs ---------------------------------------------------- */

/// 4.3.2: every pair but RGBA/FLOAT and the implementation pair is an error:
/// GL_INVALID_OPERATION for a known format and type, GL_INVALID_ENUM for a
/// name that is neither. No pixel is written on an error.
#[test]
fn other_pairs_are_errors() {
    let _g = context();
    for f in FORMATS {
        let (fb, t, _) = texture_read_buffer(f);
        for &format in &FORMAT_ENUMS {
            for &type_ in &TYPE_ENUMS {
                if format == GL_RGBA && matches!(type_, GL_UNSIGNED_BYTE | GL_FLOAT) {
                    continue;
                }
                let mut out = [0xa5u8; 2 * 2 * 16];
                let e = read(format, type_, &mut out);
                assert_eq!(e, GL_INVALID_OPERATION, "{GPU}: {} 0x{format:x}/0x{type_:x}", f.3);
                assert!(out.iter().all(|b| *b == 0xa5), "{GPU}: {} 0x{format:x}/0x{type_:x} wrote pixels", f.3);
            }
        }
        let mut out = [0u8; 2 * 2 * 16];
        assert_eq!(read(0x1234, GL_FLOAT, &mut out), GL_INVALID_ENUM, "{GPU}: {} bad format", f.3);
        assert_eq!(read(GL_RGBA, 0x1234, &mut out), GL_INVALID_ENUM, "{GPU}: {} bad type", f.3);
        release(fb, t);
    }
}

/// A negative size is GL_INVALID_VALUE, an empty read is no error.
#[test]
fn sizes_on_a_float_target() {
    let _g = context();
    let (fb, t, _) = texture_read_buffer(FORMATS[0]);
    let mut out = [0u8; 16];
    while err() != GL_NO_ERROR {}
    // SAFETY: nothing is written for an invalid or empty read.
    unsafe { glReadPixels(0, 0, -1, 1, GL_RGBA, GL_FLOAT, out.as_mut_ptr() as *mut c_void) };
    assert_eq!(err(), GL_INVALID_VALUE, "{GPU}: negative width");
    // SAFETY: as above.
    unsafe { glReadPixels(0, 0, 0, 1, GL_RGBA, GL_FLOAT, out.as_mut_ptr() as *mut c_void) };
    assert_eq!(err(), GL_NO_ERROR, "{GPU}: empty read");
    release(fb, t);
}
