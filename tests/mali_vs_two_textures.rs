// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! A vertex shader that reads several textures, 2D textures and cube maps,
//! draws on the Mali back end (issue #11308).
//!
//! The eight dEQP-GLES2 cases
//! `uniform_api.value.assigned.{by_value,by_pointer}.render.{basic_struct,
//! struct_in_array,array_in_struct,nested_structs_arrays}.sampler2D_samplerCube_vertex`
//! failed on the NanoPC-T6 at their draw with `GL_INVALID_OPERATION`: the
//! frame built the vertex stage's texture and sampler tables for one
//! texture and refused a vertex shader that reads more ("a vertex shader that
//! reads more than one texture is not built yet"). The frame now builds the
//! vertex tables as the fragment ones, texture i of sampler binding i, as
//! Mesa 26.2.1 does for every stage (`panfrost_emit_texture_descriptors`,
//! `panfrost_emit_sampler_descriptors`, pan_cmdstream.c:1934-1985,
//! 2022-2043). The tables against Mesa's words are src/mali/frame_tex_tests.rs
//! `a_vertex_2d_texture_and_cube_map_are_the_shim_s`.
//!
//! This test replays each case's GL sequence (es2fUniformApiTests.cpp
//! `UniformCase::renderTest` and `setupTexture`: every sampler of the
//! struct its own unit, each unit a 32x32 RGBA8 2D texture or cube map,
//! nearest, clamped) through the GL core on the real Mali back end over a
//! mock of its OSAL, with the case's own shaders
//! (tools/mali/testdata/deqp_vstex2_*). Every draw must leave `GL_NO_ERROR`
//! and `draws_refused` unchanged. No pixels are checked.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_vs_two_textures
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

const FS: &str = include_str!("../tools/mali/testdata/deqp_vstex2.frag");

/// A sampler of the case's struct: the name dEQP looks it up by, and
/// whether it is a samplerCube (else a sampler2D).
type Leaf = (&'static str, bool);

/// The four cases: the vertex shader and its samplers in the order dEQP
/// assigns them units 0, 1, ... (the .qpa's `glGetUniformLocation` lines).
const CASES: [(&str, &str, &[Leaf]); 4] = [
    (
        "basic_struct",
        include_str!("../tools/mali/testdata/deqp_vstex2_basic_struct.vert"),
        &[("u_var.m0", false), ("u_var.m1", true)],
    ),
    (
        "struct_in_array",
        include_str!("../tools/mali/testdata/deqp_vstex2_struct_in_array.vert"),
        &[
            ("u_var[0].m0", false),
            ("u_var[0].m1", true),
            ("u_var[1].m0", false),
            ("u_var[1].m1", true),
            ("u_var[2].m0", false),
            ("u_var[2].m1", true),
        ],
    ),
    (
        "array_in_struct",
        include_str!("../tools/mali/testdata/deqp_vstex2_array_in_struct.vert"),
        &[
            ("u_var.m0", false),
            ("u_var.m1", true),
            ("u_var.m2[0]", false),
            ("u_var.m2[1]", false),
            ("u_var.m2[2]", false),
            ("u_var.m3[0]", true),
            ("u_var.m3[1]", true),
            ("u_var.m3[2]", true),
        ],
    ),
    (
        "nested_structs_arrays",
        include_str!("../tools/mali/testdata/deqp_vstex2_nested_structs_arrays.vert"),
        &[
            ("u_var.m0", false),
            ("u_var.m1.ms0", true),
            ("u_var.m1.ms1[0]", false),
            ("u_var.m1.ms1[1]", false),
            ("u_var.m1.ms2[0].mss0", false),
            ("u_var.m1.ms2[0].mss1", true),
            ("u_var.m1.ms2[1].mss0", false),
            ("u_var.m1.ms2[1].mss1", true),
            ("u_var.m2", true),
        ],
    ),
];

const FACES: [GLenum; 6] = [
    GL_TEXTURE_CUBE_MAP_POSITIVE_X,
    GL_TEXTURE_CUBE_MAP_NEGATIVE_X,
    GL_TEXTURE_CUBE_MAP_POSITIVE_Y,
    GL_TEXTURE_CUBE_MAP_NEGATIVE_Y,
    GL_TEXTURE_CUBE_MAP_POSITIVE_Z,
    GL_TEXTURE_CUBE_MAP_NEGATIVE_Z,
];

/// es2fUniformApiTests.cpp `UniformCase::render`'s quad.
static POSITION: [f32; 16] = [-1.0, -1.0, 0.0, 1.0, -1.0, 1.0, 0.0, 1.0, 1.0, -1.0, 0.0, 1.0, 1.0, 1.0, 0.0, 1.0];
static INDICES: [u16; 6] = [0, 1, 2, 2, 1, 3];

/// Every GL call of a case leaves no error.
fn no_error(what: &str, step: &str) {
    // SAFETY: plain GL call.
    let e = unsafe { glGetError() };
    assert_eq!(e, GL_NO_ERROR, "[{what}] {step}: glGetError 0x{e:x}");
}

/// `UniformCase::setupTexture`: a 32x32 RGBA8 texture (each face of a cube
/// map) of one colour at `unit`, nearest, clamped to the edge.
fn fill_unit(unit: u32, cube: bool) {
    let texels = [0x80u8; 32 * 32 * 4];
    let target = if cube { GL_TEXTURE_CUBE_MAP } else { GL_TEXTURE_2D };
    // SAFETY: one name; plain GL calls; `texels` covers a 32x32 RGBA8 image.
    unsafe {
        let mut t: GLuint = 0;
        glGenTextures(1, &mut t);
        glActiveTexture(GL_TEXTURE0 + unit);
        glBindTexture(target, t);
        for image in if cube { &FACES[..] } else { &[GL_TEXTURE_2D][..] } {
            glTexImage2D(*image, 0, GL_RGBA as GLint, 32, 32, 0, GL_RGBA, GL_UNSIGNED_BYTE, texels.as_ptr().cast());
        }
        for (name, v) in [
            (GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE),
            (GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE),
            (GL_TEXTURE_MIN_FILTER, GL_NEAREST),
            (GL_TEXTURE_MAG_FILTER, GL_NEAREST),
        ] {
            glTexParameteri(target, name, v as GLint);
        }
    }
}

/// One case: the program, each sampler's unit set by value
/// (`glUniform1i`) or by pointer (`glUniform1iv`), its texture, the quad.
fn draw_case(name: &str, vs: &str, leaves: &[Leaf], by_pointer: bool) {
    let what = format!("{}.{name}", if by_pointer { "by_pointer" } else { "by_value" });
    let before = backend::draws_refused();
    // SAFETY: plain GL calls; the names are NUL-terminated, the arrays static.
    unsafe {
        let p = glCreateProgram();
        glAttachShader(p, shader(GL_VERTEX_SHADER, vs));
        glAttachShader(p, shader(GL_FRAGMENT_SHADER, FS));
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        assert_eq!(ok, 1, "[{what}] link failed");
        glUseProgram(p);
        for (unit, (leaf, _)) in leaves.iter().enumerate() {
            let n = CString::new(*leaf).unwrap();
            let loc = glGetUniformLocation(p, n.as_ptr());
            assert!(loc >= 0, "[{what}] no location for {leaf}");
            let unit = unit as GLint;
            if by_pointer {
                glUniform1iv(loc, 1, &unit);
            } else {
                glUniform1i(loc, unit);
            }
        }
        no_error(&what, "the sampler units");
        for (unit, (_, cube)) in leaves.iter().enumerate() {
            fill_unit(unit as u32, *cube);
        }
        no_error(&what, "the textures");
        glViewport(0, 0, 32, 32);
        let pos = glGetAttribLocation(p, c"a_position".as_ptr());
        assert!(pos >= 0, "[{what}] no a_position");
        glEnableVertexAttribArray(pos as GLuint);
        glVertexAttribPointer(pos as GLuint, 4, GL_FLOAT, 0, 0, POSITION.as_ptr().cast());
        glDrawElements(GL_TRIANGLES, 6, GL_UNSIGNED_SHORT, INDICES.as_ptr().cast());
        no_error(&what, "glDrawElements");
        glFinish();
    }
    assert_eq!(backend::draws_refused() - before, 0, "[{what}] the draw was refused");
}

#[test]
fn vertex_shaders_reading_several_textures_draw() {
    setup();
    for by_pointer in [false, true] {
        for (name, vs, leaves) in CASES {
            assert!(leaves.iter().any(|l| l.1) && leaves.iter().any(|l| !l.1), "{name}: a 2D texture and a cube map");
            draw_case(name, vs, leaves, by_pointer);
        }
    }
}
