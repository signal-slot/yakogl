// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The row-8 draws that `attrgap-fix` leaves (docs/es2-cts-failures.md
//! section M.1 row 8: `GL_INVALID_OPERATION` at a draw) through the GL core
//! on the Mali mock (plan #373, item `drawinval-pin2`). tests/mali_draw_inval.rs
//! pins the shapes of the cases that draw now; this file pins the ones that
//! do not, each with the refusal behind it.
//!
//! A draw is replayed as in that file: a program linked with no
//! `glBindAttribLocation`, every active attribute fed from a client array,
//! one triangle by `glDrawArrays` into an 8x8 RGBA8 framebuffer object, and
//! the test asserts `GL_NO_ERROR`, `draws_refused() == 0` and no refusal line.
//! A test that is `#[ignore]` asserts what dEQP wants (the draw succeeds)
//! and names in its reason the GL error, the refusal text and where it
//! arises; `cargo test -- --ignored` shows the refusal.
//!
//! Sources. dEQP is not on the machine this was written on and could not be
//! fetched (tools/cts/deps.toml pins VK-GL-CTS 93bca01; only its GLES3 shader
//! corpus is in `$CONNECTED_DEPS_DIR`, not `glsRandomShaderCase.cpp` or the
//! random shader generator). So only a shader the repository holds byte for
//! byte is a replay: `shaders.random.basic_expression.vertex.96`, whose body
//! `src/shaderc/online/mali/tests.rs`
//! (`a_late_fold_is_refused_only_where_the_attribute_numbering_depends_on_it`)
//! keeps as dEQP's text (`1405_0`). The other row-8 cases of the board
//! slices (`basic_expression.vertex.{53,85,90,95}`,
//! `scalar_conversion.vertex.53`, `texture.vertex.19`,
//! `interaction.basic_shader.{11,66}`) have no source or refusal line in the
//! repository (the 2026-11-1x slices predate the per-case refusal line, and
//! their `refusals.tsv` has no row for them); they are left to the board
//! rerun and not pinned here.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_draw_inval_rest
//! ```

#![cfg(all(feature = "host-test", feature = "shaderc-online", feature = "gpu-mali", not(feature = "report-es2")))]

use core::ffi::{c_char, c_void};
use std::ffi::CString;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend;
use yakogl::gl::buffer::glBindBuffer;
use yakogl::gl::consts::*;
use yakogl::gl::draw::*;
use yakogl::gl::framebuffer::*;
use yakogl::gl::program::*;
use yakogl::gl::shader::*;
use yakogl::gl::state::*;
use yakogl::gl::types::*;
use yakogl::gl::vertex::{glDisableVertexAttribArray, glEnableVertexAttribArray, glVertexAttribPointer};
use yakogl::osal::mali::{OsalMaliGpu, OsalMaliHeap, OSAL_QUERY_MALI_GPU, OSAL_QUERY_MALI_TILER_HEAP};
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};
use yakogl::util::ENOTSUP;

/* ---- the Mali over a mock OSAL ------------------------------------------ */

/// The OSAL's answers: the board's (panthor-frame on the T6).
const GPU: OsalMaliGpu = OsalMaliGpu::T6;
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

/// The log lines since the last `context()`.
static LOG: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// The OSAL's log hook: the library only calls it as `log("%s", line)`, so a
/// two-pointer function takes its place (the same calling convention for
/// these arguments).
unsafe extern "C" fn m_log(_fmt: *const c_char, line: *const c_char) {
    let line = core::ffi::CStr::from_ptr(line).to_string_lossy().into_owned();
    eprintln!("osal log: {line}");
    LOG.lock().unwrap_or_else(|e| e.into_inner()).push(line);
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

/// One context for the whole file; the tests take turns on it.
static LOCK: Mutex<()> = Mutex::new(());
static ONCE: Once = Once::new();

/// The GL context and an 8x8 RGBA8 framebuffer object bound.
fn context() -> MutexGuard<'static, ()> {
    let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    ONCE.call_once(|| {
        backend::HOST_DRAW_TARGETS.store(1, Ordering::Relaxed);
        // SAFETY: static tables and one context; the lock above serialises
        // the tests.
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
        }
    });
    LOG.lock().unwrap_or_else(|e| e.into_inner()).clear();
    // SAFETY: plain GL calls.
    unsafe {
        glClearColor(0.5, 0.25, 0.75, 0.4);
        glClear(GL_COLOR_BUFFER_BIT);
        while glGetError() != GL_NO_ERROR {}
    }
    g
}

/// Covers the whole target.
static FULL: [f32; 12] = [-1.0, -1.0, 0.0, 1.0, 3.0, -1.0, 0.0, 1.0, -1.0, 3.0, 0.0, 1.0];
/// One vec4 per vertex.
static ATTR: [f32; 12] = [0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9, 0.3, 0.2, 0.1];

fn compile(ty: GLenum, text: &str) -> GLuint {
    let src = CString::new(text).unwrap();
    // SAFETY: plain GL calls; one NUL-terminated source.
    unsafe {
        let s = glCreateShader(ty);
        let p: [*const c_char; 1] = [src.as_ptr()];
        glShaderSource(s, 1, p.as_ptr(), core::ptr::null());
        glCompileShader(s);
        let mut ok: GLint = 0;
        glGetShaderiv(s, GL_COMPILE_STATUS, &mut ok);
        assert_eq!(ok, 1, "the shader does not compile:\n{text}");
        s
    }
}

/// Link `vs` + `fs` with no bound locations (the GL core numbers the
/// attributes), feed every active attribute from a client array (the one
/// named `pos` from `FULL`, the others from `ATTR`), call `setup` for the
/// state, draw one triangle and assert it drew unrefused.
fn draw(label: &str, vs: &str, fs: &str, pos: &str, setup: impl FnOnce()) {
    let (v, f) = (compile(GL_VERTEX_SHADER, vs), compile(GL_FRAGMENT_SHADER, fs));
    let before = backend::draws_refused();
    // SAFETY: plain GL calls; `FULL` and `ATTR` are static and outlive the draw.
    let e = unsafe {
        let p = glCreateProgram();
        glAttachShader(p, v);
        glAttachShader(p, f);
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        assert_eq!(ok, 1, "{label}: the program does not link");
        glUseProgram(p);
        glBindBuffer(GL_ARRAY_BUFFER, 0);
        let mut n: GLint = 0;
        glGetProgramiv(p, GL_ACTIVE_ATTRIBUTES, &mut n);
        let mut locs = Vec::new();
        for i in 0..n as GLuint {
            let mut name = [0 as c_char; 64];
            let (mut len, mut size, mut ty) = (0, 0, 0);
            glGetActiveAttrib(p, i, 64, &mut len, &mut size, &mut ty, name.as_mut_ptr());
            let l = glGetAttribLocation(p, name.as_ptr());
            assert!(l >= 0, "{label}: attribute {i} has no location");
            let nm = core::ffi::CStr::from_ptr(name.as_ptr()).to_str().unwrap().to_owned();
            let data = if nm == pos { &FULL } else { &ATTR };
            glVertexAttribPointer(l as GLuint, 4, GL_FLOAT, 0, 0, data.as_ptr() as *const c_void);
            glEnableVertexAttribArray(l as GLuint);
            locs.push(l as GLuint);
        }
        setup();
        while glGetError() != GL_NO_ERROR {}
        glDrawArrays(GL_TRIANGLES, 0, 3);
        glFinish();
        let e = glGetError();
        for l in locs {
            glDisableVertexAttribArray(l);
        }
        glDisable(GL_BLEND);
        glDisable(GL_SCISSOR_TEST);
        glColorMask(1, 1, 1, 1);
        e
    };
    let refused = backend::draws_refused() - before;
    let log = LOG.lock().unwrap_or_else(|e| e.into_inner()).clone();
    let refusals: Vec<_> = log.iter().filter(|l| l.contains("refus")).collect();
    assert_eq!(refused, 0, "{label}: the draw was refused; refusal lines {refusals:?}");
    assert!(refusals.is_empty(), "{label}: refusal lines {refusals:?}");
    assert_eq!(e, GL_NO_ERROR, "{label}: glGetError 0x{e:x}");
}

fn nothing() {}

/// The fragment shader of dEQP's vertex-case programs.
const FS_COLOR: &str =
    "precision mediump float;\nvarying mediump vec4 v_color;\nvoid main() { gl_FragColor = v_color; }\n";

/// `shaders.random.basic_expression.vertex.96` (dEQP `1405_0`), with the
/// generator's input named `dEQP_Position` and the result in `v_color`.
/// `d`'s only use is lane 1 of a product with a constant `-0.0`: our link
/// folds the lane to a constant and would not read `d`'s location, Mesa's
/// does.
const VERTEX_96: &str = "attribute vec4 dEQP_Position; const float e = -6.5 - float(7.0) * -1.0; \
    attribute float d; float c = float(float(-7.625)) + float(float(7.5)); attribute float a; \
    attribute float b; varying mediump vec4 v_color; void main() { gl_Position = dEQP_Position; \
    v_color = vec4(float(e) * 2.0, (float(d)), c * 2.0, float(c)); \
    v_color = (v_color) * (vec4(0.5, -0.0, b, a)); }\n";

/// `shaders.random.basic_expression.vertex.96`: `GL_INVALID_OPERATION` at
/// the draw on the board (glsRandomShaderCase.cpp:611, a build before
/// `attrgap-fix`; the other `*.vertex.96` categories of the same slices
/// already read "Failed to compile shader").
///
/// At the tip `glCompileShader` fails (`GL_COMPILE_STATUS` 0, so dEQP reads
/// "Failed to compile shader", not an error at the draw): the online compiler
/// refuses the vertex shader with `MaliError::AttributeNumbering`, log line
/// "online shaderc: the code generator refused this vertex shader:
/// Mali(Valhall: a vertex shader whose attribute numbering waits for a fold
/// Mesa's back end makes after it numbers the locations, of a form the walk
/// does not model, is not lowered yet)" (src/shaderc/online/mali/nir.rs
/// `build`, the `1405_0` comment; docs/notes/mali-builtin-inputs.md,
/// "Attribute records"). Owner: the compiler plans (M.2 family
/// `AttributeNumbering`), not src/mali: the attribute records themselves
/// are `attrgap-fix`'s.
#[test]
#[ignore = "glCompileShader fails (was GL_INVALID_OPERATION at the draw): online shaderc refuses the vertex shader with \
            MaliError::AttributeNumbering (src/shaderc/online/mali/nir.rs build, the 1405_0 late-fold case; M.2, compiler plans)"]
fn random_basic_expression_vertex_96() {
    let _g = context();
    draw("random.basic_expression.vertex.96", VERTEX_96, FS_COLOR, "dEQP_Position", nothing);
}

/// The control for `vertex.96`: the same shader with the zero factor
/// replaced by `1.0`, so no lane folds away and no location depends on a
/// fold. It draws, which shows the refusal above is the late fold and not
/// the attribute numbering of the declared order (`d`, `a`, `b`).
#[test]
fn random_basic_expression_vertex_96_without_the_late_fold() {
    let _g = context();
    let vs = VERTEX_96.replace("vec4(0.5, -0.0, b, a)", "vec4(0.5, 1.0, b, a)");
    assert_ne!(vs, VERTEX_96);
    draw("vertex.96 without the late fold", &vs, FS_COLOR, "dEQP_Position", nothing);
}
