// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GL-on-mock pin (apistub-pin, plan #373): dEQP's stub programs of the
//! `negative_api`, `state_query` and `shader_api.compile_link` cases through
//! the GL core on the Mali mock.
//!
//! Those dEQP cases build a program from a one-line stub pair
//! (`void main(void) { gl_Position = vec4(0.0); }` and
//! `void main(void) { gl_FragColor = vec4(0.0); }`, as in dEQP-GLES2's
//! es2fNegativeShaderApiTests, es2fShaderStateQueryTests and
//! es2fShaderApiTests; the sources were not at hand, the text is from
//! memory of them) and then check an error code or a query value. The online
//! Mali compiler refuses the constant `gl_Position`, so the program does not
//! link and the later GL calls answer something else.
//!
//! Each test is one dEQP expectation. A test that passes at the tip runs
//! normally. A test the stub-shader refusal breaks carries
//! `#[ignore = "apierr-fix: stub vertex shader constant (<dEQP case>)"]`;
//! apierr-fix removes the ignores when the stub vertex shader links.
//! `cargo test ... -- --ignored` shows which of them still fail.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_api_stub
//! ```

#![cfg(all(feature = "host-test", feature = "shaderc-online", feature = "gpu-mali", not(feature = "report-es2")))]

use core::ffi::{c_char, c_void};
use std::ffi::CString;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use yakogl::backend;
use yakogl::gl::consts::*;
use yakogl::gl::framebuffer::*;
use yakogl::gl::program::*;
use yakogl::gl::shader::*;
use yakogl::gl::state::*;
use yakogl::gl::types::*;
use yakogl::osal::mali::{OsalMaliGpu, OsalMaliHeap, OSAL_QUERY_MALI_GPU, OSAL_QUERY_MALI_TILER_HEAP};
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};
use yakogl::util::ENOTSUP;

/* ---- the Mali over a mock OSAL ------------------------------------------ */

/// The OSAL's answers: the board's (panthor-frame on the T6).
const GPU: OsalMaliGpu =
    OsalMaliGpu { gpu_id: 0xa867_0005, tiler_features: 0x809, cs_reg_count: 96, unpreserved_cs_reg_count: 4 };
const HEAP: OsalMaliHeap =
    OsalMaliHeap { ctx_gpu: 0x8000_0001_5000, first_chunk_gpu: 0x8000_00a0_0000, chunk_size: 2 << 20, reserved: 0 };

/// Buffers made so far: each one's GPU VA is its number times 16 MiB, far
/// from any CPU address. Nothing is given back (a few MiB for the run).
static ALLOCS: AtomicU64 = AtomicU64::new(0);
static FENCES: AtomicI64 = AtomicI64::new(0);

unsafe extern "C" fn m_alloc(_heap: u64, size: usize, _flags: u32, out: *mut OsalGpuMem) -> i32 {
    let n = ALLOCS.fetch_add(1, Ordering::Relaxed) + 1;
    assert!(size <= 16 << 20);
    let cpu = Box::leak(vec![0u64; size.div_ceil(8)].into_boxed_slice()).as_mut_ptr().cast::<c_void>();
    *out = OsalGpuMem { cpu, gpu: 0x40_0000_0000 + n * (16 << 20), phys: 0, size, impl_: n as *mut c_void };
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
unsafe extern "C" fn m_submit(_kick: *const c_void, _len: usize) -> i64 {
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

/// The OSAL's log hook: the library only calls it as `log("%s", line)`, so a
/// two-pointer function takes its place (the same calling convention for
/// these arguments).
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
    // SAFETY: see `m_log`.
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

/// The GL context and an 8x8 RGBA8 framebuffer object bound.
fn setup() {
    backend::HOST_DRAW_TARGETS.store(1, Ordering::Relaxed);
    // SAFETY: static tables and one context; this is the only test of the
    // file.
    unsafe {
        osal_set_ops(&OSAL);
        backend::g_backend = &yakogl::mali::backend::yakogl_backend_mali;
        assert_eq!(yakogl::mali::backend::mali_init(), 0);
        yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
        assert!(!yakogl::gl::context::gl_context_create(8, 8).is_null());
        let (mut fb, mut rb) = (0, 0);
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        glGenRenderbuffers(1, &mut rb);
        glBindRenderbuffer(GL_RENDERBUFFER, rb);
        glRenderbufferStorage(GL_RENDERBUFFER, GL_RGBA8, 8, 8);
        glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_RENDERBUFFER, rb);
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE);
        glClearColor(0.5, 0.25, 0.75, 0.4);
        glClear(GL_COLOR_BUFFER_BIT);
        while glGetError() != GL_NO_ERROR {}
    }
}

/* ---- the harness over the stub programs ---------------------------------- */

use std::sync::{Mutex, MutexGuard, Once};

static INIT: Once = Once::new();
static LOCK: Mutex<()> = Mutex::new(());

/// The one GL context, made once; the tests run one at a time. A fresh error
/// state and no program in use at the start of each.
fn api() -> MutexGuard<'static, ()> {
    let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    INIT.call_once(setup);
    // SAFETY: plain GL calls on the context `setup` made.
    unsafe {
        glUseProgram(0);
        while glGetError() != GL_NO_ERROR {}
    }
    g
}

/// dEQP's stub pair.
const VS_STUB: &str = "void main(void)\n{\n\tgl_Position = vec4(0.0);\n}\n";
const FS_STUB: &str = "void main(void)\n{\n\tgl_FragColor = vec4(0.0);\n}\n";

fn shader(ty: GLenum, text: &str) -> GLuint {
    let text = CString::new(text).unwrap();
    // SAFETY: plain GL calls; one NUL-terminated source.
    unsafe {
        let s = glCreateShader(ty);
        let p: [*const c_char; 1] = [text.as_ptr()];
        glShaderSource(s, 1, p.as_ptr(), core::ptr::null());
        glCompileShader(s);
        s
    }
}

fn compile_status(s: GLuint) -> GLint {
    let mut ok: GLint = -1;
    // SAFETY: one out value.
    unsafe { glGetShaderiv(s, GL_COMPILE_STATUS, &mut ok) };
    ok
}

fn program_iv(p: GLuint, pname: GLenum) -> GLint {
    let mut v: GLint = -1;
    // SAFETY: one out value.
    unsafe { glGetProgramiv(p, pname, &mut v) };
    v
}

fn err() -> GLenum {
    // SAFETY: plain GL call.
    unsafe { glGetError() }
}

/// A program of the stub pair, linked.
fn stub_program() -> (GLuint, GLuint, GLuint) {
    let v = shader(GL_VERTEX_SHADER, VS_STUB);
    let f = shader(GL_FRAGMENT_SHADER, FS_STUB);
    // SAFETY: plain GL calls.
    let p = unsafe {
        let p = glCreateProgram();
        glAttachShader(p, v);
        glAttachShader(p, f);
        glLinkProgram(p);
        p
    };
    (p, v, f)
}

/* ---- what works at the tip ------------------------------------------------ */

#[test]
fn fragment_stub_compiles() {
    let _g = api();
    let f = shader(GL_FRAGMENT_SHADER, FS_STUB);
    assert_eq!(compile_status(f), 1);
    assert_eq!(err(), GL_NO_ERROR);
}

#[test]
fn negative_api_errors_without_a_program() {
    let _g = api();
    // SAFETY: plain GL calls; the names are not objects.
    unsafe {
        // negative_api.shader.use_program: an unknown name.
        glUseProgram(0xdead);
        assert_eq!(err(), GL_INVALID_VALUE);
        // negative_api.shader.link_program / validate_program / attach_shader.
        glLinkProgram(0xdead);
        assert_eq!(err(), GL_INVALID_VALUE);
        glValidateProgram(0xdead);
        assert_eq!(err(), GL_INVALID_VALUE);
        glAttachShader(0xdead, 0xbeef);
        assert_eq!(err(), GL_INVALID_VALUE);
        // negative_api.shader.compile_shader / delete_shader / delete_program.
        glCompileShader(0xdead);
        assert_eq!(err(), GL_INVALID_VALUE);
        glDeleteShader(0xdead);
        assert_eq!(err(), GL_INVALID_VALUE);
        glDeleteProgram(0xdead);
        assert_eq!(err(), GL_INVALID_VALUE);
        // negative_api.shader.create_shader: a bad type.
        assert_eq!(glCreateShader(0x1234), 0);
        assert_eq!(err(), GL_INVALID_ENUM);
        // negative_api.shader.bind_attrib_location: gl_ prefix, bad index.
        let p = glCreateProgram();
        glBindAttribLocation(p, 0, c"gl_foo".as_ptr());
        assert_eq!(err(), GL_INVALID_OPERATION);
        glBindAttribLocation(p, 1 << 20, c"foo".as_ptr());
        assert_eq!(err(), GL_INVALID_VALUE);
        glBindAttribLocation(0xdead, 0, c"foo".as_ptr());
        assert_eq!(err(), GL_INVALID_VALUE);
    }
}

#[test]
fn negative_api_attach_detach_errors() {
    let _g = api();
    let v = shader(GL_VERTEX_SHADER, VS_STUB);
    let f = shader(GL_FRAGMENT_SHADER, FS_STUB);
    // SAFETY: plain GL calls.
    unsafe {
        let p = glCreateProgram();
        // negative_api.shader.attach_shader: a program as the shader, twice.
        glAttachShader(p, p);
        assert_eq!(err(), GL_INVALID_OPERATION);
        glAttachShader(v, v);
        assert_eq!(err(), GL_INVALID_OPERATION);
        glAttachShader(p, v);
        assert_eq!(err(), GL_NO_ERROR);
        glAttachShader(p, v);
        assert_eq!(err(), GL_INVALID_OPERATION);
        // negative_api.shader.detach_shader: not attached, not an object.
        glDetachShader(p, f);
        assert_eq!(err(), GL_INVALID_OPERATION);
        glDetachShader(p, 0xdead);
        assert_eq!(err(), GL_INVALID_VALUE);
        glDetachShader(p, v);
        assert_eq!(err(), GL_NO_ERROR);
    }
}

/* ---- what the stub vertex shader's refusal breaks ------------------------ */

#[test]
#[ignore = "apierr-fix: stub vertex shader constant (shader_api.compile_link.compile_vertex_shader)"]
fn vertex_stub_compiles() {
    let _g = api();
    let v = shader(GL_VERTEX_SHADER, VS_STUB);
    assert_eq!(compile_status(v), 1);
    assert_eq!(err(), GL_NO_ERROR);
}

#[test]
#[ignore = "apierr-fix: stub vertex shader constant (shader_api.compile_link.link_vertex_fragment)"]
fn stub_pair_links() {
    let _g = api();
    let (p, _v, _f) = stub_program();
    assert_eq!(program_iv(p, GL_LINK_STATUS), 1);
    assert_eq!(err(), GL_NO_ERROR);
}

#[test]
#[ignore = "apierr-fix: stub vertex shader constant (state_query.current_program_binding_*)"]
fn current_program_binding() {
    let _g = api();
    let (p, _v, _f) = stub_program();
    // SAFETY: plain GL calls; one out value each.
    unsafe {
        glUseProgram(p);
        assert_eq!(err(), GL_NO_ERROR);
        let mut cur: GLint = -1;
        glGetIntegerv(GL_CURRENT_PROGRAM, &mut cur);
        assert_eq!(cur as GLuint, p);
        assert_eq!(glIsProgram(p), 1);
        // A program deleted while in use stays current until it is unbound.
        glDeleteProgram(p);
        glGetIntegerv(GL_CURRENT_PROGRAM, &mut cur);
        assert_eq!(cur as GLuint, p);
        glUseProgram(0);
        assert_eq!(glIsProgram(p), 0);
    }
}

#[test]
#[ignore = "apierr-fix: stub vertex shader constant (state_query.shader.delete_status)"]
fn delete_status() {
    let _g = api();
    let (p, v, f) = stub_program();
    // SAFETY: plain GL calls; one out value each.
    unsafe {
        let mut st: GLint = -1;
        glGetShaderiv(v, GL_DELETE_STATUS, &mut st);
        assert_eq!(st, 0);
        glUseProgram(p);
        glDeleteShader(v);
        glDeleteShader(f);
        glGetShaderiv(v, GL_DELETE_STATUS, &mut st);
        assert_eq!(st, 1, "an attached shader is flagged, not freed");
        assert_eq!(glIsShader(v), 1);
        glDeleteProgram(p);
        glUseProgram(0);
        assert_eq!(glIsShader(v), 0, "gone with its program");
        assert_eq!(err(), GL_NO_ERROR);
    }
}

#[test]
#[ignore = "apierr-fix: stub vertex shader constant (state_query.shader.shader_compile_status)"]
fn shader_compile_status() {
    let _g = api();
    let v = shader(GL_VERTEX_SHADER, VS_STUB);
    let f = shader(GL_FRAGMENT_SHADER, FS_STUB);
    assert_eq!(compile_status(v), 1);
    assert_eq!(compile_status(f), 1);
    // The invalid source of the same case reports false.
    let bad = shader(GL_VERTEX_SHADER, "void main(void) { gl_Position = ; }\n");
    assert_eq!(compile_status(bad), 0);
}

#[test]
#[ignore = "apierr-fix: stub vertex shader constant (state_query.shader.program_validate_status)"]
fn program_validate_status() {
    let _g = api();
    let (p, _v, _f) = stub_program();
    assert_eq!(program_iv(p, GL_LINK_STATUS), 1);
    // SAFETY: plain GL calls.
    unsafe {
        glValidateProgram(p);
        assert_eq!(program_iv(p, GL_VALIDATE_STATUS), 1);
        assert_eq!(err(), GL_NO_ERROR);
    }
}

#[test]
fn active_uniform_query() {
    let _g = api();
    // Both uniforms in the vertex shader: a uniform read in the fragment
    // shader hits another refusal of the fragment compiler, not the stub's.
    let v = shader(
        GL_VERTEX_SHADER,
        "uniform vec4 u_v;\nuniform float u_f;\nvoid main(void)\n{\n\tgl_Position = u_v + vec4(u_f);\n}\n",
    );
    let f = shader(GL_FRAGMENT_SHADER, FS_STUB);
    // SAFETY: plain GL calls; one out value each.
    unsafe {
        let p = glCreateProgram();
        glAttachShader(p, v);
        glAttachShader(p, f);
        glLinkProgram(p);
        assert_eq!(program_iv(p, GL_LINK_STATUS), 1);
        assert_eq!(program_iv(p, GL_ACTIVE_UNIFORMS), 2);
        let (mut len, mut size, mut ty): (GLsizei, GLint, GLenum) = (0, 0, 0);
        let mut name = [0 as c_char; 32];
        for i in 0..2 {
            glGetActiveUniform(p, i, 32, &mut len, &mut size, &mut ty, name.as_mut_ptr());
            match core::ffi::CStr::from_ptr(name.as_ptr()).to_str().unwrap() {
                "u_v" => assert_eq!(ty, GL_FLOAT_VEC4),
                "u_f" => assert_eq!(ty, GL_FLOAT),
                other => panic!("uniform {other}"),
            }
            assert_eq!(size, 1);
        }
        assert_eq!(err(), GL_NO_ERROR);
    }
}

#[test]
#[ignore = "apierr-fix: stub vertex shader constant (negative_api.shader.use_program, get_uniform_location, uniform_*, link_program, validate_program)"]
fn negative_api_with_a_linked_program() {
    let _g = api();
    let (p, v, _f) = stub_program();
    // SAFETY: plain GL calls.
    unsafe {
        assert_eq!(program_iv(p, GL_LINK_STATUS), 1);
        // use_program / get_uniform_location on a shader's name.
        glUseProgram(v);
        assert_eq!(err(), GL_INVALID_OPERATION);
        glGetUniformLocation(v, c"x".as_ptr());
        assert_eq!(err(), GL_INVALID_OPERATION);
        assert_eq!(glGetUniformLocation(p, c"missing".as_ptr()), -1);
        assert_eq!(err(), GL_NO_ERROR);
        // uniform_*: no program in use, then location -1 on a program in use.
        glUniform1f(0, 1.0);
        assert_eq!(err(), GL_INVALID_OPERATION);
        glUseProgram(p);
        glUniform1f(-1, 1.0);
        assert_eq!(err(), GL_NO_ERROR);
        // link_program / validate_program on a shader's name.
        glLinkProgram(v);
        assert_eq!(err(), GL_INVALID_OPERATION);
        glValidateProgram(v);
        assert_eq!(err(), GL_INVALID_OPERATION);
        // get_active_uniform: an index past the count.
        let (mut len, mut size, mut ty): (GLsizei, GLint, GLenum) = (0, 0, 0);
        let mut name = [0 as c_char; 8];
        glGetActiveUniform(p, 1, 8, &mut len, &mut size, &mut ty, name.as_mut_ptr());
        assert_eq!(err(), GL_INVALID_VALUE);
        glUseProgram(0);
    }
}
