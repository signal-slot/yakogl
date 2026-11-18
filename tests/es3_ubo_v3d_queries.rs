// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The V3D's 16384-byte uniform block (`GL_MAX_UNIFORM_BLOCK_SIZE`, raised by
//! `cb52fae8`, `src/gl/context.rs`) seen through the GL queries, on the host:
//! - a linked program with one std140 block of 16 `highp vec4[64]` members
//!   reports `GL_UNIFORM_BLOCK_DATA_SIZE` 16384 and
//!   `GL_UNIFORM_BLOCK_ACTIVE_UNIFORMS` 16, the last member at
//!   `GL_UNIFORM_OFFSET` 15 * 1024 with `GL_UNIFORM_ARRAY_STRIDE` 16, and
//!   `glBindBufferRange` of the whole 16384 bytes raises no error;
//! - `glGetFloatv` and `glGetInteger64v` answer the block size (16384) and
//!   the combined vertex and fragment components (50688) as `glGetIntegerv`
//!   does (dEQP state_query `*_getfloat` / `*_getinteger64`), and the
//!   default-block `GL_MAX_{VERTEX,FRAGMENT}_UNIFORM_COMPONENTS` stay 1536.
//!
//! `tests/es3_limits_v3d.rs` already pins the integer values and the
//! three-form agreement of these pnames; the explicit float and 64-bit
//! values here are the dEQP rows' own numbers.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_ubo_v3d_queries
//! ```

// The values are the V3D's; uniform blocks need the compiler and an ES 3.0 report. The other
// configurations build the file empty.
#![cfg(all(feature = "host-test", feature = "gpu-v3d", feature = "shaderc-online", not(feature = "report-es2")))]

use core::ffi::{c_char, c_void};
use std::ffi::{CStr, CString};
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{GlesDraw, GlesHeap, YakoglBackend};
use yakogl::gl::buffer::*;
use yakogl::gl::consts::*;
use yakogl::gl::es3::*;
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
    name: c"es3-ubo-v3d-queries".as_ptr() as *const c_char,
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

/* ---- the checks ------------------------------------------------------------ */

const BLOCK_BYTES: usize = 16384;

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

/// One block `B` of 16 members `m0..m15`, each `highp vec4[64]` (1024 bytes),
/// read by the vertex shader.
fn full_block_program() -> GLuint {
    let members: String = (0..16).map(|k| format!("    highp vec4 m{k}[64];\n")).collect();
    let sum: String = (0..16).map(|k| format!(" + m{k}[63]")).collect();
    let vs = format!(
        "#version 300 es\nin vec4 a_pos;\nuniform B {{\n{members}}};\n\
         void main() {{ gl_Position = a_pos{sum}; }}\n"
    );
    let fs = "#version 300 es\nprecision mediump float;\nout vec4 o;\nvoid main() { o = vec4(1.0); }\n";
    let vs = shader(GL_VERTEX_SHADER, &vs);
    let fs = shader(GL_FRAGMENT_SHADER, fs);
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
            panic!("link failed: {}", CStr::from_ptr(log.as_ptr()).to_string_lossy());
        }
        p
    }
}

fn block_iv(p: GLuint, block: GLuint, pname: GLenum) -> GLint {
    let mut v: GLint = -1;
    // SAFETY: plain GL call; one value.
    unsafe { glGetActiveUniformBlockiv(p, block, pname, &mut v) };
    v
}

#[test]
fn a_16384_byte_block_reports_its_size_and_members() {
    let _g = context();
    let p = full_block_program();
    // SAFETY: plain GL calls on the program above and local buffers.
    unsafe {
        let b = glGetUniformBlockIndex(p, c"B".as_ptr());
        assert_ne!(b, GL_INVALID_INDEX, "block B not found");
        assert_eq!(block_iv(p, b, GL_UNIFORM_BLOCK_DATA_SIZE), BLOCK_BYTES as GLint, "UNIFORM_BLOCK_DATA_SIZE");
        assert_eq!(block_iv(p, b, GL_UNIFORM_BLOCK_ACTIVE_UNIFORMS), 16, "UNIFORM_BLOCK_ACTIVE_UNIFORMS");

        // the last member, by name; the array is named `m15[0]` or `m15`
        let mut idx: GLuint = GL_INVALID_INDEX;
        let name = c"m15";
        let names: [*const c_char; 1] = [name.as_ptr()];
        glGetUniformIndices(p, 1, names.as_ptr(), &mut idx);
        assert_ne!(idx, GL_INVALID_INDEX, "m15 not found");
        let q = |pname: GLenum| {
            let mut v: GLint = -1;
            glGetActiveUniformsiv(p, 1, &idx, pname, &mut v);
            v
        };
        assert_eq!(q(GL_UNIFORM_OFFSET), 15 * 1024, "UNIFORM_OFFSET of m15");
        assert_eq!(q(GL_UNIFORM_ARRAY_STRIDE), 16, "UNIFORM_ARRAY_STRIDE of m15");
        assert_eq!(q(GL_UNIFORM_BLOCK_INDEX), b as GLint, "UNIFORM_BLOCK_INDEX of m15");
        assert_eq!(q(GL_UNIFORM_SIZE), 64, "UNIFORM_SIZE of m15");
        assert_eq!(glGetError(), GL_NO_ERROR);
        glDeleteProgram(p);
    }
}

#[test]
fn a_buffer_range_of_the_whole_block_is_no_error() {
    let _g = context();
    let p = full_block_program();
    // SAFETY: plain GL calls; the data outlives glBufferData.
    unsafe {
        let data = vec![0u8; BLOCK_BYTES];
        let mut buf: GLuint = 0;
        glGenBuffers(1, &mut buf);
        glBindBuffer(GL_UNIFORM_BUFFER, buf);
        glBufferData(GL_UNIFORM_BUFFER, BLOCK_BYTES as GLsizeiptr, data.as_ptr() as *const c_void, GL_STATIC_DRAW);
        glBindBufferRange(GL_UNIFORM_BUFFER, 0, buf, 0, BLOCK_BYTES as GLsizeiptr);
        assert_eq!(glGetError(), GL_NO_ERROR, "glBindBufferRange of the whole 16384 bytes");
        let b = glGetUniformBlockIndex(p, c"B".as_ptr());
        glUniformBlockBinding(p, b, 0);
        assert_eq!(glGetError(), GL_NO_ERROR, "glUniformBlockBinding");
        glBindBuffer(GL_UNIFORM_BUFFER, 0);
        glDeleteBuffers(1, &buf);
        glDeleteProgram(p);
    }
}

#[test]
fn the_uniform_block_limits_answer_16384_and_50688_in_float_and_64_bit_forms() {
    let _g = context();
    let rows: [(&str, GLenum, i64); 5] = [
        ("MAX_UNIFORM_BLOCK_SIZE", GL_MAX_UNIFORM_BLOCK_SIZE, 16384),
        ("MAX_COMBINED_VERTEX_UNIFORM_COMPONENTS", GL_MAX_COMBINED_VERTEX_UNIFORM_COMPONENTS, 50688),
        ("MAX_COMBINED_FRAGMENT_UNIFORM_COMPONENTS", GL_MAX_COMBINED_FRAGMENT_UNIFORM_COMPONENTS, 50688),
        ("MAX_VERTEX_UNIFORM_COMPONENTS", GL_MAX_VERTEX_UNIFORM_COMPONENTS, 1536),
        ("MAX_FRAGMENT_UNIFORM_COMPONENTS", GL_MAX_FRAGMENT_UNIFORM_COMPONENTS, 1536),
    ];
    for (name, pname, want) in rows {
        let (mut i, mut f, mut w): (GLint, GLfloat, GLint64) = (-1, -1.0, -1);
        // SAFETY: plain GL calls; one value each.
        unsafe {
            glGetIntegerv(pname, &mut i);
            glGetFloatv(pname, &mut f);
            glGetInteger64v(pname, &mut w);
            assert_eq!(glGetError(), GL_NO_ERROR, "{name}");
        }
        assert_eq!(i as i64, want, "{name}: glGetIntegerv");
        assert_eq!(f as f64, want as f64, "{name}: glGetFloatv");
        assert_eq!(w, want, "{name}: glGetInteger64v");
    }
}
