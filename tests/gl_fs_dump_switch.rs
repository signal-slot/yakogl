// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The fragment uniform-block dump (`dump_small_fs_block`, a bring-up aid for
//! a uniform-bounded loop) is off by default: a draw with a small FS block of
//! a large fragment shader logs no `fs uniform[` line. On a 115200-baud
//! console those ~5 KB were the likely cause of the Pi 4 terrain scene's
//! 7.8 ms a frame in uniforms (docs/notes/pi4-terrain-uniforms.md).
//! `yakogl_fs_dump_budget`, which bring-up writes, turns it on.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test gl_fs_dump_switch
//! ```

#![cfg(all(feature = "host-test", feature = "shaderc-online"))]

use core::ffi::{c_char, c_void};
use std::ffi::CString;
use std::sync::atomic::Ordering;
use std::sync::{Mutex, MutexGuard, Once, OnceLock};
use yakogl::backend::{GlesDraw, GlesHeap, YakoglBackend};
use yakogl::gl::buffer::*;
use yakogl::gl::consts::*;
use yakogl::gl::draw::*;
use yakogl::gl::program::*;
use yakogl::gl::shader::*;
use yakogl::gl::state::*;
use yakogl::gl::types::*;
use yakogl::gl::vertex::*;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};

/* ---- the model: what the back end got, what the library logged ------------- */

/// `fs_uniform_dwords` of each draw that reached the back end.
static DRAWS: Mutex<Vec<u16>> = Mutex::new(Vec::new());
static LOG: Mutex<Vec<String>> = Mutex::new(Vec::new());

unsafe extern "C" fn m_gpu_alloc(_heap: u64, size: usize, _flags: u32, out: *mut OsalGpuMem) -> i32 {
    let mem = Box::leak(vec![0u8; size].into_boxed_slice());
    let p = mem.as_mut_ptr();
    *out = OsalGpuMem { cpu: p as *mut c_void, gpu: p as u64, phys: p as u64, size, impl_: p as *mut c_void };
    0
}
unsafe extern "C" fn m_gpu_free(_mem: *mut OsalGpuMem) -> i32 {
    0
}
unsafe extern "C" fn m_flush(_cpu: *mut c_void, _n: usize) {}
unsafe extern "C" fn m_invd(_cpu: *mut c_void, _n: usize) {}
/// The library logs every line as `log("%s", line)` (`osal::log_str`).
unsafe extern "C" fn m_log(_fmt: *const c_char, line: *const c_char) {
    let s = std::ffi::CStr::from_ptr(line).to_string_lossy().into_owned();
    LOG.lock().unwrap_or_else(|e| e.into_inner()).push(s);
}
type LogFn = unsafe extern "C" fn(fmt: *const c_char, ...);
unsafe extern "C" fn b_init() -> i32 {
    0
}
unsafe extern "C" fn b_heap_base(_heap: GlesHeap) -> u64 {
    0x1000_0000
}
unsafe extern "C" fn b_draw(d: *const GlesDraw) -> i32 {
    DRAWS.lock().unwrap_or_else(|e| e.into_inner()).push((*d).fs_uniform_dwords);
    0
}
unsafe extern "C" fn b_flush() -> i32 {
    0
}
unsafe extern "C" fn b_finish() -> i32 {
    0
}
unsafe extern "C" fn b_kick_id() -> u32 {
    1
}
unsafe extern "C" fn b_kick_pending(_id: u32) -> bool {
    false
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
    cache_flush: Some(m_flush),
    cache_invd: Some(m_invd),
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
    name: c"gl-fs-dump-switch".as_ptr() as *const c_char,
    native_attrib_formats: 0,
    rt_rgba8888_is_bgra: false,
    mip_level_align: 0,
    linear_row_align: 0,
    uniforms_host_only: false,
    ds_write: None,
    stencil_resolve: None,
    compressed_blocks_in_rows: false,
    native_layered_textures: false,
    front_facing_shader_variant: false,
    etc1_decoded_at_upload: false,
    init: Some(b_init),
    shutdown: None,
    draw: Some(b_draw),
    flush: Some(b_flush),
    finish: Some(b_finish),
    finish_previous: None,
    kick_id: Some(b_kick_id),
    kick_pending: Some(b_kick_pending),
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
};

static LOCK: Mutex<()> = Mutex::new(());
static ONCE: Once = Once::new();
static PROGRAM: OnceLock<GLuint> = OnceLock::new();

const VS: &str = "attribute vec3 a;\nvoid main() { gl_Position = vec4(a, 1.0); }\n";

/// A fragment shader of well over 900 USC dwords whose uniform block is the
/// one vec4 (4 dwords, under the dump's 8).
fn fs_text() -> String {
    let mut s = String::from("precision mediump float;\nuniform vec4 u;\nvoid main() {\n  vec4 c = u;\n");
    for i in 0..120 {
        s += &format!("  c = c * 0.93 + sin(c.yzwx * {}.0) * u.xyzw;\n", i + 1);
    }
    s + "  gl_FragColor = c;\n}\n"
}

fn program() -> GLuint {
    let shader = |kind: GLenum, text: &str| -> GLuint {
        let text = CString::new(text).unwrap();
        // SAFETY: plain GL calls; one NUL-terminated source.
        unsafe {
            let s = glCreateShader(kind);
            let p: [*const c_char; 1] = [text.as_ptr()];
            glShaderSource(s, 1, p.as_ptr(), core::ptr::null());
            glCompileShader(s);
            let mut ok: GLint = 0;
            glGetShaderiv(s, GL_COMPILE_STATUS, &mut ok);
            assert_eq!(ok, 1, "{text:?} did not compile");
            s
        }
    };
    // SAFETY: plain GL calls.
    unsafe {
        let p = glCreateProgram();
        glAttachShader(p, shader(GL_VERTEX_SHADER, VS));
        glAttachShader(p, shader(GL_FRAGMENT_SHADER, &fs_text()));
        glBindAttribLocation(p, 0, c"a".as_ptr());
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        assert_eq!(ok, 1, "the program did not link");
        p
    }
}

/// The one context and its program; the dump budget is back to its default.
fn context() -> MutexGuard<'static, ()> {
    let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    ONCE.call_once(|| {
        // SAFETY: the tables are statics, the log hook is only ever called
        // with ("%s", line); the lock above serialises the tests.
        unsafe {
            let log = core::mem::transmute::<unsafe extern "C" fn(*const c_char, *const c_char), LogFn>(m_log);
            let mut ops = core::ptr::read(&OSAL);
            ops.log = Some(log);
            osal_set_ops(Box::leak(Box::new(ops)));
            yakogl::backend::g_backend = &BACKEND;
            yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
            assert!(!yakogl::gl::context::gl_context_create(8, 8).is_null());
        }
    });
    let prog = *PROGRAM.get_or_init(program);
    // SAFETY: plain GL calls on the one context.
    unsafe {
        glFinish();
        while glGetError() != GL_NO_ERROR {}
        glUseProgram(prog);
    }
    yakogl_fs_dump_budget.store(0, Ordering::Relaxed);
    DRAWS.lock().unwrap_or_else(|e| e.into_inner()).clear();
    LOG.lock().unwrap_or_else(|e| e.into_inner()).clear();
    g
}

/* ---- the cases -------------------------------------------------------------- */

/// Draw once; returns the `fs uniform[` lines logged.
fn draw_lines() -> Vec<String> {
    let tri: [f32; 9] = [0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0];
    // SAFETY: plain GL calls; the client array outlives the draw.
    unsafe {
        let loc = glGetUniformLocation(*PROGRAM.get().unwrap(), c"u".as_ptr());
        assert!(loc >= 0, "uniform u is live");
        glUniform4f(loc, 1.5, 2.5, 3.5, 4.5);
        glBindBuffer(GL_ARRAY_BUFFER, 0);
        glVertexAttribPointer(0, 3, GL_FLOAT, GL_FALSE_B, 0, tri.as_ptr() as *const c_void);
        glEnableVertexAttribArray(0);
        glDrawArrays(GL_TRIANGLES, 0, 3);
        assert_eq!(glGetError(), GL_NO_ERROR);
    }
    let draws = DRAWS.lock().unwrap_or_else(|e| e.into_inner());
    assert_eq!(draws.len(), 1, "one draw reached the back end");
    assert!((1..=8).contains(&draws[0]), "a small FS block: {} dwords", draws[0]);
    LOG.lock().unwrap_or_else(|e| e.into_inner()).iter().filter(|l| l.contains("fs uniform[")).cloned().collect()
}

#[test]
fn off_by_default_no_line_is_logged() {
    let _g = context();
    assert_eq!(yakogl_fs_dump_budget.load(Ordering::Relaxed), 0, "the default is off");
    let lines = draw_lines();
    assert!(lines.is_empty(), "{lines:?}");
}

#[test]
fn on_the_block_is_dumped_and_the_budget_counts_down() {
    let _g = context();
    yakogl_fs_dump_budget.store(1, Ordering::Relaxed);
    let lines = draw_lines();
    assert!(!lines.is_empty(), "the lines appear with the switch on");
    assert!(lines[0].contains("float 1.5"), "{lines:?}");
    assert_eq!(yakogl_fs_dump_budget.load(Ordering::Relaxed), 0, "one block taken off the budget");
    DRAWS.lock().unwrap_or_else(|e| e.into_inner()).clear();
    LOG.lock().unwrap_or_else(|e| e.into_inner()).clear();
    assert!(draw_lines().is_empty(), "an exhausted budget logs nothing more");
}
