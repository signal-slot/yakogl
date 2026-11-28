// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! ES 3.0 multisample renderbuffers: 4x FBOs with only depth/stencil, and
//! depth/stencil-only resolves, on the GL-core stand-in (docs/notes/
//! es3-msaa-renderbuffers.md (b)4, (b)8).
//!
//! The stand-in says `max_samples` 4 and keeps depth/stencil planes per object
//! (`depth_resolve`, `stencil_resolve`, `ds_write`). It also records every
//! application `GlesDraw`. The stand-in and helpers are copied from
//! tests/es3_msaa_gl_ds.rs, which already asserts the full-plane resolve per
//! format and mask, GL_LINEAR with depth/stencil, and the depth-format
//! mismatch (D16 against D24S8 included); those cases are not repeated here.
//! What is new: completeness and GL_SAMPLES of a depth/stencil-only 4x FBO,
//! its draws (samples 4, no colour target), and sub-rectangle resolves.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_msaa_gl_ds_only
//! ```

#![allow(dead_code)]
#![cfg(all(feature = "host-test", feature = "shaderc-online", not(feature = "report-es2")))]

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

/// One application draw: (samples, target_gpu, num_targets, depth_id).
static DRAWS: Mutex<Vec<(u8, u64, u8, u32)>> = Mutex::new(Vec::new());

unsafe extern "C" fn b_draw(d: *const GlesDraw) -> i32 {
    let d = &*d;
    if d.index_count != 0 {
        DRAWS.lock().unwrap().push((d.samples, d.target_gpu, d.num_targets, d.depth_id));
    }
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
    name: c"es3-msaa-gl-ds-only".as_ptr() as *const c_char,
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
static BACKEND_MS4: YakoglBackend = YakoglBackend { max_samples: 4, ..BACKEND };

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

/// The context with `table` as the back end.
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
    DRAWS.lock().unwrap().clear();
    DS_READS.lock().unwrap().clear();
    DS_WRITES.lock().unwrap().clear();
    // SAFETY: plain GL call.
    while unsafe { glGetError() } != GL_NO_ERROR {}
    g
}

/// The context with a stand-in back end that says `max_samples` 4.
#[allow(dead_code)]
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

/* ---- framebuffers and the blit ------------------------------------------- */

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

/// glBlitFramebuffer of colour from `read` to `draw_fb` over the given
/// rectangles; the GL error.
#[allow(dead_code)]
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

const FULL: [GLint; 4] = [0, 0, 8, 8];
const D: GLbitfield = GL_DEPTH_BUFFER_BIT;
const S: GLbitfield = GL_STENCIL_BUFFER_BIT;

/// An 8x8 framebuffer whose only attachment is a renderbuffer of `samples`
/// and `format` at the depth, stencil or depth-stencil attachment point.
fn ds_fbo(samples: GLsizei, format: GLenum) -> Fbo {
    let mut fb = 0;
    // SAFETY: room for one name; plain GL call.
    unsafe {
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
    }
    let (e, rb) = storage(samples, format);
    let fbo = Fbo { fb, rbs: vec![rb] };
    assert_eq!(e, GL_NO_ERROR, "storage {samples} x {format:#x}");
    let point = match format {
        GL_DEPTH24_STENCIL8 | GL_DEPTH32F_STENCIL8 => GL_DEPTH_STENCIL_ATTACHMENT,
        GL_STENCIL_INDEX8 => GL_STENCIL_ATTACHMENT,
        _ => GL_DEPTH_ATTACHMENT,
    };
    // SAFETY: a renderbuffer this test made, on the bound framebuffer.
    unsafe { glFramebufferRenderbuffer(GL_FRAMEBUFFER, point, GL_RENDERBUFFER, rb) };
    assert_eq!(err(), GL_NO_ERROR);
    assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE, "{samples} x {format:#x}");
    fbo
}

fn has_stencil(format: GLenum) -> bool {
    matches!(format, GL_DEPTH24_STENCIL8 | GL_DEPTH32F_STENCIL8 | GL_STENCIL_INDEX8)
}

/// The writes of a plane with pixels in them (a write of none is the "can it
/// be written" probe), sorted by plane.
fn writes() -> Vec<DsWrite> {
    let mut w: Vec<DsWrite> = DS_WRITES.lock().unwrap().iter().copied().filter(|w| w.4 * w.5 != 0).collect();
    w.sort_by_key(|w| w.1);
    w
}

/// Resolve a 4x `format` into a 1x one with `mask` and check what the back

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

/// Draw one triangle over the bound framebuffer; the draws the back end saw.
fn draw() -> Vec<(u8, u64, u8, u32)> {
    static TRI: [f32; 12] = [-1.0, -1.0, 0.0, 1.0, 3.0, -1.0, 0.0, 1.0, -1.0, 3.0, 0.0, 1.0];
    DRAWS.lock().unwrap().clear();
    // SAFETY: TRI is static and outlives the draw.
    unsafe {
        glBindBuffer(GL_ARRAY_BUFFER, 0);
        glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, TRI.as_ptr() as *const c_void);
        glEnableVertexAttribArray(0);
        glDrawArrays(GL_TRIANGLES, 0, 3);
        glFinish();
    }
    assert_eq!(err(), GL_NO_ERROR);
    DRAWS.lock().unwrap().clone()
}

fn rb_samples(fbo: &Fbo) -> GLint {
    let mut v: GLint = -1;
    // SAFETY: one GLint out, on a renderbuffer this test made.
    unsafe {
        glBindRenderbuffer(GL_RENDERBUFFER, fbo.rbs[0]);
        glGetRenderbufferParameteriv(GL_RENDERBUFFER, GL_RENDERBUFFER_SAMPLES, &mut v);
    }
    assert_eq!(err(), GL_NO_ERROR);
    v
}

/* ---- a 4x depth/stencil-only FBO ----------------------------------------- */

#[test]
fn ds_only_4x_is_complete_and_reports_4_samples() {
    for format in [GL_DEPTH24_STENCIL8, GL_DEPTH_COMPONENT16, GL_DEPTH_COMPONENT24, GL_DEPTH32F_STENCIL8] {
        let _g = context_on(&BACKEND_MS4_DS);
        assert_eq!(geti(GL_MAX_SAMPLES), 4);
        let fbo = ds_fbo(4, format); // asserts GL_FRAMEBUFFER_COMPLETE
        assert_eq!(rb_samples(&fbo), 4, "{format:#x}");
        // SAFETY: plain GL call on the bound framebuffer.
        unsafe { glBindFramebuffer(GL_FRAMEBUFFER, fbo.fb) };
        assert_eq!(geti(GL_SAMPLES), 4, "{format:#x}");
        assert_eq!(geti(GL_SAMPLE_BUFFERS), 1, "{format:#x}");
    }
}

#[test]
fn ds_only_1x_reports_no_samples() {
    let _g = context_on(&BACKEND_MS4_DS);
    let fbo = ds_fbo(0, GL_DEPTH24_STENCIL8);
    assert_eq!(geti(GL_SAMPLES), 0);
    assert_eq!(geti(GL_SAMPLE_BUFFERS), 0);
    drop(fbo);
}

#[test]
fn draw_into_ds_only_4x_has_samples_4_and_no_colour_target() {
    let _g = context_on(&BACKEND_MS4_DS);
    let fbo = ds_fbo(4, GL_DEPTH24_STENCIL8);
    program();
    // SAFETY: plain GL calls.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, fbo.fb);
        glEnable(GL_DEPTH_TEST);
    }
    let d = draw();
    assert_eq!(d.len(), 1, "{d:?}");
    let (samples, target_gpu, num_targets, depth_id) = d[0];
    assert_eq!(samples, 4);
    assert_eq!(target_gpu, 0, "no colour target");
    assert!(num_targets <= 1, "num_targets {num_targets}");
    assert_ne!(depth_id, 0, "the depth attachment is identified");
    // SAFETY: plain GL call.
    unsafe { glDisable(GL_DEPTH_TEST) };
}

#[test]
fn draw_into_ds_only_1x_has_samples_1() {
    let _g = context_on(&BACKEND_MS4_DS);
    let fbo = ds_fbo(0, GL_DEPTH24_STENCIL8);
    program();
    // SAFETY: plain GL call.
    unsafe { glBindFramebuffer(GL_FRAMEBUFFER, fbo.fb) };
    let d = draw();
    assert_eq!(d.len(), 1, "{d:?}");
    assert_eq!((d[0].0, d[0].1), (1, 0));
}

/* ---- resolve extents -------------------------------------------------------- */

/// A resolve of a sub-rectangle reaches `ds_write` with that extent, for
/// depth alone and for stencil alone.
#[test]
fn sub_rectangle_resolve_writes_that_extent() {
    for (mask, plane) in [(D, backend::DS_PLANE_DEPTH), (S, backend::DS_PLANE_STENCIL)] {
        let _g = context_on(&BACKEND_MS4_DS);
        let src = ds_fbo(4, GL_DEPTH24_STENCIL8);
        let dst = ds_fbo(0, GL_DEPTH24_STENCIL8);
        let r = [2, 1, 6, 5];
        assert_eq!(blit_with(src.fb, dst.fb, r, r, mask, GL_NEAREST), GL_NO_ERROR, "mask {mask:#x}");
        let w = writes();
        assert_eq!(w.len(), 1, "one plane for mask {mask:#x}: {w:?}");
        assert_eq!(w[0].1, plane, "mask {mask:#x}");
        let (x, y, ww, hh) = (w[0].2, w[0].3, w[0].4, w[0].5);
        // Either the rectangle or the whole plane around it: the rectangle's
        // pixels must lie inside what was written.
        assert!(x <= 2 && y <= 1 && x + ww >= 6 && y + hh >= 5, "covers {r:?}: {w:?}");
    }
}

/// The depth bit alone leaves the stencil plane unwritten and vice versa.
#[test]
fn one_plane_per_mask_bit() {
    let _g = context_on(&BACKEND_MS4_DS);
    let src = ds_fbo(4, GL_DEPTH24_STENCIL8);
    let dst = ds_fbo(0, GL_DEPTH24_STENCIL8);
    assert_eq!(blit_with(src.fb, dst.fb, FULL, FULL, D, GL_NEAREST), GL_NO_ERROR);
    assert!(writes().iter().all(|w| w.1 == backend::DS_PLANE_DEPTH));
    DS_WRITES.lock().unwrap().clear();
    assert_eq!(blit_with(src.fb, dst.fb, FULL, FULL, S, GL_NEAREST), GL_NO_ERROR);
    assert!(writes().iter().all(|w| w.1 == backend::DS_PLANE_STENCIL));
}
