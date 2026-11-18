// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GL_RGB10_A2 immutable textures made with glTexStorage2D / glTexStorage3D
//! where the build has no class for the format (plan #370 item
//! texstorage-fallback-pin).
//!
//! On the V3D 4.2 build GL_RGB10_A2 is its own texture class and render
//! surface (`GLES_TEXFMT_RGB10A2` / `GLES_SURF_RGB10_A2`, src/gl/framebuffer.rs
//! `rgb10a2_surface`, docs/notes/v3d-rgb10a2.md), so glTexStorage* lays that
//! class and its 10/10/10/2 word. Everywhere else the format's storage list
//! (`es3_formats`' row for GL_RGB10_A2, `Es3Format::storage_here`) resolves to
//! the RGBA8 class: the PowerVR and GC7000 builds and every `report-es2`
//! build. This file holds what the immutable path answers there today, so that
//! the class build's word rules cannot reach them (sibling of
//! tests/es3_rgb10a2_texclear_fallback.rs, same stand-in OSAL and back end, no
//! GPU; texclear_fallback, layers_fallback, copytex_fallback and
//! packmsaa_fallback lay their textures out with glTexImage2D/3D -- this one
//! lays them out with glTexStorage2D/3D only, and fills them with
//! glTexSubImage2D):
//!
//! - the store is immutable (ES 3.0 3.8.4): GL_TEXTURE_IMMUTABLE_FORMAT reads
//!   1 and GL_TEXTURE_IMMUTABLE_LEVELS the levels the call was given, on a
//!   GL_TEXTURE_2D, a cube map, a GL_TEXTURE_2D_ARRAY and a GL_TEXTURE_3D
//!   alike, through either entry point;
//! - every level of such a texture, every cube face and every array layer
//!   attaches GL_FRAMEBUFFER_COMPLETE -- before a single texel is uploaded,
//!   which is the difference from glTexImage2D -- and every one of them answers
//!   the RGBA8 class the name fell back to: eight bits a channel,
//!   GL_UNSIGNED_NORMALIZED, GL_LINEAR, and GL_IMPLEMENTATION_COLOR_READ_FORMAT
//!   / _TYPE stay GL_RGBA / GL_UNSIGNED_BYTE (the answers queries-fallback-pin
//!   holds for a glTexImage-made attachment; the class build answers
//!   10/10/10/2, tests/es3_rgb10a2_queries_v3d.rs);
//! - an upload with GL_RGBA / GL_UNSIGNED_INT_2_10_10_10_REV (table 3.2's one
//!   pair for the name, and the pair glTexStorage* records for it) stores each
//!   word as the RGBA8 bytes the generic ES 3.0 path makes of it: the field
//!   decoded to a float (c / 1023, alpha c / 3) and encoded as (v * 255 + 0.5)
//!   truncated, alpha 1 -> 85 and 2 -> 170 (es3_formats' `decode` / `encode`,
//!   `GLES_TEXFMT_RGBA8`), not kept as a word;
//! - glGenerateMipmap on such a texture is the RGBA8 class box filter: every
//!   stored byte averaged on its own, (sum + 2) / 4 (`mip_box_row_u8`), not the
//!   class build's field-by-field average of the word (`box_average_rgb10a2`).
//!   The two rules differ at the alpha of a box holding 0, 1, 2 and 3 (byte
//!   average 128, field average 2 of 3, decoded 170), at the alpha of a box of
//!   0, 0, 0 and 3 (64 vs 85) and at the red of a box of 875, 249, 207 and 697
//!   (127 vs 126): the wanted bytes below pin the byte rule at each of them;
//! - a GL_NEAREST glBlitFramebuffer out of such an attachment into an RGBA8
//!   renderbuffer carries those bytes over, over the destination's own clear,
//!   and no GLES_SURF_RGB10_A2 target goes out on the way;
//! - an ES 2.0-reporting build names no GL_RGB10_A2 at all: `sized_format` has
//!   no row for it (`es3_formats::find` is the ES 3.0 table, and None under
//!   `report-es2`), so both entry points refuse the internal format with
//!   GL_INVALID_ENUM and lay out nothing: no immutable store of the name exists
//!   there for a rule to reach (as tests/es3_rgb10a2_fallback.rs holds for
//!   glTexImage2D).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online,test-knobs --test es3_rgb10a2_texstorage_fallback
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online,test-knobs --test es3_rgb10a2_texstorage_fallback
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table,report-es2 --test es3_rgb10a2_texstorage_fallback
//! ```
//!
//! Compiled only where the class falls back: the V3D build without
//! `report-es2` stores the class and runs zero tests of this file (its answers
//! are tests/es3_rgb10a2_texture_v3d.rs and tests/es3_rgb10a2_queries_v3d.rs).
//! No GPU runs: the stand-in back end draws nothing but the clears the tests
//! make it store.

#![cfg(all(feature = "host-test", any(feature = "gpu-powervr", feature = "gpu-vivante", feature = "report-es2")))]
// the ES 2.0 report runs the refusal pins only; the helpers of the rest are idle there
#![cfg_attr(feature = "report-es2", allow(dead_code, unused_imports))]

use core::ffi::{c_char, c_void};
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{self, GlesDraw, GlesHeap, YakoglBackend};
use yakogl::gl::consts::*;
use yakogl::gl::es3_formats;
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
    name: c"es3-rgb10a2-texstorage-fallback".as_ptr() as *const c_char,
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
    max_samples: 0,
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

fn status() -> GLenum {
    // SAFETY: plain GL call.
    unsafe { glCheckFramebufferStatus(GL_FRAMEBUFFER) }
}

/// A fresh texture name, bound to GL_TEXTURE_2D, no level defined.
fn empty_texture() -> GLuint {
    let mut t = 0;
    // SAFETY: room for one name.
    unsafe {
        glGenTextures(1, &mut t);
        glBindTexture(GL_TEXTURE_2D, t);
    }
    t
}

/* ---- the immutable path: glTexStorage2D / glTexStorage3D ------------------- */

/// A new texture of `target`, its whole chain laid out at once by
/// glTexStorage2D as GL_RGB10_A2, and left bound to `target`.
fn storage_2d(target: GLenum, levels: GLsizei, w: GLsizei, h: GLsizei) -> GLuint {
    let mut tex = 0;
    // SAFETY: room for one name; plain GL calls.
    unsafe {
        glGenTextures(1, &mut tex);
        glBindTexture(target, tex);
        glTexStorage2D(target, levels, GL_RGB10_A2, w, h);
    }
    assert_eq!(err(), GL_NO_ERROR, "glTexStorage2D({target:#x}, {levels} levels of {w} x {h})");
    tex
}

/// The same through glTexStorage3D, `d` layers of a GL_TEXTURE_2D_ARRAY or the
/// depth of a GL_TEXTURE_3D texture.
fn storage_3d(target: GLenum, levels: GLsizei, w: GLsizei, h: GLsizei, d: GLsizei) -> GLuint {
    let mut tex = 0;
    // SAFETY: room for one name; plain GL calls.
    unsafe {
        glGenTextures(1, &mut tex);
        glBindTexture(target, tex);
        glTexStorage3D(target, levels, GL_RGB10_A2, w, h, d);
    }
    assert_eq!(err(), GL_NO_ERROR, "glTexStorage3D({target:#x}, {levels} levels of {w} x {h} x {d})");
    tex
}

/// GL_TEXTURE_IMMUTABLE_FORMAT / _LEVELS of the texture bound to `target`.
fn immutable_params(target: GLenum) -> (GLint, GLint) {
    let (mut f, mut l) = (-1, -1);
    // SAFETY: room for one value each; plain GL calls.
    unsafe {
        glGetTexParameteriv(target, GL_TEXTURE_IMMUTABLE_FORMAT, &mut f);
        glGetTexParameteriv(target, GL_TEXTURE_IMMUTABLE_LEVELS, &mut l);
    }
    assert_eq!(err(), GL_NO_ERROR, "the immutable pnames of {target:#x}");
    (f, l)
}

/// Upload `words` (`w` x `h` of them) into level `level` of the texture bound
/// to `target`, with the pair table 3.2 gives GL_RGB10_A2 and glTexStorage*
/// records for it.
fn upload_words(target: GLenum, level: GLint, w: GLsizei, h: GLsizei, words: &[u32]) {
    // SAFETY: the texture is bound to `target` with a `level` of w x h; `words`
    // holds w x h texels.
    unsafe {
        glTexSubImage2D(
            target,
            level,
            0,
            0,
            w,
            h,
            GL_RGBA,
            GL_UNSIGNED_INT_2_10_10_10_REV,
            words.as_ptr() as *const c_void,
        )
    }
    assert_eq!(err(), GL_NO_ERROR, "glTexSubImage2D({target:#x} level {level}) with GL_UNSIGNED_INT_2_10_10_10_REV");
}

/// A new framebuffer, bound to GL_FRAMEBUFFER, with nothing attached yet.
fn new_framebuffer() -> GLuint {
    let mut fb = 0;
    // SAFETY: room for one name; plain GL calls.
    unsafe {
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
    }
    fb
}

/// Attach a site of `tex` to colour attachment 0 of the bound framebuffer:
/// level `level`, and layer `layer` (negative for a 2D image or a cube face,
/// which then comes from `target`, a texture target or a cube face).
fn attach(tex: GLuint, target: GLenum, level: GLint, layer: GLint) {
    // SAFETY: plain GL calls on a texture and framebuffer of the calling test.
    unsafe {
        if layer >= 0 {
            glFramebufferTextureLayer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, tex, level, layer);
        } else {
            glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, target, tex, level);
        }
    }
    assert_eq!(err(), GL_NO_ERROR, "attach {target:#x} level {level} layer {layer}");
}

const ATT_SIZES: [GLenum; 4] = [
    GL_FRAMEBUFFER_ATTACHMENT_RED_SIZE,
    GL_FRAMEBUFFER_ATTACHMENT_GREEN_SIZE,
    GL_FRAMEBUFFER_ATTACHMENT_BLUE_SIZE,
    GL_FRAMEBUFFER_ATTACHMENT_ALPHA_SIZE,
];

fn attachment(pname: GLenum) -> GLint {
    let mut v = -1;
    // SAFETY: room for one integer.
    unsafe { glGetFramebufferAttachmentParameteriv(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, pname, &mut v) };
    assert_eq!(err(), GL_NO_ERROR, "attachment pname {pname:#x}");
    v
}

/// GL_IMPLEMENTATION_COLOR_READ_FORMAT and _TYPE of the bound read buffer.
fn implementation_pair() -> (GLenum, GLenum) {
    let (mut f, mut t) = (0, 0);
    // SAFETY: room for one value each.
    unsafe {
        glGetIntegerv(GL_IMPLEMENTATION_COLOR_READ_FORMAT, &mut f);
        glGetIntegerv(GL_IMPLEMENTATION_COLOR_READ_TYPE, &mut t);
    }
    (f as GLenum, t as GLenum)
}

/// What the bound framebuffer's colour attachment 0 must answer for a
/// GL_RGB10_A2 of the fallback: complete, and the RGBA8 class the name fell
/// back to -- the attachment sizes read the stored texel format, not the
/// internal format's name (framebuffer.rs, the GL_FRAMEBUFFER_ATTACHMENT_*
/// arms), which is why they are eight and not 10/10/10/2.
fn assert_rgba8_attachment(site: &str) {
    assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE, "{site}");
    for p in ATT_SIZES {
        assert_eq!(attachment(p), 8, "{site}: {p:#x}");
    }
    let (ct, enc) =
        (attachment(GL_FRAMEBUFFER_ATTACHMENT_COMPONENT_TYPE), attachment(GL_FRAMEBUFFER_ATTACHMENT_COLOR_ENCODING));
    assert_eq!(ct, GL_UNSIGNED_NORMALIZED as GLint, "{site}: COMPONENT_TYPE");
    assert_eq!(enc, GL_LINEAR as GLint, "{site}: COLOR_ENCODING");
    assert_eq!(implementation_pair(), (GL_RGBA, GL_UNSIGNED_BYTE), "{site}: the implementation read pair");
}

/// A new RGBA8 renderbuffer on colour attachment 0 of a new framebuffer, which
/// stays bound to GL_FRAMEBUFFER and is complete.
fn rgba8_framebuffer(w: GLsizei, h: GLsizei) -> (GLuint, GLuint) {
    let (mut rb, mut fb) = (0, 0);
    // SAFETY: room for one name each; plain GL calls.
    unsafe {
        glGenRenderbuffers(1, &mut rb);
        glBindRenderbuffer(GL_RENDERBUFFER, rb);
        glRenderbufferStorage(GL_RENDERBUFFER, GL_RGBA8, w, h);
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_RENDERBUFFER, rb);
    }
    assert_eq!(err(), GL_NO_ERROR, "glRenderbufferStorage(GL_RGBA8)");
    assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE, "the RGBA8 destination");
    (rb, fb)
}

/// Attach level `level` of `tex` to a new framebuffer, which stays bound, and
/// read its w x h pixels as GL_RGBA / GL_UNSIGNED_BYTE.
fn read_level(tex: GLuint, level: GLint, w: GLsizei, h: GLsizei) -> (GLuint, Vec<u8>) {
    let fb = new_framebuffer();
    attach(tex, GL_TEXTURE_2D, level, -1);
    assert_rgba8_attachment(&format!("level {level}"));
    let px = read(w, h);
    assert_eq!(err(), GL_NO_ERROR, "glReadPixels of level {level}");
    (fb, px)
}

/// The bound framebuffer's w x h pixels as GL_RGBA / GL_UNSIGNED_BYTE, four
/// bytes a pixel, the buffer prefilled with 0x77 so that a read that writes
/// nothing shows.
fn read(w: GLsizei, h: GLsizei) -> Vec<u8> {
    let mut px = vec![0x77u8; (w * h * 4) as usize];
    // SAFETY: room for w x h pixels of four bytes.
    unsafe { glReadPixels(0, 0, w, h, GL_RGBA, GL_UNSIGNED_BYTE, px.as_mut_ptr() as *mut c_void) };
    px
}

/// A 10/10/10/2 word (R in bits 0..9, G 10..19, B 20..29, A 30..31).
const fn word(r: u32, g: u32, b: u32, a: u32) -> u32 {
    r | g << 10 | b << 20 | a << 30
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

/* ---- the ES 2.0 report: GL_RGB10_A2 is no format ---------------------------- */

#[cfg(feature = "report-es2")]
mod es2 {
    use super::*;

    /// ES 2.0 names no GL_RGB10_A2 (ES 2.0 3.7.1 and 3.8.4 name no sized
    /// colour format at all), and this build's `sized_format` has no row for
    /// the name because `es3_formats::find` is the ES 3.0 table and returns
    /// None under `report-es2`: both storage entry points answer
    /// GL_INVALID_ENUM and return before a level is laid out, so no immutable
    /// texture of the name exists for the rules above to be reached. One bound
    /// texture is enough: the refusal comes from the internal format, before
    /// the target binding is read -- and an ES 2.0 build has no 3D texture or
    /// texture array to bind in the first place
    /// (tests/es3_rgb10a2_layers_fallback.rs, tests/es2_report_guard.rs).
    #[test]
    fn texstorage_fallback_es2_entry_points_refuse_the_format() {
        let _g = context();
        let tex = empty_texture();
        // SAFETY: plain GL calls. Every value here (one level, 4 x 4, a depth
        // of 4, a target each entry point knows) is one it takes: only the
        // internal format is refused.
        unsafe { glTexStorage2D(GL_TEXTURE_2D, 1, GL_RGB10_A2, 4, 4) };
        assert_eq!(err(), GL_INVALID_ENUM, "glTexStorage2D(GL_TEXTURE_2D, GL_RGB10_A2)");
        // SAFETY: as above.
        unsafe { glTexStorage2D(GL_TEXTURE_CUBE_MAP, 1, GL_RGB10_A2, 4, 4) };
        assert_eq!(err(), GL_INVALID_ENUM, "glTexStorage2D(GL_TEXTURE_CUBE_MAP, GL_RGB10_A2)");
        // SAFETY: as above.
        unsafe { glTexStorage3D(GL_TEXTURE_2D_ARRAY, 1, GL_RGB10_A2, 4, 4, 4) };
        assert_eq!(err(), GL_INVALID_ENUM, "glTexStorage3D(GL_TEXTURE_2D_ARRAY, GL_RGB10_A2)");
        // SAFETY: as above.
        unsafe { glTexStorage3D(GL_TEXTURE_3D, 1, GL_RGB10_A2, 4, 4, 4) };
        assert_eq!(err(), GL_INVALID_ENUM, "glTexStorage3D(GL_TEXTURE_3D, GL_RGB10_A2)");
        // Nothing was laid out by the refusals: the texture still has no image
        // and no immutable format, so a framebuffer that attaches level 0 of it
        // is incomplete (tests/es3_rgb10a2_queries_fallback.rs holds the same
        // for the objects glTexImage2D refuses).
        let mut fb = 0;
        // SAFETY: room for one name; plain GL calls.
        unsafe {
            glGenFramebuffers(1, &mut fb);
            glBindFramebuffer(GL_FRAMEBUFFER, fb);
            glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex, 0);
            assert_eq!(
                glCheckFramebufferStatus(GL_FRAMEBUFFER),
                GL_FRAMEBUFFER_INCOMPLETE_ATTACHMENT,
                "no store behind the name"
            );
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
            glDeleteFramebuffers(1, &fb);
        }
        release(&[], &[], &[tex]);
    }
}

/* ---- ES 3.0 on a build without the class ------------------------------------ */

#[cfg(not(feature = "report-es2"))]
mod es3 {
    use super::*;

    /// The premise: this build's back end neither draws the 10/10/10/2 surface
    /// nor samples its texture class, so the storage entry points resolve
    /// GL_RGB10_A2 to the RGBA8 store (framebuffer.rs `rgb10a2_surface`, the
    /// `Es3Format` storage list and `storage_here`).
    #[test]
    fn texstorage_fallback_build_has_no_class() {
        let _g = context();
        assert!(!backend::rt_format_drawable(backend::GLES_SURF_RGB10_A2), "no bit, no surface");
        let (_, e) = es3_formats::find(GL_RGB10_A2).expect("GL_RGB10_A2 is an ES 3.0 name here");
        assert_eq!(e.storage_here(), Some(backend::GLES_TEXFMT_RGBA8), "the storage class glTexStorage* lays");
    }

    /// ES 3.0 3.8.4: glTexStorage* makes the texture immutable and records the
    /// levels it was given, through either entry point and on every target it
    /// takes. A texture it never touched reads 0 / 0.
    #[test]
    fn texstorage_fallback_immutable_format_and_levels() {
        let _g = context();
        let plain = empty_texture();
        assert_eq!(immutable_params(GL_TEXTURE_2D), (0, 0), "no storage call made it immutable");
        let (tex2d, cube) = (storage_2d(GL_TEXTURE_2D, 4, 8, 8), storage_2d(GL_TEXTURE_CUBE_MAP, 2, 4, 4));
        assert_eq!(immutable_params(GL_TEXTURE_2D), (1, 4), "glTexStorage2D(GL_TEXTURE_2D, 4, GL_RGB10_A2, 8, 8)");
        assert_eq!(immutable_params(GL_TEXTURE_CUBE_MAP), (1, 2), "the same with GL_TEXTURE_CUBE_MAP");
        let (arr, vol) = (storage_3d(GL_TEXTURE_2D_ARRAY, 3, 8, 8, 2), storage_3d(GL_TEXTURE_3D, 3, 4, 4, 4));
        assert_eq!(immutable_params(GL_TEXTURE_2D_ARRAY), (1, 3), "glTexStorage3D(GL_TEXTURE_2D_ARRAY, 3, ..)");
        assert_eq!(immutable_params(GL_TEXTURE_3D), (1, 3), "glTexStorage3D(GL_TEXTURE_3D, 3, ..)");
        release(&[], &[], &[plain, tex2d, cube, arr, vol]);
    }

    /// Every level of the chain is a data store of its own already, before a
    /// texel is uploaded (the difference from glTexImage2D, whose levels exist
    /// only after their upload): each attaches complete and answers the RGBA8
    /// class -- eight bits a channel, GL_UNSIGNED_NORMALIZED, GL_LINEAR, and
    /// the read pair an RGBA8 buffer's.
    #[test]
    fn texstorage_fallback_levels_are_complete_and_answer_eight() {
        let _g = context();
        let tex = storage_2d(GL_TEXTURE_2D, 4, 8, 8);
        let fb = new_framebuffer();
        for level in 0..4 {
            attach(tex, GL_TEXTURE_2D, level, -1);
            assert_rgba8_attachment(&format!("level {level} of the 8 x 8 chain"));
        }
        release(&[fb], &[], &[tex]);
    }

    /// glTexStorage2D of a cube map lays out all six faces at once (3.8.4),
    /// where glTexImage2D needs one call a face and may leave the set
    /// incomplete: every face, at level 0 and at level 1, is complete and
    /// answers the RGBA8 class.
    #[test]
    fn texstorage_fallback_cube_faces_answer_eight() {
        let _g = context();
        let cube = storage_2d(GL_TEXTURE_CUBE_MAP, 2, 4, 4);
        let fb = new_framebuffer();
        for face in 0..6u32 {
            attach(cube, GL_TEXTURE_CUBE_MAP_POSITIVE_X + face, 0, -1);
            assert_rgba8_attachment(&format!("face {} at level 0", FACES[face as usize]));
        }
        for level in 0..2 {
            attach(cube, GL_TEXTURE_CUBE_MAP_POSITIVE_X, level, -1);
            assert_rgba8_attachment(&format!("face +X at level {level}"));
        }
        release(&[fb], &[], &[cube]);
    }

    const FACES: [&str; 6] = ["+X", "-X", "+Y", "-Y", "+Z", "-Z"];

    /// glTexStorage3D of a GL_TEXTURE_2D_ARRAY lays out every layer of every
    /// level at once: each (level, layer) attaches complete and answers the
    /// RGBA8 class (layers_fallback pins the same for glTexImage3D uploads).
    #[test]
    fn texstorage_fallback_array_layers_and_levels_answer_eight() {
        let _g = context();
        let arr = storage_3d(GL_TEXTURE_2D_ARRAY, 2, 8, 8, 3);
        let fb = new_framebuffer();
        for level in 0..2 {
            for layer in 0..3 {
                attach(arr, GL_TEXTURE_2D_ARRAY, level, layer);
                assert_rgba8_attachment(&format!("layer {layer} at level {level}"));
            }
        }
        release(&[fb], &[], &[arr]);
    }

    /// A 4 x 4 texture's fields, row 0 first: the edge values (0, 1, 2, 3, 341,
    /// 511, 512, 1022, 1023, every alpha) and the boxes whose byte average
    /// differs from the class build's average of the word -- x 2..3, y 0..1 at
    /// its red, and every box at its alpha. The same grid
    /// tests/es3_rgb10a2_texclear_fallback.rs uploads with glTexImage2D, so the
    /// wanted bytes below are its wanted bytes.
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
    /// (es3_formats' decode to floats and the `GLES_TEXFMT_RGBA8` store).
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
    /// The class build's word rules would answer alpha 170, 85, 170, 170 and a
    /// red of 126 in its second box; the byte rule answers 128, 64, 128, 128
    /// and 127.
    const WANTED_L1: [[u8; 4]; 4] = [[117, 117, 149, 128], [127, 123, 98, 64], [64, 64, 64, 128], [117, 138, 160, 128]];

    /// Level 2 (1 x 1), the same byte rule over level 1's four texels:
    /// (117 + 127 + 64 + 117 + 2) / 4 = 106, (117 + 123 + 64 + 138 + 2) / 4 =
    /// 111, (149 + 98 + 64 + 160 + 2) / 4 = 118, (128 + 64 + 128 + 128 + 2) / 4
    /// = 112.
    const WANTED_L2: [u8; 4] = [106, 111, 118, 112];

    /// Concatenate the wanted bytes of a pixel list.
    fn want_bytes(pixels: &[[u8; 4]]) -> Vec<u8> {
        pixels.iter().flat_map(|p| p.iter().copied()).collect()
    }

    /// The clear of the blit row and the RGBA8 bytes it is stored as:
    /// 0.2 * 255 = 51, 0.6 * 255 = 153, 1.5 clamps to 255, alpha 0.4 is 102.
    const CLEAR: [f32; 4] = [0.2, 0.6, 1.5, 0.4];
    const CLEAR_BYTES: [u8; 4] = [51, 153, 255, 102];

    /// Clear the bound framebuffer to [`CLEAR`].
    fn clear() {
        // SAFETY: plain GL calls.
        unsafe {
            glClearColor(CLEAR[0], CLEAR[1], CLEAR[2], CLEAR[3]);
            glClear(GL_COLOR_BUFFER_BIT);
        }
        assert_eq!(err(), GL_NO_ERROR, "the clear");
    }

    /// glTexStorage2D of GL_RGB10_A2 takes table 3.2's pair, and an upload of
    /// it into the immutable store is the generic ES 3.0 path: no error, and
    /// the texels are the RGBA8 bytes of the words, read back unchanged (no
    /// 10/10/10/2 word, no decode of one).
    #[test]
    fn texstorage_fallback_upload_stores_rgba8_bytes() {
        let _g = context();
        let tex = storage_2d(GL_TEXTURE_2D, 3, 4, 4);
        let words = grid_words();
        upload_words(GL_TEXTURE_2D, 0, 4, 4, &words);
        let (fb, px) = read_level(tex, 0, 4, 4);
        assert_eq!(px, want_bytes(&WANTED_L0), "the stored RGBA8 bytes of the words");
        release(&[fb], &[], &[tex]);
    }

    /// glGenerateMipmap on such a texture is the RGBA8 class box filter, on a
    /// chain glTexStorage2D laid out already: level 0 untouched, level 1 the
    /// byte average of each 2 x 2 box, level 2 the same over level 1 -- not the
    /// class build's average of the 10/10/10/2 words.
    #[test]
    fn texstorage_fallback_generate_mipmap_averages_bytes() {
        let _g = context();
        let tex = storage_2d(GL_TEXTURE_2D, 3, 4, 4);
        let words = grid_words();
        upload_words(GL_TEXTURE_2D, 0, 4, 4, &words);
        // SAFETY: plain GL call on the texture storage_2d bound.
        unsafe { glGenerateMipmap(GL_TEXTURE_2D) };
        assert_eq!(err(), GL_NO_ERROR, "glGenerateMipmap");
        let (fb0, px) = read_level(tex, 0, 4, 4);
        assert_eq!(px, want_bytes(&WANTED_L0), "level 0 untouched");
        let (fb1, px) = read_level(tex, 1, 2, 2);
        assert_eq!(px, want_bytes(&WANTED_L1), "the byte box average of each 2 x 2 box");
        let (fb2, px) = read_level(tex, 2, 1, 1);
        assert_eq!(px, want_bytes(&[WANTED_L2]), "the byte box average over level 1");
        release(&[fb0, fb1, fb2], &[], &[tex]);
    }

    /// A GL_NEAREST blit out of a GL_RGB10_A2 attachment of the fallback into
    /// an RGBA8 renderbuffer: the destination first holds its own clear (the
    /// RGBA8 bytes an RGBA8888 target stores), and the blit carries the
    /// source's RGBA8 bytes over them. No 10/10/10/2 target goes out on the
    /// way (layers_fallback blits the same from a glTexImage3D array).
    #[test]
    fn texstorage_fallback_nearest_blit_carries_the_rgba8_bytes() {
        let _g = context();
        let words = grid_words();
        let tex = storage_2d(GL_TEXTURE_2D, 1, 4, 4);
        upload_words(GL_TEXTURE_2D, 0, 4, 4, &words);
        let src_fb = new_framebuffer();
        attach(tex, GL_TEXTURE_2D, 0, -1);
        assert_rgba8_attachment("the source attachment");
        let (rb, dst_fb) = rgba8_framebuffer(4, 4);
        clear();
        assert_eq!(read(4, 4), want_bytes(&[CLEAR_BYTES; 16]), "the destination holds its own clear");
        // SAFETY: plain GL calls on the framebuffers and the renderbuffer this
        // test made, all of them complete.
        unsafe {
            glBindFramebuffer(GL_READ_FRAMEBUFFER, src_fb);
            glReadBuffer(GL_COLOR_ATTACHMENT0);
            glBindFramebuffer(GL_DRAW_FRAMEBUFFER, dst_fb);
            let bufs = [GL_COLOR_ATTACHMENT0];
            glDrawBuffers(1, bufs.as_ptr());
            glBlitFramebuffer(0, 0, 4, 4, 0, 0, 4, 4, GL_COLOR_BUFFER_BIT, GL_NEAREST);
        }
        assert_eq!(err(), GL_NO_ERROR, "glBlitFramebuffer(GL_NEAREST)");
        let draws = DRAWS.lock().unwrap().clone();
        assert_eq!(draws.iter().filter(|d| d.1).count(), 1, "the clear went out as an RGBA8888 target: {draws:?}");
        assert!(draws.iter().all(|d| d.0 != backend::GLES_SURF_RGB10_A2), "no 10/10/10/2 target: {draws:?}");
        // SAFETY: plain GL call on the destination this test made.
        unsafe { glBindFramebuffer(GL_FRAMEBUFFER, dst_fb) };
        let px = read(4, 4);
        assert_eq!(err(), GL_NO_ERROR, "glReadPixels of the blit");
        assert_eq!(px, want_bytes(&WANTED_L0), "the blit carried the source's RGBA8 bytes over its clear");
        release(&[src_fb, dst_fb], &[rb], &[tex]);
    }
}
