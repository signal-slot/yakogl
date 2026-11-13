// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Blending on the 16-bit float colour targets of ES 3.0 (R16F, RG16F and
//! RGBA16F as `GL_COLOR_ATTACHMENT0`), as the tip accepts it today.
//!
//! Plan #360 makes blending of the 32-bit float targets follow
//! `GL_EXT_color_buffer_float`; the 16F targets must keep blending with or
//! without any extension (ES 3.0.6 4.1.8: blending applies to every
//! colour-renderable fixed-point and floating-point buffer). Held, for a
//! texture and a renderbuffer of each format:
//!
//! - the framebuffer is complete (the 16F targets are complete on every GPU
//!   build today, see tests/es3_float_target.rs);
//! - with `GL_BLEND` on and three common func/equation settings (SRC_ALPHA /
//!   ONE_MINUS_SRC_ALPHA with FUNC_ADD; ONE / ONE; CONSTANT_COLOR / ZERO with
//!   FUNC_REVERSE_SUBTRACT), a draw of a triangle ends with `GL_NO_ERROR`,
//!   reaches the back end, and leaves the framebuffer complete.
//!
//! The stand-in back end draws nothing, so the host model has no read-back
//! of these targets and no pixel value is checked. The draw needs the
//! in-process compiler, so the file is `shaderc-online` only.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test es3_half_float_blend
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online --test es3_half_float_blend
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_half_float_blend
//! ```
//!
//! Not in a `report-es2` build: ES 2.0 has none of the sized float names.

#![cfg(all(feature = "host-test", feature = "shaderc-online", not(feature = "report-es2")))]

use core::ffi::{c_char, c_void};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{self, GlesDraw, GlesHeap, YakoglBackend};
use yakogl::gl::buffer::glBindBuffer;
use yakogl::gl::consts::*;
use yakogl::gl::context;
use yakogl::gl::draw::*;
use yakogl::gl::framebuffer::*;
use yakogl::gl::program::*;
use yakogl::gl::shader::*;
use yakogl::gl::state::*;
use yakogl::gl::texture::{glBindTexture, glDeleteTextures, glGenTextures, glTexImage2D};
use yakogl::gl::types::*;
use yakogl::gl::vertex::{glEnableVertexAttribArray, glVertexAttribPointer};
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

/// Application draws (vertices, not a clear alone) the back end was handed.
static DRAWS: AtomicUsize = AtomicUsize::new(0);

unsafe extern "C" fn b_draw(d: *const GlesDraw) -> i32 {
    if (*d).index_count != 0 {
        DRAWS.fetch_add(1, Ordering::Relaxed);
    }
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
    name: c"es3-half-float-blend".as_ptr() as *const c_char,
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
            yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
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

/// The 16F colour formats, with a (format, type) glTexImage2D takes for each.
const FLOAT16: [(GLenum, GLenum, GLenum, &str); 3] = [
    (GL_R16F, GL_RED, GL_HALF_FLOAT, "R16F"),
    (GL_RG16F, GL_RG, GL_HALF_FLOAT, "RG16F"),
    (GL_RGBA16F, GL_RGBA, GL_HALF_FLOAT, "RGBA16F"),
];

const VS: &str = "#version 300 es\nin vec4 p;\nvoid main() { gl_Position = p; }\n";
const FS: &str = "#version 300 es\nprecision mediump float;\nout vec4 o;\n\
                  void main() { o = vec4(0.25, 0.5, 0.75, 0.5); }\n";

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

fn use_program() {
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

fn draw_triangle() {
    static TRI: [f32; 12] = [-1.0, -1.0, 0.0, 1.0, 3.0, -1.0, 0.0, 1.0, -1.0, 3.0, 0.0, 1.0];
    // SAFETY: TRI is static and outlives the draw.
    unsafe {
        glBindBuffer(GL_ARRAY_BUFFER, 0);
        glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, TRI.as_ptr() as *const c_void);
        glEnableVertexAttribArray(0);
        glDrawArrays(GL_TRIANGLES, 0, 3);
    }
}

/// A bound framebuffer with a 16 x 16 `internalformat` colour attachment 0,
/// a texture or a renderbuffer; returns (framebuffer, object).
fn target(internalformat: GLenum, format: GLenum, type_: GLenum, texture: bool, label: &str) -> (GLuint, GLuint) {
    let (mut fb, mut o) = (0, 0);
    // SAFETY: room for one name each; no pixels; plain GL calls.
    unsafe {
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        if texture {
            glGenTextures(1, &mut o);
            glBindTexture(GL_TEXTURE_2D, o);
            glTexImage2D(GL_TEXTURE_2D, 0, internalformat as GLint, 16, 16, 0, format, type_, NULL);
            glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, o, 0);
        } else {
            glGenRenderbuffers(1, &mut o);
            glBindRenderbuffer(GL_RENDERBUFFER, o);
            glRenderbufferStorage(GL_RENDERBUFFER, internalformat, 16, 16);
            glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_RENDERBUFFER, o);
        }
        assert_eq!(err(), GL_NO_ERROR, "{GPU}: {label} set-up");
    }
    (fb, o)
}

fn release(fb: GLuint, o: GLuint, texture: bool) {
    // SAFETY: names made by `target`.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glDeleteFramebuffers(1, &fb);
        if texture {
            glDeleteTextures(1, &o);
        } else {
            glDeleteRenderbuffers(1, &o);
        }
    }
}

unsafe fn alpha_over() {
    glBlendFunc(GL_SRC_ALPHA, GL_ONE_MINUS_SRC_ALPHA);
    glBlendEquation(GL_FUNC_ADD);
}
unsafe fn additive() {
    glBlendFuncSeparate(GL_ONE, GL_ONE, GL_ONE, GL_ONE);
    glBlendEquationSeparate(GL_FUNC_ADD, GL_FUNC_ADD);
}
unsafe fn constant_colour() {
    glBlendColor(0.5, 0.25, 0.125, 0.5);
    glBlendFunc(GL_CONSTANT_COLOR, GL_ZERO);
    glBlendEquation(GL_FUNC_REVERSE_SUBTRACT);
}

/// Common blend settings: (name, set-up).
const BLENDS: [(&str, unsafe fn()); 3] =
    [("alpha over", alpha_over), ("additive", additive), ("constant colour", constant_colour)];

#[test]
fn blending_a_16f_target_is_accepted() {
    let _g = context();
    use_program();
    for &(internalformat, format, type_, label) in &FLOAT16 {
        for texture in [true, false] {
            let kind = if texture { "texture" } else { "renderbuffer" };
            let (fb, o) = target(internalformat, format, type_, texture, label);
            // SAFETY: plain GL calls.
            unsafe {
                assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE, "{GPU}: {label} {kind}");
                glEnable(GL_BLEND);
                for &(name, set) in &BLENDS {
                    let what = format!("{GPU}: {label} {kind} {name}");
                    set();
                    assert_eq!(err(), GL_NO_ERROR, "{what}: blend state");
                    let before = DRAWS.load(Ordering::Relaxed);
                    draw_triangle();
                    assert_eq!(err(), GL_NO_ERROR, "{what}: draw");
                    assert_eq!(DRAWS.load(Ordering::Relaxed), before + 1, "{what}: the draw reaches the back end");
                    assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE, "{what}: complete");
                    assert_eq!(err(), GL_NO_ERROR, "{what}: status");
                }
                glDisable(GL_BLEND);
            }
            release(fb, o, texture);
        }
    }
    // SAFETY: plain GL call.
    unsafe { glUseProgram(0) };
}
