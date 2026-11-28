// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! ES 3.0 multisample renderbuffers: the depth/stencil sized formats at 4x,
//! one matrix row per format (docs/notes/es3-msaa-renderbuffers.md (b)).
//!
//! The stand-in back end says `max_samples` 4 and has no depth/stencil
//! planes: this file watches queries, storage, completeness and the errors of
//! the resolve blit, not pixels. Stand-in and helpers are copied from
//! tests/es3_msaa_gl_ds.rs.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_msaa_gl_ds_formats
//! ```
#![cfg(all(feature = "host-test", feature = "shaderc-online", not(feature = "report-es2")))]

use core::ffi::{c_char, c_void};
use std::sync::atomic::Ordering;
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{self, GlesHeap, YakoglBackend};
use yakogl::gl::consts::*;
use yakogl::gl::framebuffer::*;
use yakogl::gl::state::*;
use yakogl::gl::types::*;
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
    name: c"es3-msaa-gl-ds-formats".as_ptr() as *const c_char,
    native_attrib_formats: 0,
    rt_rgba8888_is_bgra: false,
    mip_level_align: 0,
    compressed_blocks_in_rows: false,
    native_layered_textures: false,
    front_facing_shader_variant: false,
    etc1_decoded_at_upload: false,
    init: Some(b_init),
    shutdown: None,
    draw: None,
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

/// (format, depth bits, stencil bits)
const FORMATS: [(GLenum, GLint, GLint); 6] = [
    (GL_DEPTH_COMPONENT16, 16, 0),
    (GL_DEPTH_COMPONENT24, 24, 0),
    (GL_DEPTH_COMPONENT32F, 32, 0),
    (GL_DEPTH24_STENCIL8, 24, 8),
    (GL_DEPTH32F_STENCIL8, 32, 8),
    (GL_STENCIL_INDEX8, 0, 8),
];

fn rb_param(pname: GLenum) -> GLint {
    let mut v = -777;
    // SAFETY: one GLint out.
    unsafe { glGetRenderbufferParameteriv(GL_RENDERBUFFER, pname, &mut v) };
    assert_eq!(err(), GL_NO_ERROR);
    v
}

#[test]
fn internalformat_samples_is_4() {
    let _g = context_ms4();
    for (f, _, _) in FORMATS {
        let (mut n, mut s) = (-1, [-1; 4]);
        // SAFETY: room for one count and four samples.
        unsafe {
            glGetInternalformativ(GL_RENDERBUFFER, f, GL_NUM_SAMPLE_COUNTS, 1, &mut n);
            glGetInternalformativ(GL_RENDERBUFFER, f, GL_SAMPLES, 4, s.as_mut_ptr());
        }
        assert_eq!(err(), GL_NO_ERROR, "{f:#x}");
        assert!(n >= 1, "{f:#x} count {n}");
        assert_eq!(s[0], 4, "{f:#x} largest first");
    }
}

#[test]
fn storage_keeps_4_samples_and_sizes() {
    let _g = context_ms4();
    for (f, d, s) in FORMATS {
        let (e, rb) = storage(4, f);
        assert_eq!(e, GL_NO_ERROR, "{f:#x}");
        assert_eq!(rb_param(GL_RENDERBUFFER_SAMPLES), 4, "{f:#x}");
        assert_eq!(rb_param(GL_RENDERBUFFER_INTERNAL_FORMAT), f as GLint);
        assert_eq!(rb_param(GL_RENDERBUFFER_DEPTH_SIZE), d, "{f:#x} depth");
        assert_eq!(rb_param(GL_RENDERBUFFER_STENCIL_SIZE), s, "{f:#x} stencil");
        // SAFETY: a name this test made.
        unsafe { glDeleteRenderbuffers(1, &rb) };
    }
}

/// (A request of 1 sample is rounded up to 4 here; 0 is the single-sample one.)
/// A framebuffer with a colour renderbuffer of `cs` samples and a
/// depth/stencil one of `ds` samples of `format`.
fn color_ds(cs: GLsizei, ds: GLsizei, format: GLenum) -> Fbo {
    let mut fbo = framebuffer(cs, &[GL_RGBA8]).unwrap();
    let (e, rb) = storage(ds, format);
    fbo.rbs.push(rb);
    assert_eq!(e, GL_NO_ERROR);
    let point = match format {
        GL_DEPTH24_STENCIL8 | GL_DEPTH32F_STENCIL8 => GL_DEPTH_STENCIL_ATTACHMENT,
        GL_STENCIL_INDEX8 => GL_STENCIL_ATTACHMENT,
        _ => GL_DEPTH_ATTACHMENT,
    };
    // SAFETY: a renderbuffer this test made, on the bound framebuffer.
    unsafe { glFramebufferRenderbuffer(GL_FRAMEBUFFER, point, GL_RENDERBUFFER, rb) };
    assert_eq!(err(), GL_NO_ERROR);
    fbo
}

#[test]
fn complete_with_4x_color() {
    let _g = context_ms4();
    for (f, _, _) in FORMATS {
        let _fbo = color_ds(4, 4, f);
        assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE, "{f:#x}");
    }
}

#[test]
fn incomplete_multisample_with_1x_color() {
    let _g = context_ms4();
    for (f, _, _) in FORMATS {
        let _fbo = color_ds(0, 4, f);
        assert_eq!(status(), GL_FRAMEBUFFER_INCOMPLETE_MULTISAMPLE, "4x {f:#x} with 0-sample colour");
    }
}

#[test]
fn incomplete_multisample_with_1x_ds_and_4x_color() {
    let _g = context_ms4();
    for (f, _, _) in FORMATS {
        let _fbo = color_ds(4, 0, f);
        assert_eq!(status(), GL_FRAMEBUFFER_INCOMPLETE_MULTISAMPLE, "0-sample {f:#x} with 4x colour");
    }
}

#[test]
fn resolve_format_mismatch_is_invalid_operation() {
    let _g = context_ms4();
    for (f, fd, fs) in FORMATS {
        for (g, gd, gs) in FORMATS {
            if f == g {
                continue;
            }
            let src = ds_fbo(4, f);
            let dst = ds_fbo(0, g);
            for mask in [D, S, D | S] {
                // A plane only one side has is not blitted: no error there.
                let both = (mask & D != 0 && fd > 0 && gd > 0) || (mask & S != 0 && fs > 0 && gs > 0);
                let want = if both { GL_INVALID_OPERATION } else { GL_NO_ERROR };
                assert_eq!(
                    blit_with(src.fb, dst.fb, FULL, FULL, mask, GL_NEAREST),
                    want,
                    "4x {f:#x} -> 1x {g:#x} mask {mask:#x}"
                );
            }
        }
    }
}

#[test]
fn resolve_linear_with_depth_or_stencil_is_invalid_operation() {
    let _g = context_ms4();
    for (f, _, _) in FORMATS {
        let src = ds_fbo(4, f);
        let dst = ds_fbo(0, f);
        for mask in [D, S, D | S] {
            assert_eq!(
                blit_with(src.fb, dst.fb, FULL, FULL, mask, GL_LINEAR),
                GL_INVALID_OPERATION,
                "{f:#x} mask {mask:#x}"
            );
        }
    }
}

#[test]
fn resolve_region_mismatch_is_invalid_operation() {
    let _g = context_ms4();
    for (f, _, _) in FORMATS {
        let src = ds_fbo(4, f);
        let dst = ds_fbo(0, f);
        for (s, d) in [([0, 0, 8, 8], [0, 0, 4, 4]), ([0, 0, 4, 4], [0, 0, 8, 8]), ([0, 0, 4, 4], [2, 2, 6, 6])] {
            assert_eq!(
                blit_with(src.fb, dst.fb, s, d, D | S, GL_NEAREST),
                GL_INVALID_OPERATION,
                "{f:#x} {s:?} -> {d:?}"
            );
        }
    }
}
