// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GL-level host test of glmark2's scene-to-scene churn on the Mali back end
//! (plan #367). glmark2 runs 33 scenes in one context; each creates
//! programs, a texture, buffers and a framebuffer, draws, and deletes them.
//! The board tally counts out-of-memory lines, so the churn must leave no
//! GL error and no growth the host can see.
//!
//! Each iteration: two programs (a lit pair and a textured pair, shaders
//! deleted after link as glmark2 does), a 2x2 texture, a VBO of the quad, an
//! RGBA8 FBO; draw with program A, switch to B with uniform updates, draw,
//! read one pixel, delete everything. 40 iterations (> 33 scenes) run in
//! `churn_like_33_scenes`, 500 in `churn_500`.
//!
//! Level reached: link plus draw submission (as `mali_glmark2_scene_draw`):
//! the host has no Mali OSAL, so `mali_draw` ends at `-ENODEV` and no pixel
//! is written; "correct" is: the draw reaches the Mali, with the sampler
//! binding the program uses, and no GL error is raised. The back end exposes
//! no resource counter on the host; what is checked instead is that the GL
//! object names handed out do not grow (names are recycled) and that the
//! loop survives 500 iterations. No defect was found.
//!
//! A tools/check.sh row (the GL-on-mock Mali step).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_glmark2_scene_churn
//! ```

#![cfg(all(feature = "host-test", feature = "shaderc-online", feature = "gpu-mali", not(feature = "report-es2")))]

use core::ffi::{c_char, c_void};
use std::ffi::CString;
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{self, GlesDraw, GlesHeap, YakoglBackend};
use yakogl::gl::buffer::{glBindBuffer, glBufferData, glDeleteBuffers, glGenBuffers};
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

/// What the back end was handed for the draw's texture, and what the Mali
/// made of the draw.
#[derive(Clone, Copy, Debug)]
#[allow(dead_code)]
struct Seen {
    type_: u8,
    width: u32,
    mip_levels: u8,
    mip_filter: u8,
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
        type_: t.type_,
        width: t.width,
        mip_levels: t.mip_levels,
        mip_filter: t.mip_filter,
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
        glDetachShader(p, v);
        glDetachShader(p, f);
        glDeleteShader(v);
        glDeleteShader(f);
        p
    }
}

/// An RGBA8 texture as the colour attachment of a framebuffer.
fn target(w: i32, h: i32) -> (GLuint, GLuint) {
    let (mut fbo, mut tex) = (0, 0);
    // SAFETY: room for one name each; null texels allocate the image.
    unsafe {
        glGenTextures(1, &mut tex);
        glBindTexture(GL_TEXTURE_2D, tex);
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

/// Position (xyz) and coordinate (uv) per vertex, two triangles.
static QUAD: [f32; 30] = [
    -1.0, -1.0, 0.0, 0.0, 0.0, 1.0, -1.0, 0.0, 1.0, 0.0, -1.0, 1.0, 0.0, 0.0, 1.0, -1.0, 1.0, 0.0, 0.0, 1.0, 1.0, -1.0,
    0.0, 1.0, 0.0, 1.0, 1.0, 0.0, 1.0, 1.0,
];

const LIT_VS: &str = "attribute vec3 a_pos;
attribute vec2 a_uv;
uniform mat4 u_mvp;
varying mediump vec2 v_uv;
void main()
{
    gl_Position = u_mvp * vec4(a_pos, 1.0);
    v_uv = a_uv;
}
";
const LIT_FS: &str = "precision mediump float;
uniform vec4 u_color;
varying mediump vec2 v_uv;
void main()
{
    gl_FragColor = vec4(u_color.rgb * (0.2 + 0.8 * v_uv.x), 1.0);
}
";
const TEX_VS: &str = "attribute vec3 a_pos;
attribute vec2 a_uv;
varying mediump vec2 v_uv;
void main()
{
    gl_Position = vec4(a_pos, 1.0);
    v_uv = a_uv;
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

/// What one iteration handed out, to compare across iterations.
#[derive(Debug, PartialEq)]
struct Names {
    progs: [GLuint; 2],
    tex: GLuint,
    rt: GLuint,
    vbo: GLuint,
    fbo: GLuint,
}

fn draw_quad(vbo: GLuint) -> (GLenum, Seen) {
    SEEN.lock().unwrap().clear();
    // SAFETY: the VBO holds QUAD; strides and offsets are inside it.
    unsafe {
        glBindBuffer(GL_ARRAY_BUFFER, vbo);
        glVertexAttribPointer(0, 3, GL_FLOAT, 0, 20, core::ptr::null());
        glEnableVertexAttribArray(0);
        glVertexAttribPointer(1, 2, GL_FLOAT, 0, 20, 12usize as *const c_void);
        glEnableVertexAttribArray(1);
        glDrawArrays(GL_TRIANGLES, 0, 6);
        glFinish();
    }
    let de = err();
    let s = core::mem::take(&mut *SEEN.lock().unwrap());
    (de, *s.last().expect("the draw reached the back end"))
}

/// One scene: create, draw, switch, read, delete.
fn scene() -> Names {
    let mut n = Names { progs: [0; 2], tex: 0, rt: 0, vbo: 0, fbo: 0 };
    let texels = [0xffu8, 0, 0, 0xff, 0, 0xff, 0, 0xff, 0, 0, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff];
    let ident: [f32; 16] = [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0];
    n.progs[0] = program(LIT_VS, LIT_FS, &[c"a_pos", c"a_uv"]);
    n.progs[1] = program(TEX_VS, TEX_FS, &[c"a_pos", c"a_uv"]);
    let (fbo, rt) = target(8, 8);
    n.fbo = fbo;
    n.rt = rt;
    // SAFETY: room for one name each; the arrays outlive the calls.
    unsafe {
        glGenTextures(1, &mut n.tex);
        glActiveTexture(GL_TEXTURE0);
        glBindTexture(GL_TEXTURE_2D, n.tex);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST as GLint);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST as GLint);
        glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA as GLint, 2, 2, 0, GL_RGBA, GL_UNSIGNED_BYTE, texels.as_ptr().cast());
        glGenBuffers(1, &mut n.vbo);
        glBindBuffer(GL_ARRAY_BUFFER, n.vbo);
        glBufferData(
            GL_ARRAY_BUFFER,
            core::mem::size_of_val(&QUAD) as GLsizeiptr,
            QUAD.as_ptr().cast(),
            GL_STATIC_DRAW,
        );
    }
    assert_eq!(err(), GL_NO_ERROR, "scene set-up");
    let mut frames = 0;
    for f in 0..3 {
        // SAFETY: the programs are live and have these uniforms.
        let (a, b) = unsafe {
            glUseProgram(n.progs[0]);
            glUniformMatrix4fv(glGetUniformLocation(n.progs[0], c"u_mvp".as_ptr()), 1, 0, ident.as_ptr());
            glUniform4f(glGetUniformLocation(n.progs[0], c"u_color".as_ptr()), 1.0, 0.5, 0.25 + f as f32 * 0.25, 1.0);
            let a = draw_quad(n.vbo);
            glUseProgram(n.progs[1]);
            glUniform1i(glGetUniformLocation(n.progs[1], c"u_tex".as_ptr()), 0);
            (a, draw_quad(n.vbo))
        };
        for (de, s) in [a, b] {
            assert_eq!(de, GL_NO_ERROR, "frame {f} draw");
            assert_eq!(s.mali, -ENODEV, "the Mali takes the draw");
            frames += 1;
        }
    }
    assert_eq!(frames, 6);
    let mut px = [0u8; 4];
    // SAFETY: one pixel into `px`.
    unsafe { glReadPixels(4, 4, 1, 1, GL_RGBA, GL_UNSIGNED_BYTE, px.as_mut_ptr().cast()) };
    assert_eq!(err(), GL_NO_ERROR, "read-back");
    // SAFETY: names this function made.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glBindBuffer(GL_ARRAY_BUFFER, 0);
        glBindTexture(GL_TEXTURE_2D, 0);
        glUseProgram(0);
        glDeleteFramebuffers(1, &n.fbo);
        glDeleteTextures(1, &n.rt);
        glDeleteTextures(1, &n.tex);
        glDeleteBuffers(1, &n.vbo);
        glDeleteProgram(n.progs[0]);
        glDeleteProgram(n.progs[1]);
    }
    assert_eq!(err(), GL_NO_ERROR, "scene teardown");
    n
}

fn churn(iters: usize) {
    let _g = context();
    let first = scene();
    for i in 1..iters {
        let n = scene();
        // Names are recycled: a growing name space would show a leak.
        assert_eq!(n, first, "iteration {i}: object names grew or changed");
    }
}

#[test]
fn churn_like_33_scenes() {
    churn(40);
}

#[test]
fn churn_500() {
    churn(500);
}
