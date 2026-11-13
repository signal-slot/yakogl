// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The varying and uniform limits a `report-es2` build answers, pinned on
//! every GPU build (plan #359, item es2-limits-guard).
//!
//! Plan #359 raises the ES 3.0 limits (MAX_VARYING_VECTORS on the V3D,
//! MAX_VERTEX_OUTPUT/FRAGMENT_INPUT_COMPONENTS, MAX_UNIFORM_BLOCK_SIZE, the
//! combined uniform components). Under `report-es2` none of that may show:
//! the ES 2.0 vector limits keep their answers, the ES 3.0-only pnames stay
//! GL_INVALID_ENUM (ES 2.0 6.1.1: a pname that is not in the state tables is
//! an error, and a failing query writes nothing) through glGetIntegerv,
//! glGetFloatv and glGetBooleanv, and the shader's `gl_MaxVaryingVectors`
//! (GLSL ES 1.00 7.4) equals the queried value. tests/es2_report_guard.rs
//! pins the wider surface on the PowerVR; this file is the part the raises
//! touch, runnable on all three GPU builds, same stand-in context.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table,report-es2 --test es2_report_limits
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online,report-es2 --test es2_report_limits
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online,report-es2 --test es2_report_limits
//! ```
//!
//! Compiled only with `report-es2` (an ES 3.0 build answers differently on
//! purpose) and `shaderc-online` (the shader rows need the compiler).

#![cfg(all(feature = "host-test", feature = "report-es2", feature = "shaderc-online"))]

use core::ffi::{c_char, c_void};
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{GlesHeap, YakoglBackend};
use yakogl::gl::consts::*;
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
    name: c"es2-report-limits".as_ptr() as *const c_char,
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

/// One context for the whole file; the tests take turns on it.
static LOCK: Mutex<()> = Mutex::new(());
static ONCE: Once = Once::new();

/// The shared setup: the context, current, with no error pending.
fn context() -> MutexGuard<'static, ()> {
    let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    ONCE.call_once(|| {
        // SAFETY: both tables are statics; the lock above serialises the tests.
        unsafe {
            osal_set_ops(&OSAL);
            yakogl::backend::g_backend = &BACKEND;
            #[cfg(feature = "shaderc-online")]
            {
                yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
            }
            assert!(!yakogl::gl::context::gl_context_create(8, 8).is_null());
        }
    });
    // SAFETY: a plain GL call on the one context.
    unsafe { while glGetError() != GL_NO_ERROR {} }
    g
}

/* ---- helpers ------------------------------------------------------------------ */

fn err() -> GLenum {
    // SAFETY: a plain GL call.
    unsafe { glGetError() }
}

/// glGetIntegerv: the first value, or the error it raised (a failing query
/// writes nothing, ES 2.0 6.1).
fn geti(pname: GLenum) -> Result<GLint, GLenum> {
    let mut v = [0x5A5A; 4];
    // SAFETY: room for four values, the most any pname yields.
    unsafe { glGetIntegerv(pname, v.as_mut_ptr()) };
    match err() {
        GL_NO_ERROR => Ok(v[0]),
        e => {
            assert_eq!(v, [0x5A5A; 4], "glGetIntegerv(0x{pname:x}) wrote while failing");
            Err(e)
        }
    }
}

/// glGetFloatv, same contract (NaN marks "untouched").
fn getf(pname: GLenum) -> Result<GLfloat, GLenum> {
    let mut v = [f32::NAN; 4];
    // SAFETY: room for four values.
    unsafe { glGetFloatv(pname, v.as_mut_ptr()) };
    match err() {
        GL_NO_ERROR => Ok(v[0]),
        e => {
            assert!(v.iter().all(|x| x.is_nan()), "glGetFloatv(0x{pname:x}) wrote while failing");
            Err(e)
        }
    }
}

/// glGetBooleanv, same contract (7 marks "untouched").
fn getb(pname: GLenum) -> Result<GLboolean, GLenum> {
    let mut v: [GLboolean; 4] = [7; 4];
    // SAFETY: room for four values.
    unsafe { glGetBooleanv(pname, v.as_mut_ptr()) };
    match err() {
        GL_NO_ERROR => Ok(v[0]),
        e => {
            assert_eq!(v, [7; 4], "glGetBooleanv(0x{pname:x}) wrote while failing");
            Err(e)
        }
    }
}

/// `[("GL_FOO", GL_FOO), ...]`: an enum beside its name, for the messages.
macro_rules! named {
    ($($e:ident),* $(,)?) => {
        [$((stringify!($e), $e)),*]
    };
}

/* ---- per-GPU values -------------------------------------------------------------- */

/// GL_MAX_VARYING_VECTORS under `report-es2`: what the GPU carries and the
/// driver programs (context.rs MAX_VARYING_VECTORS): 15 on the GC7000, 8 on
/// the PowerVR and the V3D. ES 2.0 table 6.20 asks for at least 8. Plan #359
/// raises the V3D's ES 3.0 value only; the ES 2.0 answer there stays 8.
const VARYING_VECTORS: GLint = if cfg!(feature = "gpu-vivante") { 15 } else { 8 };

/// GL_MAX_{VERTEX,FRAGMENT}_UNIFORM_VECTORS: MAX_UNIFORM_DWORDS / 4 = 384 on
/// every GPU (context.rs; tests/es2_report_guard.rs pins the same). ES 2.0
/// table 6.20 asks for 128 and 16.
const UNIFORM_VECTORS: GLint = 384;

/* ---- the queries ----------------------------------------------------------------- */

/// ES 2.0 table 6.20: the vector limits keep their ES 2.0 values through all
/// three query forms. Booleans are "non-zero" (2.1: a state value converts
/// to GL_TRUE unless it is zero).
#[test]
fn es2_vector_limits_keep_their_answers() {
    let _g = context();
    for (n, q, want) in [
        ("GL_MAX_VARYING_VECTORS (min 8)", GL_MAX_VARYING_VECTORS, VARYING_VECTORS),
        ("GL_MAX_VERTEX_UNIFORM_VECTORS (min 128)", GL_MAX_VERTEX_UNIFORM_VECTORS, UNIFORM_VECTORS),
        ("GL_MAX_FRAGMENT_UNIFORM_VECTORS (min 16)", GL_MAX_FRAGMENT_UNIFORM_VECTORS, UNIFORM_VECTORS),
    ] {
        assert_eq!(geti(q), Ok(want), "{n} glGetIntegerv");
        assert_eq!(getf(q), Ok(want as GLfloat), "{n} glGetFloatv");
        assert_eq!(getb(q), Ok(GL_TRUE as GLboolean), "{n} glGetBooleanv");
    }
}

/// ES 2.0 6.1.1: none of these is a state variable of ES 2.0 (they are ES 3.0
/// table 6.33/6.34 entries and no claimed extension adds them), so each is
/// GL_INVALID_ENUM through every query form and nothing is written. These
/// are the pnames the #359 raises move, so a raise that forgets the
/// `report-es2` gate shows here.
#[test]
fn es3_only_limits_are_invalid_enum_in_all_forms() {
    let _g = context();
    for (n, q) in named![
        GL_MAX_VARYING_COMPONENTS,
        GL_MAX_VERTEX_OUTPUT_COMPONENTS,
        GL_MAX_FRAGMENT_INPUT_COMPONENTS,
        GL_MAX_UNIFORM_BLOCK_SIZE,
        GL_MAX_COMBINED_VERTEX_UNIFORM_COMPONENTS,
        GL_MAX_COMBINED_FRAGMENT_UNIFORM_COMPONENTS,
        GL_MAX_VERTEX_UNIFORM_BLOCKS,
        GL_MAX_UNIFORM_BUFFER_BINDINGS,
    ] {
        assert_eq!(geti(q), Err(GL_INVALID_ENUM), "{n} glGetIntegerv");
        assert_eq!(getf(q), Err(GL_INVALID_ENUM), "{n} glGetFloatv");
        assert_eq!(getb(q), Err(GL_INVALID_ENUM), "{n} glGetBooleanv");
    }
}

/* ---- the shader's gl_Max* ---------------------------------------------------------- */

/// Compile and link a program from `vs` and `fs` (GLSL ES 1.00): Ok, or the
/// first refusal (a compile or a link log).
fn builds(vs: &str, fs: &str) -> Result<(), String> {
    use yakogl::gl::program::*;
    use yakogl::gl::shader::*;
    let mk = |kind: GLenum, src: &str| -> Result<GLuint, String> {
        // SAFETY: the source is live for the calls; the log buffer is as long as it is told.
        unsafe {
            let s = glCreateShader(kind);
            let p = [src.as_ptr().cast::<c_char>()];
            let l = [src.len() as GLint];
            glShaderSource(s, 1, p.as_ptr(), l.as_ptr());
            glCompileShader(s);
            let mut ok: GLint = 0;
            glGetShaderiv(s, GL_COMPILE_STATUS, &mut ok);
            let mut log = [0u8; 512];
            let mut n: GLsizei = 0;
            glGetShaderInfoLog(s, log.len() as GLsizei, &mut n, log.as_mut_ptr().cast());
            let log = String::from_utf8_lossy(&log[..n.max(0) as usize]).into_owned();
            if ok != 0 {
                Ok(s)
            } else {
                glDeleteShader(s);
                Err(format!("compile: {log}"))
            }
        }
    };
    let v = mk(GL_VERTEX_SHADER, vs)?;
    let f = mk(GL_FRAGMENT_SHADER, fs)?;
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
        glGetProgramInfoLog(p, log.len() as GLsizei, &mut n, log.as_mut_ptr().cast());
        glDeleteProgram(p);
        glDeleteShader(v);
        glDeleteShader(f);
        if ok != 0 {
            Ok(())
        } else {
            Err(format!("link: {}", String::from_utf8_lossy(&log[..n.max(0) as usize])))
        }
    }
}

/// GLSL ES 1.00 7.4: `gl_MaxVaryingVectors` is a constant integer equal to
/// the value glGetIntegerv(GL_MAX_VARYING_VECTORS) returns. A varying array of
/// exactly that size (a constant expression, so a legal array size) must
/// build, and one vector more must not (ES 2.0 2.10.4: a link fails when the
/// varyings do not fit GL_MAX_VARYING_VECTORS; the GC7000 code generator
/// already refuses the 16-vector vertex shader at compile time, which is
/// the same refusal one step earlier, so either step counts).
#[test]
fn shader_gl_max_varying_vectors_equals_the_query() {
    let _g = context();
    let n = geti(GL_MAX_VARYING_VECTORS).unwrap();
    assert_eq!(n, VARYING_VECTORS);
    let vs = |size: &str| {
        format!(
            "attribute vec4 a; varying vec4 v[{size}];\n\
             void main() {{ v[0] = a; gl_Position = a; }}"
        )
    };
    let fs = |size: &str| {
        format!(
            "precision mediump float; varying vec4 v[{size}];\n\
             void main() {{ gl_FragColor = v[0]; }}"
        )
    };
    assert_eq!(
        builds(&vs("gl_MaxVaryingVectors"), &fs("gl_MaxVaryingVectors")),
        Ok(()),
        "gl_MaxVaryingVectors-sized varying array"
    );
    let q = n.to_string();
    assert_eq!(builds(&vs(&q), &fs(&q)), Ok(()), "{n}-sized varying array");
    let over = (n + 1).to_string();
    assert!(
        builds(&vs(&over), &fs(&over)).is_err(),
        "{over} varying vectors exceed GL_MAX_VARYING_VECTORS and must not build"
    );
}
