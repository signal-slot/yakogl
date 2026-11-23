// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GLSL ES 3.00 fragment outputs at four draw buffers, through the GL API
//! (GLSL ES 3.00 4.3.8.2, ES 3.0 3.9.2): the stand-in back end claims four
//! targets through `backend::HOST_DRAW_TARGETS`, as a back end that draws
//! four will through `backend::DRAW_TARGETS`. Held: which output layouts
//! link, which fail and with what info log, and that with the stand-in at 0
//! the build's own count (3) rejects location 3.
//! (`gl_MaxDrawBuffers` itself is pinned by es3_mrt_gl.rs.)
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_mrt4_glsl
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test es3_mrt4_glsl
//! ```

#![cfg(all(feature = "host-test", feature = "shaderc-online", not(feature = "report-es2")))]

use core::ffi::{c_char, c_void};
use std::ffi::CString;
use std::sync::atomic::Ordering;
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{self, GlesDraw, GlesHeap, YakoglBackend};
use yakogl::gl::consts::*;
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
    name: c"es3-mrt4-glsl".as_ptr() as *const c_char,
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

/// The context, with the stand-in back end claiming `targets` colour
/// targets a draw (0: the build's own count, `backend::DRAW_TARGETS`).
fn context_with(targets: u8) -> MutexGuard<'static, ()> {
    let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    backend::HOST_DRAW_TARGETS.store(targets, Ordering::Relaxed);
    ONCE.call_once(|| {
        // SAFETY: both tables are statics; the lock above serialises the tests.
        unsafe {
            osal_set_ops(&OSAL);
            backend::g_backend = &BACKEND;
            yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
            assert!(!yakogl::gl::context::gl_context_create(8, 8).is_null());
        }
    });
    // SAFETY: plain GL call.
    while unsafe { glGetError() } != GL_NO_ERROR {}
    g
}

/// Restores the stand-in to 0 when a test ends, pass or fail.
struct Restore;
impl Drop for Restore {
    fn drop(&mut self) {
        backend::HOST_DRAW_TARGETS.store(0, Ordering::Relaxed);
    }
}

const VS: &str = "#version 300 es\nin vec4 p;\nvoid main() { gl_Position = p; }\n";

/// What compiling and linking `VS` + `fs` came to.
#[derive(Debug)]
struct Outcome {
    compiled: bool,
    linked: bool,
    log: String,
    error: GLenum,
}

fn log_of(get: impl FnOnce(*mut GLsizei, *mut c_char)) -> String {
    let mut buf = [0 as c_char; 1024];
    let mut len: GLsizei = 0;
    get(&mut len, buf.as_mut_ptr());
    buf[..len.clamp(0, 1023) as usize].iter().map(|&c| c as u8 as char).collect()
}

fn build(fs: &str) -> Outcome {
    let text = CString::new(fs).unwrap();
    let vs = CString::new(VS).unwrap();
    // SAFETY: plain GL calls; NUL-terminated sources and sized log buffers.
    unsafe {
        let mut compiled = true;
        let mut log = String::new();
        let p = glCreateProgram();
        for (ty, src) in [(GL_VERTEX_SHADER, &vs), (GL_FRAGMENT_SHADER, &text)] {
            let s = glCreateShader(ty);
            let ptr: [*const c_char; 1] = [src.as_ptr()];
            glShaderSource(s, 1, ptr.as_ptr(), core::ptr::null());
            glCompileShader(s);
            let mut ok: GLint = 0;
            glGetShaderiv(s, GL_COMPILE_STATUS, &mut ok);
            if ok != 1 {
                compiled = false;
                log += &log_of(|l, b| glGetShaderInfoLog(s, 1024, l, b));
            }
            glAttachShader(p, s);
        }
        let mut linked = false;
        if compiled {
            glLinkProgram(p);
            let mut ok: GLint = 0;
            glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
            linked = ok == 1;
            if !linked {
                log += &log_of(|l, b| glGetProgramInfoLog(p, 1024, l, b));
            }
        }
        Outcome { compiled, linked, log, error: glGetError() }
    }
}

fn fs(outs: &str, body: &str) -> String {
    format!("#version 300 es\nprecision mediump float;\n{outs}\nvoid main() {{ {body} }}\n")
}

/// The shader links, with no GL error.
fn must_link(outs: &str, body: &str) {
    let _g = context_with(4);
    let _r = Restore;
    let o = build(&fs(outs, body));
    assert!(o.compiled && o.linked && o.error == GL_NO_ERROR, "{outs:?}: {o:?}");
}

/// The shader fails to compile or link, with an info log and no GL error.
fn must_fail(outs: &str, body: &str) {
    let _g = context_with(4);
    let _r = Restore;
    let o = build(&fs(outs, body));
    assert!(!(o.compiled && o.linked), "{outs:?} was accepted");
    assert!(!o.log.is_empty(), "{outs:?} failed with no info log");
    assert_eq!(o.error, GL_NO_ERROR, "{outs:?}");
}

#[test]
#[cfg_attr(
    feature = "gpu-mali",
    ignore = "Mali refuses a fragment shader that leaves gl_FragColor unwritten (not lowered yet)"
)]
fn location_3_links() {
    must_link("layout(location = 3) out vec4 o;", "o = vec4(1.0);");
}

#[test]
fn array_of_four_without_layout_links() {
    must_link("out vec4 o[4];", "o[0] = vec4(0.0); o[1] = vec4(1.0); o[2] = vec4(2.0); o[3] = vec4(3.0);");
}

#[test]
fn array_of_four_at_location_0_links() {
    must_link("layout(location = 0) out vec4 o[4];", "for (int i = 0; i < 4; i++) { o[i] = vec4(float(i)); }");
}

#[test]
#[cfg_attr(
    feature = "gpu-mali",
    ignore = "Mali refuses a fragment shader that leaves gl_FragColor unwritten (not lowered yet)"
)]
fn outputs_at_1_and_3_without_0_link() {
    must_link("layout(location = 1) out vec4 a;\nlayout(location = 3) out vec4 b;", "a = vec4(1.0); b = vec4(2.0);");
}

#[test]
fn indexed_by_gl_max_draw_buffers_links() {
    must_link("out vec4 o[4];", "o[gl_MaxDrawBuffers - 1] = vec4(1.0); o[0] = vec4(0.0); o[1] = o[0]; o[2] = o[0];");
}

#[test]
fn location_4_fails() {
    must_fail("layout(location = 4) out vec4 o;", "o = vec4(1.0);");
}

#[test]
fn array_past_location_3_fails() {
    must_fail("layout(location = 2) out vec4 o[3];", "o[0] = vec4(1.0); o[1] = o[0]; o[2] = o[0];");
}

#[test]
fn two_outputs_on_one_location_fail() {
    must_fail("layout(location = 1) out vec4 a;\nlayout(location = 1) out vec4 b;", "a = vec4(1.0); b = vec4(2.0);");
}

/// The stand-in at 0 is the build's own count, 3: location 3 fails there.
#[test]
fn build_count_rejects_location_3() {
    let _g = context_with(0);
    let _r = Restore;
    let o = build(&fs("layout(location = 3) out vec4 o;", "o = vec4(1.0);"));
    assert!(!(o.compiled && o.linked), "location 3 accepted at the build's count: {o:?}");
    assert!(!o.log.is_empty(), "no info log");
    assert_eq!(o.error, GL_NO_ERROR);
}
