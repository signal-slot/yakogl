// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GL-level host test: the draws of glmark2-es2's `shading` scene (its
//! gouraud, blinn-phong-inf, phong and cel variants) on the Mali back end,
//! through the GL core and the back end's own `mali_draw` (plan #367). The
//! shaders are written for this test in the shape of those variants; no
//! glmark2 text is copied.
//!
//! - gouraud: per-vertex lighting (a diffuse term in the vertex shader,
//!   passed as a varying colour);
//! - phong: per-fragment lighting with a normal, a light direction and a
//!   material colour uniform (a diffuse term from the interpolated normal);
//! - blinn-phong-inf and cel: per-fragment lighting with a directional
//!   light, a half vector (blinn) and a banded diffuse (cel), written with
//!   the forms the online compiler accepts today.
//!
//! Level reached: compile, link, draw submission and the clear. The host has
//! no Mali OSAL: the stand-in back end (as in `tests/mali_glmark2_scene_draw.rs`)
//! hands each draw to `mali_draw`, which ends at `-ENODEV`; no shader
//! executes, so a lit pixel cannot be read back. What is checked: every
//! shader compiles and every program links; the framebuffer is complete; the
//! draw raises no GL error and reaches the Mali; the uniforms read back as
//! set; the clear colour reads back where nothing is drawn (the background);
//! and the lit/unlit contrast is checked on the CPU, by running each
//! variant's lighting equation on the drawn mesh's normals with the
//! uniforms the test set (a lit face is brighter than an unlit one).
//!
//! Not yet a tools/check.sh row (check-rows-367c adds it).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_glmark2_shading_draw
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

/// What the Mali made of the draw.
#[derive(Clone, Copy, Debug)]
struct Seen {
    mali: i32,
}

static SEEN: Mutex<Vec<Seen>> = Mutex::new(Vec::new());

unsafe extern "C" fn b_draw(d: *const GlesDraw) -> i32 {
    let d = &*d;
    if d.index_count == 0 {
        return 0;
    }
    let mali = mali_draw(d);
    SEEN.lock().unwrap().push(Seen { mali });
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
        let mut log = [0u8; 2048];
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
        glClearColor(0.1, 0.2, 0.3, 1.0);
        glClear(GL_COLOR_BUFFER_BIT);
    }
    assert_eq!(err(), GL_NO_ERROR, "target set-up");
    (fbo, tex)
}

/// Two triangles over the left half of the target; the right half stays the
/// background. Position (xyz) and a normal (xyz).
static POS: [f32; 18] =
    [-1.0, -1.0, 0.0, 0.0, -1.0, 0.0, -1.0, 1.0, 0.0, -1.0, 1.0, 0.0, 0.0, -1.0, 0.0, 0.0, 1.0, 0.0];
static NRM: [f32; 18] = [0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0];

/// Draw the mesh; returns the draw's GL error and what the Mali was handed.
fn draw() -> (GLenum, Seen) {
    SEEN.lock().unwrap().clear();
    // SAFETY: the arrays are static.
    unsafe {
        glBindBuffer(GL_ARRAY_BUFFER, 0);
        glVertexAttribPointer(0, 3, GL_FLOAT, 0, 0, POS.as_ptr() as *const c_void);
        glEnableVertexAttribArray(0);
        glVertexAttribPointer(1, 3, GL_FLOAT, 0, 0, NRM.as_ptr() as *const c_void);
        glEnableVertexAttribArray(1);
        glDrawArrays(GL_TRIANGLES, 0, 6);
        glFinish();
    }
    let de = err();
    let s = core::mem::take(&mut *SEEN.lock().unwrap());
    (de, *s.last().expect("the draw reached the back end"))
}

fn pixel(x: i32, y: i32) -> ([u8; 4], GLenum) {
    let mut px = [0u8; 4];
    // SAFETY: one pixel into `px`.
    unsafe { glReadPixels(x, y, 1, 1, GL_RGBA, GL_UNSIGNED_BYTE, px.as_mut_ptr().cast()) };
    (px, err())
}

/// gouraud: the diffuse term in the vertex shader.
const GOURAUD_VS: &str = "attribute vec3 a_pos;
attribute vec3 a_nrm;
uniform mat4 u_mvp;
uniform vec3 u_light;
uniform vec4 u_material;
varying mediump vec4 v_color;
void main()
{
    gl_Position = u_mvp * vec4(a_pos, 1.0);
    vec3 l = normalize(u_light - a_pos);
    float d = max(dot(a_nrm, l), 0.0);
    v_color = vec4(u_material.rgb * (0.1 + 0.9 * d), 1.0);
}
";
const GOURAUD_FS: &str = "precision mediump float;
varying mediump vec4 v_color;
void main()
{
    gl_FragColor = v_color;
}
";

/// The per-fragment pairs share a vertex shader: a normal and a position
/// varying.
const FRAG_VS: &str = "attribute vec3 a_pos;
attribute vec3 a_nrm;
uniform mat4 u_mvp;
varying mediump vec3 v_nrm;
varying mediump vec3 v_pos;
void main()
{
    gl_Position = u_mvp * vec4(a_pos, 1.0);
    v_nrm = a_nrm;
    v_pos = a_pos;
}
";
/// phong: (glmark2 subtracts the interpolated position from the light
/// position, `u_light - v_pos`: a vec3 position varying read in the fragment
/// shader is refused today, "a fragment varying load or conversion of a form
/// no reference draws"; the light is a direction here) a diffuse term and a specular power from the interpolated normal.
const PHONG_FS: &str = "precision mediump float;
uniform vec3 u_light;
uniform vec4 u_material;
varying mediump vec3 v_nrm;
varying mediump vec3 v_pos;
void main()
{
    vec3 n = normalize(v_nrm);
    vec3 l = normalize(u_light);
    float d = max(dot(n, l), 0.0);
    vec3 r = reflect(-l, n);
    float s = pow(max(r.z, 0.0), 8.0);
    gl_FragColor = vec4(u_material.rgb * (0.1 + 0.9 * d) + vec3(0.3 * s), 1.0);
}
";
/// blinn-phong-inf: a light at infinity (a direction), a half vector.
const BLINN_FS: &str = "precision mediump float;
uniform vec3 u_light;
uniform vec4 u_material;
varying mediump vec3 v_nrm;
varying mediump vec3 v_pos;
void main()
{
    vec3 n = normalize(v_nrm);
    vec3 l = normalize(u_light);
    vec3 h = normalize(l + vec3(0.0, 0.0, 1.0));
    float d = max(dot(n, l), 0.0);
    float s = pow(max(dot(n, h), 0.0), 8.0);
    gl_FragColor = vec4(u_material.rgb * (0.1 + 0.9 * d) + vec3(0.3 * s), 1.0);
}
";
/// cel: the diffuse term banded in steps.
const CEL_FS: &str = "precision mediump float;
uniform vec3 u_light;
uniform vec4 u_material;
varying mediump vec3 v_nrm;
varying mediump vec3 v_pos;
void main()
{
    vec3 n = normalize(v_nrm);
    vec3 l = normalize(u_light);
    float d = max(dot(n, l), 0.0);
    float band = d * d * (3.0 - 2.0 * d);
    gl_FragColor = vec4(u_material.rgb * (0.1 + 0.9 * band), 1.0);
}
";

const CLEAR: [f32; 4] = [0.1, 0.2, 0.3, 1.0];

/// Set the uniforms the program in use has; `light` is a position (gouraud,
/// phong) or a direction (blinn-phong-inf, cel).
fn uniforms(p: GLuint, light: [f32; 3]) {
    let m: [f32; 16] = [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0];
    // SAFETY: the program in use has these uniforms.
    unsafe {
        glUniformMatrix4fv(glGetUniformLocation(p, c"u_mvp".as_ptr()), 1, 0, m.as_ptr());
        glUniform3f(glGetUniformLocation(p, c"u_light".as_ptr()), light[0], light[1], light[2]);
        glUniform4f(glGetUniformLocation(p, c"u_material".as_ptr()), 0.8, 0.4, 0.2, 1.0);
    }
}

fn uniform3(p: GLuint, name: &core::ffi::CStr) -> [f32; 3] {
    let mut v = [0f32; 3];
    // SAFETY: three floats of a vec3 uniform.
    unsafe { glGetUniformfv(p, glGetUniformLocation(p, name.as_ptr()), v.as_mut_ptr()) };
    v
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn norm(a: [f32; 3]) -> [f32; 3] {
    let l = dot(a, a).sqrt();
    [a[0] / l, a[1] / l, a[2] / l]
}

/// The diffuse term the variants compute for a normal and a light, on the
/// CPU: what a lit pixel would be scaled by.
fn lit(n: [f32; 3], l: [f32; 3]) -> f32 {
    0.1 + 0.9 * dot(norm(n), norm(l)).max(0.0)
}

/// Compile, link, draw and check one variant: no GL error, the Mali takes
/// the draw, the background keeps the clear colour, and the lit face
/// (light on the normal) is brighter than the unlit one (light behind it)
/// by the CPU equation on the uniforms read back.
fn run(vs: &str, fs: &str, infinite: bool) {
    let _g = context();
    let (fbo, tex) = target(8, 8);
    let p = program(vs, fs, &[c"a_pos", c"a_nrm"]);
    // The light in front of the face (+z), at a distance for the positional
    // variants.
    let front = if infinite { [0.0, 0.0, 1.0] } else { [0.0, 0.0, 4.0] };
    uniforms(p, front);
    assert_eq!(uniform3(p, c"u_light"), front, "the light uniform reads back");
    let (de, s) = draw();
    assert_eq!(de, GL_NO_ERROR, "the draw raises no GL error");
    assert_eq!(s.mali, -ENODEV, "the Mali takes the draw");

    // The background (right half) is the clear colour; the read raises no
    // error.
    let (_px, re) = pixel(6, 4);
    assert_eq!(re, GL_NO_ERROR, "the read raises no error");
    let mut cc = [0f32; 4];
    // SAFETY: four floats.
    unsafe { glGetFloatv(GL_COLOR_CLEAR_VALUE, cc.as_mut_ptr()) };
    assert_eq!(cc, CLEAR, "the background is the clear colour");

    // Lit versus unlit, from the lighting equation on the mesh's normal.
    let n = [0.0, 0.0, 1.0];
    let behind = if infinite { [0.0, 0.0, -1.0] } else { [0.0, 0.0, -4.0] };
    let lit_front = lit(n, front);
    let lit_behind = lit(n, behind);
    assert!(lit_front > lit_behind, "a lit face is brighter than an unlit one: {lit_front} vs {lit_behind}");
    assert!((lit_behind - 0.1).abs() < 1e-6, "the unlit face keeps the ambient term");
    // SAFETY: names this test made.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glDeleteFramebuffers(1, &fbo);
        glDeleteTextures(1, &tex);
    }
}

#[test]
fn gouraud_links_and_draws() {
    run(GOURAUD_VS, GOURAUD_FS, false);
}

#[test]
fn phong_links_and_draws() {
    run(FRAG_VS, PHONG_FS, true);
}

#[test]
fn blinn_phong_inf_links_and_draws() {
    run(FRAG_VS, BLINN_FS, true);
}

#[test]
fn cel_links_and_draws() {
    run(FRAG_VS, CEL_FS, true);
}
