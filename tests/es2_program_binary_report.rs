// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! What a `report-es2` build answers about program binaries, pinned so that
//! G7 (docs/notes/es3-program-binary.md, section (f)) cannot move it: ES 2.0
//! has no program binary, so the ES 3.0 pnames are GL_INVALID_ENUM, the
//! binary calls refuse, and GL_EXTENSIONS does not list
//! GL_OES_get_program_binary.
//!
//! - glGetIntegerv(GL_NUM_PROGRAM_BINARY_FORMATS, GL_PROGRAM_BINARY_FORMATS):
//!   GL_INVALID_ENUM, nothing written.
//! - glGetProgramiv(GL_PROGRAM_BINARY_LENGTH, _RETRIEVABLE_HINT):
//!   GL_INVALID_ENUM, nothing written.
//! - glGetProgramBinary: `*length = 0` and GL_INVALID_OPERATION for every
//!   bufSize; a bad name is GL_INVALID_OPERATION too.
//! - glProgramBinary: the name check (GL_INVALID_OPERATION), `length < 0`
//!   (GL_INVALID_VALUE), then GL_INVALID_ENUM.
//! - glProgramParameteri: the name check, GL_INVALID_ENUM for another pname,
//!   GL_INVALID_VALUE for a value other than GL_TRUE/GL_FALSE; the hint is
//!   accepted and not observable (the query is GL_INVALID_ENUM).
//!
//! The linked program needs the in-process compiler without the offline
//! table (`shaderc-online,no-shader-table`: a table miss fails the link). No
//! GPU runs: the stand-in back end draws nothing.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online,no-shader-table,report-es2 --test es2_program_binary_report
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online,no-shader-table,report-es2 --test es2_program_binary_report
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table,report-es2 --test es2_program_binary_report
//! ```

#![cfg(all(feature = "host-test", feature = "report-es2", feature = "shaderc-online"))]

use core::ffi::{c_char, c_void, CStr};
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{self, GlesDraw, GlesHeap, YakoglBackend};
use yakogl::gl::consts::*;
use yakogl::gl::context;
use yakogl::gl::es3::{glGetProgramBinary, glProgramBinary, glProgramParameteri};
use yakogl::gl::program::*;
use yakogl::gl::shader::*;
use yakogl::gl::state::*;
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
    name: c"es2-program-binary-report".as_ptr() as *const c_char,
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
        while glGetError() != GL_NO_ERROR {}
    }
    g
}

fn err() -> GLenum {
    // SAFETY: plain GL call.
    unsafe { glGetError() }
}

const VS: &str = "attribute vec4 p;\nvoid main() { gl_Position = p; }\n";
const FS: &str = "precision mediump float;\nvoid main() { gl_FragColor = vec4(1.0); }\n";

/// A linked program (compile and link are checked).
fn linked_program() -> GLuint {
    let mk = |kind: GLenum, src: &str| -> GLuint {
        // SAFETY: the source is live for the calls.
        unsafe {
            let s = glCreateShader(kind);
            let p = [src.as_ptr().cast::<c_char>()];
            let l = [src.len() as GLint];
            glShaderSource(s, 1, p.as_ptr(), l.as_ptr());
            glCompileShader(s);
            let mut ok: GLint = 0;
            glGetShaderiv(s, GL_COMPILE_STATUS, &mut ok);
            assert_ne!(ok, 0, "compile");
            s
        }
    };
    let (v, f) = (mk(GL_VERTEX_SHADER, VS), mk(GL_FRAGMENT_SHADER, FS));
    // SAFETY: names made above.
    unsafe {
        let p = glCreateProgram();
        glAttachShader(p, v);
        glAttachShader(p, f);
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        let mut log = [0u8; 512];
        let mut n: GLsizei = 0;
        glGetProgramInfoLog(p, 512, &mut n, log.as_mut_ptr().cast());
        assert_ne!(ok, 0, "link: {}", String::from_utf8_lossy(&log[..n.max(0) as usize]));
        glDeleteShader(v);
        glDeleteShader(f);
        assert_eq!(glGetError(), GL_NO_ERROR);
        p
    }
}

/// A name no program has.
fn bad_name() -> GLuint {
    // SAFETY: plain GL call; the new name is deleted again.
    unsafe {
        let p = glCreateProgram();
        glDeleteProgram(p);
        p + 1000
    }
}

#[test]
fn the_integer_queries_are_invalid_enum_and_write_nothing() {
    let _g = context();
    for pname in [GL_NUM_PROGRAM_BINARY_FORMATS, GL_PROGRAM_BINARY_FORMATS] {
        let mut v = [0x5a5a; 4];
        // SAFETY: room for four values.
        unsafe { glGetIntegerv(pname, v.as_mut_ptr()) };
        assert_eq!(err(), GL_INVALID_ENUM, "{pname:#x}");
        assert_eq!(v, [0x5a5a; 4], "{pname:#x}: the output was written");
    }
}

#[test]
fn the_program_queries_are_invalid_enum_for_a_linked_program() {
    let _g = context();
    let p = linked_program();
    for pname in [GL_PROGRAM_BINARY_LENGTH, GL_PROGRAM_BINARY_RETRIEVABLE_HINT] {
        let mut v = 0x5a5a;
        // SAFETY: room for one value.
        unsafe { glGetProgramiv(p, pname, &mut v) };
        assert_eq!(err(), GL_INVALID_ENUM, "{pname:#x}");
        assert_eq!(v, 0x5a5a, "{pname:#x}: the output was written");
    }
    // SAFETY: plain GL call.
    unsafe { glDeleteProgram(p) };
}

#[test]
fn get_program_binary_is_zero_length_and_invalid_operation_for_every_buf_size() {
    let _g = context();
    let p = linked_program();
    let mut buf = [0xa5u8; 64];
    for buf_size in [-1, 0, 1, 16, 64] {
        let mut length: GLsizei = 77;
        let mut format: GLenum = 0x1234;
        // SAFETY: the buffer is 64 bytes; bufSize never exceeds it.
        unsafe { glGetProgramBinary(p, buf_size, &mut length, &mut format, buf.as_mut_ptr().cast()) };
        assert_eq!(err(), GL_INVALID_OPERATION, "bufSize {buf_size}");
        assert_eq!(length, 0, "bufSize {buf_size}");
        assert_eq!(format, 0x1234, "bufSize {buf_size}: the format was written");
        assert_eq!(buf, [0xa5u8; 64], "bufSize {buf_size}: the buffer was written");
    }
    /* a null length is allowed */
    // SAFETY: no output is written through the null pointers.
    unsafe { glGetProgramBinary(p, 64, core::ptr::null_mut(), core::ptr::null_mut(), buf.as_mut_ptr().cast()) };
    assert_eq!(err(), GL_INVALID_OPERATION);
    // SAFETY: plain GL call.
    unsafe { glDeleteProgram(p) };
}

#[test]
fn get_program_binary_on_a_bad_name_is_invalid_operation() {
    let _g = context();
    let mut length: GLsizei = 77;
    let mut format: GLenum = 0x1234;
    let mut buf = [0u8; 8];
    // SAFETY: the buffer is 8 bytes.
    unsafe { glGetProgramBinary(bad_name(), 8, &mut length, &mut format, buf.as_mut_ptr().cast()) };
    assert_eq!(err(), GL_INVALID_OPERATION);
    assert_eq!(length, 77, "a bad name stops before the output");
    // SAFETY: plain GL call; 0 is not a program either.
    unsafe { glGetProgramBinary(0, 8, &mut length, &mut format, buf.as_mut_ptr().cast()) };
    assert_eq!(err(), GL_INVALID_OPERATION);
}

#[test]
fn program_binary_runs_its_checks_then_is_invalid_enum() {
    let _g = context();
    let p = linked_program();
    let bytes = [0u8; 16];
    // SAFETY: the binary is 16 bytes and length never exceeds it.
    unsafe {
        glProgramBinary(p, 0x1234, bytes.as_ptr().cast(), 16);
        assert_eq!(err(), GL_INVALID_ENUM, "any format, any binary");
        glProgramBinary(p, 0x1234, bytes.as_ptr().cast(), 0);
        assert_eq!(err(), GL_INVALID_ENUM, "length 0");
        glProgramBinary(p, 0x1234, bytes.as_ptr().cast(), -1);
        assert_eq!(err(), GL_INVALID_VALUE, "a negative length");
        glProgramBinary(bad_name(), 0x1234, bytes.as_ptr().cast(), 16);
        assert_eq!(err(), GL_INVALID_OPERATION, "the name is checked first");
        glProgramBinary(bad_name(), 0x1234, bytes.as_ptr().cast(), -1);
        assert_eq!(err(), GL_INVALID_OPERATION, "the name before the length");
    }
    /* the program is still linked */
    let mut ok: GLint = 0;
    // SAFETY: room for one value.
    unsafe { glGetProgramiv(p, GL_LINK_STATUS, &mut ok) };
    assert_eq!((err(), ok), (GL_NO_ERROR, GL_TRUE as GLint));
    // SAFETY: plain GL call.
    unsafe { glDeleteProgram(p) };
}

#[test]
fn program_parameteri_runs_its_checks() {
    let _g = context();
    let p = linked_program();
    // SAFETY: plain GL calls.
    unsafe {
        glProgramParameteri(p, GL_PROGRAM_BINARY_RETRIEVABLE_HINT, GL_TRUE as GLint);
        assert_eq!(err(), GL_NO_ERROR);
        glProgramParameteri(p, GL_PROGRAM_BINARY_RETRIEVABLE_HINT, GL_FALSE as GLint);
        assert_eq!(err(), GL_NO_ERROR);
        glProgramParameteri(p, GL_PROGRAM_BINARY_RETRIEVABLE_HINT, 2);
        assert_eq!(err(), GL_INVALID_VALUE, "a value other than GL_TRUE/GL_FALSE");
        glProgramParameteri(p, GL_TEXTURE_WRAP_S, GL_TRUE as GLint);
        assert_eq!(err(), GL_INVALID_ENUM, "another pname");
        glProgramParameteri(bad_name(), GL_PROGRAM_BINARY_RETRIEVABLE_HINT, GL_TRUE as GLint);
        assert_eq!(err(), GL_INVALID_OPERATION, "program_exists");
        glProgramParameteri(bad_name(), GL_TEXTURE_WRAP_S, 2);
        assert_eq!(err(), GL_INVALID_OPERATION, "the name before the pname");
    }
    /* the hint is not observable */
    let mut v = 0x5a5a;
    // SAFETY: room for one value.
    unsafe { glGetProgramiv(p, GL_PROGRAM_BINARY_RETRIEVABLE_HINT, &mut v) };
    assert_eq!((err(), v), (GL_INVALID_ENUM, 0x5a5a));
    // SAFETY: plain GL call.
    unsafe { glDeleteProgram(p) };
}

#[test]
fn the_extension_list_has_no_get_program_binary() {
    let _g = context();
    // SAFETY: glGetString returns a NUL-terminated static string.
    let ext = unsafe { CStr::from_ptr(glGetString(GL_EXTENSIONS).cast::<c_char>()) }.to_string_lossy().into_owned();
    assert_eq!(err(), GL_NO_ERROR);
    assert!(ext.split(' ').all(|e| e != "GL_OES_get_program_binary"), "{ext}");
    assert!(!ext.contains("program_binary"), "{ext}");
}
