// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GL-level host test: programs shaped like glmark2-es2's bump scene (plan
//! #367), compiled by the Mali online compiler, linked and drawn through the
//! GL core and the Mali back end's `mali_draw`. The shaders are written for
//! this test; no glmark2 text is copied.
//!
//! - height map: a fragment shader that reads the height texture at the
//!   texel and at a neighbouring texel (the neighbour's
//!   coordinate a constant) and takes the derivative;
//! - normal map: a fragment shader that unpacks a tangent-space normal from
//!   an RGBA texture and lights it;
//! - high-poly: a lit pair with a vertex normal and no texture.
//!
//! Level reached: link plus draw submission, as in
//! `tests/mali_glmark2_scene_draw.rs`. The host has no Mali OSAL, so the
//! stand-in hands the draw to `mali_draw`, which ends at `-ENODEV`; no shader
//! executes and no pixel is written, so "pixels differ where the normal
//! differs" cannot be read back here. What is checked instead: link status,
//! glGetError after set-up, draw and readback, that each draw reaches the
//! Mali with the textures the program samples, and that two different
//! normal maps reach it as two different textures (distinct GPU addresses).
//!
//! Refused glmark2-literal forms are tracked in the GitHub issues, not tested.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_glmark2_bump_draw
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
#[derive(Clone, Debug)]
#[allow(dead_code)]
struct Seen {
    num_textures: u8,
    /// (unit, width, height, incomplete, base_gpu) of each bound texture.
    tex: Vec<(u8, u32, u32, u8, u64)>,
    mali: i32,
}

static SEEN: Mutex<Vec<Seen>> = Mutex::new(Vec::new());

unsafe extern "C" fn b_draw(d: *const GlesDraw) -> i32 {
    let d = &*d;
    if d.index_count == 0 {
        return 0;
    }
    let mali = mali_draw(d);
    let tex = d.textures[..d.num_textures as usize]
        .iter()
        .map(|t| (t.unit, t.width, t.height, t.incomplete, t.base_gpu))
        .collect();
    SEEN.lock().unwrap().push(Seen { num_textures: d.num_textures, tex, mali });
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

/// Two triangles over the whole target: position (xyz) and a second
/// attribute (normal or coordinate, 3 floats).
static POS: [f32; 18] =
    [-1.0, -1.0, 0.0, 1.0, -1.0, 0.0, -1.0, 1.0, 0.0, -1.0, 1.0, 0.0, 1.0, -1.0, 0.0, 1.0, 1.0, 0.0];
static NRM: [f32; 18] = [0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0];
static UV: [f32; 12] = [0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 1.0, 1.0, 0.0, 1.0, 1.0];

/// Draw the quad with `second` as attribute 1 (`n` floats per vertex), read
/// back one pixel; returns the draw's and the read's GL errors, and what the
/// Mali was handed.
fn draw(second: &[f32], n: i32) -> (GLenum, GLenum, Seen) {
    SEEN.lock().unwrap().clear();
    let mut px = [0u8; 4];
    // SAFETY: the arrays are static; `px` holds one RGBA8 pixel.
    unsafe {
        glBindBuffer(GL_ARRAY_BUFFER, 0);
        glVertexAttribPointer(0, 3, GL_FLOAT, 0, 0, POS.as_ptr() as *const c_void);
        glEnableVertexAttribArray(0);
        glVertexAttribPointer(1, n as GLint, GL_FLOAT, 0, 0, second.as_ptr() as *const c_void);
        glEnableVertexAttribArray(1);
        glDrawArrays(GL_TRIANGLES, 0, 6);
        glFinish();
    }
    let de = err();
    // SAFETY: one pixel into `px`.
    unsafe { glReadPixels(4, 4, 1, 1, GL_RGBA, GL_UNSIGNED_BYTE, px.as_mut_ptr().cast()) };
    let re = err();
    let s = core::mem::take(&mut *SEEN.lock().unwrap());
    (de, re, s.last().expect("the draw reached the back end").clone())
}

/// Height-map bump: the derivative of a height texture from its neighbours.
const HEIGHT_VS: &str = "attribute vec3 a_pos;
attribute vec2 a_uv;
varying mediump vec2 v_uv;
void main()
{
    gl_Position = vec4(a_pos, 1.0);
    v_uv = a_uv;
}
";
const HEIGHT_FS: &str = "precision mediump float;
uniform sampler2D u_height;
varying mediump vec2 v_uv;
void main()
{
    vec4 d = texture2D(u_height, v_uv) - texture2D(u_height, vec2(0.75, 0.25));
    gl_FragColor = d * 0.5 + 0.5;
}
";

/// Normal-map bump: a tangent-space normal from an RGBA texture, one light.
const NORMAL_FS: &str = "precision mediump float;
uniform sampler2D u_normals;
uniform vec3 u_light;
varying mediump vec2 v_uv;
void main()
{
    vec3 n = normalize(texture2D(u_normals, v_uv).rgb * 2.0 - 1.0);
    float d = max(dot(n, normalize(u_light)), 0.0);
    gl_FragColor = vec4(vec3(0.1 + 0.9 * d), 1.0);
}
";

/// High-poly bump: a per-vertex diffuse term, no texture.
const POLY_VS: &str = "attribute vec3 a_pos;
attribute vec3 a_nrm;
uniform vec3 u_light;
varying mediump float v_diff;
void main()
{
    gl_Position = vec4(a_pos, 1.0);
    v_diff = max(dot(a_nrm, normalize(u_light)), 0.0);
}
";
const POLY_FS: &str = "precision mediump float;
varying mediump float v_diff;
void main()
{
    gl_FragColor = vec4(vec3(0.1 + 0.9 * v_diff), 1.0);
}
";

/// A 2x2 RGBA texture on unit `unit`, nearest filtering; returns its name.
fn texture(unit: GLenum, texels: &[u8; 16]) -> GLuint {
    let mut t = 0;
    // SAFETY: room for one name; the texels cover 2x2 RGBA.
    unsafe {
        glGenTextures(1, &mut t);
        glActiveTexture(unit);
        glBindTexture(GL_TEXTURE_2D, t);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST as GLint);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST as GLint);
        glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA as GLint, 2, 2, 0, GL_RGBA, GL_UNSIGNED_BYTE, texels.as_ptr().cast());
    }
    assert_eq!(err(), GL_NO_ERROR, "the texture");
    t
}

fn finish(fbo: GLuint, tex: GLuint, extra: &[GLuint]) {
    // SAFETY: names this test made.
    unsafe {
        glActiveTexture(GL_TEXTURE0);
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glDeleteFramebuffers(1, &fbo);
        glDeleteTextures(1, &tex);
        for t in extra {
            glDeleteTextures(1, t);
        }
    }
}

fn loc(p: GLuint, name: &core::ffi::CStr) -> GLint {
    // SAFETY: plain GL call.
    unsafe { glGetUniformLocation(p, name.as_ptr()) }
}

#[test]
fn height_map_links_and_draws() {
    let _g = context();
    let (fbo, tex) = target(8, 8);
    let p = program(HEIGHT_VS, HEIGHT_FS, &[c"a_pos", c"a_uv"]);
    // A ramp: the height rises left to right.
    let h = texture(GL_TEXTURE0, &[0, 0, 0, 255, 255, 0, 0, 255, 0, 0, 0, 255, 255, 0, 0, 255]);
    // SAFETY: the program in use has these uniforms.
    unsafe {
        glUniform1i(loc(p, c"u_height"), 0);
    }
    let (de, re, s) = draw(&UV, 2);
    assert_eq!((de, re), (GL_NO_ERROR, GL_NO_ERROR));
    assert_eq!(s.num_textures, 1, "one sampler, two look-ups");
    assert_eq!((s.tex[0].1, s.tex[0].2, s.tex[0].3), (2, 2, 0));
    assert_eq!(s.mali, -ENODEV, "the Mali takes the draw");
    finish(fbo, tex, &[h]);
}

#[test]
fn normal_map_links_and_draws() {
    let _g = context();
    let (fbo, tex) = target(8, 8);
    let p = program(HEIGHT_VS, NORMAL_FS, &[c"a_pos", c"a_uv"]);
    // Flat (0,0,1) and tilted (+x) normal maps.
    let flat = texture(GL_TEXTURE0, &[128, 128, 255, 255, 128, 128, 255, 255, 128, 128, 255, 255, 128, 128, 255, 255]);
    // SAFETY: the program in use has these uniforms.
    unsafe {
        glUniform1i(loc(p, c"u_normals"), 0);
        glUniform3f(loc(p, c"u_light"), 0.5, 0.0, 0.8);
    }
    let (de, re, s_flat) = draw(&UV, 2);
    assert_eq!((de, re), (GL_NO_ERROR, GL_NO_ERROR));
    assert_eq!(s_flat.num_textures, 1);
    assert_eq!(s_flat.mali, -ENODEV, "the Mali takes the draw");

    let tilt = texture(GL_TEXTURE0, &[255, 128, 160, 255, 255, 128, 160, 255, 255, 128, 160, 255, 255, 128, 160, 255]);
    let (de, re, s_tilt) = draw(&UV, 2);
    assert_eq!((de, re), (GL_NO_ERROR, GL_NO_ERROR));
    assert_eq!(s_tilt.mali, -ENODEV);
    assert_eq!((s_tilt.tex[0].1, s_tilt.tex[0].2, s_tilt.tex[0].3), (2, 2, 0));
    assert_ne!(s_flat.tex[0].4, s_tilt.tex[0].4, "the second normal map is another texture in the Mali's hands");
    finish(fbo, tex, &[flat, tilt]);
}

#[test]
fn high_poly_links_and_draws() {
    let _g = context();
    let (fbo, tex) = target(8, 8);
    let p = program(POLY_VS, POLY_FS, &[c"a_pos", c"a_nrm"]);
    // SAFETY: the program in use has this uniform.
    unsafe { glUniform3f(loc(p, c"u_light"), 0.0, 0.0, 1.0) };
    let (de, re, s) = draw(&NRM, 3);
    assert_eq!((de, re), (GL_NO_ERROR, GL_NO_ERROR));
    assert_eq!(s.num_textures, 0);
    assert_eq!(s.mali, -ENODEV, "the Mali takes the draw");
    finish(fbo, tex, &[]);
}
