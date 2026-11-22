// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! ES 2.0 context against 70909751 (glLinkProgram rejects duplicate
//! fragment-output locations and locations at or past `draw_targets()`),
//! pinned only by the ES 3.0 tests in tests/es3_mrt4_glsl.rs. The check is in
//! src/gl/program.rs `check_fragment_output_locations`; it returns early when
//! the build reports ES 2.0 (`REPORT_ES2`), and GLSL ES 1.00 has no
//! `layout(location)` anyway. This drives the real GL core in a `report-es2`
//! build and asserts, in the ES 2.0 context:
//!
//! (a) a `#version 100` fragment shader writing gl_FragColor links and draws
//!     (the draw reaches the back end, no error);
//! (b) one writing gl_FragData[0] links, and draws;
//! (c) one writing both gl_FragColor and gl_FragData[0] is no program (the
//!     compile or the link refuses it), and its link log carries none of the
//!     new check's "fragment output" lines;
//! (d) a `layout(location = 0) out` declaration in a `#version 100` shader
//!     is refused at compile, not as a location error, and `#version 300 es`
//!     shaders with a duplicate or an out-of-range location raise no
//!     "fragment output" link line in this context.
//!
//! The stand-in back end executes nothing; a draw is asserted by the draws it
//! hands the back end. The GL core is the same under every back end.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online,report-es2 --test gl_es2ctx_drift_frag_location
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table,report-es2 --test gl_es2ctx_drift_frag_location
//! cargo clippy --no-default-features --features host-test,gpu-vivante,shaderc-online,report-es2 --test gl_es2ctx_drift_frag_location -- -D warnings
//! cargo clippy --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table,report-es2 --test gl_es2ctx_drift_frag_location -- -D warnings
//! ```
#![cfg(all(
    feature = "host-test",
    feature = "shaderc-online",
    feature = "report-es2",
    any(feature = "gpu-vivante", feature = "gpu-v3d")
))]

use core::ffi::{c_char, c_void};
use std::ffi::CString;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::*;
use yakogl::gl::buffer::*;
use yakogl::gl::consts::*;
use yakogl::gl::draw::*;
use yakogl::gl::program::*;
use yakogl::gl::shader::*;
use yakogl::gl::state::*;
use yakogl::gl::types::*;
use yakogl::gl::vertex::*;
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

static DRAWS: AtomicUsize = AtomicUsize::new(0);

unsafe extern "C" fn b_draw(_d: *const GlesDraw) -> i32 {
    DRAWS.fetch_add(1, Ordering::SeqCst);
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
    name: c"es2ctx-frag-location".as_ptr() as *const c_char,
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

/// Compile `text`: the shader and whether it compiled.
fn compile(ty: GLenum, text: &str) -> (GLuint, bool) {
    let text = CString::new(text).unwrap();
    // SAFETY: plain GL calls; one NUL-terminated source.
    unsafe {
        let s = glCreateShader(ty);
        let p: [*const c_char; 1] = [text.as_ptr()];
        glShaderSource(s, 1, p.as_ptr(), core::ptr::null());
        glCompileShader(s);
        let mut ok: GLint = 0;
        glGetShaderiv(s, GL_COMPILE_STATUS, &mut ok);
        (s, ok == 1)
    }
}

/// Compile and link `vs`/`fs`: (both compiled, linked, link log, program).
fn build(vs: &str, fs: &str) -> (bool, bool, String, GLuint) {
    let (v, vok) = compile(GL_VERTEX_SHADER, vs);
    let (f, fok) = compile(GL_FRAGMENT_SHADER, fs);
    // SAFETY: plain GL calls; the log buffer is 512 bytes.
    unsafe {
        let p = glCreateProgram();
        glAttachShader(p, v);
        glAttachShader(p, f);
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        let mut log = [0u8; 512];
        let mut n: GLsizei = 0;
        glGetProgramInfoLog(p, 512, &mut n, log.as_mut_ptr().cast::<c_char>());
        let log = String::from_utf8_lossy(&log[..n.max(0) as usize]).into_owned();
        while glGetError() != GL_NO_ERROR {}
        (vok && fok, ok == 1, log, p)
    }
}

const VS100: &str = "attribute vec4 p;\nvoid main() { gl_Position = p; }\n";

fn fs100(body: &str) -> String {
    format!("precision mediump float;\nvoid main() {{ {body} }}\n")
}

/// Use `p` and draw one triangle from a buffer: the draws that reached the
/// back end, and the error raised.
fn draw_with(p: GLuint) -> (usize, GLenum) {
    let verts: [f32; 12] = [0.0, 0.0, 0.0, 1.0, 1.0, 0.0, 0.0, 1.0, 0.0, 1.0, 0.0, 1.0];
    let before = DRAWS.load(Ordering::SeqCst);
    // SAFETY: plain GL calls; glBufferData copies the vertex data.
    unsafe {
        let mut b = 0;
        glGenBuffers(1, &mut b);
        glBindBuffer(GL_ARRAY_BUFFER, b);
        glBufferData(GL_ARRAY_BUFFER, 48, verts.as_ptr().cast::<c_void>(), GL_STATIC_DRAW);
        glUseProgram(p);
        let a = glGetAttribLocation(p, c"p".as_ptr());
        assert!(a >= 0, "p is active");
        glVertexAttribPointer(a as GLuint, 4, GL_FLOAT, 0, 0, core::ptr::null());
        glEnableVertexAttribArray(a as GLuint);
        glDrawArrays(GL_TRIANGLES, 0, 3);
        glFinish();
        glDisableVertexAttribArray(a as GLuint);
        glBindBuffer(GL_ARRAY_BUFFER, 0);
        glDeleteBuffers(1, &b);
    }
    (DRAWS.load(Ordering::SeqCst) - before, err())
}

#[test]
fn the_context_is_es2() {
    let _g = context();
    // SAFETY: plain GL call; the string is static.
    let v = unsafe { std::ffi::CStr::from_ptr(glGetString(GL_VERSION).cast::<c_char>()) };
    assert!(v.to_str().unwrap().starts_with("OpenGL ES 2.0"), "{v:?}");
}

#[test]
fn fragcolor_links_and_draws() {
    let _g = context();
    let (c, l, log, p) = build(VS100, &fs100("gl_FragColor = vec4(1.0, 0.0, 0.0, 1.0);"));
    assert!(c && l, "gl_FragColor program: compiled={c} linked={l}\n{log}");
    let (d, e) = draw_with(p);
    assert_eq!(e, GL_NO_ERROR);
    assert_eq!(d, 1, "the draw reaches the back end");
}

#[test]
fn fragdata0_links_and_draws() {
    let _g = context();
    let (c, l, log, p) = build(VS100, &fs100("gl_FragData[0] = vec4(0.0, 1.0, 0.0, 1.0);"));
    assert!(c && l, "gl_FragData[0] program: compiled={c} linked={l}\n{log}");
    let (d, e) = draw_with(p);
    assert_eq!(e, GL_NO_ERROR);
    assert_eq!(d, 1, "the draw reaches the back end");
}

#[test]
fn fragcolor_and_fragdata_together_is_not_a_location_error() {
    let _g = context();
    let fs = fs100("gl_FragColor = vec4(1.0); gl_FragData[0] = vec4(0.5);");
    let (c, l, log, _) = build(VS100, &fs);
    // GLSL ES 1.00 7.2: writing both is undefined; the compiler or the linker
    // refuses it, as before 70909751. Either way it is not a program, and the
    // log is not the new check's.
    assert!(!(c && l), "both outputs written: compiled={c} linked={l}");
    assert!(!log.contains("fragment output"), "no location line in an ES 2.0 link: {log}");
}

#[test]
fn layout_location_is_not_es2() {
    let _g = context();
    let fs =
        "#version 100\nprecision mediump float;\nlayout(location = 0) out vec4 c;\nvoid main() { c = vec4(1.0); }\n";
    let (c, l, log, _) = build(VS100, fs);
    assert!(!c && !l, "layout(location) in #version 100: compiled={c} linked={l}");
    assert!(!log.contains("fragment output"), "{log}");
}

#[test]
fn version_300_es_raises_no_location_line() {
    let _g = context();
    let vs = "#version 300 es\nin vec4 p;\nvoid main() { gl_Position = p; }\n";
    // two outputs at one location, and one far past draw_targets()
    for fs in [
        "#version 300 es\nprecision mediump float;\nlayout(location = 0) out vec4 a;\nlayout(location = 0) out vec4 b;\nvoid main() { a = vec4(1.0); b = vec4(1.0); }\n",
        "#version 300 es\nprecision mediump float;\nlayout(location = 40) out vec4 a;\nvoid main() { a = vec4(1.0); }\n",
    ] {
        let (_, _, log, _) = build(vs, fs);
        assert!(!log.contains("fragment output"), "the ES 3.0 check is not run in an ES 2.0 context: {log}");
    }
}
