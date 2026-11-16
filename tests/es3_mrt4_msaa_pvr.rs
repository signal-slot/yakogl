// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! PowerVR (BXS-4-64, AM62P): a 4x multisample framebuffer with four RGBA8
//! colour renderbuffers on COLOR_ATTACHMENT0-3, through the GL API, on the
//! host (plan #368, docs/notes/mrt4-v3d-pvr.md section 2,
//! docs/notes/pvr-int-mixed-targets.md).
//!
//! Two things meet here that are each only half built today: four draw targets
//! (`backend::HOST_DRAW_TARGETS` stands in for the knob; the GL core handles
//! any count) and 4x renderbuffers (docs/notes/es3-msaa-renderbuffers.md: the
//! GL core refuses a 2 or 4 sample renderbuffer, one sample is plain storage,
//! and `GlesDraw::samples` is 1 for an FBO draw). The GL core is driven
//! through the API into a stand-in back end that records what it is handed
//! (targets, samples) and fills no pixels, so the pixel expectations of the
//! resolve are `#[ignore]`d; the PowerVR's own draw needs the board's job
//! memory. What the host can see of the PowerVR's per-target state is the
//! pure rule `pvr::mrt_layout` (the output registers a target takes, the
//! end-of-tile program that emits them) and the render layer's constants
//! (`osal::render`); the PBE words themselves are built inside
//! `src/pvr/backend.rs`, out of this file's reach.
//!
//! Passing tests pin today. The `#[ignore]`d ones are the expected result
//! once the work lands, each naming the owner and the file and line.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test es3_mrt4_msaa_pvr
//! ```

#![cfg(all(feature = "host-test", feature = "shaderc-online", feature = "gpu-powervr", not(feature = "report-es2")))]

use core::ffi::{c_char, c_void};
use std::ffi::CString;
use std::sync::atomic::Ordering;
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{self, GlesDraw, GlesHeap, YakoglBackend, *};
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

/// What the back end was handed by one draw.
#[derive(Clone, Debug)]
struct Seen {
    /// An application draw (vertices), not a clear alone.
    draws: bool,
    /// `targets[..num_targets]`: (gpu, format).
    targets: Vec<(u64, u8)>,
    /// `GlesDraw::samples`.
    samples: u8,
}

static SEEN: Mutex<Vec<Seen>> = Mutex::new(Vec::new());
/// The ids `attachment_gone` was told, in order.
static GONE: Mutex<Vec<u32>> = Mutex::new(Vec::new());

unsafe extern "C" fn b_draw(d: *const GlesDraw) -> i32 {
    let d = &*d;
    let n = (d.num_targets.max(1) as usize).min(GLES_MAX_TARGETS);
    SEEN.lock().unwrap().push(Seen {
        draws: d.index_count != 0,
        targets: d.targets[..n].iter().map(|t| (t.gpu, t.format)).collect(),
        samples: d.samples,
    });
    0
}

unsafe extern "C" fn b_gone(id: u32) {
    GONE.lock().unwrap().push(id);
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
    name: c"es3-mrt4-msaa-pvr".as_ptr() as *const c_char,
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
    attachment_gone: Some(b_gone),
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
        glDisable(GL_DEPTH_TEST);
        glFinish();
    }
    SEEN.lock().unwrap().clear();
    GONE.lock().unwrap().clear();
    // SAFETY: plain GL call.
    while unsafe { glGetError() } != GL_NO_ERROR {}
    g
}

fn err() -> GLenum {
    // SAFETY: plain GL call.
    unsafe { glGetError() }
}

/* ---- framebuffers, the program, the draw ------------------------------- */

const ATTACHMENTS: [GLenum; 4] =
    [GL_COLOR_ATTACHMENT0, GL_COLOR_ATTACHMENT1, GL_COLOR_ATTACHMENT2, GL_COLOR_ATTACHMENT3];

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

/// The error of `glRenderbufferStorageMultisample(.., samples, GL_RGBA8, 8, 8)`
/// on a fresh renderbuffer, and the renderbuffer.
fn storage(samples: GLsizei) -> (GLenum, GLuint) {
    let mut rb = 0;
    // SAFETY: plain GL calls.
    unsafe {
        glGenRenderbuffers(1, &mut rb);
        glBindRenderbuffer(GL_RENDERBUFFER, rb);
        while glGetError() != GL_NO_ERROR {}
        glRenderbufferStorageMultisample(GL_RENDERBUFFER, samples, GL_RGBA8, 8, 8);
    }
    (err(), rb)
}

/// A bound 8x8 framebuffer with four RGBA8 colour renderbuffers of `samples`
/// on COLOR_ATTACHMENT0-3, all four drawn. A storage error comes back as
/// `Err(error)`.
fn framebuffer4(samples: GLsizei) -> Result<Fbo, GLenum> {
    let mut fb = 0;
    // SAFETY: room for one name; plain GL call.
    unsafe {
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
    }
    let mut fbo = Fbo { fb, rbs: Vec::new() };
    for a in ATTACHMENTS {
        let (e, rb) = storage(samples);
        fbo.rbs.push(rb);
        if e != GL_NO_ERROR {
            return Err(e);
        }
        // SAFETY: a renderbuffer this test made.
        unsafe { glFramebufferRenderbuffer(GL_FRAMEBUFFER, a, GL_RENDERBUFFER, rb) };
    }
    // SAFETY: plain GL calls; four enums.
    unsafe {
        glDrawBuffers(4, ATTACHMENTS.as_ptr());
        assert_eq!(err(), GL_NO_ERROR, "four draw buffers under the stand-in");
    }
    Ok(fbo)
}

fn status() -> GLenum {
    // SAFETY: plain GL call.
    unsafe { glCheckFramebufferStatus(GL_FRAMEBUFFER) }
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

/// The colour output `i` of the four-output program.
fn colour(i: usize) -> [f32; 4] {
    [[1.0, 0.0, 0.0, 1.0], [0.0, 1.0, 0.0, 1.0], [0.0, 0.0, 1.0, 1.0], [1.0, 1.0, 0.0, 0.5]][i]
}

/// The current program: four float outputs, `o_i` = `colour(i)`.
fn program4() {
    let mut fs = String::from("#version 300 es\nprecision highp float;\n");
    for i in 0..4 {
        fs += &format!("layout(location = {i}) out vec4 o{i};\n");
    }
    fs += "void main() {\n";
    for i in 0..4 {
        let [r, g, b, a] = colour(i);
        fs += &format!("  o{i} = vec4({r:?}, {g:?}, {b:?}, {a:?});\n");
    }
    fs += "}\n";
    let vs = "#version 300 es\nin vec4 p;\nvoid main() { gl_Position = p; }\n";
    let v = shader(GL_VERTEX_SHADER, vs);
    let f = shader(GL_FRAGMENT_SHADER, &fs);
    // SAFETY: plain GL calls.
    unsafe {
        let p = glCreateProgram();
        glAttachShader(p, v);
        glAttachShader(p, f);
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        assert_eq!(ok, 1, "four outputs link under the stand-in");
        glUseProgram(p);
    }
}

/// The 8x8 target's whole area: one oversized triangle.
static FULL: [f32; 12] = [-1.0, -1.0, 0.0, 1.0, 3.0, -1.0, 0.0, 1.0, -1.0, 3.0, 0.0, 1.0];
/// Left of x = -0.125: three whole columns of 8 and the left half of column
/// 3, so a 4x pixel there has two of its four samples covered.
static EDGE: [f32; 24] = [
    -1.0, -1.0, 0.0, 1.0, -0.125, -1.0, 0.0, 1.0, -0.125, 1.0, 0.0, 1.0, -1.0, -1.0, 0.0, 1.0, -0.125, 1.0, 0.0, 1.0,
    -1.0, 1.0, 0.0, 1.0,
];

/// Draw `verts` (`n` vertices of 4 floats) and hand back what the back end was
/// given for it.
fn draw(verts: &'static [f32], n: GLsizei) -> Seen {
    SEEN.lock().unwrap().clear();
    // SAFETY: `verts` is static and outlives the draw.
    unsafe {
        glBindBuffer(GL_ARRAY_BUFFER, 0);
        glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, verts.as_ptr() as *const c_void);
        glEnableVertexAttribArray(0);
        glDrawArrays(GL_TRIANGLES, 0, n);
        glFinish();
    }
    assert_eq!(err(), GL_NO_ERROR);
    let s = core::mem::take(&mut *SEEN.lock().unwrap());
    s.into_iter().rev().find(|s| s.draws).expect("the draw reached the back end")
}

/// glClear of colour on the bound framebuffer; what the back end saw of it.
fn clear_all(c: [f32; 4]) -> Vec<Seen> {
    SEEN.lock().unwrap().clear();
    // SAFETY: plain GL calls.
    unsafe {
        glClearColor(c[0], c[1], c[2], c[3]);
        glClear(GL_COLOR_BUFFER_BIT);
        glFinish();
    }
    assert_eq!(err(), GL_NO_ERROR);
    core::mem::take(&mut *SEEN.lock().unwrap())
}

/// Resolve blit of attachment `a` of `read` into a fresh single-sample
/// four-attachment framebuffer (left bound for reading); the GL error and the
/// framebuffer.
fn resolve_blit(read: &Fbo, a: GLenum) -> (GLenum, Fbo) {
    let dst = framebuffer4(0).expect("a single-sample target");
    // SAFETY: plain GL calls on names this test made.
    unsafe {
        glBindFramebuffer(GL_READ_FRAMEBUFFER, read.fb);
        glBindFramebuffer(GL_DRAW_FRAMEBUFFER, dst.fb);
        glReadBuffer(a);
        while glGetError() != GL_NO_ERROR {}
        glBlitFramebuffer(0, 0, 8, 8, 0, 0, 8, 8, GL_COLOR_BUFFER_BIT, GL_NEAREST);
    }
    (err(), dst)
}

/// The 8x8 RGBA8 pixels of attachment `a` of `fbo`, row-major from the
/// bottom left.
fn pixels(fbo: &Fbo, a: GLenum) -> Vec<[u8; 4]> {
    let mut v = vec![[0u8; 4]; 64];
    // SAFETY: room for 64 RGBA8 pixels; a framebuffer this test made.
    unsafe {
        glBindFramebuffer(GL_READ_FRAMEBUFFER, fbo.fb);
        glReadBuffer(a);
        glReadPixels(0, 0, 8, 8, GL_RGBA, GL_UNSIGNED_BYTE, v.as_mut_ptr() as *mut c_void);
    }
    assert_eq!(err(), GL_NO_ERROR);
    v
}

fn near(a: [u8; 4], b: [u8; 4], tol: i32) -> bool {
    a.iter().zip(b).all(|(x, y)| (*x as i32 - y as i32).abs() <= tol)
}

fn to8(c: [f32; 4]) -> [u8; 4] {
    c.map(|v| (v * 255.0 + 0.5) as u8)
}

fn geti(pname: GLenum) -> GLint {
    let mut v = -1;
    // SAFETY: one value.
    unsafe { glGetIntegerv(pname, &mut v) };
    v
}

fn rb_samples(rb: GLuint) -> GLint {
    let mut v = -1;
    // SAFETY: one value; a renderbuffer this test made.
    unsafe {
        glBindRenderbuffer(GL_RENDERBUFFER, rb);
        glGetRenderbufferParameteriv(GL_RENDERBUFFER, GL_RENDERBUFFER_SAMPLES, &mut v);
    }
    v
}

/* ---- pins: what the GL core and the PowerVR pieces do today ------------- */

/// The stand-in answers four draw targets; the multisample cap stays 1.
#[test]
fn four_targets_and_one_sample_today() {
    let _g = context();
    assert_eq!(geti(GL_MAX_DRAW_BUFFERS), 4);
    assert_eq!(geti(GL_MAX_COLOR_ATTACHMENTS), 4);
    assert_eq!(geti(GL_MAX_SAMPLES), 1, "src/gl/msaa.rs:29 backend_max_samples() is 0");
}

/// A 4 sample RGBA8 renderbuffer is refused at storage, so no 4x framebuffer
/// with four attachments exists: the first attachment fails already.
#[test]
fn four_samples_on_four_attachments_is_invalid_operation_today() {
    let _g = context();
    for samples in [2, 4, 8] {
        assert_eq!(storage(samples).0, GL_INVALID_OPERATION, "{samples}");
    }
    assert_eq!(framebuffer4(4).err(), Some(GL_INVALID_OPERATION));
}

/// The one-sample class: four RGBA8 renderbuffers, complete, each of 1 sample
/// as renderbuffer, the framebuffer a 1-sample buffer.
#[test]
fn four_one_sample_attachments_are_complete() {
    let _g = context();
    let fbo = framebuffer4(1).unwrap();
    assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE);
    for rb in &fbo.rbs {
        assert_eq!(rb_samples(*rb), 1);
    }
    assert_eq!((geti(GL_SAMPLE_BUFFERS), geti(GL_SAMPLES)), (1, 1));
    let single = framebuffer4(0).unwrap();
    assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE);
    assert_eq!((geti(GL_SAMPLE_BUFFERS), geti(GL_SAMPLES)), (0, 0));
    drop(single);
}

/// Attachment 3 alone, with 0..2 empty, is complete too (a gap is legal).
#[test]
fn attachment_three_alone_is_complete() {
    let _g = context();
    let mut fb = 0;
    let (e, rb) = storage(1);
    assert_eq!(e, GL_NO_ERROR);
    // SAFETY: plain GL calls on names this test made.
    unsafe {
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT3, GL_RENDERBUFFER, rb);
    }
    assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE);
    let fbo = Fbo { fb, rbs: vec![rb] };
    drop(fbo);
}

/// A four-output draw into the four 1-sample attachments reaches the back end
/// with four distinct targets of one format and `samples` 1.
#[test]
fn a_four_output_draw_reaches_the_back_end_with_four_targets() {
    let _g = context();
    let _fbo = framebuffer4(1).unwrap();
    program4();
    let s = draw(&FULL, 3);
    assert_eq!(s.targets.len(), 4);
    let mut gpus: Vec<u64> = s.targets.iter().map(|t| t.0).collect();
    gpus.sort();
    gpus.dedup();
    assert_eq!(gpus.len(), 4, "four attachments, four addresses: {:?}", s.targets);
    assert!(s.targets.iter().all(|t| t.1 == s.targets[0].1), "one format: {:?}", s.targets);
    assert_eq!(s.samples, 1, "an FBO draw is 1 sample (es3-msaa-renderbuffers.md (0))");
}

/// The target count follows glDrawBuffers: two drawn, two targets.
#[test]
fn the_target_count_follows_the_draw_buffers() {
    let _g = context();
    let _fbo = framebuffer4(1).unwrap();
    program4();
    // SAFETY: two enums.
    unsafe { glDrawBuffers(2, ATTACHMENTS.as_ptr()) };
    assert_eq!(err(), GL_NO_ERROR);
    assert_eq!(draw(&FULL, 3).targets.len(), 2);
    // SAFETY: four enums.
    unsafe { glDrawBuffers(4, ATTACHMENTS.as_ptr()) };
    assert_eq!(draw(&FULL, 3).targets.len(), 4);
}

/// glClear with four draw buffers: no error, and every colour clear the back
/// end sees covers the four targets (or none: a clear may stay in the GL core).
#[test]
fn a_clear_with_four_draw_buffers_is_valid() {
    let _g = context();
    let _fbo = framebuffer4(1).unwrap();
    for s in clear_all([0.25, 0.5, 0.75, 1.0]) {
        assert_eq!(s.targets.len(), 4, "{s:?}");
    }
}

/// Each attachment as the read buffer of a blit into a single-sample
/// four-attachment framebuffer: no error at any of the four (the 1-sample
/// class has no resolve, it is a same-size copy). The pixels are the stand-in's
/// to fill, so none are checked here.
#[test]
fn a_blit_from_each_attachment_is_valid() {
    let _g = context();
    let src = framebuffer4(1).unwrap();
    for a in ATTACHMENTS {
        let (e, _dst) = resolve_blit(&src, a);
        assert_eq!(e, GL_NO_ERROR, "{a:#x}");
    }
}

/// A blit *into* a multisample framebuffer is refused by the spec at any
/// count (ES 3.0 4.3.3).
#[test]
fn a_blit_into_a_multisample_framebuffer_is_invalid_operation() {
    let _g = context();
    let ms = framebuffer4(1).unwrap();
    let single = framebuffer4(0).unwrap();
    // SAFETY: plain GL calls on names this test made.
    unsafe {
        glBindFramebuffer(GL_READ_FRAMEBUFFER, single.fb);
        glBindFramebuffer(GL_DRAW_FRAMEBUFFER, ms.fb);
        glReadBuffer(GL_COLOR_ATTACHMENT3);
        while glGetError() != GL_NO_ERROR {}
        glBlitFramebuffer(0, 0, 8, 8, 0, 0, 8, 8, GL_COLOR_BUFFER_BIT, GL_NEAREST);
    }
    assert_eq!(err(), GL_INVALID_OPERATION);
}

/* ---- what the host sees of the PowerVR's per-target state ----------------- */

/// Four RGBA8 targets are four 32-bit registers a pixel: r0..r3 one each,
/// the end-of-tile program `eot4` with all four emits from registers and no
/// spill. The rule has no sample count: the four registers are per sample
/// (the USC runs per sample), so 4x changes nothing here.
#[test]
fn four_rgba8_targets_take_four_output_registers_and_eot4() {
    use yakogl::pvr::mrt_layout::{layout, EotProgram, Place, OUTPUT_REGS};
    let l = layout(&[1, 1, 1, 1]).unwrap();
    assert_eq!(OUTPUT_REGS, 4);
    for i in 0..4u8 {
        assert_eq!(l.place[i as usize], Place::Register { first: i });
        assert_eq!(l.place[i as usize].source_pos(), i, "PBE source position of target {i}");
    }
    assert_eq!((l.targets, l.output_regs, l.tile_buffers), (4, 4, 0));
    assert_eq!(l.program, EotProgram::Plain { targets: 4 });
    assert_eq!((l.program.name(), l.program.emits(), l.program.spilled_mask()), ("eot4", 4, 0));
}

/// A fourth target that does not fit the registers spills to tile buffer 0
/// and the `eot4_tb` program emits it; the captured tile-buffer programs are
/// one-sample ones (mrt_layout.rs module docs), so with 4x this spill is not
/// covered by a capture today. RGBA8 x4 never reaches it.
#[test]
fn a_wide_target_spills_the_fourth_to_eot4_tb() {
    use yakogl::pvr::mrt_layout::{layout, EotProgram, Place};
    let l = layout(&[1, 1, 1, 2]).unwrap();
    assert_eq!(l.place[3], Place::TileBuffer { index: 0 });
    assert_eq!(l.program, EotProgram::Eot4Tb { spilled: 1 });
    assert_eq!(l.program.name(), "eot4_tb");
    assert_eq!(l.program.spilled_mask(), 0b1000);
}

/// The render layer's tables already have a fourth slot: the end-of-tile
/// programs are counted 1..=4.
#[test]
fn the_render_layer_tables_reach_four() {
    use yakogl::osal::render::{PvrRenderStatic, PVR_RENDER_EOT_MAX_RT, PVR_RENDER_MAX_RT};
    assert_eq!(PVR_RENDER_EOT_MAX_RT, 4);
    const { assert!(PVR_RENDER_MAX_RT <= PVR_RENDER_EOT_MAX_RT) };
    // SAFETY: a plain table of integers; all zero is a valid value.
    let st: PvrRenderStatic = unsafe { core::mem::zeroed() };
    assert_eq!(st.eot_doutu_src0_n.len(), 5, "index = emit count, 0 unused");
    assert_eq!(st.event_pixel_pds_info_n.len(), 5);
}

/* ---- the expected result: ignored until the work lands -------------------- */

/// 4 once the GL core reads the back end's capability.
#[test]
#[ignore = "GL core gl-msaa-cap, src/gl/msaa.rs:29: backend_max_samples() is the constant 0, so GL_MAX_SAMPLES is 1 until the PowerVR sets max_samples 4 (pvr-msaa-fbo)"]
fn max_samples_is_four() {
    let _g = context();
    assert_eq!(geti(GL_MAX_SAMPLES), 4);
}

/// The back end's per-kick render-target count is the render layer's
/// `PVR_RENDER_MAX_RT`: three, so a four-target draw loses the fourth.
#[test]
#[ignore = "PowerVR pvr-render, driver/src/pvr_render.h:57 and src/osal/render.rs:20: PVR_RENDER_MAX_RT is 3, and src/pvr/backend.rs:5235 clamps num_targets to it (PVR_BACKEND_MAX_RT, :274), so the fourth PBE state is not built"]
fn the_render_layer_has_four_render_targets() {
    assert_eq!(yakogl::osal::render::PVR_RENDER_MAX_RT, 4);
}

/// Storage, completeness and the draw of a 4x framebuffer with four RGBA8
/// attachments: four targets, `samples` 4.
#[test]
#[ignore = "GL core gl-msaa-cap, src/gl/msaa.rs:53: glRenderbufferStorageMultisample(4) is GL_INVALID_OPERATION and an FBO draw has samples 1; PowerVR pvr-msaa-fbo (docs/notes/es3-msaa-renderbuffers.md (c1)) must then draw four targets at 4x (src/pvr/backend.rs:5721 frame_samples)"]
fn a_four_output_4x_draw_has_four_targets_and_samples_four() {
    let _g = context();
    let fbo = framebuffer4(4).expect("4 samples are legal storage");
    assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE);
    for rb in &fbo.rbs {
        assert_eq!(rb_samples(*rb), 4);
    }
    assert_eq!(geti(GL_SAMPLES), 4);
    program4();
    let s = draw(&FULL, 3);
    assert_eq!((s.samples, s.targets.len()), (4, 4));
}

/// glClear of four 4x attachments reaches the back end as a four-target clear
/// at 4 samples.
#[test]
#[ignore = "GL core gl-msaa-cap, src/gl/msaa.rs:53 (no 4x storage) and PowerVR pvr-msaa-fbo: no 4x four-target frame to clear"]
fn a_clear_of_four_4x_attachments_has_four_targets_and_samples_four() {
    let _g = context();
    let _fbo = framebuffer4(4).expect("4 samples are legal storage");
    let seen = clear_all([0.25, 0.5, 0.75, 1.0]);
    assert!(!seen.is_empty(), "the clear reaches the back end");
    for s in seen {
        assert_eq!((s.samples, s.targets.len()), (4, 4), "{s:?}");
    }
}

/// Each of the four attachments, drawn with its own output over a half
/// covered edge, resolves by `glBlitFramebuffer`: the interior exact, the
/// edge pixel half of every channel (two of four samples), the rest clear.
#[test]
#[ignore = "GL core gl-msaa-resolve (the blit, src/gl/msaa.rs:53 no 4x storage) and PowerVR pvr-msaa-fbo (PBE downscale store per render target, src/pvr/backend.rs:5235 clamps at three): no 4x four-target frame; the stand-in fills no pixels, run it against the PowerVR's own host path"]
fn each_attachment_resolves_by_blit() {
    let _g = context();
    let fbo = framebuffer4(4).expect("4 samples are legal storage");
    clear_all([0.0; 4]);
    program4();
    assert_eq!(draw(&EDGE, 6).samples, 4);
    for (i, a) in ATTACHMENTS.into_iter().enumerate() {
        let (e, dst) = resolve_blit(&fbo, a);
        assert_eq!(e, GL_NO_ERROR, "attachment {i}");
        let want = to8(colour(i));
        let half = want.map(|v| v / 2);
        // the blit lands in every draw buffer of dst; read its attachment 0
        let px = pixels(&dst, GL_COLOR_ATTACHMENT0);
        for y in 0..8 {
            for x in 0..8 {
                let p = px[y * 8 + x];
                match x {
                    0..=2 => assert_eq!(p, want, "attachment {i} interior ({x},{y})"),
                    3 => assert!(near(p, half, 2), "attachment {i} edge ({x},{y}) {p:?}"),
                    _ => assert_eq!(p, [0, 0, 0, 0], "attachment {i} outside ({x},{y})"),
                }
            }
        }
    }
}
