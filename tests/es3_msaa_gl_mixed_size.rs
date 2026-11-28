// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! ES 3.0 attachments of different sizes in one 4x framebuffer, on the 4x
//! stand-in back end of tests/es3_msaa_gl.rs (its helpers are copied here).
//! ES 3.0 4.4.4 has no size rule: the framebuffer is complete, and drawing
//! and resolving are limited to the smallest attachment. The tests pin what
//! the GL core hands the back end today; an expected result that differs is
//! `#[ignore]`d and names its GL-core cause.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_msaa_gl_mixed_size
//! ```

#![cfg(all(feature = "host-test", feature = "shaderc-online", not(feature = "report-es2")))]
#![allow(dead_code)]

use core::ffi::{c_char, c_void};
use std::ffi::CString;
use std::sync::atomic::Ordering;
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{self, GlesDraw, GlesHeap, YakoglBackend};
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
    /// `GlesDraw::samples`.
    samples: u8,
    /// `target_w`, `target_h`.
    target: (u32, u32),
    /// Each of `targets[..num_targets]` as (w, h).
    targets: Vec<(u32, u32)>,
    /// `ds_w`, `ds_h`.
    ds: (u32, u32),
}

static SEEN: Mutex<Vec<Seen>> = Mutex::new(Vec::new());

/// The 4x targets drawn since the last `finish`: (`target_gpu`, w, h).
static PENDING: Mutex<Vec<(u64, u32, u32)>> = Mutex::new(Vec::new());

/// The colour the stand-in's job-end resolve leaves in an RGBA8 target.
const RESOLVED: [u8; 4] = [0x40, 0x80, 0xc0, 0xff];

unsafe extern "C" fn b_draw(d: *const GlesDraw) -> i32 {
    let d = &*d;
    SEEN.lock().unwrap().push(Seen {
        draws: d.index_count != 0,
        samples: d.samples,
        target: (d.target_w, d.target_h),
        targets: d.targets[..(d.num_targets as usize).min(d.targets.len())].iter().map(|t| (t.w, t.h)).collect(),
        ds: (d.ds_w, d.ds_h),
    });
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

/// The current program: one float output.
fn program() {
    let v = shader(GL_VERTEX_SHADER, "#version 300 es\nin vec4 p;\nvoid main() { gl_Position = p; }\n");
    let f = shader(
        GL_FRAGMENT_SHADER,
        "#version 300 es\nprecision highp float;\nlayout(location = 0) out vec4 o;\nvoid main() { o = vec4(0.25, 0.5, 0.75, 1.0); }\n",
    );
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

/// Draw one triangle over the bound framebuffer, the job left unfinished.
fn draw_unfinished() {
    static TRI: [f32; 12] = [-1.0, -1.0, 0.0, 1.0, 3.0, -1.0, 0.0, 1.0, -1.0, 3.0, 0.0, 1.0];
    // SAFETY: TRI is static and outlives the draw.
    unsafe {
        glBindBuffer(GL_ARRAY_BUFFER, 0);
        glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, TRI.as_ptr() as *const c_void);
        glEnableVertexAttribArray(0);
        glDrawArrays(GL_TRIANGLES, 0, 3);
    }
}

/// Draw one triangle and hand back what the back end was given for it.
fn draw() -> Seen {
    SEEN.lock().unwrap().clear();
    draw_unfinished();
    // SAFETY: plain GL call.
    unsafe { glFinish() };
    assert_eq!(err(), GL_NO_ERROR);
    let s = core::mem::take(&mut *SEEN.lock().unwrap());
    s.into_iter().rev().find(|s| s.draws).expect("the draw reached the back end")
}

/// glBlitFramebuffer of colour from `read` to `draw_fb`; the GL error.
fn blit(read: GLuint, draw_fb: GLuint, src: [GLint; 4], dst: [GLint; 4]) -> GLenum {
    // SAFETY: plain GL calls on names this test made.
    unsafe {
        glBindFramebuffer(GL_READ_FRAMEBUFFER, read);
        glBindFramebuffer(GL_DRAW_FRAMEBUFFER, draw_fb);
        glReadBuffer(GL_COLOR_ATTACHMENT0);
        while glGetError() != GL_NO_ERROR {}
        glBlitFramebuffer(
            src[0],
            src[1],
            src[2],
            src[3],
            dst[0],
            dst[1],
            dst[2],
            dst[3],
            GL_COLOR_BUFFER_BIT,
            GL_NEAREST,
        );
    }
    err()
}

/// The `w` x `h` RGBA8 pixels of colour attachment 0 of single-sample `fb`.
fn pixels(fb: GLuint, w: usize, h: usize) -> Vec<[u8; 4]> {
    let mut px = vec![[0x11u8; 4]; w * h];
    // SAFETY: room for w x h RGBA/UNSIGNED_BYTE pixels.
    unsafe {
        glBindFramebuffer(GL_READ_FRAMEBUFFER, fb);
        glReadBuffer(GL_COLOR_ATTACHMENT0);
        glReadPixels(0, 0, w as GLsizei, h as GLsizei, GL_RGBA, GL_UNSIGNED_BYTE, px.as_mut_ptr() as *mut c_void);
    }
    assert_eq!(err(), GL_NO_ERROR);
    px
}

/// A framebuffer (bound) and its renderbuffers, deleted on drop.
struct Fb {
    fb: GLuint,
    rbs: Vec<GLuint>,
}

impl Drop for Fb {
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

impl Fb {
    fn new() -> Fb {
        let mut fb = 0;
        // SAFETY: room for one name; plain GL calls.
        unsafe {
            glGenFramebuffers(1, &mut fb);
            glBindFramebuffer(GL_FRAMEBUFFER, fb);
        }
        Fb { fb, rbs: Vec::new() }
    }

    /// A `w` x `h` renderbuffer of `format` with `samples`, attached at `at`.
    fn rb(&mut self, at: GLenum, samples: GLsizei, format: GLenum, w: GLsizei, h: GLsizei) {
        let mut rb = 0;
        // SAFETY: plain GL calls.
        unsafe {
            glGenRenderbuffers(1, &mut rb);
            glBindRenderbuffer(GL_RENDERBUFFER, rb);
            glRenderbufferStorageMultisample(GL_RENDERBUFFER, samples, format, w, h);
            glFramebufferRenderbuffer(GL_FRAMEBUFFER, at, GL_RENDERBUFFER, rb);
        }
        assert_eq!(err(), GL_NO_ERROR, "{w}x{h} {format:#x} at {at:#x}");
        self.rbs.push(rb);
    }
}

fn status() -> GLenum {
    // SAFETY: plain GL call.
    unsafe { glCheckFramebufferStatus(GL_FRAMEBUFFER) }
}

const C0: GLenum = GL_COLOR_ATTACHMENT0;
const C1: GLenum = GL_COLOR_ATTACHMENT1;

/// 4x colour of 64x64 on attachment 0 and 32x48 on attachment 1, drawn; the
/// framebuffer is left bound. `first` is attachment 0's size, `second`
/// attachment 1's.
fn two_colours(first: (GLsizei, GLsizei), second: (GLsizei, GLsizei)) -> (Fb, Seen) {
    let mut f = Fb::new();
    f.rb(C0, 4, GL_RGBA8, first.0, first.1);
    f.rb(C1, 4, GL_RGBA8, second.0, second.1);
    assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE, "sizes are no completeness rule in ES 3.0");
    program();
    let s = draw();
    (f, s)
}

/// A 4x 64x64 + 32x48 framebuffer drawn, its job submitted but not finished.
fn drawn_mixed() -> Fb {
    let mut f = Fb::new();
    f.rb(C0, 4, GL_RGBA8, 64, 64);
    f.rb(C1, 4, GL_RGBA8, 32, 48);
    program();
    draw_unfinished();
    assert_eq!(err(), GL_NO_ERROR);
    assert_eq!(PENDING.lock().unwrap().len(), 1, "the 4x draw's job is pending");
    f
}

fn count_in(px: &[[u8; 4]], w: usize, x1: usize, y1: usize) -> (usize, usize) {
    let mut inside = 0;
    let mut outside = 0;
    for (i, p) in px.iter().enumerate() {
        if *p == [0; 4] {
            continue;
        }
        if i % w < x1 && i / w < y1 {
            inside += 1;
        } else {
            outside += 1;
        }
    }
    (inside, outside)
}

/* ---- two colour attachments of different sizes -------------------------- */

/// ES 3.0 4.4.4 has no size rule: 4x colours of 64x64 and 32x48 are complete,
/// and the draw reaches the back end at 4 samples with both targets.
#[test]
fn four_x_colours_of_different_sizes_are_complete() {
    let _g = context_ms4();
    let (_f, s) = two_colours((64, 64), (32, 48));
    assert_eq!(s.samples, 4);
    assert_eq!(s.targets, vec![(64, 64), (32, 48)], "each target keeps its own size");
}

/// Today `GlesDraw::target_w/h` are the frame attachment's own size (here
/// colour 0, 64x64); nothing takes the minimum over the attachments.
#[test]
fn draw_extent_is_the_frame_attachments_own_size_today() {
    let _g = context_ms4();
    let (_f, s) = two_colours((64, 64), (32, 48));
    assert_eq!(s.target, (64, 64));
    /* the smaller one first: the extent is the smallest only by position */
    let (_f2, s2) = two_colours((32, 48), (64, 64));
    assert_eq!(s2.target, (32, 48));
    assert_eq!(s2.targets, vec![(32, 48), (64, 64)]);
}

/// ES 3.0: the framebuffer is the smallest attachment, and a draw reports
/// that extent. GL-core cause: src/gl/framebuffer.rs (`frame_attachment` /
/// `fb_target_pitched_n` give one attachment's size, no minimum over them),
/// read by src/gl/draw.rs into `GlesDraw::target_w/h`.
#[test]
#[ignore = "GL core, src/gl/framebuffer.rs: the draw extent is colour 0's size, not the smallest attachment"]
fn draw_extent_is_the_smallest_attachment() {
    let _g = context_ms4();
    let (_f, s) = two_colours((64, 64), (32, 48));
    assert_eq!(s.target, (32, 48));
    assert_eq!(s.targets, vec![(32, 48), (32, 48)]);
}

/* ---- a 4x depth attachment smaller than the colour ----------------------- */

/// 4x colour of 64x64 with a 4x depth of 40x40 is complete; the back end is
/// given the colour's 64x64 and the depth's own 40x40.
#[test]
fn four_x_colour_with_a_smaller_four_x_depth() {
    let _g = context_ms4();
    let mut f = Fb::new();
    f.rb(C0, 4, GL_RGBA8, 64, 64);
    f.rb(GL_DEPTH_ATTACHMENT, 4, GL_DEPTH_COMPONENT16, 40, 40);
    assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE);
    program();
    let s = draw();
    assert_eq!(s.samples, 4);
    assert_eq!(s.target, (64, 64));
    assert_eq!(s.ds, (40, 40), "the depth plane keeps its own size");
}

/// The same with a packed 4x depth/stencil of 40x40.
#[test]
fn four_x_colour_with_a_smaller_four_x_depth_stencil() {
    let _g = context_ms4();
    let mut f = Fb::new();
    f.rb(C0, 4, GL_RGBA8, 64, 64);
    f.rb(GL_DEPTH_STENCIL_ATTACHMENT, 4, GL_DEPTH24_STENCIL8, 40, 40);
    assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE);
    program();
    let s = draw();
    assert_eq!((s.samples, s.target, s.ds), (4, (64, 64), (40, 40)));
}

/// ES 3.0: with a 40x40 depth the framebuffer is 40x40 and the draw says so.
/// GL-core cause: src/gl/framebuffer.rs (no minimum over the colour and
/// depth sizes; `GlesDraw::target_w/h` come from colour 0 alone).
#[test]
#[ignore = "GL core, src/gl/framebuffer.rs: the draw extent ignores a smaller depth attachment"]
fn draw_extent_with_a_smaller_depth_is_the_depth_size() {
    let _g = context_ms4();
    let mut f = Fb::new();
    f.rb(C0, 4, GL_RGBA8, 64, 64);
    f.rb(GL_DEPTH_ATTACHMENT, 4, GL_DEPTH_COMPONENT16, 40, 40);
    program();
    let s = draw();
    assert_eq!(s.target, (40, 40));
}

/* ---- the resolve blit ---------------------------------------------------- */

/// A resolve of the full 64x64 into a 1x 64x64 target: source and
/// destination rectangles match, so there is no error (ES 3.0 4.3.3: a
/// multisample resolve needs equal rectangles; clipping to the read
/// framebuffer comes after the rule).
#[test]
fn full_rectangle_resolve_into_a_64x64_target_is_accepted() {
    let _g = context_ms4();
    let src = drawn_mixed();
    let mut dst = Fb::new();
    dst.rb(C0, 0, GL_RGBA8, 64, 64);
    assert_eq!(blit(src.fb, dst.fb, [0, 0, 64, 64], [0, 0, 64, 64]), GL_NO_ERROR);
}

/// Today the resolve copies colour 0's whole 64x64: pixels outside the
/// smallest extent (32x48) are written too.
#[test]
fn full_resolve_copies_the_whole_colour_attachment_today() {
    let _g = context_ms4();
    let src = drawn_mixed();
    let mut dst = Fb::new();
    dst.rb(C0, 0, GL_RGBA8, 64, 64);
    assert_eq!(blit(src.fb, dst.fb, [0, 0, 64, 64], [0, 0, 64, 64]), GL_NO_ERROR);
    let px = pixels(dst.fb, 64, 64);
    assert_eq!(count_in(&px, 64, 32, 48), (32 * 48, 64 * 64 - 32 * 48));
}

/// ES 3.0: the read framebuffer is the smallest attachment (32x48); the
/// source rectangle is clipped to it and nothing outside it is written.
/// GL-core cause: src/gl/blit.rs clips to colour 0's size, and
/// src/gl/framebuffer.rs gives no minimum extent for it to clip to.
#[test]
#[ignore = "GL core, src/gl/blit.rs: the resolve clips to colour 0's size, not the smallest attachment"]
fn full_resolve_stays_inside_the_smallest_extent() {
    let _g = context_ms4();
    let src = drawn_mixed();
    let mut dst = Fb::new();
    dst.rb(C0, 0, GL_RGBA8, 64, 64);
    assert_eq!(blit(src.fb, dst.fb, [0, 0, 64, 64], [0, 0, 64, 64]), GL_NO_ERROR);
    let px = pixels(dst.fb, 64, 64);
    assert_eq!(count_in(&px, 64, 32, 48), (32 * 48, 0));
}

/// A resolve of exactly the smallest extent copies exactly it.
#[test]
fn resolve_of_the_smallest_extent_copies_it() {
    let _g = context_ms4();
    let src = drawn_mixed();
    let mut dst = Fb::new();
    dst.rb(C0, 0, GL_RGBA8, 64, 64);
    assert_eq!(blit(src.fb, dst.fb, [0, 0, 32, 48], [0, 0, 32, 48]), GL_NO_ERROR);
    let px = pixels(dst.fb, 64, 64);
    assert_eq!(count_in(&px, 64, 32, 48), (32 * 48, 0));
}

/// The multisample resolve rule holds whatever the extent: a destination
/// of the smallest extent for a full-size source is GL_INVALID_OPERATION,
/// and so is the reverse; nothing is copied.
#[test]
fn unequal_rectangles_are_refused_whatever_the_extent() {
    let _g = context_ms4();
    let src = drawn_mixed();
    let mut dst = Fb::new();
    dst.rb(C0, 0, GL_RGBA8, 64, 64);
    assert_eq!(blit(src.fb, dst.fb, [0, 0, 64, 64], [0, 0, 32, 48]), GL_INVALID_OPERATION);
    assert_eq!(blit(src.fb, dst.fb, [0, 0, 32, 48], [0, 0, 64, 64]), GL_INVALID_OPERATION);
    assert_eq!(blit(src.fb, dst.fb, [0, 0, 32, 48], [32, 16, 64, 64]), GL_INVALID_OPERATION, "moved");
    assert_eq!(PENDING.lock().unwrap().len(), 1, "a refused blit waits for nothing");
    assert!(pixels(dst.fb, 64, 64).iter().all(|p| *p == [0; 4]), "no copy");
}
