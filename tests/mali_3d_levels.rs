// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The levels and the mip filter the GL core hands the Mali for a 3D texture
//! (case 206 es3levels, docs/notes/mali-3d-levels.md section 4), through the
//! GL API on the host.
//!
//! The core lays a 3D texture out as a grid of slices with its levels after
//! level 0 (`native_layered_textures` false); the Mali builds the whole chain
//! from it (`volume_of`), so the draw's binding must count the levels the
//! application defined, clipped by `GL_TEXTURE_MAX_LEVEL`, and carry the mip
//! filter of the min filter. `bytes` stays level 0's.
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
use yakogl::gl::program::*;
use yakogl::gl::shader::*;
use yakogl::gl::state::*;
use yakogl::gl::texture::*;
use yakogl::gl::types::*;
use yakogl::gl::vertex::{glEnableVertexAttribArray, glVertexAttribPointer};
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};

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

/// (type, mip_levels, mip_filter, bytes) of the first texture binding of the
/// last draw that had one.
static SEEN: Mutex<Option<(u8, u8, u8, u32)>> = Mutex::new(None);

unsafe extern "C" fn b_draw(d: *const GlesDraw) -> i32 {
    let d = &*d;
    if d.num_textures > 0 {
        let t = &d.textures[0];
        *SEEN.lock().unwrap() = Some((t.type_, t.mip_levels, t.mip_filter, t.bytes));
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
            let c = yakogl::gl::context::gl_context_create(8, 8);
            assert!(!c.is_null());
        }
    });
    // SAFETY: plain GL calls.
    unsafe {
        glFinish();
        while glGetError() != GL_NO_ERROR {}
    }
    *SEEN.lock().unwrap() = None;
    g
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

fn program() {
    let v = shader(GL_VERTEX_SHADER, "#version 300 es\nin vec4 p;\nvoid main() { gl_Position = p; }\n");
    let f = shader(
        GL_FRAGMENT_SHADER,
        "#version 300 es\nprecision highp float;\nprecision highp sampler3D;\nuniform sampler3D s;\nlayout(location = 0) out vec4 o;\nvoid main() { o = texture(s, vec3(0.5, 0.5, 0.5)); }\n",
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

static TRI: [f32; 12] = [-1.0, -1.0, 0.0, 1.0, 3.0, -1.0, 0.0, 1.0, -1.0, 3.0, 0.0, 1.0];

/// A draw with the texture bound; what the back end saw of its binding.
fn draw() -> (u8, u8, u8, u32) {
    *SEEN.lock().unwrap() = None;
    // SAFETY: `TRI` is static and outlives the draw.
    unsafe {
        glBindBuffer(GL_ARRAY_BUFFER, 0);
        glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, TRI.as_ptr() as *const c_void);
        glEnableVertexAttribArray(0);
        glDrawArrays(GL_TRIANGLES, 0, 3);
        glFinish();
        assert_eq!(glGetError(), GL_NO_ERROR);
    }
    SEEN.lock().unwrap().expect("the draw reached the back end with a texture")
}

/// A 4x4x2 RGBA8 3D texture of `levels` levels, every level written.
fn volume(levels: GLsizei, min_filter: GLenum) {
    let mut tex = 0;
    // SAFETY: plain GL calls; the texel arrays are as large as each level.
    unsafe {
        glGenTextures(1, &mut tex);
        glBindTexture(GL_TEXTURE_3D, tex);
        glTexStorage3D(GL_TEXTURE_3D, levels, GL_RGBA8, 4, 4, 2);
        for l in 0..levels {
            let (w, h, d) = ((4 >> l).max(1), (4 >> l).max(1), (2 >> l).max(1));
            let texels = vec![0x80u8; (w * h * d * 4) as usize];
            glTexSubImage3D(
                GL_TEXTURE_3D,
                l,
                0,
                0,
                0,
                w,
                h,
                d,
                GL_RGBA,
                GL_UNSIGNED_BYTE,
                texels.as_ptr() as *const c_void,
            );
        }
        glTexParameteri(GL_TEXTURE_3D, GL_TEXTURE_MIN_FILTER, min_filter as GLint);
        glTexParameteri(GL_TEXTURE_3D, GL_TEXTURE_MAG_FILTER, GL_NEAREST as GLint);
        assert_eq!(glGetError(), GL_NO_ERROR);
    }
}

#[test]
fn a_mipmapped_volume_reaches_the_mali_with_its_chain() {
    let _g = context();
    program();
    volume(1, GL_NEAREST);
    let level0_bytes = draw().3;
    let table = [
        (GL_NEAREST_MIPMAP_NEAREST, 1),
        (GL_LINEAR_MIPMAP_NEAREST, 1),
        (GL_NEAREST_MIPMAP_LINEAR, 2),
        (GL_LINEAR_MIPMAP_LINEAR, 2),
        (GL_NEAREST, 0),
        (GL_LINEAR, 0),
    ];
    for (filter, mip_filter) in table {
        volume(3, filter);
        let (ty, levels, mf, bytes) = draw();
        assert_eq!(ty, GLES_TEXTYPE_3D);
        assert_eq!((levels, mf), (3, mip_filter), "min filter 0x{filter:x}");
        assert_eq!(bytes, level0_bytes, "`bytes` stays level 0's; the Mali reads the levels after it");
    }
}

#[test]
fn the_levels_follow_max_level() {
    let _g = context();
    program();
    volume(3, GL_LINEAR_MIPMAP_LINEAR);
    // SAFETY: plain GL call.
    unsafe { glTexParameteri(GL_TEXTURE_3D, GL_TEXTURE_MAX_LEVEL, 1) };
    assert_eq!(draw().1, 2, "the application capped the sampler at level 1");
    // SAFETY: plain GL call.
    unsafe { glTexParameteri(GL_TEXTURE_3D, GL_TEXTURE_MAX_LEVEL, 0) };
    let (_, levels, mf, _) = draw();
    assert_eq!((levels, mf), (1, 0), "level 0 alone: no mip filter");
}

#[test]
fn a_volume_of_one_level_keeps_one_level() {
    let _g = context();
    program();
    volume(1, GL_LINEAR_MIPMAP_LINEAR);
    let (_, levels, mf, _) = draw();
    assert_eq!((levels, mf), (1, 0));
}
