// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GL-level host test: several programs, textures and framebuffers switched
//! within one frame on the Mali back end (plan #367, glmark2-es2's shadow,
//! refract, desktop and effect2d scenes bind two or three programs before one
//! swap). The shaders are written for this test; no glmark2 text is copied.
//!
//! One frame is three passes:
//!
//! 1. program A (flat colour, `u_color`) into an FBO's colour texture, viewport
//!    4x4, blend off;
//! 2. program B (one sampler on unit 1, unit 0 holds an unrelated texture) into
//!    the default framebuffer, the FBO texture bound as the sampled image,
//!    viewport 8x8, blend off;
//! 3. program A again, other `u_color`, into the default framebuffer, blend on,
//!    viewport 6x6.
//!
//! Level reached: link plus draw submission. The host has no Mali OSAL, so the
//! stand-in back end hands each draw to `mali_draw`, which takes it and ends at
//! `-ENODEV`; no shader executes and no pixel is written. glReadPixels is
//! called after every pass and must raise no GL error, but its values are not
//! checked (nothing wrote them). What is checked is what the Mali was handed
//! for each pass: the target, the viewport, the blend state, the program's own
//! shader pair, the sampled texture's storage and unit, and the uniform words.
//! No pass may show state of another pass. The frame is repeated, with the FBO
//! texture kept and with it re-created, to catch stale per-program state.
//!
//! Not a tools/check.sh row (a later check-rows item adds it).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_glmark2_program_switch
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

/// What the back end was handed for one draw, and what the Mali made of it.
#[derive(Clone, Debug, PartialEq)]
struct Seen {
    target_gpu: u64,
    target_w: u32,
    viewport: [i32; 4],
    blend: bool,
    vs: usize,
    fs: usize,
    num_textures: u8,
    tex_unit: u8,
    tex_gpu: u64,
    tex_w: u32,
    fs_words: Vec<u32>,
    mali: i32,
}

static SEEN: Mutex<Vec<Seen>> = Mutex::new(Vec::new());

unsafe extern "C" fn b_draw(d: *const GlesDraw) -> i32 {
    let d = &*d;
    if d.index_count == 0 {
        return 0;
    }
    let t = &d.textures[0];
    let fs_words = if d.fs_uniform_cpu.is_null() {
        Vec::new()
    } else {
        core::slice::from_raw_parts(d.fs_uniform_cpu as *const u32, d.fs_uniform_dwords as usize).to_vec()
    };
    let mali = mali_draw(d);
    SEEN.lock().unwrap().push(Seen {
        target_gpu: d.target_gpu,
        target_w: d.target_w,
        viewport: d.viewport,
        blend: d.blend,
        vs: d.vs as usize,
        fs: d.fs as usize,
        num_textures: d.num_textures,
        tex_unit: t.unit,
        tex_gpu: t.base_gpu,
        tex_w: t.width,
        fs_words,
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

/// An RGBA8 texture as the colour attachment of a framebuffer.
fn target(w: i32, h: i32) -> (GLuint, GLuint) {
    let (mut fbo, mut tex) = (0, 0);
    // SAFETY: room for one name each; null texels allocate the image.
    unsafe {
        glGenTextures(1, &mut tex);
        glBindTexture(GL_TEXTURE_2D, tex);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_LINEAR as GLint);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_LINEAR as GLint);
        glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA as GLint, w, h, 0, GL_RGBA, GL_UNSIGNED_BYTE, core::ptr::null());
        glGenFramebuffers(1, &mut fbo);
        glBindFramebuffer(GL_FRAMEBUFFER, fbo);
        glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex, 0);
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE);
    }
    assert_eq!(err(), GL_NO_ERROR, "target set-up");
    (fbo, tex)
}

static POS: [f32; 18] =
    [-1.0, -1.0, 0.0, 1.0, -1.0, 0.0, -1.0, 1.0, 0.0, -1.0, 1.0, 0.0, 1.0, -1.0, 0.0, 1.0, 1.0, 0.0];
static UV: [f32; 12] = [0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 1.0, 1.0, 0.0, 1.0, 1.0];

/// Draw the quad with the current program (`a_pos`, `a_uv`), read one pixel
/// back; returns the GL errors of the draw and of the read, and the draw.
fn draw() -> (GLenum, GLenum, Seen) {
    SEEN.lock().unwrap().clear();
    let mut px = [0u8; 4];
    // SAFETY: the arrays are static; `px` holds one RGBA8 pixel.
    unsafe {
        glBindBuffer(GL_ARRAY_BUFFER, 0);
        glVertexAttribPointer(0, 3, GL_FLOAT, 0, 0, POS.as_ptr() as *const c_void);
        glEnableVertexAttribArray(0);
        glVertexAttribPointer(1, 2, GL_FLOAT, 0, 0, UV.as_ptr() as *const c_void);
        glEnableVertexAttribArray(1);
        glDrawArrays(GL_TRIANGLES, 0, 6);
        glFinish();
    }
    let de = err();
    // SAFETY: one pixel into `px`.
    unsafe { glReadPixels(1, 1, 1, 1, GL_RGBA, GL_UNSIGNED_BYTE, px.as_mut_ptr().cast()) };
    let re = err();
    let s = core::mem::take(&mut *SEEN.lock().unwrap());
    assert_eq!(s.len(), 1, "one draw reached the back end");
    (de, re, s[0].clone())
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
/// Program A: a flat colour from a uniform.
const A_FS: &str = "precision mediump float;
uniform vec4 u_color;
varying mediump vec2 v_uv;
void main()
{
    gl_FragColor = u_color;
}
";
/// Program B: one sampler.
const B_FS: &str = "precision mediump float;
uniform sampler2D u_tex;
varying mediump vec2 v_uv;
void main()
{
    gl_FragColor = texture2D(u_tex, v_uv);
}
";

fn link(vs: &str, fs: &str) -> GLuint {
    let (v, f) = (shader(GL_VERTEX_SHADER, vs), shader(GL_FRAGMENT_SHADER, fs));
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
        assert_eq!(ok, 1, "link failed");
        p
    }
}

fn colour(p: GLuint, c: [f32; 4]) {
    // SAFETY: `p` is in use and has u_color.
    unsafe { glUniform4f(glGetUniformLocation(p, c"u_color".as_ptr()), c[0], c[1], c[2], c[3]) };
}

fn has_word(words: &[u32], v: f32) -> bool {
    words.iter().any(|w| *w == v.to_bits())
}

struct Frame {
    a: GLuint,
    b: GLuint,
    other: GLuint,
}

fn setup() -> Frame {
    let a = link(VS, A_FS);
    let b = link(VS, B_FS);
    let mut other = 0;
    let texels = [0x40u8; 16];
    // SAFETY: room for one name; the texels cover 2x2 RGBA.
    unsafe {
        glGenTextures(1, &mut other);
        glActiveTexture(GL_TEXTURE0);
        glBindTexture(GL_TEXTURE_2D, other);
        glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA as GLint, 2, 2, 0, GL_RGBA, GL_UNSIGNED_BYTE, texels.as_ptr().cast());
    }
    assert_eq!(err(), GL_NO_ERROR, "set-up");
    Frame { a, b, other }
}

/// One frame: the three passes, every check on what each was handed. Returns
/// the FBO texture's storage address.
fn frame(f: &Frame, fbo: GLuint, tex: GLuint, n: usize) -> u64 {
    // Never 0.5 or 0.75, the words of pass 3.
    let red = 0.0625 * (n as f32 + 1.0);
    // Pass 1: A into the FBO texture.
    // SAFETY: plain GL calls on names this test made.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, fbo);
        glViewport(0, 0, 4, 4);
        glDisable(GL_BLEND);
        glUseProgram(f.a);
    }
    colour(f.a, [red, 0.0, 0.0, 1.0]);
    let (de, re, p1) = draw();
    assert_eq!((de, re), (GL_NO_ERROR, GL_NO_ERROR), "pass 1 errors, frame {n}");
    assert_eq!(p1.mali, -ENODEV, "the Mali takes pass 1");
    assert_eq!((p1.target_w, p1.viewport[2], p1.viewport[3]), (4, 4, 4), "pass 1 target and viewport");
    assert!(!p1.blend, "pass 1 blend");
    assert_eq!(p1.num_textures, 0, "program A samples nothing, whatever is bound");
    assert!(has_word(&p1.fs_words, red) || p1.fs_words.is_empty(), "pass 1 colour {:?}", p1.fs_words);

    // Pass 2: B samples the FBO texture on unit 1 into the default framebuffer.
    // SAFETY: plain GL calls on names this test made.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glViewport(0, 0, 8, 8);
        glUseProgram(f.b);
        glActiveTexture(GL_TEXTURE1);
        glBindTexture(GL_TEXTURE_2D, tex);
        glUniform1i(glGetUniformLocation(f.b, c"u_tex".as_ptr()), 1);
    }
    let (de, re, p2) = draw();
    assert_eq!((de, re), (GL_NO_ERROR, GL_NO_ERROR), "pass 2 errors, frame {n}");
    assert_eq!(p2.mali, -ENODEV, "the Mali takes pass 2");
    assert_eq!((p2.target_w, p2.viewport), (8, [0, 0, 8, 8]), "pass 2 target and viewport");
    assert!(!p2.blend, "pass 2 blend");
    assert_eq!(
        (p2.num_textures, p2.tex_unit, p2.tex_w),
        (1, 0, 4),
        "pass 2 samples the FBO texture (sampler binding 0, GL unit 1)"
    );
    assert_ne!(p2.tex_gpu, 0);
    assert_eq!(p2.tex_gpu, p1.target_gpu, "pass 2 samples what pass 1 wrote");
    assert_ne!(p2.target_gpu, p1.target_gpu, "pass 2 draws into the default surface, not the FBO");
    assert_ne!((p2.vs, p2.fs), (p1.vs, p1.fs), "B has its own shader pair");

    // Pass 3: A again, other colour, blend on, smaller viewport.
    // SAFETY: plain GL calls on names this test made.
    unsafe {
        glViewport(0, 0, 6, 6);
        glEnable(GL_BLEND);
        glBlendFunc(GL_SRC_ALPHA, GL_ONE_MINUS_SRC_ALPHA);
        glUseProgram(f.a);
    }
    colour(f.a, [0.0, 0.5, 0.75, 0.5]);
    let (de, re, p3) = draw();
    assert_eq!((de, re), (GL_NO_ERROR, GL_NO_ERROR), "pass 3 errors, frame {n}");
    assert_eq!(p3.mali, -ENODEV, "the Mali takes pass 3");
    assert_eq!((p3.target_gpu, p3.target_w, p3.viewport), (p2.target_gpu, 8, [0, 0, 6, 6]), "pass 3 target");
    assert!(p3.blend, "pass 3 blend");
    assert_eq!(p3.num_textures, 0, "A samples nothing: unit 1 still holds a texture");
    assert_eq!((p3.vs, p3.fs), (p1.vs, p1.fs), "A's shader pair is the one of pass 1");
    assert!(!has_word(&p3.fs_words, red), "pass 3 carries no colour of pass 1: {:x?} {:x?}", p1.fs_words, p3.fs_words);
    assert!(has_word(&p3.fs_words, 0.75) || p3.fs_words.is_empty(), "pass 3 colour {:?}", p3.fs_words);
    if !p1.fs_words.is_empty() {
        assert_ne!(p1.fs_words, p3.fs_words, "the two colours differ");
    }
    // SAFETY: plain GL calls.
    unsafe {
        glDisable(GL_BLEND);
        glActiveTexture(GL_TEXTURE0);
        glFinish();
    }
    assert_eq!(err(), GL_NO_ERROR, "end of frame {n}");
    p1.target_gpu
}

fn finish(f: &Frame, fbo: GLuint, tex: GLuint) {
    // SAFETY: names this test made.
    unsafe {
        glUseProgram(0);
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glDeleteFramebuffers(1, &fbo);
        glDeleteTextures(1, &tex);
        glDeleteTextures(1, &f.other);
        glDeleteProgram(f.a);
        glDeleteProgram(f.b);
    }
    assert_eq!(err(), GL_NO_ERROR, "clean-up");
}

#[test]
fn three_pass_frame_repeats_with_one_target() {
    let _g = context();
    let f = setup();
    let (fbo, tex) = target(4, 4);
    let first = frame(&f, fbo, tex, 0);
    for n in 1..6 {
        assert_eq!(frame(&f, fbo, tex, n), first, "the FBO texture keeps its storage, frame {n}");
    }
    finish(&f, fbo, tex);
}

#[test]
fn three_pass_frame_with_a_new_target_each_frame() {
    let _g = context();
    let f = setup();
    for n in 0..4 {
        let (fbo, tex) = target(4, 4);
        frame(&f, fbo, tex, n);
        // SAFETY: names this test made.
        unsafe {
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
            glDeleteFramebuffers(1, &fbo);
            glDeleteTextures(1, &tex);
        }
    }
    let (fbo, tex) = target(4, 4);
    finish(&f, fbo, tex);
}

#[test]
fn program_state_survives_a_switch_away_and_back() {
    let _g = context();
    let f = setup();
    // SAFETY: plain GL calls.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glViewport(0, 0, 8, 8);
        glUseProgram(f.a);
    }
    // A's colour is set once; B is used in between; A draws without setting it again.
    colour(f.a, [0.5, 0.25, 0.125, 1.0]);
    let (_, _, before) = draw();
    // SAFETY: plain GL calls.
    unsafe {
        glUseProgram(f.b);
        glUniform1i(glGetUniformLocation(f.b, c"u_tex".as_ptr()), 0);
    }
    let (_, _, mid) = draw();
    // SAFETY: plain GL call.
    unsafe { glUseProgram(f.a) };
    let (de, re, after) = draw();
    assert_eq!((de, re), (GL_NO_ERROR, GL_NO_ERROR));
    assert_eq!((mid.num_textures, mid.tex_unit), (1, 0));
    assert_eq!(after.num_textures, 0);
    assert_eq!((after.vs, after.fs), (before.vs, before.fs));
    assert_eq!(after.fs_words, before.fs_words, "A keeps its uniform across B's draw");
    assert_eq!(after.mali, -ENODEV);
    let (fbo, tex) = target(4, 4);
    finish(&f, fbo, tex);
}
