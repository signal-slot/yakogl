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
//! Sources. `shaders.random.basic_expression.vertex.96` is replayed from the
//! text `src/shaderc/online/mali/tests.rs`
//! (`a_late_fold_is_refused_only_where_the_attribute_numbering_depends_on_it`)
//! keeps as dEQP's (`1405_0`). `fragment_ops.interaction.basic_shader.{11,66}`
//! are ported from VK-GL-CTS 93bca01 (tools/cts/deps.toml; no CTS file is in
//! the tree): `modules/gles2/functional/es2fFragOpInteractionTests.cpp` (seed
//! `0x667eacfd ^ deInt32Hash(case)`, the ES 2.0 random-shader parameters),
//! `modules/glshared/glsFragOpInteractionCase.cpp` (`init`, `iterate`,
//! `setRenderState`, `renderQuad`), `modules/glshared/glsInteractionTestUtil.cpp`
//! (`computeRandomRenderState`) and the random shader generator of
//! `framework/randomshaders/` (`rsgProgramGenerator.cpp`). The shaders and
//! the five iterations of five render commands were produced by building
//! that generator and the two functions above on the host (deRandom,
//! deInt32Hash and the generator unchanged) and printing what `iterate`
//! would hand to GL; the tables below are that output. The other row-8 cases
//! of the board slices (`shaders.random.basic_expression.vertex.{53,85,90,95}`,
//! `scalar_conversion.vertex.53`, `texture.vertex.19`; item
//! `inval-generator`) are ported the same way from
//! `modules/gles2/functional/es2fRandomShaderTests.cpp` (the group's
//! `ProgramParameters`, seed = the case number, vertex-only) and
//! `modules/glshared/glsRandomShaderCase.cpp` (`init`, `iterate`): the same
//! generator built on the host printed each case's two shaders, its inputs'
//! value ranges, its uniform values (`computeUnifiedUniforms`,
//! `computeUniformValues`) and its viewport; `vertex.96`'s shader as that
//! build printed it equals the text above, a check of the build. The 256x256
//! target is assumed as for the interaction cases (only the viewport origin
//! depends on it).
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

/* ---- fragment_ops.interaction.basic_shader.{11,66} ---------------------- */

/// One stencil face of dEQP's `StencilState`.
struct St {
    func: GLenum,
    reference: GLint,
    compare_mask: GLuint,
    fail: GLenum,
    depth_fail: GLenum,
    pass: GLenum,
    write_mask: GLuint,
}

/// The blend equation, source and destination function of the RGB and alpha
/// parts, and the blend colour.
struct Blend {
    rgb: [GLenum; 3],
    alpha: [GLenum; 3],
    color: [f32; 4],
}

/// One `RenderCommand` of `FragOpInteractionCase::iterate`: the quad's corners
/// (`posA`, `posB` in viewport coordinates), the depth its four vertices
/// carry, and the `RenderState` (`None` where the feature is disabled).
struct Cmd {
    depth: f32,
    quad: [i32; 4],
    scissor: Option<[i32; 4]>,
    /// Back face, then front face (`rr::FACETYPE_BACK`, `FACETYPE_FRONT`).
    stencil: Option<[St; 2]>,
    /// Function and write mask.
    depth_test: Option<(GLenum, bool)>,
    blend: Option<Blend>,
    dither: bool,
    mask: [bool; 4],
}

/// The render target of the board run was 256x256 and the case's viewport
/// 64x64 (`FragOpInteractionCase::init`); `sglr::GLContext` adds the
/// viewport's origin (`INTERACTION_ORIGIN`, from `rnd.getInt(0, 256 - 64)`
/// twice, seed `^ 0x232faac`) to every viewport and scissor rectangle.
const INTERACTION_TARGET: GLsizei = 256;
const INTERACTION_VIEWPORT: GLsizei = 64;

/// The vertex shader of both cases, as the random shader generator wrote it.
const INTERACTION_VS: &str = "attribute vec4 dEQP_Position;\n\nvoid main()\n{\n\tgl_Position = dEQP_Position;\n}\n";

/// Case 11's fragment shader.
const INTERACTION_11_FS: &str = "precision mediump float;\nconst float a = (0.5);\n\n\
    void main()\n{\n\tgl_FragColor = vec4(-1, a, 5.0, 2.5) - vec4(-2, a, 5.0, 2.5);\n}\n";

/// Case 66's fragment shader.
const INTERACTION_66_FS: &str =
    "precision mediump float;\nvec4 c = vec4(-4.5, -4.5, 3.25, 8.5) + vec4(5.5, 5, -2.5, -8.0);\n\
    int a = int(6) * int(1);\nint b = (int(3.25));\n\nvoid main()\n{\n\tgl_FragColor = c;\n\
    \tif (int(b) <= a)\n\t\tgl_FragColor = (gl_FragColor);\n}\n";

/// Case 11's five iterations of five render commands (seed `0x667eacfd ^ deInt32Hash(11)` = 0x0d517a3b).
#[rustfmt::skip]
static INTERACTION_11: [[Cmd; 5]; 5] = [
    [
        Cmd { depth: 0.5, quad: [61, 0, 3, 55], scissor: None, stencil: None, depth_test: Some((0x201, false)), blend: Some(Blend { rgb: [0x8006, 0x306, 0x307], alpha: [0x8006, 0x1, 0x304], color: [0.2, 0.6, 0.6, 0.6] }), dither: false, mask: [false, true, true, true] },
        Cmd { depth: 0.4231725, quad: [63, 57, 6, 5], scissor: None, stencil: None, depth_test: None, blend: Some(Blend { rgb: [0x800b, 0x304, 0x8003], alpha: [0x800b, 0x304, 0x8003], color: [0.8, 0.0, 0.5, 0.0] }), dither: true, mask: [false, false, true, false] },
        Cmd { depth: 0.446345, quad: [53, 13, 3, 60], scissor: None, stencil: Some([St { func: 0x206, reference: 8, compare_mask: 0x1afbb7a9, fail: 0x1e03, depth_fail: 0x1e02, pass: 0x150a, write_mask: 0xf75185eb }, St { func: 0x201, reference: 15, compare_mask: 0x8bc5251d, fail: 0x0, depth_fail: 0x8507, pass: 0x1e00, write_mask: 0xaea8b921 }]), depth_test: None, blend: Some(Blend { rgb: [0x800b, 0x302, 0x305], alpha: [0x8006, 0x301, 0x300], color: [0.4, 1.0, 1.0, 0.0] }), dither: true, mask: [false, false, false, false] },
        Cmd { depth: 0.5695175, quad: [44, 61, 7, 10], scissor: Some([4, 11, 67, 61]), stencil: None, depth_test: Some((0x206, true)), blend: None, dither: false, mask: [true, true, true, true] },
        Cmd { depth: 0.60269, quad: [0, 63, 43, 2], scissor: Some([-5, -9, 62, 72]), stencil: Some([St { func: 0x205, reference: 66, compare_mask: 0x463072e3, fail: 0x8507, depth_fail: 0x8508, pass: 0x8508, write_mask: 0x4e536fce }, St { func: 0x201, reference: 182, compare_mask: 0xd6954aa2, fail: 0x0, depth_fail: 0x1e02, pass: 0x150a, write_mask: 0xc35ee55e }]), depth_test: Some((0x207, true)), blend: Some(Blend { rgb: [0x8006, 0x300, 0x304], alpha: [0x800b, 0x308, 0x306], color: [0.0, 0.0, 0.2, 0.5] }), dither: false, mask: [true, true, true, true] },
    ],
    [
        Cmd { depth: 0.4, quad: [0, 60, 34, 18], scissor: None, stencil: Some([St { func: 0x202, reference: 248, compare_mask: 0x332f1b76, fail: 0x150a, depth_fail: 0x150a, pass: 0x1e03, write_mask: 0x73bdddab }, St { func: 0x202, reference: 111, compare_mask: 0x3de212dd, fail: 0x1e01, depth_fail: 0x1e01, pass: 0x150a, write_mask: 0x75ec9be3 }]), depth_test: None, blend: None, dither: true, mask: [true, true, true, true] },
        Cmd { depth: 0.5331725, quad: [61, 2, 25, 62], scissor: None, stencil: None, depth_test: None, blend: None, dither: true, mask: [true, true, false, true] },
        Cmd { depth: 0.996345, quad: [38, 60, 2, 1], scissor: Some([-8, 3, 67, 48]), stencil: None, depth_test: None, blend: None, dither: true, mask: [true, true, true, false] },
        Cmd { depth: 0.4695175, quad: [1, 61, 59, 11], scissor: None, stencil: None, depth_test: Some((0x200, true)), blend: None, dither: true, mask: [false, false, true, true] },
        Cmd { depth: 0.60269, quad: [0, 5, 58, 61], scissor: None, stencil: Some([St { func: 0x200, reference: 222, compare_mask: 0xcd2523ca, fail: 0x1e03, depth_fail: 0x1e01, pass: 0x8507, write_mask: 0xfadb068f }, St { func: 0x200, reference: 47, compare_mask: 0x72415bdd, fail: 0x1e01, depth_fail: 0x150a, pass: 0x8508, write_mask: 0x78a80c46 }]), depth_test: None, blend: None, dither: true, mask: [true, false, true, true] },
    ],
    [
        Cmd { depth: 0.95, quad: [3, 1, 44, 51], scissor: Some([12, -8, 50, 65]), stencil: None, depth_test: None, blend: None, dither: false, mask: [true, true, false, false] },
        Cmd { depth: 0.9731725, quad: [63, 56, 0, 12], scissor: Some([-5, 21, 66, 53]), stencil: Some([St { func: 0x205, reference: 260, compare_mask: 0x4da2b665, fail: 0x8507, depth_fail: 0x8508, pass: 0x8508, write_mask: 0xa811c1e4 }, St { func: 0x206, reference: 53, compare_mask: 0xbc6834ab, fail: 0x150a, depth_fail: 0x1e01, pass: 0x8507, write_mask: 0x9f3d06bc }]), depth_test: None, blend: None, dither: true, mask: [true, true, true, true] },
        Cmd { depth: 0.646345, quad: [0, 2, 53, 63], scissor: Some([-6, -9, 79, 69]), stencil: None, depth_test: None, blend: None, dither: true, mask: [true, true, false, true] },
        Cmd { depth: 0.0695175, quad: [1, 61, 42, 1], scissor: None, stencil: None, depth_test: None, blend: None, dither: true, mask: [true, true, true, false] },
        Cmd { depth: 0.60269, quad: [10, 63, 60, 9], scissor: Some([21, 11, 47, 59]), stencil: None, depth_test: Some((0x205, false)), blend: Some(Blend { rgb: [0x800b, 0x307, 0x302], alpha: [0x800a, 0x303, 0x8001], color: [0.2, 1.0, 0.0, 0.0] }), dither: false, mask: [true, true, true, true] },
    ],
    [
        Cmd { depth: 0.8, quad: [63, 59, 5, 0], scissor: Some([-9, -8, 80, 70]), stencil: Some([St { func: 0x204, reference: 39, compare_mask: 0x8fcee2b2, fail: 0x8508, depth_fail: 0x0, pass: 0x1e02, write_mask: 0xbe2257fc }, St { func: 0x204, reference: 246, compare_mask: 0xe69680e4, fail: 0x0, depth_fail: 0x150a, pass: 0x1e02, write_mask: 0xa1814285 }]), depth_test: Some((0x203, true)), blend: Some(Blend { rgb: [0x8006, 0x300, 0x302], alpha: [0x800a, 0x308, 0x300], color: [0.4, 0.4, 0.4, 0.6] }), dither: true, mask: [false, true, true, true] },
        Cmd { depth: 0.2231725, quad: [62, 16, 3, 60], scissor: None, stencil: None, depth_test: None, blend: Some(Blend { rgb: [0x800a, 0x8001, 0x304], alpha: [0x800a, 0x1, 0x8004], color: [0.0, 0.8, 0.6, 0.5] }), dither: false, mask: [false, false, true, true] },
        Cmd { depth: 0.996345, quad: [47, 9, 4, 46], scissor: None, stencil: Some([St { func: 0x205, reference: 70, compare_mask: 0xa6a3de8d, fail: 0x1e03, depth_fail: 0x150a, pass: 0x1e01, write_mask: 0x196d34f2 }, St { func: 0x201, reference: 176, compare_mask: 0x7172d33f, fail: 0x1e02, depth_fail: 0x1e03, pass: 0x1e00, write_mask: 0x2ccb6fc2 }]), depth_test: None, blend: None, dither: false, mask: [true, false, false, true] },
        Cmd { depth: 0.4695175, quad: [6, 4, 55, 62], scissor: None, stencil: Some([St { func: 0x204, reference: 143, compare_mask: 0x110f8159, fail: 0x0, depth_fail: 0x1e01, pass: 0x1e03, write_mask: 0xbb8d7cef }, St { func: 0x204, reference: 123, compare_mask: 0x827ce7, fail: 0x8508, depth_fail: 0x150a, pass: 0x1e01, write_mask: 0x15c7bada }]), depth_test: None, blend: Some(Blend { rgb: [0x800b, 0x306, 0x8001], alpha: [0x800a, 0x307, 0x8004], color: [1.0, 0.4, 0.5, 0.4] }), dither: true, mask: [true, true, false, false] },
        Cmd { depth: 0.49269, quad: [15, 0, 62, 50], scissor: Some([-5, -5, 79, 79]), stencil: None, depth_test: Some((0x206, true)), blend: None, dither: false, mask: [true, true, true, false] },
    ],
    [
        Cmd { depth: 0.6, quad: [55, 25, 3, 60], scissor: None, stencil: Some([St { func: 0x207, reference: 143, compare_mask: 0xd4e23d52, fail: 0x1e00, depth_fail: 0x1e00, pass: 0x1e02, write_mask: 0xd6230cde }, St { func: 0x207, reference: 70, compare_mask: 0x69215633, fail: 0x1e02, depth_fail: 0x8507, pass: 0x1e00, write_mask: 0x8f823b51 }]), depth_test: None, blend: None, dither: false, mask: [true, true, true, true] },
        Cmd { depth: 0.2231725, quad: [50, 48, 14, 4], scissor: Some([10, 9, 46, 65]), stencil: Some([St { func: 0x207, reference: 186, compare_mask: 0xab11e1ee, fail: 0x1e02, depth_fail: 0x8508, pass: 0x150a, write_mask: 0x587d9b82 }, St { func: 0x201, reference: 158, compare_mask: 0xb279a771, fail: 0x0, depth_fail: 0x8508, pass: 0x150a, write_mask: 0x736d8529 }]), depth_test: Some((0x205, true)), blend: Some(Blend { rgb: [0x8006, 0x304, 0x8002], alpha: [0x800b, 0x306, 0x302], color: [0.6, 0.6, 0.8, 0.8] }), dither: false, mask: [true, true, true, true] },
        Cmd { depth: 0.846345, quad: [55, 61, 5, 2], scissor: Some([10, -10, 59, 84]), stencil: None, depth_test: Some((0x207, false)), blend: None, dither: true, mask: [true, false, true, false] },
        Cmd { depth: 0.0695175, quad: [10, 15, 48, 58], scissor: Some([13, -7, 45, 79]), stencil: None, depth_test: Some((0x206, true)), blend: Some(Blend { rgb: [0x8006, 0x303, 0x300], alpha: [0x800b, 0x8002, 0x307], color: [0.2, 0.2, 0.8, 0.4] }), dither: false, mask: [true, false, true, false] },
        Cmd { depth: 0.49269, quad: [62, 10, 4, 46], scissor: None, stencil: None, depth_test: Some((0x200, false)), blend: None, dither: false, mask: [false, true, true, true] },
    ],
];

/// Case 66's five iterations of five render commands (seed `0x667eacfd ^ deInt32Hash(66)` = 0xade7f7bc).
#[rustfmt::skip]
static INTERACTION_66: [[Cmd; 5]; 5] = [
    [
        Cmd { depth: 0.95, quad: [37, 62, 4, 0], scissor: None, stencil: None, depth_test: Some((0x200, true)), blend: None, dither: true, mask: [true, true, true, true] },
        Cmd { depth: 0.5331725, quad: [2, 2, 60, 35], scissor: None, stencil: None, depth_test: Some((0x202, false)), blend: None, dither: false, mask: [true, true, false, false] },
        Cmd { depth: 0.556345, quad: [62, 9, 5, 59], scissor: None, stencil: None, depth_test: Some((0x201, false)), blend: Some(Blend { rgb: [0x8006, 0x8002, 0x303], alpha: [0x800b, 0x8003, 0x300], color: [0.6, 1.0, 0.0, 0.4] }), dither: true, mask: [true, true, true, false] },
        Cmd { depth: 1.0195175, quad: [53, 62, 2, 28], scissor: None, stencil: None, depth_test: None, blend: None, dither: false, mask: [true, false, true, true] },
        Cmd { depth: 1.04269, quad: [52, 54, 4, 11], scissor: None, stencil: None, depth_test: None, blend: Some(Blend { rgb: [0x800a, 0x305, 0x302], alpha: [0x800b, 0x301, 0x307], color: [0.5, 0.6, 0.4, 1.0] }), dither: true, mask: [false, false, true, true] },
    ],
    [
        Cmd { depth: 0.5, quad: [9, 38, 59, 3], scissor: None, stencil: Some([St { func: 0x206, reference: 146, compare_mask: 0x4b00cab6, fail: 0x0, depth_fail: 0x150a, pass: 0x0, write_mask: 0xee037cba }, St { func: 0x201, reference: 104, compare_mask: 0x26780d46, fail: 0x0, depth_fail: 0x150a, pass: 0x1e01, write_mask: 0xe9a0e1cb }]), depth_test: Some((0x200, false)), blend: Some(Blend { rgb: [0x800a, 0x301, 0x8001], alpha: [0x800b, 0x308, 0x1], color: [0.2, 0.6, 1.0, 0.0] }), dither: false, mask: [false, true, false, true] },
        Cmd { depth: 0.5231725, quad: [54, 63, 2, 4], scissor: None, stencil: Some([St { func: 0x204, reference: 120, compare_mask: 0x7d32ab9e, fail: 0x8507, depth_fail: 0x0, pass: 0x1e02, write_mask: 0xa181d4e }, St { func: 0x207, reference: 7, compare_mask: 0x727dd3f6, fail: 0x8507, depth_fail: 0x8508, pass: 0x1e01, write_mask: 0x8a5551e7 }]), depth_test: Some((0x201, false)), blend: None, dither: true, mask: [true, true, true, false] },
        Cmd { depth: 0.996345, quad: [13, 59, 62, 27], scissor: None, stencil: None, depth_test: Some((0x205, false)), blend: None, dither: false, mask: [true, true, true, true] },
        Cmd { depth: 1.0195175, quad: [11, 38, 46, 5], scissor: None, stencil: None, depth_test: None, blend: None, dither: true, mask: [true, true, true, true] },
        Cmd { depth: 0.49269, quad: [2, 11, 52, 54], scissor: None, stencil: Some([St { func: 0x207, reference: 182, compare_mask: 0xc6d5b78d, fail: 0x1e00, depth_fail: 0x0, pass: 0x0, write_mask: 0x36a09903 }, St { func: 0x202, reference: 69, compare_mask: 0x2f174b8e, fail: 0x8507, depth_fail: 0x150a, pass: 0x150a, write_mask: 0xa75b3822 }]), depth_test: Some((0x201, true)), blend: None, dither: true, mask: [true, true, true, true] },
    ],
    [
        Cmd { depth: 0.51, quad: [60, 1, 0, 61], scissor: None, stencil: None, depth_test: None, blend: Some(Blend { rgb: [0x800b, 0x300, 0x306], alpha: [0x8006, 0x304, 0x8001], color: [0.8, 0.0, 0.6, 0.8] }), dither: false, mask: [false, true, true, false] },
        Cmd { depth: 0.4231725, quad: [5, 1, 60, 61], scissor: None, stencil: Some([St { func: 0x204, reference: 242, compare_mask: 0x32e4fd70, fail: 0x1e03, depth_fail: 0x1e01, pass: 0x150a, write_mask: 0x907199c7 }, St { func: 0x207, reference: 44, compare_mask: 0xc4e45654, fail: 0x0, depth_fail: 0x1e02, pass: 0x1e01, write_mask: 0x3faf33f2 }]), depth_test: Some((0x202, true)), blend: None, dither: true, mask: [true, true, true, true] },
        Cmd { depth: 0.046345, quad: [10, 63, 62, 1], scissor: None, stencil: Some([St { func: 0x201, reference: 58, compare_mask: 0x5213b26c, fail: 0x1e00, depth_fail: 0x1e01, pass: 0x8507, write_mask: 0x79fdbce5 }, St { func: 0x200, reference: 204, compare_mask: 0xfbe49b95, fail: 0x8508, depth_fail: 0x1e03, pass: 0x1e00, write_mask: 0x54e9e0c5 }]), depth_test: None, blend: None, dither: false, mask: [true, true, true, true] },
        Cmd { depth: 0.4695175, quad: [3, 63, 57, 1], scissor: None, stencil: None, depth_test: Some((0x202, true)), blend: None, dither: true, mask: [true, true, true, false] },
        Cmd { depth: 0.89269, quad: [3, 21, 54, 57], scissor: None, stencil: None, depth_test: Some((0x207, true)), blend: None, dither: false, mask: [true, true, false, true] },
    ],
    [
        Cmd { depth: 0.4, quad: [58, 58, 11, 10], scissor: None, stencil: None, depth_test: Some((0x202, true)), blend: Some(Blend { rgb: [0x8006, 0x305, 0x8001], alpha: [0x800b, 0x303, 0x305], color: [0.8, 0.8, 0.6, 0.8] }), dither: false, mask: [false, false, false, true] },
        Cmd { depth: 0.5331725, quad: [1, 2, 63, 43], scissor: None, stencil: None, depth_test: Some((0x203, true)), blend: Some(Blend { rgb: [0x800b, 0x8003, 0x8001], alpha: [0x8006, 0x301, 0x8004], color: [0.4, 0.8, 0.5, 0.0] }), dither: false, mask: [true, false, true, true] },
        Cmd { depth: 0.846345, quad: [50, 0, 13, 54], scissor: None, stencil: None, depth_test: Some((0x201, true)), blend: None, dither: true, mask: [false, true, true, true] },
        Cmd { depth: 0.0695175, quad: [1, 56, 50, 17], scissor: Some([-3, 10, 77, 55]), stencil: None, depth_test: None, blend: Some(Blend { rgb: [0x800a, 0x0, 0x1], alpha: [0x800b, 0x307, 0x301], color: [0.0, 0.0, 0.4, 0.8] }), dither: true, mask: [true, false, false, true] },
        Cmd { depth: 0.09269, quad: [37, 20, 1, 63], scissor: None, stencil: None, depth_test: Some((0x206, true)), blend: Some(Blend { rgb: [0x800a, 0x8003, 0x305], alpha: [0x800b, 0x300, 0x1], color: [0.6, 0.0, 0.6, 0.5] }), dither: false, mask: [true, false, true, true] },
    ],
    [
        Cmd { depth: 0.2, quad: [51, 38, 14, 6], scissor: None, stencil: Some([St { func: 0x202, reference: 233, compare_mask: 0x9f789550, fail: 0x0, depth_fail: 0x1e01, pass: 0x150a, write_mask: 0x4dcb3eb2 }, St { func: 0x204, reference: 213, compare_mask: 0xe52f8c48, fail: 0x150a, depth_fail: 0x8507, pass: 0x1e02, write_mask: 0x452cf294 }]), depth_test: Some((0x202, true)), blend: None, dither: false, mask: [true, true, false, true] },
        Cmd { depth: 0.5231725, quad: [61, 40, 26, 2], scissor: None, stencil: None, depth_test: Some((0x204, false)), blend: Some(Blend { rgb: [0x800a, 0x1, 0x306], alpha: [0x8006, 0x302, 0x307], color: [0.5, 1.0, 0.0, 0.5] }), dither: true, mask: [true, true, true, true] },
        Cmd { depth: 0.246345, quad: [7, 2, 59, 53], scissor: Some([1, -3, 66, 68]), stencil: None, depth_test: None, blend: None, dither: false, mask: [false, true, true, true] },
        Cmd { depth: 0.5695175, quad: [63, 49, 16, 5], scissor: Some([-8, -4, 80, 61]), stencil: None, depth_test: Some((0x206, false)), blend: Some(Blend { rgb: [0x800a, 0x302, 0x306], alpha: [0x800a, 0x303, 0x8003], color: [0.4, 0.8, 0.4, 1.0] }), dither: false, mask: [false, true, true, true] },
        Cmd { depth: 0.09269, quad: [53, 58, 13, 2], scissor: None, stencil: None, depth_test: None, blend: None, dither: true, mask: [false, true, true, true] },
    ],
];

/// `FragOpInteractionCase::init` and `iterate` for one case, as plain GL
/// calls: an RGBA8 + DEPTH24_STENCIL8 framebuffer object of the target's
/// size, the program, then for each of the five iterations the clear
/// (`getWellBehavingColor` of (0, 0.25, 0.5, 1) for 8 bits, depth 1,
/// stencil 0), the four vertices of each command and six `u16` indices drawn
/// from client arrays, the `glGetError` after every command and the
/// `glReadPixels` of the viewport. Each draw must return `GL_NO_ERROR`, be
/// unrefused and leave no refusal line.
fn interaction(label: &str, fs: &str, origin: [i32; 2], iterations: &[[Cmd; 5]; 5]) {
    let _g = context();
    let (v, f) = (compile(GL_VERTEX_SHADER, INTERACTION_VS), compile(GL_FRAGMENT_SHADER, fs));
    // SAFETY: plain GL calls; every client array outlives its draw.
    unsafe {
        let mut old: GLint = 0;
        glGetIntegerv(GL_FRAMEBUFFER_BINDING, &mut old);
        let (mut fb, mut rbs) = (0, [0; 2]);
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        glGenRenderbuffers(2, rbs.as_mut_ptr());
        for (rb, (format, attachment)) in
            rbs.iter().zip([(GL_RGBA8, GL_COLOR_ATTACHMENT0), (GL_DEPTH24_STENCIL8, GL_DEPTH_STENCIL_ATTACHMENT)])
        {
            glBindRenderbuffer(GL_RENDERBUFFER, *rb);
            glRenderbufferStorage(GL_RENDERBUFFER, format, INTERACTION_TARGET, INTERACTION_TARGET);
            glFramebufferRenderbuffer(GL_FRAMEBUFFER, attachment, GL_RENDERBUFFER, *rb);
        }
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE, "{label}: the framebuffer");
        let p = glCreateProgram();
        glAttachShader(p, v);
        glAttachShader(p, f);
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        assert_eq!(ok, 1, "{label}: the program does not link");
        glUseProgram(p);
        let pos = glGetAttribLocation(p, c"dEQP_Position".as_ptr());
        assert!(pos >= 0, "{label}: dEQP_Position has no location");
        let pos = pos as GLuint;
        glBindBuffer(GL_ARRAY_BUFFER, 0);
        glBindBuffer(GL_ELEMENT_ARRAY_BUFFER, 0);
        glEnableVertexAttribArray(pos);

        let (ox, oy) = (origin[0], origin[1]);
        let indices: [u16; 6] = [0, 1, 2, 2, 1, 3];
        let mut pixels = vec![0u8; (INTERACTION_VIEWPORT * INTERACTION_VIEWPORT * 4) as usize];
        let mut failures = Vec::new();
        for (iter, cmds) in iterations.iter().enumerate() {
            glDisable(GL_SCISSOR_TEST);
            glColorMask(1, 1, 1, 1);
            glDepthMask(1);
            glStencilMask(!0);
            glClearColor(0.0, 512.0 / 2047.0, 1024.0 / 2047.0, 1.0);
            glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT | GL_STENCIL_BUFFER_BIT);
            for (n, c) in cmds.iter().enumerate() {
                let at = format!("{label}: iteration {iter} command {n}");
                set_interaction_state(c, ox, oy);
                // renderQuad
                let (flip_x, flip_y) = (c.quad[2] < c.quad[0], c.quad[3] < c.quad[1]);
                let (vx, vy) = (c.quad[0].min(c.quad[2]), c.quad[1].min(c.quad[3]));
                let (vw, vh) = ((c.quad[0] - c.quad[2]).abs() + 1, (c.quad[1] - c.quad[3]).abs() + 1);
                let (a, b) = (
                    [if flip_x { 1.0f32 } else { -1.0 }, if flip_y { 1.0 } else { -1.0 }],
                    [if flip_x { -1.0f32 } else { 1.0 }, if flip_y { -1.0 } else { 1.0 }],
                );
                let d = c.depth;
                let verts: [f32; 16] = [a[0], a[1], d, 1.0, b[0], a[1], d, 1.0, a[0], b[1], d, 1.0, b[0], b[1], d, 1.0];
                glVertexAttribPointer(pos, 4, GL_FLOAT, 0, 0, verts.as_ptr() as *const c_void);
                glViewport(ox + vx, oy + vy, vw, vh);
                while glGetError() != GL_NO_ERROR {}
                let before = backend::draws_refused();
                LOG.lock().unwrap_or_else(|e| e.into_inner()).clear();
                glDrawElements(GL_TRIANGLES, 6, GL_UNSIGNED_SHORT, indices.as_ptr() as *const c_void);
                let e = glGetError();
                let refused = backend::draws_refused() - before;
                let log = LOG.lock().unwrap_or_else(|e| e.into_inner()).clone();
                let refusals: Vec<_> = log.iter().filter(|l| l.contains("refus")).collect();
                if e != GL_NO_ERROR || refused != 0 || !refusals.is_empty() {
                    failures.push(format!("{at}: glGetError 0x{e:x}, {refused} refused, refusal lines {refusals:?}"));
                }
            }
            glReadPixels(
                ox,
                oy,
                INTERACTION_VIEWPORT,
                INTERACTION_VIEWPORT,
                GL_RGBA,
                GL_UNSIGNED_BYTE,
                pixels.as_mut_ptr() as *mut c_void,
            );
            let e = glGetError();
            if e != GL_NO_ERROR {
                failures.push(format!("{label}: iteration {iter} glReadPixels: glGetError 0x{e:x}"));
            }
        }
        glDisableVertexAttribArray(pos);
        glDisable(GL_BLEND);
        glDisable(GL_SCISSOR_TEST);
        glDisable(GL_STENCIL_TEST);
        glDisable(GL_DEPTH_TEST);
        glColorMask(1, 1, 1, 1);
        glDepthMask(1);
        glStencilMask(!0);
        glBindFramebuffer(GL_FRAMEBUFFER, old as GLuint);
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }
}

/// `setRenderState`.
fn set_interaction_state(c: &Cmd, ox: i32, oy: i32) {
    // SAFETY: plain GL calls.
    unsafe {
        match c.scissor {
            Some([x, y, w, h]) => {
                glEnable(GL_SCISSOR_TEST);
                glScissor(ox + x, oy + y, w, h);
            }
            None => glDisable(GL_SCISSOR_TEST),
        }
        match &c.stencil {
            Some(faces) => {
                glEnable(GL_STENCIL_TEST);
                for (face, s) in [GL_BACK, GL_FRONT].into_iter().zip(faces) {
                    glStencilFuncSeparate(face, s.func, s.reference, s.compare_mask);
                    glStencilOpSeparate(face, s.fail, s.depth_fail, s.pass);
                    glStencilMaskSeparate(face, s.write_mask);
                }
            }
            None => glDisable(GL_STENCIL_TEST),
        }
        match c.depth_test {
            Some((func, write)) => {
                glEnable(GL_DEPTH_TEST);
                glDepthFunc(func);
                glDepthMask(write as GLboolean);
            }
            None => glDisable(GL_DEPTH_TEST),
        }
        match &c.blend {
            Some(b) => {
                glEnable(GL_BLEND);
                glBlendEquationSeparate(b.rgb[0], b.alpha[0]);
                glBlendFuncSeparate(b.rgb[1], b.rgb[2], b.alpha[1], b.alpha[2]);
                glBlendColor(b.color[0], b.color[1], b.color[2], b.color[3]);
            }
            None => glDisable(GL_BLEND),
        }
        if c.dither {
            glEnable(GL_DITHER);
        } else {
            glDisable(GL_DITHER);
        }
        let m = c.mask;
        glColorMask(m[0] as GLboolean, m[1] as GLboolean, m[2] as GLboolean, m[3] as GLboolean);
    }
}

/// `fragment_ops.interaction.basic_shader.11`: a constant-colour fragment shader under random blend, stencil, depth, scissor and
/// colour-mask states. Viewport origin (91, 52).
#[test]
fn interaction_basic_shader_11() {
    interaction("interaction.basic_shader.11", INTERACTION_11_FS, [91, 52], &INTERACTION_11);
}

/// `fragment_ops.interaction.basic_shader.66`: a constant fragment colour
/// behind an integer compare under random states. Viewport origin (76, 37).
#[test]
fn interaction_basic_shader_66() {
    interaction("interaction.basic_shader.66", INTERACTION_66_FS, [76, 37], &INTERACTION_66);
}

/* ---- the generator cases: shaders.random.*.vertex.* -------------------- */

/// One `rsg::ShaderInput` of a vertex shader: its name, its component count
/// and its value range (lanes past `n` unused).
struct Input {
    name: &'static str,
    n: usize,
    min: [f32; 4],
    max: [f32; 4],
}

/// One uniform and the value `computeUniformValues` gave it.
enum Uniform {
    Float(&'static str, &'static [f32]),
    Sampler2D(&'static str, GLint),
}

/// The fragment shader the generator writes for a vertex-only case
/// (`fragmentParameters.randomize` false).
const FS_GENERATOR: &str = concat!(
    "precision mediump float;\n",
    "varying mediump vec4 v_color;\n",
    "\n",
    "void main()\n",
    "{\n",
    "\tgl_FragColor = v_color;\n",
    "}\n",
);

/// `RandomShaderCase::init` and `iterate` (modules/glshared/glsRandomShaderCase.cpp)
/// for one vertex case, as plain GL calls: the 256x256 RGBA8 framebuffer
/// object of the board run's target (as `interaction`), the program, the
/// 2x2 vertices of the 1x1 grid (`dEQP_Position` the corners, every other
/// input `min + (xd + yd) / 2 * (max - min)` per lane,
/// `getVertexInterpolationCoords`), the uniforms, a 64x64 RGBA texture for a
/// `sampler2D` (`fillWithComponentGradients` of (-1,-1,-1,2)..(1,1,1,0),
/// clamp to edge, linear), the 64x64 viewport at `origin` (`rnd.getInt(0,
/// 256 - 64)` twice, `de::Random(seed)`), six `u16` indices by
/// `glDrawElements` and the `glReadPixels` of the viewport. The draw must
/// return `GL_NO_ERROR` (dEQP's `GLU_CHECK_MSG("Draw")`), be unrefused and
/// leave no refusal line.
fn random_case(label: &str, vs: &str, inputs: &[Input], uniforms: &[Uniform], origin: [i32; 2]) {
    let _g = context();
    let (v, f) = (compile(GL_VERTEX_SHADER, vs), compile(GL_FRAGMENT_SHADER, FS_GENERATOR));
    // The vertex arrays, built before any GL call as `init` does.
    let arrays: Vec<Vec<f32>> = inputs
        .iter()
        .map(|i| {
            let mut a = Vec::with_capacity(4 * i.n);
            for vtx in 0..4 {
                let (xf, yf) = ((vtx % 2) as f32, (vtx / 2) as f32);
                if i.name == "dEQP_Position" {
                    a.extend_from_slice(&[-1.0 + xf * 2.0, 1.0 - yf * 2.0, 0.0, 1.0]);
                    continue;
                }
                for c in 0..i.n {
                    let xd = if c % 4 < 2 { xf } else { 1.0 - xf };
                    let yd = if c % 2 == 0 { yf } else { 1.0 - yf };
                    a.push(i.min[c] + (xd + yd) / 2.0 * (i.max[c] - i.min[c]));
                }
            }
            a
        })
        .collect();
    // SAFETY: plain GL calls; every client array outlives its draw.
    unsafe {
        let mut old: GLint = 0;
        glGetIntegerv(GL_FRAMEBUFFER_BINDING, &mut old);
        let (mut fb, mut rb) = (0, 0);
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        glGenRenderbuffers(1, &mut rb);
        glBindRenderbuffer(GL_RENDERBUFFER, rb);
        glRenderbufferStorage(GL_RENDERBUFFER, GL_RGBA8, INTERACTION_TARGET, INTERACTION_TARGET);
        glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_RENDERBUFFER, rb);
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE, "{label}: the framebuffer");
        let mut textures = Vec::new();
        for u in uniforms {
            if let Uniform::Sampler2D(_, unit) = u {
                let mut texels = vec![0u8; 64 * 64 * 4];
                for y in 0..64 {
                    for x in 0..64 {
                        let (s, t) = ((x as f32 + 0.5) / 64.0, (y as f32 + 0.5) / 64.0);
                        let c = [(s + t) * 0.5, (s + 1.0 - t) * 0.5, (1.0 - s + t) * 0.5, (2.0 - s - t) * 0.5];
                        let (lo, hi) = ([-1.0f32, -1.0, -1.0, 2.0], [1.0f32, 1.0, 1.0, 0.0]);
                        for k in 0..4 {
                            let val = (lo[k] + c[k] * (hi[k] - lo[k])).clamp(0.0, 1.0);
                            texels[(y * 64 + x) * 4 + k] = (val * 255.0 + 0.5) as u8;
                        }
                    }
                }
                let mut tex = 0;
                yakogl::gl::texture::glGenTextures(1, &mut tex);
                yakogl::gl::texture::glActiveTexture(GL_TEXTURE0 + *unit as GLenum);
                yakogl::gl::texture::glBindTexture(GL_TEXTURE_2D, tex);
                yakogl::gl::texture::glTexImage2D(
                    GL_TEXTURE_2D,
                    0,
                    GL_RGBA as GLint,
                    64,
                    64,
                    0,
                    GL_RGBA,
                    GL_UNSIGNED_BYTE,
                    texels.as_ptr() as *const c_void,
                );
                for (pname, param) in [
                    (GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE),
                    (GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE),
                    (GL_TEXTURE_MIN_FILTER, GL_LINEAR),
                    (GL_TEXTURE_MAG_FILTER, GL_LINEAR),
                ] {
                    yakogl::gl::texture::glTexParameteri(GL_TEXTURE_2D, pname, param as GLint);
                }
                textures.push(tex);
            }
        }
        let p = glCreateProgram();
        glAttachShader(p, v);
        glAttachShader(p, f);
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        assert_eq!(ok, 1, "{label}: the program does not link");
        glUseProgram(p);
        glBindBuffer(GL_ARRAY_BUFFER, 0);
        glBindBuffer(GL_ELEMENT_ARRAY_BUFFER, 0);
        let mut failures = Vec::new();
        let mut locs = Vec::new();
        for (i, a) in inputs.iter().zip(&arrays) {
            let name = CString::new(i.name).unwrap();
            let l = glGetAttribLocation(p, name.as_ptr());
            if l >= 0 {
                glVertexAttribPointer(l as GLuint, i.n as GLint, GL_FLOAT, 0, 0, a.as_ptr() as *const c_void);
                glEnableVertexAttribArray(l as GLuint);
                locs.push(l as GLuint);
            }
        }
        let e = glGetError();
        if e != GL_NO_ERROR {
            failures.push(format!("{label}: after attribute setup: glGetError 0x{e:x}"));
        }
        for u in uniforms {
            let (name, l) = match u {
                Uniform::Float(n, _) | Uniform::Sampler2D(n, _) => {
                    let c = CString::new(*n).unwrap();
                    (*n, glGetUniformLocation(p, c.as_ptr()))
                }
            };
            if l < 0 {
                continue;
            }
            match u {
                Uniform::Float(_, v) => match v.len() {
                    1 => glUniform1fv(l, 1, v.as_ptr()),
                    2 => glUniform2fv(l, 1, v.as_ptr()),
                    3 => glUniform3fv(l, 1, v.as_ptr()),
                    4 => glUniform4fv(l, 1, v.as_ptr()),
                    n => panic!("{label}: uniform {name} has {n} lanes"),
                },
                Uniform::Sampler2D(_, unit) => glUniform1iv(l, 1, unit),
            }
        }
        let e = glGetError();
        if e != GL_NO_ERROR {
            failures.push(format!("{label}: after uniform setup: glGetError 0x{e:x}"));
        }
        glViewport(origin[0], origin[1], 64, 64);
        LOG.lock().unwrap_or_else(|e| e.into_inner()).clear();
        let before = backend::draws_refused();
        let indices: [u16; 6] = [0, 2, 1, 1, 2, 3];
        glDrawElements(GL_TRIANGLES, 6, GL_UNSIGNED_SHORT, indices.as_ptr() as *const c_void);
        glFlush();
        let e = glGetError();
        let refused = backend::draws_refused() - before;
        let log = LOG.lock().unwrap_or_else(|e| e.into_inner()).clone();
        let refusals: Vec<_> = log.iter().filter(|l| l.contains("refus")).collect();
        if e != GL_NO_ERROR || refused != 0 || !refusals.is_empty() {
            failures
                .push(format!("{label}: the draw: glGetError 0x{e:x}, {refused} refused, refusal lines {refusals:?}"));
        }
        let mut pixels = vec![0u8; 64 * 64 * 4];
        glReadPixels(origin[0], origin[1], 64, 64, GL_RGBA, GL_UNSIGNED_BYTE, pixels.as_mut_ptr() as *mut c_void);
        let e = glGetError();
        if e != GL_NO_ERROR {
            failures.push(format!("{label}: glReadPixels: glGetError 0x{e:x}"));
        }
        for l in locs {
            glDisableVertexAttribArray(l);
        }
        yakogl::gl::texture::glActiveTexture(GL_TEXTURE0);
        glViewport(0, 0, 8, 8);
        glBindFramebuffer(GL_FRAMEBUFFER, old as GLuint);
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }
}

/// `shaders.random.basic_expression.vertex.53`'s vertex shader, as the generator wrote it.
const BASIC_EXPRESSION_VERTEX_53_VS: &str = concat!(
    "attribute vec4 dEQP_Position;\n",
    "attribute vec4 g;\n",
    "const vec4 f = vec4(-1.0, 13.5, 9.0, 6.25) + vec4(7.5, -5.5, -1.0, -3.5) + vec4(-7.5, -16.0, -7.0, -4.0) * vec4(1.0, 0.5, 1.0, 0.75);\n",
    "attribute vec4 a;\n",
    "attribute vec4 b;\n",
    "float c = float(float(5.125)) * (float(-0.75));\n",
    "const float d = (0.3) * -1.0;\n",
    "float e = float(-30.55) + float(3.75) + float(float(7.0));\n",
    "varying mediump vec4 v_color;\n",
    "\n",
    "void main()\n",
    "{\n",
    "\tgl_Position = dEQP_Position;\n",
    "\tv_color = g * (f);\n",
    "\tc = c;\n",
    "\tv_color = vec4(e, 1.15, d, c) - b - v_color;\n",
    "\tv_color = a;\n",
    "\tv_color = v_color;\n",
    "}\n",
);
/// Its inputs in `Shader::getInputs` order.
const BASIC_EXPRESSION_VERTEX_53_INPUTS: &[Input] = &[
    Input { name: "b", n: 4, min: [-16.0, -8.5, 1.5, -1.5], max: [-5.0, 2.5, 5.5, 8.0] },
    Input { name: "a", n: 4, min: [0.0, 0.0, 0.0, 0.0], max: [1.0, 1.0, 1.0, 1.0] },
    Input { name: "g", n: 4, min: [6.125, -13.0, -10.125, 11.625], max: [8.125, 4.75, -2.625, 12.125] },
    Input { name: "dEQP_Position", n: 4, min: [-1.0, -1.0, 0.0, 1.0], max: [1.0, 1.0, 0.0, 1.0] },
];
/// Its uniforms and their values (`computeUniformValues`, `de::Random(53)`).
const BASIC_EXPRESSION_VERTEX_53_UNIFORMS: &[Uniform] = &[];

/// `shaders.random.basic_expression.vertex.85`'s vertex shader, as the generator wrote it.
const BASIC_EXPRESSION_VERTEX_85_VS: &str = concat!(
    "attribute vec4 dEQP_Position;\n",
    "attribute vec4 b;\n",
    "attribute vec4 c;\n",
    "attribute float d;\n",
    "attribute float e;\n",
    "const float f = 3.0;\n",
    "attribute float g;\n",
    "float h = float(f) * f - f * float(f);\n",
    "attribute float i;\n",
    "attribute float j;\n",
    "uniform mediump float k;\n",
    "varying mediump vec4 v_color;\n",
    "\n",
    "void main()\n",
    "{\n",
    "\tgl_Position = dEQP_Position;\n",
    "\th = -2.4 + i;\n",
    "\tv_color = (vec4(-19.1, k, j, i) - vec4(h, 2.25, g, d));\n",
    "\tv_color = c + (vec4(e, 1.5, f, -5.5));\n",
    "\tvec4 a = vec4(d, e, d, 8.0) * c * (c);\n",
    "\ta = b;\n",
    "}\n",
);
/// Its inputs in `Shader::getInputs` order.
const BASIC_EXPRESSION_VERTEX_85_INPUTS: &[Input] = &[
    Input { name: "j", n: 1, min: [15.300001, 0.0, 0.0, 0.0], max: [16.800001, 0.0, 0.0, 0.0] },
    Input { name: "i", n: 1, min: [-2.5, 0.0, 0.0, 0.0], max: [-2.1000004, 0.0, 0.0, 0.0] },
    Input { name: "g", n: 1, min: [1.0, 0.0, 0.0, 0.0], max: [2.0, 0.0, 0.0, 0.0] },
    Input { name: "e", n: 1, min: [7.5, 0.0, 0.0, 0.0], max: [8.5, 0.0, 0.0, 0.0] },
    Input { name: "d", n: 1, min: [-2.3000002, 0.0, 0.0, 0.0], max: [2.5, 0.0, 0.0, 0.0] },
    Input { name: "c", n: 4, min: [-7.5, -1.5, -3.0, 5.5], max: [-7.5, -0.5, -2.0, 6.5] },
    Input { name: "b", n: 4, min: [0.0, 0.0, 0.0, 0.0], max: [1.0, 1.0, 1.0, 1.0] },
    Input { name: "dEQP_Position", n: 4, min: [-1.0, -1.0, 0.0, 1.0], max: [1.0, 1.0, 0.0, 1.0] },
];
/// Its uniforms and their values (`computeUniformValues`, `de::Random(85)`).
const BASIC_EXPRESSION_VERTEX_85_UNIFORMS: &[Uniform] = &[Uniform::Float("k", &[-4.5249996])];

/// `shaders.random.basic_expression.vertex.90`'s vertex shader, as the generator wrote it.
const BASIC_EXPRESSION_VERTEX_90_VS: &str = concat!(
    "attribute vec4 dEQP_Position;\n",
    "uniform mediump float a;\n",
    "attribute float b;\n",
    "uniform mediump vec3 c;\n",
    "attribute float e;\n",
    "attribute float f;\n",
    "attribute float g;\n",
    "attribute vec4 h;\n",
    "attribute vec4 i;\n",
    "varying mediump vec4 v_color;\n",
    "\n",
    "void main()\n",
    "{\n",
    "\tgl_Position = dEQP_Position;\n",
    "\tv_color = vec4(1.75, -3.0, -0.25, -1.5) * vec4(-2.0, f, 1.0, -1.0) + (vec4(2.0, 3.5, -0.0, -1.0));\n",
    "\tv_color = (v_color);\n",
    "\tv_color = vec4(-11.0, -5.75, 3.5, -1.5) + vec4(7.75, 8.5, 4.0, -2.5) + vec4(-3.25, -2.0, 25.0, g) * v_color;\n",
    "\tv_color = i * v_color;\n",
    "\tv_color = i;\n",
    "\tv_color = h;\n",
    "\tv_color = (v_color);\n",
    "\tvec4 d = v_color + vec4(g, a, f, e) * v_color;\n",
    "\td = vec4(1.0, float(0.0), float(e), float(-0.25)) * vec4(f, float(f), e, a);\n",
    "\t(c) - vec3(float(3.75) * -1.5, b + 5.0, float(a));\n",
    "\tv_color = v_color;\n",
    "}\n",
);
/// Its inputs in `Shader::getInputs` order.
const BASIC_EXPRESSION_VERTEX_90_INPUTS: &[Input] = &[
    Input { name: "i", n: 4, min: [4.0, -14.7, -5.75, 6.25], max: [6.0, -11.599999, 1.5, 13.400001] },
    Input { name: "h", n: 4, min: [0.0, 0.0, 0.0, 0.0], max: [1.0, 1.0, 1.0, 1.0] },
    Input { name: "g", n: 1, min: [9.0, 0.0, 0.0, 0.0], max: [9.0, 0.0, 0.0, 0.0] },
    Input { name: "f", n: 1, min: [0.75, 0.0, 0.0, 0.0], max: [0.75, 0.0, 0.0, 0.0] },
    Input { name: "e", n: 1, min: [-0.5, 0.0, 0.0, 0.0], max: [-0.5, 0.0, 0.0, 0.0] },
    Input { name: "b", n: 1, min: [-9.5, 0.0, 0.0, 0.0], max: [-8.5, 0.0, 0.0, 0.0] },
    Input { name: "dEQP_Position", n: 4, min: [-1.0, -1.0, 0.0, 1.0], max: [1.0, 1.0, 0.0, 1.0] },
];
/// Its uniforms and their values (`computeUniformValues`, `de::Random(90)`).
const BASIC_EXPRESSION_VERTEX_90_UNIFORMS: &[Uniform] =
    &[Uniform::Float("c", &[-11.85, 4.9750004, -10.799999]), Uniform::Float("a", &[-2.375])];

/// `shaders.random.basic_expression.vertex.95`'s vertex shader, as the generator wrote it.
const BASIC_EXPRESSION_VERTEX_95_VS: &str = concat!(
    "attribute vec4 dEQP_Position;\n",
    "const float j = float(-6.75) * 0.5 * float(float(-2.0));\n",
    "const float i = float(float(-3.25)) - -2.25;\n",
    "attribute vec4 b;\n",
    "attribute float c;\n",
    "attribute float d;\n",
    "attribute float e;\n",
    "attribute float f;\n",
    "attribute float g;\n",
    "attribute vec4 h;\n",
    "varying mediump vec4 v_color;\n",
    "\n",
    "void main()\n",
    "{\n",
    "\tgl_Position = dEQP_Position;\n",
    "\tv_color = vec4(j, g, e, -1.75) - (vec4(6.5, 5.75, -8.25, i));\n",
    "\tv_color = (v_color);\n",
    "\tv_color = v_color;\n",
    "\tv_color = v_color * h;\n",
    "\tvec4 a = vec4(g, -0.5, f, e) * v_color * (vec4(-9.0, d, d, c));\n",
    "\ta = b;\n",
    "}\n",
);
/// Its inputs in `Shader::getInputs` order.
const BASIC_EXPRESSION_VERTEX_95_INPUTS: &[Input] = &[
    Input { name: "h", n: 4, min: [0.5, -0.5, -0.75, -1.25], max: [2.5, -0.0, -0.0, -0.25] },
    Input { name: "g", n: 1, min: [4.0, 0.0, 0.0, 0.0], max: [5.0, 0.0, 0.0, 0.0] },
    Input { name: "f", n: 1, min: [0.0, 0.0, 0.0, 0.0], max: [1.0, 0.0, 0.0, 0.0] },
    Input { name: "e", n: 1, min: [-9.25, 0.0, 0.0, 0.0], max: [-8.25, 0.0, 0.0, 0.0] },
    Input { name: "d", n: 1, min: [0.0, 0.0, 0.0, 0.0], max: [1.0, 0.0, 0.0, 0.0] },
    Input { name: "c", n: 1, min: [0.0, 0.0, 0.0, 0.0], max: [1.0, 0.0, 0.0, 0.0] },
    Input { name: "b", n: 4, min: [0.0, 0.0, 0.0, 0.0], max: [1.0, 1.0, 1.0, 1.0] },
    Input { name: "dEQP_Position", n: 4, min: [-1.0, -1.0, 0.0, 1.0], max: [1.0, 1.0, 0.0, 1.0] },
];
/// Its uniforms and their values (`computeUniformValues`, `de::Random(95)`).
const BASIC_EXPRESSION_VERTEX_95_UNIFORMS: &[Uniform] = &[];

/// `shaders.random.scalar_conversion.vertex.53`'s vertex shader, as the generator wrote it.
const SCALAR_CONVERSION_VERTEX_53_VS: &str = concat!(
    "attribute vec4 dEQP_Position;\n",
    "attribute vec4 e;\n",
    "attribute vec4 a;\n",
    "attribute vec4 b;\n",
    "const float c = float(float(0.3) * float(-1.0));\n",
    "uniform mediump float d;\n",
    "varying mediump vec4 v_color;\n",
    "\n",
    "void main()\n",
    "{\n",
    "\tgl_Position = dEQP_Position;\n",
    "\tv_color = (e);\n",
    "\t5;\n",
    "\tv_color = vec4(d, -7.6, c, -2) - b - v_color;\n",
    "\tv_color = a;\n",
    "\tv_color = v_color;\n",
    "}\n",
);
/// Its inputs in `Shader::getInputs` order.
const SCALAR_CONVERSION_VERTEX_53_INPUTS: &[Input] = &[
    Input { name: "b", n: 4, min: [-16.0, -8.5, 1.5, -1.5], max: [-5.0, 2.5, 5.5, 8.0] },
    Input { name: "a", n: 4, min: [0.0, 0.0, 0.0, 0.0], max: [1.0, 1.0, 1.0, 1.0] },
    Input { name: "e", n: 4, min: [-8.4, -3.7000008, -10.2, -15.300001], max: [-5.5, 2.0, -2.5, -2.5] },
    Input { name: "dEQP_Position", n: 4, min: [-1.0, -1.0, 0.0, 1.0], max: [1.0, 1.0, 0.0, 1.0] },
];
/// Its uniforms and their values (`computeUniformValues`, `de::Random(53)`).
const SCALAR_CONVERSION_VERTEX_53_UNIFORMS: &[Uniform] = &[Uniform::Float("d", &[-12.299999])];

/// `shaders.random.texture.vertex.19`'s vertex shader, as the generator wrote it.
const TEXTURE_VERTEX_19_VS: &str = concat!(
    "attribute vec4 dEQP_Position;\n",
    "const float f = float(float(-3.125) * float(-2.0));\n",
    "attribute float e;\n",
    "float d = 3.25;\n",
    "uniform sampler2D a;\n",
    "attribute vec3 b;\n",
    "attribute vec4 c;\n",
    "varying mediump vec4 v_color;\n",
    "\n",
    "void main()\n",
    "{\n",
    "\tgl_Position = dEQP_Position;\n",
    "\tv_color = vec4(f * float(e), e, float(d) - float(d), float(int(6)));\n",
    "\tb;\n",
    "\tv_color = v_color;\n",
    "\tv_color = (c) + v_color.arbg;\n",
    "\tv_color = texture2DProj(a, b);\n",
    "}\n",
);
/// Its inputs in `Shader::getInputs` order.
const TEXTURE_VERTEX_19_INPUTS: &[Input] = &[
    Input { name: "c", n: 4, min: [-16.1, 8.0, 2.9000006, 2.0], max: [-13.1, 17.5, 3.4000006, 6.0] },
    Input { name: "b", n: 3, min: [0.0, 0.0, 0.25, 0.0], max: [1.0, 1.0, 4.0, 0.0] },
    Input { name: "e", n: 1, min: [-0.5, 0.0, 0.0, 0.0], max: [-0.5, 0.0, 0.0, 0.0] },
    Input { name: "dEQP_Position", n: 4, min: [-1.0, -1.0, 0.0, 1.0], max: [1.0, 1.0, 0.0, 1.0] },
];
/// Its uniforms and their values (`computeUniformValues`, `de::Random(19)`).
const TEXTURE_VERTEX_19_UNIFORMS: &[Uniform] = &[Uniform::Sampler2D("a", 0)];

/// `shaders.random.basic_expression.vertex.53`: four vec4 inputs, `a` and
/// `g` read, `b` only into a dead store. `GL_INVALID_OPERATION` at the draw
/// on the board (glsRandomShaderCase.cpp:611) before `attrgap-fix`; draws at
/// the tip.
#[test]
fn random_basic_expression_vertex_53() {
    random_case(
        "random.basic_expression.vertex.53",
        BASIC_EXPRESSION_VERTEX_53_VS,
        BASIC_EXPRESSION_VERTEX_53_INPUTS,
        BASIC_EXPRESSION_VERTEX_53_UNIFORMS,
        [33, 97],
    );
}

/// `shaders.random.basic_expression.vertex.85`: seven inputs and a float
/// uniform, `b` only into a dead local. Was `GL_INVALID_OPERATION` at the
/// draw; draws at the tip.
#[test]
fn random_basic_expression_vertex_85() {
    random_case(
        "random.basic_expression.vertex.85",
        BASIC_EXPRESSION_VERTEX_85_VS,
        BASIC_EXPRESSION_VERTEX_85_INPUTS,
        BASIC_EXPRESSION_VERTEX_85_UNIFORMS,
        [149, 51],
    );
}

/// `shaders.random.basic_expression.vertex.90`: six inputs, two uniforms,
/// `b` and `c` only in an expression statement. Was `GL_INVALID_OPERATION`
/// at the draw; draws at the tip.
#[test]
fn random_basic_expression_vertex_90() {
    random_case(
        "random.basic_expression.vertex.90",
        BASIC_EXPRESSION_VERTEX_90_VS,
        BASIC_EXPRESSION_VERTEX_90_INPUTS,
        BASIC_EXPRESSION_VERTEX_90_UNIFORMS,
        [14, 54],
    );
}

/// `shaders.random.basic_expression.vertex.95`: seven inputs, `b`, `c`, `d`
/// and `f` only into a dead local. Was `GL_INVALID_OPERATION` at the draw;
/// draws at the tip.
#[test]
fn random_basic_expression_vertex_95() {
    random_case(
        "random.basic_expression.vertex.95",
        BASIC_EXPRESSION_VERTEX_95_VS,
        BASIC_EXPRESSION_VERTEX_95_INPUTS,
        BASIC_EXPRESSION_VERTEX_95_UNIFORMS,
        [10, 36],
    );
}

/// `shaders.random.scalar_conversion.vertex.53`: three vec4 inputs, `b` and
/// `e` only into values overwritten later, a float uniform. Was
/// `GL_INVALID_OPERATION` at the draw; draws at the tip.
#[test]
fn random_scalar_conversion_vertex_53() {
    random_case(
        "random.scalar_conversion.vertex.53",
        SCALAR_CONVERSION_VERTEX_53_VS,
        SCALAR_CONVERSION_VERTEX_53_INPUTS,
        SCALAR_CONVERSION_VERTEX_53_UNIFORMS,
        [33, 97],
    );
}

/// `shaders.random.texture.vertex.19`: a `texture2DProj` vertex look-up on
/// unit 0 (the 64x64 gradient texture), `c` and `e` only into values
/// overwritten later. Was `GL_INVALID_OPERATION` at the draw; draws at the
/// tip.
#[test]
fn random_texture_vertex_19() {
    random_case(
        "random.texture.vertex.19",
        TEXTURE_VERTEX_19_VS,
        TEXTURE_VERTEX_19_INPUTS,
        TEXTURE_VERTEX_19_UNIFORMS,
        [61, 22],
    );
}
