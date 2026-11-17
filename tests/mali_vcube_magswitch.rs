// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The vertex-shader cube look-up switches to magnification at the ES 2.0
//! level of detail (draft of item vcube-magswitch, plan #373 cause 7).
//!
//! Six dEQP-GLES2 cases, texture.vertex.cube.filtering.nearest_mipmap_
//! {nearest,linear}_linear_{clamp,mirror,repeat}, still give wrong pixels on
//! the Mali-G610 though their sampler descriptor equals Mesa 26.2.1's
//! (`tests/mali_cube_samplers.rs`). The cause is where filtering turns from
//! minification to magnification. ES 2.0 3.7.8 and dEQP's reference
//! (`tcuTexture.cpp`) switch at lambda = 0.5 when the magnification filter is
//! LINEAR and the minification filter is NEAREST_MIPMAP_NEAREST or
//! NEAREST_MIPMAP_LINEAR (the 0.5 offset keeps a LINEAR magnification at
//! the texel centres the NEAREST minification would have chosen); in every
//! other state (mag NEAREST, or min LINEAR_MIPMAP_*, or no mipmap filter) the
//! switch is at 0, which is the hardware's. So for 0 < lambda <= 0.5 the
//! hardware uses the minification filter where the reference uses the
//! magnification one. The brief's rule is right as it stands. The fix is the
//! level remap `lod <= 0.5 ? -1.0 : lod` of `lodkey::remap_mag_switch`
//! applied by the Mali vertex compile; this file only pins it.
//!
//! What the assertions rest on. The mock gives the test the buffers it
//! handed out, not a decoder of them, and it does not show the compiler's
//! lowering. The Mali back end reuses its arenas (code, data, descriptors)
//! from one draw to the next, so the test snapshots those persistent buffers
//! after each draw, zeroes every sampler-shaped 32-byte chunk (the filters
//! live there and must not count), and compares two states of the same
//! vertex shader (the reference's `vcube.vert`, whose level is the uniform
//! `u_lod`, so the remap is a run-time select): the bytes that differ and
//! are not also different between two draws of one state (the back end's
//! bookkeeping) are the program's code. That is the strongest the mock
//! gives: the instruction words themselves, not a descriptor. A level
//! remap adds the compare and the select for 0.5 to the code, so
//!
//! - fix cases (ignored): the code of mag LINEAR with min NEAREST_MIPMAP_
//!   NEAREST (case 2), and with min NEAREST_MIPMAP_LINEAR (case 1), differs
//!   from the code of a state that needs no remap, and the two are the same
//!   remap;
//! - controls (active): every state that needs none (mag NEAREST with
//!   min NEAREST_MIPMAP_*, min LINEAR_MIPMAP_*, no mipmap filter) gets one
//!   and the same code, and the probe does see a program change (a different
//!   level expression in the source), and sees nothing in a repeated draw.
//!
//! The level in (0, 0.5] is not a test input: with a uniform level the
//! remap is one program for every value, so the draws all use 0.19 (dEQP's
//! value). A constant level in the source is no use here: the online compiler
//! refuses a constant look-up level in this vertex shader today (a compiler
//! refusal of the M.2 kind, not this test's).
//!
//! Caveat for vcube-magswitch: the GL core asks the back end for the key
//! only where `online::backend::sensitivity_of` reports a sensitivity, and
//! it does so only in a `report-es2` build (the dEQP board library); in the
//! build of this file's header, without it, the fix cases cannot pass unless
//! the item also lets the Mali ask in every build. The file's cfg does not
//! exclude `report-es2`, so the cases can be run there too.
//!
//! ```text
//! cargo test --test mali_vcube_magswitch --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table
//! ```

#![cfg(all(feature = "host-test", feature = "shaderc-online", feature = "gpu-mali"))]

use core::ffi::{c_char, c_void};
use std::ffi::CString;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::sync::Mutex;
use yakogl::backend;
use yakogl::gl::buffer::glBindBuffer;
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
/// from any CPU address. Nothing is given back (a few MiB for the run). Each
/// one's CPU address and size are kept so the test can read them.
static ALLOCS: AtomicU64 = AtomicU64::new(0);
static FENCES: AtomicI64 = AtomicI64::new(0);
static BUFFERS: Mutex<Vec<(usize, usize)>> = Mutex::new(Vec::new());

unsafe extern "C" fn m_alloc(_heap: u64, size: usize, _flags: u32, out: *mut OsalGpuMem) -> i32 {
    let n = ALLOCS.fetch_add(1, Ordering::Relaxed) + 1;
    assert!(size <= 16 << 20);
    let cpu = Box::leak(vec![0u64; size.div_ceil(8)].into_boxed_slice()).as_mut_ptr().cast::<c_void>();
    BUFFERS.lock().unwrap().push((cpu as usize, size));
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

/* ---- the GL calls --------------------------------------------------------- */

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
        let (mut log, mut n) = ([0u8; 512], 0);
        glGetShaderInfoLog(s, log.len() as GLsizei, &mut n, log.as_mut_ptr().cast());
        let log = String::from_utf8_lossy(&log[..n.max(0) as usize]).into_owned();
        assert_eq!(ok, 1, "{text:?} did not compile: {log}");
        s
    }
}

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

static POS2: [f32; 6] = [-1.0, -1.0, 3.0, -1.0, -1.0, 3.0];
static DIR: [f32; 9] = [-1.0, -1.0, 1.0, 1.0, -0.5, 1.0, -1.0, 0.5, 1.0];

/// Link `vert` and `frag`, bind `pos` (`pos_comps` components) at 0 and the
/// cube direction at 1, set the uniforms (`set`, with the program current)
/// and draw. Returns the draw's `glGetError` and its refusals.
fn draw(vert: &str, frag: &str, pos: &[f32], pos_comps: i32, set: impl FnOnce(GLuint)) -> (GLenum, u32) {
    let (v, f) = (shader(GL_VERTEX_SHADER, vert), shader(GL_FRAGMENT_SHADER, frag));
    let before = backend::draws_refused();
    // SAFETY: plain GL calls; the vertex arrays are static.
    let e = unsafe {
        let p = glCreateProgram();
        glAttachShader(p, v);
        glAttachShader(p, f);
        glBindAttribLocation(p, 0, c"a_position".as_ptr());
        glBindAttribLocation(p, 1, c"a_texCoord".as_ptr());
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        assert_eq!(ok, 1, "link failed");
        glUseProgram(p);
        set(p);
        glBindBuffer(GL_ARRAY_BUFFER, 0);
        glVertexAttribPointer(0, pos_comps, GL_FLOAT, 0, 0, pos.as_ptr().cast());
        glEnableVertexAttribArray(0);
        glVertexAttribPointer(1, 3, GL_FLOAT, 0, 0, DIR.as_ptr().cast());
        glEnableVertexAttribArray(1);
        while glGetError() != GL_NO_ERROR {}
        glDrawArrays(GL_TRIANGLES, 0, 3);
        glFinish();
        glGetError()
    };
    (e, backend::draws_refused() - before)
}

fn uniform(p: GLuint, name: &std::ffi::CStr) -> GLint {
    // SAFETY: a plain GL call with a NUL-terminated name.
    let l = unsafe { glGetUniformLocation(p, name.as_ptr()) };
    assert!(l >= 0, "{name:?} is no uniform");
    l
}

const VCUBE_VERT: &str = include_str!("../tools/mali/testdata/pan_tex_ref/vcube.vert");
const VCUBE_FRAG: &str = include_str!("../tools/mali/testdata/pan_tex_ref/vcube.frag");

/// The cube map of dEQP's vertex cube cases: 32x32 RGB, six levels, every
/// face of every level uploaded, wraps clamp. Unit 0 is left bound; one
/// object serves every draw, so the texture descriptor never moves.
fn cube_texture() {
    const FACES: [GLenum; 6] = [
        GL_TEXTURE_CUBE_MAP_POSITIVE_X,
        GL_TEXTURE_CUBE_MAP_NEGATIVE_X,
        GL_TEXTURE_CUBE_MAP_POSITIVE_Y,
        GL_TEXTURE_CUBE_MAP_NEGATIVE_Y,
        GL_TEXTURE_CUBE_MAP_POSITIVE_Z,
        GL_TEXTURE_CUBE_MAP_NEGATIVE_Z,
    ];
    // SAFETY: plain GL calls; the texel arrays outlive each call.
    unsafe {
        let mut t = 0;
        glGenTextures(1, &mut t);
        glActiveTexture(GL_TEXTURE0);
        glBindTexture(GL_TEXTURE_CUBE_MAP, t);
        glPixelStorei(GL_UNPACK_ALIGNMENT, 1);
        for level in 0..6 {
            let s = 32 >> level;
            let texels = vec![0x80u8; (s * s * 3) as usize];
            for face in FACES {
                glTexImage2D(face, level, GL_RGB as GLint, s, s, 0, GL_RGB, GL_UNSIGNED_BYTE, texels.as_ptr().cast());
            }
        }
        glTexParameteri(GL_TEXTURE_CUBE_MAP, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE as GLint);
        glTexParameteri(GL_TEXTURE_CUBE_MAP, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE as GLint);
    }
}

/// Whether a 32-byte chunk is shaped like a packed sampler: type `Sampler`
/// (1), reduction mode and the reserved bits of word 0 clear, `Normalized
/// Coordinates` set, bits 88..=127 clear and a zero border colour.
fn sampler_shaped(c: &[u8; 32]) -> bool {
    let w0 = u32::from_le_bytes(c[0..4].try_into().unwrap());
    let w2 = u32::from_le_bytes(c[8..12].try_into().unwrap());
    w0 & 0xf == 1 && w0 & 0x30 == 0 && w0 & (1 << 25) != 0 && w2 >> 24 == 0 && c[12..16] == [0; 4] && c[16..] == [0; 16]
}

/// How many buffers the mock had made after the warm-up draw: the arenas the
/// back end reuses from one draw to the next. Only these are compared.
static PERSISTENT: AtomicU64 = AtomicU64::new(0);

/// The persistent buffers' bytes with every sampler-shaped chunk zeroed.
fn snapshot() -> Vec<u8> {
    let mut out = Vec::new();
    let n = PERSISTENT.load(Ordering::Relaxed) as usize;
    for &(cpu, size) in BUFFERS.lock().unwrap().iter().take(n) {
        // SAFETY: a leaked allocation of `size` bytes; nothing draws now.
        let s = unsafe { core::slice::from_raw_parts(cpu as *const u8, size) };
        for chunk in s.chunks(32) {
            match <&[u8; 32]>::try_from(chunk) {
                Ok(c) if sampler_shaped(c) => out.extend_from_slice(&[0; 32]),
                _ => out.extend_from_slice(chunk),
            }
        }
    }
    out
}

/// Set the unit-0 cube's filters, draw `vert` with the reference fragment
/// shader (`u_lod` = `lod` when the shader has it), and return the
/// persistent buffers after it.
fn drawn(vert: &str, min: GLenum, mag: GLenum, lod: f32) -> Vec<u8> {
    // SAFETY: plain GL calls.
    unsafe {
        glTexParameteri(GL_TEXTURE_CUBE_MAP, GL_TEXTURE_MIN_FILTER, min as GLint);
        glTexParameteri(GL_TEXTURE_CUBE_MAP, GL_TEXTURE_MAG_FILTER, mag as GLint);
    }
    let (e, refused) = draw(vert, VCUBE_FRAG, &POS2, 2, |p| unsafe {
        glUniform1i(uniform(p, c"u_texture"), 0);
        if vert.contains("u_lod") {
            glUniform1f(uniform(p, c"u_lod"), lod);
        }
    });
    assert_eq!((e, refused), (GL_NO_ERROR, 0), "the draw was refused or failed");
    snapshot()
}

/// One GL state of the unit-0 cube: (minification, magnification).
type State = (GLenum, GLenum);

/// The one GL context of the file (the tests share it, one at a time) and
/// the count of draws, which numbers the comment that makes each draw's
/// shader source new.
static LOCK: Mutex<()> = Mutex::new(());
static SETUP: std::sync::Once = std::sync::Once::new();
static DRAWS: AtomicU64 = AtomicU64::new(0);

/// Take the file's lock and make the context, the cube map and the
/// persistent buffer set (after one warm-up draw) the first time.
fn fixture() -> std::sync::MutexGuard<'static, ()> {
    let guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    SETUP.call_once(|| {
        setup();
        cube_texture();
        drawn(&source(VCUBE_VERT), GL_NEAREST, GL_NEAREST, 0.19);
        PERSISTENT.store(BUFFERS.lock().unwrap().len() as u64, Ordering::Relaxed);
    });
    guard
}

/// `vert` with a trailing comment no other draw has: a source the shader
/// cache has not seen, with the same code.
fn source(vert: &str) -> String {
    format!("{vert}\n// draw {}\n", DRAWS.fetch_add(1, Ordering::Relaxed))
}

/// The bytes that differ between a draw of `vert_a` in state `a` and a draw
/// of `vert_b` in state `b`, outside the bytes that differ between two draws
/// of `vert_a` in state `a` (the back end's per-draw bookkeeping: kick and
/// fence numbers, addresses of fresh descriptors). Sampler descriptors are
/// zeroed in the snapshots, so the filters themselves do not count: what is
/// left is the program's code, its constants and its uniforms.
fn code_diff(vert_a: &str, a: State, vert_b: &str, b: State) -> usize {
    let first = drawn(&source(vert_a), a.0, a.1, 0.19);
    let again = drawn(&source(vert_a), a.0, a.1, 0.19);
    let other = drawn(&source(vert_b), b.0, b.1, 0.19);
    assert!(first.len() == again.len() && first.len() == other.len());
    (0..first.len()).filter(|&i| first[i] != other[i] && first[i] == again[i]).count()
}

/// Whether the code a draw of the reference vertex shader gets in state `b`
/// differs from the code it gets in state `a`.
fn state_changes_code(a: State, b: State) -> bool {
    code_diff(VCUBE_VERT, a, VCUBE_VERT, b) != 0
}

const NN: GLenum = GL_NEAREST_MIPMAP_NEAREST;
const NL: GLenum = GL_NEAREST_MIPMAP_LINEAR;
const LN: GLenum = GL_LINEAR_MIPMAP_NEAREST;
const LL: GLenum = GL_LINEAR_MIPMAP_LINEAR;
/// States that need no remap: mag NEAREST, or min LINEAR_MIPMAP_*, or no
/// mipmap filter at all. (ES 2.0 3.7.8 puts the magnification switch at
/// c = 0.5 only for mag LINEAR with min NEAREST_MIPMAP_*; the hardware
/// switches at 0 and is right for the others.)
const PLAIN: [State; 7] = [
    (NN, GL_NEAREST),
    (NL, GL_NEAREST),
    (LN, GL_NEAREST),
    (LN, GL_LINEAR),
    (LL, GL_LINEAR),
    (GL_LINEAR, GL_LINEAR),
    (GL_NEAREST, GL_LINEAR),
];

/// The probe sees the vertex program: a different look-up level in the
/// source is a different program, and it shows outside the noise.
#[test]
fn the_probe_sees_the_vertex_program() {
    let _g = fixture();
    let other = VCUBE_VERT.replace("u_lod)", "u_lod * 2.0 + 0.25)");
    assert_ne!(other, VCUBE_VERT);
    assert!(code_diff(VCUBE_VERT, (NN, GL_LINEAR), &other, (NN, GL_LINEAR)) > 0);
    // and the same program drawn again shows nothing
    assert_eq!(code_diff(VCUBE_VERT, (NN, GL_LINEAR), VCUBE_VERT, (NN, GL_LINEAR)), 0);
}

/// Controls (3): no state without "mag LINEAR and min NEAREST_MIPMAP_*"
/// changes the program, whatever its filters: every one of them gets the
/// code of the first (equality is transitive; each draw compiles a new
/// source, and the compiler's memory for them is bounded).
#[test]
fn states_needing_no_remap_get_the_same_vertex_code() {
    let _g = fixture();
    let a = PLAIN[0];
    for b in PLAIN {
        assert!(!state_changes_code(a, b), "min {:#x} mag {:#x} against min {:#x} mag {:#x}", a.0, a.1, b.0, b.1);
    }
}

/// The ES 2.0 magnification switch (3.7.8, dEQP's `tcuTexture.cpp`): a
/// look-up at λ in (0, 0.5] magnifies when mag is LINEAR and min is
/// NEAREST_MIPMAP_*. The vertex program of the cube look-up must carry the
/// remap (`lod <= 0.5 ? -1.0 : lod`, `lodkey::remap_mag_switch`) in this
/// state: its code differs from the code of a state that needs none.
fn assert_remapped(min: GLenum) {
    let _g = fixture();
    for plain in [PLAIN[0], PLAIN[4]] {
        assert!(
            state_changes_code(plain, (min, GL_LINEAR)),
            "min {min:#x} mag LINEAR: the vertex code equals that of min {:#x} mag {:#x}: no switch at 0.5",
            plain.0,
            plain.1
        );
    }
}

#[test]
#[ignore = "needs vcube-magswitch (ES 2.0 magnification switch at lod 0.5)"]
fn nearest_mipmap_linear_with_mag_linear_remaps_the_lod() {
    assert_remapped(NL);
}

#[test]
#[ignore = "needs vcube-magswitch (ES 2.0 magnification switch at lod 0.5)"]
fn nearest_mipmap_nearest_with_mag_linear_remaps_the_lod() {
    assert_remapped(NN);
}

/// The two keyed states differ from each other in sampler words only (the
/// descriptor), not in the program: both carry the same remap.
#[test]
#[ignore = "needs vcube-magswitch (ES 2.0 magnification switch at lod 0.5)"]
fn both_nearest_mipmap_filters_get_the_same_remap() {
    let _g = fixture();
    assert!(state_changes_code((GL_NEAREST, GL_LINEAR), (NN, GL_LINEAR)), "the remap is missing");
    assert!(!state_changes_code((NN, GL_LINEAR), (NL, GL_LINEAR)));
}
