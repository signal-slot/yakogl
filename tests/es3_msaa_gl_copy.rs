// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Copy and read paths after a multisample resolve, on the host
//! (docs/notes/es3-msaa-renderbuffers.md (a), (b); ES 3.0 4.3.2, 3.8.5).
//!
//! The errors of glReadPixels, glCopyTexImage2D, glCopyTexSubImage2D and
//! glCopyTexSubImage3D from a 4x read framebuffer are pinned in
//! tests/es3_msaa_read_errors.rs and tests/es3_msaa_gl_clear.rs. This file
//! covers the other half: after a resolve blit into a single-sample
//! framebuffer, those copies from the resolved framebuffer work (the resolved
//! colour reaches the texture, the 4x job having been waited for), and the
//! resolved texture is what a later draw samples. Also that the 4x source
//! stays refused after the resolve. The stand-in is the 4x table of
//! tests/es3_msaa_gl.rs (`finish` writes the resolved colour).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_msaa_gl_copy
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
use yakogl::gl::texture::*;
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
    /// The first texel of the first sampler binding, if the draw samples one.
    tex0: Option<[u8; 4]>,
}

static SEEN: Mutex<Vec<Seen>> = Mutex::new(Vec::new());

/// The 4x targets drawn since the last `finish`: (`target_gpu`, w, h).
static PENDING: Mutex<Vec<(u64, u32, u32)>> = Mutex::new(Vec::new());

/// The colour the stand-in's job-end resolve leaves in an RGBA8 target.
const RESOLVED: [u8; 4] = [0x40, 0x80, 0xc0, 0xff];

unsafe extern "C" fn b_draw(d: *const GlesDraw) -> i32 {
    let d = &*d;
    let tex0 = (d.num_textures > 0 && d.textures[0].base_gpu != 0).then(|| *(d.textures[0].base_gpu as *const [u8; 4]));
    SEEN.lock().unwrap().push(Seen { draws: d.index_count != 0, samples: d.samples, tex0 });
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
    name: c"es3-msaa-gl".as_ptr() as *const c_char,
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

/// A 4x RGBA8 framebuffer drawn into, its job submitted but not finished.
fn drawn_4x() -> Fbo {
    let src = framebuffer(4, &[GL_RGBA8]).unwrap();
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

/* ---- helpers of this file ---------------------------------------------------- */

/// A single-sample RGBA8 renderbuffer framebuffer (left bound for drawing).
fn resolved_1x() -> Fbo {
    framebuffer(0, &[GL_RGBA8]).unwrap()
}

/// Resolve a fresh pending 4x draw into a single-sample framebuffer and
/// return that framebuffer, its read buffer holding `RESOLVED` everywhere.
fn resolved() -> Fbo {
    let dst = resolved_1x();
    let src = drawn_4x();
    assert_eq!(blit(src.fb, dst.fb, FULL, FULL), GL_NO_ERROR);
    assert!(PENDING.lock().unwrap().is_empty(), "the blit waited for the 4x job");
    drop(src);
    dst
}

/// The 8x8 pixels of level 0 of `tex` (2D, or layer `layer` of a layered
/// texture) through a framebuffer attachment.
fn texture_pixels(target: GLenum, tex: GLuint, layer: GLint) -> Vec<[u8; 4]> {
    let mut fb = 0;
    let mut px = vec![[0x11u8; 4]; 64];
    // SAFETY: names made here; room for 8x8 RGBA/UNSIGNED_BYTE pixels.
    unsafe {
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_READ_FRAMEBUFFER, fb);
        if target == GL_TEXTURE_2D {
            glFramebufferTexture2D(GL_READ_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex, 0);
        } else {
            glFramebufferTextureLayer(GL_READ_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, tex, 0, layer);
        }
        glReadBuffer(GL_COLOR_ATTACHMENT0);
        glReadPixels(0, 0, 8, 8, GL_RGBA, GL_UNSIGNED_BYTE, px.as_mut_ptr() as *mut c_void);
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glDeleteFramebuffers(1, &fb);
    }
    assert_eq!(err(), GL_NO_ERROR);
    px
}

const OLD: [u8; 4] = [0x11; 4];

/// A 3D or 2D-array RGBA8 texture of 8x8x2, left bound.
fn texture_layered(target: GLenum) -> GLuint {
    let mut tex = 0;
    // SAFETY: plain GL calls; one name out; null data.
    unsafe {
        glGenTextures(1, &mut tex);
        glBindTexture(target, tex);
        glTexImage3D(target, 0, GL_RGBA8 as GLint, 8, 8, 2, 0, GL_RGBA, GL_UNSIGNED_BYTE, core::ptr::null());
    }
    assert_eq!(err(), GL_NO_ERROR);
    tex
}

/// Make the read framebuffer `fb` with colour attachment 0 read.
fn read_from(fb: GLuint) {
    // SAFETY: a name this test made.
    unsafe {
        glBindFramebuffer(GL_READ_FRAMEBUFFER, fb);
        glReadBuffer(GL_COLOR_ATTACHMENT0);
        while glGetError() != GL_NO_ERROR {}
    }
}

/* ---- copies from the resolved framebuffer ---------------------------------- */

/// ES 3.0 3.8.5: after the resolve the read framebuffer has no sample buffers
/// and glReadPixels returns the resolved colour.
#[test]
fn read_pixels_after_resolve() {
    let _g = context_ms4();
    let dst = resolved();
    assert!(pixels(dst.fb).iter().all(|p| *p == RESOLVED));
    assert_eq!(err(), GL_NO_ERROR);
}

#[test]
fn copy_tex_image_2d_after_resolve() {
    let _g = context_ms4();
    let dst = resolved();
    read_from(dst.fb);
    let tex = texture();
    // SAFETY: a 2D texture is bound; plain GL call.
    unsafe { glCopyTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA8, 0, 0, 8, 8, 0) };
    assert_eq!(err(), GL_NO_ERROR);
    assert!(texture_pixels(GL_TEXTURE_2D, tex, 0).iter().all(|p| *p == RESOLVED));
}

#[test]
fn copy_tex_sub_image_2d_after_resolve() {
    let _g = context_ms4();
    let dst = resolved();
    let tex = texture();
    let old = vec![OLD; 64];
    // SAFETY: 8x8 RGBA pixels; plain GL call.
    unsafe { glTexSubImage2D(GL_TEXTURE_2D, 0, 0, 0, 8, 8, GL_RGBA, GL_UNSIGNED_BYTE, old.as_ptr() as *const c_void) };
    assert_eq!(err(), GL_NO_ERROR);
    read_from(dst.fb);
    // a 4x4 corner of the resolved framebuffer into the middle of the texture
    // SAFETY: plain GL call.
    unsafe { glCopyTexSubImage2D(GL_TEXTURE_2D, 0, 2, 2, 0, 0, 4, 4) };
    assert_eq!(err(), GL_NO_ERROR);
    for (i, p) in texture_pixels(GL_TEXTURE_2D, tex, 0).iter().enumerate() {
        let want = if inside(i, [2, 2, 6, 6]) { RESOLVED } else { OLD };
        assert_eq!(*p, want, "texel {i}");
    }
}

fn copy_tex_sub_image_3d_after_resolve(target: GLenum) {
    let _g = context_ms4();
    let dst = resolved();
    let tex = texture_layered(target);
    let old = vec![OLD; 128];
    // SAFETY: 8x8x2 RGBA pixels; plain GL call.
    unsafe { glTexSubImage3D(target, 0, 0, 0, 0, 8, 8, 2, GL_RGBA, GL_UNSIGNED_BYTE, old.as_ptr() as *const c_void) };
    assert_eq!(err(), GL_NO_ERROR);
    read_from(dst.fb);
    // SAFETY: plain GL call.
    unsafe { glCopyTexSubImage3D(target, 0, 0, 0, 1, 0, 0, 8, 8) };
    assert_eq!(err(), GL_NO_ERROR);
    assert!(texture_pixels(target, tex, 1).iter().all(|p| *p == RESOLVED), "layer 1 holds the resolve");
    assert!(texture_pixels(target, tex, 0).iter().all(|p| *p == OLD), "layer 0 is untouched");
}

#[test]
fn copy_tex_sub_image_3d_after_resolve_3d() {
    copy_tex_sub_image_3d_after_resolve(GL_TEXTURE_3D);
}

#[test]
fn copy_tex_sub_image_3d_after_resolve_2d_array() {
    copy_tex_sub_image_3d_after_resolve(GL_TEXTURE_2D_ARRAY);
}

/// The resolve does not turn the 4x source into a readable one: it still
/// answers GL_INVALID_OPERATION to every copy and read.
#[test]
fn the_4x_source_stays_refused_after_the_resolve() {
    let _g = context_ms4();
    let dst = resolved_1x();
    let src = drawn_4x();
    assert_eq!(blit(src.fb, dst.fb, FULL, FULL), GL_NO_ERROR);
    let tex = texture();
    read_from(src.fb);
    let mut px = [0u8; 4];
    // SAFETY: room for one pixel; plain GL calls on the bound texture.
    unsafe {
        glReadPixels(0, 0, 1, 1, GL_RGBA, GL_UNSIGNED_BYTE, px.as_mut_ptr() as *mut c_void);
        assert_eq!(glGetError(), GL_INVALID_OPERATION, "glReadPixels");
        glCopyTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA8, 0, 0, 8, 8, 0);
        assert_eq!(glGetError(), GL_INVALID_OPERATION, "glCopyTexImage2D");
        glCopyTexSubImage2D(GL_TEXTURE_2D, 0, 0, 0, 0, 0, 8, 8);
        assert_eq!(glGetError(), GL_INVALID_OPERATION, "glCopyTexSubImage2D");
    }
    assert!(texture_pixels(GL_TEXTURE_2D, tex, 0).iter().all(|p| *p == [0; 4]), "nothing was copied");
}

/// A copy with a 4x draw framebuffer but a single-sample read framebuffer
/// (split bindings) is fine: only the read framebuffer counts.
#[test]
fn copy_with_a_4x_draw_framebuffer_and_a_1x_read_framebuffer() {
    let _g = context_ms4();
    let dst = resolved();
    let other = framebuffer(4, &[GL_RGBA8]).unwrap();
    let tex = texture();
    // SAFETY: names this test made.
    unsafe {
        glBindFramebuffer(GL_DRAW_FRAMEBUFFER, other.fb);
        glBindFramebuffer(GL_READ_FRAMEBUFFER, dst.fb);
        glReadBuffer(GL_COLOR_ATTACHMENT0);
        while glGetError() != GL_NO_ERROR {}
        glCopyTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA8, 0, 0, 8, 8, 0);
    }
    assert_eq!(err(), GL_NO_ERROR);
    assert!(texture_pixels(GL_TEXTURE_2D, tex, 0).iter().all(|p| *p == RESOLVED));
}

/* ---- sampling the resolved texture ------------------------------------------ */

/// Resolve into a texture level, then draw a program that samples it: the
/// back end is handed that texture, holding the resolved colour.
#[test]
fn a_later_draw_samples_the_resolved_texture() {
    let _g = context_ms4();
    let tex = texture();
    // SAFETY: plain GL calls on the bound texture.
    unsafe {
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST as GLint);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST as GLint);
    }
    let mut fb = 0;
    // SAFETY: one name out; a texture this test made.
    unsafe {
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_DRAW_FRAMEBUFFER, fb);
        glFramebufferTexture2D(GL_DRAW_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex, 0);
    }
    let src = drawn_4x();
    assert_eq!(blit(src.fb, fb, FULL, FULL), GL_NO_ERROR);
    assert!(PENDING.lock().unwrap().is_empty());
    drop(src);

    // a single-sample target to draw into, sampling the texture
    let target = resolved_1x();
    let v = shader(GL_VERTEX_SHADER, "#version 300 es\nin vec4 p;\nvoid main() { gl_Position = p; }\n");
    let f = shader(
        GL_FRAGMENT_SHADER,
        "#version 300 es\nprecision highp float;\nuniform sampler2D s;\nlayout(location = 0) out vec4 o;\nvoid main() { o = texture(s, vec2(0.5)); }\n",
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
        glUniform1i(glGetUniformLocation(p, c"s".as_ptr()), 0);
        glBindFramebuffer(GL_FRAMEBUFFER, target.fb);
        glBindTexture(GL_TEXTURE_2D, tex);
    }
    let seen = draw();
    assert_eq!(seen.samples, 1, "the target is single-sample");
    assert_eq!(seen.tex0, Some(RESOLVED), "the sampler reads the resolved colour");
    // SAFETY: a name this test made.
    unsafe { glDeleteFramebuffers(1, &fb) };
}
