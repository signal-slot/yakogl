// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! ES 3.0 framebuffer completeness (item fbo-fragops of plan #19): one test
//! per root cause, through the GL API on the host, with the dEQP-GLES3 cases
//! each one stands for. The host dEQP run that found them, with the verdicts
//! before and after, is docs/handoff/20261014-es3-fbo-completeness.md.
//!
//!   - Depth and stencil images need not be the colour's size (ES 3.0
//!     4.4.4.2 has no size rule): fbo.completeness.size.distinct.
//!   - Colour attachment 0 may be empty while 1.. are not: the 14
//!     fbo.completeness.attachment_combinations.none_{rbo,tex}_* cases.
//!   - Sample counts: one image of one sample beside one of none, or beside a
//!     texture, is GL_FRAMEBUFFER_INCOMPLETE_MULTISAMPLE; counts past
//!     GL_MAX_SAMPLES are GL_INVALID_OPERATION (the cases end NotSupported):
//!     fbo.completeness.samples.*.
//!   - A layer of a 2D-array or 3D texture is complete when it is smaller
//!     than the texture's depth, incomplete when not: fbo.completeness.layer.*.
//!   - The depth formats of table 3.13 attach as renderbuffers and as
//!     textures, with the sizes and component types they report:
//!     fbo.completeness.renderable.*.depth and the fbo.depth.* attach step.
//!
//! The context is the smallest OSAL and back end that make one, as in
//! tests/es3_gl_core.rs; no pixel is rendered. An ES 2.0-reporting build
//! (`report-es2`) runs `es2_report_is_unchanged` only: every rule above is
//! ES 3.0's, and the ES 2.0 report keeps its answers.

#![cfg(feature = "host-test")]
// the ES 2.0 report runs one test of the file; the helpers of the rest are idle there
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
    lock: None,
    unlock: None,
    thread_id: None,
};

static BACKEND: YakoglBackend = YakoglBackend {
    name: c"es3-fbo-fragops".as_ptr() as *const c_char,
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
        while glGetError() != GL_NO_ERROR {}
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
    }
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

/// A framebuffer, bound, with nothing attached.
fn framebuffer() -> GLuint {
    let mut fb = 0;
    // SAFETY: room for one name.
    unsafe {
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
    }
    fb
}

/// A renderbuffer of `format` (`samples` 0: glRenderbufferStorage), bound.
fn renderbuffer(format: GLenum, w: GLsizei, h: GLsizei, samples: GLsizei) -> GLuint {
    let mut rb = 0;
    // SAFETY: room for one name.
    unsafe {
        glGenRenderbuffers(1, &mut rb);
        glBindRenderbuffer(GL_RENDERBUFFER, rb);
        if samples == 0 {
            glRenderbufferStorage(GL_RENDERBUFFER, format, w, h);
        } else {
            glRenderbufferStorageMultisample(GL_RENDERBUFFER, samples, format, w, h);
        }
    }
    rb
}

fn attach_rb(attachment: GLenum, rb: GLuint) {
    // SAFETY: plain GL call.
    unsafe { glFramebufferRenderbuffer(GL_FRAMEBUFFER, attachment, GL_RENDERBUFFER, rb) };
}

/// A 2D texture of `internalformat` / `format` / `type_`, `w` x `h`, level 0
/// allocated, bound.
fn texture(internalformat: GLenum, format: GLenum, type_: GLenum, w: GLsizei, h: GLsizei) -> GLuint {
    let mut t = 0;
    // SAFETY: room for one name; null pixels only allocate.
    unsafe {
        glGenTextures(1, &mut t);
        glBindTexture(GL_TEXTURE_2D, t);
        glTexImage2D(GL_TEXTURE_2D, 0, internalformat as GLint, w, h, 0, format, type_, core::ptr::null());
    }
    t
}

fn rgba_texture(w: GLsizei, h: GLsizei) -> GLuint {
    texture(GL_RGBA8, GL_RGBA, GL_UNSIGNED_BYTE, w, h)
}

fn attach_tex(attachment: GLenum, t: GLuint) {
    // SAFETY: plain GL call.
    unsafe { glFramebufferTexture2D(GL_FRAMEBUFFER, attachment, GL_TEXTURE_2D, t, 0) };
}

/// A `target` (2D array or 3D) RGBA8 texture of `w` x `h` x `depth`, bound.
fn layered_texture(target: GLenum, w: GLsizei, h: GLsizei, depth: GLsizei) -> GLuint {
    let mut t = 0;
    // SAFETY: room for one name; null pixels only allocate.
    unsafe {
        glGenTextures(1, &mut t);
        glBindTexture(target, t);
        glTexImage3D(target, 0, GL_RGBA8 as GLint, w, h, depth, 0, GL_RGBA, GL_UNSIGNED_BYTE, core::ptr::null());
    }
    t
}

fn attach_layer(t: GLuint, layer: GLint) {
    // SAFETY: plain GL call.
    unsafe { glFramebufferTextureLayer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, t, 0, layer) };
}

fn release(fb: GLuint) {
    // SAFETY: a name this test made.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glDeleteFramebuffers(1, &fb);
    }
}

fn attachment_param(attachment: GLenum, pname: GLenum) -> GLint {
    let mut v = -1;
    // SAFETY: room for one value.
    unsafe { glGetFramebufferAttachmentParameteriv(GL_FRAMEBUFFER, attachment, pname, &mut v) };
    v
}

/* ---- size.distinct --------------------------------------------------------- */

/// dEQP fbo.completeness.size.distinct: a 64 x 64 colour renderbuffer and a
/// 128 x 128 depth one. ES 3.0 4.4.4.2 has no rule that the sizes agree
/// (GL_FRAMEBUFFER_INCOMPLETE_DIMENSIONS "cannot be generated"); the back
/// ends are given the depth size apart (`GlesDraw::ds_w`) and render the
/// intersection. Smaller depth, a stencil of another size and a texture
/// image are the same rule.
#[test]
#[cfg(not(feature = "report-es2"))]
fn images_of_different_sizes_are_complete() {
    let _g = context();
    let fb = framebuffer();
    attach_rb(GL_COLOR_ATTACHMENT0, renderbuffer(GL_RGBA8, 64, 64, 0));
    attach_rb(GL_DEPTH_ATTACHMENT, renderbuffer(GL_DEPTH_COMPONENT16, 128, 128, 0));
    assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE, "depth larger than colour (size.distinct)");

    attach_rb(GL_DEPTH_ATTACHMENT, renderbuffer(GL_DEPTH_COMPONENT16, 32, 16, 0));
    assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE, "depth smaller than colour");

    attach_rb(GL_DEPTH_ATTACHMENT, 0);
    attach_rb(GL_STENCIL_ATTACHMENT, renderbuffer(GL_STENCIL_INDEX8, 8, 8, 0));
    assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE, "stencil of another size");

    attach_rb(GL_STENCIL_ATTACHMENT, 0);
    attach_tex(GL_COLOR_ATTACHMENT0, rgba_texture(16, 16));
    attach_rb(GL_DEPTH_ATTACHMENT, renderbuffer(GL_DEPTH_COMPONENT24, 64, 64, 0));
    assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE, "a texture and a renderbuffer of another size");

    /* an image with no size is still incomplete (size.zero) */
    attach_rb(GL_DEPTH_ATTACHMENT, 0);
    attach_rb(GL_COLOR_ATTACHMENT0, renderbuffer(GL_RGBA8, 0, 0, 0));
    assert_eq!(status(), GL_FRAMEBUFFER_INCOMPLETE_ATTACHMENT, "size.zero");
    assert_eq!(err(), GL_NO_ERROR);
    release(fb);
}

/* ---- colour attachment 0 empty --------------------------------------------- */

/// dEQP fbo.completeness.attachment_combinations.none_{rbo,tex}_*: colour on
/// attachments 1.. only, with and without depth and stencil. ES 3.0 4.4.4.2
/// asks nothing of attachment 0. (A draw into such a framebuffer still has no
/// colour target; the draw buffers that name attachments 1.. are the MRT
/// item's.)
#[test]
#[cfg(not(feature = "report-es2"))]
fn colour_attachments_past_zero_complete_a_framebuffer_alone() {
    let _g = context();
    let fb = framebuffer();
    attach_rb(GL_COLOR_ATTACHMENT1, renderbuffer(GL_RGBA8, 64, 64, 0));
    assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE, "none_rbo_none_none");

    attach_tex(GL_COLOR_ATTACHMENT1, rgba_texture(64, 64));
    attach_tex(GL_COLOR_ATTACHMENT2, rgba_texture(64, 64));
    assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE, "none_tex_none_none, two textures");

    attach_rb(GL_DEPTH_STENCIL_ATTACHMENT, renderbuffer(GL_DEPTH24_STENCIL8, 64, 64, 0));
    assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE, "none_tex_rbo_rbo");

    /* attachment 1 still has to be an attachment-complete colour image */
    attach_rb(GL_COLOR_ATTACHMENT1, renderbuffer(GL_DEPTH_COMPONENT16, 64, 64, 0));
    assert_eq!(status(), GL_FRAMEBUFFER_INCOMPLETE_ATTACHMENT, "a depth format on attachment 1");

    /* nothing attached anywhere is still missing */
    attach_rb(GL_COLOR_ATTACHMENT1, 0);
    attach_tex(GL_COLOR_ATTACHMENT2, 0);
    attach_rb(GL_DEPTH_STENCIL_ATTACHMENT, 0);
    assert_eq!(status(), GL_FRAMEBUFFER_INCOMPLETE_MISSING_ATTACHMENT);
    assert_eq!(err(), GL_NO_ERROR);
    release(fb);
}

/* ---- depth and stencil as one image ---------------------------------------- */

/// dEQP fbo.completeness.attachment_combinations.{none,...}_*_{rbo_tex,tex_rbo}
/// with no colour attachment 0: a depth image of one kind and a stencil image
/// of another is GL_FRAMEBUFFER_UNSUPPORTED (ES 3.0 4.4.4.2), one image on
/// both points is complete.
#[test]
#[cfg(not(feature = "report-es2"))]
fn separate_depth_and_stencil_without_colour_zero_are_unsupported() {
    let _g = context();
    let fb = framebuffer();
    attach_rb(GL_COLOR_ATTACHMENT1, renderbuffer(GL_RGBA8, 64, 64, 0));
    attach_rb(GL_DEPTH_ATTACHMENT, renderbuffer(GL_DEPTH_COMPONENT16, 64, 64, 0));
    attach_tex(GL_STENCIL_ATTACHMENT, texture(GL_DEPTH24_STENCIL8, GL_DEPTH_STENCIL, GL_UNSIGNED_INT_24_8, 64, 64));
    assert_eq!(status(), GL_FRAMEBUFFER_UNSUPPORTED, "none_rbo_rbo_tex");

    let shared = renderbuffer(GL_DEPTH24_STENCIL8, 64, 64, 0);
    attach_rb(GL_DEPTH_ATTACHMENT, shared);
    attach_rb(GL_STENCIL_ATTACHMENT, shared);
    assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE, "none_rbo_rbo_rbo: one D24S8 on both points");

    attach_rb(GL_COLOR_ATTACHMENT1, 0);
    attach_rb(GL_DEPTH_ATTACHMENT, renderbuffer(GL_DEPTH_COMPONENT16, 64, 64, 0));
    attach_rb(GL_STENCIL_ATTACHMENT, renderbuffer(GL_STENCIL_INDEX8, 64, 64, 0));
    assert_eq!(status(), GL_FRAMEBUFFER_UNSUPPORTED, "none_none_rbo_rbo with two renderbuffers");
    assert_eq!(err(), GL_NO_ERROR);
    release(fb);
}

/// dEQP fbo.completeness.attachment_combinations.{rbo,tex}_*_{rbo_tex,tex_rbo}
/// (8 cases, 4 more with colour on 1.. too): the same rule with a colour
/// attachment 0. Open: unit tests of draw.rs
/// (`fbo_depth_and_stencil_renderbuffers_both_name_the_ds_buffer`,
/// `one_depth_stencil_texture_on_both_points_is_one_object`) and of
/// v3d/backend_tests.rs (`fbos_sharing_a_depth_renderbuffer_with_their_own_
/// stencil`, `a_recreated_stencil_renderbuffer_keeps_the_fbos_depth`) build
/// ES 2.0-style separate depth and stencil renderbuffers on an ES 3.0 build
/// and expect them complete; they move with the rule, in files this item
/// does not own (docs/handoff/20261014-es3-fbo-completeness.md).
#[test]
#[ignore = "separate depth and stencil images with a colour 0 are still complete (draw.rs and v3d unit tests build them)"]
fn separate_depth_and_stencil_with_colour_zero_are_unsupported() {
    let _g = context();
    let fb = framebuffer();
    attach_rb(GL_COLOR_ATTACHMENT0, renderbuffer(GL_RGBA8, 64, 64, 0));
    attach_rb(GL_DEPTH_ATTACHMENT, renderbuffer(GL_DEPTH_COMPONENT16, 64, 64, 0));
    attach_rb(GL_STENCIL_ATTACHMENT, renderbuffer(GL_STENCIL_INDEX8, 64, 64, 0));
    assert_eq!(status(), GL_FRAMEBUFFER_UNSUPPORTED, "rbo_none_rbo_rbo with two renderbuffers");
    release(fb);
}

/* ---- samples --------------------------------------------------------------- */

/// dEQP fbo.completeness.samples.*: ES 3.0 4.4.4.2 wants every attached
/// renderbuffer to have one GL_RENDERBUFFER_SAMPLES, and zero if a texture is
/// attached. GL_MAX_SAMPLES is 1 (`rbo2_*` and `rbo4_*` cases end
/// NotSupported on the GL_INVALID_OPERATION below).
#[test]
#[cfg(not(feature = "report-es2"))]
fn mixed_sample_counts_are_incomplete_multisample() {
    let _g = context();
    let fb = framebuffer();
    let one = renderbuffer(GL_RGBA8, 64, 64, 1);
    let none = renderbuffer(GL_RGBA8, 64, 64, 0);
    let mut max = 0;
    // SAFETY: room for one value.
    unsafe { glGetIntegerv(GL_MAX_SAMPLES, &mut max) };
    assert_eq!(max, 1);
    assert_eq!(err(), GL_NO_ERROR, "one sample is storage");

    attach_rb(GL_COLOR_ATTACHMENT0, one);
    assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE, "rbo1_none_none");
    attach_rb(GL_COLOR_ATTACHMENT0, none);
    assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE, "rbo0_none_none");

    attach_rb(GL_COLOR_ATTACHMENT0, one);
    attach_rb(GL_COLOR_ATTACHMENT1, renderbuffer(GL_RGBA8, 64, 64, 1));
    attach_rb(GL_COLOR_ATTACHMENT2, renderbuffer(GL_RGBA8, 64, 64, 1));
    assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE, "rbo1_rbo1_rbo1");

    attach_rb(GL_COLOR_ATTACHMENT1, none);
    assert_eq!(status(), GL_FRAMEBUFFER_INCOMPLETE_MULTISAMPLE, "one and zero samples");

    attach_rb(GL_COLOR_ATTACHMENT1, 0);
    attach_rb(GL_COLOR_ATTACHMENT2, 0);
    attach_tex(GL_COLOR_ATTACHMENT1, rgba_texture(64, 64));
    assert_eq!(status(), GL_FRAMEBUFFER_INCOMPLETE_MULTISAMPLE, "rbo1_tex_none: a texture counts as zero");

    attach_rb(GL_COLOR_ATTACHMENT0, none);
    assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE, "rbo0_tex_none");

    /* a count past GL_MAX_SAMPLES is refused when the storage is made */
    for samples in [2, 4] {
        let rb = renderbuffer(GL_RGBA8, 64, 64, samples);
        assert_eq!(err(), GL_INVALID_OPERATION, "{samples} samples");
        attach_rb(GL_COLOR_ATTACHMENT0, rb);
        let _ = err();
    }
    release(fb);
}

/* ---- layers ---------------------------------------------------------------- */

/// dEQP fbo.completeness.layer.2darr_*: a layer of a 2D array texture is
/// complete when it is smaller than the number of layers (ES 3.0 4.4.4.2),
/// incomplete when not; glFramebufferTextureLayer itself only refuses a layer
/// past GL_MAX_ARRAY_TEXTURE_LAYERS (GL_INVALID_VALUE).
#[test]
#[cfg(not(feature = "report-es2"))]
fn a_2d_array_layer_is_complete_below_the_layer_count() {
    let _g = context();
    let fb = framebuffer();
    for (layers, layer, want) in [
        (1, 0, GL_FRAMEBUFFER_COMPLETE),
        (1, 3, GL_FRAMEBUFFER_INCOMPLETE_ATTACHMENT),
        (4, 3, GL_FRAMEBUFFER_COMPLETE),
        (4, 15, GL_FRAMEBUFFER_INCOMPLETE_ATTACHMENT),
        (4, 4, GL_FRAMEBUFFER_INCOMPLETE_ATTACHMENT),
    ] {
        let t = layered_texture(GL_TEXTURE_2D_ARRAY, 64, 64, layers);
        attach_layer(t, layer);
        assert_eq!(err(), GL_NO_ERROR, "{layers} layers, layer {layer}");
        assert_eq!(status(), want, "2darr_{layers}_{layer}");
        assert_eq!(attachment_param(GL_COLOR_ATTACHMENT0, GL_FRAMEBUFFER_ATTACHMENT_TEXTURE_LAYER), layer);
    }
    let t = layered_texture(GL_TEXTURE_2D_ARRAY, 8, 8, 4);
    attach_layer(t, 256);
    assert_eq!(err(), GL_INVALID_VALUE, "past GL_MAX_ARRAY_TEXTURE_LAYERS");
    attach_layer(t, -1);
    assert_eq!(err(), GL_INVALID_VALUE, "a negative layer");
    release(fb);
}

/// dEQP fbo.completeness.layer.3d_*: the same rule on the depth of a 3D
/// texture. A slice below the depth of a texture deeper than one is a
/// sub-rectangle of the grid the PowerVR and GC7000 keep 3D textures in
/// (`GlTexture::grid_cols`), which `attachment_storage` cannot render into
/// yet: see `a_3d_slice_below_the_depth_is_complete` for what is open.
#[test]
#[cfg(not(feature = "report-es2"))]
fn a_3d_layer_past_the_depth_is_incomplete() {
    let _g = context();
    let fb = framebuffer();
    for (depth, layer, want) in [
        (1, 0, GL_FRAMEBUFFER_COMPLETE),
        (1, 15, GL_FRAMEBUFFER_INCOMPLETE_ATTACHMENT),
        (4, 15, GL_FRAMEBUFFER_INCOMPLETE_ATTACHMENT),
        (4, 4, GL_FRAMEBUFFER_INCOMPLETE_ATTACHMENT),
    ] {
        let t = layered_texture(GL_TEXTURE_3D, 64, 64, depth);
        attach_layer(t, layer);
        assert_eq!(err(), GL_NO_ERROR, "depth {depth}, layer {layer}");
        assert_eq!(status(), want, "3d_{depth}_{layer}");
    }
    release(fb);
}

/// dEQP fbo.completeness.layer.3d_64_15 (and every 3d_N_M with M < N, N > 1):
/// complete. Open: on a back end that does not walk slices itself
/// (`native_layered_textures`) the slices sit in a grid, a slice has the grid's
/// row pitch, and `attachment_storage` refuses the whole texture
/// (`t.grid_cols > 1`). The fix is the grid slice's offset and pitch through
/// `loc_pitch`, which the draw (draw.rs), the blit (blit.rs) and the readers
/// all have to follow; docs/handoff/20261014-es3-fbo-completeness.md.
#[test]
#[ignore = "3D textures in a grid are not render targets yet (attachment_storage, grid_cols > 1)"]
fn a_3d_slice_below_the_depth_is_complete() {
    let _g = context();
    let fb = framebuffer();
    let t = layered_texture(GL_TEXTURE_3D, 64, 64, 64);
    attach_layer(t, 15);
    assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE, "3d_64_15");
    release(fb);
}

/* ---- depth formats --------------------------------------------------------- */

/// dEQP fbo.completeness.renderable.renderbuffer.depth.* and the attach step
/// of fbo.depth.*: every depth-renderable format of table 3.13 is
/// renderbuffer storage, attaches on the depth point and reports its sizes
/// and component type; the stencil half only where the format has one.
#[test]
#[cfg(not(feature = "report-es2"))]
fn depth_renderbuffers_attach_with_their_sizes() {
    let _g = context();
    let fb = framebuffer();
    attach_rb(GL_COLOR_ATTACHMENT0, renderbuffer(GL_RGBA8, 16, 16, 0));
    for (format, depth_bits, stencil_bits, ctype) in [
        (GL_DEPTH_COMPONENT16, 16, 0, GL_UNSIGNED_NORMALIZED),
        (GL_DEPTH_COMPONENT24, 24, 0, GL_UNSIGNED_NORMALIZED),
        (GL_DEPTH_COMPONENT32F, 32, 0, GL_FLOAT),
        (GL_DEPTH24_STENCIL8, 24, 8, GL_UNSIGNED_NORMALIZED),
        (GL_DEPTH32F_STENCIL8, 32, 8, GL_FLOAT),
    ] {
        let rb = renderbuffer(format, 16, 16, 0);
        assert_eq!(err(), GL_NO_ERROR, "{format:#x} is renderbuffer storage");
        attach_rb(GL_DEPTH_ATTACHMENT, rb);
        assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE, "{format:#x} on the depth point");
        assert_eq!(
            attachment_param(GL_DEPTH_ATTACHMENT, GL_FRAMEBUFFER_ATTACHMENT_DEPTH_SIZE),
            depth_bits,
            "{format:#x} depth size"
        );
        assert_eq!(
            attachment_param(GL_DEPTH_ATTACHMENT, GL_FRAMEBUFFER_ATTACHMENT_COMPONENT_TYPE),
            ctype as GLint,
            "{format:#x} component type"
        );
        attach_rb(GL_DEPTH_ATTACHMENT, 0);
        attach_rb(GL_STENCIL_ATTACHMENT, rb);
        let want = if stencil_bits > 0 { GL_FRAMEBUFFER_COMPLETE } else { GL_FRAMEBUFFER_INCOMPLETE_ATTACHMENT };
        assert_eq!(status(), want, "{format:#x} on the stencil point");
        attach_rb(GL_STENCIL_ATTACHMENT, 0);
        if stencil_bits > 0 {
            attach_rb(GL_DEPTH_STENCIL_ATTACHMENT, rb);
            assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE, "{format:#x} on both points");
            assert_eq!(
                attachment_param(GL_STENCIL_ATTACHMENT, GL_FRAMEBUFFER_ATTACHMENT_STENCIL_SIZE),
                stencil_bits,
                "{format:#x} stencil size"
            );
            attach_rb(GL_DEPTH_STENCIL_ATTACHMENT, 0);
        }
    }
    release(fb);
}

/// dEQP fbo.completeness.renderable.texture.{depth,stencil}.* for the sized
/// depth formats: a depth texture attaches on the depth point, a
/// depth-stencil one on depth, stencil and both; a depth-only texture is no
/// stencil image.
#[test]
#[cfg(not(feature = "report-es2"))]
fn depth_textures_attach_with_their_formats() {
    let _g = context();
    let fb = framebuffer();
    attach_tex(GL_COLOR_ATTACHMENT0, rgba_texture(16, 16));
    for (internal, format, type_, stencil) in [
        (GL_DEPTH_COMPONENT16, GL_DEPTH_COMPONENT, GL_UNSIGNED_SHORT, false),
        (GL_DEPTH_COMPONENT24, GL_DEPTH_COMPONENT, GL_UNSIGNED_INT, false),
        (GL_DEPTH_COMPONENT32F, GL_DEPTH_COMPONENT, GL_FLOAT, false),
        (GL_DEPTH24_STENCIL8, GL_DEPTH_STENCIL, GL_UNSIGNED_INT_24_8, true),
        (GL_DEPTH32F_STENCIL8, GL_DEPTH_STENCIL, GL_FLOAT_32_UNSIGNED_INT_24_8_REV, true),
    ] {
        let t = texture(internal, format, type_, 16, 16);
        assert_eq!(err(), GL_NO_ERROR, "{internal:#x} is a texture format");
        attach_tex(GL_DEPTH_ATTACHMENT, t);
        assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE, "{internal:#x} on the depth point");
        attach_tex(GL_DEPTH_ATTACHMENT, 0);
        attach_tex(GL_STENCIL_ATTACHMENT, t);
        let want = if stencil { GL_FRAMEBUFFER_COMPLETE } else { GL_FRAMEBUFFER_INCOMPLETE_ATTACHMENT };
        assert_eq!(status(), want, "{internal:#x} on the stencil point");
        attach_tex(GL_STENCIL_ATTACHMENT, 0);
        /* neither is a colour image */
        attach_tex(GL_COLOR_ATTACHMENT0, t);
        assert_eq!(status(), GL_FRAMEBUFFER_INCOMPLETE_ATTACHMENT, "{internal:#x} as colour");
        attach_tex(GL_COLOR_ATTACHMENT0, rgba_texture(16, 16));
    }
    release(fb);
}

/* ---- the ES 2.0 report ----------------------------------------------------- */

/// None of the above reaches the ES 2.0 surface: attachments of different
/// sizes are GL_FRAMEBUFFER_INCOMPLETE_DIMENSIONS there (2.0.25 4.4.5) and
/// colour on attachment 1 alone is unsupported.
#[test]
#[cfg(feature = "report-es2")]
fn es2_report_is_unchanged() {
    let _g = context();
    let fb = framebuffer();
    attach_tex(GL_COLOR_ATTACHMENT0, rgba_texture(64, 64));
    attach_rb(GL_DEPTH_ATTACHMENT, renderbuffer(GL_DEPTH_COMPONENT16, 128, 128, 0));
    assert_eq!(status(), GL_FRAMEBUFFER_INCOMPLETE_DIMENSIONS);
    attach_rb(GL_DEPTH_ATTACHMENT, 0);
    attach_rb(GL_STENCIL_ATTACHMENT, renderbuffer(GL_STENCIL_INDEX8, 8, 8, 0));
    assert_eq!(status(), GL_FRAMEBUFFER_INCOMPLETE_DIMENSIONS);
    release(fb);

    let fb = framebuffer();
    attach_tex(GL_COLOR_ATTACHMENT1, rgba_texture(64, 64));
    assert_eq!(status(), GL_FRAMEBUFFER_UNSUPPORTED);
    release(fb);
}
