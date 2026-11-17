// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! A cube look-up on a unit without a complete cube map draws on the Mali
//! back end, reading Mesa's one-texel black cube (board-preflight, plan #373;
//! `texture.completeness.cube.*`, section M.1 row 6 of
//! docs/es2-cts-failures.md, row `T6S1R11`).
//!
//! The 16 cases were refused with `NOT_BUILT_CUBE_INCOMPLETE` ("a cube map
//! look-up whose unit has no complete cube map ... is not built yet"), which
//! the GL core received as `-ENOTSUP`, `GL_INVALID_OPERATION` and one more in
//! `draws_refused`. Mesa 26.2.1 binds a fallback texture of the target's kind
//! (`_mesa_get_fallback_texture`, src/mesa/main/texobj.c), six 1x1 black
//! faces for a cube, and ES 2.0 3.8.2 reads (0, 0, 0, 1) there
//! (docs/notes/mali-cube-textures.md). This test replays the GL sequences of
//! es2fTextureCompletenessTests.cpp (`IncompleteCubeSizeCase`,
//! `IncompleteCubeFormatCase`, `IncompleteCubeMissingLevelCase`, an empty and
//! an extra level, a non-positive level 0, and the wrap cases that set images
//! on the default cube texture while `GL_TEXTURE_2D` is bound) through the
//! real Mali back end over a mock of its OSAL, drawing the case's quad with a
//! `samplerCube` program. Every draw must leave `GL_NO_ERROR` and
//! `draws_refused` unchanged. No pixels are checked (the descriptor is in
//! src/mali/backend_tests.rs
//! `a_cube_look_up_without_a_cube_map_reads_a_six_face_black_cube`).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_cube_complete
//! ```

#![cfg(all(feature = "host-test", feature = "shaderc-online", feature = "gpu-mali"))]

use core::ffi::{c_char, c_void};
use std::ffi::CString;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use yakogl::backend;
use yakogl::gl::consts::*;
use yakogl::gl::draw::*;
use yakogl::gl::framebuffer::*;
use yakogl::gl::program::*;
use yakogl::gl::shader::*;
use yakogl::gl::state::*;
use yakogl::gl::texture::*;
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

/// The GL context and a 64x64 RGBA8 framebuffer object bound.
fn setup() {
    backend::HOST_DRAW_TARGETS.store(1, Ordering::Relaxed);
    // SAFETY: static tables and one context; this is the only test of the
    // file.
    unsafe {
        osal_set_ops(&OSAL);
        backend::g_backend = &yakogl::mali::backend::yakogl_backend_mali;
        assert_eq!(yakogl::mali::backend::mali_init(), 0);
        yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
        assert!(!yakogl::gl::context::gl_context_create(64, 64).is_null());
        let (mut fb, mut rb) = (0, 0);
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        glGenRenderbuffers(1, &mut rb);
        glBindRenderbuffer(GL_RENDERBUFFER, rb);
        glRenderbufferStorage(GL_RENDERBUFFER, GL_RGBA8, 64, 64);
        glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_RENDERBUFFER, rb);
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE);
        while glGetError() != GL_NO_ERROR {}
    }
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
        assert_eq!(ok, 1, "{text:?} did not compile");
        s
    }
}

const VS: &str = "attribute vec4 p;\nvarying vec3 v;\nvoid main() { v = vec3(p.xy, 1.0); gl_Position = p; }\n";
const FS: &str = "precision mediump float;\nuniform samplerCube s;\nvarying vec3 v;\n\
                  void main() { gl_FragColor = textureCube(s, v); }\n";

static QUAD: [f32; 16] = [-1.0, -1.0, 0.5, 1.0, 1.0, -1.0, 0.5, 1.0, -1.0, 1.0, 0.5, 1.0, 1.0, 1.0, 0.5, 1.0];

const FACES: [GLenum; 6] = [
    GL_TEXTURE_CUBE_MAP_POSITIVE_X,
    GL_TEXTURE_CUBE_MAP_NEGATIVE_X,
    GL_TEXTURE_CUBE_MAP_POSITIVE_Y,
    GL_TEXTURE_CUBE_MAP_NEGATIVE_Y,
    GL_TEXTURE_CUBE_MAP_POSITIVE_Z,
    GL_TEXTURE_CUBE_MAP_NEGATIVE_Z,
];

/// Every GL call of a sequence leaves no error.
fn no_error(what: &str, step: &str) {
    // SAFETY: plain GL call.
    let e = unsafe { glGetError() };
    assert_eq!(e, GL_NO_ERROR, "[{what}] {step}: glGetError 0x{e:x}");
}

fn param(target: GLenum, name: GLenum, v: GLenum) {
    // SAFETY: plain GL call.
    unsafe { glTexParameteri(target, name, v as GLint) };
}

fn image(target: GLenum, level: GLint, size: GLsizei, format: GLenum) {
    let texels = vec![0u8; (size.max(1) * size.max(1)) as usize * 4];
    // SAFETY: plain GL call; `texels` covers any size and format used here.
    unsafe {
        glTexImage2D(target, level, format as GLint, size, size, 0, format, GL_UNSIGNED_BYTE, texels.as_ptr().cast());
    }
}

/// The CTS's TextureRenderer::renderQuad with a cube program: one quad. The
/// draw must leave `GL_NO_ERROR` and not one more refusal.
fn render_quad(what: &str) {
    let before = backend::draws_refused();
    // SAFETY: plain GL calls; QUAD is a static.
    unsafe {
        glViewport(0, 0, 64, 64);
        let p = glCreateProgram();
        glAttachShader(p, shader(GL_VERTEX_SHADER, VS));
        glAttachShader(p, shader(GL_FRAGMENT_SHADER, FS));
        glBindAttribLocation(p, 0, c"p".as_ptr());
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        assert_eq!(ok, 1, "[{what}] link failed");
        glUseProgram(p);
        glUniform1i(glGetUniformLocation(p, c"s".as_ptr()), 0);
        glEnableVertexAttribArray(0);
        glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, QUAD.as_ptr() as *const c_void);
        glDrawArrays(GL_TRIANGLE_STRIP, 0, 4);
        glFinish();
    }
    no_error(what, "renderQuad");
    assert_eq!(backend::draws_refused() - before, 0, "[{what}] the cube draw was refused");
}

/// A fresh cube texture bound to unit 0, NEAREST_MIPMAP_NEAREST / NEAREST,
/// REPEAT: the state `IncompleteCube*Case::createTexture` sets.
fn new_cube() {
    // SAFETY: one name; plain GL calls.
    unsafe {
        let mut t: GLuint = 0;
        glGenTextures(1, &mut t);
        glPixelStorei(GL_UNPACK_ALIGNMENT, 1);
        glActiveTexture(GL_TEXTURE0);
        glBindTexture(GL_TEXTURE_CUBE_MAP, t);
    }
    param(GL_TEXTURE_CUBE_MAP, GL_TEXTURE_WRAP_S, GL_REPEAT);
    param(GL_TEXTURE_CUBE_MAP, GL_TEXTURE_WRAP_T, GL_REPEAT);
    param(GL_TEXTURE_CUBE_MAP, GL_TEXTURE_MIN_FILTER, GL_NEAREST_MIPMAP_NEAREST);
    param(GL_TEXTURE_CUBE_MAP, GL_TEXTURE_MAG_FILTER, GL_NEAREST);
}

/// `IncompleteCubeSizeCase`: 64x64 faces, the chain to level 6, a face (all
/// six when `face` is None) of `bad_size` at `bad_level`.
fn size_cube(bad_size: i32, bad_level: i32, face: Option<usize>) {
    new_cube();
    for l in 0..7 {
        for (f, target) in FACES.iter().enumerate() {
            let w = if l == bad_level && face.is_none_or(|b| b == f) { bad_size } else { (64 >> l).max(1) };
            image(*target, l, w, GL_RGBA);
        }
    }
}

/// `IncompleteCubeMissingLevelCase`: the chain without `level` on `face` (or
/// every face).
fn missing_level(level: i32, face: Option<usize>) {
    new_cube();
    for l in 0..7 {
        for (f, target) in FACES.iter().enumerate() {
            if l == level && face.is_none_or(|b| b == f) {
                continue;
            }
            image(*target, l, (64 >> l).max(1), GL_RGBA);
        }
    }
}

/// `IncompleteCubeFormatCase`: level 0 of `face` in `other`, the rest RGBA.
fn format_mismatch(face: usize, other: GLenum) {
    new_cube();
    for (f, target) in FACES.iter().enumerate() {
        image(*target, 0, 64, if f == face { other } else { GL_RGBA });
    }
}

/// The wrap cases (`IncompleteCubeWrapModeCase`): the texture is bound to
/// GL_TEXTURE_2D, and the state and the six 127x127 faces go to the cube
/// binding, the default cube texture (name 0).
fn wrap_cube(wrap_t: GLenum, wrap_s: GLenum) {
    // SAFETY: one name; plain GL calls.
    unsafe {
        let mut t: GLuint = 0;
        glGenTextures(1, &mut t);
        glPixelStorei(GL_UNPACK_ALIGNMENT, 1);
        glActiveTexture(GL_TEXTURE0);
        glBindTexture(GL_TEXTURE_CUBE_MAP, 0);
        glBindTexture(GL_TEXTURE_2D, t);
    }
    param(GL_TEXTURE_CUBE_MAP, GL_TEXTURE_WRAP_S, wrap_s);
    param(GL_TEXTURE_CUBE_MAP, GL_TEXTURE_WRAP_T, wrap_t);
    param(GL_TEXTURE_CUBE_MAP, GL_TEXTURE_MIN_FILTER, GL_NEAREST);
    param(GL_TEXTURE_CUBE_MAP, GL_TEXTURE_MAG_FILTER, GL_NEAREST);
    for target in FACES {
        image(target, 0, 127, GL_RGBA);
    }
}

fn extra_level() {
    size_cube(64, 0, None);
    for target in FACES {
        image(target, 7, 1, GL_RGBA);
    }
}

fn no_texture() {
    // SAFETY: plain GL call.
    unsafe { glBindTexture(GL_TEXTURE_CUBE_MAP, 0) };
}

/// One case's GL calls.
type Seq = Box<dyn Fn()>;

#[test]
fn incomplete_cube_maps_draw_the_black_cube() {
    setup();
    let cases: Vec<(&str, Seq)> = vec![
        // nothing bound: the default cube texture has no images
        ("no_texture", Box::new(no_texture)),
        ("empty_object", Box::new(new_cube)),
        ("extra_level", Box::new(extra_level)),
        ("missing_level_1", Box::new(|| missing_level(1, None))),
        ("missing_level_1_pos_y", Box::new(|| missing_level(1, Some(2)))),
        ("missing_level_3", Box::new(|| missing_level(3, None))),
        ("missing_level_3_neg_y", Box::new(|| missing_level(3, Some(3)))),
        ("format_mismatch_rgb_rgba_level_0", Box::new(|| format_mismatch(0, GL_RGB))),
        ("format_mismatch_rgba_rgb_level_0_neg_z", Box::new(|| format_mismatch(5, GL_RGB))),
        ("format_mismatch_luminance_luminance_alpha_level_0", Box::new(|| format_mismatch(1, GL_LUMINANCE_ALPHA))),
        ("not_positive_level_0", Box::new(|| size_cube(0, 0, None))),
        ("npot_size_level_0", Box::new(|| size_cube(63, 0, None))),
        ("npot_size_level_1", Box::new(|| size_cube(31, 1, None))),
        ("npot_size_level_0_pos_x", Box::new(|| size_cube(63, 0, Some(0)))),
        ("npot_size_level_1_neg_x", Box::new(|| size_cube(31, 1, Some(1)))),
        // images on the default cube texture; complete or not, the draw
        // must build
        ("npot_t_repeat", Box::new(|| wrap_cube(GL_REPEAT, GL_CLAMP_TO_EDGE))),
        ("npot_s_repeat", Box::new(|| wrap_cube(GL_CLAMP_TO_EDGE, GL_REPEAT))),
        ("npot_all_repeat", Box::new(|| wrap_cube(GL_REPEAT, GL_REPEAT))),
        ("npot_mirrored_repeat", Box::new(|| wrap_cube(GL_MIRRORED_REPEAT, GL_MIRRORED_REPEAT))),
    ];
    for (name, seq) in &cases {
        seq();
        no_error(name, "set texturing state");
        render_quad(name);
    }
    // a complete cube map is the control: six 1x1 faces
    new_cube();
    param(GL_TEXTURE_CUBE_MAP, GL_TEXTURE_MIN_FILTER, GL_NEAREST);
    for target in FACES {
        image(target, 0, 1, GL_RGBA);
    }
    render_quad("complete_control");
}
