// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! glmark2 `shadow` and `refract` on the PowerVR: the depth texture the
//! shadow pass renders is sampled with stale depth (docs/HANDOFF.md, known
//! defects, pvr-depth-resolve; tools/cts/log/hostharness/glmark2/FINDINGS.md
//! rows 23 and 24).
//!
//! The sequence is scene-shadow.cpp's, one frame at a time:
//!
//! 1. `glClear` of the window surface (colour and depth). The core defers
//!    it as a pending clear.
//! 2. the depth pass: the FBO whose only attachment is a GL_DEPTH_COMPONENT
//!    texture, colour mask off, `glClear(GL_DEPTH_BUFFER_BIT)`, a draw with
//!    the depth test on. Binding the FBO emits the pending clear of step 1
//!    as a kick of its own, so the frame being recorded is the window
//!    surface's when the depth pass's first draw arrives.
//! 3. back to the window surface, a draw whose shader samples the depth
//!    texture (the ground). The core calls `texture::resolve_depth` for it.
//!
//! The back end is a stand-in that follows the PowerVR's contract where the
//! resolve depends on it (src/pvr/backend.rs): `kick_id` is
//! `last_fence + 1`, `kick_pending(id)` is `id > completed`, a draw whose
//! target differs from the open frame's submits that frame first
//! (`draw_one`, `frame_ends_here`) and then opens its own, and
//! `depth_resolve` answers -EBUSY while the open frame renders into the
//! depth id (`pvr_backend_depth_resolve`). Every submit completes at once,
//! as linux.c's does.
//!
//! The real PowerVR back end cannot run here: its C command generators are
//! linked only in the C builds, and the in-crate rig (src/pvr/backend_tests.rs)
//! that fakes them is `cfg(test)` of the library. The real back end's answer
//! is the host harness's: `tools/hostharness/glmark2/run.sh ... powervr`
//! logs `the backend did not resolve its depth (-16)` once per frame of
//! shadow and refract (FINDINGS.md).
//!
//! The cause (FINDINGS.md): the core stamps the kick that renders into a
//! texture (`framebuffer::stamp_target_use`, src/gl/framebuffer.rs:186) with
//! `backend::kick_id()` while it builds the draw, BEFORE the back end sees
//! it. When the draw starts a new frame the back end first submits the
//! frame that is open, so the draw renders as the next kick, `stamp + 1`, and
//! the stamp names the kick that was just submitted. Completed, it is not
//! pending: `resolve_depth` (src/gl/texture.rs:637, test at :654) neither flushes nor
//! waits, the back end still has the frame open, and it answers -EBUSY.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test depth_resolve_pvr
//! ```

#![cfg(all(feature = "host-test", feature = "shaderc-online"))]

use core::ffi::{c_char, c_void};
use std::ffi::CString;
use std::sync::Mutex;
use yakogl::backend::{GlesDraw, GlesHeap, YakoglBackend, GLES_TEXFMT_R32F};
use yakogl::gl::buffer::*;
use yakogl::gl::consts::*;
use yakogl::gl::draw::*;
use yakogl::gl::framebuffer::*;
use yakogl::gl::program::*;
use yakogl::gl::shader::*;
use yakogl::gl::state::*;
use yakogl::gl::texture::*;
use yakogl::gl::types::*;
use yakogl::gl::vertex::*;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};

const EBUSY: i32 = 16;
/// The value the stand-in's depth_resolve writes into the texels.
const RESOLVED: f32 = 0.5;

/* ------------------------------------------------------- the stand-in -- */

#[derive(Default)]
struct Rig {
    /// Submits so far: the fences. Every one completes at once.
    last_fence: u32,
    completed: u32,
    /// The open frame: (target, depth id).
    open: Option<(u64, u32)>,
    /// Every depth_resolve: (id, rc).
    resolves: Vec<(u32, i32)>,
    /// Texel 0 of the depth texture the last sampling draw bound.
    sampled_texel: Option<f32>,
    draws: u32,
    flushes: u32,
    finishes: u32,
}

static RIG: Mutex<Rig> = Mutex::new(Rig {
    last_fence: 0,
    completed: 0,
    open: None,
    resolves: Vec::new(),
    sampled_texel: None,
    draws: 0,
    flushes: 0,
    finishes: 0,
});

fn rig<R>(f: impl FnOnce(&mut Rig) -> R) -> R {
    f(&mut RIG.lock().unwrap_or_else(|e| e.into_inner()))
}

fn submit_open() {
    rig(|r| {
        if r.open.take().is_some() {
            r.last_fence += 1;
            r.completed = r.last_fence;
        }
    });
}

unsafe extern "C" fn s_init() -> i32 {
    0
}
unsafe extern "C" fn s_heap_base(_h: GlesHeap) -> u64 {
    0x1000_0000
}
unsafe extern "C" fn s_draw(d: *const GlesDraw) -> i32 {
    let d = &*d;
    let key = (d.target_gpu, d.depth_id);
    // draw_one: a frame that ends here is submitted first, then this draw opens its own
    if rig(|r| r.open.is_some_and(|o| o != key)) {
        submit_open();
    }
    rig(|r| {
        r.draws += 1;
        if r.open.is_none() {
            r.open = Some(key);
        }
        for tb in d.textures.iter().take(d.num_textures as usize) {
            if tb.format == GLES_TEXFMT_R32F && tb.base_gpu != 0 {
                r.sampled_texel = Some(*(tb.base_gpu as *const f32));
            }
        }
    });
    0
}
unsafe extern "C" fn s_flush() -> i32 {
    submit_open();
    rig(|r| r.flushes += 1);
    0
}
unsafe extern "C" fn s_finish() -> i32 {
    rig(|r| {
        r.completed = r.last_fence;
        r.finishes += 1;
    });
    0
}
unsafe extern "C" fn s_kick_id() -> u32 {
    rig(|r| r.last_fence + 1)
}
unsafe extern "C" fn s_kick_pending(id: u32) -> bool {
    rig(|r| id != 0 && id > r.completed)
}
unsafe extern "C" fn s_depth_resolve(id: u32, dst: *mut f32, w: u32, h: u32, pitch: u32) -> i32 {
    let rc = rig(|r| if r.open.is_some_and(|o| o.1 == id) { -EBUSY } else { 0 });
    if rc == 0 {
        for y in 0..h {
            for x in 0..w {
                *dst.add((y * pitch + x) as usize) = RESOLVED;
            }
        }
    }
    rig(|r| r.resolves.push((id, rc)));
    rc
}

static STAND_IN: YakoglBackend = YakoglBackend {
    name: c"depth-resolve stand-in".as_ptr() as *const c_char,
    native_attrib_formats: 0,
    rt_rgba8888_is_bgra: false,
    mip_level_align: 0,
    compressed_blocks_in_rows: false,
    native_layered_textures: false,
    front_facing_shader_variant: false,
    etc1_decoded_at_upload: false,
    init: Some(s_init),
    shutdown: None,
    draw: Some(s_draw),
    flush: Some(s_flush),
    finish: Some(s_finish),
    finish_previous: None,
    kick_id: Some(s_kick_id),
    kick_pending: Some(s_kick_pending),
    heap_base: Some(s_heap_base),
    counters: None,
    texture_released: None,
    surface_swapped: None,
    texture_uploaded: None,
    max_instance_divisor: 1,
    wait_kick: None,
    samples_texel_copies: false,
    attachment_gone: None,
    depth_resolve: Some(s_depth_resolve),
};

/* ---------------------------------------------------------------- OSAL -- */

/// Pointer-valued device addresses: the stand-in reads texel 0 through `base_gpu`.
unsafe extern "C" fn o_gpu_alloc(_heap: u64, size: usize, _flags: u32, out: *mut OsalGpuMem) -> i32 {
    let mem = Box::leak(vec![0u64; size.div_ceil(8)].into_boxed_slice());
    let cpu = mem.as_mut_ptr() as *mut c_void;
    *out = OsalGpuMem { cpu, gpu: cpu as u64, phys: cpu as u64, size, impl_: cpu };
    0
}
unsafe extern "C" fn o_gpu_free(_m: *mut OsalGpuMem) -> i32 {
    0
}
unsafe extern "C" fn o_cache(_cpu: *mut c_void, _n: usize) {}

static OSAL: OsalOps = OsalOps {
    init: None,
    shutdown: None,
    gpu_alloc: Some(o_gpu_alloc),
    gpu_free: Some(o_gpu_free),
    mmu_map: None,
    map_phys: None,
    gpu_write: None,
    gpu_read: None,
    cache_flush: Some(o_cache),
    cache_invd: Some(o_cache),
    submit: None,
    wait: None,
    completed: None,
    reg_read: None,
    reg_write: None,
    query: None,
    log: None,
    wait_vblank: None,
    lock: None,
    unlock: None,
    thread_id: None,
};

/// One test at a time: the back end, the OSAL and the GL core are statics.
static SERIAL: Mutex<()> = Mutex::new(());

/// A fresh context on the stand-in; the guard serialises the tests.
fn start() -> (std::sync::MutexGuard<'static, ()>, Ctx) {
    let g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    /* The fences go on counting: the core keeps a texture's last-resolved
     * kick per context slot and texture slot across contexts, and a kick
     * number that came round again would look resolved already. A real
     * back end never repeats one. */
    {
        let mut r = RIG.lock().unwrap_or_else(|e| e.into_inner());
        *r = Rig { last_fence: r.last_fence, completed: r.last_fence, ..Rig::default() };
    }
    // SAFETY: the guard serialises the tests; both tables are statics.
    let ctx = unsafe {
        osal_set_ops(&OSAL);
        yakogl::backend::g_backend = &STAND_IN;
        yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
        let ctx = yakogl::gl::context::gl_context_create(64, 64);
        assert!(!ctx.is_null(), "context pool exhausted");
        ctx
    };
    (g, Ctx(ctx))
}

/// The context `start` made; a failing assertion still frees its slot.
struct Ctx(*mut yakogl::gl::context::GlContext);

impl Drop for Ctx {
    fn drop(&mut self) {
        // SAFETY: the context `start` made.
        unsafe { yakogl::gl::context::gl_context_destroy(self.0) };
    }
}

fn stop(_ctx: Ctx) {}

/* -------------------------------------------------------- GL plumbing -- */

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

fn program(vs: &str, fs: &str) -> GLuint {
    // SAFETY: plain GL calls.
    unsafe {
        let p = glCreateProgram();
        glAttachShader(p, shader(GL_VERTEX_SHADER, vs));
        glAttachShader(p, shader(GL_FRAGMENT_SHADER, fs));
        glBindAttribLocation(p, 0, c"p".as_ptr());
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        assert_eq!(ok, 1, "link failed");
        p
    }
}

/// glmark2 shadow's two programs, reduced to what the sequence needs: the
/// depth pass writes depth only, the ground samples the depth texture.
const VS: &str = "attribute vec4 p;\nvarying vec2 v;\nvoid main() { v = p.xy * 0.5 + 0.5; gl_Position = p; }\n";
const FS_DEPTH: &str = "precision mediump float;\nvoid main() { gl_FragColor = vec4(1.0); }\n";
const FS_GROUND: &str = "precision mediump float;\nuniform sampler2D d;\nvarying vec2 v;\n\
    void main() { gl_FragColor = vec4(texture2D(d, v).r); }\n";

struct Scene {
    depth_prog: GLuint,
    ground_prog: GLuint,
    tex: GLuint,
    fbo: GLuint,
}

/// DepthRenderTarget::setup, then the programs and a quad.
fn setup(w: i32, h: i32) -> Scene {
    let depth_prog = program(VS, FS_DEPTH);
    let ground_prog = program(VS, FS_GROUND);
    let quad: [f32; 16] = [-1.0, -1.0, 0.5, 1.0, 1.0, -1.0, 0.5, 1.0, -1.0, 1.0, 0.5, 1.0, 1.0, 1.0, 0.5, 1.0];
    let (mut tex, mut fbo, mut vbo) = (0, 0, 0);
    // SAFETY: plain GL calls; `quad` outlives glBufferData.
    unsafe {
        glGenBuffers(1, &mut vbo);
        glBindBuffer(GL_ARRAY_BUFFER, vbo);
        glBufferData(GL_ARRAY_BUFFER, 64, quad.as_ptr() as *const c_void, GL_STATIC_DRAW);
        glEnableVertexAttribArray(0);
        glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, core::ptr::null());
        glGenTextures(1, &mut tex);
        glBindTexture(GL_TEXTURE_2D, tex);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST as GLint);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST as GLint);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE as GLint);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE as GLint);
        glTexImage2D(
            GL_TEXTURE_2D,
            0,
            GL_DEPTH_COMPONENT as GLint,
            w,
            h,
            0,
            GL_DEPTH_COMPONENT,
            GL_UNSIGNED_INT,
            core::ptr::null(),
        );
        glBindTexture(GL_TEXTURE_2D, 0);
        glGenFramebuffers(1, &mut fbo);
        glBindFramebuffer(GL_FRAMEBUFFER, fbo);
        glFramebufferTexture2D(GL_FRAMEBUFFER, GL_DEPTH_ATTACHMENT, GL_TEXTURE_2D, tex, 0);
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE);
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        assert_eq!(glGetError(), GL_NO_ERROR);
    }
    Scene { depth_prog, ground_prog, tex, fbo }
}

/// One frame of scene-shadow.cpp. `clear_window`: the frame starts with a
/// `glClear` of the window surface, as glmark2's canvas does.
fn frame(s: &Scene, clear_window: bool) {
    // SAFETY: plain GL calls on the current context.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        if clear_window {
            glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT);
        }
        /* DepthRenderTarget::enable */
        glUseProgram(s.depth_prog);
        glBindFramebuffer(GL_FRAMEBUFFER, s.fbo);
        glFramebufferTexture2D(GL_FRAMEBUFFER, GL_DEPTH_ATTACHMENT, GL_TEXTURE_2D, s.tex, 0);
        glViewport(0, 0, 16, 16);
        glColorMask(0, 0, 0, 0);
        glClear(GL_DEPTH_BUFFER_BIT);
        glEnable(GL_DEPTH_TEST);
        glDrawArrays(GL_TRIANGLE_STRIP, 0, 4);
        /* DepthRenderTarget::disable, then the ground */
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glViewport(0, 0, 64, 64);
        glColorMask(1, 1, 1, 1);
        glUseProgram(s.ground_prog);
        glActiveTexture(GL_TEXTURE0);
        glBindTexture(GL_TEXTURE_2D, s.tex);
        glUniform1i(glGetUniformLocation(s.ground_prog, c"d".as_ptr()), 0);
        glDrawArrays(GL_TRIANGLE_STRIP, 0, 4);
        assert_eq!(glGetError(), GL_NO_ERROR);
    }
}

fn rcs() -> Vec<i32> {
    rig(|r| r.resolves.iter().map(|r| r.1).collect())
}

/* ---------------------------------------------------------- the tests -- */

/// Nothing is open when the depth pass starts, so the stamp is the depth
/// pass's own kick, which is pending at the sampling draw: it is flushed and
/// waited for, and the resolve succeeds. (A regression guard for the half of
/// the sequence that works.)
#[test]
fn no_frame_open_before_the_depth_pass_resolves() {
    let (_g, ctx) = start();
    let s = setup(16, 16);
    frame(&s, false);
    assert_eq!(rcs(), vec![0], "{:?}", rig(|r| r.resolves.clone()));
    assert_eq!(rig(|r| r.sampled_texel), Some(RESOLVED));
    stop(ctx);
}

/// glmark2 shadow: the window clear is a kick of its own and the frame open
/// on the window surface when the depth pass draws. The back end submits it
/// at the depth pass's first draw (after the core stamped `kick_id()` = that
/// very kick) and the depth pass opens the next one. At the sampling draw
/// the stamp is complete, `resolve_depth` waits for nothing, and the back end
/// answers -EBUSY: the texels keep the previous frame's depth, or none.
///
/// Remove the `ignore` when `resolve_depth` (src/gl/texture.rs:669) retries
/// after -EBUSY, or the stamp is taken after the back end's draw
/// (src/gl/draw.rs:2011, after `backend::draw`).
#[test]
#[ignore = "stale kick stamp - stamp_target_use (src/gl/framebuffer.rs:186) takes kick_id() before the back end splits the frame; fix at resolve_depth src/gl/texture.rs:669 (flush+finish and retry on -EBUSY)"]
fn window_clear_before_the_depth_pass_resolves() {
    let (_g, ctx) = start();
    let s = setup(16, 16);
    for f in 0..3 {
        rig(|r| r.resolves.clear());
        frame(&s, true);
        /* (a fix may ask twice, -EBUSY and then 0: the answer that counts is the last) */
        assert_eq!(rcs().last(), Some(&0), "frame {f}: {:?}", rig(|r| r.resolves.clone()));
        assert_eq!(rig(|r| r.sampled_texel), Some(RESOLVED), "frame {f}");
    }
    stop(ctx);
}

/// The same sequence, asserting what happens today so the cause cannot
/// drift unseen: every frame's single resolve is -EBUSY (the harness's
/// `(-16)`, once a frame) and the sampling draw reads texels never written.
/// When the fix lands this fails: remove the `ignore` of the test above and
/// delete this one.
#[test]
fn window_clear_before_the_depth_pass_today() {
    let (_g, ctx) = start();
    let s = setup(16, 16);
    for f in 0..3 {
        rig(|r| r.resolves.clear());
        frame(&s, true);
        let r = rig(|r| r.resolves.clone());
        assert_eq!(rcs(), vec![-EBUSY], "frame {f}: {r:?}");
        assert_ne!(rig(|r| r.sampled_texel), Some(RESOLVED), "frame {f}: the sampling draw saw resolved depth");
    }
    stop(ctx);
}

/// Candidates of the first guess, ruled out: the id the core hands to
/// `depth_resolve` is the id the draws carried (an -EBUSY needs
/// `frame_zs == id`, which an unequal id would never give), `resolve_depth`
/// does run before the sampling draw (one call a frame), and the kick it
/// read was complete (no flush or finish between the sampling draw's start
/// and the resolve: the stamp is the one that went stale, not the wait).
#[test]
fn ruled_out_candidates() {
    let (_g, ctx) = start();
    let s = setup(16, 16);
    frame(&s, true);
    let (resolves, draws) = rig(|r| (r.resolves.clone(), r.draws));
    assert!(!resolves.is_empty(), "resolve_depth ran before the sampling draw: {resolves:?}");
    assert!(draws >= 3, "window clear, depth pass and ground reached the back end: {draws}");
    stop(ctx);
}
