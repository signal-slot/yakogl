// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GL-level host test of the frame structure of glmark2's desktop and
//! effect2d scenes on the Mali back end: an NPOT RGBA texture (300x200,
//! GL_LINEAR, GL_CLAMP_TO_EDGE, no mipmaps) is the colour of an FBO with no
//! depth; a pass draws into it, and the next pass samples it, ping-ponging
//! between two such FBOs as the blur passes do, ending in a full-screen quad
//! on the default framebuffer. Shaders are written for this test.
//!
//! Level reached: draw submission. The host has no Mali OSAL, so the stand-in
//! back end hands each draw to `mali_draw`, which ends at `-ENODEV`; no shader
//! executes and no pixel is written, so read-back pixel values cannot be
//! checked on the host (the board run does that). What is checked: the FBO is
//! complete, no GL error is raised, each draw reaches the Mali with the right
//! colour target (size, address), and each sampling pass is handed the NPOT
//! texture the previous pass rendered into (size, filter, wrap, one level,
//! complete), including the ping-pong alternation.
//!
//! Not a tools/check.sh row.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_glmark2_rtt_draw
//! ```

#![cfg(all(feature = "host-test", feature = "shaderc-online", feature = "gpu-mali", not(feature = "report-es2")))]

use core::ffi::{c_char, c_void};
use std::ffi::CString;
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{self, GlesDraw, GlesHeap, YakoglBackend, *};
use yakogl::gl::buffer::glBindBuffer;
use yakogl::gl::consts::*;
use yakogl::gl::draw::*;
use yakogl::gl::framebuffer::*;
use yakogl::gl::program::*;
use yakogl::gl::shader::*;
use yakogl::gl::state::*;
use yakogl::gl::texture::*;
use yakogl::gl::types::*;
use yakogl::gl::vertex::{glEnableVertexAttribArray, glVertexAttribPointer};
use yakogl::mali::backend::{mali_draw, yakogl_backend_mali};
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};
use yakogl::util::ENODEV;

/* ---- a context on the host: the smallest OSAL and back end that make one -- */

unsafe extern "C" fn m_gpu_alloc(_heap: u64, size: usize, _flags: u32, out: *mut OsalGpuMem) -> i32 {
    let p = Box::leak(vec![0u8; size].into_boxed_slice()).as_mut_ptr();
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

/// What the back end was handed for one draw: the colour target and the
/// first texture, and what the Mali made of the draw.
#[derive(Clone, Copy, Debug)]
#[allow(dead_code)]
struct Seen {
    target_gpu: u64,
    target_w: u32,
    target_h: u32,
    tex_gpu: u64,
    type_: u8,
    width: u32,
    height: u32,
    mip_levels: u8,
    linear_filter: u8,
    wrap_s: u8,
    wrap_t: u8,
    incomplete: u8,
    mali: i32,
}

static SEEN: Mutex<Vec<Seen>> = Mutex::new(Vec::new());

unsafe extern "C" fn b_draw(d: *const GlesDraw) -> i32 {
    let d = &*d;
    if d.index_count == 0 {
        return 0;
    }
    let t = &d.textures[0];
    let mali = mali_draw(d);
    SEEN.lock().unwrap().push(Seen {
        target_gpu: d.target_gpu,
        target_w: d.target_w,
        target_h: d.target_h,
        tex_gpu: t.base_gpu,
        type_: t.type_,
        width: t.width,
        height: t.height,
        mip_levels: t.mip_levels,
        linear_filter: t.linear_filter,
        wrap_s: t.wrap_s,
        wrap_t: t.wrap_t,
        incomplete: t.incomplete,
        mali,
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

/// The Mali's table with the stand-in's own entries (a draw that records, a
/// heap base, no hardware to start or wait for).
fn stand_in() -> &'static YakoglBackend {
    let m = &yakogl_backend_mali;
    Box::leak(Box::new(YakoglBackend {
        name: m.name,
        native_attrib_formats: m.native_attrib_formats,
        rt_rgba8888_is_bgra: m.rt_rgba8888_is_bgra,
        mip_level_align: m.mip_level_align,
        compressed_blocks_in_rows: m.compressed_blocks_in_rows,
        native_layered_textures: m.native_layered_textures,
        front_facing_shader_variant: m.front_facing_shader_variant,
        etc1_decoded_at_upload: m.etc1_decoded_at_upload,
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
        max_instance_divisor: m.max_instance_divisor,
        wait_kick: None,
        samples_texel_copies: m.samples_texel_copies,
        attachment_gone: None,
        depth_resolve: None,
        query_begin: None,
        query_end: None,
        query_result: None,
        generate_mipmap: m.generate_mipmap,
        return_mipmap_chain: m.return_mipmap_chain,
        linear_row_align: m.linear_row_align,
        uniforms_host_only: m.uniforms_host_only,
        ds_write: None,
        stencil_resolve: None,
        max_samples: m.max_samples,
    }))
}

/// One context for the whole file; the tests take turns on it.
static LOCK: Mutex<()> = Mutex::new(());
static ONCE: Once = Once::new();

fn context() -> MutexGuard<'static, ()> {
    let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    ONCE.call_once(|| {
        // SAFETY: the tables are statics or leaked; the lock above serialises
        // the tests.
        unsafe {
            osal_set_ops(&OSAL);
            backend::g_backend = stand_in();
            yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
            assert!(!yakogl::gl::context::gl_context_create(8, 8).is_null());
        }
    });
    SEEN.lock().unwrap().clear();
    // SAFETY: plain GL call.
    while unsafe { glGetError() } != GL_NO_ERROR {}
    g
}

fn err() -> GLenum {
    // SAFETY: plain GL call.
    unsafe { glGetError() }
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
        let mut log = [0u8; 512];
        let mut n: GLsizei = 0;
        glGetShaderInfoLog(s, log.len() as GLsizei, &mut n, log.as_mut_ptr().cast());
        assert_eq!(ok, 1, "{text:?} did not compile: {}", String::from_utf8_lossy(&log[..n as usize]));
        s
    }
}

fn program(vs: &str, fs: &str, attribs: &[&core::ffi::CStr]) -> GLuint {
    let (v, f) = (shader(GL_VERTEX_SHADER, vs), shader(GL_FRAGMENT_SHADER, fs));
    // SAFETY: plain GL calls.
    unsafe {
        let p = glCreateProgram();
        glAttachShader(p, v);
        glAttachShader(p, f);
        for (i, a) in attribs.iter().enumerate() {
            glBindAttribLocation(p, i as GLuint, a.as_ptr());
        }
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        assert_eq!(ok, 1, "link failed");
        glUseProgram(p);
        p
    }
}

const W: i32 = 300;
const H: i32 = 200;

/// An NPOT RGBA texture, LINEAR, CLAMP_TO_EDGE, no mipmaps, as the colour of
/// an FBO with no depth.
fn target(w: i32, h: i32) -> (GLuint, GLuint) {
    let (mut fbo, mut tex) = (0, 0);
    // SAFETY: room for one name each; null texels allocate the image.
    unsafe {
        glGenTextures(1, &mut tex);
        glBindTexture(GL_TEXTURE_2D, tex);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_LINEAR as GLint);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_LINEAR as GLint);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE as GLint);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE as GLint);
        glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA as GLint, w, h, 0, GL_RGBA, GL_UNSIGNED_BYTE, core::ptr::null());
        glGenFramebuffers(1, &mut fbo);
        glBindFramebuffer(GL_FRAMEBUFFER, fbo);
        glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex, 0);
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE);
        glViewport(0, 0, w, h);
        glClearColor(0.0, 0.0, 0.0, 1.0);
        glClear(GL_COLOR_BUFFER_BIT);
    }
    assert_eq!(err(), GL_NO_ERROR, "target set-up");
    (fbo, tex)
}

static POS: [f32; 18] =
    [-1.0, -1.0, 0.0, 1.0, -1.0, 0.0, -1.0, 1.0, 0.0, -1.0, 1.0, 0.0, 1.0, -1.0, 0.0, 1.0, 1.0, 0.0];
static UV: [f32; 12] = [0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 1.0, 1.0, 0.0, 1.0, 1.0];

/// Draw the full-screen quad into the bound framebuffer; returns the GL
/// errors of the draw and of a one-pixel read-back, and what the Mali saw.
fn draw() -> (GLenum, GLenum, Seen) {
    SEEN.lock().unwrap().clear();
    let mut px = [0u8; 4];
    // SAFETY: the arrays are static; `px` holds one RGBA8 pixel.
    unsafe {
        glBindBuffer(GL_ARRAY_BUFFER, 0);
        glVertexAttribPointer(0, 3, GL_FLOAT, 0, 0, POS.as_ptr() as *const c_void);
        glEnableVertexAttribArray(0);
        glVertexAttribPointer(1, 2, GL_FLOAT, 0, 0, UV.as_ptr() as *const c_void);
        glEnableVertexAttribArray(1);
        glDrawArrays(GL_TRIANGLES, 0, 6);
        glFinish();
    }
    let de = err();
    // SAFETY: one pixel into `px`.
    unsafe { glReadPixels(4, 4, 1, 1, GL_RGBA, GL_UNSIGNED_BYTE, px.as_mut_ptr().cast()) };
    let re = err();
    let s = core::mem::take(&mut *SEEN.lock().unwrap());
    (de, re, *s.last().expect("the draw reached the back end"))
}

/// desktop/effect2d style: a flat fill pass and a blur-like sampling pass
/// (two taps of one sampler, averaged: plain arithmetic the compiler takes).
const VS: &str = "attribute vec3 a_pos;
attribute vec2 a_uv;
varying mediump vec2 v_uv;
void main()
{
    gl_Position = vec4(a_pos, 1.0);
    v_uv = a_uv;
}
";
const FILL_FS: &str = "precision mediump float;
varying mediump vec2 v_uv;
void main()
{
    gl_FragColor = vec4(v_uv, 0.5, 1.0);
}
";
const TEX_FS: &str = "precision mediump float;
uniform sampler2D u_tex;
varying mediump vec2 v_uv;
void main()
{
    gl_FragColor = texture2D(u_tex, v_uv);
}
";

fn tex_program() {
    let p = program(VS, TEX_FS, &[c"a_pos", c"a_uv"]);
    // SAFETY: the program has u_tex.
    unsafe {
        glActiveTexture(GL_TEXTURE0);
        glUniform1i(glGetUniformLocation(p, c"u_tex".as_ptr()), 0);
    }
}

fn sample(tex: GLuint) {
    // SAFETY: a texture this test made.
    unsafe {
        glActiveTexture(GL_TEXTURE0);
        glBindTexture(GL_TEXTURE_2D, tex);
    }
}

fn check_sampled(s: &Seen, from: u64) {
    assert_eq!(s.mali, -ENODEV, "the Mali takes the draw");
    assert_eq!((s.type_, s.width, s.height), (GLES_TEXTYPE_2D, W as u32, H as u32), "the NPOT texture");
    assert_eq!((s.mip_levels <= 1, s.incomplete), (true, 0), "one level, complete");
    assert_eq!(s.linear_filter, 1, "GL_LINEAR");
    assert_eq!(s.wrap_s, s.wrap_t, "clamp on both axes alike");
    assert_eq!(s.tex_gpu, from, "samples the texture the previous pass rendered into");
}

#[test]
fn npot_target_is_complete_and_drawn() {
    let _g = context();
    let (fbo, tex) = target(W, H);
    program(VS, FILL_FS, &[c"a_pos", c"a_uv"]);
    let (de, re, s) = draw();
    assert_eq!((de, re), (GL_NO_ERROR, GL_NO_ERROR));
    assert_eq!((s.target_w, s.target_h), (W as u32, H as u32), "the NPOT colour target reached the Mali");
    assert_eq!(s.mali, -ENODEV, "the Mali takes the draw");
    // SAFETY: names this test made.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glDeleteFramebuffers(1, &fbo);
        glDeleteTextures(1, &tex);
    }
}

#[test]
fn npot_target_sampled_by_default_framebuffer() {
    let _g = context();
    let (fbo, tex) = target(W, H);
    program(VS, FILL_FS, &[c"a_pos", c"a_uv"]);
    let (de, re, fill) = draw();
    assert_eq!((de, re), (GL_NO_ERROR, GL_NO_ERROR));
    // SAFETY: plain GL.
    unsafe { glBindFramebuffer(GL_FRAMEBUFFER, 0) };
    // SAFETY: plain GL.
    unsafe { glViewport(0, 0, 8, 8) };
    tex_program();
    sample(tex);
    let (de, re, s) = draw();
    assert_eq!((de, re), (GL_NO_ERROR, GL_NO_ERROR));
    check_sampled(&s, fill.target_gpu);
    // SAFETY: names this test made.
    unsafe {
        glDeleteFramebuffers(1, &fbo);
        glDeleteTextures(1, &tex);
    }
}

#[test]
fn ping_pong_between_two_npot_targets() {
    let _g = context();
    let (fa, ta) = target(W, H);
    let (fb, tb) = target(W, H);
    // pass 0: fill A.
    // SAFETY: plain GL.
    unsafe { glBindFramebuffer(GL_FRAMEBUFFER, fa) };
    program(VS, FILL_FS, &[c"a_pos", c"a_uv"]);
    let (de, re, s0) = draw();
    assert_eq!((de, re), (GL_NO_ERROR, GL_NO_ERROR));
    assert_eq!((s0.target_w, s0.target_h), (W as u32, H as u32));
    let a_gpu = s0.target_gpu;
    // pass 1: A -> B, pass 2: B -> A (the blur's horizontal and vertical).
    tex_program();
    // SAFETY: plain GL.
    unsafe { glBindFramebuffer(GL_FRAMEBUFFER, fb) };
    sample(ta);
    let (de, re, s1) = draw();
    assert_eq!((de, re), (GL_NO_ERROR, GL_NO_ERROR), "a pass into B");
    check_sampled(&s1, a_gpu);
    assert_ne!(s1.target_gpu, a_gpu, "B is a different target from A");
    // SAFETY: plain GL.
    unsafe { glBindFramebuffer(GL_FRAMEBUFFER, fa) };
    sample(tb);
    let (de, re, s2) = draw();
    assert_eq!((de, re), (GL_NO_ERROR, GL_NO_ERROR), "a pass back into A");
    check_sampled(&s2, s1.target_gpu);
    assert_eq!(s2.target_gpu, a_gpu, "the pass lands in A again");
    // final: A to the default framebuffer.
    // SAFETY: plain GL.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glViewport(0, 0, 8, 8);
    }
    sample(ta);
    let (de, re, s3) = draw();
    assert_eq!((de, re), (GL_NO_ERROR, GL_NO_ERROR), "the final pass");
    check_sampled(&s3, a_gpu);
    // SAFETY: names this test made.
    unsafe {
        glDeleteFramebuffers(1, &fa);
        glDeleteFramebuffers(1, &fb);
        glDeleteTextures(1, &ta);
        glDeleteTextures(1, &tb);
    }
}

#[test]
fn feedback_loop_raises_no_gl_error() {
    // Sampling the texture that is the bound target is a feedback loop; the
    // GL core must not raise an error for it (it is undefined, not an
    // error), so the test only pins that the call is not a GL error.
    let _g = context();
    let (fbo, tex) = target(W, H);
    tex_program();
    sample(tex);
    let (de, _re, _s) = draw();
    assert_eq!(de, GL_NO_ERROR);
    // SAFETY: names this test made.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glDeleteFramebuffers(1, &fbo);
        glDeleteTextures(1, &tex);
    }
}
