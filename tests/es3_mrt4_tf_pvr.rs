// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Transform feedback beside a four-target draw, with the PowerVR build and
//! a four-target stand-in (plan #368 item `tf4-pvr`; RC14,
//! docs/notes/mrt4-v3d-pvr.md, docs/notes/pvr-mrt4-budget.md).
//!
//! `backend::HOST_DRAW_TARGETS` = 4 stands in for the knob (as
//! `es3_mrt4_instanced_pvr`; `pvr_debug_set_draw_targets` needs `test-knobs`
//! and is the later spelling, left to the knob-switch item).
//! THIS RIG CANNOT SEE PIXELS: the host back end is a recorder. The GL core
//! captures transform feedback on the CPU (`src/gl/tf_capture.rs`) and then
//! hands the draw on as it would without capture, so what is pinned is the
//! capture and what the back end is handed, for an ESSL 3.00 program with a
//! captured varying and four RGBA8 outputs (locations 0..3):
//!
//! 1. the feedback buffer holds the captured varying of every vertex and
//!    `GL_TRANSFORM_FEEDBACK_PRIMITIVES_WRITTEN` counts the primitives, while
//!    the same draw reaches the back end once with four distinct targets and
//!    `mrt_layout` puts the four RGBA8 outputs in registers 0..3 with no
//!    tile-buffer spill (the render layer's four-target EOT/PBE state is
//!    pinned by `es3_mrt4_pvr`);
//! 2. `GL_RASTERIZER_DISCARD` on: the buffer and the count are the same, and
//!    nothing reaches the back end (discard before the back end);
//! 3. `glDrawBuffers({ATT0, NONE, NONE, ATT3})`: capture as before, the draw
//!    reaches the back end with four targets, `out_shift` 0 and slots 1
//!    and 2 present as holes with gpu 0 (`es3_mrt4_gaps_pvr`), and the
//!    capture is unchanged; again under discard.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test es3_mrt4_tf_pvr
//! ```

#![cfg(all(feature = "host-test", feature = "shaderc-online", feature = "gpu-powervr", not(feature = "report-es2")))]

use core::ffi::{c_char, c_void};
use std::ffi::CString;
use std::sync::atomic::Ordering;
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{self, GlesDraw, GlesHeap, YakoglBackend, *};
use yakogl::gl::buffer::*;
use yakogl::gl::consts::*;
use yakogl::gl::draw::*;
use yakogl::gl::es3::*;
use yakogl::gl::framebuffer::*;
use yakogl::gl::program::*;
use yakogl::gl::shader::*;
use yakogl::gl::state::*;
use yakogl::gl::types::*;
use yakogl::gl::vertex::*;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};
use yakogl::pvr::mrt_layout::{layout, Place};

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
#[derive(Clone)]
struct Seen {
    draws: bool,
    instance_count: u32,
    #[allow(dead_code)]
    instance_id: u32,
    index_count: u32,
    /// `targets[..num_targets]`: (gpu, format).
    targets: Vec<(u64, u8)>,
    num_targets: u8,
    out_shift: u8,
    /// (per_instance, divisor) of the attributes.
    #[allow(dead_code)]
    attribs: Vec<(u8, u16)>,
}

static SEEN: Mutex<Vec<Seen>> = Mutex::new(Vec::new());

unsafe extern "C" fn b_draw(d: *const GlesDraw) -> i32 {
    let d = &*d;
    let n = (d.num_targets.max(1) as usize).min(GLES_MAX_TARGETS);
    SEEN.lock().unwrap().push(Seen {
        draws: d.index_count != 0,
        instance_count: d.instance_count,
        instance_id: d.instance_id,
        index_count: d.index_count,
        targets: d.targets[..n].iter().map(|t| (t.gpu, t.format)).collect(),
        num_targets: d.num_targets,
        out_shift: d.targets[0].out_shift,
        attribs: d.attribs[..d.num_attribs as usize].iter().map(|a| (a.per_instance, a.divisor)).collect(),
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
    name: c"es3-mrt4-tf-pvr".as_ptr() as *const c_char,
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

const ATT: [GLenum; 4] = [GL_COLOR_ATTACHMENT0, GL_COLOR_ATTACHMENT1, GL_COLOR_ATTACHMENT2, GL_COLOR_ATTACHMENT3];

/// A bound 8x8 framebuffer with an RGBA8 renderbuffer on each attachment of
/// `which` (indices 0..4); the framebuffer and renderbuffers.
fn framebuffer(which: &[usize]) -> (GLuint, Vec<GLuint>) {
    let mut fb = 0;
    let mut rbs = Vec::new();
    // SAFETY: room for the names; plain GL calls.
    unsafe {
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        for &i in which {
            let mut rb = 0;
            glGenRenderbuffers(1, &mut rb);
            glBindRenderbuffer(GL_RENDERBUFFER, rb);
            glRenderbufferStorage(GL_RENDERBUFFER, GL_RGBA8, 8, 8);
            glFramebufferRenderbuffer(GL_FRAMEBUFFER, ATT[i], GL_RENDERBUFFER, rb);
            rbs.push(rb);
        }
    }
    (fb, rbs)
}

fn release(fb: GLuint, rbs: &[GLuint]) {
    // SAFETY: names this test made.
    unsafe {
        glUseProgram(0);
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glDeleteFramebuffers(1, &fb);
        glDeleteRenderbuffers(rbs.len() as GLsizei, rbs.as_ptr());
    }
}

fn complete() -> bool {
    // SAFETY: plain GL call.
    unsafe { glCheckFramebufferStatus(GL_FRAMEBUFFER) == GL_FRAMEBUFFER_COMPLETE }
}

fn draw_buffers(bufs: &[GLenum]) {
    // SAFETY: `bufs.len()` readable enums.
    unsafe { glDrawBuffers(bufs.len() as GLsizei, bufs.as_ptr()) };
    assert_eq!(err(), GL_NO_ERROR, "glDrawBuffers {bufs:x?}");
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

/// The current program: gl_InstanceID shifts the triangle along x, the
/// per-instance attribute `k` (location 1) is the flat value; four RGBA8

const VS: &str = "#version 300 es\nlayout(location = 0) in vec4 p;\nout vec4 cap;\nout float k;\n\
    void main() { cap = p * 2.0; k = p.x + 10.0; gl_Position = p; }\n";
const FS: &str = "#version 300 es\nprecision highp float;\nin vec4 cap;\nin float k;\n\
    layout(location = 0) out vec4 o0;\nlayout(location = 1) out vec4 o1;\n\
    layout(location = 2) out vec4 o2;\nlayout(location = 3) out vec4 o3;\n\
    void main() { o0 = vec4(1.0, 0.0, 0.0, 1.0); o1 = vec4(0.0, 1.0, 0.0, 1.0);\n\
    o2 = vec4(0.0, 0.0, 1.0, 1.0); o3 = vec4(cap.x, k * 0.0, 0.0, 1.0); }\n";

/// The captured varyings `cap` (4 floats) and `k` (1 float), interleaved.
const CAPTURED: [&str; 2] = ["cap", "k"];

/// The current program: `cap` and `k` captured, four RGBA8 outputs.
fn program() {
    let v = shader(GL_VERTEX_SHADER, VS);
    let f = shader(GL_FRAGMENT_SHADER, FS);
    // SAFETY: plain GL calls; every pointer outlives its call.
    unsafe {
        let p = glCreateProgram();
        glAttachShader(p, v);
        glAttachShader(p, f);
        let names: Vec<CString> = CAPTURED.iter().map(|n| CString::new(*n).unwrap()).collect();
        let ptrs: Vec<*const c_char> = names.iter().map(|n| n.as_ptr()).collect();
        glTransformFeedbackVaryings(p, ptrs.len() as GLsizei, ptrs.as_ptr(), GL_INTERLEAVED_ATTRIBS);
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        assert_eq!(ok, 1, "the four-output capturing program links");
        glUseProgram(p);
    }
}

/// One triangle, five floats a vertex in the buffer: `cap` then `k`.
static TRI: [f32; 12] = [-1.0, -1.0, 0.0, 1.0, 0.5, -1.0, 0.0, 1.0, -1.0, 0.5, 0.0, 1.0];

fn want() -> Vec<f32> {
    TRI.chunks(4).flat_map(|a| [a[0] * 2.0, a[1] * 2.0, a[2] * 2.0, a[3] * 2.0, a[0] + 10.0]).collect()
}

/// A bound four-attachment framebuffer, the program, a 1 KiB capture buffer
/// on index 0 and the position array; (framebuffer, renderbuffers, buffer).
fn setup(bufs: &[GLenum]) -> (GLuint, Vec<GLuint>, GLuint) {
    let (fb, rbs) = framebuffer(&[0, 1, 2, 3]);
    assert!(complete(), "four RGBA8 attachments are complete");
    draw_buffers(bufs);
    program();
    let mut b = 0;
    // SAFETY: plain GL calls; the static outlives the draws.
    unsafe {
        glGenBuffers(1, &mut b);
        glBindBuffer(GL_ARRAY_BUFFER, b);
        glBufferData(GL_ARRAY_BUFFER, 1024, core::ptr::null(), GL_DYNAMIC_COPY);
        glBindBufferBase(GL_TRANSFORM_FEEDBACK_BUFFER, 0, b);
        glBindBuffer(GL_ARRAY_BUFFER, 0);
        glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, TRI.as_ptr() as *const c_void);
        glEnableVertexAttribArray(0);
    }
    assert_eq!(err(), GL_NO_ERROR, "setup");
    (fb, rbs, b)
}

/// One captured triangle draw; (buffer contents, primitives written).
fn capture_draw() -> (Vec<f32>, GLuint) {
    let (mut q, mut n) = (0, 99);
    // SAFETY: plain GL calls; a mapped read of 60 bytes of a 1 KiB buffer.
    unsafe {
        glGenQueries(1, &mut q);
        glBeginQuery(GL_TRANSFORM_FEEDBACK_PRIMITIVES_WRITTEN, q);
        glBeginTransformFeedback(GL_TRIANGLES);
        assert_eq!(err(), GL_NO_ERROR, "begin");
        glDrawArrays(GL_TRIANGLES, 0, 3);
        assert_eq!(err(), GL_NO_ERROR, "captured draw");
        glEndTransformFeedback();
        glEndQuery(GL_TRANSFORM_FEEDBACK_PRIMITIVES_WRITTEN);
        glGetQueryObjectuiv(q, GL_QUERY_RESULT, &mut n);
        glDeleteQueries(1, &q);
        assert_eq!(err(), GL_NO_ERROR, "query");
        let mut b: GLint = 0;
        glGetIntegeri_v(GL_TRANSFORM_FEEDBACK_BUFFER_BINDING, 0, &mut b);
        glBindBuffer(GL_TRANSFORM_FEEDBACK_BUFFER, b as GLuint);
        let p = glMapBufferRange(GL_TRANSFORM_FEEDBACK_BUFFER, 0, 60, GL_MAP_READ_BIT) as *const f32;
        assert!(!p.is_null(), "mapped");
        let got = core::slice::from_raw_parts(p, 15).to_vec();
        glUnmapBuffer(GL_TRANSFORM_FEEDBACK_BUFFER);
        glBindBuffer(GL_TRANSFORM_FEEDBACK_BUFFER, 0);
        (got, n)
    }
}

fn flush() -> Vec<Seen> {
    // SAFETY: plain GL call.
    unsafe { glFinish() };
    assert_eq!(err(), GL_NO_ERROR);
    core::mem::take(&mut *SEEN.lock().unwrap()).into_iter().filter(|s| s.draws).collect()
}

fn finish(fb: GLuint, rbs: &[GLuint], b: GLuint) {
    // SAFETY: names this test made.
    unsafe {
        glDisable(GL_RASTERIZER_DISCARD);
        glBindBufferBase(GL_TRANSFORM_FEEDBACK_BUFFER, 0, 0);
        glDeleteBuffers(1, &b);
        glDisableVertexAttribArray(0);
    }
    release(fb, rbs);
}

/// The draw hands the back end four distinct, drawable RGBA8 targets, and
/// the output registers for them are the four of `mrt_layout`.
fn four_targets(d: &Seen) -> Vec<u64> {
    four_targets_of(d, &[0, 1, 2, 3])
}

/// As `four_targets`, with images in `live` slots only (the others, GL_NONE
/// inside the list, are four-slot holes with gpu 0).
fn four_targets_of(d: &Seen, live: &[usize]) -> Vec<u64> {
    assert_eq!((d.num_targets, d.out_shift), (4, 0), "four targets, draw buffer 0 first");
    assert_eq!(d.targets.len(), 4);
    let gpus: Vec<u64> = d.targets.iter().map(|t| t.0).collect();
    for i in 0..4 {
        assert_eq!(gpus[i] != 0, live.contains(&i), "slot {i} has an image iff it is listed");
        for j in i + 1..4 {
            assert!(gpus[i] == 0 || gpus[i] != gpus[j], "targets {i} and {j} are different images");
        }
    }
    let l = layout(&[1u8; 4]).expect("four one-dword targets");
    assert_eq!(l.tile_buffers, 0, "four RGBA8 outputs fit the four output registers");
    let regs: Vec<u8> = (0..4)
        .map(|i| match l.place[i] {
            Place::Register { first } => first,
            p => panic!("{p:?}"),
        })
        .collect();
    assert_eq!(regs, [0, 1, 2, 3]);
    gpus
}

#[test]
fn capture_and_count_beside_a_four_target_draw() {
    let _g = context();
    let (fb, rbs, b) = setup(&ATT);
    let (got, n) = capture_draw();
    assert_eq!(got, want(), "the feedback buffer holds cap and k of the three vertices");
    assert_eq!(n, 1, "one triangle written");
    let s = flush();
    assert_eq!(s.len(), 1, "the captured draw is still drawn, once");
    assert_eq!((s[0].instance_count, s[0].index_count), (1, 3));
    four_targets(&s[0]);
    finish(fb, &rbs, b);
}

#[test]
fn rasterizer_discard_keeps_the_capture_and_skips_the_four_target_draw() {
    let _g = context();
    let (fb, rbs, b) = setup(&ATT);
    // SAFETY: plain GL call.
    unsafe { glEnable(GL_RASTERIZER_DISCARD) };
    assert_eq!(err(), GL_NO_ERROR);
    let (got, n) = capture_draw();
    assert_eq!(got, want(), "capture is independent of rasterisation");
    assert_eq!(n, 1);
    assert!(flush().is_empty(), "nothing reaches the back end under discard");
    // SAFETY: plain GL call.
    unsafe { glDisable(GL_RASTERIZER_DISCARD) };
    let (got, n) = capture_draw();
    assert_eq!((got, n), (want(), 1));
    let s = flush();
    assert_eq!(s.len(), 1, "without discard the draw is submitted again");
    four_targets(&s[0]);
    finish(fb, &rbs, b);
}

#[test]
fn draw_buffers_with_gaps_keep_the_capture_and_four_targets() {
    let _g = context();
    let (fb, rbs, b) = setup(&[GL_COLOR_ATTACHMENT0, GL_NONE, GL_NONE, GL_COLOR_ATTACHMENT3]);
    let (got, n) = capture_draw();
    assert_eq!(got, want());
    assert_eq!(n, 1);
    let s = flush();
    assert_eq!(s.len(), 1);
    four_targets_of(&s[0], &[0, 3]);
    // SAFETY: plain GL calls.
    unsafe { glEnable(GL_RASTERIZER_DISCARD) };
    let (got, n) = capture_draw();
    assert_eq!((got, n), (want(), 1));
    assert!(flush().is_empty());
    finish(fb, &rbs, b);
}
