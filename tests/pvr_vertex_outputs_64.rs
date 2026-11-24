// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! A PowerVR vertex shader with 16 `vec4` outputs, `gl_Position` and
//! `gl_PointSize` compiles and links, on the host (item pvr-vout64-test).
//!
//! The PowerVR answers GL_MAX_VERTEX_OUTPUT_COMPONENTS 60: the GL core
//! reports `MAX_VARYING_VECTORS * 4` for every GPU but the V3D
//! (src/gl/context.rs:262, with the expectation at src/gl/state.rs:3251).
//! That is a software choice, not a hardware limit, and it is why
//! dEQP-GLES3.functional.implementation_limits.max_vertex_output_components
//! fails: ES 3.0 asks for at least 64. A later change to src/gl/context.rs
//! would raise that answer to 64; this file is the guard that the PowerVR
//! compiler path already carries what such a program declares, so the
//! change needs no board. It pins tri-vout's probe: a vertex shader
//! declaring 16 `vec4` outputs plus `gl_Position` and `gl_PointSize`,
//! linked with a fragment shader that reads all 16, compiles and links on
//! the PowerVR path today. Nothing in the ES 3.0 link refuses it:
//! `varying_budget` (src/shaderc/online/reflect.rs) passes an ES 3.00
//! pair, and no GL-core check counts vertex outputs against the reported
//! limit -- that limit is only ever reported, which is the defect behind
//! the dEQP case, not this test's subject.
//!
//! The limit that is enforced sits one row lower: a `ShcShader` carries 16
//! varying rows (src/shaderc/mod.rs:1380), so a 17th `vec4` output is
//! refused when the vertex shader compiles, "more varyings than the record
//! holds" (src/shaderc/online/powervr/assemble.rs
//! `vertex_output_writes`). The second case pins that refusal, the same
//! string tri-vout's new-issue record quotes.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test pvr_vertex_outputs_64
//! ```

#![cfg(all(feature = "host-test", feature = "gpu-powervr", feature = "shaderc-online", not(feature = "report-es2")))]

use core::ffi::{c_char, c_void};
use std::ffi::CString;
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{GlesHeap, YakoglBackend};
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
    name: c"pvr-vertex-outputs-64".as_ptr() as *const c_char,
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
            assert!(!yakogl::gl::context::gl_context_create(8, 8).is_null());
        }
    });
    // SAFETY: plain GL calls on the one context.
    unsafe {
        glUseProgram(0);
        while glGetError() != GL_NO_ERROR {}
    }
    g
}

/* ---- the program pair ------------------------------------------------------ */

/// A vertex shader with `n` `vec4` outputs `v0..v<n>`, each written, plus
/// `gl_Position` and `gl_PointSize`.
///
/// Every output is a copy of one input, so each output's write reaches the
/// epilogue as four consecutive `uvsw.write` lanes (the input's own VtxIn
/// registers). The assembler has room for 31 write groups
/// (`assemble.rs build_output_writes` and its caller `build_shader`), and
/// that ceiling counts the *shape* of the writes, not the number of
/// outputs: a body whose lanes land in scattered temporaries runs into it
/// at far fewer outputs and would test that shape limit, not the
/// declaration count this file is about.
fn vertex_source(n: usize) -> String {
    let mut s = String::from("#version 300 es\nin vec4 a_in;\n");
    for i in 0..n {
        s += &format!("out vec4 v{i};\n");
    }
    s += "void main() {\n";
    for i in 0..n {
        s += &format!("  v{i} = a_in;\n");
    }
    s += "  gl_Position = a_in;\n  gl_PointSize = 1.0;\n}\n";
    s
}

/// A fragment shader reading all `n` of them and summing them into its one
/// output, so every one is active.
fn fragment_source(n: usize) -> String {
    let mut s = String::from("#version 300 es\nprecision mediump float;\n");
    for i in 0..n {
        s += &format!("in vec4 v{i};\n");
    }
    s += "out vec4 o;\nvoid main() {\n  o = ";
    for i in 0..n {
        if i > 0 {
            s += " + ";
        }
        s += &format!("v{i}");
    }
    s += ";\n}\n";
    s
}

fn shader_log(s: GLuint) -> String {
    let mut log = [0 as c_char; 1024];
    // SAFETY: plain GL calls; a log buffer of the size passed.
    unsafe {
        glGetShaderInfoLog(s, 1024, core::ptr::null_mut(), log.as_mut_ptr());
        std::ffi::CStr::from_ptr(log.as_ptr()).to_string_lossy().into_owned()
    }
}

fn program_log(p: GLuint) -> String {
    let mut log = [0 as c_char; 1024];
    // SAFETY: plain GL calls; a log buffer of the size passed.
    unsafe {
        glGetProgramInfoLog(p, 1024, core::ptr::null_mut(), log.as_mut_ptr());
        std::ffi::CStr::from_ptr(log.as_ptr()).to_string_lossy().into_owned()
    }
}

/// Compile one shader: its handle, or the refusal with the info log.
fn compile(ty: GLenum, text: &str) -> Result<GLuint, String> {
    let text = CString::new(text).unwrap();
    // SAFETY: plain GL calls; one NUL-terminated source.
    unsafe {
        let s = glCreateShader(ty);
        let p: [*const c_char; 1] = [text.as_ptr()];
        glShaderSource(s, 1, p.as_ptr(), core::ptr::null());
        glCompileShader(s);
        let mut ok: GLint = 0;
        glGetShaderiv(s, GL_COMPILE_STATUS, &mut ok);
        if ok != 1 {
            let who = if ty == GL_VERTEX_SHADER { "vertex" } else { "fragment" };
            return Err(format!("the {who} shader did not compile: {}", shader_log(s)));
        }
        Ok(s)
    }
}

/// Compile both shaders of a pair: the handles, or the refusal.
fn compile_pair(vs_src: &str, fs_src: &str) -> Result<(GLuint, GLuint), String> {
    let vs = compile(GL_VERTEX_SHADER, vs_src)?;
    let fs = compile(GL_FRAGMENT_SHADER, fs_src)?;
    Ok((vs, fs))
}

/// Attach both shaders to a fresh program and link: the program, or the
/// link refusal with its log.
fn link_pair(vs: GLuint, fs: GLuint) -> Result<GLuint, String> {
    // SAFETY: plain GL calls.
    unsafe {
        let p = glCreateProgram();
        glAttachShader(p, vs);
        glAttachShader(p, fs);
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        if ok != 1 {
            return Err(program_log(p));
        }
        Ok(p)
    }
}

/// The pair through compile and link: the linked program, or the refusal
/// (the compile error, or the link error).
fn build_pair(n: usize) -> Result<GLuint, String> {
    let (vs, fs) = compile_pair(&vertex_source(n), &fragment_source(n))?;
    link_pair(vs, fs)
}

/// The tri-vout probe: 16 `vec4` outputs plus `gl_Position` and
/// `gl_PointSize`, read by the fragment shader in full, compile and link
/// on the PowerVR path. This is what a src/gl change raising
/// GL_MAX_VERTEX_OUTPUT_COMPONENTS from 60 to 64 relies on.
#[test]
fn sixteen_vec4_outputs_with_builtins_compile_and_link() {
    let _g = context();
    const N: usize = 16;
    let p = build_pair(N)
        .unwrap_or_else(|e| panic!("16 vec4 outputs + gl_Position + gl_PointSize must compile and link: {e}"));
    // SAFETY: plain GL calls; one NUL-terminated name each.
    let locs: Vec<GLint> = unsafe {
        let locs = (0..N)
            .map(|i| {
                let name = CString::new(format!("v{i}")).unwrap();
                glGetAttribLocation(p, name.as_ptr())
            })
            .collect();
        assert_eq!(glGetError(), GL_NO_ERROR, "querying the outputs sets no error");
        locs
    };
    /* Vertex-output locations are not exposed here: glGetAttribLocation
     * answers the vertex shader's *inputs* (src/gl/program.rs), there is
     * no glGetActiveVarying in this build, and the resource queries are
     * ES 3.1 -- every output answers -1. Should a later change expose
     * them, the test starts expecting one distinct row per output. */
    if !locs.iter().all(|l| *l < 0) {
        let mut seen: Vec<GLint> = locs.iter().filter(|l| **l >= 0).copied().collect();
        let n = seen.len();
        seen.sort_unstable();
        seen.dedup();
        assert_eq!(seen.len(), n, "the 16 outputs each get a distinct varying location: {locs:?}");
    }
}

/// One `vec4` output past the 16 rows a `ShcShader` record holds: the
/// refusal arrives when the vertex shader compiles ("more varyings than
/// the record holds", assemble.rs `vertex_output_writes`), so the program
/// can never link. The same refusal tri-vout recorded for 20 `float`
/// varyings; the 16-row table is the PowerVR's real ceiling, not the
/// reported component limit.
#[test]
fn seventeen_vec4_outputs_do_not_link() {
    let _g = context();
    const N: usize = 17;
    let outcome = build_pair(N);
    match outcome {
        Err(e) => {
            assert!(e.contains("more varyings than the record holds"), "17 outputs are refused for a new reason: {e}")
        }
        Ok(_) => panic!("17 vec4 outputs linked; the 16-row ShcShader.varyings record no longer caps them"),
    }
}
