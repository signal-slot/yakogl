// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The shader built-in limits agree with the GL queries, through the public
//! GL API on the host.
//!
//! GLSL ES 3.00 section 7.3 ties `gl_MaxVertexOutputVectors`,
//! `gl_MaxFragmentInputVectors`, `gl_MaxVertexUniformVectors` and
//! `gl_MaxFragmentUniformVectors` to the GL queries (components / 4 for the
//! first two); GLSL ES 1.00 section 7.4 has `gl_MaxVaryingVectors`. Each is
//! a constant expression, so a shader declares uniform arrays sized from it
//! and the folded value is read back as the reflected array sizes
//! (glGetActiveUniform). The test compares it with glGetIntegerv of the
//! matching pname and pins no literal number.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_builtin_limits
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test es3_builtin_limits
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online --test es3_builtin_limits
//! ```

#![cfg(all(feature = "host-test", feature = "shaderc-online", not(feature = "report-es2")))]

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
    name: c"es3-builtin-limits".as_ptr() as *const c_char,
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

fn query(pname: GLenum) -> i64 {
    let mut v: GLint = i32::MIN;
    // SAFETY: plain GL call, one value.
    unsafe { glGetIntegerv(pname, &mut v) };
    assert_ne!(v, i32::MIN, "glGetIntegerv(0x{pname:x}) wrote nothing");
    i64::from(v)
}

fn compile(ty: GLenum, text: &str) -> GLuint {
    let text = CString::new(text).unwrap();
    // SAFETY: plain GL calls; one NUL-terminated source.
    unsafe {
        let s = glCreateShader(ty);
        let p: [*const c_char; 1] = [text.as_ptr()];
        glShaderSource(s, 1, p.as_ptr(), core::ptr::null());
        glCompileShader(s);
        let mut ok: GLint = 0;
        glGetShaderiv(s, GL_COMPILE_STATUS, &mut ok);
        let mut log = [0 as c_char; 512];
        glGetShaderInfoLog(s, 512, core::ptr::null_mut(), log.as_mut_ptr());
        let log = std::ffi::CStr::from_ptr(log.as_ptr()).to_string_lossy().into_owned();
        assert_eq!(ok, 1, "{text:?} did not compile: {log}");
        s
    }
}

/// Links `vs`/`fs` and returns the value the built-in folds to. A single
/// `u[N]` array would not link for the uniform limits (it is the whole
/// uniform space), so N is carried by two small arrays: `a[N / 8 + 1]` and
/// `b[N - 8 * (N / 8) + 1]`, N = 8 * (size(a) - 1) + size(b) - 1 (the shaders
/// avoid `%` and a zero size).
fn folded(vs: &str, fs: &str) -> i64 {
    let (vs, fs) = (compile(GL_VERTEX_SHADER, vs), compile(GL_FRAGMENT_SHADER, fs));
    // SAFETY: plain GL calls; buffers of the sizes passed.
    unsafe {
        let p = glCreateProgram();
        glAttachShader(p, vs);
        glAttachShader(p, fs);
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        let mut log = [0 as c_char; 512];
        glGetProgramInfoLog(p, 512, core::ptr::null_mut(), log.as_mut_ptr());
        let log = std::ffi::CStr::from_ptr(log.as_ptr()).to_string_lossy().into_owned();
        assert_eq!(ok, 1, "the program did not link: {log}");
        let mut n: GLint = 0;
        glGetProgramiv(p, GL_ACTIVE_UNIFORMS, &mut n);
        let (mut a, mut b) = (None, None);
        for i in 0..n as GLuint {
            let (mut size, mut ty): (GLint, GLenum) = (0, 0);
            let mut name = [0 as c_char; 64];
            glGetActiveUniform(p, i, 64, core::ptr::null_mut(), &mut size, &mut ty, name.as_mut_ptr());
            match std::ffi::CStr::from_ptr(name.as_ptr()).to_bytes() {
                b"a" | b"a[0]" => a = Some(i64::from(size)),
                b"b" | b"b[0]" => b = Some(i64::from(size)),
                _ => {}
            }
        }
        8 * (a.expect("uniform a is not active") - 1) + b.expect("uniform b is not active") - 1
    }
}

const TRIVIAL_VS: &str = "#version 300 es\nvoid main() { gl_Position = vec4(0.0); }\n";
const TRIVIAL_FS: &str = "#version 300 es\nprecision mediump float;\nout vec4 o;\nvoid main() { o = vec4(1.0); }\n";

fn vs_case(name: &str) -> i64 {
    let vs = format!("#version 300 es\nuniform float a[{name} / 8 + 1];\nuniform float b[{name} - 8 * ({name} / 8) + 1];\nvoid main() {{ gl_Position = vec4(a[0] + b[0]); }}\n");
    folded(&vs, TRIVIAL_FS)
}

fn fs_case(name: &str) -> i64 {
    let fs = format!(
        "#version 300 es\nprecision mediump float;\nuniform float a[{name} / 8 + 1];\nuniform float b[{name} - 8 * ({name} / 8) + 1];\nout vec4 o;\nvoid main() {{ o = vec4(a[0] + b[0]); }}\n"
    );
    folded(TRIVIAL_VS, &fs)
}

#[test]
fn vertex_output_vectors() {
    let _g = context();
    assert_eq!(vs_case("gl_MaxVertexOutputVectors"), query(GL_MAX_VERTEX_OUTPUT_COMPONENTS) / 4);
}

#[test]
fn fragment_input_vectors() {
    let _g = context();
    assert_eq!(fs_case("gl_MaxFragmentInputVectors"), query(GL_MAX_FRAGMENT_INPUT_COMPONENTS) / 4);
}

#[test]
fn vertex_uniform_vectors() {
    let _g = context();
    assert_eq!(vs_case("gl_MaxVertexUniformVectors"), query(GL_MAX_VERTEX_UNIFORM_VECTORS));
}

#[test]
fn fragment_uniform_vectors() {
    let _g = context();
    assert_eq!(fs_case("gl_MaxFragmentUniformVectors"), query(GL_MAX_FRAGMENT_UNIFORM_VECTORS));
}

/// `gl_MaxVaryingVectors` is GLSL ES 1.00 only (section 7.4).
#[test]
fn varying_vectors_es100() {
    let _g = context();
    let vs = "uniform float a[gl_MaxVaryingVectors / 8 + 1];\nuniform float b[gl_MaxVaryingVectors - 8 * (gl_MaxVaryingVectors / 8) + 1];\nvoid main() { gl_Position = vec4(a[0] + b[0]); }\n";
    let fs = "precision mediump float;\nvoid main() { gl_FragColor = vec4(1.0); }\n";
    assert_eq!(folded(vs, fs), query(GL_MAX_VARYING_VECTORS));
}
