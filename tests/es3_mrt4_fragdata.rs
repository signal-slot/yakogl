// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! ESSL 1.00 (`#version 100`) `gl_FragData[0..3]` and `gl_MaxDrawBuffers` in
//! an ES 3.0 context at four draw buffers, through the GL API: the stand-in
//! back end claims four targets through `backend::HOST_DRAW_TARGETS`. Held:
//! what the core answers today for `gl_FragData[3]` with and without
//! `#extension GL_EXT_draw_buffers : require`, `gl_MaxDrawBuffers == 4`, the
//! four targets a draw hands the back end, and the build's count (3) at a
//! stand-in of 0. GLSL ES 3.00 outputs are es3_mrt4_glsl.rs.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_mrt4_fragdata
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test es3_mrt4_fragdata
//! ```

#![cfg(all(feature = "host-test", feature = "shaderc-online", not(feature = "report-es2")))]

use core::ffi::{c_char, c_void};
use std::ffi::CString;
use std::sync::atomic::Ordering;
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{self, GlesDraw, GlesHeap, YakoglBackend};
use yakogl::gl::buffer::glBindBuffer;
use yakogl::gl::consts::*;
use yakogl::gl::draw::*;
use yakogl::gl::framebuffer::*;
use yakogl::gl::program::*;
use yakogl::gl::shader::*;
use yakogl::gl::state::*;
use yakogl::gl::types::*;
use yakogl::gl::vertex::{glEnableVertexAttribArray, glVertexAttribPointer};
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

/// `num_targets` of each application draw (vertices) the back end was handed.
static DRAWS: Mutex<Vec<usize>> = Mutex::new(Vec::new());

unsafe extern "C" fn b_draw(d: *const GlesDraw) -> i32 {
    let d = &*d;
    if d.index_count != 0 {
        DRAWS.lock().unwrap().push(d.num_targets as usize);
    }
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
    name: c"es3-mrt4-fragdata".as_ptr() as *const c_char,
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

const VS: &str = "#version 100\nattribute vec4 p;\nvoid main() { gl_Position = p; }\n";

/// What compiling and linking `VS` + `fs` came to, and the program.
#[derive(Debug)]
struct Outcome {
    compiled: bool,
    linked: bool,
    log: String,
    error: GLenum,
    program: GLuint,
}

fn log_of(get: impl FnOnce(*mut GLsizei, *mut c_char)) -> String {
    let mut buf = [0 as c_char; 1024];
    let mut len: GLsizei = 0;
    get(&mut len, buf.as_mut_ptr());
    buf[..len.clamp(0, 1023) as usize].iter().map(|&c| c as u8 as char).collect()
}

fn build(fs: &str) -> Outcome {
    build_with(VS, fs)
}

fn build_with(vs: &str, fs: &str) -> Outcome {
    let text = CString::new(fs).unwrap();
    let vs = CString::new(vs).unwrap();
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
        Outcome { compiled, linked, log, error: glGetError(), program: p }
    }
}

const EXT: &str = "#extension GL_EXT_draw_buffers : require\n";

fn fs100(ext: &str, body: &str) -> String {
    format!("#version 100\n{ext}precision mediump float;\nvoid main() {{ {body} }}\n")
}

const FOUR: &str = "gl_FragData[0] = vec4(0.0); gl_FragData[1] = vec4(1.0); \
                    gl_FragData[2] = vec4(0.5); gl_FragData[3] = vec4(0.25);";

#[test]
fn max_draw_buffers_is_four_in_a_version_100_shader() {
    let _g = context_with(4);
    let _r = Restore;
    // `a` compiles only for >= 4, `b` only for <= 4.
    let body = "float a[gl_MaxDrawBuffers - 3]; float b[5 - gl_MaxDrawBuffers]; a[0] = 1.0; b[0] = 1.0; \
                gl_FragColor = vec4(a[0] + b[0]);";
    let o = build(&fs100("", body));
    assert!(o.compiled && o.linked && o.error == GL_NO_ERROR, "{o:?}");
}

#[test]
#[cfg_attr(
    feature = "gpu-mali",
    ignore = "Mali: it draws four targets (mali::backend::DRAW_TARGETS); this test needs a three-target back end"
)]
fn max_draw_buffers_is_three_at_the_build_count() {
    let _g = context_with(0);
    let _r = Restore;
    let body = "float a[4 - gl_MaxDrawBuffers]; a[0] = 1.0; gl_FragColor = vec4(a[0]);";
    let o = build(&fs100("", body));
    assert!(o.compiled && o.linked && o.error == GL_NO_ERROR, "{o:?}");
}

/// Today's answer: ESSL 1.00 has one draw buffer in the core
/// (src/shaderc/online/lower.rs:2976 rejects a constant index but 0, as
/// src/gl/state.rs:1322 explains), even in an ES 3.0 context at four targets.
#[test]
fn frag_data_3_without_the_extension_is_rejected_today() {
    let _g = context_with(4);
    let _r = Restore;
    let o = build(&fs100("", FOUR));
    assert_eq!(o.error, GL_NO_ERROR);
    assert!(!o.compiled && !o.linked, "gl_FragData[3] accepted: {o:?}");
    assert!(o.log.contains("only gl_FragData[0] exists"), "{o:?}");
}

/// Today's answer: GL_EXT_draw_buffers is not advertised, so `require` fails
/// in the preprocessor (src/shaderc/online/pp.rs:575).
#[test]
fn frag_data_3_with_the_extension_is_rejected_today() {
    let _g = context_with(4);
    let _r = Restore;
    let o = build(&fs100(EXT, FOUR));
    assert_eq!(o.error, GL_NO_ERROR);
    assert!(!o.compiled && !o.linked, "{o:?}");
    assert!(o.log.contains("extension is not supported"), "{o:?}");
}

/// The wanted answer once GL_EXT_draw_buffers is claimed at four targets.
#[test]
#[ignore = "GL_EXT_draw_buffers not claimed: src/shaderc/online/pp.rs:575, lower.rs:2976"]
fn frag_data_3_with_the_extension_links() {
    let _g = context_with(4);
    let _r = Restore;
    let o = build(&fs100(EXT, FOUR));
    assert!(o.compiled && o.linked && o.error == GL_NO_ERROR, "{o:?}");
}

#[test]
fn frag_data_3_fails_at_the_build_count() {
    for ext in ["", EXT] {
        let _g = context_with(0);
        let _r = Restore;
        let o = build(&fs100(ext, FOUR));
        assert!(!(o.compiled && o.linked), "index 3 accepted at the build's count ({ext:?}): {o:?}");
        assert!(!o.log.is_empty(), "no info log");
        assert_eq!(o.error, GL_NO_ERROR);
    }
}

#[test]
fn frag_data_0_links_at_four_targets() {
    let _g = context_with(4);
    let _r = Restore;
    let o = build(&fs100("", "gl_FragData[0] = vec4(1.0);"));
    assert!(o.compiled && o.linked && o.error == GL_NO_ERROR, "{o:?}");
}

/// ESSL 1.00 cannot name four outputs today, so the four-output draw uses a
/// `#version 300 es` program; the back end is handed four targets.
#[test]
fn four_output_draw_hands_four_targets_to_the_backend() {
    static TRI: [f32; 12] = [-1.0, -1.0, 0.0, 1.0, 3.0, -1.0, 0.0, 1.0, -1.0, 3.0, 0.0, 1.0];
    let _g = context_with(4);
    let _r = Restore;
    let vs = "#version 300 es\nin vec4 p;\nvoid main() { gl_Position = p; }\n";
    let fs = "#version 300 es\nprecision mediump float;\nlayout(location = 0) out vec4 a;\n\
              layout(location = 1) out vec4 b;\nlayout(location = 2) out vec4 c;\n\
              layout(location = 3) out vec4 d;\nvoid main() { a = vec4(0.0); b = vec4(1.0); \
              c = vec4(0.5); d = vec4(0.25); }\n";
    let o = build_with(vs, fs);
    assert!(o.compiled && o.linked, "{o:?}");
    // SAFETY: plain GL calls; TRI is static; one name each.
    unsafe {
        let (mut fb, mut rbs) = (0, [0u32; 4]);
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        glGenRenderbuffers(4, rbs.as_mut_ptr());
        let list: Vec<GLenum> = (0..4).map(|i| GL_COLOR_ATTACHMENT0 + i).collect();
        for (i, rb) in rbs.iter().enumerate() {
            glBindRenderbuffer(GL_RENDERBUFFER, *rb);
            glRenderbufferStorage(GL_RENDERBUFFER, GL_RGBA8, 8, 8);
            glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0 + i as GLenum, GL_RENDERBUFFER, *rb);
        }
        glDrawBuffers(4, list.as_ptr());
        assert_eq!(glGetError(), GL_NO_ERROR);
        glUseProgram(o.program);
        glBindBuffer(GL_ARRAY_BUFFER, 0);
        let loc = glGetAttribLocation(o.program, c"p".as_ptr());
        assert!(loc >= 0);
        glVertexAttribPointer(loc as GLuint, 4, GL_FLOAT, 0, 0, TRI.as_ptr() as *const c_void);
        glEnableVertexAttribArray(loc as GLuint);
        DRAWS.lock().unwrap().clear();
        glDrawArrays(GL_TRIANGLES, 0, 3);
        glFinish();
        let e = glGetError();
        let draws = DRAWS.lock().unwrap().clone();
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glUseProgram(0);
        assert_eq!(e, GL_NO_ERROR);
        assert_eq!(draws.last().copied(), Some(4), "num_targets of the draws: {draws:?}");
    }
}
