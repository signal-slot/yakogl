// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! A 3D texture's mip chain reaches the Mali (Valhall G610) back end, through
//! the GL API, on the host (panthor-gltest case 206 es3levels, issue #32).
//!
//! The GL core keeps a 3D texture as a grid of slices with its levels laid
//! out after level 0 (`grid_level_held`), and counts one level for it
//! (`levels` stays 1). The Mali's `volume_of` copies every level the binding
//! names after level 0's `bytes` (docs/notes/mali-3d-levels.md section 4), so
//! the draw (src/gl/draw.rs) gives it the levels a mipmap filter walks --
//! defined, held, capped by GL_TEXTURE_MAX_LEVEL -- and the mip filter of the
//! min filter in force (the sampler object's when one is bound). Without that
//! the Mali samples level 0 at every level.
//!
//! The GL core is driven through the API into a stand-in back end whose draw
//! entry records the sampled binding and runs the Mali's own `mali_draw` (no
//! Mali OSAL on the host: a draw it takes ends at `-ENODEV`).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_3d_levels
//! ```

#![cfg(all(feature = "host-test", feature = "shaderc-online", feature = "gpu-mali", not(feature = "report-es2")))]

use core::ffi::{c_char, c_void};
use std::ffi::CString;
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{self, GlesDraw, GlesHeap, YakoglBackend, *};
use yakogl::gl::buffer::glBindBuffer;
use yakogl::gl::consts::*;
use yakogl::gl::draw::*;
use yakogl::gl::es3::{glBindSampler, glDeleteSamplers, glGenSamplers, glSamplerParameteri};
use yakogl::gl::program::*;
use yakogl::gl::shader::*;
use yakogl::gl::state::*;
use yakogl::gl::texture::*;
use yakogl::gl::types::*;
use yakogl::gl::vertex::{glEnableVertexAttribArray, glVertexAttribPointer};
use yakogl::mali::backend::mali_draw;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};
use yakogl::util::ENODEV;

/* ---- a context on the host: the smallest OSAL and back end that make one -- */

unsafe extern "C" fn m_gpu_alloc(_heap: u64, size: usize, _flags: u32, out: *mut OsalGpuMem) -> i32 {
    let p = Box::leak(vec![0u8; size].into_boxed_slice()).as_mut_ptr();
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

/// What the back end was handed for the draw's 3D texture, and what the
/// Mali made of the draw.
#[derive(Clone, Copy, Debug)]
struct Seen {
    type_: u8,
    mip_levels: u8,
    mip_filter: u8,
    bytes: u32,
    incomplete: u8,
    mali: i32,
}

static SEEN: Mutex<Vec<Seen>> = Mutex::new(Vec::new());

unsafe extern "C" fn b_draw(d: *const GlesDraw) -> i32 {
    let d = &*d;
    if d.index_count == 0 || d.num_textures == 0 {
        return 0;
    }
    let t = &d.textures[0];
    let mali = mali_draw(d);
    SEEN.lock().unwrap().push(Seen {
        type_: t.type_,
        mip_levels: t.mip_levels,
        mip_filter: t.mip_filter,
        bytes: t.bytes,
        incomplete: t.incomplete,
        mali,
    });
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
    name: c"mali-3d-levels".as_ptr() as *const c_char,
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
    ONCE.call_once(|| {
        // SAFETY: both tables are statics; the lock above serialises the tests.
        unsafe {
            osal_set_ops(&OSAL);
            backend::g_backend = &BACKEND;
            yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
            assert!(!yakogl::gl::context::gl_context_create(8, 8).is_null());
        }
    });
    SEEN.lock().unwrap().clear();
    // SAFETY: plain GL call.
    while unsafe { glGetError() } != GL_NO_ERROR {}
    g
}

fn err() -> GLenum {
    // SAFETY: plain GL call.
    unsafe { glGetError() }
}

fn shader(ty: GLenum, text: &str) -> GLuint {
    let text = CString::new(text).unwrap();
    // SAFETY: plain GL calls; one NUL-terminated source.
    unsafe {
        let s = glCreateShader(ty);
        let p: [*const c_char; 1] = [text.as_ptr()];
        glShaderSource(s, 1, p.as_ptr(), core::ptr::null());
        glCompileShader(s);
        let mut ok: GLint = 0;
        glGetShaderiv(s, GL_COMPILE_STATUS, &mut ok);
        assert_eq!(ok, 1, "{text:?} did not compile");
        s
    }
}

/// The current program: one sample of a sampler3D on unit 0 at a varying
/// coordinate, as case 206 samples its volume.
fn program() {
    let v = shader(
        GL_VERTEX_SHADER,
        "#version 300 es\nin vec4 p;\nout vec2 uv;\nvoid main() { uv = p.xy * 0.5 + 0.5; gl_Position = p; }\n",
    );
    let f = shader(
        GL_FRAGMENT_SHADER,
        "#version 300 es\nprecision highp float;\nprecision highp sampler3D;\nuniform sampler3D s;\nin vec2 uv;\n\
         layout(location = 0) out vec4 o;\nvoid main() { o = texture(s, vec3(uv, 0.5)); }\n",
    );
    // SAFETY: plain GL calls.
    unsafe {
        let p = glCreateProgram();
        glAttachShader(p, v);
        glAttachShader(p, f);
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        assert_eq!(ok, 1);
        glUseProgram(p);
    }
}

/// Covers the whole target.
static FULL: [f32; 12] = [-1.0, -1.0, 0.0, 1.0, 3.0, -1.0, 0.0, 1.0, -1.0, 3.0, 0.0, 1.0];

/// Draw one triangle and hand back what the back end was given for it.
fn draw() -> Seen {
    SEEN.lock().unwrap().clear();
    // SAFETY: `FULL` is static and outlives the draw.
    unsafe {
        glBindBuffer(GL_ARRAY_BUFFER, 0);
        glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, FULL.as_ptr() as *const c_void);
        glEnableVertexAttribArray(0);
        glDrawArrays(GL_TRIANGLES, 0, 3);
        glFinish();
    }
    assert_eq!(err(), GL_NO_ERROR);
    let s = core::mem::take(&mut *SEEN.lock().unwrap());
    *s.last().expect("the draw reached the back end")
}

/// A 4x4x4 RGBA8 3D texture bound to unit 0 with its levels `defined`
/// (4x4x4, 2x2x2, 1x1x1: the whole chain is three), each filled with its own
/// colour as case 206 fills them.
fn volume(defined: u32) -> GLuint {
    let mut tex = 0;
    // SAFETY: room for one name; each level's texels cover its size.
    unsafe {
        glGenTextures(1, &mut tex);
        glActiveTexture(GL_TEXTURE0);
        glBindTexture(GL_TEXTURE_3D, tex);
        for l in 0..defined {
            let side = 4usize >> l;
            let texels = vec![[64 * l as u8, 0, 0, 255]; side * side * side];
            glTexImage3D(
                GL_TEXTURE_3D,
                l as GLint,
                GL_RGBA8 as GLint,
                side as GLsizei,
                side as GLsizei,
                side as GLsizei,
                0,
                GL_RGBA,
                GL_UNSIGNED_BYTE,
                texels.as_ptr() as *const c_void,
            );
        }
        glTexParameteri(GL_TEXTURE_3D, GL_TEXTURE_MAG_FILTER, GL_NEAREST as GLint);
    }
    assert_eq!(err(), GL_NO_ERROR);
    tex
}

fn min_filter(filter: GLenum) {
    // SAFETY: plain GL call.
    unsafe { glTexParameteri(GL_TEXTURE_3D, GL_TEXTURE_MIN_FILTER, filter as GLint) };
}

fn delete(tex: GLuint) {
    // SAFETY: a name this test made.
    unsafe { glDeleteTextures(1, &tex) };
}

/* ---- the binding the Mali receives ------------------------------------------ */

/// Three defined levels and a mipmap min filter: the Mali is given the three
/// levels and the mip filter of the min filter (NEAREST 1, LINEAR 2), and
/// level 0's bytes alone (`volume_of` reads the levels after them); the
/// Mali takes the draw.
#[test]
fn three_levels_and_the_mip_filter_reach_the_mali() {
    let _g = context();
    program();
    let tex = volume(3);
    min_filter(GL_LINEAR);
    let one = draw();
    assert_eq!((one.type_, one.incomplete), (GLES_TEXTYPE_3D, 0));
    assert_eq!((one.mip_levels, one.mip_filter), (1, 0), "no mipmap filter: level 0 alone");
    for (filter, mip) in [
        (GL_NEAREST_MIPMAP_NEAREST, 1),
        (GL_LINEAR_MIPMAP_NEAREST, 1),
        (GL_NEAREST_MIPMAP_LINEAR, 2),
        (GL_LINEAR_MIPMAP_LINEAR, 2),
    ] {
        min_filter(filter);
        let s = draw();
        assert_eq!((s.type_, s.incomplete), (GLES_TEXTYPE_3D, 0), "min filter {filter:#x}");
        assert_eq!((s.mip_levels, s.mip_filter), (3, mip), "min filter {filter:#x}");
        assert_eq!(s.bytes, one.bytes, "level 0's bytes, min filter {filter:#x}");
        assert_eq!(s.mali, -ENODEV, "the Mali takes the draw, min filter {filter:#x}");
    }
    delete(tex);
}

/// GL_TEXTURE_MAX_LEVEL caps the levels given (ES 3.0 3.8.10): 1 gives two,
/// 0 gives level 0 alone with no mip filter.
#[test]
fn max_level_caps_the_levels() {
    let _g = context();
    program();
    let tex = volume(3);
    min_filter(GL_NEAREST_MIPMAP_LINEAR);
    // SAFETY: plain GL calls.
    unsafe { glTexParameteri(GL_TEXTURE_3D, GL_TEXTURE_MAX_LEVEL, 1) };
    let s = draw();
    assert_eq!((s.mip_levels, s.mip_filter), (2, 2));
    // SAFETY: plain GL call.
    unsafe { glTexParameteri(GL_TEXTURE_3D, GL_TEXTURE_MAX_LEVEL, 0) };
    let s = draw();
    assert_eq!((s.mip_levels, s.mip_filter), (1, 0));
    delete(tex);
}

/// A sampler object's min filter is the one in force (ES 3.0 3.8.2): a
/// texture filtered LINEAR sampled through a LINEAR_MIPMAP_NEAREST sampler
/// gives the Mali its three levels, and unbinding the sampler gives one.
#[test]
fn the_sampler_objects_min_filter_is_in_force() {
    let _g = context();
    program();
    let tex = volume(3);
    min_filter(GL_LINEAR);
    let mut sampler = 0;
    // SAFETY: room for one name; plain GL calls.
    unsafe {
        glGenSamplers(1, &mut sampler);
        glSamplerParameteri(sampler, GL_TEXTURE_MIN_FILTER, GL_LINEAR_MIPMAP_NEAREST as GLint);
        glSamplerParameteri(sampler, GL_TEXTURE_MAG_FILTER, GL_NEAREST as GLint);
        glBindSampler(0, sampler);
    }
    let s = draw();
    assert_eq!((s.mip_levels, s.mip_filter), (3, 1));
    // SAFETY: plain GL calls on a name this test made.
    unsafe {
        glBindSampler(0, 0);
        glDeleteSamplers(1, &sampler);
    }
    let s = draw();
    assert_eq!((s.mip_levels, s.mip_filter), (1, 0));
    delete(tex);
}
