// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The negative API of uniform buffers and blocks on the V3D under the
//! raised block limits (ES 3.0 2.10.1.1, 2.12.6.3, 6.1.x): the errors the
//! spec gives for binding points, offsets, sizes and block indexes. The
//! limits are read through glGetIntegerv, not written down here.
//!
//! * glBindBufferBase/Range at an index >= GL_MAX_UNIFORM_BUFFER_BINDINGS;
//! * glBindBufferRange at an offset off GL_UNIFORM_BUFFER_OFFSET_ALIGNMENT,
//!   and at size <= 0 or offset < 0;
//! * glUniformBlockBinding with a block index >= the active block count or a
//!   binding >= GL_MAX_UNIFORM_BUFFER_BINDINGS;
//! * glGetActiveUniformBlockiv with an invalid block index;
//! * glGetIntegeri_v of GL_UNIFORM_BUFFER_{BINDING,START,SIZE} at the limit.
//!
//! Each failing call leaves the binding it named as it was.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_ubo_v3d_negative
//! ```
#![cfg(all(feature = "host-test", feature = "shaderc-online", feature = "gpu-v3d", not(feature = "report-es2")))]

use core::ffi::{c_char, c_void};
use std::ffi::CString;
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::*;
use yakogl::gl::buffer::*;
use yakogl::gl::consts::*;
use yakogl::gl::es3::*;
use yakogl::gl::program::*;
use yakogl::gl::shader::*;
use yakogl::gl::state::*;
use yakogl::gl::types::*;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};

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
    name: c"es3-ubo-v3d-negative".as_ptr() as *const c_char,
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
            g_backend = &BACKEND;
            yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
            assert!(!yakogl::gl::context::gl_context_create(16, 16).is_null());
        }
    });
    // SAFETY: plain GL calls on the one context.
    unsafe {
        glUseProgram(0);
        while glGetError() != GL_NO_ERROR {}
    }
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
        assert_eq!(ok, 1, "shader compiles:\n{text:?}");
        s
    }
}

/// `vs`/`fs` linked and current.
fn program(vs: &str, fs: &str) -> GLuint {
    let (v, f) = (shader(GL_VERTEX_SHADER, vs), shader(GL_FRAGMENT_SHADER, fs));
    // SAFETY: plain GL calls.
    unsafe {
        let p = glCreateProgram();
        glAttachShader(p, v);
        glAttachShader(p, f);
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        assert_eq!(ok, 1, "program links");
        glUseProgram(p);
        assert_eq!(err(), GL_NO_ERROR);
        p
    }
}

fn geti(pname: GLenum) -> GLint {
    let mut v: GLint = -1;
    // SAFETY: plain GL call; one value.
    unsafe { glGetIntegerv(pname, &mut v) };
    assert_eq!(err(), GL_NO_ERROR, "glGetIntegerv({pname:#x})");
    v
}

fn geti_indexed(pname: GLenum, index: GLuint) -> (GLint, GLenum) {
    let mut v: GLint = -77;
    // SAFETY: plain GL call; one value.
    unsafe { glGetIntegeri_v(pname, index, &mut v) };
    (v, err())
}

fn buffer(size: usize) -> GLuint {
    let data = vec![0u8; size];
    let mut b: GLuint = 0;
    // SAFETY: plain GL calls; `data` holds `size` bytes.
    unsafe {
        glGenBuffers(1, &mut b);
        glBindBuffer(GL_UNIFORM_BUFFER, b);
        glBufferData(GL_UNIFORM_BUFFER, size as GLsizeiptr, data.as_ptr() as *const c_void, GL_STATIC_DRAW);
    }
    assert_eq!(err(), GL_NO_ERROR);
    b
}

/// Two blocks, one per stage, each 16 vec4 (256 bytes).
const VS: &str = "#version 300 es\nuniform A { vec4 a[16]; };\nin vec4 p;\n\
    void main() { gl_Position = p + a[int(p.x)]; }\n";
const FS: &str = "#version 300 es\nprecision highp float;\nuniform B { vec4 b[16]; };\n\
    layout(location = 0) out vec4 o;\nvoid main() { o = b[int(gl_FragCoord.x) & 15]; }\n";

fn prog() -> (GLuint, GLuint) {
    let p = program(VS, FS);
    let mut n: GLint = 0;
    // SAFETY: plain GL call.
    unsafe { glGetProgramiv(p, GL_ACTIVE_UNIFORM_BLOCKS, &mut n) };
    assert_eq!(n, 2, "two active blocks");
    (p, n as GLuint)
}

#[test]
fn limits_are_the_raised_ones() {
    let _g = context();
    assert_eq!(geti(GL_MAX_UNIFORM_BUFFER_BINDINGS), 24);
    assert!(geti(GL_UNIFORM_BUFFER_OFFSET_ALIGNMENT) >= 1);
    assert!(geti(GL_MAX_UNIFORM_BLOCK_SIZE) >= 16384);
}

#[test]
fn bind_base_index_at_limit() {
    let _g = context();
    let max = geti(GL_MAX_UNIFORM_BUFFER_BINDINGS) as GLuint;
    let b = buffer(256);
    for index in [max, max + 1, u32::MAX] {
        // SAFETY: plain GL call.
        unsafe { glBindBufferBase(GL_UNIFORM_BUFFER, index, b) };
        assert_eq!(err(), GL_INVALID_VALUE, "BindBufferBase index {index}");
    }
    /* the last valid point works */
    // SAFETY: plain GL call.
    unsafe { glBindBufferBase(GL_UNIFORM_BUFFER, max - 1, b) };
    assert_eq!(err(), GL_NO_ERROR);
    assert_eq!(geti_indexed(GL_UNIFORM_BUFFER_BINDING, max - 1), (b as GLint, GL_NO_ERROR));
    // SAFETY: plain GL call.
    unsafe { glBindBufferBase(GL_UNIFORM_BUFFER, max - 1, 0) };
}

#[test]
fn bind_range_index_at_limit() {
    let _g = context();
    let max = geti(GL_MAX_UNIFORM_BUFFER_BINDINGS) as GLuint;
    let b = buffer(256);
    for index in [max, max + 1, u32::MAX] {
        // SAFETY: plain GL call.
        unsafe { glBindBufferRange(GL_UNIFORM_BUFFER, index, b, 0, 64) };
        assert_eq!(err(), GL_INVALID_VALUE, "BindBufferRange index {index}");
    }
    // SAFETY: plain GL call.
    unsafe { glBindBufferRange(GL_UNIFORM_BUFFER, max - 1, b, 0, 64) };
    assert_eq!(err(), GL_NO_ERROR);
    // SAFETY: plain GL call.
    unsafe { glBindBufferRange(GL_UNIFORM_BUFFER, max - 1, 0, 0, 64) };
    assert_eq!(err(), GL_NO_ERROR);
}

#[test]
fn bind_range_offset_alignment() {
    let _g = context();
    let align = geti(GL_UNIFORM_BUFFER_OFFSET_ALIGNMENT) as GLintptr;
    let b = buffer(1024);
    /* a misaligned offset is an error only when the alignment is above 1 */
    for off in [1, align + 1, 2 * align - 1, 3 * align + 2] {
        if off % align == 0 {
            continue;
        }
        // SAFETY: plain GL call.
        unsafe { glBindBufferRange(GL_UNIFORM_BUFFER, 0, b, off, 16) };
        assert_eq!(err(), GL_INVALID_VALUE, "offset {off} (alignment {align})");
        assert_eq!(geti_indexed(GL_UNIFORM_BUFFER_START, 0), (0, GL_NO_ERROR), "binding left alone");
    }
    // SAFETY: plain GL call.
    unsafe { glBindBufferRange(GL_UNIFORM_BUFFER, 0, b, 2 * align, 16) };
    assert_eq!(err(), GL_NO_ERROR);
    assert_eq!(geti_indexed(GL_UNIFORM_BUFFER_START, 0), ((2 * align) as GLint, GL_NO_ERROR));
    assert_eq!(geti_indexed(GL_UNIFORM_BUFFER_SIZE, 0), (16, GL_NO_ERROR));
    // SAFETY: plain GL call.
    unsafe { glBindBufferRange(GL_UNIFORM_BUFFER, 0, 0, 0, 16) };
}

#[test]
fn bind_range_size_and_offset_sign() {
    let _g = context();
    let b = buffer(1024);
    for (off, size) in [(0, 0), (0, -1), (0, GLsizeiptr::MIN), (-4, 16), (-1, 0)] {
        // SAFETY: plain GL call.
        unsafe { glBindBufferRange(GL_UNIFORM_BUFFER, 1, b, off, size) };
        assert_eq!(err(), GL_INVALID_VALUE, "offset {off} size {size}");
    }
    assert_eq!(geti_indexed(GL_UNIFORM_BUFFER_BINDING, 1), (0, GL_NO_ERROR), "binding left alone");
}

#[test]
fn bad_target_is_invalid_enum() {
    let _g = context();
    let b = buffer(64);
    // SAFETY: plain GL calls.
    unsafe {
        glBindBufferBase(GL_ARRAY_BUFFER, 0, b);
        assert_eq!(err(), GL_INVALID_ENUM);
        glBindBufferRange(GL_ARRAY_BUFFER, 0, b, 0, 16);
        assert_eq!(err(), GL_INVALID_ENUM);
    }
}

#[test]
fn uniform_block_binding_errors() {
    let _g = context();
    let (p, count) = prog();
    let max = geti(GL_MAX_UNIFORM_BUFFER_BINDINGS) as GLuint;
    let bound = |i: GLuint| {
        let mut v: GLint = -1;
        // SAFETY: plain GL call.
        unsafe { glGetActiveUniformBlockiv(p, i, GL_UNIFORM_BLOCK_BINDING, &mut v) };
        assert_eq!(err(), GL_NO_ERROR);
        v
    };
    // SAFETY: plain GL calls.
    unsafe {
        glUniformBlockBinding(p, 0, 3);
        assert_eq!(err(), GL_NO_ERROR);
        for blk in [count, count + 1, u32::MAX] {
            glUniformBlockBinding(p, blk, 0);
            assert_eq!(err(), GL_INVALID_VALUE, "block index {blk}");
        }
        for binding in [max, max + 1, u32::MAX] {
            glUniformBlockBinding(p, 0, binding);
            assert_eq!(err(), GL_INVALID_VALUE, "binding {binding}");
        }
        /* both wrong: still one INVALID_VALUE, nothing changed */
        glUniformBlockBinding(p, count, max);
        assert_eq!(err(), GL_INVALID_VALUE);
    }
    assert_eq!(bound(0), 3, "failing calls leave the binding");
    // SAFETY: plain GL calls.
    unsafe {
        glUniformBlockBinding(p, 1, max - 1);
        assert_eq!(err(), GL_NO_ERROR);
    }
    assert_eq!(bound(1), (max - 1) as GLint);
}

#[test]
fn uniform_block_binding_bad_program() {
    let _g = context();
    // SAFETY: plain GL calls.
    unsafe {
        glUniformBlockBinding(0, 0, 0);
        assert_eq!(err(), GL_INVALID_VALUE, "program 0 is not a program object");
        glUniformBlockBinding(0xdead, 0, 0);
        assert_eq!(err(), GL_INVALID_VALUE);
    }
}

#[test]
fn active_uniform_block_iv_invalid_index() {
    let _g = context();
    let (p, count) = prog();
    for pname in [
        GL_UNIFORM_BLOCK_BINDING,
        GL_UNIFORM_BLOCK_DATA_SIZE,
        GL_UNIFORM_BLOCK_NAME_LENGTH,
        GL_UNIFORM_BLOCK_ACTIVE_UNIFORMS,
        GL_UNIFORM_BLOCK_REFERENCED_BY_VERTEX_SHADER,
    ] {
        for blk in [count, count + 1, u32::MAX] {
            let mut v: GLint = -77;
            // SAFETY: plain GL call.
            unsafe { glGetActiveUniformBlockiv(p, blk, pname, &mut v) };
            assert_eq!(err(), GL_INVALID_VALUE, "pname {pname:#x} block {blk}");
            assert_eq!(v, -77, "nothing written on an error");
        }
    }
    let mut v: GLint = 0;
    // SAFETY: plain GL call.
    unsafe { glGetActiveUniformBlockiv(p, 0, 0x1234, &mut v) };
    assert_eq!(err(), GL_INVALID_ENUM, "unknown pname on a valid block");
}

#[test]
fn indexed_state_at_limit() {
    let _g = context();
    let max = geti(GL_MAX_UNIFORM_BUFFER_BINDINGS) as GLuint;
    for pname in [GL_UNIFORM_BUFFER_BINDING, GL_UNIFORM_BUFFER_START, GL_UNIFORM_BUFFER_SIZE] {
        for index in [max, max + 1, u32::MAX] {
            assert_eq!(geti_indexed(pname, index), (-77, GL_INVALID_VALUE), "pname {pname:#x} index {index}");
        }
        let (_, e) = geti_indexed(pname, max - 1);
        assert_eq!(e, GL_NO_ERROR, "pname {pname:#x} at the last point");
    }
}
