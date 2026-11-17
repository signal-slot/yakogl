// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! 4x multisampling with four colour targets on the V3D, through the GL API
//! (v3d-msaa4-test of plan 368, docs/notes/mrt4-v3d-pvr.md section 1), on the
//! host, under the four-target stand-in (`backend::HOST_DRAW_TARGETS` = 4, what
//! testutil's `backend_draws_targets(4)` stores).
//!
//! Pinned today (passing tests):
//!
//! - `GL_MAX_SAMPLES` is 1 whatever the target count (`src/gl/msaa.rs`
//!   `backend_max_samples` is 0), and `GL_MAX_DRAW_BUFFERS` /
//!   `GL_MAX_COLOR_ATTACHMENTS` are 4 under the stand-in;
//! - four RGBA8 renderbuffers on COLOR_ATTACHMENT0-3 are complete at 0 and 1
//!   sample; at 4 samples `glRenderbufferStorageMultisample` already fails
//!   with GL_INVALID_OPERATION, so no framebuffer object is ever 4x;
//! - on the complete (0 or 1 sample) four-target framebuffer: `glClear`,
//!   `glClearBufferfv` of each buffer, a four-output draw (what the back end
//!   is handed, what `state::refusal`/`unsupported`/`job_state` make of it)
//!   and the blit of each attachment into a single-sample framebuffer;
//! - the V3D state layer takes a hand-made 4x draw with four targets
//!   (`state::refusal`, `state::job_state`); `draw_call` refuses it after
//!   that, by name, at `src/v3d/backend.rs:6319`;
//! - the tile size and per-RT store counts the 4x four-RT job should carry
//!   are a table here (`expected_job`, Mesa's `v3d_choose_tile_size`).
//!
//! Not pinned: the control list. `driver/src/v3d/v3d_job.c` and
//! `v3d_frame.c` are C a host test does not reach (the tile table is a model,
//! not a call into `v3d_frame_tiling_bpp`), so tile bpp/size, the TLB loads
//! and stores per RT and the resolve are checked only as that table, never
//! against recorded words; no pixel is computed (the back end is a stand-in).
//!
//! The `#[ignore]`d tests are the expected result after the multisample design
//! (docs/notes/v3d-msaa-es3.md), each naming the owner and file:line at fault.
//! They fail today at storage time (GL core) or at the refusal in `draw_call`.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_mrt4_msaa_v3d
//! ```

#![cfg(all(feature = "host-test", feature = "shaderc-online", feature = "gpu-v3d", not(feature = "report-es2")))]

use core::ffi::{c_char, c_void};
use std::ffi::CString;
use std::sync::atomic::Ordering;
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{self, GlesDraw, GlesHeap, YakoglBackend, *};
use yakogl::gl::buffer::glBindBuffer;
use yakogl::gl::consts::*;
use yakogl::gl::draw::*;
use yakogl::gl::es3::glClearBufferfv;
use yakogl::gl::framebuffer::*;
use yakogl::gl::program::*;
use yakogl::gl::shader::*;
use yakogl::gl::state::*;
use yakogl::gl::types::*;
use yakogl::gl::vertex::{glEnableVertexAttribArray, glVertexAttribPointer};
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};
use yakogl::v3d::state;

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

/// What the back end was handed by one draw, and what the V3D made of it.
#[derive(Clone, Debug)]
struct Seen {
    /// An application draw (vertices), not a clear alone.
    draws: bool,
    /// `targets[..num_targets]`: (gpu, format).
    targets: Vec<(u64, u8)>,
    /// `GlesDraw::samples`.
    samples: u8,
    /// `GlesDraw::do_clear` and `clear_target_mask`.
    do_clear: bool,
    clear_mask: u8,
    /// `state::refusal` / `state::unsupported`.
    refused: Option<&'static str>,
    /// `state::job_state`'s SAMPLE_STATE mask, when the V3D takes the draw.
    sample_mask: Option<u32>,
}

static SEEN: Mutex<Vec<Seen>> = Mutex::new(Vec::new());

unsafe extern "C" fn b_draw(d: *const GlesDraw) -> i32 {
    let d = &*d;
    let n = (d.num_targets.max(1) as usize).min(GLES_MAX_TARGETS);
    let refused = state::refusal(d).or_else(|| state::unsupported(d));
    let sample_mask = if d.index_count != 0 { state::job_state(d).ok().map(|j| j.sample_mask) } else { None };
    SEEN.lock().unwrap().push(Seen {
        draws: d.index_count != 0,
        targets: d.targets[..n].iter().map(|t| (t.gpu, t.format)).collect(),
        samples: d.samples,
        do_clear: d.do_clear,
        clear_mask: d.clear_target_mask,
        refused,
        sample_mask,
    });
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
    name: c"es3-mrt4-msaa".as_ptr() as *const c_char,
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
};

/// One context for the whole file; the tests take turns on it.
static LOCK: Mutex<()> = Mutex::new(());
static ONCE: Once = Once::new();

fn context() -> MutexGuard<'static, ()> {
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
    // SAFETY: plain GL calls.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glFinish();
    }
    SEEN.lock().unwrap().clear();
    // SAFETY: plain GL call.
    while unsafe { glGetError() } != GL_NO_ERROR {}
    g
}

fn err() -> GLenum {
    // SAFETY: plain GL call.
    unsafe { glGetError() }
}

fn geti(pname: GLenum) -> GLint {
    let mut v = -1;
    // SAFETY: one value.
    unsafe { glGetIntegerv(pname, &mut v) };
    v
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

/// The error of `glRenderbufferStorageMultisample(.., samples, format, 8, 8)`
/// on a fresh renderbuffer, and the renderbuffer.
fn storage(samples: GLsizei, format: GLenum) -> (GLenum, GLuint) {
    let mut rb = 0;
    // SAFETY: plain GL calls.
    unsafe {
        glGenRenderbuffers(1, &mut rb);
        glBindRenderbuffer(GL_RENDERBUFFER, rb);
        while glGetError() != GL_NO_ERROR {}
        glRenderbufferStorageMultisample(GL_RENDERBUFFER, samples, format, 8, 8);
    }
    (err(), rb)
}

/// A bound 8x8 framebuffer with a renderbuffer of `samples` on each of
/// `formats`' colour attachments, all draw buffers. A storage error comes back
/// as `Err(error)`.
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
    let bufs: Vec<GLenum> = (0..formats.len()).map(|i| GL_COLOR_ATTACHMENT0 + i as GLenum).collect();
    // SAFETY: plain GL calls.
    unsafe {
        glDrawBuffers(bufs.len() as GLsizei, bufs.as_ptr());
        assert_eq!(err(), GL_NO_ERROR);
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE, "{formats:#x?}");
    }
    Ok(fbo)
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

/// The current program: `n` float outputs.
fn program(n: usize) {
    let mut fs = String::from("#version 300 es\nprecision highp float;\n");
    for i in 0..n {
        fs += &format!("layout(location = {i}) out vec4 o{i};\n");
    }
    fs += "void main() {\n";
    for i in 0..n {
        fs += &format!("o{i} = vec4(0.25, 0.5, 0.75, 1.0);\n");
    }
    fs += "}\n";
    let v = shader(GL_VERTEX_SHADER, "#version 300 es\nin vec4 p;\nvoid main() { gl_Position = p; }\n");
    let f = shader(GL_FRAGMENT_SHADER, &fs);
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

/// Draw one triangle and hand back what the back end was given for it.
fn draw() -> Seen {
    static TRI: [f32; 12] = [-1.0, -1.0, 0.0, 1.0, 3.0, -1.0, 0.0, 1.0, -1.0, 3.0, 0.0, 1.0];
    SEEN.lock().unwrap().clear();
    // SAFETY: TRI is static and outlives the draw.
    unsafe {
        glBindBuffer(GL_ARRAY_BUFFER, 0);
        glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, TRI.as_ptr() as *const c_void);
        glEnableVertexAttribArray(0);
        glDrawArrays(GL_TRIANGLES, 0, 3);
        glFinish();
    }
    assert_eq!(err(), GL_NO_ERROR);
    let s = core::mem::take(&mut *SEEN.lock().unwrap());
    s.into_iter().rev().find(|s| s.draws).expect("the draw reached the back end")
}

/// Blit of `read`'s colour attachment `att` into a fresh single-sample RGBA8
/// framebuffer of the same size; the GL error.
fn resolve_blit(read: &Fbo, att: GLenum) -> GLenum {
    let dst = framebuffer(0, &[GL_RGBA8]).expect("a single-sample target");
    // SAFETY: plain GL calls on names this test made.
    unsafe {
        glBindFramebuffer(GL_READ_FRAMEBUFFER, read.fb);
        glBindFramebuffer(GL_DRAW_FRAMEBUFFER, dst.fb);
        glReadBuffer(att);
        while glGetError() != GL_NO_ERROR {}
        glBlitFramebuffer(0, 0, 8, 8, 0, 0, 8, 8, GL_COLOR_BUFFER_BIT, GL_NEAREST);
    }
    let e = err();
    drop(dst);
    e
}

const FOUR: [GLenum; 4] = [GL_RGBA8; 4];
const ATT: [GLenum; 4] = [GL_COLOR_ATTACHMENT0, GL_COLOR_ATTACHMENT1, GL_COLOR_ATTACHMENT2, GL_COLOR_ATTACHMENT3];

/* ---- the expected job words (note (3), after Mesa) ----------------------- */

/// What the 4x job of `n` RGBA8 targets should carry: one tile of Mesa's
/// `v3d_choose_tile_size` for V3D 4.x (index = RT class 0/1/2 for 1/2/3-4 RTs
/// + 2 for 4x + the largest internal bpp 0/1/2), and per RT a load, a kept
///   store and a resolve store.
fn expected_tile(rts: usize, max_bpp: usize) -> (u32, u32) {
    const TILES: [(u32, u32); 7] = [(64, 64), (64, 32), (32, 32), (32, 16), (16, 16), (16, 8), (8, 8)];
    let class = [0usize, 0, 1, 2, 2][rts.min(4)];
    TILES[class + 2 + max_bpp]
}

/// Four RTs take the index of three: no tile-size step for the fourth RT.
#[test]
fn the_expected_four_rt_4x_tile_is_the_three_rt_tile() {
    assert_eq!(expected_tile(4, 0), expected_tile(3, 0));
    assert_eq!(expected_tile(4, 0), (16, 16));
    assert_eq!(expected_tile(4, 1), (16, 8));
    assert_eq!(expected_tile(4, 2), (8, 8));
    assert_eq!(expected_tile(1, 0), (32, 32), "the window's shape");
}

/* ---- the queries ---------------------------------------------------------- */

/// The stand-in raises the target limits, not the sample limit.
#[test]
fn limits_under_the_stand_in() {
    let _g = context();
    assert_eq!(geti(GL_MAX_DRAW_BUFFERS), 4);
    assert_eq!(geti(GL_MAX_COLOR_ATTACHMENTS), 4);
    assert_eq!(geti(GL_MAX_SAMPLES), 1);
}

#[test]
#[ignore = "owner src/gl/msaa.rs:29 (backend_max_samples is 0 on every back end), docs/notes/v3d-msaa-es3.md (4): GL_MAX_SAMPLES stays 1, V3D answers 4"]
fn max_samples_is_four() {
    let _g = context();
    assert_eq!(geti(GL_MAX_SAMPLES), 4);
}

/* ---- four attachments: completeness and the storage errors ---------------- */

/// Four RGBA8 renderbuffers of 0 and 1 sample on attachments 0-3 are complete.
#[test]
fn four_attachments_are_complete_at_zero_and_one_sample() {
    let _g = context();
    for samples in [0, 1] {
        let fbo = framebuffer(samples, &FOUR).unwrap();
        // SAFETY: plain GL call.
        assert_eq!(unsafe { glCheckFramebufferStatus(GL_FRAMEBUFFER) }, GL_FRAMEBUFFER_COMPLETE);
        drop(fbo);
    }
}

/// 4 samples on any of the four renderbuffers: GL_INVALID_OPERATION (past
/// GL_MAX_SAMPLES 1); the first attachment already fails.
#[test]
fn four_samples_of_four_attachments_is_invalid_operation() {
    let _g = context();
    assert_eq!(storage(4, GL_RGBA8).0, GL_INVALID_OPERATION);
    assert_eq!(framebuffer(4, &FOUR).err(), Some(GL_INVALID_OPERATION));
}

/// A 1-sample framebuffer reports GL_SAMPLES 1, not 4, with four attachments.
#[test]
fn one_sample_four_attachments_report_one_sample() {
    let _g = context();
    let _fbo = framebuffer(1, &FOUR).unwrap();
    assert_eq!((geti(GL_SAMPLE_BUFFERS), geti(GL_SAMPLES)), (1, 1));
}

/* ---- clears, draw, blit on the complete four-target framebuffer ------------ */

/// glClear with four draw buffers: every clear the back end sees names the
/// four targets (or all, mask 0) and is single-sampled.
#[test]
fn glclear_reaches_the_back_end_with_four_targets() {
    let _g = context();
    let _fbo = framebuffer(1, &FOUR).unwrap();
    // SAFETY: plain GL calls.
    unsafe {
        glClearColor(0.1, 0.2, 0.3, 0.4);
        glClear(GL_COLOR_BUFFER_BIT);
        glFinish();
    }
    assert_eq!(err(), GL_NO_ERROR);
    let seen = SEEN.lock().unwrap().clone();
    let c = seen.iter().find(|s| s.do_clear).expect("the clear reached the back end");
    assert_eq!((c.targets.len(), c.samples, c.refused), (4, 1, None));
    assert!(c.clear_mask == 0 || c.clear_mask == 0xf, "all four targets, got {:#x}", c.clear_mask);
}

/// glClearBufferfv of each draw buffer 0..3 clears that target alone.
#[test]
fn glclearbufferfv_of_each_attachment_names_one_target() {
    let _g = context();
    let _fbo = framebuffer(1, &FOUR).unwrap();
    for i in 0..4 {
        SEEN.lock().unwrap().clear();
        let v = [0.5f32, 0.25, 0.75, 1.0];
        // SAFETY: four floats; plain GL calls.
        unsafe {
            glClearBufferfv(GL_COLOR, i, v.as_ptr());
            glFinish();
        }
        assert_eq!(err(), GL_NO_ERROR, "buffer {i}");
        let seen = SEEN.lock().unwrap().clone();
        let c = seen.iter().find(|s| s.do_clear).unwrap_or_else(|| panic!("buffer {i}: no clear reached the back end"));
        assert_eq!((c.clear_mask, c.samples, c.refused), (1 << i, 1, None), "buffer {i}");
    }
    /* draw buffer 4 does not exist */
    // SAFETY: four floats.
    unsafe {
        let v = [0.0f32; 4];
        glClearBufferfv(GL_COLOR, 4, v.as_ptr());
    }
    assert_eq!(err(), GL_INVALID_VALUE);
}

/// A four-output draw into 1-sample storage: the back end gets four targets
/// at one sample, the V3D takes it, SAMPLE_STATE's mask is that of a
/// single-sampled job (0xf).
#[test]
fn a_four_output_draw_reaches_the_back_end() {
    let _g = context();
    let _fbo = framebuffer(1, &FOUR).unwrap();
    program(4);
    let s = draw();
    assert_eq!((s.samples, s.targets.len(), s.refused), (1, 4, None));
    assert_eq!(s.sample_mask, Some(0xf));
    let mut gpu: Vec<u64> = s.targets.iter().map(|t| t.0).collect();
    gpu.sort();
    gpu.dedup();
    assert_eq!(gpu.len(), 4, "four distinct render target addresses");
    assert!(s.targets.iter().all(|t| t.1 == s.targets[0].1), "four RGBA8 targets");
}

/// The blit of each attachment (glReadBuffer COLOR_ATTACHMENTi) of the
/// four-target framebuffer into a single-sample one is legal for 0 and 1
/// sample storage.
#[test]
fn the_blit_of_each_attachment_is_legal() {
    let _g = context();
    for samples in [0, 1] {
        let fbo = framebuffer(samples, &FOUR).unwrap();
        for a in ATT {
            assert_eq!(resolve_blit(&fbo, a), GL_NO_ERROR, "samples {samples} attachment {a:#x}");
        }
    }
}

/// A blit *into* the four-target framebuffer from a single-sample one is
/// legal (the spec refuses only a multisample draw framebuffer).
#[test]
fn a_blit_into_the_four_target_framebuffer_is_legal() {
    let _g = context();
    let four = framebuffer(0, &FOUR).unwrap();
    let single = framebuffer(0, &[GL_RGBA8]).unwrap();
    // SAFETY: plain GL calls on names this test made.
    unsafe {
        glBindFramebuffer(GL_READ_FRAMEBUFFER, single.fb);
        glBindFramebuffer(GL_DRAW_FRAMEBUFFER, four.fb);
        while glGetError() != GL_NO_ERROR {}
        glBlitFramebuffer(0, 0, 8, 8, 0, 0, 8, 8, GL_COLOR_BUFFER_BIT, GL_NEAREST);
    }
    assert_eq!(err(), GL_NO_ERROR);
}

/* ---- the V3D's side, below the GL core ------------------------------------ */

/// A hand-made 4x draw with four RGBA8 targets: the state layer takes it
/// (`refusal` None, `job_state` Ok); `draw_call` refuses it later by name.
#[test]
fn the_state_layer_takes_a_4x_draw_of_four_targets() {
    let mut d = GlesDraw::ZERO;
    d.target_gpu = 0x0100_0000;
    (d.target_w, d.target_h, d.target_format, d.target_alpha_bits) = (8, 8, GLES_SURF_RGBA8888, 8);
    (d.samples, d.prim, d.index_count, d.line_width) = (4, GLES_PRIM_TRIANGLES, 3, 1.0);
    (d.viewport, d.depth_range, d.color_mask, d.color_write) = ([0, 0, 8, 8], [0.0, 1.0], 0xf, true);
    d.num_targets = 4;
    for (i, target) in d.targets.iter_mut().enumerate().skip(1).take(3) {
        *target = GlesDrawTarget {
            gpu: 0x0100_0000 + 0x1000 * i as u64,
            w: 8,
            h: 8,
            format: GLES_SURF_RGBA8888,
            out_shift: 0,
        };
    }
    assert_eq!(state::refusal(&d), None);
    assert_eq!(state::unsupported(&d), None);
    let j = state::job_state(&d).expect("the job state of a 4x four-target draw");
    assert_ne!(j.sample_mask, 0, "SAMPLE_STATE's mask of a 4x job");
}

/* ---- the expected result: ignored until the design lands ------------------- */

/// Four 4x RGBA8 renderbuffers: storage, completeness, the draw as the back
/// end gets it, the blit of each attachment.
fn expect_4x_four_targets() {
    let _g = context();
    let fbo = framebuffer(4, &FOUR).expect("4 samples are legal storage");
    assert_eq!(geti(GL_SAMPLES), 4);
    program(4);
    let s = draw();
    assert_eq!((s.samples, s.targets.len(), s.refused), (4, 4, None));
    assert_eq!(expected_tile(4, 0), (16, 16));
    for a in ATT {
        assert_eq!(resolve_blit(&fbo, a), GL_NO_ERROR, "{a:#x}");
    }
}

#[test]
#[ignore = "owner src/gl/framebuffer.rs:3505 (msaa::stored_samples refuses 4 samples while backend_max_samples is 0): a 4x renderbuffer is GL_INVALID_OPERATION, no FBO is 4x"]
fn four_rgba8_4x_storage_and_complete() {
    let _g = context();
    let fbo = framebuffer(4, &FOUR);
    assert!(fbo.is_ok(), "{:#x?}", fbo.err());
}

#[test]
#[ignore = "owner src/gl/framebuffer.rs:3505 (4x storage refused) then src/v3d/backend.rs:6319 ('4x multisampling of several colour attachments'): four 4x targets neither store nor draw"]
fn four_rgba8_4x_draw_and_resolve_each_attachment() {
    expect_4x_four_targets();
}

#[test]
#[ignore = "owner src/v3d/backend.rs:6319: draw_call refuses a 4x draw with more than one colour target; the job builder (driver/src/v3d/v3d_job.c) keeps one RT's samples"]
fn four_target_4x_draw_is_not_refused_by_draw_call() {
    /* the refusal is a log line in draw_call, not in the state layer, so a
     * host test sees it only through a back end driven by the real draw_call
     * (src/v3d/backend_tests.rs:3958 pins it with the log); the GL path is
     * closed one step earlier at storage, so this stays red until both open */
    expect_4x_four_targets();
}
