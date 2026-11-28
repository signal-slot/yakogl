// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GL-level host test: glmark2's texture scene on the Mali back end, with its
//! three texture-filter settings (texture-filter=nearest, linear, mipmap).
//! A POT RGBA and a POT RGB texture are uploaded, a minified quad is drawn
//! with each min/mag filter, and what the Mali is handed is checked against
//! the filter: the level count, `mip_filter` (0 none, 1 nearest level, 2
//! between levels), `linear_filter` (mag), `min_linear`, `no_alpha` and
//! `incomplete`.
//!
//! Levels are distinct solid colours, uploaded one `glTexImage2D` per level
//! (and read back through the GL core, which is where a level's texels live
//! on the host). The host has no Mali OSAL, so the stand-in back end hands the
//! draw to `mali_draw`, which ends at `-ENODEV`: no shader executes and no
//! pixel is sampled. The consistency checked is therefore the draw's
//! texture state (which level set and which filters the Mali would sample
//! with), not shaded pixels. Shaders are written for this test.
//!
//! POT only; NPOT chains are `mali_npot_mip_gl.rs` and `mali_refract_draw.rs`.
//! A tools/check.sh row (the Mali GL-on-mock step).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_glmark2_texture_filters
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
    linear_filter: u8,
    min_linear: u8,
    no_alpha: u8,
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
        linear_filter: t.linear_filter,
        min_linear: t.min_linear,
        no_alpha: t.no_alpha,
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

const VS: &str = "attribute vec3 a_pos;
attribute vec2 a_uv;
varying mediump vec2 v_uv;
void main()
{
    gl_Position = vec4(a_pos, 1.0);
    v_uv = a_uv;
}
";
const FS: &str = "precision mediump float;
uniform sampler2D u_tex;
varying mediump vec2 v_uv;
void main()
{
    gl_FragColor = texture2D(u_tex, v_uv);
}
";

static POS: [f32; 18] =
    [-1.0, -1.0, 0.0, 1.0, -1.0, 0.0, -1.0, 1.0, 0.0, -1.0, 1.0, 0.0, 1.0, -1.0, 0.0, 1.0, 1.0, 0.0];
/// A coordinate range of 0..1 over an 8x8 target of a 64x64 texture: minified 8:1.
static UV: [f32; 12] = [0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 1.0, 1.0, 0.0, 1.0, 1.0];

fn setup() -> (GLuint, GLuint) {
    let (mut fbo, mut tex) = (0, 0);
    // SAFETY: plain GL calls; room for one name each.
    unsafe {
        glGenTextures(1, &mut tex);
        glBindTexture(GL_TEXTURE_2D, tex);
        glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA as GLint, 8, 8, 0, GL_RGBA, GL_UNSIGNED_BYTE, core::ptr::null());
        glGenFramebuffers(1, &mut fbo);
        glBindFramebuffer(GL_FRAMEBUFFER, fbo);
        glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex, 0);
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE);
        glViewport(0, 0, 8, 8);
        let (v, f) = (shader(GL_VERTEX_SHADER, VS), shader(GL_FRAGMENT_SHADER, FS));
        let p = glCreateProgram();
        glAttachShader(p, v);
        glAttachShader(p, f);
        glBindAttribLocation(p, 0, c"a_pos".as_ptr());
        glBindAttribLocation(p, 1, c"a_uv".as_ptr());
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        assert_eq!(ok, 1, "link failed");
        glUseProgram(p);
        glUniform1i(glGetUniformLocation(p, c"u_tex".as_ptr()), 0);
    }
    assert_eq!(err(), GL_NO_ERROR, "set-up");
    (fbo, tex)
}

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

/// Distinct solid colour of level `l`.
fn colour(l: u32) -> [u8; 4] {
    [(40 * l + 20) as u8, (255 - 40 * l) as u8, (17 * l) as u8, 255]
}

/// A POT texture on unit 0 with the filters set, level 0 only.
fn texture(size: i32, fmt: GLenum, min: GLenum, mag: GLenum) -> GLuint {
    let mut tex = 0;
    // SAFETY: room for one name.
    unsafe {
        glGenTextures(1, &mut tex);
        glActiveTexture(GL_TEXTURE0);
        glBindTexture(GL_TEXTURE_2D, tex);
        glPixelStorei(GL_UNPACK_ALIGNMENT, 1);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, min as GLint);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, mag as GLint);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE as GLint);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE as GLint);
    }
    upload(size, fmt, 0);
    tex
}

/// One level of the chain as a solid colour; level `l` is `size >> l` square.
fn upload(size: i32, fmt: GLenum, l: u32) {
    let n = (size >> l).max(1);
    let c = colour(l);
    let bpp = if fmt == GL_RGB { 3 } else { 4 };
    let mut texels = Vec::new();
    for _ in 0..n * n {
        texels.extend_from_slice(&c[..bpp]);
    }
    // SAFETY: the texels cover the level.
    unsafe {
        glTexImage2D(GL_TEXTURE_2D, l as GLint, fmt as GLint, n, n, 0, fmt, GL_UNSIGNED_BYTE, texels.as_ptr().cast())
    };
    assert_eq!(err(), GL_NO_ERROR, "level {l}");
}

/// The whole chain by hand, one solid colour per level.
fn upload_chain(size: i32, fmt: GLenum) {
    for l in 0..=size.ilog2() {
        upload(size, fmt, l);
    }
}

/// Read level 0 back through the GL core (a framebuffer on the texture).
fn level0(tex: GLuint) -> [u8; 4] {
    let mut px = [0u8; 4];
    let mut fbo = 0;
    // SAFETY: names made here; one pixel into `px`.
    unsafe {
        glGenFramebuffers(1, &mut fbo);
        glBindFramebuffer(GL_FRAMEBUFFER, fbo);
        glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex, 0);
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE);
        glReadPixels(0, 0, 1, 1, GL_RGBA, GL_UNSIGNED_BYTE, px.as_mut_ptr().cast());
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glDeleteFramebuffers(1, &fbo);
    }
    assert_eq!(err(), GL_NO_ERROR, "read-back");
    px
}

const SIZE: i32 = 64;
const CHAIN: u8 = 7; // 64, 32, .. 1

/// glmark2 texture-filter=nearest: NEAREST / NEAREST, no mipmaps.
#[test]
fn nearest_has_no_mip_filter() {
    let _g = context();
    let (fbo, tgt) = setup();
    for fmt in [GL_RGBA, GL_RGB] {
        let tex = texture(SIZE, fmt, GL_NEAREST, GL_NEAREST);
        let (e, s) = draw();
        assert_eq!(e, GL_NO_ERROR);
        assert_eq!((s.type_, s.width, s.incomplete), (GLES_TEXTYPE_2D, SIZE as u32, 0));
        assert_eq!((s.linear_filter, s.min_linear, s.mip_filter), (0, 0, 0), "nearest, nearest, no mip filter");
        assert_eq!(s.no_alpha, (fmt == GL_RGB) as u8);
        assert_eq!(s.mali, -ENODEV, "the Mali takes the draw");
        assert_eq!(level0(tex)[..3], colour(0)[..3], "level 0 texels");
        // SAFETY: a name this test made.
        unsafe { glDeleteTextures(1, &tex) };
    }
    // SAFETY: names this test made.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glDeleteFramebuffers(1, &fbo);
        glDeleteTextures(1, &tgt);
    }
}

/// glmark2 texture-filter=linear: LINEAR / LINEAR, no mipmaps.
#[test]
fn linear_has_no_mip_filter() {
    let _g = context();
    let (fbo, tgt) = setup();
    for fmt in [GL_RGBA, GL_RGB] {
        let tex = texture(SIZE, fmt, GL_LINEAR, GL_LINEAR);
        let (e, s) = draw();
        assert_eq!(e, GL_NO_ERROR);
        assert_eq!(s.incomplete, 0);
        assert_eq!((s.linear_filter, s.min_linear, s.mip_filter), (1, 1, 0), "linear, linear, no mip filter");
        assert_eq!(s.mali, -ENODEV);
        // SAFETY: a name this test made.
        unsafe { glDeleteTextures(1, &tex) };
    }
    // SAFETY: names this test made.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glDeleteFramebuffers(1, &fbo);
        glDeleteTextures(1, &tgt);
    }
}

/// glmark2 texture-filter=mipmap: LINEAR_MIPMAP_LINEAR / LINEAR, the chain
/// from glGenerateMipmap.
#[test]
fn mipmap_generated_chain_is_trilinear() {
    let _g = context();
    let (fbo, tgt) = setup();
    for fmt in [GL_RGBA, GL_RGB] {
        let tex = texture(SIZE, fmt, GL_LINEAR_MIPMAP_LINEAR, GL_LINEAR);
        // SAFETY: plain GL call.
        unsafe { glGenerateMipmap(GL_TEXTURE_2D) };
        assert_eq!(err(), GL_NO_ERROR);
        let (e, s) = draw();
        assert_eq!(e, GL_NO_ERROR);
        assert_eq!(s.incomplete, 0);
        assert_eq!((s.mip_levels, s.mip_filter), (CHAIN, 2), "full chain, trilinear");
        assert_eq!((s.linear_filter, s.min_linear), (1, 1));
        assert_eq!(s.no_alpha, (fmt == GL_RGB) as u8);
        assert_eq!(s.mali, -ENODEV);
        assert_eq!(level0(tex)[..3], colour(0)[..3], "glGenerateMipmap leaves level 0");
        // SAFETY: a name this test made.
        unsafe { glDeleteTextures(1, &tex) };
    }
    // SAFETY: names this test made.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glDeleteFramebuffers(1, &fbo);
        glDeleteTextures(1, &tgt);
    }
}

/// Every min filter over a chain whose levels are distinct solid colours
/// (one glTexImage2D each): the Mali is handed the whole chain and the
/// filter's own mip mode.
#[test]
fn each_min_filter_over_a_hand_built_chain() {
    let _g = context();
    let (fbo, tgt) = setup();
    let cases = [
        (GL_NEAREST, 0, 0),
        (GL_LINEAR, 1, 0),
        (GL_NEAREST_MIPMAP_NEAREST, 0, 1),
        (GL_LINEAR_MIPMAP_NEAREST, 1, 1),
        (GL_NEAREST_MIPMAP_LINEAR, 0, 2),
        (GL_LINEAR_MIPMAP_LINEAR, 1, 2),
    ];
    for fmt in [GL_RGBA, GL_RGB] {
        let tex = texture(SIZE, fmt, GL_NEAREST, GL_NEAREST);
        upload_chain(SIZE, fmt);
        for (min, min_linear, mip) in cases {
            // SAFETY: plain GL call.
            unsafe { glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, min as GLint) };
            let (e, s) = draw();
            assert_eq!(e, GL_NO_ERROR, "min {min:#x}");
            assert_eq!(s.incomplete, 0, "min {min:#x}");
            assert_eq!((s.min_linear, s.mip_filter), (min_linear, mip), "min {min:#x}");
            assert_eq!(s.linear_filter, 0, "mag nearest");
            if mip != 0 {
                assert_eq!(s.mip_levels, CHAIN, "min {min:#x}: every level handed over");
            }
            assert_eq!(s.mali, -ENODEV);
        }
        // SAFETY: plain GL call.
        unsafe { glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_LINEAR as GLint) };
        let (_, s) = draw();
        assert_eq!(s.linear_filter, 1, "mag linear");
        assert_eq!(level0(tex)[..3], colour(0)[..3]);
        // SAFETY: a name this test made.
        unsafe { glDeleteTextures(1, &tex) };
    }
    // SAFETY: names this test made.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glDeleteFramebuffers(1, &fbo);
        glDeleteTextures(1, &tgt);
    }
}

/// A mipmap min filter with only level 0 specified is GL's incomplete
/// texture: the Mali samples opaque black, whatever level 0 holds.
#[test]
fn mip_filter_without_a_chain_is_incomplete() {
    let _g = context();
    let (fbo, tgt) = setup();
    let tex = texture(SIZE, GL_RGBA, GL_LINEAR_MIPMAP_LINEAR, GL_LINEAR);
    let (e, s) = draw();
    assert_eq!(e, GL_NO_ERROR);
    assert_eq!(s.incomplete, 1, "level 0 alone under a mipmap filter");
    // glGenerateMipmap completes it.
    // SAFETY: plain GL call.
    unsafe { glGenerateMipmap(GL_TEXTURE_2D) };
    let (e, s) = draw();
    assert_eq!(e, GL_NO_ERROR);
    assert_eq!((s.incomplete, s.mip_levels), (0, CHAIN));
    // SAFETY: names this test made.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glDeleteFramebuffers(1, &fbo);
        glDeleteTextures(1, &tgt);
        glDeleteTextures(1, &tex);
    }
}
