// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GL_RGB10_A2 texture upload, glGenerateMipmap and clears where the build
//! has no class for the format (plan #370 item texclear-fallback-pin).
//!
//! On the V3D 4.2 build GL_RGB10_A2 is its own texture class and render
//! surface (`GLES_TEXFMT_RGB10A2` / `GLES_SURF_RGB10_A2`, the 10/10/10/2
//! word: src/gl/framebuffer.rs `rgb10a2_surface`, docs/notes/v3d-rgb10a2.md);
//! there the word is stored, the box filter averages it field by field and
//! the read-back decodes it (tests/es3_rgb10a2_texture_v3d.rs,
//! tests/es3_rgb10a2_clear_v3d.rs). Everywhere else a GL_RGB10_A2 renderbuffer
//! is RGBA8888 and a texture of the name is in the RGBA8 class (es3_formats'
//! fallback): the PowerVR and GC7000 builds and every `report-es2` build.
//! This file holds what those paths answer there today, so that the class
//! build's word rules cannot reach them (sibling of
//! tests/es3_rgb10a2_fallback.rs and tests/es3_rgb10a2_copytex_fallback.rs,
//! same stand-in OSAL and back end, no GPU):
//!
//! - a GL_RGB10_A2 texture uploaded with GL_RGBA /
//!   GL_UNSIGNED_INT_2_10_10_10_REV (no error; the pair the fallback takes)
//!   stores each word as the RGBA8 bytes the generic ES 3.0 path makes of it:
//!   the field decoded to a float (c / 1023, alpha c / 3) and encoded as
//!   (v * 255 + 0.5) truncated, alpha 1 -> 85 and 2 -> 170, not kept as a
//!   word (es3_formats' `decode` / `encode`, `GLES_TEXFMT_RGBA8`);
//! - glGenerateMipmap on that texture is the RGBA8 class box filter: every
//!   stored byte averaged on its own, (sum + 2) / 4 (`mip_box_row_u8`), not
//!   the class build's field-by-field average of the word. The two rules
//!   differ at the alpha of a box holding 0, 1, 2 and 3 (byte average 128,
//!   field average 2 of 3, decoded 170), at the alpha of a box of 0, 0, 0 and
//!   3 (64 vs 85) and at the red of a box of 875, 249, 207 and 697 (127 vs
//!   126): the wanted bytes below pin the byte rule at each of them;
//! - a glClear or glClearBufferfv of a GL_RGB10_A2 renderbuffer or texture
//!   attachment reaches the back end as an RGBA8888 target and reads back the
//!   RGBA8 bytes with GL_RGBA / GL_UNSIGNED_BYTE (alpha 0.4 is 102, where the
//!   class build's 2-bit alpha would decode 85); no `GLES_SURF_RGB10_A2`
//!   target is ever handed out.
//!
//! An ES 2.0-reporting build cannot allocate GL_RGB10_A2 at all (ES 2.0 names
//! no such format), so nothing here is reachable there: the renderbuffer
//! storage is GL_INVALID_ENUM, the texture with an ES 2.0 type GL_INVALID_VALUE
//! and with GL_UNSIGNED_INT_2_10_10_10_REV GL_INVALID_ENUM (as
//! tests/es3_rgb10a2_fallback.rs holds; those refusals are what its rows run).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online,test-knobs --test es3_rgb10a2_texclear_fallback
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online,test-knobs --test es3_rgb10a2_texclear_fallback
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table,report-es2 --test es3_rgb10a2_texclear_fallback
//! ```
//!
//! Compiled only where the class falls back: the V3D build without
//! `report-es2` stores the class and runs zero tests of this file (its answers
//! are tests/es3_rgb10a2_texture_v3d.rs and tests/es3_rgb10a2_clear_v3d.rs).

#![cfg(all(feature = "host-test", any(feature = "gpu-powervr", feature = "gpu-vivante", feature = "report-es2")))]
// the ES 2.0 report runs the refusal pins only; the helpers of the rest are idle there
#![cfg_attr(feature = "report-es2", allow(dead_code, unused_imports))]

use core::ffi::{c_char, c_void};
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{self, GlesDraw, GlesHeap, YakoglBackend};
use yakogl::gl::consts::*;
use yakogl::gl::es3::*;
use yakogl::gl::framebuffer::*;
use yakogl::gl::state::*;
use yakogl::gl::texture::*;
use yakogl::gl::types::*;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};

/* ---- a context on the host: the smallest OSAL and backend that make one --- */

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

/// Targets the stand-in back end was handed: (surface, stored a clear).
static DRAWS: Mutex<Vec<(u8, bool)>> = Mutex::new(Vec::new());

/// A clear as a back end stores it into an RGBA8888 target: the colour
/// clamped to [0, 1] as one byte a channel, R, G, B, A, every pixel, rows at
/// the pair-padded pitch. Every other target is left alone.
unsafe extern "C" fn b_draw(d: *const GlesDraw) -> i32 {
    let d = &*d;
    for i in 0..d.num_targets as usize {
        let t = &d.targets[i];
        let hit = d.do_clear && (d.clear_target_mask == 0 || d.clear_target_mask & (1 << i) != 0) && t.gpu != 0;
        let rgba8 = t.format == backend::GLES_SURF_RGBA8888;
        DRAWS.lock().unwrap().push((t.format, hit && rgba8));
        if !hit || !rgba8 {
            continue;
        }
        let px = d.clear_rgba.map(unorm8);
        let pitch = ((t.w + 1) & !1) as usize;
        let base = t.gpu as *mut u8;
        for y in 0..t.h as usize {
            for x in 0..t.w as usize {
                core::ptr::copy_nonoverlapping(px.as_ptr(), base.add((y * pitch + x) * 4), 4);
            }
        }
    }
    0
}

/// A colour channel as the byte an RGBA8 target stores: clamped, rounded.
fn unorm8(v: f32) -> u8 {
    (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8
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
    name: c"es3-rgb10a2-texclear-fallback".as_ptr() as *const c_char,
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

/// One context for the whole file; the tests take turns on it.
static LOCK: Mutex<()> = Mutex::new(());
static ONCE: Once = Once::new();

fn context() -> MutexGuard<'static, ()> {
    let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    ONCE.call_once(|| {
        // SAFETY: both tables are statics; the lock above serialises the tests.
        unsafe {
            osal_set_ops(&OSAL);
            backend::g_backend = &BACKEND;
            assert!(!yakogl::gl::context::gl_context_create(8, 8).is_null());
        }
    });
    // SAFETY: plain GL calls.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glFinish();
    }
    DRAWS.lock().unwrap().clear();
    // SAFETY: plain GL call.
    while unsafe { glGetError() } != GL_NO_ERROR {}
    g
}

fn err() -> GLenum {
    // SAFETY: plain GL call.
    unsafe { glGetError() }
}

fn renderbuffer(format: GLenum, w: GLsizei, h: GLsizei) -> GLuint {
    let mut rb = 0;
    // SAFETY: room for one name.
    unsafe {
        glGenRenderbuffers(1, &mut rb);
        glBindRenderbuffer(GL_RENDERBUFFER, rb);
        glRenderbufferStorage(GL_RENDERBUFFER, format, w, h);
    }
    rb
}

/// A texture of `w` x `h`, its level 0 from `pixels` (null: only allocate).
fn texture(
    internalformat: GLenum,
    format: GLenum,
    ty: GLenum,
    w: GLsizei,
    h: GLsizei,
    pixels: *const c_void,
) -> GLuint {
    let mut t = 0;
    // SAFETY: room for one name; `pixels` is null or holds w x h texels.
    unsafe {
        glGenTextures(1, &mut t);
        glBindTexture(GL_TEXTURE_2D, t);
        glTexImage2D(GL_TEXTURE_2D, 0, internalformat as GLint, w, h, 0, format, ty, pixels);
    }
    t
}

/// A 10/10/10/2 word (R in bits 0..9, G 10..19, B 20..29, A 30..31).
const fn word(r: u32, g: u32, b: u32, a: u32) -> u32 {
    r | g << 10 | b << 20 | a << 30
}

/// An RGB10_A2 texture of `w` x `h`, uploaded through the pair the fallback
/// takes for it (GL_RGBA / GL_UNSIGNED_INT_2_10_10_10_REV).
fn rgb10a2_texture(w: GLsizei, h: GLsizei, words: Option<&[u32]>) -> GLuint {
    let p = words.map_or(core::ptr::null(), |w| w.as_ptr() as *const c_void);
    texture(GL_RGB10_A2, GL_RGBA, GL_UNSIGNED_INT_2_10_10_10_REV, w, h, p)
}

/// A framebuffer, bound to GL_FRAMEBUFFER, with renderbuffer `rb` on colour
/// attachment 0.
fn rb_framebuffer(rb: GLuint) -> GLuint {
    let mut fb = 0;
    // SAFETY: room for one name.
    unsafe {
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_RENDERBUFFER, rb);
    }
    fb
}

/// A framebuffer, bound to GL_FRAMEBUFFER, with level `level` of texture `tex`
/// on colour attachment 0.
fn tex_framebuffer(tex: GLuint, level: GLint) -> GLuint {
    let mut fb = 0;
    // SAFETY: room for one name.
    unsafe {
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex, level);
    }
    fb
}

fn release(fbs: &[GLuint], rbs: &[GLuint], texs: &[GLuint]) {
    // SAFETY: names this test made.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glDeleteFramebuffers(fbs.len() as GLsizei, fbs.as_ptr());
        glDeleteRenderbuffers(rbs.len() as GLsizei, rbs.as_ptr());
        glDeleteTextures(texs.len() as GLsizei, texs.as_ptr());
    }
}

fn status() -> GLenum {
    // SAFETY: plain GL call.
    unsafe { glCheckFramebufferStatus(GL_FRAMEBUFFER) }
}

/// The bound read framebuffer's w x h pixels as GL_RGBA / GL_UNSIGNED_BYTE,
/// four bytes a pixel, the buffer prefilled with 0x77 so that a read that
/// writes nothing shows.
fn read(w: GLsizei, h: GLsizei) -> Vec<u8> {
    let mut px = vec![0x77u8; (w * h * 4) as usize];
    // SAFETY: room for w x h pixels of four bytes.
    unsafe { glReadPixels(0, 0, w, h, GL_RGBA, GL_UNSIGNED_BYTE, px.as_mut_ptr() as *mut c_void) };
    px
}

/// The clear of the clear tests and the RGBA8 bytes it is stored as:
/// 0.2 * 255 = 51, 0.6 * 255 = 153, 1.5 clamps to 255, alpha 0.4 is 102.
/// As a 10/10/10/2 word decoded per channel the same clear would read
/// 51, 153, 255, 85 (alpha 0.4 is 1 of 3): the alpha tells the two apart.
const CLEAR: [f32; 4] = [0.2, 0.6, 1.5, 0.4];
const CLEAR_BYTES: [u8; 4] = [51, 153, 255, 102];

/// The second clear (glClearBufferfv's own value) and its bytes: 0.25 * 255
/// = 63.75 -> 64, 0.75 * 255 = 191.25 -> 191, 0.5 * 255 = 127.5 -> 128, alpha
/// 0.4 again 102 (the 2-bit alpha of the class build would decode 85).
const CLEAR2: [f32; 4] = [0.25, 0.75, 0.5, 0.4];
const CLEAR2_BYTES: [u8; 4] = [64, 191, 128, 102];

/// Clear the bound framebuffer to `c` with glClear, then with
/// glClearBufferfv, and read its w x h pixels back as GL_RGBA /
/// GL_UNSIGNED_BYTE after each.
fn clear_and_read(w: GLsizei, h: GLsizei) -> (Vec<u8>, Vec<u8>) {
    // SAFETY: plain GL calls.
    unsafe {
        glClearColor(CLEAR[0], CLEAR[1], CLEAR[2], CLEAR[3]);
        glClear(GL_COLOR_BUFFER_BIT);
    }
    assert_eq!(err(), GL_NO_ERROR, "glClear");
    let px = read(w, h);
    assert_eq!(err(), GL_NO_ERROR, "glReadPixels after glClear");
    // SAFETY: four floats of room.
    unsafe { glClearBufferfv(GL_COLOR, 0, CLEAR2.as_ptr()) };
    assert_eq!(err(), GL_NO_ERROR, "glClearBufferfv");
    let px2 = read(w, h);
    assert_eq!(err(), GL_NO_ERROR, "glReadPixels after glClearBufferfv");
    (px, px2)
}

/// What every pixel of `px` must be, `want` a single pixel.
fn assert_uniform(px: &[u8], want: &[u8; 4]) {
    for (i, p) in px.as_chunks::<4>().0.iter().enumerate() {
        assert_eq!(*p, *want, "pixel {i}");
    }
}

/* ---- the ES 2.0 report: GL_RGB10_A2 is no format ---------------------------- */

#[cfg(feature = "report-es2")]
mod es2 {
    use super::*;

    /// ES 2.0 4.4.3 names no GL_RGB10_A2 renderbuffer storage and 3.7.1 no
    /// such texture internal format: neither is allocated, so no upload,
    /// mipmap or clear of the class build can be reached; the texture only
    /// with an ES 2.0 type is GL_INVALID_VALUE, with ES 3.0's
    /// GL_UNSIGNED_INT_2_10_10_10_REV (no ES 2.0 type) GL_INVALID_ENUM
    /// (tests/es2_report_guard.rs, tests/es3_rgb10a2_fallback.rs).
    #[test]
    fn texclear_fallback_es2_rgb10a2_is_not_allocated() {
        let _g = context();
        let rb = renderbuffer(GL_RGB10_A2, 4, 2);
        assert_eq!(err(), GL_INVALID_ENUM, "glRenderbufferStorage(GL_RGB10_A2)");
        let tex = texture(GL_RGB10_A2, GL_RGBA, GL_UNSIGNED_BYTE, 4, 2, core::ptr::null());
        assert_eq!(err(), GL_INVALID_VALUE, "glTexImage2D(GL_RGB10_A2, GL_RGBA, GL_UNSIGNED_BYTE)");
        let tex2 = texture(GL_RGB10_A2, GL_RGBA, GL_UNSIGNED_INT_2_10_10_10_REV, 4, 2, core::ptr::null());
        assert_eq!(err(), GL_INVALID_ENUM, "glTexImage2D(GL_RGB10_A2, .., GL_UNSIGNED_INT_2_10_10_10_REV)");
        release(&[], &[rb], &[tex, tex2]);
    }
}

/* ---- ES 3.0 on a build without the class -------------------------------------- */

#[cfg(not(feature = "report-es2"))]
mod es3 {
    use super::*;

    /// The premise: this build's back end does not draw the 10/10/10/2
    /// surface, so GL_RGB10_A2 falls back to RGBA8 (framebuffer.rs
    /// `rgb10a2_surface`).
    #[test]
    fn texclear_fallback_build_has_no_class() {
        assert!(!backend::rt_format_drawable(backend::GLES_SURF_RGB10_A2));
    }

    /// A 4 x 4 texture's fields, row 0 first. Edge values (0, 1, 2, 3, 341,
    /// 511, 512, 1022, 1023, all four alphas) and one box (x 2..3, y 0..1)
    /// whose byte average differs from the class build's field average at its
    /// red channel; every box's alpha differs there.
    const GRID: [[u32; 4]; 16] = [
        [0, 0, 0, 0],
        [341, 341, 341, 1],
        [875, 249, 207, 0],
        [249, 697, 640, 0],
        [511, 512, 1022, 2],
        [1023, 1023, 1023, 3],
        [207, 138, 461, 0],
        [697, 894, 258, 3],
        [1, 2, 3, 1],
        [2, 3, 1, 1],
        [0, 1023, 0, 3],
        [1023, 0, 1023, 0],
        [3, 1, 2, 2],
        [1022, 1021, 1020, 2],
        [512, 511, 512, 1],
        [341, 682, 1023, 2],
    ];

    /// [`GRID`] as 10/10/10/2 words.
    fn grid_words() -> Vec<u32> {
        GRID.iter().map(|c| word(c[0], c[1], c[2], c[3])).collect()
    }

    /// Level 0 as the RGBA8 class stores [`GRID`]: the field decoded to
    /// c / 1023 (alpha c / 3) and encoded as (v * 255 + 0.5) truncated
    /// (es3_formats' decode to floats and `GLES_TEXFMT_RGBA8` store).
    const WANTED_L0: [[u8; 4]; 16] = [
        [0, 0, 0, 0],
        [85, 85, 85, 85],
        [218, 62, 52, 0],
        [62, 174, 160, 0],
        [127, 128, 255, 170],
        [255, 255, 255, 255],
        [52, 34, 115, 0],
        [174, 223, 64, 255],
        [0, 0, 1, 85],
        [0, 1, 0, 85],
        [0, 255, 0, 255],
        [255, 0, 255, 0],
        [1, 0, 0, 170],
        [255, 255, 254, 170],
        [128, 127, 128, 85],
        [85, 170, 255, 170],
    ];

    /// Level 1 (2 x 2) after glGenerateMipmap: each byte the (sum + 2) / 4
    /// average of the four stored bytes of its 2 x 2 box (`mip_box_row_u8`).
    /// The class build's word rules would answer alpha 170, 85, 170, 170 and
    /// a red of 126 in the second box; the byte rule answers 128, 64, 128,
    /// 128 and 127.
    const WANTED_L1: [[u8; 4]; 4] = [[117, 117, 149, 128], [127, 123, 98, 64], [64, 64, 64, 128], [117, 138, 160, 128]];

    /// The 2 x 2 texture of the rounding row and its stored bytes: the four
    /// fields 511, 511, 512 and 1 average to 96 (382 + 2 over 4) where the
    /// field average 384 decodes to 96 as well, the greens 1023, 0, 0 and
    /// 1022 to 128 where the field average 511 decodes to 127.
    const ROUND: [[u32; 4]; 4] = [[511, 1023, 0, 1], [511, 0, 0, 1], [512, 0, 1023, 2], [1, 1022, 341, 0]];
    const ROUND_L1: [u8; 4] = [96, 128, 85, 85];

    /// Concatenate the wanted bytes of a pixel list.
    fn want_bytes(pixels: &[[u8; 4]]) -> Vec<u8> {
        pixels.iter().flat_map(|p| p.iter().copied()).collect()
    }

    /// Read level 0 of `tex` through a new framebuffer, which stays bound;
    /// returns its name with the bytes.
    fn read_texture(tex: GLuint, w: GLsizei, h: GLsizei) -> (GLuint, Vec<u8>) {
        let fb = tex_framebuffer(tex, 0);
        assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE);
        let px = read(w, h);
        assert_eq!(err(), GL_NO_ERROR, "glReadPixels of level 0");
        (fb, px)
    }

    /// A GL_RGB10_A2 texture uploaded with GL_RGBA /
    /// GL_UNSIGNED_INT_2_10_10_10_REV: the pair the fallback takes without
    /// error, its texels stored as the RGBA8 bytes of the words and read back
    /// unchanged (no 10/10/10/2 word, no decode).
    #[test]
    fn texclear_fallback_upload_stores_rgba8_bytes() {
        let _g = context();
        let words = grid_words();
        let tex = rgb10a2_texture(4, 4, Some(&words));
        assert_eq!(err(), GL_NO_ERROR, "glTexImage2D(GL_RGB10_A2) with words");
        let (fb, px) = read_texture(tex, 4, 4);
        assert_eq!(px, want_bytes(&WANTED_L0), "the stored RGBA8 bytes of the words");
        release(&[fb], &[], &[tex]);
    }

    /// glGenerateMipmap on such a texture is the RGBA8 class box filter:
    /// level 0 untouched, level 1 the byte average of each 2 x 2 box, not the
    /// class build's average of the 10/10/10/2 words.
    #[test]
    fn texclear_fallback_generate_mipmap_averages_bytes() {
        let _g = context();
        let words = grid_words();
        let tex = rgb10a2_texture(4, 4, Some(&words));
        assert_eq!(err(), GL_NO_ERROR);
        // SAFETY: plain GL call.
        unsafe { glGenerateMipmap(GL_TEXTURE_2D) };
        assert_eq!(err(), GL_NO_ERROR, "glGenerateMipmap");
        let fb = tex_framebuffer(tex, 0);
        assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE);
        let px = read(4, 4);
        assert_eq!(err(), GL_NO_ERROR);
        assert_eq!(px, want_bytes(&WANTED_L0), "level 0 untouched");
        // SAFETY: plain GL calls on a framebuffer this test made.
        unsafe { glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex, 1) };
        assert_eq!(err(), GL_NO_ERROR, "attach level 1");
        assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE, "level 1");
        let px = read(2, 2);
        assert_eq!(err(), GL_NO_ERROR, "glReadPixels of level 1");
        assert_eq!(px, want_bytes(&WANTED_L1), "the byte box average");
        release(&[fb], &[], &[tex]);
    }

    /// One 2 x 2 box: the half-up byte averages, including the green where
    /// the class build's field average would decode 127 instead of 128.
    #[test]
    fn texclear_fallback_mipmap_rounds_half_up_in_bytes() {
        let _g = context();
        let words: Vec<u32> = ROUND.iter().map(|c| word(c[0], c[1], c[2], c[3])).collect();
        let tex = rgb10a2_texture(2, 2, Some(&words));
        assert_eq!(err(), GL_NO_ERROR);
        // SAFETY: plain GL call.
        unsafe { glGenerateMipmap(GL_TEXTURE_2D) };
        assert_eq!(err(), GL_NO_ERROR, "glGenerateMipmap");
        let fb = tex_framebuffer(tex, 1);
        assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE, "level 1");
        let px = read(1, 1);
        assert_eq!(err(), GL_NO_ERROR);
        assert_eq!(px, want_bytes(&[ROUND_L1]), "the byte box average of the one box");
        release(&[fb], &[], &[tex]);
    }

    /// A cleared GL_RGB10_A2 renderbuffer reaches the back end as an
    /// RGBA8888 target (glClear and glClearBufferfv alike) and reads back the
    /// RGBA8 bytes it holds; no 10/10/10/2 surface is ever handed out.
    #[test]
    fn texclear_fallback_renderbuffer_clear_reads_rgba8() {
        let _g = context();
        let rb = renderbuffer(GL_RGB10_A2, 4, 2);
        assert_eq!(err(), GL_NO_ERROR, "glRenderbufferStorage(GL_RGB10_A2)");
        let fb = rb_framebuffer(rb);
        assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE);
        let (px, px2) = clear_and_read(4, 2);
        let draws = DRAWS.lock().unwrap().clone();
        assert_eq!(draws.iter().filter(|d| d.1).count(), 2, "both clears went out as RGBA8888: {draws:?}");
        assert!(draws.iter().all(|d| d.0 == backend::GLES_SURF_RGBA8888), "only RGBA8888: {draws:?}");
        assert!(draws.iter().all(|d| d.0 != backend::GLES_SURF_RGB10_A2), "no 10/10/10/2 surface: {draws:?}");
        assert_uniform(&px, &CLEAR_BYTES);
        assert_uniform(&px2, &CLEAR2_BYTES);
        release(&[fb], &[rb], &[]);
    }

    /// The same for a GL_RGB10_A2 texture attachment: the words it was
    /// uploaded as are RGBA8 bytes, and a clear overwrites them with the
    /// clear's RGBA8 bytes.
    #[test]
    fn texclear_fallback_texture_clear_reads_rgba8() {
        let _g = context();
        let words = grid_words();
        let tex = rgb10a2_texture(4, 2, Some(&words[..8]));
        assert_eq!(err(), GL_NO_ERROR, "glTexImage2D(GL_RGB10_A2) with words");
        let fb = tex_framebuffer(tex, 0);
        assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE);
        let (px, px2) = clear_and_read(4, 2);
        let draws = DRAWS.lock().unwrap().clone();
        assert_eq!(draws.iter().filter(|d| d.1).count(), 2, "both clears went out as RGBA8888: {draws:?}");
        assert!(draws.iter().all(|d| d.0 != backend::GLES_SURF_RGB10_A2), "no 10/10/10/2 surface: {draws:?}");
        assert_uniform(&px, &CLEAR_BYTES);
        assert_uniform(&px2, &CLEAR2_BYTES);
        release(&[fb], &[], &[tex]);
    }
}
