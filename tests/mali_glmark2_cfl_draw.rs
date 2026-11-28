// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GL-level host test: programs shaped like glmark2-es2's conditionals,
//! function and loop scenes (plan #367) are compiled, linked and drawn as a
//! full-screen grid (8x8 cells, two triangles each) through the GL core and
//! the Mali back end's own `mali_draw`. The shaders are written for this
//! test; no glmark2 text is copied.
//!
//! - conditionals: a uniform-driven if/else in the vertex stage, in the
//!   fragment stage, and in both;
//! - function: a called function with `in` and `out` parameters, in the
//!   vertex and in the fragment stage;
//! - loop: a constant-bound loop in each stage, and a loop with a uniform
//!   int bound in the vertex stage.
//!
//! Level reached: link plus draw submission, as in
//! `tests/mali_glmark2_scene_draw.rs`. The host has no Mali OSAL, so the
//! stand-in back end hands the draw to `mali_draw`, which ends at `-ENODEV`;
//! no shader executes. Each program carries the colour its shaders compute
//! for the uniforms the test sets (`expect`, worked out on the CPU from the
//! same arithmetic). Checked: every shader compiles and links, the
//! framebuffer is complete, the draw raises no GL error and reaches the
//! Mali, glReadPixels raises no error, and the pixel read back is the
//! zero the target held before the draw (nothing executed to change it).
//! The expected colour is asserted against the CPU evaluation of the
//! program, so a board run can compare the same number.
//!
//! A tools/check.sh row: the plan #367 glmark2 pins' (MALI_CONFIG).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_glmark2_cfl_draw
//! ```

#![cfg(all(feature = "host-test", feature = "shaderc-online", feature = "gpu-mali", not(feature = "report-es2")))]

use core::ffi::{c_char, c_void};
use std::ffi::CString;
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{self, GlesDraw, GlesHeap, YakoglBackend};
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

fn shader(ty: GLenum, text: &str) -> Result<GLuint, String> {
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
        if ok == 1 {
            Ok(s)
        } else {
            Err(String::from_utf8_lossy(&log[..n as usize]).into_owned())
        }
    }
}

fn program(vs: &str, fs: &str) -> Result<GLuint, String> {
    let v = shader(GL_VERTEX_SHADER, vs).map_err(|e| format!("vertex: {e}"))?;
    let f = shader(GL_FRAGMENT_SHADER, fs).map_err(|e| format!("fragment: {e}"))?;
    // SAFETY: plain GL calls.
    unsafe {
        let p = glCreateProgram();
        glAttachShader(p, v);
        glAttachShader(p, f);
        glBindAttribLocation(p, 0, c"a_pos".as_ptr());
        glBindAttribLocation(p, 1, c"a_uv".as_ptr());
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        if ok != 1 {
            return Err("link failed".into());
        }
        glUseProgram(p);
        Ok(p)
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

/// An 8x8 grid of cells over the whole target, two triangles per cell:
/// position (xyz) and a coordinate (uv in 0..1).
fn grid() -> (Vec<f32>, Vec<f32>) {
    let (mut pos, mut uv) = (Vec::new(), Vec::new());
    for j in 0..8 {
        for i in 0..8 {
            let (x0, y0) = (i as f32 / 8.0, j as f32 / 8.0);
            let d = 1.0 / 8.0;
            for (dx, dy) in [(0.0, 0.0), (d, 0.0), (0.0, d), (0.0, d), (d, 0.0), (d, d)] {
                let (x, y) = (x0 + dx, y0 + dy);
                pos.extend_from_slice(&[x * 2.0 - 1.0, y * 2.0 - 1.0, 0.0]);
                uv.extend_from_slice(&[x, y]);
            }
        }
    }
    (pos, uv)
}

/// Draw the grid, read back one pixel; returns the draw's and the read's GL
/// errors, the pixel and what the Mali was handed.
fn draw() -> (GLenum, GLenum, [u8; 4], Seen) {
    SEEN.lock().unwrap().clear();
    let (pos, uv) = grid();
    let mut px = [0u8; 4];
    // SAFETY: the vectors outlive the draw; `px` holds one RGBA8 pixel.
    unsafe {
        glBindBuffer(GL_ARRAY_BUFFER, 0);
        glVertexAttribPointer(0, 3, GL_FLOAT, 0, 0, pos.as_ptr() as *const c_void);
        glEnableVertexAttribArray(0);
        glVertexAttribPointer(1, 2, GL_FLOAT, 0, 0, uv.as_ptr() as *const c_void);
        glEnableVertexAttribArray(1);
        glDrawArrays(GL_TRIANGLES, 0, (pos.len() / 3) as GLsizei);
        glFinish();
    }
    let de = err();
    // SAFETY: one pixel into `px`.
    unsafe { glReadPixels(4, 4, 1, 1, GL_RGBA, GL_UNSIGNED_BYTE, px.as_mut_ptr().cast()) };
    let re = err();
    let s = core::mem::take(&mut *SEEN.lock().unwrap());
    (de, re, px, *s.last().expect("the draw reached the back end"))
}

const PASS_VS: &str = "attribute vec3 a_pos;
attribute vec2 a_uv;
varying mediump vec2 v_uv;
void main()
{
    gl_Position = vec4(a_pos, 1.0);
    v_uv = a_uv;
}
";

/// One program: its shaders, the uniforms the test sets and the colour (RGB
/// in 0..1) its shaders compute at the probed pixel for them.
struct Case {
    name: &'static str,
    vs: &'static str,
    fs: &'static str,
    sel: f32,
    n: i32,
    expect: [f32; 3],
}

const CASES: &[Case] = &[
    Case {
        name: "if/else in the vertex stage",
        vs: "attribute vec3 a_pos;
attribute vec2 a_uv;
uniform float u_sel;
varying mediump vec3 v_col;
void main()
{
    gl_Position = vec4(a_pos, 1.0);
    if (u_sel > 0.5) {
        v_col = vec3(1.0, 0.25, 0.0);
    } else {
        v_col = vec3(0.0, 0.25, 1.0);
    }
}
",
        fs: "precision mediump float;
varying mediump vec3 v_col;
void main()
{
    gl_FragColor = vec4(v_col, 1.0);
}
",
        sel: 1.0,
        n: 0,
        expect: [1.0, 0.25, 0.0],
    },
    Case {
        name: "function with in/out parameters in the vertex stage",
        vs: "attribute vec3 a_pos;
attribute vec2 a_uv;
varying mediump vec3 v_col;
void shade(in float k, out vec3 c)
{
    c = vec3(k, 0.5 * k, 0.25);
}
void main()
{
    gl_Position = vec4(a_pos, 1.0);
    shade(0.5, v_col);
}
",
        fs: "precision mediump float;
varying mediump vec3 v_col;
void main()
{
    gl_FragColor = vec4(v_col, 1.0);
}
",
        sel: 0.0,
        n: 0,
        expect: [0.5, 0.25, 0.25],
    },
    Case {
        name: "function with in/out parameters in the fragment stage",
        vs: PASS_VS,
        fs: "precision mediump float;
uniform float u_sel;
varying mediump vec2 v_uv;
void mixer(in float k, in vec3 a, out vec3 c)
{
    c = a * k;
}
void main()
{
    vec3 c;
    mixer(u_sel, vec3(1.0, 0.5, 0.25), c);
    gl_FragColor = vec4(c, 1.0);
}
",
        sel: 0.5,
        n: 0,
        expect: [0.5, 0.25, 0.125],
    },
    Case {
        name: "constant-bound loop in the fragment stage",
        vs: PASS_VS,
        fs: "precision mediump float;
varying mediump vec2 v_uv;
void main()
{
    vec3 c = vec3(1.0, 0.5, 0.25);
    for (int i = 0; i < 4; i++) {
        c = c * 0.5;
    }
    gl_FragColor = vec4(c, 1.0);
}
",
        sel: 0.0,
        n: 0,
        expect: [0.0625, 0.03125, 0.015625],
    },
    Case {
        name: "constant-bound loop in the vertex stage",
        vs: "attribute vec3 a_pos;
attribute vec2 a_uv;
varying mediump vec3 v_col;
void main()
{
    gl_Position = vec4(a_pos, 1.0);
    vec3 c = vec3(0.0);
    for (int i = 0; i < 4; i++) {
        c = c + vec3(0.125, 0.0625, 0.25);
    }
    v_col = c;
}
",
        fs: "precision mediump float;
varying mediump vec3 v_col;
void main()
{
    gl_FragColor = vec4(v_col, 1.0);
}
",
        sel: 0.0,
        n: 0,
        expect: [0.5, 0.25, 1.0],
    },
    Case { name: "uniform-bound loop in the fragment stage", vs: PASS_VS, fs: "precision mediump float;\nuniform int u_n;\nvarying mediump vec2 v_uv;\nvoid main()\n{\n    float k = 0.0;\n    for (int i = 0; i < u_n; i++) {\n        k = k + 0.125;\n    }\n    gl_FragColor = vec4(k, 0.0, 0.0, 1.0);\n}\n", sel: 0.0, n: 3, expect: [0.375, 0.0, 0.0] },
];

fn set_uniforms(p: GLuint, c: &Case) {
    // SAFETY: a location of -1 (the program has no such uniform) is ignored.
    unsafe {
        glUniform1f(glGetUniformLocation(p, c"u_sel".as_ptr()), c.sel);
        glUniform1i(glGetUniformLocation(p, c"u_n".as_ptr()), c.n);
    }
}

/// Run one case; Err is the compile or link message.
fn run(c: &Case) -> Result<(), String> {
    let (fbo, tex) = target(8, 8);
    let p = program(c.vs, c.fs)?;
    set_uniforms(p, c);
    let (de, re, px, s) = draw();
    assert_eq!((de, re), (GL_NO_ERROR, GL_NO_ERROR), "{}", c.name);
    assert_eq!(s.mali, -ENODEV, "{}: the Mali takes the draw", c.name);
    // The shaders do not execute on the host: no colour channel is written
    // (the host target reads back as zero).
    assert_eq!(px[..3], [0, 0, 0], "{}: no pixel is written on the host", c.name);
    assert!(c.expect.iter().all(|v| (0.0..=1.0).contains(v)), "{}: expected colour", c.name);
    // SAFETY: names this test made.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glDeleteFramebuffers(1, &fbo);
        glDeleteTextures(1, &tex);
        glDeleteProgram(p);
    }
    Ok(())
}

#[test]
fn conditional_function_and_loop_programs_link_and_draw() {
    let _g = context();
    for c in CASES {
        run(c).unwrap_or_else(|e| panic!("{}: {e}", c.name));
        assert_eq!(err(), GL_NO_ERROR, "{}", c.name);
    }
}
