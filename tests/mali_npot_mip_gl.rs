// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! glmark2's refract scene builds a mipmapped texture of non-power-of-two
//! (NPOT) size with glGenerateMipmap and samples it with
//! GL_LINEAR_MIPMAP_LINEAR; on the Mali (Valhall G610) back end the chain
//! keeps one level (cause D-NPOTMIP, plan #367, issues #32 and #20).
//!
//! The texture is `RefractRenderTarget`'s colour texture in
//! `scene-refract.cpp` (glmark2 22c527cb: `width_ = canvas_.width() * 2`,
//! `height_ = canvas_.height() * 2`, GL_LINEAR_MIPMAP_LINEAR, RGBA, mipmaps
//! from glGenerateMipmap when the extension is there, line 429): at glmark2's
//! default 800x600 canvas that is 1600x1200. Both sides are NPOT.
//!
//! The GL core is driven through the API into a stand-in back end whose draw
//! entry records the sampled binding and runs the Mali's own `mali_draw`
//! (the same stand-in as `tests/mali_3d_levels.rs`; no Mali OSAL on the
//! host, a draw it takes ends at `-ENODEV`). The stand-in copies every field
//! of the real Mali table except the entries that need hardware, so the GL
//! core sees the Mali's `mip_level_align` and the absent `generate_mipmap`
//! hook.
//!
//! What it pins:
//!
//! - the control, a power-of-two 1024x512 texture: GL_NO_ERROR, complete, the
//!   full chain of 11 levels (floor(log2(1024)) + 1) and the trilinear mip
//!   filter reach the Mali, which takes the draw;
//! - the NPOT 1600x1200 texture as it is today: no GL error, complete, the
//!   draw is taken, and the Mali is given level 0 alone with no mip filter
//!   (a picture difference only: no error anywhere);
//! - `#[ignore]`d, what panfrost does: the chain of 1600x1200 runs down to
//!   1x1 by the floor rule, 11 levels (1600, 800, 400, 200, 100, 50, 25, 12,
//!   6, 3, 1 wide), so the Mali's binding must carry 11 levels and the
//!   trilinear mip filter.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_npot_mip_gl
//! ```

#![cfg(all(feature = "host-test", feature = "shaderc-online", feature = "gpu-mali", not(feature = "report-es2")))]

use core::ffi::{c_char, c_void};
use std::ffi::CString;
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{self, GlesDraw, GlesHeap, YakoglBackend, *};
use yakogl::gl::buffer::glBindBuffer;
use yakogl::gl::consts::*;
use yakogl::gl::draw::*;
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
    if d.index_count == 0 || d.num_textures == 0 {
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

const VERT: &str = "attribute vec4 a_position;
varying mediump vec2 v_uv;
void main()
{
    gl_Position = a_position;
    v_uv = a_position.xy * 0.5 + 0.5;
}
";
const FRAG: &str = "precision mediump float;
uniform sampler2D u_tex;
varying mediump vec2 v_uv;
void main()
{
    gl_FragColor = texture2D(u_tex, v_uv);
}
";

fn program() {
    let (v, f) = (shader(GL_VERTEX_SHADER, VERT), shader(GL_FRAGMENT_SHADER, FRAG));
    // SAFETY: plain GL calls.
    unsafe {
        let p = glCreateProgram();
        glAttachShader(p, v);
        glAttachShader(p, f);
        glBindAttribLocation(p, 0, c"a_position".as_ptr());
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        assert_eq!(ok, 1, "link failed");
        glUseProgram(p);
        glUniform1i(glGetUniformLocation(p, c"u_tex".as_ptr()), 0);
    }
}

/// Covers the whole target.
static FULL: [f32; 12] = [-1.0, -1.0, 0.0, 1.0, 3.0, -1.0, 0.0, 1.0, -1.0, 3.0, 0.0, 1.0];

/// Draw one triangle; the draw's `glGetError` and what the back end was given.
fn draw() -> (GLenum, Seen) {
    SEEN.lock().unwrap().clear();
    // SAFETY: `FULL` is static and outlives the draw.
    unsafe {
        glBindBuffer(GL_ARRAY_BUFFER, 0);
        glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, FULL.as_ptr() as *const c_void);
        glEnableVertexAttribArray(0);
        glDrawArrays(GL_TRIANGLES, 0, 3);
        glFinish();
    }
    let e = err();
    let s = core::mem::take(&mut *SEEN.lock().unwrap());
    (e, *s.last().expect("the draw reached the back end"))
}

/// refract's colour texture, as glmark2 builds it: RGBA at `w` x `h`,
/// GL_LINEAR_MIPMAP_LINEAR / GL_LINEAR, then glGenerateMipmap. Bound to unit
/// 0. Returns the name and the glGenerateMipmap error.
fn mipmapped(w: i32, h: i32) -> (GLuint, GLenum) {
    let mut tex = 0;
    let texels = vec![0x80u8; (w * h * 4) as usize];
    // SAFETY: room for one name; the texels cover the image.
    unsafe {
        glGenTextures(1, &mut tex);
        glActiveTexture(GL_TEXTURE0);
        glBindTexture(GL_TEXTURE_2D, tex);
        glPixelStorei(GL_UNPACK_ALIGNMENT, 1);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_LINEAR_MIPMAP_LINEAR as GLint);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_LINEAR as GLint);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE as GLint);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE as GLint);
        glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA as GLint, w, h, 0, GL_RGBA, GL_UNSIGNED_BYTE, texels.as_ptr().cast());
        assert_eq!(err(), GL_NO_ERROR, "the image");
        glGenerateMipmap(GL_TEXTURE_2D);
    }
    (tex, err())
}

/// The levels of a full chain by the floor rule: floor(log2(max side)) + 1.
fn full_chain(w: u32, h: u32) -> u8 {
    (w.max(h).ilog2() + 1) as u8
}

/// Control: a power-of-two texture of the same shape gets its whole chain.
#[test]
fn a_pot_texture_gets_the_full_chain() {
    let _g = context();
    program();
    let (tex, e) = mipmapped(1024, 512);
    assert_eq!(e, GL_NO_ERROR);
    let (e, s) = draw();
    assert_eq!(e, GL_NO_ERROR);
    assert_eq!((s.type_, s.width, s.incomplete), (GLES_TEXTYPE_2D, 1024, 0));
    assert_eq!(full_chain(1024, 512), 11);
    assert_eq!((s.mip_levels, s.mip_filter), (11, 2), "11 levels, trilinear");
    assert_eq!(s.mali, -ENODEV, "the Mali takes the draw");
    // SAFETY: a name this test made.
    unsafe { glDeleteTextures(1, &tex) };
}

/// refract's 1600x1200 as the Mali gets it today: nothing is refused or
/// raised, the texture is complete, and the Mali samples level 0 alone.
#[test]
fn an_npot_texture_is_complete_and_drawn_with_level_0_only_today() {
    let _g = context();
    program();
    let (tex, e) = mipmapped(1600, 1200);
    assert_eq!(e, GL_NO_ERROR, "glGenerateMipmap raises nothing (ES 3.0 / OES_texture_npot)");
    let (e, s) = draw();
    assert_eq!(e, GL_NO_ERROR, "the draw raises nothing");
    assert_eq!((s.type_, s.width, s.incomplete), (GLES_TEXTYPE_2D, 1600, 0), "complete");
    assert_eq!(s.mali, -ENODEV, "the Mali takes the draw");
    assert_eq!((s.mip_levels, s.mip_filter), (1, 0), "D-NPOTMIP today: level 0 alone, no mip filter");
    // SAFETY: a name this test made.
    unsafe { glDeleteTextures(1, &tex) };
}

/// What panfrost does: the whole chain of the NPOT image, down to 1x1.
#[test]
#[ignore = "D-NPOTMIP: the GL core lays no NPOT chain out for the Mali (npot_mip_chains, src/gl/texture.rs:1308, false for mip_level_align != 0) and the Mali refuses one it was given (NOT_BUILT_TEXTURE_NPOT_CHAIN, src/mali/backend.rs:2337), so refract's 1600x1200 keeps one level"]
fn an_npot_texture_gets_the_full_chain_as_panfrost_builds_it() {
    let _g = context();
    program();
    let (tex, e) = mipmapped(1600, 1200);
    assert_eq!(e, GL_NO_ERROR);
    let (e, s) = draw();
    assert_eq!(e, GL_NO_ERROR);
    assert_eq!(full_chain(1600, 1200), 11, "1600, 800, 400, 200, 100, 50, 25, 12, 6, 3, 1");
    assert_eq!((s.mip_levels, s.mip_filter), (11, 2), "11 levels, trilinear");
    assert_eq!(s.mali, -ENODEV, "the Mali takes the draw");
    // SAFETY: a name this test made.
    unsafe { glDeleteTextures(1, &tex) };
}
