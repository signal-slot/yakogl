// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The GL behaviour around a shader the Mali online compiler refuses, as
//! glmark2 meets it. The NanoPC-T6 record (main `a57d8104`, tools/cts/
//! RESULTS.md, "Mali: NanoPC-T6 glmark2-es2") has 80 GL error lines: after
//! a refused `glCompileShader` glmark2 goes on with the program.
//!
//! glmark2's call order (libmatrix/program.cc, `GLMARK2_SRC` fetched by
//! tools/hostharness/glmark2/run.sh, never committed):
//!   `Shader::compile`: glCompileShader, glGetShaderiv(COMPILE_STATUS), on
//!     failure glGetShaderiv(INFO_LOG_LENGTH) and glGetShaderInfoLog; the
//!     shader is then not `ready_`, so `Shader::attach` skips it;
//!   `Program::build`: with no shader attached it returns without
//!     glLinkProgram (the program stays never linked);
//!   `Program::start`: not ready, so no glUseProgram at all (the previous
//!     program, or none, stays current);
//!   the scene then calls `getUniformLocation` / `getAttribIndex`
//!     (glGetUniformLocation / glGetAttribLocation on the unlinked program),
//!     `UniformSetter`s (glUniform*(-1, ...)) and draws.
//!
//! What each case explains of the record's lines:
//!   - `refused_shader_compile_status_and_log`: the compile refusal itself
//!     (31 scenes, "Set up failed" in 27); glmark2 prints the info log.
//!   - `unlinked_program_locations_are_errors`: the `0x501` lines in
//!     `glGetAttribLocation` / `glGetUniformLocation` (desktop blur and the
//!     three buffer scenes: 16 lines each; terrain: 16 before its abort,
//!     where the -1 attribute index is then used to index a vector).
//!   - `failed_link_use_program_is_invalid_operation`: the `glUseProgram`
//!     glmark2 does not reach when `Program::start` is skipped, and the
//!     same program with a refused shader attached and linked.
//!   - `uniform_to_minus_one_is_silent`: the `glUniform*(-1, ...)` of every
//!     scene after the location query: ignored with a program in use, but
//!     INVALID_OPERATION with none current (the `0x502` in `store_uniform`
//!     of the scenes whose program was refused).
//!   - `draw_without_program_draws_nothing`: the draw of the scenes that
//!     ran on (blur, buffer): nothing reaches the back end and the error is
//!     0x502, as in the record's `0x502` at the draw (ES 2.0 leaves it
//!     undefined; the test accepts no error too). The attribute index -1
//!     of `negative_attribute_index_is_invalid_value` is INVALID_VALUE.
//!   - `good_program_draws_after_refusal`: the context stays usable.
//!
//! The real Mali back end runs over a mock OSAL; no GPU runs.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_glmark2_refused_program
//! ```

#![cfg(all(feature = "host-test", feature = "shaderc-online", feature = "gpu-mali"))]

use core::ffi::{c_char, c_void};
use std::ffi::CString;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{self, GlesDraw, YakoglBackend};
use yakogl::gl::buffer::glBindBuffer;
use yakogl::gl::consts::*;
use yakogl::gl::draw::*;
use yakogl::gl::program::*;
use yakogl::gl::shader::*;
use yakogl::gl::state::*;
use yakogl::gl::types::*;
use yakogl::gl::vertex::{glEnableVertexAttribArray, glVertexAttribPointer};
use yakogl::osal::mali::{OsalMaliGpu, OsalMaliHeap, OSAL_QUERY_MALI_GPU, OSAL_QUERY_MALI_TILER_HEAP};
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};
use yakogl::util::ENOTSUP;

/* ---- the Mali over a mock OSAL ------------------------------------------ */

const GPU: OsalMaliGpu = OsalMaliGpu::T6;
const HEAP: OsalMaliHeap =
    OsalMaliHeap { ctx_gpu: 0x8000_0001_5000, first_chunk_gpu: 0x8000_00a0_0000, chunk_size: 2 << 20, reserved: 0 };

static FENCES: AtomicI64 = AtomicI64::new(0);

/// Buffers made so far: (GPU VA, CPU address, bytes). Each VA is its number
/// times 16 MiB, far from any CPU address.
static BUFFERS: Mutex<Vec<(u64, usize, usize)>> = Mutex::new(Vec::new());

unsafe extern "C" fn m_alloc(_heap: u64, size: usize, _flags: u32, out: *mut OsalGpuMem) -> i32 {
    assert!(size <= 16 << 20);
    let cpu = Box::leak(vec![0u64; size.div_ceil(8)].into_boxed_slice()).as_mut_ptr().cast::<c_void>();
    let mut b = BUFFERS.lock().unwrap();
    let gpu = 0x40_0000_0000 + (b.len() as u64 + 1) * (16 << 20);
    b.push((gpu, cpu as usize, size));
    *out = OsalGpuMem { cpu, gpu, phys: 0, size, impl_: cpu };
    0
}
unsafe extern "C" fn m_free(mem: *mut OsalGpuMem) -> i32 {
    *mem = OsalGpuMem::ZERO;
    0
}
unsafe extern "C" fn m_cache(_cpu: *mut c_void, _n: usize) {}
unsafe extern "C" fn m_init(_arg: *mut c_void) -> i32 {
    0
}

/// The CPU address of `n` bytes at GPU VA `gpu`, inside one buffer.
#[allow(dead_code)]
fn cpu_of(gpu: u64, n: usize) -> Option<usize> {
    let b = BUFFERS.lock().unwrap();
    let &(g, cpu, size) = b.iter().find(|&&(g, _, s)| (g..g + s as u64).contains(&gpu))?;
    let off = (gpu - g) as usize;
    (off + n <= size).then_some(cpu + off)
}

unsafe extern "C" fn m_submit(kick: *const c_void, len: usize) -> i64 {
    let _ = (kick, len);
    FENCES.fetch_add(1, Ordering::Relaxed) + 1
}
unsafe extern "C" fn m_wait(_fence: i64, _timeout_us: i64) -> i32 {
    0
}
unsafe extern "C" fn m_completed() -> i64 {
    FENCES.load(Ordering::Relaxed)
}
unsafe extern "C" fn m_query(what: u32, out: *mut c_void, sz: usize) -> i32 {
    match what {
        OSAL_QUERY_MALI_GPU if sz == core::mem::size_of::<OsalMaliGpu>() => *out.cast::<OsalMaliGpu>() = GPU,
        OSAL_QUERY_MALI_TILER_HEAP if sz == core::mem::size_of::<OsalMaliHeap>() => *out.cast::<OsalMaliHeap>() = HEAP,
        _ => return -ENOTSUP,
    }
    0
}

/// The OSAL's log hook: the library only calls it as `log("%s", line)`.
unsafe extern "C" fn m_log(_fmt: *const c_char, line: *const c_char) {
    eprintln!("osal log: {}", core::ffi::CStr::from_ptr(line).to_string_lossy());
}

static OSAL: OsalOps = OsalOps {
    init: Some(m_init),
    shutdown: None,
    gpu_alloc: Some(m_alloc),
    gpu_free: Some(m_free),
    mmu_map: None,
    map_phys: None,
    gpu_write: None,
    gpu_read: None,
    cache_flush: Some(m_cache),
    cache_invd: Some(m_cache),
    submit: Some(m_submit),
    wait: Some(m_wait),
    completed: Some(m_completed),
    reg_read: None,
    reg_write: None,
    query: Some(m_query),
    // SAFETY: see `m_log`: the same calling convention for these arguments.
    log: Some(unsafe {
        core::mem::transmute::<
            unsafe extern "C" fn(*const c_char, *const c_char),
            unsafe extern "C" fn(*const c_char, ...),
        >(m_log)
    }),
    wait_vblank: None,
    present: None,
    lock: None,
    unlock: None,
    thread_id: None,
    display_busy: None,
};

/// What the back end was given for one draw.
#[allow(dead_code)]
#[derive(Clone, Copy, Debug)]
struct Seen {
    index_count: u32,
    target_gpu: u64,
    num_textures: u8,
    tex_format: u8,
    tex_incomplete: u8,
    tex_base_gpu: u64,
    tex_w: u32,
    tex_h: u32,
    tex_pitch: u32,
    mali: i32,
}

static SEEN: Mutex<Vec<Seen>> = Mutex::new(Vec::new());

unsafe extern "C" fn b_draw(d: *const GlesDraw) -> i32 {
    let dr = &*d;
    let mali = yakogl::mali::backend::mali_draw(d);
    let t = &dr.textures[0];
    SEEN.lock().unwrap().push(Seen {
        index_count: dr.index_count,
        target_gpu: dr.target_gpu,
        num_textures: dr.num_textures,
        tex_format: t.format,
        tex_incomplete: t.incomplete,
        tex_base_gpu: t.base_gpu,
        tex_w: t.width,
        tex_h: t.height,
        tex_pitch: 4 * t.stride_texels,
        mali,
    });
    mali
}

static LOCK: Mutex<()> = Mutex::new(());
static ONCE: Once = Once::new();

const N: GLsizei = 32;

fn context() -> MutexGuard<'static, ()> {
    let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    ONCE.call_once(|| {
        backend::HOST_DRAW_TARGETS.store(1, Ordering::Relaxed);
        // SAFETY: the Mali table copied bit for bit with only `draw`
        // replaced; the OSAL and the copy are leaked; the lock serialises
        // the tests.
        unsafe {
            osal_set_ops(&OSAL);
            let mut b: YakoglBackend = core::ptr::read(&yakogl::mali::backend::yakogl_backend_mali);
            b.draw = Some(b_draw);
            backend::g_backend = Box::leak(Box::new(b));
            assert_eq!(yakogl::mali::backend::mali_init(), 0);
            yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
            assert!(!yakogl::gl::context::gl_context_create(N as u32, N as u32).is_null());
            glViewport(0, 0, N, N);
        }
    });
    SEEN.lock().unwrap().clear();
    // SAFETY: plain GL call.
    while unsafe { glGetError() } != GL_NO_ERROR {}
    g
}

fn ok(what: &str) {
    // SAFETY: plain GL call.
    let e = unsafe { glGetError() };
    assert_eq!(e, GL_NO_ERROR, "{what}: glGetError 0x{e:x}");
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
        let mut log = [0u8; 512];
        let mut n: GLsizei = 0;
        glGetShaderInfoLog(s, log.len() as GLsizei, &mut n, log.as_mut_ptr().cast());
        assert_eq!(ok, 1, "{text:?} did not compile: {}", String::from_utf8_lossy(&log[..n as usize]));
        s
    }
}

fn program(vs: &str, fs: &str) -> GLuint {
    let v = shader(GL_VERTEX_SHADER, vs);
    let f = shader(GL_FRAGMENT_SHADER, fs);
    // SAFETY: plain GL calls.
    unsafe {
        let p = glCreateProgram();
        glAttachShader(p, v);
        glAttachShader(p, f);
        glBindAttribLocation(p, 0, c"p".as_ptr());
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        assert_eq!(ok, 1, "link");
        p
    }
}

const VS1: &str = "attribute vec4 p;\nvoid main() { gl_Position = p; }\n";
const FS1: &str = "precision mediump float;\nvoid main() { gl_FragColor = vec4(1.0); }\n";

/// Compile `ty` text; returns (shader, COMPILE_STATUS, info log).
fn compile(ty: GLenum, text: &str) -> (GLuint, GLint, String) {
    let text = CString::new(text).unwrap();
    // SAFETY: plain GL calls; one NUL-terminated source; glmark2's
    // `Shader::compile` order (status, then log length, then the log).
    unsafe {
        let s = glCreateShader(ty);
        let p: [*const c_char; 1] = [text.as_ptr()];
        glShaderSource(s, 1, p.as_ptr(), core::ptr::null());
        glCompileShader(s);
        let mut st: GLint = -1;
        glGetShaderiv(s, GL_COMPILE_STATUS, &mut st);
        let mut len: GLint = 0;
        glGetShaderiv(s, GL_INFO_LOG_LENGTH, &mut len);
        let mut log = vec![0u8; len as usize + 1];
        let mut n: GLsizei = 0;
        glGetShaderInfoLog(s, len + 1, &mut n, log.as_mut_ptr().cast());
        (s, st, String::from_utf8_lossy(&log[..n as usize]).into_owned())
    }
}

/// A fragment shader every compiler refuses (a syntax error).
const BAD_FS: &str = "precision mediump float;\nvoid main() { gl_FragColor = vec4(1.0) }\n";

fn err() -> GLenum {
    // SAFETY: plain GL call.
    unsafe { glGetError() }
}

/// A program of `vs` and the refused `BAD_FS`, with glmark2's rule: a shader
/// that did not compile is not attached. `link` also calls glLinkProgram.
fn refused_program(link: bool, attach_refused: bool) -> GLuint {
    let (v, st, _) = compile(GL_VERTEX_SHADER, VS1);
    assert_eq!(st, 1);
    let (f, st, _) = compile(GL_FRAGMENT_SHADER, BAD_FS);
    assert_eq!(st, 0);
    // SAFETY: plain GL calls.
    unsafe {
        let p = glCreateProgram();
        glAttachShader(p, v);
        if attach_refused {
            glAttachShader(p, f);
        }
        if link {
            glLinkProgram(p);
        }
        p
    }
}

fn link_status(p: GLuint) -> GLint {
    let mut ok: GLint = -1;
    // SAFETY: plain GL call.
    unsafe { glGetProgramiv(p, GL_LINK_STATUS, &mut ok) };
    ok
}

#[test]
fn refused_shader_compile_status_and_log() {
    let _g = context();
    let (_s, st, log) = compile(GL_FRAGMENT_SHADER, BAD_FS);
    assert_eq!(st, 0, "COMPILE_STATUS of a shader with a syntax error");
    assert!(!log.trim_matches('\0').trim().is_empty(), "the info log is empty");
    ok("compile");
    /* a vertex shader the same */
    let (_s, st, log) = compile(GL_VERTEX_SHADER, "attribute vec4 p;\nvoid main() { gl_Position = p +; }\n");
    assert_eq!(st, 0);
    assert!(!log.trim().is_empty());
    ok("compile vs");
}

#[test]
fn unlinked_program_locations_are_errors() {
    let _g = context();
    /* glmark2: the refused shader is not attached, build() does not link */
    let p = refused_program(false, false);
    assert_eq!(link_status(p), 0, "a never-linked program");
    ok("link status");
    // SAFETY: plain GL calls; NUL-terminated names.
    unsafe {
        assert_eq!(glGetUniformLocation(p, c"u_mvp".as_ptr()), -1);
        assert_eq!(err(), GL_INVALID_OPERATION, "glGetUniformLocation on an unlinked program");
        assert_eq!(err(), GL_NO_ERROR, "the error flag is cleared by glGetError");
        assert_eq!(glGetAttribLocation(p, c"p".as_ptr()), -1);
        assert_eq!(err(), GL_INVALID_OPERATION, "glGetAttribLocation on an unlinked program");
    }
    /* the same for a program linked with the refused shader attached */
    let q = refused_program(true, true);
    assert_eq!(link_status(q), 0, "GL_LINK_STATUS with a refused shader attached");
    ok("link of a refused program");
    // SAFETY: plain GL calls.
    unsafe {
        assert_eq!(glGetUniformLocation(q, c"u_mvp".as_ptr()), -1);
        assert_eq!(err(), GL_INVALID_OPERATION);
        assert_eq!(glGetAttribLocation(q, c"p".as_ptr()), -1);
        assert_eq!(err(), GL_INVALID_OPERATION);
    }
}

#[test]
fn failed_link_use_program_is_invalid_operation() {
    let _g = context();
    for (link, attach) in [(false, false), (true, true)] {
        let p = refused_program(link, attach);
        assert_eq!(link_status(p), 0);
        // SAFETY: plain GL calls.
        unsafe {
            glUseProgram(p);
            assert_eq!(err(), GL_INVALID_OPERATION, "glUseProgram on an unlinked program (link {link})");
            /* the current program is unchanged: no program */
            let mut cur: GLint = -1;
            glGetIntegerv(GL_CURRENT_PROGRAM, &mut cur);
            assert_eq!(cur, 0, "a refused glUseProgram leaves the current program alone");
            glUseProgram(0);
        }
        ok("glUseProgram(0)");
    }
}

#[test]
fn uniform_to_minus_one_is_silent() {
    let _g = context();
    let p = program(VS1, FS1);
    let v = [1.0f32; 16];
    // SAFETY: plain GL calls; `v` holds 16 floats, enough for every call.
    unsafe {
        glUseProgram(p);
        glUniform1f(-1, 1.0);
        glUniform1i(-1, 1);
        glUniform2fv(-1, 1, v.as_ptr());
        glUniform3fv(-1, 1, v.as_ptr());
        glUniform4f(-1, 1.0, 2.0, 3.0, 4.0);
        glUniform4fv(-1, 1, v.as_ptr());
        glUniformMatrix3fv(-1, 1, 0, v.as_ptr());
        glUniformMatrix4fv(-1, 1, 0, v.as_ptr());
        assert_eq!(err(), GL_NO_ERROR, "glUniform*(-1, ...) with a program in use");
        /* ES 2.0 2.10.4: with no current program every glUniform* is
         * INVALID_OPERATION, the -1 location included (the `store_uniform`
         * 0x502 lines when glmark2's use of a refused program left none
         * current) */
        glUseProgram(0);
        glUniform4f(-1, 1.0, 2.0, 3.0, 4.0);
        assert_eq!(err(), GL_INVALID_OPERATION, "glUniform4f(-1, ...) with no current program");
    }
}

#[test]
fn negative_attribute_index_is_invalid_value() {
    let _g = context();
    let p = [0.0f32; 4];
    // SAFETY: plain GL calls; the arrays are not drawn from.
    unsafe {
        glBindBuffer(GL_ARRAY_BUFFER, 0);
        glEnableVertexAttribArray(u32::MAX);
        assert_eq!(err(), GL_INVALID_VALUE, "glEnableVertexAttribArray(-1)");
        glVertexAttribPointer(u32::MAX, 4, GL_FLOAT, 0, 0, p.as_ptr().cast());
        assert_eq!(err(), GL_INVALID_VALUE, "glVertexAttribPointer(-1, ...)");
    }
}

/// One quad (two triangles, client arrays) on attribute 0.
fn draw_quad() -> GLenum {
    let p: [f32; 16] = [-1.0, -1.0, 0.0, 1.0, -1.0, 1.0, 0.0, 1.0, 1.0, -1.0, 0.0, 1.0, 1.0, 1.0, 0.0, 1.0];
    static IDX: [u16; 6] = [0, 2, 1, 2, 1, 3];
    // SAFETY: `p` and `IDX` outlive the draw, which `glFinish` completes.
    unsafe {
        glBindBuffer(GL_ARRAY_BUFFER, 0);
        glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, p.as_ptr().cast());
        glEnableVertexAttribArray(0);
        glDrawElements(GL_TRIANGLES, 6, GL_UNSIGNED_SHORT, IDX.as_ptr().cast());
        glFinish();
        err()
    }
}

#[test]
fn draw_without_program_draws_nothing() {
    let _g = context();
    // SAFETY: plain GL call.
    unsafe { glUseProgram(0) };
    let before = backend::draws_refused();
    let e = draw_quad();
    /* ES 2.0 leaves a draw with no program undefined; Mesa raises
     * INVALID_OPERATION, which is also what this back end does and what the
     * record's 0x502 lines at the draw are */
    assert!(e == GL_NO_ERROR || e == GL_INVALID_OPERATION, "a draw with no current program: glGetError 0x{e:x}");
    assert!(SEEN.lock().unwrap().is_empty(), "a draw with no program reached the back end: {:?}", SEEN.lock().unwrap());
    assert_eq!(backend::draws_refused(), before, "the no-program draw counts as refused");
}

#[test]
fn good_program_draws_after_refusal() {
    let _g = context();
    /* the glmark2 sequence: refusal, queries, uniforms, a draw, all ignored */
    let bad = refused_program(false, false);
    // SAFETY: plain GL calls.
    unsafe {
        glGetUniformLocation(bad, c"u".as_ptr());
        glGetAttribLocation(bad, c"p".as_ptr());
        glUniform4f(-1, 0.0, 0.0, 0.0, 0.0);
        glUseProgram(bad);
    }
    while err() != GL_NO_ERROR {}
    let e = draw_quad();
    assert!(e == GL_NO_ERROR || e == GL_INVALID_OPERATION, "0x{e:x}");
    assert!(SEEN.lock().unwrap().is_empty(), "the refused program drew");
    while err() != GL_NO_ERROR {}
    /* then a good program on the same context */
    let p = program(VS1, FS1);
    // SAFETY: plain GL calls.
    unsafe { glUseProgram(p) };
    ok("use good program");
    let before = backend::draws_refused();
    SEEN.lock().unwrap().clear();
    assert_eq!(draw_quad(), GL_NO_ERROR);
    assert_eq!(backend::draws_refused(), before, "the good draw was refused");
    let s = SEEN.lock().unwrap().iter().find(|s| s.index_count == 6).copied().expect("the draw reached the back end");
    assert_eq!(s.mali, 0, "mali_draw refused the good program's draw");
    assert_ne!(s.target_gpu, 0, "the draw has a colour surface");
}
