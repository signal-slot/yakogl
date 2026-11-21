// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The `fragment_ops.random` states of tests/mali_fragment_random.rs drawn in
//! the `report-es2` build, the build the NanoPC-T6 board session runs
//! (es2pin-fragrandom, plan #373).
//!
//! tests/mali_fragment_random.rs is gated `not(report-es2)`; this file
//! replays the same state set (a copy of its generator and draws) with
//! `report-es2` on, so the board's build is pinned too. Every draw must leave
//! `GL_NO_ERROR` and `draws_refused` unchanged. A refused draw reaches the GL
//! core as `-ENOTSUP` and fails the test with the case, the draw and the
//! state. The shaders are GLSL ES 1.00 and the framebuffer is the dEQP
//! GLES2 one's RGBA8 + separate DEPTH_COMPONENT16 + STENCIL_INDEX8 pair
//! (complete only in this build). No pixels and no descriptor words are checked.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table,report-es2 --test mali_fragment_random_es2
//! ```

#![cfg(all(feature = "host-test", feature = "shaderc-online", feature = "gpu-mali", feature = "report-es2"))]
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

fn renderbuffer(format: GLenum, attachment: GLenum) {
    // SAFETY: plain GL calls.
    unsafe {
        let mut rb = 0;
        glGenRenderbuffers(1, &mut rb);
        glBindRenderbuffer(GL_RENDERBUFFER, rb);
        glRenderbufferStorage(GL_RENDERBUFFER, format, 8, 8);
        glFramebufferRenderbuffer(GL_FRAMEBUFFER, attachment, GL_RENDERBUFFER, rb);
    }
}

/// The GL context, an 8x8 RGBA8 + packed depth/stencil framebuffer object
/// bound, and the program.
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
        let mut fb = 0;
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        renderbuffer(GL_RGBA8, GL_COLOR_ATTACHMENT0);
        renderbuffer(GL_DEPTH_COMPONENT16, GL_DEPTH_ATTACHMENT);
        renderbuffer(GL_STENCIL_INDEX8, GL_STENCIL_ATTACHMENT);
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE);
        let v = shader(GL_VERTEX_SHADER, "attribute vec4 p;\nvoid main() { gl_Position = p; }\n");
        let f = shader(
            GL_FRAGMENT_SHADER,
            "precision mediump float;\n\
             void main() { gl_FragColor = vec4(0.25, 0.5, 0.75, 0.6); }\n",
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
        glClearDepthf(1.0);
        glClearStencil(0);
        glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT | GL_STENCIL_BUFFER_BIT);
        while glGetError() != GL_NO_ERROR {}
    }
}

/// An xorshift128 generator (the family of de::Random), seeded by the case.
struct Rng {
    x: u32,
    y: u32,
    z: u32,
    w: u32,
}

impl Rng {
    fn new(seed: u32) -> Self {
        Rng {
            x: 123456789 ^ seed,
            y: 362436069 ^ seed.wrapping_mul(3),
            z: 521288629 ^ seed.wrapping_mul(7),
            w: 88675123 ^ seed.wrapping_mul(13),
        }
    }
    fn next(&mut self) -> u32 {
        let t = self.x ^ (self.x << 11);
        self.x = self.y;
        self.y = self.z;
        self.z = self.w;
        self.w = self.w ^ (self.w >> 19) ^ t ^ (t >> 8);
        self.w
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u32) as usize
    }
    fn coin(&mut self) -> bool {
        self.next() & 1 != 0
    }
    fn pick(&mut self, v: &[GLenum]) -> GLenum {
        v[self.below(v.len())]
    }
}

const EQUATIONS: [GLenum; 3] = [GL_FUNC_ADD, GL_FUNC_SUBTRACT, GL_FUNC_REVERSE_SUBTRACT];
const DST_FUNCS: [GLenum; 14] = [
    GL_ZERO,
    GL_ONE,
    GL_SRC_COLOR,
    GL_ONE_MINUS_SRC_COLOR,
    GL_DST_COLOR,
    GL_ONE_MINUS_DST_COLOR,
    GL_SRC_ALPHA,
    GL_ONE_MINUS_SRC_ALPHA,
    GL_DST_ALPHA,
    GL_ONE_MINUS_DST_ALPHA,
    GL_CONSTANT_COLOR,
    GL_ONE_MINUS_CONSTANT_COLOR,
    GL_CONSTANT_ALPHA,
    GL_ONE_MINUS_CONSTANT_ALPHA,
];
const COMPARES: [GLenum; 8] = [GL_NEVER, GL_LESS, GL_LEQUAL, GL_EQUAL, GL_GEQUAL, GL_GREATER, GL_NOTEQUAL, GL_ALWAYS];
const STENCIL_OPS: [GLenum; 8] =
    [GL_KEEP, GL_ZERO, GL_REPLACE, GL_INCR, GL_DECR, GL_INVERT, GL_INCR_WRAP, GL_DECR_WRAP];

/// A source factor: the destination factors and SRC_ALPHA_SATURATE.
fn src_func(r: &mut Rng) -> GLenum {
    let k = r.below(DST_FUNCS.len() + 1);
    if k == DST_FUNCS.len() {
        GL_SRC_ALPHA_SATURATE
    } else {
        DST_FUNCS[k]
    }
}

/// Apply one random draw's state; returns its description.
fn random_state(r: &mut Rng) -> String {
    let mut d = String::new();
    // SAFETY: plain GL calls.
    unsafe {
        if r.coin() {
            let (x, y) = (r.below(6) as GLint, r.below(6) as GLint);
            let (w, h) = (1 + r.below(8) as GLsizei, 1 + r.below(8) as GLsizei);
            glEnable(GL_SCISSOR_TEST);
            glScissor(x, y, w, h);
            d += &format!("scissor({x},{y},{w},{h}) ");
        } else {
            glDisable(GL_SCISSOR_TEST);
        }
        if r.coin() {
            glEnable(GL_BLEND);
            let mut c = [0.0f32; 4];
            for v in &mut c {
                *v = r.below(256) as f32 / 255.0;
            }
            glBlendColor(c[0], c[1], c[2], c[3]);
            let (er, ea) = (r.pick(&EQUATIONS), r.pick(&EQUATIONS));
            let (sr, dr) = (src_func(r), r.pick(&DST_FUNCS));
            let (sa, da) = (src_func(r), r.pick(&DST_FUNCS));
            if r.coin() {
                glBlendEquation(er);
                glBlendFunc(sr, dr);
                d += &format!("blend(eq 0x{er:x}, 0x{sr:x}:0x{dr:x}) ");
            } else {
                glBlendEquationSeparate(er, ea);
                glBlendFuncSeparate(sr, dr, sa, da);
                d += &format!("blend(eq 0x{er:x}/0x{ea:x}, 0x{sr:x}:0x{dr:x}/0x{sa:x}:0x{da:x}) ");
            }
        } else {
            glDisable(GL_BLEND);
        }
        if r.coin() {
            glEnable(GL_STENCIL_TEST);
            for face in [GL_FRONT, GL_BACK] {
                let (f, rf, m) = (r.pick(&COMPARES), r.below(256) as GLint, r.below(256) as GLuint);
                glStencilFuncSeparate(face, f, rf, m);
                let (a, b, c) = (r.pick(&STENCIL_OPS), r.pick(&STENCIL_OPS), r.pick(&STENCIL_OPS));
                glStencilOpSeparate(face, a, b, c);
                let wm = r.below(256) as GLuint;
                glStencilMaskSeparate(face, wm);
                d += &format!("stencil(0x{face:x}: 0x{f:x} {rf} 0x{m:x} ops 0x{a:x} 0x{b:x} 0x{c:x} mask 0x{wm:x}) ");
            }
        } else {
            glDisable(GL_STENCIL_TEST);
        }
        if r.coin() {
            glEnable(GL_DEPTH_TEST);
            let f = r.pick(&COMPARES);
            glDepthFunc(f);
            d += &format!("depth(0x{f:x}) ");
        } else {
            glDisable(GL_DEPTH_TEST);
        }
        glDepthMask(r.coin() as GLboolean);
        let m = [r.coin(), r.coin(), r.coin(), r.coin()];
        glColorMask(m[0] as GLboolean, m[1] as GLboolean, m[2] as GLboolean, m[3] as GLboolean);
        d += &format!("colormask({m:?}) ");
        if r.coin() {
            glEnable(GL_DITHER);
        } else {
            glDisable(GL_DITHER);
        }
    }
    d
}

/// A triangle covering the target at depth `z`.
fn tri(z: f32) -> [f32; 12] {
    [-1.0, -1.0, z, 1.0, 3.0, -1.0, z, 1.0, -1.0, 3.0, z, 1.0]
}

/// Draw one triangle under the current state; true when it was drawn
/// (`GL_NO_ERROR`), false when the Mali refused it (`GL_INVALID_OPERATION`,
/// one more in `draws_refused`).
fn drawn(tri: &[f32; 12]) -> bool {
    let before = backend::draws_refused();
    // SAFETY: `tri` outlives the draw (`glFinish` below).
    let e = unsafe {
        glBindBuffer(GL_ARRAY_BUFFER, 0);
        glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, tri.as_ptr() as *const c_void);
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

const CASES: u32 = 100;
const DRAWS: usize = 4;

/// Draws still refused, `(case, draw)`; none.
const EXPECTED_REFUSED: [(u32, usize); 0] = [];

#[test]
fn fragment_random() {
    setup();
    let mut refused = Vec::new();
    for case in 0..CASES {
        let mut r = Rng::new(case);
        for draw in 0..DRAWS {
            let desc = random_state(&mut r);
            let z = r.below(201) as f32 / 100.0 - 1.0;
            if !drawn(&tri(z)) {
                refused.push((case, draw, desc));
            }
        }
    }
    let unexpected: Vec<_> = refused.iter().filter(|(c, d, _)| !EXPECTED_REFUSED.contains(&(*c, *d))).collect();
    let now_drawn: Vec<_> =
        EXPECTED_REFUSED.iter().filter(|(c, d)| !refused.iter().any(|(rc, rd, _)| rc == c && rd == d)).collect();
    assert!(
        unexpected.is_empty() && now_drawn.is_empty(),
        "refused draws (case, draw, state): {unexpected:#?}\nlisted but drawn: {now_drawn:?}"
    );
}
