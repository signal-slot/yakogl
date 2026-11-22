// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GL_RGBA32F is stored in its own 32-bit class on the PowerVR (plan #388,
//! issue #313), as halves before. dEQP-GLES3 fills float textures with values
//! up to +-1e5, beyond the half maximum 65504, so every case that samples
//! RGBA32F failed (texture.format.sized.*.rgba32f_*, texture.units cases with
//! RGBA32F, shaders.derivate). The V3D got the same opt-in in cdb74fd2.
//!
//! The rig is the one of tests/es3_rgba32f_v3d.rs: a dEQP renderQuad over the
//! texture, the bind the GL core hands the back end and the texels in the
//! memory it laid out. GL_RGBA32F must bind as `GLES_TEXFMT_RGBA32F`, four
//! 32-bit words a texel holding the uploaded floats (1e5, -1e5 included), the
//! PowerVR's 128-bit TPU class (`TEXF_RGBA32F`, src/pvr/backend.rs). The
//! GL_RGBA16F control stays halves. What it does not show: the picture the
//! TPU makes, and rendering into RGBA32F (unchanged: colour-unrenderable
//! without GL_EXT_color_buffer_float).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test es3_rgba32f_pvr
//! ```
//! Under `report-es2`, or without `shaderc-online`, the file is empty.

#![cfg(all(feature = "host-test", feature = "shaderc-online", feature = "gpu-powervr", not(feature = "report-es2")))]

use core::ffi::{c_char, c_void};
use std::ffi::CString;
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::*;
use yakogl::gl::consts::*;
use yakogl::gl::draw::*;
use yakogl::gl::framebuffer::*;
use yakogl::gl::program::*;
use yakogl::gl::shader::*;
use yakogl::gl::state::*;
use yakogl::gl::texture::*;
use yakogl::gl::types::*;
use yakogl::gl::vertex::*;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};

/* ---- a context on the host: the smallest OSAL and a recording back end ---- */

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

/// The first binding of every draw since the last take, with the first texel
/// at its base (read from the memory the OSAL handed the GL core: its GPU
/// address is the host pointer).
struct Seen(GlesTextureBind, Vec<u8>);
// SAFETY: the bind'"'"'s pointers are never dereferenced by the test.
unsafe impl Send for Seen {}
static DRAWS: Mutex<Vec<Seen>> = Mutex::new(Vec::new());
const TEXEL_BYTES: usize = 16;

unsafe extern "C" fn b_draw(d: *const GlesDraw) -> i32 {
    // SAFETY: the core passes a live job.
    let d = unsafe { &*d };
    let b = d.textures[0];
    // SAFETY: base_gpu is the address m_gpu_alloc gave (a leaked host
    // allocation holding at least the first texel of the level).
    let texels = unsafe { core::slice::from_raw_parts(b.base_gpu as *const u8, TEXEL_BYTES) }.to_vec();
    DRAWS.lock().unwrap_or_else(|e| e.into_inner()).push(Seen(b, texels));
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

/// A recording stand-in. The storage class the GL core picks does not depend
/// on the layout answers; the sampled classes are the PowerVR's (`gpu-powervr`).
static BACKEND: YakoglBackend = YakoglBackend {
    name: c"es3-rgba32f-pvr".as_ptr() as *const c_char,
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
    ONCE.call_once(|| {
        // SAFETY: both tables are statics; the lock above serialises the tests.
        unsafe {
            osal_set_ops(&OSAL);
            g_backend = &BACKEND;
            yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
            assert!(!yakogl::gl::context::gl_context_create(16, 16).is_null());
        }
    });
    // SAFETY: plain GL calls on the one context.
    unsafe {
        glUseProgram(0);
        glActiveTexture(GL_TEXTURE0);
        glPixelStorei(GL_UNPACK_ALIGNMENT, 1);
        while glGetError() != GL_NO_ERROR {}
    }
    DRAWS.lock().unwrap_or_else(|e| e.into_inner()).clear();
    g
}

fn err() -> GLenum {
    // SAFETY: plain GL call.
    unsafe { glGetError() }
}

fn shader(ty: GLenum, text: &str) -> GLuint {
    let c = CString::new(text).unwrap();
    // SAFETY: plain GL calls; one NUL-terminated source.
    unsafe {
        let s = glCreateShader(ty);
        let p: [*const c_char; 1] = [c.as_ptr()];
        glShaderSource(s, 1, p.as_ptr(), core::ptr::null());
        glCompileShader(s);
        let mut ok: GLint = 0;
        glGetShaderiv(s, GL_COMPILE_STATUS, &mut ok);
        assert_eq!(ok, 1, "compile: {text}");
        s
    }
}

const VS: &str =
    "#version 300 es\nin vec4 p;\nout vec3 v;\nvoid main() { v = vec3(p.xy * 0.5 + 0.5, 0.5); gl_Position = p; }\n";

/// dEQP's renderQuad over `sampler` on unit 0: one draw.
fn draw_sampling(sampler: &str) {
    let fs = format!(
        "#version 300 es\nprecision highp float;\nprecision highp {sampler};\nuniform {sampler} s;\n\
         in vec3 v;\nout vec4 o;\nvoid main() {{ o = texture(s, v); }}\n"
    );
    let fs = if sampler == "sampler2D" { fs.replace("texture(s, v)", "texture(s, v.xy)") } else { fs };
    let (v, f) = (shader(GL_VERTEX_SHADER, VS), shader(GL_FRAGMENT_SHADER, &fs));
    const QUAD: [f32; 16] = [-1.0, -1.0, 0.5, 1.0, 1.0, -1.0, 0.5, 1.0, -1.0, 1.0, 0.5, 1.0, 1.0, 1.0, 0.5, 1.0];
    // SAFETY: plain GL calls; QUAD is a static.
    unsafe {
        let p = glCreateProgram();
        glAttachShader(p, v);
        glAttachShader(p, f);
        glBindAttribLocation(p, 0, c"p".as_ptr());
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        assert_eq!(ok, 1, "link {sampler}");
        glUseProgram(p);
        glUniform1i(glGetUniformLocation(p, c"s".as_ptr()), 0);
        glViewport(0, 0, 16, 16);
        glEnableVertexAttribArray(0);
        glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, QUAD.as_ptr() as *const c_void);
        glDrawArrays(GL_TRIANGLE_STRIP, 0, 4);
        glUseProgram(0);
        glDeleteProgram(p);
    }
    assert_eq!(err(), GL_NO_ERROR, "the quad over {sampler}");
}

const SIDE: usize = 4;
/// Beyond the half maximum (65504), both signs, plus a third and a tiny value.
const VALUES: [f32; 4] = [1.0e5, -1.0e5, 1.0 / 3.0, 1.0e-8];

#[derive(Clone, Copy, Debug)]
enum Shape {
    D2,
    Array,
}

impl Shape {
    fn target(self) -> GLenum {
        match self {
            Shape::D2 => GL_TEXTURE_2D,
            Shape::Array => GL_TEXTURE_2D_ARRAY,
        }
    }
    fn sampler(self) -> &'static str {
        match self {
            Shape::D2 => "sampler2D",
            Shape::Array => "sampler2DArray",
        }
    }
}

/// A SIDE x SIDE (x 2 layers) texture, every texel `values`, NEAREST.
fn make(shape: Shape, internalformat: GLenum, values: [f32; 4]) -> GLuint {
    let layers = if matches!(shape, Shape::Array) { 2 } else { 1 };
    let px: Vec<f32> = (0..SIDE * SIDE * layers).flat_map(|_| values).collect();
    let mut name = 0;
    // SAFETY: plain GL calls; `px` holds SIDE x SIDE x layers RGBA floats.
    unsafe {
        glGenTextures(1, &mut name);
        glBindTexture(shape.target(), name);
        let (w, h, l, f) = (SIDE as GLsizei, SIDE as GLsizei, layers as GLsizei, internalformat as GLint);
        let data = px.as_ptr() as *const c_void;
        match shape {
            Shape::D2 => glTexImage2D(GL_TEXTURE_2D, 0, f, w, h, 0, GL_RGBA, GL_FLOAT, data),
            Shape::Array => glTexImage3D(shape.target(), 0, f, w, h, l, 0, GL_RGBA, GL_FLOAT, data),
        }
        for p in [GL_TEXTURE_MIN_FILTER, GL_TEXTURE_MAG_FILTER] {
            glTexParameteri(shape.target(), p, GL_NEAREST as GLint);
        }
    }
    assert_eq!(err(), GL_NO_ERROR, "{shape:?} {internalformat:#x}");
    name
}

fn drawn(shape: Shape, internalformat: GLenum, values: [f32; 4]) -> (GlesTextureBind, Vec<u8>) {
    let t = make(shape, internalformat, values);
    draw_sampling(shape.sampler());
    // SAFETY: a name made above.
    unsafe { glDeleteTextures(1, &t) };
    let d = core::mem::take(&mut *DRAWS.lock().unwrap_or_else(|e| e.into_inner()));
    assert_eq!(d.len(), 1, "one draw of {shape:?}");
    let Seen(b, texel) = d.into_iter().next().unwrap();
    (b, texel)
}

fn floats(texel: &[u8]) -> [f32; 4] {
    core::array::from_fn(|i| f32::from_ne_bytes(texel[i * 4..i * 4 + 4].try_into().unwrap()))
}

#[test]
fn rgba32f_samples_full_floats() {
    let _g = context();
    for shape in [Shape::D2, Shape::Array] {
        let (b, texel) = drawn(shape, GL_RGBA32F, VALUES);
        assert_eq!(b.format, GLES_TEXFMT_RGBA32F, "{shape:?}: bound class");
        assert_eq!(floats(&texel), VALUES, "{shape:?}: texels as uploaded, not halves");
    }
}

#[test]
fn rgba16f_control_stays_halves() {
    let _g = context();
    let (b, _) = drawn(Shape::D2, GL_RGBA16F, [1.0, 2.0, 0.5, 4.0]);
    assert_eq!(b.format, GLES_TEXFMT_RGBA16F);
}

#[test]
fn rgba32f_without_the_extension_is_not_a_colour_attachment() {
    let _g = context();
    let t = make(Shape::D2, GL_RGBA32F, VALUES);
    let mut fb = 0;
    // SAFETY: plain GL calls on names made here.
    let status = unsafe {
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, t, 0);
        let s = glCheckFramebufferStatus(GL_FRAMEBUFFER);
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glDeleteFramebuffers(1, &fb);
        glDeleteTextures(1, &t);
        s
    };
    while err() != GL_NO_ERROR {}
    assert_ne!(status, GL_FRAMEBUFFER_COMPLETE);
}
