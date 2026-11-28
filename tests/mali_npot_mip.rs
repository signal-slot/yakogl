// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! D-NPOTMIP (plan #367, issues #32 and #20): the descriptor the Mali back end
//! builds for refract's non-power-of-two mipmapped texture, and what it still
//! refuses (docs/notes/mali-npot-mip.md).
//!
//! The GL core is driven through the API into the stand-in back end of
//! `tests/mali_npot_mip_gl.rs`, which keeps the draw's texture binding;
//! `yakogl::mali::backend::host_texture_of` then describes it as `mali_draw`
//! does on the board (on the host `mali_draw` returns `-ENODEV` before it
//! builds any texture).
//!
//! What it pins:
//!
//! - glmark2's refract texture, 1600x1200 RGBA with glGenerateMipmap: 11
//!   levels (floor rule, 1600 .. 1), trilinear, every level in place at
//!   level 0's pitch on a 64-byte boundary, each level `max(w >> l, 1)` wide;
//! - a power-of-two chain describes the same way (the control);
//! - the NPOT chain of a 2D array is still refused by name
//!   (`NOT_BUILT_TEXTURE_NPOT_CHAIN`), a cube map's stays one level.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_npot_mip
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
use yakogl::mali::arena::Pool;
use yakogl::mali::backend::{host_texture_of, mali_draw, yakogl_backend_mali};
use yakogl::mali::frame::{FrameError, SampledTexture};
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
#[derive(Clone, Copy)]
struct Seen {
    bind: GlesTextureBind,
    mali: i32,
}

// SAFETY: the binding's pointer is only read on the test's own thread, under LOCK.
unsafe impl Send for Seen {}

static SEEN: Mutex<Vec<Seen>> = Mutex::new(Vec::new());

unsafe extern "C" fn b_draw(d: *const GlesDraw) -> i32 {
    let d = &*d;
    if d.index_count == 0 || d.num_textures == 0 {
        return 0;
    }
    let t = &d.textures[0];
    let mali = mali_draw(d);
    SEEN.lock().unwrap().push(Seen { bind: *t, mali });
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

/// Draw one triangle with the bound texture on unit 0; what the back end was
/// given.
fn draw() -> Seen {
    SEEN.lock().unwrap().clear();
    // SAFETY: `FULL` is static and outlives the draw.
    unsafe {
        glBindBuffer(GL_ARRAY_BUFFER, 0);
        glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, FULL.as_ptr() as *const c_void);
        glEnableVertexAttribArray(0);
        glDrawArrays(GL_TRIANGLES, 0, 3);
        glFinish();
    }
    assert_eq!(err(), GL_NO_ERROR);
    let s = core::mem::take(&mut *SEEN.lock().unwrap());
    let s = *s.last().expect("the draw reached the back end");
    assert_eq!(s.mali, -ENODEV, "the Mali takes the draw");
    s
}

/// A RGBA texture of `w` x `h`, trilinear, with glGenerateMipmap, on unit 0.
fn mipmapped(w: i32, h: i32) -> GLuint {
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
    assert_eq!(err(), GL_NO_ERROR, "glGenerateMipmap");
    tex
}

fn describe(s: &Seen) -> Result<SampledTexture, FrameError> {
    let mut mem = vec![0u8; 1 << 20];
    let mut pool = Pool::new("data", 0x4000_0000, &mut mem).unwrap();
    host_texture_of(&s.bind, &mut pool)
}

/// Every level in place: level 0 at `base_gpu`, the next on the next 64-byte
/// boundary (of the storage) after the rows of the one before, all on level 0's pitch, each
/// `max(w >> l, 1)` wide and `max(h >> l, 1)` high.
fn check_chain(s: &Seen, w: u32, h: u32, levels: u32) {
    let t = describe(s).expect("the Mali describes the texture");
    assert_eq!((t.width, t.height, t.levels), (w, h, levels));
    assert_eq!(t.mipmap, yakogl::mali::genxml::v10::MipmapMode::TRILINEAR);
    let pitch = s.bind.stride_texels * 4;
    assert_eq!(pitch % 64, 0);
    /* the stand-in's allocator is 16-byte aligned: offsets are the GL core's */
    let base = s.bind.base_gpu;
    let mut at = 0u64;
    for l in 0..levels as usize {
        assert_eq!(t.level[l].gpu - base, at, "level {l}");
        assert_eq!(t.level[l].row_stride, pitch, "level {l} keeps level 0's pitch");
        let rows = (h >> l).max(1) as u64;
        at = (at + rows * u64::from(pitch)).next_multiple_of(64);
    }
}

/// refract's colour texture at glmark2's 800x600 canvas.
#[test]
fn refracts_npot_texture_keeps_all_11_levels() {
    let _g = context();
    program();
    let tex = mipmapped(1600, 1200);
    let s = draw();
    assert_eq!((s.bind.mip_levels, s.bind.mip_filter, s.bind.incomplete), (11, 2, 0));
    check_chain(&s, 1600, 1200, 11);
    // SAFETY: a name this test made.
    unsafe { glDeleteTextures(1, &tex) };
}

/// A small NPOT chain whose levels fall to 1 on one side first.
#[test]
fn a_tall_npot_chain_runs_to_1x1() {
    let _g = context();
    program();
    let tex = mipmapped(24, 40);
    let s = draw();
    assert_eq!(s.bind.mip_levels, 6, "40, 20, 10, 5, 2, 1");
    check_chain(&s, 24, 40, 6);
    // SAFETY: a name this test made.
    unsafe { glDeleteTextures(1, &tex) };
}

/// The control: the same shape a power of two.
#[test]
fn a_pot_chain_describes_the_same_way() {
    let _g = context();
    program();
    let tex = mipmapped(1024, 512);
    let s = draw();
    assert_eq!(s.bind.mip_levels, 11);
    check_chain(&s, 1024, 512, 11);
    // SAFETY: a name this test made.
    unsafe { glDeleteTextures(1, &tex) };
}

/// A stack of layers is not walked: a Mali binding of an NPOT array with a
/// chain is refused by name.
#[test]
fn an_npot_array_chain_is_refused_by_name() {
    let _g = context();
    program();
    let tex = mipmapped(1600, 1200);
    let mut s = draw();
    s.bind.type_ = GLES_TEXTYPE_2D_ARRAY;
    s.bind.layers = 2;
    s.bind.height = 2400;
    match describe(&s) {
        Err(FrameError::NotBuilt(why)) => assert_eq!(why, yakogl::mali::backend::NOT_BUILT_TEXTURE_NPOT_CHAIN),
        Err(e) => panic!("another refusal: {e:?}"),
        Ok(_) => panic!("an NPOT array chain was described"),
    }
    // SAFETY: a name this test made.
    unsafe { glDeleteTextures(1, &tex) };
}
