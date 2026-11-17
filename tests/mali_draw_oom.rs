// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! No draw of the cause-6 shapes returns `GL_OUT_OF_MEMORY` on the Mali back
//! end (oom-pin, plan #373 cause 6).
//!
//! In plan #327's whole dEQP-GLES2 tree 33 cases ended as ResourceError: a
//! draw returned `-EINVAL` (the frame builder's "varyings, no varying
//! program" check), which `src/mali/backend.rs` reports as
//! `GL_OUT_OF_MEMORY`. The cause was a placeholder vertex-layout row (every
//! lane folded to a constant, `comps == 0`) counted as a varying written;
//! commit b2585cf2 (`StageMeta::of`, src/mali/program.rs; note
//! docs/notes/mali-draw-limits.md) counts only rows with components. This
//! test links GLSL ES 1.00 pairs shaped like those cases through
//! `glLinkProgram`, draws one triangle of each into an 8x8 RGBA8 framebuffer
//! object through `glDrawArrays`, into the real Mali back end over a mock of
//! its OSAL, and asserts that `glGetError` after the draw is never
//! `GL_OUT_OF_MEMORY`. No pixels are checked.
//!
//! A program the online compiler refuses (link failure, or a draw refused as
//! `GL_INVALID_OPERATION`) is allowed, but listed: `COMPILER_REFUSED` holds
//! the labels that do not draw, and every other pair must draw with
//! `GL_NO_ERROR`, so a change in either direction fails the test.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_draw_oom
//! ```

#![cfg(all(feature = "host-test", feature = "shaderc-online", feature = "gpu-mali", not(feature = "report-es2")))]

use core::ffi::{c_char, c_void};
use std::ffi::CString;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use yakogl::backend;
use yakogl::gl::buffer::glBindBuffer;
use yakogl::gl::consts::*;
use yakogl::gl::draw::*;
use yakogl::gl::framebuffer::*;
use yakogl::gl::program::*;
use yakogl::gl::shader::*;
use yakogl::gl::state::*;
use yakogl::gl::types::*;
use yakogl::gl::vertex::{glEnableVertexAttribArray, glVertexAttribPointer};
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

/// Covers the whole target.
static FULL: [f32; 12] = [-1.0, -1.0, 0.0, 1.0, 3.0, -1.0, 0.0, 1.0, -1.0, 3.0, 0.0, 1.0];
/// A second attribute, one vec4 per vertex.
static ATTR: [f32; 12] = [0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9, 0.3, 0.2, 0.1];

#[derive(Debug, PartialEq)]
enum Outcome {
    /// Linked and drawn, `GL_NO_ERROR`.
    Drawn,
    /// The compile or the link failed (the online compiler's refusal).
    LinkFailed,
    /// Linked, the draw refused as `GL_INVALID_OPERATION`.
    DrawRefused,
}

fn compile(ty: GLenum, text: &str) -> Option<GLuint> {
    let text = CString::new(text).unwrap();
    // SAFETY: plain GL calls; one NUL-terminated source.
    unsafe {
        let s = glCreateShader(ty);
        let p: [*const c_char; 1] = [text.as_ptr()];
        glShaderSource(s, 1, p.as_ptr(), core::ptr::null());
        glCompileShader(s);
        let mut ok: GLint = 0;
        glGetShaderiv(s, GL_COMPILE_STATUS, &mut ok);
        (ok == 1).then_some(s)
    }
}

/// Link `vs` + `fs` (`a_pos` at 0, `a_attr` at 1) and draw one triangle.
/// Panics on `GL_OUT_OF_MEMORY` or any other error.
fn run(label: &str, vs: &str, fs: &str) -> Outcome {
    let (Some(v), Some(f)) = (compile(GL_VERTEX_SHADER, vs), compile(GL_FRAGMENT_SHADER, fs)) else {
        return Outcome::LinkFailed;
    };
    let before = backend::draws_refused();
    // SAFETY: plain GL calls; `FULL` and `ATTR` are static and outlive the draw.
    let (linked, e) = unsafe {
        let p = glCreateProgram();
        glAttachShader(p, v);
        glAttachShader(p, f);
        glBindAttribLocation(p, 0, c"a_pos".as_ptr());
        glBindAttribLocation(p, 1, c"a_attr".as_ptr());
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        if ok != 1 {
            (false, GL_NO_ERROR)
        } else {
            glUseProgram(p);
            glBindBuffer(GL_ARRAY_BUFFER, 0);
            glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, FULL.as_ptr() as *const c_void);
            glEnableVertexAttribArray(0);
            glVertexAttribPointer(1, 4, GL_FLOAT, 0, 0, ATTR.as_ptr() as *const c_void);
            glEnableVertexAttribArray(1);
            while glGetError() != GL_NO_ERROR {}
            glDrawArrays(GL_TRIANGLES, 0, 3);
            glFinish();
            (true, glGetError())
        }
    };
    let refused = backend::draws_refused() - before;
    assert_ne!(e, GL_OUT_OF_MEMORY, "{label}: the draw returned GL_OUT_OF_MEMORY (cause 6)");
    if !linked {
        return Outcome::LinkFailed;
    }
    match e {
        GL_NO_ERROR => {
            assert_eq!(refused, 0, "{label}");
            Outcome::Drawn
        }
        GL_INVALID_OPERATION => Outcome::DrawRefused,
        other => panic!("{label}: glGetError 0x{other:x}"),
    }
}

/* ---- the pairs ----------------------------------------------------------- */

/// The fragment shader every pair uses unless it reads more: it outputs the
/// varying `v_c` (vec4).
const FS_C: &str = "precision mediump float;\nvarying vec4 v_c;\nvoid main() { gl_FragColor = v_c; }\n";

/// A vertex shader of `a_pos`, `a_attr` and `body` (which sets `v_c`).
fn vs(decl: &str, body: &str) -> String {
    format!(
        "attribute vec4 a_pos;\nattribute vec4 a_attr;\nvarying vec4 v_c;\n{decl}\n\
         void main() {{\n  gl_Position = a_pos;\n{body}\n}}\n"
    )
}

struct Pair {
    label: &'static str,
    vs: String,
    fs: String,
}

fn pairs() -> Vec<Pair> {
    let mut v = Vec::new();
    let mut add = |label: &'static str, vs: String, fs: &str| v.push(Pair { label, vs, fs: fs.to_string() });

    // Control: a plain varying carried from an attribute (stored row, no
    // placeholder).
    add("control_attribute_varying", vs("", "  v_c = a_attr;"), FS_C);
    // Control: a position only, no varying at all.
    add(
        "control_no_varying",
        "attribute vec4 a_pos;\nvoid main() { gl_Position = a_pos; }\n".to_string(),
        "precision mediump float;\nvoid main() { gl_FragColor = vec4(0.5); }\n",
    );

    // shaders.matrix.{mul,div,add,sub,...}_{mat2,mat3,mat4}..._vertex: the
    // operation on constants in the vertex shader, the result through a
    // varying; every lane folds to a constant (all rows placeholder).
    add(
        "matrix_mul_mat2_const_vertex",
        vs("", "  mat2 m = mat2(0.5, 1.0, 1.5, 2.0) * mat2(1.0, 0.5, 0.25, 2.0);\n  v_c = vec4(m[0], m[1]);"),
        FS_C,
    );
    add(
        "matrix_add_mat3_const_vertex",
        vs("", "  mat3 m = mat3(1.0) + mat3(0.5);\n  v_c = vec4(m[0][0] + m[1][1], m[2][2], m[0][1], 1.0) * 0.25;"),
        FS_C,
    );
    add("matrix_sub_mat4_const_vertex", vs("", "  mat4 m = mat4(2.0) - mat4(0.5);\n  v_c = m[1] + m[2] * 0.25;"), FS_C);
    add(
        "matrix_mul_vec_mat4_const_vertex",
        vs("", "  mat4 m = mat4(0.5);\n  v_c = m * vec4(0.25, 0.5, 0.75, 1.0);"),
        FS_C,
    );
    // The same on uniforms: nothing folds, stored rows.
    add(
        "matrix_mul_mat2_uniform_vertex",
        vs("uniform mat2 u_a;\nuniform mat2 u_b;", "  mat2 m = u_a * u_b;\n  v_c = vec4(m[0], m[1]);"),
        FS_C,
    );
    add(
        "matrix_mul_mat3_uniform_vertex",
        vs(
            "uniform mat3 u_a;\nuniform mat3 u_b;",
            "  mat3 m = u_a * u_b;\n  v_c = vec4(m[0], m[1][2]) + vec4(m[2], 0.0);",
        ),
        FS_C,
    );
    add(
        "matrix_add_mat4_uniform_vertex",
        vs("uniform mat4 u_a;\nuniform mat4 u_b;", "  mat4 m = u_a + u_b;\n  v_c = m[0] + m[3];"),
        FS_C,
    );
    // The same on an attribute: the matrix built from attribute lanes.
    add(
        "matrix_mul_mat2_attribute_vertex",
        vs("", "  mat2 m = mat2(a_attr.xy, a_attr.zw) * mat2(a_attr.wzyx);\n  v_c = vec4(m[0], m[1]);"),
        FS_C,
    );
    add(
        "matrix_mul_mat4_attribute_vertex",
        vs("", "  mat4 m = mat4(a_attr, a_attr.yzwx, a_attr.zwxy, a_attr.wxyz);\n  v_c = (m * m)[1];"),
        FS_C,
    );
    // Constants and attribute mixed: some lanes fold, some do not (a
    // placeholder lane beside stored lanes of one row).
    add(
        "matrix_mixed_const_attribute_vertex",
        vs("", "  mat2 m = mat2(1.0, 0.0, 0.0, 1.0) * mat2(a_attr.x, 0.0, 0.0, a_attr.y);\n  v_c = vec4(m[0], m[1]);"),
        FS_C,
    );
    // Two varyings: one folds to a constant (a placeholder row), the other
    // is stored (a placeholder row beside a stored row).
    add(
        "placeholder_beside_stored_row",
        "attribute vec4 a_pos;\nattribute vec4 a_attr;\nvarying vec4 v_c;\nvarying vec4 v_k;\n\
         void main() {\n  gl_Position = a_pos;\n  mat4 m = mat4(2.0) - mat4(0.5);\n  v_k = m[1] + m[2] * 0.25;\n  v_c = a_attr;\n}\n"
            .to_string(),
        "precision mediump float;\nvarying vec4 v_c;\nvarying vec4 v_k;\nvoid main() { gl_FragColor = v_c * v_k; }\n",
    );
    // Both varyings fold: all rows placeholder.
    add(
        "all_rows_placeholder",
        "attribute vec4 a_pos;\nattribute vec4 a_attr;\nvarying vec4 v_c;\nvarying vec4 v_k;\n\
         void main() {\n  gl_Position = a_pos;\n  v_k = vec4(0.25, 0.5, 0.75, 1.0);\n  v_c = vec4(1.0, 0.5, 0.25, 1.0);\n}\n"
            .to_string(),
        "precision mediump float;\nvarying vec4 v_c;\nvarying vec4 v_k;\n\
         void main() { gl_FragColor = vec4(v_c.x, v_c.y, v_k.z, v_k.w); }\n",
    );

    // shaders.conditionals.*: a varying written on only some paths.
    add(
        "conditionals_varying_one_path",
        vs("uniform float u_s;", "  v_c = vec4(0.0);\n  if (u_s > 0.5) v_c = vec4(1.0, 0.5, 0.25, 1.0);"),
        FS_C,
    );
    add(
        "conditionals_varying_both_paths_const",
        vs(
            "uniform float u_s;",
            "  if (u_s > 0.5) v_c = vec4(1.0, 0.5, 0.25, 1.0);\n  else v_c = vec4(0.25, 0.5, 1.0, 1.0);",
        ),
        FS_C,
    );
    add(
        "conditionals_attribute_select",
        vs("", "  if (a_attr.x > 0.5) v_c = vec4(1.0, 0.0, 0.0, 1.0);\n  else v_c = vec4(0.0, 1.0, 0.0, 1.0);"),
        FS_C,
    );
    // shaders.random.*: more varyings declared than written (never
    // assigned), and varyings assigned constants.
    add(
        "random_declared_not_written",
        "attribute vec4 a_pos;\nattribute vec4 a_attr;\nvarying vec4 v_c;\nvarying vec4 v_u;\nvarying vec2 v_w;\n\
         void main() {\n  gl_Position = a_pos;\n  v_c = a_attr;\n}\n"
            .to_string(),
        "precision mediump float;\nvarying vec4 v_c;\nvarying vec4 v_u;\nvarying vec2 v_w;\n\
         void main() { gl_FragColor = v_c; }\n",
    );
    add(
        "random_declared_unused_in_fragment",
        "attribute vec4 a_pos;\nattribute vec4 a_attr;\nvarying vec4 v_c;\nvarying vec4 v_u;\n\
         void main() {\n  gl_Position = a_pos;\n  v_c = a_attr;\n  v_u = vec4(0.5);\n}\n"
            .to_string(),
        FS_C,
    );
    add(
        "random_constant_varying_through_loop",
        vs("", "  vec4 c = vec4(0.0);\n  for (int i = 0; i < 3; i++) c += vec4(0.25);\n  v_c = c;"),
        FS_C,
    );
    // fragment_ops.interaction (one case): a constant varying with a
    // fragment-side use.
    add(
        "interaction_constant_varying",
        vs("", "  v_c = vec4(0.1, 0.2, 0.3, 1.0);"),
        "precision mediump float;\nvarying vec4 v_c;\nvoid main() { gl_FragColor = v_c * 0.5; }\n",
    );
    v
}

/// The labels of the pairs the online compiler refuses (link failure or a
/// draw refused as `GL_INVALID_OPERATION`). Each is a compiler refusal of
/// the M.2 kind (the compiler plans' to lift), named after the entry:
const COMPILER_REFUSED: &[&str] = &[
    // a fused multiply-add other than a matrix product's
    "matrix_mul_mat2_const_vertex",
    "matrix_mul_vec_mat4_const_vertex",
    "matrix_mul_mat2_attribute_vertex",
    "matrix_mul_mat4_attribute_vertex",
    "matrix_mixed_const_attribute_vertex",
    // a uniform other than a float, vec2, vec3, vec4 or mat4
    "matrix_mul_mat2_uniform_vertex",
    "matrix_mul_mat3_uniform_vertex",
    // two varying lanes of equal value
    "conditionals_varying_one_path",
    // a varying lane computed from no attribute
    "conditionals_attribute_select",
    // a varying lane left unwritten
    "random_declared_not_written",
    // a loop in a vertex shader
    "random_constant_varying_through_loop",
];

#[test]
fn cause6_shapes_never_draw_out_of_memory() {
    setup();
    let mut bad = Vec::new();
    for p in pairs() {
        let o = run(p.label, &p.vs, &p.fs);
        let refused = o != Outcome::Drawn;
        eprintln!("{}: {o:?}", p.label);
        if refused != COMPILER_REFUSED.contains(&p.label) {
            bad.push(format!("{} -> {o:?}", p.label));
        }
    }
    assert!(bad.is_empty(), "outcome differs from COMPILER_REFUSED: {bad:#?}");
    // A control draws.
    assert_eq!(run("control", &pairs()[0].vs, &pairs()[0].fs), Outcome::Drawn);
}
