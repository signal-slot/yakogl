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

/* ---- a destination no frame has drawn into (msaa-ds-design Option B) ------- */

/// The objects the stand-in below holds a depth/stencil buffer for: those a
/// draw named (`GlesDraw::depth_id` / `stencil_id`).
static HELD: Mutex<Vec<u32>> = Mutex::new(Vec::new());

/// What one draw the stand-in got asked for, of the fields a clear sets.
#[derive(Clone, Copy, Debug, PartialEq)]
struct SeenDraw {
    depth_id: u32,
    stencil_id: u32,
    do_clear_depth: bool,
    clear_depth: f32,
    do_clear_stencil: bool,
    clear_stencil: u8,
    clear_stencil_write_mask: u8,
    scissor: bool,
    color_write: bool,
    index_count: u32,
}

/// Every draw the stand-in got.
static DRAWS: Mutex<Vec<SeenDraw>> = Mutex::new(Vec::new());

unsafe extern "C" fn b_draw_holds(d: *const backend::GlesDraw) -> i32 {
    let d = &*d;
    DRAWS.lock().unwrap().push(SeenDraw {
        depth_id: d.depth_id,
        stencil_id: d.stencil_id,
        do_clear_depth: d.do_clear_depth,
        clear_depth: d.clear_depth,
        do_clear_stencil: d.do_clear_stencil,
        clear_stencil: d.clear_stencil,
        clear_stencil_write_mask: d.clear_stencil_write_mask,
        scissor: d.scissor,
        color_write: d.color_write,
        index_count: d.index_count,
    });
    let mut held = HELD.lock().unwrap();
    held.extend([d.depth_id, d.stencil_id].into_iter().filter(|&id| id != 0));
    0
}

/// `ds_write` of a back end that makes an object's buffer on its first draw
/// (the PowerVR before plan #400, the V3D): 1, "nothing held", for an object
/// no draw named.
#[allow(clippy::too_many_arguments)]
unsafe extern "C" fn b_ds_write_held(
    id: u32,
    plane: u32,
    src: *const c_void,
    x: u32,
    y: u32,
    w: u32,
    h: u32,
    pitch: u32,
) -> i32 {
    if !HELD.lock().unwrap().contains(&id) {
        return 1;
    }
    b_ds_write(id, plane, src, x, y, w, h, pitch)
}

/// `ds_write` that never holds a buffer, a clear or not.
#[allow(clippy::too_many_arguments)]
unsafe extern "C" fn b_ds_write_never(
    _id: u32,
    _plane: u32,
    _src: *const c_void,
    _x: u32,
    _y: u32,
    _w: u32,
    _h: u32,
    _pitch: u32,
) -> i32 {
    1
}

/// [`BACKEND_MS4_DS`] whose `ds_write` answers 1 for an object no draw named.
static BACKEND_MS4_DS_HELD: YakoglBackend =
    YakoglBackend { draw: Some(b_draw_holds), ds_write: Some(b_ds_write_held), ..BACKEND_MS4_DS };

/// [`BACKEND_MS4_DS_HELD`] whose `ds_write` holds nothing even after a clear.
static BACKEND_MS4_DS_NEVER: YakoglBackend = YakoglBackend { ds_write: Some(b_ds_write_never), ..BACKEND_MS4_DS_HELD };

/// [`BACKEND_MS4_DS`] without `ds_write`: it reads planes but cannot write one.
static BACKEND_MS4_NO_WRITE: YakoglBackend =
    YakoglBackend { draw: Some(b_draw_holds), ds_write: None, ..BACKEND_MS4_DS };

fn context_held(table: &'static YakoglBackend) -> MutexGuard<'static, ()> {
    let g = context_on(table);
    HELD.lock().unwrap().clear();
    DRAWS.lock().unwrap().clear();
    g
}

/// The renderbuffer serial id the back end sees for the destination: the
/// one `ds_write` writes.
fn dst_object(format: GLenum, mask: GLbitfield) -> u32 {
    let w = writes();
    assert!(!w.is_empty(), "{format:#x} mask {mask:#x}: a plane was written");
    w[0].0
}

/// A 4x `format` resolved into a fresh 1x one that no frame drew into: the
/// GL core clears the destination's refused planes first, the resolve then
/// goes ahead with no error.
fn undrawn_case(format: GLenum, mask: GLbitfield) {
    let _g = context_held(&BACKEND_MS4_DS_HELD);
    let src = ds_fbo(4, format);
    let dst = ds_fbo(0, format);
    DRAWS.lock().unwrap().clear();
    assert_eq!(blit_with(src.fb, dst.fb, FULL, FULL, mask, GL_NEAREST), GL_NO_ERROR, "{format:#x} mask {mask:#x}");
    let id = dst_object(format, mask);
    let depth = mask & D != 0 && format != GL_STENCIL_INDEX8;
    let stencil = mask & S != 0 && has_stencil(format);
    let draws = DRAWS.lock().unwrap().clone();
    let clears: Vec<SeenDraw> = draws.iter().copied().filter(|d| d.depth_id == id || d.stencil_id == id).collect();
    assert_eq!(clears.len(), 1, "one clear of the destination: {draws:?}");
    let k = clears[0];
    assert_eq!((k.do_clear_depth, k.do_clear_stencil), (depth, stencil), "the refused planes: {k:?}");
    if depth {
        assert_eq!(k.clear_depth, 1.0, "{k:?}");
    }
    if stencil {
        assert_eq!((k.clear_stencil, k.clear_stencil_write_mask), (0, 0xff), "{k:?}");
    }
    assert!(!k.scissor && !k.color_write && k.index_count == 0, "a whole-plane depth/stencil-only clear: {k:?}");
    assert_eq!(draws.len(), 1, "nothing else drawn: {draws:?}");
    /* the resolve wrote what the source holds, over the whole plane */
    let mut want: Vec<(u32, u32)> = Vec::new();
    if depth {
        want.push((backend::DS_PLANE_DEPTH, DEPTH.to_bits()));
    }
    if stencil {
        want.push((backend::DS_PLANE_STENCIL, STENCIL as u32));
    }
    let w = writes();
    assert_eq!(w.iter().map(|w| (w.1, w.6)).collect::<Vec<_>>(), want, "{w:?}");
    assert!(w.iter().all(|w| (w.0, w.2, w.3, w.4, w.5) == (id, 0, 0, 8, 8)), "{w:?}");
}

#[test]
fn undrawn_destination_depth16() {
    undrawn_case(GL_DEPTH_COMPONENT16, D);
}
#[test]
fn undrawn_destination_depth24_stencil8_both() {
    undrawn_case(GL_DEPTH24_STENCIL8, D | S);
}
#[test]
fn undrawn_destination_depth24_stencil8_stencil() {
    undrawn_case(GL_DEPTH24_STENCIL8, S);
}
#[test]
fn undrawn_destination_depth32f_stencil8_depth() {
    undrawn_case(GL_DEPTH32F_STENCIL8, D);
}
#[test]
fn undrawn_destination_stencil8() {
    undrawn_case(GL_STENCIL_INDEX8, S);
}

/// A destination a frame already holds is not cleared again: the probe
/// answers 0 and the blit is the resolve alone.
#[test]
fn held_destination_is_not_cleared() {
    let _g = context_held(&BACKEND_MS4_DS_HELD);
    let src = ds_fbo(4, GL_DEPTH24_STENCIL8);
    let dst = ds_fbo(0, GL_DEPTH24_STENCIL8);
    assert_eq!(blit_with(src.fb, dst.fb, FULL, FULL, D | S, GL_NEAREST), GL_NO_ERROR);
    DRAWS.lock().unwrap().clear();
    DS_WRITES.lock().unwrap().clear();
    assert_eq!(blit_with(src.fb, dst.fb, FULL, FULL, D | S, GL_NEAREST), GL_NO_ERROR);
    assert!(DRAWS.lock().unwrap().is_empty(), "no second clear: {:?}", DRAWS.lock().unwrap());
    assert_eq!(writes().len(), 2, "both planes written");
}

fn getb(pname: GLenum) -> Vec<GLboolean> {
    let mut v = [7 as GLboolean; 4];
    // SAFETY: room for four values.
    unsafe { glGetBooleanv(pname, v.as_mut_ptr()) };
    assert_eq!(err(), GL_NO_ERROR);
    v.to_vec()
}

fn getf(pname: GLenum) -> GLfloat {
    let mut v: GLfloat = -7.0;
    // SAFETY: one GLfloat out.
    unsafe { glGetFloatv(pname, &mut v) };
    assert_eq!(err(), GL_NO_ERROR);
    v
}

/// The clear the GL core issues for an undrawn destination runs under its
/// own scissor, masks and clear values: the application's are as they were
/// afterwards, and its pending clear state did not leak into it.
#[test]
fn undrawn_destination_leaves_state_unchanged() {
    let _g = context_held(&BACKEND_MS4_DS_HELD);
    let src = ds_fbo(4, GL_DEPTH24_STENCIL8);
    let dst = ds_fbo(0, GL_DEPTH24_STENCIL8);
    // SAFETY: plain GL calls.
    unsafe {
        glEnable(GL_SCISSOR_TEST);
        glScissor(1, 2, 3, 4);
        glDepthMask(GL_FALSE as GLboolean);
        glStencilMask(0x0f);
        glColorMask(0, 1, 0, 1);
        glClearDepthf(0.25);
        glClearStencil(7);
    }
    DRAWS.lock().unwrap().clear();
    /* the scissor cuts the resolve, not the clear before it */
    assert_eq!(blit_with(src.fb, dst.fb, FULL, FULL, D | S, GL_NEAREST), GL_NO_ERROR);
    let draws = DRAWS.lock().unwrap().clone();
    assert_eq!(draws.len(), 1, "{draws:?}");
    let k = draws[0];
    assert!(k.do_clear_depth && k.do_clear_stencil, "{k:?}");
    assert_eq!((k.clear_depth, k.clear_stencil, k.clear_stencil_write_mask), (1.0, 0, 0xff), "{k:?}");
    assert!(!k.scissor && !k.color_write, "{k:?}");
    // SAFETY: plain GL call.
    assert_eq!(unsafe { glIsEnabled(GL_SCISSOR_TEST) }, GL_TRUE as GLboolean);
    let mut b = [0; 4];
    // SAFETY: room for four values.
    unsafe { glGetIntegerv(GL_SCISSOR_BOX, b.as_mut_ptr()) };
    assert_eq!(b, [1, 2, 3, 4]);
    assert_eq!(getb(GL_DEPTH_WRITEMASK)[0], GL_FALSE as GLboolean);
    assert_eq!(geti(GL_STENCIL_WRITEMASK), 0x0f);
    assert_eq!(geti(GL_STENCIL_BACK_WRITEMASK), 0x0f);
    assert_eq!(getb(GL_COLOR_WRITEMASK), vec![0, 1, 0, 1]);
    assert_eq!(getf(GL_DEPTH_CLEAR_VALUE), 0.25);
    assert_eq!(geti(GL_STENCIL_CLEAR_VALUE), 7);
    // SAFETY: plain GL calls: the state put back for the next test.
    unsafe {
        glDisable(GL_SCISSOR_TEST);
        glScissor(0, 0, 8, 8);
        glDepthMask(GL_TRUE as GLboolean);
        glStencilMask(0xff);
        glColorMask(1, 1, 1, 1);
        glClearDepthf(1.0);
        glClearStencil(0);
    }
}

/// A back end that still holds nothing after the clear: the blit is refused
/// whole, as before, with nothing written.
#[test]
fn destination_still_unheld_after_the_clear_is_refused() {
    let _g = context_held(&BACKEND_MS4_DS_NEVER);
    let src = ds_fbo(4, GL_DEPTH24_STENCIL8);
    let dst = ds_fbo(0, GL_DEPTH24_STENCIL8);
    DRAWS.lock().unwrap().clear();
    assert_eq!(blit_with(src.fb, dst.fb, FULL, FULL, D | S, GL_NEAREST), GL_INVALID_OPERATION);
    assert_eq!(DRAWS.lock().unwrap().len(), 1, "the clear was tried once");
    assert!(writes().is_empty(), "nothing copied");
}

/// A back end without `ds_write` is refused as today, with no clear issued.
#[test]
fn back_end_without_ds_write_is_still_refused() {
    let _g = context_held(&BACKEND_MS4_NO_WRITE);
    let src = ds_fbo(4, GL_DEPTH24_STENCIL8);
    let dst = ds_fbo(0, GL_DEPTH24_STENCIL8);
    DRAWS.lock().unwrap().clear();
    for mask in [D, S, D | S] {
        assert_eq!(blit_with(src.fb, dst.fb, FULL, FULL, mask, GL_NEAREST), GL_INVALID_OPERATION, "{mask:#x}");
    }
    assert!(DRAWS.lock().unwrap().is_empty(), "no clear: {:?}", DRAWS.lock().unwrap());
    assert!(writes().is_empty(), "nothing copied");
}
