// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GL-level host test of glGenerateMipmap on power-of-two textures through
//! the Mali back end, as glmark2-es2's texture scene (texture-filter=mipmap)
//! and others use it: a POT RGBA or RGB image, glGenerateMipmap, then a
//! mipmap minification filter. NPOT chains are D-NPOTMIP's
//! (`tests/mali_npot_mip_gl.rs`); this file stays with POT sizes.
//!
//! The stand-in back end (as in `tests/mali_glmark2_scene_draw.rs`) hands a
//! draw to `mali_draw`, which ends at `-ENODEV`; no shader executes on the
//! host, so a drawn pixel is not read. What is checked instead:
//!
//! - the texture is complete under GL_LINEAR_MIPMAP_LINEAR and
//!   GL_NEAREST_MIPMAP_NEAREST and reaches the Mali with the full chain of
//!   levels and the matching mip filter (2 trilinear, 1 nearest level);
//! - the generated chain itself, read back through a framebuffer attached to
//!   level n: a two-colour checkerboard (black and white texels) averages to
//!   mid grey on every level below 0, not level 0's texels (the colour a
//!   quad small enough to select a low level samples);
//! - glGenerateMipmap after glTexSubImage2D on level 0 refreshes the chain;
//! - the same for an RGB upload.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_glmark2_pot_mipmap
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
        glUseProgram(p);
        p
    }
}

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

static POS: [f32; 18] =
    [-1.0, -1.0, 0.0, 1.0, -1.0, 0.0, -1.0, 1.0, 0.0, -1.0, 1.0, 0.0, 1.0, -1.0, 0.0, 1.0, 1.0, 0.0];
static UV: [f32; 12] = [0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 1.0, 1.0, 0.0, 1.0, 1.0];

/// Draw the textured quad into the bound framebuffer; returns the draw's GL
/// error and what the Mali was handed for unit 0.
fn draw() -> (GLenum, Seen) {
    SEEN.lock().unwrap().clear();
    // SAFETY: the arrays are static.
    unsafe {
        glBindBuffer(GL_ARRAY_BUFFER, 0);
        glVertexAttribPointer(0, 3, GL_FLOAT, 0, 0, POS.as_ptr() as *const c_void);
        glEnableVertexAttribArray(0);
        glVertexAttribPointer(1, 2, GL_FLOAT, 0, 0, UV.as_ptr() as *const c_void);
        glEnableVertexAttribArray(1);
        glDrawArrays(GL_TRIANGLES, 0, 6);
        glFinish();
    }
    let e = err();
    let s = core::mem::take(&mut *SEEN.lock().unwrap());
    (e, *s.last().expect("the draw reached the back end"))
}

/// A `size` x `size` checkerboard of single black and white texels, with
/// `n` bytes per texel (4 RGBA, 3 RGB).
fn checker(size: usize, n: usize) -> Vec<u8> {
    let mut v = Vec::with_capacity(size * size * n);
    for y in 0..size {
        for x in 0..size {
            let c = if (x + y) % 2 == 0 { 255 } else { 0 };
            v.extend_from_slice(&[c, c, c, 255][..n]);
        }
    }
    v
}

/// Upload `texels` as level 0 of a new `size` x `size` texture on unit 0
/// with the given min filter; no error is allowed.
fn upload(size: i32, fmt: GLenum, texels: &[u8], min: GLenum) -> GLuint {
    let mut tex = 0;
    // SAFETY: room for one name; the texels cover size x size of `fmt`.
    unsafe {
        glPixelStorei(GL_UNPACK_ALIGNMENT, 1);
        glGenTextures(1, &mut tex);
        glActiveTexture(GL_TEXTURE0);
        glBindTexture(GL_TEXTURE_2D, tex);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, min as GLint);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_LINEAR as GLint);
        glTexImage2D(GL_TEXTURE_2D, 0, fmt as GLint, size, size, 0, fmt, GL_UNSIGNED_BYTE, texels.as_ptr().cast());
    }
    assert_eq!(err(), GL_NO_ERROR, "upload");
    tex
}

fn generate() {
    // SAFETY: plain GL call.
    unsafe { glGenerateMipmap(GL_TEXTURE_2D) };
    assert_eq!(err(), GL_NO_ERROR, "glGenerateMipmap");
}

/// Read the first texel of `level` of `tex` through a framebuffer.
fn level_texel(tex: GLuint, level: i32) -> [u8; 4] {
    let (mut fbo, mut px) = (0, [0u8; 4]);
    // SAFETY: room for one name and one RGBA8 pixel.
    unsafe {
        glGenFramebuffers(1, &mut fbo);
        glBindFramebuffer(GL_FRAMEBUFFER, fbo);
        glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex, level);
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE, "level {level}");
        glReadPixels(0, 0, 1, 1, GL_RGBA, GL_UNSIGNED_BYTE, px.as_mut_ptr().cast());
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glDeleteFramebuffers(1, &fbo);
    }
    assert_eq!(err(), GL_NO_ERROR, "reading level {level}");
    px
}

/// Mid grey, as a box filter of black and white gives (127 or 128 by rounding).
fn assert_grey(px: [u8; 4], what: &str) {
    for c in &px[..3] {
        assert!((126..=129).contains(c), "{what}: {px:?} is not the average of black and white");
    }
}

fn setup() -> GLuint {
    program(TEX_VS, TEX_FS, &[c"a_pos", c"a_uv"])
}

fn mip_draw(min: GLenum, filter_code: u8) {
    let _g = context();
    let p = setup();
    let tex = upload(64, GL_RGBA, &checker(64, 4), min);
    generate();
    // SAFETY: the program in use has u_tex.
    unsafe { glUniform1i(glGetUniformLocation(p, c"u_tex".as_ptr()), 0) };
    let (e, s) = draw();
    assert_eq!(e, GL_NO_ERROR, "the draw raises nothing");
    assert_eq!((s.type_, s.width, s.incomplete), (GLES_TEXTYPE_2D, 64, 0), "complete");
    assert_eq!((s.mip_levels, s.mip_filter), (7, filter_code), "64..1: 7 levels");
    assert_eq!(s.mali, -ENODEV, "the Mali takes the draw");
    // SAFETY: a name this test made.
    unsafe { glDeleteTextures(1, &tex) };
}

#[test]
fn linear_mipmap_linear_is_complete_with_the_full_chain() {
    mip_draw(GL_LINEAR_MIPMAP_LINEAR, 2);
}

#[test]
fn nearest_mipmap_nearest_is_complete_with_the_full_chain() {
    mip_draw(GL_NEAREST_MIPMAP_NEAREST, 1);
}

/// Without glGenerateMipmap the same texture is incomplete under a mipmap
/// filter (ES 2.0 3.7.10), so the Mali is told to read black.
#[test]
fn a_mipmap_filter_without_a_chain_is_incomplete() {
    let _g = context();
    setup();
    let tex = upload(64, GL_RGBA, &checker(64, 4), GL_LINEAR_MIPMAP_LINEAR);
    let (e, s) = draw();
    assert_eq!(e, GL_NO_ERROR);
    assert_eq!(s.incomplete, 1);
    // SAFETY: a name this test made.
    unsafe { glDeleteTextures(1, &tex) };
}

fn chain_is_averaged(fmt: GLenum, n: usize) {
    let _g = context();
    let tex = upload(16, fmt, &checker(16, n), GL_LINEAR_MIPMAP_LINEAR);
    generate();
    let l0 = level_texel(tex, 0);
    assert_eq!(l0[0], 255, "level 0 keeps the checkerboard's white texel");
    for level in 1..=4 {
        assert_grey(level_texel(tex, level), &format!("level {level}"));
    }
    // SAFETY: a name this test made.
    unsafe { glDeleteTextures(1, &tex) };
}

#[test]
fn the_rgba_chain_is_the_average_of_the_checkerboard() {
    chain_is_averaged(GL_RGBA, 4);
}

#[test]
fn the_rgb_chain_is_the_average_of_the_checkerboard() {
    chain_is_averaged(GL_RGB, 3);
}

#[test]
fn generate_after_a_sub_image_refreshes_the_chain() {
    let _g = context();
    let tex = upload(16, GL_RGBA, &checker(16, 4), GL_LINEAR_MIPMAP_LINEAR);
    generate();
    assert_grey(level_texel(tex, 2), "first chain");
    // Level 0 becomes all white; the old chain stays until regenerated.
    let white = vec![255u8; 16 * 16 * 4];
    // SAFETY: the texels cover 16x16 RGBA.
    unsafe { glTexSubImage2D(GL_TEXTURE_2D, 0, 0, 0, 16, 16, GL_RGBA, GL_UNSIGNED_BYTE, white.as_ptr().cast()) };
    assert_eq!(err(), GL_NO_ERROR, "glTexSubImage2D");
    assert_eq!(level_texel(tex, 0), [255; 4], "level 0 is white");
    assert_grey(level_texel(tex, 2), "before regenerating");
    generate();
    for level in 1..=4 {
        assert_eq!(level_texel(tex, level), [255; 4], "level {level} after regenerating");
    }
    // SAFETY: a name this test made.
    unsafe { glDeleteTextures(1, &tex) };
}
