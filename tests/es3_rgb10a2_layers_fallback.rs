// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GL_RGB10_A2 cube-face, 2D-array-layer, 3D-slice and non-zero-level
//! attachments where the build has no class for the format (plan #370 item
//! layers-fallback-pin; the class build's rows are
//! tests/es3_rgb10a2_layers_v3d.rs, see 391ab633).
//!
//! On the V3D 4.2 build GL_RGB10_A2 is its own texture class and render
//! surface (`GLES_TEXFMT_RGB10A2` / `GLES_SURF_RGB10_A2`, docs/notes/v3d-rgb10a2.md).
//! Everywhere else a GL_RGB10_A2 texture is in the RGBA8 class (es3_formats'
//! fallback): the PowerVR and GC7000 builds and every `report-es2` build.
//! This file holds what the layered and levelled attachments answer there
//! today (sibling of tests/es3_rgb10a2_texclear_fallback.rs and
//! tests/es3_rgb10a2_packmsaa_fallback.rs, same stand-in OSAL and back end,
//! no GPU). For a cube face other than +X, a 2D-array layer > 0, a 3D slice
//! > 0 and level 1 of a 2D texture:
//!
//! - an RGB10_A2 upload (GL_RGBA / GL_UNSIGNED_INT_2_10_10_10_REV) stores the
//!   RGBA8 bytes of each word, byte = trunc(c / 1023 * 255 + 0.5), alpha
//!   trunc(a / 3 * 255 + 0.5) (as texclear_fallback pins);
//! - glFramebufferTexture2D / glFramebufferTextureLayer of the site is
//!   complete and glReadPixels GL_RGBA / GL_UNSIGNED_BYTE reads those bytes,
//!   each site its own, the siblings never mixed up;
//! - GL_IMPLEMENTATION_COLOR_READ_FORMAT / _TYPE answer for the attachment as
//!   they do for an RGBA8 buffer on the build (GL_RGBA / GL_UNSIGNED_BYTE);
//! - one glBlitFramebuffer from the layer attachment into an RGBA8
//!   renderbuffer copies the bytes.
//!
//! An ES 2.0-reporting build cannot allocate GL_RGB10_A2 at all (and has no
//! 3D texture or texture array): the uploads are refused and the rows assert
//! the error the build gives today, so no layered or levelled attachment of
//! the format exists there.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online,test-knobs --test es3_rgb10a2_layers_fallback
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online,test-knobs --test es3_rgb10a2_layers_fallback
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table,report-es2 --test es3_rgb10a2_layers_fallback
//! ```
//!
//! Compiled only where the class falls back: the V3D build without
//! `report-es2` stores the class and runs zero tests of this file.

#![cfg(all(feature = "host-test", any(feature = "gpu-powervr", feature = "gpu-vivante", feature = "report-es2")))]
// the ES 2.0 report runs the refusal pins only; the helpers of the rest are idle there
#![cfg_attr(feature = "report-es2", allow(dead_code, unused_imports))]

use core::ffi::{c_char, c_void};
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{self, GlesDraw, GlesHeap, YakoglBackend};
use yakogl::gl::consts::*;
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
/// No draw reaches the stand-in GPU in this file (no clear, no draw call).
unsafe extern "C" fn b_draw(_d: *const GlesDraw) -> i32 {
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
    name: c"es3-rgb10a2-layers-fallback".as_ptr() as *const c_char,
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
    // SAFETY: plain GL call.
    while unsafe { glGetError() } != GL_NO_ERROR {}
    g
}

fn err() -> GLenum {
    // SAFETY: plain GL call.
    unsafe { glGetError() }
}

/// A 10/10/10/2 word (R in bits 0..9, G 10..19, B 20..29, A 30..31).
const fn word(r: u32, g: u32, b: u32, a: u32) -> u32 {
    r | g << 10 | b << 20 | a << 30
}

const N: GLsizei = 4;

#[derive(Clone, Copy, Debug)]
enum Kind {
    Cube,
    Array,
    Volume,
    Level,
}

/// Sites (faces, layers, slices or levels) of a texture of `kind`.
fn sites(kind: Kind) -> u32 {
    match kind {
        Kind::Cube => 6,
        Kind::Array | Kind::Volume => 3,
        Kind::Level => 2,
    }
}

/// Edge of site `i`.
fn side(kind: Kind, i: u32) -> GLsizei {
    match kind {
        Kind::Level => N >> i,
        _ => N,
    }
}

fn target(kind: Kind) -> GLenum {
    match kind {
        Kind::Cube => GL_TEXTURE_CUBE_MAP,
        Kind::Array => GL_TEXTURE_2D_ARRAY,
        Kind::Volume => GL_TEXTURE_3D,
        Kind::Level => GL_TEXTURE_2D,
    }
}

/// The word every texel of site `i` starts with; one per site, with a
/// 10-bit field that no byte rounding confuses with a neighbour's.
fn sentinel(i: u32) -> u32 {
    word(1000 - i, 5 + i, 600, 1 + i % 3)
}

/// A field of `max` as the RGBA8 byte the fallback stores:
/// trunc(c / max * 255 + 0.5).
fn byte(c: u32, max: u32) -> u8 {
    (c as f32 / max as f32 * 255.0 + 0.5) as u8
}

/// The RGBA8 bytes the fallback stores for a word.
fn stored(w: u32) -> [u8; 4] {
    [byte(w & 0x3ff, 1023), byte(w >> 10 & 0x3ff, 1023), byte(w >> 20 & 0x3ff, 1023), byte(w >> 30, 3)]
}

/// Upload every site of a `kind` texture as RGB10_A2 words (the pair the
/// fallback takes), nearest filtering; returns the texture, the target bound.
fn make_texture(kind: Kind) -> GLuint {
    let mut tex = 0;
    let t = target(kind);
    // SAFETY: room for one name; every upload holds side x side (x depth) words.
    unsafe {
        glGenTextures(1, &mut tex);
        glBindTexture(t, tex);
        let internal = GL_RGB10_A2 as GLint;
        let ty = GL_UNSIGNED_INT_2_10_10_10_REV;
        match kind {
            Kind::Cube | Kind::Level => {
                for i in 0..sites(kind) {
                    let s = side(kind, i);
                    let px = vec![sentinel(i); (s * s) as usize];
                    let (tgt, lvl) = match kind {
                        Kind::Cube => (GL_TEXTURE_CUBE_MAP_POSITIVE_X + i, 0),
                        _ => (GL_TEXTURE_2D, i as GLint),
                    };
                    glTexImage2D(tgt, lvl, internal, s, s, 0, GL_RGBA, ty, px.as_ptr() as *const c_void);
                }
            }
            Kind::Array | Kind::Volume => {
                let mut px = Vec::new();
                for i in 0..sites(kind) {
                    px.extend(vec![sentinel(i); (N * N) as usize]);
                }
                glTexImage3D(t, 0, internal, N, N, 3, 0, GL_RGBA, ty, px.as_ptr() as *const c_void);
            }
        }
        glTexParameteri(t, GL_TEXTURE_MIN_FILTER, GL_NEAREST as GLint);
    }
    assert_eq!(err(), GL_NO_ERROR, "upload {kind:?}");
    tex
}

/// Attach site `i` of `tex` to colour attachment 0 of the bound framebuffer.
fn attach_site(kind: Kind, tex: GLuint, i: u32) {
    // SAFETY: plain GL calls.
    unsafe {
        match kind {
            Kind::Cube => {
                glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_CUBE_MAP_POSITIVE_X + i, tex, 0)
            }
            Kind::Array | Kind::Volume => {
                glFramebufferTextureLayer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, tex, 0, i as GLint)
            }
            Kind::Level => glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex, i as GLint),
        }
    }
    assert_eq!(err(), GL_NO_ERROR, "attach {kind:?} site {i}");
}

/// The bound read framebuffer's w x w pixels as GL_RGBA / GL_UNSIGNED_BYTE.
fn read_bound(w: GLsizei) -> Vec<[u8; 4]> {
    let mut px = vec![0x77u8; (w * w * 4) as usize];
    // SAFETY: `px` holds w x w RGBA8 pixels.
    unsafe { glReadPixels(0, 0, w, w, GL_RGBA, GL_UNSIGNED_BYTE, px.as_mut_ptr() as *mut c_void) };
    assert_eq!(err(), GL_NO_ERROR, "glReadPixels");
    px.chunks(4).map(|c| [c[0], c[1], c[2], c[3]]).collect()
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

/* ---- the ES 2.0 report: GL_RGB10_A2 is no format ---------------------------- */

#[cfg(feature = "report-es2")]
mod es2 {
    use super::*;

    /// No layered or levelled RGB10_A2 attachment exists on an ES 2.0
    /// report: every upload of the format is refused (ES 2.0 names no
    /// GL_RGB10_A2 and the 2_10_10_10_REV type is ES 3.0's), and the 3D and
    /// array targets are not texture targets at all. The errors asserted are
    /// what the build gives today.
    #[test]
    fn layers_fallback_es2_uploads_are_refused() {
        let _g = context();
        let mut texs = [0; 4];
        // SAFETY: room for four names; the null pixels allocate only.
        unsafe {
            glGenTextures(4, texs.as_mut_ptr());
            glBindTexture(GL_TEXTURE_CUBE_MAP, texs[0]);
            glTexImage2D(
                GL_TEXTURE_CUBE_MAP_NEGATIVE_Y,
                0,
                GL_RGB10_A2 as GLint,
                N,
                N,
                0,
                GL_RGBA,
                GL_UNSIGNED_INT_2_10_10_10_REV,
                core::ptr::null(),
            );
        }
        assert_eq!(err(), GL_INVALID_ENUM, "cube face, 2_10_10_10_REV");
        // SAFETY: as above.
        unsafe {
            glBindTexture(GL_TEXTURE_2D, texs[1]);
            glTexImage2D(
                GL_TEXTURE_2D,
                1,
                GL_RGB10_A2 as GLint,
                N / 2,
                N / 2,
                0,
                GL_RGBA,
                GL_UNSIGNED_INT_2_10_10_10_REV,
                core::ptr::null(),
            );
        }
        assert_eq!(err(), GL_INVALID_ENUM, "level 1, 2_10_10_10_REV");
        // SAFETY: as above.
        unsafe {
            glBindTexture(GL_TEXTURE_2D_ARRAY, texs[2]);
        }
        assert_eq!(err(), GL_INVALID_ENUM, "bind GL_TEXTURE_2D_ARRAY");
        // SAFETY: as above.
        unsafe {
            glBindTexture(GL_TEXTURE_3D, texs[3]);
        }
        assert_eq!(err(), GL_INVALID_ENUM, "bind GL_TEXTURE_3D");
        // SAFETY: names this test made.
        unsafe { glDeleteTextures(4, texs.as_ptr()) };
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
    fn layers_fallback_build_has_no_class() {
        assert!(!backend::rt_format_drawable(backend::GLES_SURF_RGB10_A2));
    }

    /// Every site of `kind` attaches complete, reads back the RGBA8 bytes of
    /// its own word and answers the implementation read pair of an RGBA8
    /// buffer; then one blit of the last site into an RGBA8 renderbuffer
    /// copies those bytes.
    fn run(kind: Kind) {
        let _g = context();
        let tex = make_texture(kind);
        let (mut fb, mut rb, mut fb8) = (0, 0, 0);
        // SAFETY: room for one name each.
        unsafe {
            glGenFramebuffers(1, &mut fb);
            glGenRenderbuffers(1, &mut rb);
            glGenFramebuffers(1, &mut fb8);
            glBindFramebuffer(GL_FRAMEBUFFER, fb);
        }
        for i in 0..sites(kind) {
            attach_site(kind, tex, i);
            // SAFETY: plain GL call.
            assert_eq!(
                unsafe { glCheckFramebufferStatus(GL_FRAMEBUFFER) },
                GL_FRAMEBUFFER_COMPLETE,
                "{kind:?} site {i}"
            );
            let got = read_bound(side(kind, i));
            let want = stored(sentinel(i));
            assert!(got.iter().all(|p| *p == want), "{kind:?} site {i}: {:?} want {want:?}", got[0]);
            assert_eq!(
                implementation_pair(),
                (GL_RGBA, GL_UNSIGNED_BYTE),
                "{kind:?} site {i}: the implementation read pair is an RGBA8 buffer's"
            );
        }
        // Blit the last site into an RGBA8 renderbuffer.
        let pick = sites(kind) - 1;
        attach_site(kind, tex, pick);
        let s = side(kind, pick);
        // SAFETY: plain GL calls on names made above.
        unsafe {
            glBindRenderbuffer(GL_RENDERBUFFER, rb);
            glRenderbufferStorage(GL_RENDERBUFFER, GL_RGBA8, s, s);
            glBindFramebuffer(GL_FRAMEBUFFER, fb8);
            glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_RENDERBUFFER, rb);
            assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE, "RGBA8 destination");
            glBindFramebuffer(GL_READ_FRAMEBUFFER, fb);
            glReadBuffer(GL_COLOR_ATTACHMENT0);
            glBindFramebuffer(GL_DRAW_FRAMEBUFFER, fb8);
            let draw = [GL_COLOR_ATTACHMENT0];
            glDrawBuffers(1, draw.as_ptr());
            glBlitFramebuffer(0, 0, s, s, 0, 0, s, s, GL_COLOR_BUFFER_BIT, GL_NEAREST);
        }
        assert_eq!(err(), GL_NO_ERROR, "glBlitFramebuffer");
        // SAFETY: plain GL call.
        unsafe { glBindFramebuffer(GL_READ_FRAMEBUFFER, fb8) };
        let got = read_bound(s);
        let want = stored(sentinel(pick));
        assert!(got.iter().all(|p| *p == want), "blit {kind:?}: {:?} want {want:?}", got[0]);
        // SAFETY: names made above.
        unsafe {
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
            glDeleteFramebuffers(1, &fb);
            glDeleteFramebuffers(1, &fb8);
            glDeleteRenderbuffers(1, &rb);
            glDeleteTextures(1, &tex);
        }
    }

    #[test]
    fn layers_fallback_cube_face_attachment() {
        run(Kind::Cube);
    }

    #[test]
    fn layers_fallback_array_layer_attachment() {
        run(Kind::Array);
    }

    #[test]
    fn layers_fallback_volume_slice_attachment() {
        run(Kind::Volume);
    }

    #[test]
    fn layers_fallback_mip_level_attachment() {
        run(Kind::Level);
    }
}
