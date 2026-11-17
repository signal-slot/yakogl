// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Does a pending glClearBufferfv follow an attachment change? On the V3D
//! host back end, through the GL entry points only (plan #370,
//! clear-rebind-rgba8), for GL_RGBA8 and GL_RGB10_A2. Image A is attached,
//! glClearBufferfv clears it with a distinct value, the attachment changes to
//! image B with no glReadPixels, glFinish or draw between, and then B and A
//! are read back by glReadPixels RGBA/UNSIGNED_BYTE. ES 3.0 4.2.3 clears the
//! image attached at the call: A holds the clear, B its initial contents.
//! Images: two 2D textures (glFramebufferTexture2D), two layers of a 2D-array
//! texture (glFramebufferTextureLayer), two renderbuffers
//! (glFramebufferRenderbuffer); and a control row per format with a
//! glReadPixels between the clear and the rebind.
//!
//! Rows that show the defect are `#[ignore]`d with the observed values. The
//! sibling `es3_rgb10a2_layers_v3d` keeps one such row for RGB10_A2 layers.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_clear_rebind_v3d
//! ```

#![cfg(all(feature = "host-test", feature = "shaderc-online", feature = "gpu-v3d", not(feature = "report-es2")))]

use core::ffi::{c_char, c_void};
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{self, GlesHeap, YakoglBackend};
use yakogl::gl::consts::*;
use yakogl::gl::es3::*;
use yakogl::gl::framebuffer::*;
use yakogl::gl::state::*;
use yakogl::gl::texture::*;
use yakogl::gl::types::*;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};

unsafe extern "C" fn m_gpu_alloc(_heap: u64, size: usize, _flags: u32, out: *mut OsalGpuMem) -> i32 {
    let mem = Box::leak(vec![0u8; size].into_boxed_slice());
    let p = mem.as_mut_ptr();
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
/// The stand-in GPU: a clear draw into a `GLES_SURF_RGB10_A2` or
/// `GLES_SURF_RGBA8888` target writes the clear's word inside the draw's
/// scissor box, for the channels of `clear_color_mask` only. That is the V3D
/// back end's job (the TLB clear or the clear quad), which does not run on
/// the host; the test judges which image the GL core hands the clear to.
///
/// Quantisation: RGB10_A2 red, green and blue `clamp(v, 0, 1) * 1023`
/// rounded half up, alpha `* 3` (the GL core has already rounded alpha to 2
/// bits); RGBA8 `clamp(v, 0, 1) * 255` rounded half up, bytes in R, G, B, A
/// order. glReadPixels decodes a 10/10/10/2 field to a byte as
/// (v * 255 + max / 2) / max.
unsafe extern "C" fn b_draw(d: *const backend::GlesDraw) -> i32 {
    let d = &*d;
    if d.do_clear
        && d.target_gpu != 0
        && (d.target_format == backend::GLES_SURF_RGB10_A2 || d.target_format == backend::GLES_SURF_RGBA8888)
    {
        let ten = d.target_format == backend::GLES_SURF_RGB10_A2;
        let q = |v: f32, n: f32| (v.clamp(0.0, 1.0) * n + 0.5) as u32;
        let c = d.clear_rgba;
        let new = if ten {
            [q(c[0], 1023.0), q(c[1], 1023.0), q(c[2], 1023.0), q(c[3], 3.0)]
        } else {
            [q(c[0], 255.0), q(c[1], 255.0), q(c[2], 255.0), q(c[3], 255.0)]
        };
        let pitch = backend::draw_pitch(d) as usize;
        let base = d.target_gpu as *mut u32;
        let [sx, sy, sw, sh] = if d.scissor { d.scissor_rect } else { [0, 0, d.target_w as i32, d.target_h as i32] };
        for y in sy.max(0)..(sy + sh).min(d.target_h as i32) {
            for x in sx.max(0)..(sx + sw).min(d.target_w as i32) {
                let p = base.add(y as usize * pitch + x as usize);
                let w = *p;
                let old = if ten {
                    [w & 0x3ff, w >> 10 & 0x3ff, w >> 20 & 0x3ff, w >> 30]
                } else {
                    w.to_le_bytes().map(u32::from)
                };
                let f: Vec<u32> =
                    (0..4).map(|i| if d.clear_color_mask >> i & 1 != 0 { new[i] } else { old[i] }).collect();
                *p = if ten {
                    f[0] | f[1] << 10 | f[2] << 20 | f[3] << 30
                } else {
                    f[0] | f[1] << 8 | f[2] << 16 | f[3] << 24
                };
            }
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

static BACKEND: YakoglBackend = YakoglBackend {
    name: c"es3-clear-rebind".as_ptr() as *const c_char,
    native_attrib_formats: 0,
    rt_rgba8888_is_bgra: false,
    mip_level_align: 0,
    compressed_blocks_in_rows: false,
    native_layered_textures: false,
    front_facing_shader_variant: false,
    etc1_decoded_at_upload: false,
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
    max_instance_divisor: 1,
    wait_kick: None,
    samples_texel_copies: false,
    attachment_gone: None,
    depth_resolve: None,
    query_begin: None,
    query_end: None,
    query_result: None,
    generate_mipmap: None,
    return_mipmap_chain: None,
    linear_row_align: 0,
    uniforms_host_only: false,
    ds_write: None,
    stencil_resolve: None,
};

static LOCK: Mutex<()> = Mutex::new(());
static ONCE: Once = Once::new();

fn context() -> MutexGuard<'static, ()> {
    let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    ONCE.call_once(|| {
        // SAFETY: both tables are statics; the lock above serialises the tests.
        unsafe {
            osal_set_ops(&OSAL);
            backend::g_backend = &BACKEND;
            yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
            assert!(!yakogl::gl::context::gl_context_create(8, 8).is_null());
        }
    });
    // SAFETY: plain GL calls.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glFinish();
    }
    while err() != GL_NO_ERROR {}
    g
}

fn err() -> GLenum {
    // SAFETY: plain GL call.
    unsafe { glGetError() }
}

fn scale(v: u32, max: u32) -> u8 {
    ((v * 255 + max / 2) / max) as u8
}

const N: GLsizei = 4;

#[derive(Clone, Copy, Debug, PartialEq)]
enum Fmt {
    Rgba8,
    Rgb10A2,
}

#[derive(Clone, Copy, Debug)]
enum Site {
    Textures,
    Layers,
    Renderbuffers,
}

/// The pixel word site `i` (0 = A, 1 = B) of a texture starts with.
fn initial_word(f: Fmt, i: u32) -> u32 {
    match f {
        Fmt::Rgba8 => u32::from_le_bytes([200 - i as u8, 30 + i as u8, 90, 255 - i as u8]),
        Fmt::Rgb10A2 => (1000 - i) | (5 + i) << 10 | 600 << 20 | 1 << 30,
    }
}

/// What glReadPixels RGBA/UNSIGNED_BYTE gives for a word.
fn decoded(f: Fmt, w: u32) -> [u8; 4] {
    match f {
        Fmt::Rgba8 => w.to_le_bytes(),
        Fmt::Rgb10A2 => {
            [scale(w & 0x3ff, 1023), scale((w >> 10) & 0x3ff, 1023), scale((w >> 20) & 0x3ff, 1023), scale(w >> 30, 3)]
        }
    }
}

/// glClearBufferfv's colour: exactly representable in the format.
fn clear_value(f: Fmt) -> [f32; 4] {
    match f {
        Fmt::Rgba8 => [51.0 / 255.0, 102.0 / 255.0, 153.0 / 255.0, 204.0 / 255.0],
        Fmt::Rgb10A2 => [300.0 / 1023.0, 700.0 / 1023.0, 50.0 / 1023.0, 2.0 / 3.0],
    }
}

fn clear_bytes(f: Fmt) -> [u8; 4] {
    match f {
        Fmt::Rgba8 => [51, 102, 153, 204],
        Fmt::Rgb10A2 => decoded(f, 300 | 700 << 10 | 50 << 20 | 2 << 30),
    }
}

fn internal(f: Fmt) -> GLenum {
    match f {
        Fmt::Rgba8 => GL_RGBA8,
        Fmt::Rgb10A2 => GL_RGB10_A2,
    }
}

fn upload(f: Fmt, target: GLenum, layers: bool, words: &[u32]) {
    let ty = if f == Fmt::Rgba8 { GL_UNSIGNED_BYTE } else { GL_UNSIGNED_INT_2_10_10_10_REV };
    // SAFETY: `words` holds N x N (x layers) 32-bit pixels.
    unsafe {
        if layers {
            glTexImage3D(target, 0, internal(f) as GLint, N, N, 2, 0, GL_RGBA, ty, words.as_ptr() as *const c_void);
        } else {
            glTexImage2D(target, 0, internal(f) as GLint, N, N, 0, GL_RGBA, ty, words.as_ptr() as *const c_void);
        }
        glTexParameteri(target, GL_TEXTURE_MIN_FILTER, GL_NEAREST as GLint);
    }
}

/// Two images of `site`: two names (textures or renderbuffers); for layers
/// the one texture is named twice.
fn make_images(f: Fmt, site: Site) -> [GLuint; 2] {
    let mut n = [0; 2];
    // SAFETY: room for two names; every upload holds N x N pixels per layer.
    unsafe {
        match site {
            Site::Textures => {
                glGenTextures(2, n.as_mut_ptr());
                for i in 0..2 {
                    glBindTexture(GL_TEXTURE_2D, n[i]);
                    upload(f, GL_TEXTURE_2D, false, &vec![initial_word(f, i as u32); (N * N) as usize]);
                }
            }
            Site::Layers => {
                glGenTextures(1, n.as_mut_ptr());
                glBindTexture(GL_TEXTURE_2D_ARRAY, n[0]);
                let mut px = vec![initial_word(f, 0); (N * N) as usize];
                px.extend(vec![initial_word(f, 1); (N * N) as usize]);
                upload(f, GL_TEXTURE_2D_ARRAY, true, &px);
                n[1] = n[0];
            }
            Site::Renderbuffers => {
                glGenRenderbuffers(2, n.as_mut_ptr());
                for i in 0..2 {
                    glBindRenderbuffer(GL_RENDERBUFFER, n[i]);
                    glRenderbufferStorage(GL_RENDERBUFFER, internal(f), N, N);
                }
            }
        }
    }
    assert_eq!(err(), GL_NO_ERROR, "make images {f:?} {site:?}");
    n
}

/// The bytes image `i` reads as before any clear: a renderbuffer is zeroed.
fn initial_bytes(f: Fmt, site: Site, i: u32) -> [u8; 4] {
    match site {
        Site::Renderbuffers => [0; 4],
        _ => decoded(f, initial_word(f, i)),
    }
}

fn attach(site: Site, n: [GLuint; 2], i: usize) {
    // SAFETY: plain GL calls.
    unsafe {
        match site {
            Site::Textures => glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, n[i], 0),
            Site::Layers => glFramebufferTextureLayer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, n[0], 0, i as GLint),
            Site::Renderbuffers => {
                glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_RENDERBUFFER, n[i])
            }
        }
    }
    assert_eq!(err(), GL_NO_ERROR, "attach {site:?} image {i}");
}

/// Reads the bound framebuffer; returns the first pixel and whether all pixels agree.
fn read_bound() -> ([u8; 4], bool) {
    let mut px = vec![0u8; (N * N * 4) as usize];
    // SAFETY: `px` holds N x N RGBA8 pixels.
    unsafe { glReadPixels(0, 0, N, N, GL_RGBA, GL_UNSIGNED_BYTE, px.as_mut_ptr() as *mut c_void) };
    assert_eq!(err(), GL_NO_ERROR, "glReadPixels");
    let first = [px[0], px[1], px[2], px[3]];
    (first, px.chunks(4).all(|c| c == first))
}

fn read_image(fb: GLuint, site: Site, n: [GLuint; 2], i: usize) -> [u8; 4] {
    // SAFETY: plain GL call.
    unsafe { glBindFramebuffer(GL_FRAMEBUFFER, fb) };
    attach(site, n, i);
    // SAFETY: plain GL call.
    assert_eq!(unsafe { glCheckFramebufferStatus(GL_FRAMEBUFFER) }, GL_FRAMEBUFFER_COMPLETE, "{site:?} image {i}");
    let (p, uniform) = read_bound();
    assert!(uniform, "{site:?} image {i}: pixels differ");
    p
}

/// Clear A, optionally read it back (control), rebind B, read B, read A.
fn run(f: Fmt, site: Site, read_between: bool) {
    let _g = context();
    let n = make_images(f, site);
    let mut fb = 0;
    // SAFETY: room for one name.
    unsafe {
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
    }
    // Starting contents, as a check of the set-up.
    for i in 0..2 {
        assert_eq!(read_image(fb, site, n, i), initial_bytes(f, site, i as u32), "{f:?} {site:?} initial image {i}");
    }
    attach(site, n, 0);
    // SAFETY: plain GL call.
    unsafe { glDrawBuffers(1, [GL_COLOR_ATTACHMENT0].as_ptr()) };
    let c = clear_value(f);
    // SAFETY: `c` holds four floats.
    unsafe { glClearBufferfv(GL_COLOR, 0, c.as_ptr()) };
    assert_eq!(err(), GL_NO_ERROR, "glClearBufferfv");
    if read_between {
        let (p, uniform) = read_bound();
        assert!(uniform);
        assert_eq!(p, clear_bytes(f), "{f:?} {site:?}: A read before the rebind");
    }
    attach(site, n, 1);
    let b = read_image(fb, site, n, 1);
    let a = read_image(fb, site, n, 0);
    // SAFETY: names made above.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glDeleteFramebuffers(1, &fb);
        match site {
            Site::Textures | Site::Layers => {
                glDeleteTextures(if matches!(site, Site::Layers) { 1 } else { 2 }, n.as_ptr())
            }
            Site::Renderbuffers => glDeleteRenderbuffers(2, n.as_ptr()),
        }
    }
    assert!(
        a == clear_bytes(f) && b == initial_bytes(f, site, 1),
        "{f:?} {site:?}: A = {a:?} (want {:?}), B = {b:?} (want {:?})",
        clear_bytes(f),
        initial_bytes(f, site, 1)
    );
}

#[test]
fn rgba8_textures_control() {
    run(Fmt::Rgba8, Site::Textures, true);
}

#[test]
fn rgb10a2_textures_control() {
    run(Fmt::Rgb10A2, Site::Textures, true);
}

#[test]
fn rgba8_layers_control() {
    run(Fmt::Rgba8, Site::Layers, true);
}

#[test]
fn rgb10a2_layers_control() {
    run(Fmt::Rgb10A2, Site::Layers, true);
}

#[test]
fn rgba8_renderbuffers_control() {
    run(Fmt::Rgba8, Site::Renderbuffers, true);
}

#[test]
fn rgb10a2_renderbuffers_control() {
    run(Fmt::Rgb10A2, Site::Renderbuffers, true);
}

#[test]
#[ignore = "defect: a pending clear lands on the attachment bound at the flush, not at the glClearBufferfv call: A stays [200, 30, 90, 255], B reads the clear [51, 102, 153, 204]"]
fn rgba8_textures() {
    run(Fmt::Rgba8, Site::Textures, false);
}

#[test]
#[ignore = "defect: a pending clear lands on the attachment bound at the flush, not at the glClearBufferfv call: A stays [249, 1, 150, 85], B reads the clear [75, 174, 12, 170]"]
fn rgb10a2_textures() {
    run(Fmt::Rgb10A2, Site::Textures, false);
}

#[test]
#[ignore = "defect: a pending clear lands on the attachment bound at the flush, not at the glClearBufferfv call: A stays [200, 30, 90, 255], B reads the clear [51, 102, 153, 204]"]
fn rgba8_layers() {
    run(Fmt::Rgba8, Site::Layers, false);
}

#[test]
#[ignore = "defect: a pending clear lands on the attachment bound at the flush, not at the glClearBufferfv call: A stays [249, 1, 150, 85], B reads the clear [75, 174, 12, 170]"]
fn rgb10a2_layers() {
    run(Fmt::Rgb10A2, Site::Layers, false);
}

#[test]
#[ignore = "defect: a pending clear lands on the attachment bound at the flush, not at the glClearBufferfv call: A stays [0, 0, 0, 0], B reads the clear [51, 102, 153, 204]"]
fn rgba8_renderbuffers() {
    run(Fmt::Rgba8, Site::Renderbuffers, false);
}

#[test]
#[ignore = "defect: a pending clear lands on the attachment bound at the flush, not at the glClearBufferfv call: A stays [0, 0, 0, 0], B reads the clear [75, 174, 12, 170]"]
fn rgb10a2_renderbuffers() {
    run(Fmt::Rgb10A2, Site::Renderbuffers, false);
}
