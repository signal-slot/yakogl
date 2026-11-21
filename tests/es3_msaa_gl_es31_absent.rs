// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! ES 3.1-only multisample entry points and enums stay absent from the ES 3.0
//! context, on the 4x stand-in back end (docs/notes/es3-msaa-renderbuffers.md).
//!
//! Multisample textures, `GL_SAMPLE_MASK` and the three ES 3.1 sample-count
//! limits are not part of ES 3.0: each is `GL_INVALID_ENUM`. The library does
//! not export `glTexStorage2DMultisample` at all (no symbol in src/ or
//! capi/), so there is no call to pin; `no_multisample_texture_storage_symbol`
//! pins that by source search. `GL_SAMPLE_COVERAGE` and
//! `GL_SAMPLE_ALPHA_TO_COVERAGE` are ES 2.0 and stay accepted.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_msaa_gl_es31_absent
//! ```

#![cfg(all(feature = "host-test", feature = "shaderc-online", not(feature = "report-es2")))]
#![allow(dead_code)] // helpers copied whole from es3_msaa_gl.rs

use core::ffi::{c_char, c_void};
use std::sync::atomic::Ordering;
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{self, GlesDraw, GlesHeap, YakoglBackend};
use yakogl::gl::consts::*;
use yakogl::gl::framebuffer::*;
use yakogl::gl::state::*;
use yakogl::gl::texture::{glBindTexture, glDeleteTextures, glGenTextures};
use yakogl::gl::types::*;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};

const GL_SAMPLE_MASK: GLenum = 0x8e51;
const GL_MAX_COLOR_TEXTURE_SAMPLES: GLenum = 0x910e;
const GL_MAX_DEPTH_TEXTURE_SAMPLES: GLenum = 0x910f;
const GL_MAX_INTEGER_SAMPLES: GLenum = 0x9110;

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

/// The error of `glGetIntegerv(pname)`.
fn geti_err(pname: GLenum) -> GLenum {
    let mut v: GLint = -777;
    // SAFETY: one GLint out.
    unsafe { glGetIntegerv(pname, &mut v) };
    err()
}

#[test]
fn bind_multisample_texture_is_invalid_enum() {
    let _g = context_ms4();
    let mut t = 0;
    // SAFETY: plain GL calls.
    unsafe {
        glGenTextures(1, &mut t);
        glBindTexture(GL_TEXTURE_2D_MULTISAMPLE, t);
    }
    assert_eq!(err(), GL_INVALID_ENUM);
    // SAFETY: plain GL calls.
    unsafe { glDeleteTextures(1, &t) };
}

#[test]
fn es31_sample_limits_are_invalid_enum() {
    let _g = context_ms4();
    for p in [GL_MAX_COLOR_TEXTURE_SAMPLES, GL_MAX_DEPTH_TEXTURE_SAMPLES, GL_MAX_INTEGER_SAMPLES] {
        assert_eq!(geti_err(p), GL_INVALID_ENUM, "pname {p:#x}");
    }
}

#[test]
fn sample_mask_enable_is_invalid_enum() {
    let _g = context_ms4();
    // SAFETY: plain GL calls.
    unsafe { glEnable(GL_SAMPLE_MASK) };
    assert_eq!(err(), GL_INVALID_ENUM);
    // SAFETY: plain GL calls.
    unsafe { glDisable(GL_SAMPLE_MASK) };
    assert_eq!(err(), GL_INVALID_ENUM);
}

#[test]
fn internalformat_multisample_texture_target_is_invalid_enum() {
    let _g = context_ms4();
    let mut v: GLint = -777;
    // SAFETY: room for one GLint.
    unsafe { glGetInternalformativ(GL_TEXTURE_2D_MULTISAMPLE, GL_RGBA8, GL_SAMPLES, 1, &mut v) };
    assert_eq!(err(), GL_INVALID_ENUM);
    assert_eq!(v, -777);
}

/// No `glTexStorage2DMultisample` is exported: neither src/ nor capi/ names it.
#[test]
fn no_multisample_texture_storage_symbol() {
    fn scan(dir: &std::path::Path, hits: &mut Vec<String>) {
        let Ok(rd) = std::fs::read_dir(dir) else { return };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                scan(&p, hits);
            } else if let Ok(s) = std::fs::read_to_string(&p) {
                if s.contains("TexStorage2DMultisample") {
                    hits.push(p.display().to_string());
                }
            }
        }
    }
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut hits = Vec::new();
    scan(&root.join("src"), &mut hits);
    scan(&root.join("capi"), &mut hits);
    assert!(hits.is_empty(), "exported: {hits:?}");
}

#[test]
fn es2_sample_state_is_accepted() {
    let _g = context_ms4();
    for cap in [GL_SAMPLE_COVERAGE, GL_SAMPLE_ALPHA_TO_COVERAGE] {
        // SAFETY: plain GL calls.
        unsafe {
            glEnable(cap);
            assert_eq!(glGetError(), GL_NO_ERROR, "enable {cap:#x}");
            assert_eq!(glIsEnabled(cap) as u32, GL_TRUE);
            glDisable(cap);
            assert_eq!(glGetError(), GL_NO_ERROR, "disable {cap:#x}");
            assert_eq!(glIsEnabled(cap) as u32, GL_FALSE);
        }
    }
}
