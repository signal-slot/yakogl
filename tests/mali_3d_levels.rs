// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The GL core hands the Mali a 3D texture's defined levels and its mip filter
//! (gl-3dlevels; docs/notes/mali-3d-levels.md, sections 1 and 4; panthor-gltest
//! sweep case 206 es3levels, issue #32).
//!
//! The core keeps a 3D grid's `levels` at 1, so `sampling_fields` alone gives
//! `mip_levels` 1 and `mip_filter` 0. For the Mali (`gpu-mali`) a draw's
//! binding of a grid carries the contiguous levels the application defined, as
//! far as GL_TEXTURE_MAX_LEVEL, and the filter from the minification filter;
//! `bytes` stays level 0's. A stand-in back end records the binding.
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
use yakogl::gl::texture::{glBindTexture, glGenTextures, glTexImage3D, glTexParameteri};
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

/// What the back end was given for texture 0 of a draw: (type, mip_levels,
/// mip_filter).
static SEEN: Mutex<Vec<(u8, u8, u8)>> = Mutex::new(Vec::new());

unsafe extern "C" fn b_draw(d: *const GlesDraw) -> i32 {
    let d = &*d;
    if d.index_count != 0 && d.num_textures > 0 {
        let t = &d.textures[0];
        SEEN.lock().unwrap().push((t.type_, t.mip_levels, t.mip_filter));
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
            assert!(!yakogl::gl::context::gl_context_create(8, 8).is_null());
        }
    });
    SEEN.lock().unwrap().clear();
    // SAFETY: plain GL call.
    while unsafe { glGetError() } != GL_NO_ERROR {}
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

/// A 4x4x4 RGBA8 texture with `defined` levels (4, 2, 1 wide), the min filter
/// and GL_TEXTURE_MAX_LEVEL given; one draw samples it. Returns what the back
/// end received for it.
fn draw_with(defined: usize, min_filter: GLenum, max_level: Option<GLint>) -> (u8, u8, u8) {
    let _g = context();
    let v = shader(
        GL_VERTEX_SHADER,
        "#version 300 es\nin vec4 p;\nout vec3 c;\nvoid main() { gl_Position = p; c = vec3(0.5); }\n",
    );
    let f = shader(
        GL_FRAGMENT_SHADER,
        "#version 300 es\nprecision highp float;\nprecision highp sampler3D;\nuniform sampler3D s;\nin vec3 c;\nlayout(location = 0) out vec4 o;\nvoid main() { o = texture(s, c); }\n",
    );
    static TRI: [f32; 12] = [-1.0, -1.0, 0.0, 1.0, 3.0, -1.0, 0.0, 1.0, -1.0, 3.0, 0.0, 1.0];
    // SAFETY: plain GL calls; the texels and the triangle outlive them.
    unsafe {
        let p = glCreateProgram();
        glAttachShader(p, v);
        glAttachShader(p, f);
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        assert_eq!(ok, 1);
        glUseProgram(p);
        let mut tex = 0;
        glGenTextures(1, &mut tex);
        glBindTexture(GL_TEXTURE_3D, tex);
        for l in 0..defined {
            let side = (4 >> l).max(1) as GLsizei;
            let texels = vec![(l as u8 + 1) * 40; (side * side * side * 4) as usize];
            glTexImage3D(
                GL_TEXTURE_3D,
                l as GLint,
                GL_RGBA8 as GLint,
                side,
                side,
                side,
                0,
                GL_RGBA,
                GL_UNSIGNED_BYTE,
                texels.as_ptr() as *const c_void,
            );
            assert_eq!(glGetError(), GL_NO_ERROR, "level {l}");
        }
        glTexParameteri(GL_TEXTURE_3D, GL_TEXTURE_MIN_FILTER, min_filter as GLint);
        glTexParameteri(GL_TEXTURE_3D, GL_TEXTURE_MAG_FILTER, GL_NEAREST as GLint);
        if let Some(m) = max_level {
            glTexParameteri(GL_TEXTURE_3D, GL_TEXTURE_MAX_LEVEL, m);
        }
        glUniform1i(glGetUniformLocation(p, c"s".as_ptr()), 0);
        glBindBuffer(GL_ARRAY_BUFFER, 0);
        glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, TRI.as_ptr() as *const c_void);
        glEnableVertexAttribArray(0);
        glDrawArrays(GL_TRIANGLES, 0, 3);
        glFinish();
        assert_eq!(glGetError(), GL_NO_ERROR);
    }
    SEEN.lock().unwrap().pop().expect("the draw reached the back end with a texture")
}

#[test]
fn three_defined_levels_reach_the_mali_with_a_nearest_chain_filter() {
    let (ty, levels, filter) = draw_with(3, GL_NEAREST_MIPMAP_NEAREST, None);
    assert_eq!(ty, GLES_TEXTYPE_3D);
    assert_eq!((levels, filter), (3, 1));
}

#[test]
fn three_defined_levels_reach_the_mali_with_a_linear_chain_filter() {
    let (_, levels, filter) = draw_with(3, GL_LINEAR_MIPMAP_LINEAR, None);
    assert_eq!((levels, filter), (3, 2));
    let (_, levels, filter) = draw_with(3, GL_NEAREST_MIPMAP_LINEAR, None);
    assert_eq!((levels, filter), (3, 2));
    let (_, levels, filter) = draw_with(3, GL_LINEAR_MIPMAP_NEAREST, None);
    assert_eq!((levels, filter), (3, 1));
}

#[test]
fn texture_max_level_caps_the_levels() {
    let (_, levels, filter) = draw_with(3, GL_NEAREST_MIPMAP_NEAREST, Some(1));
    assert_eq!((levels, filter), (2, 1));
}

#[test]
fn only_the_levels_the_application_defined_count() {
    let (_, levels, filter) = draw_with(2, GL_NEAREST_MIPMAP_NEAREST, None);
    assert_eq!((levels, filter), (2, 1));
}

#[test]
fn a_plain_filter_has_no_mip_filter() {
    let (_, _, filter) = draw_with(3, GL_NEAREST, None);
    assert_eq!(filter, 0);
    let (_, levels, filter) = draw_with(1, GL_LINEAR_MIPMAP_LINEAR, None);
    assert_eq!((levels, filter), (1, 0));
}
