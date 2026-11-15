// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Uniform-block arrays of 256 or more elements through the GL API on the
//! host. `UniformRow::array_len` was a byte, so `vec4 a[300]` was refused or
//! truncated at reflection; it is 16 bits now (`struct shc_uniform`).
//!
//! What is pinned, for std140 blocks holding `vec4[300]`, `float[300]` and
//! (where the build's `GL_MAX_UNIFORM_BLOCK_SIZE` is 16384 or more)
//! `float[1024]` and `vec4[1024]`:
//! - the program links;
//! - `GL_UNIFORM_SIZE` is the element count, `GL_UNIFORM_ARRAY_STRIDE` 16
//!   (std140 rounds a scalar array's stride up to a vec4) and
//!   `GL_UNIFORM_BLOCK_DATA_SIZE` is `16 * count` bytes (ES 3.0 2.12.6.4).
//!
//! `float[1024]` is a 16384-byte block; the blocks stay within the current
//! `MAX_UNIFORM_BLOCK_SIZE` (6144) otherwise, so those cases are skipped
//! until a build reports 16384.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test es3_ubo_array_len
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online --test es3_ubo_array_len
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_ubo_array_len
//! ```

// Uniform blocks need the compiler and an ES 3.0 report; the other
// configurations build the file empty.
#![cfg(all(feature = "host-test", feature = "shaderc-online", not(feature = "report-es2")))]

use core::ffi::{c_char, c_void};
use std::ffi::{CStr, CString};
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{GlesDraw, GlesHeap, YakoglBackend};
use yakogl::gl::consts::*;
use yakogl::gl::program::*;
use yakogl::gl::shader::*;
use yakogl::gl::state::*;
use yakogl::gl::types::*;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};

/* ---- a context on the host: the smallest OSAL and back end that make one --- */

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
    name: c"es3-ubo-array-len".as_ptr() as *const c_char,
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
            yakogl::backend::g_backend = &BACKEND;
            yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
            assert!(!yakogl::gl::context::gl_context_create(64, 64).is_null());
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

/* ---- the programs ---------------------------------------------------------- */

fn shader(ty: GLenum, text: &str) -> GLuint {
    let c = CString::new(text).unwrap();
    // SAFETY: plain GL calls; one NUL-terminated source and a log buffer.
    unsafe {
        let s = glCreateShader(ty);
        let p: [*const c_char; 1] = [c.as_ptr()];
        glShaderSource(s, 1, p.as_ptr(), core::ptr::null());
        glCompileShader(s);
        let mut ok: GLint = 0;
        glGetShaderiv(s, GL_COMPILE_STATUS, &mut ok);
        if ok != 1 {
            let mut log = [0 as c_char; 1024];
            glGetShaderInfoLog(s, 1024, core::ptr::null_mut(), log.as_mut_ptr());
            panic!("compile failed: {}\n{text}", CStr::from_ptr(log.as_ptr()).to_string_lossy());
        }
        s
    }
}

/// A program whose vertex shader declares `uniform Blk { <ty> m[<n>]; }` and
/// reads its last element.
fn program(ty: &str, n: usize) -> GLuint {
    let last = n - 1;
    let vs_src = format!(
        "#version 300 es\nin vec4 a_pos;\nuniform Blk {{ highp {ty} m[{n}]; }};\nvoid main() {{ gl_Position = a_pos + vec4(m[{last}]); }}\n"
    );
    let fs_src = "#version 300 es\nprecision mediump float;\nout vec4 o;\nvoid main() { o = vec4(1.0); }\n";
    let vs = shader(GL_VERTEX_SHADER, &vs_src);
    let fs = shader(GL_FRAGMENT_SHADER, fs_src);
    // SAFETY: plain GL calls on handles this test owns.
    unsafe {
        let p = glCreateProgram();
        glAttachShader(p, vs);
        glAttachShader(p, fs);
        glLinkProgram(p);
        glDeleteShader(vs);
        glDeleteShader(fs);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        if ok != 1 {
            let mut log = [0 as c_char; 1024];
            glGetProgramInfoLog(p, 1024, core::ptr::null_mut(), log.as_mut_ptr());
            panic!("link failed: {}\n{vs_src}", CStr::from_ptr(log.as_ptr()).to_string_lossy());
        }
        p
    }
}

/// `pname` of the one member `m[0]` (a block without an instance name), through glGetActiveUniformsiv.
fn member_iv(p: GLuint, pname: GLenum) -> GLint {
    let name = CString::new("m[0]").unwrap();
    let names: [*const GLchar; 1] = [name.as_ptr()];
    let mut idx: GLuint = GL_INVALID_INDEX;
    let mut v: GLint = -1;
    // SAFETY: plain GL calls; one name, one index, one value.
    unsafe {
        glGetUniformIndices(p, 1, names.as_ptr(), &mut idx);
        assert_ne!(idx, GL_INVALID_INDEX, "m[0] is not an active uniform");
        glGetActiveUniformsiv(p, 1, &idx, pname, &mut v);
    }
    assert_eq!(err(), GL_NO_ERROR);
    v
}

fn block_data_size(p: GLuint) -> GLint {
    let name = CString::new("Blk").unwrap();
    let mut v: GLint = -1;
    // SAFETY: plain GL calls; one name, one value.
    unsafe {
        let b = glGetUniformBlockIndex(p, name.as_ptr());
        assert_ne!(b, GL_INVALID_INDEX, "no block Blk");
        glGetActiveUniformBlockiv(p, b, GL_UNIFORM_BLOCK_DATA_SIZE, &mut v);
    }
    assert_eq!(err(), GL_NO_ERROR);
    v
}

fn max_block_size() -> GLint {
    let mut v: GLint = 0;
    // SAFETY: plain GL call; one value.
    unsafe { glGetIntegerv(GL_MAX_UNIFORM_BLOCK_SIZE, &mut v) };
    v
}

/// std140: every element of a `vec4` or scalar array takes a vec4 slot.
fn check(ty: &str, n: usize) {
    let _g = context();
    if n * 16 > max_block_size() as usize {
        eprintln!("{ty}[{n}] skipped: {} bytes exceed GL_MAX_UNIFORM_BLOCK_SIZE {}", n * 16, max_block_size());
        return;
    }
    let p = program(ty, n);
    assert_eq!(member_iv(p, GL_UNIFORM_SIZE), n as GLint, "{ty}[{n}] GL_UNIFORM_SIZE");
    assert_eq!(member_iv(p, GL_UNIFORM_ARRAY_STRIDE), 16, "{ty}[{n}] GL_UNIFORM_ARRAY_STRIDE");
    assert_eq!(member_iv(p, GL_UNIFORM_OFFSET), 0, "{ty}[{n}] GL_UNIFORM_OFFSET");
    assert_eq!(block_data_size(p), 16 * n as GLint, "{ty}[{n}] GL_UNIFORM_BLOCK_DATA_SIZE");
    // SAFETY: a program this test made.
    unsafe { glDeleteProgram(p) };
}

#[test]
fn vec4_300() {
    check("vec4", 300);
}

#[test]
fn float_300() {
    check("float", 300);
}

#[test]
fn vec4_255_256_257_straddle_the_old_byte() {
    check("vec4", 255);
    check("vec4", 256);
    check("vec4", 257);
}

#[test]
fn float_1024() {
    check("float", 1024);
}

#[test]
fn vec4_1024() {
    check("vec4", 1024);
}
