// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! ES 3.0 multisample renderbuffers, the GL core's depth/stencil resolve, on
//! the host (docs/notes/es3-msaa-renderbuffers.md (b)8; src/gl/blit.rs).
//!
//! The stand-in back end says `max_samples` 4 and keeps depth/stencil planes
//! per object (`depth_resolve`, `stencil_resolve`, `ds_write`), one sample a
//! pixel. A resolve blit of a 4x depth/stencil renderbuffer into a 1x one of
//! the same format must read the source's planes through the back end and
//! write each whole into the destination's; the tests watch those three
//! entries. The stand-in and its helpers are copied from tests/es3_msaa_gl.rs.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_msaa_gl_ds
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
    name: c"es3-msaa-gl-ds".as_ptr() as *const c_char,
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
        _ => GL_DEPTH_ATTACHMENT,
    };
    // SAFETY: a renderbuffer this test made, on the bound framebuffer.
    unsafe { glFramebufferRenderbuffer(GL_FRAMEBUFFER, point, GL_RENDERBUFFER, rb) };
    assert_eq!(err(), GL_NO_ERROR);
    assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE, "{samples} x {format:#x}");
    fbo
}

fn has_stencil(format: GLenum) -> bool {
    matches!(format, GL_DEPTH24_STENCIL8 | GL_DEPTH32F_STENCIL8)
}

/// The writes of a plane with pixels in them (a write of none is the "can it
/// be written" probe), sorted by plane.
fn writes() -> Vec<DsWrite> {
    let mut w: Vec<DsWrite> = DS_WRITES.lock().unwrap().iter().copied().filter(|w| w.4 * w.5 != 0).collect();
    w.sort_by_key(|w| w.1);
    w
}

/// Resolve a 4x `format` into a 1x one with `mask` and check what the back
/// end saw: each plane asked for and held by the format read from the 4x
/// object and written whole, with one sample's value, into the other.
fn resolve_case(format: GLenum, mask: GLbitfield) {
    let _g = context_on(&BACKEND_MS4_DS);
    assert_eq!(geti(GL_MAX_SAMPLES), 4);
    let src = ds_fbo(4, format);
    let dst = ds_fbo(0, format);
    assert_eq!(blit_with(src.fb, dst.fb, FULL, FULL, mask, GL_NEAREST), GL_NO_ERROR, "{format:#x} mask {mask:#x}");
    let mut want: Vec<(u32, u32)> = Vec::new();
    if mask & D != 0 {
        want.push((backend::DS_PLANE_DEPTH, DEPTH.to_bits()));
    }
    if mask & S != 0 && has_stencil(format) {
        want.push((backend::DS_PLANE_STENCIL, STENCIL as u32));
    }
    want.sort();
    let w = writes();
    let got: Vec<(u32, u32)> = w.iter().map(|w| (w.1, w.6)).collect();
    assert_eq!(got, want, "{format:#x} mask {mask:#x}: {w:?}");
    let dst_id = w.first().map(|w| w.0);
    for x in &w {
        assert_eq!((x.2, x.3, x.4, x.5), (0, 0, 8, 8), "the whole plane: {x:?}");
        assert_eq!(Some(x.0), dst_id, "one destination object");
    }
    if let Some(dst_id) = dst_id {
        let reads = DS_READS.lock().unwrap().clone();
        assert!(reads.iter().any(|r| r.0 != dst_id), "the 4x source's plane was read: {reads:?}");
        assert!(reads.iter().all(|r| (r.1, r.2) == (8, 8)), "whole 8x8 planes: {reads:?}");
    }
}

/* ---- the resolve, every format and mask ---------------------------------- */

#[test]
fn depth16_depth() {
    resolve_case(GL_DEPTH_COMPONENT16, D);
}
#[test]
fn depth16_stencil_bit_is_ignored() {
    resolve_case(GL_DEPTH_COMPONENT16, S);
}
#[test]
fn depth16_both() {
    resolve_case(GL_DEPTH_COMPONENT16, D | S);
}
#[test]
fn depth24_depth() {
    resolve_case(GL_DEPTH_COMPONENT24, D);
}
#[test]
fn depth24_stencil_bit_is_ignored() {
    resolve_case(GL_DEPTH_COMPONENT24, S);
}
#[test]
fn depth24_both() {
    resolve_case(GL_DEPTH_COMPONENT24, D | S);
}
#[test]
fn depth24_stencil8_depth() {
    resolve_case(GL_DEPTH24_STENCIL8, D);
}
#[test]
fn depth24_stencil8_stencil() {
    resolve_case(GL_DEPTH24_STENCIL8, S);
}
#[test]
fn depth24_stencil8_both() {
    resolve_case(GL_DEPTH24_STENCIL8, D | S);
}
#[test]
fn depth32f_stencil8_depth() {
    resolve_case(GL_DEPTH32F_STENCIL8, D);
}
#[test]
fn depth32f_stencil8_stencil() {
    resolve_case(GL_DEPTH32F_STENCIL8, S);
}
#[test]
fn depth32f_stencil8_both() {
    resolve_case(GL_DEPTH32F_STENCIL8, D | S);
}

/* ---- the errors ------------------------------------------------------------ */

/// ES 3.0 4.3.3: a depth or stencil blit with GL_LINEAR is GL_INVALID_OPERATION,
/// before anything is copied.
#[test]
fn linear_with_depth_or_stencil_is_invalid_operation() {
    let _g = context_on(&BACKEND_MS4_DS);
    let src = ds_fbo(4, GL_DEPTH24_STENCIL8);
    let dst = ds_fbo(0, GL_DEPTH24_STENCIL8);
    for (name, mask) in [("depth", D), ("stencil", S), ("both", D | S), ("colour and depth", GL_COLOR_BUFFER_BIT | D)] {
        assert_eq!(blit_with(src.fb, dst.fb, FULL, FULL, mask, GL_LINEAR), GL_INVALID_OPERATION, "{name}");
    }
    assert!(DS_READS.lock().unwrap().is_empty() && writes().is_empty(), "nothing copied");
}

/// A depth format that differs between the read and the draw framebuffer is
/// GL_INVALID_OPERATION (the plane blitted must match).
#[test]
fn depth_format_mismatch_is_invalid_operation() {
    let _g = context_on(&BACKEND_MS4_DS);
    let formats = [GL_DEPTH_COMPONENT16, GL_DEPTH_COMPONENT24, GL_DEPTH24_STENCIL8, GL_DEPTH32F_STENCIL8];
    for r in formats {
        let src = ds_fbo(4, r);
        for d in formats {
            /* the same depth bits and type: D24 and D24S8 match as depth planes */
            let same_depth = r == d || [r, d].iter().all(|f| matches!(*f, GL_DEPTH_COMPONENT24 | GL_DEPTH24_STENCIL8));
            if same_depth {
                continue;
            }
            let dst = ds_fbo(0, d);
            let e = blit_with(src.fb, dst.fb, FULL, FULL, D, GL_NEAREST);
            assert_eq!(e, GL_INVALID_OPERATION, "depth {r:#x} -> {d:#x}");
        }
    }
    assert!(writes().is_empty(), "nothing copied");
}

/// The stencil plane of D24S8 and D32FS8 has the same bits but the depth
/// beside it differs: the other plane must match where both have it.
#[test]
fn stencil_with_a_different_depth_format_is_invalid_operation() {
    let _g = context_on(&BACKEND_MS4_DS);
    let src = ds_fbo(4, GL_DEPTH24_STENCIL8);
    let dst = ds_fbo(0, GL_DEPTH32F_STENCIL8);
    assert_eq!(blit_with(src.fb, dst.fb, FULL, FULL, S, GL_NEAREST), GL_INVALID_OPERATION);
    assert!(writes().is_empty(), "nothing copied");
}

/// ES 3.0: when only one side has a depth (or stencil) attachment the bit is
/// ignored, with no error and no copy.
#[test]
fn one_side_without_depth_is_a_no_op() {
    let _g = context_on(&BACKEND_MS4_DS);
    let with_ds = ds_fbo(4, GL_DEPTH24_STENCIL8);
    let colour_only = framebuffer(0, &[GL_RGBA8]).unwrap();
    for mask in [D, S, D | S] {
        assert_eq!(
            blit_with(with_ds.fb, colour_only.fb, FULL, FULL, mask, GL_NEAREST),
            GL_NO_ERROR,
            "4x -> none {mask:#x}"
        );
        assert_eq!(
            blit_with(colour_only.fb, ds_fbo_1x().fb, FULL, FULL, mask, GL_NEAREST),
            GL_NO_ERROR,
            "none -> 1x {mask:#x}"
        );
    }
    assert!(writes().is_empty(), "no plane written");
}

fn ds_fbo_1x() -> Fbo {
    ds_fbo(0, GL_DEPTH24_STENCIL8)
}

/// Stencil asked for where only the depth side has stencil: the format with
/// no stencil on the draw side drops the bit without an error.
#[test]
fn stencil_bit_with_a_depth_only_draw_side_is_a_no_op() {
    let _g = context_on(&BACKEND_MS4_DS);
    let src = ds_fbo(4, GL_DEPTH24_STENCIL8);
    let dst = ds_fbo(0, GL_DEPTH_COMPONENT24);
    assert_eq!(blit_with(src.fb, dst.fb, FULL, FULL, S, GL_NEAREST), GL_NO_ERROR);
    assert!(writes().is_empty(), "no stencil plane to write");
}
