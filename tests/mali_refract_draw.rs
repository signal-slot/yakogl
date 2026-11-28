// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The draw-level check of D-NPOTMIP (plan #367, issues #32 and #20): refract's
//! non-power-of-two texture, glGenerateMipmap, GL_LINEAR_MIPMAP_LINEAR, and
//! minified quads that read level 0 and the deeper levels.
//! `tests/mali_npot_mip.rs` checks the descriptor; this test checks what a
//! draw reads through it.
//!
//! The texture is a 1-texel checker of two greys (200 and 40, by texel
//! parity), so level 0 reads one of the two and every level the box filter
//! built reads their mean, 120. A binding that kept one level reads the
//! checker at every minification. The shaders are a plain textured quad
//! (the Mali compiler accepts them at the tip; written for this test).
//!
//! The host has no Mali OSAL: `mali_draw` takes the draw and ends at
//! `-ENODEV`, so no shader executes. The stand-in back end therefore does
//! the fragment stage of `texture2D` in software over the Mali's binding (the
//! level the minification selects, the texel under the pixel centre, read
//! from the chain at the descriptor's offsets) and writes the framebuffer, and
//! the pixels are read back through glReadPixels. What this pins is the GL
//! core's chain as the Mali binding presents it and the draw's GL errors; the
//! GPU's own filtering is the board's (not checked here).
//!
//! Not a tools/check.sh row.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_refract_draw
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

/// What the Mali made of the draw, and the pixels the stand-in's software
/// sampler wrote for it.
static MALI: Mutex<Vec<i32>> = Mutex::new(Vec::new());

/// Level `l`'s texel (x, y) of the bound chain, as the descriptor lays it
/// out (level 0 at `base_gpu`, every level on level 0's pitch, each level on
/// the next 64-byte boundary after the rows of the one before).
///
/// # Safety
/// `t.base_gpu` is a host pointer of the stand-in's allocator.
unsafe fn texel(t: &GlesTextureBind, l: u32, x: u32, y: u32) -> [u8; 4] {
    let pitch = u64::from(t.stride_texels) * 4;
    let mut at = 0u64;
    for k in 0..l {
        at = (at + u64::from((t.height >> k).max(1)) * pitch).next_multiple_of(64);
    }
    let p = (t.base_gpu + at + u64::from(y) * pitch + u64::from(x) * 4) as *const u8;
    [*p, *p.add(1), *p.add(2), *p.add(3)]
}

unsafe extern "C" fn b_draw(d: *const GlesDraw) -> i32 {
    let d = &*d;
    if d.index_count == 0 || d.num_textures == 0 {
        return 0;
    }
    let t = &d.textures[0];
    MALI.lock().unwrap().push(mali_draw(d));
    /* No Mali executes on the host: the stand-in does the fragment stage of
     * `texture2D(u_tex, v_uv)` over the viewport (the quad covers it), with
     * the level the minification asks for (log2 of texture over viewport
     * width, the quads here keep it integral) and the texel under the pixel
     * centre of that level. A binding that kept one level reads level 0. */
    let [vx, vy, vw, vh] = d.viewport;
    let want = (f64::from(t.width) / f64::from(vw)).log2().round().max(0.0) as u32;
    let l = want.min(u32::from(t.mip_levels).saturating_sub(1));
    let (lw, lh) = ((t.width >> l).max(1), (t.height >> l).max(1));
    let pitch = if d.target_pitch != 0 { d.target_pitch } else { d.target_w };
    for j in 0..vh {
        for i in 0..vw {
            let (x, y) = ((i as u64 * u64::from(lw) / vw as u64) as u32, (j as u64 * u64::from(lh) / vh as u64) as u32);
            let px = texel(t, l, x, y);
            let o = d.target_gpu as *mut u8;
            let at = ((vy + j) as usize * pitch as usize + (vx + i) as usize) * 4;
            core::ptr::copy_nonoverlapping(px.as_ptr(), o.add(at), 4);
        }
    }
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
    MALI.lock().unwrap().clear();
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
    assert_eq!(err(), GL_NO_ERROR, "program");
}

/// Covers the whole viewport.
static FULL: [f32; 12] = [-1.0, -1.0, 0.0, 1.0, 3.0, -1.0, 0.0, 1.0, -1.0, 3.0, 0.0, 1.0];

const LOW: u8 = 40;
const HIGH: u8 = 200;
const MEAN: u8 = 120;

/// A `w` x `h` RGBA texture of the checker, trilinear, with glGenerateMipmap,
/// on unit 0.
fn refract_texture(w: i32, h: i32) -> GLuint {
    let mut tex = 0;
    let mut texels = vec![0u8; (w * h * 4) as usize];
    for (i, p) in texels.chunks_mut(4).enumerate() {
        let (x, y) = (i as i32 % w, i as i32 / w);
        let g = if (x + y) & 1 == 0 { HIGH } else { LOW };
        p.copy_from_slice(&[g, g, g, 0xff]);
    }
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

/// An RGBA8 framebuffer object of `w` x `h`; returns (fbo, texture).
fn target(w: i32, h: i32) -> (GLuint, GLuint) {
    let (mut fbo, mut tex) = (0, 0);
    // SAFETY: room for one name each; null texels allocate the image. The
    // texture unit 1 keeps unit 0's mipmapped texture bound.
    unsafe {
        glActiveTexture(GL_TEXTURE1);
        glGenTextures(1, &mut tex);
        glBindTexture(GL_TEXTURE_2D, tex);
        glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA as GLint, w, h, 0, GL_RGBA, GL_UNSIGNED_BYTE, core::ptr::null());
        glGenFramebuffers(1, &mut fbo);
        glBindFramebuffer(GL_FRAMEBUFFER, fbo);
        glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex, 0);
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE);
        glActiveTexture(GL_TEXTURE0);
        glClearColor(0.0, 0.0, 0.0, 1.0);
        glClear(GL_COLOR_BUFFER_BIT);
    }
    assert_eq!(err(), GL_NO_ERROR, "target");
    (fbo, tex)
}

/// Draw one minified quad into the viewport (x, y, w, h) and read back its
/// centre pixel; also asserts that nothing was refused and that the Mali took
/// the draw.
fn quad(x: i32, y: i32, w: i32, h: i32) -> [u8; 4] {
    MALI.lock().unwrap().clear();
    let mut px = [0u8; 4];
    // SAFETY: `FULL` is static; `px` holds one RGBA8 pixel.
    unsafe {
        glViewport(x, y, w, h);
        glBindBuffer(GL_ARRAY_BUFFER, 0);
        glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, FULL.as_ptr() as *const c_void);
        glEnableVertexAttribArray(0);
        glDrawArrays(GL_TRIANGLES, 0, 3);
        glFinish();
        assert_eq!(err(), GL_NO_ERROR, "the draw is not refused");
        glReadPixels(x + w / 2, y + h / 2, 1, 1, GL_RGBA, GL_UNSIGNED_BYTE, px.as_mut_ptr().cast());
        assert_eq!(err(), GL_NO_ERROR, "glReadPixels");
    }
    assert_eq!(*MALI.lock().unwrap(), vec![-ENODEV], "the Mali takes the draw");
    px
}

fn level_count(w: i32, h: i32) -> u32 {
    32 - (w.max(h) as u32).leading_zeros()
}

/// Draw the texture's levels in turn: viewport width `w >> l` shows level `l`.
/// `levels` are the ones drawn.
fn minify(w: i32, h: i32, levels: core::ops::RangeInclusive<u32>, tolerance: i32) {
    let _g = context();
    program();
    let tex = refract_texture(w, h);
    let (fbo, rt) = target(w.min(256), h.min(256));
    for l in levels {
        let (vw, vh) = ((w >> l).max(1), (h >> l).max(1));
        let px = quad(0, 0, vw, vh);
        if l == 0 {
            assert!(px[0] == HIGH || px[0] == LOW, "level 0 reads the checker, got {px:?}");
        } else {
            assert!(
                (i32::from(px[0]) - i32::from(MEAN)).abs() <= tolerance,
                "level {l} of {w}x{h} ({vw}x{vh} quad) reads its average {MEAN}, got {px:?}"
            );
        }
        assert_eq!(px[3], 0xff);
    }
    // SAFETY: names this test made.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glDeleteFramebuffers(1, &fbo);
        glDeleteTextures(1, &rt);
        glDeleteTextures(1, &tex);
    }
}

/// An NPOT texture both sides of which stay NPOT for several levels (160x120:
/// 160, 80, 40, 20, 10, 5, 2, 1 wide), every level drawn.
#[test]
fn npot_160x120_every_level_reads_its_average() {
    assert_eq!(level_count(160, 120), 8);
    minify(160, 120, 0..=7, 0);
}

/// refract's own 1600x1200 colour texture (glmark2's 800x600 canvas, twice each
/// side): levels 3 (200x150) to 10 (1x1) fit the 256-texel target; the NPOT
/// floor rule makes the deeper odd levels' averages approximate, so they get
/// a tolerance. Levels 0 to 2 are the 160x120 test's job.
#[test]
fn refract_1600x1200_deep_levels_read_their_average() {
    assert_eq!(level_count(1600, 1200), 11);
    minify(1600, 1200, 3..=10, 8);
}
