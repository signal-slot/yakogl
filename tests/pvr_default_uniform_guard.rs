// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The PowerVR default uniform block at its 1536-dword budget, a guard for
//! plan #378 (uniform-block members move from shared registers to `ld`
//! memory loads; default-block uniforms stay as they are). Pinned, outcomes
//! and limits only:
//!
//! - a vertex and a fragment shader whose default uniforms fill exactly the
//!   reported `GL_MAX_VERTEX_UNIFORM_VECTORS` / `GL_MAX_FRAGMENT_UNIFORM_VECTORS`
//!   compile and link;
//! - one vec4 more is refused at link, naming the limit;
//! - a uniform array read at its first and last constant index and at a
//!   run-time index compiles and links.
//!
//! Where block members live and how many shared registers a program uses are
//! NOT pinned here.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test pvr_default_uniform_guard
//! ```

#![cfg(all(feature = "host-test", feature = "gpu-powervr", feature = "shaderc-online", not(feature = "report-es2")))]

use core::ffi::{c_char, c_void};
use std::sync::Mutex;
use std::sync::MutexGuard;
use std::sync::Once;
use yakogl::backend::{GlesHeap, YakoglBackend};
use yakogl::gl::consts::*;
use yakogl::gl::program::*;
use yakogl::gl::shader::*;
use yakogl::gl::state::*;
use yakogl::gl::types::*;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};

/* ---- a context on the host: the smallest OSAL and back end that make one -- */

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
    name: c"pvr-default-uniform-guard".as_ptr() as *const c_char,
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
    flush: None,
    finish: None,
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

static LIBRARY: Mutex<()> = Mutex::new(());
static CONTEXT: Once = Once::new();

/// The context, made once; the guard keeps the tests of this file off the
/// library (single-threaded by contract) one at a time.
fn context() -> MutexGuard<'static, ()> {
    let guard = LIBRARY.lock().unwrap_or_else(|e| e.into_inner());
    CONTEXT.call_once(|| {
        // SAFETY: both tables are statics, and the guard above is held.
        unsafe {
            osal_set_ops(&OSAL);
            yakogl::backend::g_backend = &BACKEND;
            yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
            assert!(!yakogl::gl::context::gl_context_create(8, 8).is_null());
        }
    });
    guard
}

/* ---- shaders and links ----------------------------------------------------- */

fn cstr(b: &[u8]) -> String {
    let n = b.iter().position(|&c| c == 0).unwrap_or(b.len());
    String::from_utf8_lossy(&b[..n]).into_owned()
}

fn shader(ty: GLenum, text: &str) -> Result<GLuint, String> {
    let c = std::ffi::CString::new(text).unwrap();
    // SAFETY: plain GL calls; one NUL-terminated source.
    unsafe {
        let s = glCreateShader(ty);
        let p: [*const c_char; 1] = [c.as_ptr()];
        glShaderSource(s, 1, p.as_ptr(), core::ptr::null());
        glCompileShader(s);
        let mut ok: GLint = 0;
        glGetShaderiv(s, GL_COMPILE_STATUS, &mut ok);
        if ok != 1 {
            let mut log = [0u8; 1024];
            glGetShaderInfoLog(s, 1024, core::ptr::null_mut(), log.as_mut_ptr() as *mut c_char);
            return Err(cstr(&log));
        }
        Ok(s)
    }
}

/// Compile and link; Err carries the shader or program info log.
fn link(vs: &str, fs: &str) -> Result<(), String> {
    let (v, f) = (shader(GL_VERTEX_SHADER, vs)?, shader(GL_FRAGMENT_SHADER, fs)?);
    // SAFETY: plain GL calls.
    unsafe {
        let p = glCreateProgram();
        glAttachShader(p, v);
        glAttachShader(p, f);
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        let r = if ok == 1 {
            Ok(())
        } else {
            let mut log = [0u8; 1024];
            glGetProgramInfoLog(p, 1024, core::ptr::null_mut(), log.as_mut_ptr() as *mut c_char);
            Err(cstr(&log))
        };
        glDeleteProgram(p);
        glDeleteShader(v);
        glDeleteShader(f);
        r
    }
}

fn geti(pname: GLenum) -> usize {
    let mut v = 0;
    // SAFETY: one value.
    unsafe { glGetIntegerv(pname, &mut v) };
    v as usize
}

/// `rows` vec4 of default uniforms as arrays u0, u1, ... of at most 128
/// elements (the online front end keeps an array size in a u8), with the
/// statements that sum every element into `s`.
fn rows_decl(rows: usize) -> (String, String) {
    let (mut decl, mut body) = (String::new(), String::new());
    let (mut left, mut k) = (rows, 0);
    while left > 0 {
        let n = left.min(128);
        decl += &format!("uniform vec4 u{k}[{n}];\n");
        body += &format!("for (int i = 0; i < {n}; i++) s += u{k}[i];\n");
        left -= n;
        k += 1;
    }
    (decl, body)
}

/// A vertex shader with `rows` vec4 of default uniforms, all read.
fn vs_rows(rows: usize) -> String {
    let (decl, body) = rows_decl(rows);
    format!("#version 300 es\nin vec4 p;\n{decl}void main() {{ vec4 s = vec4(0.0);\n{body}gl_Position = p + s; }}\n")
}

fn fs_rows(rows: usize) -> String {
    let (decl, body) = rows_decl(rows);
    format!("#version 300 es\nprecision mediump float;\n{decl}out vec4 o;\nvoid main() {{ vec4 s = vec4(0.0);\n{body}o = s; }}\n")
}

const VS_OK: &str = "#version 300 es\nin vec4 p;\nvoid main() { gl_Position = p; }\n";
const FS_OK: &str = "#version 300 es\nprecision mediump float;\nout vec4 o;\nvoid main() { o = vec4(1.0); }\n";

#[test]
fn reported_vector_limits_are_the_1536_dword_budget() {
    let _g = context();
    assert_eq!(geti(GL_MAX_VERTEX_UNIFORM_VECTORS), 384);
    assert_eq!(geti(GL_MAX_FRAGMENT_UNIFORM_VECTORS), 384);
}

#[test]
fn vertex_default_uniforms_fill_the_reported_limit() {
    let _g = context();
    let n = geti(GL_MAX_VERTEX_UNIFORM_VECTORS);
    link(&vs_rows(n), FS_OK).unwrap_or_else(|e| panic!("{n} vec4 in the vertex shader: {e}"));
}

#[test]
fn fragment_default_uniforms_fill_the_reported_limit() {
    let _g = context();
    let n = geti(GL_MAX_FRAGMENT_UNIFORM_VECTORS);
    link(VS_OK, &fs_rows(n)).unwrap_or_else(|e| panic!("{n} vec4 in the fragment shader: {e}"));
}

#[test]
fn one_vec4_more_in_the_vertex_shader_is_refused_at_link() {
    let _g = context();
    let n = geti(GL_MAX_VERTEX_UNIFORM_VECTORS);
    let e = link(&vs_rows(n + 1), FS_OK).expect_err("over the vertex budget");
    assert!(e.contains("MAX_VERTEX_UNIFORM_VECTORS"), "{e}");
}

#[test]
fn one_vec4_more_in_the_fragment_shader_is_refused_at_link() {
    let _g = context();
    let n = geti(GL_MAX_FRAGMENT_UNIFORM_VECTORS);
    let e = link(VS_OK, &fs_rows(n + 1)).expect_err("over the fragment budget");
    assert!(e.contains("MAX_FRAGMENT_UNIFORM_VECTORS"), "{e}");
}

#[test]
fn uniform_array_reads_at_first_last_and_run_time_index_compile() {
    let _g = context();
    /* arrays of 128, 128 and 127 vec4 and the int k fill the 384-row budget: the first element of
     * the first, the last of the last, and a run-time index into each */
    assert_eq!(geti(GL_MAX_VERTEX_UNIFORM_VECTORS), 3 * 128);
    assert_eq!(geti(GL_MAX_FRAGMENT_UNIFORM_VECTORS), 3 * 128);
    const DECL: &str = "uniform vec4 a[128];\nuniform vec4 b[128];\nuniform vec4 c[127];\nuniform int k;\n";
    const SUM: &str = "a[0] + c[126] + a[k] + b[k] + c[k]";
    let vs = format!("#version 300 es\nin vec4 p;\n{DECL}void main() {{ gl_Position = p + {SUM}; }}\n");
    link(&vs, FS_OK).unwrap_or_else(|e| panic!("vertex: {e}"));
    let fs = format!("#version 300 es\nprecision mediump float;\n{DECL}out vec4 o;\nvoid main() {{ o = {SUM}; }}\n");
    link(VS_OK, &fs).unwrap_or_else(|e| panic!("fragment: {e}"));
}
