// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! ES 3.0 multisample resolve, the sRGB / RGB10_A2 / 16-bit format rules, on
//! the 4x stand-in back end of tests/es3_msaa_gl.rs
//! (docs/notes/es3-msaa-renderbuffers.md (b), (f1)-(f4)).
//!
//! The stand-in's `finish` does the resolve; these tests pin only what the GL
//! core decides (the error of a blit) and what it copies (the bytes, for a
//! same-format resolve). No averaging space is asserted — neither linear nor
//! encoded: that is the GPU's, and note (f) reads it differently per GPU
//! (V3D: sRGB and RGB10_A2 sit as 16F halves in the tile buffer, so the TLB
//! averages linear for sRGB and the unorm value for RGB10_A2,
//! `v3dx_format_table.c:250-259`; PowerVR: the PBE's F16 source with `gamma`
//! on for sRGB, `pvr_arch_job_common.c:56-60`; GC7000UL: the RS's
//! `A8R8G8B8`, the encoded bytes; Mali: the blitter on the linear view, the
//! encoded values, `pan_blit.c:91`). The dEQP row compares with a tolerance
//! (f), which is what a board run measures.
//!
//! ES 3.0 4.3.3 asks a resolve for identical colour formats. The core folds
//! sRGB into linear first (`linear_format`, src/gl/blit.rs) as Mesa's
//! `compatible_resolve_formats` does ("Blits between linear and sRGB formats
//! are also allowed", src/mesa/main/blit.c), and tests/es3_msaa_gl.rs and
//! tests/es3_msaa_gl_formats.rs pin that fold as intended: the two folding
//! pairs below pass with the fold, and a strict-ES-3.0 test of each is
//! `#[ignore]`d. Everything else of ES 3.0 4.3.3 — RGB10_A2 or a 16-bit
//! format against a different format, in either direction — the core does
//! enforce, and is pinned passing, on renderbuffers and on sRGB textures.
//! One texture case is build-dependent and `#[ignore]`d: an attachment is
//! judged by the format it stores, and an RGB10_A2 texture stores RGBA8
//! outside gpu-v3d (note (d)), so its resolve rule is the back end's storage
//! choice, not the core's.
//!
//! The helpers are those of es3_msaa_gl.rs (not shared: each test file is its
//! own crate). A test the core fails today is `#[ignore = "reason"]`.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_msaa_gl_srgb
//! ```

#![cfg(all(feature = "host-test", feature = "shaderc-online", not(feature = "report-es2")))]
#![allow(dead_code)]

use core::ffi::{c_char, c_void};
use std::ffi::CString;
use std::sync::atomic::Ordering;
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{self, GlesDraw, GlesHeap, YakoglBackend};
use yakogl::gl::buffer::glBindBuffer;
use yakogl::gl::consts::*;
use yakogl::gl::draw::*;
use yakogl::gl::framebuffer::*;
use yakogl::gl::program::*;
use yakogl::gl::shader::*;
use yakogl::gl::state::*;
use yakogl::gl::texture::{glBindTexture, glDeleteTextures, glGenTextures, glTexImage2D};
use yakogl::gl::types::*;
use yakogl::gl::vertex::{glEnableVertexAttribArray, glVertexAttribPointer};
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};

/* ---- a context on the host: the smallest OSAL and back end that make one -- */

unsafe extern "C" fn m_gpu_alloc(_heap: u64, size: usize, _flags: u32, out: *mut OsalGpuMem) -> i32 {
    let mem = Box::leak(vec![0u8; size].into_boxed_slice());
    let p = mem.as_mut_ptr();
    *out = OsalGpuMem { cpu: p as *mut c_void, gpu: p as u64, phys: p as u64, size, impl_: p as *mut c_void };
    0
}
unsafe extern "C" fn m_gpu_free(_mem: *mut OsalGpuMem) -> i32 {
    0
}
unsafe extern "C" fn m_cache(_cpu: *mut c_void, _n: usize) {}
unsafe extern "C" fn b_init() -> i32 {
    0
}
unsafe extern "C" fn b_heap_base(_heap: GlesHeap) -> u64 {
    0x1000_0000
}
unsafe extern "C" fn b_ok() -> i32 {
    0
}

/// What the back end was handed by one draw.
#[derive(Clone, Debug)]
struct Seen {
    /// An application draw (vertices), not a clear alone.
    draws: bool,
    /// `GlesDraw::samples`.
    samples: u8,
}

static SEEN: Mutex<Vec<Seen>> = Mutex::new(Vec::new());

/// The 4x targets drawn since the last `finish`: (`target_gpu`, w, h).
static PENDING: Mutex<Vec<(u64, u32, u32)>> = Mutex::new(Vec::new());

/// The colour the stand-in's job-end resolve leaves in an RGBA8 target.
const RESOLVED: [u8; 4] = [0x40, 0x80, 0xc0, 0xff];

unsafe extern "C" fn b_draw(d: *const GlesDraw) -> i32 {
    let d = &*d;
    SEEN.lock().unwrap().push(Seen { draws: d.index_count != 0, samples: d.samples });
    if d.samples > 1 {
        PENDING.lock().unwrap().push((d.target_gpu, d.target_w, d.target_h));
    }
    0
}

/// Note (b)7 in the stand-in: when the jobs are complete, `rb.mem` of every
/// 4x renderbuffer drawn holds the resolved colour, and not before (a copy
/// that did not wait reads the zeros the storage was made with).
unsafe extern "C" fn b_finish_resolve() -> i32 {
    for (gpu, w, h) in core::mem::take(&mut *PENDING.lock().unwrap()) {
        /* the stand-in OSAL's gpu address is the CPU pointer; RGBA8 rows of w*4 */
        let px = core::slice::from_raw_parts_mut(gpu as *mut [u8; 4], (w * h) as usize);
        px.fill(RESOLVED);
    }
    0
}

/// What `depth_resolve` / `stencil_resolve` were asked for: (id, w, h).
static DS_READS: Mutex<Vec<(u32, u32, u32)>> = Mutex::new(Vec::new());
/// One `ds_write`: (id, plane, x, y, w, h, the first element as bits).
type DsWrite = (u32, u32, u32, u32, u32, u32, u32);
/// What `ds_write` was handed.
static DS_WRITES: Mutex<Vec<DsWrite>> = Mutex::new(Vec::new());

/// The depth the stand-in keeps for every object: one sample a pixel (note (b)8).
const DEPTH: f32 = 0.375;
const STENCIL: u8 = 0x5a;

unsafe extern "C" fn b_depth_resolve(id: u32, dst: *mut f32, w: u32, h: u32, pitch: u32) -> i32 {
    DS_READS.lock().unwrap().push((id, w, h));
    core::slice::from_raw_parts_mut(dst, (pitch * h) as usize).fill(DEPTH);
    0
}
unsafe extern "C" fn b_stencil_resolve(id: u32, dst: *mut u8, w: u32, h: u32, pitch: u32) -> i32 {
    DS_READS.lock().unwrap().push((id, w, h));
    core::slice::from_raw_parts_mut(dst, (pitch * h) as usize).fill(STENCIL);
    0
}
#[allow(clippy::too_many_arguments)]
unsafe extern "C" fn b_ds_write(
    id: u32,
    plane: u32,
    src: *const c_void,
    x: u32,
    y: u32,
    w: u32,
    h: u32,
    _pitch: u32,
) -> i32 {
    let first =
        if plane == backend::DS_PLANE_STENCIL { *(src as *const u8) as u32 } else { (*(src as *const f32)).to_bits() };
    DS_WRITES.lock().unwrap().push((id, plane, x, y, w, h, first));
    0
}

static OSAL: OsalOps = OsalOps {
    init: None,
    shutdown: None,
    gpu_alloc: Some(m_gpu_alloc),
    gpu_free: Some(m_gpu_free),
    mmu_map: None,
    map_phys: None,
    gpu_write: None,
    gpu_read: None,
    cache_flush: Some(m_cache),
    cache_invd: Some(m_cache),
    submit: None,
    wait: None,
    completed: None,
    reg_read: None,
    reg_write: None,
    query: None,
    log: None,
    wait_vblank: None,
    present: None,
    lock: None,
    unlock: None,
    thread_id: None,
    display_busy: None,
};

static BACKEND: YakoglBackend = YakoglBackend {
    name: c"es3-msaa-gl-srgb".as_ptr() as *const c_char,
    native_attrib_formats: 0,
    rt_rgba8888_is_bgra: false,
    mip_level_align: 0,
    compressed_blocks_in_rows: false,
    native_layered_textures: false,
    front_facing_shader_variant: false,
    etc1_decoded_at_upload: false,
    init: Some(b_init),
    shutdown: None,
    draw: Some(b_draw),
    flush: Some(b_ok),
    finish: Some(b_ok),
    finish_previous: None,
    kick_id: None,
    kick_pending: None,
    heap_base: Some(b_heap_base),
    counters: None,
    texture_released: None,
    surface_swapped: None,
    texture_uploaded: None,
    max_instance_divisor: 1,
    wait_kick: None,
    samples_texel_copies: false,
    attachment_gone: None,
    depth_resolve: None,
    query_begin: None,
    query_end: None,
    query_result: None,
    generate_mipmap: None,
    return_mipmap_chain: None,
    linear_row_align: 0,
    uniforms_host_only: false,
    ds_write: None,
    stencil_resolve: None,
    max_samples: 0,
};

/// [`BACKEND`] with the multisample capability on: it keeps 4 samples.
static BACKEND_MS4: YakoglBackend = YakoglBackend { max_samples: 4, finish: Some(b_finish_resolve), ..BACKEND };

/// [`BACKEND_MS4`] that also keeps depth/stencil planes per object and reads
/// them back one sample a pixel (note (b)8).
static BACKEND_MS4_DS: YakoglBackend = YakoglBackend {
    depth_resolve: Some(b_depth_resolve),
    stencil_resolve: Some(b_stencil_resolve),
    ds_write: Some(b_ds_write),
    ..BACKEND_MS4
};

/// One context for the whole file; the tests take turns on it.
static LOCK: Mutex<()> = Mutex::new(());
static ONCE: Once = Once::new();

/// The context with the stand-in back end of today: no multisample capability.
fn context() -> MutexGuard<'static, ()> {
    context_on(&BACKEND)
}

/// The context with `table` as the back end (a test that swapped in the 4x
/// one does not leak it into the next).
fn context_on(table: &'static YakoglBackend) -> MutexGuard<'static, ()> {
    let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    backend::HOST_DRAW_TARGETS.store(4, Ordering::Relaxed);
    ONCE.call_once(|| {
        // SAFETY: both tables are statics; the lock above serialises the tests.
        unsafe {
            osal_set_ops(&OSAL);
            backend::g_backend = &BACKEND;
            yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
            assert!(!yakogl::gl::context::gl_context_create(8, 8).is_null());
        }
    });
    // SAFETY: a static table; the lock above serialises the tests. Plain GL calls.
    unsafe {
        backend::g_backend = table;
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glFinish();
    }
    SEEN.lock().unwrap().clear();
    PENDING.lock().unwrap().clear();
    DS_READS.lock().unwrap().clear();
    DS_WRITES.lock().unwrap().clear();
    // SAFETY: plain GL call.
    while unsafe { glGetError() } != GL_NO_ERROR {}
    g
}

/// The context with a stand-in back end that says `max_samples` 4.
fn context_ms4() -> MutexGuard<'static, ()> {
    context_on(&BACKEND_MS4)
}

fn err() -> GLenum {
    // SAFETY: plain GL call.
    unsafe { glGetError() }
}

fn geti(pname: GLenum) -> GLint {
    let mut v: GLint = -777;
    // SAFETY: one GLint out.
    unsafe { glGetIntegerv(pname, &mut v) };
    assert_eq!(err(), GL_NO_ERROR);
    v
}

/// glGetInternalformativ(GL_RENDERBUFFER, format, pname) with room for `n`
/// values: the values written (-777 marks one not written) and the error.
fn internalformat(format: GLenum, pname: GLenum, n: usize) -> (Vec<GLint>, GLenum) {
    let mut v = vec![-777; n];
    // SAFETY: room for `n` GLints.
    unsafe { glGetInternalformativ(GL_RENDERBUFFER, format, pname, n as GLsizei, v.as_mut_ptr()) };
    (v, err())
}

/* ---- framebuffers, the program, the draw ------------------------------- */

struct Fbo {
    fb: GLuint,
    rbs: Vec<GLuint>,
}

impl Drop for Fbo {
    fn drop(&mut self) {
        // SAFETY: names this test made.
        unsafe {
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
            glDeleteFramebuffers(1, &self.fb);
            glDeleteRenderbuffers(self.rbs.len() as GLsizei, self.rbs.as_ptr());
            glUseProgram(0);
        }
    }
}

/// `glRenderbufferStorageMultisample(.., samples, format, w, h)` on a fresh
/// renderbuffer: its error and the renderbuffer (left bound).
fn storage_sized(samples: GLsizei, format: GLenum, w: GLsizei, h: GLsizei) -> (GLenum, GLuint) {
    let mut rb = 0;
    // SAFETY: plain GL calls.
    unsafe {
        glGenRenderbuffers(1, &mut rb);
        glBindRenderbuffer(GL_RENDERBUFFER, rb);
        while glGetError() != GL_NO_ERROR {}
        glRenderbufferStorageMultisample(GL_RENDERBUFFER, samples, format, w, h);
    }
    (err(), rb)
}

fn storage(samples: GLsizei, format: GLenum) -> (GLenum, GLuint) {
    storage_sized(samples, format, 8, 8)
}

/// GL_RENDERBUFFER_SAMPLES of `rb`.
fn rb_samples(rb: GLuint) -> GLint {
    let mut v: GLint = -777;
    // SAFETY: plain GL calls; one GLint out.
    unsafe {
        glBindRenderbuffer(GL_RENDERBUFFER, rb);
        glGetRenderbufferParameteriv(GL_RENDERBUFFER, GL_RENDERBUFFER_SAMPLES, &mut v);
    }
    assert_eq!(err(), GL_NO_ERROR);
    v
}

/// A bound 8x8 framebuffer with a renderbuffer of `samples` on each of
/// `formats`' colour attachments. A storage error comes back as `Err(error)`.
fn framebuffer(samples: GLsizei, formats: &[GLenum]) -> Result<Fbo, GLenum> {
    let mut fb = 0;
    // SAFETY: room for one name; plain GL call.
    unsafe {
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
    }
    let mut fbo = Fbo { fb, rbs: Vec::new() };
    for (i, f) in formats.iter().enumerate() {
        let (e, rb) = storage(samples, *f);
        fbo.rbs.push(rb);
        if e != GL_NO_ERROR {
            return Err(e);
        }
        // SAFETY: a renderbuffer this test made.
        unsafe { glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0 + i as GLenum, GL_RENDERBUFFER, rb) };
    }
    assert_eq!(err(), GL_NO_ERROR);
    Ok(fbo)
}

fn status() -> GLenum {
    // SAFETY: plain GL call.
    unsafe { glCheckFramebufferStatus(GL_FRAMEBUFFER) }
}

/// A 2D RGBA8 texture of 8x8, left bound.
fn texture() -> GLuint {
    let mut tex = 0;
    // SAFETY: plain GL calls; one name out.
    unsafe {
        glGenTextures(1, &mut tex);
        glBindTexture(GL_TEXTURE_2D, tex);
        glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA8 as GLint, 8, 8, 0, GL_RGBA, GL_UNSIGNED_BYTE, core::ptr::null());
    }
    assert_eq!(err(), GL_NO_ERROR);
    tex
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

/// The current program: one float output.
fn program() {
    let v = shader(GL_VERTEX_SHADER, "#version 300 es\nin vec4 p;\nvoid main() { gl_Position = p; }\n");
    let f = shader(
        GL_FRAGMENT_SHADER,
        "#version 300 es\nprecision highp float;\nlayout(location = 0) out vec4 o;\nvoid main() { o = vec4(0.25, 0.5, 0.75, 1.0); }\n",
    );
    // SAFETY: plain GL calls.
    unsafe {
        let p = glCreateProgram();
        glAttachShader(p, v);
        glAttachShader(p, f);
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        assert_eq!(ok, 1);
        glUseProgram(p);
    }
}

/// Draw one triangle over the bound framebuffer, the job left unfinished.
fn draw_unfinished() {
    static TRI: [f32; 12] = [-1.0, -1.0, 0.0, 1.0, 3.0, -1.0, 0.0, 1.0, -1.0, 3.0, 0.0, 1.0];
    // SAFETY: TRI is static and outlives the draw.
    unsafe {
        glBindBuffer(GL_ARRAY_BUFFER, 0);
        glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, TRI.as_ptr() as *const c_void);
        glEnableVertexAttribArray(0);
        glDrawArrays(GL_TRIANGLES, 0, 3);
    }
}

/// Draw one triangle and hand back what the back end was given for it.
fn draw() -> Seen {
    SEEN.lock().unwrap().clear();
    draw_unfinished();
    // SAFETY: plain GL call.
    unsafe { glFinish() };
    assert_eq!(err(), GL_NO_ERROR);
    let s = core::mem::take(&mut *SEEN.lock().unwrap());
    s.into_iter().rev().find(|s| s.draws).expect("the draw reached the back end")
}

/// glBlitFramebuffer of colour from `read` to `draw_fb` over the given
/// rectangles; the GL error.
fn blit(read: GLuint, draw_fb: GLuint, src: [GLint; 4], dst: [GLint; 4]) -> GLenum {
    blit_with(read, draw_fb, src, dst, GL_COLOR_BUFFER_BIT, GL_NEAREST)
}

/// glBlitFramebuffer from `read` to `draw_fb` with any mask and filter; the
/// GL error.
fn blit_with(
    read: GLuint,
    draw_fb: GLuint,
    src: [GLint; 4],
    dst: [GLint; 4],
    mask: GLbitfield,
    filter: GLenum,
) -> GLenum {
    // SAFETY: plain GL calls on names this test made.
    unsafe {
        glBindFramebuffer(GL_READ_FRAMEBUFFER, read);
        glBindFramebuffer(GL_DRAW_FRAMEBUFFER, draw_fb);
        glReadBuffer(GL_COLOR_ATTACHMENT0);
        while glGetError() != GL_NO_ERROR {}
        glBlitFramebuffer(src[0], src[1], src[2], src[3], dst[0], dst[1], dst[2], dst[3], mask, filter);
    }
    err()
}

/// The 8x8 RGBA8 pixels of colour attachment 0 of single-sample `fb`, row
/// by row from the bottom.
fn pixels(fb: GLuint) -> Vec<[u8; 4]> {
    let mut px = vec![[0x11u8; 4]; 64];
    // SAFETY: room for 8x8 RGBA/UNSIGNED_BYTE pixels (rows of 32 bytes).
    unsafe {
        glBindFramebuffer(GL_READ_FRAMEBUFFER, fb);
        glReadBuffer(GL_COLOR_ATTACHMENT0);
        glReadPixels(0, 0, 8, 8, GL_RGBA, GL_UNSIGNED_BYTE, px.as_mut_ptr() as *mut c_void);
    }
    assert_eq!(err(), GL_NO_ERROR);
    px
}

/// A 4x framebuffer of `format` drawn into, its job submitted but not
/// finished (4-byte formats only: the stand-in's resolve writes w*h*4 bytes).
fn drawn_4x_of(format: GLenum) -> Fbo {
    let src = framebuffer(4, &[format]).unwrap();
    program();
    draw_unfinished();
    assert_eq!(err(), GL_NO_ERROR);
    assert_eq!(PENDING.lock().unwrap().len(), 1, "the 4x draw's job is pending");
    src
}

/// Is pixel (x, y) of an 8x8 image inside rectangle `r` (x0, y0, x1, y1)?
fn inside(i: usize, r: [GLint; 4]) -> bool {
    let (x, y) = ((i % 8) as GLint, (i / 8) as GLint);
    x >= r[0] && x < r[2] && y >= r[1] && y < r[3]
}

const FULL: [GLint; 4] = [0, 0, 8, 8];

/* ---- sRGB: the same-format resolve --------------------------------------- */

/// 4x SRGB8_ALPHA8 into 1x SRGB8_ALPHA8: allowed; the job is waited for and
/// the bytes the back end left reach the destination (no encode by the core
/// of a same-format copy). Where the samples were averaged — linear halves
/// on the V3D, the F16 `gamma` pass on the PowerVR, the encoded bytes on the
/// GC7000UL's RS — is the GPU's, note (f), and not pinned here.
#[test]
fn srgb_to_srgb_resolves() {
    let _g = context_ms4();
    let dst = framebuffer(0, &[GL_SRGB8_ALPHA8]).unwrap();
    let src = drawn_4x_of(GL_SRGB8_ALPHA8);
    assert_eq!(blit(src.fb, dst.fb, FULL, FULL), GL_NO_ERROR);
    assert!(PENDING.lock().unwrap().is_empty(), "the blit waited for the 4x job");
    assert!(pixels(dst.fb).iter().all(|p| *p == RESOLVED), "the resolved bytes, every pixel");
}

/// The same over a sub-region: only it is copied.
#[test]
fn srgb_to_srgb_region_copies_only_it() {
    let _g = context_ms4();
    let dst = framebuffer(0, &[GL_SRGB8_ALPHA8]).unwrap();
    let src = drawn_4x_of(GL_SRGB8_ALPHA8);
    let region = [2, 1, 6, 5];
    assert_eq!(blit(src.fb, dst.fb, region, region), GL_NO_ERROR);
    for (i, p) in pixels(dst.fb).iter().enumerate() {
        let want = if inside(i, region) { RESOLVED } else { [0; 4] };
        assert_eq!(*p, want, "pixel ({}, {})", i % 8, i / 8);
    }
}

/* ---- sRGB against linear RGBA8: the core's fold --------------------------- */

/// ES 3.0 4.3.3 asks a resolve for identical colour formats, but the core
/// folds sRGB into linear first (`linear_format`, src/gl/blit.rs) as Mesa's
/// `compatible_resolve_formats` does; es3_msaa_gl.rs and
/// es3_msaa_gl_formats.rs pin the fold, both directions, as intended. The
/// resolve is waited for and copies: the destination holds one colour, not
/// the zeros its storage was made with, and its alpha is the resolved byte
/// (alpha is linear, never encoded). The exact bytes are the core's
/// decode/encode of the sample across the fold, whose table the sRGB unit
/// tests of src/gl/blit.rs pin; they are not pinned here, and no averaging
/// space is (note (f)).
#[test]
fn srgb_to_rgba8_resolves_across_the_fold() {
    let _g = context_ms4();
    let dst = framebuffer(0, &[GL_RGBA8]).unwrap();
    let src = drawn_4x_of(GL_SRGB8_ALPHA8);
    assert_eq!(blit(src.fb, dst.fb, FULL, FULL), GL_NO_ERROR);
    assert!(PENDING.lock().unwrap().is_empty(), "the blit waited for the 4x job");
    let px = pixels(dst.fb);
    assert!(px.iter().all(|p| *p == px[0]), "one colour");
    assert_ne!(px[0], [0; 4], "the resolved colour, not the zeros of the storage");
    assert_eq!(px[0][3], RESOLVED[3], "alpha, linear, as stored");
}

#[test]
fn rgba8_to_srgb_resolves_across_the_fold() {
    let _g = context_ms4();
    let dst = framebuffer(0, &[GL_SRGB8_ALPHA8]).unwrap();
    let src = drawn_4x_of(GL_RGBA8);
    assert_eq!(blit(src.fb, dst.fb, FULL, FULL), GL_NO_ERROR);
    assert!(PENDING.lock().unwrap().is_empty(), "the blit waited for the 4x job");
    let px = pixels(dst.fb);
    assert!(px.iter().all(|p| *p == px[0]), "one colour");
    assert_ne!(px[0], [0; 4], "the resolved colour, not the zeros of the storage");
    assert_eq!(px[0][3], RESOLVED[3], "alpha, linear, as stored");
}

/// ES 3.0 4.3.3 strict: SRGB8_ALPHA8 and RGBA8 are different internal
/// formats, so a resolve between them is GL_INVALID_OPERATION. The core
/// folds sRGB into linear instead (the two tests above), which Mesa does on
/// purpose; these hold the spec text and flip if the core ever adopts it.
#[test]
#[ignore = "the core folds sRGB into linear for a resolve (linear_format, src/gl/blit.rs; Mesa compatible_resolve_formats), pinned as intended by es3_msaa_gl.rs and es3_msaa_gl_formats.rs"]
fn srgb_to_rgba8_strict_es30_is_invalid_operation() {
    let _g = context_ms4();
    let dst = framebuffer(0, &[GL_RGBA8]).unwrap();
    let src = drawn_4x_of(GL_SRGB8_ALPHA8);
    assert_eq!(blit(src.fb, dst.fb, FULL, FULL), GL_INVALID_OPERATION);
    assert_eq!(PENDING.lock().unwrap().len(), 1, "a refused resolve waits for nothing");
    assert!(pixels(dst.fb).iter().all(|p| *p == [0; 4]), "no copy");
}

#[test]
#[ignore = "the core folds sRGB into linear for a resolve (linear_format, src/gl/blit.rs; Mesa compatible_resolve_formats), pinned as intended by es3_msaa_gl.rs and es3_msaa_gl_formats.rs"]
fn rgba8_to_srgb_strict_es30_is_invalid_operation() {
    let _g = context_ms4();
    let dst = framebuffer(0, &[GL_SRGB8_ALPHA8]).unwrap();
    let src = drawn_4x_of(GL_RGBA8);
    assert_eq!(blit(src.fb, dst.fb, FULL, FULL), GL_INVALID_OPERATION);
    assert_eq!(PENDING.lock().unwrap().len(), 1, "a refused resolve waits for nothing");
    assert!(pixels(dst.fb).iter().all(|p| *p == [0; 4]), "no copy");
}

/* ---- RGB10_A2 ------------------------------------------------------------- */

/// 4x RGB10_A2 into 1x RGB10_A2: allowed, the job waited for. The bytes are
/// not pinned: the core reads the destination through the format's
/// conversion (the stand-in left RGBA8 words), and what space the samples
/// were averaged in is the GPU's, note (f).
#[test]
fn rgb10_a2_to_rgb10_a2_resolves() {
    let _g = context_ms4();
    let dst = framebuffer(0, &[GL_RGB10_A2]).unwrap();
    let src = drawn_4x_of(GL_RGB10_A2);
    assert_eq!(blit(src.fb, dst.fb, FULL, FULL), GL_NO_ERROR);
    assert!(PENDING.lock().unwrap().is_empty(), "the blit waited for the 4x job");
}

/// 4x RGB10_A2 into RGBA8 or SRGB8_ALPHA8, and RGBA8 into RGB10_A2: the
/// formats differ (RGB10_A2 is no part of the sRGB fold), so ES 3.0 4.3.3:
/// GL_INVALID_OPERATION, no wait, no copy.
#[test]
fn rgb10_a2_to_rgba8_is_invalid_operation() {
    let _g = context_ms4();
    let src = drawn_4x_of(GL_RGB10_A2);
    let rgba8 = framebuffer(0, &[GL_RGBA8]).unwrap();
    assert_eq!(blit(src.fb, rgba8.fb, FULL, FULL), GL_INVALID_OPERATION);
    /* the no-copy check reads pixels, and the read waits for the pending
     * job: the no-wait pin has to come before it */
    assert_eq!(PENDING.lock().unwrap().len(), 1, "a refused resolve waits for nothing");
    assert!(pixels(rgba8.fb).iter().all(|p| *p == [0; 4]), "no copy");
    let srgb = framebuffer(0, &[GL_SRGB8_ALPHA8]).unwrap();
    assert_eq!(blit(src.fb, srgb.fb, FULL, FULL), GL_INVALID_OPERATION, "into sRGB");
    let src8 = framebuffer(4, &[GL_RGBA8]).unwrap();
    let dst10 = framebuffer(0, &[GL_RGB10_A2]).unwrap();
    assert_eq!(blit(src8.fb, dst10.fb, FULL, FULL), GL_INVALID_OPERATION, "RGBA8 into RGB10_A2");
}

/* ---- the 16-bit formats ---------------------------------------------------- */

/// RGBA4, RGB5_A1 and RGB565: a pair of the same format resolves; every
/// other pair among them, and either against RGBA8 or RGB10_A2, is
/// GL_INVALID_OPERATION (none of them is part of the sRGB fold).
#[test]
fn sixteen_bit_pairs() {
    let _g = context_ms4();
    let fmts = [GL_RGBA4, GL_RGB5_A1, GL_RGB565];
    for s in fmts {
        let src = framebuffer(4, &[s]).unwrap();
        for d in fmts {
            let dst = framebuffer(0, &[d]).unwrap();
            let want = if s == d { GL_NO_ERROR } else { GL_INVALID_OPERATION };
            assert_eq!(blit(src.fb, dst.fb, FULL, FULL), want, "{s:#x} into {d:#x}");
        }
        let dst = framebuffer(0, &[GL_RGBA8]).unwrap();
        assert_eq!(blit(src.fb, dst.fb, FULL, FULL), GL_INVALID_OPERATION, "{s:#x} into RGBA8");
        let dst = framebuffer(0, &[GL_RGB10_A2]).unwrap();
        assert_eq!(blit(src.fb, dst.fb, FULL, FULL), GL_INVALID_OPERATION, "{s:#x} into RGB10_A2");
        let dst = framebuffer(0, &[GL_SRGB8_ALPHA8]).unwrap();
        assert_eq!(blit(src.fb, dst.fb, FULL, FULL), GL_INVALID_OPERATION, "{s:#x} into SRGB8_ALPHA8");
    }
}

/* ---- a texture destination -------------------------------------------------- */

/// A 1x 8x8 texture of the sized format, attached to a new framebuffer.
fn texture_fbo(internal: GLenum, format: GLenum, ty: GLenum) -> (GLuint, GLuint) {
    let (mut tex, mut fb) = (0, 0);
    // SAFETY: plain GL calls; one name each out.
    unsafe {
        glGenTextures(1, &mut tex);
        glBindTexture(GL_TEXTURE_2D, tex);
        glTexImage2D(GL_TEXTURE_2D, 0, internal as GLint, 8, 8, 0, format, ty, core::ptr::null());
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex, 0);
    }
    assert_eq!(err(), GL_NO_ERROR);
    (fb, tex)
}

/// A 1x texture of the same sized format is a legal resolve destination:
/// 4x SRGB8_ALPHA8 into an SRGB8_ALPHA8 texture is allowed, waited for, and
/// the resolved bytes reach it; 4x RGB10_A2 into it is GL_INVALID_OPERATION
/// (the fold folds only sRGB, and never RGB10_A2).
#[test]
fn texture_destination_same_format() {
    let _g = context_ms4();
    let (tfb, tex) = texture_fbo(GL_SRGB8_ALPHA8, GL_RGBA, GL_UNSIGNED_BYTE);
    let src = drawn_4x_of(GL_SRGB8_ALPHA8);
    assert_eq!(blit(src.fb, tfb, FULL, FULL), GL_NO_ERROR, "sRGB into an sRGB texture");
    assert_eq!(PENDING.lock().unwrap().len(), 0, "the blit waited for the 4x job");
    assert!(pixels(tfb).iter().all(|p| *p == RESOLVED), "the resolved bytes reach the texture");
    let src10 = drawn_4x_of(GL_RGB10_A2);
    assert_eq!(blit(src10.fb, tfb, FULL, FULL), GL_INVALID_OPERATION, "RGB10_A2 into an sRGB texture");
    // SAFETY: names this test made.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glDeleteFramebuffers(1, &tfb);
        glDeleteTextures(1, &tex);
    }
}

/// ES 3.0: 4x RGB10_A2 into a 1x RGB10_A2 texture is the same sized format
/// and allowed, and RGBA8 into it is GL_INVALID_OPERATION. Whether the core
/// sees the texture's format that way is the back end's storage choice, not
/// the core's rule: gpu-v3d holds the 2_10_10_10 words, the other back ends
/// store the texture RGBA8 (note (d)), and the attachment is judged by what
/// it stores (the `t.texfmt` match in `attachment_storage`,
/// src/gl/framebuffer.rs) — so this passes only in a gpu-v3d build and is
/// GL_INVALID_OPERATION in the others. tests/es3_msaa_gl_texdst.rs pins the
/// same storage distortion for RGB8/RGBA4 textures; the RGB10_A2
/// *renderbuffer* rule above is build-independent and pinned passing.
#[test]
#[ignore = "build-dependent: an RGB10_A2 texture stores as RGBA8 on the non-v3d back ends (docs/notes/es3-msaa-renderbuffers.md (d)), so the resolve compares the stored format; allowed only in a gpu-v3d build"]
fn rgb10_a2_texture_destination() {
    let _g = context_ms4();
    let (tfb10, tex10) = texture_fbo(GL_RGB10_A2, GL_RGBA, GL_UNSIGNED_INT_2_10_10_10_REV);
    let src10 = drawn_4x_of(GL_RGB10_A2);
    assert_eq!(blit(src10.fb, tfb10, FULL, FULL), GL_NO_ERROR, "RGB10_A2 into an RGB10_A2 texture");
    let src8 = framebuffer(4, &[GL_RGBA8]).unwrap();
    assert_eq!(blit(src8.fb, tfb10, FULL, FULL), GL_INVALID_OPERATION, "RGBA8 into an RGB10_A2 texture");
    // SAFETY: names this test made.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glDeleteFramebuffers(1, &tfb10);
        glDeleteTextures(1, &tex10);
    }
}
