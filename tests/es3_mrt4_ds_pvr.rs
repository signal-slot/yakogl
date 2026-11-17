// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Depth/stencil beside four colour attachments on the PowerVR (BXS-4-64,
//! AM62P), through the GL API, on the host (pvr-ds4-test, plan #368).
//!
//! `backend::HOST_DRAW_TARGETS` stands in a draw-target count of 4 (the
//! shipped `DRAW_TARGETS` is 3). As in tests/es3_msaa_pvr.rs the GL core is
//! driven into a stand-in back end that records what it is handed and fills
//! no pixels of its own; the PowerVR's draw needs the board's job memory. What
//! is pinned is therefore what the GL core asks of the back end with four
//! colour attachments and a depth/stencil attachment (DEPTH24_STENCIL8, and
//! DEPTH_COMPONENT16 alone): which buffers a `glClear` and
//! `glClearBuffer*` name, the depth/stencil identities, the draw's depth and
//! stencil state, the four targets of a depth-tested draw, and that a read
//! of attachment 3 reads target 3's storage (a marker the test puts there,
//! since the stand-in's gpu address is its CPU pointer).
//!
//! Defects found are `#[ignore = "defect: ..."]`; none is fixed here.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test es3_mrt4_ds_pvr
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
use yakogl::gl::es3::*;
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

/// What the back end was handed by one call of `draw`.
#[derive(Clone, Debug)]
struct Seen {
    /// An application draw (vertices), not a clear alone.
    draws: bool,
    num_targets: u8,
    /// `targets[..GLES_MAX_TARGETS]`: (gpu, format).
    targets: Vec<(u64, u8)>,
    do_clear: bool,
    clear_target_mask: u8,
    clear_rgba: [f32; 4],
    do_clear_depth: bool,
    clear_depth: f32,
    do_clear_stencil: bool,
    clear_stencil: u8,
    depth_id: u32,
    stencil_id: u32,
    depth_test: bool,
    depth_func: u8,
    depth_write: bool,
    stencil: bool,
    stencil_func: u8,
    stencil_ref: u8,
    color_write: bool,
    color_mask: u8,
}

static SEEN: Mutex<Vec<Seen>> = Mutex::new(Vec::new());

unsafe extern "C" fn b_draw(d: *const GlesDraw) -> i32 {
    let d = &*d;
    SEEN.lock().unwrap().push(Seen {
        draws: d.index_count != 0,
        num_targets: d.num_targets,
        targets: d.targets.iter().map(|t| (t.gpu, t.format)).collect(),
        do_clear: d.do_clear,
        clear_target_mask: d.clear_target_mask,
        clear_rgba: d.clear_rgba,
        do_clear_depth: d.do_clear_depth,
        clear_depth: d.clear_depth,
        do_clear_stencil: d.do_clear_stencil,
        clear_stencil: d.clear_stencil,
        depth_id: d.depth_id,
        stencil_id: d.stencil_id,
        depth_test: d.depth_test,
        depth_func: d.depth_func as u8,
        depth_write: d.depth_write,
        stencil: d.stencil,
        stencil_func: d.stencil_func as u8,
        stencil_ref: d.stencil_ref,
        color_write: d.color_write,
        color_mask: d.color_mask,
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
    name: c"es3-mrt4-ds-pvr".as_ptr() as *const c_char,
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

/// The lock, the context, and every piece of GL state a test here changes
/// put back to its default, so the result does not depend on the test order.
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
        glUseProgram(0);
        glDisable(GL_DEPTH_TEST);
        glDisable(GL_STENCIL_TEST);
        glDisable(GL_SCISSOR_TEST);
        glDisable(GL_BLEND);
        glDepthMask(1);
        glDepthFunc(GL_LESS);
        glStencilFunc(GL_ALWAYS, 0, 0xff);
        glStencilOp(GL_KEEP, GL_KEEP, GL_KEEP);
        glStencilMask(0xff);
        glColorMask(1, 1, 1, 1);
        glClearColor(0.0, 0.0, 0.0, 0.0);
        glClearDepthf(1.0);
        glClearStencil(0);
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

/// Everything the back end was handed since the last call, oldest first.
fn take_seen() -> Vec<Seen> {
    // SAFETY: plain GL call.
    unsafe { glFinish() };
    core::mem::take(&mut *SEEN.lock().unwrap())
}

/* ---- framebuffers, the program, the draw ------------------------------- */

/// The depth/stencil attachment beside the four colour ones.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Ds {
    D24S8,
    D16,
}

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

fn renderbuffer(format: GLenum) -> GLuint {
    let mut rb = 0;
    // SAFETY: plain GL calls.
    unsafe {
        glGenRenderbuffers(1, &mut rb);
        glBindRenderbuffer(GL_RENDERBUFFER, rb);
        glRenderbufferStorage(GL_RENDERBUFFER, format, 8, 8);
    }
    assert_eq!(err(), GL_NO_ERROR);
    rb
}

/// A bound, complete 8x8 framebuffer: four RGBA8 renderbuffers on
/// COLOR_ATTACHMENT0..3, all four draw buffers, the depth/stencil of `ds`.
fn framebuffer(ds: Ds) -> Fbo {
    let mut fb = 0;
    // SAFETY: room for one name; plain GL calls.
    unsafe {
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
    }
    let mut fbo = Fbo { fb, rbs: Vec::new() };
    for i in 0..4 {
        let rb = renderbuffer(GL_RGBA8);
        fbo.rbs.push(rb);
        // SAFETY: a renderbuffer this test made.
        unsafe { glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0 + i, GL_RENDERBUFFER, rb) };
    }
    let (format, attachment) = match ds {
        Ds::D24S8 => (GL_DEPTH24_STENCIL8, GL_DEPTH_STENCIL_ATTACHMENT),
        Ds::D16 => (GL_DEPTH_COMPONENT16, GL_DEPTH_ATTACHMENT),
    };
    let rb = renderbuffer(format);
    fbo.rbs.push(rb);
    let bufs: [GLenum; 4] = core::array::from_fn(|i| GL_COLOR_ATTACHMENT0 + i as GLenum);
    // SAFETY: a renderbuffer this test made; four enums; plain GL calls.
    unsafe {
        glFramebufferRenderbuffer(GL_FRAMEBUFFER, attachment, GL_RENDERBUFFER, rb);
        glDrawBuffers(4, bufs.as_ptr());
        assert_eq!(err(), GL_NO_ERROR);
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE, "{ds:?}");
    }
    fbo
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

const FRAGMENT: &str = "#version 300 es
precision highp float;
layout(location = 0) out vec4 o0;
layout(location = 1) out vec4 o1;
layout(location = 2) out vec4 o2;
layout(location = 3) out vec4 o3;
void main() {
    o0 = vec4(1.0, 0.0, 0.0, 1.0);
    o1 = vec4(0.0, 1.0, 0.0, 1.0);
    o2 = vec4(0.0, 0.0, 1.0, 1.0);
    o3 = vec4(1.0, 1.0, 0.0, 1.0);
}
";

/// The current program: four float outputs, vertices at clip depth `z`.
fn program(z: f32) {
    let vs = format!("#version 300 es\nin vec4 p;\nvoid main() {{ gl_Position = vec4(p.xy, {z:?}, 1.0); }}\n");
    let v = shader(GL_VERTEX_SHADER, &vs);
    let f = shader(GL_FRAGMENT_SHADER, FRAGMENT);
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

/// The 8x8 target's whole area: one oversized triangle.
static FULL: [f32; 12] = [-1.0, -1.0, 0.0, 1.0, 3.0, -1.0, 0.0, 1.0, -1.0, 3.0, 0.0, 1.0];

/// Draw `FULL` and keep everything the back end was handed since the last
/// `take_seen`, so a clear the GL core held back for this draw is in it.
fn draw() -> Vec<Seen> {
    // SAFETY: `FULL` is static and outlives the draw.
    unsafe {
        glBindBuffer(GL_ARRAY_BUFFER, 0);
        glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, FULL.as_ptr() as *const c_void);
        glEnableVertexAttribArray(0);
        glDrawArrays(GL_TRIANGLES, 0, 3);
    }
    assert_eq!(err(), GL_NO_ERROR);
    take_seen()
}

/// The last application draw of `seen`.
fn the_draw(seen: &[Seen]) -> &Seen {
    seen.iter().rev().find(|s| s.draws).expect("the draw reached the back end")
}

/// Whether some call of `seen` clears colour target `n` (a mask of 0 is every
/// target, as glClear).
fn clears_colour(seen: &[Seen], n: usize) -> bool {
    seen.iter().any(|s| s.do_clear && (s.clear_target_mask == 0 || s.clear_target_mask & (1 << n) != 0))
}

/* ---- glClear: colour, depth and stencil together ------------------------ */

/// `glClear(COLOR | DEPTH | STENCIL)` names all four colour targets (and the
/// depth and the stencil where there is one), with one value each.
#[test]
fn clear_all_reaches_four_targets_and_depth_stencil() {
    for ds in [Ds::D24S8, Ds::D16] {
        let _g = context();
        let fbo = framebuffer(ds);
        take_seen();
        // SAFETY: plain GL calls.
        unsafe {
            glClearColor(0.25, 0.5, 0.75, 1.0);
            glClearDepthf(0.5);
            glClearStencil(0x5a);
            glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT | GL_STENCIL_BUFFER_BIT);
        }
        assert_eq!(err(), GL_NO_ERROR);
        program(0.0);
        let seen = draw();
        for n in 0..4 {
            assert!(clears_colour(&seen, n), "{ds:?}: colour {n} not cleared: {seen:#?}");
        }
        let c = seen.iter().find(|s| s.do_clear).unwrap();
        assert_eq!(c.clear_rgba, [0.25, 0.5, 0.75, 1.0], "{ds:?}");
        assert!(seen.iter().any(|s| s.do_clear_depth && s.clear_depth == 0.5), "{ds:?}: the depth clear: {seen:#?}");
        let stencil = seen.iter().any(|s| s.do_clear_stencil && s.clear_stencil == 0x5a);
        assert_eq!(stencil, ds == Ds::D24S8, "{ds:?}: the stencil clear: {seen:#?}");
        let d = the_draw(&seen);
        assert_eq!(d.num_targets, 4, "{ds:?}");
        assert_ne!(d.depth_id, 0, "{ds:?}");
        match ds {
            Ds::D24S8 => assert_eq!(d.stencil_id, d.depth_id),
            Ds::D16 => assert_eq!(d.stencil_id, 0),
        }
        drop(fbo);
    }
}

/// A clear of depth and stencil alone clears no colour target.
#[test]
fn clear_depth_stencil_alone_clears_no_colour() {
    let _g = context();
    let fbo = framebuffer(Ds::D24S8);
    take_seen();
    // SAFETY: plain GL calls.
    unsafe {
        glClearDepthf(0.25);
        glClearStencil(3);
        glClear(GL_DEPTH_BUFFER_BIT | GL_STENCIL_BUFFER_BIT);
    }
    assert_eq!(err(), GL_NO_ERROR);
    program(0.0);
    let seen = draw();
    for n in 0..4 {
        assert!(!clears_colour(&seen, n), "colour {n} cleared: {seen:#?}");
    }
    assert!(seen.iter().any(|s| s.do_clear_depth && s.clear_depth == 0.25), "{seen:#?}");
    assert!(seen.iter().any(|s| s.do_clear_stencil && s.clear_stencil == 3), "{seen:#?}");
    drop(fbo);
}

/* ---- glClearBufferfi and glClearBufferfv(GL_COLOR, 3) -------------------- */

/// `glClearBufferfv(GL_COLOR, 3)` and `glClearBufferfi(GL_DEPTH_STENCIL, 0)`
/// together: target 3 alone takes its colour, and the depth/stencil its pair.
#[test]
fn clear_buffer_fi_and_color_3_together() {
    let _g = context();
    let fbo = framebuffer(Ds::D24S8);
    take_seen();
    let colour = [0.0f32, 1.0, 0.5, 1.0];
    // SAFETY: four floats; plain GL calls.
    unsafe {
        glClearBufferfv(GL_COLOR, 3, colour.as_ptr());
        glClearBufferfi(GL_DEPTH_STENCIL, 0, 0.75, 7);
    }
    assert_eq!(err(), GL_NO_ERROR);
    program(0.0);
    let seen = draw();
    assert!(clears_colour(&seen, 3), "{seen:#?}");
    for n in 0..3 {
        assert!(!clears_colour(&seen, n), "colour {n} cleared: {seen:#?}");
    }
    let c = seen.iter().find(|s| s.do_clear && s.clear_target_mask & 8 != 0).unwrap();
    assert_eq!(c.clear_rgba, colour);
    assert!(seen.iter().any(|s| s.do_clear_depth && s.clear_depth == 0.75), "{seen:#?}");
    assert!(seen.iter().any(|s| s.do_clear_stencil && s.clear_stencil == 7), "{seen:#?}");
    drop(fbo);
}

/// The other order: the depth/stencil pair first, then colour 3.
#[test]
fn clear_buffer_color_3_after_fi() {
    let _g = context();
    let fbo = framebuffer(Ds::D24S8);
    take_seen();
    let colour = [1.0f32, 0.0, 0.5, 1.0];
    // SAFETY: four floats; plain GL calls.
    unsafe {
        glClearBufferfi(GL_DEPTH_STENCIL, 0, 0.25, 9);
        glClearBufferfv(GL_COLOR, 3, colour.as_ptr());
    }
    assert_eq!(err(), GL_NO_ERROR);
    program(0.0);
    let seen = draw();
    assert!(clears_colour(&seen, 3), "{seen:#?}");
    for n in 0..3 {
        assert!(!clears_colour(&seen, n), "colour {n} cleared: {seen:#?}");
    }
    assert!(seen.iter().any(|s| s.do_clear_depth && s.clear_depth == 0.25), "{seen:#?}");
    assert!(seen.iter().any(|s| s.do_clear_stencil && s.clear_stencil == 9), "{seen:#?}");
    drop(fbo);
}

/* ---- a depth-tested draw ------------------------------------------------ */

/// A depth-tested draw hands the back end all four targets with their own
/// storage, the depth test (LESS, writing) and the depth identity.
#[test]
fn depth_tested_draw_hands_four_targets_and_the_test() {
    for ds in [Ds::D24S8, Ds::D16] {
        let _g = context();
        let fbo = framebuffer(ds);
        program(0.0);
        // SAFETY: plain GL calls.
        unsafe {
            glEnable(GL_DEPTH_TEST);
            glDepthFunc(GL_LESS);
            glDepthMask(1);
        }
        take_seen();
        let seen = draw();
        let d = the_draw(&seen);
        assert_eq!(d.num_targets, 4, "{ds:?}");
        assert!(d.depth_test && d.depth_write, "{ds:?}");
        assert_eq!(d.depth_func, GLES_CMP_LESS as u8, "{ds:?}");
        assert_ne!(d.depth_id, 0, "{ds:?}");
        assert!(d.color_write && d.color_mask == 0xf, "{ds:?}: {d:?}");
        let gpus: Vec<u64> = d.targets[..4].iter().map(|t| t.0).collect();
        assert!(gpus.iter().all(|&g| g != 0), "{ds:?}: {gpus:x?}");
        for i in 0..4 {
            for j in 0..i {
                assert_ne!(gpus[i], gpus[j], "{ds:?}: targets {i} and {j} share storage");
            }
            assert_eq!(d.targets[i].1, d.targets[0].1, "{ds:?}: target {i} format");
        }
        drop(fbo);
    }
}

/// A depth test that rejects the draw everywhere (GL_GREATER against the
/// cleared 1.0, vertices at 0.5) still names four targets and keeps the colour
/// write on: the test is the back end's, so the targets are written only where
/// it passes (the GL core does not drop them).
#[test]
fn a_failing_depth_test_still_names_four_targets() {
    let _g = context();
    let fbo = framebuffer(Ds::D24S8);
    // SAFETY: plain GL calls.
    unsafe {
        glClearDepthf(1.0);
        glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT);
        glEnable(GL_DEPTH_TEST);
        glDepthFunc(GL_GREATER);
        glDepthMask(0);
    }
    program(0.5);
    let seen = draw();
    let d = the_draw(&seen);
    assert_eq!(d.num_targets, 4);
    assert!(d.depth_test && !d.depth_write);
    assert_eq!(d.depth_func, GLES_CMP_GREATER as u8);
    assert!(d.color_write);
    drop(fbo);
}

/// A stencil-tested draw beside four colour targets: the stencil state and the
/// stencil identity (the D24S8's, equal to the depth's) reach the back end.
#[test]
fn stencil_tested_draw_hands_four_targets() {
    let _g = context();
    let fbo = framebuffer(Ds::D24S8);
    // SAFETY: plain GL calls.
    unsafe {
        glEnable(GL_STENCIL_TEST);
        glStencilFunc(GL_EQUAL, 5, 0xff);
        glStencilOp(GL_KEEP, GL_KEEP, GL_REPLACE);
    }
    program(0.0);
    let seen = draw();
    let d = the_draw(&seen);
    assert_eq!(d.num_targets, 4);
    assert!(d.stencil);
    assert_eq!((d.stencil_func, d.stencil_ref), (GLES_CMP_EQUAL as u8, 5));
    assert_ne!(d.stencil_id, 0);
    assert_eq!(d.stencil_id, d.depth_id);
    drop(fbo);
}

/* ---- read-back of attachment 3 ----------------------------------------- */

/// A read of `GL_COLOR_ATTACHMENT3` reads target 3's storage and no other:
/// the test puts a marker in target 3's memory (the stand-in's gpu address is
/// its CPU pointer) and another in each of the other three.
#[test]
fn read_back_of_attachment_3_after_a_depth_tested_draw() {
    let _g = context();
    let fbo = framebuffer(Ds::D24S8);
    program(0.0);
    // SAFETY: plain GL calls.
    unsafe {
        glEnable(GL_DEPTH_TEST);
        glDepthFunc(GL_LESS);
        glDepthMask(1);
    }
    let seen = draw();
    let d = the_draw(&seen);
    assert_eq!(d.num_targets, 4);
    for (i, target) in d.targets.iter().enumerate().take(4) {
        let marker: u8 = if i == 3 { 0xc3 } else { 0x10 + i as u8 };
        // SAFETY: the target is the host buffer `m_gpu_alloc` leaked, 8x8
        // RGBA8, and nothing else touches it while the lock is held.
        unsafe { core::ptr::write_bytes(target.0 as *mut u8, marker, 8 * 8 * 4) };
    }
    let mut px = [[0u8; 4]; 64];
    // SAFETY: room for 64 RGBA8 pixels; a framebuffer this test made.
    unsafe {
        glBindFramebuffer(GL_READ_FRAMEBUFFER, fbo.fb);
        glReadBuffer(GL_COLOR_ATTACHMENT3);
        glReadPixels(0, 0, 8, 8, GL_RGBA, GL_UNSIGNED_BYTE, px.as_mut_ptr() as *mut c_void);
    }
    assert_eq!(err(), GL_NO_ERROR);
    assert!(px.iter().all(|p| *p == [0xc3; 4]), "{px:?}");
    // SAFETY: as above, for attachment 0.
    unsafe {
        glReadBuffer(GL_COLOR_ATTACHMENT0);
        glReadPixels(0, 0, 8, 8, GL_RGBA, GL_UNSIGNED_BYTE, px.as_mut_ptr() as *mut c_void);
    }
    assert!(px.iter().all(|p| *p == [0x10; 4]), "{px:?}");
}
