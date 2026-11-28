// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Instanced draws that write four colour targets, with the PowerVR build
//! and a four-target stand-in (plan #368 item `inst4-pvr`; RC14,
//! docs/notes/mrt4-v3d-pvr.md, docs/notes/pvr-mrt4-budget.md).
//!
//! `backend::HOST_DRAW_TARGETS` = 4 stands in for the knob (as
//! `es3_mrt4_gaps_pvr`; `pvr_debug_set_draw_targets` is the later spelling).
//! THIS RIG CANNOT SEE PIXELS of an instanced draw: the PowerVR's own draw
//! needs the board's job memory and the host back end here is a recorder.
//! What is pinned is what the GL core hands the back end for a
//! four-RGBA8-output program (outputs at locations 0..3) drawn with
//! `glDrawArraysInstanced` / `glDrawElementsInstanced`, which decides every
//! instance's region and every target's value on the device:
//!
//! 1. divisor 0 (`gl_InstanceID` offsets in the shader): one draw whose
//!    `instance_count` is the count, `instance_id` 0 (the USC counts), and
//!    the four targets with distinct addresses;
//! 2. `glVertexAttribDivisor(k, 1)`: the same one draw, the attribute
//!    `per_instance` with divisor 1 (the stand-in back end's
//!    `max_instance_divisor` is 1);
//! 3. a divisor past that maximum (2): drawn instance by instance, each draw
//!    carrying its `instance_id` 0..n-1, the array a constant (not
//!    per-instance) and all four targets in every draw;
//! 4. the render pass: every draw of a mixed frame (instanced, plain, elements
//!    instanced) hands the back end the same four targets, formats,
//!    `num_targets` 4 and `out_shift` 0, and `mrt_layout` puts the four
//!    RGBA8 outputs in registers 0..3 with no tile-buffer spill (the render
//!    layer's four-target EOT/PBE state is pinned by `es3_mrt4_pvr`).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test es3_mrt4_instanced_pvr
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
    instance_id: u32,
    index_count: u32,
    /// `targets[..num_targets]`: (gpu, format).
    targets: Vec<(u64, u8)>,
    num_targets: u8,
    out_shift: u8,
    /// (per_instance, divisor) of the attributes.
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
    name: c"es3-mrt4-instanced-pvr".as_ptr() as *const c_char,
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
/// outputs at locations 0..3, each a distinct channel mix of it.
fn program() {
    let vs = "#version 300 es\nlayout(location = 0) in vec4 p;\nlayout(location = 1) in float k;\n\
        flat out float v;\n\
        void main() { gl_Position = vec4(p.x + float(gl_InstanceID) * 0.25, p.yzw); v = k; }\n";
    let fs = "#version 300 es\nprecision highp float;\nflat in float v;\n\
        layout(location = 0) out vec4 o0;\nlayout(location = 1) out vec4 o1;\n\
        layout(location = 2) out vec4 o2;\nlayout(location = 3) out vec4 o3;\n\
        void main() { o0 = vec4(v, 0.0, 0.0, 1.0); o1 = vec4(0.0, v, 0.0, 1.0);\n\
        o2 = vec4(0.0, 0.0, v, 1.0); o3 = vec4(v, v, 0.0, 1.0); }\n";
    let v = shader(GL_VERTEX_SHADER, vs);
    let f = shader(GL_FRAGMENT_SHADER, fs);
    // SAFETY: plain GL calls.
    unsafe {
        let p = glCreateProgram();
        glAttachShader(p, v);
        glAttachShader(p, f);
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        assert_eq!(ok, 1, "the four-output instanced program links");
        glUseProgram(p);
    }
}

static TRI: [f32; 12] = [-1.0, -1.0, 0.0, 1.0, 0.0, -1.0, 0.0, 1.0, -1.0, 1.0, 0.0, 1.0];
static KS: [f32; 4] = [0.25, 0.5, 0.75, 1.0];
static IDX: [u16; 3] = [0, 1, 2];

/// Arrays: positions from client memory, `k` with `divisor` (0: constant 0.5).
fn arrays(divisor: GLuint) {
    // SAFETY: the statics outlive the draws.
    unsafe {
        glBindBuffer(GL_ARRAY_BUFFER, 0);
        glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, TRI.as_ptr() as *const c_void);
        glEnableVertexAttribArray(0);
        glVertexAttribPointer(1, 1, GL_FLOAT, 0, 0, KS.as_ptr() as *const c_void);
        if divisor == 0 {
            glDisableVertexAttribArray(1);
            glVertexAttrib1f(1, 0.5);
        } else {
            glEnableVertexAttribArray(1);
        }
        glVertexAttribDivisor(1, divisor);
    }
}

fn flush() -> Vec<Seen> {
    // SAFETY: plain GL call.
    unsafe { glFinish() };
    assert_eq!(err(), GL_NO_ERROR);
    core::mem::take(&mut *SEEN.lock().unwrap()).into_iter().filter(|s| s.draws).collect()
}

/// A bound four-attachment framebuffer; its attachments' addresses, from the
/// first draw.
fn setup() -> (GLuint, Vec<GLuint>) {
    let (fb, rbs) = framebuffer(&[0, 1, 2, 3]);
    assert!(complete(), "four RGBA8 attachments are complete");
    draw_buffers(&ATT);
    program();
    (fb, rbs)
}

/// Every draw hands the back end four distinct, drawable RGBA8 targets, and
/// the output registers for them are the four of `mrt_layout`.
fn four_targets(d: &Seen) -> Vec<u64> {
    assert_eq!((d.num_targets, d.out_shift), (4, 0), "four targets, draw buffer 0 first");
    assert_eq!(d.targets.len(), 4);
    let gpus: Vec<u64> = d.targets.iter().map(|t| t.0).collect();
    assert!(gpus.iter().all(|&g| g != 0), "every slot has an image");
    for i in 0..4 {
        for j in i + 1..4 {
            assert_ne!(gpus[i], gpus[j], "targets {i} and {j} are different images");
        }
    }
    assert!(d.targets.iter().all(|t| t.1 == d.targets[0].1), "four RGBA8 attachments, one format");
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
fn draw_arrays_instanced_is_one_four_target_draw_with_the_count() {
    let _g = context();
    let (fb, rbs) = setup();
    arrays(0);
    // SAFETY: plain GL call.
    unsafe { glDrawArraysInstanced(GL_TRIANGLES, 0, 3, 4) };
    let s = flush();
    assert_eq!(s.len(), 1, "gl_InstanceID is the USC's: one draw for all instances");
    assert_eq!((s[0].instance_count, s[0].instance_id, s[0].index_count), (4, 0, 3));
    four_targets(&s[0]);
    release(fb, &rbs);
}

#[test]
fn draw_elements_instanced_is_one_four_target_draw_with_the_count() {
    let _g = context();
    let (fb, rbs) = setup();
    arrays(0);
    // SAFETY: `IDX` is static.
    unsafe { glDrawElementsInstanced(GL_TRIANGLES, 3, GL_UNSIGNED_SHORT, IDX.as_ptr() as *const c_void, 3) };
    let s = flush();
    assert_eq!(s.len(), 1);
    assert_eq!((s[0].instance_count, s[0].instance_id, s[0].index_count), (3, 0, 3));
    four_targets(&s[0]);
    release(fb, &rbs);
}

#[test]
fn divisor_one_attribute_is_a_per_instance_stream_in_one_four_target_draw() {
    let _g = context();
    let (fb, rbs) = setup();
    arrays(1);
    // SAFETY: plain GL calls.
    unsafe {
        glDrawArraysInstanced(GL_TRIANGLES, 0, 3, 4);
        glDrawElementsInstanced(GL_TRIANGLES, 3, GL_UNSIGNED_SHORT, IDX.as_ptr() as *const c_void, 4);
    }
    let s = flush();
    assert_eq!(s.len(), 2);
    for d in &s {
        assert_eq!((d.instance_count, d.instance_id), (4, 0));
        assert!(d.attribs.contains(&(1, 1)), "the divisor-1 array is per-instance: {:?}", d.attribs);
        four_targets(d);
    }
    release(fb, &rbs);
}

#[test]
fn divisor_past_the_back_ends_maximum_draws_each_instance_into_four_targets() {
    let _g = context();
    let (fb, rbs) = setup();
    arrays(2);
    // SAFETY: plain GL call.
    unsafe { glDrawArraysInstanced(GL_TRIANGLES, 0, 3, 4) };
    let s = flush();
    assert_eq!(s.iter().map(|d| d.instance_id).collect::<Vec<_>>(), [0, 1, 2, 3], "one draw per instance");
    let first = four_targets(&s[0]);
    for d in &s {
        assert_eq!(d.instance_count, 1);
        assert!(d.attribs.iter().all(|a| a.0 == 0), "the array is the instance's constant: {:?}", d.attribs);
        assert_eq!(four_targets(d), first, "every instance writes the same four images");
    }
    // SAFETY: plain GL call.
    unsafe { glDrawArrays(GL_TRIANGLES, 0, 3) };
    let s = flush();
    assert_eq!((s[0].instance_id, s[0].instance_count), (0, 1), "a later plain draw is instance 0 again");
    release(fb, &rbs);
}

#[test]
fn a_mixed_frame_keeps_four_targets_across_every_draw() {
    let _g = context();
    let (fb, rbs) = setup();
    // SAFETY: statics outlive the draws; plain GL calls.
    unsafe {
        arrays(1);
        glDrawArraysInstanced(GL_TRIANGLES, 0, 3, 3);
        glDrawArrays(GL_TRIANGLES, 0, 3);
        arrays(2);
        glDrawElementsInstanced(GL_TRIANGLES, 3, GL_UNSIGNED_SHORT, IDX.as_ptr() as *const c_void, 2);
        arrays(1);
        glDrawElementsInstanced(GL_TRIANGLES, 3, GL_UNSIGNED_SHORT, IDX.as_ptr() as *const c_void, 5);
    }
    let s = flush();
    // 1 + 1 + 2 (per instance) + 1
    assert_eq!(s.len(), 5);
    let first = four_targets(&s[0]);
    for d in &s {
        assert_eq!(four_targets(d), first, "the render pass keeps its four targets");
        assert_eq!(d.targets, s[0].targets, "formats too");
    }
    assert_eq!(
        s.iter().map(|d| (d.instance_count, d.instance_id)).collect::<Vec<_>>(),
        [(3, 0), (1, 0), (1, 0), (1, 1), (5, 0)]
    );
    release(fb, &rbs);
}
