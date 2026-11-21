// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! ES 3.0 multisample renderbuffers, the GL core's format queries and storage
//! for every sized format, on the host
//! (docs/notes/es3-msaa-renderbuffers.md (a); ES 3.0.6 6.1.15 and 4.4.2).
//!
//! `glGetInternalformativ(GL_RENDERBUFFER, ..)` and
//! `glRenderbufferStorageMultisample` are driven through the API into the
//! stand-in back ends of tests/es3_msaa_gl.rs (copied: that file is owned by
//! another item): the capability off (`max_samples` 0, today's answers) and
//! on (`max_samples` 4, `context_ms4`). Every sized format of
//! `es3_formats::FORMATS` and the depth/stencil formats is asked.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_msaa_gl_iformat
//! ```

#![allow(dead_code)]
#![cfg(all(feature = "host-test", feature = "shaderc-online", not(feature = "report-es2")))]

use core::ffi::{c_char, c_void};
use std::sync::atomic::Ordering;
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{self, GlesDraw, GlesHeap, YakoglBackend};
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

/// The renderbuffer the last call made: deleted by the next, so a loop over
/// every format does not run out of names.
static LAST_RB: Mutex<GLuint> = Mutex::new(0);

/// `glRenderbufferStorageMultisample(.., samples, format, 8, 8)` on a fresh
/// renderbuffer (left bound): its error and the renderbuffer.
fn storage(samples: GLsizei, format: GLenum) -> (GLenum, GLuint) {
    let mut last = LAST_RB.lock().unwrap_or_else(|e| e.into_inner());
    // SAFETY: a name this file made (0 is ignored).
    unsafe { glDeleteRenderbuffers(1, &*last) };
    let mut rb = 0;
    // SAFETY: plain GL calls.
    unsafe {
        glGenRenderbuffers(1, &mut rb);
        glBindRenderbuffer(GL_RENDERBUFFER, rb);
        while glGetError() != GL_NO_ERROR {}
        glRenderbufferStorageMultisample(GL_RENDERBUFFER, samples, format, 8, 8);
    }
    *last = rb;
    (err(), rb)
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

/* ---- the formats ------------------------------------------------------- */

/// ES 3.0 table 3.13's colour formats that have multisample storage.
const COLOR_MS: [GLenum; 9] =
    [GL_RGBA8, GL_SRGB8_ALPHA8, GL_RGB10_A2, GL_RGB8, GL_RGB565, GL_RGBA4, GL_RGB5_A1, GL_R8, GL_RG8];
const DEPTH_STENCIL: [GLenum; 6] = [
    GL_DEPTH_COMPONENT16,
    GL_DEPTH_COMPONENT24,
    GL_DEPTH_COMPONENT32F,
    GL_DEPTH24_STENCIL8,
    GL_DEPTH32F_STENCIL8,
    GL_STENCIL_INDEX8,
];
/// Colour-renderable integer formats (table 3.13): 0 sample counts.
const INT_RENDERABLE: [GLenum; 19] = [
    GL_R8I,
    GL_R8UI,
    GL_R16I,
    GL_R16UI,
    GL_R32I,
    GL_R32UI,
    GL_RG8I,
    GL_RG8UI,
    GL_RG16I,
    GL_RG16UI,
    GL_RG32I,
    GL_RG32UI,
    GL_RGBA8I,
    GL_RGBA8UI,
    GL_RGBA16I,
    GL_RGBA16UI,
    GL_RGBA32I,
    GL_RGBA32UI,
    GL_RGB10_A2UI,
];

#[derive(Debug, PartialEq, Clone, Copy)]
enum Want {
    /// Colour, depth or stencil: the counts listed.
    Counts,
    /// An integer colour format: none.
    NoCounts,
    /// Renderable only through GL_EXT_color_buffer_half_float / _float: the
    /// spec table 3.13 says GL_INVALID_ENUM, the GL core takes the storage;
    /// not asserted.
    Ext,
    /// Not renderable: GL_INVALID_ENUM.
    Enum,
}

fn want(f: GLenum) -> Want {
    if COLOR_MS.contains(&f) || DEPTH_STENCIL.contains(&f) {
        Want::Counts
    } else if INT_RENDERABLE.contains(&f) {
        Want::NoCounts
    } else if f == GL_RGB16F {
        Want::Ext
    } else {
        Want::Enum
    }
}

/// Every sized format asked: `FORMATS` (renderable or not), the colour and
/// depth/stencil formats of ES 3.0.
fn all_formats() -> Vec<(GLenum, String)> {
    let mut v: Vec<(GLenum, String)> =
        yakogl::gl::es3_formats::FORMATS.iter().map(|e| (e.internalformat, e.name.to_string())).collect();
    for f in COLOR_MS.iter().chain(DEPTH_STENCIL.iter()) {
        if !v.iter().any(|(g, _)| g == f) {
            v.push((*f, format!("{f:#x}")));
        }
    }
    v
}

/// The counts a format lists with the back end keeping `max` samples.
fn counts(max: i32) -> Vec<GLint> {
    if max >= 4 {
        vec![4]
    } else {
        vec![1]
    }
}

fn check_counts(max: i32) {
    for (f, name) in all_formats() {
        let (n, e) = internalformat(f, GL_NUM_SAMPLE_COUNTS, 1);
        match want(f) {
            Want::Ext => {}
            Want::Enum => {
                assert_eq!(e, GL_INVALID_ENUM, "{name}: NUM_SAMPLE_COUNTS");
                assert_eq!(n[0], -777, "{name}: written on error");
                let (_, e) = internalformat(f, GL_SAMPLES, 4);
                assert_eq!(e, GL_INVALID_ENUM, "{name}: SAMPLES");
            }
            Want::NoCounts => {
                assert_eq!((e, n[0]), (GL_NO_ERROR, 0), "{name}: integer formats have no counts");
                let (v, e) = internalformat(f, GL_SAMPLES, 4);
                assert_eq!((e, v), (GL_NO_ERROR, vec![-777; 4]), "{name}: SAMPLES writes none");
            }
            Want::Counts => {
                let c = counts(max);
                assert_eq!((e, n[0]), (GL_NO_ERROR, c.len() as GLint), "{name}: NUM_SAMPLE_COUNTS");
                let (v, e) = internalformat(f, GL_SAMPLES, 4);
                assert_eq!(e, GL_NO_ERROR, "{name}");
                assert_eq!(&v[..c.len()], &c[..], "{name}: SAMPLES");
                assert!(v[c.len()..].iter().all(|x| *x == -777), "{name}: wrote past the count");
                assert!(c.windows(2).all(|w| w[0] > w[1]), "{name}: descending");
            }
        }
    }
}

#[test]
fn counts_capability_on() {
    let _g = context_ms4();
    assert_eq!(geti(GL_MAX_SAMPLES), 4);
    check_counts(4);
}

/// The capability off keeps today's answer, the one count 1 (below ES 3.0's
/// minimum of 4: the on table is the one that meets the spec).
#[test]
fn counts_capability_off() {
    let _g = context();
    assert_eq!(geti(GL_MAX_SAMPLES), 1);
    check_counts(0);
}

#[test]
fn bad_target_pname_and_size() {
    for table in [&BACKEND, &BACKEND_MS4] {
        let _g = context_on(table);
        let mut v: GLint = -777;
        for t in [GL_TEXTURE_2D, GL_FRAMEBUFFER, GL_NONE, GL_RENDERBUFFER_SAMPLES] {
            // SAFETY: one GLint out.
            unsafe { glGetInternalformativ(t, GL_RGBA8, GL_SAMPLES, 1, &mut v) };
            assert_eq!(err(), GL_INVALID_ENUM, "target {t:#x}");
        }
        // SAFETY: one GLint out.
        unsafe { glGetInternalformativ(GL_RENDERBUFFER, GL_RGBA8, GL_RENDERBUFFER_SAMPLES, 1, &mut v) };
        assert_eq!(err(), GL_INVALID_ENUM, "pname");
        // SAFETY: as above; a negative bufSize.
        unsafe { glGetInternalformativ(GL_RENDERBUFFER, GL_RGBA8, GL_SAMPLES, -1, &mut v) };
        assert_eq!(err(), GL_INVALID_VALUE, "negative bufSize");
        assert_eq!(v, -777);
    }
}

#[test]
fn buf_size_zero_writes_nothing() {
    for table in [&BACKEND, &BACKEND_MS4] {
        let _g = context_on(table);
        for (f, name) in all_formats() {
            if matches!(want(f), Want::Enum | Want::Ext) {
                continue;
            }
            for pname in [GL_NUM_SAMPLE_COUNTS, GL_SAMPLES] {
                let mut x: GLint = -777;
                // SAFETY: bufSize 0 writes nothing.
                unsafe { glGetInternalformativ(GL_RENDERBUFFER, f, pname, 0, &mut x) };
                assert_eq!((err(), x), (GL_NO_ERROR, -777), "{name} {pname:#x}");
            }
        }
    }
}

/// GL_SAMPLES with a bufSize of 1 writes the first (largest) count only.
#[test]
fn samples_truncated_to_buf_size() {
    let _g = context_ms4();
    let mut v = [-777; 2];
    // SAFETY: two GLints out, one written.
    unsafe { glGetInternalformativ(GL_RENDERBUFFER, GL_RGBA8, GL_SAMPLES, 1, v.as_mut_ptr()) };
    assert_eq!((err(), v), (GL_NO_ERROR, [4, -777]));
}

/* ---- glRenderbufferStorageMultisample ---------------------------------- */

fn check_storage(max: i32) {
    for (f, name) in all_formats() {
        for samples in [0, 1, 2, 3, 4] {
            let (e, rb) = storage(samples, f);
            match want(f) {
                Want::Ext => {}
                Want::Enum => assert_eq!(e, GL_INVALID_ENUM, "{name} x{samples}"),
                Want::NoCounts if samples > 0 => {
                    assert_eq!(e, GL_INVALID_OPERATION, "{name} x{samples}: integer")
                }
                _ if samples > max.max(1) => {
                    assert_eq!(e, GL_INVALID_OPERATION, "{name} x{samples}: past MAX_SAMPLES")
                }
                _ => {
                    assert_eq!(e, GL_NO_ERROR, "{name} x{samples}");
                    let got = rb_samples(rb);
                    if samples == 0 {
                        assert_eq!(got, 0, "{name} x0");
                    } else {
                        assert!(got >= samples, "{name}: asked {samples}, stored {got}");
                        assert!(got <= max.max(1), "{name}: stored {got} past MAX_SAMPLES");
                    }
                }
            }
        }
    }
}

#[test]
fn storage_capability_on() {
    let _g = context_ms4();
    check_storage(4);
    /* the count is 4 for a request of 1..=4 */
    for f in COLOR_MS.iter().chain(DEPTH_STENCIL.iter()) {
        for samples in 1..=4 {
            let (e, rb) = storage(samples, *f);
            assert_eq!(e, GL_NO_ERROR);
            assert_eq!(rb_samples(rb), 4, "{f:#x} x{samples}");
        }
    }
}

#[test]
fn storage_capability_off() {
    let _g = context();
    check_storage(0);
}

#[test]
fn storage_past_max_samples() {
    let _g = context_ms4();
    for samples in [5, 8, 16, 1 << 20] {
        assert_eq!(storage(samples, GL_RGBA8).0, GL_INVALID_OPERATION, "x{samples}");
    }
    assert_eq!(storage(-1, GL_RGBA8).0, GL_INVALID_VALUE);
}
