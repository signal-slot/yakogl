// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! D-DEPTHTEX on the Mali, through the GL API on the host: glmark2 shadow's
//! shape. A first pass renders into a depth-only framebuffer (a
//! GL_DEPTH_COMPONENT texture alone as GL_DEPTH_ATTACHMENT, no colour); a
//! second program samples that texture while drawing to the default
//! framebuffer. The real Mali back end runs over a mock OSAL (no GPU runs, so
//! the depth plane holds what was never written); the test wraps its draw
//! entry to log what each draw was given. It checks no GL error, a complete
//! FBO, the depth-only draw without a colour surface, the texture the second
//! draw binds (complete, R32F: the format `mali_depth_resolve` writes and
//! panfrost samples), and that no draw is refused.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_depthtex_gl
//! ```

#![cfg(all(feature = "host-test", feature = "shaderc-online", feature = "gpu-mali"))]

use core::ffi::{c_char, c_void};
use std::ffi::CString;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{self, GlesDraw, YakoglBackend};
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

static FENCES: AtomicI64 = AtomicI64::new(0);

/// Buffers made so far: each one's GPU VA is its number times 16 MiB, far
/// from any CPU address (the frame wants page-aligned VAs); `BUFFERS` keeps
/// each one's CPU view so the test can read the texels a draw was handed.
static BUFFERS: Mutex<Vec<(u64, usize, usize)>> = Mutex::new(Vec::new());

unsafe extern "C" fn m_alloc(_heap: u64, size: usize, _flags: u32, out: *mut OsalGpuMem) -> i32 {
    assert!(size <= 16 << 20);
    let cpu = Box::leak(vec![0u64; size.div_ceil(8)].into_boxed_slice()).as_mut_ptr().cast::<c_void>();
    let mut b = BUFFERS.lock().unwrap();
    let gpu = 0x40_0000_0000 + (b.len() as u64 + 1) * (16 << 20);
    b.push((gpu, cpu as usize, size));
    *out = OsalGpuMem { cpu, gpu, phys: 0, size, impl_: cpu };
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

/// What the back end was given for one draw, recorded at its entry.
#[derive(Clone, Copy, Debug)]
struct Seen {
    index_count: u32,
    target_gpu: u64,
    depth_test: bool,
    depth_write: bool,
    num_textures: u8,
    tex_format: u8,
    tex_incomplete: u8,
    tex_base_gpu: u64,
    tex_w: u32,
    tex_h: u32,
    /// `mali_draw`'s answer.
    mali: i32,
}

static SEEN: Mutex<Vec<Seen>> = Mutex::new(Vec::new());

/// The real Mali draw, with its inputs and verdict logged.
unsafe extern "C" fn b_draw(d: *const GlesDraw) -> i32 {
    let dr = &*d;
    let mali = yakogl::mali::backend::mali_draw(d);
    let t = &dr.textures[0];
    SEEN.lock().unwrap().push(Seen {
        index_count: dr.index_count,
        target_gpu: dr.target_gpu,
        depth_test: dr.depth_test,
        depth_write: dr.depth_write,
        num_textures: dr.num_textures,
        tex_format: t.format,
        tex_incomplete: t.incomplete,
        tex_base_gpu: t.base_gpu,
        tex_w: t.width,
        tex_h: t.height,
        mali,
    });
    mali
}

static LOCK: Mutex<()> = Mutex::new(());
static ONCE: Once = Once::new();

const N: GLsizei = 32;

/// One Mali context for the file; the tests take turns on it.
fn context() -> MutexGuard<'static, ()> {
    let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    ONCE.call_once(|| {
        backend::HOST_DRAW_TARGETS.store(1, Ordering::Relaxed);
        // SAFETY: the Mali table is a static of plain fields and function
        // pointers, copied bit for bit with only `draw` replaced; the OSAL
        // and the copy is leaked; the lock serialises the tests.
        unsafe {
            osal_set_ops(&OSAL);
            let mut b: YakoglBackend = core::ptr::read(&yakogl::mali::backend::yakogl_backend_mali);
            b.draw = Some(b_draw);
            backend::g_backend = Box::leak(Box::new(b));
            assert_eq!(yakogl::mali::backend::mali_init(), 0);
            yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
            assert!(!yakogl::gl::context::gl_context_create(N as u32, N as u32).is_null());
            glViewport(0, 0, N, N);
        }
    });
    SEEN.lock().unwrap().clear();
    // SAFETY: plain GL call.
    while unsafe { glGetError() } != GL_NO_ERROR {}
    g
}

fn ok(what: &str) {
    // SAFETY: plain GL call.
    let e = unsafe { glGetError() };
    assert_eq!(e, GL_NO_ERROR, "{what}: glGetError 0x{e:x}");
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
        let mut log = [0u8; 512];
        let mut n: GLsizei = 0;
        glGetShaderInfoLog(s, log.len() as GLsizei, &mut n, log.as_mut_ptr().cast());
        assert_eq!(ok, 1, "{text:?} did not compile: {}", String::from_utf8_lossy(&log[..n as usize]));
        s
    }
}

fn program(vs: &str, fs: &str) -> GLuint {
    let v = shader(GL_VERTEX_SHADER, vs);
    let f = shader(GL_FRAGMENT_SHADER, fs);
    // SAFETY: plain GL calls.
    unsafe {
        let p = glCreateProgram();
        glAttachShader(p, v);
        glAttachShader(p, f);
        glBindAttribLocation(p, 0, c"p".as_ptr());
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        assert_eq!(ok, 1, "link");
        p
    }
}

/// The first pass's shaders: glmark2 shadow's depth pass writes only depth.
const VS1: &str = "attribute vec4 p;\nvoid main() { gl_Position = p; }\n";
const FS1: &str = "precision mediump float;\nvoid main() { gl_FragColor = vec4(1.0); }\n";
/// The second pass: the depth texture's look-up as the colour (a whole vec4: a lane alone is
/// the TEX_SWIZZLE refusal at the tip).
const VS2: &str = "attribute vec4 p;\nvarying vec2 uv;\nvoid main() { gl_Position = p; uv = p.xy * 0.5 + 0.5; }\n";
const FS2: &str = "precision mediump float;\nuniform sampler2D shadow;\nvarying vec2 uv;\nvoid main() { gl_FragColor = texture2D(shadow, uv); }\n";

/// A quad of constant depth `z` as two triangles.
fn quad(z: f32, what: &str) -> Seen {
    let p: [f32; 16] = [-1.0, -1.0, z, 1.0, -1.0, 1.0, z, 1.0, 1.0, -1.0, z, 1.0, 1.0, 1.0, z, 1.0];
    static IDX: [u16; 6] = [0, 2, 1, 2, 1, 3];
    SEEN.lock().unwrap().clear();
    let before = backend::draws_refused();
    // SAFETY: `p` and `IDX` outlive the draw, which `glFinish` completes.
    let e = unsafe {
        glBindBuffer(GL_ARRAY_BUFFER, 0);
        glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, p.as_ptr() as *const c_void);
        glEnableVertexAttribArray(0);
        glDrawElements(GL_TRIANGLES, 6, GL_UNSIGNED_SHORT, IDX.as_ptr() as *const c_void);
        glFinish();
        glGetError()
    };
    assert_eq!(e, GL_NO_ERROR, "{what}: glGetError 0x{e:x}, draws {:?}", SEEN.lock().unwrap());
    assert_eq!(backend::draws_refused(), before, "{what}: a draw was refused");
    let s = SEEN.lock().unwrap().iter().rev().find(|s| s.index_count == 6).copied();
    let s = s.unwrap_or_else(|| panic!("{what}: the draw did not reach the back end"));
    assert_eq!(s.mali, 0, "{what}: mali_draw refused");
    s
}

/// shadow's first pass: a depth texture of sized `internal` format and
/// pixel type `ty` (an unsized one is no attachment in the default ES 3.0
/// report, ES 3.0 4.4.4.2; glmark2's ES 2.0 report is `report-es2`), attached alone as
/// GL_DEPTH_ATTACHMENT, drawn into with the depth test on. Returns its name.
fn depth_pass(internal: GLenum, ty: GLenum, z: f32) -> GLuint {
    let (mut tex, mut fbo) = (0, 0);
    // SAFETY: room for one name each; null pixels.
    unsafe {
        glActiveTexture(GL_TEXTURE0);
        glGenTextures(1, &mut tex);
        glBindTexture(GL_TEXTURE_2D, tex);
        glTexImage2D(GL_TEXTURE_2D, 0, internal as GLint, N, N, 0, GL_DEPTH_COMPONENT, ty, core::ptr::null());
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST as GLint);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST as GLint);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE as GLint);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE as GLint);
    }
    ok("depth texture");
    // SAFETY: plain GL calls.
    unsafe {
        glGenFramebuffers(1, &mut fbo);
        glBindFramebuffer(GL_FRAMEBUFFER, fbo);
        glFramebufferTexture2D(GL_FRAMEBUFFER, GL_DEPTH_ATTACHMENT, GL_TEXTURE_2D, tex, 0);
    }
    ok("attach");
    // SAFETY: plain GL call.
    let s = unsafe { glCheckFramebufferStatus(GL_FRAMEBUFFER) };
    assert_eq!(s, GL_FRAMEBUFFER_COMPLETE, "depth-only FBO: status 0x{s:x}");
    let p = program(VS1, FS1);
    // SAFETY: plain GL calls.
    unsafe {
        glUseProgram(p);
        glViewport(0, 0, N, N);
        glClearDepthf(1.0);
        glClear(GL_DEPTH_BUFFER_BIT);
        glEnable(GL_DEPTH_TEST);
        glDepthFunc(GL_LESS);
    }
    ok("clear");
    let d = quad(z, "depth pass");
    assert_eq!(d.target_gpu, 0, "the depth-only pass has no colour surface");
    assert!(d.depth_test && d.depth_write, "the depth pass tests and writes depth");
    // SAFETY: plain GL calls.
    unsafe {
        glDisable(GL_DEPTH_TEST);
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
    }
    ok("unbind");
    tex
}

/// shadow's second pass: the depth texture on unit 0, drawn to the default
/// framebuffer.
fn sample_pass(tex: GLuint) -> Seen {
    let p = program(VS2, FS2);
    // SAFETY: plain GL calls.
    unsafe {
        glUseProgram(p);
        glActiveTexture(GL_TEXTURE0);
        glBindTexture(GL_TEXTURE_2D, tex);
        glUniform1i(glGetUniformLocation(p, c"shadow".as_ptr()), 0);
    }
    ok("second pass setup");
    quad(0.0, "sample pass")
}

fn texel_at(s: &Seen, x: u32, y: u32) -> f32 {
    let off = ((y * s.tex_w + x) * 4) as u64;
    let b = BUFFERS.lock().unwrap();
    let &(gpu, cpu, size) =
        b.iter().find(|&&(g, _, n)| (g..g + n as u64).contains(&s.tex_base_gpu)).expect("the texture's buffer");
    let at = (s.tex_base_gpu - gpu + off) as usize;
    assert!(at + 4 <= size);
    // SAFETY: inside the leaked buffer, 4-byte aligned (R32F texels).
    unsafe { *((cpu + at) as *const f32) }
}

#[test]
fn depth_only_pass_then_sampled_by_second_pass() {
    let _g = context();
    let tex = depth_pass(GL_DEPTH_COMPONENT24, GL_UNSIGNED_INT, 0.0);
    let s = sample_pass(tex);
    assert_ne!(s.target_gpu, 0, "the second pass draws to the default framebuffer");
    assert_eq!(s.num_textures, 1);
    assert_eq!(s.tex_incomplete, 0, "the depth texture is complete");
    assert_eq!((s.tex_w, s.tex_h), (N as u32, N as u32));
    // panfrost samples a depth texture through its resolved R32_FLOAT image
    // (mali_depth_resolve; docs/notes/mali-texture-refusals.md (c)).
    assert_eq!(s.tex_format, backend::GLES_TEXFMT_R32F);
    assert_eq!(backend::draws_refused(), 0);
}

#[test]
fn sampled_depth_texels_are_resolved() {
    let _g = context();
    // The mock GPU runs nothing, so the Z/S write-back holds zeros; the
    // texel can only show that the resolve ran and left a finite depth.
    let tex = depth_pass(GL_DEPTH_COMPONENT16, GL_UNSIGNED_SHORT, 0.5);
    let s = sample_pass(tex);
    let t = texel_at(&s, N as u32 / 2, N as u32 / 2);
    eprintln!("resolved texel: {t}");
    assert!(t.is_finite() && (0.0..=1.0).contains(&t), "texel {t}");
}

#[test]
fn depth_texture_resampled_without_a_new_depth_draw() {
    let _g = context();
    let tex = depth_pass(GL_DEPTH_COMPONENT24, GL_UNSIGNED_INT, 0.25);
    let a = sample_pass(tex);
    let b = sample_pass(tex);
    assert_eq!((a.tex_format, a.tex_w, a.tex_h), (b.tex_format, b.tex_w, b.tex_h));
    assert_eq!(backend::draws_refused(), 0);
}
