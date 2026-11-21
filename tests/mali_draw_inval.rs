// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The row-8 draws of docs/es2-cts-failures.md section M.1 (23 Mali-G610 cases
//! with `GL_INVALID_OPERATION` at a draw) through the GL core on the Mali mock
//! (plan #373, item `drawinval-pin`).
//!
//! `attrgap-fix` (src/shaderc/online/mali/mod.rs, docs/notes/mali-attribute-records.md)
//! builds attribute records with an unread or dead attribute between read
//! ones; tests/mali_attr_gap.rs pins the `default_vertex_attrib` mat2/mat3
//! shapes (8 of the 23). This file pins the other groups the same way: a
//! program linked through `glLinkProgram` with no `glBindAttribLocation` (as
//! dEQP's shader-render cases do), every active attribute fed from a client
//! array, one triangle drawn by `glDrawArrays` into an 8x8 RGBA8 framebuffer
//! object. Each case asserts `GL_NO_ERROR`, `draws_refused() == 0` and no
//! refusal line in the OSAL log. No pixels are checked.
//!
//! Sources. The dEQP C++ files (es2fShaderOperatorTests.cpp,
//! glsRandomShaderCase.cpp, glsFragOpInteractionCase.cpp) are not on the
//! machine this test was written on, so the programs are rebuilt from the
//! shapes the repository holds (tools/mali/testdata/deqp_mixcol_f2i_seq.*,
//! deqp_mixcol_hp_seq.*, deqp_intval3_opfs_sequence_*: the fragment cases'
//! `v_in0..v_in2` varyings are the vertex cases' `a_in0..a_in2` attributes)
//! and from the case names of the board log
//! (tools/cts/log/2026-11-15-t6-gles2-S2-37305608/results.tsv). They are
//! not byte copies; the random generator's seeds and the interaction seeds
//! 11 and 66 are not replayed. The pins hold the attribute shape of each
//! case (which attributes it declares, which it reads), which is what
//! `attrgap-fix` changed. A byte-exact replay needs the dEQP sources and is
//! left to the board rerun.
//!
//! The 23 cases of row 8:
//!
//! * `shaders.operator.sequence.side_effects.{lowp,mediump,highp}_vec4_vertex`
//!   (3): `sequence_side_effects_vec4_vertex`.
//! * `shaders.operator.sequence.no_side_effects.{lowp,mediump,highp}_vec4_ivec4_bvec4_vertex`
//!   (3): `sequence_vec4_ivec4_bvec4_vertex`.
//! * `shaders.random.basic_expression.vertex.{53,85,90,95,96}`,
//!   `scalar_conversion.vertex.53`, `texture.vertex.19` (7): the generator's
//!   shape, inputs `in0..inN` declared and some unread: `random_vertex_*`.
//! * `fragment_ops.interaction.basic_shader.{11,66}` (2): the basic shader
//!   (`a_position`, `a_color`) under blend, scissor and colour-mask state:
//!   `interaction_basic_shader_*`.
//! * `default_vertex_attrib.{mat2,mat3}` (8): tests/mali_attr_gap.rs.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_draw_inval
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
const GPU: OsalMaliGpu = OsalMaliGpu {
    gpu_id: 0xa867_0005,
    tiler_features: 0x809,
    cs_reg_count: 96,
    unpreserved_cs_reg_count: 4,
    core_features: 0,
};
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

/* ---- shaders.operator.sequence --------------------------------------------- */

/// ShaderRenderCase's vertex program for an operator case: `a_position`, the
/// inputs `a_in0..a_in2` and the result in `v_color`.
fn operator_vs(prec: &str, body: &str) -> String {
    format!(
        "attribute highp vec4 a_position;\nattribute {prec} vec4 a_in0;\nattribute {prec} vec4 a_in1;\n\
         attribute {prec} vec4 a_in2;\nvarying mediump vec4 v_color;\n\
         void main()\n{{\n\tgl_Position = a_position;\n{body}\n}}\n"
    )
}

/// `no_side_effects.{lowp,mediump,highp}_vec4_ivec4_bvec4_vertex`: the
/// sequence `(in0 + vec4(in1), in2, in1)` of the fragment case
/// tools/mali/testdata `deqp_mixcol_f2i_seq`, in a vertex shader. All three inputs
/// are read, so this is also the control for the attribute numbering.
#[test]
fn sequence_vec4_ivec4_bvec4_vertex() {
    let _g = context();
    for prec in ["lowp", "mediump", "highp"] {
        let body = format!(
            "\t{prec} vec4 in0 = a_in0.yzwx;\n\t{prec} ivec4 in1 = ivec4(a_in1.wzyx);\n\
             \tbvec4 in2 = greaterThan(a_in2.xwzy, vec4(0.0));\n\t{prec} ivec4 res = ivec4(0.0);\n\
             \tres = (in0 + vec4(in1), in2, in1);\n\t{prec} vec4 color = vec4(0.0, 0.0, 0.0, 1.0);\n\
             \tcolor.xyzw = vec4(res);\n\tv_color = color * 0.50;"
        );
        draw(&format!("{prec}_vec4_ivec4_bvec4_vertex"), &operator_vs(prec, &body), FS_COLOR, "a_position", nothing);
    }
}

/// `side_effects.{lowp,mediump,highp}_vec4_vertex`: the sequence assigns its
/// first operand (`in0 = in1`), only `a_in0` and `a_in1` are read and `a_in2`
/// is declared and unread (the gap the board refused).
#[test]
fn sequence_side_effects_vec4_vertex() {
    let _g = context();
    for prec in ["lowp", "mediump", "highp"] {
        let body = format!(
            "\t{prec} vec4 in0 = a_in0.yzwx;\n\t{prec} vec4 in1 = a_in1.wzyx;\n\t{prec} vec4 res = vec4(0.0);\n\
             \tres = (in0 = in1, in0 + in1);\n\t{prec} vec4 color = vec4(0.0, 0.0, 0.0, 1.0);\n\
             \tcolor.xyzw = res + in0;\n\tv_color = color * 0.50;"
        );
        draw(&format!("{prec}_vec4_vertex side effects"), &operator_vs(prec, &body), FS_COLOR, "a_position", nothing);
    }
}

/* ---- shaders.random ------------------------------------------------------------ */

/// glsRandomShaderCase's vertex program: `dEQP_Position`, inputs `in0..inN`
/// of mixed types declared in order, the result in `out0`. Which inputs the
/// body reads is the generator's choice; the pins read some and skip others.
fn random_vs(decls: &str, body: &str) -> String {
    format!(
        "precision mediump float;\nattribute highp vec4 dEQP_Position;\n{decls}\
         varying mediump vec4 v_color;\nvoid main()\n{{\n{body}\tgl_Position = dEQP_Position;\n}}\n"
    )
}

/// `basic_expression.vertex.*` shape: a `vec2`, an unread `vec3` and a read
/// `float` (the unread attribute between two read ones).
#[test]
fn random_vertex_unread_between_read() {
    let _g = context();
    draw(
        "random: vec2, unread vec3, float",
        &random_vs(
            "attribute vec2 in0;\nattribute vec3 in1;\nattribute float in2;\n",
            "\tv_color = vec4(in0, in2, 1.0);\n",
        ),
        FS_COLOR,
        "dEQP_Position",
        nothing,
    );
}

/// An unread first input (`in0`), the read ones after it.
#[test]
fn random_vertex_unread_first() {
    let _g = context();
    draw(
        "random: unread vec4, vec3, vec2",
        &random_vs(
            "attribute vec4 in0;\nattribute vec3 in1;\nattribute vec2 in2;\n",
            "\tv_color = vec4(in1.zyx * in2.x, in2.y);\n",
        ),
        FS_COLOR,
        "dEQP_Position",
        nothing,
    );
}

/// `scalar_conversion.vertex.53` shape: float inputs, a conversion through
/// `bool` and `int`, one input unread among them.
#[test]
fn random_vertex_scalar_conversion() {
    let _g = context();
    draw(
        "random: scalar conversion",
        &random_vs(
            "attribute float in0;\nattribute float in1;\nattribute float in2;\nattribute float in3;\n",
            "\tfloat a = float(int(in0 * 4.0)) * 0.25;\n\tfloat b = float(bool(in3));\n\
             \tv_color = vec4(a, b, in3, 1.0);\n",
        ),
        FS_COLOR,
        "dEQP_Position",
        nothing,
    );
}

/// `texture.vertex.19` shape: the texture coordinate inputs read, a `vec4`
/// before them unread. (No texture is sampled: the attribute record is the
/// pin; vertex texture look-ups are tests/mali_vcube_magswitch.rs's.)
#[test]
fn random_vertex_texture_inputs() {
    let _g = context();
    draw(
        "random: texture coordinates after an unread input",
        &random_vs(
            "attribute vec4 in0;\nattribute vec2 in1;\nattribute vec3 in2;\n",
            "\tv_color = vec4(in1, in2.xy) * 0.5;\n",
        ),
        FS_COLOR,
        "dEQP_Position",
        nothing,
    );
}

/// The unread input last, and an input read only through a `varying` branch.
#[test]
fn random_vertex_unread_last() {
    let _g = context();
    draw(
        "random: read, read, unread",
        &random_vs(
            "attribute vec3 in0;\nattribute vec2 in1;\nattribute vec4 in2;\n",
            "\tv_color = vec4(in0.xy + in1, in0.z, 1.0);\n",
        ),
        FS_COLOR,
        "dEQP_Position",
        nothing,
    );
}

/* ---- fragment_ops.interaction -------------------------------------------------- */

/// glsFragOpInteractionCase's basic shader: `a_position` and `a_color`
/// carried to `v_color`.
const INTERACTION_VS: &str = "attribute highp vec4 a_position;\nattribute mediump vec4 a_color;\n\
    varying mediump vec4 v_color;\nvoid main() { gl_Position = a_position; v_color = a_color; }\n";

/// `basic_shader.11` and `.66` shape: blend with a constant-free factor
/// pair, scissor and colour mask set at once, drawn with the basic shader.
/// (The seeds pick the states at random; the draw's attribute side is the
/// basic shader's two attributes.)
#[test]
fn interaction_basic_shader_state() {
    let _g = context();
    draw("interaction basic shader: blend, scissor, colour mask", INTERACTION_VS, FS_COLOR, "a_position", || {
        // SAFETY: plain GL calls.
        unsafe {
            glEnable(GL_BLEND);
            glBlendFunc(GL_SRC_ALPHA, GL_ONE_MINUS_SRC_ALPHA);
            glEnable(GL_SCISSOR_TEST);
            glScissor(1, 1, 6, 6);
            glColorMask(1, 0, 1, 1);
        }
    });
}

/// The same program with no state: the control.
#[test]
fn interaction_basic_shader_control() {
    let _g = context();
    draw("interaction basic shader: control", INTERACTION_VS, FS_COLOR, "a_position", nothing);
}
