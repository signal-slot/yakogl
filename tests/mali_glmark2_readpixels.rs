// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GL-level host test of the Mali back end: glReadPixels as glmark2-es2 uses
//! it (plan #367, item test-readpixels).
//!
//! glmark2 reads the framebuffer back with `glReadPixels(x, y, w, h,
//! GL_RGBA, GL_UNSIGNED_BYTE)` (its screenshot and validation paths) and
//! asks `GL_IMPLEMENTATION_COLOR_READ_FORMAT/TYPE`. Covered: the whole
//! target and an odd-sized sub-rectangle at an odd offset under
//! GL_PACK_ALIGNMENT 1 and 4 (row stride and that padding is untouched), the
//! implementation read pair and a read in it, reads from an RGBA8 FBO colour
//! texture and from the default framebuffer after a clear and a draw, and
//! GL_INVALID_OPERATION for a format/type pair that is not allowed.
//!
//! Level reached: GL core plus the Mali back end's `mali_draw` through a
//! stand-in back end (no Mali OSAL on the host: the draw ends at -ENODEV and
//! writes no pixel), so pixels are the clear colours.
//!
//! Not a tools/check.sh row.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_glmark2_readpixels
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
use yakogl::gl::vertex::{glDisableVertexAttribArray, glEnableVertexAttribArray, glVertexAttribPointer};
use yakogl::mali::backend::{mali_draw, yakogl_backend_mali};
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};

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

static DRAWS: Mutex<u32> = Mutex::new(0);

unsafe extern "C" fn b_draw(d: *const GlesDraw) -> i32 {
    let d = &*d;
    if d.index_count == 0 {
        return 0;
    }
    *DRAWS.lock().unwrap() += 1;
    mali_draw(d);
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
    *DRAWS.lock().unwrap() = 0;
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

const VS: &str = "attribute vec3 a_pos;
void main()
{
    gl_Position = vec4(a_pos, 1.0);
}
";
const FS: &str = "precision mediump float;
uniform vec4 u_color;
void main()
{
    gl_FragColor = u_color;
}
";

fn program() -> GLuint {
    let (v, f) = (shader(GL_VERTEX_SHADER, VS), shader(GL_FRAGMENT_SHADER, FS));
    // SAFETY: plain GL calls.
    unsafe {
        let p = glCreateProgram();
        glAttachShader(p, v);
        glAttachShader(p, f);
        glBindAttribLocation(p, 0, c"a_pos".as_ptr());
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        assert_eq!(ok, 1, "link failed");
        glUseProgram(p);
        glUniform4f(glGetUniformLocation(p, c"u_color".as_ptr()), 1.0, 0.5, 0.25, 1.0);
        p
    }
}

fn draw_quad() {
    static Q: [f32; 12] = [-1.0, -1.0, 0.0, 1.0, -1.0, 0.0, -1.0, 1.0, 0.0, 1.0, 1.0, 0.0];
    // SAFETY: client array outlives the draw.
    unsafe {
        glBindBuffer(GL_ARRAY_BUFFER, 0);
        glEnableVertexAttribArray(0);
        glVertexAttribPointer(0, 3, GL_FLOAT, 0, 0, Q.as_ptr().cast());
        glDrawArrays(GL_TRIANGLE_STRIP, 0, 4);
        glDisableVertexAttribArray(0);
    }
}

fn clear(r: f32, g: f32, b: f32, a: f32) {
    // SAFETY: plain GL calls.
    unsafe {
        glClearColor(r, g, b, a);
        glClear(GL_COLOR_BUFFER_BIT);
    }
}

fn pack(a: GLint) {
    // SAFETY: plain GL call.
    unsafe { glPixelStorei(GL_PACK_ALIGNMENT, a) };
}

fn read(x: i32, y: i32, w: i32, h: i32, fmt: GLenum, ty: GLenum, buf: &mut [u8]) {
    // SAFETY: callers size `buf` for the rectangle under the pack alignment.
    unsafe { glReadPixels(x, y, w, h, fmt, ty, buf.as_mut_ptr().cast()) };
}

/// Bytes of one row of `w` RGBA8 pixels under alignment `a`.
fn stride(w: usize, a: usize) -> usize {
    (w * 4).div_ceil(a) * a
}

/// The whole 8x8 target as tightly packed RGBA8 (alignment 1).
fn whole() -> Vec<u8> {
    pack(1);
    let mut w = vec![0xEEu8; 8 * 8 * 4];
    read(0, 0, 8, 8, GL_RGBA, GL_UNSIGNED_BYTE, &mut w);
    assert_eq!(err(), GL_NO_ERROR);
    assert!(!w.contains(&0xEE) || w.iter().any(|&b| b != 0xEE), "the read wrote nothing");
    w
}

/// Reads `(x, y, w, h)` under alignment `a` and compares every pixel with the
/// same pixel of the whole read; the bytes after the data and the row padding
/// stay untouched. (The host stand-in rasterises nothing, so the values are
/// whatever the target holds; the test pins position, stride and extent.)
fn check_rect(whole: &[u8], x: usize, y: usize, w: usize, h: usize, a: usize) {
    pack(a as GLint);
    let st = stride(w, a);
    let mut sub = vec![0xEEu8; st * h + 8];
    read(x as i32, y as i32, w as i32, h as i32, GL_RGBA, GL_UNSIGNED_BYTE, &mut sub);
    assert_eq!(err(), GL_NO_ERROR, "({x},{y}) {w}x{h} align {a}");
    for r in 0..h {
        for p in 0..w {
            assert_eq!(
                &sub[r * st + p * 4..][..4],
                &whole[((y + r) * 8 + x + p) * 4..][..4],
                "px {p},{r} of ({x},{y}) align {a}"
            );
        }
        assert!(sub[r * st + w * 4..(r + 1) * st].iter().all(|&b| b == 0xEE), "row padding written");
    }
    assert!(sub[st * h..].iter().all(|&b| b == 0xEE), "wrote past the rectangle");
}

fn release(fbo: GLuint, tex: GLuint) {
    // SAFETY: names the test made.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glDeleteFramebuffers(1, &fbo);
        glDeleteTextures(1, &tex);
    }
    pack(4);
}

#[test]
fn fbo_whole_and_odd_rect_both_alignments() {
    let _g = context();
    let (fbo, tex) = target(8, 8);
    clear(1.0, 0.0, 0.0, 1.0);
    let w = whole();
    for a in [1, 4] {
        check_rect(&w, 0, 0, 8, 8, a);
        // odd size at an odd offset: 3x5 at (3, 1), 5x3 at (1, 5), 1x1 at (7, 7)
        check_rect(&w, 3, 1, 3, 5, a);
        check_rect(&w, 1, 5, 5, 3, a);
        check_rect(&w, 7, 7, 1, 1, a);
    }
    release(fbo, tex);
}

#[test]
fn scissored_clear_does_not_change_the_read_extent() {
    let _g = context();
    let (fbo, tex) = target(8, 8);
    clear(0.0, 0.0, 1.0, 1.0);
    // SAFETY: plain GL calls.
    unsafe {
        glEnable(GL_SCISSOR_TEST);
        glScissor(2, 2, 3, 3);
    }
    clear(1.0, 0.0, 0.0, 1.0);
    // SAFETY: plain GL call.
    unsafe { glDisable(GL_SCISSOR_TEST) };
    let w = whole();
    for a in [1, 4] {
        check_rect(&w, 1, 3, 3, 3, a);
    }
    release(fbo, tex);
}

#[test]
fn implementation_read_format_and_a_read_in_it() {
    let _g = context();
    let (fbo, tex) = target(8, 8);
    clear(0.0, 1.0, 0.0, 1.0);
    let (mut f, mut t) = (0, 0);
    // SAFETY: one int each.
    unsafe {
        glGetIntegerv(GL_IMPLEMENTATION_COLOR_READ_FORMAT, &mut f);
        glGetIntegerv(GL_IMPLEMENTATION_COLOR_READ_TYPE, &mut t);
    }
    assert_eq!(err(), GL_NO_ERROR);
    assert!(f != 0 && t != 0, "no implementation read pair: {f:#x} {t:#x}");
    eprintln!("implementation read pair: format {f:#x} type {t:#x}");
    pack(1);
    let mut b = vec![0xEEu8; 8 * 8 * 8];
    read(0, 0, 8, 8, f as GLenum, t as GLenum, &mut b);
    assert_eq!(err(), GL_NO_ERROR, "a read in the reported pair must be accepted");
    assert_eq!((f as GLenum, t as GLenum), (GL_RGBA, GL_UNSIGNED_BYTE), "glmark2 reads RGBA/UNSIGNED_BYTE");
    // SAFETY: names made above.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glDeleteFramebuffers(1, &fbo);
        glDeleteTextures(1, &tex);
    }
    pack(4);
}

#[test]
fn default_framebuffer_after_clear_and_draw() {
    let _g = context();
    // SAFETY: plain GL call.
    unsafe { glBindFramebuffer(GL_FRAMEBUFFER, 0) };
    // SAFETY: plain GL calls.
    unsafe { glViewport(0, 0, 8, 8) };
    clear(0.0, 0.0, 1.0, 1.0);
    let prog = program();
    draw_quad();
    // SAFETY: plain GL call.
    unsafe { glFinish() };
    assert_eq!(err(), GL_NO_ERROR);
    assert_eq!(*DRAWS.lock().unwrap(), 1, "the draw reached the back end");
    for a in [1, 4] {
        pack(a);
        let mut b = vec![0u8; stride(5, a as usize) * 3];
        read(1, 2, 5, 3, GL_RGBA, GL_UNSIGNED_BYTE, &mut b);
        assert_eq!(err(), GL_NO_ERROR, "align {a}");
    }
    // SAFETY: name made above.
    unsafe {
        glUseProgram(0);
        glDeleteProgram(prog);
    }
    pack(4);
}

#[test]
fn fbo_after_draw_reads_without_error() {
    let _g = context();
    let (fbo, tex) = target(8, 8);
    clear(1.0, 0.0, 0.0, 1.0);
    let prog = program();
    draw_quad();
    let mut b = vec![0u8; 4 * 4 * 4];
    read(2, 2, 4, 4, GL_RGBA, GL_UNSIGNED_BYTE, &mut b);
    assert_eq!(err(), GL_NO_ERROR);
    assert_eq!(*DRAWS.lock().unwrap(), 1);
    // SAFETY: names made above.
    unsafe {
        glUseProgram(0);
        glDeleteProgram(prog);
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glDeleteFramebuffers(1, &fbo);
        glDeleteTextures(1, &tex);
    }
}

#[test]
fn invalid_format_type_pairs_are_refused() {
    let _g = context();
    let (fbo, tex) = target(8, 8);
    clear(1.0, 0.0, 0.0, 1.0);
    pack(1);
    let mut b = vec![0u8; 8 * 8 * 8];
    // an RGBA8 target is not readable as RGB/UNSIGNED_BYTE or as a packed short
    // pair other than the implementation one; ES 2.0 requires RGBA/UNSIGNED_BYTE
    // and the implementation pair only. Type/format combinations that do not
    // exist at all are GL_INVALID_OPERATION.
    for (f, t) in [
        (GL_RGBA, GL_UNSIGNED_SHORT_5_6_5),
        (GL_RGB, GL_UNSIGNED_SHORT_4_4_4_4),
        (GL_DEPTH_COMPONENT, GL_UNSIGNED_BYTE),
    ] {
        read(0, 0, 2, 2, f, t, &mut b);
        let e = err();
        assert!(e == GL_INVALID_OPERATION || e == GL_INVALID_ENUM, "{f:#x}/{t:#x}: error {e:#x}");
    }
    // an unknown enum is GL_INVALID_ENUM
    read(0, 0, 2, 2, 0x1234, GL_UNSIGNED_BYTE, &mut b);
    assert_eq!(err(), GL_INVALID_ENUM);
    // a negative size is GL_INVALID_VALUE
    read(0, 0, -1, 2, GL_RGBA, GL_UNSIGNED_BYTE, &mut b);
    assert_eq!(err(), GL_INVALID_VALUE);
    // the valid pair still works afterwards
    read(0, 0, 2, 2, GL_RGBA, GL_UNSIGNED_BYTE, &mut b);
    assert_eq!(err(), GL_NO_ERROR);
    // SAFETY: names made above.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glDeleteFramebuffers(1, &fbo);
        glDeleteTextures(1, &tex);
    }
    pack(4);
}
