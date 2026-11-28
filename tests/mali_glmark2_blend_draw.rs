// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GL-level host test: glmark2-style blended draws on the Mali back end
//! (plan #367). A flat-colour program with an alpha (as pulsar, jellyfish and
//! ideas draw their translucent layers) is drawn with the blend states glmark2
//! uses, `glBlendFunc(GL_SRC_ALPHA, GL_ONE_MINUS_SRC_ALPHA)` and additive
//! `GL_ONE, GL_ONE`, with the depth test on and off, into an RGBA8
//! framebuffer object with a 16-bit depth renderbuffer. The shaders are
//! written for this test.
//!
//! Checked: both shaders compile and the program links on the Mali compiler,
//! the framebuffer is complete, each draw raises no GL error and reaches the
//! Mali with the blend factor and equation codes (`SHC_BF_*`,
//! `SHC_BLEND_ADD`), the depth test and the depth write that the GL state
//! set, and the Mali takes it. Both blend states are fixed-function on the
//! Mali (`yakogl::mali::blend`, as panfrost: no blend shader).
//!
//! Pixels: the host has no Mali OSAL, so `mali_draw` takes the draw and ends
//! at `-ENODEV`; no shader executes and no pixel is written (the stand-in of
//! `tests/mali_glmark2_scene_draw.rs`). This is a host limit, not a back-end
//! defect. glReadPixels therefore returns the zeroed texture (the clear is not
//! run either), which is asserted as the current behaviour, beside the value the draw writes on a board,
//! computed by hand (`blend_src_alpha`, `blend_additive`) and checked here
//! against fixed numbers; the board session reads the real pixels.
//!
//! Not yet a tools/check.sh row (check-rows-367c adds it).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_glmark2_blend_draw
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
use yakogl::mali::blend::{self as mb, Blending, Channel, Factor, Func};
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};
use yakogl::shaderc::{SHC_BF_INV_SRC_ALPHA, SHC_BF_ONE, SHC_BF_SRC_ALPHA, SHC_BLEND_ADD};
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
    blend: bool,
    src_rgb: u8,
    dst_rgb: u8,
    src_a: u8,
    dst_a: u8,
    eq_rgb: u8,
    eq_a: u8,
    depth_test: bool,
    depth_write: bool,
    mali: i32,
}

static SEEN: Mutex<Vec<Seen>> = Mutex::new(Vec::new());

unsafe extern "C" fn b_draw(d: *const GlesDraw) -> i32 {
    let d = &*d;
    if d.index_count == 0 {
        return 0;
    }
    let mali = mali_draw(d);
    SEEN.lock().unwrap().push(Seen {
        blend: d.blend,
        src_rgb: d.blend_src_rgb,
        dst_rgb: d.blend_dst_rgb,
        src_a: d.blend_src_alpha,
        dst_a: d.blend_dst_alpha,
        eq_rgb: d.blend_eq_rgb,
        eq_a: d.blend_eq_alpha,
        depth_test: d.depth_test,
        depth_write: d.depth_write,
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

fn err() -> GLenum {
    // SAFETY: plain GL call.
    unsafe { glGetError() }
}

/// What glReadPixels returns on the host: the clear and the draw are both
/// handed to the stand-in back end, which writes nothing, so the freshly
/// allocated (zeroed) texture is read.
const UNWRITTEN: [u8; 4] = [0, 0, 0, 0];
const DST: [f32; 4] = [0.25, 0.5, 0.75, 1.0];

/// An RGBA8 texture and a 16-bit depth renderbuffer as a framebuffer's
/// attachments, cleared to `DST` and depth 1.0.
fn target(w: i32, h: i32) -> (GLuint, GLuint, GLuint) {
    let (mut fbo, mut tex, mut rb) = (0, 0, 0);
    // SAFETY: room for one name each; null texels allocate the image.
    unsafe {
        glGenTextures(1, &mut tex);
        glBindTexture(GL_TEXTURE_2D, tex);
        glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA as GLint, w, h, 0, GL_RGBA, GL_UNSIGNED_BYTE, core::ptr::null());
        glGenRenderbuffers(1, &mut rb);
        glBindRenderbuffer(GL_RENDERBUFFER, rb);
        glRenderbufferStorage(GL_RENDERBUFFER, GL_DEPTH_COMPONENT16, w, h);
        glGenFramebuffers(1, &mut fbo);
        glBindFramebuffer(GL_FRAMEBUFFER, fbo);
        glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex, 0);
        glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_DEPTH_ATTACHMENT, GL_RENDERBUFFER, rb);
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE);
        glViewport(0, 0, w, h);
        glClearColor(DST[0], DST[1], DST[2], DST[3]);
        glClearDepthf(1.0);
        glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT);
    }
    assert_eq!(err(), GL_NO_ERROR, "target set-up");
    (fbo, tex, rb)
}

fn release(t: (GLuint, GLuint, GLuint)) {
    // SAFETY: names this test made.
    unsafe {
        glDisable(GL_BLEND);
        glDisable(GL_DEPTH_TEST);
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glDeleteFramebuffers(1, &t.0);
        glDeleteTextures(1, &t.1);
        glDeleteRenderbuffers(1, &t.2);
    }
}

static POS: [f32; 18] =
    [-1.0, -1.0, 0.0, 1.0, -1.0, 0.0, -1.0, 1.0, 0.0, -1.0, 1.0, 0.0, 1.0, -1.0, 0.0, 1.0, 1.0, 0.0];

const VS: &str = "attribute vec3 a_pos;
uniform float u_z;
void main()
{
    gl_Position = vec4(a_pos.xy, u_z, 1.0);
}
";
const FS: &str = "precision mediump float;
uniform vec4 u_color;
void main()
{
    gl_FragColor = u_color;
}
";

/// RGB the draw writes under glBlendFunc(SRC_ALPHA, ONE_MINUS_SRC_ALPHA):
/// s * a + d * (1 - a), rounded to RGBA8.
fn blend_src_alpha(s: [f32; 4], d: [f32; 4]) -> [u8; 3] {
    let f = |i: usize| ((s[i] * s[3] + d[i] * (1.0 - s[3])).clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
    [f(0), f(1), f(2)]
}

/// RGB under glBlendFunc(ONE, ONE): s + d, saturated.
fn blend_additive(s: [f32; 4], d: [f32; 4]) -> [u8; 3] {
    let f = |i: usize| ((s[i] + d[i]).clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
    [f(0), f(1), f(2)]
}

/// Set the state, draw the quad at depth `z` with `color`, read one pixel.
fn draw(p: GLuint, func: Option<(GLenum, GLenum)>, depth: bool, z: f32, color: [f32; 4]) -> Seen {
    SEEN.lock().unwrap().clear();
    let mut px = [0u8; 4];
    // SAFETY: the array is static; `px` holds one RGBA8 pixel.
    unsafe {
        match func {
            Some((s, d)) => {
                glEnable(GL_BLEND);
                glBlendFunc(s, d);
            }
            None => glDisable(GL_BLEND),
        }
        if depth {
            glEnable(GL_DEPTH_TEST);
            glDepthFunc(GL_LEQUAL);
        } else {
            glDisable(GL_DEPTH_TEST);
        }
        glUniform1f(glGetUniformLocation(p, c"u_z".as_ptr()), z);
        glUniform4f(glGetUniformLocation(p, c"u_color".as_ptr()), color[0], color[1], color[2], color[3]);
        glBindBuffer(GL_ARRAY_BUFFER, 0);
        glVertexAttribPointer(0, 3, GL_FLOAT, 0, 0, POS.as_ptr() as *const c_void);
        glEnableVertexAttribArray(0);
        glDrawArrays(GL_TRIANGLES, 0, 6);
        glFinish();
    }
    assert_eq!(err(), GL_NO_ERROR, "the draw");
    // SAFETY: one pixel into `px`.
    unsafe { glReadPixels(4, 4, 1, 1, GL_RGBA, GL_UNSIGNED_BYTE, px.as_mut_ptr().cast()) };
    assert_eq!(err(), GL_NO_ERROR, "the read");
    // Current behaviour: the host runs no shader and clears nothing.
    assert_eq!(px, UNWRITTEN, "no pixel is written on the host");
    let s = core::mem::take(&mut *SEEN.lock().unwrap());
    let s = *s.last().expect("the draw reached the back end");
    assert_eq!(s.mali, -ENODEV, "the Mali takes the draw");
    s
}

fn run(func: Option<(GLenum, GLenum)>, depth: bool, color: [f32; 4]) -> Seen {
    let _g = context();
    let t = target(8, 8);
    let p = program(VS, FS, &[c"a_pos"]);
    let s = draw(p, func, depth, 0.0, color);
    release(t);
    s
}

const ALPHA: Option<(GLenum, GLenum)> = Some((GL_SRC_ALPHA, GL_ONE_MINUS_SRC_ALPHA));
const ADD: Option<(GLenum, GLenum)> = Some((GL_ONE, GL_ONE));

#[test]
fn hand_computed_values() {
    // pulsar-style: 0.5 alpha, colour (1, 0, 0.5) over (0.25, 0.5, 0.75).
    // R: 1*0.5 + 0.25*0.5 = 0.625 -> 159; G: 0.25 -> 64; B: 0.25+0.375 = 0.625 -> 159.
    assert_eq!(blend_src_alpha([1.0, 0.0, 0.5, 0.5], DST), [159, 64, 159]);
    // jellyfish-style additive: (0.5, 0.25, 0.5) + (0.25, 0.5, 0.75) = (0.75, 0.75, 1.25 -> 1).
    assert_eq!(blend_additive([0.5, 0.25, 0.5, 1.0], DST), [191, 191, 255]);
}

/// The codes the GL core hands the back end for `func`: (src, dst) on both
/// channel groups, the ADD equation.
fn assert_codes(s: &Seen, src: u32, dst: u32) {
    let c = |v: u8| u32::from(v);
    assert_eq!((c(s.src_rgb), c(s.dst_rgb), c(s.src_a), c(s.dst_a)), (src, dst, src, dst), "{s:?}");
    assert_eq!((c(s.eq_rgb), c(s.eq_a)), (SHC_BLEND_ADD, SHC_BLEND_ADD), "{s:?}");
}

fn both(src: Factor, dst: Factor) -> Blending {
    let c = Channel { func: Func::Add, src, dst };
    Blending { rgb: c, alpha: c }
}

#[test]
fn mali_blends_both_in_fixed_function() {
    // The Mali blends glmark2's two states with its fixed-function blend
    // descriptor (no blend shader), reading the destination, as panfrost.
    for b in [both(Factor::SRC_ALPHA, Factor::INV_SRC_ALPHA), both(Factor::ONE, Factor::ONE)] {
        assert!(mb::can_fixed_function(&b), "{b:?}");
        assert!(!mb::needs_shader(&b, &[0.0; 4]), "{b:?}");
        assert!(!mb::is_opaque(&b), "{b:?}");
        assert!(mb::reads_dest(&b), "{b:?}");
    }
}

#[test]
fn alpha_blend_depth_off() {
    let s = run(ALPHA, false, [1.0, 0.0, 0.5, 0.5]);
    assert!(s.blend && !s.depth_test);
    assert_codes(&s, SHC_BF_SRC_ALPHA, SHC_BF_INV_SRC_ALPHA);
}

#[test]
fn alpha_blend_depth_on() {
    let s = run(ALPHA, true, [1.0, 0.0, 0.5, 0.5]);
    assert!(s.blend && s.depth_test && s.depth_write);
    assert_codes(&s, SHC_BF_SRC_ALPHA, SHC_BF_INV_SRC_ALPHA);
}

#[test]
fn additive_blend_depth_off() {
    let s = run(ADD, false, [0.5, 0.25, 0.5, 1.0]);
    assert!(s.blend && !s.depth_test);
    assert_codes(&s, SHC_BF_ONE, SHC_BF_ONE);
}

#[test]
fn additive_blend_depth_on() {
    let s = run(ADD, true, [0.5, 0.25, 0.5, 1.0]);
    assert!(s.blend && s.depth_test && s.depth_write);
    assert_codes(&s, SHC_BF_ONE, SHC_BF_ONE);
}

#[test]
fn state_differs_between_modes() {
    let a = run(ALPHA, false, [1.0, 0.0, 0.5, 0.5]);
    let b = run(ADD, false, [0.5, 0.25, 0.5, 1.0]);
    let o = run(None, false, [1.0, 0.0, 0.5, 0.5]);
    assert!(!o.blend);
    assert_ne!((a.src_rgb, a.dst_rgb), (b.src_rgb, b.dst_rgb));
    assert!(o.depth_write == a.depth_write, "blending does not change the depth write");
}
