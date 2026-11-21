// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Which blend states the Mali back end draws and which it refuses, through
//! the GL API on the host (blend-states, plan #373 cause 3).
//!
//! dEQP-GLES2.functional.fragment_ops.blend draws a blended quad into an
//! RGBA8 surface for three state families: `equation_src_func_dst_func` (the
//! three equations times every source and destination factor, the constant
//! colour and constant alpha factors among them), `rgb_func_alpha_func`
//! (separate factors for colour and alpha) and `rgb_equation_alpha_equation`
//! (separate equations). This test draws one blended triangle for each state
//! of those families into an 8x8 RGBA8 framebuffer object, through
//! `glDrawArrays`, into the real Mali back end over a mock of its OSAL
//! (the board's GPU answers, buffers at VAs far from any CPU address, every
//! submit completing). A draw the Mali refuses reaches the GL core as
//! `-ENOTSUP`, which sets `GL_INVALID_OPERATION` (`submit_draw`,
//! src/gl/draw.rs) and counts one in `draws_refused`; a taken draw leaves
//! `GL_NO_ERROR`. No pixels are checked: the Mali's own frames are compared
//! with Mesa's in `src/mali/frame_blend_tests.rs`.
//!
//! Since the blend shaders of any equation into RGBA8 (961cbf77) no state of
//! the three families is refused: the lists `REFUSED_*` below are empty. The
//! test fails when a state is refused; a change that makes one refuse again
//! pins it there (a failing `check` prints the list's form), and a later
//! refusal must be a named one. The control draws (no blend, ONE/ZERO) are
//! taken too.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_blend_states
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
        assert_eq!(ok, 1, "{text:?} did not compile");
        s
    }
}

/// The GL context, an 8x8 RGBA8 framebuffer object bound, and the program.
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
        let v = shader(GL_VERTEX_SHADER, "#version 300 es\nin vec4 p;\nvoid main() { gl_Position = p; }\n");
        let f = shader(
            GL_FRAGMENT_SHADER,
            "#version 300 es\nprecision highp float;\nlayout(location = 0) out vec4 o;\n\
             void main() { o = vec4(0.25, 0.5, 0.75, 0.6); }\n",
        );
        let p = glCreateProgram();
        glAttachShader(p, v);
        glAttachShader(p, f);
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        assert_eq!(ok, 1);
        glUseProgram(p);
        glClearColor(0.5, 0.25, 0.75, 0.4);
        glClear(GL_COLOR_BUFFER_BIT);
        glEnable(GL_BLEND);
        glBlendColor(0.3, 0.6, 0.9, 0.45);
        while glGetError() != GL_NO_ERROR {}
    }
}

/// Covers the whole target.
static FULL: [f32; 12] = [-1.0, -1.0, 0.0, 1.0, 3.0, -1.0, 0.0, 1.0, -1.0, 3.0, 0.0, 1.0];

/// Draw one triangle under the current blend state; true when it was drawn
/// (`GL_NO_ERROR`), false when the Mali refused it (`GL_INVALID_OPERATION`,
/// one more in `draws_refused`).
fn drawn() -> bool {
    let before = backend::draws_refused();
    // SAFETY: `FULL` is static and outlives the draw.
    let e = unsafe {
        glBindBuffer(GL_ARRAY_BUFFER, 0);
        glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, FULL.as_ptr() as *const c_void);
        glEnableVertexAttribArray(0);
        glDrawArrays(GL_TRIANGLES, 0, 3);
        glFinish();
        glGetError()
    };
    let refused = backend::draws_refused() - before;
    match e {
        GL_NO_ERROR => {
            assert_eq!(refused, 0);
            true
        }
        GL_INVALID_OPERATION => {
            assert_eq!(refused, 1);
            false
        }
        other => panic!("glGetError 0x{other:x}"),
    }
}

/* ---- the state space of the dEQP groups --------------------------------- */

const EQUATIONS: [(&str, GLenum); 3] =
    [("add", GL_FUNC_ADD), ("subtract", GL_FUNC_SUBTRACT), ("reverse_subtract", GL_FUNC_REVERSE_SUBTRACT)];

/// dEQP's destination factors; the source factors add `src_alpha_saturate`.
const FUNCS: [(&str, GLenum); 14] = [
    ("zero", GL_ZERO),
    ("one", GL_ONE),
    ("src_color", GL_SRC_COLOR),
    ("one_minus_src_color", GL_ONE_MINUS_SRC_COLOR),
    ("dst_color", GL_DST_COLOR),
    ("one_minus_dst_color", GL_ONE_MINUS_DST_COLOR),
    ("src_alpha", GL_SRC_ALPHA),
    ("one_minus_src_alpha", GL_ONE_MINUS_SRC_ALPHA),
    ("dst_alpha", GL_DST_ALPHA),
    ("one_minus_dst_alpha", GL_ONE_MINUS_DST_ALPHA),
    ("constant_color", GL_CONSTANT_COLOR),
    ("one_minus_constant_color", GL_ONE_MINUS_CONSTANT_COLOR),
    ("constant_alpha", GL_CONSTANT_ALPHA),
    ("one_minus_constant_alpha", GL_ONE_MINUS_CONSTANT_ALPHA),
];
const SATURATE: (&str, GLenum) = ("src_alpha_saturate", GL_SRC_ALPHA_SATURATE);

fn src_funcs() -> Vec<(&'static str, GLenum)> {
    let mut v = FUNCS.to_vec();
    v.push(SATURATE);
    v
}

/// `equation_src_func_dst_func`: one equation on colour and alpha, a source
/// and a destination factor on both (630 states). Labelled
/// `<equation>_<src>:<dst>` as dEQP names the cases.
fn family_equation_src_dst() -> Vec<String> {
    let mut refused = Vec::new();
    for (en, e) in EQUATIONS {
        for (sn, s) in src_funcs() {
            for (dn, d) in FUNCS {
                // SAFETY: plain GL calls.
                unsafe {
                    glBlendEquation(e);
                    glBlendFunc(s, d);
                }
                if !drawn() {
                    refused.push(format!("{en}_{sn}:{dn}"));
                }
            }
        }
    }
    refused
}

/// `rgb_func_alpha_func`: the add equation, factors that differ between
/// colour and alpha. dEQP's `src` and `dst` groups vary the source and the
/// destination pair; here every (src, dst) pair of the colour channel is
/// drawn against each alpha pair of `ALPHA_PAIRS`, labelled
/// `<srcRGB>:<dstRGB>/<srcA>_<dstA>`.
const ALPHA_PAIRS: [(&str, GLenum, &str, GLenum); 6] = [
    ("zero", GL_ZERO, "one", GL_ONE),
    ("one", GL_ONE, "zero", GL_ZERO),
    ("src_alpha", GL_SRC_ALPHA, "one_minus_src_alpha", GL_ONE_MINUS_SRC_ALPHA),
    ("dst_alpha", GL_DST_ALPHA, "one_minus_dst_alpha", GL_ONE_MINUS_DST_ALPHA),
    ("constant_alpha", GL_CONSTANT_ALPHA, "one_minus_constant_alpha", GL_ONE_MINUS_CONSTANT_ALPHA),
    ("src_alpha_saturate", GL_SRC_ALPHA_SATURATE, "dst_color", GL_DST_COLOR),
];

fn family_rgb_func_alpha_func() -> Vec<String> {
    let mut refused = Vec::new();
    // SAFETY: plain GL call.
    unsafe { glBlendEquation(GL_FUNC_ADD) };
    for (sn, s) in src_funcs() {
        for (dn, d) in FUNCS {
            for (san, sa, dan, da) in ALPHA_PAIRS {
                // SAFETY: plain GL call.
                unsafe { glBlendFuncSeparate(s, d, sa, da) };
                if !drawn() {
                    refused.push(format!("{sn}:{dn}/{san}_{dan}"));
                }
            }
        }
    }
    refused
}

/// `rgb_equation_alpha_equation`: every pair of equations, factors ONE and
/// ONE (the factors dEQP's group uses are not equation-dependent).
fn family_rgb_equation_alpha_equation() -> Vec<String> {
    let mut refused = Vec::new();
    // SAFETY: plain GL call.
    unsafe { glBlendFunc(GL_ONE, GL_ONE) };
    for (rn, r) in EQUATIONS {
        for (an, a) in EQUATIONS {
            // SAFETY: plain GL call.
            unsafe { glBlendEquationSeparate(r, a) };
            if !drawn() {
                refused.push(format!("{rn}:{an}"));
            }
        }
    }
    refused
}

/* ---- refusals ---------------------------------------------------- */

/// One line a source factor (or, for the separate-factor family, a colour
/// source factor): `<prefix>: <item> <item> ...`; each pair is a state
/// `<prefix>:<item>` of the family's labels.
fn expand(lines: &str) -> Vec<String> {
    let mut v = Vec::new();
    for line in lines.lines().filter(|l| !l.trim().is_empty()) {
        let (prefix, items) = line.split_once(':').expect("a prefix and its items");
        v.extend(items.split_whitespace().map(|i| format!("{}:{i}", prefix.trim())));
    }
    v
}

/// The lines of `refused` in `expand`'s form: how to refresh the lists below
/// when a change builds more blend shaders (a failing `check` prints it).
fn lines(refused: &[String]) -> String {
    let mut out: Vec<(String, Vec<String>)> = Vec::new();
    for r in refused {
        let (prefix, item) = r.split_once(':').unwrap();
        match out.last_mut() {
            Some((p, items)) if p == prefix => items.push(item.to_string()),
            _ => out.push((prefix.to_string(), vec![item.to_string()])),
        }
    }
    out.iter().map(|(p, i)| format!("{p}: {}\n", i.join(" "))).collect()
}

const REFUSED_EQUATION_SRC_DST: &str = "";
const REFUSED_RGB_FUNC_ALPHA_FUNC: &str = "";
const REFUSED_RGB_EQUATION_ALPHA_EQUATION: &str = "";

fn check(family: &str, got: Vec<String>, pinned: &str) {
    let pinned = expand(pinned);
    let unexpected: Vec<&String> = got.iter().filter(|g| !pinned.contains(g)).collect();
    let now_drawn: Vec<&String> = pinned.iter().filter(|p| !got.contains(p)).collect();
    assert!(
        unexpected.is_empty() && now_drawn.is_empty(),
        "{family}: refused {} states; refused but not listed: {unexpected:?}; listed but drawn: {now_drawn:?}\n\
         the refused states now, in the list's form:\n{}",
        got.len(),
        lines(&got)
    );
}

#[test]
fn blend_states() {
    setup();
    // SAFETY: plain GL calls.
    unsafe {
        /* controls: no blend and an opaque blend reach no blend shader */
        glDisable(GL_BLEND);
        assert!(drawn(), "an unblended draw");
        glEnable(GL_BLEND);
        glBlendEquation(GL_FUNC_ADD);
        glBlendFunc(GL_ONE, GL_ZERO);
        assert!(drawn(), "ONE, ZERO");
    }
    let a = family_equation_src_dst();
    let b = family_rgb_func_alpha_func();
    let c = family_rgb_equation_alpha_equation();
    assert_eq!(backend::draws_refused(), 0, "no draw of the process was refused");
    check("equation_src_func_dst_func", a, REFUSED_EQUATION_SRC_DST);
    check("rgb_func_alpha_func", b, REFUSED_RGB_FUNC_ALPHA_FUNC);
    check("rgb_equation_alpha_equation", c, REFUSED_RGB_EQUATION_ALPHA_EQUATION);
}
