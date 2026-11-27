// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GL-level host test: vertex texture limits and a vertex texture look-up on
//! the Mali back end (plan #367; glmark2 terrain samples a height map in its
//! vertex shader, TEX_VERTEX, vs 18bc).
//!
//! - GL_MAX_VERTEX_TEXTURE_IMAGE_UNITS and GL_MAX_COMBINED_TEXTURE_IMAGE_UNITS
//!   are queried and pinned.
//! - A minimal vertex shader that displaces a vertex by `texture2D(height,
//!   uv).r` (one look-up, then two as terrain does) is compiled and linked
//!   with a plain fragment shader and drawn into an RGBA8 framebuffer object.
//!
//! Level reached: compile, link, draw submission. The host has no Mali OSAL,
//! so the stand-in back end hands the draw to `mali_draw`, which ends at
//! `-ENODEV`; no shader executes and no vertex position is produced, so
//! read-back positions cannot be checked on the host (the read raises no GL
//! error; the board run checks the pixels). Both shapes are accepted now
//! (TEX_VERTEX lowered); a new refusal fails the test with its log, which
//! names TEX_VERTEX.
//!
//! Not a tools/check.sh row.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_glmark2_vertex_texture
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
static UV: [f32; 12] = [0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 1.0, 1.0, 0.0, 1.0, 1.0];

/// Compile one shader; Err carries the info log of a refusal.
fn try_shader(ty: GLenum, text: &str) -> Result<GLuint, String> {
    let text = CString::new(text).unwrap();
    // SAFETY: plain GL calls; one NUL-terminated source.
    unsafe {
        let s = glCreateShader(ty);
        let p: [*const c_char; 1] = [text.as_ptr()];
        glShaderSource(s, 1, p.as_ptr(), core::ptr::null());
        glCompileShader(s);
        let mut ok: GLint = 0;
        glGetShaderiv(s, GL_COMPILE_STATUS, &mut ok);
        if ok == 1 {
            return Ok(s);
        }
        let mut log = [0u8; 512];
        let mut n: GLsizei = 0;
        glGetShaderInfoLog(s, log.len() as GLsizei, &mut n, log.as_mut_ptr().cast());
        Err(String::from_utf8_lossy(&log[..n as usize]).into_owned())
    }
}

/// Compile and link a pair; Err carries the first refusal's log.
fn try_program(vs: &str, fs: &str) -> Result<GLuint, String> {
    let v = try_shader(GL_VERTEX_SHADER, vs).map_err(|e| format!("vs: {e}"))?;
    let f = try_shader(GL_FRAGMENT_SHADER, fs).map_err(|e| format!("fs: {e}"))?;
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
            let mut log = [0u8; 512];
            let mut n: GLsizei = 0;
            glGetProgramInfoLog(p, log.len() as GLsizei, &mut n, log.as_mut_ptr().cast());
            return Err(format!("link: {}", String::from_utf8_lossy(&log[..n as usize])));
        }
        glUseProgram(p);
        Ok(p)
    }
}

/// A 2x2 height map in unit 0 (and, for two look-ups, unit 1).
fn height_maps(n: usize) -> Vec<GLuint> {
    let texels = [0x00u8, 0, 0, 0xff, 0x80, 0, 0, 0xff, 0xff, 0, 0, 0xff, 0x40, 0, 0, 0xff];
    let mut v = Vec::new();
    for i in 0..n {
        let mut t = 0;
        // SAFETY: room for one name; the texels cover 2x2 RGBA.
        unsafe {
            glGenTextures(1, &mut t);
            glActiveTexture(GL_TEXTURE0 + i as GLenum);
            glBindTexture(GL_TEXTURE_2D, t);
            glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST as GLint);
            glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST as GLint);
            glTexImage2D(
                GL_TEXTURE_2D,
                0,
                GL_RGBA as GLint,
                2,
                2,
                0,
                GL_RGBA,
                GL_UNSIGNED_BYTE,
                texels.as_ptr().cast(),
            );
        }
        v.push(t);
    }
    v
}

const FS: &str = "precision mediump float;
varying mediump float v_h;
void main()
{
    gl_FragColor = vec4(v_h, 0.0, 0.0, 1.0);
}
";

/// One look-up: a displacement along y by the height's red channel.
const VS1: &str = "attribute vec3 a_pos;
attribute vec2 a_uv;
uniform sampler2D u_height;
varying mediump float v_h;
void main()
{
    float h = texture2D(u_height, a_uv).r;
    gl_Position = vec4(a_pos.x, a_pos.y + h * 0.25, a_pos.z, 1.0);
    v_h = h;
}
";

/// Two look-ups, as terrain's vertex shader has: two height maps summed.
const VS2: &str = "attribute vec3 a_pos;
attribute vec2 a_uv;
uniform sampler2D u_height;
uniform sampler2D u_detail;
varying mediump float v_h;
void main()
{
    float h = texture2D(u_height, a_uv).r + texture2D(u_detail, a_uv * 2.0).r;
    gl_Position = vec4(a_pos.x, a_pos.y + h * 0.25, a_pos.z, 1.0);
    v_h = h;
}
";

fn geti(pname: GLenum) -> GLint {
    let mut v: GLint = -1;
    // SAFETY: one int out.
    unsafe { glGetIntegerv(pname, &mut v) };
    v
}

#[test]
fn vertex_texture_limits() {
    let _g = context();
    // Pinned from the back end's caps today; a change is deliberate.
    assert_eq!(geti(GL_MAX_VERTEX_TEXTURE_IMAGE_UNITS), VTX_UNITS, "GL_MAX_VERTEX_TEXTURE_IMAGE_UNITS");
    assert_eq!(geti(GL_MAX_COMBINED_TEXTURE_IMAGE_UNITS), COMBINED_UNITS, "GL_MAX_COMBINED_TEXTURE_IMAGE_UNITS");
    assert_eq!(err(), GL_NO_ERROR);
}

/// Compile, link and draw `vs`; `units` height maps are bound. Returns None
/// when the Mali refuses (the refusal is Err), else the draw's checks run.
fn run(vs: &str, units: usize, uniforms: &[&core::ffi::CStr]) -> Result<(), String> {
    let (fbo, tex) = target(8, 8);
    let p = match try_program(vs, FS) {
        Ok(p) => p,
        Err(e) => {
            // SAFETY: names this test made.
            unsafe {
                glBindFramebuffer(GL_FRAMEBUFFER, 0);
                glDeleteFramebuffers(1, &fbo);
                glDeleteTextures(1, &tex);
            }
            return Err(e);
        }
    };
    let maps = height_maps(units);
    let mut px = [0u8; 4];
    // SAFETY: statics and locals; the program has these uniforms.
    unsafe {
        for (i, u) in uniforms.iter().enumerate() {
            glUniform1i(glGetUniformLocation(p, u.as_ptr()), i as GLint);
        }
        glBindBuffer(GL_ARRAY_BUFFER, 0);
        glVertexAttribPointer(0, 3, GL_FLOAT, 0, 0, POS.as_ptr() as *const c_void);
        glEnableVertexAttribArray(0);
        glVertexAttribPointer(1, 2, GL_FLOAT, 0, 0, UV.as_ptr() as *const c_void);
        glEnableVertexAttribArray(1);
    }
    assert_eq!(err(), GL_NO_ERROR, "set-up");
    SEEN.lock().unwrap().clear();
    // SAFETY: as above; `px` holds one pixel.
    unsafe {
        glDrawArrays(GL_TRIANGLES, 0, 6);
        glFinish();
    }
    assert_eq!(err(), GL_NO_ERROR, "draw");
    // SAFETY: one pixel into `px`.
    unsafe { glReadPixels(4, 4, 1, 1, GL_RGBA, GL_UNSIGNED_BYTE, px.as_mut_ptr().cast()) };
    assert_eq!(err(), GL_NO_ERROR, "read");
    let s = SEEN.lock().unwrap().last().copied().expect("the draw reached the back end");
    assert_eq!(s.mali, -ENODEV, "the Mali takes the draw");
    // SAFETY: names this test made.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glDeleteFramebuffers(1, &fbo);
        glDeleteTextures(1, &tex);
        glDeleteTextures(maps.len() as GLsizei, maps.as_ptr());
        glActiveTexture(GL_TEXTURE0);
    }
    Ok(())
}

#[test]
fn one_vertex_lookup() {
    let _g = context();
    // TEX_VERTEX is lowered (lower-tex-vertex): the shape compiles, links and
    // draws. Were it refused again, the log would name TEX_VERTEX.
    run(VS1, 1, &[c"u_height"]).unwrap_or_else(|e| panic!("TEX_VERTEX: one vertex look-up refused: {e}"));
}

#[test]
fn two_vertex_lookups_as_terrain() {
    let _g = context();
    run(VS2, 2, &[c"u_height", c"u_detail"]).unwrap_or_else(|e| panic!("TEX_VERTEX: two vertex look-ups refused: {e}"));
}

/// The pinned limits: 16 vertex units, 32 combined (vertex plus fragment).
const VTX_UNITS: GLint = 16;
const COMBINED_UNITS: GLint = 32;
