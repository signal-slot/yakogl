// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Four colour attachments of different sizes on the PowerVR host path
//! (pvr-sizes4-test, plan #368), through the GL API and the real GL core,
//! with `backend::HOST_DRAW_TARGETS` standing in four draw targets (the
//! pattern of tests/es3_msaa_pvr.rs).
//!
//! ES 3.0 4.4.4.2 allows attachments of different sizes and 4.4.1 renders
//! their intersection (tests/es3_mrt_sizes_v3d.rs is the V3D rule; the
//! PowerVR clips to each target's own size, `GlesDrawTarget::w`/`h`). The
//! real PowerVR draw needs the board's job memory, so the back end here is a
//! stand-in that rasterises nothing but the contract: it is handed a draw,
//! takes each target's own size from `targets[k].w/h`, and writes the
//! fragment colour of output `k` (or the clear colour of a clear) into
//! target `k` over the intersection of the written targets' sizes, at the
//! GL core's own pitch (`tex_pitch`). What the file therefore pins is what
//! the GL core hands the PowerVR back end for four attachments of different
//! sizes, and what the application then reads back:
//!
//! - glCheckFramebufferStatus is COMPLETE, whichever attachment is the
//!   smallest and whichever the largest;
//! - the draw carries four targets, each with its own size, the frame's
//!   target being attachment 0's;
//! - a draw and a glClear (and glClearBufferfv of attachment 3 alone) write
//!   each attachment inside the intersection only; the pixels of attachment
//!   3 outside it, read back, keep the values the texture was uploaded with.
//!
//! The real PowerVR back end's own clip (`pbe_reg_words` per target size in
//! `src/pvr/backend.rs`) is not run here; no defect shows in what the GL core
//! hands it, so nothing is `#[ignore]`d.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test es3_mrt4_sizes_pvr
//! ```

#![cfg(all(feature = "host-test", feature = "shaderc-online", feature = "gpu-powervr", not(feature = "report-es2")))]

use core::ffi::{c_char, c_void};
use std::ffi::CString;
use std::sync::atomic::Ordering;
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{self, GlesDraw, GlesHeap, YakoglBackend, *};
use yakogl::gl::buffer::glBindBuffer;
use yakogl::gl::consts::*;
use yakogl::gl::context::tex_pitch;
use yakogl::gl::draw::*;
use yakogl::gl::es3::glClearBufferfv;
use yakogl::gl::framebuffer::*;
use yakogl::gl::program::*;
use yakogl::gl::shader::*;
use yakogl::gl::state::*;
use yakogl::gl::texture::*;
use yakogl::gl::types::*;
use yakogl::gl::vertex::{glEnableVertexAttribArray, glVertexAttribPointer};
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};

/* ---- the stand-in back end: the contract, rasterised ---------------------- */

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
    do_clear: bool,
    /// `(target_w, target_h)`, the frame's own target.
    frame: (u32, u32),
    /// `targets[..num_targets]`: (w, h, format).
    targets: Vec<(u32, u32, u8)>,
}

static SEEN: Mutex<Vec<Seen>> = Mutex::new(Vec::new());

/// The colour output `k` of the test program writes, as bytes.
const OUT: [[u8; 4]; 4] = [[255, 0, 0, 255], [0, 255, 0, 255], [0, 0, 255, 255], [255, 255, 0, 255]];
/// The clear colour of the tests, as bytes.
const CLEAR: [u8; 4] = [0, 255, 255, 255];
/// What every attachment is uploaded with: the "old value".
const OLD: [u8; 4] = [0x11, 0x22, 0x33, 0x44];

unsafe extern "C" fn b_draw(d: *const GlesDraw) -> i32 {
    let d = &*d;
    let n = (d.num_targets.max(1) as usize).min(GLES_MAX_TARGETS);
    let ts = &d.targets[..n];
    SEEN.lock().unwrap().push(Seen {
        draws: d.index_count != 0,
        do_clear: d.do_clear,
        frame: (d.target_w, d.target_h),
        targets: ts.iter().map(|t| (t.w, t.h, t.format)).collect(),
    });
    /* the render area: the intersection of the written targets */
    let live: Vec<_> = ts.iter().filter(|t| t.gpu != 0).collect();
    let aw = live.iter().map(|t| t.w).min().unwrap_or(0);
    let ah = live.iter().map(|t| t.h).min().unwrap_or(0);
    for (k, t) in ts.iter().enumerate() {
        if t.gpu == 0 || t.format != GLES_SURF_RGBA8888 {
            continue;
        }
        let cleared = d.do_clear && (d.clear_target_mask == 0 || (d.clear_target_mask >> k) & 1 != 0);
        let clear = cleared.then(|| d.clear_rgba.map(|v| (v * 255.0 + 0.5) as u8));
        let drawn = (d.index_count != 0).then_some(OUT[k]);
        /* a clear is fused ahead of the draw it rides on */
        for c in [clear, drawn].into_iter().flatten() {
            let pitch = tex_pitch(t.w) as usize;
            for y in 0..ah as usize {
                for x in 0..aw as usize {
                    // SAFETY: the target is `t.w` x `t.h` (pitch `tex_pitch(w)`) RGBA8 of host memory.
                    unsafe {
                        core::ptr::copy_nonoverlapping(c.as_ptr(), (t.gpu as *mut u8).add((y * pitch + x) * 4), 4)
                    };
                }
            }
        }
    }
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
    name: c"es3-mrt4-sizes-pvr".as_ptr() as *const c_char,
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

/* ---- the framebuffer, the program, the draw ---------------------------- */

/// A framebuffer of four RGBA8 textures, `sizes[k]` on COLOR_ATTACHMENTk,
/// each uploaded with `OLD`; all four are draw buffers.
struct Fbo {
    fb: GLuint,
    texs: Vec<GLuint>,
    sizes: [(u32, u32); 4],
}

impl Fbo {
    fn new(sizes: [(u32, u32); 4]) -> Fbo {
        let mut fb = 0;
        let mut texs = vec![0; 4];
        // SAFETY: room for one framebuffer name and four texture names.
        unsafe {
            glGenFramebuffers(1, &mut fb);
            glBindFramebuffer(GL_FRAMEBUFFER, fb);
            glGenTextures(4, texs.as_mut_ptr());
        }
        for (k, &(w, h)) in sizes.iter().enumerate() {
            let old: Vec<u8> = OLD.iter().copied().cycle().take((w * h * 4) as usize).collect();
            // SAFETY: `old` holds w x h RGBA8 pixels; names this test made.
            unsafe {
                glBindTexture(GL_TEXTURE_2D, texs[k]);
                glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST as GLint);
                glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST as GLint);
                glTexImage2D(
                    GL_TEXTURE_2D,
                    0,
                    GL_RGBA8 as GLint,
                    w as GLsizei,
                    h as GLsizei,
                    0,
                    GL_RGBA,
                    GL_UNSIGNED_BYTE,
                    old.as_ptr() as *const c_void,
                );
                glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0 + k as GLenum, GL_TEXTURE_2D, texs[k], 0);
            }
        }
        let bufs: [GLenum; 4] = core::array::from_fn(|k| GL_COLOR_ATTACHMENT0 + k as GLenum);
        let fbo = Fbo { fb, texs, sizes };
        let (aw, ah) = fbo.area();
        // SAFETY: four enums.
        unsafe {
            glDrawBuffers(4, bufs.as_ptr());
            glViewport(0, 0, aw as GLsizei, ah as GLsizei);
        }
        assert_eq!(err(), GL_NO_ERROR, "{sizes:?}");
        fbo
    }

    /// The intersection of the four sizes.
    fn area(&self) -> (u32, u32) {
        (self.sizes.iter().map(|s| s.0).min().unwrap(), self.sizes.iter().map(|s| s.1).min().unwrap())
    }

    fn status(&self) -> GLenum {
        // SAFETY: plain GL call.
        unsafe { glCheckFramebufferStatus(GL_FRAMEBUFFER) }
    }

    /// The RGBA8 pixels of attachment `k`, row-major from the bottom left,
    /// over its whole size.
    fn pixels(&self, k: usize) -> Vec<[u8; 4]> {
        let (w, h) = self.sizes[k];
        let mut v = vec![[0u8; 4]; (w * h) as usize];
        // SAFETY: room for w x h RGBA8 pixels; a framebuffer this test made.
        unsafe {
            glBindFramebuffer(GL_READ_FRAMEBUFFER, self.fb);
            glReadBuffer(GL_COLOR_ATTACHMENT0 + k as GLenum);
            glReadPixels(0, 0, w as GLsizei, h as GLsizei, GL_RGBA, GL_UNSIGNED_BYTE, v.as_mut_ptr() as *mut c_void);
        }
        assert_eq!(err(), GL_NO_ERROR, "read-back of attachment {k}");
        v
    }

    /// Attachment `k` holds `inside` at the pixels of the intersection and
    /// `OLD` at every other pixel of its own size.
    fn assert_area(&self, what: &str, k: usize, inside: [u8; 4]) {
        let (w, _) = self.sizes[k];
        let (aw, ah) = self.area();
        for (i, p) in self.pixels(k).iter().enumerate() {
            let (x, y) = (i as u32 % w, i as u32 / w);
            let want = if x < aw && y < ah { inside } else { OLD };
            assert_eq!(*p, want, "{what}: attachment {k} pixel ({x},{y}), sizes {:?}", self.sizes);
        }
    }
}

impl Drop for Fbo {
    fn drop(&mut self) {
        // SAFETY: names this test made.
        unsafe {
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
            glDeleteFramebuffers(1, &self.fb);
            glDeleteTextures(4, self.texs.as_ptr());
            glUseProgram(0);
        }
    }
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

/// The current program: four `vec4` outputs, `OUT[k]` at location k.
fn program() {
    let mut fs = String::from("#version 300 es\nprecision highp float;\n");
    for k in 0..4 {
        fs += &format!("layout(location = {k}) out vec4 o{k};\n");
    }
    fs += "void main() {\n";
    for (k, c) in OUT.iter().enumerate() {
        let f = |b: u8| if b == 255 { "1.0" } else { "0.0" };
        fs += &format!("o{k} = vec4({}, {}, {}, {});\n", f(c[0]), f(c[1]), f(c[2]), f(c[3]));
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

/// One oversized triangle over the whole area; the draws the back end saw.
fn draw() -> Vec<Seen> {
    static FULL: [f32; 12] = [-1.0, -1.0, 0.0, 1.0, 3.0, -1.0, 0.0, 1.0, -1.0, 3.0, 0.0, 1.0];
    SEEN.lock().unwrap().clear();
    // SAFETY: `FULL` is static and outlives the draw.
    unsafe {
        glBindBuffer(GL_ARRAY_BUFFER, 0);
        glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, FULL.as_ptr() as *const c_void);
        glEnableVertexAttribArray(0);
        glDrawArrays(GL_TRIANGLES, 0, 3);
        glFinish();
    }
    assert_eq!(err(), GL_NO_ERROR);
    core::mem::take(&mut *SEEN.lock().unwrap())
}

fn clear() -> Vec<Seen> {
    SEEN.lock().unwrap().clear();
    let c = CLEAR.map(|b| b as f32 / 255.0);
    // SAFETY: plain GL calls.
    unsafe {
        glClearColor(c[0], c[1], c[2], c[3]);
        glClear(GL_COLOR_BUFFER_BIT);
        glFinish();
    }
    assert_eq!(err(), GL_NO_ERROR);
    core::mem::take(&mut *SEEN.lock().unwrap())
}

/* ---- the cases ------------------------------------------------------------ */

/// Attachment 3 the smallest: its 12x10 is the intersection.
const SMALLEST_LAST: [(u32, u32); 4] = [(32, 32), (24, 40), (28, 20), (12, 10)];
/// Attachment 3 the largest; attachment 0 the smallest (12x10 again).
const LARGEST_LAST: [(u32, u32); 4] = [(12, 10), (16, 20), (20, 14), (40, 36)];
/// Attachment 3 neither: the intersection takes its width from 1 and its
/// height from 2.
const CROSSED: [(u32, u32); 4] = [(30, 30), (14, 40), (36, 9), (20, 25)];

const CASES: [[(u32, u32); 4]; 3] = [SMALLEST_LAST, LARGEST_LAST, CROSSED];

#[test]
fn four_attachments_of_different_sizes_are_complete() {
    let _g = context();
    for sizes in CASES {
        let fbo = Fbo::new(sizes);
        assert_eq!(fbo.status(), GL_FRAMEBUFFER_COMPLETE, "{sizes:?}");
    }
}

/// The draw reaches the back end with four targets, each its own size, the
/// frame's target attachment 0's.
#[test]
fn the_draw_hands_each_target_its_own_size() {
    let _g = context();
    for sizes in CASES {
        let fbo = Fbo::new(sizes);
        program();
        let seen = draw();
        let d = seen.iter().rev().find(|s| s.draws).expect("the draw reached the back end");
        let got: Vec<(u32, u32)> = d.targets.iter().map(|t| (t.0, t.1)).collect();
        assert_eq!(got, sizes, "per-target sizes");
        assert_eq!(d.frame, sizes[0], "the frame is attachment 0's");
        assert!(d.targets.iter().all(|t| t.2 == GLES_SURF_RGBA8888));
        drop(fbo);
    }
}

#[test]
fn a_draw_writes_each_attachment_inside_the_intersection_only() {
    let _g = context();
    for sizes in CASES {
        let fbo = Fbo::new(sizes);
        program();
        draw();
        assert_eq!(fbo.status(), GL_FRAMEBUFFER_COMPLETE);
        for (k, out) in OUT.iter().enumerate() {
            fbo.assert_area("draw", k, *out);
        }
    }
}

#[test]
fn a_clear_writes_each_attachment_inside_the_intersection_only() {
    let _g = context();
    for sizes in CASES {
        let fbo = Fbo::new(sizes);
        let seen = clear();
        let d = seen.iter().find(|s| s.do_clear).expect("the clear reached the back end");
        assert_eq!(d.targets.len(), 4, "a clear of all four draw buffers");
        for k in 0..4 {
            fbo.assert_area("glClear", k, CLEAR);
        }
    }
}

/// Clear, then draw over it: the draw's colours replace the clear inside
/// the area, and attachment 3's pixels outside it are still the upload's.
#[test]
fn a_clear_then_a_draw_leave_the_outside_of_attachment_3_alone() {
    let _g = context();
    for sizes in CASES {
        let fbo = Fbo::new(sizes);
        program();
        clear();
        draw();
        for (k, out) in OUT.iter().enumerate() {
            fbo.assert_area("clear then draw", k, *out);
        }
    }
}

/// glClearBufferfv(GL_COLOR, 3): attachment 3 only; the others are
/// untouched everywhere, attachment 3 is cleared inside the area.
#[test]
fn a_clear_buffer_of_attachment_3_writes_only_it_and_only_inside() {
    let _g = context();
    for sizes in CASES {
        let fbo = Fbo::new(sizes);
        let c = CLEAR.map(|b| b as f32 / 255.0);
        SEEN.lock().unwrap().clear();
        // SAFETY: four floats; plain GL calls.
        unsafe {
            glClearBufferfv(GL_COLOR, 3, c.as_ptr());
            glFinish();
        }
        assert_eq!(err(), GL_NO_ERROR);
        fbo.assert_area("glClearBufferfv 3", 3, CLEAR);
        for k in 0..3 {
            assert!(fbo.pixels(k).iter().all(|p| *p == OLD), "attachment {k} untouched, sizes {sizes:?}");
        }
    }
}
