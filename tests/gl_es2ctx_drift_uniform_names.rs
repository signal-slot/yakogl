// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! ES 2.0 context: uniform names of 63, 64, 65 and well over 64 bytes keep
//! their answers (guard for eceaabe6, "the last 64-byte field readers go
//! through the whole uniform name").
//!
//! dEQP-GLES2 uniform_api.* (info_query, value.*, random.*) queries and sets
//! uniforms by names such as `uniformStruct[1].arrayMember[2].fieldName`.
//! A GLSL ES 1.00 fragment shader declares `struct S { float f; vec2 v;
//! float a[2]; }` and `uniform S <name>[2]` plus a plain `uniform float
//! <name>x[3]`, with the base name chosen so that the member names are under
//! 64, exactly 63, 64, 65 and well over 64 bytes. For each:
//! glGetActiveUniform (names, sizes, types) and GL_ACTIVE_UNIFORM_MAX_LENGTH,
//! glGetUniformLocation for every element/member form (with and without
//! `[0]`), glUniform* then glGetUniform* round trips, and an unknown or
//! too-long name answering -1 without an error. Both back ends run the same
//! body, each in its own module.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online,report-es2 --test gl_es2ctx_drift_uniform_names
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table,report-es2 --test gl_es2ctx_drift_uniform_names
//! ```
#![cfg(all(
    feature = "host-test",
    feature = "shaderc-online",
    feature = "report-es2",
    any(feature = "gpu-vivante", feature = "gpu-v3d")
))]

use core::ffi::{c_char, c_void, CStr};
use std::ffi::CString;
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::*;
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
    name: c"gl-es2ctx-drift-uniform-names".as_ptr() as *const c_char,
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

static LOCK: Mutex<()> = Mutex::new(());
static ONCE: Once = Once::new();

/// The one ES 2.0 context, the online compiler behind it; serialises the tests.
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
    unsafe { while glGetError() != GL_NO_ERROR {} }
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
        assert_eq!(ok, 1, "the shader compiles");
        s
    }
}

fn program_log(p: GLuint) -> String {
    let mut buf = [0 as c_char; 1024];
    let mut len: GLsizei = 0;
    // SAFETY: a buffer of the size announced; GL NUL-terminates the log.
    unsafe {
        glGetProgramInfoLog(p, buf.len() as GLsizei, &mut len, buf.as_mut_ptr());
        CStr::from_ptr(buf.as_ptr()).to_string_lossy().into_owned()
    }
}

const VS: &str = "attribute highp vec4 a_position;\nvoid main() { gl_Position = a_position; }\n";

/// `n` characters of identifier: "u" then lower-case letters.
fn base(n: usize) -> String {
    let mut s = String::from("u");
    while s.len() < n {
        s.push((b'a' + (s.len() % 26) as u8) as char);
    }
    s
}

fn fs(b: &str) -> String {
    format!(
        "precision mediump float;\n\
         struct S {{ mediump float f; mediump vec2 v; mediump float a[2]; }};\n\
         uniform S {b}[2];\n\
         uniform mediump float {b}x[3];\n\
         void main() {{\n\
           float t = {b}[0].f + {b}[1].f + {b}[0].v.x + {b}[1].v.y + {b}[0].a[0] + {b}[0].a[1] + {b}[1].a[0] + {b}[1].a[1];\n\
           t += {b}x[0] + {b}x[1] + {b}x[2];\n\
           gl_FragColor = vec4(t);\n\
         }}\n"
    )
}

fn loc(p: GLuint, name: &str) -> GLint {
    let n = CString::new(name).unwrap();
    // SAFETY: a NUL-terminated name.
    unsafe { glGetUniformLocation(p, n.as_ptr()) }
}

fn active(p: GLuint) -> Vec<(String, GLint, GLenum)> {
    let (mut n, mut maxlen): (GLint, GLint) = (0, 0);
    // SAFETY: plain queries on a linked program.
    unsafe {
        glGetProgramiv(p, GL_ACTIVE_UNIFORMS, &mut n);
        glGetProgramiv(p, GL_ACTIVE_UNIFORM_MAX_LENGTH, &mut maxlen);
    }
    let mut out = Vec::new();
    for i in 0..n as GLuint {
        let mut buf = vec![0 as c_char; 512];
        let (mut len, mut size, mut ty): (GLsizei, GLint, GLenum) = (0, 0, 0);
        // SAFETY: a buffer of the size announced.
        unsafe { glGetActiveUniform(p, i, buf.len() as GLsizei, &mut len, &mut size, &mut ty, buf.as_mut_ptr()) };
        // SAFETY: GL NUL-terminates.
        let name = unsafe { CStr::from_ptr(buf.as_ptr()) }.to_string_lossy().into_owned();
        assert_eq!(len as usize, name.len(), "the length written is the name's: {name}");
        out.push((name, size, ty));
    }
    let longest = out.iter().map(|(s, _, _)| s.len() + 1).max().unwrap_or(0) as GLint;
    assert_eq!(maxlen, longest, "GL_ACTIVE_UNIFORM_MAX_LENGTH is the longest name and its NUL: {out:?}");
    out
}

fn getf(p: GLuint, l: GLint, n: usize) -> Vec<f32> {
    let mut v = vec![-77.0f32; n + 1];
    // SAFETY: room for n floats and a guard.
    unsafe { glGetUniformfv(p, l, v.as_mut_ptr()) };
    assert_eq!(v[n], -77.0, "no write past the value");
    v.truncate(n);
    v
}

/// One base name of `n` characters through the whole list of checks.
fn one(n: usize) {
    let b = base(n);
    let (v, f) = (shader(GL_VERTEX_SHADER, VS), shader(GL_FRAGMENT_SHADER, &fs(&b)));
    // SAFETY: plain GL calls on names made above.
    unsafe {
        let p = glCreateProgram();
        glAttachShader(p, v);
        glAttachShader(p, f);
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        assert_eq!(ok, 1, "base {n} links: {}", program_log(p));
        assert_eq!(glGetError(), GL_NO_ERROR);

        let act = active(p);
        let find = |nm: &str| act.iter().find(|(s, _, _)| s == nm).cloned();
        // The plain array: one row, `x[0]`, size 3.
        assert_eq!(find(&format!("{b}x[0]")), Some((format!("{b}x[0]"), 3, GL_FLOAT)), "{act:?}");
        // The struct members.
        let mut members = 0;
        for (nm, size, ty) in &act {
            if nm.starts_with(&b) && !nm.starts_with(&format!("{b}x")) {
                members += 1;
                let want = match nm.rsplit('.').next().unwrap() {
                    s if s.starts_with('f') => (1, GL_FLOAT),
                    s if s.starts_with('v') => (1, GL_FLOAT_VEC2),
                    s if s.starts_with('a') => (2, GL_FLOAT),
                    s => panic!("member {s}"),
                };
                assert_eq!((*size, *ty), want, "{nm} in {act:?}");
            }
        }
        assert!(members >= 6, "the struct array's members are active: {act:?}");

        // Locations: every form dEQP uses.
        let forms = [
            format!("{b}[0].f"),
            format!("{b}[1].f"),
            format!("{b}[0].v"),
            format!("{b}[1].v"),
            format!("{b}[0].a"),
            format!("{b}[0].a[0]"),
            format!("{b}[0].a[1]"),
            format!("{b}[1].a"),
            format!("{b}[1].a[0]"),
            format!("{b}[1].a[1]"),
            format!("{b}x"),
            format!("{b}x[0]"),
            format!("{b}x[1]"),
            format!("{b}x[2]"),
        ];
        let locs: Vec<GLint> = forms.iter().map(|s| loc(p, s)).collect();
        for (s, l) in forms.iter().zip(&locs) {
            assert!(*l >= 0, "{s} has a location (base {n}): {act:?}");
        }
        assert_eq!(locs[10], locs[11], "x and x[0] are one location");
        assert_eq!(locs[4], locs[5], "a and a[0] are one location");
        assert_eq!(locs[7], locs[8], "a and a[0] are one location");
        let mut uniq = locs.clone();
        uniq.sort();
        uniq.dedup();
        assert_eq!(uniq.len(), locs.len() - 3, "distinct elements, distinct locations: {locs:?}");
        assert_eq!(glGetError(), GL_NO_ERROR);

        // Round trips.
        glUseProgram(p);
        glUniform1f(locs[0], 1.5);
        glUniform1f(locs[1], 2.5);
        glUniform2f(locs[2], 3.0, 4.0);
        glUniform2f(locs[3], 5.0, 6.0);
        let a = [7.0f32, 8.0];
        glUniform1fv(locs[4], 2, a.as_ptr());
        glUniform1f(locs[8], 9.0);
        glUniform1f(locs[9], 10.0);
        let x = [11.0f32, 12.0, 13.0];
        glUniform1fv(locs[10], 3, x.as_ptr());
        assert_eq!(glGetError(), GL_NO_ERROR, "base {n} sets");
        assert_eq!(getf(p, locs[0], 1), [1.5]);
        assert_eq!(getf(p, locs[1], 1), [2.5]);
        assert_eq!(getf(p, locs[2], 2), [3.0, 4.0]);
        assert_eq!(getf(p, locs[3], 2), [5.0, 6.0]);
        assert_eq!(getf(p, locs[5], 1), [7.0]);
        assert_eq!(getf(p, locs[6], 1), [8.0]);
        assert_eq!(getf(p, locs[8], 1), [9.0]);
        assert_eq!(getf(p, locs[9], 1), [10.0]);
        assert_eq!(getf(p, locs[11], 1), [11.0]);
        assert_eq!(getf(p, locs[12], 1), [12.0]);
        assert_eq!(getf(p, locs[13], 1), [13.0]);
        assert_eq!(glGetError(), GL_NO_ERROR, "base {n} gets");

        // Unknown and too-long names: -1, no error.
        let unknown = [
            format!("{b}y"),
            format!("{b}[2].f"),
            format!("{b}[0].g"),
            format!("{b}[0].f.x"),
            format!("{b}x[3]"),
            format!("{b}{b}{b}"),
            format!("{b}[0].f{}", "z".repeat(300)),
            b[..b.len() - 1].to_string(),
            String::new(),
        ];
        for s in &unknown {
            assert_eq!(loc(p, s), -1, "unknown name of {} bytes", s.len());
        }
        assert_eq!(glGetError(), GL_NO_ERROR, "base {n} unknown names raise nothing");
        glUseProgram(0);
        glDeleteShader(v);
        glDeleteShader(f);
        glDeleteProgram(p);
    }
}

/// The longest member name is `base + 8` (`[0].a[0]`); the base lengths put
/// it under 64, at 63, 64, 65 and well over 64.
fn run_all() {
    let _g = context();
    // SAFETY: the context's static version string.
    let version = unsafe { CStr::from_ptr(glGetString(GL_VERSION) as *const c_char) }.to_string_lossy().into_owned();
    assert!(version.starts_with("OpenGL ES 2.0"), "an ES 2.0 context: {version}");
    for n in [8, 40, 55, 56, 57, 58, 59, 60, 62, 70, 120, 200] {
        one(n);
    }
}

#[cfg(feature = "gpu-vivante")]
mod vivante {
    #[test]
    fn long_uniform_names_on_the_gc7000() {
        super::run_all();
    }
}

#[cfg(feature = "gpu-v3d")]
mod v3d {
    #[test]
    fn long_uniform_names_on_the_v3d() {
        super::run_all();
    }
}
